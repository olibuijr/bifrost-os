//! Parity sdk-complete-36: headless tests for the typed file descriptor. Each test works on its own file under the
//! crate's target temporary directory, never under a shared system directory.

use bifrost_app::file_descriptor::{Access, FileDescriptor, FileError, OpenFlags};
use std::io::SeekFrom;
use std::path::PathBuf;

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("sdk-complete-36-fd-{name}"))
}

fn create_read_write() -> OpenFlags {
    OpenFlags { create: true, truncate: true, append: false }
}

#[test]
fn bytes_written_through_a_descriptor_read_back_after_a_seek() {
    let path = scratch("round-trip");
    let mut fd = FileDescriptor::open(&path, Access::ReadWrite, create_read_write()).unwrap();
    assert_eq!(fd.write(b"hello").unwrap(), 5);
    assert_eq!(fd.seek(SeekFrom::Start(0)).unwrap(), 0);
    let mut buf = [0u8; 5];
    assert_eq!(fd.read(&mut buf).unwrap(), 5);
    assert_eq!(&buf, b"hello");
}

#[test]
fn positional_io_reads_and_writes_at_an_offset_without_moving_the_position() {
    let path = scratch("positional");
    let fd = FileDescriptor::open(&path, Access::ReadWrite, create_read_write()).unwrap();
    assert_eq!(fd.write_at(b"ab", 4).unwrap(), 2);
    let mut buf = [0u8; 2];
    assert_eq!(fd.read_at(&mut buf, 4).unwrap(), 2);
    assert_eq!(&buf, b"ab");
    let mut mover = fd;
    assert_eq!(mover.seek(SeekFrom::Current(0)).unwrap(), 0, "positional calls leave the offset at zero");
}

#[test]
fn seeking_from_the_end_reports_the_file_length_minus_the_offset() {
    let path = scratch("seek-end");
    let mut fd = FileDescriptor::open(&path, Access::ReadWrite, create_read_write()).unwrap();
    fd.write(b"abcdef").unwrap();
    assert_eq!(fd.seek(SeekFrom::End(-2)).unwrap(), 4);
    let mut buf = [0u8; 2];
    fd.read(&mut buf).unwrap();
    assert_eq!(&buf, b"ef");
}

#[test]
fn a_read_only_descriptor_refuses_writes_before_touching_the_file() {
    let path = scratch("read-only");
    std::fs::write(&path, b"keep").unwrap();
    let mut fd = FileDescriptor::open(&path, Access::Read, OpenFlags::default()).unwrap();
    assert!(matches!(fd.write(b"x"), Err(FileError::NotWritable)));
    assert!(matches!(fd.write_at(b"x", 0), Err(FileError::NotWritable)));
    assert_eq!(std::fs::read(&path).unwrap(), b"keep");
}

#[test]
fn a_write_only_descriptor_refuses_reads() {
    let path = scratch("write-only");
    let mut fd = FileDescriptor::open(&path, Access::Write, create_read_write()).unwrap();
    let mut buf = [0u8; 1];
    assert!(matches!(fd.read(&mut buf), Err(FileError::NotReadable)));
    assert!(matches!(fd.read_at(&mut buf, 0), Err(FileError::NotReadable)));
}

#[test]
fn opening_a_missing_file_without_create_reports_not_found() {
    let path = scratch("missing");
    let _ = std::fs::remove_file(&path);
    match FileDescriptor::open(&path, Access::Read, OpenFlags::default()) {
        Err(FileError::Io(error)) => assert_eq!(error.kind(), std::io::ErrorKind::NotFound),
        other => panic!("expected a not-found error, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn create_truncate_and_append_need_write_access() {
    let path = scratch("invalid-options");
    let truncate = OpenFlags { create: false, truncate: true, append: false };
    assert!(matches!(FileDescriptor::open(&path, Access::Read, truncate), Err(FileError::InvalidOptions)));
    let create = OpenFlags { create: true, truncate: false, append: false };
    assert!(matches!(FileDescriptor::open(&path, Access::Read, create), Err(FileError::InvalidOptions)));
    let append = OpenFlags { create: false, truncate: false, append: true };
    assert!(matches!(FileDescriptor::open(&path, Access::Read, append), Err(FileError::InvalidOptions)));
}

#[test]
fn an_append_descriptor_writes_at_the_end_whatever_the_position() {
    let path = scratch("append");
    std::fs::write(&path, b"one").unwrap();
    let append = OpenFlags { create: false, truncate: false, append: true };
    let mut fd = FileDescriptor::open(&path, Access::Write, append).unwrap();
    fd.seek(SeekFrom::Start(0)).unwrap();
    fd.write(b"two").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"onetwo");
}

#[test]
fn a_duplicate_has_its_own_number_and_shares_the_offset() {
    let path = scratch("duplicate");
    let mut original = FileDescriptor::open(&path, Access::ReadWrite, create_read_write()).unwrap();
    original.write(b"0123456789").unwrap();
    let mut copy = original.duplicate().unwrap();
    assert_ne!(copy.raw(), original.raw());
    copy.seek(SeekFrom::Start(7)).unwrap();
    assert_eq!(original.seek(SeekFrom::Current(0)).unwrap(), 7, "the duplicate moves the shared offset");
}

#[test]
fn a_duplicate_keeps_the_access_of_its_original() {
    let path = scratch("duplicate-access");
    std::fs::write(&path, b"data").unwrap();
    let original = FileDescriptor::open(&path, Access::Read, OpenFlags::default()).unwrap();
    let mut copy = original.duplicate().unwrap();
    assert!(matches!(copy.write(b"x"), Err(FileError::NotWritable)));
}
