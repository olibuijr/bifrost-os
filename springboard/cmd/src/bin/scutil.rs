//! `scutil`: reads and sets the computer and host names, and prints the DNS resolvers and proxy settings. See
//! `bifrost_cmd::scutil` for what is saved and what is not applied yet.

use bifrost_cmd::die;
use bifrost_cmd::network;
use bifrost_cmd::scutil;
use bifrost_core::proxy::ProxySettings;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = match scutil::parse(&args) {
        Ok(cmd) => cmd,
        Err(e) => die("scutil", &e, 1),
    };
    let resolver = network::read_resolver(Path::new(network::RESOLV_CONF));
    let proxy: ProxySettings = network::proxy_from_env(|k| std::env::var(k).ok());
    match scutil::run(&cmd, &scutil::state_dir(), &resolver, &proxy) {
        Ok(text) => print!("{text}"),
        Err(e) => die("scutil", &e, 1),
    }
}
