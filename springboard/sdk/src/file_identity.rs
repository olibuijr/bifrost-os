//! File ownership for apps: the user, group, device and inode that identify a file. These are strongly typed
//! wrappers over the platform's uid, gid, device and inode numbers, read from file metadata only. Two files
//! are the same file when their device and inode match, which is the check a document grant makes when it
//! reopens a file.

use std::fs::Metadata;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// A user account number (the owner of a file).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UserId(pub u32);

/// A group account number (the group of a file).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GroupId(pub u32);

/// The device a file lives on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceId(pub u64);

/// The number of a file within its device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Inode(pub u64);

/// Who owns a file and which file it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileIdentity {
    /// The owning user.
    pub user: UserId,
    /// The owning group.
    pub group: GroupId,
    /// The device holding the file.
    pub device: DeviceId,
    /// The file's number on its device.
    pub inode: Inode,
}

impl FileIdentity {
    /// Reads the identity of the file at `path`, following symbolic links. A missing path is an error.
    pub fn of(path: &Path) -> io::Result<FileIdentity> {
        std::fs::metadata(path).map(|m| Self::from_metadata(&m))
    }

    /// The identity recorded in already-read metadata.
    pub fn from_metadata(metadata: &Metadata) -> FileIdentity {
        FileIdentity {
            user: UserId(metadata.uid()),
            group: GroupId(metadata.gid()),
            device: DeviceId(metadata.dev()),
            inode: Inode(metadata.ino()),
        }
    }

    /// True when `user` owns the file.
    pub fn is_owned_by(&self, user: UserId) -> bool {
        self.user == user
    }

    /// True when both identities name the same file: the same device and the same inode.
    pub fn is_same_file(&self, other: &FileIdentity) -> bool {
        self.device == other.device && self.inode == other.inode
    }
}
