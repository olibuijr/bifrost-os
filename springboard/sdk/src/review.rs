//! Review prompts (`RequestReviewAction`): an app asks for the store's rating prompt, and the store decides whether to show
//! it. This module is the app's own limit on how often it asks, so a prompt is not requested after every launch. It keeps
//! the days of past asks and answers whether a new ask fits the policy. The prompt itself is not built yet, so
//! [`ReviewRequests::request`] only records the decision. Days count from 1970-01-01.

/// How often an app may ask: at least `min_gap` days after its last ask, and at most `max_asks` asks in any `window` days.
/// These defaults are BifrOSt's choice, not a figure from the store documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub min_gap: i64,
    pub max_asks: usize,
    pub window: i64,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { min_gap: 120, max_asks: 3, window: 365 }
    }
}

/// The days on which this app asked for a review, kept by the app and sorted oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReviewRequests {
    days: Vec<i64>,
}

impl ReviewRequests {
    pub fn new() -> Self {
        Self::default()
    }

    /// Restores the days saved by the app, in any order.
    pub fn from_days(mut days: Vec<i64>) -> Self {
        days.sort_unstable();
        ReviewRequests { days }
    }

    pub fn days(&self) -> &[i64] {
        &self.days
    }

    /// Whether an ask on `day` fits the policy.
    pub fn allowed(&self, day: i64, policy: Policy) -> bool {
        if let Some(&last) = self.days.last() {
            if day - last < policy.min_gap {
                return false;
            }
        }
        let recent = self.days.iter().filter(|&&d| day - d < policy.window).count();
        recent < policy.max_asks
    }

    /// Asks on `day` when the policy allows it, and records the ask. True when the ask was made.
    pub fn request(&mut self, day: i64, policy: Policy) -> bool {
        if !self.allowed(day, policy) {
            return false;
        }
        self.days.push(day);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_ask_is_allowed_and_recorded() {
        let mut r = ReviewRequests::new();
        assert!(r.request(100, Policy::default()));
        assert_eq!(r.days(), &[100]);
    }

    #[test]
    fn the_minimum_gap_blocks_an_ask_that_comes_too_soon() {
        let mut r = ReviewRequests::new();
        let p = Policy::default();
        assert!(r.request(100, p));
        assert!(!r.request(150, p), "50 days is less than the 120-day gap");
        assert!(!r.request(100, p), "the same day is never a second ask");
        assert!(r.request(220, p));
        assert_eq!(r.days(), &[100, 220]);
    }

    #[test]
    fn max_asks_caps_the_asks_inside_the_window() {
        let mut r = ReviewRequests::new();
        let p = Policy { min_gap: 0, max_asks: 2, window: 30 };
        assert!(r.request(0, p));
        assert!(r.request(1, p));
        assert!(!r.request(2, p), "two asks already fall in the window");
        assert!(r.request(30, p), "the ask on day 0 has left the 30-day window");
    }

    #[test]
    fn the_window_counts_only_asks_from_its_last_days() {
        let mut r = ReviewRequests::new();
        let p = Policy { min_gap: 0, max_asks: 1, window: 10 };
        assert!(r.request(0, p));
        assert!(!r.request(9, p), "day 0 is still inside a 10-day window on day 9");
        assert!(r.request(10, p), "day 0 is outside the window on day 10");
    }

    #[test]
    fn restored_days_are_sorted_and_checked_against_the_policy() {
        let r = ReviewRequests::from_days(vec![300, 100]);
        assert_eq!(r.days(), &[100, 300]);
        let p = Policy::default();
        assert!(!r.allowed(350, p), "only 50 days after the last ask");
        assert!(r.allowed(430, p), "130 days after the last ask, and only two asks fall inside the year");
    }

    #[test]
    fn zero_max_asks_never_asks() {
        let mut r = ReviewRequests::new();
        assert!(!r.request(5, Policy { min_gap: 0, max_asks: 0, window: 365 }));
        assert!(r.days().is_empty());
    }
}
