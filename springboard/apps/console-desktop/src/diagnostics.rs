//! Hang diagnostics and the daily summary, the two Reports rows that sit beside Crash Reports.
//!
//! A hang is the shell's UI loop stopping for `heartbeat::STALL_MS`. The heartbeat writes `ui loop stalled for Ns` to
//! the shell's `crash.log`, and `springboard::shell::diag` reads that file. A crash is a row of `coredumpctl`, as in
//! Crash Reports. The daily summary counts both per UTC day.
//!
//! The pure parts (the stall parser, the day grouping and the summary text) are here and tested without a system. The
//! sources read the files and run `coredumpctl`. No call stack is kept for a hang: the shell records how long its loop
//! stalled, not where it was. Crash reports keep their stacks in `coredumpctl info`, which Crash Reports shows.

use crate::model::{Level, LogEntry};
use crate::source::{Coredumps, Source};
use bifrost_app::testing;
use springboard::shell::diag::{self, CrashEntry};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The process column of the rows that the shell writes.
pub const PROCESS: &str = "springboard";
/// The text the heartbeat writes before the stall length in seconds.
pub const STALL_PREFIX: &str = "ui loop stalled for ";
/// The footer note while the hang view is shown.
pub const HANG_NOTE: &str = "A hang is the shell's UI loop stopping for at least 20 s. No call stack is kept for a hang.";
/// The footer note while the daily summary is shown.
pub const DAILY_NOTE: &str = "Crash reports and hangs per day, in UTC. Choose a row for its details.";

const DAY_SECS: i64 = 86_400;

/// The seconds a stall message names: `ui loop stalled for 21s` gives 21. Anything else gives None.
pub fn stall_seconds(message: &str) -> Option<u64> {
    message.strip_prefix(STALL_PREFIX)?.strip_suffix('s')?.parse().ok()
}

/// The hang row for one stall at `unix` seconds. Times are microseconds, as the journal's are.
pub fn hang_row(unix: i64, message: &str) -> LogEntry {
    LogEntry {
        date: unix.saturating_mul(1_000_000),
        process: PROCESS.into(),
        pid: 0,
        subsystem: String::new(),
        category: "heartbeat".into(),
        level: Level::Fault,
        message: message.into(),
        invocation: String::new(),
    }
}

/// One row per stall in the crash.log entries, oldest first. Panics and other entries are not hangs and are left out.
pub fn hang_rows(entries: &[CrashEntry]) -> Vec<LogEntry> {
    let mut stalls: Vec<&CrashEntry> = entries.iter().filter(|e| stall_seconds(&e.signature).is_some()).collect();
    stalls.sort_by_key(|e| e.unix);
    stalls.into_iter().map(|e| hang_row(e.unix, &e.signature)).collect()
}

/// The directories whose crash.log the shell may have written: the system's (a root shell) and the user's.
pub fn crash_log_dirs() -> Vec<PathBuf> {
    let xdg = std::env::var("XDG_DATA_HOME").ok();
    let home = std::env::var("HOME").ok();
    let user = springboard::bare::platform::data_dir(false, xdg.as_deref(), home.as_deref());
    let system = springboard::bare::platform::data_dir(true, None, None);
    let mut dirs = vec![PathBuf::from(&system)];
    if user != system {
        dirs.push(PathBuf::from(user));
    }
    dirs
}

/// Every crash.log entry the shell kept, from each data directory, oldest first.
pub fn shell_crashes() -> Vec<CrashEntry> {
    let mut all: Vec<CrashEntry> = crash_log_dirs().iter().flat_map(|dir| diag::load(dir)).collect();
    all.sort_by_key(|e| e.unix);
    all
}

/// One day's counts. `day` is the UTC day number (Unix days), so a negative time falls on a negative day.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Day {
    pub day: i64,
    pub crashes: u32,
    pub hangs: u32,
    /// The stall seconds of the day's hangs, added up.
    pub stalled_secs: u64,
}

/// Groups crash times and hang times by UTC day, oldest day first. Times are Unix seconds; a hang carries its stall
/// length in seconds. A day with neither a crash nor a hang is not listed.
pub fn by_day(crash_times: &[i64], hangs: &[(i64, u64)]) -> Vec<Day> {
    let mut days: BTreeMap<i64, Day> = BTreeMap::new();
    for &t in crash_times {
        let d = t.div_euclid(DAY_SECS);
        let day = days.entry(d).or_insert(Day { day: d, ..Day::default() });
        day.crashes = day.crashes.saturating_add(1);
    }
    for &(t, secs) in hangs {
        let d = t.div_euclid(DAY_SECS);
        let day = days.entry(d).or_insert(Day { day: d, ..Day::default() });
        day.hangs = day.hangs.saturating_add(1);
        day.stalled_secs = day.stalled_secs.saturating_add(secs);
    }
    days.into_values().collect()
}

fn count(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The day's message: "2 crashes, 1 hang, 21 s stalled". Only the parts that happened are named.
pub fn summary(day: &Day) -> String {
    let mut parts = Vec::new();
    if day.crashes > 0 {
        parts.push(count(day.crashes, "crash", "crashes"));
    }
    if day.hangs > 0 {
        parts.push(format!("{}, {} s stalled", count(day.hangs, "hang", "hangs"), day.stalled_secs));
    }
    parts.join(", ")
}

/// One row per day with a crash or a hang, oldest first. The time is the start of the UTC day. A day with a crash is a
/// Fault; a day with only hangs is an Error.
pub fn daily_rows(crash_times: &[i64], hangs: &[(i64, u64)]) -> Vec<LogEntry> {
    by_day(crash_times, hangs)
        .iter()
        .map(|day| LogEntry {
            date: day.day.saturating_mul(DAY_SECS).saturating_mul(1_000_000),
            process: String::new(),
            pid: 0,
            subsystem: String::new(),
            category: "daily".into(),
            level: if day.crashes > 0 { Level::Fault } else { Level::Error },
            message: summary(day),
            invocation: String::new(),
        })
        .collect()
}

/// Keeps the newest `limit` rows, as the other sources do.
fn newest(mut rows: Vec<LogEntry>, limit: usize) -> Vec<LogEntry> {
    if rows.len() > limit {
        rows.drain(..rows.len() - limit);
    }
    rows
}

/// The hangs the shell recorded, one row each.
pub struct HangSource;

impl Source for HangSource {
    fn load(&mut self, limit: usize) -> Result<Vec<LogEntry>, String> {
        if testing::pinned() {
            return Ok(Vec::new());
        }
        Ok(newest(hang_rows(&shell_crashes()), limit))
    }

    fn detail(&mut self, e: &LogEntry) -> Option<String> {
        let secs = stall_seconds(&e.message)?;
        Some(format!(
            "The shell's UI loop stopped answering for {secs} s.\nCall stack: none. The shell records how long the loop stalled, not where it was.\nCrash reports with their call stacks are under Crash Reports."
        ))
    }
}

/// Crash reports and hangs per UTC day.
pub struct DailySource;

impl Source for DailySource {
    fn load(&mut self, limit: usize) -> Result<Vec<LogEntry>, String> {
        if testing::pinned() {
            return Ok(Vec::new());
        }
        // without coredumpctl the crash count is zero, and the hangs are still counted
        let mut coredumps = Coredumps;
        let crash_times: Vec<i64> = coredumps.load(usize::MAX).unwrap_or_default().iter().map(|e| e.date.div_euclid(1_000_000)).collect();
        let shell = shell_crashes();
        let hangs: Vec<(i64, u64)> = shell.iter().filter_map(|e| stall_seconds(&e.signature).map(|s| (e.unix, s))).collect();
        Ok(newest(daily_rows(&crash_times, &hangs), limit))
    }

    fn detail(&mut self, e: &LogEntry) -> Option<String> {
        Some(format!("{}\nThe counts are for one UTC day. Crash Reports and Hang Diagnostics list each entry.", e.message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crash_entry(unix: i64, signature: &str) -> CrashEntry {
        CrashEntry { unix, signature: signature.into(), file: PathBuf::from("/d/crash.log") }
    }

    #[test]
    fn a_stall_message_gives_its_seconds_and_nothing_else_does() {
        assert_eq!(stall_seconds("ui loop stalled for 21s"), Some(21));
        assert_eq!(stall_seconds("ui loop stalled for 0s"), Some(0));
        assert_eq!(stall_seconds("ui loop stalled for s"), None, "no number");
        assert_eq!(stall_seconds("ui loop stalled for 21"), None, "no unit");
        assert_eq!(stall_seconds("src/x.rs:1 boom"), None);
    }

    #[test]
    fn hang_rows_keep_only_stalls_oldest_first_with_microsecond_times() {
        let entries = vec![
            crash_entry(200, "ui loop stalled for 25s"),
            crash_entry(100, "src/x.rs:1 boom"),
            crash_entry(50, "ui loop stalled for 21s"),
        ];
        let rows = hang_rows(&entries);
        assert_eq!(rows.len(), 2, "the panic is not a hang");
        assert_eq!(rows[0].message, "ui loop stalled for 21s");
        assert_eq!(rows[0].date, 50_000_000);
        assert_eq!(rows[1].message, "ui loop stalled for 25s");
        assert_eq!(rows[1].date, 200_000_000);
        assert_eq!(rows[0].level, Level::Fault);
        assert_eq!(rows[0].process, PROCESS);
    }

    #[test]
    fn by_day_groups_crashes_and_hangs_by_utc_day() {
        // day 0: crashes at 0 and 86399, a hang of 21 s at 100
        // day 1: a crash at 86400, a hang of 4 s at 86500
        // day 3: a crash at 3 * 86400 + 5
        let days = by_day(&[0, 86_399, 86_400, 3 * 86_400 + 5], &[(100, 21), (86_500, 4)]);
        assert_eq!(
            days,
            vec![
                Day { day: 0, crashes: 2, hangs: 1, stalled_secs: 21 },
                Day { day: 1, crashes: 1, hangs: 1, stalled_secs: 4 },
                Day { day: 3, crashes: 1, hangs: 0, stalled_secs: 0 },
            ]
        );
    }

    #[test]
    fn a_time_before_the_epoch_falls_on_the_day_before() {
        let days = by_day(&[-1], &[]);
        assert_eq!(days, vec![Day { day: -1, crashes: 1, hangs: 0, stalled_secs: 0 }]);
    }

    #[test]
    fn the_summary_names_only_the_parts_that_happened() {
        let both = Day { day: 0, crashes: 2, hangs: 1, stalled_secs: 21 };
        assert_eq!(summary(&both), "2 crashes, 1 hang, 21 s stalled");
        let one_crash = Day { day: 0, crashes: 1, hangs: 0, stalled_secs: 0 };
        assert_eq!(summary(&one_crash), "1 crash");
        let two_hangs = Day { day: 0, crashes: 0, hangs: 2, stalled_secs: 40 };
        assert_eq!(summary(&two_hangs), "2 hangs, 40 s stalled");
    }

    #[test]
    fn daily_rows_start_at_midnight_utc_and_take_their_level_from_the_crashes() {
        let rows = daily_rows(&[86_400], &[(86_500, 4)]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].date, 86_400_000_000);
        assert_eq!(rows[0].level, Level::Fault);
        assert_eq!(rows[0].message, "1 crash, 1 hang, 4 s stalled");

        let hang_only = daily_rows(&[], &[(0, 20)]);
        assert_eq!(hang_only[0].level, Level::Error);
        assert_eq!(hang_only[0].message, "1 hang, 20 s stalled");
    }
}
