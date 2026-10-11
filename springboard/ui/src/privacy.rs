//! Privacy declarations an app ships in its package: whether it tracks people, which tracking hosts it contacts, the
//! kinds of data it collects, and the required-reason system APIs it uses. This mirrors the privacy manifest of the
//! official bundle resources (tracking, tracking domains, collected data types, accessed API types with reasons).
//! Pure data and checks, no I/O: the shell reads `privacy.toml` from the package and shows the declaration. Nothing
//! is enforced at run time yet; `undeclared_apis` lets a build step or the store review check the declaration
//! against the APIs an app actually calls.
//!
//! Format (same `key = value` lines as `manifest.toml`):
//! ```text
//! tracking = false
//! tracking_domains = ["ads.example.com"]
//!
//! [[collected]]
//! kind = "contact-info"
//! linked = true
//! tracking = false
//! purposes = ["app-functionality"]
//!
//! [[accessed_api]]
//! category = "file-timestamp"
//! reasons = ["own-app-data"]
//! ```
use std::collections::HashSet;

/// One kind of data the app collects.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Collected {
    pub kind: String,
    /// Tied to the person's identity.
    pub linked: bool,
    /// Used to track the person across other companies' apps and sites.
    pub tracking: bool,
    pub purposes: Vec<String>,
}

/// One required-reason API category and the reasons the app gives for using it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccessedApi {
    pub category: String,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrivacyManifest {
    pub tracking: bool,
    pub tracking_domains: Vec<String>,
    pub collected: Vec<Collected>,
    pub accessed_apis: Vec<AccessedApi>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Top,
    Collected,
    Api,
}

impl PrivacyManifest {
    /// Parses the format above. Unknown keys and tables are errors, so a typo cannot silently drop a declaration.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut m = PrivacyManifest::default();
        let mut section = Section::Top;
        for (n, raw) in text.lines().enumerate() {
            let at = n + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            match line {
                "[[collected]]" => {
                    m.collected.push(Collected::default());
                    section = Section::Collected;
                    continue;
                }
                "[[accessed_api]]" => {
                    m.accessed_apis.push(AccessedApi::default());
                    section = Section::Api;
                    continue;
                }
                _ if line.starts_with('[') => return Err(format!("privacy line {at}: unknown table {line}")),
                _ => {}
            }
            let (k, v) = line.split_once('=').ok_or_else(|| format!("privacy line {at}: expected key = value"))?;
            let (k, v) = (k.trim(), v.trim());
            match section {
                Section::Top => match k {
                    "tracking" => m.tracking = boolean(v, at)?,
                    "tracking_domains" => m.tracking_domains = strings(v, at)?,
                    _ => return Err(format!("privacy line {at}: unknown key {k}")),
                },
                Section::Collected => {
                    let c = m.collected.last_mut().ok_or_else(|| format!("privacy line {at}: no table"))?;
                    match k {
                        "kind" => c.kind = string(v, at)?,
                        "linked" => c.linked = boolean(v, at)?,
                        "tracking" => c.tracking = boolean(v, at)?,
                        "purposes" => c.purposes = strings(v, at)?,
                        _ => return Err(format!("privacy line {at}: unknown key {k} in [[collected]]")),
                    }
                }
                Section::Api => {
                    let a = m.accessed_apis.last_mut().ok_or_else(|| format!("privacy line {at}: no table"))?;
                    match k {
                        "category" => a.category = string(v, at)?,
                        "reasons" => a.reasons = strings(v, at)?,
                        _ => return Err(format!("privacy line {at}: unknown key {k} in [[accessed_api]]")),
                    }
                }
            }
        }
        Ok(m)
    }

    /// Checks what the parser cannot: tracking hosts are bare host names, every declaration is named, no data kind
    /// is declared twice, and every API category gives at least one reason.
    pub fn validate(&self) -> Result<(), String> {
        for d in &self.tracking_domains {
            if !is_host(d) {
                return Err(format!("tracking domain {d:?} is not a bare host name"));
            }
        }
        let mut kinds = HashSet::new();
        for c in &self.collected {
            if c.kind.is_empty() {
                return Err("a collected data entry has no kind".into());
            }
            if !kinds.insert(c.kind.as_str()) {
                return Err(format!("collected data kind {:?} is declared twice", c.kind));
            }
        }
        for a in &self.accessed_apis {
            if a.category.is_empty() {
                return Err("an accessed API entry has no category".into());
            }
            if a.reasons.is_empty() {
                return Err(format!("accessed API {:?} gives no reason", a.category));
            }
        }
        Ok(())
    }

    /// True when the app tracks people, either for the whole app or for one kind of data.
    pub fn tracks(&self) -> bool {
        self.tracking || self.collected.iter().any(|c| c.tracking)
    }

    /// The API categories `used` (what the code calls) that the manifest does not declare, in the order given.
    pub fn undeclared_apis<'a>(&self, used: &[&'a str]) -> Vec<&'a str> {
        let declared: HashSet<&str> = self.accessed_apis.iter().map(|a| a.category.as_str()).collect();
        used.iter().copied().filter(|c| !declared.contains(c)).collect()
    }
}

fn boolean(v: &str, at: usize) -> Result<bool, String> {
    match v {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("privacy line {at}: expected true or false")),
    }
}

fn string(v: &str, at: usize) -> Result<String, String> {
    v.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .map(str::to_string)
        .ok_or_else(|| format!("privacy line {at}: expected a quoted string"))
}

fn strings(v: &str, at: usize) -> Result<Vec<String>, String> {
    let inner = v
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(|| format!("privacy line {at}: expected a list in brackets"))?;
    inner
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| string(s, at))
        .collect()
}

/// A bare host: dot-separated labels of ASCII letters, digits and inner hyphens. No scheme, port, path or wildcard.
fn is_host(s: &str) -> bool {
    s.len() <= 253
        && s.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = "# privacy\n\
        tracking = false\n\
        tracking_domains = [\"ads.example.com\", \"t.example.org\"]\n\
        \n\
        [[collected]]\n\
        kind = \"contact-info\"\n\
        linked = true\n\
        tracking = false\n\
        purposes = [\"app-functionality\", \"analytics\"]\n\
        \n\
        [[accessed_api]]\n\
        category = \"file-timestamp\"\n\
        reasons = [\"own-app-data\"]\n";

    #[test]
    fn parses_top_level_keys_and_both_tables() {
        let m = PrivacyManifest::parse(FULL).unwrap();
        assert!(!m.tracking);
        assert_eq!(m.tracking_domains, ["ads.example.com", "t.example.org"]);
        assert_eq!(m.collected.len(), 1);
        assert_eq!(m.collected[0].kind, "contact-info");
        assert!(m.collected[0].linked && !m.collected[0].tracking);
        assert_eq!(m.collected[0].purposes, ["app-functionality", "analytics"]);
        assert_eq!(m.accessed_apis, [AccessedApi { category: "file-timestamp".into(), reasons: vec!["own-app-data".into()] }]);
    }

    #[test]
    fn empty_text_is_a_manifest_that_tracks_nothing() {
        let m = PrivacyManifest::parse("  \n# only a comment\n").unwrap();
        assert_eq!(m, PrivacyManifest::default());
        assert!(m.validate().is_ok() && !m.tracks());
    }

    #[test]
    fn rejects_unknown_keys_tables_and_keys_outside_their_table() {
        assert!(PrivacyManifest::parse("trackng = true").unwrap_err().contains("unknown key trackng"));
        assert!(PrivacyManifest::parse("[[nope]]").unwrap_err().contains("unknown table"));
        assert!(PrivacyManifest::parse("kind = \"x\"").unwrap_err().contains("unknown key kind"));
        assert!(PrivacyManifest::parse("tracking = yes").is_err());
        assert!(PrivacyManifest::parse("tracking_domains = [a.example]").is_err(), "list items must be quoted");
        assert!(PrivacyManifest::parse("just a line").unwrap_err().contains("expected key = value"));
    }

    #[test]
    fn validate_accepts_bare_hosts_and_rejects_the_rest() {
        for good in ["example.com", "a-b.c1.example", "x"] {
            let m = PrivacyManifest { tracking_domains: vec![good.into()], ..Default::default() };
            assert!(m.validate().is_ok(), "{good} is a host");
        }
        for bad in ["https://example.com", "example.com/path", "-bad.example", "bad-.example", "a..b", "*.example.com", "a_b.example", ""] {
            let m = PrivacyManifest { tracking_domains: vec![bad.into()], ..Default::default() };
            assert!(m.validate().unwrap_err().contains("not a bare host name"), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn validate_rejects_duplicate_kinds_and_reasonless_apis() {
        let dup = PrivacyManifest::parse(&format!("{FULL}\n[[collected]]\nkind = \"contact-info\"\n")).unwrap();
        assert!(dup.validate().unwrap_err().contains("declared twice"));
        let bare = PrivacyManifest::parse("[[accessed_api]]\ncategory = \"file-timestamp\"\n").unwrap();
        assert!(bare.validate().unwrap_err().contains("gives no reason"));
        let unnamed = PrivacyManifest::parse("[[collected]]\nlinked = false\n").unwrap();
        assert!(unnamed.validate().unwrap_err().contains("no kind"));
    }

    #[test]
    fn tracks_when_the_app_or_one_data_kind_tracks() {
        let app = PrivacyManifest { tracking: true, ..Default::default() };
        assert!(app.tracks());
        let one = PrivacyManifest::parse("[[collected]]\nkind = \"device-id\"\ntracking = true\n").unwrap();
        assert!(!one.tracking && one.tracks());
        assert!(!PrivacyManifest::parse(FULL).unwrap().tracks());
    }

    #[test]
    fn undeclared_apis_lists_only_the_missing_categories_in_call_order() {
        let m = PrivacyManifest::parse(FULL).unwrap();
        let used = ["disk-space", "file-timestamp", "user-defaults"];
        assert_eq!(m.undeclared_apis(&used), ["disk-space", "user-defaults"]);
        assert!(m.undeclared_apis(&["file-timestamp"]).is_empty());
        assert_eq!(PrivacyManifest::default().undeclared_apis(&used), used);
    }
}
