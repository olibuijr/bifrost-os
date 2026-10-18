//! `leaks`: reports the memory a program leaks, by running it under valgrind. See `bifrost_cmd::leaks`.

use bifrost_cmd::die;
use bifrost_cmd::leaks::{self, Plan};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match leaks::parse(&args) {
        Err(e) => die("leaks", &e, 1),
        Ok(Plan::Help) => println!("{}", leaks::USAGE),
        Ok(Plan::Run(forwarded)) => bifrost_cmd::forward("leaks", leaks::PROGRAM, &forwarded),
    }
}
