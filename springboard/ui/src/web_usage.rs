//! Web-usage reporting for one webpage. A controller per page or tab keeps the page's address, the bundle that reports
//! the usage, the browsing profile, whether usage recording is suppressed, whether a video is playing and whether a
//! picture-in-picture window is open. A block list decides the blocked flag. A controller turns a visit into a
//! `UsageRecord` for the history store (`web_history`). Pure model: the caller supplies the clock and the block list.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The bundle identifier that reports usage is empty.
    EmptyBundleIdentifier,
    /// A block rule names no host.
    EmptyHost,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::EmptyBundleIdentifier => write!(f, "the bundle identifier is empty"),
            Error::EmptyHost => write!(f, "the block rule names no host"),
        }
    }
}

impl std::error::Error for Error {}

/// One visit reported by a controller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageRecord {
    pub bundle_id: String,
    pub profile_id: Option<String>,
    /// The canonical address (see `canonical_url`).
    pub url: String,
    /// Seconds since the Unix epoch, as given by the caller.
    pub at: i64,
}

/// Lowercases the scheme and the host, drops the fragment, and keeps the path and query as they were given.
pub fn canonical_url(url: &str) -> String {
    let no_fragment = url.trim().split('#').next().unwrap_or("");
    let (scheme, rest) = match no_fragment.split_once("://") {
        Some((scheme, rest)) => (Some(scheme.to_ascii_lowercase()), rest),
        None => (None, no_fragment),
    };
    let cut = rest.find(|c: char| c == '/' || c == '?').unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(cut);
    let authority = authority.to_ascii_lowercase();
    match scheme {
        Some(scheme) => format!("{scheme}://{authority}{tail}"),
        None => format!("{authority}{tail}"),
    }
}

/// The host (lowercase, without user information or port) and the path (`/` when empty, without the query).
fn host_and_path(url: &str) -> (String, String) {
    let canonical = canonical_url(url);
    let rest = match canonical.split_once("://") {
        Some((_, rest)) => rest,
        None => canonical.as_str(),
    };
    let cut = rest.find(|c: char| c == '/' || c == '?').unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(cut);
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = host_port.split(':').next().unwrap_or("");
    let path = tail.split('?').next().unwrap_or("");
    let path = if path.is_empty() { "/" } else { path };
    (host.to_string(), path.to_string())
}

/// Whether `path` lies under `prefix` on a segment boundary: `/games` covers `/games` and `/games/x`, not `/gamesroom`.
fn path_under(path: &str, prefix: &str) -> bool {
    if prefix == "/" {
        return true;
    }
    let prefix = prefix.trim_end_matches('/');
    path == prefix || path.starts_with(format!("{prefix}/").as_str())
}

/// Parent or guardian block rules. A rule is a host, optionally followed by a path: `example.com` blocks that host and
/// its subdomains; `example.com/games` blocks `/games` and the paths under it. A scheme in a rule is ignored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockList {
    rules: Vec<(String, String)>,
}

impl BlockList {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a rule. Adding the same rule twice keeps one copy.
    pub fn block(&mut self, pattern: &str) -> Result<(), Error> {
        let (host, path) = host_and_path(pattern);
        if host.is_empty() {
            return Err(Error::EmptyHost);
        }
        if !self.rules.iter().any(|(h, p)| h == &host && p == &path) {
            self.rules.push((host, path));
        }
        Ok(())
    }

    /// Removes the rule with exactly this host and path. Returns whether a rule was removed.
    pub fn unblock(&mut self, pattern: &str) -> bool {
        let (host, path) = host_and_path(pattern);
        let before = self.rules.len();
        self.rules.retain(|(h, p)| !(h == &host && p == &path));
        self.rules.len() != before
    }

    /// Whether the address falls under any rule.
    pub fn is_blocked(&self, url: &str) -> bool {
        let (host, path) = host_and_path(url);
        self.rules.iter().any(|(h, p)| {
            let host_matches = host == *h || host.ends_with(format!(".{h}").as_str());
            host_matches && path_under(&path, p)
        })
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

/// The reporting state of one webpage or tab.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebpageController {
    url: Option<String>,
    bundle_id: Option<String>,
    profile_id: Option<String>,
    suppress: bool,
    playing_video: bool,
    picture_in_picture: bool,
    blocked: bool,
}

impl WebpageController {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the page address (`None` for a page without one) and recomputes the blocked flag from `blocks`.
    pub fn set_url(&mut self, url: Option<&str>, blocks: &BlockList) {
        self.url = url.map(canonical_url).filter(|u| !u.is_empty());
        self.blocked = self.url.as_deref().map_or(false, |u| blocks.is_blocked(u));
    }

    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    pub fn url_is_blocked(&self) -> bool {
        self.blocked
    }

    /// Sets the bundle that reports usage. The identifier is trimmed, and an empty one is refused.
    pub fn set_bundle_identifier(&mut self, id: &str) -> Result<(), Error> {
        let id = id.trim();
        if id.is_empty() {
            return Err(Error::EmptyBundleIdentifier);
        }
        self.bundle_id = Some(id.to_string());
        Ok(())
    }

    pub fn bundle_identifier(&self) -> Option<&str> {
        self.bundle_id.as_deref()
    }

    /// Sets the browsing profile. Builder style, because the profile is fixed once the page is created.
    pub fn with_profile(mut self, id: Option<&str>) -> Self {
        self.profile_id = id.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        self
    }

    pub fn profile_identifier(&self) -> Option<&str> {
        self.profile_id.as_deref()
    }

    pub fn set_suppress_usage_recording(&mut self, on: bool) {
        self.suppress = on;
    }

    pub fn suppress_usage_recording(&self) -> bool {
        self.suppress
    }

    pub fn set_url_is_playing_video(&mut self, on: bool) {
        self.playing_video = on;
    }

    pub fn url_is_playing_video(&self) -> bool {
        self.playing_video
    }

    pub fn set_url_is_picture_in_picture(&mut self, on: bool) {
        self.picture_in_picture = on;
    }

    pub fn url_is_picture_in_picture(&self) -> bool {
        self.picture_in_picture
    }

    /// The visit to report at `at`. Returns `None` when there is no address, no bundle, or recording is suppressed.
    pub fn usage_record(&self, at: i64) -> Option<UsageRecord> {
        if self.suppress {
            return None;
        }
        let url = self.url.clone()?;
        let bundle_id = self.bundle_id.clone()?;
        Some(UsageRecord { bundle_id, profile_id: self.profile_id.clone(), url, at })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_url_lowercases_scheme_and_host_and_drops_the_fragment() {
        assert_eq!(canonical_url("HTTPS://Example.COM/Path?Q=1#frag"), "https://example.com/Path?Q=1");
        assert_eq!(canonical_url("  https://a.test  "), "https://a.test");
        assert_eq!(canonical_url("Example.com/Docs"), "example.com/Docs");
    }

    #[test]
    fn a_host_rule_blocks_its_subdomains_but_not_look_alike_hosts() {
        let mut blocks = BlockList::new();
        blocks.block("example.com").unwrap();
        assert!(blocks.is_blocked("https://www.example.com/a"));
        assert!(blocks.is_blocked("https://example.com:8443/"));
        assert!(!blocks.is_blocked("https://notexample.com/"));
        assert!(!blocks.is_blocked("https://example.com.evil.test/"));
    }

    #[test]
    fn a_path_rule_blocks_on_a_segment_boundary() {
        let mut blocks = BlockList::new();
        blocks.block("https://video.test/games/").unwrap();
        assert!(blocks.is_blocked("https://video.test/games"));
        assert!(blocks.is_blocked("https://video.test/games/level1?x=2"));
        assert!(!blocks.is_blocked("https://video.test/gamesroom"));
        assert!(!blocks.is_blocked("https://video.test/"));
    }

    #[test]
    fn a_rule_needs_a_host_and_repeats_are_kept_once() {
        let mut blocks = BlockList::new();
        assert_eq!(blocks.block("https:///x"), Err(Error::EmptyHost));
        blocks.block("example.com").unwrap();
        blocks.block("https://EXAMPLE.com/").unwrap();
        assert_eq!(blocks.len(), 1);
        assert!(blocks.unblock("example.com"));
        assert!(!blocks.unblock("example.com"));
        assert!(blocks.is_empty());
    }

    #[test]
    fn set_url_sets_the_blocked_flag_from_the_block_list() {
        let mut blocks = BlockList::new();
        blocks.block("https://video.test/games").unwrap();
        let mut page = WebpageController::new();
        page.set_url(Some("https://Video.Test/games/x#t"), &blocks);
        assert_eq!(page.url(), Some("https://video.test/games/x"));
        assert!(page.url_is_blocked());
        page.set_url(Some("https://video.test/news"), &blocks);
        assert!(!page.url_is_blocked());
        page.set_url(None, &blocks);
        assert_eq!(page.url(), None);
        assert!(!page.url_is_blocked());
    }

    #[test]
    fn usage_is_recorded_only_with_an_address_a_bundle_and_recording_on() {
        let blocks = BlockList::new();
        let mut page = WebpageController::new().with_profile(Some(" work "));
        page.set_url(Some("https://a.test/p"), &blocks);
        assert_eq!(page.usage_record(100), None);
        assert_eq!(page.set_bundle_identifier("  "), Err(Error::EmptyBundleIdentifier));
        page.set_bundle_identifier(" com.example.app ").unwrap();
        assert_eq!(page.bundle_identifier(), Some("com.example.app"));
        assert_eq!(
            page.usage_record(100),
            Some(UsageRecord {
                bundle_id: "com.example.app".to_string(),
                profile_id: Some("work".to_string()),
                url: "https://a.test/p".to_string(),
                at: 100,
            })
        );
        page.set_suppress_usage_recording(true);
        assert_eq!(page.usage_record(101), None);
        assert!(page.suppress_usage_recording());
    }

    #[test]
    fn media_flags_are_kept_separately_from_the_address() {
        let mut page = WebpageController::new();
        page.set_url_is_playing_video(true);
        page.set_url_is_picture_in_picture(true);
        assert!(page.url_is_playing_video());
        assert!(page.url_is_picture_in_picture());
        page.set_url_is_playing_video(false);
        assert!(!page.url_is_playing_video());
        assert!(page.url_is_picture_in_picture());
        assert_eq!(page.url(), None);
    }
}
