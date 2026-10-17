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
//! Not here: the copy-on-upgrade snapshot and the scheduling of the migration at app launch. The caller decides when to
//! run [`migrate`] (the updater runs it after an OS install, before the first launch of the new version).

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
}
