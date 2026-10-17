//! `usdextract`: unpacks a USDZ package into a folder. See `bifrost_cmd::usdextract`.

use bifrost_cmd::die;
use bifrost_cmd::usdextract::{self, Cmd};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = match usdextract::parse(&args) {
        Ok(cmd) => cmd,
        Err(e) => die("usdextract", &e, 1),
    };
    if cmd == Cmd::Help {
        println!("{}", usdextract::USAGE);
        return;
    }
    if let Err(e) = usdextract::run(&cmd) {
        die("usdextract", &e, 1);
    }
}
