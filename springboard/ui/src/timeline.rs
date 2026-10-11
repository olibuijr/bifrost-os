//! Timelines for glanceable content (`Timeline`, `TimelineEntry`, `TimelineReloadPolicy`): dated values, the entry on
//! screen at a given time, and the instant the owner should be asked for a new timeline. Dates are seconds since
//! 1970-01-01 as `f64`. Nothing here reads a clock, so callers pass `now`.

/// One dated value. It is shown from its date until the next entry's date.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry<T> {
    pub date: f64,
    pub value: T,
}

/// When the owner of a timeline should be asked for a new one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReloadPolicy {
    /// After the date of the last entry passes.
    AtEnd,
    /// At this date.
    After(f64),
    /// Only when the owner reports that a new timeline is ready.
    Never,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Timeline<T> {
    entries: Vec<Entry<T>>,
    policy: ReloadPolicy,
}

impl<T> Timeline<T> {
    /// Entries are sorted by date. Entries with equal dates keep the order they were given in.
    pub fn new(mut entries: Vec<Entry<T>>, policy: ReloadPolicy) -> Self {
        entries.sort_by(|a, b| a.date.total_cmp(&b.date));
        Self { entries, policy }
    }

    pub fn entries(&self) -> &[Entry<T>] {
        &self.entries
    }

    pub fn policy(&self) -> ReloadPolicy {
        self.policy
    }

    /// The entry on screen at `now`: the latest one whose date has passed. Before the first date the first entry
    /// is shown, as a widget shows its first entry before the timeline starts. None when the timeline is empty.
    pub fn entry_at(&self, now: f64) -> Option<&Entry<T>> {
        let passed = self.entries.partition_point(|e| e.date <= now);
        self.entries.get(passed.saturating_sub(1))
    }

    /// The instant, not earlier than `now`, at which a new timeline should be requested. None for `Never`, and for
    /// `AtEnd` on an empty timeline.
    pub fn next_reload(&self, now: f64) -> Option<f64> {
        match self.policy {
            ReloadPolicy::AtEnd => self.entries.last().map(|e| e.date.max(now)),
            ReloadPolicy::After(date) => Some(date.max(now)),
            ReloadPolicy::Never => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(date: f64, value: &'static str) -> Entry<&'static str> {
        Entry { date, value }
    }

    #[test]
    fn entries_are_sorted_by_date_and_equal_dates_keep_their_order() {
        let t = Timeline::new(vec![at(30.0, "c"), at(10.0, "a"), at(10.0, "b")], ReloadPolicy::Never);
        let order: Vec<_> = t.entries().iter().map(|e| e.value).collect();
        assert_eq!(order, ["a", "b", "c"]);
    }

    #[test]
    fn the_entry_on_screen_is_the_latest_one_whose_date_has_passed() {
        let t = Timeline::new(vec![at(10.0, "a"), at(20.0, "b"), at(30.0, "c")], ReloadPolicy::AtEnd);
        assert_eq!(t.entry_at(10.0).map(|e| e.value), Some("a"), "an entry shows from its own date");
        assert_eq!(t.entry_at(19.9).map(|e| e.value), Some("a"));
        assert_eq!(t.entry_at(20.0).map(|e| e.value), Some("b"));
        assert_eq!(t.entry_at(10_000.0).map(|e| e.value), Some("c"), "the last entry stays until a reload");
    }

    #[test]
    fn before_the_first_date_the_first_entry_shows_and_an_empty_timeline_shows_nothing() {
        let t = Timeline::new(vec![at(10.0, "a"), at(20.0, "b")], ReloadPolicy::Never);
        assert_eq!(t.entry_at(-5.0).map(|e| e.value), Some("a"));
        let empty: Timeline<&str> = Timeline::new(vec![], ReloadPolicy::AtEnd);
        assert!(empty.entry_at(0.0).is_none());
    }

    #[test]
    fn at_end_reloads_when_the_last_entry_date_passes() {
        let t = Timeline::new(vec![at(10.0, "a"), at(50.0, "b")], ReloadPolicy::AtEnd);
        assert_eq!(t.next_reload(0.0), Some(50.0));
        assert_eq!(t.next_reload(70.0), Some(70.0), "a passed reload date is due now, never in the past");
        let empty: Timeline<&str> = Timeline::new(vec![], ReloadPolicy::AtEnd);
        assert_eq!(empty.next_reload(0.0), None);
    }

    #[test]
    fn a_fixed_reload_date_is_kept_and_never_means_no_reload() {
        let t = Timeline::new(vec![at(10.0, "a")], ReloadPolicy::After(300.0));
        assert_eq!(t.next_reload(100.0), Some(300.0));
        assert_eq!(t.next_reload(400.0), Some(400.0));
        let manual = Timeline::new(vec![at(10.0, "a")], ReloadPolicy::Never);
        assert_eq!(manual.next_reload(100.0), None);
        assert_eq!(manual.policy(), ReloadPolicy::Never);
    }
}
