//! Named image catalog with scale variants, as a pure model of the reference image-by-name lookup (`UIImage(named:)` and an
//! asset catalog's 1x, 2x and 3x sets). Each name maps to one file per display scale. `resolve` picks the file for the
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
