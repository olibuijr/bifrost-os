//! Typed file descriptors: a file opened with an explicit access mode and open flags, read and written at the
//! current position or at an offset, seeked, and duplicated. An app holds its files through this type instead of
//! a raw integer. The access mode is checked before the system call, so a read-only descriptor refuses a write
//! without touching the file. The descriptor closes when it is dropped; pipes are not created here.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::FileExt;
use std::os::unix::io::{AsRawFd, RawFd};
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
