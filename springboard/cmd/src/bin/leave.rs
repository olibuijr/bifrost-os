//! `leave`: waits until a time, then rings the bell and prints a reminder. See `bifrost_cmd::leave`.

use bifrost_cmd::die;
use bifrost_cmd::leave::{self, REMINDER, USAGE};
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = match args.as_slice() {
        [flag] if flag == "-h" || flag == "-help" || flag == "--help" => {
            println!("{USAGE}");
            return;
        }
        [arg] => arg,
        _ => die("leave", USAGE, 1),
    };
    let spec = match leave::parse(arg) {
        Ok(spec) => spec,
        Err(e) => die("leave", &e, 1),
    };
    let wait = match leave::delay(spec, leave::now_of_day) {
        Ok(wait) => wait,
        Err(e) => die("leave", &e, 1),
    };
    std::thread::sleep(wait);
    println!("{REMINDER}");
    let _ = std::io::stdout().flush();
}
