//! Unary file system module for apps (FSKit `FSUnaryFileSystem` and `FSVolume`), read-only first version.
//!
//! An app describes one volume by implementing `UnaryFileSystem`: lookup, item attributes, directory listing and
//! reads. `Dispatcher` answers the kernel's FUSE requests for that volume (`fuse::decode` in, `fuse::reply_*` out), and
//! `fuse_mount::Mount::serve` runs it over a mounted session.
//!
//! Reference concepts: `FSUnaryFileSystem` loads one resource into one volume, `FSVolume` answers lookup, attributes,
//! directory reads and reads, and `FSItem` is the node. Here `UnaryFileSystem` is the volume, `ItemId` the node
//! identifier (the FUSE node id), `ItemAttributes` the item attributes, and one value serves one volume.
//!
//! Scope of this first version: read only (writes are refused with EROFS), no resource probing, no grant check and no
//! shell mount service. Nothing mounts a module on its own yet.

use crate::fuse::{self, Attr, Op};

/// Node identifier of a volume's root directory (FSKit's `FSItem.Identifier.rootDirectory`).
pub const ROOT_ITEM: ItemId = fuse::ROOT_NODE_ID;

/// Identifier of an item within one volume. Stable for the life of a mount.
pub type ItemId = u64;

/// The kind of an item (`FSItem.ItemType`), read-only subset: regular files, directories and symbolic links.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    File,
    Directory,
    Symlink,
}

impl ItemKind {
    /// The `S_IF*` file type bits for the item's mode.
    pub fn mode_bits(self) -> u32 {
        match self {
            ItemKind::File => 0o100000,
            ItemKind::Directory => 0o040000,
            ItemKind::Symlink => 0o120000,
        }
    }

    /// The `DT_*` value a READDIR entry carries for this kind (`mode >> 12`).
    pub fn dirent_type(self) -> u32 {
        match self {
            ItemKind::File => 8,
            ItemKind::Directory => 4,
            ItemKind::Symlink => 10,
        }
    }
}

/// Attributes of one item (`FSItem.Attributes`): type, size, permission bits, link count and modification time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemAttributes {
    pub kind: ItemKind,
    pub size: u64,
    /// Permission bits only (`0o777`).
    pub permissions: u16,
    pub links: u32,
    /// Modification time in seconds since the Unix epoch.
    pub modified: u64,
    pub owner: u32,
    pub group: u32,
}

impl ItemAttributes {
    /// A read-only regular file of `size` bytes.
    pub fn file(size: u64) -> ItemAttributes {
        ItemAttributes {
            kind: ItemKind::File,
            size,
            permissions: 0o444,
            links: 1,
            modified: 0,
            owner: 0,
            group: 0,
        }
    }

    /// A read-only directory.
    pub fn directory() -> ItemAttributes {
        ItemAttributes {
            kind: ItemKind::Directory,
            size: 0,
            permissions: 0o555,
            links: 2,
            modified: 0,
            owner: 0,
            group: 0,
        }
    }
}

/// One child of a directory (`FSDirectoryEntryPacker` input). `.` and `..` are added by the dispatcher.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub name: String,
    pub item: ItemId,
    pub kind: ItemKind,
}

/// Volume statistics (`FSStatFSResult`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeStatistics {
    pub block_size: u32,
    pub blocks: u64,
    pub blocks_free: u64,
    pub files: u64,
    pub files_free: u64,
    pub max_name_bytes: u32,
}

impl Default for VolumeStatistics {
    fn default() -> Self {
        VolumeStatistics {
            block_size: 4096,
            blocks: 0,
            blocks_free: 0,
            files: 0,
            files_free: 0,
            max_name_bytes: 255,
        }
    }
}

/// Why a volume operation failed. Each maps to the POSIX error the kernel shows the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsError {
    NotFound,
    NotDirectory,
    IsDirectory,
    PermissionDenied,
    ReadOnly,
    NotSupported,
    Io,
}

impl FsError {
    /// The positive Linux errno for this error.
    pub fn errno(self) -> i32 {
        match self {
            FsError::NotFound => fuse::ENOENT,
            FsError::NotDirectory => fuse::ENOTDIR,
            FsError::IsDirectory => fuse::EISDIR,
            FsError::PermissionDenied => fuse::EACCES,
            FsError::ReadOnly => fuse::EROFS,
            FsError::NotSupported => fuse::ENOSYS,
            FsError::Io => fuse::EIO,
        }
    }
}

/// A read-only volume an app provides (`FSUnaryFileSystem` with its `FSVolume` operations).
///
/// Every method takes the item identifiers this volume handed out; an unknown identifier is `NotFound`. Methods are
/// called on the session's thread and must not block for long, because the kernel waits for each reply.
pub trait UnaryFileSystem {
    /// The volume name shown to the user (`FSVolume.volumeName`).
    fn volume_name(&self) -> &str;

    /// Volume statistics for STATFS.
    fn statistics(&self) -> VolumeStatistics {
        VolumeStatistics::default()
    }

    /// The parent of `item`. The root is its own parent.
    fn parent(&self, item: ItemId) -> Result<ItemId, FsError>;

    /// The child of directory `parent` called `name`. `name` is one path component, never empty, never containing `/`.
    fn lookup(&self, parent: ItemId, name: &str) -> Result<ItemId, FsError>;

    /// The attributes of `item`.
    fn attributes(&self, item: ItemId) -> Result<ItemAttributes, FsError>;

    /// The children of directory `item`, in a stable order: READDIR resumes by position in this list.
    fn read_directory(&self, item: ItemId) -> Result<Vec<DirectoryEntry>, FsError>;

    /// Up to `size` bytes of file `item` starting at `offset`. An empty result means end of file.
    fn read(&self, item: ItemId, offset: u64, size: u32) -> Result<Vec<u8>, FsError>;
}

/// Answers the kernel's requests for one volume.
pub struct Dispatcher<F: UnaryFileSystem> {
    fs: F,
}

impl<F: UnaryFileSystem> Dispatcher<F> {
    pub fn new(fs: F) -> Dispatcher<F> {
        Dispatcher { fs }
    }

    pub fn volume(&self) -> &F {
        &self.fs
    }

    /// Answers one request held in `request` (exactly one request, as `/dev/fuse` delivers it).
    ///
    /// Returns the reply bytes, or `None` when the kernel expects no reply (FORGET, INTERRUPT) or the header itself
    /// cannot be read, because then there is no `unique` to answer.
    pub fn handle(&self, request: &[u8]) -> Option<Vec<u8>> {
        let header: fuse::InHeader = fuse::read_wire(request)?;
        let unique = header.unique;
        let op = match fuse::decode(request) {
            Ok(decoded) => decoded.op,
            Err(_) => {
                return if fuse::expects_reply(header.opcode) {
                    Some(fuse::reply_err(unique, fuse::EINVAL))
                } else {
                    None
                };
            }
        };
        if !fuse::expects_reply(header.opcode) {
            return None;
        }
        Some(match self.answer(header.nodeid, op) {
            Ok(body) => fuse::reply_ok(unique, &body),
            Err(error) => fuse::reply_err(unique, error.errno()),
        })
    }

    fn answer(&self, nodeid: u64, op: Op<'_>) -> Result<Vec<u8>, FsError> {
        let fs = &self.fs;
        Ok(match op {
            Op::Init(init) => fuse::wire_bytes(&fuse::InitOut::answer(&init)).to_vec(),
            Op::Destroy | Op::Release { .. } | Op::ReleaseDir { .. } => Vec::new(),
            Op::Lookup { name } => {
                let name = std::str::from_utf8(name).map_err(|_| FsError::NotFound)?;
                if name.is_empty() || name.contains('/') {
                    return Err(FsError::NotFound);
                }
                let item = fs.lookup(nodeid, name)?;
                let attributes = fs.attributes(item)?;
                let entry = fuse::EntryOut {
                    nodeid: item,
                    generation: 0,
                    entry_valid: fuse::CACHE_SECS,
                    attr_valid: fuse::CACHE_SECS,
                    entry_valid_nsec: 0,
                    attr_valid_nsec: 0,
                    attr: wire_attr(item, &attributes),
                };
                fuse::wire_bytes(&entry).to_vec()
            }
            Op::GetAttr { .. } => {
                let attributes = fs.attributes(nodeid)?;
                let out = fuse::AttrOut {
                    attr_valid: fuse::CACHE_SECS,
                    attr_valid_nsec: 0,
                    dummy: 0,
                    attr: wire_attr(nodeid, &attributes),
                };
                fuse::wire_bytes(&out).to_vec()
            }
            Op::Open { flags } => {
                if fs.attributes(nodeid)?.kind == ItemKind::Directory {
                    return Err(FsError::IsDirectory);
                }
                refuse_write(flags)?;
                fuse::wire_bytes(&fuse::OpenOut::default()).to_vec()
            }
            Op::OpenDir { flags } => {
                if fs.attributes(nodeid)?.kind != ItemKind::Directory {
                    return Err(FsError::NotDirectory);
                }
                refuse_write(flags)?;
                fuse::wire_bytes(&fuse::OpenOut::default()).to_vec()
            }
            Op::Read(args) => {
                if fs.attributes(nodeid)?.kind == ItemKind::Directory {
                    return Err(FsError::IsDirectory);
                }
                let mut data = fs.read(nodeid, args.offset, args.size)?;
                data.truncate(args.size as usize);
                data
            }
            Op::ReadDir(args) => {
                if fs.attributes(nodeid)?.kind != ItemKind::Directory {
                    return Err(FsError::NotDirectory);
                }
                // "." and ".." come first, then the volume's children. The offset written for an entry is the
                // position of the entry after it, so a READDIR at that offset resumes there.
                let mut entries = vec![
                    DirectoryEntry {
                        name: ".".to_string(),
                        item: nodeid,
                        kind: ItemKind::Directory,
                    },
                    DirectoryEntry {
                        name: "..".to_string(),
                        item: fs.parent(nodeid)?,
                        kind: ItemKind::Directory,
                    },
                ];
                entries.extend(fs.read_directory(nodeid)?);
                let mut writer = fuse::DirentWriter::new(args.size as usize);
                for (index, entry) in entries.iter().enumerate().skip(args.offset as usize) {
                    let off = index as u64 + 1;
                    if !writer.push(entry.item, off, entry.kind.dirent_type(), entry.name.as_bytes()) {
                        break;
                    }
                }
                writer.into_bytes()
            }
            Op::StatFs => {
                let stats = fs.statistics();
                let out = fuse::KStatFs {
                    blocks: stats.blocks,
                    bfree: stats.blocks_free,
                    bavail: stats.blocks_free,
                    files: stats.files,
                    ffree: stats.files_free,
                    bsize: stats.block_size,
                    namelen: stats.max_name_bytes,
                    frsize: stats.block_size,
                    padding: 0,
                    spare: [0; 6],
                };
                fuse::wire_bytes(&out).to_vec()
            }
            // FORGET, INTERRUPT are answered by `handle` without a reply; any other opcode is not implemented.
            Op::Forget | Op::Interrupt | Op::Other { .. } => return Err(FsError::NotSupported),
        })
    }
}

/// Refuses any open for writing: the volume is read-only.
fn refuse_write(flags: u32) -> Result<(), FsError> {
    if flags & fuse::O_ACCMODE != 0 {
        Err(FsError::ReadOnly)
    } else {
        Ok(())
    }
}

/// The wire attributes of `item`, with the block count rounded up to 512-byte units.
fn wire_attr(item: ItemId, attributes: &ItemAttributes) -> Attr {
    Attr {
        ino: item,
        size: attributes.size,
        blocks: attributes.size.div_ceil(512),
        atime: attributes.modified,
        mtime: attributes.modified,
        ctime: attributes.modified,
        mode: attributes.kind.mode_bits() | (attributes.permissions as u32 & 0o7777),
        nlink: attributes.links,
        uid: attributes.owner,
        gid: attributes.group,
        blksize: 4096,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fuse::{self, DirentWriter, EntryOut, InitIn, InitOut, OpenIn, OutHeader, ReadIn, ReleaseIn};

    /// An in-memory volume: `hello.txt` ("hello, world") and `docs/notes.txt` (300 bytes) under the root.
    struct Memory {
        /// Index `i` holds item `i + 1`. Index 0 is the root, whose parent is itself.
        items: Vec<Node>,
    }

    struct Node {
        parent: ItemId,
        name: &'static str,
        kind: ItemKind,
        data: Vec<u8>,
    }

    impl Memory {
        fn sample() -> Memory {
            Memory {
                items: vec![
                    Node { parent: 1, name: "", kind: ItemKind::Directory, data: Vec::new() },
                    Node { parent: 1, name: "hello.txt", kind: ItemKind::File, data: b"hello, world".to_vec() },
                    Node { parent: 1, name: "docs", kind: ItemKind::Directory, data: Vec::new() },
                    Node { parent: 3, name: "notes.txt", kind: ItemKind::File, data: vec![7; 300] },
                ],
            }
        }

        fn node(&self, item: ItemId) -> Result<&Node, FsError> {
            item.checked_sub(1)
                .and_then(|index| self.items.get(index as usize))
                .ok_or(FsError::NotFound)
        }
    }

    impl UnaryFileSystem for Memory {
        fn volume_name(&self) -> &str {
            "Memory"
        }

        fn parent(&self, item: ItemId) -> Result<ItemId, FsError> {
            Ok(self.node(item)?.parent)
        }

        fn lookup(&self, parent: ItemId, name: &str) -> Result<ItemId, FsError> {
            if self.node(parent)?.kind != ItemKind::Directory {
                return Err(FsError::NotDirectory);
            }
            self.items
                .iter()
                .enumerate()
                .skip(1)
                .find(|(_, node)| node.parent == parent && node.name == name)
                .map(|(index, _)| index as ItemId + 1)
                .ok_or(FsError::NotFound)
        }

        fn attributes(&self, item: ItemId) -> Result<ItemAttributes, FsError> {
            let node = self.node(item)?;
            Ok(if node.kind == ItemKind::Directory {
                ItemAttributes::directory()
            } else {
                ItemAttributes::file(node.data.len() as u64)
            })
        }

        fn read_directory(&self, item: ItemId) -> Result<Vec<DirectoryEntry>, FsError> {
            if self.node(item)?.kind != ItemKind::Directory {
                return Err(FsError::NotDirectory);
            }
            Ok(self
                .items
                .iter()
                .enumerate()
                .skip(1)
                .filter(|(_, node)| node.parent == item)
                .map(|(index, node)| DirectoryEntry {
                    name: node.name.to_string(),
                    item: index as ItemId + 1,
                    kind: node.kind,
                })
                .collect())
        }

        fn read(&self, item: ItemId, offset: u64, size: u32) -> Result<Vec<u8>, FsError> {
            let node = self.node(item)?;
            if node.kind == ItemKind::Directory {
                return Err(FsError::IsDirectory);
            }
            let start = (offset as usize).min(node.data.len());
            let end = start.saturating_add(size as usize).min(node.data.len());
            Ok(node.data[start..end].to_vec())
        }
    }

    fn request(opcode: u32, nodeid: u64, body: &[u8]) -> Vec<u8> {
        fuse::encode_request(opcode, 42, nodeid, body)
    }

    fn name_body(name: &str) -> Vec<u8> {
        let mut body = name.as_bytes().to_vec();
        body.push(0);
        body
    }

    fn read_body(offset: u64, size: u32) -> Vec<u8> {
        fuse::wire_bytes(&ReadIn {
            offset,
            size,
            ..Default::default()
        })
        .to_vec()
    }

    /// Splits a reply into its header and body, checking the header's length field.
    fn split(reply: &[u8]) -> (OutHeader, &[u8]) {
        let header: OutHeader = fuse::read_wire(reply).unwrap();
        assert_eq!(header.len as usize, reply.len());
        (header, &reply[16..])
    }

    fn dispatcher() -> Dispatcher<Memory> {
        Dispatcher::new(Memory::sample())
    }

    #[test]
    fn init_answers_with_the_smaller_minor_and_no_features() {
        let init = fuse::wire_bytes(&InitIn {
            major: 7,
            minor: 36,
            max_readahead: 131072,
            flags: 0,
        })
        .to_vec();
        let reply = dispatcher().handle(&request(fuse::OP_INIT, 0, &init)).unwrap();
        let (header, body) = split(&reply);
        assert_eq!((header.error, header.unique), (0, 42));
        let out: InitOut = fuse::read_wire(body).unwrap();
        assert_eq!((out.major, out.minor, out.flags), (7, 31, 0));
        assert_eq!(out.max_readahead, 131072);
        assert_eq!(out.max_write, fuse::MAX_WRITE);
        assert_eq!(reply.len(), 16 + 64);
    }

    #[test]
    fn lookup_returns_the_entry_and_attributes() {
        let reply = dispatcher()
            .handle(&request(fuse::OP_LOOKUP, ROOT_ITEM, &name_body("hello.txt")))
            .unwrap();
        let (header, body) = split(&reply);
        assert_eq!(header.error, 0);
        let entry: EntryOut = fuse::read_wire(body).unwrap();
        assert_eq!(entry.nodeid, 2);
        assert_eq!(entry.attr.size, 12);
        assert_eq!(entry.attr.mode, 0o100000 | 0o444);
        assert_eq!(entry.attr.nlink, 1);
        assert_eq!(entry.attr.blocks, 1);
        assert_eq!(entry.entry_valid, fuse::CACHE_SECS);
    }

    #[test]
    fn lookup_of_a_missing_name_is_enoent() {
        let reply = dispatcher()
            .handle(&request(fuse::OP_LOOKUP, ROOT_ITEM, &name_body("missing")))
            .unwrap();
        let (header, body) = split(&reply);
        assert_eq!(header.error, -fuse::ENOENT);
        assert!(body.is_empty());
    }

    #[test]
    fn lookup_under_a_file_is_enotdir_and_a_slash_name_is_enoent() {
        let reply = dispatcher()
            .handle(&request(fuse::OP_LOOKUP, 2, &name_body("x")))
            .unwrap();
        assert_eq!(split(&reply).0.error, -fuse::ENOTDIR);
        let reply = dispatcher()
            .handle(&request(fuse::OP_LOOKUP, ROOT_ITEM, &name_body("docs/notes.txt")))
            .unwrap();
        assert_eq!(split(&reply).0.error, -fuse::ENOENT);
    }

    #[test]
    fn getattr_reports_directory_and_file_modes() {
        let reply = dispatcher().handle(&request(fuse::OP_GETATTR, ROOT_ITEM, &[0; 16])).unwrap();
        let (header, body) = split(&reply);
        assert_eq!(header.error, 0);
        let out: fuse::AttrOut = fuse::read_wire(body).unwrap();
        assert_eq!(out.attr.mode, 0o040000 | 0o555);
        assert_eq!(out.attr.nlink, 2);
        let reply = dispatcher().handle(&request(fuse::OP_GETATTR, 4, &[0; 16])).unwrap();
        let out: fuse::AttrOut = fuse::read_wire(split(&reply).1).unwrap();
        assert_eq!(out.attr.size, 300);
        assert_eq!(out.attr.blocks, 1);
        assert_eq!(out.attr.ino, 4);
    }

    #[test]
    fn open_of_a_directory_is_eisdir_and_opendir_of_a_file_is_enotdir() {
        let open = fuse::wire_bytes(&OpenIn { flags: 0, open_flags: 0 }).to_vec();
        let reply = dispatcher().handle(&request(fuse::OP_OPEN, ROOT_ITEM, &open)).unwrap();
        assert_eq!(split(&reply).0.error, -fuse::EISDIR);
        let reply = dispatcher().handle(&request(fuse::OP_OPENDIR, 2, &open)).unwrap();
        assert_eq!(split(&reply).0.error, -fuse::ENOTDIR);
        let reply = dispatcher().handle(&request(fuse::OP_OPEN, 2, &open)).unwrap();
        let (header, body) = split(&reply);
        assert_eq!(header.error, 0);
        assert_eq!(body.len(), 16);
    }

    #[test]
    fn write_access_is_refused_with_erofs() {
        // O_WRONLY is 1 and O_RDWR is 2 in the access-mode bits.
        for flags in [1u32, 2] {
            let open = fuse::wire_bytes(&OpenIn { flags, open_flags: 0 }).to_vec();
            let reply = dispatcher().handle(&request(fuse::OP_OPEN, 2, &open)).unwrap();
            assert_eq!(split(&reply).0.error, -fuse::EROFS, "flags {flags}");
        }
    }

    #[test]
    fn read_returns_the_requested_slice_and_stops_at_end() {
        let d = dispatcher();
        let reply = d.handle(&request(fuse::OP_READ, 2, &read_body(7, 5))).unwrap();
        let (header, body) = split(&reply);
        assert_eq!(header.error, 0);
        assert_eq!(body, b"world");
        // Past the end is an empty reply, which the kernel reads as end of file.
        let reply = d.handle(&request(fuse::OP_READ, 2, &read_body(100, 5))).unwrap();
        assert_eq!(split(&reply).1, b"");
        // A read larger than the file is cut at its end.
        let reply = d.handle(&request(fuse::OP_READ, 2, &read_body(0, 4096))).unwrap();
        assert_eq!(split(&reply).1, b"hello, world");
        // Reading a directory is EISDIR.
        let reply = d.handle(&request(fuse::OP_READ, ROOT_ITEM, &read_body(0, 16))).unwrap();
        assert_eq!(split(&reply).0.error, -fuse::EISDIR);
    }

    #[test]
    fn readdir_lists_dots_then_children_with_resumable_offsets() {
        let reply = dispatcher()
            .handle(&request(fuse::OP_READDIR, ROOT_ITEM, &read_body(0, 4096)))
            .unwrap();
        let (header, body) = split(&reply);
        assert_eq!(header.error, 0);
        let entries = fuse::decode_dirents(body).unwrap();
        let names: Vec<&[u8]> = entries.iter().map(|(_, name)| *name).collect();
        assert_eq!(names, vec![&b"."[..], b"..", b"hello.txt", b"docs"]);
        let offsets: Vec<u64> = entries.iter().map(|(d, _)| d.off).collect();
        assert_eq!(offsets, vec![1, 2, 3, 4]);
        assert_eq!(entries[2].0.ino, 2);
        assert_eq!(entries[2].0.typ, 8);
        assert_eq!(entries[3].0.typ, 4);
        assert_eq!(body.len() % 8, 0);
    }

    #[test]
    fn readdir_stops_at_the_buffer_size_and_resumes_at_the_next_offset() {
        let d = dispatcher();
        // Room for exactly two entries: "." and ".." are 32 bytes each, so "hello.txt" does not fit.
        let size = (fuse::dirent_len(1) * 2) as u32;
        let reply = d.handle(&request(fuse::OP_READDIR, ROOT_ITEM, &read_body(0, size))).unwrap();
        let first = fuse::decode_dirents(split(&reply).1).unwrap();
        assert_eq!(first.len(), 2);
        let next = first.last().unwrap().0.off;
        assert_eq!(next, 2);
        let reply = d.handle(&request(fuse::OP_READDIR, ROOT_ITEM, &read_body(next, 4096))).unwrap();
        let rest = fuse::decode_dirents(split(&reply).1).unwrap();
        let names: Vec<&[u8]> = rest.iter().map(|(_, name)| *name).collect();
        assert_eq!(names, vec![&b"hello.txt"[..], b"docs"]);
    }

    #[test]
    fn readdir_of_a_file_is_enotdir() {
        let reply = dispatcher().handle(&request(fuse::OP_READDIR, 2, &read_body(0, 4096))).unwrap();
        assert_eq!(split(&reply).0.error, -fuse::ENOTDIR);
    }

    #[test]
    fn forget_and_interrupt_get_no_reply() {
        let d = dispatcher();
        assert!(d.handle(&request(fuse::OP_FORGET, 2, &[1, 0, 0, 0, 0, 0, 0, 0])).is_none());
        assert!(d.handle(&request(fuse::OP_INTERRUPT, 2, &[0; 8])).is_none());
    }

    #[test]
    fn unknown_opcodes_get_enosys() {
        let reply = dispatcher().handle(&request(5, 2, &[])).unwrap();
        assert_eq!(split(&reply).0.error, -fuse::ENOSYS);
    }

    #[test]
    fn statfs_reports_the_block_and_name_limits() {
        let reply = dispatcher().handle(&request(fuse::OP_STATFS, ROOT_ITEM, &[])).unwrap();
        let (header, body) = split(&reply);
        assert_eq!(header.error, 0);
        assert_eq!(body.len(), 80);
        let st: fuse::KStatFs = fuse::read_wire(body).unwrap();
        assert_eq!((st.bsize, st.frsize, st.namelen), (4096, 4096, 255));
    }

    #[test]
    fn release_and_destroy_are_acknowledged() {
        let d = dispatcher();
        let release = fuse::wire_bytes(&ReleaseIn::default()).to_vec();
        let reply = d.handle(&request(fuse::OP_RELEASE, 2, &release)).unwrap();
        assert_eq!(reply.len(), 16);
        assert_eq!(split(&reply).0.error, 0);
        let reply = d.handle(&request(fuse::OP_RELEASEDIR, ROOT_ITEM, &release)).unwrap();
        assert_eq!(split(&reply).0.error, 0);
        let reply = d.handle(&request(fuse::OP_DESTROY, 0, &[])).unwrap();
        assert_eq!(reply.len(), 16);
    }

    #[test]
    fn a_short_body_is_einval_and_a_short_header_gets_no_reply() {
        let d = dispatcher();
        let reply = d.handle(&request(fuse::OP_READ, 2, &[0u8; 4])).unwrap();
        assert_eq!(split(&reply).0.error, -fuse::EINVAL);
        assert!(d.handle(&[0u8; 10]).is_none());
    }

    #[test]
    fn a_packed_entry_round_trips_through_the_writer() {
        // The writer and the dispatcher agree on the entry size the kernel reads.
        let mut w = DirentWriter::new(4096);
        assert!(w.push(2, 1, 8, b"hello.txt"));
        let bytes = w.into_bytes();
        assert_eq!(bytes.len(), fuse::dirent_len(9));
    }
}
