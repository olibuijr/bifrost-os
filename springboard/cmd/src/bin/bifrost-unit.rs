//! `bifrost-unit`: inspects and renders audio units. See `bifrost_cmd::unitctl`.

use bifrost_cmd::die;
use bifrost_cmd::unitctl::{self, Plan};

const TOOL: &str = "bifrost-unit";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match unitctl::parse(&args) {
        Err(e) => die(TOOL, &e, 1),
        Ok(Plan::Help) => println!("{}", unitctl::USAGE),
        Ok(Plan::Scan { dirs }) => {
            let (text, loaded) = unitctl::scan(&dirs);
            print!("{text}");
            if loaded == 0 {
                std::process::exit(1);
            }
        }
        Ok(Plan::Render { unit, seconds, rate, note, sets, out }) => {
            let loaded = unitctl::load_unit(&unit).unwrap_or_else(|e| die(TOOL, &e, 1));
            let planes = unitctl::render(loaded, seconds, rate, note, &sets).unwrap_or_else(|e| die(TOOL, &e, 1));
            let peak = planes.iter().flat_map(|p| p.iter()).fold(0.0f32, |m, s| m.max(s.abs()));
            if let Err(e) = std::fs::write(&out, unitctl::wav(&planes, rate)) {
                die(TOOL, &format!("cannot write {}: {e}", out.display()), 1);
            }
            println!("{}: {} channels, {} frames at {} Hz, peak {:.3}", out.display(), planes.len(), planes.first().map_or(0, Vec::len), rate, peak);
        }
    }
}
