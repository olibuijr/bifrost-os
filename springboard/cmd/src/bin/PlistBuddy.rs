//! `PlistBuddy`: runs `-c` commands on a property list and writes the result back. See `bifrost_cmd::plistbuddy`.

use bifrost_cmd::die;
use bifrost_cmd::plistbuddy::{self, USAGE};
use bifrost_core::plist;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = match plistbuddy::parse(&args) {
        Ok(opts) => opts,
        Err(e) => die("PlistBuddy", &e, 1),
    };
    if opts.help {
        println!("{USAGE}");
        return;
    }
    let Some(file) = opts.file else {
        die("PlistBuddy", "no input file", 1);
    };
    let bytes = match std::fs::read(&file) {
        Ok(bytes) => bytes,
        Err(e) => die("PlistBuddy", &format!("{file}: {e}"), 1),
    };
    let mut root = match plist::parse_bytes(&bytes) {
        Ok(root) => root,
        Err(e) => die("PlistBuddy", &format!("{file}: {e}"), 1),
    };
    if opts.commands.is_empty() {
        let save = |root: &plist::Plist| {
            bifrost_core::fsutil::write_atomic(Path::new(&file), plist::to_xml(root)).map_err(|e| format!("{file}: {e}"))
        };
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout();
        if let Err(e) = plistbuddy::interactive(&mut root, stdin.lock(), &mut stdout, opts.xml, save) {
            die("PlistBuddy", &e, 1);
        }
        return;
    }
    let run = match plistbuddy::run(&mut root, &opts.commands, opts.xml) {
        Ok(run) => run,
        Err(e) => die("PlistBuddy", &format!("{file}: {e}"), 1),
    };
    print!("{}", run.output);
    if run.changed {
        if let Err(e) = bifrost_core::fsutil::write_atomic(Path::new(&file), plist::to_xml(&root)) {
            die("PlistBuddy", &format!("{file}: {e}"), 1);
        }
    }
}
