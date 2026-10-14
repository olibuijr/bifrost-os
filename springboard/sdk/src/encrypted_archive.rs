//! Encrypted archives for apps: the bytes of an archive sealed under a ChaCha20-Poly1305 key into one self-describing box.
//!
//! Layout: the magic `BFEA`, one version byte, then a sealed box from [`crate::chacha_poly`] (a 12-byte nonce, the
//! ciphertext and a 16-byte tag). The header is authenticated as additional data, so a changed header byte fails the
//! tag check. The archive bytes are the caller's (for example the output of `archive::compress`): this module seals and
//! opens them and does not compress them. The reference system's encrypted archive has more profiles and header
//! fields; only this profile, version 1, is implemented.
//!
//! ```ignore
//! let key = ChaChaPolyKey::generate()?;
//! let sealed = encrypted_archive::seal(&key, &packed)?;
//! let packed = encrypted_archive::open(&key, &sealed)?;
//! ```

use crate::chacha_poly::{self, ChaChaPolyKey};

/// The first four bytes of every encrypted archive.
pub const MAGIC: &[u8; 4] = b"BFEA";
/// The format version this module writes and reads.
pub const VERSION: u8 = 1;
/// Header length in bytes: the magic and the version byte.
pub const HEADER_BYTES: usize = MAGIC.len() + 1;

/// Why an encrypted archive could not be sealed or opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The data does not start with the magic, or is too short to hold a header, a nonce and a tag.
    NotEncryptedArchive,
    /// The version byte names a format this module does not read.
    UnsupportedVersion(u8),
    /// The tag does not match: the key is wrong, or the archive was changed.
    WrongKeyOrCorrupt,
    /// The random pool could not supply a nonce.
    Unavailable,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotEncryptedArchive => f.write_str("data is not an encrypted archive"),
            Error::UnsupportedVersion(version) => write!(f, "encrypted archive version {version} is not supported"),
            Error::WrongKeyOrCorrupt => f.write_str("the key is wrong or the archive was changed"),
            Error::Unavailable => f.write_str("the random pool could not supply bytes"),
        }
    }
}

/// Seals `archive` under `key` and returns the encrypted archive. Two seals of the same bytes differ, because each
/// seal draws a fresh nonce.
pub fn seal(key: &ChaChaPolyKey, archive: &[u8]) -> Result<Vec<u8>, Error> {
    let header = header();
    let sealed = key.seal(archive, &header).map_err(|_| Error::Unavailable)?;
    let mut out = Vec::with_capacity(HEADER_BYTES + sealed.len());
    out.extend_from_slice(&header);
    out.extend_from_slice(&sealed);
    Ok(out)
}

/// Opens an encrypted archive made by [`seal`] and returns the archive bytes. A wrong key or a changed byte fails with
/// [`Error::WrongKeyOrCorrupt`] and yields no bytes.
pub fn open(key: &ChaChaPolyKey, data: &[u8]) -> Result<Vec<u8>, Error> {
    let minimum = HEADER_BYTES + chacha_poly::NONCE_BYTES + chacha_poly::TAG_BYTES;
    if data.len() < minimum || !data.starts_with(MAGIC) {
        return Err(Error::NotEncryptedArchive);
    }
    let version = data[MAGIC.len()];
    if version != VERSION {
        return Err(Error::UnsupportedVersion(version));
    }
    let (header, sealed) = data.split_at(HEADER_BYTES);
    key.open(sealed, header).map_err(|error| match error {
        chacha_poly::Error::Authentication => Error::WrongKeyOrCorrupt,
        _ => Error::NotEncryptedArchive,
    })
}

fn header() -> [u8; HEADER_BYTES] {
    let mut header = [0u8; HEADER_BYTES];
    header[..MAGIC.len()].copy_from_slice(MAGIC);
    header[MAGIC.len()] = VERSION;
    header
}
