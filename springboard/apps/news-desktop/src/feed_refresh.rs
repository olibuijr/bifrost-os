//! The schedule of the background feed refresh. Each feed is reloaded once its last load is at least `interval` old, and
//! a feed never loaded is due at once. These are the pure rules: which feeds are due at a time, and how long the app
//! waits until the next one is due. The News loop sleeps for [`next_wait`], then reloads the [`due`] feeds.

use std::time::{Duration, Instant};

/// How long a loaded feed stays current before the background refresh reloads it.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(30 * 60);

/// Whether a feed last loaded at `last` is due at `now`. A feed that was never loaded (`None`) is due.
pub fn is_due(last: Option<Instant>, now: Instant, interval: Duration) -> bool {
    match last {
        None => true,
        Some(t) => now.saturating_duration_since(t) >= interval,
    }
}

/// The indexes of the feeds that are due at `now`, in feed order.
pub fn due(last: &[Option<Instant>], now: Instant, interval: Duration) -> Vec<usize> {
    last.iter().enumerate().filter(|(_, l)| is_due(**l, now, interval)).map(|(i, _)| i).collect()
}

/// How long to wait at `now` before the next feed is due: zero when one is due already, otherwise the least time left
/// before any feed expires. `None` when there are no feeds to refresh.
pub fn next_wait(last: &[Option<Instant>], now: Instant, interval: Duration) -> Option<Duration> {
    last.iter()
        .map(|l| match l {
            None => Duration::ZERO,
            Some(t) => interval.saturating_sub(now.saturating_duration_since(*t)),
        })
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: Duration = Duration::from_secs(3600);

    #[test]
    fn a_feed_is_due_once_its_last_load_is_a_full_interval_old() {
        let t0 = Instant::now();
        assert!(is_due(None, t0, HOUR), "a feed never loaded is due");
        assert!(!is_due(Some(t0), t0 + Duration::from_secs(3599), HOUR));
        assert!(is_due(Some(t0), t0 + HOUR, HOUR), "at exactly one interval the feed is due");
    }

    #[test]
    fn due_lists_the_feeds_that_need_a_reload_in_feed_order() {
        let t0 = Instant::now();
        let now = t0 + HOUR;
        let last = [Some(t0), None, Some(t0 + Duration::from_secs(1800)), Some(t0)];
        assert_eq!(due(&last, now, HOUR), vec![0, 1, 3]);
        assert_eq!(due(&[], now, HOUR), Vec::<usize>::new());
    }

    #[test]
    fn next_wait_is_the_time_until_the_first_feed_expires() {
        let t0 = Instant::now();
        let last = [Some(t0), Some(t0 + Duration::from_secs(1200))];
        assert_eq!(next_wait(&last, t0, HOUR), Some(HOUR));
        assert_eq!(next_wait(&last, t0 + Duration::from_secs(600), HOUR), Some(Duration::from_secs(3000)));
        assert_eq!(next_wait(&[Some(t0), None], t0, HOUR), Some(Duration::ZERO), "a feed never loaded is due now");
        assert_eq!(next_wait(&[], t0, HOUR), None);
        assert_eq!(next_wait(&[Some(t0)], t0 + Duration::from_secs(9000), HOUR), Some(Duration::ZERO), "an overdue feed waits zero");
    }

    #[test]
    fn the_default_interval_is_thirty_minutes() {
        assert_eq!(REFRESH_INTERVAL, Duration::from_secs(1800));
    }
}
