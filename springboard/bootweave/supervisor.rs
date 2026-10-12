//! BifrOSt's boot-to-desktop handoff supervisor.
//!
//! The Rust EFI loader paints the branded frame, simpledrm keeps it visible
//! while Linux starts, and this process owns the final lifecycle transition to
//! the BifrOSt session. One owner avoids the old Plymouth/compositor race.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::{Command, ExitStatus};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const STATE_DIR: &str = "/run/bifrost/bootweave";
const LOG: &str = "/var/log/bifrost-bootweave.log";
const SESSION: &str = "/usr/lib/bifrost/session";
const RUNTIME: &str = "/run/bifrost/user";

fn timestamp_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()
}

fn mark(event: &str) {
    if !event.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') { return; }
    springboard::bootsplash::mark(event);
    // Journal copy for `tools/boot-bench.py`: a shutdown-start mark is read back from the previous boot's journal,
    // because /run/bifrost/boot-marks is gone after the reboot.
    eprintln!("boot-mark {event} {} {:.1}", timestamp_ms(), springboard::bootsplash::boottime_ms());
    let _ = fs::create_dir_all(STATE_DIR);
    let line = format!("{} {}\n", timestamp_ms(), event);
    let _ = fs::write(format!("{STATE_DIR}/last"), &line);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(LOG) {
        let _ = file.write_all(line.as_bytes());
    }
}

fn exit_status(status: ExitStatus) -> ! {
    std::process::exit(status.code().unwrap_or(1));
}

fn run() -> ! {
    mark("bootweave-start");
    mark("display-handoff-start");
    let mut child = match Command::new(SESSION).env("BIFROST_BOOTWEAVE", "1").spawn() {
        Ok(child) => child,
        Err(error) => {
            mark("session-spawn-failed");
            eprintln!("bifrost-bootweave: cannot start session: {error}");
            std::process::exit(1);
        }
    };
    // Marks come from a short-lived watcher that ends once both milestones are seen; after the
    // first frame this process only blocks in wait().
    thread::spawn(|| {
        let ready = Path::new(RUNTIME).join("shell-ready");
        let wayland = Path::new(RUNTIME).join("wayland-0");
        let mut compositor_seen = false;
        loop {
            if !compositor_seen && wayland.exists() {
                compositor_seen = true;
                mark("compositor-ready");
            }
            if ready.exists() {
                if !compositor_seen { mark("compositor-ready"); }
                mark("desktop-first-frame");
                // Provisional until the window server's three-part gate (docs/boot-architecture.md, "Reveal once"):
                // today the shell's first ready frame is the desktop.
                mark("desktop-ready");
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
    });
    match child.wait() {
        Ok(status) => { mark("session-exit"); exit_status(status); }
        Err(error) => {
            eprintln!("bifrost-bootweave: wait failed: {error}");
            std::process::exit(1);
        }
    }
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("run") => run(),
        Some("mark") => { if let Some(event) = std::env::args().nth(2) { mark(&event); } }
        _ => { eprintln!("usage: bifrost-bootweave run|mark EVENT"); std::process::exit(2); }
    }
}
