//! `purge`: flushes file buffers and drops the kernel's caches. See `bifrost_cmd::purge`.

use bifrost_cmd::die;
use bifrost_cmd::purge::{self, Request};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match purge::parse(&args) {
        Ok(Request::Help) => println!("{}", purge::USAGE),
        Ok(Request::Run) => {
            if let Err(e) = purge::run_with(purge::sync_all, purge::write_knob) {
                die("purge", &e, 1);
            }
        }
        Err(e) => die("purge", &e, 1),
    }
}
