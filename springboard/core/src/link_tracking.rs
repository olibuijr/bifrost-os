//! Web privacy: strips the tracking parameters that links carry, and decides which third-party hosts are known trackers.
//! Both run on the machine, from lists kept in this file. Nothing is looked up, reported or fetched.
//!
//! - [`strip_tracking`] removes campaign and click-identifier query parameters (`utm_source`, `fbclid`, `gclid`, ...)
//!   from an http or https address and leaves every other part untouched, in the same order. The page still loads: these
//!   parameters exist only so the link's author can follow a person from one site to another.
//! - [`TrackerList`] holds hosts that are known to exist to track people. A request is blocked when its host is on the
//!   list and the page it was made from is another site, so visiting a listed site itself still works. A list the person
//!   edits (allow a host, add one) is applied on top of the built-in one.
//!
//! The address is parsed as text, not percent-decoded: a parameter name is compared as written, in lower case. IPv6
//! literals are not handled (the shared URL parser refuses them), and "same site" is a host-name suffix test, not the
//! public suffix list.

use crate::content_filter::parse_url;
use crate::dns_settings::{domain_matches, normalise_domain, valid_domain};
use std::collections::BTreeSet;

/// Query parameters that only identify a click or a campaign, compared in lower case.
pub const TRACKING_PARAMS: &[&str] = &[
    "fbclid", "gclid", "dclid", "gbraid", "wbraid", "msclkid", "mc_eid", "mc_cid", "igshid", "yclid", "twclid", "ttclid", "_hsenc", "_hsmi", "mkt_tok", "vero_id",
    "oly_anon_id", "oly_enc_id", "s_cid", "__s", "_openstat", "rb_clickid",
];

/// Parameter name prefixes that mark a campaign parameter (`utm_medium`, `pk_campaign`, ...).
pub const TRACKING_PREFIXES: &[&str] = &["utm_", "pk_", "mtm_", "matomo_", "hsa_", "stm_"];

/// Hosts that exist to track people across sites. A name covers itself and every name under it.
pub const BUILTIN_TRACKERS: &[&str] = &[
    "doubleclick.net",
    "googlesyndication.com",
    "google-analytics.com",
    "googletagmanager.com",
    "googletagservices.com",
    "adservice.google.com",
    "connect.facebook.net",
    "pixel.facebook.com",
    "scorecardresearch.com",
    "quantserve.com",
    "hotjar.com",
    "mixpanel.com",
    "segment.io",
    "amplitude.com",
    "criteo.com",
    "taboola.com",
    "outbrain.com",
    "adnxs.com",
    "bat.bing.com",
    "analytics.tiktok.com",
    "ads-twitter.com",
    "static.ads-twitter.com",
];

/// An address with its tracking parameters removed, and the names that were removed (in the order they appeared).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stripped {
    pub url: String,
    pub removed: Vec<String>,
}

/// True when the query parameter name `name` only identifies a click or a campaign.
pub fn is_tracking_param(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    TRACKING_PARAMS.contains(&lower.as_str()) || TRACKING_PREFIXES.iter().any(|p| lower.starts_with(p))
}

/// Removes the tracking parameters from an http or https address. Any other address, and an address with none, comes
/// back unchanged. The fragment is kept; a query left empty is dropped together with its `?`.
pub fn strip_tracking(url: &str) -> Stripped {
    let unchanged = || Stripped { url: url.to_string(), removed: Vec::new() };
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return unchanged();
    }
    let (before_fragment, fragment) = match url.split_once('#') {
        Some((a, f)) => (a, Some(f)),
        None => (url, None),
    };
    let Some((base, query)) = before_fragment.split_once('?') else { return unchanged() };
    let mut kept: Vec<&str> = Vec::new();
    let mut removed: Vec<String> = Vec::new();
    for piece in query.split('&') {
        if piece.is_empty() {
            continue;
        }
        let name = piece.split('=').next().unwrap_or("");
        if is_tracking_param(name) {
            removed.push(name.to_string());
        } else {
            kept.push(piece);
        }
    }
    if removed.is_empty() {
        return unchanged();
    }
    let mut out = base.to_string();
    if !kept.is_empty() {
        out.push('?');
        out.push_str(&kept.join("&"));
    }
    if let Some(f) = fragment {
        out.push('#');
        out.push_str(f);
    }
    Stripped { url: out, removed }
}

/// Why a host was not added to the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListError {
    /// The text is not a dotted host name.
    NotAHost,
}

/// The hosts treated as trackers: the built-in list plus the person's additions, minus the hosts they allowed.
#[derive(Clone, Debug)]
pub struct TrackerList {
    hosts: BTreeSet<String>,
    allowed: BTreeSet<String>,
    pub enabled: bool,
}

impl Default for TrackerList {
    fn default() -> Self {
        TrackerList { hosts: BUILTIN_TRACKERS.iter().map(|h| h.to_string()).collect(), allowed: BTreeSet::new(), enabled: true }
    }
}

impl TrackerList {
    /// The number of hosts on the list (built-in and added).
    pub fn len(&self) -> usize {
        self.hosts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hosts.is_empty()
    }

    /// Adds a tracker host; it also stops being allowed. `Err` when the text is not a dotted host name.
    pub fn add(&mut self, host: &str) -> Result<(), ListError> {
        let host = normalise_domain(host);
        if !host.contains('.') || !valid_domain(&host) {
            return Err(ListError::NotAHost);
        }
        self.allowed.remove(&host);
        self.hosts.insert(host);
        Ok(())
    }

    /// Lets requests to `host` (and the names under it) through even when a listed name covers them.
    pub fn allow(&mut self, host: &str) -> Result<(), ListError> {
        let host = normalise_domain(host);
        if !host.contains('.') || !valid_domain(&host) {
            return Err(ListError::NotAHost);
        }
        self.allowed.insert(host);
        Ok(())
    }

    /// True when `host` is covered by a listed name and not by an allowed one that is at least as specific.
    pub fn is_tracker(&self, host: &str) -> bool {
        let listed = self.hosts.iter().filter(|h| domain_matches(host, h)).map(|h| h.split('.').count()).max();
        let allowed = self.allowed.iter().filter(|h| domain_matches(host, h)).map(|h| h.split('.').count()).max();
        match (listed, allowed) {
            (Some(l), Some(a)) => l > a,
            (Some(_), None) => true,
            _ => false,
        }
    }

    /// True when a request to `request_url`, made by the page at `page_url`, should be blocked: protection is on, the
    /// request goes to a listed tracker, and the page is another site. A request that cannot be parsed is allowed.
    pub fn blocks(&self, page_url: &str, request_url: &str) -> bool {
        if !self.enabled {
            return false;
        }
        let (Some(request), Some(page)) = (parse_url(request_url), parse_url(page_url)) else { return false };
        let same_site = domain_matches(&request.host, &page.host) || domain_matches(&page.host, &request.host);
        !same_site && self.is_tracker(&request.host)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn campaign_and_click_parameters_are_removed_and_the_rest_kept_in_order() {
        let s = strip_tracking("https://shop.example/p?id=7&utm_source=mail&color=red&fbclid=abc123&UTM_Medium=x#reviews");
        assert_eq!(s.url, "https://shop.example/p?id=7&color=red#reviews");
        assert_eq!(s.removed, vec!["utm_source", "fbclid", "UTM_Medium"]);
    }

    #[test]
    fn a_query_left_empty_loses_its_question_mark() {
        assert_eq!(strip_tracking("http://a.example/?gclid=1&utm_campaign=2").url, "http://a.example/");
        assert_eq!(strip_tracking("http://a.example/x?msclkid=1#top").url, "http://a.example/x#top");
    }

    #[test]
    fn addresses_without_tracking_come_back_unchanged() {
        for u in ["https://a.example/?q=rust&page=2", "https://a.example/", "https://a.example/#utm_source=in-fragment", "ftp://a.example/?utm_source=x", "about:blank", "", "https:/"] {
            let s = strip_tracking(u);
            assert_eq!(s.url, u);
            assert!(s.removed.is_empty(), "{u}");
        }
    }

    #[test]
    fn a_parameter_is_matched_by_whole_name_or_by_campaign_prefix() {
        assert!(is_tracking_param("utm_term") && is_tracking_param("pk_kwd") && is_tracking_param("GCLID"));
        assert!(!is_tracking_param("utmost"), "a prefix must include the underscore");
        assert!(!is_tracking_param("q") && !is_tracking_param("fbclidx") && !is_tracking_param(""));
    }

    #[test]
    fn a_value_with_equals_signs_and_a_bare_name_are_handled() {
        let s = strip_tracking("https://a.example/?token=a=b&utm_source&flag&gclid=");
        assert_eq!(s.url, "https://a.example/?token=a=b&flag");
        assert_eq!(s.removed, vec!["utm_source", "gclid"]);
    }

    #[test]
    fn a_listed_tracker_is_blocked_on_another_site_but_not_on_itself() {
        let list = TrackerList::default();
        assert!(list.blocks("https://news.example/story", "https://www.google-analytics.com/collect?v=1"));
        assert!(list.blocks("https://news.example/", "https://stats.g.doubleclick.net/pixel"), "names under a listed host are covered");
        assert!(!list.blocks("https://news.example/", "https://news.example/app.js"));
        assert!(!list.blocks("https://hotjar.com/about", "https://static.hotjar.com/c.js"), "visiting the site itself works");
        assert!(!list.blocks("https://news.example/", "https://notdoubleclick.net/x"), "only whole labels match");
    }

    #[test]
    fn the_person_can_add_a_tracker_and_allow_one_and_switch_it_all_off() {
        let mut list = TrackerList::default();
        assert!(!list.blocks("https://a.example/", "https://beacon.metrics.example/x"));
        list.add("Metrics.Example.").unwrap();
        assert!(list.blocks("https://a.example/", "https://beacon.metrics.example/x"));
        list.allow("beacon.metrics.example").unwrap();
        assert!(!list.blocks("https://a.example/", "https://beacon.metrics.example/x"), "the more specific allow wins");
        assert!(list.blocks("https://a.example/", "https://other.metrics.example/x"));
        list.add("beacon.metrics.example").unwrap();
        assert!(list.blocks("https://a.example/", "https://beacon.metrics.example/x"), "adding it again lifts the allow");
        list.enabled = false;
        assert!(!list.blocks("https://a.example/", "https://beacon.metrics.example/x"));
        assert_eq!(list.add("localhost"), Err(ListError::NotAHost));
        assert_eq!(list.allow("bad host.example"), Err(ListError::NotAHost));
    }

    #[test]
    fn an_address_that_cannot_be_parsed_is_not_blocked() {
        let list = TrackerList::default();
        assert!(!list.blocks("https://a.example/", "not a url"));
        assert!(!list.blocks("", "https://doubleclick.net/x"));
        assert!(!list.is_empty() && list.len() == BUILTIN_TRACKERS.len());
    }
}
