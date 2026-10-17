//! Repair Library (`LibraryRepair` parity: the agent that fixes a Photos library's derived data).
//!
//! The library is the pictures on disk (`library::scan`); everything else is derived and can be rebuilt: the
//! thumbnail cache (`thumbs::DiskCache`, one `<key>.raw` file per picture, the key hashing path, time and size).
//! `check` reads the library and the cache and lists what is wrong; `repair` fixes it:
//!
//! - a thumbnail for a picture that is gone or was edited (its key is not any picture's key now) is deleted;
//! - a thumbnail file that is not a whole RIMG image (cut short by a crash, wrong magic, size not w * h * 4) is deleted;
//! - a leftover `.tmp` from an interrupted thumbnail write is deleted;
//! - a picture file of zero bytes is moved to the library trash, not deleted, so the person can still see it.
//!
//! The pictures themselves are never rewritten. The thumbnails come back when the grid asks for them.

use super::library::{self, Photo, TRASH};
use std::path::{Path, PathBuf};

/// What `check` found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Pictures and movies in the library and its trash.
    pub photos: usize,
    /// Cached thumbnails whose picture is gone or changed.
    pub orphan_thumbs: Vec<PathBuf>,
    /// Cached thumbnails that are not a whole RIMG image.
    pub damaged_thumbs: Vec<PathBuf>,
    /// Interrupted thumbnail writes.
    pub leftovers: Vec<PathBuf>,
    /// Picture files with no content.
    pub empty_files: Vec<PathBuf>,
}

impl Report {
    /// Nothing to fix.
    pub fn is_clean(&self) -> bool {
        self.orphan_thumbs.is_empty() && self.damaged_thumbs.is_empty() && self.leftovers.is_empty() && self.empty_files.is_empty()
    }

    /// Items `repair` will touch.
    pub fn problems(&self) -> usize {
        self.orphan_thumbs.len() + self.damaged_thumbs.len() + self.leftovers.len() + self.empty_files.len()
    }

    /// One line for the progress sheet.
    pub fn summary(&self) -> String {
        if self.is_clean() {
            return format!("The library is fine ({} items).", self.photos);
        }
        format!(
            "{} items checked: {} stale thumbnails, {} damaged thumbnails, {} interrupted writes, {} empty files.",
            self.photos,
            self.orphan_thumbs.len(),
            self.damaged_thumbs.len(),
            self.leftovers.len(),
            self.empty_files.len()
        )
    }
}

/// True when `bytes` is a whole RIMG file: magic, width, height (little endian), then `w * h` four-byte pixels.
pub fn is_whole_thumbnail(bytes: &[u8]) -> bool {
    if bytes.len() < 12 || &bytes[..4] != b"RIMG" {
        return false;
    }
    let w = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as u64;
    let h = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as u64;
    w > 0 && h > 0 && 12 + w * h * 4 == bytes.len() as u64
}

/// The key a thumbnail file name carries (`<key>.raw`).
fn key_of(path: &Path) -> Option<String> {
    (path.extension()? == "raw").then(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))?
}

/// Reads the library under `pictures` (with `extra_dirs`) and the thumbnail cache in `cache_dir`. `offset` is the
/// zone the grid reads capture times in, so the keys match the ones the grid wrote.
pub fn check(pictures: &Path, extra_dirs: &[PathBuf], cache_dir: &Path, offset: i32) -> Report {
    let mut dirs = vec![pictures.to_path_buf()];
    dirs.extend(extra_dirs.iter().cloned());
    let mut photos = library::scan_at(&dirs, offset);
    photos.extend(library::scan_trash(pictures, offset));
    let live: std::collections::HashSet<String> = photos.iter().map(Photo::key).collect();

    let mut report = Report { photos: photos.len(), ..Report::default() };
    for p in &photos {
        if p.size == 0 && !p.path.starts_with(pictures.join(TRASH)) {
            report.empty_files.push(p.path.clone());
        }
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(cache_dir).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    files.sort();
    for path in files {
        if path.extension().is_some_and(|x| x == "tmp") {
            report.leftovers.push(path);
            continue;
        }
        let Some(key) = key_of(&path) else { continue };
        match std::fs::read(&path) {
            Ok(bytes) if is_whole_thumbnail(&bytes) => {
                if !live.contains(&key) {
                    report.orphan_thumbs.push(path);
                }
            }
            _ => report.damaged_thumbs.push(path),
        }
    }
    report.empty_files.sort();
    report
}

/// Fixes what `check` found; returns how many items were fixed. Items that cannot be fixed (a read-only cache) are
/// skipped and counted out, so a second `check` still lists them.
pub fn repair(report: &Report, pictures: &Path) -> usize {
    let mut fixed = 0;
    for path in report.orphan_thumbs.iter().chain(&report.damaged_thumbs).chain(&report.leftovers) {
        if std::fs::remove_file(path).is_ok() {
            fixed += 1;
        }
    }
    for path in &report.empty_files {
        if library::move_to(path, &pictures.join(TRASH)).is_ok() {
            fixed += 1;
        }
    }
    fixed
}

#[cfg(test)]
mod tests {
    use super::super::library::tests::{picture, scratch};
    use super::super::thumbs::{to_raw, DiskCache};
    use super::*;
    use crate::gfx::Image;

    fn solid() -> Image {
        Image { w: 2, h: 2, px: vec![7; 4] }
    }

    #[test]
    fn whole_thumbnails_are_told_from_cut_short_ones() {
        let good = to_raw(&solid());
        assert!(is_whole_thumbnail(&good));
        assert!(!is_whole_thumbnail(&good[..good.len() - 1]));
        assert!(!is_whole_thumbnail(b"RIMG"));
        let mut bad_magic = good.clone();
        bad_magic[0] = b'X';
        assert!(!is_whole_thumbnail(&bad_magic));
        let mut zero = good;
        zero[4..8].copy_from_slice(&0u32.to_le_bytes());
        assert!(!is_whole_thumbnail(&zero));
    }

    #[test]
    fn a_healthy_library_is_clean() {
        let dir = scratch("repair-clean");
        let pictures = dir.join("Pictures");
        picture(&pictures.join("a.png"), 1_790_000_000);
        let cache = DiskCache::new(dir.join("thumbs"), 100);
        let photo = library::scan(&[pictures.clone()]).remove(0);
        cache.write(&photo.key(), &solid());
        let report = check(&pictures, &[], cache.dir(), 0);
        assert!(report.is_clean(), "{report:?}");
        assert_eq!(report.photos, 1);
        assert!(report.summary().contains("fine"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_damaged_and_interrupted_thumbnails_are_found_and_removed() {
        let dir = scratch("repair-thumbs");
        let pictures = dir.join("Pictures");
        picture(&pictures.join("a.png"), 1_790_000_000);
        let cache = DiskCache::new(dir.join("thumbs"), 100);
        let photo = library::scan(&[pictures.clone()]).remove(0);
        cache.write(&photo.key(), &solid());
        cache.write("deadbeefdeadbeef", &solid()); // a picture that is gone
        std::fs::write(cache.path("0123456789abcdef"), b"RIMG\x02\0\0\0").unwrap(); // cut short
        std::fs::write(cache.dir().join("fedcba9876543210.tmp"), b"half").unwrap();

        let report = check(&pictures, &[], cache.dir(), 0);
        assert_eq!(report.orphan_thumbs, vec![cache.path("deadbeefdeadbeef")]);
        assert_eq!(report.damaged_thumbs, vec![cache.path("0123456789abcdef")]);
        assert_eq!(report.leftovers.len(), 1);
        assert_eq!(report.problems(), 3);
        assert!(!report.is_clean() && report.summary().contains("1 stale thumbnails"));

        assert_eq!(repair(&report, &pictures), 3);
        assert!(cache.read(&photo.key()).is_some(), "the live thumbnail stays");
        assert_eq!(cache.len(), 1);
        assert!(check(&pictures, &[], cache.dir(), 0).is_clean());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_picture_file_goes_to_the_trash_not_away() {
        let dir = scratch("repair-empty");
        let pictures = dir.join("Pictures");
        picture(&pictures.join("ok.png"), 1_790_000_000);
        std::fs::write(pictures.join("empty.png"), b"").unwrap();
        let report = check(&pictures, &[], &dir.join("no-cache"), 0);
        assert_eq!(report.empty_files, vec![pictures.join("empty.png")]);
        assert_eq!(repair(&report, &pictures), 1);
        assert!(!pictures.join("empty.png").exists());
        assert!(pictures.join(TRASH).join("empty.png").exists(), "kept in Recently Deleted");
        assert!(pictures.join("ok.png").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_thumbnail_of_a_trashed_picture_is_not_stale() {
        let dir = scratch("repair-trash");
        let pictures = dir.join("Pictures");
        picture(&pictures.join(TRASH).join("gone.png"), 1_790_000_000);
        let cache = DiskCache::new(dir.join("thumbs"), 100);
        let photo = library::scan_trash(&pictures, 0).remove(0);
        cache.write(&photo.key(), &solid());
        let report = check(&pictures, &[], cache.dir(), 0);
        assert!(report.is_clean(), "{report:?}");
        assert_eq!(report.photos, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
