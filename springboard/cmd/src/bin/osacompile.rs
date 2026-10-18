//! `osacompile`: checks a script and writes its compiled form. See `bifrost_cmd::osacompile`.

use bifrost_cmd::die;
use bifrost_cmd::osacompile::{self, Parsed};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match osacompile::parse(&args) {
        Ok(Parsed::Help) => {
            println!("{}", osacompile::USAGE);
            return;
        }
        Ok(Parsed::Run(options)) => options,
        Err(e) => die("osacompile", &e, 1),
    };
    if let Err(e) = osacompile::compile_to_file(&options) {
        die("osacompile", &e, 1);
    }
}
