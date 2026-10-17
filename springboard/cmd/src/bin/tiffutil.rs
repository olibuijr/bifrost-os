//! `tiffutil`: looks at and rearranges TIFF files through the libtiff tools. See `bifrost_cmd::tiffutil`.

use bifrost_cmd::die;
use bifrost_cmd::tiffutil::{self, Plan};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match tiffutil::parse(&args) {
        Err(e) => die("tiffutil", &e, 1),
        Ok(Plan::Help) => println!("{}", tiffutil::USAGE),
        Ok(Plan::Run { program, args }) => bifrost_cmd::forward("tiffutil", program, &args),
    }
}
