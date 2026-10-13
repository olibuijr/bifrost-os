//! Hourly snapshot backups with retention, for an external disk (std only, no disk I/O).
//!
//! A snapshot is a copy of the backed-up folders taken at one instant and named by its UTC time, `YYYY-MM-DD-HHMMSS`.
//! While the backup disk is attached a snapshot is due every hour. Retention keeps every snapshot from the last 24 hours,
//! the newest snapshot of each UTC day for the last 30 days, and the newest snapshot of each UTC week after that.
//! Excluded items (`backup_exclude::BackupExclusions`) are never copied, so the writer and the restore copy only need the
//! times this module returns. Times are Unix seconds.

/// One day in Unix seconds.
pub const DAY: u64 = 86_400;
/// How often a snapshot is taken while the disk is attached.
pub const INTERVAL: u64 = 3_600;
/// Snapshots newer than this are all kept.
pub const HOURLY_WINDOW: u64 = DAY;
/// Snapshots newer than this (and older than `HOURLY_WINDOW`) keep one per day.
pub const DAILY_WINDOW: u64 = 30 * DAY;

/// The time of the next snapshot. `None` (never backed up) or an overdue one means it is due now.
pub fn next_due(last: Option<u64>, now: u64) -> u64 {
    last.map_or(now, |l| (l + INTERVAL).max(now))
}

/// Whether a snapshot should be taken at `now`, given the time of the last one.
pub fn is_due(last: Option<u64>, now: u64) -> bool {
    last.is_none_or( |l| now >= l + INTERVAL)
}

/// The newest snapshot taken at or before `wanted`, the version a restore picks.
pub fn version_at(times: &[u64], wanted: u64) -> Option<u64> {
    times.iter().copied().filter(|&t| t <= wanted).max()
}

/// Which snapshots retention keeps at `now`. Returns `(keep, drop)`, both sorted oldest first.
pub fn retention_plan(times: &[u64], now: u64) -> (Vec<u64>, Vec<u64>) {
    let mut sorted: Vec<u64> = times.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut keep = Vec::new();
    let mut drop = Vec::new();
    // Newest first, so the first snapshot seen in a day or week bucket is the one kept.
    let mut days_seen = std::collections::BTreeSet::new();
    let mut weeks_seen = std::collections::BTreeSet::new();
    for &t in sorted.iter().rev() {
        let age = now.saturating_sub(t);
        let kept = if age < HOURLY_WINDOW {
            true
        } else if age < DAILY_WINDOW {
            days_seen.insert(t / DAY)
        } else {
            weeks_seen.insert(t / (7 * DAY))
        };
        if kept {
            keep.push(t);
        } else {
            drop.push(t);
        }
    }
    keep.reverse();
    drop.reverse();
    (keep, drop)
}

/// The civil date and time (UTC) of a Unix time: `(year, month, day, hour, minute, second)`.
pub fn civil(t: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (t / DAY) as i64;
    let secs = t % DAY;
    // Howard Hinnant's civil-from-days algorithm (proleptic Gregorian calendar).
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = yoe as i64 + era * 400 + i64::from(m <= 2);
    (y, m, d, (secs / 3_600) as u32, (secs % 3_600 / 60) as u32, (secs % 60) as u32)
}

/// The Unix time of a UTC civil date and time, or `None` if the date is out of range or before 1970.
pub fn from_civil(y: i64, m: u32, d: u32, hh: u32, mm: u32, ss: u32) -> Option<u64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 59 {
        return None;
    }
    // Howard Hinnant's days-from-civil algorithm.
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = (y - era * 400) as u64;
    let mp = (if m > 2 { m - 3 } else { m + 9 }) as u64;
    let doy = (153 * mp + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe as i64 - 719_468;
    if days < 0 {
        return None;
    }
    Some(days as u64 * DAY + u64::from(hh) * 3_600 + u64::from(mm) * 60 + u64::from(ss))
}

/// The snapshot name of a Unix time: `YYYY-MM-DD-HHMMSS` in UTC.
pub fn stamp(t: u64) -> String {
    let (y, m, d, hh, mm, ss) = civil(t);
    format!("{y:04}-{m:02}-{d:02}-{hh:02}{mm:02}{ss:02}")
}

/// Reads a snapshot name back to its Unix time. Only names that `stamp` would produce are accepted.
pub fn parse_stamp(name: &str) -> Option<u64> {
    let parts: Vec<&str> = name.split('-').collect();
    if parts.len() != 4 || parts[3].len() != 6 || !parts[3].bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let y: i64 = parts[0].parse().ok()?;
    let m: u32 = parts[1].parse().ok()?;
    let d: u32 = parts[2].parse().ok()?;
    let hms: u32 = parts[3].parse().ok()?;
    let t = from_civil(y, m, d, hms / 10_000, hms / 100 % 100, hms % 100)?;
    (stamp(t) == name).then_some(t)
}

/// Snapshot times, one per line, sorted ascending. Lines that are not snapshot times are skipped.
pub fn parse_list(text: &str) -> Vec<u64> {
    let mut times: Vec<u64> = text.lines().filter_map(|l| l.trim().parse().ok()).collect();
    times.sort_unstable();
    times.dedup();
    times
}

/// The text form read by `parse_list`.
pub fn to_list(times: &[u64]) -> String {
    times.iter().map(|t| format!("{t}\n")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: u64 = 3_600;

    #[test]
    fn civil_round_trips_known_times() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        let t = from_civil(2026, 10, 13, 12, 34, 56).unwrap();
        assert_eq!(civil(t), (2026, 10, 13, 12, 34, 56));
        assert_eq!(from_civil(2000, 2, 29, 0, 0, 0).map(civil), Some((2000, 2, 29, 0, 0, 0)));
        assert_eq!(from_civil(1969, 12, 31, 23, 59, 59), None);
    }

    #[test]
    fn stamp_and_parse_are_inverse() {
        let t = from_civil(2026, 10, 13, 9, 5, 7).unwrap();
        assert_eq!(stamp(t), "2026-10-13-090507");
        assert_eq!(parse_stamp("2026-10-13-090507"), Some(t));
        assert_eq!(parse_stamp("2026-02-30-000000"), None);
        assert_eq!(parse_stamp("2026-10-13-09057"), None);
        assert_eq!(parse_stamp("2026-10-13-09x507"), None);
        assert_eq!(parse_stamp("26-10-13-090507"), None);
    }

    #[test]
    fn schedule_is_hourly() {
        assert!(is_due(None, 100));
        assert!(!is_due(Some(100), 100 + INTERVAL - 1));
        assert!(is_due(Some(100), 100 + INTERVAL));
        assert_eq!(next_due(Some(100), 200), 100 + INTERVAL);
        assert_eq!(next_due(Some(100), 100 + 5 * INTERVAL), 100 + 5 * INTERVAL);
        assert_eq!(next_due(None, 7), 7);
    }

    #[test]
    fn version_at_picks_newest_not_after_wanted() {
        let times = [10, 20, 30];
        assert_eq!(version_at(&times, 25), Some(20));
        assert_eq!(version_at(&times, 30), Some(30));
        assert_eq!(version_at(&times, 9), None);
        assert_eq!(version_at(&[], 9), None);
    }

    #[test]
    fn retention_keeps_all_recent_then_one_per_day() {
        // `now` is UTC midnight, so the offsets below stay within one calendar day.
        let now = 100 * DAY;
        let mut times: Vec<u64> = (0..24).map(|h| now - h * INTERVAL).collect();
        for d in 2..5u64 {
            times.push(now - d * DAY + 2 * HOUR);
            times.push(now - d * DAY + 5 * HOUR);
        }
        let (keep, drop) = retention_plan(&times, now);
        assert_eq!(keep.len() + drop.len(), times.len());
        for h in 0..24 {
            assert!(keep.contains(&(now - h * INTERVAL)));
        }
        let daily_kept: Vec<u64> = keep.iter().copied().filter(|&t| now - t >= HOURLY_WINDOW).collect();
        assert_eq!(daily_kept.len(), 3);
        assert_eq!(drop.len(), 3);
        assert!(drop.iter().all(|&t| now - t >= HOURLY_WINDOW));
        // The newer of each day's pair is the one kept.
        for d in 2..5u64 {
            assert!(keep.contains(&(now - d * DAY + 5 * HOUR)));
        }
    }

    #[test]
    fn retention_keeps_one_per_week_past_the_daily_window() {
        let now = 400 * DAY;
        let times: Vec<u64> = (0..10).map(|d| now - (100 + d) * DAY).collect();
        let (keep, drop) = retention_plan(&times, now);
        assert_eq!(keep.len() + drop.len(), 10);
        assert!(keep.len() <= 3, "kept {keep:?}");
        assert!(!keep.is_empty());
        assert!(keep.contains(times.iter().max().unwrap()));
    }

    #[test]
    fn list_round_trips() {
        let times = vec![5, 9, 42];
        assert_eq!(parse_list(&to_list(&times)), times);
        assert_eq!(parse_list("9\nbad\n\n5\n9\n"), vec![5, 9]);
    }
}
