//! Package layout rules: where each entry of an app package (`.aap`, see `springboard/docs/apps.md`) may sit.
//!
//! A package has a fixed shape. The manifest, the icon and the executable named by `exec` sit at the top level.
//! Resources live under `assets/` at any depth. Shared libraries go flat in `libs/` and loadable plug-ins flat in
//! `plugins/`, each as one `.so` file, so the shell can find them without a search. Every other entry is refused.
//!
//! The rules are pure: they look only at the entry names as the archive spells them (`/` separated) and never
//! touch the file system.

use std::fmt;

/// Where an entry of a package belongs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// `manifest.toml`, the package description.
    Manifest,
    /// The executable named by the manifest `exec` key.
    Executable,
    /// `icon.png`.
    Icon,
    /// Anything under `assets/`.
    Resource,
    /// A flat `libs/<name>.so` shared library.
    Library,
    /// A flat `plugins/<name>.so` plug-in.
    Plugin,
}

/// Why a package entry or a package was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutError {
    /// The entry name is absolute, empty, or contains `.` or `..` components.
    UnsafeName(String),
    /// The entry is not at any place the layout allows.
    Unplaced(String),
    /// A library or plug-in is nested, unnamed or not a `.so` file.
    BadModule { path: String, expected: &'static str },
    /// The `exec` value is not a bare file name.
    BadExec(String),
    /// A required entry is missing.
    Missing(&'static str),
    /// The same entry name appears twice.
    Duplicate(String),
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayoutError::UnsafeName(p) => write!(f, "unsafe package entry name: {p}"),
            LayoutError::Unplaced(p) => write!(f, "package entry has no place in the layout: {p}"),
            LayoutError::BadModule { path, expected } => write!(f, "{path}: {expected}"),
            LayoutError::BadExec(e) => write!(f, "exec must be a bare file name: {e}"),
            LayoutError::Missing(what) => write!(f, "package is missing {what}"),
            LayoutError::Duplicate(p) => write!(f, "package entry appears twice: {p}"),
        }
    }
}

/// Splits an entry name into components, refusing absolute names, empty components and `.` or `..`.
fn components(name: &str) -> Result<Vec<&str>, LayoutError> {
    if name.is_empty() || name.starts_with('/') || name.contains('\\') {
        return Err(LayoutError::UnsafeName(name.to_string()));
    }
    let parts: Vec<&str> = name.split('/').collect();
    if parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") {
        return Err(LayoutError::UnsafeName(name.to_string()));
    }
    Ok(parts)
}

/// Whether `file` is a shared-object name with a non-empty stem, such as `libfoo.so`.
fn is_module_file(file: &str) -> bool {
    file.strip_suffix(".so").is_some_and(|stem| !stem.is_empty())
}

/// Classifies one entry name of a package whose executable is `exec`.
pub fn classify(name: &str, exec: &str) -> Result<Placement, LayoutError> {
    let parts = components(name)?;
    match parts.as_slice() {
        ["manifest.toml"] => Ok(Placement::Manifest),
        ["icon.png"] => Ok(Placement::Icon),
        [file] if *file == exec => Ok(Placement::Executable),
        ["libs"] | ["plugins"] => Err(LayoutError::Unplaced(name.to_string())),
        ["assets", ..] if parts.len() > 1 => Ok(Placement::Resource),
        ["libs", file] if is_module_file(file) => Ok(Placement::Library),
        ["plugins", file] if is_module_file(file) => Ok(Placement::Plugin),
        ["libs", _file] | ["plugins", _file] => Err(LayoutError::BadModule {
            path: name.to_string(),
            expected: "a shared module must be a named .so file directly in libs/ or plugins/",
        }),
        ["libs", ..] | ["plugins", ..] => Err(LayoutError::BadModule {
            path: name.to_string(),
            expected: "modules are flat: no folders inside libs/ or plugins/",
        }),
        _ => Err(LayoutError::Unplaced(name.to_string())),
    }
}

/// Checks a whole package listing. Returns every problem found, or the placement of each entry in order.
/// The package must hold a manifest and the executable, and no name may appear twice.
pub fn check_package<'a, I>(names: I, exec: &str) -> Result<Vec<Placement>, Vec<LayoutError>>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut errors = Vec::new();
    if exec.is_empty() || exec.contains('/') || exec == "." || exec == ".." {
        errors.push(LayoutError::BadExec(exec.to_string()));
    }
    let mut placements = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for name in names {
        if seen.contains(&name) {
            errors.push(LayoutError::Duplicate(name.to_string()));
            continue;
        }
        seen.push(name);
        match classify(name, exec) {
            Ok(p) => placements.push(p),
            Err(e) => errors.push(e),
        }
    }
    for (present, what) in [(Placement::Manifest, "manifest.toml"), (Placement::Executable, "the exec binary")] {
        if !placements.contains(&present) {
            errors.push(LayoutError::Missing(what));
        }
    }
    if errors.is_empty() {
        Ok(placements)
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_level_entries_are_placed() {
        assert_eq!(classify("manifest.toml", "calc"), Ok(Placement::Manifest));
        assert_eq!(classify("icon.png", "calc"), Ok(Placement::Icon));
        assert_eq!(classify("calc", "calc"), Ok(Placement::Executable));
    }

    #[test]
    fn resources_go_under_assets_at_any_depth() {
        assert_eq!(classify("assets/words/en.txt", "calc"), Ok(Placement::Resource));
        assert_eq!(classify("assets/OFL.txt", "calc"), Ok(Placement::Resource));
        assert!(matches!(classify("assets", "calc"), Err(LayoutError::Unplaced(_))));
    }

    #[test]
    fn libraries_and_plugins_are_flat_so_files() {
        assert_eq!(classify("libs/libgfx.so", "calc"), Ok(Placement::Library));
        assert_eq!(classify("plugins/export.so", "calc"), Ok(Placement::Plugin));
        assert!(matches!(classify("libs/x.dylib", "calc"), Err(LayoutError::BadModule { .. })));
        assert!(matches!(classify("libs/.so", "calc"), Err(LayoutError::BadModule { .. })));
        assert!(matches!(classify("plugins/deep/export.so", "calc"), Err(LayoutError::BadModule { .. })));
        assert!(matches!(classify("libs", "calc"), Err(LayoutError::Unplaced(_))));
    }

    #[test]
    fn unknown_top_level_files_are_refused() {
        assert!(matches!(classify("secret.txt", "calc"), Err(LayoutError::Unplaced(_))));
        assert!(matches!(classify("bin/calc", "calc"), Err(LayoutError::Unplaced(_))));
    }

    #[test]
    fn unsafe_names_are_refused() {
        for bad in ["", "/etc/passwd", "assets/../../x", "./manifest.toml", "assets//a", "assets\\a", "a/./b"] {
            assert!(matches!(classify(bad, "calc"), Err(LayoutError::UnsafeName(_))), "{bad}");
        }
    }

    #[test]
    fn a_complete_package_passes() {
        let names = ["manifest.toml", "icon.png", "calc", "assets/OFL.txt", "libs/libmath.so"];
        let placed = check_package(names, "calc").unwrap();
        assert_eq!(placed, vec![
            Placement::Manifest,
            Placement::Icon,
            Placement::Executable,
            Placement::Resource,
            Placement::Library,
        ]);
    }

    #[test]
    fn missing_manifest_or_binary_is_reported() {
        let errs = check_package(["icon.png"], "calc").unwrap_err();
        assert!(errs.contains(&LayoutError::Missing("manifest.toml")));
        assert!(errs.contains(&LayoutError::Missing("the exec binary")));
    }

    #[test]
    fn every_problem_is_collected_at_once() {
        let errs = check_package(["manifest.toml", "calc", "calc", "junk", "../up"], "calc").unwrap_err();
        assert!(errs.contains(&LayoutError::Duplicate("calc".into())));
        assert!(errs.contains(&LayoutError::Unplaced("junk".into())));
        assert!(errs.iter().any(|e| matches!(e, LayoutError::UnsafeName(_))));
        assert_eq!(errs.len(), 3);
    }

    #[test]
    fn exec_must_be_a_bare_name() {
        let errs = check_package(["manifest.toml", "bin/calc"], "bin/calc").unwrap_err();
        assert!(errs.iter().any(|e| matches!(e, LayoutError::BadExec(_))));
    }
}
