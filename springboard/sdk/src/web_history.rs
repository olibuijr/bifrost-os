//! Browsing history for apps: one entry per page visit, with the app bundle that made it and the profile it belongs
//! to. An app reads entries filtered by bundle, profile and a time range, and deletes them by the same filters. Times
//! are Unix seconds and a range is half open (`from` included, `to` excluded). The store is in memory; the browser
//! that owns it writes it to its own data directory.

/// One visit to a page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    /// The full URL that was shown.
    pub url: String,
    /// The title of the page when it was shown, if the browser knew one.
    pub title: String,
    /// The bundle identifier of the app that made the visit.
    pub bundle: String,
    /// The browsing profile the visit belongs to. Empty for the default profile.
    pub profile: String,
    /// When the visit started, in Unix seconds.
    pub visited_at: i64,
}

/// Which entries a query or a delete covers. Each field that is `None` matches everything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HistoryFilter {
    pub bundle: Option<String>,
    pub profile: Option<String>,
    /// The first second included.
    pub from: Option<i64>,
    /// The first second excluded.
    pub until: Option<i64>,
}

impl HistoryFilter {
    fn matches(&self, entry: &HistoryEntry) -> bool {
        self.bundle.as_deref().is_none_or(|b| b == entry.bundle)
            && self.profile.as_deref().is_none_or(|p| p == entry.profile)
            && self.from.is_none_or(|from| entry.visited_at >= from)
            && self.until.is_none_or(|until| entry.visited_at < until)
    }
}

/// The browsing history held by the browser.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebHistory {
    entries: Vec<HistoryEntry>,
}

impl WebHistory {
    /// Adds a visit. Entries keep the order they were added in.
    pub fn add(&mut self, entry: HistoryEntry) {
        self.entries.push(entry);
    }

    /// The number of stored visits.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The visits that match `filter`, in the order they were added.
    pub fn fetch(&self, filter: &HistoryFilter) -> Vec<&HistoryEntry> {
        self.entries.iter().filter(|e| filter.matches(e)).collect()
    }

    /// Removes the visits that match `filter` and returns how many were removed.
    pub fn delete(&mut self, filter: &HistoryFilter) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| !filter.matches(e));
        before - self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visit(url: &str, bundle: &str, profile: &str, at: i64) -> HistoryEntry {
        HistoryEntry { url: url.into(), title: String::new(), bundle: bundle.into(), profile: profile.into(), visited_at: at }
    }

    fn sample() -> WebHistory {
        let mut h = WebHistory::default();
        h.add(visit("https://a.example/", "org.browser", "", 100));
        h.add(visit("https://b.example/", "org.browser", "work", 200));
        h.add(visit("https://c.example/", "org.other", "", 300));
        h
    }

    #[test]
    fn an_empty_filter_fetches_everything_in_order() {
        let h = sample();
        let urls: Vec<&str> = h.fetch(&HistoryFilter::default()).iter().map(|e| e.url.as_str()).collect();
        assert_eq!(urls, ["https://a.example/", "https://b.example/", "https://c.example/"]);
    }

    #[test]
    fn filters_by_bundle_profile_and_range() {
        let h = sample();
        let by_bundle = HistoryFilter { bundle: Some("org.browser".into()), ..Default::default() };
        assert_eq!(h.fetch(&by_bundle).len(), 2);
        let by_profile = HistoryFilter { profile: Some("work".into()), ..Default::default() };
        assert_eq!(h.fetch(&by_profile)[0].url, "https://b.example/");
        let range = HistoryFilter { from: Some(200), until: Some(300), ..Default::default() };
        assert_eq!(h.fetch(&range).len(), 1, "the start is included and the end is not");
    }

    #[test]
    fn delete_removes_only_the_matching_visits() {
        let mut h = sample();
        let by_bundle = HistoryFilter { bundle: Some("org.browser".into()), ..Default::default() };
        assert_eq!(h.delete(&by_bundle), 2);
        assert_eq!(h.len(), 1);
        assert_eq!(h.delete(&by_bundle), 0);
        assert_eq!(h.fetch(&HistoryFilter::default())[0].bundle, "org.other");
    }

    #[test]
    fn delete_by_range_keeps_the_visits_outside_it() {
        let mut h = sample();
        let range = HistoryFilter { from: Some(0), until: Some(250), ..Default::default() };
        assert_eq!(h.delete(&range), 2);
        assert!(!h.is_empty());
        assert_eq!(h.fetch(&HistoryFilter::default())[0].url, "https://c.example/");
    }
}
