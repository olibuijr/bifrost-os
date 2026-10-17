//! `smbutil`: looks at SMB servers through the Samba client programs. See `bifrost_cmd::smbutil`.

use bifrost_cmd::die;
use bifrost_cmd::smbutil::{self, Plan};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match smbutil::parse(&args) {
        Err(e) => die("smbutil", &e, 1),
        Ok(Plan::Help) => println!("{}", smbutil::USAGE),
        Ok(Plan::Run { program, args }) => bifrost_cmd::forward("smbutil", program, &args),
    }
}
