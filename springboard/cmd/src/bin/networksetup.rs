//! `networksetup`: lists network services and hardware ports, and reads and drives Wi-Fi, DNS and proxies. See
//! `bifrost_cmd::networksetup` and `bifrost_cmd::network` for what each option reads and what it changes.

use bifrost_cmd::die;
use bifrost_cmd::network;
use bifrost_cmd::networksetup;
use bifrost_core::proxy::ProxySettings;
use bifrost_core::sysinfo::{Report, Roots};
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = match networksetup::parse(&args) {
        Ok(cmd) => cmd,
        Err(e) => die("networksetup", &e, 1),
    };
    let links = Report::collect(&Roots::system()).net;
    let resolver = network::read_resolver(Path::new(network::RESOLV_CONF));
    let proxy: ProxySettings = network::proxy_from_env(|k| std::env::var(k).ok());
    match networksetup::run(&cmd, &links, &resolver, &proxy, &network::service_socket()) {
        Ok(text) => print!("{text}"),
        Err(e) => die("networksetup", &e, 1),
    }
}
