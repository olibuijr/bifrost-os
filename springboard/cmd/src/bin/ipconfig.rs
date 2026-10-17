//! `ipconfig`: asks for the IP configuration of an interface. See `bifrost_cmd::ipconfig`.

use bifrost_cmd::die;
use bifrost_cmd::ifconfig;
use bifrost_cmd::ipconfig::{self, Action, USAGE};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match ipconfig::parse(&args) {
        Err(e) => die("ipconfig", &e, 1),
        Ok(Action::Help) => println!("{USAGE}"),
        Ok(Action::IfCount) => {
            println!("{}", ifconfig::read_interfaces(std::path::Path::new(ifconfig::SYS_NET)).len());
        }
        Ok(Action::GetIfAddr(iface)) => match ifconfig::read_inet4(&iface) {
            Some(v4) => println!("{}", v4.addr),
            None => std::process::exit(1),
        },
        Ok(Action::GetV6Address(iface)) => {
            let inet6 = std::fs::read_to_string(ifconfig::INET6_PATH).map(|t| ifconfig::parse_inet6(&t)).unwrap_or_default();
            match ipconfig::global_v6(&iface, &inet6) {
                Some(a) => println!("{a}"),
                None => std::process::exit(1),
            }
        }
        Ok(Action::GetOption(iface, name)) => {
            let route = std::fs::read_to_string(ipconfig::ROUTE_PATH).unwrap_or_default();
            let resolv = std::fs::read_to_string(ipconfig::RESOLV_PATH).unwrap_or_default();
            match ipconfig::option_value(&name, ifconfig::read_inet4(&iface).as_ref(), &route, &iface, &resolv) {
                Err(e) => die("ipconfig", &e, 1),
                Ok(Some(v)) => println!("{v}"),
                Ok(None) => std::process::exit(1),
            }
        }
    }
}
