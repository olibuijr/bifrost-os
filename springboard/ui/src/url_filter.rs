//! URL filter: a pure allow and deny list over match patterns, returning a verdict for a URL.
//!
//! An app that opens connections outside the web engine asks the filter before it connects. The rule is:
//! a URL is allowed when an allow pattern matches it, denied when a deny pattern matches and no allow pattern
//! does, and allowed when no pattern matches. An allow pattern therefore overrides a broader deny pattern.
//! The caller supplies the URL; nothing is fetched or stored here.

use crate::match_pattern::{MatchPattern, PatternError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Deny,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UrlFilter {
    allow: Vec<MatchPattern>,
    deny: Vec<MatchPattern>,
}

impl UrlFilter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an allow pattern. A malformed pattern is reported and changes nothing.
    pub fn allow(&mut self, pattern: &str) -> Result<(), PatternError> {
        self.allow.push(MatchPattern::parse(pattern)?);
        Ok(())
    }

    /// Adds a deny pattern. A malformed pattern is reported and changes nothing.
    pub fn deny(&mut self, pattern: &str) -> Result<(), PatternError> {
        self.deny.push(MatchPattern::parse(pattern)?);
        Ok(())
    }

    /// The verdict for `url`, following the rule in the module comment.
    pub fn verdict(&self, url: &str) -> Verdict {
        if self.allow.iter().any(|p| p.matches(url)) {
            return Verdict::Allow;
        }
        if self.deny.iter().any(|p| p.matches(url)) {
            return Verdict::Deny;
        }
        Verdict::Allow
    }

    /// Whether the app may connect to `url`. Only a denied URL must not be connected to.
    pub fn should_connect(&self, url: &str) -> bool {
        self.verdict(url) == Verdict::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_filter_allows_everything() {
        let f = UrlFilter::new();
        assert_eq!(f.verdict("https://anything.example/"), Verdict::Allow);
        assert!(f.should_connect("https://anything.example/"));
    }

    #[test]
    fn deny_pattern_blocks_its_urls() {
        let mut f = UrlFilter::new();
        f.deny("https://ads.example.com/*").unwrap();
        assert_eq!(f.verdict("https://ads.example.com/banner?id=3"), Verdict::Deny);
        assert!(!f.should_connect("https://ads.example.com/banner"));
        assert_eq!(f.verdict("https://news.example.com/"), Verdict::Allow);
    }

    #[test]
    fn wildcard_deny_matches_subdomains_only_of_the_domain() {
        let mut f = UrlFilter::new();
        f.deny("*://*.tracker.net/*").unwrap();
        assert_eq!(f.verdict("http://a.tracker.net/"), Verdict::Deny);
        assert_eq!(f.verdict("https://tracker.net.evil.org/"), Verdict::Allow);
    }

    #[test]
    fn allow_pattern_overrides_a_broader_deny() {
        let mut f = UrlFilter::new();
        f.deny("*://*/*").unwrap();
        f.allow("https://intranet.example/*").unwrap();
        assert_eq!(f.verdict("https://intranet.example/a"), Verdict::Allow);
        assert_eq!(f.verdict("https://other.com/"), Verdict::Deny);
    }

    #[test]
    fn star_scheme_deny_does_not_cover_ftp() {
        let mut f = UrlFilter::new();
        f.deny("*://*/*").unwrap();
        assert_eq!(f.verdict("ftp://files.example/x"), Verdict::Allow);
    }

    #[test]
    fn malformed_pattern_is_reported_and_ignored() {
        let mut f = UrlFilter::new();
        assert_eq!(f.deny("bad"), Err(PatternError::MissingSeparator));
        assert_eq!(f.verdict("https://bad/"), Verdict::Allow);
        assert_eq!(f, UrlFilter::new());
    }
}
