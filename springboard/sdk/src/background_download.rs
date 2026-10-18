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

#[cfg(test)]
mod tests {
    use super::*;

    // SHA-256 of "abc" (FIPS 180-2 test vector).
    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bifrost-dl-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn spec_rejects_bad_names_and_digests() {
        assert!(matches!(DownloadSpec::new("", 3, ABC_SHA256), Err(DownloadError::InvalidName(_))));
        assert!(matches!(DownloadSpec::new("..", 3, ABC_SHA256), Err(DownloadError::InvalidName(_))));
        assert!(matches!(DownloadSpec::new("a/b", 3, ABC_SHA256), Err(DownloadError::InvalidName(_))));
        assert_eq!(DownloadSpec::new("a.bin", 3, "short"), Err(DownloadError::InvalidDigest));
        assert_eq!(DownloadSpec::new("a.bin", 3, &"z".repeat(64)), Err(DownloadError::InvalidDigest));
    }

    #[test]
    fn spec_stores_digest_in_lower_case() {
        let spec = DownloadSpec::new("a.bin", 3, &ABC_SHA256.to_ascii_uppercase()).unwrap();
        assert_eq!(spec.sha256(), ABC_SHA256);
        assert_eq!(spec.name(), "a.bin");
        assert_eq!(spec.expected_size(), 3);
        assert_eq!(spec.partial_name(), "a.bin.part");
    }

    #[test]
    fn verify_accepts_matching_payload() {
        let spec = DownloadSpec::new("a.bin", 3, ABC_SHA256).unwrap();
        assert_eq!(spec.verify(b"abc"), Ok(()));
    }

    #[test]
    fn verify_reports_size_and_digest_mismatch() {
        let spec = DownloadSpec::new("a.bin", 3, ABC_SHA256).unwrap();
        assert_eq!(spec.verify(b"abcd"), Err(DownloadError::SizeMismatch { expected: 3, actual: 4 }));
        match spec.verify(b"xyz") {
            Err(DownloadError::DigestMismatch { expected, actual }) => {
                assert_eq!(expected, ABC_SHA256);
                assert_ne!(actual, ABC_SHA256);
                assert_eq!(actual.len(), 64);
            }
            other => panic!("expected a digest mismatch, got {other:?}"),
        }
    }

    #[test]
    fn fraction_is_clamped_and_complete_for_empty_payload() {
        let spec = DownloadSpec::new("a.bin", 4, ABC_SHA256).unwrap();
        assert_eq!(spec.fraction(0), 0.0);
        assert_eq!(spec.fraction(2), 0.5);
        assert_eq!(spec.fraction(99), 1.0);
        let empty = DownloadSpec::new("e.bin", 0, ABC_SHA256).unwrap();
        assert_eq!(empty.fraction(0), 1.0);
    }

    #[test]
    fn stage_writes_verified_payload_under_final_name() {
        let dir = scratch_dir("ok");
        let spec = DownloadSpec::new("a.bin", 3, ABC_SHA256).unwrap();
        let path = stage(&dir, &spec, b"abc").unwrap();
        assert_eq!(path, dir.join("a.bin"));
        assert_eq!(fs::read(&path).unwrap(), b"abc");
        assert!(!dir.join("a.bin.part").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stage_never_writes_a_payload_that_fails_the_check() {
        let dir = scratch_dir("bad");
        let spec = DownloadSpec::new("a.bin", 3, ABC_SHA256).unwrap();
        assert!(stage(&dir, &spec, b"nope").is_err());
        assert!(!dir.join("a.bin").exists());
        assert!(!dir.join("a.bin.part").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
