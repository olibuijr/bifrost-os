//! `osascript`: runs a script in the BifrOSt scripting language. See `bifrost_cmd::osascript`.

use bifrost_cmd::die;
use bifrost_cmd::osascript::{self, Parsed};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match osascript::parse(&args) {
        Ok(Parsed::Help) => {
            println!("{}", osascript::USAGE);
            return;
        }
        Ok(Parsed::Run(options)) => options,
        Err(e) => die("osascript", &e, 1),
    };
    match osascript::run(&options) {
        Ok(Some(line)) => println!("{line}"),
        Ok(None) => {}
        Err(e) => die("osascript", &e, 1),
    }
}
