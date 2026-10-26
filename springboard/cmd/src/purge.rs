//! `purge`: flushes file buffers to disk and then drops the kernel's page cache, dentries and inodes, the way the reference desktop
//! `purge` empties the disk and memory caches. Linux does this by writing `3` to `drop_caches`, which needs root; the
//! write is refused with a message when the process is not allowed to make it.

use std::io;
use std::process::Command;

pub const USAGE: &str = "usage: purge";
/// The kernel knob that drops the page cache (1), dentries and inodes (2), or both (3).
pub const DROP_CACHES: &str = "/proc/sys/vm/drop_caches";
/// The value written to `DROP_CACHES`: page cache, dentries and inodes.
pub const DROP_ALL: &str = "3";

/// What the command line asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Help,
    Run,
}

/// Reads the arguments (without the program name). `purge` takes no options.
pub fn parse(args: &[String]) -> Result<Request, String> {
    match args {
        [] => Ok(Request::Run),
        [a] if a == "-h" || a == "--help" => Ok(Request::Help),
        [a, ..] => Err(format!("unknown option: {a}")),
    }
}

/// Flushes the file buffers, then writes `DROP_ALL` to the kernel knob. `write` is the knob writer (the real one is
/// `write_knob`); `sync` runs first, so dirty pages reach the disk before the caches are dropped.
pub fn run_with(sync: impl FnOnce() -> io::Result<()>, write: impl FnOnce(&str, &str) -> io::Result<()>) -> Result<(), String> {
    sync().map_err(|e| format!("cannot flush file buffers: {e}"))?;
    write(DROP_CACHES, DROP_ALL).map_err(|e| match e.kind() {
        io::ErrorKind::PermissionDenied => "permission denied: purge needs root".to_string(),
        _ => format!("cannot drop caches: {e}"),
    })
}

/// Runs `sync(1)`, the real flush.
pub fn sync_all() -> io::Result<()> {
    let status = Command::new("sync").status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("sync failed"))
    }
}

/// Writes `value` to the kernel knob at `path`.
pub fn write_knob(path: &str, value: &str) -> io::Result<()> {
    std::fs::write(path, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_takes_no_options() {
        assert_eq!(parse(&args(&[])), Ok(Request::Run));
        assert_eq!(parse(&args(&["-h"])), Ok(Request::Help));
        assert_eq!(parse(&args(&["--help"])), Ok(Request::Help));
        assert!(parse(&args(&["-x"])).unwrap_err().contains("-x"));
    }

    #[test]
    fn run_syncs_then_writes_drop_all() {
        let log = RefCell::new(Vec::new());
        let r = run_with(
            || {
                log.borrow_mut().push("sync".to_string());
                Ok(())
            },
            |p, v| {
                log.borrow_mut().push(format!("{p}={v}"));
                Ok(())
            },
        );
        assert_eq!(r, Ok(()));
        assert_eq!(*log.borrow(), vec!["sync".to_string(), format!("{DROP_CACHES}={DROP_ALL}")]);
    }

    #[test]
    fn run_reports_permission_denied_as_root_needed() {
        let r = run_with(|| Ok(()), |_, _| Err(io::Error::from(io::ErrorKind::PermissionDenied)));
        assert_eq!(r, Err("permission denied: purge needs root".to_string()));
    }

    #[test]
    fn run_stops_when_sync_fails() {
        let wrote = RefCell::new(false);
        let r = run_with(
            || Err(io::Error::other("disk gone")),
            |_, _| {
                *wrote.borrow_mut() = true;
                Ok(())
            },
        );
        assert!(r.unwrap_err().starts_with("cannot flush file buffers"));
        assert!(!*wrote.borrow());
    }
}
