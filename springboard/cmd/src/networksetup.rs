//! `networksetup`: lists the network services and hardware ports, reads and drives Wi-Fi (power, the joined network and the
//! saved networks), and reads the DNS servers, search domains and proxies. The Wi-Fi state comes from and goes to the
//! network service's socket (`bifrost_cmd::network`). See `network` for the gaps: DNS and proxy settings are read-only.

use crate::network::{self, Port, Resolver};
use bifrost_core::proxy::{ProxyEndpoint, ProxySettings};
use bifrost_core::sysinfo::NetInterface;
use bifrost_proto::svc::net::{Cmd, Status};
use std::path::Path;

pub const USAGE: &str = "usage: networksetup -listallnetworkservices | -listallhardwareports | -getairportpower <port> | \
-setairportpower <port> on|off | -getairportnetwork <port> | -setairportnetwork <port> <network> [password] | \
-listpreferredwirelessnetworks <port> | -removepreferredwirelessnetwork <port> <network> | -getdnsservers <service> | \
-getsearchdomains <service> | -getwebproxy <service> | -getsecurewebproxy <service> | \
-getsocksfirewallproxy <service> | -getproxybypassdomains <service>";

/// Which proxy a `-get...proxy` option reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyName {
    Web,
    SecureWeb,
    Socks,
}

/// One parsed option.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    ListServices,
    ListHardwarePorts,
    GetAirportPower(String),
    SetAirportPower(String, bool),
    GetAirportNetwork(String),
    SetAirportNetwork { port: String, network: String, password: Option<String> },
    ListPreferred(String),
    RemovePreferred { port: String, network: String },
    GetDnsServers(String),
    GetSearchDomains(String),
    GetProxy(ProxyName, String),
    GetBypassDomains(String),
}

/// Reads the arguments (without the program name). Options that change anything other than Wi-Fi are refused by name.
pub fn parse(args: &[String]) -> Result<Command, String> {
    let Some(option) = args.first() else { return Ok(Command::Help) };
    let operands: Vec<&str> = args[1..].iter().map(String::as_str).collect();
    let none = || -> Result<(), String> {
        if operands.is_empty() {
            Ok(())
        } else {
            Err(format!("{option} takes no operands"))
        }
    };
    let one = || -> Result<String, String> {
        match operands.as_slice() {
            [a] => Ok(a.to_string()),
            _ => Err(format!("{option} takes one operand")),
        }
    };
    match option.as_str() {
        "-help" | "--help" | "-h" => Ok(Command::Help),
        "-listallnetworkservices" => none().map(|()| Command::ListServices),
        "-listallhardwareports" => none().map(|()| Command::ListHardwarePorts),
        "-getairportpower" => one().map(Command::GetAirportPower),
        "-setairportpower" => match operands.as_slice() {
            [port, state] => match *state {
                "on" => Ok(Command::SetAirportPower(port.to_string(), true)),
                "off" => Ok(Command::SetAirportPower(port.to_string(), false)),
                _ => Err("-setairportpower takes on or off".to_string()),
            },
            _ => Err("-setairportpower takes a port and on or off".to_string()),
        },
        "-getairportnetwork" => one().map(Command::GetAirportNetwork),
        "-setairportnetwork" => match operands.as_slice() {
            [port, network] => Ok(Command::SetAirportNetwork { port: port.to_string(), network: network.to_string(), password: None }),
            [port, network, password] => {
                Ok(Command::SetAirportNetwork { port: port.to_string(), network: network.to_string(), password: Some(password.to_string()) })
            }
            _ => Err("-setairportnetwork takes a port, a network and an optional password".to_string()),
        },
        "-listpreferredwirelessnetworks" => one().map(Command::ListPreferred),
        "-removepreferredwirelessnetwork" => match operands.as_slice() {
            [port, network] => Ok(Command::RemovePreferred { port: port.to_string(), network: network.to_string() }),
            _ => Err("-removepreferredwirelessnetwork takes a port and a network".to_string()),
        },
        "-getdnsservers" => one().map(Command::GetDnsServers),
        "-getsearchdomains" => one().map(Command::GetSearchDomains),
        "-getwebproxy" => one().map(|s| Command::GetProxy(ProxyName::Web, s)),
        "-getsecurewebproxy" => one().map(|s| Command::GetProxy(ProxyName::SecureWeb, s)),
        "-getsocksfirewallproxy" => one().map(|s| Command::GetProxy(ProxyName::Socks, s)),
        "-getproxybypassdomains" => one().map(Command::GetBypassDomains),
        "-setdnsservers" | "-setsearchdomains" => {
            Err(format!("{option} needs the privileged network configuration, which the command line cannot reach yet"))
        }
        other if other.starts_with("-set") => Err(format!("{other} is not supported yet; this command reads proxies and DNS and only changes Wi-Fi")),
        other => Err(format!("unknown option {other}")),
    }
}

/// The text for one command. `links` are the system's network links, `resolver` the name servers, `proxy` the proxy
/// settings and `socket` the network service's socket. Errors are the message to print.
pub fn run(cmd: &Command, links: &[NetInterface], resolver: &Resolver, proxy: &ProxySettings, socket: &Path) -> Result<String, String> {
    let ports = network::ports(links);
    let port = |key: &str| network::find_port(&ports, key).cloned().ok_or_else(|| format!("The following hardware port was not found: {key}"));
    let wifi = |key: &str| -> Result<Port, String> {
        let p = port(key)?;
        if p.wireless {
            Ok(p)
        } else {
            Err(format!("{} is not a Wi-Fi port", p.name))
        }
    };
    match cmd {
        Command::Help => Ok(format!("{USAGE}\n")),
        Command::ListServices => Ok(list_services(&ports)),
        Command::ListHardwarePorts => Ok(list_hardware_ports(&ports)),
        Command::GetAirportPower(key) => {
            let p = wifi(key)?;
            Ok(format!("{}\n", airport_power_line(&p, &network::status(socket)?)))
        }
        Command::SetAirportPower(key, on) => {
            wifi(key)?;
            network::send(socket, &Cmd::Enable(*on))?;
            Ok(String::new())
        }
        Command::GetAirportNetwork(key) => {
            wifi(key)?;
            Ok(format!("{}\n", airport_network_line(&network::status(socket)?)))
        }
        Command::SetAirportNetwork { port: key, network, password } => {
            wifi(key)?;
            network::send(socket, &Cmd::Join { ssid: network.clone(), psk: password.clone() })?;
            Ok(String::new())
        }
        Command::ListPreferred(key) => {
            let p = wifi(key)?;
            Ok(preferred_text(&p, &network::status(socket)?))
        }
        Command::RemovePreferred { port: key, network } => {
            wifi(key)?;
            network::send(socket, &Cmd::Forget(network.clone()))?;
            Ok(String::new())
        }
        // The resolver file is system-wide, so every service reports the same servers.
        Command::GetDnsServers(service) => {
            let p = port(service)?;
            Ok(dns_text(&p.name, &resolver.servers))
        }
        Command::GetSearchDomains(service) => {
            let p = port(service)?;
            Ok(search_text(&p.name, &resolver.search))
        }
        Command::GetProxy(name, service) => {
            port(service)?;
            let endpoint = match name {
                ProxyName::Web => proxy.http.as_ref(),
                ProxyName::SecureWeb => proxy.https.as_ref(),
                ProxyName::Socks => proxy.socks.as_ref(),
            };
            Ok(proxy_text(endpoint))
        }
        Command::GetBypassDomains(service) => {
            let p = port(service)?;
            Ok(bypass_text(&p.name, &proxy.bypass))
        }
    }
}

/// The network services, one per hardware port. A service whose link is down is marked with an asterisk.
pub fn list_services(ports: &[Port]) -> String {
    let mut out = String::from("An asterisk (*) denotes that a network service is disabled.\n");
    for p in ports {
        let mark = if p.state == "down" { "*" } else { "" };
        out.push_str(&format!("{mark}{}\n", p.name));
    }
    out
}

/// The hardware ports with their device names and addresses, one block per port.
pub fn list_hardware_ports(ports: &[Port]) -> String {
    let mut out = String::new();
    for p in ports {
        let mac = if p.mac.is_empty() { "not available" } else { p.mac.as_str() };
        out.push_str(&format!("Hardware Port: {}\nDevice: {}\nEthernet Address: {mac}\n\n", p.name, p.device));
    }
    out
}

/// Wi-Fi power: on when the radio is past the off state.
pub fn airport_power_line(port: &Port, status: &Status) -> String {
    let on = if status.state == 0 { "Off" } else { "On" };
    format!("Wi-Fi Power ({}): {on}", port.device)
}

/// The network the radio is connected to, or a line that says there is none.
pub fn airport_network_line(status: &Status) -> String {
    match status.ssid.as_deref().filter(|_| status.state == 4) {
        Some(ssid) => format!("Current Wi-Fi Network: {ssid}"),
        None => "You are not associated with a Wi-Fi network.".to_string(),
    }
}

/// The saved networks, highest priority first.
pub fn preferred_text(port: &Port, status: &Status) -> String {
    let mut out = format!("Preferred networks on {}:\n", port.device);
    for ssid in &status.saved {
        out.push_str(&format!("\t{ssid}\n"));
    }
    out
}

/// The DNS servers of a service, or a line that says there are none.
pub fn dns_text(service: &str, servers: &[String]) -> String {
    if servers.is_empty() {
        return format!("There aren't any DNS Servers set on {service}.\n");
    }
    servers.iter().map(|s| format!("{s}\n")).collect()
}

/// The search domains of a service, or a line that says there are none.
pub fn search_text(service: &str, domains: &[String]) -> String {
    if domains.is_empty() {
        return format!("There aren't any Search Domains set on {service}.\n");
    }
    domains.iter().map(|d| format!("{d}\n")).collect()
}

/// One proxy's state: enabled with its server and port, or disabled with an empty server.
pub fn proxy_text(endpoint: Option<&ProxyEndpoint>) -> String {
    match endpoint {
        Some(e) => format!("Enabled: Yes\nServer: {}\nPort: {}\nAuthenticated Proxy Enabled: 0\n", e.host(), e.port()),
        None => "Enabled: No\nServer: \nPort: 0\nAuthenticated Proxy Enabled: 0\n".to_string(),
    }
}

/// The bypass list of a service, or a line that says there is none.
pub fn bypass_text(service: &str, bypass: &[String]) -> String {
    if bypass.is_empty() {
        return format!("There aren't any bypass domains set on {service}.\n");
    }
    bypass.iter().map(|d| format!("{d}\n")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn wifi_port() -> Port {
        Port { name: "Wi-Fi".into(), device: "wlan0".into(), mac: "aa:bb".into(), wireless: true, state: "up".into() }
    }

    #[test]
    fn options_parse_into_commands() {
        assert_eq!(parse(&args(&[])), Ok(Command::Help));
        assert_eq!(parse(&args(&["-listallnetworkservices"])), Ok(Command::ListServices));
        assert_eq!(parse(&args(&["-setairportpower", "Wi-Fi", "off"])), Ok(Command::SetAirportPower("Wi-Fi".into(), false)));
        assert_eq!(
            parse(&args(&["-setairportnetwork", "wlan0", "Home", "pw"])),
            Ok(Command::SetAirportNetwork { port: "wlan0".into(), network: "Home".into(), password: Some("pw".into()) })
        );
        assert_eq!(parse(&args(&["-getsocksfirewallproxy", "Wi-Fi"])), Ok(Command::GetProxy(ProxyName::Socks, "Wi-Fi".into())));
    }

    #[test]
    fn bad_operands_and_changes_outside_wifi_are_refused() {
        assert!(parse(&args(&["-setairportpower", "Wi-Fi", "maybe"])).is_err());
        assert!(parse(&args(&["-listallhardwareports", "extra"])).is_err());
        assert!(parse(&args(&["-setdnsservers", "Wi-Fi", "1.1.1.1"])).unwrap_err().contains("privileged"));
        assert!(parse(&args(&["-setwebproxy", "Wi-Fi"])).unwrap_err().contains("not supported yet"));
        assert!(parse(&args(&["-frobnicate"])).unwrap_err().contains("unknown option"));
    }

    #[test]
    fn wifi_power_reads_the_radio_state() {
        let mut st = Status::default();
        assert_eq!(airport_power_line(&wifi_port(), &st), "Wi-Fi Power (wlan0): Off");
        st.state = 4;
        assert_eq!(airport_power_line(&wifi_port(), &st), "Wi-Fi Power (wlan0): On");
    }

    #[test]
    fn the_joined_network_is_named_only_when_connected() {
        let mut st = Status { ssid: Some("Home".into()), state: 4, ..Default::default() };
        assert_eq!(airport_network_line(&st), "Current Wi-Fi Network: Home");
        st.state = 3;
        assert_eq!(airport_network_line(&st), "You are not associated with a Wi-Fi network.");
    }

    #[test]
    fn saved_networks_are_listed_in_order() {
        let st = Status { saved: vec!["Home".into(), "Work".into()], ..Default::default() };
        assert_eq!(preferred_text(&wifi_port(), &st), "Preferred networks on wlan0:\n\tHome\n\tWork\n");
    }

    #[test]
    fn services_and_ports_list_names_and_devices() {
        let mut down = wifi_port();
        down.state = "down".into();
        assert_eq!(list_services(&[down.clone()]), "An asterisk (*) denotes that a network service is disabled.\n*Wi-Fi\n");
        assert_eq!(list_hardware_ports(&[wifi_port()]), "Hardware Port: Wi-Fi\nDevice: wlan0\nEthernet Address: aa:bb\n\n");
    }

    #[test]
    fn dns_search_and_proxy_say_when_nothing_is_set() {
        assert_eq!(dns_text("Wi-Fi", &[]), "There aren't any DNS Servers set on Wi-Fi.\n");
        assert_eq!(dns_text("Wi-Fi", &["1.1.1.1".into()]), "1.1.1.1\n");
        assert_eq!(search_text("Wi-Fi", &[]), "There aren't any Search Domains set on Wi-Fi.\n");
        assert_eq!(bypass_text("Wi-Fi", &[]), "There aren't any bypass domains set on Wi-Fi.\n");
        assert_eq!(proxy_text(None), "Enabled: No\nServer: \nPort: 0\nAuthenticated Proxy Enabled: 0\n");
        let ep = ProxyEndpoint::new("web.lan", 8080).unwrap();
        assert!(proxy_text(Some(&ep)).starts_with("Enabled: Yes\nServer: web.lan\nPort: 8080\n"));
    }

    #[test]
    fn run_reads_dns_from_the_resolver_without_the_service() {
        let links = [NetInterface { name: "wlan0".into(), wireless: true, ..Default::default() }];
        let resolver = Resolver { servers: vec!["9.9.9.9".into()], search: vec![] };
        let socket = Path::new("/nonexistent/netd.sock");
        let out = run(&Command::GetDnsServers("Wi-Fi".into()), &links, &resolver, &ProxySettings::default(), socket);
        assert_eq!(out, Ok("9.9.9.9\n".to_string()));
        let missing = run(&Command::GetDnsServers("Ethernet".into()), &links, &resolver, &ProxySettings::default(), socket);
        assert!(missing.unwrap_err().contains("not found"));
        let power = run(&Command::GetAirportPower("Wi-Fi".into()), &links, &resolver, &ProxySettings::default(), socket);
        assert!(power.unwrap_err().contains("not reachable"), "the status read needs the service");
    }
}
