//! `sw_vers`: prints the operating system's name, version and build. See `bifrost_cmd::sw_vers`.

use bifrost_cmd::die;
use bifrost_cmd::sw_vers::{self, Request};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let field = match sw_vers::parse(&args) {
        Ok(Request::Help) => {
            println!("{}", sw_vers::USAGE);
            return;
        }
        Ok(Request::Show(field)) => field,
        Err(e) => die("sw_vers", &e, 1),
    };
    print!("{}", sw_vers::render(&sw_vers::current(), field));
}
