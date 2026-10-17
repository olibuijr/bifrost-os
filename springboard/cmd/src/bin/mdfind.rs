//! `mdfind`: finds files by name under a folder. See `bifrost_cmd::mdfind`.

use bifrost_cmd::die;
use bifrost_cmd::mdfind::{self, Parsed};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match mdfind::parse(&args) {
        Ok(Parsed::Help) => {
            println!("{}", mdfind::USAGE);
            return;
        }
        Ok(Parsed::Run(options)) => options,
        Err(e) => die("mdfind", &e, 1),
    };
    let start = mdfind::start_folder(options.onlyin.as_deref(), std::env::var("HOME").ok().as_deref());
    let found = mdfind::search_with(&start, &options.query, options.content);
    if options.count {
        println!("{}", found.len());
        return;
    }
    for path in found {
        println!("{}", path.display());
    }
}
