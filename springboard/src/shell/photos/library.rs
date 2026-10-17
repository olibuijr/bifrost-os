//! The photo library on disk: which files count, how they are ordered and
//! grouped into month sections, and the calendar arithmetic for the labels.

use std::path::{Path, PathBuf};

/// Directories scanned for pictures, in order.
pub fn dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    vec![home.join("Pictures")]
}
/// Deleted photos wait here (inside the Pictures directory) until purged.
pub const TRASH: &str = ".trash";
/// How deep `scan` descends below each directory.
const MAX_DEPTH: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Photo {
    pub path: PathBuf,
    /// when the picture was taken (EXIF DateTimeOriginal) or, without one, the
    /// file's modification time; Unix seconds
    pub mtime: i64,
    pub size: u64,
}

impl Photo {
    /// Reads the file's metadata; None when it is gone.
    pub fn from_path(path: PathBuf) -> Option<Photo> {
        Photo::from_path_at(path, 0)
    }

    /// Like `from_path`; the EXIF capture time (camera wall clock) is read in a
    /// zone `offset` seconds east of UTC.
    pub fn from_path_at(path: PathBuf, offset: i32) -> Option<Photo> {
        let md = std::fs::metadata(&path).ok()?;
        let modified = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
        let mtime = taken_at(&path, offset).unwrap_or(modified);
        Some(Photo { path, mtime, size: md.len() })
    }

    /// Cache key: a hash of path, time and size, so an edited file gets a new thumbnail.
    pub fn key(&self) -> String {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in self.path.to_string_lossy().bytes().chain(self.mtime.to_le_bytes()).chain(self.size.to_le_bytes()) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        format!("{h:016x}")
    }

    pub fn name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

/// The movie files Photos shows beside the pictures: MP4 containers, QuickTime included.
pub fn is_video_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.ends_with(".mp4") || name.ends_with(".m4v") || name.ends_with(".mov")
}

/// How much of a JPEG is read to find its capture time.
const EXIF_HEAD: u64 = 64 * 1024;

/// The EXIF capture time of a JPEG, when it has one.
fn taken_at(path: &Path, offset: i32) -> Option<i64> {
    use std::io::Read;
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    if !(name.ends_with(".jpg") || name.ends_with(".jpeg")) {
        return None;
    }
    let mut head = Vec::new();
    std::fs::File::open(path).ok()?.take(EXIF_HEAD).read_to_end(&mut head).ok()?;
    super::exif::taken_unix(&super::exif::parse(&head)?.taken?, offset)
}

/// Duplicate: copies the picture at `path` beside it as `name 2.ext`, `name 3.ext`... and returns the copy's path.
/// Nothing is overwritten.
pub fn duplicate(path: &Path) -> std::io::Result<PathBuf> {
    let dir = path.parent().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no folder"))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no file name"))?;
    let mut source = std::fs::File::open(path)?;
    let (copy, mut out) = crate::net::share::create_unique(dir, &name)?;
    if let Err(e) = std::io::copy(&mut source, &mut out) {
        let _ = std::fs::remove_file(&copy);
        return Err(e);
    }
    Ok(copy)
}

/// Every picture under `dirs` (hidden entries skipped), oldest first.
pub fn scan(dirs: &[PathBuf]) -> Vec<Photo> {
    scan_at(dirs, 0)
}

/// `scan` reading capture times in a zone `offset` seconds east of UTC.
pub fn scan_at(dirs: &[PathBuf], offset: i32) -> Vec<Photo> {
    let mut out = Vec::new();
    for d in dirs {
        walk(d, 0, &mut out, offset);
    }
    out.sort_by(|a, b| a.mtime.cmp(&b.mtime).then_with(|| a.path.cmp(&b.path)));
    out.dedup();
    out
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<Photo>, offset: i32) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = e.path();
        if path.is_dir() {
            if depth + 1 < MAX_DEPTH {
                walk(&path, depth + 1, out, offset);
            }
        } else if crate::cg::imageio::is_image_name(&name) || is_video_name(&name) {
            out.extend(Photo::from_path_at(path, offset));
        }
    }
}

/// The pictures in the trash of `pictures`, oldest deletion first.
pub fn scan_trash(pictures: &Path, offset: i32) -> Vec<Photo> {
    scan_at(&[pictures.join(TRASH)], offset)
}

/// Moves `path` into `dir` keeping its name (numbered on a clash); returns the new path.
pub fn move_to(path: &Path, dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().ok_or_else(|| std::io::Error::other("no file name"))?.to_string_lossy().into_owned();
    let (stem, ext) = name.rsplit_once('.').map(|(s, e)| (s.to_string(), format!(".{e}"))).unwrap_or((name.clone(), String::new()));
    let mut target = dir.join(&name);
    let mut n = 2;
    while target.exists() {
        target = dir.join(format!("{stem} {n}{ext}"));
        n += 1;
    }
    std::fs::rename(path, &target)?;
    Ok(target)
}

/// Groups of pictures whose files are byte for byte the same, each group in the order given and with at least two
/// members. Files are told apart by size first; only those that share a size are read and hashed.
pub fn duplicate_groups(photos: &[Photo]) -> Vec<Vec<PathBuf>> {
    use std::collections::BTreeMap;
    let mut by_size: BTreeMap<u64, Vec<&Photo>> = BTreeMap::new();
    for p in photos {
        by_size.entry(p.size).or_default().push(p);
    }
    let mut groups: Vec<Vec<PathBuf>> = Vec::new();
    for (_, same) in by_size.into_iter().filter(|(size, list)| *size > 0 && list.len() > 1) {
        let mut by_hash: BTreeMap<[u8; 32], Vec<PathBuf>> = BTreeMap::new();
        for p in same {
            if let Some(hash) = hash_file(&p.path) {
                by_hash.entry(hash).or_default().push(p.path.clone());
            }
        }
        groups.extend(by_hash.into_values().filter(|list| list.len() > 1));
    }
    groups.sort();
    groups
}

/// The SHA-256 of a file's contents; None when it cannot be read.
fn hash_file(path: &Path) -> Option<[u8; 32]> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut hash = crate::shell::sha256::Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            return Some(hash.finish());
        }
        hash.update(&buf[..n]);
    }
}

/// A calendar date and time of day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Date {
    pub year: i32,
    /// 1..=12
    pub month: u32,
    /// 1..=31
    pub day: u32,
    pub hour: u32,
    pub min: u32,
    pub sec: u32,
    /// 0 = Sunday
    pub wday: u32,
}

pub const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
pub const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// Local calendar time of `unix` seconds in a zone `offset` seconds east of UTC.
pub fn civil(unix: i64, offset: i32) -> Date {
    let local = unix + offset as i64;
    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400) as u32;
    let (year, month, day) = crate::bare::timezone::civil_from_days(days);
    Date { year: year as i32, month, day, hour: secs / 3600, min: secs / 60 % 60, sec: secs % 60, wday: crate::bare::timezone::weekday(days) as u32 }
}

/// "October 2026"
pub fn month_title(d: &Date) -> String {
    format!("{} {}", MONTHS[(d.month - 1) as usize], d.year)
}

/// "4 October 2026"
pub fn day_title(d: &Date) -> String {
    format!("{} {} {}", d.day, MONTHS[(d.month - 1) as usize], d.year)
}

/// "00:23"
pub fn time_title(d: &Date) -> String {
    format!("{:02}:{:02}", d.hour, d.min)
}

/// "Screenshot-20261004-002315"
pub fn stamp(d: &Date) -> String {
    format!("{:04}{:02}{:02}-{:02}{:02}{:02}", d.year, d.month, d.day, d.hour, d.min, d.sec)
}

/// A run of photos from one month, in library order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    pub start: usize,
    pub len: usize,
}

/// Groups consecutive photos (already sorted by time) by local month.
pub fn sections(photos: &[Photo], offset: i32) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    let mut last = None;
    for (i, p) in photos.iter().enumerate() {
        let d = civil(p.mtime, offset);
        let month = (d.year, d.month);
        if last == Some(month) {
            out.last_mut().unwrap().len += 1;
        } else {
            out.push(Section { title: month_title(&d), start: i, len: 1 });
            last = Some(month);
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    /// A fresh scratch directory under target/ for one test.
    pub fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-photos")).join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Writes a tiny PNG at `path` dated `unix`.
    pub fn picture(path: &Path, unix: i64) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, crate::cg::imageio::tests::png_bytes(4, 4, [10, 20, 30, 255])).unwrap();
        let f = std::fs::File::options().write(true).open(path).unwrap();
        f.set_modified(UNIX_EPOCH + Duration::from_secs(unix as u64)).unwrap();
    }

    #[test]
    fn duplicate_makes_numbered_copies_and_keeps_the_original() {
        let dir = scratch("duplicate-file");
        let a = dir.join("IMG_1.png");
        picture(&a, 1_790_000_000);
        let one = duplicate(&a).unwrap();
        assert_eq!(one, dir.join("IMG_1 2.png"));
        assert_eq!(duplicate(&a).unwrap(), dir.join("IMG_1 3.png"));
        assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&one).unwrap());
        assert!(duplicate(&dir.join("missing.png")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn civil_dates_match_known_instants() {
        let d = civil(0, 0);
        assert_eq!((d.year, d.month, d.day, d.hour, d.wday), (1970, 1, 1, 0, 4), "Thursday");
        let d = civil(1_790_000_000, 0);
        assert_eq!((d.year, d.month, d.day), (2026, 9, 21));
        // 2026-10-04 00:23:15 UTC
        let d = civil(1_791_073_395, 0);
        assert_eq!((d.year, d.month, d.day, d.hour, d.min, d.sec, d.wday), (2026, 10, 4, 0, 23, 15, 0));
        assert_eq!(month_title(&d), "October 2026");
        assert_eq!(day_title(&d), "4 October 2026");
        assert_eq!(time_title(&d), "00:23");
        assert_eq!(stamp(&d), "20261004-002315");
        // an hour west of UTC moves it back to the 3rd
        let w = civil(1_791_073_395, -3600);
        assert_eq!((w.day, w.hour, w.wday), (3, 23, 6));
        let leap = civil(951_782_400, 0);
        assert_eq!((leap.year, leap.month, leap.day), (2000, 2, 29));
        assert_eq!(civil(-86_400, 0).year, 1969);
    }

    #[test]
    fn scan_finds_pictures_recursively_oldest_first_and_skips_hidden() {
        let dir = scratch("scan");
        picture(&dir.join("b.jpg"), 2000);
        picture(&dir.join("sub/a.png"), 1000);
        picture(&dir.join("c.JPEG"), 3000);
        picture(&dir.join(".trash/gone.png"), 500);
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        let missing = dir.join("nope");
        let photos = scan(&[dir.clone(), missing]);
        let names: Vec<String> = photos.iter().map(Photo::name).collect();
        assert_eq!(names, ["a.png", "b.jpg", "c.JPEG"]);
        assert_eq!(photos[0].mtime, 1000);
        assert_eq!(photos[0].size, std::fs::metadata(dir.join("sub/a.png")).unwrap().len());
        assert_eq!(scan_trash(&dir, 0).iter().map(Photo::name).collect::<Vec<_>>(), ["gone.png"]);
        let k = photos[0].key();
        assert_eq!(k.len(), 16);
        assert_ne!(k, photos[1].key());
        assert!(Photo::from_path(dir.join("absent.png")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn exif_capture_time_replaces_the_file_time() {
        use super::super::exif::tests::{ascii, jpeg, tiff};
        let dir = scratch("taken");
        let path = dir.join("shot.jpg");
        std::fs::write(&path, jpeg(&tiff(true, vec![], vec![ascii(0x9003, "2026:10:04 12:30:00")], vec![]), 4, 4)).unwrap();
        let f = std::fs::File::options().write(true).open(&path).unwrap();
        f.set_modified(UNIX_EPOCH + Duration::from_secs(1_000)).unwrap();
        // 12:30 wall clock an hour east of UTC is 11:30 UTC
        assert_eq!(Photo::from_path_at(path.clone(), 3600).unwrap().mtime, 1_791_117_000 - 3600);
        assert_eq!(Photo::from_path(path).unwrap().mtime, 1_791_117_000);
        picture(&dir.join("plain.jpg"), 1_000);
        assert_eq!(Photo::from_path_at(dir.join("plain.jpg"), 0).unwrap().mtime, 1_000, "no EXIF keeps the file time");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sections_group_by_local_month() {
        let p = |mtime| Photo { path: PathBuf::from(format!("/p/{mtime}.png")), mtime, size: 1 };
        // Sept 21 2026, Sept 30 23:30 UTC, Oct 2 2026
        let photos = [p(1_790_000_000), p(1_790_811_000), p(1_790_907_795)];
        let s = sections(&photos, 0);
        assert_eq!(s, [Section { title: "September 2026".into(), start: 0, len: 2 }, Section { title: "October 2026".into(), start: 2, len: 1 }]);
        // an hour east, the late-September photo is already October
        let east = sections(&photos, 3600);
        assert_eq!(east.iter().map(|s| s.len).collect::<Vec<_>>(), [1, 2]);
        assert!(sections(&[], 0).is_empty());
    }

    #[test]
    fn duplicates_are_files_with_the_same_bytes() {
        let dir = scratch("dupes");
        picture(&dir.join("a.png"), 100);
        std::fs::copy(dir.join("a.png"), dir.join("a copy.png")).unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::copy(dir.join("a.png"), dir.join("sub/same.png")).unwrap();
        // same size as a.png but other bytes
        let mut other = std::fs::read(dir.join("a.png")).unwrap();
        let last = other.len() - 1;
        other[last] ^= 0xff;
        std::fs::write(dir.join("c.png"), other).unwrap();
        // a different picture altogether
        std::fs::write(dir.join("d.png"), crate::cg::imageio::tests::png_bytes(5, 5, [1, 2, 3, 255])).unwrap();
        let photos = scan(&[dir.clone()]);
        assert_eq!(photos.len(), 5);
        let groups = duplicate_groups(&photos);
        assert_eq!(groups.len(), 1);
        let mut names: Vec<String> = groups[0].iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["a copy.png", "a.png", "same.png"]);
        assert!(duplicate_groups(&[]).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn move_to_keeps_the_name_and_numbers_clashes() {
        let dir = scratch("move");
        let trash = dir.join(TRASH);
        picture(&dir.join("x.png"), 1);
        let moved = move_to(&dir.join("x.png"), &trash).unwrap();
        assert_eq!(moved, trash.join("x.png"));
        assert!(!dir.join("x.png").exists());
        picture(&dir.join("x.png"), 2);
        assert_eq!(move_to(&dir.join("x.png"), &trash).unwrap(), trash.join("x 2.png"));
        assert!(move_to(&dir.join("x.png"), &trash).is_err(), "nothing left to move");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
