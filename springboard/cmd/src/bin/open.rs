//! `open`: opens folders, URLs and apps with the desktop's handlers. See `bifrost_cmd::open` for what is supported.

use bifrost_cmd::die;
use bifrost_cmd::open::{self, Parsed};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match open::parse(&args) {
        Ok(Parsed::Help) => {
            println!("{}", open::USAGE);
            return;
        }
        Ok(Parsed::Run(options)) => options,
        Err(e) => die("open", &e, 2),
    };
    let requests = match open::plan(&options) {
        Ok(requests) => requests,
        Err(e) => die("open", &e, 1),
    };
    let port = open::port();
    let mut failed = false;
    for request in &requests {
        if let Err(e) = open::send(request, port) {
            eprintln!("open: {e}");
            failed = true;
        }
    }
    if failed {
        std::process::exit(1);
    }
}
