//! `notifyutil`: posts named notifications, watches them, and reads or sets their state values. See `bifrost_cmd::notifyutil`.

use bifrost_cmd::control;
use bifrost_cmd::die;
use bifrost_cmd::notifyutil::{self, Control};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = match notifyutil::parse(&args) {
        Ok(opts) => opts,
        Err(e) => die("notifyutil", &format!("{e}\n{}", notifyutil::USAGE), 1),
    };
    let mut bus = Control { socket: control::socket() };
    let mut out = std::io::stdout();
    if let Err(e) = notifyutil::run(&mut bus, &opts, &mut out) {
        die("notifyutil", &e, 1);
    }
}
