//! `pmset`: reads and sets the desktop's power settings. See `bifrost_cmd::pmset`.

use bifrost_cmd::die;
use bifrost_cmd::pmset::{self, Action, USAGE};
use springboard::shell::prefs::Prefs;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let action = match pmset::parse(&args) {
        Ok(action) => action,
        Err(e) => die("pmset", &e, 1),
    };
    let home = std::env::var("HOME").ok();
    let xdg = std::env::var("XDG_DATA_HOME").ok();
    let path = pmset::settings_path(pmset::is_root(), xdg.as_deref(), home.as_deref());
    match action {
        Action::Help => println!("{USAGE}"),
        Action::Show => print!("{}", pmset::show_text(&Prefs::load(&path))),
        Action::ShowBatt => print!("{}", pmset::batt_text(&bifrost_core::power_supply::sources_in(std::path::Path::new(pmset::SUPPLY_CLASS)))),
        Action::SetLowPower(on) => {
            let mut prefs = Prefs::load(&path);
            if let Err(e) = pmset::set_low_power(&mut prefs, on) {
                die("pmset", &format!("{}: {e}", path.display()), 1);
            }
        }
        Action::SetSleep(minutes) => {
            let mut prefs = Prefs::load(&path);
            if let Err(e) = pmset::set_sleep(&mut prefs, minutes) {
                die("pmset", &format!("{}: {e}", path.display()), 1);
            }
        }
    }
}
