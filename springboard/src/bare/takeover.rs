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

/// How a run ended, for the crash-loop counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    /// systemd stopped the service (SIGTERM), a reboot or shutdown was requested, a restart or logout: deliberate.
    Clean,
    /// The UI loop stopped beating (see `after_stall`).
    Stall,
    /// A panic, a kill, a crash or a power loss: nothing wrote an exit, the counter stays as this start left it.
    Crash,
}

/// The attempt counter's content after a run ended `ending`, given the file's `raw` content. Only crashes, panics,
/// kills and stalls leave a failed start counted: a clean stop (three quick benchmark reboots) must not latch the
/// guard and show the start-failed screen.
pub fn attempts_after(raw: &str, ending: Ending) -> u32 {
    match ending {
        Ending::Clean => 0,
        Ending::Stall => after_stall(raw),
        Ending::Crash => parse_attempts(raw),
    }
}

/// A deliberate stop: records `last_exit` (so the next start does not log an unclean death) and clears the attempt
/// counter. Call it before `process::exit` on a requested reboot, shutdown or restart, and from the SIGTERM watcher.
pub fn clean_stop(dir: &str, status: i32, reason: &str) {
    super::crashlog::write_last_exit(dir, status, reason);
    let file = format!("{dir}/boot_attempts");
    let raw = fs::read_to_string(&file).unwrap_or_default();
    let _ = fs::write(&file, attempts_after(&raw, Ending::Clean).to_string());
}

static TERM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn on_term(_sig: i32) {
    TERM.store(true, std::sync::atomic::Ordering::SeqCst);
}

extern "C" {
    fn signal(signum: i32, handler: extern "C" fn(i32)) -> usize;
}

/// SIGTERM (systemd stopping the unit on reboot or shutdown) is a clean stop: a watcher thread, so a hung UI loop
/// cannot hold it, writes `last_exit`, clears the counter and exits 0.
fn watch_term(dir: &str) {
    const SIGTERM: i32 = 15;
    // SAFETY: the handler only stores an atomic flag, which is async-signal-safe.
    unsafe { signal(SIGTERM, on_term) };
    let dir = dir.to_string();
    crate::bare::sys::spawn_named("term-watch", move || loop {
        std::thread::sleep(Duration::from_millis(100));
        if TERM.load(std::sync::atomic::Ordering::SeqCst) {
            clean_stop(&dir, 0, "SIGTERM from the service manager");
            std::process::exit(0);
        }
    });
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
    watch_term(dir);
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
    fn only_crashes_and_stalls_leave_a_failed_start_counted() {
        assert_eq!(attempts_after("2", Ending::Clean), 0, "a clean reboot or shutdown resets the counter");
        assert_eq!(attempts_after("3", Ending::Clean), 0);
        assert_eq!(attempts_after("2", Ending::Crash), 2, "panics, kills and crashes keep the count");
        assert_eq!(attempts_after("2", Ending::Stall), 1, "a stall takes its attempt back");
        assert_eq!(attempts_after("junk", Ending::Crash), 0);
    }

    #[test]
    fn three_quick_clean_reboots_never_latch_the_guard() {
        let mut raw = String::from("0");
        for _ in 0..5 {
            let Boot::TakeOver(n) = decide(parse_attempts(&raw)) else { panic!("latched") };
            raw = n.to_string();
            raw = attempts_after(&raw, Ending::Clean).to_string();
        }
        // three crashes in a row do latch it
        let mut raw = String::from("0");
        for _ in 0..MAX_FAILED_BOOTS {
            let Boot::TakeOver(n) = decide(parse_attempts(&raw)) else { panic!("latched early") };
            raw = attempts_after(&n.to_string(), Ending::Crash).to_string();
        }
        assert_eq!(decide(parse_attempts(&raw)), Boot::CrashLoop(MAX_FAILED_BOOTS));
    }

    #[test]
    fn a_clean_stop_writes_last_exit_and_clears_the_counter() {
        let dir = std::env::temp_dir().join(format!("takeover-clean-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let d = dir.to_str().unwrap();
        fs::write(dir.join("boot_attempts"), "2").unwrap();
        clean_stop(d, 0, "test");
        assert_eq!(fs::read_to_string(dir.join("boot_attempts")).unwrap(), "0");
        assert!(fs::read_to_string(dir.join("last_exit")).unwrap().starts_with("0 test"));
        let _ = fs::remove_dir_all(&dir);
    }

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
