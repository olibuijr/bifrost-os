//! Lists commands recorded by Linux process accounting. The account file exists only when the system has enabled
//! accounting; BifrOSt does not turn it on automatically.

use std::fs;

use bifrost_cmd::die;
use bifrost_cmd::lastcomm::{self, Parsed};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let names = match lastcomm::parse(&args) {
        Ok(Parsed::Help) => {
            println!("{}", lastcomm::USAGE);
            return;
        }
        Ok(Parsed::Run(names)) => names,
        Err(error) => die("lastcomm", &error, 1),
    };
    let bytes = fs::read(lastcomm::ACCT_FILE)
        .unwrap_or_else(|error| die("lastcomm", &format!("{}: {error}", lastcomm::ACCT_FILE), 1));
    match lastcomm::report(&bytes, &names) {
        Ok(report) => print!("{report}"),
        Err(error) => die("lastcomm", &error, 1),
    }
}
