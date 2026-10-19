//! Bundle information: the info property list of a package, with CFBundle-style keys.
//!
//! An [`InfoDictionary`] is read from `Info.plist` (XML, binary or the older text form) when the package has one,
//! and otherwise from the top-level keys of `manifest.toml`, which are renamed to their CFBundle spelling
//! (`id` becomes `CFBundleIdentifier`, `version` becomes `CFBundleShortVersionString` and `CFBundleVersion`, and so
//! on; see [`MANIFEST_KEYS`]). Keys with no CFBundle name keep their manifest spelling. Either way an app reads
//! the same keys: identity, version, executable, icon, category, minimum system version, URL types and document
//! types.
//!
//! Per-language values come from `<lang>.lproj/InfoPlist.strings`, searched most specific language first and then
//! the dictionary itself, so a name can be translated without a second plist ([`InfoDictionary::localized`]).
//!
//! [`InfoDictionary::validate`] lists what a package must fix before it installs: a missing or malformed
//! identifier, name, version or executable, a bad minimum system version, an unusable URL scheme.
//!
//! Limits: an info file larger than [`MAX_INFO_BYTES`] is refused, so a hostile package cannot make the reader
//! allocate without bound; the plist readers are depth-limited.
//!
//! ```ignore
//! let info = bundle_info::InfoDictionary::load(&package_dir)?;
//! let title = info.display_name(&package_dir, &Locale::new("is_IS"));
//! for problem in info.validate() { eprintln!("{problem}"); }
//! ```

use bifrost_core::plist::{self, Plist};
use std::collections::HashMap;
use std::path::Path;

use crate::bundle_resources::{url_scheme, BundleTypeRole, HandlerRank};
use crate::locale::{self, Locale};

/// The largest info file read, in bytes.
pub const MAX_INFO_BYTES: u64 = 1 << 20;

/// Manifest key to info key. A manifest key may feed more than one info key (`version` is both the short version and
/// the build version).
pub const MANIFEST_KEYS: &[(&str, &[&str])] = &[
    ("id", &["CFBundleIdentifier"]),
    ("name", &["CFBundleName", "CFBundleDisplayName"]),
    ("version", &["CFBundleShortVersionString", "CFBundleVersion"]),
    ("exec", &["CFBundleExecutable"]),
    ("icon", &["CFBundleIconFile"]),
    ("category", &["LSApplicationCategoryType"]),
    ("min_shell", &["LSMinimumSystemVersion"]),
];

/// Why an info dictionary could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InfoError {
    /// Neither `Info.plist` nor `manifest.toml` exists, or the file could not be read.
    Missing,
    /// The file is larger than [`MAX_INFO_BYTES`].
    TooBig,
    /// The plist or the manifest text does not parse, or its top level is not a dictionary. The text is the parser's.
    Malformed(String),
}

impl std::fmt::Display for InfoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InfoError::Missing => f.write_str("the package has no Info.plist or manifest.toml"),
            InfoError::TooBig => f.write_str("the info file is too large"),
            InfoError::Malformed(why) => write!(f, "the info file is malformed: {why}"),
        }
    }
}

/// One thing wrong with an info dictionary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InfoProblem {
    /// A required key is absent or empty.
    Missing(&'static str),
    /// A key is present with a value that cannot be used.
    Invalid { key: &'static str, reason: &'static str },
}

impl std::fmt::Display for InfoProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InfoProblem::Missing(key) => write!(f, "{key} is missing"),
            InfoProblem::Invalid { key, reason } => write!(f, "{key} {reason}"),
        }
    }
}

/// A `CFBundleDocumentTypes` entry: what the app opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentType {
    pub name: String,
    pub role: BundleTypeRole,
    pub rank: HandlerRank,
    /// `LSItemContentTypes`: uniform type identifiers.
    pub content_types: Vec<String>,
    /// `CFBundleTypeExtensions`: lowercase, without the dot.
    pub extensions: Vec<String>,
    /// `CFBundleTypeMIMETypes`: lowercase.
    pub mime_types: Vec<String>,
}

impl DocumentType {
    /// Whether this type covers a file with `extension` (case-insensitive, dot optional).
    pub fn handles_extension(&self, extension: &str) -> bool {
        let ext = extension.trim_start_matches('.').to_ascii_lowercase();
        !ext.is_empty() && self.extensions.iter().any(|e| *e == ext)
    }

    /// Whether this type covers the uniform type identifier `uti` (exact match, case-insensitive).
    pub fn handles_content_type(&self, uti: &str) -> bool {
        self.content_types.iter().any(|t| t.eq_ignore_ascii_case(uti))
    }
}

/// The info dictionary of a package: top-level keys and values in file order.
#[derive(Clone, Debug, PartialEq)]
pub struct InfoDictionary {
    entries: Vec<(String, Plist)>,
}

fn strings_of(value: Option<&Plist>) -> Vec<String> {
    match value {
        Some(Plist::Array(items)) => items.iter().filter_map(|i| if let Plist::Str(s) = i { Some(s.clone()) } else { None }).collect(),
        _ => Vec::new(),
    }
}

fn dict_get<'a>(entries: &'a [(String, Plist)], key: &str) -> Option<&'a Plist> {
    entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn role_of(text: Option<&str>) -> BundleTypeRole {
    match text {
        Some("Viewer") => BundleTypeRole::Viewer,
        Some("Editor") => BundleTypeRole::Editor,
        Some("Shell") => BundleTypeRole::Shell,
        _ => BundleTypeRole::None,
    }
}

fn rank_of(text: Option<&str>) -> HandlerRank {
    match text {
        Some("Owner") => HandlerRank::Owner,
        Some("Alternate") => HandlerRank::Alternate,
        Some("None") => HandlerRank::None,
        _ => HandlerRank::Default,
    }
}

/// Reads the top level of a manifest: `key = value` lines before the first table. A value is a quoted string, an
/// integer, `true` or `false`, or a one-line array of quoted strings. A line it cannot read is an error, so a typo is
/// not silently dropped.
fn parse_manifest_top(text: &str) -> Result<Vec<(String, Plist)>, InfoError> {
    let mut out: Vec<(String, Plist)> = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.starts_with('[') {
            break;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bad = |why: &str| InfoError::Malformed(format!("line {}: {why}", n + 1));
        let (key, value) = line.split_once('=').ok_or_else(|| bad("expected key = value"))?;
        let key = key.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(bad("bad key"));
        }
        let value = parse_manifest_value(value.trim()).ok_or_else(|| bad("unreadable value"))?;
        if out.iter().any(|(k, _)| k == key) {
            return Err(bad("key appears twice"));
        }
        out.push((key.to_string(), value));
    }
    Ok(out)
}

fn parse_manifest_value(text: &str) -> Option<Plist> {
    if let Some(s) = parse_quoted(text) {
        return Some(Plist::Str(s));
    }
    match text {
        "true" => return Some(Plist::Bool(true)),
        "false" => return Some(Plist::Bool(false)),
        _ => {}
    }
    if let Ok(n) = text.parse::<i64>() {
        return Some(Plist::Int(n));
    }
    let inner = text.strip_prefix('[')?.strip_suffix(']')?.trim();
    let mut items = Vec::new();
    let mut rest = inner;
    while !rest.is_empty() {
        let end = quoted_end(rest)?;
        items.push(Plist::Str(parse_quoted(&rest[..end])?));
        rest = rest[end..].trim_start();
        if let Some(after) = rest.strip_prefix(',') {
            rest = after.trim_start();
        } else if !rest.is_empty() {
            return None;
        }
    }
    Some(Plist::Array(items))
}

/// The byte length of the quoted string at the start of `text`, quotes included.
fn quoted_end(text: &str) -> Option<usize> {
    let mut chars = text.char_indices();
    if chars.next()?.1 != '"' {
        return None;
    }
    let mut escaped = false;
    for (i, c) in chars {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            return Some(i + 1);
        }
    }
    None
}

/// A whole quoted string with `\"`, `\\`, `\n` and `\t` escapes. `None` unless the text is exactly one string.
fn parse_quoted(text: &str) -> Option<String> {
    if quoted_end(text)? != text.len() {
        return None;
    }
    let inner = &text[1..text.len() - 1];
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                'n' => out.push('\n'),
                't' => out.push('\t'),
                _ => return None,
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// Dotted decimal numbers such as `1`, `1.2` or `0.7.25`: one to three parts, each digits only.
fn is_version(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    (1..=3).contains(&parts.len()) && parts.iter().all(|p| !p.is_empty() && p.len() <= 9 && p.chars().all(|c| c.is_ascii_digit()))
}

/// Reverse-DNS: two or more labels of letters, digits and hyphens, none empty or starting or ending with a hyphen.
fn is_identifier(text: &str) -> bool {
    let labels: Vec<&str> = text.split('.').collect();
    text.len() <= 255
        && labels.len() >= 2
        && labels.iter().all(|l| !l.is_empty() && !l.starts_with('-') && !l.ends_with('-') && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

impl InfoDictionary {
    /// A dictionary from key-value pairs.
    pub fn from_entries(entries: Vec<(String, Plist)>) -> InfoDictionary {
        InfoDictionary { entries }
    }

    /// Reads an `Info.plist` (binary, XML or text form). The top level must be a dictionary.
    pub fn from_plist_bytes(bytes: &[u8]) -> Result<InfoDictionary, InfoError> {
        if bytes.len() as u64 > MAX_INFO_BYTES {
            return Err(InfoError::TooBig);
        }
        match plist::parse_bytes(bytes) {
            Ok(Plist::Dict(entries)) => Ok(InfoDictionary { entries }),
            Ok(_) => Err(InfoError::Malformed("the top level is not a dictionary".to_string())),
            Err(why) => Err(InfoError::Malformed(why)),
        }
    }

    /// Reads the top-level keys of a `manifest.toml` and renames them as [`MANIFEST_KEYS`] says. A key the manifest
    /// already spells in CFBundle form is kept as written.
    pub fn from_manifest_toml(text: &str) -> Result<InfoDictionary, InfoError> {
        let top = parse_manifest_top(text)?;
        let mut entries: Vec<(String, Plist)> = Vec::new();
        for (key, value) in top {
            match MANIFEST_KEYS.iter().find(|(m, _)| *m == key) {
                Some((_, names)) => {
                    for name in *names {
                        if !entries.iter().any(|(k, _)| k == name) {
                            entries.push(((*name).to_string(), value.clone()));
                        }
                    }
                }
                None => {
                    if !entries.iter().any(|(k, _)| *k == key) {
                        entries.push((key, value));
                    }
                }
            }
        }
        Ok(InfoDictionary { entries })
    }

    /// Reads the info of the package in `dir`: `Info.plist` when there is one, else `manifest.toml`.
    pub fn load(dir: &Path) -> Result<InfoDictionary, InfoError> {
        for (name, is_plist) in [("Info.plist", true), ("manifest.toml", false)] {
            let path = dir.join(name);
            let Ok(meta) = std::fs::metadata(&path) else { continue };
            if !meta.is_file() {
                continue;
            }
            if meta.len() > MAX_INFO_BYTES {
                return Err(InfoError::TooBig);
            }
            let bytes = std::fs::read(&path).map_err(|_| InfoError::Missing)?;
            return if is_plist {
                InfoDictionary::from_plist_bytes(&bytes)
            } else {
                let text = String::from_utf8(bytes).map_err(|_| InfoError::Malformed("the manifest is not UTF-8".to_string()))?;
                InfoDictionary::from_manifest_toml(&text)
            };
        }
        Err(InfoError::Missing)
    }

    /// Every key and value, in file order.
    pub fn entries(&self) -> &[(String, Plist)] {
        &self.entries
    }

    /// The value of `key`.
    pub fn get(&self, key: &str) -> Option<&Plist> {
        dict_get(&self.entries, key)
    }

    /// The value of `key` when it is a string.
    pub fn string(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(Plist::Str(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn identifier(&self) -> Option<&str> {
        self.string("CFBundleIdentifier")
    }

    pub fn name(&self) -> Option<&str> {
        self.string("CFBundleName")
    }

    pub fn short_version(&self) -> Option<&str> {
        self.string("CFBundleShortVersionString")
    }

    pub fn build_version(&self) -> Option<&str> {
        self.string("CFBundleVersion")
    }

    pub fn executable(&self) -> Option<&str> {
        self.string("CFBundleExecutable")
    }

    pub fn icon_file(&self) -> Option<&str> {
        self.string("CFBundleIconFile")
    }

    pub fn category(&self) -> Option<&str> {
        self.string("LSApplicationCategoryType")
    }

    pub fn minimum_system_version(&self) -> Option<&str> {
        self.string("LSMinimumSystemVersion")
    }

    /// The value of `key` for `locale`: the `InfoPlist.strings` tables in `tables` (most specific first) before the
    /// dictionary itself.
    pub fn localized_with(&self, tables: &[HashMap<String, String>], key: &str) -> Option<String> {
        tables.iter().find_map(|t| t.get(key).cloned()).or_else(|| self.string(key).map(str::to_string))
    }

    /// Reads `<dir>/<lang>.lproj/InfoPlist.strings` for each language of `locale`, most specific first. A missing or
    /// unreadable file is skipped; a file larger than [`MAX_INFO_BYTES`] is skipped.
    pub fn info_strings(dir: &Path, locale: &Locale) -> Vec<HashMap<String, String>> {
        let mut tables = Vec::new();
        for lang in locale::lproj_candidates(locale) {
            let path = dir.join(format!("{lang}.lproj/InfoPlist.strings"));
            if std::fs::metadata(&path).map(|m| m.len() > MAX_INFO_BYTES).unwrap_or(true) {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&path) {
                tables.push(locale::parse_strings(&text));
            }
        }
        tables
    }

    /// The value of `key` for `locale`, from the package in `dir`.
    pub fn localized(&self, dir: &Path, locale: &Locale, key: &str) -> Option<String> {
        self.localized_with(&InfoDictionary::info_strings(dir, locale), key)
    }

    /// The name shown to people: the translated `CFBundleDisplayName`, else `CFBundleName`, else the identifier.
    pub fn display_name(&self, dir: &Path, locale: &Locale) -> Option<String> {
        let tables = InfoDictionary::info_strings(dir, locale);
        ["CFBundleDisplayName", "CFBundleName"]
            .iter()
            .find_map(|k| self.localized_with(&tables, k))
            .or_else(|| self.identifier().map(str::to_string))
    }

    /// The `CFBundleURLTypes` entries. An entry with no usable scheme is dropped; schemes are lowercased.
    pub fn url_types(&self) -> Vec<crate::bundle_resources::URLType> {
        let Some(Plist::Array(items)) = self.get("CFBundleURLTypes") else { return Vec::new() };
        items
            .iter()
            .filter_map(|item| {
                let Plist::Dict(d) = item else { return None };
                let schemes: Vec<String> = strings_of(dict_get(d, "CFBundleURLSchemes"))
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_ascii_lowercase())
                    .collect();
                if schemes.is_empty() {
                    return None;
                }
                let text = |k: &str| match dict_get(d, k) {
                    Some(Plist::Str(s)) => Some(s.as_str()),
                    _ => None,
                };
                Some(crate::bundle_resources::URLType {
                    url_name: text("CFBundleURLName").unwrap_or_default().to_string(),
                    url_schemes: schemes,
                    type_role: role_of(text("CFBundleTypeRole")),
                    handler_rank: rank_of(text("LSHandlerRank")),
                })
            })
            .collect()
    }

    /// The `CFBundleDocumentTypes` entries. An entry that names no extension, content type or MIME type is dropped.
    pub fn document_types(&self) -> Vec<DocumentType> {
        let Some(Plist::Array(items)) = self.get("CFBundleDocumentTypes") else { return Vec::new() };
        items
            .iter()
            .filter_map(|item| {
                let Plist::Dict(d) = item else { return None };
                let extensions: Vec<String> = strings_of(dict_get(d, "CFBundleTypeExtensions"))
                    .into_iter()
                    .map(|e| e.trim_start_matches('.').to_ascii_lowercase())
                    .filter(|e| !e.is_empty())
                    .collect();
                let content_types: Vec<String> = strings_of(dict_get(d, "LSItemContentTypes")).into_iter().filter(|t| !t.is_empty()).collect();
                let mime_types: Vec<String> = strings_of(dict_get(d, "CFBundleTypeMIMETypes"))
                    .into_iter()
                    .map(|m| m.to_ascii_lowercase())
                    .filter(|m| !m.is_empty())
                    .collect();
                if extensions.is_empty() && content_types.is_empty() && mime_types.is_empty() {
                    return None;
                }
                let text = |k: &str| match dict_get(d, k) {
                    Some(Plist::Str(s)) => Some(s.as_str()),
                    _ => None,
                };
                Some(DocumentType {
                    name: text("CFBundleTypeName").unwrap_or_default().to_string(),
                    role: role_of(text("CFBundleTypeRole")),
                    rank: rank_of(text("LSHandlerRank")),
                    content_types,
                    extensions,
                    mime_types,
                })
            })
            .collect()
    }

    /// The document type with the highest handler rank that covers `extension` (the first one wins a tie).
    pub fn document_type_for_extension(&self, extension: &str) -> Option<DocumentType> {
        let mut best: Option<DocumentType> = None;
        for t in self.document_types() {
            if t.handles_extension(extension) && best.as_ref().map_or(true, |b| t.rank > b.rank) {
                best = Some(t);
            }
        }
        best
    }

    /// Everything a package must fix. Empty means the info is usable.
    pub fn validate(&self) -> Vec<InfoProblem> {
        let mut out = Vec::new();
        match self.identifier() {
            None | Some("") => out.push(InfoProblem::Missing("CFBundleIdentifier")),
            Some(id) if !is_identifier(id) => out.push(InfoProblem::Invalid { key: "CFBundleIdentifier", reason: "is not a reverse-DNS name" }),
            Some(_) => {}
        }
        if self.name().map_or(true, str::is_empty) {
            out.push(InfoProblem::Missing("CFBundleName"));
        }
        match self.short_version() {
            None | Some("") => out.push(InfoProblem::Missing("CFBundleShortVersionString")),
            Some(v) if !is_version(v) => out.push(InfoProblem::Invalid { key: "CFBundleShortVersionString", reason: "is not one to three dotted numbers" }),
            Some(_) => {}
        }
        match self.executable() {
            None | Some("") => out.push(InfoProblem::Missing("CFBundleExecutable")),
            Some(e) if e.contains('/') || e.contains('\\') || e == "." || e == ".." || e.chars().any(|c| c.is_control()) => {
                out.push(InfoProblem::Invalid { key: "CFBundleExecutable", reason: "is not a plain file name" })
            }
            Some(_) => {}
        }
        if let Some(v) = self.minimum_system_version() {
            if !is_version(v) {
                out.push(InfoProblem::Invalid { key: "LSMinimumSystemVersion", reason: "is not one to three dotted numbers" });
            }
        }
        if let Some(Plist::Array(items)) = self.get("CFBundleURLTypes") {
            for item in items {
                let Plist::Dict(d) = item else {
                    out.push(InfoProblem::Invalid { key: "CFBundleURLTypes", reason: "has an entry that is not a dictionary" });
                    continue;
                };
                for scheme in strings_of(dict_get(d, "CFBundleURLSchemes")) {
                    let ok = url_scheme(&format!("{scheme}:x")).is_some_and(|s| s == scheme.to_ascii_lowercase());
                    if !ok {
                        out.push(InfoProblem::Invalid { key: "CFBundleURLTypes", reason: "has a scheme that is not valid" });
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = "# package\nid = \"is.olibuijr.notes\"\nname = \"Notes\"\nversion = \"1.4.2\"\nexec = \"notes\"\nicon = \"icon.png\"\ncategory = \"productivity\"\nmin_shell = \"0.7.25\"\npermissions = [\"documents\", \"keychain\"]\nwindow = \"full-size\"\n\n[extra]\nignored = 1\n";

    const PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>is.olibuijr.draw</string>
<key>CFBundleName</key><string>Draw</string>
<key>CFBundleShortVersionString</key><string>2.0</string>
<key>CFBundleExecutable</key><string>draw</string>
<key>CFBundleURLTypes</key><array><dict>
<key>CFBundleURLName</key><string>Draw links</string>
<key>CFBundleURLSchemes</key><array><string>Draw</string><string>x-draw</string></array>
<key>CFBundleTypeRole</key><string>Editor</string></dict>
<dict><key>CFBundleURLSchemes</key><array/></dict></array>
<key>CFBundleDocumentTypes</key><array>
<dict><key>CFBundleTypeName</key><string>Sketch</string><key>CFBundleTypeExtensions</key><array><string>.SKT</string><string>sketch</string></array>
<key>LSHandlerRank</key><string>Owner</string><key>CFBundleTypeRole</key><string>Editor</string></dict>
<dict><key>CFBundleTypeName</key><string>Any image</string><key>CFBundleTypeExtensions</key><array><string>skt</string></array>
<key>LSItemContentTypes</key><array><string>public.image</string></array><key>LSHandlerRank</key><string>Alternate</string></dict>
<dict><key>CFBundleTypeName</key><string>Nothing</string></dict>
</array></dict></plist>"#;

    #[test]
    fn manifest_keys_become_bundle_keys() {
        let info = InfoDictionary::from_manifest_toml(MANIFEST).unwrap();
        assert_eq!(info.identifier(), Some("is.olibuijr.notes"));
        assert_eq!(info.name(), Some("Notes"));
        assert_eq!(info.string("CFBundleDisplayName"), Some("Notes"));
        assert_eq!(info.short_version(), Some("1.4.2"));
        assert_eq!(info.build_version(), Some("1.4.2"));
        assert_eq!(info.executable(), Some("notes"));
        assert_eq!(info.icon_file(), Some("icon.png"));
        assert_eq!(info.category(), Some("productivity"));
        assert_eq!(info.minimum_system_version(), Some("0.7.25"));
        // a key with no bundle name keeps its manifest spelling, and arrays survive
        assert_eq!(info.string("window"), Some("full-size"));
        assert_eq!(info.get("permissions"), Some(&Plist::Array(vec![Plist::Str("documents".into()), Plist::Str("keychain".into())])));
        assert_eq!(info.get("ignored"), None, "tables after the top level are not read");
        assert!(info.validate().is_empty(), "{:?}", info.validate());
    }

    #[test]
    fn a_manifest_that_already_uses_bundle_keys_keeps_them() {
        let info = InfoDictionary::from_manifest_toml("CFBundleIdentifier = \"is.olibuijr.a\"\nid = \"is.olibuijr.b\"\n").unwrap();
        assert_eq!(info.identifier(), Some("is.olibuijr.a"));
    }

    #[test]
    fn manifest_values_parse_strings_numbers_and_booleans() {
        let info = InfoDictionary::from_manifest_toml("a = \"q\\\"uote\\\\\"\nb = 7\nc = true\nd = [\"x\", \"y,z\"]\ne = []\n").unwrap();
        assert_eq!(info.get("a"), Some(&Plist::Str("q\"uote\\".into())));
        assert_eq!(info.get("b"), Some(&Plist::Int(7)));
        assert_eq!(info.get("c"), Some(&Plist::Bool(true)));
        assert_eq!(info.get("d"), Some(&Plist::Array(vec![Plist::Str("x".into()), Plist::Str("y,z".into())])));
        assert_eq!(info.get("e"), Some(&Plist::Array(vec![])));
    }

    #[test]
    fn a_bad_manifest_line_is_an_error_not_a_dropped_key() {
        for bad in ["name Notes\n", "name = Notes\n", "name = \"open\n", "= \"x\"\n", "a = \"1\"\na = \"2\"\n", "k = [\"a\" \"b\"]\n", "k = \"a\\q\"\n", "bad key = 1\n"] {
            assert!(matches!(InfoDictionary::from_manifest_toml(bad), Err(InfoError::Malformed(_))), "{bad:?}");
        }
    }

    #[test]
    fn an_info_plist_reads_the_same_accessors_and_url_types() {
        let info = InfoDictionary::from_plist_bytes(PLIST.as_bytes()).unwrap();
        assert_eq!((info.identifier(), info.short_version()), (Some("is.olibuijr.draw"), Some("2.0")));
        let urls = info.url_types();
        assert_eq!(urls.len(), 1, "an entry with no scheme is dropped");
        assert_eq!(urls[0].url_schemes, ["draw", "x-draw"]);
        assert_eq!(urls[0].url_name, "Draw links");
        assert_eq!(urls[0].type_role, BundleTypeRole::Editor);
        assert_eq!(urls[0].handler_rank, HandlerRank::Default);
        assert!(info.validate().is_empty(), "{:?}", info.validate());
    }

    #[test]
    fn document_types_match_extensions_and_content_types_and_rank_wins() {
        let info = InfoDictionary::from_plist_bytes(PLIST.as_bytes()).unwrap();
        let types = info.document_types();
        assert_eq!(types.len(), 2, "a type that names nothing is dropped");
        assert_eq!(types[0].extensions, ["skt", "sketch"]);
        assert!(types[0].handles_extension(".SKT") && types[0].handles_extension("Sketch"));
        assert!(!types[0].handles_extension("") && !types[0].handles_extension("png"));
        assert!(types[1].handles_content_type("PUBLIC.IMAGE") && !types[1].handles_content_type("public.text"));
        // both types claim .skt; the Owner rank beats Alternate even though it is listed first here and last in other files
        assert_eq!(info.document_type_for_extension("skt").unwrap().name, "Sketch");
        assert_eq!(info.document_type_for_extension("txt"), None);
    }

    #[test]
    fn a_binary_plist_round_trips_through_the_reader() {
        let dict = Plist::Dict(vec![("CFBundleIdentifier".into(), Plist::Str("is.olibuijr.bin".into())), ("CFBundleName".into(), Plist::Str("Bin".into()))]);
        let bytes = bifrost_core::plist_binary::write(&dict).unwrap();
        assert!(bytes.starts_with(b"bplist"));
        let info = InfoDictionary::from_plist_bytes(&bytes).unwrap();
        assert_eq!(info.identifier(), Some("is.olibuijr.bin"));
    }

    #[test]
    fn hostile_info_files_are_refused() {
        assert!(matches!(InfoDictionary::from_plist_bytes(b"not a plist"), Err(InfoError::Malformed(_))));
        assert!(matches!(InfoDictionary::from_plist_bytes(b"<?xml version=\"1.0\"?><plist version=\"1.0\"><array/></plist>"), Err(InfoError::Malformed(_))));
        assert_eq!(InfoDictionary::from_plist_bytes(&vec![b' '; MAX_INFO_BYTES as usize + 1]), Err(InfoError::TooBig));
        assert!(matches!(InfoDictionary::from_plist_bytes(b"bplist00\x00\x00"), Err(InfoError::Malformed(_))));
        assert!(matches!(InfoDictionary::from_plist_bytes(b""), Err(InfoError::Malformed(_))));
        let nested = format!("<plist version=\"1.0\">{}{}</plist>", "<array>".repeat(50_000), "</array>".repeat(50_000));
        assert!(matches!(InfoDictionary::from_plist_bytes(nested.as_bytes()), Err(InfoError::Malformed(_))));
    }

    #[test]
    fn validation_names_each_problem() {
        let info = InfoDictionary::from_manifest_toml("id = \"nodots\"\nname = \"\"\nversion = \"1.x\"\nexec = \"../bin\"\nmin_shell = \"v2\"\n").unwrap();
        let problems = info.validate();
        assert!(problems.contains(&InfoProblem::Invalid { key: "CFBundleIdentifier", reason: "is not a reverse-DNS name" }));
        assert!(problems.contains(&InfoProblem::Missing("CFBundleName")));
        assert!(problems.contains(&InfoProblem::Invalid { key: "CFBundleShortVersionString", reason: "is not one to three dotted numbers" }));
        assert!(problems.contains(&InfoProblem::Invalid { key: "CFBundleExecutable", reason: "is not a plain file name" }));
        assert!(problems.contains(&InfoProblem::Invalid { key: "LSMinimumSystemVersion", reason: "is not one to three dotted numbers" }));
        assert_eq!(InfoDictionary::from_entries(vec![]).validate().len(), 4, "identifier, name, version and executable are required");
        assert_eq!(InfoProblem::Missing("CFBundleName").to_string(), "CFBundleName is missing");
    }

    #[test]
    fn identifiers_and_versions_follow_the_rules() {
        for ok in ["is.olibuijr.app", "a.b", "com.x-y.z9"] {
            assert!(is_identifier(ok), "{ok}");
        }
        for bad in ["", "a", "a..b", ".a.b", "a.b.", "-a.b", "a.b-", "a.b c", "a.b/c"] {
            assert!(!is_identifier(bad), "{bad:?}");
        }
        for ok in ["1", "1.2", "0.7.25"] {
            assert!(is_version(ok), "{ok}");
        }
        for bad in ["", "1.2.3.4", "1..2", "a", "1.2-beta", "1234567890"] {
            assert!(!is_version(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_bad_url_scheme_is_reported() {
        let info = InfoDictionary::from_plist_bytes(
            b"<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>CFBundleURLTypes</key><array><dict><key>CFBundleURLSchemes</key><array><string>1bad</string><string>ok-scheme</string></array></dict></array></dict></plist>",
        )
        .unwrap();
        let bad = info.validate().into_iter().filter(|p| matches!(p, InfoProblem::Invalid { key: "CFBundleURLTypes", .. })).count();
        assert_eq!(bad, 1);
    }

    #[test]
    fn translated_names_come_from_the_info_strings_of_the_package() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("manifest.toml"), MANIFEST).unwrap();
        std::fs::create_dir_all(dir.path().join("is.lproj")).unwrap();
        std::fs::write(dir.path().join("is.lproj/InfoPlist.strings"), "\"CFBundleDisplayName\" = \"Minnisbók\";").unwrap();
        std::fs::create_dir_all(dir.path().join("de.lproj")).unwrap();
        std::fs::write(dir.path().join("de.lproj/InfoPlist.strings"), "/* no display name */ \"Other\" = \"x\";").unwrap();
        let info = InfoDictionary::load(dir.path()).unwrap();
        assert_eq!(info.display_name(dir.path(), &Locale::new("is_IS")).as_deref(), Some("Minnisbók"));
        assert_eq!(info.display_name(dir.path(), &Locale::new("de")).as_deref(), Some("Notes"), "no translation: the dictionary value");
        assert_eq!(info.display_name(dir.path(), &Locale::new("fr")).as_deref(), Some("Notes"));
        assert_eq!(info.localized(dir.path(), &Locale::new("is"), "CFBundleName").as_deref(), Some("Notes"));
        assert_eq!(info.localized(dir.path(), &Locale::new("is"), "NoSuchKey"), None);
    }

    #[test]
    fn load_prefers_the_plist_and_reports_a_missing_or_oversized_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(InfoDictionary::load(dir.path()), Err(InfoError::Missing));
        std::fs::write(dir.path().join("manifest.toml"), MANIFEST).unwrap();
        assert_eq!(InfoDictionary::load(dir.path()).unwrap().identifier(), Some("is.olibuijr.notes"));
        std::fs::write(dir.path().join("Info.plist"), PLIST).unwrap();
        assert_eq!(InfoDictionary::load(dir.path()).unwrap().identifier(), Some("is.olibuijr.draw"));
        std::fs::write(dir.path().join("Info.plist"), vec![b'x'; MAX_INFO_BYTES as usize + 1]).unwrap();
        assert_eq!(InfoDictionary::load(dir.path()), Err(InfoError::TooBig));
    }
}
