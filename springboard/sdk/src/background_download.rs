//! Background download staging: a payload is checked against an expected size and SHA-256 digest, written to a
//! `.part` file beside its final name, and renamed into place. A payload that fails the check never reaches its final
//! name, and a failed write removes the partial file.
//!
//! This is the pure model. The shell's system update follows the same sequence (`springboard/src/shell/update.rs`
//! stages `springboard.part`, then renames it to `springboard.new`). Apps get the model here for their own downloads.
//! The model performs no transfer: the caller supplies the bytes, so it needs no network.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::digest::sha256;

/// What a finished download must look like: its file name, its exact size and its SHA-256 digest in hexadecimal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadSpec {
    name: String,
    expected_size: u64,
    sha256: String,
}

/// Why a download was refused or could not be stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadError {
    /// The file name is empty, is `.` or `..`, or contains a path separator or a NUL character.
    InvalidName(String),
    /// The digest is not 64 hexadecimal digits.
    InvalidDigest,
    /// The payload has a different number of bytes than the spec.
    SizeMismatch { expected: u64, actual: u64 },
    /// The payload has a different SHA-256 digest than the spec. Both values are lower-case hexadecimal.
    DigestMismatch { expected: String, actual: String },
    /// The file system refused the write or the rename.
    Io(String),
}

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DownloadError::InvalidName(name) => write!(f, "invalid download name {name:?}"),
            DownloadError::InvalidDigest => f.write_str("the digest is not 64 hexadecimal digits"),
            DownloadError::SizeMismatch { expected, actual } => write!(f, "expected {expected} bytes, got {actual}"),
            DownloadError::DigestMismatch { expected, actual } => write!(f, "expected sha256 {expected}, got {actual}"),
            DownloadError::Io(reason) => write!(f, "could not store the download: {reason}"),
        }
    }
}

impl DownloadSpec {
    /// Builds a spec. The digest may be upper or lower case; it is stored in lower case.
    pub fn new(name: &str, expected_size: u64, sha256_hex: &str) -> Result<DownloadSpec, DownloadError> {
        if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
            return Err(DownloadError::InvalidName(name.into()));
        }
        let digest = sha256_hex.to_ascii_lowercase();
        if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(DownloadError::InvalidDigest);
        }
        Ok(DownloadSpec { name: name.into(), expected_size, sha256: digest })
    }

    /// The final file name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The exact payload size in bytes.
    pub fn expected_size(&self) -> u64 {
        self.expected_size
    }

    /// The expected SHA-256 digest in lower-case hexadecimal.
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// The name a payload has while it is being written: the final name with a `.part` suffix.
    pub fn partial_name(&self) -> String {
        format!("{}.part", self.name)
    }

    /// Checks a complete payload against the size and the digest.
    pub fn verify(&self, data: &[u8]) -> Result<(), DownloadError> {
        let actual_size = data.len() as u64;
        if actual_size != self.expected_size {
            return Err(DownloadError::SizeMismatch { expected: self.expected_size, actual: actual_size });
        }
        let actual = hex(&sha256(data));
        if actual != self.sha256 {
            return Err(DownloadError::DigestMismatch { expected: self.sha256.clone(), actual });
        }
        Ok(())
    }

    /// The share of the payload received so far, from 0.0 to 1.0. A spec with no bytes is complete from the start.
    pub fn fraction(&self, received: u64) -> f32 {
        if self.expected_size == 0 {
            return 1.0;
        }
        received.min(self.expected_size) as f32 / self.expected_size as f32
    }
}

/// Verifies `data` against `spec`, writes it to `<dir>/<name>.part` and renames it to `<dir>/<name>`. Returns the final
/// path. A payload that fails the check is never written. A write or rename that fails removes the partial file.
pub fn stage(dir: &Path, spec: &DownloadSpec, data: &[u8]) -> Result<PathBuf, DownloadError> {
    spec.verify(data)?;
    let part = dir.join(spec.partial_name());
    let target = dir.join(spec.name());
    let written = fs::write(&part, data).and_then(|()| fs::rename(&part, &target));
    if written.is_err() {
        let _ = fs::remove_file(&part);
    }
    written.map(|()| target).map_err(|e| DownloadError::Io(e.to_string()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
