//! Web page usage: the state of one page as a usage reporter sees it.
//!
//! The reporter holds the page's URL, the app's bundle identifier and profile identifier, whether the page is playing
//! video or showing picture-in-picture, and whether recording is suppressed. A page is blocked when the block filter
//! denies its URL. Reporting writes one visit to a web history store, and only when recording is not suppressed and
//! both the URL and the bundle identifier are known. Nothing here reads a page or connects to a network.

use crate::url_filter::{UrlFilter, Verdict};
use crate::web_history::WebHistory;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebpageUsage {
    url: Option<String>,
    bundle: Option<String>,
    profile: Option<String>,
    playing_video: bool,
    picture_in_picture: bool,
    suppress_recording: bool,
}

fn optional(text: &str) -> Option<String> {
    if text.is_empty() {
        None
    } else {
        Some(text.into())
    }
}

impl WebpageUsage {
    pub fn new() -> Self {
        Self::default()
    }

    /// The page's URL, or `None` when none is set.
    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    /// Sets the page's URL. An empty string clears it.
    pub fn set_url(&mut self, url: &str) {
        self.url = optional(url);
    }

    pub fn bundle_identifier(&self) -> Option<&str> {
        self.bundle.as_deref()
    }

    /// Sets the bundle identifier of the app showing the page. An empty string clears it.
    pub fn set_bundle_identifier(&mut self, bundle: &str) {
        self.bundle = optional(bundle);
    }

    pub fn profile_identifier(&self) -> Option<&str> {
        self.profile.as_deref()
    }

    /// Sets the profile the page belongs to. An empty string clears it.
    pub fn set_profile_identifier(&mut self, profile: &str) {
        self.profile = optional(profile);
    }

    pub fn playing_video(&self) -> bool {
        self.playing_video
    }

    pub fn set_playing_video(&mut self, on: bool) {
        self.playing_video = on;
    }

    pub fn picture_in_picture(&self) -> bool {
        self.picture_in_picture
    }

    pub fn set_picture_in_picture(&mut self, on: bool) {
        self.picture_in_picture = on;
    }

    pub fn suppress_recording(&self) -> bool {
        self.suppress_recording
    }

    pub fn set_suppress_recording(&mut self, on: bool) {
        self.suppress_recording = on;
    }

    /// Whether the block filter denies the current URL. A page with no URL is not blocked.
    pub fn is_blocked(&self, filter: &UrlFilter) -> bool {
        match &self.url {
            Some(url) => filter.verdict(url) == Verdict::Deny,
            None => false,
        }
    }

    /// Writes one visit to `history` at `time`. Returns whether a visit was written. Nothing is written while
    /// recording is suppressed, or when the URL or the bundle identifier is unknown.
    pub fn report(&self, history: &mut WebHistory, time: u64) -> bool {
        if self.suppress_recording {
            return false;
        }
        match (&self.url, &self.bundle) {
            (Some(url), Some(bundle)) => history.record(bundle, url, time),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_empty_and_off() {
        let p = WebpageUsage::new();
        assert_eq!(p.url(), None);
        assert_eq!(p.bundle_identifier(), None);
        assert_eq!(p.profile_identifier(), None);
        assert!(!p.playing_video());
        assert!(!p.picture_in_picture());
        assert!(!p.suppress_recording());
    }

    #[test]
    fn flags_and_identifiers_round_trip() {
        let mut p = WebpageUsage::new();
        p.set_playing_video(true);
        p.set_picture_in_picture(true);
        p.set_profile_identifier("profile.one");
        p.set_bundle_identifier("app.reader");
        assert!(p.playing_video());
        assert!(p.picture_in_picture());
        assert_eq!(p.profile_identifier(), Some("profile.one"));
        assert_eq!(p.bundle_identifier(), Some("app.reader"));
    }

    #[test]
    fn blocked_follows_the_block_filter() {
        let mut filter = UrlFilter::new();
        filter.deny("*://*.tracker.net/*").unwrap();
        let mut p = WebpageUsage::new();
        assert!(!p.is_blocked(&filter), "no URL, not blocked");
        p.set_url("http://a.tracker.net/");
        assert!(p.is_blocked(&filter));
        p.set_url("https://news.example/");
        assert!(!p.is_blocked(&filter));
    }

    #[test]
    fn report_writes_one_visit_when_url_and_bundle_are_known() {
        let mut p = WebpageUsage::new();
        let mut h = WebHistory::new();
        p.set_url("https://news.example/a");
        assert!(!p.report(&mut h, 100), "no bundle identifier yet");
        assert!(h.is_empty());
        p.set_bundle_identifier("app.reader");
        assert!(p.report(&mut h, 100));
        let all = h.fetch_all();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].bundle, "app.reader");
        assert_eq!(all[0].url, "https://news.example/a");
        assert_eq!(all[0].time, 100);
    }

    #[test]
    fn suppressed_recording_writes_nothing() {
        let mut p = WebpageUsage::new();
        p.set_url("https://news.example/");
        p.set_bundle_identifier("app.reader");
        p.set_suppress_recording(true);
        let mut h = WebHistory::new();
        assert!(!p.report(&mut h, 5));
        assert!(h.is_empty());
        p.set_suppress_recording(false);
        assert!(p.report(&mut h, 5));
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn empty_string_clears_the_url() {
        let mut p = WebpageUsage::new();
        p.set_url("https://a.test/");
        p.set_bundle_identifier("app.reader");
        p.set_url("");
        assert_eq!(p.url(), None);
        let mut h = WebHistory::new();
        assert!(!p.report(&mut h, 1));
    }
}
