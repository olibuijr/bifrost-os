//! `chflags`: sets or clears file flags through `chattr`. See `bifrost_cmd::chflags`.

use bifrost_cmd::chflags::{self, Parsed};
use bifrost_cmd::die;
use std::process::Command;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let request = match chflags::parse(&args) {
        Ok(Parsed::Help) => {
            println!("{}", chflags::USAGE);
            return;
        }
        Ok(Parsed::Run(request)) => request,
        Err(e) => die("chflags", &e, 1),
    };
    for mode in &request.modes {
        let status = Command::new("chattr")
            .args(chflags::chattr_args(mode, request.recursive, &request.files))
            .status()
            .unwrap_or_else(|e| die("chflags", &format!("chattr could not run: {e}"), 1));
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }
    }
}
