//! Network facts for the command line tools: the hardware ports and the network services that carry them, the Wi-Fi
//! state and commands through the network service's socket, the name servers and search domains from the resolver file,
//! and the proxy settings the environment names.
//! Gap: the network service owns Wi-Fi only. It has no DNS or proxy surface, so the DNS and proxy readers here see the
//! resolver file and the environment, and nothing the command line writes there is applied.

use bifrost_core::proxy::{ProxyEndpoint, ProxySettings};
use bifrost_core::sysinfo::NetInterface;
use bifrost_proto::svc::{self, net::Cmd, net::Evt, net::Status};
use std::fs;
use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The network service's name; its socket is `<run dir>/<id>.sock`.
pub const NET_SERVICE_ID: &str = "is.olibuijr.net";
/// The resolver file the name servers are read from.
pub const RESOLV_CONF: &str = "/etc/resolv.conf";
const TIMEOUT: Duration = Duration::from_secs(3);

/// A hardware port: the name people see, the device under it, its address and its link state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Port {
    pub name: String,
    pub device: String,
    pub mac: String,
    pub wireless: bool,
    /// "up", "down" or "unknown", as the kernel reports the link.
    pub state: String,
}

/// The hardware ports, in device order. Wi-Fi devices are named "Wi-Fi", wired ones "Ethernet", and later ones of
/// the same kind carry a number ("Ethernet 2"). The loopback device is not a port.
pub fn ports(links: &[NetInterface]) -> Vec<Port> {
    let (mut wifi, mut wired) = (0usize, 0usize);
    let mut out = Vec::new();
    for link in links.iter().filter(|l| l.name != "lo") {
        let name = if link.wireless {
            wifi += 1;
            numbered("Wi-Fi", wifi)
        } else {
            wired += 1;
            numbered("Ethernet", wired)
        };
        out.push(Port { name, device: link.name.clone(), mac: link.mac.clone(), wireless: link.wireless, state: link.state.clone() });
    }
    out
}

fn numbered(base: &str, n: usize) -> String {
    if n == 1 {
        base.to_string()
    } else {
        format!("{base} {n}")
    }
}

/// The port named `key`, by its name or its device, ignoring case.
pub fn find_port<'a>(ports: &'a [Port], key: &str) -> Option<&'a Port> {
    ports.iter().find(|p| p.name.eq_ignore_ascii_case(key) || p.device.eq_ignore_ascii_case(key))
}

/// The socket of the network service: `BIFROST_SERVICE_SOCK` when set, else the shell's run directory.
pub fn service_socket() -> PathBuf {
    match std::env::var_os("BIFROST_SERVICE_SOCK") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(format!("/var/lib/springboard/run/{NET_SERVICE_ID}.sock")),
    }
}

fn connect(path: &Path) -> Result<UnixStream, String> {
    let stream = UnixStream::connect(path).map_err(|e| format!("the network service is not reachable ({e})"))?;
    let _ = stream.set_read_timeout(Some(TIMEOUT));
    let _ = stream.set_write_timeout(Some(TIMEOUT));
    Ok(stream)
}

fn io_error(e: io::Error) -> String {
    e.to_string()
}

/// The network service's status, from its reply to a subscribe request.
pub fn status(path: &Path) -> Result<Status, String> {
    let mut stream = connect(path)?;
    svc::write_frame(&mut stream, &Cmd::Subscribe.encode()).map_err(io_error)?;
    let frame = svc::read_frame(&mut stream).map_err(io_error)?;
    match Evt::decode(&frame) {
        Ok(Evt::Status(status)) => Ok(status),
        Ok(_) => Err("the network service sent an unexpected reply".to_string()),
        Err(e) => Err(format!("the network service sent a reply that cannot be read ({e:?})")),
    }
}

/// Sends one command. The service applies it and broadcasts the new status to its subscribers; this does not wait.
pub fn send(path: &Path, cmd: &Cmd) -> Result<(), String> {
    let mut stream = connect(path)?;
    svc::write_frame(&mut stream, &cmd.encode()).map_err(io_error)
}

/// The name servers and search domains from a resolver file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolver {
    pub servers: Vec<String>,
    pub search: Vec<String>,
}

/// Reads the `nameserver` and `search` lines. `#` and `;` start comments.
pub fn parse_resolv(text: &str) -> Resolver {
    let mut out = Resolver::default();
    for line in text.lines() {
        let line = line.split(['#', ';']).next().unwrap_or("").trim();
        let mut words = line.split_ascii_whitespace();
        match words.next() {
            Some("nameserver") => {
                if let Some(server) = words.next() {
                    out.servers.push(server.to_string());
                }
            }
            Some("search") => out.search.extend(words.map(str::to_string)),
            _ => {}
        }
    }
    out
}

/// The resolver file's contents; an empty resolver when the file cannot be read.
pub fn read_resolver(path: &Path) -> Resolver {
    parse_resolv(&fs::read_to_string(path).unwrap_or_default())
}

/// The host and port a proxy URL names, with its scheme (`http` when none is written). `None` without a valid port.
pub fn parse_proxy_url(url: &str) -> Option<(String, ProxyEndpoint)> {
    let url = url.trim();
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        None => ("http".to_string(), url),
    };
    let authority = rest.split('/').next()?.rsplit('@').next()?;
    let (host, port) = authority.rsplit_once(':')?;
    let port: u16 = port.parse().ok()?;
    Some((scheme, ProxyEndpoint::new(host, port)?))
}

/// The proxy settings the environment names: `http_proxy` and `https_proxy`, `all_proxy` (a SOCKS proxy when its
/// scheme says so, else both web proxies), and `no_proxy` as the bypass list. `get` looks a variable up.
pub fn proxy_from_env(get: impl Fn(&str) -> Option<String>) -> ProxySettings {
    let read = |names: &[&str]| names.iter().find_map(|n| get(*n)).filter(|v| !v.trim().is_empty());
    let mut settings = ProxySettings::default();
    settings.http = read(&["http_proxy", "HTTP_PROXY"]).as_deref().and_then(parse_proxy_url).map(|(_, e)| e);
    settings.https = read(&["https_proxy", "HTTPS_PROXY"]).as_deref().and_then(parse_proxy_url).map(|(_, e)| e);
    if let Some((scheme, endpoint)) = read(&["all_proxy", "ALL_PROXY"]).as_deref().and_then(parse_proxy_url) {
        if scheme.starts_with("socks") {
            settings.socks = Some(endpoint);
        } else {
            settings.http = settings.http.or_else(|| Some(endpoint.clone()));
            settings.https = settings.https.or(Some(endpoint));
        }
    }
    settings.bypass = read(&["no_proxy", "NO_PROXY"])
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|h| !h.is_empty())
        .map(str::to_string)
        .collect();
    settings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(name: &str, wireless: bool, state: &str) -> NetInterface {
        NetInterface { name: name.into(), wireless, state: state.into(), mac: "aa:bb:cc:dd:ee:ff".into(), ..Default::default() }
    }

    #[test]
    fn ports_name_wifi_and_wired_devices_and_skip_loopback() {
        let links = [link("lo", false, "unknown"), link("wlan0", true, "up"), link("eth0", false, "down"), link("eth1", false, "up"), link("wlan1", true, "down")];
        let ports = ports(&links);
        let names: Vec<&str> = ports.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Wi-Fi", "Ethernet", "Ethernet 2", "Wi-Fi 2"]);
        assert_eq!(ports[0].device, "wlan0");
        assert!(ports[0].wireless);
    }

    #[test]
    fn a_port_is_found_by_name_or_device_ignoring_case() {
        let ports = ports(&[link("wlan0", true, "up")]);
        assert_eq!(find_port(&ports, "wi-fi").map(|p| p.device.as_str()), Some("wlan0"));
        assert_eq!(find_port(&ports, "WLAN0").map(|p| p.name.as_str()), Some("Wi-Fi"));
        assert!(find_port(&ports, "Thunderbolt").is_none());
    }

    #[test]
    fn resolver_lines_give_servers_and_search_domains() {
        let text = "# generated\nnameserver 1.1.1.1\nnameserver\t8.8.8.8 ; comment\nsearch lan example.org\nsearch  home\noptions edns0\n";
        let r = parse_resolv(text);
        assert_eq!(r.servers, ["1.1.1.1", "8.8.8.8"]);
        assert_eq!(r.search, ["lan", "example.org", "home"]);
        assert_eq!(parse_resolv(""), Resolver::default());
    }

    #[test]
    fn proxy_urls_give_scheme_host_and_port() {
        let (scheme, ep) = parse_proxy_url("http://user:pw@proxy.lan:3128/").unwrap();
        assert_eq!((scheme.as_str(), ep.host(), ep.port()), ("http", "proxy.lan", 3128));
        let (scheme, ep) = parse_proxy_url("socks5://10.0.0.2:1080").unwrap();
        assert_eq!((scheme.as_str(), ep.to_string().as_str()), ("socks5", "10.0.0.2:1080"));
        assert_eq!(parse_proxy_url("proxy.lan").map(|(s, _)| s), None);
        assert!(parse_proxy_url("http://proxy.lan").is_none(), "a port is required");
    }

    #[test]
    fn the_environment_fills_the_proxy_settings() {
        let env = |k: &str| match k {
            "http_proxy" => Some("http://web.lan:8080".to_string()),
            "all_proxy" => Some("socks5://sock.lan:1080".to_string()),
            "NO_PROXY" => Some("localhost, .lan".to_string()),
            _ => None,
        };
        let s = proxy_from_env(env);
        assert_eq!(s.http.as_ref().map(|e| e.port()), Some(8080));
        assert_eq!(s.https, None);
        assert_eq!(s.socks.as_ref().map(|e| e.host().to_string()), Some("sock.lan".to_string()));
        assert_eq!(s.bypass, ["localhost", ".lan"]);
        assert_eq!(proxy_from_env(|_| None), ProxySettings::default());
    }
}
