//! `diskutil`: lists disks, shows a device's details, mounts, unmounts or ejects it, and runs the privileged erase,
//! partition and first-aid requests. See `bifrost_cmd::diskutil`.

use bifrost_cmd::die;
use bifrost_cmd::diskutil::{self, Action, Cmd};
use bifrost_core::disk_run;
use bifrost_core::disk_scan;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = match diskutil::parse(&args) {
        Ok(cmd) => cmd,
        Err(e) => die("diskutil", &e, 1),
    };
    if cmd == Cmd::Help {
        println!("{}", diskutil::USAGE);
        return;
    }
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    let entries = disk_scan::scan(Path::new("/sys/block"), &mounts);
    let result = match cmd {
        Cmd::Help => return,
        Cmd::List => {
            print!("{}", diskutil::list_text(&entries));
            Ok(())
        }
        Cmd::Info(name) => diskutil::info_text(&entries, &name).map(|text| print!("{text}")),
        Cmd::Act(action, name) => match diskutil::plan(action, &entries, &name) {
            Ok(plan) => disk_run::run_plan(&plan, false).map(|_| println!("{}", done_text(action, &name))),
            Err(refusal) => Err(refusal.sentence()),
        },
        Cmd::Request(request) => diskutil::request(&request).map(|_| println!("{}", diskutil::request_done_text(&request))),
    };
    if let Err(e) = result {
        die("diskutil", &e, 1);
    }
}

fn done_text(action: Action, name: &str) -> String {
    match action {
        Action::Mount => format!("Mounted {name}."),
        Action::Unmount => format!("Unmounted {name}."),
        Action::Eject => format!("Ejected {name}."),
    }
}
