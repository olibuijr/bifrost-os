//! `pbcopy`: puts standard input on the general clipboard or a named board. See `bifrost_cmd::pasteboard`.

use bifrost_cmd::die;
use bifrost_cmd::pasteboard::{self, COPY_USAGE};
use std::io::Read;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let parsed = match pasteboard::parse(&args, false) {
        Ok(parsed) => parsed,
        Err(e) => die("pbcopy", &e, 1),
    };
    if parsed.help {
        println!("{COPY_USAGE}");
        return;
    }
    let mut input = Vec::new();
    if let Err(e) = std::io::stdin().read_to_end(&mut input) {
        die("pbcopy", &e.to_string(), 1);
    }
    if let Err(e) = pasteboard::copy_for(parsed.board.as_deref(), &input) {
        die("pbcopy", &e, 1);
    }
}
