//! Frozen-UI detection.
//!
//! Contract (for tools/boot.sh and anything else reading it):
//! - The main UI loop calls `beat()` every iteration; at most once per 2 s
//!   that rewrites `DIR/heartbeat` with one line, `<uptime_ms>\n`, where the
//!   value is the CLOCK_MONOTONIC reading in whole milliseconds (it stops
//!   during suspend, like the loop does). A reader that sees the value stand
//!   still while the process lives can treat the UI as hung.
//! - `watch()` runs a thread that, once the loop has not beaten for
//!   STALL_MS (20 s), appends "ui loop stalled for Ns" to crash.log, writes
//!   last_exit and exits the process with status STALL_EXIT (5). The launcher
//!   may restart the shell or fall back to Android on 5.
//!
//! Suspend: CLOCK_MONOTONIC stops while asleep, so stall detection on it never
//! counts sleep as a stall. The watcher also tracks the sleep total
//! (CLOCK_BOOTTIME minus CLOCK_MONOTONIC); when it grew since the last check
//! the device just resumed, and the loop gets a fresh STALL_MS window because
//! it cannot have beaten before the watcher thread got scheduled. The file's
//! optional second line is `<boottime_ms>`, for readers that want a clock
//! which keeps running through suspend; the first line is unchanged.
//!
//! The loop must therefore wake at least every BEAT_CAP_MS even when idle.

use super::{crashlog, sys};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

pub const STALL_MS: f64 = 20_000.0;
/// Exit status when the UI loop stalled.
pub const STALL_EXIT: i32 = 5;
/// Minimum spacing between writes of the heartbeat file.
pub const WRITE_EVERY_MS: f64 = 2_000.0;
/// Longest the main loop may sleep without beating.
pub const BEAT_CAP_MS: i32 = 10_000;
/// The stall watcher checks this often (it was 1 s: one wakeup a second for a 20 s deadline).
/// Worst-case detection is STALL_MS plus this.
pub const WATCH_EVERY_SECS: u64 = 5;

static DIR: OnceLock<String> = OnceLock::new();
/// Last beat (monotonic ms, as bits of an f64 would be overkill: whole ms).
static LAST_BEAT: AtomicU64 = AtomicU64::new(0);
static LAST_WRITE: AtomicU64 = AtomicU64::new(0);

/// True once `now_ms` is STALL_MS or more past the last beat.
pub fn stalled(last_beat_ms: f64, now_ms: f64) -> bool {
    now_ms - last_beat_ms >= STALL_MS
}

/// True when the UI loop has not beaten for longer than its idle cap plus slack: it may be hung,
/// so the recovery chord watcher samples the keys itself.
pub fn ui_stale(now_ms: f64) -> bool {
    now_ms - LAST_BEAT.load(Ordering::Relaxed) as f64 >= (BEAT_CAP_MS + 2_000) as f64
}

/// Whether the heartbeat file is due for a rewrite.
pub fn write_due(last_write_ms: f64, now_ms: f64) -> bool {
    now_ms - last_write_ms >= WRITE_EVERY_MS
}

/// Wakeups and presented frames of the main loop, reported once a minute so idle cost can be read from the journal.
#[derive(Default)]
pub struct IdleStats {
    since: Option<f64>,
    wakeups: u64,
    frames: u64,
}

/// Longest spacing between "idle: ..." lines.
pub const IDLE_REPORT_MS: f64 = 60_000.0;

impl IdleStats {
    /// One loop iteration; `frame` is true when it presented. Returns `(wakeups, frames)` once a minute has passed, then starts over.
    pub fn note(&mut self, now: f64, frame: bool) -> Option<(u64, u64)> {
        let since = *self.since.get_or_insert(now);
        self.wakeups += 1;
        self.frames += u64::from(frame);
        if now - since < IDLE_REPORT_MS {
            return None;
        }
        let out = (self.wakeups, self.frames);
        *self = IdleStats { since: Some(now), ..Default::default() };
        Some(out)
    }
}

/// File body for an uptime.
pub fn format_line(uptime_ms: f64) -> String {
    format!("{}\n", uptime_ms as u64)
}

/// Time spent suspended between two watcher checks, given the sleep totals
/// (boottime - monotonic) seen at each; 0 when none or the clocks misbehave.
pub fn slept_between(prev_total_ms: f64, total_ms: f64) -> f64 {
    (total_ms - prev_total_ms).max(0.0)
}

/// True when a resume of `slept_ms` is long enough to rebase the beat (anything over scheduling noise).
pub fn resumed(slept_ms: f64) -> bool {
    slept_ms >= 100.0
}

/// Arms the heartbeat for `dir` and counts now as the first beat.
pub fn init(dir: &str) {
    let _ = DIR.set(dir.to_string());
    LAST_BEAT.store(sys::now_ms() as u64, Ordering::Relaxed);
    LAST_WRITE.store(0, Ordering::Relaxed);
}

/// Called by the main loop each iteration; cheap.
pub fn beat() {
    let now = sys::now_ms();
    LAST_BEAT.store(now as u64, Ordering::Relaxed);
    if write_due(LAST_WRITE.load(Ordering::Relaxed) as f64, now) {
        LAST_WRITE.store(now as u64, Ordering::Relaxed);
        if let Some(dir) = DIR.get() {
            let _ = std::fs::write(format!("{dir}/heartbeat"), format!("{}{}\n", format_line(now), sys::boottime_ms() as u64));
        }
    }
}

/// Starts the stall watcher thread (call after `init`).
pub fn watch(dir: &str) {
    let dir = dir.to_string();
    std::thread::Builder::new()
        .name("heartbeat-watch".into())
        .spawn(move || {
            let mut total = sys::boottime_ms() - sys::now_ms();
            loop {
            std::thread::sleep(Duration::from_secs(WATCH_EVERY_SECS));
            let now = sys::now_ms();
            let new_total = sys::boottime_ms() - now;
            if resumed(slept_between(total, new_total)) {
                LAST_BEAT.store(now as u64, Ordering::Relaxed);
            }
            total = new_total;
            let last = LAST_BEAT.load(Ordering::Relaxed) as f64;
            if ui_stale(now) {
                super::chord_recovery::poke();
            }
            if stalled(last, now) {
                let secs = ((now - last) / 1e3).round() as u64;
                crashlog::append(&dir, &format!("ui loop stalled for {secs}s"));
                crashlog::write_last_exit(&dir, STALL_EXIT, &format!("ui loop stalled for {secs}s"));
                // a stall is not a start failure: do not let it count toward the crash-loop latch
                super::takeover::forgive_stall(&dir);
                eprintln!("springboard: ui loop stalled for {secs}s; exiting");
                std::process::exit(STALL_EXIT);
            }
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_stats_report_once_a_minute() {
        let mut s = IdleStats::default();
        assert_eq!(s.note(0.0, true), None);
        assert_eq!(s.note(30_000.0, false), None);
        assert_eq!(s.note(60_000.0, true), Some((3, 2)));
        assert_eq!(s.note(60_001.0, false), None);
    }

    #[test]
    fn stalls_after_twenty_seconds() {
        assert!(!stalled(1_000.0, 20_999.0));
        assert!(stalled(1_000.0, 21_000.0));
    }

    #[test]
    fn writes_every_two_seconds() {
        assert!(!write_due(0.0, 1_999.0));
        assert!(write_due(0.0, 2_000.0));
    }

    #[test]
    fn sleep_total_growth_is_a_resume() {
        assert_eq!(slept_between(5_000.0, 65_000.0), 60_000.0);
        assert_eq!(slept_between(5_000.0, 4_990.0), 0.0);
        assert!(resumed(60_000.0));
        assert!(!resumed(3.0));
    }

    #[test]
    fn line_is_whole_ms_and_newline() {
        assert_eq!(format_line(12_345.9), "12345\n");
    }
}
