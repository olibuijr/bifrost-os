//! App extensions for apps: the extension points a host app declares and the extensions a package provides for them.
//!
//! The model is `bifrost_ui::extension` (points, identities, the registry that discovers enabled extensions) and the
//! process table is `bifrost_core::extension_process` (one process per extension, one in-memory connection per host).
//! Both are re-exported here. A package writes the text from [`manifest`] into its `manifest.toml`, so the shell can
//! list the extensions before any of them runs. The shell's `extension_host` reads those tables, starts an extension's
//! app binary in extension mode and calls it; the extension side is `bifrost_app::extension_service`. See
//! docs/parity/extensionfoundation.md.

pub use bifrost_core::extension_process::{ConnectionId, Key, ProcessId, ProcessTable, State as ProcessState};
pub use bifrost_ui::actionrequest::{ActionRequest, Outcome, RequestError};
pub use bifrost_ui::extension::{valid_bundle_id, Error, Identity, Point, Registry, Scope};
/// Reading the extension tables of installed manifests: `scan_manifests` builds the registry from `(app id, manifest
/// text)` pairs, `FILE_PROVIDER_POINT` is the system point of file providers.
pub use bifrost_ui::extension_manifest::{parse as parse_manifest, scan as scan_manifests, Declared, Owned, Scan, FILE_PROVIDER_POINT};

/// The `[[extension_point]]` and `[[extension]]` tables for a manifest: every point, then every identity, in order.
pub fn manifest(points: &[Point], identities: &[Identity]) -> String {
    let mut text: String = points.iter().map(Point::manifest_toml).collect::<Vec<_>>().join("\n");
    for identity in identities {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&identity.manifest_toml());
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_and_an_extension_resolve_through_the_registry() {
        let point = Point::new("com.example.notes", "com.example.notes.share", Scope::Any).unwrap();
        let mut registry = Registry::new();
        registry.declare(point.clone()).unwrap();
        registry.install(Identity::new("com.other.clip", "com.example.notes.share", "Clip").unwrap()).unwrap();

        let offered = registry.discover("com.example.notes.share");
        assert_eq!(offered.len(), 1);
        assert_eq!(offered[0].bundle_id, "com.other.clip");

        let mut table = ProcessTable::new();
        let key = Key::new(&offered[0].bundle_id, &offered[0].point);
        let process = table.acquire(&key);
        assert_eq!(table.state(process), Some(ProcessState::Launching));
    }

    #[test]
    fn manifest_lists_points_then_extensions() {
        let points = [Point::new("com.example.notes", "com.example.notes.share", Scope::SameDeveloper).unwrap()];
        let identities = [Identity::new("com.other.clip", "com.example.notes.share", "Clip").unwrap()];
        let text = manifest(&points, &identities);
        let point_at = text.find("[[extension_point]]").unwrap();
        let ext_at = text.find("[[extension]]").unwrap();
        assert!(point_at < ext_at);
        assert!(text.contains("scope = \"same-developer\"\n"));
        assert!(text.contains("bundle_id = \"com.other.clip\"\n"));
        assert_eq!(manifest(&[], &[]), "");
    }

    #[test]
    fn invalid_bundle_identifiers_are_refused_before_a_manifest_is_written() {
        assert_eq!(Identity::new("clip", "com.example.notes.share", "Clip").unwrap_err(), Error::InvalidBundleId);
        assert!(valid_bundle_id("com.other.clip"));
    }
}
