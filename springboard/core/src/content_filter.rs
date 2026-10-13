//! URL filtering and the verdict a content filter gives for a new connection.
//!
//! A [`UrlFilter`] holds rules that name a host and an optional path prefix. For a URL it enumerates the sub-URLs (each
//! host suffix of the name, crossed with each path prefix on a segment boundary) and takes the most specific rule that
//! matches one of them. A rule's specificity is the number of labels in its host, then the number of path segments.
//! Two rules with the same host and path are one rule: adding the second replaces the first.
//!
//! [`UrlFilter::flow_verdict`] maps the decision to what a flow filter returns for a new connection: allow or drop. A
//! URL that no rule covers, or that cannot be parsed, is allowed unless the filter is set to fail closed. The model
//! reads no traffic and makes no request: the caller passes the URL in. Hosts are compared in lower case with no
//! trailing dot. IPv6 literals and punycode conversion are not handled.

use crate::dns_settings::normalise_domain;

/// What a rule does with the URLs it covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleAction {
    Allow,
    Block,
}

/// The decision for one URL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Block,
    /// No rule covers the URL, or the URL cannot be parsed.
    Undecided,
}

/// What a flow filter returns for a new connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowVerdict {
    Allow,
    Drop,
}

/// The host and path of a URL, with the path defaulting to `/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedUrl {
    pub host: String,
    pub path: String,
}

/// One filter rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UrlRule {
    pub host: String,
    pub path_prefix: String,
    pub action: RuleAction,
}

/// The URL filter: its rules, whether it is on, and whether an undecided URL is dropped.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UrlFilter {
    pub enabled: bool,
    pub fail_closed: bool,
    rules: Vec<UrlRule>,
}

/// Splits `scheme://host[:port]/path?query#fragment`. Returns None when there is no scheme, no host, or an IPv6
/// literal (a bracketed host).
pub fn parse_url(url: &str) -> Option<ParsedUrl> {
    let (_, rest) = url.split_once("://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if host_port.contains('[') {
        return None;
    }
    let host = normalise_domain(host_port.split(':').next().unwrap_or(""));
    if host.is_empty() {
        return None;
    }
    let tail = &rest[end..];
    let path_end = tail.find(['?', '#']).unwrap_or(tail.len());
    let path = &tail[..path_end];
    let path = if path.is_empty() { "/".to_string() } else { path.to_string() };
    Some(ParsedUrl { host, path })
}

/// A path prefix as a rule stores it: leading slash, no trailing slash, and `/` for the root.
pub fn normalise_path(path: &str) -> String {
    let p = path.trim().trim_end_matches('/');
    if p.is_empty() {
        "/".to_string()
    } else if p.starts_with('/') {
        p.to_string()
    } else {
        format!("/{p}")
    }
}

/// The number of non-empty segments in a path.
fn segments(path: &str) -> usize {
    path.split('/').filter(|s| !s.is_empty()).count()
}

/// Every (host suffix, path prefix) pair of a URL. The path prefixes are `/`, then each growing run of segments.
pub fn sub_urls(url: &ParsedUrl) -> Vec<(String, String)> {
    let labels: Vec<&str> = url.host.split('.').collect();
    let segs: Vec<&str> = url.path.split('/').filter(|s| !s.is_empty()).collect();
    let mut paths = vec!["/".to_string()];
    for k in 1..=segs.len() {
        paths.push(format!("/{}", segs[..k].join("/")));
    }
    let mut out = Vec::new();
    for i in 0..labels.len() {
        let host = labels[i..].join(".");
        for p in &paths {
            out.push((host.clone(), p.clone()));
        }
    }
    out
}

impl UrlFilter {
    /// The number of rules.
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Adds a rule, replacing any rule with the same host and path. Returns false when the host is not a valid name.
    pub fn add_rule(&mut self, host: &str, path: &str, action: RuleAction) -> bool {
        let host = normalise_domain(host);
        if host.is_empty() || !host.split('.').all(|label| !label.is_empty()) {
            return false;
        }
        let path_prefix = normalise_path(path);
        self.rules.retain(|r| !(r.host == host && r.path_prefix == path_prefix));
        self.rules.push(UrlRule { host, path_prefix, action });
        true
    }

    /// The decision for `url` from the most specific matching rule.
    pub fn check(&self, url: &str) -> Verdict {
        let Some(parsed) = parse_url(url) else {
            return Verdict::Undecided;
        };
        let mut best: Option<((usize, usize), RuleAction)> = None;
        for (host, path) in sub_urls(&parsed) {
            if let Some(rule) = self.rules.iter().find(|r| r.host == host && r.path_prefix == path) {
                let key = (rule.host.split('.').count(), segments(&rule.path_prefix));
                if best.is_none_or( |(k, _)| key > k) {
                    best = Some((key, rule.action));
                }
            }
        }
        match best {
            None => Verdict::Undecided,
            Some((_, RuleAction::Allow)) => Verdict::Allow,
            Some((_, RuleAction::Block)) => Verdict::Block,
        }
    }

    /// What a flow filter returns for a new connection to `url`. A disabled filter allows every flow.
    pub fn flow_verdict(&self, url: &str) -> FlowVerdict {
        if !self.enabled {
            return FlowVerdict::Allow;
        }
        match self.check(url) {
            Verdict::Allow => FlowVerdict::Allow,
            Verdict::Block => FlowVerdict::Drop,
            Verdict::Undecided if self.fail_closed => FlowVerdict::Drop,
            Verdict::Undecided => FlowVerdict::Allow,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_is_split_into_host_and_path() {
        let p = parse_url("https://WWW.Example.com:8443/a/b?q=1#f").unwrap();
        assert_eq!(p.host, "www.example.com");
        assert_eq!(p.path, "/a/b");
        assert_eq!(parse_url("https://example.com").unwrap().path, "/");
        assert_eq!(parse_url("https://user@example.com/x").unwrap().host, "example.com");
        assert!(parse_url("example.com/x").is_none(), "no scheme");
        assert!(parse_url("https://").is_none(), "no host");
        assert!(parse_url("https://[::1]:8080/").is_none(), "IPv6 literals are not handled");
    }

    #[test]
    fn sub_urls_cross_each_host_suffix_with_each_path_prefix() {
        let p = parse_url("https://a.b.com/x/y").unwrap();
        let subs = sub_urls(&p);
        assert_eq!(subs.len(), 9, "three host suffixes times three path prefixes");
        assert!(subs.contains(&("b.com".to_string(), "/x".to_string())));
        assert!(subs.contains(&("com".to_string(), "/".to_string())));
        assert!(!subs.contains(&("a.b.com".to_string(), "/x/z".to_string())));
    }

    #[test]
    fn a_block_rule_covers_subdomains_and_its_path_prefix() {
        let mut f = UrlFilter::default();
        assert!(f.add_rule("example.com", "/track", RuleAction::Block));
        assert_eq!(f.check("https://cdn.example.com/track/1"), Verdict::Block);
        assert_eq!(f.check("https://example.com/track"), Verdict::Block);
        assert_eq!(f.check("https://example.com/tracker"), Verdict::Undecided, "a prefix must end on a segment");
        assert_eq!(f.check("https://other.org/track"), Verdict::Undecided);
        assert_eq!(f.check("not a url"), Verdict::Undecided);
    }

    #[test]
    fn the_most_specific_rule_wins() {
        let mut f = UrlFilter::default();
        f.add_rule("example.com", "/", RuleAction::Block);
        f.add_rule("news.example.com", "/", RuleAction::Allow);
        assert_eq!(f.check("https://news.example.com/a"), Verdict::Allow);
        assert_eq!(f.check("https://shop.example.com/a"), Verdict::Block);
        f.add_rule("example.com", "/news/", RuleAction::Allow);
        assert_eq!(f.check("https://example.com/news/today"), Verdict::Allow, "a longer path beats a shorter one");
        assert_eq!(f.rule_count(), 3);
    }

    #[test]
    fn a_rule_with_the_same_host_and_path_replaces_the_old_one() {
        let mut f = UrlFilter::default();
        f.add_rule("Example.com.", "/x", RuleAction::Block);
        f.add_rule("example.com", "/x/", RuleAction::Allow);
        assert_eq!(f.rule_count(), 1);
        assert_eq!(f.check("https://example.com/x/y"), Verdict::Allow);
        assert!(!f.add_rule("a..b", "/", RuleAction::Block), "an empty label is refused");
        assert!(!f.add_rule("", "/", RuleAction::Block));
        assert_eq!(f.rule_count(), 1);
    }

    #[test]
    fn a_flow_is_dropped_only_when_blocked_or_undecided_under_fail_closed() {
        let mut f = UrlFilter::default();
        f.add_rule("ads.example", "/", RuleAction::Block);
        assert_eq!(f.flow_verdict("https://ads.example/x"), FlowVerdict::Allow, "a disabled filter allows everything");
        f.enabled = true;
        assert_eq!(f.flow_verdict("https://ads.example/x"), FlowVerdict::Drop);
        assert_eq!(f.flow_verdict("https://news.example/x"), FlowVerdict::Allow, "undecided and fail-open");
        f.fail_closed = true;
        assert_eq!(f.flow_verdict("https://news.example/x"), FlowVerdict::Drop, "undecided and fail-closed");
        assert_eq!(f.flow_verdict("junk"), FlowVerdict::Drop, "an unparseable URL is undecided");
    }
}
