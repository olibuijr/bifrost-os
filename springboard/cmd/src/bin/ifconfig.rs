//! `ifconfig`: lists the network interfaces (read-only). See `bifrost_cmd::ifconfig`.

use bifrost_cmd::die;
use bifrost_cmd::ifconfig::{self, Action};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let action = match ifconfig::parse(&args) {
        Ok(action) => action,
        Err(e) => die("ifconfig", &e, 1),
    };
    let ifaces = ifconfig::read_interfaces(std::path::Path::new(ifconfig::SYS_NET));
    let inet6 = std::fs::read_to_string(ifconfig::INET6_PATH).map(|t| ifconfig::parse_inet6(&t)).unwrap_or_default();
    match action {
        Action::Help => println!("{}", ifconfig::USAGE),
        Action::List => {
            for iface in &ifaces {
                print!("{}", ifconfig::render(iface, &inet6));
            }
        }
        Action::Show(name) => match ifaces.iter().find(|i| i.name == name) {
            Some(iface) => print!("{}", ifconfig::render(iface, &inet6)),
            None => die("ifconfig", &format!("{name}: interface does not exist"), 1),
        },
    }
}
