//! Parity sdk-tests-12: headless tests of the ustar reader that opens app packages. Each test builds its archive in
//! memory with the header layout, so no file, network or device is involved.

use springboard::shell::store::install::{index_tar, read_tar, TarEntry};
use std::io::Cursor;
use std::path::PathBuf;

const BLOCK: usize = 512;

/// One archive entry: its name, its type flag, its mode and its contents.
struct Entry<'a> {
    name: &'a str,
    kind: u8,
    mode: u32,
    body: &'a [u8],
}

/// A numeric header field: zero-padded octal digits and a terminating NUL, `len` bytes in all.
fn octal_field(value: u64, len: usize) -> Vec<u8> {
    let mut field = format!("{:0width$o}", value, width = len - 1).into_bytes();
    field.push(0);
    field
}

fn header(name: &str, size: u64, mode: u32, kind: u8) -> [u8; BLOCK] {
    let mut h = [0u8; BLOCK];
    h[..name.len()].copy_from_slice(name.as_bytes());
    h[100..108].copy_from_slice(&octal_field(u64::from(mode), 8));
    h[108..116].copy_from_slice(&octal_field(0, 8));
    h[116..124].copy_from_slice(&octal_field(0, 8));
    h[124..136].copy_from_slice(&octal_field(size, 12));
    h[136..148].copy_from_slice(&octal_field(0, 12));
    h[148..156].copy_from_slice(b"        ");
    h[156] = kind;
    h[257..263].copy_from_slice(b"ustar\0");
    let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    h
}

/// Lays out the entries, each body padded to whole blocks, and ends the archive with two empty blocks.
fn archive(entries: &[Entry]) -> Vec<u8> {
    let mut out = Vec::new();
    for e in entries {
        out.extend_from_slice(&header(e.name, e.body.len() as u64, e.mode, e.kind));
        out.extend_from_slice(e.body);
        let pad = (BLOCK - e.body.len() % BLOCK) % BLOCK;
        out.resize(out.len() + pad, 0);
    }
    out.resize(out.len() + 2 * BLOCK, 0);
    out
}

fn two_entry_package() -> Vec<u8> {
    archive(&[
        Entry { name: "notes/", kind: b'5', mode: 0o755, body: b"" },
        Entry { name: "notes/readme.txt", kind: b'0', mode: 0o644, body: b"hello" },
    ])
}

#[test]
fn headless_a_tar_entry_keeps_its_path_mode_directory_flag_and_contents() {
    let files: Vec<TarEntry> = read_tar(&two_entry_package()).unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].path, PathBuf::from("notes"), "the trailing slash of a directory is dropped");
    assert!(files[0].is_dir);
    assert_eq!(files[0].mode, 0o755);
    assert_eq!(files[1].path, PathBuf::from("notes/readme.txt"));
    assert!(!files[1].is_dir);
    assert_eq!(files[1].mode, 0o644);
    assert_eq!(files[1].size, 5);
    assert_eq!(files[1].data, b"hello".to_vec());
}

#[test]
fn headless_the_contents_offset_points_past_the_header_block() {
    let files = read_tar(&two_entry_package()).unwrap();
    assert_eq!(files[0].offset, 512, "the directory has no contents, so the next header follows at block 1");
    assert_eq!(files[1].offset, 1024, "the file header is block 1, its contents start at block 2");
    let bytes = two_entry_package();
    assert_eq!(&bytes[1024..1029], b"hello");
}

#[test]
fn headless_an_index_reads_headers_and_leaves_the_contents_out() {
    let bytes = two_entry_package();
    let indexed = index_tar(&mut Cursor::new(bytes.clone())).unwrap();
    let loaded = read_tar(&bytes).unwrap();
    assert_eq!(indexed.len(), 2);
    assert!(indexed[1].data.is_empty(), "an index holds no contents");
    assert_eq!((indexed[1].offset, indexed[1].size), (loaded[1].offset, loaded[1].size));
    assert_eq!(indexed[1].mode, 0o644);
}

#[test]
fn headless_an_archive_with_an_absolute_or_escaping_path_is_refused() {
    let escape = archive(&[Entry { name: "../escape.txt", kind: b'0', mode: 0o644, body: b"x" }]);
    assert!(read_tar(&escape).is_err());
    let absolute = archive(&[Entry { name: "/etc/passwd", kind: b'0', mode: 0o644, body: b"x" }]);
    assert!(read_tar(&absolute).is_err());
}

#[test]
fn headless_links_and_devices_make_the_whole_package_invalid() {
    let link = archive(&[Entry { name: "a-link", kind: b'2', mode: 0o777, body: b"" }]);
    let err = read_tar(&link).unwrap_err();
    assert!(err.contains("unsupported tar entry type"), "{err}");
}

#[test]
fn headless_extended_pax_headers_are_refused() {
    let pax = archive(&[Entry { name: "PaxHeader", kind: b'x', mode: 0o644, body: b"30 path=a\n" }]);
    let err = read_tar(&pax).unwrap_err();
    assert!(err.contains("pax"), "{err}");
}

#[test]
fn headless_a_path_that_appears_twice_is_refused() {
    let twice = archive(&[
        Entry { name: "a.txt", kind: b'0', mode: 0o644, body: b"one" },
        Entry { name: "a.txt", kind: b'0', mode: 0o644, body: b"two" },
    ]);
    let err = read_tar(&twice).unwrap_err();
    assert!(err.contains("duplicate"), "{err}");
}

#[test]
fn headless_an_archive_without_an_end_marker_is_refused() {
    let open = header("a.txt", 0, 0o644, b'0').to_vec();
    let err = read_tar(&open).unwrap_err();
    assert!(err.contains("end marker"), "{err}");
}

#[test]
fn headless_a_header_whose_checksum_is_wrong_is_refused() {
    let mut bytes = two_entry_package();
    bytes[0] ^= 0x01;
    let err = read_tar(&bytes).unwrap_err();
    assert!(err.contains("checksum"), "{err}");
}
