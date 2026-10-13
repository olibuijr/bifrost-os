//! `plutil`: checks, prints, converts and edits property lists. See `bifrost_cmd::plutil` for what is supported.

use bifrost_cmd::plutil::{self, Command, Produced};
use std::io::{Read, Write};
use std::path::Path;

fn read_input(file: &str) -> Result<Vec<u8>, String> {
    if file == "-" {
        let mut buf = Vec::new();
        std::io::stdin().read_to_end(&mut buf).map_err(|e| format!("-: {e}"))?;
        Ok(buf)
    } else {
        std::fs::read(file).map_err(|e| format!("{file}: {e}"))
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = match plutil::parse(&args) {
        Ok(opts) => opts,
        Err(e) => bifrost_cmd::die("plutil", &e, 1),
    };
    if opts.command == Command::Help {
        println!("{}", plutil::USAGE);
        return;
    }
    // `-create` names the file it writes; it has no input to read.
    let creating = matches!(opts.command, Command::Create(_));
    let mut failed = false;
    for file in &opts.files {
        let input = if creating {
            Vec::new()
        } else {
            match read_input(file) {
                Ok(bytes) => bytes,
                Err(e) => {
                    eprintln!("plutil: {e}");
                    failed = true;
                    continue;
                }
            }
        };
        match plutil::process(&opts, &input) {
            Ok(Produced::Lint) => {
                if !opts.silent {
                    println!("{file}: OK");
                }
            }
            Ok(Produced::Text(text)) => print!("{text}"),
            Ok(Produced::Bytes(bytes)) => {
                let target = plutil::target_for(&opts, file);
                let result = if target == "-" {
                    std::io::stdout().write_all(&bytes).map_err(|e| e.to_string())
                } else {
                    bifrost_core::fsutil::write_atomic(Path::new(&target), &bytes).map_err(|e| format!("{target}: {e}"))
                };
                if let Err(e) = result {
                    eprintln!("plutil: {e}");
                    failed = true;
                }
            }
            Err(e) => {
                eprintln!("plutil: {file}: {e}");
                failed = true;
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
}
