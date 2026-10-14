//! `ditto`: copies a file or folder tree, and writes or extracts zip archives.
//!
//! A copy keeps symbolic links as links and the permission bits of every file and folder. When the destination is an
//! existing folder, the source is copied into it under its own name; otherwise the copy takes the destination's name.
//! Archives go through the `zip` and `unzip` programs (Arch packages `zip` and `unzip`): `-c -k` writes a zip and
//! `-x -k` extracts one. `--keepParent` keeps the source folder's name as the top folder of the archive.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const USAGE: &str = "usage: ditto SOURCE DESTINATION\n       ditto -c -k [--keepParent] SOURCE ARCHIVE.zip\n       ditto -x -k ARCHIVE.zip DESTINATION";

/// What the command line asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    Help,
    Copy { src: PathBuf, dst: PathBuf },
    Zip { src: PathBuf, dst: PathBuf, keep_parent: bool },
    Unzip { archive: PathBuf, dst: PathBuf },
}

/// A program, its arguments and the folder it runs in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    pub program: &'static str,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
}

/// Reads the arguments (without the program name).
pub fn parse(args: &[String]) -> Result<Cmd, String> {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        return Ok(Cmd::Help);
    }
    let (mut create, mut extract, mut zip_kind, mut keep_parent) = (false, false, false, false);
    let mut positional: Vec<&String> = Vec::new();
    for arg in args {
        match arg.as_str() {
            "-c" => create = true,
            "-x" => extract = true,
            "-k" => zip_kind = true,
            "--keepParent" => keep_parent = true,
            "--sequesterRsrc" => {}
            a if a.starts_with('-') && a.len() > 1 => return Err(format!("unknown option {a}")),
            _ => positional.push(arg),
        }
    }
    if create && extract {
        return Err("use either -c or -x, not both".to_string());
    }
    if (create || extract) && !zip_kind {
        return Err("only zip archives are supported; add -k".to_string());
    }
    if zip_kind && !(create || extract) {
        return Err("-k needs -c (make an archive) or -x (extract one)".to_string());
    }
    if keep_parent && !create {
        return Err("--keepParent needs -c -k".to_string());
    }
    let [src, dst] = positional.as_slice() else {
        return Err("needs a SOURCE and a DESTINATION".to_string());
    };
    let (src, dst) = (PathBuf::from(src), PathBuf::from(dst));
    if create {
        Ok(Cmd::Zip { src: absolute(&src)?, dst: absolute(&dst)?, keep_parent })
    } else if extract {
        Ok(Cmd::Unzip { archive: src, dst })
    } else {
        Ok(Cmd::Copy { src, dst })
    }
}

/// `path` made absolute against the current folder. The zip program runs inside the source folder, so its paths must be absolute.
fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::current_dir().map(|dir| dir.join(path)).map_err(|e| e.to_string())
}

/// The zip call that writes `src` into the archive `dst` (both absolute). Symbolic links are stored as links (`-y`).
pub fn zip_invocation(src: &Path, dst: &Path, keep_parent: bool) -> Result<Invocation, String> {
    let (cwd, item) = if keep_parent {
        let name = src.file_name().ok_or("the source has no name to keep")?;
        let parent = src.parent().ok_or("the source has no parent folder")?;
        (parent.to_path_buf(), name.to_string_lossy().into_owned())
    } else {
        (src.to_path_buf(), ".".to_string())
    };
    Ok(Invocation {
        program: "zip",
        args: vec!["-r".into(), "-q".into(), "-y".into(), dst.to_string_lossy().into_owned(), item],
        cwd: Some(cwd),
    })
}

/// The unzip call that extracts `archive` into `dst`, replacing files without asking.
pub fn unzip_invocation(archive: &Path, dst: &Path) -> Invocation {
    Invocation {
        program: "unzip",
        args: vec![
            "-q".into(),
            "-o".into(),
            archive.to_string_lossy().into_owned(),
            "-d".into(),
            dst.to_string_lossy().into_owned(),
        ],
        cwd: None,
    }
}

/// Where a copy of `src` lands for the destination `dst`: inside `dst` when it is an existing folder, else `dst` itself.
pub fn copy_target(src: &Path, dst: &Path) -> PathBuf {
    match (dst.is_dir(), src.file_name()) {
        (true, Some(name)) => dst.join(name),
        _ => dst.to_path_buf(),
    }
}

/// Copies `src` to the place `copy_target` gives for `dst`.
pub fn copy_into(src: &Path, dst: &Path) -> io::Result<()> {
    copy_tree(src, &copy_target(src, dst))
}

fn copy_tree(src: &Path, dst: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(src)?;
    let file_type = meta.file_type();
    if file_type.is_symlink() {
        let target = fs::read_link(src)?;
        if fs::symlink_metadata(dst).is_ok() {
            fs::remove_file(dst)?;
        }
        return std::os::unix::fs::symlink(target, dst);
    }
    if file_type.is_dir() {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_tree(&entry.path(), &dst.join(entry.file_name()))?;
        }
        return fs::set_permissions(dst, meta.permissions());
    }
    fs::copy(src, dst).map(|_| ())
}

/// Runs the command line. Help is printed by the binary, so it does nothing here.
pub fn run(cmd: &Cmd) -> Result<(), String> {
    match cmd {
        Cmd::Help => Ok(()),
        Cmd::Copy { src, dst } => copy_into(src, dst).map_err(|e| format!("cannot copy {}: {e}", src.display())),
        Cmd::Zip { src, dst, keep_parent } => run_invocation(&zip_invocation(src, dst, *keep_parent)?),
        Cmd::Unzip { archive, dst } => {
            fs::create_dir_all(dst).map_err(|e| format!("cannot create {}: {e}", dst.display()))?;
            run_invocation(&unzip_invocation(archive, dst))
        }
    }
}

fn run_invocation(inv: &Invocation) -> Result<(), String> {
    let mut command = Command::new(inv.program);
    command.args(&inv.args);
    if let Some(cwd) = &inv.cwd {
        command.current_dir(cwd);
    }
    let status = command.status().map_err(|e| format!("cannot run {}: {e}", inv.program))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} failed ({status})", inv.program))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn plain_copy_takes_two_paths() {
        assert_eq!(
            parse(&args(&["a", "b"])),
            Ok(Cmd::Copy { src: "a".into(), dst: "b".into() })
        );
    }

    #[test]
    fn help_is_recognised() {
        assert_eq!(parse(&args(&["--help"])), Ok(Cmd::Help));
    }

    #[test]
    fn zip_with_keep_parent() {
        assert_eq!(
            parse(&args(&["-c", "-k", "--keepParent", "/work/photos", "/arch/out.zip"])),
            Ok(Cmd::Zip { src: "/work/photos".into(), dst: "/arch/out.zip".into(), keep_parent: true })
        );
    }

    #[test]
    fn unzip_takes_archive_and_destination() {
        assert_eq!(
            parse(&args(&["-x", "-k", "a.zip", "out"])),
            Ok(Cmd::Unzip { archive: "a.zip".into(), dst: "out".into() })
        );
    }

    #[test]
    fn rejects_bad_combinations() {
        assert!(parse(&args(&["-c", "src", "/x/out.zip"])).is_err(), "archives need -k");
        assert!(parse(&args(&["-k", "a", "b"])).is_err(), "-k alone is not a mode");
        assert!(parse(&args(&["-c", "-x", "-k", "a", "b"])).is_err(), "-c and -x exclude each other");
        assert!(parse(&args(&["--keepParent", "a", "b"])).is_err(), "--keepParent needs -c -k");
        assert!(parse(&args(&["-z", "a", "b"])).is_err(), "unknown option");
        assert!(parse(&args(&["a"])).is_err(), "one path is not enough");
    }

    #[test]
    fn zip_runs_inside_source_without_keep_parent() {
        let inv = zip_invocation(Path::new("/work/photos"), Path::new("/arch/out.zip"), false).unwrap();
        assert_eq!(inv.program, "zip");
        assert_eq!(inv.args, vec!["-r", "-q", "-y", "/arch/out.zip", "."]);
        assert_eq!(inv.cwd, Some(PathBuf::from("/work/photos")));
    }

    #[test]
    fn zip_with_keep_parent_names_the_folder_from_its_parent() {
        let inv = zip_invocation(Path::new("/work/photos"), Path::new("/arch/out.zip"), true).unwrap();
        assert_eq!(inv.args, vec!["-r", "-q", "-y", "/arch/out.zip", "photos"]);
        assert_eq!(inv.cwd, Some(PathBuf::from("/work")));
    }

    #[test]
    fn zip_keep_parent_refuses_a_root_source() {
        assert!(zip_invocation(Path::new("/"), Path::new("/arch/out.zip"), true).is_err());
    }

    #[test]
    fn unzip_extracts_quietly_over_existing_files() {
        let inv = unzip_invocation(Path::new("a.zip"), Path::new("out"));
        assert_eq!(inv.program, "unzip");
        assert_eq!(inv.args, vec!["-q", "-o", "a.zip", "-d", "out"]);
        assert_eq!(inv.cwd, None);
    }

    #[test]
    fn copy_target_goes_inside_an_existing_folder() {
        let dir = std::env::temp_dir().join(format!("bifrost-ditto-target-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("existing")).unwrap();
        assert_eq!(copy_target(Path::new("/src/tree"), &dir.join("existing")), dir.join("existing").join("tree"));
        assert_eq!(copy_target(Path::new("/src/tree"), &dir.join("new")), dir.join("new"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn copy_keeps_contents_links_and_permissions() {
        let root = std::env::temp_dir().join(format!("bifrost-ditto-copy-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let src = root.join("tree");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.txt"), "hello").unwrap();
        fs::write(src.join("sub").join("b.txt"), "nested").unwrap();
        fs::set_permissions(src.join("a.txt"), fs::Permissions::from_mode(0o750)).unwrap();
        std::os::unix::fs::symlink("a.txt", src.join("link")).unwrap();

        let dst = root.join("copy");
        copy_into(&src, &dst).unwrap();

        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "hello");
        assert_eq!(fs::read_to_string(dst.join("sub").join("b.txt")).unwrap(), "nested");
        assert_eq!(fs::read_link(dst.join("link")).unwrap(), PathBuf::from("a.txt"));
        let mode = fs::metadata(dst.join("a.txt")).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o750);
        fs::remove_dir_all(&root).unwrap();
    }
}
