//! `pbpaste`: writes the general clipboard to standard output. See `bifrost_cmd::pasteboard`.

use bifrost_cmd::die;
use bifrost_cmd::pasteboard::{self, PASTE_USAGE};
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let parsed = match pasteboard::parse(&args, true) {
        Ok(parsed) => parsed,
        Err(e) => die("pbpaste", &e, 1),
    };
    if parsed.help {
        println!("{PASTE_USAGE}");
        return;
    }
    let data = match pasteboard::paste(parsed.prefer) {
        Ok(data) => data,
        Err(e) => die("pbpaste", &e, 1),
    };
    let mut out = std::io::stdout().lock();
    if let Err(e) = out.write_all(&data).and_then(|()| out.flush()) {
        die("pbpaste", &e.to_string(), 1);
    }
}
