//! `b64encode`: base64-encodes a file or standard input. See `bifrost_cmd::b64`.

use bifrost_cmd::b64::{self, ENCODE_USAGE};
use bifrost_cmd::die;
use std::io::{Read, Write};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut raw = false;
    let mut paths: Vec<&str> = Vec::new();
    for arg in &args {
        match arg.as_str() {
            "-r" => raw = true,
            "-h" | "--help" => {
                println!("{ENCODE_USAGE}");
                return;
            }
            other if other.starts_with('-') && other.len() > 1 => die("b64encode", &format!("unknown option {other}\n{ENCODE_USAGE}"), 1),
            other => paths.push(other),
        }
    }
    if paths.len() > 2 {
        die("b64encode", ENCODE_USAGE, 1);
    }
    let mut data = Vec::new();
    match paths.first() {
        Some(p) if *p != "-" => data = std::fs::read(p).unwrap_or_else(|e| die("b64encode", &format!("{p}: {e}"), 1)),
        _ => {
            std::io::stdin().read_to_end(&mut data).unwrap_or_else(|e| die("b64encode", &e.to_string(), 1));
        }
    }
    let text = b64::encode(&data, !raw);
    match paths.get(1) {
        Some(out) => std::fs::write(out, text).unwrap_or_else(|e| die("b64encode", &format!("{out}: {e}"), 1)),
        None => {
            let mut stdout = std::io::stdout();
            let _ = stdout.write_all(text.as_bytes());
        }
    }
}
