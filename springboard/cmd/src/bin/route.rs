//! `route`: prints the IPv4 routing table (`route -n`). See `bifrost_cmd::route`.

use bifrost_cmd::die;
use bifrost_cmd::route::{self, Action};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match route::parse(&args) {
        Ok(Action::Help) => println!("{}", route::USAGE),
        Ok(Action::Show) => {
            let text = std::fs::read_to_string(route::PROC_PATH)
                .unwrap_or_else(|e| die("route", &format!("{}: {e}", route::PROC_PATH), 1));
            print!("{}", route::table_text(&route::parse_table(&text)));
        }
        Err(e) => die("route", &e, 1),
    }
}
