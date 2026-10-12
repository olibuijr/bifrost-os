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
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const STATE_DIR: &str = "/run/bifrost/bootweave";
const LOG: &str = "/var/log/bifrost-bootweave.log";
const SESSION: &str = "/usr/lib/bifrost/session";
const RUNTIME: &str = "/run/bifrost/user";
const COMPOSITOR_CONF: &str = "/etc/bifrost/compositor";

const WINDOW_SERVER: &str = "/usr/bin/bifrost-wm";
// The shutdown handshake with bifrost-wm (springboard/wm/src/boot.rs, "Symmetric shutdown"). The request is written
// here by root; the faded marker is written by the window server, which runs as the account, so it lives in RUNTIME.
const SHUTDOWN_REQUEST: &str = "/run/bifrost/bootweave/shutdown";
const SHUTDOWN_FADED: &str = "/run/bifrost/user/shutdown-faded";
/// How long `shutdown` waits for the fade. `springboard.service` has a 2 s stop budget (`TimeoutStopSec`), and the
/// session's own stop comes after this, so the wait stays well inside it.
const SHUTDOWN_WAIT_MS: u64 = 1200;
/// The unit's stop budget in seconds, the value `packaging/arch/springboard.service` must carry.
#[cfg(test)]
const STOP_BUDGET_S: u64 = 2;

fn timestamp_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()
}

fn mark(event: &str) {
    if !event.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') { return; }
    springboard::bootsplash::mark(event);
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

/// What `shutdown` does, decided from the configured window server.
#[derive(Debug, PartialEq, Eq)]
enum ShutdownPlan {
    /// bifrost-wm draws the boot frame over the desktop: request the fade and wait for it.
    Fade,
    /// Another window server (or none) has no fade: return at once, the stop goes on.
    Skip,
}

/// The same choice the session script makes (`packaging/arch/wayland/session`): only the bifrost-wm name fades.
fn shutdown_plan(compositor_conf: &str) -> ShutdownPlan {
    if compositor_conf.trim() == "bifrost-wm" { ShutdownPlan::Fade } else { ShutdownPlan::Skip }
}

/// Removes the handshake files of an earlier run (`/run` is cleared on reboot, a restart of the unit is not).
fn clear_shutdown_state() {
    let _ = fs::remove_file(SHUTDOWN_REQUEST);
    let _ = fs::remove_file(SHUTDOWN_FADED);
}

/// `ExecStop=` of springboard.service: asks the window server to fade the desktop back to the boot frame, then waits
/// (at most `SHUTDOWN_WAIT_MS`) until it has. The frame stays up until power-off; the stop of the session follows.
/// Always exits 0, so a missing or stuck window server never blocks the stop.
fn shutdown() {
    let conf = fs::read_to_string(COMPOSITOR_CONF).unwrap_or_default();
    // the session falls back to labwc when the binary is missing, so no fade is coming either
    if shutdown_plan(&conf) == ShutdownPlan::Skip || !Path::new(WINDOW_SERVER).exists() {
        return;
    }
    clear_shutdown_state();
    let _ = fs::create_dir_all(STATE_DIR);
    if fs::write(SHUTDOWN_REQUEST, b"1\n").is_err() {
        return;
    }
    mark("shutdown-request");
    let started = Instant::now();
    while !Path::new(SHUTDOWN_FADED).exists() {
        if started.elapsed() >= Duration::from_millis(SHUTDOWN_WAIT_MS) {
            mark("shutdown-fade-timeout");
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn run() -> ! {
    clear_shutdown_state();
    mark("supervisor-start");
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
        Some("shutdown") => shutdown(),
        _ => { eprintln!("usage: bifrost-bootweave run|mark EVENT|shutdown"); std::process::exit(2); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped unit, read at test time so the tests check the file that gets installed.
    const UNIT: &str = include_str!("../../packaging/arch/springboard.service");

    fn unit_value(key: &str) -> Option<&str> {
        UNIT.lines().map(str::trim).find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
    }

    #[test]
    fn only_the_window_server_that_draws_the_boot_frame_fades_at_shutdown() {
        assert_eq!(shutdown_plan("bifrost-wm\n"), ShutdownPlan::Fade);
        assert_eq!(shutdown_plan("bifrost-wm"), ShutdownPlan::Fade);
        assert_eq!(shutdown_plan("labwc\n"), ShutdownPlan::Skip);
        assert_eq!(shutdown_plan(""), ShutdownPlan::Skip);
        assert_eq!(shutdown_plan("bifrost-wm; rm -rf /"), ShutdownPlan::Skip);
    }

    #[test]
    fn the_handshake_files_are_where_their_writers_can_write() {
        assert!(SHUTDOWN_REQUEST.starts_with(STATE_DIR));
        assert!(SHUTDOWN_FADED.starts_with(RUNTIME));
        assert_ne!(SHUTDOWN_REQUEST, SHUTDOWN_FADED);
    }

    #[test]
    fn the_supervisor_and_the_window_server_agree_on_the_handshake_paths() {
        // bifrost-wm reads the request and writes the marker at these exact paths (springboard/wm/src/boot.rs)
        assert_eq!(SHUTDOWN_REQUEST, "/run/bifrost/bootweave/shutdown");
        assert_eq!(SHUTDOWN_FADED, "/run/bifrost/user/shutdown-faded");
    }

    #[test]
    fn the_window_server_binary_is_the_one_the_session_starts() {
        assert_eq!(WINDOW_SERVER, "/usr/bin/bifrost-wm");
    }

    #[test]
    fn the_fade_wait_fits_inside_the_unit_stop_budget() {
        assert_eq!(STOP_BUDGET_S, 2, "the 2 s budget of the shutdown contract");
        assert!(SHUTDOWN_WAIT_MS < STOP_BUDGET_S * 1000, "wait {SHUTDOWN_WAIT_MS} ms");
        // the fade itself (450 ms) fits in the wait with room for the frame to be presented
        assert!(SHUTDOWN_WAIT_MS >= 450 * 2);
    }

    #[test]
    fn the_unit_asks_for_the_fade_before_it_stops_the_session() {
        assert_eq!(unit_value("ExecStop"), Some("/usr/bin/bifrost-bootweave shutdown"));
        assert_eq!(unit_value("TimeoutStopSec"), Some("2s"));
    }

    #[test]
    fn the_unit_no_longer_parks_the_console_on_vt_12() {
        assert!(!UNIT.contains("ExecStopPost"), "the VT 12 parking is removed");
        assert!(!UNIT.contains("chvt 12"));
    }

    #[test]
    fn the_unit_keeps_its_shutdown_ordering() {
        // shutdown.target still stops this unit, and the unit still conflicts with it
        assert!(UNIT.lines().any(|l| l.trim() == "Before=shutdown.target"));
        assert!(UNIT.lines().any(|l| l.trim() == "Conflicts=shutdown.target"));
    }
}
