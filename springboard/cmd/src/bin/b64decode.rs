//! `b64decode`: decodes base64 from a file or standard input. See `bifrost_cmd::b64`.

use bifrost_cmd::b64::{self, DECODE_USAGE};
use bifrost_cmd::die;
use std::io::{Read, Write};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut paths: Vec<&str> = Vec::new();
    for arg in &args {
        match arg.as_str() {
            // Accepted for symmetry with b64encode; decoding reads the same text either way.
            "-r" => {}
            "-h" | "--help" => {
                println!("{DECODE_USAGE}");
                return;
            }
            other if other.starts_with('-') && other.len() > 1 => die("b64decode", &format!("unknown option {other}\n{DECODE_USAGE}"), 1),
            other => paths.push(other),
        }
    }
    if paths.len() > 2 {
        die("b64decode", DECODE_USAGE, 1);
    }
    let mut text = String::new();
    match paths.first() {
        Some(p) if *p != "-" => text = std::fs::read_to_string(p).unwrap_or_else(|e| die("b64decode", &format!("{p}: {e}"), 1)),
        _ => {
            std::io::stdin().read_to_string(&mut text).unwrap_or_else(|e| die("b64decode", &e.to_string(), 1));
        }
    }
    let data = b64::decode(&text).unwrap_or_else(|e| die("b64decode", &e, 1));
    match paths.get(1) {
        Some(out) => std::fs::write(out, data).unwrap_or_else(|e| die("b64decode", &format!("{out}: {e}"), 1)),
        None => {
            let mut stdout = std::io::stdout();
            let _ = stdout.write_all(&data);
        }
    }
}
