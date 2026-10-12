//! Typed system error numbers (std only).
//!
//! A system call reports a failure as a small integer. This module gives that integer a type,
//! names the common values with the names the platform headers use, prints a short description
//! for each, and maps the value to the closest `std::io::ErrorKind`. The numbers are the Linux
//! values. It makes no system call.

use std::fmt;
use std::io;

/// A system error number, as stored in `errno`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Errno(pub i32);

impl Errno {
    pub const PERM: Errno = Errno(1);
    pub const NOENT: Errno = Errno(2);
    pub const SRCH: Errno = Errno(3);
    pub const INTR: Errno = Errno(4);
    pub const IO: Errno = Errno(5);
    pub const BADF: Errno = Errno(9);
    pub const AGAIN: Errno = Errno(11);
    pub const NOMEM: Errno = Errno(12);
    pub const ACCES: Errno = Errno(13);
    pub const BUSY: Errno = Errno(16);
    pub const EXIST: Errno = Errno(17);
    pub const NOTDIR: Errno = Errno(20);
    pub const ISDIR: Errno = Errno(21);
    pub const INVAL: Errno = Errno(22);
    pub const MFILE: Errno = Errno(24);
    pub const FBIG: Errno = Errno(27);
    pub const NOSPC: Errno = Errno(28);
    pub const SPIPE: Errno = Errno(29);
    pub const ROFS: Errno = Errno(30);
    pub const PIPE: Errno = Errno(32);
    pub const NAMETOOLONG: Errno = Errno(36);
    pub const NOTEMPTY: Errno = Errno(39);
    pub const LOOP: Errno = Errno(40);
    pub const TIMEDOUT: Errno = Errno(110);
    pub const CONNREFUSED: Errno = Errno(111);

    /// The platform name, such as `ENOENT`, or `None` when the value is not in the table.
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            1 => "EPERM",
            2 => "ENOENT",
            3 => "ESRCH",
            4 => "EINTR",
            5 => "EIO",
            9 => "EBADF",
            11 => "EAGAIN",
            12 => "ENOMEM",
            13 => "EACCES",
            16 => "EBUSY",
            17 => "EEXIST",
            20 => "ENOTDIR",
            21 => "EISDIR",
            22 => "EINVAL",
            24 => "EMFILE",
            27 => "EFBIG",
            28 => "ENOSPC",
            29 => "ESPIPE",
            30 => "EROFS",
            32 => "EPIPE",
            36 => "ENAMETOOLONG",
            39 => "ENOTEMPTY",
            40 => "ELOOP",
            110 => "ETIMEDOUT",
            111 => "ECONNREFUSED",
            _ => return None,
        })
    }

    /// A short English description of the value, or `None` when it is not in the table.
    pub fn message(self) -> Option<&'static str> {
        Some(match self.0 {
            1 => "Operation not permitted",
            2 => "No such file or directory",
            3 => "No such process",
            4 => "Interrupted system call",
            5 => "Input/output error",
            9 => "Bad file descriptor",
            11 => "Resource temporarily unavailable",
            12 => "Cannot allocate memory",
            13 => "Permission denied",
            16 => "Device or resource busy",
            17 => "File exists",
            20 => "Not a directory",
            21 => "Is a directory",
            22 => "Invalid argument",
            24 => "Too many open files",
            27 => "File too large",
            28 => "No space left on device",
            29 => "Illegal seek",
            30 => "Read-only file system",
            32 => "Broken pipe",
            36 => "File name too long",
            39 => "Directory not empty",
            40 => "Too many levels of symbolic links",
            110 => "Connection timed out",
            111 => "Connection refused",
            _ => return None,
        })
    }

    /// The closest standard error kind. Values with no direct match map to `Other`.
    pub fn to_io_kind(self) -> io::ErrorKind {
        match self {
            Errno::NOENT => io::ErrorKind::NotFound,
            Errno::ACCES | Errno::PERM => io::ErrorKind::PermissionDenied,
            Errno::EXIST => io::ErrorKind::AlreadyExists,
            Errno::INTR => io::ErrorKind::Interrupted,
            Errno::INVAL => io::ErrorKind::InvalidInput,
            Errno::TIMEDOUT => io::ErrorKind::TimedOut,
            Errno::PIPE => io::ErrorKind::BrokenPipe,
            Errno::CONNREFUSED => io::ErrorKind::ConnectionRefused,
            Errno::AGAIN => io::ErrorKind::WouldBlock,
            _ => io::ErrorKind::Other,
        }
    }

    /// The error number carried by an `io::Error`, or `None` for a failure that did not come
    /// from the system.
    pub fn from_io(err: &io::Error) -> Option<Errno> {
        err.raw_os_error().map(Errno)
    }
}

impl fmt::Display for Errno {
    /// `No such file or directory (ENOENT)` for a known value, `errno 999` for an unknown one.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.message(), self.name()) {
            (Some(message), Some(name)) => write!(f, "{message} ({name})"),
            _ => write!(f, "errno {}", self.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values_have_names_and_descriptions() {
        assert_eq!(Errno::NOENT.name(), Some("ENOENT"));
        assert_eq!(Errno::ACCES.message(), Some("Permission denied"));
        assert_eq!(Errno::AGAIN.name(), Some("EAGAIN"));
        assert_eq!(Errno(999).name(), None);
        assert_eq!(Errno(999).message(), None);
    }

    #[test]
    fn display_joins_the_description_and_the_name() {
        assert_eq!(Errno::NOENT.to_string(), "No such file or directory (ENOENT)");
        assert_eq!(Errno(999).to_string(), "errno 999");
    }

    #[test]
    fn values_map_to_the_closest_error_kind() {
        assert_eq!(Errno::NOENT.to_io_kind(), io::ErrorKind::NotFound);
        assert_eq!(Errno::ACCES.to_io_kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(Errno::PERM.to_io_kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(Errno::INTR.to_io_kind(), io::ErrorKind::Interrupted);
        assert_eq!(Errno::AGAIN.to_io_kind(), io::ErrorKind::WouldBlock);
        assert_eq!(Errno::NOSPC.to_io_kind(), io::ErrorKind::Other);
    }

    #[test]
    fn raw_os_errors_are_read_back_from_io_errors() {
        let missing = io::Error::from_raw_os_error(2);
        assert_eq!(Errno::from_io(&missing), Some(Errno::NOENT));
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
        let custom = io::Error::new(io::ErrorKind::Other, "not from the system");
        assert_eq!(Errno::from_io(&custom), None);
    }

    #[test]
    fn errors_compare_by_value() {
        assert_eq!(Errno(13), Errno::ACCES);
        assert_ne!(Errno::EXIST, Errno::NOENT);
    }
}
