//! `ping6`: pings an IPv6 host through the installed `ping -6`. See `bifrost_cmd::ping6`.

use bifrost_cmd::die;
use bifrost_cmd::ping6::{self, USAGE};
use std::os::unix::process::CommandExt;
use std::process::Command;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("-h" | "-help" | "--help")) {
        println!("{USAGE}");
        return;
    }
    let ping_args = match ping6::ping_args(&args) {
        Ok(ping_args) => ping_args,
        Err(e) => die("ping6", &e, 1),
    };
    // exec replaces this process, so ping's own output, signals and exit status are the ones the caller sees
    let err = Command::new("/usr/bin/ping").args(&ping_args).exec();
    die("ping6", &format!("/usr/bin/ping: {err}"), 127);
}
