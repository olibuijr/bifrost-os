//! `mdutil`: shows or changes a volume's search indexing. See `bifrost_cmd::mdutil`.

use bifrost_cmd::die;
use bifrost_cmd::mdutil::{self, Action};
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let action = match mdutil::parse(&args) {
        Ok(action) => action,
        Err(e) => die("mdutil", &e, 1),
    };
    let root = mdutil::state_root(std::env::var("XDG_STATE_HOME").ok().as_deref(), std::env::var("HOME").ok().as_deref());
    let result = match action {
        Action::Help => {
            println!("{}", mdutil::USAGE);
            return;
        }
        Action::Status(volumes) => {
            let mut out = std::io::stdout().lock();
            volumes.iter().try_for_each(|v| write!(out, "{}", mdutil::status_text(&root, v)))
        }
        Action::Set { on, volumes } => volumes.iter().try_for_each(|v| mdutil::set_enabled(&root, v, on)),
        Action::Erase(volumes) => volumes.iter().try_for_each(|v| mdutil::erase(&root, v)),
    };
    if let Err(e) = result {
        die("mdutil", &e.to_string(), 1);
    }
}
