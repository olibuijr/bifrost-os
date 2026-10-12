//! File status records for apps: the device, inode, link count, owner, size, block use and the
//! three times of one path, read through the system's stat calls (Unix only).
//!
//! `stat` follows a final symbolic link and describes its target. `lstat` describes the link
//! itself. The mode is the typed `FileMode` from `bifrost_core::file_mode`, so the file type and
//! the permission bits are decoded in one place. Stat flags and the file generation number are
//! not exposed, and no call here opens the file.

use bifrost_core::file_mode::{FileMode, FilePermissions, FileType};
use std::fs::{self, Metadata};
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// A point in time as whole seconds since the epoch plus nanoseconds within that second.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp {
    pub secs: i64,
    pub nanos: i64,
}

/// The status of one path at the moment it was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStat {
    /// Identifier of the device that holds the file.
    pub device: u64,
    pub inode: u64,
    pub mode: FileMode,
    pub link_count: u64,
    pub user_id: u32,
    pub group_id: u32,
    /// Size in bytes.
    pub size: u64,
    /// Number of 512-byte blocks allocated to the file.
    pub blocks_allocated: u64,
    /// Preferred block size for I/O on the file, in bytes.
    pub block_size: u64,
    pub accessed: Timestamp,
    pub modified: Timestamp,
    /// Time of the last change to the inode (owner, mode, link count), not of the contents.
    pub changed: Timestamp,
}

impl FileStat {
    /// Reads the status from metadata that the caller already holds.
    pub fn from_metadata(meta: &Metadata) -> FileStat {
        FileStat {
            device: meta.dev(),
            inode: meta.ino(),
            mode: FileMode::from_raw(meta.mode()),
            link_count: meta.nlink(),
            user_id: meta.uid(),
            group_id: meta.gid(),
            size: meta.size(),
            blocks_allocated: meta.blocks(),
            block_size: meta.blksize(),
            accessed: Timestamp { secs: meta.atime(), nanos: meta.atime_nsec() },
            modified: Timestamp { secs: meta.mtime(), nanos: meta.mtime_nsec() },
            changed: Timestamp { secs: meta.ctime(), nanos: meta.ctime_nsec() },
        }
    }

    pub fn file_type(&self) -> FileType {
        self.mode.file_type()
    }

    pub fn permissions(&self) -> FilePermissions {
        self.mode.permissions()
    }
}

/// Status of `path`, following a final symbolic link to its target.
pub fn stat(path: &Path) -> io::Result<FileStat> {
    fs::metadata(path).map(|m| FileStat::from_metadata(&m))
}

/// Status of `path` itself. A symbolic link is described, not its target.
pub fn lstat(path: &Path) -> io::Result<FileStat> {
    fs::symlink_metadata(path).map(|m| FileStat::from_metadata(&m))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::errno::Errno;
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-file-stat"))
            .join(format!("{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_regular_file_reports_its_size_mode_and_one_link() {
        let dir = scratch("regular");
        let path = dir.join("a.txt");
        fs::write(&path, "hello").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let st = stat(&path).unwrap();
        assert_eq!(st.size, 5);
        assert_eq!(st.file_type(), FileType::Regular);
        assert_eq!(st.permissions().bits(), 0o640);
        assert_eq!(st.mode.to_string(), "-rw-r-----");
        assert_eq!(st.link_count, 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_directory_reports_the_directory_type() {
        let dir = scratch("directory");
        let st = stat(&dir).unwrap();
        assert_eq!(st.file_type(), FileType::Directory);
        assert_eq!(st.mode.to_string().chars().next(), Some('d'));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn lstat_describes_a_link_while_stat_follows_it_to_the_target() {
        let dir = scratch("link");
        let target = dir.join("target.txt");
        fs::write(&target, "abc").unwrap();
        let link = dir.join("link");
        symlink(&target, &link).unwrap();
        assert_eq!(lstat(&link).unwrap().file_type(), FileType::Symlink);
        let followed = stat(&link).unwrap();
        assert_eq!(followed.file_type(), FileType::Regular);
        assert_eq!(followed.size, 3);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn two_names_for_one_file_share_the_inode_and_count_two_links() {
        let dir = scratch("hardlink");
        let first = dir.join("one");
        let second = dir.join("two");
        fs::write(&first, "x").unwrap();
        fs::hard_link(&first, &second).unwrap();
        let a = stat(&first).unwrap();
        let b = stat(&second).unwrap();
        assert_eq!(a.inode, b.inode);
        assert_eq!(a.device, b.device);
        assert_eq!(a.link_count, 2);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_path_fails_with_no_such_file_and_a_typed_errno() {
        let dir = scratch("missing");
        let err = stat(&dir.join("absent")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert_eq!(Errno::from_io(&err), Some(Errno::NOENT));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn modified_time_is_after_2001_and_nanoseconds_stay_within_one_second() {
        let dir = scratch("time");
        let path = dir.join("t");
        fs::write(&path, "t").unwrap();
        let st = stat(&path).unwrap();
        assert!(st.modified.secs > 1_000_000_000);
        assert!((0..1_000_000_000).contains(&st.modified.nanos));
        let _ = fs::remove_dir_all(dir);
    }
}
