//! Browser-style web extensions as a pure model: the identity a web extension or content blocker declares, the host
//! permissions the user grants it, its enabled state, and the registry that decides which pages it may touch. The
//! shell keeps the records and the user's choices; nothing here runs extension code or opens a page.
//!
//! A web extension names a bundle identifier (lowercase reverse-DNS tokens such as `com.example.reader.ext`) and a
//! [`Kind`]. Its [`HostPattern`]s say which pages it may act on: a scheme (`https`, `http` or `*`), a host (`example.com`,
//! `*.example.com` or `*`). [`Registry::allows`] answers for a URL only when the extension is enabled and one of its
//! patterns matches. A new extension starts disabled, so nothing reaches a page until the user turns it on.

use std::collections::BTreeMap;

/// What an extension does. A content blocker only filters loads; a web extension can also read and change pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    ContentBlocker,
    WebExtension,
}

/// One host permission: the schemes and hosts an extension may act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostPattern {
    /// `http`, `https` or `*` (any of those two).
    scheme: String,
    /// `*` (any host), `example.com` (exactly that host) or `*.example.com` (that host and its subdomains).
    host: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The bundle identifier is not lowercase reverse-DNS tokens.
    InvalidBundleId,
    /// The display name is empty or longer than 128 characters.
    InvalidName,
    /// The host pattern is not `scheme://host` with a known scheme and a valid host.
    InvalidPattern,
    /// A message name is empty or not a plain token.
    InvalidMessageName,
    /// An extension with this bundle identifier is already installed.
    Duplicate,
    /// No installed extension has this bundle identifier.
    Unknown,
}

impl HostPattern {
    /// Parses `scheme://host`, for example `https://*.example.com` or `*://*`.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let (scheme, host) = text.split_once("://").ok_or(Error::InvalidPattern)?;
        let scheme = scheme.to_ascii_lowercase();
        if !matches!(scheme.as_str(), "http" | "https" | "*") {
            return Err(Error::InvalidPattern);
        }
        let host = host.to_ascii_lowercase();
        let bare = host.strip_prefix("*.").unwrap_or(&host);
        let host_ok = host == "*"
            || (!bare.is_empty()
                && bare.split('.').all(|label| {
                    !label.is_empty() && label.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
                }));
        if !host_ok {
            return Err(Error::InvalidPattern);
        }
        Ok(HostPattern { scheme, host })
    }

    /// Whether this pattern covers `url`. Only http and https URLs with a host can match.
    pub fn matches(&self, url: &str) -> bool {
        let Some((scheme, rest)) = url.split_once("://") else { return false };
        let scheme = scheme.to_ascii_lowercase();
        if scheme != "http" && scheme != "https" {
            return false;
        }
        if self.scheme != "*" && self.scheme != scheme {
            return false;
        }
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        // Drop any user info and port; the host comparison is case-insensitive.
        let host_port = authority.rsplit('@').next().unwrap_or("");
        let host = host_port.split(':').next().unwrap_or("").to_ascii_lowercase();
        if host.is_empty() {
            return false;
        }
        if self.host == "*" {
            return true;
        }
        match self.host.strip_prefix("*.") {
            Some(base) => host == base || host.ends_with(&format!(".{base}")),
            None => host == self.host,
        }
    }

    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    pub fn host(&self) -> &str {
        &self.host
    }
}

/// An extension as the shell records it: identity, what it may act on, and whether the user enabled it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extension {
    bundle_id: String,
    name: String,
    kind: Kind,
    permissions: Vec<HostPattern>,
    enabled: bool,
}

impl Extension {
    /// A new extension, disabled until the user enables it. Validates the bundle identifier and the name.
    pub fn new(bundle_id: &str, name: &str, kind: Kind, permissions: Vec<HostPattern>) -> Result<Self, Error> {
        if !valid_bundle_id(bundle_id) {
            return Err(Error::InvalidBundleId);
        }
        let trimmed = name.trim();
        if trimmed.is_empty() || trimmed.chars().count() > 128 {
            return Err(Error::InvalidName);
        }
        Ok(Extension {
            bundle_id: bundle_id.to_string(),
            name: trimmed.to_string(),
            kind,
            permissions,
            enabled: false,
        })
    }

    pub fn bundle_id(&self) -> &str {
        &self.bundle_id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn permissions(&self) -> &[HostPattern] {
        &self.permissions
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Whether this extension may act on `url`: it is enabled and one of its patterns covers the URL.
    pub fn allows(&self, url: &str) -> bool {
        self.enabled && self.permissions.iter().any(|p| p.matches(url))
    }
}

/// A message an extension sends to its host or the reverse: a name and a dictionary of string values. Names are plain
/// tokens (letters, digits, `.`, `-`, `_`), up to 64 characters, so a host can route them without parsing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    name: String,
    user_info: BTreeMap<String, String>,
}

impl Message {
    pub fn new(name: &str) -> Result<Self, Error> {
        if !valid_token(name, 64) {
            return Err(Error::InvalidMessageName);
        }
        Ok(Message { name: name.to_string(), user_info: BTreeMap::new() })
    }

    /// Adds or replaces one value. Keys follow the same token rule as names.
    pub fn with(mut self, key: &str, value: &str) -> Result<Self, Error> {
        if !valid_token(key, 64) {
            return Err(Error::InvalidMessageName);
        }
        self.user_info.insert(key.to_string(), value.to_string());
        Ok(self)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn value(&self, key: &str) -> Option<&str> {
        self.user_info.get(key).map(String::as_str)
    }
}

/// The installed web extensions. The shell rebuilds it from its package records and the user's enable choices.
#[derive(Debug, Default)]
pub struct Registry {
    extensions: Vec<Extension>,
}

impl Registry {
    pub fn new() -> Self {
        Registry::default()
    }

    /// Installs an extension. Bundle identifiers are unique; a duplicate is refused and leaves the registry as it was.
    pub fn install(&mut self, extension: Extension) -> Result<(), Error> {
        if self.get(extension.bundle_id()).is_some() {
            return Err(Error::Duplicate);
        }
        self.extensions.push(extension);
        Ok(())
    }

    pub fn get(&self, bundle_id: &str) -> Option<&Extension> {
        self.extensions.iter().find(|e| e.bundle_id == bundle_id)
    }

    /// Turns an installed extension on or off.
    pub fn set_enabled(&mut self, bundle_id: &str, enabled: bool) -> Result<(), Error> {
        let ext = self.extensions.iter_mut().find(|e| e.bundle_id == bundle_id).ok_or(Error::Unknown)?;
        ext.enabled = enabled;
        Ok(())
    }

    /// Removes an installed extension and returns it.
    pub fn remove(&mut self, bundle_id: &str) -> Result<Extension, Error> {
        let at = self.extensions.iter().position(|e| e.bundle_id == bundle_id).ok_or(Error::Unknown)?;
        Ok(self.extensions.remove(at))
    }

    /// The extensions that may act on `url`, in install order.
    pub fn allowed_for(&self, url: &str) -> Vec<&Extension> {
        self.extensions.iter().filter(|e| e.allows(url)).collect()
    }

    /// The enabled extensions of one kind, in install order.
    pub fn enabled_of(&self, kind: Kind) -> Vec<&Extension> {
        self.extensions.iter().filter(|e| e.enabled && e.kind == kind).collect()
    }

    pub fn len(&self) -> usize {
        self.extensions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.extensions.is_empty()
    }
}

/// Why a message exchange step was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExchangeError {
    /// The extension is not installed, is disabled, or has no permission for the page.
    NotAllowed,
    /// No open request has this id: it was never made, already answered or cancelled.
    NoSuchRequest,
    /// The reply came from an extension other than the one the request was sent to.
    WrongSender,
}

/// Routes requests from pages to web extensions and their replies back. A request is possible only where the registry
/// allows the extension for the page URL, and it is answered once, by the extension it was sent to.
#[derive(Debug, Default)]
pub struct Exchange {
    next_id: u64,
    /// Open requests: id to the bundle identifier and the page URL.
    open: BTreeMap<u64, (String, String)>,
}

impl Exchange {
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens a request from a page to an extension and returns the id its reply must carry.
    pub fn request(&mut self, registry: &Registry, bundle_id: &str, page_url: &str) -> Result<u64, ExchangeError> {
        if !registry.get(bundle_id).is_some_and(|e| e.allows(page_url)) {
            return Err(ExchangeError::NotAllowed);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.open.insert(id, (bundle_id.to_string(), page_url.to_string()));
        Ok(id)
    }

    /// Answers request `id`. The request closes whether or not the reply is delivered, so a refused reply cannot be retried.
    /// A reply from another extension leaves the request open.
    pub fn reply(
        &mut self,
        registry: &Registry,
        bundle_id: &str,
        id: u64,
        message: Message,
    ) -> Result<Message, ExchangeError> {
        let (sent_to, page_url) = self.open.get(&id).ok_or(ExchangeError::NoSuchRequest)?;
        if sent_to != bundle_id {
            return Err(ExchangeError::WrongSender);
        }
        let page_url = page_url.clone();
        self.open.remove(&id);
        if !registry.get(bundle_id).is_some_and(|e| e.allows(&page_url)) {
            return Err(ExchangeError::NotAllowed);
        }
        Ok(message)
    }

    /// Closes an open request without a reply. Returns false when no such request is open.
    pub fn cancel(&mut self, id: u64) -> bool {
        self.open.remove(&id).is_some()
    }

    /// Requests waiting for a reply.
    pub fn pending_count(&self) -> usize {
        self.open.len()
    }
}

/// Lowercase reverse-DNS tokens separated by dots, at least two labels: `com.example.app`.
fn valid_bundle_id(id: &str) -> bool {
    let labels: Vec<&str> = id.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && l.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                && !l.starts_with('-')
                && !l.ends_with('-')
        })
}

fn valid_token(text: &str, max: usize) -> bool {
    !text.is_empty() && text.len() <= max && text.bytes().all(|c| c.is_ascii_alphanumeric() || b".-_".contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(s: &str) -> HostPattern {
        HostPattern::parse(s).expect("valid pattern")
    }

    fn reader() -> Extension {
        Extension::new("com.example.reader", "Reader", Kind::WebExtension, vec![pattern("https://*.example.com")]).unwrap()
    }

    #[test]
    fn bundle_ids_must_be_lowercase_reverse_dns() {
        assert!(valid_bundle_id("com.example.reader"));
        assert!(valid_bundle_id("is.olibuijr.ext-2"));
        assert!(!valid_bundle_id("reader"));
        assert!(!valid_bundle_id("Com.example.reader"));
        assert!(!valid_bundle_id("com..reader"));
        assert!(!valid_bundle_id("com.-bad.reader"));
        assert!(!valid_bundle_id("com.example.reader!"));
    }

    #[test]
    fn new_extension_validates_identity_and_starts_disabled() {
        assert_eq!(
            Extension::new("bad", "Reader", Kind::ContentBlocker, vec![]).unwrap_err(),
            Error::InvalidBundleId
        );
        assert_eq!(
            Extension::new("com.example.reader", "   ", Kind::ContentBlocker, vec![]).unwrap_err(),
            Error::InvalidName
        );
        let e = reader();
        assert!(!e.is_enabled());
        assert_eq!(e.name(), "Reader");
        assert_eq!(e.kind(), Kind::WebExtension);
    }

    #[test]
    fn pattern_parses_scheme_and_host() {
        let p = HostPattern::parse("HTTPS://Example.COM").unwrap();
        assert_eq!(p.scheme(), "https");
        assert_eq!(p.host(), "example.com");
        assert!(HostPattern::parse("*://*").is_ok());
        assert_eq!(HostPattern::parse("ftp://example.com").unwrap_err(), Error::InvalidPattern);
        assert_eq!(HostPattern::parse("example.com").unwrap_err(), Error::InvalidPattern);
        assert_eq!(HostPattern::parse("https://*.").unwrap_err(), Error::InvalidPattern);
        assert_eq!(HostPattern::parse("https://a..b").unwrap_err(), Error::InvalidPattern);
    }

    #[test]
    fn subdomain_pattern_covers_the_host_and_its_children_only() {
        let p = pattern("https://*.example.com");
        assert!(p.matches("https://example.com/page"));
        assert!(p.matches("https://news.example.com/x?y=1"));
        assert!(p.matches("https://a.b.example.com:8443/"));
        assert!(!p.matches("https://evil-example.com/"));
        assert!(!p.matches("https://example.com.evil.org/"));
        assert!(!p.matches("http://news.example.com/"));
        assert!(!p.matches("mailto:someone@example.com"));
    }

    #[test]
    fn exact_host_and_wildcards() {
        assert!(pattern("http://example.com").matches("http://EXAMPLE.com/a"));
        assert!(!pattern("http://example.com").matches("http://news.example.com/"));
        assert!(pattern("*://*").matches("http://anything.test/"));
        assert!(!pattern("*://*").matches("file:///etc/passwd"));
        assert!(pattern("*://*").matches("https://user@host.test:9/"));
    }

    #[test]
    fn allows_needs_enabled_and_a_matching_pattern() {
        let mut e = reader();
        assert!(!e.allows("https://news.example.com/"));
        let mut r = Registry::new();
        r.install(e.clone()).unwrap();
        r.set_enabled("com.example.reader", true).unwrap();
        assert_eq!(r.allowed_for("https://news.example.com/").len(), 1);
        assert!(r.allowed_for("https://other.test/").is_empty());
        e.enabled = true;
        assert!(e.allows("https://example.com/"));
    }

    #[test]
    fn registry_refuses_duplicates_and_unknown_ids() {
        let mut r = Registry::new();
        assert!(r.is_empty());
        r.install(reader()).unwrap();
        assert_eq!(r.install(reader()), Err(Error::Duplicate));
        assert_eq!(r.len(), 1);
        assert_eq!(r.set_enabled("com.example.none", true), Err(Error::Unknown));
        assert_eq!(r.remove("com.example.none").unwrap_err(), Error::Unknown);
        let removed = r.remove("com.example.reader").unwrap();
        assert_eq!(removed.bundle_id(), "com.example.reader");
        assert!(r.is_empty());
    }

    #[test]
    fn enabled_of_filters_by_kind() {
        let mut r = Registry::new();
        r.install(reader()).unwrap();
        let blocker = Extension::new("com.example.blocker", "Blocker", Kind::ContentBlocker, vec![pattern("*://*")]).unwrap();
        r.install(blocker).unwrap();
        assert!(r.enabled_of(Kind::ContentBlocker).is_empty());
        r.set_enabled("com.example.blocker", true).unwrap();
        let blockers = r.enabled_of(Kind::ContentBlocker);
        assert_eq!(blockers.len(), 1);
        assert_eq!(blockers[0].bundle_id(), "com.example.blocker");
        assert!(r.enabled_of(Kind::WebExtension).is_empty());
    }

    #[test]
    fn message_names_and_values_are_plain_tokens() {
        assert_eq!(Message::new("").unwrap_err(), Error::InvalidMessageName);
        assert_eq!(Message::new("has space").unwrap_err(), Error::InvalidMessageName);
        let m = Message::new("reader.open").unwrap().with("url", "https://example.com/").unwrap();
        assert_eq!(m.name(), "reader.open");
        assert_eq!(m.value("url"), Some("https://example.com/"));
        assert_eq!(m.value("missing"), None);
        assert_eq!(Message::new("ok").unwrap().with("bad key", "x").unwrap_err(), Error::InvalidMessageName);
    }
}
