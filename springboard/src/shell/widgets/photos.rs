//! The Photos widget: one thumbnail from the Photos app's cache, filling
//! the tile; a different one every minute.

use super::{Kind, RADIUS_PT};
use crate::deildu::art::cover;
use crate::gfx::{rrect_mask, Image};
use std::path::{Path, PathBuf};

/// Where the Photos app keeps its thumbnails (`RIMG` files).
pub fn thumbs_dir() -> String {
    crate::shell::apphost::root().join("cache/thumbs").to_string_lossy().into_owned()
}

/// The thumbnail files in `dir`, sorted; empty when there is no cache.
pub fn thumbs(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut v: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|e| e == "raw")).collect();
    v.sort();
    v
}

/// The cached thumbnail of the picture the person chose for the widget (`key` is the thumbnail key the picker gave); None
/// when the key is blank or the file is gone. The file name follows `photos::thumbs::DiskCache::path`.
pub fn pinned(dir: &Path, key: &str) -> Option<PathBuf> {
    let safe: String = key.chars().filter(char::is_ascii_alphanumeric).collect();
    if safe.is_empty() {
        return None;
    }
    let path = dir.join(format!("{safe}.raw"));
    path.is_file().then_some(path)
}

/// The thumbnail to show at `unix`: a new pick every minute.
pub fn pick(thumbs: &[PathBuf], unix: i64) -> Option<&PathBuf> {
    if thumbs.is_empty() {
        return None;
    }
    let i = (unix.div_euclid(60) as u64).wrapping_mul(2_654_435_761) as usize % thumbs.len();
    thumbs.get(i)
}

pub fn key(path: Option<&Path>) -> String {
    format!("photos:{}", path.map(|p| p.display().to_string()).unwrap_or_default())
}

/// The thumbnail at `path` cropped to fill the tile; None if unreadable.
pub fn render(s: f32, path: &Path) -> Option<Image> {
    let img = Image::from_raw(&std::fs::read(path).ok()?)?;
    if img.w == 0 || img.h == 0 {
        return None;
    }
    let (w, h) = Kind::Photos.size(s);
    let mut out = cover(&img, w, h);
    out.apply_mask(&rrect_mask(w, h, RADIUS_PT * s));
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::pack;

    fn raw(w: u32, h: u32, color: u32) -> Vec<u8> {
        let mut b = b"RIMG".to_vec();
        b.extend(w.to_le_bytes());
        b.extend(h.to_le_bytes());
        for _ in 0..w * h {
            b.extend(color.to_le_bytes());
        }
        b
    }

    #[test]
    fn lists_picks_and_renders_thumbnails() {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-photos-widget")).join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&dir);
        assert!(thumbs(&dir).is_empty(), "missing cache");
        assert_eq!(pick(&[], 0), None);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.raw"), raw(4, 6, pack(10, 20, 30, 255))).unwrap();
        std::fs::write(dir.join("a.raw"), raw(6, 4, pack(1, 2, 3, 255))).unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        let t = thumbs(&dir);
        assert_eq!(t.len(), 2);
        assert!(t[0].ends_with("a.raw"));
        let a = pick(&t, 0).unwrap();
        assert_eq!(pick(&t, 59), Some(a), "same pick within the minute");
        let picks: std::collections::HashSet<_> = (0..20).map(|m| pick(&t, m * 60).unwrap()).collect();
        assert_eq!(picks.len(), 2, "both thumbnails come up over time");
        let img = render(1.5625, &t[0]).unwrap();
        assert_eq!((img.w, img.h), Kind::Photos.size(1.5625));
        assert_eq!(img.px[img.w * img.h / 2 + img.w / 2], pack(1, 2, 3, 255));
        assert_eq!(img.px[0] >> 24, 0, "rounded corner");
        assert_ne!(key(Some(&t[0])), key(None));
        std::fs::write(dir.join("c.raw"), b"junk").unwrap();
        assert!(render(1.5625, &dir.join("c.raw")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pinned_picture_is_found_by_its_key_and_only_while_its_thumbnail_exists() {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-photos-widget-pin")).join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("0123abcd.raw"), raw(2, 2, pack(9, 9, 9, 255))).unwrap();
        assert_eq!(pinned(&dir, "0123abcd"), Some(dir.join("0123abcd.raw")));
        assert_eq!(pinned(&dir, "../0123abcd"), Some(dir.join("0123abcd.raw")), "a key is reduced to letters and digits");
        assert_eq!(pinned(&dir, "ffff"), None, "no thumbnail, nothing pinned");
        assert_eq!(pinned(&dir, ""), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
