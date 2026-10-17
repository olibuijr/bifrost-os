//! `sips`: reads an image's properties, resamples pictures and converts them to PNG or JPEG. See `bifrost_cmd::sips`.

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
        Action::Convert { to, out, file } => {
            let data = std::fs::read(&file).unwrap_or_else(|e| die("sips", &format!("{file}: {e}"), 1));
            let bytes = sips::convert(&data, to).unwrap_or_else(|e| die("sips", &format!("{file}: {e}"), 1));
            std::fs::write(&out, bytes).unwrap_or_else(|e| die("sips", &format!("{out}: {e}"), 1));
        }
        Action::Resample { target, out, files } => {
            for file in &files {
                let data = std::fs::read(file).unwrap_or_else(|e| die("sips", &format!("{file}: {e}"), 1));
                let dest = out.clone().unwrap_or_else(|| file.clone());
                let out_is_png = dest.to_ascii_lowercase().ends_with(".png");
                let png = sips::resample(&data, target, out_is_png).unwrap_or_else(|e| die("sips", &format!("{file}: {e}"), 1));
                std::fs::write(&dest, png).unwrap_or_else(|e| die("sips", &format!("{dest}: {e}"), 1));
            }
        }
    }
}
