//! `what`: prints the `@(#)` version markers found in each file. See `bifrost_cmd::what`.

use bifrost_cmd::die;
use bifrost_cmd::what::{self, Parsed};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match what::parse(&args) {
        Ok(Parsed::Help) => {
            println!("{}", what::USAGE);
            return;
        }
        Ok(Parsed::Run(options)) => options,
        Err(e) => die("what", &e, 1),
    };
    let mut status = 0;
    for file in &options.files {
        match std::fs::read(file) {
            Ok(data) => print!("{}", what::render(file, &what::markers(&data, options.first_only))),
            Err(e) => {
                eprintln!("what: {file}: {e}");
                status = 1;
            }
        }
    }
    std::process::exit(status);
}
