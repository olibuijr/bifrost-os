//! Resource seals: a static check that the files of a program still match the digests recorded when it was built
//! (task w5-security, "Code signing services").
//!
//! A [`Seal`] lists each file of a program bundle by its relative path and the SHA-256 of its bytes. [`Seal::verify`]
//! compares a set of files with the seal and reports every sealed file that changed or is missing, and every file the
//! seal does not list. The model reads no disk and no signature; the caller passes the file bytes in. The seal is an
//! integrity record, not a signature: BifrOSt has no signing key yet, so anyone who can rewrite the seal file can also
//! recompute its digest. Use it to detect a damaged or edited bundle, not to prove who built it.
//!
//! The text form is one line per file, `<64 hex digits>  <path>` (two spaces, as the common checksum tools write it),
//! sorted by path, so a seal file can be diffed and checked with ordinary tools.

use std::collections::{BTreeMap, BTreeSet};

use crate::sha256;

/// One sealed file: its path inside the bundle and the SHA-256 of its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub digest: [u8; 32],
}

/// Why a seal cannot be built from the given files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildError {
    /// The path is empty, absolute, has an empty, `.` or `..` part, or contains a backslash or a NUL byte.
    BadPath(String),
    /// Two files have the same path.
    Duplicate(String),
}

/// Why a seal text cannot be read. Line numbers start at 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The line is not `<64 hex digits>  <path>`.
    BadLine(usize),
    /// The path on the line is not allowed (see [`BuildError::BadPath`]).
    BadPath(usize),
    /// The same path appears on two lines.
    Duplicate(String),
}

/// The outcome of checking files against a seal. Every list is sorted by path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Verdict {
    /// Sealed paths whose bytes differ from the seal, or that were passed in more than once.
    pub modified: Vec<String>,
    /// Sealed paths for which no file was passed in.
    pub missing: Vec<String>,
    /// Passed-in paths that the seal does not list.
    pub unsealed: Vec<String>,
}

impl Verdict {
    /// True when every sealed file is present and unchanged and no file is unsealed.
    pub fn is_valid(&self) -> bool {
        self.modified.is_empty() && self.missing.is_empty() && self.unsealed.is_empty()
    }
}

/// The digests of a program bundle's files, sorted by path with no repeats.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seal {
    entries: Vec<Entry>,
}

/// True for a relative path made only of normal parts, with `/` as the separator.
fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && !path.contains('\0')
        && path.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

impl Seal {
    /// Seals `files`, given as (relative path, bytes) pairs in any order. An empty list makes an empty seal.
    pub fn build(files: &[(&str, &[u8])]) -> Result<Seal, BuildError> {
        let mut map: BTreeMap<String, [u8; 32]> = BTreeMap::new();
        for &(path, bytes) in files {
            if !valid_path(path) {
                return Err(BuildError::BadPath(path.to_string()));
            }
            if map.insert(path.to_string(), sha256::digest(bytes)).is_some() {
                return Err(BuildError::Duplicate(path.to_string()));
            }
        }
        Ok(Seal::from_map(map))
    }

    fn from_map(map: BTreeMap<String, [u8; 32]>) -> Seal {
        let entries = map.into_iter().map(|(path, digest)| Entry { path, digest }).collect();
        Seal { entries }
    }

    /// The sealed files, sorted by path.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Number of sealed files.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the seal lists no files.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The seal as text: one `<hex digest>  <path>` line per file, each ending in a newline.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for entry in &self.entries {
            out.push_str(&sha256::hex(&entry.digest));
            out.push_str("  ");
            out.push_str(&entry.path);
            out.push('\n');
        }
        out
    }

    /// SHA-256 of [`Seal::to_text`]. It changes whenever a path or a digest changes, so a stored copy can be checked
    /// against it. It is not a signature (see the module comment).
    pub fn seal_digest(&self) -> [u8; 32] {
        sha256::digest(self.to_text().as_bytes())
    }

    /// Reads a seal written by [`Seal::to_text`]. Lines may come in any order; the result is sorted.
    pub fn parse(text: &str) -> Result<Seal, ParseError> {
        let mut map: BTreeMap<String, [u8; 32]> = BTreeMap::new();
        for (index, line) in text.lines().enumerate() {
            let number = index + 1;
            let (hex, path) = line.split_once("  ").ok_or(ParseError::BadLine(number))?;
            let bytes = sha256::unhex(hex).ok_or(ParseError::BadLine(number))?;
            if bytes.len() != 32 {
                return Err(ParseError::BadLine(number));
            }
            if !valid_path(path) {
                return Err(ParseError::BadPath(number));
            }
            let mut digest = [0u8; 32];
            digest.copy_from_slice(&bytes);
            if map.insert(path.to_string(), digest).is_some() {
                return Err(ParseError::Duplicate(path.to_string()));
            }
        }
        Ok(Seal::from_map(map))
    }

    /// Checks `files` (relative path and bytes) against the seal. A path passed in twice is reported as modified,
    /// because the caller cannot tell which copy is the real file.
    pub fn verify(&self, files: &[(&str, &[u8])]) -> Verdict {
        let mut present: BTreeMap<&str, [u8; 32]> = BTreeMap::new();
        let mut repeated: BTreeSet<String> = BTreeSet::new();
        for &(path, bytes) in files {
            if present.insert(path, sha256::digest(bytes)).is_some() {
                repeated.insert(path.to_string());
            }
        }
        let mut verdict = Verdict::default();
        for entry in &self.entries {
            match present.remove(entry.path.as_str()) {
                None => verdict.missing.push(entry.path.clone()),
                Some(digest) if digest != entry.digest => verdict.modified.push(entry.path.clone()),
                Some(_) => {}
            }
        }
        verdict.modified.extend(repeated);
        verdict.modified.sort();
        verdict.modified.dedup();
        verdict.unsealed = present.keys().map(|path| path.to_string()).collect();
        verdict
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY_DIGEST: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn an_empty_file_has_the_known_sha256_digest() {
        let empty: &[u8] = b"";
        let seal = Seal::build(&[("a", empty)]).unwrap();
        assert_eq!(seal.to_text(), format!("{EMPTY_DIGEST}  a\n"));
    }

    #[test]
    fn entries_are_sorted_by_path_whatever_the_input_order() {
        let x: &[u8] = b"x";
        let y: &[u8] = b"y";
        let seal = Seal::build(&[("b/two", x), ("a", y), ("b/one", x)]).unwrap();
        let paths: Vec<&str> = seal.entries().iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["a", "b/one", "b/two"]);
        assert_eq!(seal.len(), 3);
    }

    #[test]
    fn unchanged_files_verify_and_any_change_is_reported() {
        let one: &[u8] = b"one";
        let two: &[u8] = b"two";
        let seal = Seal::build(&[("bin/app", one), ("data/b", two)]).unwrap();
        assert!(seal.verify(&[("bin/app", one), ("data/b", two)]).is_valid());

        // Edit bin/app, drop data/b, add an unsealed file.
        let edited: &[u8] = b"ONE";
        let extra: &[u8] = b"extra";
        let verdict = seal.verify(&[("bin/app", edited), ("notes", extra)]);
        assert!(!verdict.is_valid());
        assert_eq!(verdict.modified, vec!["bin/app".to_string()]);
        assert_eq!(verdict.missing, vec!["data/b".to_string()]);
        assert_eq!(verdict.unsealed, vec!["notes".to_string()]);
    }

    #[test]
    fn a_path_passed_twice_is_reported_as_modified() {
        let one: &[u8] = b"one";
        let seal = Seal::build(&[("bin/app", one)]).unwrap();
        let verdict = seal.verify(&[("bin/app", one), ("bin/app", one)]);
        assert_eq!(verdict.modified, vec!["bin/app".to_string()]);
        assert!(!verdict.is_valid());
    }

    #[test]
    fn unsafe_paths_and_repeated_paths_are_refused_when_sealing() {
        let body: &[u8] = b"x";
        for bad in ["", "/etc/passwd", "a/../b", "a//b", "dir/", "./a", "a\\b", "a\0b"] {
            assert_eq!(Seal::build(&[(bad, body)]), Err(BuildError::BadPath(bad.to_string())), "{bad:?}");
        }
        assert!(Seal::build(&[("a/b.txt", body)]).is_ok());
        assert_eq!(
            Seal::build(&[("a", body), ("a", body)]),
            Err(BuildError::Duplicate("a".to_string()))
        );
    }

    #[test]
    fn the_text_form_round_trips_and_damage_is_detected() {
        let one: &[u8] = b"one";
        let two: &[u8] = b"two";
        let seal = Seal::build(&[("x", one), ("y/z", two)]).unwrap();
        let text = seal.to_text();
        assert_eq!(Seal::parse(&text), Ok(seal.clone()));

        // A line with no double space, a short digest, and a repeated path are each refused.
        assert_eq!(Seal::parse("not-a-seal\n"), Err(ParseError::BadLine(1)));
        assert_eq!(Seal::parse("abcd  x\n"), Err(ParseError::BadLine(1)));
        let twice = format!("{EMPTY_DIGEST}  x\n{EMPTY_DIGEST}  x\n");
        assert_eq!(Seal::parse(&twice), Err(ParseError::Duplicate("x".to_string())));

        // Changing one digest changes the seal digest.
        let altered = text.replacen(&sha256::hex(&seal.entries()[0].digest), EMPTY_DIGEST, 1);
        assert_ne!(Seal::parse(&altered).unwrap().seal_digest(), seal.seal_digest());
    }
}
