//! `sips`: reads an image's properties. See `bifrost_cmd::sips`.

use bifrost_cmd::die;
use bifrost_cmd::sips::{self, Action};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let action = match sips::parse(&args) {
        Ok(action) => action,
        Err(e) => die("sips", &e, 1),
    };
    match action {
        Action::Help => println!("{}", sips::USAGE),
        Action::Query { keys, files } => {
            for file in &files {
                let data = std::fs::read(file).unwrap_or_else(|e| die("sips", &format!("{file}: {e}"), 1));
                let info = sips::info(&data).unwrap_or_else(|e| die("sips", &format!("{file}: {e}"), 1));
                print!("{}", sips::render(file, &info, &keys));
            }
        }
    }
}
