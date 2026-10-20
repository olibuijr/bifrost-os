//! Where the system's own assets (UI fonts, app icons, wallpapers) live on an installed machine.
//!
//! The `springboard-shell` update package owns [`SYSTEM_ASSETS`], so an update can change them. Older installs, the live
//! medium, host previews and tests keep them in [`LEGACY_ASSETS`], which no package owns. Every reader asks this module
//! for the root instead of naming a path: the development override `BIFROST_ASSETS` wins, then the package-owned
//! directory when it holds files, then the legacy one.

use std::path::{Path, PathBuf};

/// The package-owned asset directory (`springboard-shell` ships it; docs/bifrost-pc-workflow.md, "System assets").
pub const SYSTEM_ASSETS: &str = "/usr/share/springboard/system-assets";
/// The unowned directory older installs and the live medium use.
pub const LEGACY_ASSETS: &str = "/usr/share/springboard/assets";
/// The development override: a directory that replaces both.
pub const ENV: &str = "BIFROST_ASSETS";

/// Whether `dir` is a directory with at least one entry (an empty package directory must not hide the legacy one).
fn populated(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some())
}

/// The assets root for an override (`over`, empty counts as unset), the package-owned directory `system` and the
/// fallback `legacy`. The pure core of [`root`].
pub fn pick(over: Option<&str>, system: &Path, legacy: &Path) -> PathBuf {
    match over.filter(|o| !o.is_empty()) {
        Some(o) => PathBuf::from(o),
        None if populated(system) => system.to_path_buf(),
        None => legacy.to_path_buf(),
    }
}

/// The assets root of this machine.
pub fn root() -> PathBuf {
    pick(std::env::var(ENV).ok().as_deref(), Path::new(SYSTEM_ASSETS), Path::new(LEGACY_ASSETS))
}

/// [`root`] as a string, for the readers that build paths with `format!`.
pub fn root_string() -> String {
    root().to_string_lossy().into_owned()
}

/// Whether the system mounted at `prefix` (the installer's `/mnt`) has the package-owned asset directory with files in it.
pub fn provided_under(prefix: &Path) -> bool {
    populated(&prefix.join(SYSTEM_ASSETS.trim_start_matches('/')))
}

/// The assets root of another system mounted at `prefix`: no override, the package-owned directory when the installed
/// package provided it, else the legacy one.
pub fn root_under(prefix: &Path) -> PathBuf {
    let under = |p: &str| prefix.join(p.trim_start_matches('/'));
    pick(None, &under(SYSTEM_ASSETS), &under(LEGACY_ASSETS))
}

/// Every directory a reader that searches several places should try, in order: the override, the package-owned
/// directory, the legacy one.
pub fn candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(o) = std::env::var_os(ENV).filter(|o| !o.is_empty()) {
        v.push(PathBuf::from(o));
    }
    v.push(PathBuf::from(SYSTEM_ASSETS));
    v.push(PathBuf::from(LEGACY_ASSETS));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("system-assets-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_override_wins_over_both_directories() {
        let d = tmp("override");
        std::fs::write(d.join("SFUI.ttf"), b"x").unwrap();
        assert_eq!(pick(Some("/dev/assets"), &d, Path::new("/legacy")), PathBuf::from("/dev/assets"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_empty_override_counts_as_unset() {
        let d = tmp("empty-override");
        std::fs::write(d.join("SFUI.ttf"), b"x").unwrap();
        assert_eq!(pick(Some(""), &d, Path::new("/legacy")), d);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_package_owned_directory_comes_before_the_legacy_one() {
        let d = tmp("order");
        std::fs::write(d.join("SFUI.ttf"), b"x").unwrap();
        assert_eq!(pick(None, &d, Path::new("/legacy")), d);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_missing_or_empty_package_directory_falls_back_to_the_legacy_one() {
        let d = tmp("fallback");
        let legacy = Path::new("/legacy");
        assert_eq!(pick(None, &d.join("absent"), legacy), legacy);
        assert_eq!(pick(None, &d, legacy), legacy, "an empty directory must not hide the legacy assets");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn root_under_looks_inside_the_prefix() {
        let d = tmp("under");
        std::fs::create_dir_all(d.join("usr/share/springboard/assets")).unwrap();
        assert_eq!(root_under(&d), d.join("usr/share/springboard/assets"));
        std::fs::create_dir_all(d.join("usr/share/springboard/system-assets")).unwrap();
        assert_eq!(root_under(&d), d.join("usr/share/springboard/assets"), "empty system-assets is ignored");
        assert!(!provided_under(&d));
        std::fs::write(d.join("usr/share/springboard/system-assets/SFUI.ttf"), b"x").unwrap();
        assert_eq!(root_under(&d), d.join("usr/share/springboard/system-assets"));
        assert!(provided_under(&d));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_search_list_ends_with_the_two_installed_paths() {
        let c = candidates();
        assert_eq!(&c[c.len() - 2..], [PathBuf::from(SYSTEM_ASSETS), PathBuf::from(LEGACY_ASSETS)]);
    }
}
