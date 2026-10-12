//! Web-usage history: the store of visits that `web_usage` controllers report, and a scoped handle that fetches and
//! deletes visits by bundle identifier, profile identifier, address or time interval. The handle borrows the store, so a
//! deletion through one handle is seen by every other handle. Pure model: the caller supplies the times and the store.

use crate::web_usage::{canonical_url, UsageRecord};
use std::collections::BTreeSet;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A bundle or profile identifier is empty.
    EmptyIdentifier,
    /// The interval ends before it starts.
    InvertedInterval,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::EmptyIdentifier => write!(f, "the identifier is empty"),
            Error::InvertedInterval => write!(f, "the interval ends before it starts"),
        }
    }
}

impl std::error::Error for Error {}

/// A half-open interval `[start, end)` of Unix seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeInterval {
    start: i64,
    end: i64,
}

impl TimeInterval {
    /// An interval from `start` up to, not including, `end`. An empty interval (`start == end`) is allowed.
    pub fn new(start: i64, end: i64) -> Result<Self, Error> {
        if end < start {
            return Err(Error::InvertedInterval);
        }
        Ok(TimeInterval { start, end })
    }

    pub fn start(&self) -> i64 {
        self.start
    }

    pub fn end(&self) -> i64 {
        self.end
    }

    pub fn contains(&self, at: i64) -> bool {
        self.start <= at && at < self.end
    }
}

/// One stored visit. The address is kept canonical.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Visit {
    bundle_id: String,
    profile_id: Option<String>,
    url: String,
    at: i64,
}

/// Every visit reported for every bundle and profile.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebHistoryStore {
    visits: Vec<Visit>,
}

impl WebHistoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Keeps a visit reported by a controller.
    pub fn record(&mut self, usage: &UsageRecord) {
        self.visits.push(Visit {
            bundle_id: usage.bundle_id.clone(),
            profile_id: usage.profile_id.clone(),
            url: canonical_url(&usage.url),
            at: usage.at,
        });
    }

    pub fn len(&self) -> usize {
        self.visits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.visits.is_empty()
    }
}

fn clean(id: &str) -> Result<String, Error> {
    let id = id.trim();
    if id.is_empty() {
        return Err(Error::EmptyIdentifier);
    }
    Ok(id.to_string())
}

/// Whether a visit belongs to the scope. A `None` bundle or profile matches every value of that field.
fn in_scope(visit: &Visit, bundle: Option<&str>, profile: Option<&str>) -> bool {
    bundle.map_or(true, |b| b == visit.bundle_id.as_str())
        && profile.map_or(true, |p| visit.profile_id.as_deref() == Some(p))
}

/// A scoped view of a `WebHistoryStore`.
#[derive(Debug)]
pub struct WebHistory<'a> {
    store: &'a mut WebHistoryStore,
    bundle_id: Option<String>,
    profile_id: Option<String>,
}

impl<'a> WebHistory<'a> {
    /// A handle over the visits of one bundle, in every profile.
    pub fn for_bundle(store: &'a mut WebHistoryStore, bundle_id: &str) -> Result<Self, Error> {
        Ok(WebHistory { store, bundle_id: Some(clean(bundle_id)?), profile_id: None })
    }

    /// A handle over the visits of one profile, from every bundle.
    pub fn for_profile(store: &'a mut WebHistoryStore, profile_id: &str) -> Result<Self, Error> {
        Ok(WebHistory { store, bundle_id: None, profile_id: Some(clean(profile_id)?) })
    }

    /// A handle over the visits of one bundle within one profile.
    pub fn for_bundle_and_profile(
        store: &'a mut WebHistoryStore,
        bundle_id: &str,
        profile_id: &str,
    ) -> Result<Self, Error> {
        Ok(WebHistory {
            store,
            bundle_id: Some(clean(bundle_id)?),
            profile_id: Some(clean(profile_id)?),
        })
    }

    /// The addresses of every visit in scope, sorted and without repeats.
    pub fn fetch_all_history(&self) -> BTreeSet<String> {
        self.collect_where(|_| true)
    }

    /// The addresses of the visits in scope that fall inside `interval`.
    pub fn fetch_history_during(&self, interval: TimeInterval) -> BTreeSet<String> {
        self.collect_where(|v| interval.contains(v.at))
    }

    /// Deletes every visit in scope. Returns how many were removed.
    pub fn delete_all_history(&mut self) -> usize {
        self.delete_where(|_| true)
    }

    /// Deletes the visits in scope that fall inside `interval`. Returns how many were removed.
    pub fn delete_history_during(&mut self, interval: TimeInterval) -> usize {
        self.delete_where(|v| interval.contains(v.at))
    }

    /// Deletes every visit in scope to `url`, compared in canonical form. Returns how many were removed.
    pub fn delete_history_for(&mut self, url: &str) -> usize {
        let url = canonical_url(url);
        self.delete_where(|v| v.url == url)
    }

    fn collect_where<F: Fn(&Visit) -> bool>(&self, matches: F) -> BTreeSet<String> {
        self.store
            .visits
            .iter()
            .filter(|&v| in_scope(v, self.bundle_id.as_deref(), self.profile_id.as_deref()) && matches(v))
            .map(|v| v.url.clone())
            .collect()
    }

    fn delete_where<F: Fn(&Visit) -> bool>(&mut self, matches: F) -> usize {
        let bundle = self.bundle_id.clone();
        let profile = self.profile_id.clone();
        let visits = &mut self.store.visits;
        let before = visits.len();
        visits.retain(|v| !(in_scope(v, bundle.as_deref(), profile.as_deref()) && matches(v)));
        before - visits.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(bundle: &str, profile: Option<&str>, url: &str, at: i64) -> UsageRecord {
        UsageRecord {
            bundle_id: bundle.to_string(),
            profile_id: profile.map(|p| p.to_string()),
            url: url.to_string(),
            at,
        }
    }

    /// Visits: A/p1 `a/x` at 10, A/p2 `a/y` at 20, B/p1 `b/` at 30.
    fn sample() -> WebHistoryStore {
        let mut store = WebHistoryStore::new();
        store.record(&usage("A", Some("p1"), "https://a.test/x", 10));
        store.record(&usage("A", Some("p2"), "https://a.test/y", 20));
        store.record(&usage("B", Some("p1"), "https://b.test/", 30));
        store
    }

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_bundle_handle_sees_only_that_bundle_in_every_profile() {
        let mut store = sample();
        let handle = WebHistory::for_bundle(&mut store, "A").unwrap();
        assert_eq!(handle.fetch_all_history(), set(&["https://a.test/x", "https://a.test/y"]));
    }

    #[test]
    fn a_profile_handle_sees_that_profile_from_every_bundle() {
        let mut store = sample();
        let handle = WebHistory::for_profile(&mut store, "p1").unwrap();
        assert_eq!(handle.fetch_all_history(), set(&["https://a.test/x", "https://b.test/"]));
    }

    #[test]
    fn an_interval_is_half_open_and_fetch_respects_it() {
        let mut store = sample();
        let handle = WebHistory::for_profile(&mut store, "p1").unwrap();
        // [10, 30) keeps the visits at 10 and drops the one at 30.
        let during = handle.fetch_history_during(TimeInterval::new(10, 30).unwrap());
        assert_eq!(during, set(&["https://a.test/x"]));
        let empty = handle.fetch_history_during(TimeInterval::new(10, 10).unwrap());
        assert!(empty.is_empty());
    }

    #[test]
    fn deleting_during_an_interval_removes_only_scoped_visits_inside_it() {
        let mut store = sample();
        let removed = {
            let mut handle = WebHistory::for_bundle(&mut store, "A").unwrap();
            handle.delete_history_during(TimeInterval::new(10, 21).unwrap())
        };
        // Both A visits (10 and 20) are inside [10, 21); the B visit at 30 is out of scope and stays.
        assert_eq!(removed, 2);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn deleting_for_an_address_compares_canonical_forms() {
        let mut store = sample();
        let removed = {
            let mut handle = WebHistory::for_profile(&mut store, "p1").unwrap();
            handle.delete_history_for("HTTPS://B.test/")
        };
        assert_eq!(removed, 1);
        let left = WebHistory::for_bundle(&mut store, "B").unwrap().fetch_all_history();
        assert!(left.is_empty());
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn delete_all_in_a_scope_leaves_other_scopes_and_is_seen_by_other_handles() {
        let mut store = sample();
        {
            let mut handle = WebHistory::for_bundle(&mut store, "A").unwrap();
            assert_eq!(handle.delete_all_history(), 2);
            assert_eq!(handle.delete_all_history(), 0);
        }
        // A new handle on the same store sees the deletion, and the B visit is still there.
        let all = WebHistory::for_profile(&mut store, "p1").unwrap().fetch_all_history();
        assert_eq!(all, set(&["https://b.test/"]));
    }

    #[test]
    fn empty_identifiers_and_inverted_intervals_are_refused() {
        let mut store = WebHistoryStore::new();
        assert_eq!(WebHistory::for_bundle(&mut store, "  ").err(), Some(Error::EmptyIdentifier));
        assert_eq!(
            WebHistory::for_bundle_and_profile(&mut store, "A", "").err(),
            Some(Error::EmptyIdentifier)
        );
        assert_eq!(TimeInterval::new(5, 1), Err(Error::InvertedInterval));
    }
}
