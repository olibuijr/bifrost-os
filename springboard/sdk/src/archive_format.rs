//! The BifrOSt archive format (`BAR1`): a container for a directory tree that keeps file attributes, compresses the
//! whole tree with any codec of [`crate::compression`], and can be signed and sealed. It is the counterpart of the
//! reference system's archive container: a header with the file attributes, a stream of entries, directory pack and
//! extract, and signed and encrypted forms. It is not the reference byte format; the files are `.bar`.
//!
//! # Layout (all integers little-endian)
//!
//! ```text
//! 0   "BAR1"          magic
//! 4   u8              version (1)
//! 5   u8              compression: 0 none, 1 deflate, 2 lz4, 3 lzma
//! 6   u16             reserved, zero
//! 8   u32             entry count
//! 12  u64             length of the payload after decompression
//! 20  [u8; 32]        SHA-256 of the payload after decompression
//! 52  ...             the payload, compressed as one stream
//! ```
//!
//! The payload is the entries one after another. Each entry is a header and then its data:
//!
//! ```text
//! u8 kind (1 file, 2 folder, 3 symbolic link)   u16 path length   path (UTF-8, '/' separated, relative)
//! u32 mode  u32 uid  u32 gid  i64 mtime seconds  u32 mtime nanoseconds  u64 data length  u16 attribute count
//! attributes: u16 name length, u32 value length, name, value
//! data: the file contents, or the link target for a symbolic link (a folder has none)
//! ```
//!
//! # Hostile input
//!
//! A decoder refuses, before it allocates for them: a count of entries or a payload length past [`Limits`], an entry
//! count that cannot fit in the payload, an entry or attribute that runs past the end, a path that is absolute, has
//! `..`, `.`, an empty part, a backslash or a control character, a path that appears twice, and a link whose target
//! is absolute or climbs out of the archive root. The decompressed size is capped at the length the header declares,
//! so a small input cannot expand without bound, and the SHA-256 must match. [`extract`] checks every entry before
//! it writes any, never writes through a symbolic link (each parent folder must be a real folder), never replaces an
//! existing file, and drops the set-user-id, set-group-id and sticky bits.
//!
//! Extraction assumes nobody else is changing the destination folder while it runs.
//!
//! # Signed and sealed
//!
//! [`sign`] puts an ML-DSA signature (FIPS 204, [`crate::signature_pq`]) in front of the archive bytes, and [`verify`]
//! checks it against a public key the caller trusts. [`encode_sealed`] seals the archive under a ChaCha20-Poly1305 key
//! with [`crate::encrypted_archive`]. [`encode_signed_sealed`] signs and then seals, so the signature is not visible
//! without the key. Keys and their storage are the caller's.
//!
//! ```ignore
//! let packed = archive_format::pack_directory(&folder, &Limits::default())?;
//! let bytes = archive_format::encode(&packed.entries, Compression::Lzma)?;
//! let signed = archive_format::sign(&bytes, &private_key)?;
//! let entries = archive_format::decode(archive_format::verify(&signed, &public_key)?, &Limits::default())?;
//! archive_format::extract(&entries, &dest, &ExtractOptions::default())?;
//! ```

use crate::archive::ByteStream;
use crate::chacha_poly::ChaChaPolyKey;
use crate::compression::{self, Algorithm};
use crate::digest;
use crate::encrypted_archive;
use crate::file_metadata;
use crate::signature_pq::{PrivateKey, PublicKey};
use std::collections::HashSet;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{chown, lchown, symlink, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The first four bytes of an archive.
pub const MAGIC: &[u8; 4] = b"BAR1";
/// The format version this module writes and reads.
pub const VERSION: u8 = 1;
/// Bytes before the payload.
pub const HEADER_BYTES: usize = 52;
/// The first four bytes of a signed archive.
pub const SIGNED_MAGIC: &[u8; 4] = b"BARS";
/// The signature context: a signature over an archive does not verify as anything else.
const SIGN_CONTEXT: &[u8] = b"bifrost-archive/v1";
/// The longest signature a signed archive may carry (an ML-DSA-87 signature is 4627 bytes).
const MAX_SIGNATURE: usize = 8192;

/// The longest path in bytes.
pub const MAX_PATH: usize = 4096;
/// The longest path component in bytes.
pub const MAX_NAME: usize = 255;
/// The most components in a path.
pub const MAX_DEPTH: usize = 64;
/// The longest attribute value in bytes.
pub const MAX_XATTR_VALUE: usize = 64 * 1024;
/// The fewest payload bytes one entry takes: the fixed header fields and a one-byte path.
const MIN_ENTRY_BYTES: u64 = 1 + 2 + 1 + 4 + 4 + 4 + 8 + 4 + 8 + 2;

/// What an entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Directory,
    Symlink,
}

impl Kind {
    fn code(self) -> u8 {
        match self {
            Kind::File => 1,
            Kind::Directory => 2,
            Kind::Symlink => 3,
        }
    }

    fn from_code(code: u8) -> Option<Kind> {
        match code {
            1 => Some(Kind::File),
            2 => Some(Kind::Directory),
            3 => Some(Kind::Symlink),
            _ => None,
        }
    }
}

/// How the payload is compressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    None,
    Deflate,
    Lz4,
    Lzma,
}

impl Compression {
    fn code(self) -> u8 {
        match self {
            Compression::None => 0,
            Compression::Deflate => 1,
            Compression::Lz4 => 2,
            Compression::Lzma => 3,
        }
    }

    fn from_code(code: u8) -> Option<Compression> {
        match code {
            0 => Some(Compression::None),
            1 => Some(Compression::Deflate),
            2 => Some(Compression::Lz4),
            3 => Some(Compression::Lzma),
            _ => None,
        }
    }

    fn algorithm(self) -> Option<Algorithm> {
        match self {
            Compression::None => None,
            Compression::Deflate => Some(Algorithm::Deflate),
            Compression::Lz4 => Some(Algorithm::Lz4),
            Compression::Lzma => Some(Algorithm::Lzma),
        }
    }
}

/// Why an archive could not be made, read or extracted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The data does not start with the magic, or is too short for the header.
    NotArchive,
    /// The version byte names a format this module does not read.
    UnsupportedVersion(u8),
    /// The compression byte names a codec this module does not know.
    UnsupportedCompression(u8),
    /// The data ends before the archive does.
    Truncated,
    /// The structure is damaged: the SHA-256 does not match, a field is out of range, or bytes are left over.
    Corrupt(&'static str),
    /// More entries than [`Limits::max_entries`].
    TooManyEntries,
    /// More payload bytes than [`Limits::max_payload_bytes`].
    TooBig,
    /// A folder tree deeper than [`MAX_DEPTH`].
    TooDeep,
    /// A path the rules refuse (absolute, `..`, empty part, control character, too long).
    UnsafePath(String),
    /// A symbolic link whose target is absolute, climbs out of the root, or is not text.
    UnsafeLink(String),
    /// A path that appears twice.
    Duplicate(String),
    /// A file system error.
    Io(io::ErrorKind, String),
    /// The compression codec failed.
    Compression,
    /// The signature is missing, damaged or made by another key.
    BadSignature,
    /// Signing failed (the random pool is unavailable).
    SigningFailed,
    /// The sealed form could not be made or opened.
    Sealed(encrypted_archive::Error),
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e.kind(), e.to_string())
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotArchive => f.write_str("data is not an archive"),
            Error::UnsupportedVersion(v) => write!(f, "archive version {v} is not supported"),
            Error::UnsupportedCompression(c) => write!(f, "archive compression {c} is not supported"),
            Error::Truncated => f.write_str("the archive ends early"),
            Error::Corrupt(why) => write!(f, "the archive is damaged: {why}"),
            Error::TooManyEntries => f.write_str("the archive has more entries than the limit"),
            Error::TooBig => f.write_str("the archive is larger than the limit"),
            Error::TooDeep => f.write_str("the folder tree is deeper than the limit"),
            Error::UnsafePath(p) => write!(f, "unsafe path: {p}"),
            Error::UnsafeLink(p) => write!(f, "unsafe link: {p}"),
            Error::Duplicate(p) => write!(f, "path appears twice: {p}"),
            Error::Io(_, why) => write!(f, "file system error: {why}"),
            Error::Compression => f.write_str("the compression codec failed"),
            Error::BadSignature => f.write_str("the signature does not match"),
            Error::SigningFailed => f.write_str("signing failed"),
            Error::Sealed(e) => write!(f, "sealed archive: {e}"),
        }
    }
}

/// What a reader will accept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The most entries.
    pub max_entries: usize,
    /// The most payload bytes after decompression (paths, attributes and data).
    pub max_payload_bytes: u64,
    /// The most attributes on one entry.
    pub max_xattrs: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits { max_entries: 1_000_000, max_payload_bytes: 1 << 30, max_xattrs: 64 }
    }
}

/// The attributes of one entry (the archive header of the reference API).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// The path relative to the archive root, `/` separated.
    pub path: String,
    pub kind: Kind,
    /// The permission bits. Extraction keeps the low nine and drops the rest.
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    /// The modification time: seconds since 1970 and nanoseconds.
    pub mtime_secs: i64,
    pub mtime_nanos: u32,
    /// The length of the data. [`encode`] writes the length of `Entry::data`.
    pub size: u64,
    /// Extended attributes, name and value.
    pub xattrs: Vec<(String, Vec<u8>)>,
}

/// One entry: its attributes and its data (file contents, or the link target as UTF-8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub header: Header,
    pub data: Vec<u8>,
}

impl Entry {
    fn new(path: &str, kind: Kind, mode: u32, data: Vec<u8>) -> Entry {
        Entry {
            header: Header { path: path.to_string(), kind, mode, uid: 0, gid: 0, mtime_secs: 0, mtime_nanos: 0, size: data.len() as u64, xattrs: Vec::new() },
            data,
        }
    }

    /// A regular file.
    pub fn file(path: &str, data: &[u8], mode: u32) -> Entry {
        Entry::new(path, Kind::File, mode, data.to_vec())
    }

    /// A folder.
    pub fn directory(path: &str, mode: u32) -> Entry {
        Entry::new(path, Kind::Directory, mode, Vec::new())
    }

    /// A symbolic link to `target`.
    pub fn symlink(path: &str, target: &str) -> Entry {
        Entry::new(path, Kind::Symlink, 0o777, target.as_bytes().to_vec())
    }
}

/// Checks a path against the rules in the module doc.
pub fn validate_path(path: &str) -> Result<(), Error> {
    let bad = || Error::UnsafePath(path.to_string());
    if path.is_empty() || path.len() > MAX_PATH || path.starts_with('/') || path.contains('\\') || path.chars().any(|c| c.is_control()) {
        return Err(bad());
    }
    let mut parts = 0;
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.len() > MAX_NAME {
            return Err(bad());
        }
        parts += 1;
    }
    if parts > MAX_DEPTH {
        return Err(bad());
    }
    Ok(())
}

/// Checks the target of the link at `path`: relative, no control characters, and never above the archive root.
fn validate_link(path: &str, target: &str) -> Result<(), Error> {
    let bad = || Error::UnsafeLink(path.to_string());
    if target.is_empty() || target.len() > MAX_PATH || target.starts_with('/') || target.contains('\\') || target.chars().any(|c| c.is_control()) {
        return Err(bad());
    }
    // the number of folders between the root and the link
    let mut depth = path.split('/').count() as i64 - 1;
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return Err(bad());
                }
            }
            _ => depth += 1,
        }
    }
    Ok(())
}

fn validate_entry(e: &Entry, max_xattrs: usize) -> Result<(), Error> {
    let h = &e.header;
    validate_path(&h.path)?;
    if h.mtime_nanos >= 1_000_000_000 {
        return Err(Error::Corrupt("nanoseconds out of range"));
    }
    match h.kind {
        Kind::Directory => {
            if !e.data.is_empty() {
                return Err(Error::Corrupt("a folder has data"));
            }
        }
        Kind::Symlink => {
            let target = std::str::from_utf8(&e.data).map_err(|_| Error::UnsafeLink(h.path.clone()))?;
            validate_link(&h.path, target)?;
        }
        Kind::File => {}
    }
    if h.xattrs.len() > max_xattrs || h.xattrs.len() > u16::MAX as usize {
        return Err(Error::Corrupt("too many attributes"));
    }
    for (name, value) in &h.xattrs {
        if name.is_empty() || name.len() > MAX_NAME || name.contains('\0') || value.len() > MAX_XATTR_VALUE {
            return Err(Error::Corrupt("bad attribute"));
        }
    }
    Ok(())
}

fn validate_all(entries: &[Entry], max_xattrs: usize) -> Result<(), Error> {
    let mut seen: HashSet<&str> = HashSet::new();
    for e in entries {
        validate_entry(e, max_xattrs)?;
        if !seen.insert(e.header.path.as_str()) {
            return Err(Error::Duplicate(e.header.path.clone()));
        }
    }
    Ok(())
}

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Writes the archive for `entries`. Every entry is checked first, so an unsafe path or link is refused here too.
pub fn encode(entries: &[Entry], compression: Compression) -> Result<Vec<u8>, Error> {
    let limits = Limits::default();
    if entries.len() > limits.max_entries {
        return Err(Error::TooManyEntries);
    }
    validate_all(entries, limits.max_xattrs)?;
    let mut payload: Vec<u8> = Vec::new();
    for e in entries {
        let h = &e.header;
        payload.push(h.kind.code());
        put_u16(&mut payload, h.path.len() as u16);
        payload.extend_from_slice(h.path.as_bytes());
        put_u32(&mut payload, h.mode);
        put_u32(&mut payload, h.uid);
        put_u32(&mut payload, h.gid);
        payload.extend_from_slice(&h.mtime_secs.to_le_bytes());
        put_u32(&mut payload, h.mtime_nanos);
        payload.extend_from_slice(&(e.data.len() as u64).to_le_bytes());
        put_u16(&mut payload, h.xattrs.len() as u16);
        for (name, value) in &h.xattrs {
            put_u16(&mut payload, name.len() as u16);
            put_u32(&mut payload, value.len() as u32);
            payload.extend_from_slice(name.as_bytes());
            payload.extend_from_slice(value);
        }
        payload.extend_from_slice(&e.data);
        if payload.len() as u64 > limits.max_payload_bytes {
            return Err(Error::TooBig);
        }
    }
    let digest = digest::sha256(&payload);
    let body = match compression.algorithm() {
        None => payload.clone(),
        Some(alg) => compression::encode_buffer(alg, &payload).map_err(|_| Error::Compression)?,
    };
    let mut out = Vec::with_capacity(HEADER_BYTES + body.len());
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.push(compression.code());
    put_u16(&mut out, 0);
    put_u32(&mut out, entries.len() as u32);
    out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    out.extend_from_slice(&digest);
    out.extend_from_slice(&body);
    Ok(out)
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.pos.checked_add(n).ok_or(Error::Truncated)?;
        let slice = self.buf.get(self.pos..end).ok_or(Error::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, Error> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, Error> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, Error> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    fn i64(&mut self) -> Result<i64, Error> {
        Ok(self.u64()? as i64)
    }

    fn text(&mut self, n: usize, what: &'static str) -> Result<String, Error> {
        let bytes = self.take(n)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| Error::Corrupt(what))
    }
}

/// The entry count and the compression of an archive, read from the header without decompressing anything.
pub fn peek(data: &[u8]) -> Result<(u32, Compression, u64), Error> {
    if data.len() < HEADER_BYTES || !data.starts_with(MAGIC) {
        return Err(Error::NotArchive);
    }
    if data[4] != VERSION {
        return Err(Error::UnsupportedVersion(data[4]));
    }
    let compression = Compression::from_code(data[5]).ok_or(Error::UnsupportedCompression(data[5]))?;
    if data[6] != 0 || data[7] != 0 {
        return Err(Error::Corrupt("reserved bytes are not zero"));
    }
    let count = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    let mut len = [0u8; 8];
    len.copy_from_slice(&data[12..20]);
    Ok((count, compression, u64::from_le_bytes(len)))
}

/// Reads an archive: checks the header, decompresses within the declared length, checks the SHA-256, and parses and
/// validates every entry. Nothing is returned for an archive with any bad entry.
pub fn decode(data: &[u8], limits: &Limits) -> Result<Vec<Entry>, Error> {
    let (count, compression, payload_len) = peek(data)?;
    if count as usize > limits.max_entries {
        return Err(Error::TooManyEntries);
    }
    if payload_len > limits.max_payload_bytes {
        return Err(Error::TooBig);
    }
    if (count as u64).saturating_mul(MIN_ENTRY_BYTES) > payload_len {
        return Err(Error::Corrupt("more entries than the payload can hold"));
    }
    let payload_len = usize::try_from(payload_len).map_err(|_| Error::TooBig)?;
    let body = &data[HEADER_BYTES..];
    let payload: Vec<u8> = match compression.algorithm() {
        None => body.to_vec(),
        Some(alg) => compression::decode_buffer_limited(alg, body, payload_len).map_err(|e| match e {
            compression::Error::Truncated => Error::Truncated,
            compression::Error::TooBig => Error::Corrupt("the payload is larger than its header says"),
            _ => Error::Corrupt("the compressed payload is damaged"),
        })?,
    };
    if payload.len() != payload_len {
        return Err(Error::Corrupt("the payload length does not match its header"));
    }
    if !digest::constant_time_eq(&digest::sha256(&payload), &data[20..52]) {
        return Err(Error::Corrupt("the checksum does not match"));
    }
    let mut r = Reader { buf: &payload, pos: 0 };
    let mut entries: Vec<Entry> = Vec::with_capacity((count as usize).min(4096));
    let mut seen: HashSet<String> = HashSet::new();
    for _ in 0..count {
        let kind = Kind::from_code(r.u8()?).ok_or(Error::Corrupt("unknown entry kind"))?;
        let path_len = r.u16()? as usize;
        let path = r.text(path_len, "path is not UTF-8")?;
        let mode = r.u32()?;
        let uid = r.u32()?;
        let gid = r.u32()?;
        let mtime_secs = r.i64()?;
        let mtime_nanos = r.u32()?;
        let size = r.u64()?;
        let xattr_count = r.u16()? as usize;
        if xattr_count > limits.max_xattrs {
            return Err(Error::Corrupt("too many attributes"));
        }
        let mut xattrs = Vec::with_capacity(xattr_count);
        for _ in 0..xattr_count {
            let name_len = r.u16()? as usize;
            let value_len = r.u32()? as usize;
            if value_len > MAX_XATTR_VALUE {
                return Err(Error::Corrupt("bad attribute"));
            }
            let name = r.text(name_len, "attribute name is not UTF-8")?;
            let value = r.take(value_len)?.to_vec();
            xattrs.push((name, value));
        }
        let size_usize = usize::try_from(size).map_err(|_| Error::Truncated)?;
        let body = r.take(size_usize)?.to_vec();
        let entry = Entry { header: Header { path, kind, mode, uid, gid, mtime_secs, mtime_nanos, size, xattrs }, data: body };
        validate_entry(&entry, limits.max_xattrs)?;
        if !seen.insert(entry.header.path.clone()) {
            return Err(Error::Duplicate(entry.header.path));
        }
        entries.push(entry);
    }
    if r.pos != payload.len() {
        return Err(Error::Corrupt("bytes after the last entry"));
    }
    Ok(entries)
}

/// Writes the archive for `entries` at the cursor of `stream` and returns the number of bytes written.
pub fn encode_to(stream: &mut ByteStream, entries: &[Entry], compression: Compression) -> Result<usize, Error> {
    let bytes = encode(entries, compression)?;
    stream.write_all(&bytes)?;
    Ok(bytes.len())
}

/// Reads an archive from the cursor to the end of `stream`.
pub fn decode_from(stream: &mut ByteStream, limits: &Limits) -> Result<Vec<Entry>, Error> {
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes)?;
    decode(&bytes, limits)
}

/// The entries of a folder, and the paths that were left out.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Packed {
    pub entries: Vec<Entry>,
    /// Paths skipped because they are not regular files, folders or safe links (sockets, devices, names that are not
    /// text or hold control characters, links that climb out of the folder).
    pub skipped: Vec<String>,
}

/// Reads a folder tree into entries, sorted by name so the same tree gives the same archive. Links are recorded, never
/// followed. Paths are relative to `root`, which is not itself an entry. Mode, owner, modification time and extended
/// attributes are kept.
pub fn pack_directory(root: &Path, limits: &Limits) -> Result<Packed, Error> {
    if !fs::metadata(root)?.is_dir() {
        return Err(Error::Io(io::ErrorKind::InvalidInput, "not a folder".to_string()));
    }
    let mut packed = Packed::default();
    let mut total = 0u64;
    walk(root, "", 0, limits, &mut total, &mut packed)?;
    Ok(packed)
}

fn walk(dir: &Path, rel: &str, depth: usize, limits: &Limits, total: &mut u64, out: &mut Packed) -> Result<(), Error> {
    if depth >= MAX_DEPTH {
        return Err(Error::TooDeep);
    }
    let mut names: Vec<std::ffi::OsString> = Vec::new();
    for item in fs::read_dir(dir)? {
        names.push(item?.file_name());
    }
    names.sort();
    for name in names {
        let Some(name_text) = name.to_str().map(str::to_string) else {
            out.skipped.push(name.to_string_lossy().into_owned());
            continue;
        };
        let rel_path = if rel.is_empty() { name_text } else { format!("{rel}/{name_text}") };
        let full = dir.join(&name);
        let meta = fs::symlink_metadata(&full)?;
        if out.entries.len() >= limits.max_entries {
            return Err(Error::TooManyEntries);
        }
        let file_type = meta.file_type();
        let (kind, data) = if file_type.is_dir() {
            (Kind::Directory, Vec::new())
        } else if file_type.is_symlink() {
            match fs::read_link(&full)?.to_str() {
                Some(target) => (Kind::Symlink, target.as_bytes().to_vec()),
                None => {
                    out.skipped.push(rel_path);
                    continue;
                }
            }
        } else if file_type.is_file() {
            if total.saturating_add(meta.len()) > limits.max_payload_bytes {
                return Err(Error::TooBig);
            }
            (Kind::File, fs::read(&full)?)
        } else {
            out.skipped.push(rel_path);
            continue;
        };
        *total += data.len() as u64 + rel_path.len() as u64 + MIN_ENTRY_BYTES;
        if *total > limits.max_payload_bytes {
            return Err(Error::TooBig);
        }
        let mut xattrs: Vec<(String, Vec<u8>)> = Vec::new();
        if let Ok(names) = file_metadata::list_xattrs(&full) {
            for attr in names {
                if xattrs.len() >= limits.max_xattrs {
                    break;
                }
                if let Ok(Some(value)) = file_metadata::xattr(&full, &attr) {
                    if value.len() <= MAX_XATTR_VALUE {
                        xattrs.push((attr, value));
                    }
                }
            }
        }
        xattrs.sort();
        let entry = Entry {
            header: Header {
                path: rel_path.clone(),
                kind,
                mode: meta.mode() & 0o7777,
                uid: meta.uid(),
                gid: meta.gid(),
                mtime_secs: meta.mtime(),
                mtime_nanos: (meta.mtime_nsec().clamp(0, 999_999_999)) as u32,
                size: data.len() as u64,
                xattrs,
            },
            data,
        };
        if validate_entry(&entry, limits.max_xattrs).is_err() {
            out.skipped.push(rel_path);
            continue;
        }
        out.entries.push(entry);
        if file_type.is_dir() {
            walk(&full, &rel_path, depth + 1, limits, total, out)?;
        }
    }
    Ok(())
}

/// What [`extract`] does beyond writing names and contents.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExtractOptions {
    /// Set the owner and group from the archive. Needs the privilege to do it; a failure is an error.
    pub preserve_ownership: bool,
    /// Set the extended attributes. A failure is counted in [`Extraction::xattr_failures`], not raised, because
    /// some file systems have none.
    pub restore_xattrs: bool,
}

/// What [`extract`] wrote.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Extraction {
    pub files: usize,
    pub directories: usize,
    pub symlinks: usize,
    /// File contents written, in bytes.
    pub bytes: u64,
    pub xattr_failures: usize,
}

fn header_time(h: &Header) -> Option<SystemTime> {
    if h.mtime_secs >= 0 {
        UNIX_EPOCH.checked_add(Duration::new(h.mtime_secs as u64, h.mtime_nanos))
    } else {
        UNIX_EPOCH
            .checked_sub(Duration::from_secs(h.mtime_secs.unsigned_abs()))?
            .checked_add(Duration::from_nanos(h.mtime_nanos as u64))
    }
}

/// A parent folder on the way to an entry: it must be a real folder. A missing one is created. A link or a file
/// in its place is an error, so nothing is ever written through a link.
fn ensure_real_dir(path: &Path, shown: &str) -> Result<(), Error> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_dir() => Ok(()),
        Ok(_) => Err(Error::UnsafePath(shown.to_string())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(fs::create_dir(path)?),
        Err(e) => Err(e.into()),
    }
}

/// Writes `entries` below `dest` (created if missing). Every entry is checked before anything is written. An existing
/// file or link is never replaced, and an entry whose parent is a link or a file is refused. File modes keep the
/// low nine bits.
pub fn extract(entries: &[Entry], dest: &Path, options: &ExtractOptions) -> Result<Extraction, Error> {
    validate_all(entries, usize::MAX)?;
    fs::create_dir_all(dest)?;
    let mut done = Extraction::default();
    // folder modes and times are set last, deepest first, so a read-only folder does not block its own contents
    let mut folders: Vec<(PathBuf, &Header)> = Vec::new();
    for e in entries {
        let h = &e.header;
        let parts: Vec<&str> = h.path.split('/').collect();
        let mut current = dest.to_path_buf();
        for part in &parts[..parts.len() - 1] {
            current.push(part);
            ensure_real_dir(&current, &h.path)?;
        }
        let target = current.join(parts[parts.len() - 1]);
        match h.kind {
            Kind::Directory => {
                ensure_real_dir(&target, &h.path)?;
                done.directories += 1;
                folders.push((target.clone(), h));
            }
            Kind::File => {
                let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&target)?;
                file.write_all(&e.data)?;
                file.set_permissions(fs::Permissions::from_mode(h.mode & 0o777))?;
                if let Some(time) = header_time(h) {
                    let _ = file.set_modified(time);
                }
                done.files += 1;
                done.bytes += e.data.len() as u64;
            }
            Kind::Symlink => {
                let link_target = std::str::from_utf8(&e.data).map_err(|_| Error::UnsafeLink(h.path.clone()))?;
                symlink(link_target, &target)?;
                done.symlinks += 1;
            }
        }
        if options.preserve_ownership {
            if h.kind == Kind::Symlink {
                lchown(&target, Some(h.uid), Some(h.gid))?;
            } else {
                chown(&target, Some(h.uid), Some(h.gid))?;
            }
        }
        if options.restore_xattrs {
            for (name, value) in &h.xattrs {
                if file_metadata::set_xattr(&target, name, value).is_err() {
                    done.xattr_failures += 1;
                }
            }
        }
    }
    for (path, h) in folders.into_iter().rev() {
        fs::set_permissions(&path, fs::Permissions::from_mode(h.mode & 0o777))?;
        if let (Some(time), Ok(dir)) = (header_time(h), fs::File::open(&path)) {
            let _ = dir.set_modified(time);
        }
    }
    Ok(done)
}

/// Packs the folder `src` into the file `dest_file` (which must not exist). Returns the paths that were left out.
pub fn archive_directory(src: &Path, dest_file: &Path, compression: Compression, limits: &Limits) -> Result<Vec<String>, Error> {
    let packed = pack_directory(src, limits)?;
    let bytes = encode(&packed.entries, compression)?;
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(dest_file)?;
    file.write_all(&bytes)?;
    Ok(packed.skipped)
}

/// Reads the archive file `archive` and extracts it below `dest`. A file larger than the limits allow is refused
/// before it is read.
pub fn extract_archive(archive: &Path, dest: &Path, limits: &Limits, options: &ExtractOptions) -> Result<Extraction, Error> {
    let size = fs::metadata(archive)?.len();
    if size > limits.max_payload_bytes.saturating_mul(2).saturating_add(4096) {
        return Err(Error::TooBig);
    }
    let bytes = fs::read(archive)?;
    let entries = decode(&bytes, limits)?;
    extract(&entries, dest, options)
}

/// Signs `archive` (the bytes of [`encode`]) and returns the signed form: `BARS`, a version byte, the signature length
/// as a u32, the signature, then the archive.
pub fn sign(archive: &[u8], key: &PrivateKey) -> Result<Vec<u8>, Error> {
    let signature = key.sign(archive, SIGN_CONTEXT).map_err(|_| Error::SigningFailed)?;
    let mut out = Vec::with_capacity(9 + signature.len() + archive.len());
    out.extend_from_slice(SIGNED_MAGIC);
    out.push(VERSION);
    put_u32(&mut out, signature.len() as u32);
    out.extend_from_slice(&signature);
    out.extend_from_slice(archive);
    Ok(out)
}

/// Checks a signed archive against `key` and returns the archive bytes inside it. Any damage, a different key or a
/// signature made for something else is [`Error::BadSignature`].
pub fn verify<'a>(signed: &'a [u8], key: &PublicKey) -> Result<&'a [u8], Error> {
    if signed.len() < 9 || !signed.starts_with(SIGNED_MAGIC) {
        return Err(Error::BadSignature);
    }
    if signed[4] != VERSION {
        return Err(Error::UnsupportedVersion(signed[4]));
    }
    let sig_len = u32::from_le_bytes([signed[5], signed[6], signed[7], signed[8]]) as usize;
    if sig_len > MAX_SIGNATURE {
        return Err(Error::BadSignature);
    }
    let end = 9usize.checked_add(sig_len).ok_or(Error::BadSignature)?;
    let signature = signed.get(9..end).ok_or(Error::BadSignature)?;
    let archive = &signed[end..];
    if key.is_valid_signature(signature, archive, SIGN_CONTEXT) {
        Ok(archive)
    } else {
        Err(Error::BadSignature)
    }
}

/// Writes the archive and seals it under `key`.
pub fn encode_sealed(entries: &[Entry], compression: Compression, key: &ChaChaPolyKey) -> Result<Vec<u8>, Error> {
    let bytes = encode(entries, compression)?;
    encrypted_archive::seal(key, &bytes).map_err(Error::Sealed)
}

/// Opens a sealed archive and reads it. A wrong key or a changed byte is `Sealed(WrongKeyOrCorrupt)`.
pub fn decode_sealed(data: &[u8], key: &ChaChaPolyKey, limits: &Limits) -> Result<Vec<Entry>, Error> {
    let bytes = encrypted_archive::open(key, data).map_err(Error::Sealed)?;
    decode(&bytes, limits)
}

/// Writes the archive, signs it, and seals the signed form under `key`.
pub fn encode_signed_sealed(entries: &[Entry], compression: Compression, signer: &PrivateKey, key: &ChaChaPolyKey) -> Result<Vec<u8>, Error> {
    let bytes = encode(entries, compression)?;
    let signed = sign(&bytes, signer)?;
    encrypted_archive::seal(key, &signed).map_err(Error::Sealed)
}

/// Opens a sealed archive, checks its signature against `verifier`, and reads it.
pub fn decode_signed_sealed(data: &[u8], key: &ChaChaPolyKey, verifier: &PublicKey, limits: &Limits) -> Result<Vec<Entry>, Error> {
    let signed = encrypted_archive::open(key, data).map_err(Error::Sealed)?;
    let archive = verify(&signed, verifier)?;
    decode(archive, limits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signature_pq::Variant;
    use std::io::{Seek, SeekFrom};

    fn sample() -> Vec<Entry> {
        let mut readme = Entry::file("docs/readme.txt", b"hello hello hello hello hello hello", 0o644);
        readme.header.uid = 1000;
        readme.header.gid = 100;
        readme.header.mtime_secs = 1_700_000_000;
        readme.header.mtime_nanos = 123_456_789;
        readme.header.xattrs = vec![("user.tag".to_string(), b"blue".to_vec())];
        vec![
            Entry::directory("docs", 0o755),
            readme,
            Entry::file("empty", b"", 0o600),
            Entry::symlink("docs/latest", "readme.txt"),
        ]
    }

    fn bytes_of(n: usize) -> Vec<u8> {
        let mut s = 9u32;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                (s >> 24) as u8
            })
            .collect()
    }

    #[test]
    fn every_compression_round_trips_headers_and_data() {
        for c in [Compression::None, Compression::Deflate, Compression::Lz4, Compression::Lzma] {
            let packed = encode(&sample(), c).unwrap();
            assert_eq!(&packed[..4], MAGIC);
            assert_eq!(peek(&packed).unwrap().0, 4);
            let back = decode(&packed, &Limits::default()).unwrap();
            assert_eq!(back, sample(), "{c:?}");
        }
    }

    #[test]
    fn an_empty_archive_round_trips() {
        let packed = encode(&[], Compression::Deflate).unwrap();
        assert_eq!(decode(&packed, &Limits::default()).unwrap(), Vec::<Entry>::new());
    }

    #[test]
    fn repetitive_data_compresses_and_the_header_reports_the_payload_length() {
        let big = vec![Entry::file("a.bin", &[7u8; 100_000], 0o644)];
        let plain = encode(&big, Compression::None).unwrap();
        let small = encode(&big, Compression::Deflate).unwrap();
        assert!(small.len() < plain.len() / 10);
        assert_eq!(peek(&small).unwrap().2, peek(&plain).unwrap().2);
        assert_eq!(peek(&small).unwrap().1, Compression::Deflate);
    }

    #[test]
    fn the_same_entries_give_the_same_bytes() {
        assert_eq!(encode(&sample(), Compression::Lzma).unwrap(), encode(&sample(), Compression::Lzma).unwrap());
    }

    #[test]
    fn unsafe_paths_are_refused_when_writing() {
        for bad in ["/etc/passwd", "../x", "a/../../x", "a//b", "./a", "a/.", "", "a\\b", "a\nb", "a\0b"] {
            let e = vec![Entry::file(bad, b"x", 0o644)];
            assert_eq!(encode(&e, Compression::None), Err(Error::UnsafePath(bad.to_string())), "{bad:?}");
        }
        let long = "a".repeat(MAX_NAME + 1);
        assert!(matches!(encode(&[Entry::file(&long, b"", 0o644)], Compression::None), Err(Error::UnsafePath(_))));
        let deep = vec!["d"; MAX_DEPTH + 1].join("/");
        assert!(matches!(encode(&[Entry::file(&deep, b"", 0o644)], Compression::None), Err(Error::UnsafePath(_))));
    }

    #[test]
    fn unsafe_links_are_refused() {
        for (path, target) in [("l", "/etc/passwd"), ("l", "../x"), ("a/l", "../../x"), ("l", ""), ("a/b/l", "../../../x")] {
            let e = vec![Entry::symlink(path, target)];
            assert!(matches!(encode(&e, Compression::None), Err(Error::UnsafeLink(_))), "{path} -> {target}");
        }
        // a link that stays inside the root is fine
        assert!(encode(&[Entry::symlink("a/b/l", "../../c"), Entry::symlink("m", "a/b")], Compression::None).is_ok());
    }

    #[test]
    fn duplicate_paths_and_a_folder_with_data_are_refused() {
        let dup = vec![Entry::file("a", b"1", 0o644), Entry::file("a", b"2", 0o644)];
        assert_eq!(encode(&dup, Compression::None), Err(Error::Duplicate("a".to_string())));
        let mut folder = Entry::directory("d", 0o755);
        folder.data = b"x".to_vec();
        assert!(matches!(encode(&[folder], Compression::None), Err(Error::Corrupt(_))));
    }

    /// Rewrites a hand-edited payload into a well-formed archive (valid checksum) so the entry rules are what refuses it.
    fn with_payload(payload: &[u8], count: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.push(0);
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        out.extend_from_slice(&digest::sha256(payload));
        out.extend_from_slice(payload);
        out
    }

    fn raw_entry(kind: u8, path: &str, data: &[u8]) -> Vec<u8> {
        let mut p = vec![kind];
        p.extend_from_slice(&(path.len() as u16).to_le_bytes());
        p.extend_from_slice(path.as_bytes());
        p.extend_from_slice(&0o644u32.to_le_bytes());
        p.extend_from_slice(&[0u8; 8]);
        p.extend_from_slice(&0i64.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
        p.extend_from_slice(&(data.len() as u64).to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes());
        p.extend_from_slice(data);
        p
    }

    #[test]
    fn a_decoder_refuses_unsafe_paths_in_a_well_formed_archive() {
        for path in ["../escape", "/abs", "a/../b"] {
            let archive = with_payload(&raw_entry(1, path, b"x"), 1);
            assert_eq!(decode(&archive, &Limits::default()), Err(Error::UnsafePath(path.to_string())));
        }
        let ok = with_payload(&raw_entry(1, "fine", b"x"), 1);
        assert_eq!(decode(&ok, &Limits::default()).unwrap()[0].data, b"x".to_vec());
        let link = with_payload(&raw_entry(3, "l", b"../../x"), 1);
        assert!(matches!(decode(&link, &Limits::default()), Err(Error::UnsafeLink(_))));
        let unknown = with_payload(&raw_entry(9, "k", b""), 1);
        assert!(matches!(decode(&unknown, &Limits::default()), Err(Error::Corrupt(_))));
        let mut twice = raw_entry(1, "a", b"1");
        twice.extend(raw_entry(1, "a", b"2"));
        assert_eq!(decode(&with_payload(&twice, 2), &Limits::default()), Err(Error::Duplicate("a".to_string())));
    }

    #[test]
    fn a_decoder_refuses_sizes_that_run_past_the_payload() {
        // the entry claims a 4 GiB data length but the payload ends right after the header
        let mut p = raw_entry(1, "big", b"");
        let at = p.len() - 2 - 8;
        p[at..at + 8].copy_from_slice(&(u32::MAX as u64).to_le_bytes());
        assert_eq!(decode(&with_payload(&p, 1), &Limits::default()), Err(Error::Truncated));
        // and a length that does not fit in a usize on any target
        p[at..at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert_eq!(decode(&with_payload(&p, 1), &Limits::default()), Err(Error::Truncated));
    }

    #[test]
    fn a_decoder_refuses_counts_and_lengths_past_the_limits_before_allocating() {
        let packed = encode(&sample(), Compression::None).unwrap();
        let few = Limits { max_entries: 3, ..Limits::default() };
        assert_eq!(decode(&packed, &few), Err(Error::TooManyEntries));
        let small = Limits { max_payload_bytes: 10, ..Limits::default() };
        assert_eq!(decode(&packed, &small), Err(Error::TooBig));
        // a header that claims four billion entries in a tiny payload
        let mut lie = with_payload(&raw_entry(1, "a", b""), 1);
        lie[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode(&lie, &Limits { max_entries: usize::MAX, ..Limits::default() }), Err(Error::Corrupt("more entries than the payload can hold")));
        // a header that claims a payload of 2^63 bytes
        let mut huge = packed.clone();
        huge[12..20].copy_from_slice(&(1u64 << 63).to_le_bytes());
        assert_eq!(decode(&huge, &Limits::default()), Err(Error::TooBig));
    }

    #[test]
    fn a_compression_bomb_is_stopped_at_the_declared_length() {
        let bomb_entries = vec![Entry::file("zeros", &vec![0u8; 5_000_000], 0o644)];
        let packed = encode(&bomb_entries, Compression::Deflate).unwrap();
        assert!(packed.len() < 100_000, "five megabytes of zeros pack small");
        // lower the declared length: the decoder must refuse to inflate past it
        let mut lying = packed.clone();
        lying[12..20].copy_from_slice(&1000u64.to_le_bytes());
        assert!(matches!(decode(&lying, &Limits::default()), Err(Error::Corrupt(_))));
        // and the caller's limit refuses it before any inflate
        let limits = Limits { max_payload_bytes: 1_000_000, ..Limits::default() };
        assert_eq!(decode(&packed, &limits), Err(Error::TooBig));
    }

    #[test]
    fn damage_to_any_byte_is_caught() {
        let packed = encode(&sample(), Compression::None).unwrap();
        for i in 0..packed.len() {
            let mut bad = packed.clone();
            bad[i] ^= 0x55;
            assert!(decode(&bad, &Limits::default()).is_err(), "byte {i}");
        }
    }

    #[test]
    fn truncation_at_every_length_is_an_error_not_a_panic() {
        for c in [Compression::None, Compression::Deflate, Compression::Lz4, Compression::Lzma] {
            let packed = encode(&sample(), c).unwrap();
            for n in 0..packed.len() {
                assert!(decode(&packed[..n], &Limits::default()).is_err(), "{c:?} cut at {n}");
            }
        }
    }

    #[test]
    fn header_errors_are_named() {
        let packed = encode(&sample(), Compression::None).unwrap();
        assert_eq!(decode(b"BAR", &Limits::default()), Err(Error::NotArchive));
        assert_eq!(decode(&[0u8; 80], &Limits::default()), Err(Error::NotArchive));
        let mut v = packed.clone();
        v[4] = 2;
        assert_eq!(decode(&v, &Limits::default()), Err(Error::UnsupportedVersion(2)));
        let mut c = packed.clone();
        c[5] = 9;
        assert_eq!(decode(&c, &Limits::default()), Err(Error::UnsupportedCompression(9)));
        let mut r = packed.clone();
        r[6] = 1;
        assert!(matches!(decode(&r, &Limits::default()), Err(Error::Corrupt(_))));
        let mut extra = packed;
        extra.push(0);
        assert!(decode(&extra, &Limits::default()).is_err(), "a byte after the payload changes the payload length");
    }

    #[test]
    fn the_byte_stream_form_writes_at_the_cursor_and_reads_to_the_end() {
        let mut s = ByteStream::new();
        let n = encode_to(&mut s, &sample(), Compression::Lz4).unwrap();
        assert_eq!(s.len(), n);
        s.seek(SeekFrom::Start(0)).unwrap();
        assert_eq!(decode_from(&mut s, &Limits::default()).unwrap(), sample());
    }

    #[test]
    fn a_folder_packs_and_extracts_with_modes_links_and_times() {
        let src = tempfile::tempdir().unwrap();
        let root = src.path().join("tree");
        fs::create_dir_all(root.join("sub/deeper")).unwrap();
        fs::write(root.join("a.txt"), b"alpha").unwrap();
        fs::write(root.join("sub/b.txt"), bytes_of(5000)).unwrap();
        fs::write(root.join("sub/deeper/c.bin"), vec![1u8; 70_000]).unwrap();
        fs::set_permissions(root.join("a.txt"), fs::Permissions::from_mode(0o640)).unwrap();
        symlink("a.txt", root.join("link")).unwrap();
        symlink("../../outside", root.join("sub/escape")).unwrap();
        let packed = pack_directory(&root, &Limits::default()).unwrap();
        let paths: Vec<&str> = packed.entries.iter().map(|e| e.header.path.as_str()).collect();
        assert_eq!(paths, ["a.txt", "link", "sub", "sub/b.txt", "sub/deeper", "sub/deeper/c.bin"]);
        assert_eq!(packed.skipped, ["sub/escape"], "a link that climbs out of the folder is left out");
        let a = &packed.entries[0];
        assert_eq!((a.header.mode, a.header.size), (0o640, 5));
        assert_eq!(packed.entries[1].header.kind, Kind::Symlink);

        let file = src.path().join("tree.bar");
        let skipped = archive_directory(&root, &file, Compression::Lzma, &Limits::default()).unwrap();
        assert_eq!(skipped, ["sub/escape"]);
        assert!(archive_directory(&root, &file, Compression::Lzma, &Limits::default()).is_err(), "an existing file is not replaced");

        let dest = src.path().join("out");
        let done = extract_archive(&file, &dest, &Limits::default(), &ExtractOptions::default()).unwrap();
        assert_eq!((done.files, done.directories, done.symlinks, done.bytes), (3, 2, 1, 5 + 5000 + 70_000));
        assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"alpha");
        assert_eq!(fs::metadata(dest.join("a.txt")).unwrap().permissions().mode() & 0o777, 0o640);
        assert_eq!(fs::read(dest.join("sub/deeper/c.bin")).unwrap().len(), 70_000);
        assert_eq!(fs::read_link(dest.join("link")).unwrap(), PathBuf::from("a.txt"));
        let was = fs::metadata(root.join("a.txt")).unwrap().mtime();
        assert_eq!(fs::metadata(dest.join("a.txt")).unwrap().mtime(), was);
        // the extracted tree packs to the same entries again
        assert_eq!(pack_directory(&dest, &Limits::default()).unwrap().entries.len(), 6);
    }

    #[test]
    fn extraction_never_writes_through_a_link_or_over_a_file() {
        let base = tempfile::tempdir().unwrap();
        let outside = base.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let dest = base.path().join("dest");
        fs::create_dir(&dest).unwrap();
        // a link that points out of the destination exists already (made by something else)
        symlink(&outside, dest.join("trap")).unwrap();
        let err = extract(&[Entry::file("trap/pwned.txt", b"x", 0o644)], &dest, &ExtractOptions::default()).unwrap_err();
        assert!(matches!(err, Error::UnsafePath(_)));
        assert!(!outside.join("pwned.txt").exists());
        // an archive cannot plant its own link and then write through it
        let two = vec![Entry::symlink("l", "sub"), Entry::file("l/x.txt", b"x", 0o644)];
        let fresh = base.path().join("fresh");
        assert!(matches!(extract(&two, &fresh, &ExtractOptions::default()), Err(Error::UnsafePath(_))));
        // an existing file is not replaced
        fs::write(dest.join("keep.txt"), b"mine").unwrap();
        let err = extract(&[Entry::file("keep.txt", b"theirs", 0o644)], &dest, &ExtractOptions::default()).unwrap_err();
        assert!(matches!(err, Error::Io(io::ErrorKind::AlreadyExists, _)));
        assert_eq!(fs::read(dest.join("keep.txt")).unwrap(), b"mine");
        // nothing is written when a later entry is unsafe
        let mixed = vec![Entry::file("early.txt", b"x", 0o644), Entry::file("../late.txt", b"x", 0o644)];
        let clean = base.path().join("clean");
        assert!(extract(&mixed, &clean, &ExtractOptions::default()).is_err());
        assert!(!clean.join("early.txt").exists());
    }

    #[test]
    fn extraction_drops_the_special_mode_bits() {
        let base = tempfile::tempdir().unwrap();
        extract(&[Entry::file("tool", b"#!/bin/sh\n", 0o4755)], base.path(), &ExtractOptions::default()).unwrap();
        assert_eq!(fs::metadata(base.path().join("tool")).unwrap().permissions().mode() & 0o7777, 0o755);
    }

    #[test]
    fn a_read_only_folder_still_receives_its_files() {
        let base = tempfile::tempdir().unwrap();
        let entries = vec![Entry::directory("ro", 0o555), Entry::file("ro/inside.txt", b"in", 0o444)];
        extract(&entries, base.path(), &ExtractOptions::default()).unwrap();
        assert_eq!(fs::read(base.path().join("ro/inside.txt")).unwrap(), b"in");
        assert_eq!(fs::metadata(base.path().join("ro")).unwrap().permissions().mode() & 0o777, 0o555);
        fs::set_permissions(base.path().join("ro"), fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn a_folder_larger_than_the_limit_is_refused_while_packing() {
        let src = tempfile::tempdir().unwrap();
        fs::write(src.path().join("big"), vec![0u8; 5000]).unwrap();
        let small = Limits { max_payload_bytes: 1000, ..Limits::default() };
        assert_eq!(pack_directory(src.path(), &small), Err(Error::TooBig));
        let few = Limits { max_entries: 0, ..Limits::default() };
        assert_eq!(pack_directory(src.path(), &few), Err(Error::TooManyEntries));
        assert!(pack_directory(&src.path().join("big"), &Limits::default()).is_err(), "a file is not a folder");
    }

    #[test]
    fn a_signed_archive_verifies_and_any_change_does_not() {
        let key = PrivateKey::from_seed(Variant::MlDsa65, &[3u8; 32]);
        let other = PrivateKey::from_seed(Variant::MlDsa65, &[4u8; 32]);
        let bytes = encode(&sample(), Compression::Deflate).unwrap();
        let signed = sign(&bytes, &key).unwrap();
        assert_eq!(&signed[..4], SIGNED_MAGIC);
        assert_eq!(verify(&signed, &key.public_key()).unwrap(), &bytes[..]);
        assert_eq!(verify(&signed, &other.public_key()), Err(Error::BadSignature));
        let mut tampered = signed.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert_eq!(verify(&tampered, &key.public_key()), Err(Error::BadSignature));
        let mut sig_byte = signed.clone();
        sig_byte[20] ^= 1;
        assert_eq!(verify(&sig_byte, &key.public_key()), Err(Error::BadSignature));
        assert_eq!(verify(&signed[..12], &key.public_key()), Err(Error::BadSignature));
        assert_eq!(verify(b"BARS", &key.public_key()), Err(Error::BadSignature));
        let mut huge = signed;
        huge[5..9].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(verify(&huge, &key.public_key()), Err(Error::BadSignature));
    }

    #[test]
    fn a_sealed_archive_opens_only_with_its_key() {
        let key = ChaChaPolyKey::from_bytes(&[7u8; 32]).unwrap();
        let wrong = ChaChaPolyKey::from_bytes(&[8u8; 32]).unwrap();
        let sealed = encode_sealed(&sample(), Compression::Lz4, &key).unwrap();
        assert_eq!(decode_sealed(&sealed, &key, &Limits::default()).unwrap(), sample());
        assert_eq!(decode_sealed(&sealed, &wrong, &Limits::default()), Err(Error::Sealed(encrypted_archive::Error::WrongKeyOrCorrupt)));
        let mut changed = sealed.clone();
        let mid = changed.len() / 2;
        changed[mid] ^= 1;
        assert_eq!(decode_sealed(&changed, &key, &Limits::default()), Err(Error::Sealed(encrypted_archive::Error::WrongKeyOrCorrupt)));
        assert!(!sealed.windows(5).any(|w| w == b"hello"), "the contents are not visible in the sealed form");
    }

    #[test]
    fn signed_then_sealed_needs_both_keys() {
        let signer = PrivateKey::from_seed(Variant::MlDsa65, &[5u8; 32]);
        let stranger = PrivateKey::from_seed(Variant::MlDsa65, &[6u8; 32]);
        let key = ChaChaPolyKey::from_bytes(&[9u8; 32]).unwrap();
        let sealed = encode_signed_sealed(&sample(), Compression::Deflate, &signer, &key).unwrap();
        assert_eq!(decode_signed_sealed(&sealed, &key, &signer.public_key(), &Limits::default()).unwrap(), sample());
        assert_eq!(decode_signed_sealed(&sealed, &key, &stranger.public_key(), &Limits::default()), Err(Error::BadSignature));
        // a sealed archive that was never signed is refused by the signed reader
        let unsigned = encode_sealed(&sample(), Compression::Deflate, &key).unwrap();
        assert_eq!(decode_signed_sealed(&unsigned, &key, &signer.public_key(), &Limits::default()), Err(Error::BadSignature));
    }

    #[test]
    fn extended_attributes_survive_encode_and_decode_and_bad_ones_are_refused() {
        let back = decode(&encode(&sample(), Compression::None).unwrap(), &Limits::default()).unwrap();
        assert_eq!(back[1].header.xattrs, vec![("user.tag".to_string(), b"blue".to_vec())]);
        let mut e = Entry::file("f", b"", 0o644);
        e.header.xattrs = vec![(String::new(), b"v".to_vec())];
        assert!(matches!(encode(&[e], Compression::None), Err(Error::Corrupt(_))));
        let mut big = Entry::file("f", b"", 0o644);
        big.header.xattrs = vec![("user.big".to_string(), vec![0u8; MAX_XATTR_VALUE + 1])];
        assert!(matches!(encode(&[big], Compression::None), Err(Error::Corrupt(_))));
    }

    #[test]
    fn times_before_1970_and_the_nanosecond_range_are_handled() {
        let mut old = Entry::file("old", b"", 0o644);
        old.header.mtime_secs = -86_400;
        old.header.mtime_nanos = 5;
        assert!(header_time(&old.header).is_some());
        let back = decode(&encode(&[old.clone()], Compression::None).unwrap(), &Limits::default()).unwrap();
        assert_eq!(back[0].header.mtime_secs, -86_400);
        old.header.mtime_nanos = 1_000_000_000;
        assert!(matches!(encode(&[old], Compression::None), Err(Error::Corrupt(_))));
    }
}
