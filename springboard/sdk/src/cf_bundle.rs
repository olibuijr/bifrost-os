//! Core Foundation bundles and plug-ins: `CFBundleCreate`, `CFBundleGetValueForInfoDictionaryKey` and the
//! `CFPlugIn` factory registry. The info dictionary comes from the top-level keys of the package's `manifest.toml`
//! (`CFBundleName` -> `name`, `CFBundleIdentifier` -> `id`, `CFBundleShortVersionString` -> `version`,
//! `CFBundleExecutable` -> `exec`); keys are also readable under their manifest spelling. Resource lookup defers to
//! `bundle_resources`.

use std::any::Any;
use std::path::{Path, PathBuf};

use crate::bundle_resources::ResourceBundle;
use crate::locale::Locale;

/// Info dictionary key to manifest key.
const KEY_MAP: &[(&str, &str)] = &[
    ("CFBundleIdentifier", "id"),
    ("CFBundleName", "name"),
    ("CFBundleShortVersionString", "version"),
    ("CFBundleExecutable", "exec"),
    ("CFBundleIconFile", "icon"),
    ("LSApplicationCategoryType", "category"),
    ("LSMinimumSystemVersion", "min_shell"),
];

#[derive(Clone, Debug)]
pub struct CfBundle {
    root: PathBuf,
    info: Vec<(String, String)>,
}

/// Reads `key = value` lines before the first table header. Strings lose their quotes; other values stay raw.
fn parse_top_level(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            break;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim();
        let v = match v.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
            Some(s) => s.replace("\\\"", "\"").replace("\\\\", "\\"),
            None => v.to_string(),
        };
        out.push((k.trim().to_string(), v));
    }
    out
}

impl CfBundle {
    /// `CFBundleCreate`: None when the directory has no readable `manifest.toml`.
    pub fn create(root: impl AsRef<Path>) -> Option<CfBundle> {
        let root = root.as_ref().to_path_buf();
        let text = std::fs::read_to_string(root.join("manifest.toml")).ok()?;
        Some(CfBundle { root, info: parse_top_level(&text) })
    }

    /// `CFBundleCopyBundleURL`.
    pub fn bundle_url(&self) -> &Path {
        &self.root
    }

    /// `CFBundleGetInfoDictionary`: manifest keys and values.
    pub fn info_dictionary(&self) -> &[(String, String)] {
        &self.info
    }

    /// `CFBundleGetValueForInfoDictionaryKey`.
    pub fn value_for_info_dictionary_key(&self, key: &str) -> Option<&str> {
        let mapped = KEY_MAP.iter().find(|(cf, _)| *cf == key).map_or(key, |(_, m)| m);
        self.info.iter().find(|(k, _)| k == mapped).map(|(_, v)| v.as_str())
    }

    /// `CFBundleCopyResourceURL`.
    pub fn resource_url(&self, name: &str, ext: &str, subdir: &str, locale: &Locale) -> Option<PathBuf> {
        ResourceBundle::new(self.root.clone(), locale).find(name, ext, subdir)
    }
}

/// A plug-in factory (`CFPlugInFactoryFunction`): builds one instance.
pub type Factory = fn() -> Box<dyn Any + Send>;

/// The factory table (`CFPlugInRegisterFactories`, `CFPlugInInstanceCreate`). Factories are keyed by an id string
/// (the factory UUID) and serve one or more type ids.
#[derive(Default)]
pub struct PlugInRegistry {
    factories: Vec<(String, Factory)>,
    types: Vec<(String, String)>,
}

impl PlugInRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// `CFPlugInRegisterFactoryFunction`.
    pub fn register_factory(&mut self, factory_id: &str, factory: Factory) {
        self.factories.retain(|(id, _)| id != factory_id);
        self.factories.push((factory_id.to_string(), factory));
    }

    /// `CFPlugInRegisterPlugInType`: factory `factory_id` creates instances of `type_id`.
    pub fn register_type(&mut self, type_id: &str, factory_id: &str) -> bool {
        if !self.factories.iter().any(|(id, _)| id == factory_id) {
            return false;
        }
        self.types.push((type_id.to_string(), factory_id.to_string()));
        true
    }

    /// `CFPlugInUnregisterFactory`.
    pub fn unregister_factory(&mut self, factory_id: &str) {
        self.factories.retain(|(id, _)| id != factory_id);
        self.types.retain(|(_, f)| f != factory_id);
    }

    /// `CFPlugInInstanceCreate`: an instance from the first factory registered for `type_id`.
    pub fn create_instance(&self, type_id: &str) -> Option<Box<dyn Any + Send>> {
        let (_, fid) = self.types.iter().find(|(t, _)| t == type_id)?;
        let (_, f) = self.factories.iter().find(|(id, _)| id == fid)?;
        Some(f())
    }

    /// `CFPlugInFindFactoriesForPlugInType`.
    pub fn factories_for_type(&self, type_id: &str) -> Vec<&str> {
        self.types.iter().filter(|(t, _)| t == type_id).map(|(_, f)| f.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(manifest: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("manifest.toml"), manifest).unwrap();
        d
    }

    #[test]
    fn info_dictionary_reads_top_level_keys_only() {
        let d = pkg("id = \"is.x.app\"\nname = \"X\"\nversion = \"1.2\"\n[[intent]]\nid = \"nested\"\n");
        let b = CfBundle::create(d.path()).unwrap();
        assert_eq!(b.value_for_info_dictionary_key("CFBundleIdentifier"), Some("is.x.app"));
        assert_eq!(b.value_for_info_dictionary_key("CFBundleName"), Some("X"));
        assert_eq!(b.value_for_info_dictionary_key("version"), Some("1.2"));
        assert_eq!(b.info_dictionary().len(), 3);
        assert_eq!(b.value_for_info_dictionary_key("missing"), None);
    }

    #[test]
    fn create_fails_without_manifest() {
        let d = tempfile::tempdir().unwrap();
        assert!(CfBundle::create(d.path()).is_none());
    }

    #[test]
    fn resource_url_finds_a_file() {
        let d = pkg("id = \"a\"\n");
        std::fs::write(d.path().join("logo.png"), b"p").unwrap();
        let b = CfBundle::create(d.path()).unwrap();
        let loc = Locale::new("en");
        assert_eq!(b.resource_url("logo", "png", "", &loc), Some(d.path().join("logo.png")));
        assert_eq!(b.resource_url("nope", "png", "", &loc), None);
    }

    #[test]
    fn factory_creates_instance_for_type() {
        let mut r = PlugInRegistry::new();
        r.register_factory("F1", || Box::new(7u32));
        assert!(r.register_type("T", "F1"));
        assert!(!r.register_type("T2", "unknown"));
        let inst = r.create_instance("T").unwrap();
        assert_eq!(*inst.downcast::<u32>().unwrap(), 7);
        assert_eq!(r.factories_for_type("T"), vec!["F1"]);
        assert!(r.create_instance("T2").is_none());
    }

    #[test]
    fn unregister_drops_types() {
        let mut r = PlugInRegistry::new();
        r.register_factory("F1", || Box::new(1u8));
        r.register_type("T", "F1");
        r.unregister_factory("F1");
        assert!(r.create_instance("T").is_none());
    }
}
