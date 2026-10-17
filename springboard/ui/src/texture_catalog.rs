//! Named image catalog with scale variants: a pure model of an image lookup by name, with one file per display scale
//! (1x, 2x and 3x sets, as an asset catalog holds them). Each name maps to one file per display scale. `resolve` picks the file for the
//! screen's scale: the smallest variant whose scale is at least the requested one, so a picture is scaled down rather than
//! up, and the largest variant when every variant is smaller. No file is read here; the caller loads the path it gets back.

use std::collections::BTreeMap;

/// One file of a name, for one display scale.
#[derive(Clone, Debug, PartialEq)]
struct Variant {
    scale: f64,
    path: String,
}

/// The catalog: names, and the files each name has per scale.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextureCatalog {
    entries: BTreeMap<String, Vec<Variant>>,
}

impl TextureCatalog {
    /// An empty catalog.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the file for one scale of `name`. Returns false, and changes nothing, when the name or path is empty, the
    /// scale is not finite and positive, or that name already has a file at that scale.
    pub fn add(&mut self, name: &str, scale: f64, path: &str) -> bool {
        if name.is_empty() || path.is_empty() || !scale.is_finite() || scale <= 0.0 {
            return false;
        }
        let variants = self.entries.entry(name.to_string()).or_default();
        if variants.iter().any(|v| v.scale == scale) {
            return false;
        }
        let at = variants.partition_point(|v| v.scale < scale);
        variants.insert(at, Variant { scale, path: path.to_string() });
        true
    }

    /// The file for `name` at `scale`: the first variant whose scale is at least `scale`, or the largest variant when all
    /// are smaller. None when the name is unknown.
    pub fn resolve(&self, name: &str, scale: f64) -> Option<&str> {
        let variants = self.entries.get(name)?;
        variants
            .iter()
            .find(|v| v.scale >= scale)
            .or_else(|| variants.last())
            .map(|v| v.path.as_str())
    }

    /// The scales a name has files for, smallest first.
    pub fn scales(&self, name: &str) -> Vec<f64> {
        self.entries.get(name).map(|v| v.iter().map(|x| x.scale).collect()).unwrap_or_default()
    }

    /// Every name in the catalog, in alphabetical order.
    pub fn names(&self) -> Vec<&str> {
        self.entries.keys().map(|k| k.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icons() -> TextureCatalog {
        let mut c = TextureCatalog::new();
        assert!(c.add("folder", 1.0, "folder.png"));
        assert!(c.add("folder", 3.0, "folder@3x.png"));
        assert!(c.add("folder", 2.0, "folder@2x.png"));
        c
    }

    #[test]
    fn the_smallest_variant_that_is_big_enough_is_used() {
        let c = icons();
        assert_eq!(c.resolve("folder", 1.0), Some("folder.png"));
        assert_eq!(c.resolve("folder", 1.5), Some("folder@2x.png"));
        assert_eq!(c.resolve("folder", 2.0), Some("folder@2x.png"));
        assert_eq!(c.resolve("folder", 2.5), Some("folder@3x.png"));
    }

    #[test]
    fn a_scale_above_every_variant_uses_the_largest() {
        assert_eq!(icons().resolve("folder", 4.0), Some("folder@3x.png"));
    }

    #[test]
    fn an_unknown_name_resolves_to_nothing() {
        assert_eq!(icons().resolve("trash", 1.0), None);
        assert!(TextureCatalog::new().names().is_empty());
    }

    #[test]
    fn bad_entries_and_repeated_scales_are_refused() {
        let mut c = icons();
        assert!(!c.add("", 1.0, "x.png"));
        assert!(!c.add("x", 1.0, ""));
        assert!(!c.add("x", 0.0, "x.png"));
        assert!(!c.add("x", -1.0, "x.png"));
        assert!(!c.add("x", f64::NAN, "x.png"));
        assert!(!c.add("folder", 2.0, "other.png"));
        assert_eq!(c.resolve("folder", 2.0), Some("folder@2x.png"));
        assert!(c.resolve("x", 1.0).is_none());
    }

    #[test]
    fn scales_and_names_are_listed_in_order() {
        let mut c = icons();
        assert!(c.add("app", 1.0, "app.png"));
        assert_eq!(c.scales("folder"), vec![1.0, 2.0, 3.0]);
        assert!(c.scales("missing").is_empty());
        assert_eq!(c.names(), vec!["app", "folder"]);
    }
}
