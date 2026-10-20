//! `pbcopy`: puts standard input on the general clipboard or a named board. See `bifrost_cmd::pasteboard`.

use bifrost_cmd::die;
use bifrost_cmd::pasteboard::{self, COPY_USAGE};

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
    let input = match pasteboard::read_bounded(&mut std::io::stdin()) {
        Ok(input) => input,
        Err(e) => die("pbcopy", &e, 1),
    };
    if let Err(e) = pasteboard::copy_for(parsed.board.as_deref(), &input) {
        die("pbcopy", &e, 1);
    }
}
