//! `cu`: opens a serial line as a terminal. See `bifrost_cmd::cu`.

use bifrost_cmd::cu::{self, Parsed};
use bifrost_cmd::die;
use std::process::Command;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cu::parse(&args) {
        Ok(Parsed::Help) => println!("{}", cu::USAGE),
        Ok(Parsed::Open { line, speed }) => {
            let status = Command::new(cu::PROGRAM)
                .args(cu::picocom_args(&line, &speed))
                .status()
                .unwrap_or_else(|e| die("cu", &format!("{} could not run: {e}", cu::PROGRAM), 1));
            std::process::exit(status.code().unwrap_or(1));
        }
        Err(e) => die("cu", &e, 1),
    }
}
