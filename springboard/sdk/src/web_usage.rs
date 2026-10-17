//! Web usage for apps: the seconds spent on each website, how much of it was video or picture-in-picture, and
//! whether a URL is on the block list. A browser reports each page it shows through `report`, and Screen Time
//! reads the totals back with `usage` and `ranked`. Hosts match the block list by domain and subdomain, so
//! blocking `example.com` also blocks `www.example.com`. No network is touched here.

use std::collections::{BTreeMap, BTreeSet};

/// The time one host has been shown, in whole seconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostUsage {
    /// All time on the host.
    pub seconds: u32,
    /// The part of that time with video playing.
    pub video_seconds: u32,
    /// The part of that time with a picture-in-picture video showing.
    pub picture_in_picture_seconds: u32,
}

/// The host of `url` in lowercase, without scheme, user information, port or path. None when there is no host.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, after)| after);
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = host.split(':').next()?.trim_end_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

/// Website time and the block list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebUsage {
    hosts: BTreeMap<String, HostUsage>,
    blocked: BTreeSet<String>,
}

impl WebUsage {
    /// Adds `seconds` of time on the host of `url`. Video and picture-in-picture time are counted within `seconds`,
    /// so a flag adds the whole interval to its own total. Returns the host recorded, or None for a URL with no host.
    pub fn report(&mut self, url: &str, seconds: u32, playing_video: bool, picture_in_picture: bool) -> Option<String> {
        let host = host_of(url)?;
        let entry = self.hosts.entry(host.clone()).or_default();
        entry.seconds = entry.seconds.saturating_add(seconds);
        if playing_video {
            entry.video_seconds = entry.video_seconds.saturating_add(seconds);
        }
        if picture_in_picture {
            entry.picture_in_picture_seconds = entry.picture_in_picture_seconds.saturating_add(seconds);
        }
        Some(host)
    }

    /// The recorded time for `host` (matched in lowercase). A host never reported has zero time.
    pub fn usage(&self, host: &str) -> HostUsage {
        self.hosts.get(&host.to_ascii_lowercase()).copied().unwrap_or_default()
    }

    /// Hosts ranked by total time, most first; ties are ordered by host name.
    pub fn ranked(&self) -> Vec<(&str, u32)> {
        let mut rows: Vec<(&str, u32)> = self.hosts.iter().map(|(h, u)| (h.as_str(), u.seconds)).collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        rows
    }

    /// Adds a domain to the block list. The domain is stored in lowercase, without a leading dot.
    pub fn block(&mut self, domain: &str) {
        let domain = domain.trim().trim_start_matches('.').to_ascii_lowercase();
        if !domain.is_empty() {
            self.blocked.insert(domain);
        }
    }

    /// Removes a domain from the block list.
    pub fn unblock(&mut self, domain: &str) {
        self.blocked.remove(&domain.trim().trim_start_matches('.').to_ascii_lowercase());
    }

    /// True when the host of `url` is a blocked domain or one of its subdomains. A URL with no host is not blocked.
    pub fn is_blocked(&self, url: &str) -> bool {
        let Some(host) = host_of(url) else { return false };
        self.blocked.iter().any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
    }

    /// The blocked domains, in order.
    pub fn blocked_domains(&self) -> impl Iterator<Item = &str> {
        self.blocked.iter().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_of_strips_scheme_user_port_and_path() {
        assert_eq!(host_of("https://User@Example.COM:8443/a/b?q=1#x").as_deref(), Some("example.com"));
        assert_eq!(host_of("example.org/path").as_deref(), Some("example.org"));
        assert_eq!(host_of("https://example.net./").as_deref(), Some("example.net"));
        assert_eq!(host_of("file:///etc/hosts").as_deref(), None);
        assert_eq!(host_of("").as_deref(), None);
    }

    #[test]
    fn report_adds_time_and_its_video_and_picture_in_picture_share() {
        let mut w = WebUsage::default();
        assert_eq!(w.report("https://a.example/one", 30, false, false).as_deref(), Some("a.example"));
        w.report("https://a.example/two", 20, true, false);
        w.report("https://A.example/three", 10, true, true);
        assert_eq!(w.usage("a.example"), HostUsage { seconds: 60, video_seconds: 30, picture_in_picture_seconds: 10 });
        assert_eq!(w.usage("never.example"), HostUsage::default());
        assert_eq!(w.report("file:///tmp", 5, false, false), None);
    }

    #[test]
    fn ranked_orders_by_time_then_host() {
        let mut w = WebUsage::default();
        w.report("https://b.example", 10, false, false);
        w.report("https://a.example", 10, false, false);
        w.report("https://c.example", 99, false, false);
        assert_eq!(w.ranked(), vec![("c.example", 99), ("a.example", 10), ("b.example", 10)]);
    }

    #[test]
    fn a_blocked_domain_covers_its_subdomains_but_not_look_alikes() {
        let mut w = WebUsage::default();
        w.block(".Example.com");
        assert!(w.is_blocked("https://example.com/"));
        assert!(w.is_blocked("https://www.EXAMPLE.com/page"));
        assert!(!w.is_blocked("https://notexample.com/"));
        assert!(!w.is_blocked("https://example.com.evil.test/"));
        assert!(!w.is_blocked("file:///tmp"));
        w.unblock("example.com");
        assert!(!w.is_blocked("https://example.com/"));
    }

    #[test]
    fn blank_domains_are_ignored() {
        let mut w = WebUsage::default();
        w.block("   ");
        assert_eq!(w.blocked_domains().count(), 0);
    }
}
