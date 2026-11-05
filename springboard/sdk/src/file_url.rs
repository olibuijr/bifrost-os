//! File URL value type with path component operations.
//!
//! Matches the path-handling subset of the reference SDK's URL type: building,
//! parsing and printing `file://` URLs, and appending, deleting and
//! standardizing path components and extensions. Pure std; `standardized`
//! never touches the filesystem, `resolving_symlinks` does.
//!
//! Not matched: non-file schemes and network URLs (`url.rs` covers requests),
//! bookmark data, resource-value caching on the URL object (`file_manager.rs`
//! has resource values) and the enumerator (`file_manager.rs` enumerates).

use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};

/// A file URL: a path plus whether it names a directory.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FileUrl {
    path: PathBuf,
    is_directory: bool,
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn percent_decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hi = hex(*b.get(i + 1)?)?;
            let lo = hex(*b.get(i + 2)?)?;
            out.push(hi << 4 | lo);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Some(out)
}

fn percent_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &c in bytes {
        if c.is_ascii_alphanumeric() || matches!(c, b'/' | b'-' | b'.' | b'_' | b'~') {
            out.push(c as char);
        } else {
            out.push_str(&format!("%{c:02X}"));
        }
    }
    out
}

impl FileUrl {
    pub fn from_path(p: impl Into<PathBuf>, is_directory: bool) -> FileUrl {
        FileUrl { path: p.into(), is_directory }
    }

    /// Parses `file:///a/b%20c/`. A trailing slash marks a directory. Other
    /// schemes and hosts other than empty or `localhost` are rejected.
    pub fn parse(s: &str) -> Option<FileUrl> {
        let rest = s.strip_prefix("file://")?;
        let rest = if rest.starts_with('/') {
            rest
        } else {
            rest.strip_prefix("localhost").filter(|r| r.starts_with('/'))?
        };
        let bytes = percent_decode(rest)?;
        let is_directory = bytes.ends_with(b"/");
        let mut path = PathBuf::from(OsString::from_vec(bytes));
        if is_directory && path.as_os_str().len() > 1 {
            // PathBuf normalizes the trailing slash away on components.
            path = path.components().collect();
        }
        Some(FileUrl { path, is_directory })
    }

    pub fn as_string(&self) -> String {
        let mut s = format!("file://{}", percent_encode(self.path.as_os_str().as_bytes()));
        if self.is_directory && !s.ends_with('/') {
            s.push('/');
        }
        s
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_directory(&self) -> bool {
        self.is_directory
    }

    pub fn appending_path_component(&self, name: &str, is_directory: bool) -> FileUrl {
        FileUrl { path: self.path.join(name), is_directory }
    }

    /// The parent. Deleting from `/` stays `/`; a single relative name gives `.`.
    pub fn deleting_last_path_component(&self) -> FileUrl {
        let path = match self.path.parent() {
            Some(p) if p.as_os_str().is_empty() => PathBuf::from("."),
            Some(p) => p.to_path_buf(),
            None => self.path.clone(),
        };
        FileUrl { path, is_directory: true }
    }

    pub fn last_path_component(&self) -> String {
        match self.path.file_name() {
            Some(n) => n.to_string_lossy().into_owned(),
            None if self.path.has_root() => "/".to_string(),
            None => String::new(),
        }
    }

    /// Extension of the last component; `.bashrc` has none.
    pub fn path_extension(&self) -> String {
        self.path
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub fn appending_path_extension(&self, ext: &str) -> FileUrl {
        let mut name = match self.path.file_name() {
            Some(n) => n.to_os_string(),
            None => return self.clone(),
        };
        name.push(".");
        name.push(ext);
        FileUrl { path: self.path.with_file_name(name), is_directory: self.is_directory }
    }

    pub fn deleting_path_extension(&self) -> FileUrl {
        if self.path.extension().is_none() {
            return self.clone();
        }
        FileUrl { path: self.path.with_extension(""), is_directory: self.is_directory }
    }

    /// Resolves `.` and `..` lexically. Leading `..` of a relative path are
    /// kept; `..` at the root is dropped.
    pub fn standardized(&self) -> FileUrl {
        let mut out: Vec<Component> = Vec::new();
        for c in self.path.components() {
            match c {
                Component::CurDir => {}
                Component::ParentDir => match out.last() {
                    Some(Component::Normal(_)) => {
                        out.pop();
                    }
                    Some(Component::RootDir) | Some(Component::Prefix(_)) => {}
                    _ => out.push(c),
                },
                _ => out.push(c),
            }
        }
        let path: PathBuf = out.into_iter().collect();
        FileUrl { path, is_directory: self.is_directory }
    }

    /// Canonicalizes through the filesystem; returns self when that fails.
    pub fn resolving_symlinks(&self) -> FileUrl {
        match std::fs::canonicalize(&self.path) {
            Ok(p) => FileUrl { path: p, is_directory: self.is_directory },
            Err(_) => self.clone(),
        }
    }

    /// Components in order; an absolute path starts with `/`.
    pub fn path_components(&self) -> Vec<String> {
        self.path
            .components()
            .map(|c| match c {
                Component::RootDir => "/".to_string(),
                other => other.as_os_str().to_string_lossy().into_owned(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(p: &str) -> FileUrl {
        FileUrl::from_path(p, false)
    }

    #[test]
    fn parse_decodes_and_round_trips() {
        let u = FileUrl::parse("file:///a/b%20c/").unwrap();
        assert_eq!(u.path(), Path::new("/a/b c"));
        assert!(u.is_directory());
        assert_eq!(u.as_string(), "file:///a/b%20c/");
        let n = FileUrl::from_path("/h/Skjöl/ég.txt", false);
        assert_eq!(n.as_string(), "file:///h/Skj%C3%B6l/%C3%A9g.txt");
        assert_eq!(FileUrl::parse(&n.as_string()).unwrap(), n);
    }

    #[test]
    fn directory_slash() {
        assert_eq!(FileUrl::from_path("/a", true).as_string(), "file:///a/");
        assert_eq!(FileUrl::from_path("/a", false).as_string(), "file:///a");
        assert_eq!(FileUrl::from_path("/", true).as_string(), "file:///");
        assert!(!FileUrl::parse("file:///a").unwrap().is_directory());
        assert!(FileUrl::parse("file://localhost/a/").is_some());
    }

    #[test]
    fn other_schemes_rejected() {
        assert!(FileUrl::parse("https://example.com/a").is_none());
        assert!(FileUrl::parse("/a/b").is_none());
        assert!(FileUrl::parse("file://host/a").is_none());
        assert!(FileUrl::parse("file:///a%2").is_none());
    }

    #[test]
    fn component_operations() {
        let u = f("/a/b").appending_path_component("c", true);
        assert_eq!(u.path(), Path::new("/a/b/c"));
        assert!(u.is_directory());
        assert_eq!(u.last_path_component(), "c");
        assert_eq!(u.deleting_last_path_component().path(), Path::new("/a/b"));
        assert_eq!(f("/").deleting_last_path_component().path(), Path::new("/"));
        assert_eq!(f("/").last_path_component(), "/");
        assert_eq!(f("x").deleting_last_path_component().path(), Path::new("."));
        assert_eq!(f("/a/b").path_components(), vec!["/", "a", "b"]);
        assert_eq!(f("a/b").path_components(), vec!["a", "b"]);
    }

    #[test]
    fn extension_operations() {
        assert_eq!(f("/a/x.txt").path_extension(), "txt");
        assert_eq!(f("/a/.bashrc").path_extension(), "");
        assert_eq!(f("/a/x").path_extension(), "");
        assert_eq!(f("/a/x").appending_path_extension("md").path(), Path::new("/a/x.md"));
        assert_eq!(f("/a/x.tar.gz").deleting_path_extension().path(), Path::new("/a/x.tar"));
        assert_eq!(f("/a/.bashrc").deleting_path_extension().path(), Path::new("/a/.bashrc"));
        assert_eq!(f("/a/x").deleting_path_extension().path(), Path::new("/a/x"));
    }

    #[test]
    fn standardized_is_lexical() {
        assert_eq!(f("a/b/../c").standardized().path(), Path::new("a/c"));
        assert_eq!(f("/a/./b//c/..").standardized().path(), Path::new("/a/b"));
        assert_eq!(f("/../a").standardized().path(), Path::new("/a"));
        assert_eq!(f("../a/..").standardized().path(), Path::new(".."));
        assert_eq!(f("/no/such/dir/../x").standardized().path(), Path::new("/no/such/x"));
    }

    #[test]
    fn resolving_symlinks_falls_back() {
        let missing = f("/no/such/path/for/file_url");
        assert_eq!(missing.resolving_symlinks(), missing);
        assert!(f("/").resolving_symlinks().path().is_absolute());
    }
}
