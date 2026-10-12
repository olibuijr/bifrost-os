//! Match patterns: a pure matcher that decides whether a URL falls inside a group of websites.
//!
//! A pattern is either `<all_urls>` or three parts written as `scheme://host/path`:
//! - scheme: `*` (http or https only), or one of `http`, `https`, `ws`, `wss`, `ftp`, `file`.
//! - host: `*` (any host), `*.example.com` (the domain and its subdomains), or an exact host. A file pattern
//!   may leave the host empty.
//! - path: a glob where `*` matches any run of characters, and it must start with `/`.
//!
//! Matching ignores a port in the URL and the query and fragment, which are not part of the path. The caller
//! supplies the URL; nothing is fetched or stored here.

/// Schemes that `<all_urls>` covers, and the schemes a pattern may name.
const SCHEMES: [&str; 6] = ["http", "https", "ws", "wss", "ftp", "file"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternError {
    Empty,
    /// The text has no `://` separator.
    MissingSeparator,
    BadScheme(String),
    EmptyHost,
    BadHost(String),
    /// The host is not followed by a `/`.
    MissingPath,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchPattern {
    raw: String,
    all_urls: bool,
    scheme: String,
    host: String,
    path: String,
}

impl MatchPattern {
    /// Parses a pattern string. The text is trimmed first.
    pub fn parse(text: &str) -> Result<Self, PatternError> {
        let raw = text.trim();
        if raw.is_empty() {
            return Err(PatternError::Empty);
        }
        if raw == "<all_urls>" {
            return Ok(MatchPattern {
                raw: raw.to_string(),
                all_urls: true,
                scheme: "*".to_string(),
                host: "*".to_string(),
                path: "/*".to_string(),
            });
        }
        let (scheme, rest) = raw.split_once("://").ok_or(PatternError::MissingSeparator)?;
        let scheme = scheme.to_ascii_lowercase();
        if scheme != "*" && !SCHEMES.contains(&scheme.as_str()) {
            return Err(PatternError::BadScheme(scheme));
        }
        let slash = rest.find('/').ok_or(PatternError::MissingPath)?;
        let host = rest[..slash].to_ascii_lowercase();
        if host.is_empty() && scheme != "file" {
            return Err(PatternError::EmptyHost);
        }
        if !valid_host(&host) {
            return Err(PatternError::BadHost(host));
        }
        Ok(MatchPattern {
            raw: raw.to_string(),
            all_urls: false,
            scheme,
            host,
            path: rest[slash..].to_string(),
        })
    }

    /// The pattern text as it was given, trimmed.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Whether this pattern is `<all_urls>`.
    pub fn is_all_urls(&self) -> bool {
        self.all_urls
    }

    /// Whether `url` falls inside the pattern. A URL without `://` never matches.
    pub fn matches(&self, url: &str) -> bool {
        let Some((scheme, rest)) = url.split_once("://") else {
            return false;
        };
        let scheme = scheme.to_ascii_lowercase();
        let end = rest
            .find(|c: char| c == '/' || c == '?' || c == '#')
            .unwrap_or(rest.len());
        let authority = &rest[..end];
        let after_host = &rest[end..];
        let path = after_host
            .split(|c: char| c == '?' || c == '#')
            .next()
            .unwrap_or("");
        let path = if path.is_empty() { "/" } else { path };
        let host = match authority.rfind(':') {
            Some(i) => &authority[..i],
            None => authority,
        };
        let host = host.to_ascii_lowercase();
        if !self.scheme_matches(&scheme) || !self.host_matches(&host) {
            return false;
        }
        glob(self.path.as_bytes(), path.as_bytes())
    }

    fn scheme_matches(&self, scheme: &str) -> bool {
        if self.all_urls {
            return SCHEMES.contains(&scheme);
        }
        match self.scheme.as_str() {
            "*" => scheme == "http" || scheme == "https",
            wanted => wanted == scheme,
        }
    }

    fn host_matches(&self, host: &str) -> bool {
        if self.host == "*" {
            return true;
        }
        if let Some(domain) = self.host.strip_prefix("*.") {
            return host == domain || host.ends_with(format!(".{}", domain).as_str());
        }
        host == self.host
    }
}

/// A host is empty (file patterns), `*`, `*.domain` with a non-empty domain, or an exact name without `*`.
fn valid_host(host: &str) -> bool {
    if host.is_empty() || host == "*" {
        return true;
    }
    match host.strip_prefix("*.") {
        Some(domain) => !domain.is_empty() && !domain.contains('*'),
        None => !host.contains('*'),
    }
}

/// Matches `text` against `pattern`, where `*` in the pattern matches any run of bytes, including `/`.
fn glob(pattern: &[u8], text: &[u8]) -> bool {
    match pattern.first().copied() {
        None => text.is_empty(),
        Some(b'*') => glob(&pattern[1..], text) || (!text.is_empty() && glob(pattern, &text[1..])),
        Some(c) => match text.first().copied() {
            Some(d) if d == c => glob(&pattern[1..], &text[1..]),
            _ => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_urls_covers_the_web_schemes_only() {
        let p = MatchPattern::parse("<all_urls>").unwrap();
        assert!(p.is_all_urls());
        assert!(p.matches("https://a.example/x"));
        assert!(p.matches("ftp://files.example/readme"));
        assert!(!p.matches("mailto:someone@example.com"));
    }

    #[test]
    fn star_scheme_is_http_or_https_only() {
        let p = MatchPattern::parse("*://*/*").unwrap();
        assert!(p.matches("http://a.com/"));
        assert!(p.matches("https://a.com/page?q=1"));
        assert!(!p.matches("ftp://a.com/"));
    }

    #[test]
    fn wildcard_host_covers_the_domain_and_subdomains() {
        let p = MatchPattern::parse("https://*.example.com/*").unwrap();
        assert!(p.matches("https://a.example.com/p?q=1"));
        assert!(p.matches("https://example.com/"));
        assert!(!p.matches("https://notexample.com/"));
        assert!(!p.matches("http://a.example.com/"));
    }

    #[test]
    fn path_glob_and_port_handling() {
        let docs = MatchPattern::parse("https://example.com/docs/*").unwrap();
        assert!(docs.matches("https://example.com/docs/x/y"));
        assert!(!docs.matches("https://example.com/doc"));
        let any = MatchPattern::parse("https://example.com/*").unwrap();
        assert!(any.matches("https://example.com:8443/a"));
    }

    #[test]
    fn parse_rejects_malformed_patterns() {
        assert_eq!(MatchPattern::parse("  "), Err(PatternError::Empty));
        assert_eq!(MatchPattern::parse("example.com/x"), Err(PatternError::MissingSeparator));
        assert_eq!(
            MatchPattern::parse("ftp2://x/"),
            Err(PatternError::BadScheme("ftp2".to_string()))
        );
        assert_eq!(MatchPattern::parse("https:///x"), Err(PatternError::EmptyHost));
        assert_eq!(MatchPattern::parse("https://x"), Err(PatternError::MissingPath));
        assert_eq!(
            MatchPattern::parse("https://a*b.com/"),
            Err(PatternError::BadHost("a*b.com".to_string()))
        );
    }

    #[test]
    fn url_without_separator_never_matches() {
        let p = MatchPattern::parse("<all_urls>").unwrap();
        assert!(!p.matches("example.com/page"));
    }
}
