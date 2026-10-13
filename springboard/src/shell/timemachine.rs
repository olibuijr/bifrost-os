//! Time Machine-style snapshot backups: the pure model behind the Backups pane.
//!
//! Snapshots are read-only btrfs subvolume snapshots of `@home` and `@system`,
//! taken on a schedule and pruned by a tiered retention policy:
//! hourly for the last 24 hours, daily for the last 30 days, then weekly.
//! Everything here is a pure function of times and paths so it can be tested
//! without a disk. Creating, pruning and mounting snapshots are privileged and
//! go through privd (`backup.snapshot`, `backup.prune`); this module only builds
//! the requests and the restore paths, and never runs them.

use std::path::{Path, PathBuf};

pub const HOUR: u64 = 3600;
pub const DAY: u64 = 24 * HOUR;
pub const WEEK: u64 = 7 * DAY;
/// Hourly snapshots are kept for this long.
pub const HOURLY_WINDOW: u64 = DAY;
/// Daily snapshots are kept for this long; older ones are kept weekly.
pub const DAILY_WINDOW: u64 = 30 * DAY;

/// privd methods used by the engine.
pub const METHOD_SNAPSHOT: &str = "backup.snapshot";
pub const METHOD_PRUNE: &str = "backup.prune";

/// How often a snapshot is taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interval {
    Hourly,
    Daily,
    Weekly,
}

impl Interval {
    pub fn secs(self) -> u64 {
        match self {
            Interval::Hourly => HOUR,
            Interval::Daily => DAY,
            Interval::Weekly => WEEK,
        }
    }

    pub fn from_key(key: &str) -> Option<Interval> {
        match key {
            "hourly" => Some(Interval::Hourly),
            "daily" => Some(Interval::Daily),
            "weekly" => Some(Interval::Weekly),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Interval::Hourly => "hourly",
            Interval::Daily => "daily",
            Interval::Weekly => "weekly",
        }
    }
}

/// The schedule and the time of the last successful snapshot (unix seconds).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Schedule {
    pub interval: Interval,
    pub enabled: bool,
    pub last: Option<u64>,
}

impl Schedule {
    pub fn new(interval: Interval) -> Self {
        Schedule { interval, enabled: true, last: None }
    }

    /// When the next snapshot is due. `None` while backups are off.
    pub fn next_due(&self, now: u64) -> Option<u64> {
        if !self.enabled {
            return None;
        }
        Some(match self.last {
            // Never backed up: the first one is due now.
            None => now,
            Some(last) => last + self.interval.secs(),
        })
    }

    /// True when a snapshot should be taken at `now`.
    pub fn is_due(&self, now: u64) -> bool {
        match self.next_due(now) {
            Some(due) => now >= due,
            None => false,
        }
    }
}

/// Which snapshots to keep. Returns `(keep, prune)`, both sorted ascending.
///
/// Within each tier the newest snapshot of every bucket is kept: one per hour
/// (younger than 24 h), one per calendar day (younger than 30 days), and one
/// per week beyond that. Snapshots from the future are kept as they are newest.
pub fn retention(snapshots: &[u64], now: u64) -> (Vec<u64>, Vec<u64>) {
    let mut sorted: Vec<u64> = snapshots.to_vec();
    sorted.sort_unstable();
    sorted.dedup();

    // Newest snapshot per (tier, bucket) wins; iterate ascending so later ones overwrite.
    let mut newest: std::collections::BTreeMap<(u8, u64), u64> = std::collections::BTreeMap::new();
    for &t in &sorted {
        let age = now.saturating_sub(t);
        let key = if age < HOURLY_WINDOW {
            (0, t / HOUR)
        } else if age < DAILY_WINDOW {
            (1, t / DAY)
        } else {
            (2, t / WEEK)
        };
        newest.insert(key, t);
    }
    let keep: std::collections::BTreeSet<u64> = newest.values().copied().collect();
    let (mut kept, mut pruned) = (Vec::new(), Vec::new());
    for t in sorted {
        if keep.contains(&t) {
            kept.push(t);
        } else {
            pruned.push(t);
        }
    }
    (kept, pruned)
}

/// Paths and names that are never backed up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exclusions {
    pub paths: Vec<String>,
}

impl Default for Exclusions {
    fn default() -> Self {
        let paths = ["/tmp", "/var/tmp", "/var/cache", "/run", "/proc", "/sys", "/dev", "/swap"];
        Exclusions { paths: paths.iter().map(|s| s.to_string()).collect() }
    }
}

impl Exclusions {
    /// True when `path` is an excluded path or lies below one.
    pub fn is_excluded(&self, path: &str) -> bool {
        let path = path.trim_end_matches('/');
        self.paths.iter().any(|p| {
            let p = p.trim_end_matches('/');
            !p.is_empty() && (path == p || path.strip_prefix(p).is_some_and(|rest| rest.starts_with('/')))
        })
    }
}

/// Directory name of a snapshot taken at `time` (unix seconds).
pub fn snapshot_name(time: u64) -> String {
    format!("snapshot-{time}")
}

/// Parses a snapshot directory name back into its time.
pub fn parse_snapshot_name(name: &str) -> Option<u64> {
    name.strip_prefix("snapshot-")?.parse().ok()
}

/// The path of `rel` (a path inside the subvolume) as it was in the snapshot
/// taken at `time`, under `root` (the backup volume). Rejects absolute paths
/// and any `..` or empty component so a restore cannot leave the snapshot.
pub fn restore_path(root: &Path, time: u64, rel: &str) -> Result<PathBuf, String> {
    if rel.is_empty() || rel.starts_with('/') || rel.contains('\0') {
        return Err("path must be relative".into());
    }
    if rel.split('/').any(|c| c.is_empty() || c == "." || c == "..") {
        return Err("path has an empty or relative component".into());
    }
    Ok(root.join(snapshot_name(time)).join(rel))
}

/// The privd request that takes a snapshot of `subvolume` at `time`.
pub fn snapshot_request(subvolume: &str, time: u64) -> (&'static str, Vec<(&'static str, String)>) {
    (METHOD_SNAPSHOT, vec![("subvolume", subvolume.to_string()), ("name", snapshot_name(time))])
}

/// The privd request that deletes the snapshots listed in `prune`.
pub fn prune_request(subvolume: &str, prune: &[u64]) -> (&'static str, Vec<(&'static str, String)>) {
    let names: Vec<String> = prune.iter().map(|t| snapshot_name(*t)).collect();
    (METHOD_PRUNE, vec![("subvolume", subvolume.to_string()), ("names", names.join(","))])
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;

    #[test]
    fn interval_keys_round_trip() {
        for i in [Interval::Hourly, Interval::Daily, Interval::Weekly] {
            assert_eq!(Interval::from_key(i.key()), Some(i));
        }
        assert_eq!(Interval::from_key("monthly"), None);
    }

    #[test]
    fn first_backup_is_due_now_and_next_follows_the_interval() {
        let mut s = Schedule::new(Interval::Daily);
        assert_eq!(s.next_due(NOW), Some(NOW));
        assert!(s.is_due(NOW));
        s.last = Some(NOW);
        assert_eq!(s.next_due(NOW), Some(NOW + DAY));
        assert!(!s.is_due(NOW + DAY - 1));
        assert!(s.is_due(NOW + DAY));
    }

    #[test]
    fn disabled_schedule_is_never_due() {
        let mut s = Schedule::new(Interval::Hourly);
        s.enabled = false;
        assert_eq!(s.next_due(NOW), None);
        assert!(!s.is_due(NOW + 10 * WEEK));
    }

    #[test]
    fn retention_keeps_hourly_daily_and_weekly_tiers() {
        // Hourly snapshots every 10 minutes for the last 3 hours: one per hour survives.
        let mut snaps: Vec<u64> = (0..18).map(|i| NOW - i * 600).collect();
        let (keep, prune) = retention(&snaps, NOW);
        assert_eq!(keep.len() + prune.len(), snaps.len());
        let newest = *snaps.iter().max().unwrap();
        assert!(keep.contains(&newest));
        assert!(prune.len() > keep.len());

        // Daily snapshots for 60 days: 30 within the daily window plus weekly ones beyond.
        snaps = (0..60).map(|d| NOW - d * DAY - 2 * HOUR).collect();
        let (keep, _) = retention(&snaps, NOW);
        let recent = keep.iter().filter(|t| NOW - **t < DAILY_WINDOW).count();
        assert_eq!(recent, 30, "one per day inside 30 days");
        let old = keep.iter().filter(|t| NOW - **t >= DAILY_WINDOW).count();
        assert!(old >= 4 && old <= 5, "about one per week beyond 30 days, got {old}");
    }

    #[test]
    fn retention_never_prunes_the_only_snapshot() {
        let (keep, prune) = retention(&[NOW - 5 * WEEK], NOW);
        assert_eq!(keep, vec![NOW - 5 * WEEK]);
        assert!(prune.is_empty());
        assert_eq!(retention(&[], NOW), (vec![], vec![]));
    }

    #[test]
    fn exclusions_match_whole_components_only() {
        let ex = Exclusions::default();
        assert!(ex.is_excluded("/tmp"));
        assert!(ex.is_excluded("/tmp/x/y"));
        assert!(ex.is_excluded("/var/cache/pacman/pkg/"));
        assert!(!ex.is_excluded("/tmpfoo"));
        assert!(!ex.is_excluded("/home/user/Documents"));
    }

    #[test]
    fn snapshot_names_round_trip() {
        assert_eq!(parse_snapshot_name(&snapshot_name(NOW)), Some(NOW));
        assert_eq!(parse_snapshot_name("backup-1"), None);
    }

    #[test]
    fn restore_path_stays_inside_the_snapshot() {
        let root = Path::new("/mnt/backup");
        let p = restore_path(root, NOW, "Documents/a.txt").unwrap();
        assert_eq!(p, PathBuf::from(format!("/mnt/backup/snapshot-{NOW}/Documents/a.txt")));
        assert!(restore_path(root, NOW, "/etc/passwd").is_err());
        assert!(restore_path(root, NOW, "../x").is_err());
        assert!(restore_path(root, NOW, "a//b").is_err());
        assert!(restore_path(root, NOW, "").is_err());
    }

    #[test]
    fn requests_name_the_privd_methods() {
        let (m, params) = snapshot_request("@home", NOW);
        assert_eq!(m, METHOD_SNAPSHOT);
        assert_eq!(params[1].1, format!("snapshot-{NOW}"));
        let (m, params) = prune_request("@home", &[1, 2]);
        assert_eq!(m, METHOD_PRUNE);
        assert_eq!(params[1].1, "snapshot-1,snapshot-2");
    }
}
