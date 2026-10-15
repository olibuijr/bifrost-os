//! App-facing reader for uncompressed POSIX (ustar) tar packages, the format of install packages.
//!
//! `read` loads every member and its contents. `index` reads only the headers, seeking past the contents, so a large
//! package is checked without being loaded. `extract_from` writes members below a folder, copying each one's contents
//! from the archive it was indexed in.
//!
//! The rules are the shell's install rules: regular files and folders only (no links, devices or pax headers), no
//! absolute paths or `..`, no duplicate paths, valid header checksums, and a package that ends with an end marker.
//! A package the rules refuse, and a damaged header, are `InvalidData`. A failed read or write of the file system is
//! the underlying `io::Error`, or `Other` when the shell reports it as text.
//!
//! This reader is for tar only. ZIP archives are in [`crate::archive`].
//!
//! ```ignore
//! let entries = tar_package::read(&bytes)?;
//! let mut file = std::io::Cursor::new(bytes);
//! let index = tar_package::index(&mut file)?;
//! tar_package::extract_from(&mut file, &index, &folder)?;
//! ```

use springboard::shell::store::install::{self, TarEntry};
use std::io::{self, Read, Seek};
use std::path::{Path, PathBuf};

/// One member of a tar package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The member path, relative to the package root, with no `..` and no absolute start.
    pub path: PathBuf,
    /// True for a folder member.
    pub is_dir: bool,
    /// The permission bits recorded in the header.
    pub mode: u32,
    /// The length of the contents in bytes; zero for a folder.
    pub size: u64,
    /// Where the contents start in the package, in bytes from its beginning.
    pub offset: u64,
    /// The contents. Empty for a folder and for entries from `index`.
    pub contents: Vec<u8>,
}

/// Reads every member of an in-memory tar package, with its contents.
pub fn read(data: &[u8]) -> io::Result<Vec<Entry>> {
    install::read_tar(data).map(from_tar).map_err(refused)
}

/// Reads the headers of a tar package from `r`, which must be positioned at its start. The contents are not read.
pub fn index<R: Read + Seek>(r: &mut R) -> io::Result<Vec<Entry>> {
    install::index_tar(r).map(from_tar).map_err(refused)
}

/// Writes `entries` below `dir`, copying each file's contents from `r`, the package they were indexed in.
///
/// Every path is checked again before anything is written, so entries built by hand cannot leave `dir`.
pub fn extract_from<R: Read + Seek>(r: &mut R, entries: &[Entry], dir: &Path) -> io::Result<()> {
    let mut checked = Vec::with_capacity(entries.len());
    for e in entries {
        let text = e.path.to_str().ok_or_else(|| refused(format!("unsafe path {:?}", e.path)))?;
        let path = install::safe_path(text).map_err(refused)?;
        checked.push(TarEntry { path, data: Vec::new(), mode: e.mode, is_dir: e.is_dir, offset: e.offset, size: e.size });
    }
    install::extract_from(r, &checked, dir).map_err(io::Error::other)
}

fn from_tar(files: Vec<TarEntry>) -> Vec<Entry> {
    files
        .into_iter()
        .map(|f| Entry { path: f.path, is_dir: f.is_dir, mode: f.mode, size: f.size, offset: f.offset, contents: f.data })
        .collect()
}

fn refused(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}
