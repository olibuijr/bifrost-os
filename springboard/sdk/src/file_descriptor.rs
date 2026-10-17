//! Typed file descriptors: a file opened with an explicit access mode and open flags, read and written at the
//! current position or at an offset, seeked, and duplicated. An app holds its files through this type instead of
//! a raw integer. The access mode is checked before the system call, so a read-only descriptor refuses a write
//! without touching the file. The descriptor closes when it is dropped. Pipes come from `FileDescriptor::pipe`.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::FileExt;
use std::os::unix::io::{AsRawFd, OwnedFd, RawFd};
use std::path::Path;

/// What a descriptor may do with its file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
    ReadWrite,
}

impl Access {
    fn reads(self) -> bool {
        matches!(self, Access::Read | Access::ReadWrite)
    }

    fn writes(self) -> bool {
        matches!(self, Access::Write | Access::ReadWrite)
    }
}

/// The flags that change how a file is opened. Each of them needs write access.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpenFlags {
    /// Create the file when it does not exist.
    pub create: bool,
    /// Cut the file to zero length when it is opened.
    pub truncate: bool,
    /// Send every write to the end of the file, whatever the position.
    pub append: bool,
}

/// Why a descriptor operation is refused.
#[derive(Debug)]
pub enum FileError {
    /// The flags need write access, but the descriptor is read-only.
    InvalidOptions,
    /// The descriptor was opened without read access.
    NotReadable,
    /// The descriptor was opened without write access.
    NotWritable,
    /// The system call failed.
    Io(io::Error),
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FileError::InvalidOptions => f.write_str("the open flags need write access"),
            FileError::NotReadable => f.write_str("the file was not opened for reading"),
            FileError::NotWritable => f.write_str("the file was not opened for writing"),
            FileError::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for FileError {}

/// An open file with a known access mode.
#[derive(Debug)]
pub struct FileDescriptor {
    access: Access,
    file: File,
}

impl FileDescriptor {
    /// Opens `path` with the given access and flags. Flags that create, cut or append need write access.
    pub fn open(path: &Path, access: Access, flags: OpenFlags) -> Result<Self, FileError> {
        let changes_file = flags.create || flags.truncate || flags.append;
        if changes_file && !access.writes() {
            return Err(FileError::InvalidOptions);
        }
        let file = OpenOptions::new()
            .read(access.reads())
            .write(access.writes())
            .create(flags.create)
            .truncate(flags.truncate)
            .append(flags.append)
            .open(path)
            .map_err(FileError::Io)?;
        Ok(FileDescriptor { access, file })
    }

    /// The access mode the descriptor was opened with.
    pub fn access(&self) -> Access {
        self.access
    }

    /// Reads into `buf` at the current position, which moves forward by the bytes read.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, FileError> {
        self.check_readable()?;
        self.file.read(buf).map_err(FileError::Io)
    }

    /// Writes `data` at the current position, which moves forward by the bytes written.
    pub fn write(&mut self, data: &[u8]) -> Result<usize, FileError> {
        self.check_writable()?;
        self.file.write(data).map_err(FileError::Io)
    }

    /// Reads into `buf` from `offset`. The current position does not move.
    pub fn read_at(&self, buf: &mut [u8], offset: u64) -> Result<usize, FileError> {
        self.check_readable()?;
        self.file.read_at(buf, offset).map_err(FileError::Io)
    }

    /// Writes `data` at `offset`. The current position does not move.
    pub fn write_at(&self, data: &[u8], offset: u64) -> Result<usize, FileError> {
        self.check_writable()?;
        self.file.write_at(data, offset).map_err(FileError::Io)
    }

    /// Moves the current position and returns the new position from the start of the file.
    pub fn seek(&mut self, position: SeekFrom) -> Result<u64, FileError> {
        self.file.seek(position).map_err(FileError::Io)
    }

    /// A second descriptor for the same open file. Both share the current position, as the system's duplicate does.
    pub fn duplicate(&self) -> Result<Self, FileError> {
        let file = self.file.try_clone().map_err(FileError::Io)?;
        Ok(FileDescriptor { access: self.access, file })
    }

    /// Creates a pipe. The first descriptor reads and the second writes; bytes written to one come out of the other
    /// in order. Closing the writer makes reads return end of file once the pipe is drained.
    pub fn pipe() -> Result<(FileDescriptor, FileDescriptor), FileError> {
        let (reader, writer) = std::io::pipe().map_err(FileError::Io)?;
        let reader = FileDescriptor { access: Access::Read, file: File::from(OwnedFd::from(reader)) };
        let writer = FileDescriptor { access: Access::Write, file: File::from(OwnedFd::from(writer)) };
        Ok((reader, writer))
    }

    /// The system's number for this descriptor, for a caller that hands it to a child process or a system call.
    pub fn raw(&self) -> RawFd {
        self.file.as_raw_fd()
    }

    fn check_readable(&self) -> Result<(), FileError> {
        if self.access.reads() {
            Ok(())
        } else {
            Err(FileError::NotReadable)
        }
    }

    fn check_writable(&self) -> Result<(), FileError> {
        if self.access.writes() {
            Ok(())
        } else {
            Err(FileError::NotWritable)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pipe_carries_bytes_from_the_writer_to_the_reader_in_order() {
        let (mut reader, writer) = FileDescriptor::pipe().expect("the system can make a pipe");
        assert_eq!(reader.access(), Access::Read);
        assert_eq!(writer.access(), Access::Write);
        assert_eq!(writer.write(b"abc").expect("write"), 3);
        assert_eq!(writer.write(b"de").expect("write"), 2);
        drop(writer);
        let mut out = Vec::new();
        let mut buf = [0u8; 8];
        loop {
            let n = reader.read(&mut buf).expect("read");
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        assert_eq!(out, b"abcde");
    }

    #[test]
    fn a_pipe_end_refuses_the_direction_it_does_not_have() {
        let (mut reader, mut writer) = FileDescriptor::pipe().expect("the system can make a pipe");
        assert!(matches!(writer.read(&mut [0u8; 1]), Err(FileError::NotReadable)));
        assert!(matches!(reader.write(b"x"), Err(FileError::NotWritable)));
        assert!(matches!(reader.write_at(b"x", 0), Err(FileError::NotWritable)));
    }

    #[test]
    fn a_duplicated_pipe_end_still_reaches_the_same_pipe() {
        let (mut reader, writer) = FileDescriptor::pipe().expect("the system can make a pipe");
        let second = writer.duplicate().expect("duplicate the writer");
        second.write(b"z").expect("write through the copy");
        drop(writer);
        drop(second);
        let mut buf = [0u8; 4];
        assert_eq!(reader.read(&mut buf).expect("read"), 1);
        assert_eq!(buf[0], b'z');
    }
}
