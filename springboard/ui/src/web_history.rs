//! Web history per app: each visit records the app's bundle identifier, the URL and the time it happened.
//!
//! A reader fetches the visits of every app, of one app, or of a time range. A deleter removes them the same way and
//! is told how many it removed. Fetches return visits oldest first, and a time range is half-open: a visit at `from`
//! is in the range and a visit at `to` is not. Times are seconds since the Unix epoch. Nothing is stored on disk and
//! nothing is fetched from the network; the caller owns the history value.

/// One recorded visit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Visit {
    pub bundle: String,
    pub url: String,
    /// Seconds since the Unix epoch.
    pub time: u64,
}

/// The web visits of all apps, kept in the order they were recorded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebHistory {
    visits: Vec<Visit>,
}

fn in_range(time: u64, from: u64, to: u64) -> bool {
    from <= time && time < to
}

impl WebHistory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one visit. An empty bundle identifier or URL records nothing. Returns whether the visit was kept.
    pub fn record(&mut self, bundle: &str, url: &str, time: u64) -> bool {
        if bundle.is_empty() || url.is_empty() {
            return false;
        }
        self.visits.push(Visit { bundle: bundle.into(), url: url.into(), time });
        true
    }

    /// The number of visits kept.
    pub fn len(&self) -> usize {
        self.visits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.visits.is_empty()
    }

    /// Every visit, oldest first. Visits at the same time keep their recorded order.
    pub fn fetch_all(&self) -> Vec<Visit> {
        let mut out = self.visits.clone();
        out.sort_by_key(|v| v.time);
        out
    }

    /// The visits of one app, oldest first.
    pub fn fetch_for(&self, bundle: &str) -> Vec<Visit> {
        self.fetch_all().into_iter().filter(|v| v.bundle == bundle).collect()
    }

    /// The visits whose time is in `from..to`, oldest first.
    pub fn fetch_during(&self, from: u64, to: u64) -> Vec<Visit> {
        self.fetch_all().into_iter().filter(|v| in_range(v.time, from, to)).collect()
    }

    /// Deletes every visit. Returns how many were removed.
    pub fn delete_all(&mut self) -> usize {
        let removed = self.visits.len();
        self.visits.clear();
        removed
    }

    /// Deletes the visits of one app. Returns how many were removed.
    pub fn delete_for(&mut self, bundle: &str) -> usize {
        let before = self.visits.len();
        self.visits.retain(|v| v.bundle != bundle);
        before - self.visits.len()
    }

    /// Deletes the visits whose time is in `from..to`, whatever app made them. Returns how many were removed.
    pub fn delete_during(&mut self, from: u64, to: u64) -> usize {
        let before = self.visits.len();
        self.visits.retain(|v| !in_range(v.time, from, to));
        before - self.visits.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> WebHistory {
        // Recorded out of time order on purpose: a.app at 30 and 20, b.app at 10.
        let mut h = WebHistory::new();
        h.record("a.app", "https://x.test/", 30);
        h.record("b.app", "https://y.test/", 10);
        h.record("a.app", "https://z.test/", 20);
        h
    }

    fn times(v: &[Visit]) -> Vec<u64> {
        v.iter().map(|x| x.time).collect()
    }

    #[test]
    fn fetch_all_is_oldest_first() {
        let h = sample();
        let all = h.fetch_all();
        assert_eq!(times(&all), vec![10, 20, 30]);
        assert_eq!(all[0].url, "https://y.test/");
        assert_eq!(all[2].bundle, "a.app");
        assert_eq!(h.len(), 3);
    }

    #[test]
    fn empty_bundle_or_url_records_nothing() {
        let mut h = WebHistory::new();
        assert!(!h.record("", "https://x.test/", 1));
        assert!(!h.record("a.app", "", 1));
        assert!(h.is_empty());
        assert!(h.record("a.app", "https://x.test/", 1));
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn fetch_for_returns_one_app() {
        let h = sample();
        let a = h.fetch_for("a.app");
        assert_eq!(times(&a), vec![20, 30]);
        assert_eq!(a[0].url, "https://z.test/");
        assert!(h.fetch_for("missing.app").is_empty());
    }

    #[test]
    fn fetch_during_is_half_open() {
        let h = sample();
        assert_eq!(times(&h.fetch_during(10, 30)), vec![10, 20]);
        assert_eq!(times(&h.fetch_during(11, 31)), vec![20, 30]);
        assert!(h.fetch_during(40, 50).is_empty());
    }

    #[test]
    fn delete_during_reports_the_count_removed() {
        let mut h = sample();
        assert_eq!(h.delete_during(10, 30), 2);
        assert_eq!(times(&h.fetch_all()), vec![30]);
        assert_eq!(h.delete_during(0, 0), 0);
    }

    #[test]
    fn delete_for_only_removes_that_app() {
        let mut h = sample();
        assert_eq!(h.delete_for("a.app"), 2);
        assert_eq!(h.fetch_all()[0].bundle, "b.app");
        assert_eq!(h.len(), 1);
        assert_eq!(h.delete_for("a.app"), 0);
    }

    #[test]
    fn delete_all_empties_the_history() {
        let mut h = sample();
        assert_eq!(h.delete_all(), 3);
        assert!(h.is_empty());
        assert_eq!(h.delete_all(), 0);
    }
}
