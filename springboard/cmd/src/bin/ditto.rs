//! `ditto`: copies a file or folder tree, or writes and extracts zip archives. See `bifrost_cmd::ditto`.

use bifrost_cmd::die;
use bifrost_cmd::ditto::{self, Cmd};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = match ditto::parse(&args) {
        Ok(cmd) => cmd,
        Err(e) => die("ditto", &e, 1),
    };
    if cmd == Cmd::Help {
        println!("{}", ditto::USAGE);
        return;
    }
    if let Err(e) = ditto::run(&cmd) {
        die("ditto", &e, 1);
    }
}
