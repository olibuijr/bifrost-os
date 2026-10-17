//! The asset pack file: the on-disk format of one downloadable pack, and how an installed pack lands in an app's
//! container.
//!
//! Layout (all integers big endian): `BFPK1\n`, pack id (u16 length, bytes), pack version (u32), file count (u32), then
//! per file the relative name (u16 length, bytes) and the data (u64 length, bytes); the file ends with SHA-256 over every
//! byte before it. A pack is read only when the digest matches, the id is a plain identifier, and every file name is a
//! safe relative path with no duplicates, so a damaged or hostile pack changes nothing.
//!
//! An installed pack lives under `Library/Caches/AssetPacks/<id>/` in the app's container (`container_layout`): packs can
//! always be fetched again, so the system may purge them to free space. Installing writes every file through a
//! temporary name and a rename, and replaces an older version of the same pack as a whole.

use crate::container_layout::{create_parent_dirs, ContainerLayout};
use crate::sha256;
use std::fs;
use std::io;
use std::path::Path;

/// First bytes of a pack file.
pub const MAGIC: &[u8] = b"BFPK1\n";
/// Where a pack's files go, relative to the container root; `<id>` follows.
pub const PACKS_FOLDER: &str = "Library/Caches/AssetPacks";
/// Most files one pack may hold.
pub const MAX_FILES: usize = 100_000;

/// One pack: its id, version and files (relative name, bytes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackFile {
    pub id: String,
    pub version: u32,
    pub files: Vec<(String, Vec<u8>)>,
}

/// Why a pack file was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackFileError {
    /// Not a pack file (wrong first bytes) or cut short.
    Truncated,
    /// The trailing digest does not match the bytes.
    BadDigest,
    /// The pack id is empty, too long or has characters other than letters, digits, `.`, `_` and `-`.
    BadId,
    /// A file name is empty, absolute, or has a `.`, `..` or empty component, or is not valid UTF-8.
    BadName(String),
    /// Two files share one name.
    Duplicate(String),
    /// More than [`MAX_FILES`] files.
    TooMany,
}

impl std::fmt::Display for PackFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackFileError::Truncated => write!(f, "asset pack: not a pack file or cut short"),
            PackFileError::BadDigest => write!(f, "asset pack: checksum mismatch"),
            PackFileError::BadId => write!(f, "asset pack: invalid pack id"),
            PackFileError::BadName(n) => write!(f, "asset pack: unsafe file name {n:?}"),
            PackFileError::Duplicate(n) => write!(f, "asset pack: duplicate file {n:?}"),
            PackFileError::TooMany => write!(f, "asset pack: too many files"),
        }
    }
}

impl std::error::Error for PackFileError {}

/// True for a pack or app id that is safe as one path component.
pub fn id_ok(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id != "."
        && id != ".."
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 1024
        && !name.contains('\0')
        && !name.contains('\\')
        && !name.starts_with('/')
        && name.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
}

/// Checks the id, file names and file count of a pack.
pub fn validate(pack: &PackFile) -> Result<(), PackFileError> {
    if !id_ok(&pack.id) {
        return Err(PackFileError::BadId);
    }
    if pack.files.len() > MAX_FILES {
        return Err(PackFileError::TooMany);
    }
    let mut seen = std::collections::BTreeSet::new();
    for (name, _) in &pack.files {
        if !name_ok(name) {
            return Err(PackFileError::BadName(name.clone()));
        }
        if !seen.insert(name.as_str()) {
            return Err(PackFileError::Duplicate(name.clone()));
        }
    }
    Ok(())
}

/// Writes a pack file. The pack must pass [`validate`].
pub fn encode(pack: &PackFile) -> Result<Vec<u8>, PackFileError> {
    validate(pack)?;
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(pack.id.len() as u16).to_be_bytes());
    out.extend_from_slice(pack.id.as_bytes());
    out.extend_from_slice(&pack.version.to_be_bytes());
    out.extend_from_slice(&(pack.files.len() as u32).to_be_bytes());
    for (name, data) in &pack.files {
        out.extend_from_slice(&(name.len() as u16).to_be_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&(data.len() as u64).to_be_bytes());
        out.extend_from_slice(data);
    }
    let d = sha256::digest(&out);
    out.extend_from_slice(&d);
    Ok(out)
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], PackFileError> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.b.len()).ok_or(PackFileError::Truncated)?;
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u16(&mut self) -> Result<usize, PackFileError> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()) as usize)
    }
    fn u32(&mut self) -> Result<u32, PackFileError> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, PackFileError> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
}

/// Reads a pack file, checking the digest first and then the id and names.
pub fn decode(bytes: &[u8]) -> Result<PackFile, PackFileError> {
    if bytes.len() < MAGIC.len() + 32 || &bytes[..MAGIC.len()] != MAGIC {
        return Err(PackFileError::Truncated);
    }
    let (body, tail) = bytes.split_at(bytes.len() - 32);
    if sha256::digest(body)[..] != *tail {
        return Err(PackFileError::BadDigest);
    }
    let mut r = Reader { b: body, at: MAGIC.len() };
    let id_len = r.u16()?;
    let id = String::from_utf8(r.take(id_len)?.to_vec()).map_err(|_| PackFileError::BadId)?;
    let version = r.u32()?;
    let count = r.u32()? as usize;
    if count > MAX_FILES {
        return Err(PackFileError::TooMany);
    }
    let mut files = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        let n = r.u16()?;
        let name = String::from_utf8(r.take(n)?.to_vec()).map_err(|_| PackFileError::BadName(String::new()))?;
        let len = usize::try_from(r.u64()?).map_err(|_| PackFileError::Truncated)?;
        files.push((name, r.take(len)?.to_vec()));
    }
    if r.at != body.len() {
        return Err(PackFileError::Truncated);
    }
    let pack = PackFile { id, version, files };
    validate(&pack)?;
    Ok(pack)
}

/// The container-relative folder an installed pack lives in.
pub fn pack_folder(id: &str) -> String {
    format!("{PACKS_FOLDER}/{id}")
}

/// Installs `pack` into the container: replaces any older copy of the pack, writing each file through a temporary name
/// and a rename. A symbolic link on the way stops the install with an error. Returns the number of files written.
pub fn install(layout: &ContainerLayout, pack: &PackFile) -> io::Result<usize> {
    validate(pack).map_err(io::Error::other)?;
    layout.ensure()?;
    let folder = layout.resolve(&pack_folder(&pack.id)).map_err(|e| io::Error::other(e.to_string()))?;
    // build next to the final place, then swap, so a failed install leaves the older version in use
    let staging = folder.with_file_name(format!(".{}.installing", pack.id));
    let _ = fs::remove_dir_all(&staging);
    let staged_rel = format!("{PACKS_FOLDER}/.{}.installing", pack.id);
    let result = (|| {
        for (name, data) in &pack.files {
            let target = layout.resolve(&format!("{staged_rel}/{name}")).map_err(|e| io::Error::other(e.to_string()))?;
            create_parent_dirs(layout.root(), &target)?;
            fs::write(&target, data)?;
        }
        if pack.files.is_empty() {
            create_parent_dirs(layout.root(), &staging.join("x"))?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }
    let _ = fs::remove_dir_all(&folder);
    fs::rename(&staging, &folder)?;
    Ok(pack.files.len())
}

/// Removes an installed pack; returns whether it was there.
pub fn remove(layout: &ContainerLayout, id: &str) -> io::Result<bool> {
    if !id_ok(id) {
        return Err(io::Error::other(PackFileError::BadId));
    }
    let folder = layout.resolve(&pack_folder(id)).map_err(|e| io::Error::other(e.to_string()))?;
    match fs::remove_dir_all(&folder) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// The version of an installed pack, kept in its `.version` marker; `None` when it is not installed.
pub fn installed_version(layout: &ContainerLayout, id: &str) -> Option<u32> {
    let p = layout.resolve(&format!("{}/{VERSION_FILE}", pack_folder(id))).ok()?;
    fs::read_to_string(p).ok()?.trim().parse().ok()
}

/// Marker file with the installed version; written by [`install_versioned`].
pub const VERSION_FILE: &str = ".version";

/// Installs `pack` with its version marker, so a later pass can tell it is current.
pub fn install_versioned(layout: &ContainerLayout, pack: &PackFile) -> io::Result<usize> {
    let mut with_marker = pack.clone();
    with_marker.files.retain(|(n, _)| n != VERSION_FILE);
    with_marker.files.push((VERSION_FILE.to_string(), pack.version.to_string().into_bytes()));
    install(layout, &with_marker).map(|n| n - 1)
}

/// True when `path` is a pack file name (`<id>.bfpk`) and returns its id.
pub fn pack_id_of_file(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let id = name.strip_suffix(".bfpk")?;
    id_ok(id).then(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PackFile {
        PackFile {
            id: "maps-eu".into(),
            version: 3,
            files: vec![("tiles/0.bin".into(), vec![1, 2, 3]), ("index.txt".into(), b"hello".to_vec())],
        }
    }

    fn layout() -> (ContainerLayout, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (ContainerLayout::new(dir.path().join("app")), dir)
    }

    #[test]
    fn a_pack_round_trips() {
        let bytes = encode(&sample()).unwrap();
        assert_eq!(decode(&bytes).unwrap(), sample());
    }

    #[test]
    fn a_changed_byte_fails_the_digest() {
        let mut bytes = encode(&sample()).unwrap();
        let i = bytes.len() / 2;
        bytes[i] ^= 1;
        assert_eq!(decode(&bytes), Err(PackFileError::BadDigest));
    }

    #[test]
    fn a_short_or_foreign_file_is_refused() {
        assert_eq!(decode(b"nope"), Err(PackFileError::Truncated));
        let mut bytes = encode(&sample()).unwrap();
        bytes[0] = b'X';
        assert_eq!(decode(&bytes), Err(PackFileError::Truncated));
    }

    #[test]
    fn unsafe_ids_and_names_are_refused() {
        let mut p = sample();
        p.id = "../x".into();
        assert_eq!(encode(&p), Err(PackFileError::BadId));
        let mut p = sample();
        p.files.push(("../escape".into(), vec![]));
        assert!(matches!(encode(&p), Err(PackFileError::BadName(_))));
        let mut p = sample();
        p.files.push(("/abs".into(), vec![]));
        assert!(matches!(encode(&p), Err(PackFileError::BadName(_))));
        let mut p = sample();
        p.files.push(("index.txt".into(), vec![]));
        assert!(matches!(encode(&p), Err(PackFileError::Duplicate(_))));
    }

    #[test]
    fn a_hostile_pack_with_a_valid_digest_is_still_refused() {
        // build the bytes by hand so the digest is right but the name escapes
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(b"a");
        out.extend_from_slice(&1u32.to_be_bytes());
        out.extend_from_slice(&1u32.to_be_bytes());
        out.extend_from_slice(&6u16.to_be_bytes());
        out.extend_from_slice(b"../etc");
        out.extend_from_slice(&0u64.to_be_bytes());
        let d = sha256::digest(&out);
        out.extend_from_slice(&d);
        assert!(matches!(decode(&out), Err(PackFileError::BadName(_))));
    }

    #[test]
    fn install_writes_files_under_the_purgeable_pack_folder() {
        let (l, _g) = layout();
        assert_eq!(install_versioned(&l, &sample()).unwrap(), 2);
        let root = l.resolve("Library/Caches/AssetPacks/maps-eu").unwrap();
        assert_eq!(fs::read(root.join("tiles/0.bin")).unwrap(), vec![1, 2, 3]);
        assert_eq!(fs::read(root.join("index.txt")).unwrap(), b"hello");
        assert_eq!(installed_version(&l, "maps-eu"), Some(3));
        assert_eq!(installed_version(&l, "other"), None);
    }

    #[test]
    fn a_new_version_replaces_the_old_files() {
        let (l, _g) = layout();
        install_versioned(&l, &sample()).unwrap();
        let mut next = sample();
        next.version = 4;
        next.files = vec![("new.txt".into(), b"n".to_vec())];
        install_versioned(&l, &next).unwrap();
        let root = l.resolve("Library/Caches/AssetPacks/maps-eu").unwrap();
        assert!(!root.join("index.txt").exists());
        assert!(root.join("new.txt").exists());
        assert_eq!(installed_version(&l, "maps-eu"), Some(4));
    }

    #[test]
    fn remove_deletes_an_installed_pack() {
        let (l, _g) = layout();
        install(&l, &sample()).unwrap();
        assert!(remove(&l, "maps-eu").unwrap());
        assert!(!remove(&l, "maps-eu").unwrap());
        assert!(remove(&l, "../x").is_err());
    }

    #[test]
    fn pack_file_names_carry_a_safe_id() {
        assert_eq!(pack_id_of_file(Path::new("/s/app/maps-eu.bfpk")), Some("maps-eu".into()));
        assert_eq!(pack_id_of_file(Path::new("/s/app/maps-eu.zip")), None);
        assert_eq!(pack_id_of_file(Path::new("/s/app/.bfpk")), None);
    }
}
