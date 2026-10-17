//! The `snapshots` service: the hourly backup schedule and its retention, run by the shell. The model is
//! `bifrost_core::snapshot_backup` (when a snapshot is due, which ones retention keeps); a snapshot is one encrypted
//! archive of the backed-up folders, written by `shell::backup::back_up_now` into `<data folder>/backups`.
//!
//! The schedule runs only while the data folder sits on a disk other than the system disk (an attached external
//! disk). With nothing attached, no snapshot is taken and nothing is pruned. The check runs at most once a minute,
//! and a failed snapshot is retried on the next check, so a full disk never turns into a write on every frame.
//! Pruning deletes only `backup-<unix>.sbk` files that retention drops; nothing else in the folder is touched.
//!
//! Methods (no parameters):
//! - `status {}` answers `{attached, count, last, next_due, due}`: whether the disk is attached, how many snapshots
//!   the folder holds, the newest snapshot's unix time (null when none), when the next is due and whether one is due
//!   now.
//!
//! The service sends no events.
use super::{Event, Shell, SystemService};
use crate::shell::backup;
use bifrost_core::json::Json;
use bifrost_core::snapshot_backup::{is_due, next_due, retention_plan};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// The service name.
pub const SERVICE: &str = "snapshots";

/// Seconds between two checks of the schedule.
pub const CHECK_EVERY: u64 = 60;

/// The `snapshots` service.
#[derive(Default)]
pub struct Snapshots {
    /// Unix time of the next check. Zero checks at once.
    next_check: u64,
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Whether `dir` lies on a different disk than the root filesystem, which is how an attached external disk shows.
pub fn on_external_disk(dir: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(dir), std::fs::metadata("/")) {
        (Ok(here), Ok(root)) => here.is_dir() && here.dev() != root.dev(),
        _ => false,
    }
}

impl SystemService for Snapshots {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, shell: &mut Shell, _app_id: &str, method: &str, _params: &Json, _t: f64) -> Result<Json, String> {
        match method {
            "status" => {
                let dir = shell.data_dir.clone();
                let times = backup::archive_times(&dir);
                let last = times.iter().copied().max();
                let now = unix_now();
                let num = |n: u64| Json::Num(n as f64);
                Ok(Json::Obj(vec![
                    ("attached".into(), Json::Bool(on_external_disk(&dir))),
                    ("count".into(), num(times.len() as u64)),
                    ("last".into(), last.map_or(Json::Null, num)),
                    ("next_due".into(), num(next_due(last, now))),
                    ("due".into(), Json::Bool(is_due(last, now))),
                ]))
            }
            _ => Err(format!("snapshots has no method {method}")),
        }
    }

    fn tick(&mut self, shell: &mut Shell, _t: f64) -> Vec<Event> {
        let now = unix_now();
        if now < self.next_check {
            return Vec::new();
        }
        self.next_check = now + CHECK_EVERY;
        let dir = shell.data_dir.clone();
        if !on_external_disk(&dir) {
            return Vec::new();
        }
        let last = backup::archive_times(&dir).into_iter().max();
        if is_due(last, now) && backup::back_up_now(&dir).is_ok() {
            prune(&dir, unix_now());
        }
        Vec::new()
    }
}

/// Deletes the snapshots that retention at `now` drops. Returns how many were removed.
pub fn prune(dir: &Path, now: u64) -> usize {
    let times = backup::archive_times(dir);
    let (_keep, drop) = retention_plan(&times, now);
    drop.into_iter().filter(|&t| std::fs::remove_file(backup::backup_path(dir, t)).is_ok()).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::snapshot_backup::{DAY, INTERVAL};

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("sb-snap-svc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("backups")).unwrap();
        d
    }

    #[test]
    fn prune_removes_exactly_the_snapshots_retention_drops() {
        let dir = scratch("prune");
        let now = 100 * DAY;
        // three snapshots inside the last day (all kept) and two 40 days old (retention keeps one per week)
        let recent = [now - 60, now - INTERVAL / 2, now - 5];
        let old = [now - 40 * DAY, now - 40 * DAY - 60];
        let all: Vec<u64> = recent.iter().chain(old.iter()).copied().collect();
        for t in &all {
            std::fs::write(backup::backup_path(&dir, *t), b"x").unwrap();
        }
        std::fs::write(dir.join("backups").join("notes.txt"), b"keep").unwrap();
        let (_, expected_drop) = retention_plan(&all, now);
        let removed = prune(&dir, now);
        assert_eq!(removed, expected_drop.len());
        for t in &expected_drop {
            assert!(!backup::backup_path(&dir, *t).exists());
        }
        for t in recent {
            assert!(backup::backup_path(&dir, t).exists());
        }
        assert!(dir.join("backups").join("notes.txt").exists());
    }
}
