//! Archive service: what a file is, where its contents go, and how tar streams are read.
//! Expand unpacks zip, tar and gzipped tar beside the archive; compress zips a folder beside it. Member paths
//! that would leave the output folder are skipped, and output names never overwrite what is already there.
//! Progress is `(done, total)` bytes; the callback returns false to cancel. No xz or bzip2 decoder exists yet.

use flate2::read::GzDecoder;
use springboard::files::zip;
use std::cell::Cell;
use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// The archive formats the app knows by name. `TarXz` is recognised so it can be refused with a clear message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Zip,
    Tar,
    TarGz,
    TarXz,
}

/// What the app is asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Expand,
    Compress,
}

/// The result of a finished job.
#[derive(Debug, PartialEq)]
pub struct Outcome {
    pub path: PathBuf,
    pub files: usize,
    /// members not written: unsafe paths, links, devices
    pub skipped: usize,
}

/// The format a file name declares (case ignored), or `None` for a file that is not an archive.
pub fn kind_of(name: &str) -> Option<Kind> {
    let n = name.to_ascii_lowercase();
    if n.ends_with(".tar.gz") || n.ends_with(".tgz") {
        Some(Kind::TarGz)
    } else if n.ends_with(".tar.xz") || n.ends_with(".txz") {
        Some(Kind::TarXz)
    } else if n.ends_with(".tar") {
        Some(Kind::Tar)
    } else if n.ends_with(".zip") {
        Some(Kind::Zip)
    } else {
        None
    }
}

/// The folder name an archive expands to: the name without its archive extension(s).
pub fn stem_of(name: &str) -> String {
    let n = name.to_ascii_lowercase();
    for suffix in [".tar.gz", ".tar.xz", ".tgz", ".txz", ".tar", ".zip"] {
        if n.ends_with(suffix) {
            let cut = name.len() - suffix.len();
            let stem = &name[..cut];
            if !stem.is_empty() {
                return stem.to_string();
            }
        }
    }
    name.to_string()
}

/// `dir/name` with `ext` appended, numbered " 2", " 3" and so on while the path is taken.
pub fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let make = |n: usize| {
        let file = if n == 1 { format!("{stem}{ext}") } else { format!("{stem} {n}{ext}") };
        dir.join(file)
    };
    (1..).map(make).find(|p| p.symlink_metadata().is_err()).expect("an unused name exists")
}

// ------------------------------------------------------------------ tar

const BLOCK: usize = 512;
/// Largest long-name or extended header read into memory.
const HEADER_DATA_LIMIT: u64 = 1 << 20;
/// Copy chunk size for file data.
const CHUNK: usize = 64 << 10;

/// The type of one tar member, as far as extraction cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    /// links, devices, fifos and anything else: not written
    Other,
}

/// One member header from a tar stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub kind: EntryKind,
    pub size: u64,
    /// the permission bits (`0o755` and so on)
    pub mode: u32,
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Cancelled")
}

/// A NUL-terminated text field.
fn field(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

/// An octal number field, or the base-256 form (high bit set in the first byte).
fn number(b: &[u8]) -> Option<u64> {
    if b.first().is_some_and(|&c| c & 0x80 != 0) {
        let mut v: u64 = 0;
        for &c in &b[1..] {
            v = v.checked_mul(256)?.checked_add(c as u64)?;
        }
        return Some(v);
    }
    let text: String = b.iter().take_while(|&&c| c != 0).map(|&c| c as char).collect();
    let t = text.trim_matches(|c: char| c == ' ');
    if t.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(t, 8).ok()
}

/// The unsigned checksum of a header, with the checksum field read as spaces.
fn checksum(h: &[u8]) -> u64 {
    h.iter().enumerate().map(|(i, &c)| if (148..156).contains(&i) { 32 } else { c as u64 }).sum()
}

/// The `path` record of a pax extended header (`"<len> path=<value>\n"` records).
fn pax_path(data: &[u8]) -> Option<String> {
    let mut at = 0;
    while at < data.len() {
        let sp = at + data[at..].iter().position(|&c| c == b' ')?;
        let len: usize = std::str::from_utf8(&data[at..sp]).ok()?.parse().ok()?;
        let rec = data.get(at..at + len)?;
        let body = &rec[sp - at + 1..];
        let body = body.strip_suffix(b"\n").unwrap_or(body);
        if let Some(v) = body.strip_prefix(b"path=") {
            return Some(String::from_utf8_lossy(v).into_owned());
        }
        at += len;
    }
    None
}

fn skip(r: &mut dyn Read, n: u64) -> io::Result<()> {
    let got = io::copy(&mut r.take(n), &mut io::sink())?;
    if got < n {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Truncated archive"));
    }
    Ok(())
}

fn padding(size: u64) -> u64 {
    (BLOCK as u64 - size % BLOCK as u64) % BLOCK as u64
}

/// Reads a tar stream member by member. `sink` gets each header and a reader over its data (exactly `size`
/// bytes); whatever the sink leaves unread is skipped. Long names (GNU `L` and pax `path`) apply to the next
/// member. The end is an all-zero block or the end of the stream.
pub fn read_tar(r: &mut dyn Read, sink: &mut dyn FnMut(&Entry, &mut dyn Read) -> io::Result<()>) -> io::Result<()> {
    let mut pending: Option<String> = None;
    loop {
        let mut h = [0u8; BLOCK];
        let mut got = 0;
        while got < BLOCK {
            let n = r.read(&mut h[got..])?;
            if n == 0 {
                break;
            }
            got += n;
        }
        if got == 0 || h.iter().all(|&b| b == 0) {
            return Ok(());
        }
        if got < BLOCK {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Truncated archive"));
        }
        let stored = number(&h[148..156]).ok_or_else(|| bad("Damaged tar header"))?;
        if stored != checksum(&h) {
            return Err(bad("Damaged tar header"));
        }
        let size = number(&h[124..136]).ok_or_else(|| bad("Damaged tar header"))?;
        let flag = h[156];
        match flag {
            b'x' | b'L' => {
                if size > HEADER_DATA_LIMIT {
                    return Err(bad("Tar header too large"));
                }
                let mut data = vec![0u8; size as usize];
                r.read_exact(&mut data)?;
                skip(r, padding(size))?;
                pending = Some(if flag == b'L' { field(&data) } else { pax_path(&data).unwrap_or_default() }).filter(|p| !p.is_empty());
                continue;
            }
            b'g' | b'K' => {
                skip(r, size + padding(size))?;
                continue;
            }
            _ => {}
        }
        let raw = if &h[257..262] == b"ustar" {
            let (name, prefix) = (field(&h[0..100]), field(&h[345..500]));
            if prefix.is_empty() { name } else { format!("{prefix}/{name}") }
        } else {
            field(&h[0..100])
        };
        let entry = Entry {
            path: pending.take().unwrap_or(raw),
            kind: match flag {
                b'0' | 0 | b'7' => EntryKind::File,
                b'5' => EntryKind::Dir,
                _ => EntryKind::Other,
            },
            size,
            mode: number(&h[100..108]).unwrap_or(0o644) as u32 & 0o7777,
        };
        let mut data = (&mut *r).take(size);
        sink(&entry, &mut data)?;
        let rest = data.limit();
        skip(r, rest)?;
        skip(r, padding(size))?;
    }
}

/// Unpacks a tar stream into `out` (an existing folder). `tick` runs after each chunk of file data and returns
/// false to cancel. Returns the number of files written and the paths that were skipped.
pub fn unpack_tar(r: &mut dyn Read, out: &Path, tick: &mut dyn FnMut() -> bool) -> io::Result<(usize, Vec<String>)> {
    let (mut files, mut skipped) = (0usize, Vec::new());
    let mut buf = vec![0u8; CHUNK];
    read_tar(r, &mut |e, data| {
        let Some(rel) = zip::safe_rel(&e.path) else {
            skipped.push(e.path.clone());
            return Ok(());
        };
        let target = out.join(&rel);
        match e.kind {
            EntryKind::Dir => std::fs::create_dir_all(&target)?,
            EntryKind::Other => skipped.push(e.path.clone()),
            EntryKind::File => {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut f = File::create(&target)?;
                loop {
                    let n = data.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    f.write_all(&buf[..n])?;
                    if !tick() {
                        return Err(cancelled());
                    }
                }
                set_mode(&target, e.mode);
                files += 1;
            }
        }
        Ok(())
    })?;
    Ok((files, skipped))
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    // keep the owner's read and write bits so the file stays usable even when the archive says otherwise
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode | 0o600));
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) {}

// ------------------------------------------------------------------ jobs

/// Reads the raw bytes of a file while counting them, so progress tracks the file, not the decoded stream.
struct Counted<R: Read> {
    inner: R,
    n: Rc<Cell<u64>>,
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let k = self.inner.read(buf)?;
        self.n.set(self.n.get() + k as u64);
        Ok(k)
    }
}

/// Expands a tar or gzipped tar file beside itself into a new folder.
fn expand_tar(path: &Path, kind: Kind, progress: &mut dyn FnMut(u64, u64) -> bool) -> io::Result<Outcome> {
    let file = File::open(path)?;
    let total = file.metadata()?.len();
    let raw = Rc::new(Cell::new(0u64));
    let counted = Counted { inner: BufReader::new(file), n: raw.clone() };
    let mut reader: Box<dyn Read> = match kind {
        Kind::TarGz => Box::new(GzDecoder::new(counted)),
        _ => Box::new(counted),
    };
    let parent = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let out = unique_path(parent, &stem_of(&name), "");
    std::fs::create_dir(&out)?;
    let mut tick = || progress(raw.get(), total);
    match unpack_tar(&mut *reader, &out, &mut tick) {
        Ok((files, skipped)) => Ok(Outcome { path: out, files, skipped: skipped.len() }),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&out);
            Err(e)
        }
    }
}

/// Runs one job. Expand unpacks the archive at `path`; Compress zips the folder at `path` into its parent folder.
pub fn run(op: Op, path: &Path, progress: &mut dyn FnMut(u64, u64) -> bool) -> io::Result<Outcome> {
    match op {
        Op::Compress => {
            let parent = path.parent().unwrap_or(Path::new(".")).to_path_buf();
            let made = zip::create(&[path.to_path_buf()], &parent, progress)?;
            Ok(Outcome { path: made, files: 0, skipped: 0 })
        }
        Op::Expand => {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            match kind_of(&name) {
                Some(Kind::Zip) => {
                    let parent = path.parent().unwrap_or(Path::new(".")).to_path_buf();
                    let done = zip::extract(path, &parent, progress)?;
                    Ok(Outcome { path: done.dir, files: done.files, skipped: done.skipped.len() })
                }
                Some(k @ (Kind::Tar | Kind::TarGz)) => expand_tar(path, k, progress),
                Some(Kind::TarXz) => Err(io::Error::new(io::ErrorKind::Unsupported, "XZ archives are not supported yet")),
                None => Err(io::Error::new(io::ErrorKind::InvalidInput, "Not an archive")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Cursor;

    /// One ustar header block for `name` with `size` bytes of data and type flag `flag`.
    fn header(name: &str, size: u64, flag: u8, mode: u32) -> Vec<u8> {
        let mut h = vec![0u8; BLOCK];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[100..107].copy_from_slice(format!("{mode:07o}").as_bytes());
        h[124..135].copy_from_slice(format!("{size:011o}").as_bytes());
        h[156] = flag;
        h[257..263].copy_from_slice(b"ustar\0");
        h[148..156].copy_from_slice(b"        ");
        let sum = checksum(&h);
        h[148..154].copy_from_slice(format!("{sum:06o}").as_bytes());
        h[154] = 0;
        h[155] = b' ';
        h
    }

    fn member(out: &mut Vec<u8>, name: &str, data: &[u8], flag: u8, mode: u32) {
        out.extend(header(name, data.len() as u64, flag, mode));
        out.extend_from_slice(data);
        out.resize(out.len() + padding(data.len() as u64) as usize, 0);
    }

    fn sample_tar() -> Vec<u8> {
        let mut t = vec![];
        member(&mut t, "top/", &[], b'5', 0o755);
        member(&mut t, "top/hello.txt", b"hello there", b'0', 0o644);
        member(&mut t, "top/run.sh", b"#!/bin/sh\n", b'0', 0o755);
        member(&mut t, "top/link", b"", b'2', 0o777);
        member(&mut t, "../escape.txt", b"no", b'0', 0o644);
        t.extend_from_slice(&[0u8; BLOCK * 2]);
        t
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("archive-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn kinds_and_stems_follow_the_name() {
        assert_eq!(kind_of("a.ZIP"), Some(Kind::Zip));
        assert_eq!(kind_of("a.tar"), Some(Kind::Tar));
        assert_eq!(kind_of("a.tar.gz"), Some(Kind::TarGz));
        assert_eq!(kind_of("a.tgz"), Some(Kind::TarGz));
        assert_eq!(kind_of("a.tar.xz"), Some(Kind::TarXz));
        assert_eq!(kind_of("a.txt"), None);
        assert_eq!(stem_of("photos.tar.gz"), "photos");
        assert_eq!(stem_of("photos.TGZ"), "photos");
        assert_eq!(stem_of("data.zip"), "data");
        assert_eq!(stem_of(".zip"), ".zip");
    }

    #[test]
    fn unique_paths_number_taken_names() {
        let dir = scratch("unique");
        std::fs::write(dir.join("Notes.zip"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "Notes", ".zip"), dir.join("Notes 2.zip"));
        std::fs::create_dir(dir.join("Notes 2.zip")).unwrap();
        assert_eq!(unique_path(&dir, "Notes", ".zip"), dir.join("Notes 3.zip"));
        assert_eq!(unique_path(&dir, "Fresh", ""), dir.join("Fresh"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tar_entries_are_read_in_order_with_modes_and_types() {
        let mut seen = vec![];
        read_tar(&mut Cursor::new(sample_tar()), &mut |e, data| {
            let mut body = vec![];
            data.read_to_end(&mut body)?;
            seen.push((e.path.clone(), e.kind, e.mode, body));
            Ok(())
        })
        .unwrap();
        assert_eq!(seen[0], ("top/".to_string(), EntryKind::Dir, 0o755, vec![]));
        assert_eq!(seen[1].0, "top/hello.txt");
        assert_eq!(seen[1].3, b"hello there".to_vec());
        assert_eq!(seen[2].2, 0o755);
        assert_eq!(seen[3].1, EntryKind::Other);
        assert_eq!(seen.len(), 5);
    }

    #[test]
    fn long_names_from_pax_and_gnu_headers_apply_to_the_next_member() {
        let long = format!("{}/deep.txt", "d".repeat(150));
        let body = format!(" path={long}\n");
        let mut len = body.len();
        loop {
            let next = len.to_string().len() + body.len();
            if next == len {
                break;
            }
            len = next;
        }
        let pax = format!("{len}{body}");
        let mut t = vec![];
        member(&mut t, "PaxHeader", pax.as_bytes(), b'x', 0o644);
        member(&mut t, "short", b"A", b'0', 0o644);
        let mut gnu = long.clone().into_bytes();
        gnu.push(0);
        member(&mut t, "././@LongLink", &gnu, b'L', 0o644);
        member(&mut t, "truncated", b"B", b'0', 0o644);
        t.extend_from_slice(&[0u8; BLOCK * 2]);
        let mut names = vec![];
        read_tar(&mut Cursor::new(t), &mut |e, _| {
            names.push(e.path.clone());
            Ok(())
        })
        .unwrap();
        assert_eq!(names, vec![long.clone(), long.clone()]);
    }

    #[test]
    fn checksum_damage_is_refused() {
        let mut t = sample_tar();
        t[10] ^= 0x55;
        let r = read_tar(&mut Cursor::new(t), &mut |_, _| Ok(()));
        assert!(r.is_err());
    }

    #[test]
    fn unpack_writes_files_and_skips_unsafe_or_linked_members() {
        let dir = scratch("unpack");
        let mut calls = 0;
        let (files, skipped) = unpack_tar(&mut Cursor::new(sample_tar()), &dir, &mut || {
            calls += 1;
            true
        })
        .unwrap();
        assert_eq!(files, 2);
        assert_eq!(skipped, vec!["top/link".to_string(), "../escape.txt".to_string()]);
        assert_eq!(std::fs::read(dir.join("top/hello.txt")).unwrap(), b"hello there");
        assert!(calls > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unpack_stops_when_the_tick_cancels() {
        let dir = scratch("cancel");
        let r = unpack_tar(&mut Cursor::new(sample_tar()), &dir, &mut || false);
        assert_eq!(r.unwrap_err().kind(), io::ErrorKind::Interrupted);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gzipped_tar_decodes_through_the_same_reader() {
        let mut gz = GzEncoder::new(vec![], Compression::fast());
        gz.write_all(&sample_tar()).unwrap();
        let bytes = gz.finish().unwrap();
        let mut reader = GzDecoder::new(Cursor::new(bytes));
        let dir = scratch("gz");
        let (files, _) = unpack_tar(&mut reader, &dir, &mut || true).unwrap();
        assert_eq!(files, 2);
        assert_eq!(std::fs::read(dir.join("top/run.sh")).unwrap(), b"#!/bin/sh\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn zip_members_round_trip_through_the_shell_reader() {
        let mut w = zip::Writer::new(vec![]);
        w.add_dir("pics/", 0).unwrap();
        w.add_bytes("pics/one.txt", b"first", 0).unwrap();
        let bytes = w.finish().unwrap();
        let mut ar = zip::Archive::open(Cursor::new(bytes)).unwrap();
        let members = ar.members.clone();
        assert_eq!(members.len(), 2);
        assert!(members[0].is_dir);
        assert_eq!(ar.read(&members[1]).unwrap(), b"first".to_vec());
    }

    #[test]
    fn xz_and_plain_files_are_refused_with_a_reason() {
        let dir = scratch("refuse");
        let xz = dir.join("a.tar.xz");
        std::fs::write(&xz, b"junk").unwrap();
        let e = run(Op::Expand, &xz, &mut |_, _| true).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::Unsupported);
        let txt = dir.join("a.txt");
        std::fs::write(&txt, b"x").unwrap();
        assert_eq!(run(Op::Expand, &txt, &mut |_, _| true).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
