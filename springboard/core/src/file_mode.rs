//! File type, permission bits and the mode word of a file (std only).
//!
//! A mode is one 32-bit word. The high bits hold the file type. The low twelve bits hold the
//! set-user-ID, set-group-ID and sticky bits, then nine permission bits in owner, group and other
//! triads. This module decodes that word, prints it the way a long directory listing does, parses
//! the octal form and applies a umask. It makes no system call and reads no file.

use std::fmt;
use std::ops::BitOr;

const S_IFMT: u32 = 0o170000;
const S_IFIFO: u32 = 0o010000;
const S_IFCHR: u32 = 0o020000;
const S_IFDIR: u32 = 0o040000;
const S_IFBLK: u32 = 0o060000;
const S_IFREG: u32 = 0o100000;
const S_IFLNK: u32 = 0o120000;
const S_IFSOCK: u32 = 0o140000;

/// The kind of object a mode describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileType {
    Fifo,
    CharacterDevice,
    Directory,
    BlockDevice,
    Regular,
    Symlink,
    Socket,
    /// The type bits are zero or hold a value no kind uses.
    Unknown,
}

impl FileType {
    /// The first character of a long listing line for this kind.
    pub fn letter(self) -> char {
        match self {
            FileType::Fifo => 'p',
            FileType::CharacterDevice => 'c',
            FileType::Directory => 'd',
            FileType::BlockDevice => 'b',
            FileType::Regular => '-',
            FileType::Symlink => 'l',
            FileType::Socket => 's',
            FileType::Unknown => '?',
        }
    }
}

/// Permission bits only: the low twelve bits of a mode, with no file type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FilePermissions(u32);

impl FilePermissions {
    pub const OWNER_READ: FilePermissions = FilePermissions(0o400);
    pub const OWNER_WRITE: FilePermissions = FilePermissions(0o200);
    pub const OWNER_EXECUTE: FilePermissions = FilePermissions(0o100);
    pub const GROUP_READ: FilePermissions = FilePermissions(0o040);
    pub const GROUP_WRITE: FilePermissions = FilePermissions(0o020);
    pub const GROUP_EXECUTE: FilePermissions = FilePermissions(0o010);
    pub const OTHER_READ: FilePermissions = FilePermissions(0o004);
    pub const OTHER_WRITE: FilePermissions = FilePermissions(0o002);
    pub const OTHER_EXECUTE: FilePermissions = FilePermissions(0o001);
    /// The file runs with the owner's user ID.
    pub const SET_USER_ID: FilePermissions = FilePermissions(0o4000);
    /// The file runs with the owner's group ID.
    pub const SET_GROUP_ID: FilePermissions = FilePermissions(0o2000);
    /// Only the owner may remove entries from a directory with this bit.
    pub const STICKY: FilePermissions = FilePermissions(0o1000);

    /// Keeps only the twelve permission bits of `raw`.
    pub const fn from_bits(raw: u32) -> Self {
        FilePermissions(raw & 0o7777)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    /// True when every bit set in `other` is also set here.
    pub const fn contains(self, other: FilePermissions) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for FilePermissions {
    type Output = FilePermissions;
    fn bitor(self, rhs: FilePermissions) -> FilePermissions {
        FilePermissions(self.0 | rhs.0)
    }
}

/// Why an octal mode text was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeError {
    /// The text is empty.
    Empty,
    /// A character is not an octal digit.
    Digit,
    /// More than four digits, or a value above 0o7777.
    Range,
}

impl fmt::Display for ModeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            ModeError::Empty => "mode text is empty",
            ModeError::Digit => "mode text must use octal digits only",
            ModeError::Range => "mode must have at most four octal digits and be at most 7777",
        };
        f.write_str(text)
    }
}

/// A full mode word: file type plus permission bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileMode(u32);

impl FileMode {
    /// Wraps a raw mode word as the system reports it.
    pub const fn from_raw(raw: u32) -> Self {
        FileMode(raw)
    }

    /// Parses one to four octal digits, such as `640` or `0755`. A leading zero is allowed. The
    /// result carries no file type, so `file_type` returns `Unknown` for it.
    pub fn parse_octal(text: &str) -> Result<FileMode, ModeError> {
        if text.is_empty() {
            return Err(ModeError::Empty);
        }
        if text.len() > 4 {
            return Err(ModeError::Range);
        }
        let mut value: u32 = 0;
        for c in text.chars() {
            let digit = c.to_digit(8).ok_or(ModeError::Digit)?;
            value = value * 8 + digit;
        }
        if value > 0o7777 {
            return Err(ModeError::Range);
        }
        Ok(FileMode(value))
    }

    pub const fn raw(self) -> u32 {
        self.0
    }

    pub fn file_type(self) -> FileType {
        match self.0 & S_IFMT {
            S_IFIFO => FileType::Fifo,
            S_IFCHR => FileType::CharacterDevice,
            S_IFDIR => FileType::Directory,
            S_IFBLK => FileType::BlockDevice,
            S_IFREG => FileType::Regular,
            S_IFLNK => FileType::Symlink,
            S_IFSOCK => FileType::Socket,
            _ => FileType::Unknown,
        }
    }

    pub fn permissions(self) -> FilePermissions {
        FilePermissions::from_bits(self.0)
    }

    /// Applies a umask the way file creation does: the bits set in `umask` are cleared from the
    /// nine permission bits. The file type and the special bits are kept.
    pub fn masked(self, umask: u32) -> FileMode {
        FileMode(self.0 & !(umask & 0o777))
    }
}

impl fmt::Display for FileMode {
    /// Ten characters as in `ls -l`: the type letter, then owner, group and other triads. The
    /// set-user-ID, set-group-ID and sticky bits replace the execute letter of their triad,
    /// with `s` or `t` when execute is set and `S` or `T` when it is not.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mode = self.0;
        let mut out = String::with_capacity(10);
        out.push(self.file_type().letter());
        let triads = [(0o400, 0o200, 0o100, FilePermissions::SET_USER_ID, 's', 'S'), (0o040, 0o020, 0o010, FilePermissions::SET_GROUP_ID, 's', 'S'), (0o004, 0o002, 0o001, FilePermissions::STICKY, 't', 'T')];
        for (r, w, x, special, on, off) in triads {
            out.push(if mode & r != 0 { 'r' } else { '-' });
            out.push(if mode & w != 0 { 'w' } else { '-' });
            let exec = mode & x != 0;
            let is_special = mode & special.bits() != 0;
            out.push(match (is_special, exec) {
                (true, true) => on,
                (true, false) => off,
                (false, true) => 'x',
                (false, false) => '-',
            });
        }
        f.write_str(&out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn octal_text_parses_to_permission_bits_without_a_type() {
        let mode = FileMode::parse_octal("640").unwrap();
        assert_eq!(mode.raw(), 0o640);
        assert_eq!(mode.file_type(), FileType::Unknown);
        assert_eq!(FileMode::parse_octal("0755").unwrap().raw(), 0o755);
        assert_eq!(FileMode::parse_octal("7777").unwrap().raw(), 0o7777);
    }

    #[test]
    fn invalid_octal_text_is_refused_with_its_reason() {
        assert_eq!(FileMode::parse_octal(""), Err(ModeError::Empty));
        assert_eq!(FileMode::parse_octal("8"), Err(ModeError::Digit));
        assert_eq!(FileMode::parse_octal("7x"), Err(ModeError::Digit));
        assert_eq!(FileMode::parse_octal("17777"), Err(ModeError::Range));
    }

    #[test]
    fn type_bits_select_the_kind() {
        assert_eq!(FileMode::from_raw(0o100644).file_type(), FileType::Regular);
        assert_eq!(FileMode::from_raw(0o040755).file_type(), FileType::Directory);
        assert_eq!(FileMode::from_raw(0o120777).file_type(), FileType::Symlink);
        assert_eq!(FileMode::from_raw(0o010600).file_type(), FileType::Fifo);
        assert_eq!(FileMode::from_raw(0o000644).file_type(), FileType::Unknown);
    }

    #[test]
    fn display_prints_the_long_listing_form() {
        assert_eq!(FileMode::from_raw(0o100640).to_string(), "-rw-r-----");
        assert_eq!(FileMode::from_raw(0o104755).to_string(), "-rwsr-xr-x");
        assert_eq!(FileMode::from_raw(0o104644).to_string(), "-rwSr--r--");
        assert_eq!(FileMode::from_raw(0o041766).to_string(), "drwxrw-rwT");
        assert_eq!(FileMode::from_raw(0o041777).to_string(), "drwxrwxrwt");
    }

    #[test]
    fn umask_clears_permission_bits_and_keeps_the_type() {
        let created = FileMode::from_raw(0o100666).masked(0o022);
        assert_eq!(created.raw(), 0o100644);
        assert_eq!(FileMode::from_raw(0o040777).masked(0o077).raw(), 0o040700);
        assert_eq!(FileMode::from_raw(0o104777).masked(0o022).raw(), 0o104755);
    }

    #[test]
    fn permission_sets_combine_and_test_by_containment() {
        let rw = FilePermissions::OWNER_READ | FilePermissions::OWNER_WRITE;
        assert_eq!(rw.bits(), 0o600);
        assert!(rw.contains(FilePermissions::OWNER_READ));
        assert!(!rw.contains(FilePermissions::OWNER_EXECUTE));
        assert_eq!(FilePermissions::from_bits(0o170777).bits(), 0o777);
    }
}
