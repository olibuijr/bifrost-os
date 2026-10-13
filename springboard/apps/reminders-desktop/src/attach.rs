//! Image attachments of reminders. A picture is copied into the `attachments` folder beside the reminders file as
//! `att-<time>-<n>.png` or `.jpg`, and the reminder keeps the file names. A picture is checked from its header before
//! it is stored or decoded: only PNG and JPEG, at most `MAX_BYTES` and `MAX_PIXELS`.
use bifrost_app::kit::Image;
use std::io;
use std::path::Path;

/// Largest accepted file.
pub const MAX_BYTES: usize = 12 * 1024 * 1024;
/// Largest accepted picture (width x height), about a 4000 x 3000 photo.
pub const MAX_PIXELS: usize = 12_000_000;
/// Pictures one reminder may carry.
pub const MAX_PER_REMINDER: usize = 4;

/// Width and height from a PNG or JPEG header, without decoding.
pub fn dimensions(bytes: &[u8]) -> Option<(usize, usize)> {
    if bytes.len() > 24 && bytes[..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] && &bytes[12..16] == b"IHDR" {
        let be = |i: usize| u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
        return Some((be(16), be(20)));
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        let mut p = 2;
        while p + 4 <= bytes.len() {
            if bytes[p] != 0xFF {
                return None;
            }
            let marker = bytes[p + 1];
            if marker == 0xFF {
                p += 1;
                continue;
            }
            if (0xD0..=0xD8).contains(&marker) || marker == 0x01 {
                p += 2;
                continue;
            }
            let len = u16::from_be_bytes([bytes[p + 2], bytes[p + 3]]) as usize;
            // a start-of-frame marker (not DHT, JPG or DAC) holds the size: height, then width
            if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
                if p + 9 > bytes.len() {
                    return None;
                }
                let h = u16::from_be_bytes([bytes[p + 5], bytes[p + 6]]) as usize;
                let w = u16::from_be_bytes([bytes[p + 7], bytes[p + 8]]) as usize;
                return Some((w, h));
            }
            p += 2 + len;
        }
    }
    None
}

/// True when a picture of these bytes may be stored and decoded.
pub fn acceptable(bytes: &[u8]) -> bool {
    bytes.len() <= MAX_BYTES && dimensions(bytes).is_some_and(|(w, h)| w > 0 && h > 0 && w.saturating_mul(h) <= MAX_PIXELS)
}

/// File extension for attachable bytes (`png` or `jpg`).
pub fn extension(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0xFF, 0xD8, ..] => Some("jpg"),
        [0x89, b'P', b'N', b'G', ..] => Some("png"),
        _ => None,
    }
}

/// Whether `name` is a file name this module writes: `att-<digits>-<digits>.png` or `.jpg`. The store keeps only such
/// names, so a damaged file cannot point outside the attachments folder.
pub fn valid_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("att-") else { return false };
    let Some((stem, ext)) = rest.rsplit_once('.') else { return false };
    let Some((time, n)) = stem.split_once('-') else { return false };
    matches!(ext, "png" | "jpg") && !time.is_empty() && !n.is_empty() && time.bytes().chain(n.bytes()).all(|b| b.is_ascii_digit()) && name.len() <= 48
}

/// Copies a picture into `dir` and returns its file name. The name is made from `stamp` (a time) and a counter that
/// skips names already taken.
pub fn store(dir: &Path, bytes: &[u8], stamp: i64) -> io::Result<String> {
    let ext = extension(bytes).filter(|_| acceptable(bytes)).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Unsupported or oversized image"))?;
    std::fs::create_dir_all(dir)?;
    let stamp = stamp.max(0);
    let name = (1u32..)
        .map(|n| format!("att-{stamp}-{n}.{ext}"))
        .find(|name| !dir.join(name).exists())
        .ok_or_else(|| io::Error::new(io::ErrorKind::AlreadyExists, "No free name"))?;
    springboard::shell::fsutil::write_atomic(&dir.join(&name), bytes)?;
    Ok(name)
}

/// Deletes a stored picture; a missing file or a name that is not ours is left alone.
pub fn remove(dir: &Path, name: &str) {
    if valid_name(name) {
        let _ = std::fs::remove_file(dir.join(name));
    }
}

/// The picture scaled to fit a `side` x `side` square, keeping its proportions; `None` when it is missing, refused or
/// cannot be decoded.
pub fn thumbnail(dir: &Path, name: &str, side: usize) -> Option<Image> {
    if !valid_name(name) {
        return None;
    }
    let bytes = std::fs::read(dir.join(name)).ok()?;
    if !acceptable(&bytes) {
        return None;
    }
    let img = bifrost_app::kit::cg::imageio::decode(&bytes)?;
    let (w, h) = fit(img.w, img.h, side);
    Some(img.resized(w, h))
}

/// The size of a `w` x `h` picture scaled to fit a square of `side`: never enlarged, at least one pixel each way.
pub fn fit(w: usize, h: usize, side: usize) -> (usize, usize) {
    if w == 0 || h == 0 {
        return (1, 1);
    }
    let scale = (side as f32 / w as f32).min(side as f32 / h as f32).min(1.0);
    (((w as f32 * scale).round() as usize).max(1), ((h as f32 * scale).round() as usize).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PNG signature and IHDR chunk header for a `w` x `h` picture (enough for the checks that read the header).
    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        b.extend(w.to_be_bytes());
        b.extend(h.to_be_bytes());
        b.extend([8, 6, 0, 0, 0]);
        b
    }

    /// A JPEG with a start-of-frame segment for a `w` x `h` picture.
    fn jpeg(w: u16, h: u16) -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC0, 0, 11, 8];
        b.extend(h.to_be_bytes());
        b.extend(w.to_be_bytes());
        b.extend([1, 1, 0x11, 0]);
        b
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("target/test-reminders-attach-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_size_is_read_from_the_png_or_jpeg_header() {
        assert_eq!(dimensions(&png(640, 480)), Some((640, 480)));
        assert_eq!(dimensions(&jpeg(1024, 768)), Some((1024, 768)));
        assert_eq!(dimensions(b"GIF89a not accepted"), None);
        assert_eq!(dimensions(&[]), None);
        assert_eq!(dimensions(&[0xFF, 0xD8, 0xFF]), None, "a cut-off JPEG");
    }

    #[test]
    fn a_picture_over_the_size_limits_is_refused_before_it_is_decoded() {
        assert!(acceptable(&png(4000, 3000)));
        assert!(!acceptable(&png(4001, 3000)), "over 12 million pixels");
        assert!(!acceptable(&png(0, 10)));
        assert!(!acceptable(&jpeg(65535, 65535)));
        let mut big = png(10, 10);
        big.resize(MAX_BYTES + 1, 0);
        assert!(!acceptable(&big), "over the byte limit");
        assert_eq!((extension(&png(1, 1)), extension(&jpeg(1, 1)), extension(b"BM..")), (Some("png"), Some("jpg"), None));
    }

    #[test]
    fn only_names_this_module_writes_are_valid() {
        for good in ["att-1791073380-1.png", "att-0-12.jpg"] {
            assert!(valid_name(good), "{good}");
        }
        let long = format!("att-{}-1.png", "9".repeat(60));
        for bad in ["", "att-.png", "att-1-.png", "att--1.png", "att-1-1.gif", "att-1-1", "../att-1-1.png", "att-1/2-1.png", "att-a-1.png", "x-1-1.png", long.as_str()] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn stored_pictures_get_unique_names_and_can_be_removed() {
        let dir = scratch("store");
        let a = store(&dir, &png(10, 10), 1_791_073_380).unwrap();
        let b = store(&dir, &png(20, 20), 1_791_073_380).unwrap();
        let c = store(&dir, &jpeg(30, 30), 1_791_073_380).unwrap();
        assert_eq!((a.as_str(), b.as_str(), c.as_str()), ("att-1791073380-1.png", "att-1791073380-2.png", "att-1791073380-1.jpg"));
        assert!(valid_name(&a) && valid_name(&c));
        assert_eq!(std::fs::read(dir.join(&b)).unwrap(), png(20, 20));
        assert!(store(&dir, b"not a picture", 1).is_err());
        assert!(store(&dir, &png(5000, 5000), 1).is_err(), "too many pixels");
        remove(&dir, &a);
        assert!(!dir.join(&a).exists());
        remove(&dir, "../outside.png");
        remove(&dir, "att-9-9.png");
        assert!(dir.join(&b).exists(), "the others stay");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_thumbnail_needs_a_stored_name_and_a_picture_that_decodes() {
        let dir = scratch("thumb");
        assert!(thumbnail(&dir, "att-1-1.png", 80).is_none(), "no such file");
        let name = store(&dir, &png(10, 10), 1).unwrap();
        assert!(thumbnail(&dir, &name, 80).is_none(), "a header alone does not decode");
        assert!(thumbnail(&dir, "../etc/passwd", 80).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_thumbnail_keeps_the_proportions_and_never_enlarges() {
        assert_eq!(fit(400, 200, 80), (80, 40));
        assert_eq!(fit(200, 400, 80), (40, 80));
        assert_eq!(fit(40, 20, 80), (40, 20));
        assert_eq!(fit(10000, 1, 80), (80, 1), "at least a pixel");
        assert_eq!(fit(0, 5, 80), (1, 1));
    }
}
