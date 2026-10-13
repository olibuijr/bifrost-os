//! `tmutil`: status, snapshot listing, restore of a file version and backup exclusions for the snapshot backup.
//! See `bifrost_cmd::tmutil` for the commands and the store layout.

use bifrost_cmd::die;
use bifrost_cmd::tmutil::{self, Command};
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = match tmutil::parse(&args) {
        Ok(cmd) => cmd,
        Err(e) => die("tmutil", &e, 1),
    };
    if cmd == Command::Help {
        println!("{}", tmutil::USAGE);
        return;
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    match tmutil::run(&cmd, &tmutil::store_dir(), now) {
        Ok(text) => print!("{text}"),
        Err(e) => die("tmutil", &e, 1),
    }
}
