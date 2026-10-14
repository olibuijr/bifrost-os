//! Where the files of an app package go (`Bundle` placement): the manifest, executable and icon at the package root,
//! shared libraries in `libs/`, plug-in modules in `plugins/` and resources under `assets/`. A path that climbs out of
//! the package, or that sits outside these places, is refused. Pure: the caller lists the package's entries and decides
//! what to do with a refusal. Paths use `/` and are relative to the package root.

/// The file every package carries at its root.
pub const MANIFEST: &str = "manifest.toml";

/// Where one file of a package is placed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Placement {
    /// A file at the package root: the manifest, the executable and the icon.
    Root,
    /// `libs/<name>.so`: a shared library the app loads, named without its extension.
    Library(String),
    /// `plugins/<name>.so`: a plug-in module, named without its extension.
    Plugin(String),
    /// `assets/<path>`: a resource, kept with its path below `assets/`.
    Resource(String),
}

/// Why a path is not placed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The path is empty, absolute, has a backslash or NUL, or has an empty, `.` or `..` segment.
    Unsafe(String),
    /// The path is safe but is not in a place the layout defines.
    Unplaced(String),
    /// The package has no `manifest.toml` at its root.
    NoManifest,
}

/// Places one package-relative path.
pub fn classify(path: &str) -> Result<Placement, Refusal> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') || path.contains('\0') {
        return Err(Refusal::Unsafe(path.to_string()));
    }
    let parts: Vec<&str> = path.split('/').collect();
    if parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") {
        return Err(Refusal::Unsafe(path.to_string()));
    }
    let unplaced = || Refusal::Unplaced(path.to_string());
    match parts.as_slice() {
        [_] => Ok(Placement::Root),
        ["libs", file] => module_name(file).map(Placement::Library).ok_or_else(unplaced),
        ["plugins", file] => module_name(file).map(Placement::Plugin).ok_or_else(unplaced),
        ["assets", rest @ ..] => Ok(Placement::Resource(rest.join("/"))),
        _ => Err(unplaced()),
    }
}

/// Checks every entry of a package: each path must be placed, and the manifest must be at the root.
/// The first refusal is returned. Directory entries are not files and must not be passed.
pub fn check_package(paths: &[&str]) -> Result<(), Refusal> {
    for &path in paths {
        classify(path)?;
    }
    if paths.contains(&MANIFEST) {
        Ok(())
    } else {
        Err(Refusal::NoManifest)
    }
}

/// The name of a `<name>.so` file, or `None` when the file has another extension or no name.
fn module_name(file: &str) -> Option<String> {
    file.strip_suffix(".so").filter(|name| !name.is_empty()).map(str::to_string)
}
