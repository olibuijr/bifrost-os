//! `caffeinate`: keeps the machine awake while a command runs, for a time, or until a process exits. See
//! `bifrost_cmd::caffeinate`.

use bifrost_cmd::caffeinate::{self, Parsed, USAGE};
use bifrost_cmd::die;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match caffeinate::parse(&args) {
        Ok(Parsed::Help) => {
            println!("{USAGE}");
            return;
        }
        Ok(Parsed::Run(options)) => options,
        Err(e) => die("caffeinate", &e, 1),
    };
    match caffeinate::run(&options) {
        Ok(code) => std::process::exit(code),
        Err(e) => die("caffeinate", &e, 1),
    }
}
