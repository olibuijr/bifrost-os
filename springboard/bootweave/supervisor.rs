//! BifrOSt's boot-to-desktop handoff supervisor (the `bootweave` binary, `/usr/bin/bifrost-bootweave`).
//!
//! The Rust EFI loader paints the boot frame, and this process owns the final lifecycle transition to the BifrOSt
//! session. One owner avoids the old Plymouth/compositor race. Failures are shown in the boot frame, never as text
//! (docs/boot-architecture.md, "Text-free on every path"):
//! - a kernel panic record in pstore on this boot shows "Your computer restarted because of a problem" (`crash_notice`);
//! - a session that cannot start (three quick failures in a row) shows "BifrOSt could not start" with Restart and
//!   Recovery (`start_failed`), instead of the restart loop of the service.
//!
//! Owner: boot-11 (failure screens and pstore). The loader's rollback screen is in `bootweave/efi`.

use bootweave_frame::{render, Appearance, Frame, PixelOrder, Screen};
use springboard::bare::input::{Device, Event};
use springboard::bare::kms::Display;
use springboard::gfx::Damage;
use springboard::pstore;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const STATE_DIR: &str = "/run/bifrost/bootweave";
/// Consecutive quick session failures in this boot (a file, so it survives the service's restarts).
const FAILURES: &str = "/run/bifrost/bootweave/failures";
const LOG: &str = "/var/log/bifrost-bootweave.log";
const SESSION: &str = "/usr/lib/bifrost/session";
const RUNTIME: &str = "/run/bifrost/user";
/// The boot frame's appearance until the timing record can say (task 06 stores the user's choice there).
const APPEARANCE: Appearance = Appearance::Dark;
/// How long the crash notice stays on the panel before the session takes it over.
const CRASH_NOTICE_MS: u64 = 3000;
/// How long to wait for a display to open before giving up on a screen.
const DISPLAY_WAIT_MS: u64 = 5000;

/// Quick failures before the start-failure screen replaces the restart loop.
pub const FAILS_BEFORE_SCREEN: u32 = 3;
/// A session that ran this long had started: its later exit is a crash, not a start failure.
pub const STARTED_MS: u128 = 20_000;

/// What follows a session exit.
#[derive(Debug, PartialEq, Eq)]
pub enum Next {
    /// A clean exit: the supervisor exits 0 and the service stays down.
    Done,
    /// A failure short of the screen: exit non-zero and let systemd restart the service.
    Retry,
    /// The session cannot start: show the start-failure screen and hold it.
    Screen,
}

/// Decides what follows a session exit, given the consecutive start failures so far. Returns the new count and the
/// next step. A clean exit resets the count; a session that ran for `STARTED_MS` or more counts as started, so its
/// crash resets the count too and it simply restarts.
pub fn after_exit(count: u32, clean: bool, ran_ms: u128) -> (u32, Next) {
    if clean {
        return (0, Next::Done);
    }
    if ran_ms >= STARTED_MS {
        return (0, Next::Retry);
    }
    let count = count.saturating_add(1);
    if count >= FAILS_BEFORE_SCREEN {
        (count, Next::Screen)
    } else {
        (count, Next::Retry)
    }
}

/// The failure count from the state file's text (anything unreadable counts as none).
pub fn parse_count(text: &str) -> u32 {
    text.trim().parse().unwrap_or(0)
}

// Linux key codes the failure screen uses (linux/input-event-codes.h).
const KEY_TAB: u16 = 15;
const KEY_ENTER: u16 = 28;
const KEY_SPACE: u16 = 57;
const KEY_KPENTER: u16 = 96;
const KEY_LEFT: u16 = 105;
const KEY_RIGHT: u16 = 106;
/// The start-failure screen's buttons, in order: Restart is first and selected at the start.
const BUTTONS: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Previous,
    Next,
    Activate,
}

/// The failure screen's meaning of a key press, `None` for keys it does not use.
pub fn key_of(code: u16) -> Option<Key> {
    match code {
        KEY_LEFT => Some(Key::Previous),
        KEY_RIGHT | KEY_TAB => Some(Key::Next),
        KEY_ENTER | KEY_KPENTER | KEY_SPACE => Some(Key::Activate),
        _ => None,
    }
}

/// The selected button after `key`. Moving stops at the ends (no wrap); activating keeps the selection.
pub fn step(selected: u32, key: Key) -> u32 {
    match key {
        Key::Previous => selected.saturating_sub(1),
        Key::Next => (selected + 1).min(BUTTONS - 1),
        Key::Activate => selected,
    }
}

/// The button a selection index names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Restart,
    Recovery,
}

pub fn button(selected: u32) -> Button {
    if selected == 0 { Button::Restart } else { Button::Recovery }
}

fn timestamp_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()
}

fn mark(event: &str) {
    if !event.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return;
    }
    springboard::bootsplash::mark(event);
    let _ = fs::create_dir_all(STATE_DIR);
    let line = format!("{} {}\n", timestamp_ms(), event);
    let _ = fs::write(format!("{STATE_DIR}/last"), &line);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(LOG) {
        let _ = file.write_all(line.as_bytes());
    }
}

fn read_count() -> u32 {
    fs::read_to_string(FAILURES).map(|t| parse_count(&t)).unwrap_or(0)
}

fn write_count(count: u32) {
    let _ = fs::create_dir_all(STATE_DIR);
    let _ = fs::write(FAILURES, format!("{count}\n"));
}

fn park() -> ! {
    loop {
        thread::sleep(Duration::from_secs(3600));
    }
}

/// Renders `screen` at the panel's size and presents it. The display stays open while the caller holds it.
fn present(display: &mut Display, screen: Screen) {
    let (w, h) = display.size();
    let frame = Frame { width: w as u32, height: h as u32, appearance: APPEARANCE, progress: None, screen };
    let mut pixels = vec![0u32; w * h];
    render(&frame, &mut pixels, w, PixelOrder::Bgrx);
    let mut damage = Damage::new(w, h);
    damage.all();
    display.present(&pixels, &damage);
}

/// "Your computer restarted because of a problem": one screen for the pstore crash records of this boot, then the
/// records are removed. Without a display the records stay for the next boot.
fn crash_notice(records: &[PathBuf]) {
    let Ok(mut display) = Display::open_best(DISPLAY_WAIT_MS) else {
        eprintln!("bifrost-bootweave: no display for the crash notice; keeping {} record(s)", records.len());
        return;
    };
    present(&mut display, Screen::RestartedAfterProblem);
    mark("crash-notice-shown");
    thread::sleep(Duration::from_millis(CRASH_NOTICE_MS));
    pstore::clear(records);
    // the display closes here, so the session can take the panel
}

fn open_inputs() -> Vec<Device> {
    (0..32).filter_map(|i| Device::open(&format!("/dev/input/event{i}")).ok()).collect()
}

fn restart() {
    mark("restart-requested");
    if let Err(error) = Command::new("systemctl").arg("reboot").status() {
        eprintln!("bifrost-bootweave: cannot restart: {error}");
    }
}

/// "BifrOSt could not start": Restart and Recovery, held until one is chosen. Never returns.
fn start_failed() -> ! {
    mark("start-failed");
    let Ok(mut display) = Display::open_best(DISPLAY_WAIT_MS) else {
        // no panel to draw on: hold the service rather than restart it in a loop
        park();
    };
    let mut selected = 0u32;
    present(&mut display, Screen::StartFailed { selected });
    mark("start-failed-shown");
    let mut inputs = open_inputs();
    loop {
        let mut events = Vec::new();
        for device in inputs.iter_mut() {
            device.read(&mut events);
        }
        let mut changed = false;
        for event in events {
            let Event::Key { code, down: true } = event else { continue };
            let Some(key) = key_of(code) else { continue };
            if key == Key::Activate {
                match button(selected) {
                    Button::Restart => restart(),
                    // TODO(boot-11): Recovery needs the recovery environment, which does not exist yet.
                    Button::Recovery => mark("recovery-requested"),
                }
            } else {
                let next = step(selected, key);
                changed |= next != selected;
                selected = next;
            }
        }
        if changed {
            present(&mut display, Screen::StartFailed { selected });
        }
        thread::sleep(Duration::from_millis(30));
    }
}

/// Applies the failure policy to a session exit (`status` None when it could not be spawned) and exits, shows the
/// start-failure screen, or lets systemd restart the service.
fn finish(status: Option<ExitStatus>, ran: Duration) -> ! {
    let clean = status.is_some_and(|s| s.success());
    let (count, next) = after_exit(read_count(), clean, ran.as_millis());
    write_count(count);
    match next {
        Next::Done => std::process::exit(0),
        Next::Retry => std::process::exit(status.and_then(|s| s.code()).filter(|c| *c != 0).unwrap_or(1)),
        Next::Screen => start_failed(),
    }
}

fn run() -> ! {
    mark("supervisor-start");
    mark("display-handoff-start");
    let records = pstore::crash_records(Path::new(pstore::DIR));
    if !records.is_empty() {
        crash_notice(&records);
    }
    let started = Instant::now();
    let mut child = match Command::new(SESSION).env("BIFROST_BOOTWEAVE", "1").spawn() {
        Ok(child) => child,
        Err(error) => {
            mark("session-spawn-failed");
            eprintln!("bifrost-bootweave: cannot start session: {error}");
            finish(None, started.elapsed());
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
                if !compositor_seen {
                    mark("compositor-ready");
                }
                mark("desktop-first-frame");
                // reaching the desktop ends this boot's run of start failures
                write_count(0);
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
    });
    match child.wait() {
        Ok(status) => {
            mark("session-exit");
            finish(Some(status), started.elapsed());
        }
        Err(error) => {
            eprintln!("bifrost-bootweave: wait failed: {error}");
            std::process::exit(1);
        }
    }
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("run") => run(),
        Some("mark") => {
            if let Some(event) = std::env::args().nth(2) {
                mark(&event);
            }
        }
        _ => {
            eprintln!("usage: bifrost-bootweave run|mark EVENT");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_exit_ends_the_policy_and_resets_the_count() {
        assert_eq!(after_exit(2, true, 500), (0, Next::Done));
    }

    #[test]
    fn quick_failures_retry_until_the_third_then_show_the_screen() {
        assert_eq!(after_exit(0, false, 500), (1, Next::Retry));
        assert_eq!(after_exit(1, false, 500), (2, Next::Retry));
        assert_eq!(after_exit(2, false, 500), (3, Next::Screen));
        assert_eq!(after_exit(3, false, 500), (4, Next::Screen), "held on the screen, never back to retry");
    }

    #[test]
    fn a_session_that_ran_long_enough_restarts_without_counting() {
        assert_eq!(after_exit(2, false, 19_999), (3, Next::Screen), "just under STARTED_MS still counts");
        assert_eq!(after_exit(2, false, 20_000), (0, Next::Retry), "STARTED_MS counts as started");
        assert_eq!(after_exit(0, false, 90_000), (0, Next::Retry));
    }

    #[test]
    fn a_spawn_failure_is_a_quick_failure() {
        assert_eq!(after_exit(0, false, 0), (1, Next::Retry));
    }

    #[test]
    fn the_count_file_reads_back_or_counts_as_none() {
        assert_eq!(parse_count("2\n"), 2);
        assert_eq!(parse_count(""), 0);
        assert_eq!(parse_count("two"), 0);
        assert_eq!(parse_count("-1"), 0);
    }

    #[test]
    fn keys_map_to_the_failure_screen_actions() {
        assert_eq!(key_of(KEY_LEFT), Some(Key::Previous));
        assert_eq!(key_of(KEY_RIGHT), Some(Key::Next));
        assert_eq!(key_of(KEY_TAB), Some(Key::Next));
        assert_eq!(key_of(KEY_ENTER), Some(Key::Activate));
        assert_eq!(key_of(KEY_KPENTER), Some(Key::Activate));
        assert_eq!(key_of(30), None, "the A key does nothing here");
    }

    #[test]
    fn selection_moves_between_the_two_buttons_without_wrapping() {
        assert_eq!(step(0, Key::Next), 1);
        assert_eq!(step(1, Key::Next), 1, "Recovery is the last button");
        assert_eq!(step(1, Key::Previous), 0);
        assert_eq!(step(0, Key::Previous), 0, "Restart is the first button");
        assert_eq!(step(1, Key::Activate), 1, "activating keeps the selection");
    }

    #[test]
    fn the_first_button_is_restart() {
        assert_eq!(button(0), Button::Restart);
        assert_eq!(button(1), Button::Recovery);
    }
}
