//! App Usage: a launch-history ranker that suggests apps from how often and how recently each one was opened.
//!
//! Every launch adds one point to the app's score. A score halves each `HALF_LIFE_S` seconds, so last week's use
//! outranks a habit from months ago. Apps with equal scores keep the order in which they were first launched.
//! The model reads no clock: the caller passes every launch and query time in seconds.

/// Seconds after which a launch counts half as much (one week).
pub const HALF_LIFE_S: f64 = 7.0 * 24.0 * 3600.0;

/// The launch history of every app the user has opened, kept in first-launch order.
#[derive(Debug, Default)]
pub struct AppUsage {
    entries: Vec<Entry>,
}

#[derive(Debug)]
struct Entry {
    key: String,
    /// The score as it stood at `at`, before any decay since then.
    score: f64,
    at: f64,
}

impl Entry {
    fn decayed(&self, now: f64) -> f64 {
        let age = (now - self.at).max(0.0);
        self.score * 0.5f64.powf(age / HALF_LIFE_S)
    }
}

impl AppUsage {
    /// Records one launch of the app `key` at time `at` (seconds).
    pub fn record_launch(&mut self, key: &str, at: f64) {
        match self.entries.iter_mut().find(|e| e.key == key) {
            Some(entry) => {
                entry.score = entry.decayed(at) + 1.0;
                entry.at = at;
            }
            None => self.entries.push(Entry { key: key.to_string(), score: 1.0, at }),
        }
    }

    /// The app's score at time `now`: zero when it was never launched.
    pub fn score(&self, key: &str, now: f64) -> f64 {
        self.entries.iter().find(|e| e.key == key).map_or(0.0, |e| e.decayed(now))
    }

    /// Up to `limit` apps with a non-zero score at `now`, highest score first.
    pub fn suggest(&self, now: f64, limit: usize) -> Vec<String> {
        let mut ranked: Vec<(&Entry, f64)> = self.entries.iter().map(|e| (e, e.decayed(now))).filter(|(_, score)| *score > 0.0).collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        ranked.into_iter().take(limit).map(|(entry, _)| entry.key.clone()).collect()
    }
}
