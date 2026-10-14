//! Bundle resource lookup: finds a named file in an app's package directory the way `Bundle.url(forResource:)`
//! does. Localized copies in `<lang>.lproj/` come before unlocalized files in the package, and the language list
//! runs from the most specific locale to `Base` and `en`. Only file existence is checked, and names cannot
//! step out of the package directory. The lookup order is pure; the file check is the only I/O.

use std::path::{Path, PathBuf};

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

/// One path rule of a link detail: a `/` pattern where `*` matches any run of characters. An excluded rule
/// stops the link from opening the app when it matches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkComponent {
    pub pattern: String,
    pub exclude: bool,
}

/// One entry of the domain association file: the app ids it names (`TEAMID.bundle.id` or a bare bundle id) and its
/// path rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkDetail {
    pub app_ids: Vec<String>,
    pub components: Vec<LinkComponent>,
}

/// The domain association file of one host: the file a host serves under its well-known path over https. Fetching it
/// is not part of this model.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct AssociationFile {
    pub details: Vec<LinkDetail>,
}

impl AssociationFile {
    /// Parses the file. It reads `applinks.details` with `appIDs` (or one `appID`) and `components` (objects with a
    /// `/` pattern and an optional `exclude`), and the older `paths` list where a leading `NOT ` excludes. Details
    /// without an app id, or with no usable rule, are dropped. None when the text is not JSON or has no `applinks`.
    pub fn parse(text: &str) -> Option<AssociationFile> {
        let root = bifrost_core::json::parse(text).ok()?;
        let details = root.get("applinks")?.get("details")?.as_array()?;
        let details = details
            .iter()
            .filter_map(|d| {
                let mut app_ids: Vec<String> = d
                    .get("appIDs")
                    .and_then(Json::as_array)
                    .map(|a| a.iter().filter_map(Json::as_str).map(str::to_string).collect())
                    .unwrap_or_default();
                app_ids.extend(d.str_field("appID"));
                app_ids.retain(|a| !a.is_empty());
                if app_ids.is_empty() {
                    return None;
                }
                let mut components: Vec<LinkComponent> = d
                    .get("components")
                    .and_then(Json::as_array)
                    .map(|list| {
                        list.iter()
                            .filter_map(|c| {
                                let pattern = c.str_field("/")?;
                                let exclude = matches!(c.get("exclude"), Some(Json::Bool(true)));
                                // Query and fragment rules are not evaluated. An include rule that depends on them
                                // could match too much, so it is dropped; an exclude rule is kept as a path rule so
                                // that doubt keeps the link in the browser.
                                let constrained = c.get("?").is_some() || c.get("#").is_some();
                                (exclude || !constrained).then_some(LinkComponent { pattern, exclude })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(paths) = d.get("paths").and_then(Json::as_array) {
                    for p in paths.iter().filter_map(Json::as_str) {
                        let (exclude, pattern) = match p.strip_prefix("NOT ") {
                            Some(rest) => (true, rest),
                            None => (false, p),
                        };
                        components.push(LinkComponent { pattern: pattern.trim().to_string(), exclude });
                    }
                }
                (!components.is_empty()).then_some(LinkDetail { app_ids, components })
            })
            .collect();
        Some(AssociationFile { details })
    }

    /// The installed app that an https link with `path` opens, or None. Details are tried in file order; within a
    /// detail the first matching rule decides (an excluded match means no). `installed` lists the bundle ids of the
    /// installed apps; the caller has already checked that the host is the one this file was served from.
    pub fn route(&self, path: &str, installed: &[&str]) -> Option<String> {
        for d in &self.details {
            let Some(app) = installed.iter().find(|id| d.app_ids.iter().any(|entry| app_id_matches(entry, id))) else {
                continue;
            };
            let opens = d.components.iter().find(|c| glob(c.pattern.as_bytes(), path.as_bytes())).is_some_and(|c| !c.exclude);
            if opens {
                return Some(app.to_string());
            }
        }
        None
    }
}

/// True when an `appIDs` entry names the bundle id: the entry is the bare id, or a team id (ten letters or digits)
/// followed by a dot and the id.
fn app_id_matches(entry: &str, bundle_id: &str) -> bool {
    if entry == bundle_id {
        return true;
    }
    match entry.split_once('.') {
        Some((team, rest)) => team.len() == 10 && team.chars().all(|c| c.is_ascii_alphanumeric()) && rest == bundle_id,
        None => false,
    }
}

/// Glob match where `*` stands for any run of bytes (including none), `?` for exactly one byte and every other byte
/// matches itself. Linear in the pattern length times the text length, with no recursion, because both come from a
/// file a remote host serves.
fn glob(pattern: &[u8], s: &[u8]) -> bool {
    let (mut p, mut i) = (0, 0);
    // Where the last `*` was seen and the text position it currently swallows up to.
    let mut star: Option<(usize, usize)> = None;
    while i < s.len() {
        match pattern.get(p) {
            Some(b'*') => {
                star = Some((p, i));
                p += 1;
            }
            Some(&c) if c == b'?' || c == s[i] => {
                p += 1;
                i += 1;
            }
            _ => match star {
                Some((sp, si)) => {
                    p = sp + 1;
                    i = si + 1;
                    star = Some((sp, si + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == b'*')
}

/// Splits an https URL into its lowercased host and its path (`/` when empty; the query, fragment, port and user info
/// are dropped). None for other schemes and for a URL without a host. Shares the address parsing of `AppLinks`.
pub fn split_https(url: &str) -> Option<(String, String)> {
    let (scheme, rest) = url.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let normalized = format!("https://{rest}");
    let (host, path) = bifrost_core::app_links::split_address(&normalized)?;
    Some((host.to_ascii_lowercase(), path.to_string()))
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

    #[test]
    fn association_file_lists_app_ids_and_path_rules() {
        let f = AssociationFile::parse(r#"{"applinks":{"details":[{"appIDs":["ABCDE12345.is.olibuijr.notes"],"components":[{"/":"/share/*"},{"/":"/admin/*","exclude":true}]},{"appID":"ABCDE12345.is.olibuijr.tv","paths":["/watch/*","NOT /watch/private"]},{"components":[{"/":"/x"}]}]}}"#).unwrap();
        assert_eq!(f.details.len(), 2, "a detail without an app id is dropped");
        assert_eq!(f.details[0].components[1], LinkComponent { pattern: "/admin/*".into(), exclude: true });
        assert_eq!(f.details[1].app_ids, ["ABCDE12345.is.olibuijr.tv"]);
        assert_eq!(f.details[1].components.iter().map(|c| c.exclude).collect::<Vec<_>>(), [false, true]);
        assert_eq!(AssociationFile::parse("not json"), None);
        assert_eq!(AssociationFile::parse(r#"{"webcredentials":{}}"#), None);
    }

    #[test]
    fn links_route_to_the_installed_app_that_claims_them() {
        let f = AssociationFile::parse(r#"{"applinks":{"details":[{"appIDs":["ABCDE12345.is.olibuijr.notes"],"components":[{"/":"/share/private/*","exclude":true},{"/":"/share/*"}]},{"appIDs":["ABCDE12345.is.olibuijr.tv"],"components":[{"/":"*"}]}]}}"#).unwrap();
        let installed = ["is.olibuijr.tv", "is.olibuijr.notes"];
        assert_eq!(f.route("/share/a/b", &installed), Some("is.olibuijr.notes".into()));
        assert_eq!(f.route("/share/private/x", &installed), Some("is.olibuijr.tv".into()), "the excluded rule passes it on to the next detail");
        assert_eq!(f.route("/other", &installed), Some("is.olibuijr.tv".into()));
        assert_eq!(f.route("/other", &["is.olibuijr.notes"]), None, "no installed app claims the path");
    }

    #[test]
    fn a_bare_bundle_id_or_a_team_id_both_match_and_nothing_else() {
        assert!(app_id_matches("is.olibuijr.notes", "is.olibuijr.notes"));
        assert!(app_id_matches("ABCDE12345.is.olibuijr.notes", "is.olibuijr.notes"));
        assert!(!app_id_matches("is.olibuijr.notes", "olibuijr.notes"), "a short prefix is not a team id");
        assert!(!app_id_matches("ABCDE12345.is.olibuijr.notes", "is.olibuijr.note"));
    }

    #[test]
    fn glob_star_matches_any_run_and_nothing_else() {
        assert!(glob(b"/a/*/c", b"/a/b/c"));
        assert!(glob(b"/a/*/c", b"/a//c"));
        assert!(glob(b"*", b""));
        assert!(!glob(b"/a/*", b"/b/x"));
        assert!(!glob(b"/a", b"/a/"));
        assert!(glob(b"/*.html", b"/x/y.html"));
        assert!(glob(b"/a/?", b"/a/b"));
        assert!(!glob(b"/a/?", b"/a/"));
        assert!(!glob(b"/a/?", b"/a/bc"));
        assert!(glob(b"/a*b*c", b"/aXbYbZc"));
        assert!(!glob(b"/a*b*c", b"/aXbYbZ"));
        assert!(glob(b"/a**", b"/a"));
    }

    #[test]
    fn glob_with_many_stars_does_not_blow_up() {
        let pattern = [b"/".as_slice(), &b"*a".repeat(200), b"b"].concat();
        let text = [b"/".as_slice(), &b"a".repeat(2000)].concat();
        assert!(!glob(&pattern, &text));
    }

    #[test]
    fn query_dependent_rules_are_not_treated_as_path_rules() {
        let f = AssociationFile::parse(r#"{"applinks":{"details":[{"appIDs":["is.olibuijr.notes"],"components":[{"/":"/a","?":{"x":"y"}},{"/":"/b","?":{"x":"y"},"exclude":true},{"/":"/*"}]}]}}"#).unwrap();
        let patterns: Vec<_> = f.details[0].components.iter().map(|c| (c.pattern.as_str(), c.exclude)).collect();
        assert_eq!(patterns, [("/b", true), ("/*", false)], "an include rule that needs a query is dropped, an exclude rule is kept");
        assert_eq!(f.route("/b", &["is.olibuijr.notes"]), None);
        assert_eq!(f.route("/c", &["is.olibuijr.notes"]), Some("is.olibuijr.notes".into()));
    }

    #[test]
    fn non_string_app_ids_and_empty_details_are_ignored() {
        let f = AssociationFile::parse(r#"{"applinks":{"details":[{"appIDs":[1,"","is.olibuijr.a"],"components":[]},{"appIDs":["is.olibuijr.b"],"components":[{"/":"/x"}]}]}}"#).unwrap();
        assert_eq!(f.details.len(), 1, "a detail with no rules is dropped");
        assert_eq!(f.details[0].app_ids, ["is.olibuijr.b"]);
        assert_eq!(AssociationFile::parse(r#"{"applinks":{"details":{}}}"#), None);
    }

    #[test]
    fn https_urls_split_into_host_and_path() {
        assert_eq!(split_https("https://Example.COM/share/a?x=1#frag"), Some(("example.com".into(), "/share/a".into())));
        assert_eq!(split_https("https://example.com"), Some(("example.com".into(), "/".into())));
        assert_eq!(split_https("https://example.com?q=1"), Some(("example.com".into(), "/".into())));
        assert_eq!(split_https("http://example.com/a"), None);
        assert_eq!(split_https("https:///a"), None);
        assert_eq!(split_https("HTTPS://Example.com:8443/a"), Some(("example.com".into(), "/a".into())));
        assert_eq!(split_https("https://example.com:x@evil.example/a"), Some(("evil.example".into(), "/a".into())));
        assert_eq!(split_https("mailto:a@b.c"), None);
    }
}
