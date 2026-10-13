//! Import: copies the pictures of a folder (a memory card, a USB stick or
//! another folder) into the Photos library folder. Originals are copied, never
//! moved. A picture whose name and size the library already has is skipped;
//! a name the library uses for a different file gets a numbered copy, as
//! Save to Files does. Hidden files and folders are left out.

use crate::cg::imageio::is_image_name;
use crate::net::share::create_unique;
use std::io;
use std::path::Path;

/// How deep the import looks below the source folder.
const MAX_DEPTH: usize = 6;

/// What one import did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    pub copied: usize,
    pub skipped: usize,
    pub failed: usize,
}

/// Copies every picture under `from` into `library`, creating the library folder when it is missing.
pub fn import(from: &Path, library: &Path) -> io::Result<Report> {
    std::fs::create_dir_all(library)?;
    let mut report = Report::default();
    walk(from, library, 0, &mut report);
    Ok(report)
}

fn walk(dir: &Path, library: &Path, depth: usize, report: &mut Report) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        report.failed += 1;
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = e.path();
        if path.is_dir() {
            if depth + 1 < MAX_DEPTH {
                walk(&path, library, depth + 1, report);
            }
        } else if is_image_name(&name) {
            match copy_one(&path, &name, library) {
                Ok(true) => report.copied += 1,
                Ok(false) => report.skipped += 1,
                Err(_) => report.failed += 1,
            }
        }
    }
}

/// Copies one picture into the library. False when the library already holds a file with the same name and size.
fn copy_one(path: &Path, name: &str, library: &Path) -> io::Result<bool> {
    let size = std::fs::metadata(path)?.len();
    if std::fs::metadata(library.join(name)).is_ok_and(|m| m.len() == size) {
        return Ok(false);
    }
    let (dest, mut out) = create_unique(library, name)?;
    if let Err(e) = io::copy(&mut std::fs::File::open(path)?, &mut out) {
        let _ = std::fs::remove_file(&dest);
        return Err(e);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::photos::library::tests::scratch;

    #[test]
    fn copies_pictures_recursively_and_skips_what_the_library_holds() {
        let dir = scratch("import");
        let card = dir.join("card");
        let library = dir.join("Pictures");
        std::fs::create_dir_all(card.join("DCIM/100")).unwrap();
        std::fs::create_dir_all(&library).unwrap();
        std::fs::write(card.join("DCIM/100/IMG_1.JPG"), b"one").unwrap();
        std::fs::write(card.join("notes.txt"), b"not a picture").unwrap();
        std::fs::write(card.join(".hidden.jpg"), b"hidden").unwrap();
        std::fs::write(library.join("IMG_1.JPG"), b"one").unwrap();

        // the same name and size is already in the library
        assert_eq!(import(&card, &library).unwrap(), Report { copied: 0, skipped: 1, failed: 0 });

        std::fs::write(card.join("DCIM/100/IMG_2.png"), b"two two").unwrap();
        assert_eq!(import(&card, &library).unwrap(), Report { copied: 1, skipped: 1, failed: 0 });
        assert_eq!(std::fs::read(library.join("IMG_2.png")).unwrap(), b"two two");
        assert!(card.join("DCIM/100/IMG_2.png").exists(), "originals are copied, not moved");
        assert!(!library.join(".hidden.jpg").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_the_library_uses_for_another_file_gets_a_numbered_copy() {
        let dir = scratch("import-taken");
        let card = dir.join("card");
        let library = dir.join("Pictures");
        std::fs::create_dir_all(&card).unwrap();
        std::fs::create_dir_all(&library).unwrap();
        std::fs::write(library.join("IMG_3.jpg"), b"library").unwrap();
        std::fs::write(card.join("IMG_3.jpg"), b"card picture").unwrap();
        assert_eq!(import(&card, &library).unwrap().copied, 1);
        assert_eq!(std::fs::read(library.join("IMG_3.jpg")).unwrap(), b"library", "nothing is overwritten");
        assert_eq!(std::fs::read(library.join("IMG_3 2.jpg")).unwrap(), b"card picture");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_source_is_reported_not_panicked() {
        let dir = scratch("import-missing");
        let report = import(&dir.join("no-such-card"), &dir.join("Pictures")).unwrap();
        assert_eq!(report, Report { copied: 0, skipped: 0, failed: 1 });
        let _ = std::fs::remove_dir_all(&dir);
    }
}
