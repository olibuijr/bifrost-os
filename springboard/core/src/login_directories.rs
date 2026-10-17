//! The per-user directories a login creates before any app runs (launchd's `dirhelper` job): the application
//! support directory and the cache directory. The cache directory is made private (mode 0700), as the per-user
//! cache helper makes it. Existing directories are left alone, so a second login changes nothing.
//!
//! The XDG values are read as the file manager reads them: an absolute `XDG_DATA_HOME` or `XDG_CACHE_HOME` wins,
//! a relative or empty one falls back to `$HOME/.local/share` or `$HOME/.cache`. The shell calls
//! [`ensure_for_env`] at start-up; the SDK's `NativeFileManager::ensure_login_directories` uses the same function.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

/// Creates the login directories for the user whose home is `home`. `data_home` and `cache_home` are the
/// `XDG_DATA_HOME` and `XDG_CACHE_HOME` values. Returns the support directory and the cache directory, in that order.
pub fn ensure_under(home: &Path, data_home: Option<&OsStr>, cache_home: Option<&OsStr>) -> io::Result<Vec<PathBuf>> {
    let support = base_dir(data_home, home, ".local/share");
    let cache = base_dir(cache_home, home, ".cache");
    fs::create_dir_all(&support)?;
    fs::DirBuilder::new().recursive(true).mode(0o700).create(&cache)?;
    Ok(vec![support, cache])
}

/// [`ensure_under`] for the running user: `HOME` must be set and non-empty.
pub fn ensure_for_env() -> io::Result<Vec<PathBuf>> {
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    let data_home = std::env::var_os("XDG_DATA_HOME");
    let cache_home = std::env::var_os("XDG_CACHE_HOME");
    ensure_under(&home, data_home.as_deref(), cache_home.as_deref())
}

/// An XDG base directory: the variable when it holds an absolute path, else `home/fallback`.
fn base_dir(var: Option<&OsStr>, home: &Path, fallback: &str) -> PathBuf {
    match var.map(Path::new) {
        Some(p) if p.is_absolute() => p.to_path_buf(),
        _ => home.join(fallback),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn creates_support_and_private_cache_directories() {
        let s = tempfile::tempdir().unwrap();
        let home = s.path().join("home");
        let dirs = ensure_under(&home, None, None).unwrap();
        assert_eq!(dirs, vec![home.join(".local/share"), home.join(".cache")]);
        assert!(dirs[0].is_dir() && dirs[1].is_dir());
        assert_eq!(fs::metadata(&dirs[1]).unwrap().permissions().mode() & 0o777, 0o700, "the cache directory is private");
    }

    #[test]
    fn a_second_login_keeps_existing_contents() {
        let s = tempfile::tempdir().unwrap();
        let home = s.path().join("home");
        let dirs = ensure_under(&home, None, None).unwrap();
        fs::write(dirs[1].join("entry"), "kept").unwrap();
        assert_eq!(ensure_under(&home, None, None).unwrap(), dirs);
        assert_eq!(fs::read_to_string(dirs[1].join("entry")).unwrap(), "kept");
    }

    #[test]
    fn honours_absolute_xdg_values_only() {
        let s = tempfile::tempdir().unwrap();
        let home = s.path().join("home");
        let data = s.path().join("xdg-data");
        let dirs = ensure_under(&home, Some(data.as_os_str()), Some(OsStr::new("relative"))).unwrap();
        assert_eq!(dirs, vec![data.clone(), home.join(".cache")], "a relative XDG value falls back to the home default");
        assert!(data.is_dir());
        assert!(!home.join(".local").exists(), "the overridden data home is the only support directory made");
    }
}
