//! Download integrity for updates: a file is checked against the size and SHA-256 digest its manifest announces
//! before it is used. The shell's update client does the same check in `springboard::shell::update`; this module gives
//! apps the check for their own downloads.
//!
//! The manifest is not signed. A matching digest proves the bytes are the ones the manifest names, and the manifest is
//! trusted only as far as its HTTPS source is.

use bifrost_core::sha256;

/// Why a download was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IntegrityError {
    /// The announced digest is not 64 hexadecimal digits.
    MalformedDigest,
    /// The download is not the announced size.
    SizeMismatch { expected: u64, actual: u64 },
    /// The digest of the download differs from the announced one.
    DigestMismatch,
}

/// The identity a download must have: its size in bytes and its SHA-256 digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expected {
    pub size: u64,
    /// Lowercase hexadecimal, 64 digits.
    pub sha256: String,
}

impl Expected {
    /// Builds the expectation. The digest may use either letter case; it is stored in lowercase.
    pub fn new(size: u64, sha256: &str) -> Result<Expected, IntegrityError> {
        if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(IntegrityError::MalformedDigest);
        }
        Ok(Expected { size, sha256: sha256.to_ascii_lowercase() })
    }
}

/// Checks `data` against `expected`: the size first, then the digest. Returns the digest of `data` when both match.
pub fn verify(data: &[u8], expected: &Expected) -> Result<String, IntegrityError> {
    let actual = data.len() as u64;
    if actual != expected.size {
        return Err(IntegrityError::SizeMismatch { expected: expected.size, actual });
    }
    let digest = sha256::hex(&sha256::digest(data));
    if digest != expected.sha256 {
        return Err(IntegrityError::DigestMismatch);
    }
    Ok(digest)
}
