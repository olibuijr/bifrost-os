//! `tmutil`: the command line for the snapshot backup on the backup disk (see `bifrost_core::snapshot_backup`).
//!
//! Subcommands: `status`, `startbackup`, `listbackups`, `restore [--at STAMP] SOURCE DEST` and `addexclusion` /
//! `removeexclusion PATH`. The backup store is a directory (`BIFROST_BACKUP_STORE`, else `DEFAULT_STORE`):
//! - `snapshots.txt`: one snapshot time (Unix seconds) per line, written by the backup writer;
//! - `exclusions.txt`: the excluded items, one absolute path per line (`backup_exclude::BackupExclusions`);
//! - `start-request`: written by `startbackup`, taken by the backup writer when the disk is attached;
//! - `snapshots/<stamp>/<absolute path without the leading slash>`: the copied files of each snapshot.
//!
//! Not yet supported: `startbackup --block`, per-path restore of folders into an existing destination, and
//! excluding by a rule other than an absolute path.

use bifrost_core::backup_exclude::{BackupError, BackupExclusions};
use bifrost_core::snapshot_backup as sb;
use std::path::{Component, Path, PathBuf};

/// The backup store when `BIFROST_BACKUP_STORE` is not set.
pub const DEFAULT_STORE: &str = "/var/lib/bifrost/backup";
/// The environment variable that overrides the backup store.
pub const STORE_ENV: &str = "BIFROST_BACKUP_STORE";
pub const USAGE: &str = "usage: tmutil status | startbackup | listbackups | restore [--at STAMP] SOURCE DEST | addexclusion PATH | removeexclusion PATH";

const SNAPSHOTS_FILE: &str = "snapshots.txt";
const EXCLUSIONS_FILE: &str = "exclusions.txt";
const START_REQUEST: &str = "start-request";
const SNAPSHOT_DIR: &str = "snapshots";

/// One parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Status,
    StartBackup,
    ListBackups,
    /// `at` is the snapshot time to restore from (the newest snapshot not after it); `None` means the newest one.
    Restore { at: Option<u64>, source: String, dest: String },
    AddExclusion(String),
    RemoveExclusion(String),
}

/// Reads the arguments (without the program name).
pub fn parse(args: &[String]) -> Result<Command, String> {
    let Some((verb, rest)) = args.split_first() else {
        return Err(USAGE.to_string());
    };
    match verb.as_str() {
        "-h" | "--help" | "help" => Ok(Command::Help),
        "status" => no_operands(rest, Command::Status),
        "startbackup" => no_operands(rest, Command::StartBackup),
        "listbackups" => no_operands(rest, Command::ListBackups),
        "restore" => parse_restore(rest),
        "addexclusion" => one_path(rest).map(Command::AddExclusion),
        "removeexclusion" => one_path(rest).map(Command::RemoveExclusion),
        other => Err(format!("unknown command '{other}'\n{USAGE}")),
    }
}

fn no_operands(rest: &[String], cmd: Command) -> Result<Command, String> {
    match rest.first() {
        None => Ok(cmd),
        Some(extra) => Err(format!("unexpected argument '{extra}'")),
    }
}

fn one_path(rest: &[String]) -> Result<String, String> {
    match rest {
        [path] => Ok(path.clone()),
        _ => Err(format!("expected one absolute path\n{USAGE}")),
    }
}

fn parse_restore(rest: &[String]) -> Result<Command, String> {
    let mut at = None;
    let mut operands = Vec::new();
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        if arg == "--at" {
            let value = it.next().ok_or("--at needs a snapshot name")?;
            let time = sb::parse_stamp(value).ok_or_else(|| format!("not a snapshot name: {value}"))?;
            at = Some(time);
        } else if arg.starts_with('-') {
            return Err(format!("unknown option '{arg}'"));
        } else {
            operands.push(arg.clone());
        }
    }
    let [source, dest]: [String; 2] = operands
        .try_into()
        .map_err(|_| format!("restore needs SOURCE and DEST\n{USAGE}"))?;
    Ok(Command::Restore { at, source, dest })
}

/// Runs a command against the store at `dir`, at Unix time `now`. Returns the text for standard output.
pub fn run(cmd: &Command, dir: &Path, now: u64) -> Result<String, String> {
    match cmd {
        Command::Help => Ok(format!("{USAGE}\n")),
        Command::Status => status(dir, now),
        Command::StartBackup => start_backup(dir, now),
        Command::ListBackups => list_backups(dir),
        Command::Restore { at, source, dest } => restore(dir, *at, source, dest),
        Command::AddExclusion(path) => set_exclusion(dir, path, true),
        Command::RemoveExclusion(path) => set_exclusion(dir, path, false),
    }
}

fn status(dir: &Path, now: u64) -> Result<String, String> {
    let times = read_times(dir)?;
    let last = times.last().copied();
    let next = if sb::is_due(last, now) { "now".to_string() } else { sb::stamp(sb::next_due(last, now)) };
    let excluded = read_exclusions(dir)?.len();
    let requested = if dir.join(START_REQUEST).exists() { "yes" } else { "no" };
    Ok(format!(
        "Backup disk: {}\nLatest backup: {}\nNext backup: {}\nSnapshots: {}\nExcluded items: {}\nBackup requested: {}\n",
        dir.display(),
        last.map_or_else(|| "none".to_string(), sb::stamp),
        next,
        times.len(),
        excluded,
        requested,
    ))
}

fn start_backup(dir: &Path, now: u64) -> Result<String, String> {
    if !dir.is_dir() {
        return Err(format!("no backup disk at {}", dir.display()));
    }
    std::fs::write(dir.join(START_REQUEST), format!("{now}\n")).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok("Backup requested. It starts when the backup disk is attached.\n".to_string())
}

fn list_backups(dir: &Path) -> Result<String, String> {
    let times = read_times(dir)?;
    Ok(times.iter().map(|&t| format!("{}\n", sb::stamp(t))).collect())
}

fn restore(dir: &Path, at: Option<u64>, source: &str, dest: &str) -> Result<String, String> {
    if !source.starts_with('/') {
        return Err(format!("{source} is not an absolute path"));
    }
    if Path::new(source).components().any(|c| c == Component::ParentDir) {
        return Err(format!("{source} may not contain '..'"));
    }
    let times = read_times(dir)?;
    let chosen = match at {
        Some(wanted) => sb::version_at(&times, wanted).ok_or_else(|| format!("no backup at or before {}", sb::stamp(wanted)))?,
        None => *times.last().ok_or("there are no backups yet")?,
    };
    let name = sb::stamp(chosen);
    let src = dir.join(SNAPSHOT_DIR).join(&name).join(source.trim_start_matches('/'));
    if std::fs::symlink_metadata(&src).is_err() {
        return Err(format!("{source} is not in the backup of {name}"));
    }
    let dst = Path::new(dest);
    if std::fs::symlink_metadata(dst).is_ok() {
        return Err(format!("{dest} already exists"));
    }
    copy_tree(&src, dst).map_err(|e| format!("{dest}: {e}"))?;
    Ok(format!("Restored {source} from {name} to {dest}\n"))
}

/// Copies a file, or a folder and everything under it. Symbolic links are copied as the files they point to.
fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(src)?.is_dir() {
        std::fs::create_dir(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_tree(&entry.path(), &dst.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(src, dst).map(|_| ())
    }
}

fn set_exclusion(dir: &Path, path: &str, excluded: bool) -> Result<String, String> {
    let mut set = read_exclusions(dir)?;
    set.set_excluded(Path::new(path), excluded).map_err(|e| match e {
        BackupError::NotAbsolute => format!("{path} is not an absolute path"),
        BackupError::LineBreak => format!("{path} contains a line break"),
    })?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(dir.join(EXCLUSIONS_FILE), set.to_lines()).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(if excluded {
        format!("Excluded {path} from backups\n")
    } else {
        format!("Included {path} in backups again\n")
    })
}

/// Reads a store file, treating a missing file as empty.
fn read_optional(path: &Path) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn read_times(dir: &Path) -> Result<Vec<u64>, String> {
    Ok(sb::parse_list(&read_optional(&dir.join(SNAPSHOTS_FILE))?))
}

fn read_exclusions(dir: &Path) -> Result<BackupExclusions, String> {
    let mut set = BackupExclusions::new();
    set.load_lines(&read_optional(&dir.join(EXCLUSIONS_FILE))?);
    Ok(set)
}

/// The default store directory, or the one named by `BIFROST_BACKUP_STORE`.
pub fn store_dir() -> PathBuf {
    std::env::var_os(STORE_ENV).map_or_else(|| PathBuf::from(DEFAULT_STORE), PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// An empty scratch store for one test.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tmutil-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Two snapshots at 08:00 and 09:00 UTC, each holding `/home/u/notes.txt` with different text.
    fn two_snapshots(dir: &Path) -> (u64, u64) {
        let t1 = sb::from_civil(2026, 10, 13, 8, 0, 0).unwrap();
        let t2 = sb::from_civil(2026, 10, 13, 9, 0, 0).unwrap();
        std::fs::write(dir.join(SNAPSHOTS_FILE), sb::to_list(&[t1, t2])).unwrap();
        for (t, text) in [(t1, "v1"), (t2, "v2")] {
            let notes = dir.join(SNAPSHOT_DIR).join(sb::stamp(t)).join("home/u");
            std::fs::create_dir_all(&notes).unwrap();
            std::fs::write(notes.join("notes.txt"), text).unwrap();
        }
        (t1, t2)
    }

    #[test]
    fn parses_each_subcommand() {
        assert_eq!(parse(&args(&["status"])), Ok(Command::Status));
        assert_eq!(parse(&args(&["startbackup"])), Ok(Command::StartBackup));
        assert_eq!(parse(&args(&["listbackups"])), Ok(Command::ListBackups));
        assert_eq!(parse(&args(&["-h"])), Ok(Command::Help));
        assert_eq!(
            parse(&args(&["addexclusion", "/home/u/cache"])),
            Ok(Command::AddExclusion("/home/u/cache".into()))
        );
        assert_eq!(
            parse(&args(&["removeexclusion", "/home/u/cache"])),
            Ok(Command::RemoveExclusion("/home/u/cache".into()))
        );
        assert_eq!(
            parse(&args(&["restore", "/home/u/a", "/tmp/out"])),
            Ok(Command::Restore { at: None, source: "/home/u/a".into(), dest: "/tmp/out".into() })
        );
        let at = sb::parse_stamp("2026-10-13-090000");
        assert_eq!(
            parse(&args(&["restore", "--at", "2026-10-13-090000", "/home/u/a", "/tmp/out"])),
            Ok(Command::Restore { at, source: "/home/u/a".into(), dest: "/tmp/out".into() })
        );
    }

    #[test]
    fn rejects_bad_command_lines() {
        assert!(parse(&[]).is_err());
        assert!(parse(&args(&["frobnicate"])).is_err());
        assert!(parse(&args(&["status", "now"])).is_err());
        assert!(parse(&args(&["addexclusion"])).is_err());
        assert!(parse(&args(&["addexclusion", "a", "b"])).is_err());
        assert!(parse(&args(&["restore", "/home/u/a"])).is_err());
        assert!(parse(&args(&["restore", "--at", "yesterday", "/a", "/b"])).is_err());
        assert!(parse(&args(&["restore", "--at"])).is_err());
        assert!(parse(&args(&["restore", "-x", "/a", "/b"])).is_err());
    }

    #[test]
    fn status_reports_an_empty_store() {
        let dir = scratch("status-empty");
        let text = run(&Command::Status, &dir, 1_000).unwrap();
        assert!(text.contains("Latest backup: none"), "{text}");
        assert!(text.contains("Next backup: now"), "{text}");
        assert!(text.contains("Snapshots: 0"), "{text}");
        assert!(text.contains("Backup requested: no"), "{text}");
    }

    #[test]
    fn status_reports_the_latest_and_next_backup() {
        let dir = scratch("status-latest");
        let (_, t2) = two_snapshots(&dir);
        let now = t2 + 600;
        let text = run(&Command::Status, &dir, now).unwrap();
        assert!(text.contains("Latest backup: 2026-10-13-090000"), "{text}");
        assert!(text.contains("Next backup: 2026-10-13-100000"), "{text}");
        assert!(text.contains("Snapshots: 2"), "{text}");
    }

    #[test]
    fn start_backup_writes_a_request_only_for_an_existing_disk() {
        let dir = scratch("start");
        assert!(run(&Command::StartBackup, &dir.join("missing"), 5).is_err());
        run(&Command::StartBackup, &dir, 5).unwrap();
        assert!(dir.join(START_REQUEST).exists());
        let text = run(&Command::Status, &dir, 5).unwrap();
        assert!(text.contains("Backup requested: yes"), "{text}");
    }

    #[test]
    fn list_backups_prints_each_snapshot_oldest_first() {
        let dir = scratch("list");
        two_snapshots(&dir);
        let text = run(&Command::ListBackups, &dir, 0).unwrap();
        assert_eq!(text, "2026-10-13-080000\n2026-10-13-090000\n");
    }

    #[test]
    fn restore_takes_the_newest_version_by_default() {
        let dir = scratch("restore-newest");
        two_snapshots(&dir);
        let out = dir.join("out.txt");
        let cmd = Command::Restore { at: None, source: "/home/u/notes.txt".into(), dest: out.display().to_string() };
        let text = run(&cmd, &dir, 0).unwrap();
        assert!(text.contains("from 2026-10-13-090000"), "{text}");
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "v2");
    }

    #[test]
    fn restore_picks_the_version_at_a_time() {
        let dir = scratch("restore-at");
        let (t1, _) = two_snapshots(&dir);
        let out = dir.join("old.txt");
        let at = Some(t1 + 1800);
        let cmd = Command::Restore { at, source: "/home/u/notes.txt".into(), dest: out.display().to_string() };
        run(&cmd, &dir, 0).unwrap();
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "v1");
    }

    #[test]
    fn restore_refuses_missing_versions_existing_destinations_and_parent_parts() {
        let dir = scratch("restore-refuse");
        let (t1, _) = two_snapshots(&dir);
        let out = dir.join("x.txt");
        let early = Command::Restore { at: Some(t1 - 1), source: "/home/u/notes.txt".into(), dest: out.display().to_string() };
        assert!(run(&early, &dir, 0).is_err());
        let missing = Command::Restore { at: None, source: "/home/u/nope.txt".into(), dest: out.display().to_string() };
        assert!(run(&missing, &dir, 0).unwrap_err().contains("not in the backup"));
        let up = Command::Restore { at: None, source: "/home/../etc/passwd".into(), dest: out.display().to_string() };
        assert!(run(&up, &dir, 0).is_err());
        let relative = Command::Restore { at: None, source: "home/u/notes.txt".into(), dest: out.display().to_string() };
        assert!(run(&relative, &dir, 0).is_err());
        std::fs::write(&out, "keep").unwrap();
        let exists = Command::Restore { at: None, source: "/home/u/notes.txt".into(), dest: out.display().to_string() };
        assert!(run(&exists, &dir, 0).unwrap_err().contains("already exists"));
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "keep");
    }

    #[test]
    fn exclusions_are_added_listed_and_removed() {
        let dir = scratch("exclusions");
        run(&Command::AddExclusion("/home/u/cache".into()), &dir, 0).unwrap();
        run(&Command::AddExclusion("/home/u/tmp".into()), &dir, 0).unwrap();
        let text = run(&Command::Status, &dir, 0).unwrap();
        assert!(text.contains("Excluded items: 2"), "{text}");
        run(&Command::RemoveExclusion("/home/u/cache".into()), &dir, 0).unwrap();
        let text = run(&Command::Status, &dir, 0).unwrap();
        assert!(text.contains("Excluded items: 1"), "{text}");
    }

    #[test]
    fn exclusions_refuse_relative_paths() {
        let dir = scratch("exclusions-relative");
        let err = run(&Command::AddExclusion("cache".into()), &dir, 0).unwrap_err();
        assert!(err.contains("not an absolute path"), "{err}");
    }
}
