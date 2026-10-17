//! The extension tables of an installed package: `[[extension_point]]` (a point the package's app hosts) and
//! `[[extension]]` (an extension the package provides), read from its `manifest.toml` into the
//! [`crate::extension::Registry`]. These are the tables [`crate::extension::Point::manifest_toml`] and
//! [`crate::extension::Identity::manifest_toml`] write. Pure text in, model out: the shell reads the files.
//!
//! A package can only speak for its own namespace. An extension's `bundle_id` (default: the app id) must be the app id
//! or sit under it, and so must the `owner` of a point (default: the app id), so one package cannot register an
//! extension as another package or declare a point in another developer's namespace. The system declares the points
//! that belong to no app ([`system_points`], for example [`FILE_PROVIDER_POINT`]).
//!
//! ```toml
//! [[extension]]
//! point = "is.bifrost.file-provider"   # required
//! name = "Example Drive"               # shown in lists; for a file provider also the sidebar name
//! bundle_id = "com.example.drive.sync" # optional, under the app id
//! ```

use crate::extension::{Identity, Point, Registry, Scope};

/// The system extension point of file providers (`FileProvider`): an extension that serves a domain of items to Files.
pub const FILE_PROVIDER_POINT: &str = "is.bifrost.file-provider";

/// The namespace the system's own points sit under.
pub const SYSTEM_OWNER: &str = "is.bifrost";

/// The points the system declares itself.
pub fn system_points() -> Vec<Point> {
    Point::new(SYSTEM_OWNER, FILE_PROVIDER_POINT, Scope::Any).into_iter().collect()
}

/// `id` is `owner` itself or sits under its namespace.
fn within(owner: &str, id: &str) -> bool {
    id == owner || id.strip_prefix(owner).and_then(|rest| rest.strip_prefix('.')).is_some_and(|rest| !rest.is_empty())
}

/// What one manifest declares, after the namespace checks.
#[derive(Clone, Debug, Default)]
pub struct Declared {
    pub points: Vec<Point>,
    pub extensions: Vec<Identity>,
}

/// The tables of `text` as `(header, key/value pairs)`. Other tables end the current one and are skipped by the caller.
fn tables(text: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut out: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            out.push((line.to_string(), Vec::new()));
        } else if let (Some((key, value)), Some(last)) = (line.split_once('='), out.last_mut()) {
            last.1.push((key.trim().to_string(), value.trim().trim_matches('"').to_string()));
        }
    }
    out
}

fn field<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()).filter(|v| !v.is_empty())
}

/// Reads the extension tables of the manifest of the app `app_id`. A table that is malformed, or that names a
/// namespace the app does not own, is skipped.
pub fn parse(app_id: &str, text: &str) -> Declared {
    let mut out = Declared::default();
    for (header, pairs) in tables(text) {
        match header.as_str() {
            "[[extension_point]]" => {
                let Some(identifier) = field(&pairs, "identifier") else { continue };
                let owner = field(&pairs, "owner").unwrap_or(app_id);
                let scope = match field(&pairs, "scope") {
                    Some("same-developer") => Scope::SameDeveloper,
                    Some("any") | None => Scope::Any,
                    Some(_) => continue,
                };
                if !within(app_id, owner) {
                    continue;
                }
                if let Ok(point) = Point::new(owner, identifier, scope) {
                    out.points.push(point);
                }
            }
            "[[extension]]" => {
                // Login Items tables name a `kind` and no `point`; they are not registry extensions
                let Some(point) = field(&pairs, "point") else { continue };
                let bundle_id = field(&pairs, "bundle_id").unwrap_or(app_id);
                if !within(app_id, bundle_id) {
                    continue;
                }
                let name = field(&pairs, "name").unwrap_or(bundle_id);
                if let Ok(identity) = Identity::new(bundle_id, point, name) {
                    out.extensions.push(identity);
                }
            }
            _ => {}
        }
    }
    out
}

/// Which package an installed extension came from, so a host can start its binary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owned {
    pub bundle_id: String,
    pub point: String,
    pub app_id: String,
}

/// The registry built from every installed package, and the owner of each installed extension.
#[derive(Clone, Debug, Default)]
pub struct Scan {
    pub registry: Registry,
    pub owners: Vec<Owned>,
}

impl Scan {
    /// The package that provides extension `bundle_id` for `point`.
    pub fn owner(&self, bundle_id: &str, point: &str) -> Option<&str> {
        self.owners.iter().find(|o| o.bundle_id == bundle_id && o.point == point).map(|o| o.app_id.as_str())
    }
}

/// Builds the registry from `(app id, manifest text)` pairs: the system points first, then every package's points
/// (so an extension can be installed whichever package is read first), then every package's extensions. An extension
/// whose point is not declared or does not admit it is left out; when two packages claim one bundle and point, the
/// first wins.
pub fn scan<'a>(packages: impl IntoIterator<Item = (&'a str, &'a str)>) -> Scan {
    let declared: Vec<(&str, Declared)> = packages.into_iter().map(|(id, text)| (id, parse(id, text))).collect();
    let mut out = Scan::default();
    for point in system_points() {
        let _ = out.registry.declare(point);
    }
    for (_, d) in &declared {
        for point in &d.points {
            let _ = out.registry.declare(point.clone());
        }
    }
    for (app_id, d) in &declared {
        for identity in &d.extensions {
            if out.owner(&identity.bundle_id, &identity.point).is_some() {
                continue;
            }
            let (bundle_id, point) = (identity.bundle_id.clone(), identity.point.clone());
            if out.registry.install(identity.clone()).is_ok() {
                out.owners.push(Owned { bundle_id, point, app_id: (*app_id).to_string() });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRIVE: &str = "id = \"com.example.drive\"\nname = \"Drive\"\n\n[[extension]]\npoint = \"is.bifrost.file-provider\"\nname = \"Example Drive\"\n";

    #[test]
    fn an_extension_table_defaults_to_the_app_id() {
        let d = parse("com.example.drive", DRIVE);
        assert_eq!(d.extensions.len(), 1);
        assert_eq!(d.extensions[0].bundle_id, "com.example.drive");
        assert_eq!(d.extensions[0].point, FILE_PROVIDER_POINT);
        assert_eq!(d.extensions[0].name, "Example Drive");
        assert!(d.points.is_empty());
    }

    #[test]
    fn written_tables_read_back() {
        let point = Point::new("com.example.notes", "com.example.notes.share", Scope::SameDeveloper).unwrap();
        let identity = Identity::new("com.example.notes.pdf", "com.example.notes.share", "PDF").unwrap();
        let text = format!("{}\n{}", point.manifest_toml(), identity.manifest_toml());
        let d = parse("com.example.notes", &text);
        assert_eq!(d.points, vec![point]);
        assert_eq!(d.extensions, vec![identity]);
    }

    #[test]
    fn a_package_cannot_claim_another_namespace() {
        let text = "[[extension]]\nbundle_id = \"com.other.app\"\npoint = \"is.bifrost.file-provider\"\n\n[[extension_point]]\nidentifier = \"com.other.app.share\"\nowner = \"com.other.app\"\n\n[[extension_point]]\nidentifier = \"com.example.drive.open\"\n";
        let d = parse("com.example.drive", text);
        assert!(d.extensions.is_empty(), "bundle id outside the app's namespace");
        assert_eq!(d.points.len(), 1, "only the point under the app's own id");
        assert_eq!(d.points[0].identifier, "com.example.drive.open");
    }

    #[test]
    fn tables_without_a_point_or_with_a_bad_scope_are_skipped() {
        let text = "[[extension]]\nkind = \"share\"\nname = \"Save\"\n\n[[extension_point]]\nidentifier = \"com.example.drive.x\"\nscope = \"nobody\"\n";
        let d = parse("com.example.drive", text);
        assert!(d.extensions.is_empty());
        assert!(d.points.is_empty());
    }

    #[test]
    fn scan_reads_points_before_extensions_whichever_package_comes_first() {
        let host = "[[extension_point]]\nidentifier = \"com.example.notes.share\"\n";
        let ext = "[[extension]]\npoint = \"com.example.notes.share\"\nname = \"Clip\"\n";
        let s = scan([("com.other.clip", ext), ("com.example.notes", host)]);
        let found = s.registry.discover("com.example.notes.share");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].bundle_id, "com.other.clip");
        assert_eq!(s.owner("com.other.clip", "com.example.notes.share"), Some("com.other.clip"));
    }

    #[test]
    fn scan_declares_the_system_points_and_leaves_out_unknown_ones() {
        let unknown = "[[extension]]\npoint = \"com.nobody.point\"\n";
        let s = scan([("com.example.drive", DRIVE), ("com.example.stray", unknown)]);
        assert_eq!(s.registry.discover(FILE_PROVIDER_POINT).len(), 1);
        assert!(s.registry.matching(&["com.nobody.point"]).is_empty());
        assert!(s.owner("com.example.stray", "com.nobody.point").is_none());
    }

    #[test]
    fn a_same_developer_point_refuses_a_lookalike_package() {
        let host = "[[extension_point]]\nidentifier = \"com.example.notes.own\"\nscope = \"same-developer\"\n";
        let inside = "[[extension]]\npoint = \"com.example.notes.own\"\n";
        let s = scan([("com.example.notes", host), ("com.example.notes.pdf", inside), ("com.example.notesfoo", inside)]);
        let bundles: Vec<&str> = s.registry.discover("com.example.notes.own").iter().map(|i| i.bundle_id.as_str()).collect();
        assert_eq!(bundles, ["com.example.notes.pdf"]);
    }

    #[test]
    fn the_first_package_to_claim_a_bundle_wins() {
        let a = "[[extension]]\nbundle_id = \"com.example.drive.sync\"\npoint = \"is.bifrost.file-provider\"\nname = \"First\"\n";
        let b = "[[extension]]\nbundle_id = \"com.example.drive.sync\"\npoint = \"is.bifrost.file-provider\"\nname = \"Second\"\n";
        let s = scan([("com.example.drive", a), ("com.example.drive.sync", b)]);
        let found = s.registry.discover(FILE_PROVIDER_POINT);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "First");
        assert_eq!(s.owner("com.example.drive.sync", FILE_PROVIDER_POINT), Some("com.example.drive"));
    }
}
