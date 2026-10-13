//! `say`: speaks text through the system speech engine, or writes it to a file. See `bifrost_cmd::say`.

use bifrost_cmd::die;
use bifrost_cmd::say::{self, Parsed, USAGE};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match say::parse(&args) {
        Ok(Parsed::Help) => {
            println!("{USAGE}");
            return;
        }
        Ok(Parsed::Run(options)) => options,
        Err(e) => die("say", &e, 1),
    };
    let text = match say::text_of(&options) {
        Ok(text) => text,
        Err(e) => die("say", &e, 1),
    };
    if let Err(e) = say::speak(&say::engine_args(&options), text.as_deref()) {
        die("say", &e, 1);
    }
}
