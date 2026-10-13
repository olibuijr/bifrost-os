//! `scutil`: reads and sets the computer name, the host name and the local host name (`--get`, `--set`), and prints the DNS
//! resolvers (`--dns`) and the proxy settings (`--proxy`). The names are the `bifrost_core::host_name` model, saved in the
//! shell's state directory. Gap: the kernel host name is not changed, because that needs the privileged service, which
//! the command line cannot reach yet. Gap: `--dns` and `--proxy` only read (see `bifrost_cmd::network`).

use crate::network::Resolver;
use bifrost_core::host_name::HostNames;
use bifrost_core::proxy::{ProxyEndpoint, ProxySettings};
use std::fs;
use std::path::{Path, PathBuf};

pub const USAGE: &str = "usage: scutil --get <ComputerName | LocalHostName | HostName> | --set <ComputerName | LocalHostName | HostName> <value> | --dns | --proxy";
/// The name the machine has before anyone sets one, when the kernel host name cannot be used.
pub const DEFAULT_COMPUTER_NAME: &str = "BifrOSt";
/// The file the names are saved in, inside the shell's state directory.
pub const NAMES_FILE: &str = "host-names";

/// The preferences `--get` and `--set` accept. `HostName` is the local host name: there is no separate domain name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pref {
    ComputerName,
    LocalHostName,
    HostName,
}

impl Pref {
    pub fn parse(name: &str) -> Option<Pref> {
        match name {
            "ComputerName" => Some(Pref::ComputerName),
            "LocalHostName" => Some(Pref::LocalHostName),
            "HostName" => Some(Pref::HostName),
            _ => None,
        }
    }
}

/// One parsed option.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Get(Pref),
    Set(Pref, String),
    Dns,
    Proxy,
}

/// Reads the arguments (without the program name).
pub fn parse(args: &[String]) -> Result<Command, String> {
    let pref = |name: &str| Pref::parse(name).ok_or_else(|| format!("unknown preference {name}"));
    match args.iter().map(String::as_str).collect::<Vec<&str>>().as_slice() {
        [] | ["-h"] | ["--help"] => Ok(Command::Help),
        ["--get", name] => pref(*name).map(Command::Get),
        ["--set", name, value] => pref(*name).map(|p| Command::Set(p, value.to_string())),
        ["--set", _] => Err("--set takes a preference and a value".to_string()),
        ["--dns"] => Ok(Command::Dns),
        ["--proxy"] => Ok(Command::Proxy),
        [option, ..] => Err(format!("unsupported option {option}; this command reads and sets names, DNS and proxies")),
    }
}

/// The names as the state file holds them, one `Key=value` line each.
pub fn names_text(names: &HostNames) -> String {
    format!("ComputerName={}\nLocalHostName={}\n", names.computer_name().as_str(), names.local_host_name().as_str())
}

/// The names from the state file's text. A saved computer name the model refuses falls back to the computer name the
/// kernel reports (`default_computer`), then to the default name. A saved local host name is applied when it is valid.
pub fn names_from_text(text: &str, default_computer: &str) -> HostNames {
    let field = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=')).map(|v| v.trim().to_string());
    let saved = field("ComputerName").filter(|v| !v.is_empty());
    let mut names = saved
        .as_deref()
        .and_then(|v| HostNames::from_computer_name(v).ok())
        .or_else(|| HostNames::from_computer_name(default_computer).ok())
        .unwrap_or_else(|| HostNames::from_computer_name(DEFAULT_COMPUTER_NAME).expect("the default computer name is valid"));
    if let Some(local) = field("LocalHostName") {
        let _ = names.set_local_host_name(&local);
    }
    names
}

/// The state directory the shell keeps its files in.
pub fn state_dir() -> PathBuf {
    match std::env::var_os("BIFROST_SHELL_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from("/var/lib/springboard"),
    }
}

/// The names the machine has now: the saved ones, or the kernel's host name when nothing is saved.
pub fn load_names(dir: &Path) -> HostNames {
    let kernel = fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default();
    let text = fs::read_to_string(dir.join(NAMES_FILE)).unwrap_or_default();
    let default = if kernel.trim().is_empty() { DEFAULT_COMPUTER_NAME } else { kernel.trim() };
    names_from_text(&text, default)
}

/// Saves the names in the state directory.
pub fn save_names(dir: &Path, names: &HostNames) -> Result<(), String> {
    fs::write(dir.join(NAMES_FILE), names_text(names)).map_err(|e| format!("cannot save the names: {e}"))
}

/// The DNS resolvers, in the layout of the system configuration dump. The resolver file is one resolver.
pub fn dns_text(resolver: &Resolver) -> String {
    let mut out = String::from("DNS configuration\n\n");
    if resolver.servers.is_empty() && resolver.search.is_empty() {
        return out;
    }
    out.push_str("resolver #1\n");
    for (i, domain) in resolver.search.iter().enumerate() {
        out.push_str(&format!("  search domain[{i}] : {domain}\n"));
    }
    for (i, server) in resolver.servers.iter().enumerate() {
        out.push_str(&format!("  nameserver[{i}] : {server}\n"));
    }
    out.push_str("  flags    : Request A records\n\n");
    out
}

/// The proxy settings as a dictionary: the enabled flag and the server and port of each protocol, then the bypass list.
pub fn proxy_text(settings: &ProxySettings) -> String {
    let mut out = String::from("<dictionary> {\n");
    push_proxy(&mut out, "HTTP", settings.http.as_ref());
    push_proxy(&mut out, "HTTPS", settings.https.as_ref());
    push_proxy(&mut out, "SOCKS", settings.socks.as_ref());
    out.push_str(&format!("  ExcludeSimpleHostnames : {}\n", u8::from(settings.bypass_simple_hostnames)));
    if settings.bypass.is_empty() {
        out.push_str("  ExceptionsList : <array> {\n  }\n");
    } else {
        out.push_str("  ExceptionsList : <array> {\n");
        for (i, host) in settings.bypass.iter().enumerate() {
            out.push_str(&format!("    {i} : {host}\n"));
        }
        out.push_str("  }\n");
    }
    out.push_str("}\n");
    out
}

fn push_proxy(out: &mut String, key: &str, endpoint: Option<&ProxyEndpoint>) {
    match endpoint {
        Some(e) => out.push_str(&format!("  {key}Enable : 1\n  {key}Proxy : {}\n  {key}Port : {}\n", e.host(), e.port())),
        None => out.push_str(&format!("  {key}Enable : 0\n")),
    }
}

/// The text for one command. `names` are saved under `dir` when a name is set. `resolver` and `proxy` are read by the caller.
pub fn run(cmd: &Command, dir: &Path, resolver: &Resolver, proxy: &ProxySettings) -> Result<String, String> {
    match cmd {
        Command::Help => Ok(format!("{USAGE}\n")),
        Command::Get(pref) => {
            let names = load_names(dir);
            Ok(format!("{}\n", value_of(&names, *pref)))
        }
        Command::Set(pref, value) => {
            let mut names = load_names(dir);
            match pref {
                Pref::ComputerName => names.set_computer_name(value).map_err(|e| e.to_string())?,
                Pref::LocalHostName | Pref::HostName => names.set_local_host_name(value).map_err(|e| e.to_string())?,
            }
            save_names(dir, &names)?;
            Ok(String::new())
        }
        Command::Dns => Ok(dns_text(resolver)),
        Command::Proxy => Ok(proxy_text(proxy)),
    }
}

fn value_of(names: &HostNames, pref: Pref) -> &str {
    match pref {
        Pref::ComputerName => names.computer_name().as_str(),
        Pref::LocalHostName | Pref::HostName => names.local_host_name().as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn options_parse_into_commands() {
        assert_eq!(parse(&args(&["--get", "ComputerName"])), Ok(Command::Get(Pref::ComputerName)));
        assert_eq!(parse(&args(&["--set", "HostName", "desk"])), Ok(Command::Set(Pref::HostName, "desk".into())));
        assert_eq!(parse(&args(&["--dns"])), Ok(Command::Dns));
        assert_eq!(parse(&args(&["--proxy"])), Ok(Command::Proxy));
        assert_eq!(parse(&[]), Ok(Command::Help));
    }

    #[test]
    fn unknown_preferences_and_options_are_refused() {
        assert!(parse(&args(&["--get", "Nope"])).unwrap_err().contains("unknown preference"));
        assert!(parse(&args(&["--set", "ComputerName"])).is_err());
        assert!(parse(&args(&["-r", "example.org"])).unwrap_err().contains("unsupported option -r"));
    }

    #[test]
    fn saved_names_round_trip_and_the_local_name_stays_its_own() {
        let mut names = HostNames::from_computer_name("Olafs Desk").unwrap();
        names.set_local_host_name("studio").unwrap();
        let text = names_text(&names);
        assert_eq!(text, "ComputerName=Olafs Desk\nLocalHostName=studio\n");
        assert_eq!(names_from_text(&text, "ignored"), names);
    }

    #[test]
    fn missing_or_refused_names_fall_back() {
        let kernel = names_from_text("", "kernel-box");
        assert_eq!(kernel.computer_name().as_str(), "kernel-box");
        assert_eq!(kernel.local_host_name().as_str(), "kernel-box");
        let refused = names_from_text("ComputerName=\u{7}bad\n", "kernel-box");
        assert_eq!(refused.computer_name().as_str(), "kernel-box", "a name with a control character is not kept");
    }

    #[test]
    fn setting_a_computer_name_derives_the_local_name() {
        let dir = std::env::temp_dir().join(format!("scutil-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let set = run(&Command::Set(Pref::ComputerName, "Living Room".into()), &dir, &Resolver::default(), &ProxySettings::default());
        assert_eq!(set, Ok(String::new()));
        let host = run(&Command::Get(Pref::HostName), &dir, &Resolver::default(), &ProxySettings::default());
        assert_eq!(host, Ok("living-room\n".to_string()));
        let bad = run(&Command::Set(Pref::LocalHostName, "no good".into()), &dir, &Resolver::default(), &ProxySettings::default());
        assert!(bad.unwrap_err().contains("not allowed"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dns_dump_lists_search_domains_and_servers() {
        let r = Resolver { servers: vec!["1.1.1.1".into(), "8.8.4.4".into()], search: vec!["lan".into()] };
        let text = dns_text(&r);
        assert!(text.starts_with("DNS configuration\n\nresolver #1\n  search domain[0] : lan\n"));
        assert!(text.contains("  nameserver[1] : 8.8.4.4\n"));
        assert_eq!(dns_text(&Resolver::default()), "DNS configuration\n\n");
    }

    #[test]
    fn proxy_dump_names_each_protocol_and_the_bypass_list() {
        let mut s = ProxySettings::default();
        s.http = ProxyEndpoint::new("web.lan", 8080);
        s.bypass = vec!["localhost".into()];
        let text = proxy_text(&s);
        assert!(text.contains("  HTTPEnable : 1\n  HTTPProxy : web.lan\n  HTTPPort : 8080\n"));
        assert!(text.contains("  HTTPSEnable : 0\n"));
        assert!(text.contains("  ExceptionsList : <array> {\n    0 : localhost\n  }\n"));
        assert!(text.ends_with("}\n"));
    }
}
