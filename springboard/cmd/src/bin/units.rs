//! `units`: converts between units of measure, or runs the interactive two-prompt mode. See `bifrost_cmd::units`.

use bifrost_cmd::die;
use bifrost_cmd::units::{self, Request};
use std::fs;
use std::io;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (opts, operands) = match units::parse(&args) {
        Ok(Request::Help) => {
            println!("{}", units::USAGE);
            return;
        }
        Ok(Request::Version) => {
            println!("{}", units::VERSION);
            return;
        }
        Ok(Request::Run(opts, operands)) => (opts, operands),
        Err(e) => die("units", &e, 1),
    };
    if opts.print_path {
        println!("{}", opts.file.as_deref().unwrap_or(units::BUILTIN_PATH));
        return;
    }
    let mut db = units::Database::builtin();
    if let Some(path) = &opts.file {
        let text = fs::read_to_string(path).unwrap_or_else(|e| die("units", &format!("{path}: {e}"), 1));
        if let Err(e) = db.add_definitions(&text) {
            die("units", &format!("{path}: {e}"), 1);
        }
    }
    match operands.as_slice() {
        [] => {
            let stdin = io::stdin();
            let mut input = stdin.lock();
            let mut out = io::stdout().lock();
            if let Err(e) = units::interactive(&db, &mut input, &mut out, opts.quiet) {
                die("units", &e.to_string(), 1);
            }
        }
        [from, to] => match units::one_shot(&db, from, to, &opts) {
            Ok(line) => println!("{line}"),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        },
        _ => die("units", units::USAGE, 1),
    }
}
