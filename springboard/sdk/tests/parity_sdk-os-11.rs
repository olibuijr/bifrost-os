//! Parity sdk-os-11: headless tests of the pipe constructor and the system error numbers apps get.
//! They create in-process pipes and read errno values; no test touches a device or a network.

use bifrost_app::errno::Errno;
use bifrost_app::file_descriptor::{Access, FileDescriptor, FileError, OpenFlags};
use std::io;
use std::path::Path;

#[test]
fn a_pipe_is_a_read_end_and_a_write_end() {
    let (reader, writer) = FileDescriptor::pipe().expect("the system can make a pipe");
    assert_eq!(reader.access(), Access::Read);
    assert_eq!(writer.access(), Access::Write);
}

#[test]
fn a_pipe_round_trips_a_message() {
    let (mut reader, writer) = FileDescriptor::pipe().expect("the system can make a pipe");
    writer.write(b"hello").expect("write");
    drop(writer);
    let mut buf = [0u8; 16];
    let n = reader.read(&mut buf).expect("read");
    assert_eq!(&buf[..n], b"hello");
}

#[test]
fn a_missing_file_maps_to_the_noent_errno() {
    let err = FileDescriptor::open(
        Path::new("/nonexistent/bifrost/sdk-os-11"),
        Access::Read,
        OpenFlags::default(),
    )
    .unwrap_err();
    match err {
        FileError::Io(io_err) => assert_eq!(Errno::from_io(&io_err), Some(Errno::NOENT)),
        other => panic!("expected an I/O error, got {other}"),
    }
}

#[test]
fn an_errno_names_itself_and_converts_to_an_io_kind() {
    assert_eq!(Errno::NOENT.name(), Some("ENOENT"));
    assert_eq!(Errno::NOENT.to_io_kind(), io::ErrorKind::NotFound);
}
