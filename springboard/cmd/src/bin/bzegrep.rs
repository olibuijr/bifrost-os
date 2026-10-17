//! `bzegrep`: searches bzip2-compressed files with extended regular expressions. See `bifrost_cmd::bzegrep`.

use bifrost_cmd::bzegrep::{self, Parsed};
use bifrost_cmd::die;
use std::process::Command;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match bzegrep::parse(&args) {
        Parsed::Help => println!("{}", bzegrep::USAGE),
        Parsed::Run(forwarded) => {
            let status = Command::new(bzegrep::PROGRAM)
                .args(&forwarded)
                .status()
                .unwrap_or_else(|e| die("bzegrep", &format!("{} could not run: {e}", bzegrep::PROGRAM), 2));
            std::process::exit(status.code().unwrap_or(2));
        }
    }
}
