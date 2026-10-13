//! `hdiutil`: create, attach, detach, inspect, verify and convert disk images. See `bifrost_cmd::hdiutil`.

use bifrost_cmd::die;
use bifrost_cmd::hdiutil::{self, Command};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match hdiutil::parse(&args) {
        Ok(Command::Help) => {
            println!("{}", hdiutil::USAGE);
            return;
        }
        Ok(command) => command,
        Err(e) => die("hdiutil", &format!("{e}\n{}", hdiutil::USAGE), 2),
    };
    match hdiutil::run(&command) {
        Ok(text) => println!("{text}"),
        Err(e) => die("hdiutil", &e, 1),
    }
}
