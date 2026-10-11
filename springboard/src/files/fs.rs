//! The Files app's model: directory entries, their kinds, sorting, the
//! operations the context menu offers, and the text the rows show. Pure
//! functions over the file system, so every piece is tested on a temp dir.

use std::cmp::Ordering;
use std::io;
use std::path::{Path, PathBuf};

/// What a file previews as (chosen by extension; `sniff` refines `Other`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    Text,
    Image,
    Video,
    Audio,
    Archive,
    Other,
}

impl Kind {
    /// The kind line of the info sheet.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Folder => "Folder",
            Kind::Text => "Plain Text",
            Kind::Image => "Image",
            Kind::Video => "Video",
            Kind::Audio => "Audio",
            Kind::Archive => "Archive",
            Kind::Other => "Document",
        }
    }
}

/// The lower-case extension of `name`, if any.
pub fn extension(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty() && !ext.is_empty() && !ext.contains(' ')).then(|| ext.to_ascii_lowercase())
}

/// The type identifier a name's extension maps to (`UTType(filenameExtension:)`), if it has one.
fn type_of(name: &str) -> Option<&'static str> {
    bifrost_ui::uttype::type_for_extension(&extension(name)?)
}

/// The kind a name's type implies, by `UTType.conforms(to:)`. Only PNG and JPEG count as Image: they are the formats
/// the viewer can decode (a GIF or WebP is still shown as "GIF image" in the info sheet, but opens as a document).
pub fn kind_of(name: &str, is_dir: bool) -> Kind {
    use bifrost_ui::uttype::conforms;
    if is_dir {
        return Kind::Folder;
    }
    let Some(id) = type_of(name) else { return Kind::Other };
    if conforms(id, "public.png") || conforms(id, "public.jpeg") {
        Kind::Image
    } else if conforms(id, "public.text") {
        Kind::Text
    } else if conforms(id, "public.movie") {
        Kind::Video
    } else if conforms(id, "public.audio") {
        Kind::Audio
    } else if conforms(id, "public.archive") {
        Kind::Archive
    } else {
        Kind::Other
    }
}

/// What Files's Kind column says: the type's description ("PNG image", "Rust Source"), "Folder", or the coarse kind
/// when the extension is unknown (sniffed text is "Plain Text", anything else "Document").
pub fn kind_description(name: &str, is_dir: bool, kind: Kind) -> &'static str {
    if is_dir {
        return Kind::Folder.label();
    }
    type_of(name).map_or(kind.label(), bifrost_ui::uttype::description)
}

/// Refines a file of unknown kind from its first bytes: an image by its
/// magic, text when it is valid UTF-8 without NULs, otherwise still Other.
pub fn sniff(head: &[u8]) -> Kind {
    if crate::imgdec::is_image(head) {
        return Kind::Image;
    }
    if head.is_empty() || head.contains(&0) {
        return Kind::Other;
    }
    match std::str::from_utf8(head) {
        Ok(_) => Kind::Text,
        // a multi-byte character may be cut at the end of the sample
        Err(e) if e.error_len().is_none() => Kind::Text,
        Err(_) => Kind::Other,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    /// modification time, Unix seconds
    pub mtime: i64,
    pub kind: Kind,
    /// entries inside a folder, when counted
    pub items: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Name,
    Date,
    Size,
    Kind,
}

impl Sort {
    pub const ALL: [Sort; 4] = [Sort::Name, Sort::Date, Sort::Size, Sort::Kind];

    pub fn label(self) -> &'static str {
        match self {
            Sort::Name => "Name",
            Sort::Date => "Date",
            Sort::Size => "Size",
            Sort::Kind => "Kind",
        }
    }
}

/// Case-insensitive comparison that orders embedded numbers by value
/// ("file2" before "file10"), as Files and Files do.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(ca), Some(cb)) if ca.is_ascii_digit() && cb.is_ascii_digit() => {
                let num = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().filter(|c| c.is_ascii_digit()) {
                        s.push(*c);
                        it.next();
                    }
                    s
                };
                let (na, nb) = (num(&mut ai), num(&mut bi));
                let o = na.trim_start_matches('0').len().cmp(&nb.trim_start_matches('0').len()).then_with(|| na.trim_start_matches('0').cmp(nb.trim_start_matches('0')));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(ca), Some(cb)) => {
                let o = ca.to_lowercase().cmp(cb.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                ai.next();
                bi.next();
            }
        }
    }
}

/// Orders entries as Files does: by name (natural, folders mixed in), by
/// date newest first, or by size largest first with folders after files.
pub fn sort(entries: &mut [Entry], by: Sort) {
    entries.sort_by(|a, b| match by {
        Sort::Name => natural_cmp(&a.name, &b.name),
        Sort::Date => b.mtime.cmp(&a.mtime).then_with(|| natural_cmp(&a.name, &b.name)),
        Sort::Size => a.is_dir.cmp(&b.is_dir).then_with(|| b.size.cmp(&a.size)).then_with(|| natural_cmp(&a.name, &b.name)),
        Sort::Kind => kind_description(&a.name, a.is_dir, a.kind).cmp(kind_description(&b.name, b.is_dir, b.kind)).then_with(|| natural_cmp(&a.name, &b.name)),
    });
}

/// Indices of entries whose name contains `query`, ignoring case; all when empty.
pub fn search(entries: &[Entry], query: &str) -> Vec<usize> {
    let q = query.to_lowercase();
    entries.iter().enumerate().filter(|(_, e)| q.is_empty() || e.name.to_lowercase().contains(&q)).map(|(i, _)| i).collect()
}

/// Folders with more entries than this skip the per-subfolder item count.
const COUNT_LIMIT: usize = 200;

fn mtime_of(md: &std::fs::Metadata) -> i64 {
    md.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64)
}

/// Reads a directory into unsorted entries (dot files included). The
/// Recents stand-in lists the newest files and the Recently Deleted folder
/// shows original names; the trash folder itself stays out of listings.
pub fn list(dir: &Path) -> io::Result<Vec<Entry>> {
    if dir == Path::new(RECENTS) {
        return Ok(recent_in(&recent_roots(), RECENT_LIMIT));
    }
    if dir.starts_with(TAG_ROOT) {
        return Ok(list_tag(dir));
    }
    if dir == trash_dir() {
        return list_trash(dir);
    }
    list_dir(dir)
}

fn list_dir(dir: &Path) -> io::Result<Vec<Entry>> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir)? {
        let Ok(e) = e else { continue };
        let name = e.file_name().to_string_lossy().into_owned();
        if name == super::trash::DIR || name == super::tags::SIDECAR {
            continue;
        }
        let path = e.path();
        // follow symlinks for the kind, but a dangling link is still listed
        let md = std::fs::metadata(&path).or_else(|_| e.metadata());
        let (is_dir, size, mtime) = match md {
            Ok(md) => (md.is_dir(), if md.is_dir() { 0 } else { md.len() }, mtime_of(&md)),
            Err(_) => (false, 0, 0),
        };
        out.push(Entry { kind: kind_of(&name, is_dir), name, path, is_dir, size, mtime, items: None });
    }
    if out.len() <= COUNT_LIMIT {
        for e in out.iter_mut().filter(|e| e.is_dir) {
            e.items = std::fs::read_dir(&e.path).ok().map(|d| d.count());
        }
    }
    Ok(out)
}

/// A name for the file system: not empty, no slash, not "." or "..".
pub fn valid_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty() && !n.contains('/') && !n.contains('\0') && n != "." && n != ".."
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "Invalid name")
}

/// Renames `path` to `name` beside it; fails when the name is taken.
pub fn rename(path: &Path, name: &str) -> io::Result<PathBuf> {
    if !valid_name(name) {
        return Err(invalid());
    }
    let to = path.parent().ok_or_else(invalid)?.join(name.trim());
    if to == path {
        return Ok(to);
    }
    if to.symlink_metadata().is_ok() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "A file with that name already exists"));
    }
    std::fs::rename(path, &to)?;
    Ok(to)
}

/// Deletes a file or a folder with everything in it.
pub fn delete(path: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(path)?.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// `name` in `dir`, or "name 2", "name 3"… when it is taken (the
/// extension stays at the end).
pub fn unique_name(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if first.symlink_metadata().is_err() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.').filter(|_| extension(name).is_some()) {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.to_string(), String::new()),
    };
    (2..u32::MAX).map(|n| dir.join(format!("{stem} {n}{ext}"))).find(|p| p.symlink_metadata().is_err()).unwrap_or(first)
}

/// Copies `src` into `dir`, numbering the name when it is taken.
pub fn copy_to(src: &Path, dir: &Path) -> io::Result<PathBuf> {
    let name = src.file_name().ok_or_else(invalid)?.to_string_lossy().into_owned();
    if dir.starts_with(src) && std::fs::symlink_metadata(src)?.is_dir() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Cannot copy a folder into itself"));
    }
    let dst = unique_name(dir, &name);
    super::batch::copy_path(src, &dst)?;
    Ok(dst)
}

/// Moves `src` into `dir` (a copy and delete across file systems).
pub fn move_to(src: &Path, dir: &Path) -> io::Result<PathBuf> {
    let name = src.file_name().ok_or_else(invalid)?.to_string_lossy().into_owned();
    if src.parent() == Some(dir) {
        return Ok(src.to_path_buf());
    }
    if dir.starts_with(src) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Cannot move a folder into itself"));
    }
    let dst = unique_name(dir, &name);
    match std::fs::rename(src, &dst) {
        Ok(()) => Ok(dst),
        Err(e) if e.raw_os_error() == Some(18) => {
            // EXDEV: another file system; the original goes only after a whole copy
            super::batch::copy_path(src, &dst)?;
            if let Err(e) = super::batch::verify_copy(src, &dst) {
                let _ = delete(&dst);
                return Err(e);
            }
            delete(src)?;
            Ok(dst)
        }
        Err(e) => Err(e),
    }
}

/// Creates `name` in `dir`.
pub fn new_folder(dir: &Path, name: &str) -> io::Result<PathBuf> {
    if !valid_name(name) {
        return Err(invalid());
    }
    let p = dir.join(name.trim());
    std::fs::create_dir(&p)?;
    Ok(p)
}

/// Largest text file read for a preview.
pub const TEXT_LIMIT: u64 = 1024 * 1024;

/// The first bytes of a file, for `sniff`.
pub fn head(path: &Path, n: usize) -> io::Result<Vec<u8>> {
    use std::io::Read;
    let mut buf = vec![0u8; n];
    let mut f = std::fs::File::open(path)?;
    let mut got = 0;
    while got < n {
        match f.read(&mut buf[got..])? {
            0 => break,
            k => got += k,
        }
    }
    buf.truncate(got);
    Ok(buf)
}

/// Reads up to `TEXT_LIMIT` bytes as text, tabs widened to four spaces
/// and carriage returns dropped; `truncated` says whether the file went on.
pub fn read_text(path: &Path) -> io::Result<(String, bool)> {
    let bytes = head(path, TEXT_LIMIT as usize + 1)?;
    let truncated = bytes.len() as u64 > TEXT_LIMIT;
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(TEXT_LIMIT as usize)]).replace('\t', "    ").replace('\r', "");
    Ok((text, truncated))
}

/// "12 bytes", "117 KB", "2.3 MB", "117.86 GB", as Files shows sizes.
pub fn size_text(bytes: u64) -> String {
    let b = bytes as f64;
    if bytes == 1 {
        "1 byte".into()
    } else if bytes < 1000 {
        format!("{bytes} bytes")
    } else if b < 1e6 {
        format!("{:.0} KB", b / 1e3)
    } else if b < 1e9 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{:.2} GB", b / 1e9)
    }
}

/// Civil date for a Unix time already shifted to local: (year, month 1-12, day, hour, minute).
pub fn civil(local: i64) -> (i32, u32, u32, u32, u32) {
    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    // Howard Hinnant's days-to-civil
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = (yoe + era * 400 + (m <= 2) as i64) as i32;
    (y, m, d, (secs / 3600) as u32, (secs % 3600 / 60) as u32)
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// "Today at 09:41", "Yesterday at 18:02" or "4 Oct 2026 at 00:22";
/// `offset` is the zone's seconds east of UTC.
pub fn date_text(mtime: i64, now: i64, offset: i32) -> String {
    let (y, m, d, h, min) = civil(mtime + offset as i64);
    let (ny, nm, nd, ..) = civil(now + offset as i64);
    let (yy, ym, yd, ..) = civil(now + offset as i64 - 86_400);
    let day = if (y, m, d) == (ny, nm, nd) {
        "Today".to_string()
    } else if (y, m, d) == (yy, ym, yd) {
        "Yesterday".to_string()
    } else {
        format!("{d} {} {y}", MONTHS[(m - 1) as usize])
    };
    format!("{day} at {h:02}:{min:02}")
}

/// A root the sidebar offers.
#[derive(Clone, Debug, PartialEq)]
pub struct Location {
    pub name: &'static str,
    pub path: PathBuf,
    pub glyph: Glyph,
}

/// The sidebar icon drawn for a location.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Computer,
    Download,
    Play,
    Photo,
    Root,
    Clock,
    Bin,
    /// a colour tag row in the sidebar's Tags section
    Tag(super::tags::Tag),
}

/// Where the app's own folder lives on this PC.
pub fn home() -> PathBuf {
    crate::shell::apphost::root().join("Files")
}

/// The user's home directory (`$HOME`), `/` when unset.
fn user_home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// Stands for the Recents view: a listing of the newest files, not a folder.
pub const RECENTS: &str = "/.recents";
/// How many files Recents shows.
pub const RECENT_LIMIT: usize = 50;

/// The Recently Deleted folder: hidden, under the app's own folder.
pub fn trash_dir() -> PathBuf {
    super::trash::dir(&home())
}

/// The folders Recents looks through.
pub fn recent_roots() -> Vec<PathBuf> {
    let u = user_home();
    vec![home(), u.clone(), u.join("Pictures")]
}

/// The roots, in sidebar order.
pub fn locations() -> Vec<Location> {
    let u = user_home();
    vec![
        Location { name: "On My PC", path: home(), glyph: Glyph::Computer },
        Location { name: "Recents", path: RECENTS.into(), glyph: Glyph::Clock },
        Location { name: "Downloads", path: u.join("Downloads"), glyph: Glyph::Download },
        Location { name: "Deildu", path: crate::shell::apphost::root().join("deildu/torrents"), glyph: Glyph::Play },
        Location { name: "Pictures", path: u.join("Pictures"), glyph: Glyph::Photo },
        Location { name: "Recently Deleted", path: trash_dir(), glyph: Glyph::Bin },
        Location { name: "Root", path: "/".into(), glyph: Glyph::Root },
    ]
    .into_iter()
    .chain(super::tags::Tag::ALL.into_iter().map(|t| Location { name: t.label(), path: Path::new(TAG_ROOT).join(t.label()), glyph: Glyph::Tag(t) }))
    .collect()
}

/// Stand-in folder of the sidebar's Tags rows: `/.tag/<Colour>` lists the tagged items.
pub const TAG_ROOT: &str = "/.tag";

fn list_tag(dir: &Path) -> Vec<Entry> {
    let Some(tag) = super::tags::Tag::ALL.into_iter().find(|t| dir.file_name().is_some_and(|n| n == t.label())) else { return vec![] };
    let index = super::tags::TagIndex::load(&super::tags::sidecar_path(&home()));
    index
        .with_tag(tag)
        .into_iter()
        .filter_map(|path| {
            let md = std::fs::metadata(&path).ok()?;
            let name = path.file_name()?.to_string_lossy().into_owned();
            Some(Entry { kind: kind_of(&name, md.is_dir()), name, path, is_dir: md.is_dir(), size: if md.is_dir() { 0 } else { md.len() }, mtime: mtime_of(&md), items: None })
        })
        .collect()
}

/// The newest `limit` files under `roots` (folders skipped, hidden
/// folders not entered, at most 6 levels deep), newest first.
pub fn recent_in(roots: &[PathBuf], limit: usize) -> Vec<Entry> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<Entry>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            let Ok(md) = std::fs::metadata(&path) else { continue };
            if md.is_dir() {
                if depth < 6 && !name.starts_with('.') {
                    walk(&path, depth + 1, out);
                }
            } else {
                out.push(Entry { kind: kind_of(&name, false), name, path, is_dir: false, size: md.len(), mtime: mtime_of(&md), items: None });
            }
        }
    }
    let mut out = vec![];
    for r in roots {
        walk(r, 0, &mut out);
    }
    out.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.path.cmp(&b.path)));
    out.dedup_by(|a, b| a.path == b.path);
    out.truncate(limit);
    out
}

/// Entries of the Recently Deleted folder, shown under their original names.
fn list_trash(dir: &Path) -> io::Result<Vec<Entry>> {
    let mut out = list_dir(dir)?;
    for e in &mut out {
        if let Some((_, name)) = super::trash::parse(&e.name) {
            let name = name.to_string();
            e.kind = kind_of(&name, e.is_dir);
            e.name = name;
        }
    }
    Ok(out)
}

/// (total, available) bytes of the file system holding `path`.
pub fn capacity(path: &Path) -> Option<(u64, u64)> {
    #[cfg(target_os = "linux")]
    {
        crate::bare::sys::disk_space(path.to_str()?)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        None
    }
}

/// The crumbs from a root to `path`: (title, path) pairs, root first.
pub fn breadcrumb(root: &Location, path: &Path) -> Vec<(String, PathBuf)> {
    let mut out = vec![(root.name.to_string(), root.path.clone())];
    let Ok(rest) = path.strip_prefix(&root.path) else { return out };
    let mut p = root.path.clone();
    for c in rest.components() {
        p.push(c);
        out.push((c.as_os_str().to_string_lossy().into_owned(), p.clone()));
    }
    out
}

/// The display name of a path: its file name, or the path itself for "/".
pub fn title_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string_lossy().into_owned())
}



/// Bytes free for the user on the file system holding `path`.
pub fn available(path: &Path) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        crate::bare::sys::disk_space(path.to_str()?).map(|(_, free)| free)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A fresh scratch directory under target/ named `tag`.
    pub(crate) fn temp(tag: &str) -> PathBuf {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-files")).join(tag);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entry(name: &str, is_dir: bool, size: u64, mtime: i64) -> Entry {
        Entry { kind: kind_of(name, is_dir), name: name.into(), path: PathBuf::from(name), is_dir, size, mtime, items: None }
    }

    #[test]
    fn copies_keep_links_as_links_and_failures_leave_nothing() {
        let dir = temp("copy-links");
        let src = dir.join("tree");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("f.txt"), "x").unwrap();
        std::os::unix::fs::symlink("f.txt", src.join("link")).unwrap();
        let copy = copy_to(&src, &dir).unwrap();
        assert_eq!(std::fs::read_link(copy.join("link")).unwrap(), Path::new("f.txt"));
        // a missing source fails and leaves no destination behind
        let ghost = dir.join("ghost");
        assert!(super::super::batch::copy_path(&ghost, &dir.join("out")).is_err());
        assert!(!dir.join("out").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kinds_follow_the_extension_and_the_bytes() {
        assert_eq!(kind_of("notes.TXT", false), Kind::Text);
        assert_eq!(kind_of("README.md", false), Kind::Text);
        assert_eq!(kind_of("boot.log", false), Kind::Text);
        assert_eq!(kind_of("a.jpeg", false), Kind::Image);
        assert_eq!(kind_of("film.m4v", false), Kind::Video);
        assert_eq!(kind_of("song.flac", false), Kind::Audio);
        assert_eq!(kind_of("x.tar.gz", false), Kind::Archive);
        assert_eq!(kind_of("binary", false), Kind::Other);
        assert_eq!(kind_of(".hidden", false), Kind::Other);
        assert_eq!(kind_of("a.jpg", true), Kind::Folder);
        assert_eq!(extension("a.b.c"), Some("c".into()));
        assert_eq!(extension("noext"), None);
        assert_eq!(sniff(b"hello\nworld"), Kind::Text);
        assert_eq!(sniff("héllo wörld".as_bytes()), Kind::Text);
        assert_eq!(sniff(&"é".as_bytes()[..1]), Kind::Text, "cut multibyte at the end");
        assert_eq!(sniff(&[0x89, b'P', b'N', b'G']), Kind::Image);
        assert_eq!(sniff(b"\x7fELF\0\0"), Kind::Other);
        assert_eq!(sniff(b""), Kind::Other);
        assert_eq!(Kind::Text.label(), "Plain Text");
    }

    #[test]
    fn kind_descriptions_come_from_the_uniform_types() {
        assert_eq!(kind_description("a.png", false, Kind::Image), "PNG image");
        assert_eq!(kind_description("main.rs", false, Kind::Text), "Rust Source");
        assert_eq!(kind_description("paper.PDF", false, Kind::Other), "PDF document");
        assert_eq!(kind_description("x.gif", false, Kind::Other), "GIF image");
        assert_eq!(kind_description("Makefile", false, Kind::Text), "Plain Text", "sniffed text, no extension");
        assert_eq!(kind_description("blob", false, Kind::Other), "Document");
        assert_eq!(kind_description("pics", true, Kind::Folder), "Folder");
        // every extension the old lists knew still gives the same coarse kind
        for (n, k) in [("a.toml", Kind::Text), ("a.svg", Kind::Text), ("a.srt", Kind::Text), ("a.m3u", Kind::Text), ("a.cpp", Kind::Text), ("a.ogg", Kind::Audio), ("a.opus", Kind::Audio), ("a.mov", Kind::Video), ("a.7z", Kind::Archive), ("a.apk", Kind::Archive), ("a.jar", Kind::Archive), ("a.png", Kind::Image), ("a.gif", Kind::Other)] {
            assert_eq!(kind_of(n, false), k, "{n}");
        }
        let mut v = vec![entry("b.zip", false, 1, 1), entry("a.rs", false, 1, 1), entry("c.png", false, 1, 1)];
        sort(&mut v, Sort::Kind);
        assert_eq!(v.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["c.png", "a.rs", "b.zip"], "PNG image < Rust Source < ZIP archive");
    }

    #[test]
    fn natural_order_and_sorts() {
        assert_eq!(natural_cmp("file2", "file10"), Ordering::Less);
        assert_eq!(natural_cmp("Beta", "alpha"), Ordering::Greater);
        assert_eq!(natural_cmp("a", "A"), Ordering::Greater, "ties fall back to byte order");
        assert_eq!(natural_cmp("x", "xy"), Ordering::Less);
        assert_eq!(natural_cmp("007", "7"), Ordering::Less, "equal values fall back to byte order");
        let mut v = vec![entry("b.txt", false, 50, 10), entry("Alpha", true, 0, 30), entry("a10.txt", false, 500, 20), entry("a2.txt", false, 5, 5)];
        sort(&mut v, Sort::Name);
        assert_eq!(v.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["a2.txt", "a10.txt", "Alpha", "b.txt"]);
        sort(&mut v, Sort::Date);
        assert_eq!(v.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Alpha", "a10.txt", "b.txt", "a2.txt"]);
        sort(&mut v, Sort::Size);
        assert_eq!(v.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["a10.txt", "b.txt", "a2.txt", "Alpha"], "folders last");
        assert_eq!(search(&v, "A"), vec![0, 2, 3]);
        assert_eq!(search(&v, ""), vec![0, 1, 2, 3]);
        assert!(search(&v, "zzz").is_empty());
    }

    #[test]
    fn lists_a_directory_with_sizes_and_counts() {
        let dir = temp("list");
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/x"), "").unwrap();
        std::fs::write(dir.join("sub/y"), "").unwrap();
        let mut v = list(&dir).unwrap();
        sort(&mut v, Sort::Name);
        assert_eq!(v.len(), 2);
        assert_eq!((v[0].name.as_str(), v[0].size, v[0].is_dir, v[0].kind), ("a.txt", 5, false, Kind::Text));
        assert!(v[0].mtime > 1_600_000_000);
        assert_eq!((v[1].name.as_str(), v[1].items, v[1].kind), ("sub", Some(2), Kind::Folder));
        assert_eq!(v[1].path, dir.join("sub"));
        assert!(list(&dir.join("missing")).is_err());
        assert!(available(&dir).is_some_and(|b| b > 0));
    }

    #[test]
    fn rename_delete_move_copy_and_new_folder() {
        let dir = temp("ops");
        let a = dir.join("a.txt");
        std::fs::write(&a, "A").unwrap();
        assert_eq!(rename(&a, "b.txt").unwrap(), dir.join("b.txt"));
        assert!(!a.exists() && dir.join("b.txt").exists());
        assert!(rename(&dir.join("b.txt"), "").is_err());
        assert!(rename(&dir.join("b.txt"), "x/y").is_err());
        std::fs::write(dir.join("c.txt"), "C").unwrap();
        assert_eq!(rename(&dir.join("b.txt"), "c.txt").unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(rename(&dir.join("b.txt"), "b.txt").unwrap(), dir.join("b.txt"), "same name is a no-op");

        let sub = new_folder(&dir, "sub").unwrap();
        assert!(sub.is_dir());
        assert!(new_folder(&dir, "sub").is_err(), "exists");
        assert!(new_folder(&dir, "..").is_err());
        assert!(!valid_name(" ") && valid_name("ok name"));

        assert_eq!(move_to(&dir.join("b.txt"), &sub).unwrap(), sub.join("b.txt"));
        assert!(sub.join("b.txt").exists());
        assert_eq!(move_to(&sub.join("b.txt"), &sub).unwrap(), sub.join("b.txt"), "already there");
        assert!(move_to(&sub, &sub).is_err(), "into itself");

        let copy = copy_to(&sub.join("b.txt"), &sub).unwrap();
        assert_eq!(copy, sub.join("b 2.txt"));
        assert_eq!(std::fs::read(&copy).unwrap(), b"A");
        assert_eq!(copy_to(&sub.join("b.txt"), &sub).unwrap(), sub.join("b 3.txt"));
        let tree = copy_to(&sub, &dir).unwrap();
        assert_eq!(tree, dir.join("sub 2"));
        assert!(tree.join("b 2.txt").exists());
        assert!(copy_to(&sub, &sub).is_err());
        assert_eq!(unique_name(&dir, "new"), dir.join("new"));
        std::fs::write(dir.join("new"), "").unwrap();
        assert_eq!(unique_name(&dir, "new"), dir.join("new 2"));

        delete(&tree).unwrap();
        assert!(!tree.exists());
        delete(&dir.join("c.txt")).unwrap();
        assert!(delete(&dir.join("c.txt")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn text_reading_and_sniffing() {
        let dir = temp("text");
        std::fs::write(dir.join("t.txt"), "a\tb\r\nc").unwrap();
        assert_eq!(read_text(&dir.join("t.txt")).unwrap(), ("a    b\nc".into(), false));
        assert_eq!(head(&dir.join("t.txt"), 2).unwrap(), b"a\t");
        assert!(read_text(&dir.join("none")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sizes_and_dates_read_like_files() {
        assert_eq!(size_text(0), "0 bytes");
        assert_eq!(size_text(1), "1 byte");
        assert_eq!(size_text(999), "999 bytes");
        assert_eq!(size_text(117_000), "117 KB");
        assert_eq!(size_text(2_300_000), "2.3 MB");
        assert_eq!(size_text(117_860_000_000), "117.86 GB");
        assert_eq!(civil(0), (1970, 1, 1, 0, 0));
        assert_eq!(civil(1_791_073_320), (2026, 10, 4, 0, 22));
        assert_eq!(civil(951_782_400), (2000, 2, 29, 0, 0), "leap day");
        let now = 1_791_073_320; // 2026-10-04 00:22 UTC
        assert_eq!(date_text(now - 60, now, 0), "Today at 00:21");
        assert_eq!(date_text(now - 3600, now, 0), "Yesterday at 23:22");
        assert_eq!(date_text(now - 3600, now, 3600), "Today at 00:22", "zone shifts the day");
        assert_eq!(date_text(0, now, 0), "1 Jan 1970 at 00:00");
    }

    #[test]
    fn locations_and_breadcrumbs() {
        let locs = locations();
        assert_eq!(locs.len(), 7 + crate::files::tags::Tag::ALL.len(), "seven places plus the colour tags");
        assert_eq!((locs[0].name, locs[0].path.as_path()), ("On My PC", home().as_path()));
        assert_eq!(locs[1].path, Path::new(RECENTS));
        assert_eq!(locs[5].path, trash_dir());
        assert_eq!(locs[6].path, Path::new("/"));
        let root = &locs[6];
        let crumbs = breadcrumb(root, Path::new("/system/bin"));
        assert_eq!(crumbs, vec![("Root".to_string(), PathBuf::from("/")), ("system".into(), "/system".into()), ("bin".into(), "/system/bin".into())]);
        assert_eq!(breadcrumb(&locs[0], Path::new("/elsewhere")).len(), 1, "outside the root: just the root");
        assert_eq!(title_of(Path::new("/a/b")), "b");
        assert_eq!(title_of(Path::new("/")), "/");
    }

    #[test]
    fn recents_newest_first_skips_hidden_and_limits() {
        let d = temp("recent");
        std::fs::create_dir_all(d.join("a/.hidden")).unwrap();
        std::fs::write(d.join("a/old.txt"), "1").unwrap();
        std::fs::write(d.join("a/.hidden/skip.txt"), "1").unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(5000);
        std::fs::File::options().write(true).open(d.join("a/old.txt")).unwrap().set_modified(old).unwrap();
        std::fs::write(d.join("new.txt"), "2").unwrap();
        let r = recent_in(std::slice::from_ref(&d), 10);
        assert_eq!(r.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["new.txt", "old.txt"]);
        assert_eq!(recent_in(std::slice::from_ref(&d), 1).len(), 1);
    }

    #[test]
    fn collision_names_count_up_and_see_dangling_links() {
        let dir = temp("unique");
        assert_eq!(unique_name(&dir, "a.txt"), dir.join("a.txt"));
        std::fs::write(dir.join("a.txt"), "").unwrap();
        assert_eq!(unique_name(&dir, "a.txt"), dir.join("a 2.txt"));
        std::fs::write(dir.join("a 2.txt"), "").unwrap();
        assert_eq!(unique_name(&dir, "a.txt"), dir.join("a 3.txt"));
        std::os::unix::fs::symlink("nowhere", dir.join("ln")).unwrap();
        assert_eq!(unique_name(&dir, "ln"), dir.join("ln 2"));
        std::fs::write(dir.join("noext"), "").unwrap();
        assert_eq!(unique_name(&dir, "noext"), dir.join("noext 2"));
        assert!(rename(&dir.join("a.txt"), "ln").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
