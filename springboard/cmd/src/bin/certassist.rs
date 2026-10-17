//! `certassist`: makes certificates and signing requests, shows a certificate and judges one against trusted anchors.
//! See `bifrost_cmd::certassist`.

use bifrost_cmd::certassist::{self, Action};
use bifrost_cmd::die;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let action = match certassist::parse(&args) {
        Ok(action) => action,
        Err(e) => die("certassist", &e, 2),
    };
    match certassist::run(&action) {
        Ok((text, code)) => {
            print!("{text}");
            if code != 0 || matches!(action, Action::Help) {
                std::process::exit(code);
            }
        }
        Err(e) => die("certassist", &e, 1),
    }
}
