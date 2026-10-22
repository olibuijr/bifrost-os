//! Data container migration (`ContainerMigrationService` parity): an app's data directory records the format version it
//! was written with, and when the OS or the app moves to a newer format the matching steps run in order to bring the
//! directory up to date.
//!
//! The version lives in the [`VERSION_FILE`] at the root of the container. A container with no version file is format 0
//! (written before versioning). A [`Step`] upgrades the container from `from` to `to`; [`migrate`] follows the chain of
//! steps from the recorded version to the target. The version is written after each step succeeds, so a step that
//! fails leaves the container at the last good version and the next run resumes from there. A container newer than the
//! target is refused, never downgraded, because the older code cannot read a newer format.
//!
//! [`migrate_safely`] takes a copy-on-upgrade snapshot first (a sibling `<name>.premigrate` directory) and restores it
//! when a step fails, so a half-migrated container never reaches the app. [`migrate_all`] walks a containers root and
//! migrates every app directory, reporting each result; one broken container does not stop the others.
//!
//! Not here: the scheduling of the migration at app launch. The caller decides when to run these (the updater runs
//! [`migrate_all`] after an OS install, before the first launch of the new version).

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

/// The file at the root of a container that records its format version as decimal text.
pub const VERSION_FILE: &str = ".container-version";

/// One upgrade from format `from` to format `to` (always `to > from`).
pub struct Step {
    pub from: u32,
    pub to: u32,
    pub name: &'static str,
    pub run: fn(&Path) -> io::Result<()>,
}

#[derive(Debug)]
pub enum MigrationError {
    /// The container is newer than the target. It is left untouched.
    Downgrade { found: u32, target: u32 },
    /// No chain of steps leads from `from` to `to`.
    NoPath { from: u32, to: u32 },
    /// The version file exists but is not a decimal number.
    BadVersion(String),
    /// A step ran and failed. The container stays at the last version that succeeded.
    StepFailed { name: &'static str, message: String },
    Io(String),
}

impl fmt::Display for MigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MigrationError::Downgrade { found, target } => {
                write!(f, "container format {found} is newer than {target}; refusing to downgrade")
            }
            MigrationError::NoPath { from, to } => write!(f, "no migration path from format {from} to {to}"),
            MigrationError::BadVersion(text) => write!(f, "damaged version file: {text:?}"),
            MigrationError::StepFailed { name, message } => write!(f, "migration step {name} failed: {message}"),
            MigrationError::Io(message) => write!(f, "container i/o failed: {message}"),
        }
    }
}

/// What [`migrate`] did: the version it started from, the version it reached and how many steps ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    pub from: u32,
    pub to: u32,
    pub applied: usize,
}

/// The version recorded in `dir`. A missing file is format 0.
pub fn read_version(dir: &Path) -> Result<u32, MigrationError> {
    match fs::read_to_string(dir.join(VERSION_FILE)) {
        Ok(text) => {
            let trimmed = text.trim();
            trimmed.parse::<u32>().map_err(|_| MigrationError::BadVersion(trimmed.to_string()))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(MigrationError::Io(e.to_string())),
    }
}

/// Records `version` in `dir`, replacing the old file in one rename so a crash never leaves a half-written version.
pub fn write_version(dir: &Path, version: u32) -> Result<(), MigrationError> {
    let tmp = dir.join(format!("{VERSION_FILE}.tmp"));
    fs::write(&tmp, format!("{version}\n")).map_err(|e| MigrationError::Io(e.to_string()))?;
    fs::rename(&tmp, dir.join(VERSION_FILE)).map_err(|e| MigrationError::Io(e.to_string()))
}

/// The steps that take a container from `from` to `to`, in order. Each step must start where the previous one ended.
pub fn plan(steps: &[Step], from: u32, to: u32) -> Result<Vec<&Step>, MigrationError> {
    let mut chain = Vec::new();
    let mut at = from;
    while at < to {
        let next = steps
            .iter()
            .find(|s| s.from == at && s.to > at && s.to <= to)
            .ok_or(MigrationError::NoPath { from, to })?;
        chain.push(next);
        at = next.to;
    }
    Ok(chain)
}

/// Brings the container in `dir` up to `target`. Returns what was done. An up-to-date container is a no-op.
pub fn migrate(dir: &Path, steps: &[Step], target: u32) -> Result<Outcome, MigrationError> {
    let from = read_version(dir)?;
    if from > target {
        return Err(MigrationError::Downgrade { found: from, target });
    }
    let chain = plan(steps, from, target)?;
    let mut at = from;
    for step in &chain {
        (step.run)(dir).map_err(|e| MigrationError::StepFailed { name: step.name, message: e.to_string() })?;
        write_version(dir, step.to)?;
        at = step.to;
    }
    Ok(Outcome { from, to: at, applied: chain.len() })
}

/// The sibling directory that holds the snapshot of `dir` while it migrates.
pub fn snapshot_path(dir: &Path) -> std::path::PathBuf {
    let mut name = dir.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".premigrate");
    dir.with_file_name(name)
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Copies `dir` to its [`snapshot_path`], replacing an older snapshot. Returns the snapshot directory.
pub fn snapshot(dir: &Path) -> Result<std::path::PathBuf, MigrationError> {
    let snap = snapshot_path(dir);
    let ioe = |e: io::Error| MigrationError::Io(e.to_string());
    if snap.exists() {
        fs::remove_dir_all(&snap).map_err(ioe)?;
    }
    copy_tree(dir, &snap).map_err(ioe)?;
    Ok(snap)
}

/// Puts the snapshot back in place of `dir` and removes the snapshot.
pub fn restore(dir: &Path) -> Result<(), MigrationError> {
    let snap = snapshot_path(dir);
    let ioe = |e: io::Error| MigrationError::Io(e.to_string());
    if !snap.is_dir() {
        return Err(MigrationError::Io(format!("no snapshot at {}", snap.display())));
    }
    fs::remove_dir_all(dir).map_err(ioe)?;
    fs::rename(&snap, dir).map_err(ioe)
}

/// [`migrate`] with a snapshot: nothing runs for an up-to-date container (no snapshot is taken); otherwise the
/// container is copied first, and a failing step restores the copy, so the container is back exactly as it was. The
/// snapshot is removed after a success.
pub fn migrate_safely(dir: &Path, steps: &[Step], target: u32) -> Result<Outcome, MigrationError> {
    let from = read_version(dir)?;
    if from > target {
        return Err(MigrationError::Downgrade { found: from, target });
    }
    if plan(steps, from, target)?.is_empty() {
        return Ok(Outcome { from, to: from, applied: 0 });
    }
    snapshot(dir)?;
    match migrate(dir, steps, target) {
        Ok(out) => {
            let _ = fs::remove_dir_all(snapshot_path(dir));
            Ok(out)
        }
        Err(e) => {
            restore(dir)?;
            Err(e)
        }
    }
}

/// Migrates every container directory directly under `root` (snapshot directories and plain files are skipped).
/// Returns one result per container, sorted by directory name.
pub fn migrate_all(root: &Path, steps: &[Step], target: u32) -> Vec<(String, Result<Outcome, MigrationError>)> {
    let mut names: Vec<String> = match fs::read_dir(root) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| !n.ends_with(".premigrate"))
            .collect(),
        Err(_) => Vec::new(),
    };
    names.sort();
    names
        .into_iter()
        .map(|n| {
            let r = migrate_safely(&root.join(&n), steps, target);
            (n, r)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bifrost-container-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn mark_one(dir: &Path) -> io::Result<()> {
        fs::write(dir.join("one"), "1")
    }

    fn mark_two(dir: &Path) -> io::Result<()> {
        fs::write(dir.join("two"), "2")
    }

    fn broken(_dir: &Path) -> io::Result<()> {
        Err(io::Error::other("disk full"))
    }

    fn steps() -> Vec<Step> {
        vec![
            Step { from: 0, to: 1, name: "one", run: mark_one },
            Step { from: 1, to: 2, name: "two", run: mark_two },
        ]
    }

    #[test]
    fn a_container_without_a_version_file_is_format_zero() {
        let d = dir("no-version");
        assert_eq!(read_version(&d).unwrap(), 0);
    }

    #[test]
    fn a_written_version_reads_back() {
        let d = dir("roundtrip");
        write_version(&d, 7).unwrap();
        assert_eq!(read_version(&d).unwrap(), 7);
    }

    #[test]
    fn a_damaged_version_file_is_refused() {
        let d = dir("damaged");
        fs::write(d.join(VERSION_FILE), "two\n").unwrap();
        assert!(matches!(read_version(&d), Err(MigrationError::BadVersion(_))));
    }

    #[test]
    fn plan_follows_the_chain_and_refuses_a_gap() {
        let s = steps();
        let chain = plan(&s, 0, 2).unwrap();
        assert_eq!(chain.iter().map(|c| c.name).collect::<Vec<_>>(), ["one", "two"]);
        assert!(matches!(plan(&s, 0, 3), Err(MigrationError::NoPath { from: 0, to: 3 })));
        assert!(plan(&s, 2, 2).unwrap().is_empty());
    }

    #[test]
    fn migrate_runs_every_step_and_records_the_target() {
        let d = dir("migrate");
        let out = migrate(&d, &steps(), 2).unwrap();
        assert_eq!(out, Outcome { from: 0, to: 2, applied: 2 });
        assert!(d.join("one").exists() && d.join("two").exists());
        assert_eq!(read_version(&d).unwrap(), 2);
    }

    #[test]
    fn a_second_migrate_is_a_no_op() {
        let d = dir("idempotent");
        migrate(&d, &steps(), 2).unwrap();
        fs::remove_file(d.join("one")).unwrap();
        let out = migrate(&d, &steps(), 2).unwrap();
        assert_eq!(out.applied, 0);
        assert!(!d.join("one").exists());
    }

    #[test]
    fn a_failed_step_leaves_the_last_good_version_and_a_rerun_resumes() {
        let d = dir("resume");
        let failing = vec![
            Step { from: 0, to: 1, name: "one", run: mark_one },
            Step { from: 1, to: 2, name: "broken", run: broken },
        ];
        let err = migrate(&d, &failing, 2).unwrap_err();
        assert!(matches!(err, MigrationError::StepFailed { name: "broken", .. }));
        assert_eq!(read_version(&d).unwrap(), 1);
        let out = migrate(&d, &steps(), 2).unwrap();
        assert_eq!(out, Outcome { from: 1, to: 2, applied: 1 });
    }

    #[test]
    fn a_newer_container_is_never_downgraded() {
        let d = dir("downgrade");
        write_version(&d, 5).unwrap();
        let err = migrate(&d, &steps(), 2).unwrap_err();
        assert!(matches!(err, MigrationError::Downgrade { found: 5, target: 2 }));
        assert!(!d.join("one").exists());
        assert_eq!(read_version(&d).unwrap(), 5);
    }

    #[test]
    fn the_error_text_names_the_step() {
        let text = MigrationError::StepFailed { name: "two", message: "disk full".into() }.to_string();
        assert!(text.contains("two") && text.contains("disk full"));
    }

    fn touch_and_fail(dir: &Path) -> io::Result<()> {
        fs::write(dir.join("half-written"), "x")?;
        Err(io::Error::other("disk full"))
    }

    #[test]
    fn a_failing_step_restores_the_snapshot_exactly() {
        let d = dir("safe-fail");
        fs::write(d.join("data"), "keep").unwrap();
        let failing = vec![Step { from: 0, to: 1, name: "bad", run: touch_and_fail }];
        let err = migrate_safely(&d, &failing, 1).unwrap_err();
        assert!(matches!(err, MigrationError::StepFailed { name: "bad", .. }));
        assert!(!d.join("half-written").exists());
        assert_eq!(fs::read_to_string(d.join("data")).unwrap(), "keep");
        assert_eq!(read_version(&d).unwrap(), 0);
        assert!(!snapshot_path(&d).exists());
    }

    #[test]
    fn a_successful_safe_migration_removes_the_snapshot() {
        let d = dir("safe-ok");
        let out = migrate_safely(&d, &steps(), 2).unwrap();
        assert_eq!(out.applied, 2);
        assert!(!snapshot_path(&d).exists());
        assert_eq!(read_version(&d).unwrap(), 2);
    }

    #[test]
    fn an_up_to_date_container_takes_no_snapshot() {
        let d = dir("safe-noop");
        write_version(&d, 2).unwrap();
        let out = migrate_safely(&d, &steps(), 2).unwrap();
        assert_eq!(out.applied, 0);
        assert!(!snapshot_path(&d).exists());
    }

    #[test]
    fn the_snapshot_copies_nested_directories() {
        let d = dir("snap-nested");
        fs::create_dir_all(d.join("a/b")).unwrap();
        fs::write(d.join("a/b/c"), "deep").unwrap();
        let snap = snapshot(&d).unwrap();
        assert_eq!(fs::read_to_string(snap.join("a/b/c")).unwrap(), "deep");
        let _ = fs::remove_dir_all(snap);
    }

    #[test]
    fn restore_without_a_snapshot_is_an_error() {
        let d = dir("restore-none");
        assert!(matches!(restore(&d), Err(MigrationError::Io(_))));
    }

    #[test]
    fn migrate_all_reports_each_container_and_one_failure_does_not_stop_the_rest() {
        let root = dir("all");
        for n in ["alpha", "beta", "gamma"] {
            fs::create_dir_all(root.join(n)).unwrap();
        }
        write_version(&root.join("beta"), 9).unwrap(); // newer than the target
        fs::write(root.join("not-a-container"), "file").unwrap();
        fs::create_dir_all(root.join("alpha.premigrate")).unwrap();
        let results = migrate_all(&root, &steps(), 2);
        let names: Vec<&str> = results.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["alpha", "beta", "gamma"]);
        assert!(results[0].1.is_ok());
        assert!(matches!(results[1].1, Err(MigrationError::Downgrade { found: 9, target: 2 })));
        assert_eq!(results[2].1.as_ref().unwrap().applied, 2);
    }

    #[test]
    fn migrate_all_of_a_missing_root_is_empty() {
        let root = std::env::temp_dir().join("bifrost-container-missing-root-xyz");
        assert!(migrate_all(&root, &steps(), 2).is_empty());
    }
}
