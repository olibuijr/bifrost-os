//! `iconutil`: converts an iconset folder to an ICNS file, and an ICNS file back to an iconset. See `bifrost_cmd::iconutil`.

use bifrost_cmd::die;
use bifrost_cmd::iconutil::{self, Command};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match iconutil::parse(&args) {
        Ok(Command::Help) => println!("{}", iconutil::USAGE),
        Ok(command) => {
            if let Err(e) = iconutil::run(&command) {
                die("iconutil", &e, 1);
            }
        }
        Err(e) => die("iconutil", &format!("{e}\n{}", iconutil::USAGE), 2),
    }
}
