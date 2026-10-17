//! The FUSE kernel protocol (`include/uapi/linux/fuse.h`, protocol 7.31 and later), as pure encoding and decoding.
//!
//! The wire structs are `#[repr(C)]` with the uapi layout and no padding (the `const` asserts pin their sizes to the
//! uapi sizes, which equal the sum of their fields). A request is one `fuse_in_header` followed by the opcode's argument
//! struct. A reply is one `fuse_out_header` followed by the body; an error reply is the header alone, with `error` set
//! to `-errno`. All integers are in native byte order, as the kernel expects.
//!
//! Nothing here touches a file descriptor. `fuse_mount` reads and writes `/dev/fuse`; this module turns the bytes into
//! `Request` values and back, so the protocol is tested from byte buffers.
//!
//! Scope: the read-only operations (INIT, DESTROY, LOOKUP, FORGET, GETATTR, OPEN, READ, RELEASE, OPENDIR, READDIR,
//! RELEASEDIR, STATFS, INTERRUPT). Every other opcode decodes as `Op::Other` and is answered with ENOSYS. No optional
//! feature flags are negotiated, so the kernel uses the plain request forms.

use std::mem::size_of;

/// Protocol version this module speaks. The kernel accepts any minor at or below its own, so INIT answers with the
/// smaller of this and the kernel's minor.
pub const KERNEL_MAJOR: u32 = 7;
pub const KERNEL_MINOR: u32 = 31;

/// Node id of a mounted file system's root directory (`FUSE_ROOT_ID`).
pub const ROOT_NODE_ID: u64 = 1;

/// Largest write the kernel may send. Advertised in INIT; the module is read-only, but the kernel still needs a value.
pub const MAX_WRITE: u32 = 128 * 1024;

/// Seconds the kernel may cache a lookup or attribute answer. A read-only volume changes rarely, so one second keeps
/// the cache useful without hiding a change for long.
pub const CACHE_SECS: u64 = 1;

/// The `open(2)` access-mode bits, which the kernel passes unchanged in OPEN and OPENDIR.
pub const O_ACCMODE: u32 = 3;

// Errno values used in replies (Linux numbering).
pub const ENOENT: i32 = 2;
pub const EIO: i32 = 5;
pub const EACCES: i32 = 13;
pub const ENOTDIR: i32 = 20;
pub const EISDIR: i32 = 21;
pub const EINVAL: i32 = 22;
pub const EROFS: i32 = 30;
pub const ENOSYS: i32 = 38;

// Opcodes (`enum fuse_opcode`).
pub const OP_LOOKUP: u32 = 1;
pub const OP_FORGET: u32 = 2;
pub const OP_GETATTR: u32 = 3;
pub const OP_OPEN: u32 = 14;
pub const OP_READ: u32 = 15;
pub const OP_STATFS: u32 = 17;
pub const OP_RELEASE: u32 = 18;
pub const OP_INIT: u32 = 26;
pub const OP_OPENDIR: u32 = 27;
pub const OP_READDIR: u32 = 28;
pub const OP_RELEASEDIR: u32 = 29;
pub const OP_INTERRUPT: u32 = 36;
pub const OP_DESTROY: u32 = 38;
pub const OP_BATCH_FORGET: u32 = 42;

/// `fuse_in_header`: the first 40 bytes of every request.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InHeader {
    pub len: u32,
    pub opcode: u32,
    pub unique: u64,
    pub nodeid: u64,
    pub uid: u32,
    pub gid: u32,
    pub pid: u32,
    pub total_extlen: u16,
    pub padding: u16,
}

/// `fuse_out_header`: the first 16 bytes of every reply. `error` is zero or `-errno`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OutHeader {
    pub len: u32,
    pub error: i32,
    pub unique: u64,
}

/// `fuse_init_in`, the kernel's INIT request body.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InitIn {
    pub major: u32,
    pub minor: u32,
    pub max_readahead: u32,
    pub flags: u32,
}

/// `fuse_init_out`, the INIT reply body (64 bytes at protocol 7.31).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InitOut {
    pub major: u32,
    pub minor: u32,
    pub max_readahead: u32,
    pub flags: u32,
    pub max_background: u16,
    pub congestion_threshold: u16,
    pub max_write: u32,
    pub time_gran: u32,
    pub max_pages: u16,
    pub map_alignment: u16,
    pub flags2: u32,
    pub unused: [u32; 7],
}

impl InitOut {
    /// The INIT answer for a kernel that asks with `init`: the smaller protocol minor, the read-ahead it asked for, and
    /// no optional features (flags 0).
    pub fn answer(init: &InitIn) -> InitOut {
        InitOut {
            major: KERNEL_MAJOR,
            minor: KERNEL_MINOR.min(init.minor),
            max_readahead: init.max_readahead,
            flags: 0,
            max_background: 0,
            congestion_threshold: 0,
            max_write: MAX_WRITE,
            time_gran: 1,
            max_pages: 0,
            map_alignment: 0,
            flags2: 0,
            unused: [0; 7],
        }
    }
}

/// `fuse_attr`: the attributes of one node (88 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attr {
    pub ino: u64,
    pub size: u64,
    pub blocks: u64,
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
    pub atimensec: u32,
    pub mtimensec: u32,
    pub ctimensec: u32,
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub rdev: u32,
    pub blksize: u32,
    pub flags: u32,
}

/// `fuse_entry_out`: the answer to LOOKUP (128 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EntryOut {
    pub nodeid: u64,
    pub generation: u64,
    pub entry_valid: u64,
    pub attr_valid: u64,
    pub entry_valid_nsec: u32,
    pub attr_valid_nsec: u32,
    pub attr: Attr,
}

/// `fuse_attr_out`: the answer to GETATTR (104 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttrOut {
    pub attr_valid: u64,
    pub attr_valid_nsec: u32,
    pub dummy: u32,
    pub attr: Attr,
}

/// `fuse_open_in`: the body of OPEN and OPENDIR.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpenIn {
    pub flags: u32,
    pub open_flags: u32,
}

/// `fuse_open_out`: the answer to OPEN and OPENDIR.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpenOut {
    pub fh: u64,
    pub open_flags: u32,
    pub padding: u32,
}

/// `fuse_read_in`: the body of READ and READDIR (40 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReadIn {
    pub fh: u64,
    pub offset: u64,
    pub size: u32,
    pub read_flags: u32,
    pub lock_owner: u64,
    pub flags: u32,
    pub padding: u32,
}

/// `fuse_release_in`: the body of RELEASE and RELEASEDIR.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReleaseIn {
    pub fh: u64,
    pub flags: u32,
    pub release_flags: u32,
    pub lock_owner: u64,
}

/// `fuse_getattr_in`: the body of GETATTR.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GetAttrIn {
    pub getattr_flags: u32,
    pub dummy: u32,
    pub fh: u64,
}

/// `fuse_forget_in`: the body of FORGET.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ForgetIn {
    pub nlookup: u64,
}

/// `fuse_kstatfs`: the volume statistics (80 bytes with `StatFsOut`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KStatFs {
    pub blocks: u64,
    pub bfree: u64,
    pub bavail: u64,
    pub files: u64,
    pub ffree: u64,
    pub bsize: u32,
    pub namelen: u32,
    pub frsize: u32,
    pub padding: u32,
    pub spare: [u32; 6],
}

/// `fuse_dirent`: the fixed part of one READDIR entry. The name follows at byte 24, unterminated.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dirent {
    pub ino: u64,
    pub off: u64,
    pub namelen: u32,
    pub typ: u32,
}

const _: () = assert!(size_of::<InHeader>() == 40);
const _: () = assert!(size_of::<OutHeader>() == 16);
const _: () = assert!(size_of::<InitIn>() == 16);
const _: () = assert!(size_of::<InitOut>() == 64);
const _: () = assert!(size_of::<Attr>() == 88);
const _: () = assert!(size_of::<EntryOut>() == 128);
const _: () = assert!(size_of::<AttrOut>() == 104);
const _: () = assert!(size_of::<OpenIn>() == 8);
const _: () = assert!(size_of::<OpenOut>() == 16);
const _: () = assert!(size_of::<ReadIn>() == 40);
const _: () = assert!(size_of::<ReleaseIn>() == 24);
const _: () = assert!(size_of::<GetAttrIn>() == 16);
const _: () = assert!(size_of::<ForgetIn>() == 8);
const _: () = assert!(size_of::<KStatFs>() == 80);
const _: () = assert!(size_of::<Dirent>() == 24);

/// Bytes before a READDIR entry's name (`FUSE_NAME_OFFSET`).
pub const DIRENT_HEADER: usize = size_of::<Dirent>();

/// Bytes one READDIR entry takes for a name of `name_len` bytes: the header and name, padded to 8 (`FUSE_DIRENT_ALIGN`).
pub const fn dirent_len(name_len: usize) -> usize {
    (DIRENT_HEADER + name_len + 7) & !7
}

/// One decoded request. Names and bodies borrow the receive buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request<'a> {
    pub unique: u64,
    pub nodeid: u64,
    pub uid: u32,
    pub gid: u32,
    pub pid: u32,
    pub op: Op<'a>,
}

/// The operation a request asks for, with its decoded arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op<'a> {
    Init(InitIn),
    Destroy,
    /// `name` has its trailing NUL removed.
    Lookup { name: &'a [u8] },
    /// FORGET and BATCH_FORGET: the kernel expects no reply.
    Forget,
    GetAttr { fh: u64 },
    Open { flags: u32 },
    OpenDir { flags: u32 },
    Read(ReadIn),
    ReadDir(ReadIn),
    Release { fh: u64 },
    ReleaseDir { fh: u64 },
    StatFs,
    /// INTERRUPT: the kernel expects no reply.
    Interrupt,
    /// Any opcode this module does not implement.
    Other { opcode: u32 },
}

/// Why a request buffer could not be decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The buffer is shorter than the header.
    Short,
    /// The header's length is shorter than the header or longer than the buffer.
    BadLength,
    /// The body is shorter than the opcode's argument struct.
    BadBody,
}

/// Whether the kernel waits for a reply to `opcode`. FORGET, BATCH_FORGET and INTERRUPT get none.
pub fn expects_reply(opcode: u32) -> bool {
    !matches!(opcode, OP_FORGET | OP_BATCH_FORGET | OP_INTERRUPT)
}

/// Reads a wire struct from the front of `buf`. Any bit pattern is a valid value of these integer-only structs.
pub fn read_wire<T: Copy>(buf: &[u8]) -> Option<T> {
    if buf.len() < size_of::<T>() {
        return None;
    }
    // SAFETY: the length was checked above, and `read_unaligned` copes with any alignment. Every wire struct is
    // `Copy`, `repr(C)` and built only from integers and arrays of integers, so every bit pattern is a valid value.
    Some(unsafe { std::ptr::read_unaligned(buf.as_ptr() as *const T) })
}

/// The bytes of a wire struct in native order. The structs have no padding (see the size asserts above).
pub fn wire_bytes<T: Copy>(value: &T) -> &[u8] {
    // SAFETY: the reference is valid for `size_of::<T>()` bytes, and a padding-free `repr(C)` struct of integers has
    // no uninitialised bytes to read.
    unsafe { std::slice::from_raw_parts(value as *const T as *const u8, size_of::<T>()) }
}

/// Decodes one request from a receive buffer holding exactly one request.
pub fn decode(buf: &[u8]) -> Result<Request<'_>, DecodeError> {
    let header: InHeader = read_wire(buf).ok_or(DecodeError::Short)?;
    let len = header.len as usize;
    if len < size_of::<InHeader>() || len > buf.len() {
        return Err(DecodeError::BadLength);
    }
    let body = &buf[size_of::<InHeader>()..len];
    let bad = || DecodeError::BadBody;
    let op = match header.opcode {
        OP_INIT => Op::Init(read_wire(body).ok_or_else(bad)?),
        OP_DESTROY => Op::Destroy,
        OP_LOOKUP => {
            let name = match body.split_last() {
                Some((&0, rest)) => rest,
                _ => body,
            };
            Op::Lookup { name }
        }
        OP_FORGET | OP_BATCH_FORGET => Op::Forget,
        OP_GETATTR => {
            let args: GetAttrIn = read_wire(body).ok_or_else(bad)?;
            Op::GetAttr { fh: args.fh }
        }
        OP_OPEN => {
            let args: OpenIn = read_wire(body).ok_or_else(bad)?;
            Op::Open { flags: args.flags }
        }
        OP_OPENDIR => {
            let args: OpenIn = read_wire(body).ok_or_else(bad)?;
            Op::OpenDir { flags: args.flags }
        }
        OP_READ => Op::Read(read_wire(body).ok_or_else(bad)?),
        OP_READDIR => Op::ReadDir(read_wire(body).ok_or_else(bad)?),
        OP_RELEASE => {
            let args: ReleaseIn = read_wire(body).ok_or_else(bad)?;
            Op::Release { fh: args.fh }
        }
        OP_RELEASEDIR => {
            let args: ReleaseIn = read_wire(body).ok_or_else(bad)?;
            Op::ReleaseDir { fh: args.fh }
        }
        OP_STATFS => Op::StatFs,
        OP_INTERRUPT => Op::Interrupt,
        opcode => Op::Other { opcode },
    };
    Ok(Request {
        unique: header.unique,
        nodeid: header.nodeid,
        uid: header.uid,
        gid: header.gid,
        pid: header.pid,
        op,
    })
}

/// Encodes a request, as the kernel would send it. Used by tests and by any client that talks to a session.
pub fn encode_request(opcode: u32, unique: u64, nodeid: u64, body: &[u8]) -> Vec<u8> {
    let header = InHeader {
        len: (size_of::<InHeader>() + body.len()) as u32,
        opcode,
        unique,
        nodeid,
        ..Default::default()
    };
    let mut out = Vec::with_capacity(header.len as usize);
    out.extend_from_slice(wire_bytes(&header));
    out.extend_from_slice(body);
    out
}

fn reply(unique: u64, error: i32, body: &[u8]) -> Vec<u8> {
    let header = OutHeader {
        len: (size_of::<OutHeader>() + body.len()) as u32,
        error,
        unique,
    };
    let mut out = Vec::with_capacity(header.len as usize);
    out.extend_from_slice(wire_bytes(&header));
    out.extend_from_slice(body);
    out
}

/// A successful reply carrying `body`.
pub fn reply_ok(unique: u64, body: &[u8]) -> Vec<u8> {
    reply(unique, 0, body)
}

/// An error reply: the header alone, with `error` set to `-errno`. `errno` is the positive Linux value.
pub fn reply_err(unique: u64, errno: i32) -> Vec<u8> {
    reply(unique, -errno, &[])
}

/// Packs READDIR entries into a reply body no longer than `limit` bytes.
///
/// Each entry is the 24-byte `Dirent` header, its name and zero bytes up to the next multiple of 8. The offset written
/// into an entry is the position of the entry after it, which is what the kernel passes back to resume the listing.
pub struct DirentWriter {
    buf: Vec<u8>,
    limit: usize,
}

impl DirentWriter {
    pub fn new(limit: usize) -> DirentWriter {
        DirentWriter { buf: Vec::new(), limit }
    }

    /// Appends one entry. Returns false, leaving the buffer as it was, when the entry does not fit in the limit.
    pub fn push(&mut self, ino: u64, off: u64, typ: u32, name: &[u8]) -> bool {
        let len = dirent_len(name.len());
        if self.buf.len() + len > self.limit {
            return false;
        }
        let start = self.buf.len();
        let header = Dirent {
            ino,
            off,
            namelen: name.len() as u32,
            typ,
        };
        self.buf.extend_from_slice(wire_bytes(&header));
        self.buf.extend_from_slice(name);
        self.buf.resize(start + len, 0);
        true
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }
}

/// Splits a READDIR body back into its entries, for clients and tests. `None` when an entry is truncated.
pub fn decode_dirents(mut body: &[u8]) -> Option<Vec<(Dirent, &[u8])>> {
    let mut entries = Vec::new();
    while !body.is_empty() {
        let header: Dirent = read_wire(body)?;
        let name = body.get(DIRENT_HEADER..DIRENT_HEADER + header.namelen as usize)?;
        entries.push((header, name));
        body = body.get(dirent_len(name.len())..).unwrap_or(&[]);
    }
    Some(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_header_of_a_request() {
        let mut body = Vec::new();
        body.extend_from_slice(wire_bytes(&InitIn {
            major: 7,
            minor: 36,
            max_readahead: 131072,
            flags: 0,
        }));
        let buf = encode_request(OP_INIT, 9, 0, &body);
        let req = decode(&buf).unwrap();
        assert_eq!(req.unique, 9);
        assert_eq!(req.nodeid, 0);
        assert_eq!(
            req.op,
            Op::Init(InitIn {
                major: 7,
                minor: 36,
                max_readahead: 131072,
                flags: 0,
            })
        );
    }

    #[test]
    fn decodes_a_lookup_with_its_nul_terminated_name() {
        let buf = encode_request(OP_LOOKUP, 3, 1, b"notes.txt\0");
        let req = decode(&buf).unwrap();
        assert_eq!(req.nodeid, ROOT_NODE_ID);
        assert_eq!(req.op, Op::Lookup { name: b"notes.txt" });
        // A name without the terminator is kept whole.
        let buf = encode_request(OP_LOOKUP, 3, 1, b"plain");
        assert_eq!(decode(&buf).unwrap().op, Op::Lookup { name: b"plain" });
    }

    #[test]
    fn decodes_read_and_readdir_arguments_from_the_body() {
        let args = ReadIn {
            fh: 5,
            offset: 4096,
            size: 512,
            ..Default::default()
        };
        let buf = encode_request(OP_READ, 4, 2, wire_bytes(&args));
        assert_eq!(decode(&buf).unwrap().op, Op::Read(args));
        let buf = encode_request(OP_READDIR, 4, 2, wire_bytes(&args));
        assert_eq!(decode(&buf).unwrap().op, Op::ReadDir(args));
    }

    #[test]
    fn a_body_shorter_than_its_struct_is_refused() {
        let buf = encode_request(OP_READ, 4, 2, &[0u8; 8]);
        assert_eq!(decode(&buf), Err(DecodeError::BadBody));
    }

    #[test]
    fn rejects_lengths_outside_the_buffer() {
        assert_eq!(decode(&[0u8; 10]), Err(DecodeError::Short));
        let mut buf = encode_request(OP_STATFS, 1, 1, &[]);
        // Header claims more bytes than the buffer holds.
        buf[0..4].copy_from_slice(&64u32.to_ne_bytes());
        assert_eq!(decode(&buf), Err(DecodeError::BadLength));
        // Header claims fewer bytes than the header itself.
        buf[0..4].copy_from_slice(&8u32.to_ne_bytes());
        assert_eq!(decode(&buf), Err(DecodeError::BadLength));
    }

    #[test]
    fn unknown_and_no_reply_opcodes_decode_as_such() {
        assert_eq!(decode(&encode_request(5, 1, 1, &[])).unwrap().op, Op::Other { opcode: 5 });
        assert_eq!(decode(&encode_request(OP_FORGET, 1, 1, &[0; 8])).unwrap().op, Op::Forget);
        assert!(!expects_reply(OP_FORGET));
        assert!(!expects_reply(OP_BATCH_FORGET));
        assert!(!expects_reply(OP_INTERRUPT));
        assert!(expects_reply(OP_DESTROY));
        assert!(expects_reply(OP_LOOKUP));
    }

    #[test]
    fn init_answer_caps_the_minor_at_7_31() {
        let newer = InitOut::answer(&InitIn {
            major: 7,
            minor: 40,
            max_readahead: 1,
            flags: 0,
        });
        assert_eq!((newer.major, newer.minor), (7, 31));
        assert_eq!(newer.max_readahead, 1);
        assert_eq!(newer.flags, 0);
        let older = InitOut::answer(&InitIn {
            major: 7,
            minor: 27,
            ..Default::default()
        });
        assert_eq!(older.minor, 27);
        assert_eq!(wire_bytes(&newer).len(), 64);
    }

    #[test]
    fn reply_sizes_follow_the_header_and_body() {
        let ok = reply_ok(7, &[1, 2, 3, 4]);
        assert_eq!(ok.len(), 16 + 4);
        let header: OutHeader = read_wire(&ok).unwrap();
        assert_eq!((header.len, header.error, header.unique), (20, 0, 7));
        let err = reply_err(8, 2);
        assert_eq!(err.len(), 16);
        let header: OutHeader = read_wire(&err).unwrap();
        assert_eq!((header.len, header.error, header.unique), (16, -2, 8));
    }

    #[test]
    fn dirent_entries_are_padded_to_eight_bytes() {
        assert_eq!(DIRENT_HEADER, 24);
        for name_len in 0..=40 {
            let len = dirent_len(name_len);
            assert_eq!(len % 8, 0, "name of {name_len} bytes");
            assert!(len >= 24 + name_len);
            assert!(len < 24 + name_len + 8);
        }
        assert_eq!(dirent_len(1), 32);
        assert_eq!(dirent_len(8), 32);
        assert_eq!(dirent_len(9), 40);
    }

    #[test]
    fn dirent_writer_packs_aligned_entries_and_refuses_overflow() {
        let mut w = DirentWriter::new(dirent_len(3) + dirent_len(10) - 1);
        assert!(w.push(2, 1, 8, b"abc"));
        // The second entry would overflow the limit, so it is refused and nothing is written.
        assert!(!w.push(3, 2, 4, b"0123456789"));
        let bytes = w.into_bytes();
        assert_eq!(bytes.len(), dirent_len(3));
        assert_eq!(bytes.len() % 8, 0);
        let entries = decode_dirents(&bytes).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0.ino, 2);
        assert_eq!(entries[0].0.off, 1);
        assert_eq!(entries[0].1, b"abc");
    }

    #[test]
    fn decode_dirents_reads_back_every_packed_entry() {
        let mut w = DirentWriter::new(4096);
        for (i, name) in [&b"."[..], b"..", b"hello.txt", b"a-much-longer-file-name.txt"].iter().enumerate() {
            assert!(w.push(10 + i as u64, i as u64 + 1, 8, name));
        }
        let bytes = w.into_bytes();
        let entries = decode_dirents(&bytes).unwrap();
        let names: Vec<&[u8]> = entries.iter().map(|(_, name)| *name).collect();
        assert_eq!(names, vec![&b"."[..], b"..", b"hello.txt", b"a-much-longer-file-name.txt"]);
        let offsets: Vec<u64> = entries.iter().map(|(d, _)| d.off).collect();
        assert_eq!(offsets, vec![1, 2, 3, 4]);
    }

    #[test]
    fn a_truncated_dirent_is_refused() {
        let mut w = DirentWriter::new(4096);
        assert!(w.push(2, 1, 8, b"name"));
        let bytes = w.into_bytes();
        assert!(decode_dirents(&bytes[..bytes.len() - 8]).is_none());
    }
}
