//! Image attachments for notes: stored beside the note as `<id>-<n>.png|jpg`
//! and referenced from its Markdown as `![image](<file>)`. Decoding is
//! bounded: the picture's size is read from its header first and anything
//! over `MAX_PIXELS` or `MAX_BYTES` is refused before the decoder runs.

use super::folders;
use super::md;
use springboard::gfx::Image;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

/// Largest accepted file.
pub const MAX_BYTES: usize = 12 * 1024 * 1024;
/// Largest accepted picture (width x height), about a 4000 x 3000 photo.
pub const MAX_PIXELS: usize = 12_000_000;
/// Inline images are shown at most this tall (px).
pub const MAX_SHOWN_H: usize = 640;

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

/// True when a picture of these bytes may be decoded.
pub fn acceptable(bytes: &[u8]) -> bool {
    bytes.len() <= MAX_BYTES && dimensions(bytes).is_some_and(|(w, h)| w > 0 && h > 0 && w.saturating_mul(h) <= MAX_PIXELS)
}

/// The size to show a `w` x `h` picture at inside `max_w`: never upscaled,
/// aspect kept, at most `MAX_SHOWN_H` tall.
pub fn fit(w: usize, h: usize, max_w: usize) -> (usize, usize) {
    if w == 0 || h == 0 {
        return (1, 1);
    }
    let scale = (max_w as f32 / w as f32).min(MAX_SHOWN_H as f32 / h as f32).min(1.0);
    (((w as f32 * scale).round() as usize).max(1), ((h as f32 * scale).round() as usize).max(1))
}

/// Largest file Attach File accepts (any type).
pub const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;

/// Decodes (bounded) and scales to fit `max_w`.
pub fn decode_fitted(bytes: &[u8], max_w: usize) -> Option<Image> {
    if !acceptable(bytes) {
        return None;
    }
    let img = springboard::cg::imageio::decode(bytes)?;
    let (w, h) = fit(img.w, img.h, max_w);
    Some(img.resized(w, h))
}

/// File extension for attachable bytes (`png` or `jpg`).
pub fn extension(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0xFF, 0xD8, ..] => Some("jpg"),
        [0x89, b'P', b'N', b'G', ..] => Some("png"),
        _ => None,
    }
}

/// Stores `bytes` beside note `id` in `dir`; returns the file name.
pub fn import(dir: &Path, id: &str, bytes: &[u8]) -> io::Result<String> {
    let ext = extension(bytes).filter(|_| acceptable(bytes)).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Unsupported or oversized image"))?;
    std::fs::create_dir_all(dir)?;
    let name = folders::next_attachment(dir, id, ext);
    springboard::shell::fsutil::write_atomic(&dir.join(&name), bytes)?;
    Ok(name)
}

/// The stored extension for a file named `name`: its own extension in lower case when that is a plain word of 1 to 8
/// letters or digits, else `bin`.
pub fn extension_of(name: &str) -> String {
    let ext = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext,
        _ => "",
    };
    let plain = !ext.is_empty() && ext.len() <= 8 && ext.chars().all(|c| c.is_ascii_alphanumeric());
    if plain { ext.to_ascii_lowercase() } else { "bin".to_string() }
}

/// The name a file shows in the note: its base name, without the characters that would break the Markdown link.
pub fn display_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let clean: String = base.chars().filter(|c| !c.is_control() && !matches!(c, '[' | ']' | '(' | ')')).collect();
    let clean = clean.trim();
    if clean.is_empty() { "File".to_string() } else { clean.to_string() }
}

/// Stores a file of any type beside note `id` in `dir` and returns the stored file name. The name keeps the file's
/// extension, so the attachment is listed and moves with the note. Files over `MAX_FILE_BYTES` are refused.
pub fn import_file(dir: &Path, id: &str, name: &str, bytes: &[u8]) -> io::Result<String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "File is too large"));
    }
    std::fs::create_dir_all(dir)?;
    let stored = folders::next_attachment(dir, id, &extension_of(name));
    springboard::shell::fsutil::write_atomic(&dir.join(&stored), bytes)?;
    Ok(stored)
}

/// The Markdown link line for an attached file: its display name, linking to the stored file.
pub fn file_line(name: &str, stored: &str) -> String {
    format!("[{}]({stored})", display_name(name))
}

/// Decodes every image the note's text refers to and that exists in `dir`.
pub fn load_images(dir: &Path, text: &str, max_w: usize) -> HashMap<String, Image> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let Some(name) = md::image_ref(line) else { continue };
        if out.contains_key(name) {
            continue;
        }
        let Ok(bytes) = std::fs::read(dir.join(name)) else { continue };
        if let Some(img) = decode_fitted(&bytes, max_w) {
            out.insert(name.to_string(), img);
        }
    }
    out
}

/// The newest picture among `dirs` (searched one level deep).
pub fn latest_image<P: AsRef<std::path::Path>>(dirs: &[P]) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir.as_ref()) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if !springboard::cg::imageio::is_image_name(&e.file_name().to_string_lossy()) {
                continue;
            }
            let Ok(modified) = e.metadata().and_then(|m| m.modified()) else { continue };
            if best.as_ref().is_none_or(|(t, _)| modified > *t) {
                best = Some((modified, path));
            }
        }
    }
    best.map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_header(w: u32, h: u32) -> Vec<u8> {
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        b.extend(w.to_be_bytes());
        b.extend(h.to_be_bytes());
        b.extend([8, 6, 0, 0, 0]);
        b
    }

    fn jpeg_header(w: u16, h: u16) -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC0, 0, 11, 8];
        b.extend(h.to_be_bytes());
        b.extend(w.to_be_bytes());
        b.extend([1, 1, 0x11, 0]);
        b
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-notes-attach")).join(format!("{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn reads_sizes_from_headers() {
        assert_eq!(dimensions(&png_header(640, 480)), Some((640, 480)));
        assert_eq!(dimensions(&jpeg_header(4000, 3000)), Some((4000, 3000)));
        assert_eq!(dimensions(b"GIF89a"), None);
        assert_eq!(dimensions(&[0xFF, 0xD8, 0xFF]), None);
        assert_eq!(dimensions(&[]), None);
    }

    #[test]
    fn oversized_pictures_are_refused_before_decoding() {
        assert!(acceptable(&png_header(4000, 3000)));
        assert!(!acceptable(&png_header(8000, 6000)), "too many pixels");
        assert!(!acceptable(&png_header(0, 10)));
        assert!(!acceptable(&jpeg_header(65_535, 65_535)));
        let mut big = png_header(10, 10);
        big.resize(MAX_BYTES + 1, 0);
        assert!(!acceptable(&big), "too many bytes");
        assert!(decode_fitted(&png_header(9000, 9000), 800).is_none());
    }

    #[test]
    fn fit_keeps_aspect_and_never_upscales() {
        assert_eq!(fit(400, 200, 800), (400, 200));
        assert_eq!(fit(1600, 800, 800), (800, 400));
        assert_eq!(fit(500, 2000, 800), (160, 640), "tall pictures are capped");
        assert_eq!(fit(0, 5, 100), (1, 1));
    }

    #[test]
    fn import_stores_beside_the_note_and_refuses_junk() {
        let dir = scratch("import");
        let png = png_header(10, 10);
        assert_eq!(import(&dir, "100", &png).unwrap(), "100-1.png");
        assert_eq!(import(&dir, "100", &png).unwrap(), "100-2.png");
        assert_eq!(import(&dir, "100", &jpeg_header(10, 10)).unwrap(), "100-3.jpg");
        assert!(import(&dir, "100", b"not an image").is_err());
        assert!(import(&dir, "100", &png_header(9000, 9000)).is_err());
        assert_eq!(folders::attachments(&dir, "100").len(), 3);
        assert!(load_images(&dir, "x\n![image](100-1.png)\n![image](missing.png)", 800).is_empty(), "a header-only file does not decode");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_names_keep_a_safe_extension() {
        assert_eq!(extension_of("Report.PDF"), "pdf");
        assert_eq!(extension_of("archive.tar.gz"), "gz");
        assert_eq!(extension_of("README"), "bin");
        assert_eq!(extension_of(".hidden"), "bin");
        assert_eq!(extension_of("odd.ext/with slash"), "bin");
        assert_eq!(extension_of("long.abcdefghi"), "bin");
    }

    #[test]
    fn display_names_drop_paths_and_link_characters() {
        assert_eq!(display_name("/home/a/Plan [v2].pdf"), "Plan v2.pdf");
        assert_eq!(display_name("C:\\docs\\notes.txt"), "notes.txt");
        assert_eq!(display_name("   "), "File");
        assert_eq!(file_line("Plan (final).pdf", "100-2.pdf"), "[Plan final.pdf](100-2.pdf)");
    }

    #[test]
    fn any_file_is_stored_beside_the_note_and_listed() {
        let dir = scratch("file");
        let stored = import_file(&dir, "200", "Budget.xlsx", b"sheet").unwrap();
        assert_eq!(stored, "200-1.xlsx");
        assert_eq!(import_file(&dir, "200", "Photo.png", &png_header(4, 4)).unwrap(), "200-2.png");
        assert_eq!(std::fs::read(dir.join("200-1.xlsx")).unwrap(), b"sheet");
        assert_eq!(folders::attachments(&dir, "200"), vec!["200-1.xlsx".to_string(), "200-2.png".to_string()]);
        let big = vec![1u8; MAX_FILE_BYTES + 1];
        assert!(import_file(&dir, "200", "huge.bin", &big).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_the_newest_picture() {
        let dir = scratch("latest");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(latest_image(&[dir.to_str().unwrap(), "/nonexistent-dir"]), None);
        std::fs::write(dir.join("a.png"), b"x").unwrap();
        std::fs::write(dir.join("note.txt"), b"x").unwrap();
        let newer = dir.join("b.jpg");
        std::fs::write(&newer, b"x").unwrap();
        let f = std::fs::OpenOptions::new().write(true).open(&newer).unwrap();
        f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(100)).unwrap();
        assert_eq!(latest_image(&[dir.to_str().unwrap()]), Some(newer));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
