//! `mtree`: writes and checks file hierarchy specifications. See `bifrost_cmd::mtree` for the spec format and keywords.

use std::io::{Read, Write};

use bifrost_cmd::die;
use bifrost_cmd::mtree::{self, Parsed};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match mtree::parse_args(&args) {
        Ok(Parsed::Help) => {
            println!("{}", mtree::USAGE);
            return;
        }
        Ok(Parsed::Run(options)) => options,
        Err(e) => die("mtree", &e, 1),
    };
    if options.create {
        let bytes = mtree::create(&options).unwrap_or_else(|e| die("mtree", &e, 1));
        if let Err(e) = std::io::stdout().write_all(&bytes) {
            die("mtree", &e.to_string(), 1);
        }
        return;
    }
    let text = read_spec(&options).unwrap_or_else(|e| die("mtree", &e, 1));
    let spec = mtree::parse_spec(&text).unwrap_or_else(|e| die("mtree", &e, 1));
    for keyword in &spec.unsupported {
        eprintln!("mtree: keyword {keyword} is not supported and was not checked");
    }
    let diffs = mtree::check(&options, &spec).unwrap_or_else(|e| die("mtree", &e, 1));
    if diffs.is_empty() {
        return;
    }
    for diff in &diffs {
        println!("{}", mtree::format_difference(diff));
    }
    std::process::exit(2);
}

fn read_spec(options: &mtree::Options) -> Result<Vec<u8>, String> {
    match &options.spec_file {
        Some(path) => std::fs::read(path).map_err(|e| format!("{}: {e}", path.display())),
        None => {
            let mut buf = Vec::new();
            std::io::stdin().read_to_end(&mut buf).map_err(|e| format!("stdin: {e}"))?;
            Ok(buf)
        }
    }
}
