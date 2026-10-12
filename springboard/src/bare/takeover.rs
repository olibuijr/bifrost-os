//! Start-up crash-loop guard (`springboard --boot DIR`, and every desktop start).
//!
//! After `MAX_FAILED_BOOTS` starts in a row that never stayed up `HEALTHY_AFTER_MS` the shell
//! exits with `CRASH_LOOP_EXIT` without starting, so the systemd unit runs into its StartLimit
//! and a getty can take the screen (see `begin`). Plain restarts by systemd are the recovery
//! path.
//!
//! Exit statuses the launcher sees:
//! - 3: restart request (Settings > General > Reset), the shell is started again;
//! - 5 (heartbeat::STALL_EXIT): the UI loop stopped beating (see heartbeat);
//! - 70 (CRASH_LOOP_EXIT): the crash-loop rule gave up.
//!
//! Every controlled exit also leaves DIR/last_exit (see crashlog).

use super::sys;
use std::fs;
use std::io::Write;
use std::time::Duration;

/// Consecutive starts that never ran healthily before the shell gives up.
pub const MAX_FAILED_BOOTS: u32 = 3;
/// Running this long counts as a healthy boot and clears the attempt counter.
pub const HEALTHY_AFTER_MS: f64 = 60_000.0;
/// The crash-loop rule gave up (non-zero, so `Restart=on-failure` counts it
/// against StartLimit instead of looping forever).
pub const CRASH_LOOP_EXIT: i32 = 70;
/// A boot_attempts counter this old belongs to an earlier, finished crash loop (systemd restarts
/// every couple of seconds), so a manual start after fixing the cause is not locked out.
pub const ATTEMPTS_STALE_SECS: u64 = 600;

const KEY_BITMAP_BYTES: usize = 96; // (KEY_MAX 0x2ff + 1) / 8

#[derive(Debug, PartialEq)]
pub enum Boot {
    /// This many boots in a row failed: give up.
    CrashLoop(u32),
    /// Start; this is consecutive attempt N.
    TakeOver(u32),
}

/// Whether `code` is set in an EVIOCGKEY bitmap.
pub fn key_held(bits: &[u8], code: u16) -> bool {
    bits.get(code as usize / 8).is_some_and(|b| b & (1 << (code % 8)) != 0)
}

/// The boot_attempts file's count; anything unreadable counts as none.
pub fn parse_attempts(s: &str) -> u32 {
    s.trim().parse().unwrap_or(0)
}

/// The attempt counter given the file's content and its age; an old one counts as none.
pub fn current_attempts(raw: &str, age_secs: u64) -> u32 {
    if age_secs >= ATTEMPTS_STALE_SECS {
        0
    } else {
        parse_attempts(raw)
    }
}

/// The attempt counter after a stall exit: one less. A stall means the shell got as far as its UI loop and then
/// something it waited on (a service that stopped answering) held it, which is not a start failure; counting it let
/// four stalls latch the shell for 10 minutes and the session ended a second after every start.
pub fn after_stall(raw: &str) -> u32 {
    parse_attempts(raw).saturating_sub(1)
}

/// Takes this start's attempt back after a stall (see `after_stall`).
pub fn forgive_stall(dir: &str) {
    let file = format!("{dir}/boot_attempts");
    if let Ok(raw) = fs::read_to_string(&file) {
        let _ = fs::write(&file, after_stall(&raw).to_string());
    }
}

/// What to do given the failed boots so far.
pub fn decide(failed: u32) -> Boot {
    if failed >= MAX_FAILED_BOOTS {
        Boot::CrashLoop(failed)
    } else {
        Boot::TakeOver(failed + 1)
    }
}

/// Appends a timestamped line to DIR/log.
pub fn log(dir: &str, msg: &str) {
    let (h, m, s, ..) = sys::local_time();
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(format!("{dir}/log")) {
        let _ = writeln!(f, "[{h:02}:{m:02}:{s:02}] takeover: {msg}");
    }
}

/// The keys currently down on the input device at `path` (EVIOCGKEY). It
/// needs no grab, so it works while the shell holds the device exclusively.
pub fn key_bitmap(path: &str) -> Option<[u8; KEY_BITMAP_BYTES]> {
    let fd = sys::open_dev(path, sys::O_RDONLY).ok()?;
    let mut bits = [0u8; KEY_BITMAP_BYTES];
    // EVIOCGKEY(len) = _IOC(_IOC_READ, 'E', 0x18, len) fills exactly `len` bytes of `bits`.
    let ok = unsafe { sys::ioctl_ptr(fd, sys::ioc(2, b'E', 0x18, KEY_BITMAP_BYTES), bits.as_mut_ptr()) }.is_ok();
    // SAFETY: `fd` was opened above and is not used afterwards.
    unsafe { sys::close(fd) };
    ok.then_some(bits)
}

/// Crash-loop accounting; returns whether to start. Exits the process with
/// `CRASH_LOOP_EXIT` when the shell keeps dying; otherwise counts this start and clears the
/// counter again from a thread once the shell has stayed up `HEALTHY_AFTER_MS`.
pub fn begin(dir: &str) -> bool {
    let file = format!("{dir}/boot_attempts");
    let age = fs::metadata(&file).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map_or(0, |d| d.as_secs());
    let failed = current_attempts(&fs::read_to_string(&file).unwrap_or_default(), age);
    match decide(failed) {
        Boot::CrashLoop(n) => {
            let why = format!("{n} starts in a row never stayed up {}s: giving up so systemd can show a getty", HEALTHY_AFTER_MS / 1e3);
            log(dir, &format!("{why} (delete {file} to retry sooner)"));
            // the journal too: the file alone left a restart loop looking like a shell that exits for no reason
            eprintln!("springboard: takeover: {why} (delete {file} to retry sooner)");
            super::crashlog::write_last_exit(dir, CRASH_LOOP_EXIT, &why);
            std::process::exit(CRASH_LOOP_EXIT);
        }
        Boot::TakeOver(n) => {
            let _ = fs::write(&file, n.to_string());
            log(dir, &format!("starting (attempt {n} of {MAX_FAILED_BOOTS})"));
        }
    }
    let dir = dir.to_string();
    crate::bare::sys::spawn_named("boot-healthy", move || {
        std::thread::sleep(Duration::from_millis(HEALTHY_AFTER_MS as u64));
        let _ = fs::write(format!("{dir}/boot_attempts"), "0");
        log(&dir, "healthy: attempt counter cleared");
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_bitmap_bits_are_little_endian_per_byte() {
        let mut bits = [0u8; 96];
        assert!(!key_held(&bits, 114));
        bits[14] = 1 << 2; // 114 = 14 * 8 + 2
        assert!(key_held(&bits, 114));
        assert!(!key_held(&bits, 115));
        assert!(!key_held(&bits, 113));
        assert!(!key_held(&bits[..10], 114), "short bitmap means not held");
    }

    #[test]
    fn eviocgkey_number_matches_kernel_headers() {
        // EVIOCGKEY(96) = _IOC(_IOC_READ, 'E', 0x18, 96)
        assert_eq!(sys::ioc(2, b'E', 0x18, KEY_BITMAP_BYTES), 0x8060_4518);
    }

    #[test]
    fn a_stale_counter_counts_as_none() {
        assert_eq!(current_attempts("2", 5), 2);
        assert_eq!(current_attempts("2", ATTEMPTS_STALE_SECS), 0);
        assert_eq!(decide(current_attempts("3", 5)), Boot::CrashLoop(3));
        assert_eq!(decide(current_attempts("3", 3_600)), Boot::TakeOver(1));
    }

    #[test]
    fn a_stall_gives_its_attempt_back() {
        assert_eq!(after_stall("3\n"), 2);
        assert_eq!(after_stall("1"), 0);
        assert_eq!(after_stall("0"), 0);
        assert_eq!(after_stall("junk"), 0);
        // four stalls in a row stay below the latch
        let mut raw = String::from("0");
        for _ in 0..4 {
            let started = parse_attempts(&raw) + 1;
            raw = after_stall(&started.to_string()).to_string();
        }
        assert_eq!(decide(parse_attempts(&raw)), Boot::TakeOver(1));
    }

    #[test]
    fn attempts_parse_leniently() {
        assert_eq!(parse_attempts("2\n"), 2);
        assert_eq!(parse_attempts(""), 0);
        assert_eq!(parse_attempts("junk"), 0);
    }

    #[test]
    fn crash_loop_then_start() {
        assert_eq!(decide(0), Boot::TakeOver(1));
        assert_eq!(decide(2), Boot::TakeOver(3));
        assert_eq!(decide(3), Boot::CrashLoop(3));
        assert_eq!(decide(7), Boot::CrashLoop(7));
    }
}
