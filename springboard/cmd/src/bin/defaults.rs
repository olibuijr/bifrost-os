//! `defaults`: reads and writes the preference store apps and settings use. See `bifrost_cmd::defaults` for what is supported.

use bifrost_cmd::defaults::{self, Store};
use bifrost_cmd::die;
use std::io::Read;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = match defaults::parse(&args) {
        Ok(cmd) => cmd,
        Err(e) => die("defaults", &e, 1),
    };
    let store = Store::new(defaults::root());
    let stdin = || {
        let mut buf = Vec::new();
        std::io::stdin().read_to_end(&mut buf).map(|_| buf).map_err(|e| e.to_string())
    };
    match defaults::run(&cmd, &store, stdin) {
        Ok(text) => print!("{text}"),
        Err(e) => die("defaults", &e, 1),
    }
}
