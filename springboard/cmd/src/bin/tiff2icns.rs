//! `tiff2icns`: converts an image into an icon file. See `bifrost_cmd::tiff2icns`.

use bifrost_cmd::die;
use bifrost_cmd::tiff2icns::{self, Parsed, USAGE};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match tiff2icns::parse(&args) {
        Ok(Parsed::Help) => println!("{USAGE}"),
        Ok(Parsed::Run { input, output }) => {
            if let Err(e) = tiff2icns::run(&input, &output) {
                die("tiff2icns", &e, 1);
            }
        }
        Err(e) => die("tiff2icns", &e, 1),
    }
}
