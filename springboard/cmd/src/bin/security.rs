//! `security`: reads and writes generic passwords in the BifrOSt keychain. See `bifrost_cmd::security`.

use bifrost_cmd::die;
use bifrost_cmd::pmset::is_root;
use bifrost_cmd::security::{self, Action};
use springboard::keychain::Store;
use std::io::{BufRead, Write};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let action = match security::parse(&args) {
        Ok(a) => a,
        Err(e) => die("security", &e, 1),
    };
    if action == Action::Help {
        println!("{}", security::USAGE);
        return;
    }
    let home = std::env::var("HOME").ok();
    let xdg = std::env::var("XDG_DATA_HOME").ok();
    let root = security::data_root(is_root(), xdg.as_deref(), home.as_deref());
    let store = match Store::open_group(&root, security::GROUP) {
        Ok(s) => s,
        Err(e) => die("security", &format!("{}: {e}", root.display()), 1),
    };
    let stdin_password = match &action {
        Action::Add { password: None, .. } => {
            let mut line = String::new();
            let _ = std::io::stdin().lock().read_line(&mut line);
            let line = line.trim_end_matches(['\r', '\n']).to_string();
            (!line.is_empty()).then_some(line)
        }
        _ => None,
    };
    match security::run(&store, &action, stdin_password.as_deref()) {
        Ok(out) => {
            let _ = std::io::stdout().write_all(out.as_bytes());
        }
        Err(e) => die("security", &e, 44),
    }
}
