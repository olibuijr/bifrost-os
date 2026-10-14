//! Parity sdk-rest-04: headless tests for the tar package reader (`tar_package`). Each file-system test works under the
//! crate's target temporary directory.

use bifrost_app::tar_package::{self, Entry};
use std::io::{Cursor, ErrorKind};
use std::path::{Path, PathBuf};

const FILE: u8 = b'0';
const FOLDER: u8 = b'5';
const SYMLINK: u8 = b'2';

/// One 512-byte ustar header. The checksum is the sum of every byte with the checksum field read as spaces.
fn header(name: &str, size: usize, kind: u8, mode: u32) -> [u8; 512] {
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    h[100..108].copy_from_slice(format!("{mode:07o}\0").as_bytes());
    h[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
    h[148..156].copy_from_slice(b"        ");
    h[156] = kind;
    h[257..262].copy_from_slice(b"ustar");
    let sum: u32 = h.iter().map(|&b| b as u32).sum();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    h
}

/// A complete package: each member is a header, its contents padded to 512 bytes, then two zero blocks.
fn package(members: &[(&str, u8, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, kind, data) in members {
        let mode = if *kind == FOLDER { 0o755 } else { 0o644 };
        out.extend_from_slice(&header(name, data.len(), *kind, mode));
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.extend_from_slice(&[0u8; 1024]);
    out
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("sdk-rest-04-tar-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn sample() -> Vec<u8> {
    package(&[("app", FOLDER, b""), ("app/run", FILE, b"hello"), ("app/data/notes.txt", FILE, b"0123456789")])
}

#[test]
fn read_returns_every_member_in_archive_order_with_its_contents() {
    let entries = tar_package::read(&sample()).unwrap();
    let paths: Vec<&Path> = entries.iter().map(|e| e.path.as_path()).collect();
    assert_eq!(paths, [Path::new("app"), Path::new("app/run"), Path::new("app/data/notes.txt")]);
    assert!(entries[0].is_dir && entries[0].contents.is_empty() && entries[0].size == 0);
    assert_eq!(entries[1].contents, b"hello");
    assert_eq!(entries[1].size, 5);
    assert_eq!(entries[2].contents, b"0123456789");
    assert_eq!(entries[1].mode, 0o644);
}

#[test]
fn index_reads_the_headers_and_leaves_the_contents_unloaded() {
    let mut file = Cursor::new(sample());
    let entries = tar_package::index(&mut file).unwrap();
    assert_eq!(entries.len(), 3);
    assert!(entries.iter().all(|e| e.contents.is_empty()), "an index holds no file data");
    assert_eq!(entries[1].offset, 1024, "the folder header and the run header come first");
    assert_eq!(entries[1].size, 5);
    assert_eq!(entries[2].offset, 2048, "run's contents fill one padded block, then notes has its own header");
}

#[test]
fn index_and_read_agree_on_every_header_field() {
    let bytes = sample();
    let read = tar_package::read(&bytes).unwrap();
    let indexed = tar_package::index(&mut Cursor::new(bytes)).unwrap();
    let strip = |e: &Entry| Entry { contents: Vec::new(), ..e.clone() };
    let expected: Vec<Entry> = read.iter().map(strip).collect();
    assert_eq!(indexed, expected);
}

#[test]
fn a_package_with_a_path_that_climbs_out_is_refused() {
    let bytes = package(&[("../escape", FILE, b"x")]);
    let err = tar_package::read(&bytes).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);
}

#[test]
fn a_package_with_an_absolute_path_is_refused() {
    let bytes = package(&[("/etc/passwd", FILE, b"x")]);
    assert_eq!(tar_package::read(&bytes).unwrap_err().kind(), ErrorKind::InvalidData);
}

#[test]
fn a_symbolic_link_member_is_refused_as_an_unsupported_entry() {
    let bytes = package(&[("app/link", SYMLINK, b"")]);
    let err = tar_package::index(&mut Cursor::new(bytes)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    assert!(err.to_string().contains("unsupported tar entry type"), "{err}");
}

#[test]
fn a_damaged_header_fails_its_checksum() {
    let mut bytes = sample();
    bytes[0] ^= 1;
    let err = tar_package::read(&bytes).unwrap_err();
    assert_eq!(err.to_string(), "tar header checksum is wrong");
}

#[test]
fn a_package_without_its_end_marker_is_refused() {
    let mut bytes = sample();
    bytes.truncate(bytes.len() - 1024);
    let err = tar_package::read(&bytes).unwrap_err();
    assert_eq!(err.to_string(), "tar ends without an end marker");
}

#[test]
fn a_path_listed_twice_is_refused_as_a_duplicate() {
    let bytes = package(&[("app/run", FILE, b"a"), ("app/run", FILE, b"b")]);
    let err = tar_package::read(&bytes).unwrap_err();
    assert!(err.to_string().contains("duplicate path"), "{err}");
}

#[test]
fn extract_from_writes_the_files_and_folders_below_the_destination() {
    let folder = scratch("extract");
    let bytes = sample();
    let mut file = Cursor::new(bytes);
    let entries = tar_package::index(&mut file).unwrap();
    tar_package::extract_from(&mut file, &entries, &folder).unwrap();
    assert_eq!(std::fs::read(folder.join("app/run")).unwrap(), b"hello");
    assert_eq!(std::fs::read(folder.join("app/data/notes.txt")).unwrap(), b"0123456789");
    assert!(folder.join("app/data").is_dir());
}

#[test]
fn extract_from_refuses_a_hand_built_entry_that_climbs_out_and_writes_nothing() {
    let folder = scratch("climb");
    let bytes = sample();
    let mut file = Cursor::new(bytes);
    let forged = Entry { path: PathBuf::from("../escaped.txt"), is_dir: false, mode: 0o644, size: 5, offset: 1024, contents: Vec::new() };
    let err = tar_package::extract_from(&mut file, &[forged], &folder).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    assert!(!folder.parent().unwrap().join("escaped.txt").exists());
}

#[test]
fn extract_from_keeps_a_leading_dot_member_inside_the_destination() {
    let folder = scratch("dot");
    let bytes = package(&[("./readme.txt", FILE, b"inside")]);
    let mut file = Cursor::new(bytes);
    let entries = tar_package::index(&mut file).unwrap();
    assert_eq!(entries[0].path, Path::new("readme.txt"));
    tar_package::extract_from(&mut file, &entries, &folder).unwrap();
    assert_eq!(std::fs::read(folder.join("readme.txt")).unwrap(), b"inside");
}
