//! `textutil`: converts a text document between plain text, HTML and RTF. See `bifrost_cmd::textutil`.

use bifrost_cmd::die;
use bifrost_cmd::textutil::{self, Parsed, USAGE};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match textutil::parse(&args) {
        Ok(Parsed::Help) => {
            println!("{USAGE}");
            return;
        }
        Ok(Parsed::Run(options)) => options,
        Err(e) => die("textutil", &e, 1),
    };
    if let Err(e) = textutil::run(&options) {
        die("textutil", &e, 1);
    }
}
