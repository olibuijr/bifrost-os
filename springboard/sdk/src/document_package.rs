//! Folder-package documents (`NativeDocument` packages): a document whose on-disk form is a directory that the file
//! panels and Files treat as one file. The package holds a main document file (for RTFD, `TXT.rtf`) and the files it
//! refers to (attachments such as images), all at the top level of the directory.
//!
//! Which directories are packages is decided by the extension alone (`is_package_name`), so Files and the panels can
//! tell a package from a folder without opening it. An app declares the package types it reads and writes with the
//! manifest key `package_types = ["rtfd"]`.
//!
//! Saving is atomic: the new contents are written to a sibling temporary directory, which then replaces the package by
//! renaming. A save that fails part way leaves the old package in place.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The package types the system knows: a lower-case extension and the uniform type identifier of the package.
pub const PACKAGE_TYPES: &[(&str, &str)] = &[("rtfd", "public.rtfd")];

/// The largest total size of a package read (all its files together), so a corrupt or hostile folder cannot exhaust memory.
pub const MAX_PACKAGE_BYTES: u64 = 64 * 1024 * 1024;

/// The uniform type identifier of the package type a name carries, if its extension is a package extension.
pub fn package_type_for_name(name: &str) -> Option<&'static str> {
    let (_, ext) = name.rsplit_once('.')?;
    if ext.is_empty() {
        return None;
    }
    let ext = ext.to_ascii_lowercase();
    PACKAGE_TYPES.iter().find(|(e, _)| *e == ext).map(|(_, uti)| *uti)
}

/// Whether a file name (or a folder name) is a package of a known type.
pub fn is_package_name(name: &str) -> bool {
    package_type_for_name(name).is_some()
}

/// Whether a name is safe to use as one file inside a package: a plain leaf name, not empty, not `.` or `..`, and no
/// separator.
pub fn is_leaf_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

/// Reads a package directory: every regular file at its top level, sorted by name. Subdirectories and links are
/// skipped, as a package has no folders of its own. Refuses a package whose files exceed `MAX_PACKAGE_BYTES`.
pub fn read_package_dir(dir: &Path) -> io::Result<Vec<(String, Vec<u8>)>> {
    let mut names: Vec<String> = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let meta = fs::symlink_metadata(entry.path())?;
        if !meta.is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else { continue };
        if is_leaf_name(&name) {
            names.push(name);
        }
    }
    names.sort();
    let mut total = 0u64;
    let mut files = Vec::with_capacity(names.len());
    for name in names {
        let bytes = fs::read(dir.join(&name))?;
        total += bytes.len() as u64;
        if total > MAX_PACKAGE_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "the package is too large to open"));
        }
        files.push((name, bytes));
    }
    Ok(files)
}

/// A name no other temporary or old package in `parent` uses: `.<name>.<tag>-<pid>-<nanos>`.
fn sibling_name(name: &str, tag: &str) -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!(".{name}.{tag}-{}-{nanos}", std::process::id())
}

/// Writes a package directory atomically: the files go into a temporary sibling directory first, which then takes
/// the package's place. The old package (if any) is moved aside and removed only after the new one is in place, and
/// is moved back if putting the new one in place fails. Each name must be a leaf name.
pub fn write_package_dir(dir: &Path, files: &[(&str, &[u8])]) -> io::Result<()> {
    let name = dir.file_name().and_then(|n| n.to_str()).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the package has no name"))?;
    let parent: PathBuf = match dir.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    if let Some((bad, _)) = files.iter().find(|(n, _)| !is_leaf_name(n)) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("\"{bad}\" cannot be a file in a package")));
    }
    let staging = parent.join(sibling_name(name, "saving"));
    fs::create_dir(&staging)?;
    let result = (|| -> io::Result<()> {
        for (file, bytes) in files {
            let mut f = fs::File::create(staging.join(file))?;
            f.write_all(bytes)?;
            f.sync_all()?;
        }
        Ok(())
    })();
    if let Err(err) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(err);
    }
    if !dir.exists() {
        return match fs::rename(&staging, dir) {
            Ok(()) => Ok(()),
            Err(err) => {
                let _ = fs::remove_dir_all(&staging);
                Err(err)
            }
        };
    }
    let old = parent.join(sibling_name(name, "replaced"));
    fs::rename(dir, &old)?;
    if let Err(err) = fs::rename(&staging, dir) {
        let _ = fs::rename(&old, dir);
        let _ = fs::remove_dir_all(&staging);
        return Err(err);
    }
    let _ = fs::remove_dir_all(&old);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bifrost-package-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_extension_decides_what_a_package_is() {
        assert_eq!(package_type_for_name("Notes.rtfd"), Some("public.rtfd"));
        assert_eq!(package_type_for_name("NOTES.RTFD"), Some("public.rtfd"));
        assert_eq!(package_type_for_name("notes.rtf"), None);
        assert_eq!(package_type_for_name("rtfd"), None, "a name with no dot has no extension");
        assert_eq!(package_type_for_name("Folder."), None);
        assert!(is_package_name("a.b.rtfd"));
    }

    #[test]
    fn leaf_names_refuse_separators_and_parent_steps() {
        assert!(is_leaf_name("image.png"));
        assert!(!is_leaf_name(""));
        assert!(!is_leaf_name(".."));
        assert!(!is_leaf_name("a/b.png"));
        assert!(!is_leaf_name("a\\b.png"));
    }

    #[test]
    fn a_package_round_trips_through_its_directory() {
        let root = scratch("round");
        let dir = root.join("Letter.rtfd");
        let png = [0x89, b'P', b'N', b'G', 1, 2, 3];
        write_package_dir(&dir, &[("TXT.rtf", &b"{\\rtf1 x}"[..]), ("pic.png", &png[..])]).unwrap();
        let back = read_package_dir(&dir).unwrap();
        assert_eq!(back, vec![("TXT.rtf".to_string(), b"{\\rtf1 x}".to_vec()), ("pic.png".to_string(), png.to_vec())]);
        // rewriting replaces the old contents and leaves no staging or old directories behind
        write_package_dir(&dir, &[("TXT.rtf", &b"{\\rtf1 y}"[..])]).unwrap();
        assert_eq!(read_package_dir(&dir).unwrap(), vec![("TXT.rtf".to_string(), b"{\\rtf1 y}".to_vec())]);
        let leftovers: Vec<_> = fs::read_dir(&root).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(leftovers.len(), 1, "{leftovers:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_refused_file_name_leaves_the_old_package_alone() {
        let root = scratch("refuse");
        let dir = root.join("Keep.rtfd");
        write_package_dir(&dir, &[("TXT.rtf", &b"old"[..])]).unwrap();
        assert!(write_package_dir(&dir, &[("TXT.rtf", &b"new"[..]), ("../escape", &b"x"[..])]).is_err());
        assert_eq!(read_package_dir(&dir).unwrap(), vec![("TXT.rtf".to_string(), b"old".to_vec())]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn subdirectories_are_not_read_as_package_files() {
        let root = scratch("subdir");
        let dir = root.join("Nested.rtfd");
        write_package_dir(&dir, &[("TXT.rtf", &b"t"[..])]).unwrap();
        fs::create_dir(dir.join("inner")).unwrap();
        let names: Vec<String> = read_package_dir(&dir).unwrap().into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, vec!["TXT.rtf".to_string()]);
        let _ = fs::remove_dir_all(&root);
    }
}
