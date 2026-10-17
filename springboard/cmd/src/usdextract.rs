//! `usdextract`: unpacks a USDZ package (a zip archive of USD layers and the assets they reference) into a folder.
//!
//! The archive is read by the `unzip` program (Arch package `unzip`), the same call `ditto -x -k` makes. The package
//! must carry the `.usdz` extension, and the destination folder is created when it does not exist.

use crate::ditto::{unzip_invocation, Invocation};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const USAGE: &str = "usage: usdextract ARCHIVE.usdz DESTINATION";

/// What the command line asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    Help,
    Extract { archive: PathBuf, dst: PathBuf },
}

fn args_for(list: &[String]) -> Vec<&str> {
    list.iter().map(String::as_str).collect()
}

/// Reads the command line: `usdextract ARCHIVE.usdz DESTINATION`, or `-h`/`--help`.
pub fn parse(args: &[String]) -> Result<Cmd, String> {
    let args = args_for(args);
    if args.iter().any(|a| *a == "-h" || *a == "--help") {
        return Ok(Cmd::Help);
    }
    match args.as_slice() {
        [archive, dst] => {
            if !is_usdz(archive) {
                return Err(format!("{archive}: not a USDZ package (expected a .usdz file)"));
            }
            Ok(Cmd::Extract { archive: PathBuf::from(archive), dst: PathBuf::from(dst) })
        }
        _ => Err("expected an archive and a destination folder".to_string()),
    }
}

/// True when `path` names a USDZ package by its extension, in any letter case.
pub fn is_usdz(path: &str) -> bool {
    Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case("usdz"))
}

/// The unzip call that unpacks `archive` into `dst`.
pub fn invocation(archive: &Path, dst: &Path) -> Invocation {
    unzip_invocation(archive, dst)
}

/// Runs the command. An extraction creates the destination folder and fails when the archive is missing or unreadable.
pub fn run(cmd: &Cmd) -> Result<(), String> {
    match cmd {
        Cmd::Help => Ok(()),
        Cmd::Extract { archive, dst } => {
            if !archive.is_file() {
                return Err(format!("{}: no such file", archive.display()));
            }
            std::fs::create_dir_all(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
            let inv = invocation(archive, dst);
            let status = Command::new(inv.program)
                .args(&inv.args)
                .status()
                .map_err(|e| format!("unzip: {e}"))?;
            if status.success() {
                Ok(())
            } else {
                Err(format!("{}: could not be extracted", archive.display()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_archive_and_destination() {
        assert_eq!(
            parse(&args(&["scene.usdz", "out"])),
            Ok(Cmd::Extract { archive: "scene.usdz".into(), dst: "out".into() })
        );
    }

    #[test]
    fn accepts_the_extension_in_any_case_and_help() {
        assert!(is_usdz("MODEL.USDZ"));
        assert_eq!(parse(&args(&["--help"])), Ok(Cmd::Help));
        assert_eq!(parse(&args(&["-h"])), Ok(Cmd::Help));
    }

    #[test]
    fn rejects_other_files_and_bad_argument_counts() {
        assert!(parse(&args(&["scene.usd", "out"])).is_err());
        assert!(parse(&args(&["scene.usdz"])).is_err());
        assert!(parse(&args(&["a.usdz", "b", "c"])).is_err());
        assert!(parse(&[]).is_err());
    }

    #[test]
    fn the_call_is_unzip_into_the_destination() {
        let inv = invocation(Path::new("a.usdz"), Path::new("out"));
        assert_eq!(inv.program, "unzip");
        assert_eq!(inv.args, vec!["-q", "-o", "a.usdz", "-d", "out"]);
    }

    #[test]
    fn a_missing_archive_is_reported_before_unzip_runs() {
        let missing = Cmd::Extract { archive: "/nonexistent/none.usdz".into(), dst: "/nonexistent/out".into() };
        assert_eq!(run(&missing), Err("/nonexistent/none.usdz: no such file".to_string()));
    }
}
