//! What the preview panel shows beyond the plain picture, text and page views, as pure helpers over bytes and files:
//! the listing of an archive, the information line under a picture (size, format, camera, date, exposure), and the
//! frame clock of an animated picture (GIF or animated PNG). The panel (`preview_panel.rs`) draws the results.

use crate::cg::imageio::{png, Format, ImageSource, PropValue, Properties};
use crate::gfx::Image;
use std::io::Read;
use std::path::Path;
use std::time::Instant;

/// An archive listing shows at most this many entries (the panel cuts it to the card's height anyway).
pub const MAX_LISTING_ENTRIES: usize = 400;
/// A tar file is read this far for its listing; entries past it are not named.
pub const TAR_SCAN_BYTES: u64 = 8 << 20;
/// Characters of one name kept in a listing line.
const NAME_CHARS: usize = 100;
/// A picture's header and metadata are read from this many bytes at the start of the file.
pub const INFO_HEAD_BYTES: u64 = 256 * 1024;
/// A picture file larger than this is not decoded for animation.
pub const MAX_ANIMATION_BYTES: u64 = 16 * 1024 * 1024;
/// An animation keeps at most this many frames, and at most this many pixels over all of them.
pub const MAX_ANIMATION_FRAMES: usize = 300;
pub const MAX_ANIMATION_PIXELS: usize = 32 << 20;

// ---------------------------------------------------------------- archives

/// One entry of an archive: its name, its size in bytes and whether it is a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
}

/// Why an archive has no listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListError {
    /// A format the panel cannot list (compressed streams, 7-Zip, RAR).
    Unsupported,
    /// The file cannot be read or is damaged.
    Unreadable,
}

/// The entries of a ustar or plain tar from its first bytes: 512-byte headers, each followed by its padded data. A
/// header that is not valid ends the walk, so a damaged file lists what came before. `None` when the first block is
/// not a tar header at all.
pub fn tar_entries(head: &[u8]) -> Option<Vec<Entry>> {
    if head.get(257..262)? != b"ustar" {
        return None;
    }
    let mut out = Vec::new();
    let mut pos = 0usize;
    while let Some(h) = head.get(pos..pos + 512) {
        if h.iter().all(|&b| b == 0) {
            break;
        }
        let text = |a: usize, b: usize| String::from_utf8_lossy(&h[a..b]).trim_end_matches('\0').to_string();
        let Ok(size) = u64::from_str_radix(text(124, 135).trim(), 8) else { break };
        let (name, prefix) = (text(0, 100), text(345, 500));
        let full = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        let is_dir = h[156] == b'5' || full.ends_with('/');
        // long-name and extended-header records describe the next entry and are not entries themselves
        if !matches!(h[156], b'L' | b'K' | b'x' | b'g') {
            out.push(Entry { name: full, size, is_dir });
        }
        pos = pos.saturating_add(512).saturating_add(usize::try_from(size.div_ceil(512).saturating_mul(512)).unwrap_or(usize::MAX));
    }
    Some(out)
}

/// The entries of the archive at `path`: a ZIP by its central directory, a tar by its headers.
pub fn archive_entries(path: &Path) -> Result<Vec<Entry>, ListError> {
    let mut file = std::fs::File::open(path).map_err(|_| ListError::Unreadable)?;
    let mut magic = [0u8; 4];
    let n = file.read(&mut magic).map_err(|_| ListError::Unreadable)?;
    if magic[..n].starts_with(b"PK") {
        let file = std::fs::File::open(path).map_err(|_| ListError::Unreadable)?;
        let archive = crate::files::zip::Archive::open(file).map_err(|_| ListError::Unreadable)?;
        return Ok(archive.members.iter().map(|m| Entry { name: m.name.clone(), size: m.size, is_dir: m.is_dir }).collect());
    }
    let mut head = magic[..n].to_vec();
    file.take(TAR_SCAN_BYTES).read_to_end(&mut head).map_err(|_| ListError::Unreadable)?;
    match tar_entries(&head) {
        Some(e) => Ok(e),
        // a tar has "ustar" at 257; any other content is a format with no listing here
        None if !head.is_empty() => Err(ListError::Unsupported),
        None => Err(ListError::Unreadable),
    }
}

/// A name with control characters dropped, cut to [`NAME_CHARS`] characters.
fn clean_name(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).take(NAME_CHARS).collect()
}

/// The lines of an archive listing: a summary ("12 items, 3.1 MB"), then one line per entry (name and size, a folder
/// with a trailing slash), at most [`MAX_LISTING_ENTRIES`] entries and a closing line for the rest.
pub fn listing_lines(entries: &[Entry]) -> Vec<String> {
    let files = entries.iter().filter(|e| !e.is_dir).count();
    let total: u64 = entries.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
    let item = |n: usize| if n == 1 { "1 item".to_string() } else { format!("{n} items") };
    let mut lines = vec![format!("{}, {} when unpacked", item(entries.len()), crate::files::fs::size_text(total))];
    if entries.is_empty() {
        lines[0] = "This archive is empty.".into();
        return lines;
    }
    for e in entries.iter().take(MAX_LISTING_ENTRIES) {
        let name = clean_name(&e.name);
        if e.is_dir {
            lines.push(if name.ends_with('/') { name } else { format!("{name}/") });
        } else {
            lines.push(format!("{name}  ({})", crate::files::fs::size_text(e.size)));
        }
    }
    if entries.len() > MAX_LISTING_ENTRIES {
        lines.push(format!("and {} more ({} files in all)", entries.len() - MAX_LISTING_ENTRIES, files));
    }
    lines
}

// ---------------------------------------------------------------- picture information

/// `1/250 s` for a short exposure, `2 s` or `2.5 s` for a long one.
pub fn exposure_text(seconds: f64) -> Option<String> {
    if !seconds.is_finite() || seconds <= 0.0 {
        return None;
    }
    if seconds < 0.95 {
        return Some(format!("1/{} s", (1.0 / seconds).round() as u64));
    }
    Some(if (seconds - seconds.round()).abs() < 0.05 { format!("{} s", seconds.round() as u64) } else { format!("{seconds:.1} s") })
}

fn text_of(d: &crate::cg::imageio::Dict, key: &str) -> Option<String> {
    d.get(key)?.as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// `2026:10:13 12:00:00` as `2026-10-13 12:00`. Text of another shape is returned as it is.
pub fn date_text(exif_date: &str) -> String {
    let b = exif_date.as_bytes();
    if exif_date.is_ascii() && b.len() >= 16 && b[4] == b':' && b[7] == b':' && b[10] == b' ' {
        return format!("{}-{}-{} {}", &exif_date[0..4], &exif_date[5..7], &exif_date[8..10], &exif_date[11..16]);
    }
    exif_date.to_string()
}

/// The parts of the line under a picture, in order: size in pixels, format, file size, frame count (animations), the
/// camera, the capture date, and the exposure (time, f-number, ISO, focal length). Parts the file does not have are
/// left out.
pub fn picture_info(p: &Properties, format: Format, file_bytes: u64, frames: usize) -> String {
    let mut parts = vec![format!("{} \u{D7} {}", p.pixel_width, p.pixel_height), format.extensions()[0].to_uppercase(), crate::files::fs::size_text(file_bytes)];
    if frames > 1 {
        parts.push(format!("{frames} frames"));
    }
    let (make, model) = (text_of(&p.tiff, "Make"), text_of(&p.tiff, "Model"));
    match (make, model) {
        (Some(mk), Some(md)) if md.to_lowercase().starts_with(&mk.to_lowercase()) => parts.push(md),
        (Some(mk), Some(md)) => parts.push(format!("{mk} {md}")),
        (None, Some(x)) | (Some(x), None) => parts.push(x),
        (None, None) => {}
    }
    if let Some(d) = text_of(&p.exif, "DateTimeOriginal").or_else(|| text_of(&p.tiff, "DateTime")) {
        parts.push(date_text(&d));
    }
    if let Some(t) = p.exif.get("ExposureTime").and_then(PropValue::as_f64).and_then(exposure_text) {
        parts.push(t);
    }
    if let Some(f) = p.exif.get("FNumber").and_then(PropValue::as_f64).filter(|f| *f > 0.0) {
        parts.push(format!("f/{}", trim_float(f)));
    }
    if let Some(i) = p.exif.get("ISOSpeedRatings").and_then(PropValue::as_i64).filter(|i| *i > 0) {
        parts.push(format!("ISO {i}"));
    }
    if let Some(mm) = p.exif.get("FocalLength").and_then(PropValue::as_f64).filter(|f| *f > 0.0) {
        parts.push(format!("{} mm", trim_float(mm)));
    }
    parts.join("  \u{B7}  ")
}

/// One decimal, dropped when it is zero: `2.8`, `4`.
fn trim_float(v: f64) -> String {
    let s = format!("{v:.1}");
    s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
}

/// The information line for the picture file at `path`, read from the first [`INFO_HEAD_BYTES`] of the file. `None`
/// when the file is not a picture the codecs read.
pub fn picture_info_for(path: &Path) -> Option<String> {
    let len = std::fs::metadata(path).ok()?.len();
    let mut head = Vec::new();
    std::fs::File::open(path).ok()?.take(INFO_HEAD_BYTES).read_to_end(&mut head).ok()?;
    let format = Format::detect(&head)?;
    let frames = match format {
        Format::Png => png::frame_count(&head),
        _ => 1,
    };
    let props = ImageSource::new(head).properties_at(0)?;
    Some(picture_info(&props, format, len, frames))
}

// ---------------------------------------------------------------- animated pictures

/// The index of the frame on screen `elapsed_ms` after an animation starts. Every delay of 10 ms or less shows for
/// 100 ms, the rule browsers apply. The animation plays once and then `loop_count` more times (`Some(0)` and `None`
/// repeat forever); after the last pass the last frame stays.
pub fn frame_index(delays_ms: &[u32], loop_count: Option<u32>, elapsed_ms: u64) -> usize {
    let shown: Vec<u64> = delays_ms.iter().map(|&d| if d <= 10 { 100 } else { u64::from(d) }).collect();
    let pass: u64 = shown.iter().sum();
    if pass == 0 {
        return 0;
    }
    let at = match loop_count {
        Some(repeats) if repeats > 0 && elapsed_ms >= pass.saturating_mul(u64::from(repeats) + 1) => return shown.len() - 1,
        _ => elapsed_ms % pass,
    };
    let mut end = 0u64;
    for (i, d) in shown.iter().enumerate() {
        end += d;
        if at < end {
            return i;
        }
    }
    shown.len() - 1
}

/// An animated picture open in the panel: its frames, their delays and the clock that picks the frame on screen.
pub struct PictureAnim {
    frames: Vec<Image>,
    delays_ms: Vec<u32>,
    loop_count: Option<u32>,
    started: Instant,
    shown: usize,
}

impl PictureAnim {
    /// An animation over `frames` with their `delays_ms`. `None` with fewer than two frames, or lists of unequal length.
    pub fn new(frames: Vec<Image>, delays_ms: Vec<u32>, loop_count: Option<u32>) -> Option<PictureAnim> {
        if frames.len() < 2 || frames.len() != delays_ms.len() {
            return None;
        }
        Some(PictureAnim { frames, delays_ms, loop_count, started: Instant::now(), shown: 0 })
    }

    /// Moves to the frame for the time now. True when the frame on screen changed.
    pub fn tick(&mut self) -> bool {
        self.tick_at(self.started.elapsed().as_millis() as u64)
    }

    /// Like [`PictureAnim::tick`] for a given elapsed time.
    pub fn tick_at(&mut self, elapsed_ms: u64) -> bool {
        let i = frame_index(&self.delays_ms, self.loop_count, elapsed_ms);
        std::mem::replace(&mut self.shown, i) != i
    }

    /// The frame on screen.
    pub fn frame(&self) -> &Image {
        &self.frames[self.shown]
    }

    pub fn count(&self) -> usize {
        self.frames.len()
    }
}

/// Reads the animation in the picture file at `path` when it has more than one frame: a GIF or an animated PNG, at
/// most [`MAX_ANIMATION_BYTES`], keeping at most [`MAX_ANIMATION_FRAMES`] frames of [`MAX_ANIMATION_PIXELS`] pixels
/// in all. `None` for a still picture, a file over the limits or a file that does not decode (the still preview
/// takes it).
pub fn load_animation(path: &Path) -> Option<PictureAnim> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(ext.as_str(), "gif" | "png" | "apng") || std::fs::metadata(path).ok()?.len() > MAX_ANIMATION_BYTES {
        return None;
    }
    if ext != "gif" {
        // an animated PNG says so before its first image data, so the head of the file is enough to tell
        let mut head = Vec::new();
        std::fs::File::open(path).ok()?.take(64 * 1024).read_to_end(&mut head).ok()?;
        if png::frame_count(&head) < 2 {
            return None;
        }
    }
    let src = ImageSource::new(std::fs::read(path).ok()?);
    let n = src.count();
    if !(2..=MAX_ANIMATION_FRAMES).contains(&n) {
        return None;
    }
    let (w, h) = src.pixel_size()?;
    if w.checked_mul(h)?.checked_mul(n)? > MAX_ANIMATION_PIXELS {
        return None;
    }
    let mut frames = Vec::with_capacity(n);
    let mut delays = Vec::with_capacity(n);
    for i in 0..n {
        frames.push(src.image_premultiplied(i)?);
        delays.push(src.delay_ms_at(i).unwrap_or(0));
    }
    PictureAnim::new(frames, delays, src.loop_count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg::imageio::encode::{encode_apng, ApngFrame, PngOptions};
    use crate::files::zip::Writer;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ql-info-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tar_block(name: &str, size: usize, kind: u8) -> Vec<u8> {
        let mut h = vec![0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[124..135].copy_from_slice(format!("{size:011o}").as_bytes());
        h[156] = kind;
        h[257..262].copy_from_slice(b"ustar");
        let mut t = h;
        t.extend(vec![7u8; size.div_ceil(512) * 512]);
        t
    }

    #[test]
    fn a_tar_lists_names_sizes_and_folders_and_skips_long_name_records() {
        let mut t = tar_block("docs/", 0, b'5');
        t.extend(tar_block("docs/a.txt", 600, b'0'));
        t.extend(tar_block("././@LongLink", 20, b'L'));
        t.extend(tar_block("b.bin", 3, b'0'));
        t.extend(vec![0u8; 1024]);
        let e = tar_entries(&t).unwrap();
        assert_eq!(e.len(), 3);
        assert_eq!((e[0].name.as_str(), e[0].is_dir), ("docs/", true));
        assert_eq!((e[1].name.as_str(), e[1].size), ("docs/a.txt", 600));
        assert_eq!(e[2].name, "b.bin");
        assert!(tar_entries(&[1u8; 600]).is_none(), "no ustar mark means no tar");
        assert!(tar_entries(b"short").is_none());
    }

    #[test]
    fn a_damaged_tar_lists_what_came_before_the_damage() {
        let mut t = tar_block("ok.txt", 10, b'0');
        let mut bad = tar_block("bad.txt", 10, b'0');
        bad[124..135].copy_from_slice(b"not-octal!!");
        t.extend(bad);
        let e = tar_entries(&t).unwrap();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].name, "ok.txt");
    }

    #[test]
    fn a_zip_on_disk_is_listed_through_its_central_directory() {
        let dir = scratch("zip");
        let mut w = Writer::new(std::fs::File::create(dir.join("a.zip")).unwrap());
        w.add_dir("src/", 0).unwrap();
        w.add_bytes("src/main.rs", b"fn main() {}", 0).unwrap();
        w.add_bytes("README", b"hello", 0).unwrap();
        w.finish().unwrap();
        let e = archive_entries(&dir.join("a.zip")).unwrap();
        assert_eq!(e.len(), 3);
        assert!(e[0].is_dir);
        assert_eq!((e[1].name.as_str(), e[1].size), ("src/main.rs", 12));
        let lines = listing_lines(&e);
        assert!(lines[0].starts_with("3 items, "), "{}", lines[0]);
        assert_eq!(lines[1], "src/");
        assert!(lines[2].starts_with("src/main.rs  ("));
        std::fs::write(dir.join("b.7z"), b"7z\xBC\xAF\x27\x1C and much more than that to be a long file").unwrap();
        assert_eq!(archive_entries(&dir.join("b.7z")), Err(ListError::Unsupported));
        assert_eq!(archive_entries(&dir.join("missing.zip")), Err(ListError::Unreadable));
        std::fs::write(dir.join("broken.zip"), b"PK\x03\x04 nothing else").unwrap();
        assert_eq!(archive_entries(&dir.join("broken.zip")), Err(ListError::Unreadable));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_long_listing_is_cut_with_a_closing_line_and_names_lose_control_characters() {
        let many: Vec<Entry> = (0..MAX_LISTING_ENTRIES + 25).map(|i| Entry { name: format!("f{i}\u{7}.txt"), size: 1, is_dir: false }).collect();
        let lines = listing_lines(&many);
        assert_eq!(lines.len(), 1 + MAX_LISTING_ENTRIES + 1);
        assert!(lines[1].starts_with("f0.txt  ("), "{}", lines[1]);
        assert!(lines.last().unwrap().starts_with("and 25 more"));
        assert_eq!(listing_lines(&[]), vec!["This archive is empty.".to_string()]);
        assert!(listing_lines(&[Entry { name: "x".into(), size: 5, is_dir: false }])[0].starts_with("1 item, "));
    }

    #[test]
    fn exposure_and_date_read_the_way_a_person_says_them() {
        assert_eq!(exposure_text(0.004).as_deref(), Some("1/250 s"));
        assert_eq!(exposure_text(2.0).as_deref(), Some("2 s"));
        assert_eq!(exposure_text(2.5).as_deref(), Some("2.5 s"));
        assert_eq!(exposure_text(0.0), None);
        assert_eq!(exposure_text(f64::NAN), None);
        assert_eq!(date_text("2026:10:13 12:34:56"), "2026-10-13 12:34");
        assert_eq!(date_text("yesterday"), "yesterday");
        assert_eq!(trim_float(2.8), "2.8");
        assert_eq!(trim_float(4.0), "4");
    }

    #[test]
    fn the_picture_line_names_the_camera_once_and_leaves_out_what_is_missing() {
        let mut p = Properties::new(4032, 3024);
        let bare = picture_info(&p, Format::Jpeg, 3_200_000, 1);
        assert_eq!(bare, format!("4032 \u{D7} 3024  \u{B7}  JPG  \u{B7}  {}", crate::files::fs::size_text(3_200_000)));
        p.tiff.insert("Make".into(), PropValue::Text("Acme".into()));
        p.tiff.insert("Model".into(), PropValue::Text("Acme Pocket 3".into()));
        p.exif.insert("DateTimeOriginal".into(), PropValue::Text("2026:10:13 12:00:00".into()));
        p.exif.insert("ExposureTime".into(), PropValue::Float(0.004));
        p.exif.insert("FNumber".into(), PropValue::Float(2.8));
        p.exif.insert("ISOSpeedRatings".into(), PropValue::Int(200));
        p.exif.insert("FocalLength".into(), PropValue::Float(35.0));
        let line = picture_info(&p, Format::Jpeg, 1000, 1);
        assert!(line.contains("Acme Pocket 3") && !line.contains("Acme Acme"), "{line}");
        assert!(line.ends_with("2026-10-13 12:00  \u{B7}  1/250 s  \u{B7}  f/2.8  \u{B7}  ISO 200  \u{B7}  35 mm"), "{line}");
        p.tiff.insert("Model".into(), PropValue::Text("Pocket 3".into()));
        assert!(picture_info(&p, Format::Jpeg, 1000, 1).contains("Acme Pocket 3"), "a model without the maker gets it in front");
        assert!(picture_info(&p, Format::Gif, 1000, 12).contains("12 frames"));
    }

    #[test]
    fn a_picture_file_gives_its_line_and_other_files_none() {
        let dir = scratch("pic");
        std::fs::write(dir.join("a.png"), crate::cg::imageio::tests::png_bytes(7, 5, [1, 2, 3, 255])).unwrap();
        let line = picture_info_for(&dir.join("a.png")).unwrap();
        assert!(line.starts_with("7 \u{D7} 5  \u{B7}  PNG"), "{line}");
        std::fs::write(dir.join("t.txt"), b"hello").unwrap();
        assert_eq!(picture_info_for(&dir.join("t.txt")), None);
        assert_eq!(picture_info_for(&dir.join("missing.png")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_frame_clock_follows_delays_loops_and_the_short_delay_rule() {
        let delays = [200, 300];
        assert_eq!(frame_index(&delays, None, 0), 0);
        assert_eq!(frame_index(&delays, None, 199), 0);
        assert_eq!(frame_index(&delays, None, 200), 1);
        assert_eq!(frame_index(&delays, None, 500), 0, "forever: the pass starts again");
        assert_eq!(frame_index(&delays, Some(0), 1_000_000_000), frame_index(&delays, None, 1_000_000_000));
        // two passes in all (the first plus one repeat): afterwards the last frame stays
        assert_eq!(frame_index(&delays, Some(1), 900), 1);
        assert_eq!(frame_index(&delays, Some(1), 1000), 1);
        assert_eq!(frame_index(&delays, Some(1), 5000), 1);
        assert_eq!(frame_index(&delays, Some(1), 600), 0);
        // a zero or 10 ms delay shows for 100 ms
        assert_eq!(frame_index(&[0, 10, 500], None, 99), 0);
        assert_eq!(frame_index(&[0, 10, 500], None, 100), 1);
        assert_eq!(frame_index(&[0, 10, 500], None, 200), 2);
        assert_eq!(frame_index(&[], None, 5), 0);
    }

    #[test]
    fn an_animation_needs_two_frames_and_reports_when_the_frame_changes() {
        let (a, b) = (Image::new(2, 2), Image::new(2, 2));
        assert!(PictureAnim::new(vec![a.clone()], vec![100], None).is_none());
        assert!(PictureAnim::new(vec![a.clone(), b.clone()], vec![100], None).is_none(), "one delay for two frames");
        let mut anim = PictureAnim::new(vec![a, b], vec![100, 100], None).unwrap();
        assert!(!anim.tick_at(50), "still on the first frame");
        assert!(anim.tick_at(150), "the second frame came up");
        assert!(!anim.tick_at(160));
        assert!(anim.tick_at(210), "and the first again after a pass");
        assert_eq!(anim.count(), 2);
        assert_eq!((anim.frame().w, anim.frame().h), (2, 2));
    }

    #[test]
    fn an_animated_png_on_disk_loads_as_an_animation_and_a_still_one_does_not() {
        let dir = scratch("anim");
        let (red, blue) = (vec![255u8, 0, 0, 255].repeat(6), vec![0u8, 0, 255, 255].repeat(6));
        let frames = [ApngFrame { rgba: &red, delay_ms: 120 }, ApngFrame { rgba: &blue, delay_ms: 240 }];
        std::fs::write(dir.join("a.png"), encode_apng(3, 2, &frames, 0, &PngOptions::default()).unwrap()).unwrap();
        let mut anim = load_animation(&dir.join("a.png")).unwrap();
        assert_eq!(anim.count(), 2);
        assert_eq!(anim.frame().px[0], crate::gfx::rgba(255, 0, 0, 255));
        anim.tick_at(130);
        assert_eq!(anim.frame().px[0], crate::gfx::rgba(0, 0, 255, 255));
        std::fs::write(dir.join("still.png"), crate::cg::imageio::tests::png_bytes(3, 2, [9, 9, 9, 255])).unwrap();
        assert!(load_animation(&dir.join("still.png")).is_none());
        std::fs::write(dir.join("photo.jpg"), b"not read at all").unwrap();
        assert!(load_animation(&dir.join("photo.jpg")).is_none(), "only GIF and PNG files animate");
        assert!(load_animation(&dir.join("missing.gif")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
