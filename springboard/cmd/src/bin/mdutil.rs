//! `mdutil`: shows or changes a volume's search indexing. See `bifrost_cmd::mdutil`.

use bifrost_cmd::die;
use bifrost_cmd::mdutil::{self, Action};
use springboard::shell::prefs::Prefs;
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let action = match mdutil::parse(&args) {
        Ok(action) => action,
        Err(e) => die("mdutil", &e, 1),
    };
    let root = mdutil::state_root(std::env::var("XDG_STATE_HOME").ok().as_deref(), std::env::var("HOME").ok().as_deref());
    let home = std::env::var("HOME").ok();
    let xdg = std::env::var("XDG_DATA_HOME").ok();
    let settings = bifrost_cmd::pmset::settings_path(bifrost_cmd::pmset::is_root(), xdg.as_deref(), home.as_deref());
    let result = match action {
        Action::Help => {
            println!("{}", mdutil::USAGE);
            return;
        }
        Action::Status(volumes) => {
            let mut out = std::io::stdout().lock();
            let prefs = Prefs::load(&settings);
            volumes.iter().try_for_each(|v| write!(out, "{}", mdutil::status_text_in(&root, v, mdutil::search_excludes(&prefs, v))))
        }
        Action::Set { on, volumes } => {
            let mut prefs = Prefs::load(&settings);
            volumes.iter().try_for_each(|v| {
                mdutil::set_enabled(&root, v, on)?;
                mdutil::apply_to_search(&mut prefs, v, on)
            })
        }
        Action::Erase(volumes) => volumes.iter().try_for_each(|v| mdutil::erase(&root, v)),
    };
    if let Err(e) = result {
        die("mdutil", &e.to_string(), 1);
    }
}
