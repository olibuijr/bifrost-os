//! Bundle resource lookup: finds a named file in an app's package directory the way `Bundle.url(forResource:)`
//! does. Localized copies in `<lang>.lproj/` come before unlocalized files in the package, and the language list
//! runs from the most specific locale to `Base` and `en`. Only file existence is checked, and names cannot
//! step out of the package directory. The lookup order is pure; the file check is the only I/O.

use std::path::{Path, PathBuf};

use bifrost_core::json::Json;

use bifrost_core::json::Json;

use crate::locale::Locale;

/// `CFBundleTypeRole` of a URL type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BundleTypeRole {
    None,
    Viewer,
    Editor,
    Shell,
}

/// `LSHandlerRank`, lowest to highest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HandlerRank {
    None,
    Alternate,
    Default,
    Owner,
}

/// One `CFBundleURLTypes` entry: the schemes an app declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct URLType {
    pub url_name: String,
    pub url_schemes: Vec<String>,
    pub type_role: BundleTypeRole,
    pub handler_rank: HandlerRank,
}

impl URLType {
    /// Parses the `CFBundleURLTypes` array. Entries without schemes are dropped; schemes are lowercased.
    pub fn parse_all(types: &Json) -> Vec<URLType> {
        let Some(items) = types.as_array() else { return vec![] };
        items
            .iter()
            .filter_map(|d| {
                let schemes: Vec<String> = d
                    .get("CFBundleURLSchemes")
                    .and_then(Json::as_array)
                    .map(|a| a.iter().filter_map(Json::as_str).filter(|s| !s.is_empty()).map(str::to_ascii_lowercase).collect())
                    .unwrap_or_default();
                if schemes.is_empty() {
                    return None;
                }
                let role = match d.str_field("CFBundleTypeRole").as_deref() {
                    Some("Viewer") => BundleTypeRole::Viewer,
                    Some("Editor") => BundleTypeRole::Editor,
                    Some("Shell") => BundleTypeRole::Shell,
                    _ => BundleTypeRole::None,
                };
                let rank = match d.str_field("LSHandlerRank").as_deref() {
                    Some("Owner") => HandlerRank::Owner,
                    Some("Alternate") => HandlerRank::Alternate,
                    Some("None") => HandlerRank::None,
                    _ => HandlerRank::Default,
                };
                Some(URLType { url_name: d.str_field("CFBundleURLName").unwrap_or_default(), url_schemes: schemes, type_role: role, handler_rank: rank })
            })
            .collect()
    }
}

/// The scheme of a URL string (`myapp://x` gives `myapp`), lowercased; `None` when it has no valid scheme.
pub fn url_scheme(url: &str) -> Option<String> {
    let (s, _) = url.split_once(':')?;
    let ok = s.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) && s.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c));
    ok.then(|| s.to_ascii_lowercase())
}

/// `CFBundleTypeRole` of a URL type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BundleTypeRole {
    None,
    Viewer,
    Editor,
    Shell,
}

/// `LSHandlerRank`, lowest to highest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HandlerRank {
    None,
    Alternate,
    Default,
    Owner,
}

/// One `CFBundleURLTypes` entry: the schemes an app declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct URLType {
    pub url_name: String,
    pub url_schemes: Vec<String>,
    pub type_role: BundleTypeRole,
    pub handler_rank: HandlerRank,
}

impl URLType {
    /// Parses the `CFBundleURLTypes` array. Entries without schemes are dropped; schemes are lowercased.
    pub fn parse_all(types: &Json) -> Vec<URLType> {
        let Some(items) = types.as_array() else { return vec![] };
        items
            .iter()
            .filter_map(|d| {
                let schemes: Vec<String> = d
                    .get("CFBundleURLSchemes")
                    .and_then(Json::as_array)
                    .map(|a| a.iter().filter_map(Json::as_str).filter(|s| !s.is_empty()).map(str::to_ascii_lowercase).collect())
                    .unwrap_or_default();
                if schemes.is_empty() {
                    return None;
                }
                let role = match d.str_field("CFBundleTypeRole").as_deref() {
                    Some("Viewer") => BundleTypeRole::Viewer,
                    Some("Editor") => BundleTypeRole::Editor,
                    Some("Shell") => BundleTypeRole::Shell,
                    _ => BundleTypeRole::None,
                };
                let rank = match d.str_field("LSHandlerRank").as_deref() {
                    Some("Owner") => HandlerRank::Owner,
                    Some("Alternate") => HandlerRank::Alternate,
                    Some("None") => HandlerRank::None,
                    _ => HandlerRank::Default,
                };
                Some(URLType { url_name: d.str_field("CFBundleURLName").unwrap_or_default(), url_schemes: schemes, type_role: role, handler_rank: rank })
            })
            .collect()
    }
}

/// The scheme of a URL string (`myapp://x` gives `myapp`), lowercased; `None` when it has no valid scheme.
pub fn url_scheme(url: &str) -> Option<String> {
    let (s, _) = url.split_once(':')?;
    let ok = s.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) && s.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c));
    ok.then(|| s.to_ascii_lowercase())
}

/// A package directory and the language directories to try, most specific first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceBundle {
    root: PathBuf,
    languages: Vec<String>,
}

/// `Base` is the development-language directory; `en` is the product fallback, as in `locale::Bundle::load`.
fn language_chain(locale: &Locale) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: String| {
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    };
    push(locale.identifier());
    push(locale.language.clone());
    push("Base".into());
    push("en".into());
    out
}

/// A name is one file name, or a relative path inside the package. Parent steps and absolute paths are refused.
fn safe_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('/') && !name.split(['/', '\\']).any(|seg| seg == ".." || seg.is_empty())
}

impl ResourceBundle {
    pub fn new(root: impl Into<PathBuf>, locale: &Locale) -> Self {
        ResourceBundle { root: root.into(), languages: language_chain(locale) }
    }

    /// The languages searched, in order. `ja_JP` gives `ja_JP`, `ja`, `Base`, `en`.
    pub fn languages(&self) -> &[String] {
        &self.languages
    }

    /// Every path that could hold the resource, in search order. Empty for an unsafe name.
    ///
    /// `ext` may be empty, in which case `name` is the whole file name. `subdir` is a directory inside the
    /// package such as `Images`; an empty string means the package root.
    pub fn candidates(&self, name: &str, ext: &str, subdir: &str) -> Vec<PathBuf> {
        if !safe_name(name) || !(ext.is_empty() || safe_name(ext)) || !(subdir.is_empty() || safe_name(subdir)) {
            return vec![];
        }
        let file = if ext.is_empty() { name.to_string() } else { format!("{name}.{ext}") };
        let base = if subdir.is_empty() { self.root.clone() } else { self.root.join(subdir) };
        let mut out: Vec<PathBuf> = self.languages.iter().map(|l| base.join(format!("{l}.lproj")).join(&file)).collect();
        out.push(base.join(&file));
        out
    }

    /// The first candidate for which `exists` is true.
    pub fn find_with<F: Fn(&Path) -> bool>(&self, name: &str, ext: &str, subdir: &str, exists: F) -> Option<PathBuf> {
        self.candidates(name, ext, subdir).into_iter().find(|p| exists(p))
    }

    /// The first existing file for a resource name, checked on the file system.
    pub fn find(&self, name: &str, ext: &str, subdir: &str) -> Option<PathBuf> {
        self.find_with(name, ext, subdir, |p| p.is_file())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn types(text: &str) -> Vec<URLType> {
        URLType::parse_all(&bifrost_core::json::parse(text).unwrap())
    }

    #[test]
    fn url_types_parse_schemes_role_and_rank() {
        let t = types(r#"[{"CFBundleURLName":"com.x.cb","CFBundleURLSchemes":["MyApp","x-cb"],"CFBundleTypeRole":"Editor","LSHandlerRank":"Owner"},{"CFBundleURLSchemes":[]},{"CFBundleURLSchemes":["plain"]}]"#);
        assert_eq!(t.len(), 2, "an entry without schemes is dropped");
        assert_eq!(t[0].url_schemes, ["myapp", "x-cb"]);
        assert_eq!((t[0].type_role, t[0].handler_rank), (BundleTypeRole::Editor, HandlerRank::Owner));
        assert_eq!((t[1].type_role, t[1].handler_rank), (BundleTypeRole::None, HandlerRank::Default));
        assert!(HandlerRank::Owner > HandlerRank::Alternate);
        assert!(URLType::parse_all(&Json::Null).is_empty());
    }

    #[test]
    fn url_scheme_reads_the_prefix() {
        assert_eq!(url_scheme("MyApp://x"), Some("myapp".into()));
        assert_eq!(url_scheme("mailto:a@b.c"), Some("mailto".into()));
        assert_eq!(url_scheme("no scheme"), None);
        assert_eq!(url_scheme("1a://x"), None);
    }

    fn types(text: &str) -> Vec<URLType> {
        URLType::parse_all(&bifrost_core::json::parse(text).unwrap())
    }

    #[test]
    fn url_types_parse_schemes_role_and_rank() {
        let t = types(r#"[{"CFBundleURLName":"com.x.cb","CFBundleURLSchemes":["MyApp","x-cb"],"CFBundleTypeRole":"Editor","LSHandlerRank":"Owner"},{"CFBundleURLSchemes":[]},{"CFBundleURLSchemes":["plain"]}]"#);
        assert_eq!(t.len(), 2, "an entry without schemes is dropped");
        assert_eq!(t[0].url_schemes, ["myapp", "x-cb"]);
        assert_eq!((t[0].type_role, t[0].handler_rank), (BundleTypeRole::Editor, HandlerRank::Owner));
        assert_eq!((t[1].type_role, t[1].handler_rank), (BundleTypeRole::None, HandlerRank::Default));
        assert!(HandlerRank::Owner > HandlerRank::Alternate);
        assert!(URLType::parse_all(&Json::Null).is_empty());
    }

    #[test]
    fn url_scheme_reads_the_prefix() {
        assert_eq!(url_scheme("MyApp://x"), Some("myapp".into()));
        assert_eq!(url_scheme("mailto:a@b.c"), Some("mailto".into()));
        assert_eq!(url_scheme("no scheme"), None);
        assert_eq!(url_scheme("1a://x"), None);
    }

    fn bundle(lang: &str) -> ResourceBundle {
        ResourceBundle::new("/pkg", &Locale::new(lang))
    }

    #[test]
    fn language_chain_runs_from_region_to_fallbacks() {
        let b = bundle("is_IS.UTF-8");
        assert_eq!(b.languages(), &["is_IS".to_string(), "is".to_string(), "Base".to_string(), "en".to_string()]);
        // A plain language has no region step and does not repeat itself.
        let e = bundle("en");
        assert_eq!(e.languages(), &["en".to_string(), "Base".to_string()]);
    }

    #[test]
    fn candidates_put_localized_copies_before_package_files() {
        let b = bundle("is_IS");
        let got = b.candidates("Logo", "png", "Images");
        let want: Vec<PathBuf> = vec![
            "/pkg/Images/is_IS.lproj/Logo.png",
            "/pkg/Images/is.lproj/Logo.png",
            "/pkg/Images/Base.lproj/Logo.png",
            "/pkg/Images/en.lproj/Logo.png",
            "/pkg/Images/Logo.png",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn empty_extension_and_subdirectory_mean_whole_name_at_root() {
        let b = bundle("en");
        let got = b.candidates("LICENSE", "", "");
        assert_eq!(got.last().unwrap(), &PathBuf::from("/pkg/LICENSE"));
        assert_eq!(got.len(), b.languages().len() + 1);
    }

    #[test]
    fn unsafe_names_produce_no_candidates() {
        let b = bundle("en");
        assert!(b.candidates("../secret", "txt", "").is_empty());
        assert!(b.candidates("/etc/passwd", "", "").is_empty());
        assert!(b.candidates("a", "png", "Images/../..").is_empty());
        assert!(b.candidates("", "png", "").is_empty());
        assert!(b.candidates("ok", "", "a//b").is_empty());
    }

    #[test]
    fn find_with_returns_the_first_existing_candidate() {
        let b = bundle("is_IS");
        // Only the English copy exists: the lookup must skip the Icelandic candidates first.
        let hit = b.find_with("Help", "html", "", |p| p == Path::new("/pkg/en.lproj/Help.html"));
        assert_eq!(hit, Some(PathBuf::from("/pkg/en.lproj/Help.html")));
        // Nothing exists: no result.
        let none: HashSet<PathBuf> = HashSet::new();
        assert_eq!(b.find_with("Help", "html", "", |p| none.contains(p)), None);
        // The unlocalized package file is the last resort.
        let root = b.find_with("Help", "html", "", |p| p == Path::new("/pkg/Help.html"));
        assert_eq!(root, Some(PathBuf::from("/pkg/Help.html")));
    }
}
