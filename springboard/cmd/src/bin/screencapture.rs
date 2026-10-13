//! `screencapture`: saves the screen as a PNG or puts it on the clipboard. See `bifrost_cmd::screencapture`.

use bifrost_cmd::control;
use bifrost_cmd::die;
use bifrost_cmd::screencapture::{self, Parsed, USAGE};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match screencapture::parse(&args) {
        Ok(Parsed::Help) => {
            println!("{USAGE}");
            return;
        }
        Ok(Parsed::Run(options)) => options,
        Err(e) => die("screencapture", &e, 1),
    };
    if let Err(e) = screencapture::run(&options, control::port()) {
        die("screencapture", &e, 1);
    }
}
