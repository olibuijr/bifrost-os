//! `bifrost-firewall`: sets per-app inbound policies and loads them as nftables rules. See `bifrost_cmd::firewall`.

use bifrost_cmd::die;
use bifrost_cmd::firewall::{self, Parsed, USAGE};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match firewall::parse(&args) {
        Ok(Parsed::Help) => {
            println!("{USAGE}");
            return;
        }
        Ok(Parsed::Run(options)) => options,
        Err(e) => die("bifrost-firewall", &format!("{e}\n{USAGE}"), 2),
    };
    match firewall::run(&options, &firewall::nft_load) {
        Ok(text) => print!("{text}"),
        Err(e) => die("bifrost-firewall", &e, 1),
    }
}
