//! What the app knows about prices, and when it asks for more. The feed holds the quotes and the histories (from
//! the disk cache or from the network), the times they were fetched and the failures. It decides when a fetch is due
//! and applies the results that background jobs hand back. No UI, no disk and no network are touched here, so the
//! rules can be checked with plain values.

use crate::data;
use crate::model::{Bar, Quote};
use std::collections::{HashMap, HashSet};

/// Quotes older than this are fetched again (seconds). The service's free quotes are delayed anyway.
pub const QUOTE_TTL_S: f64 = 15.0 * 60.0;
/// Daily history is fetched again after this long (seconds).
pub const HISTORY_TTL_S: f64 = 6.0 * 3600.0;
/// After a failed fetch, nothing is asked for again until this long has passed (seconds).
pub const RETRY_S: f64 = 60.0;

/// A finished background job, waiting for the app loop.
#[derive(Debug)]
pub enum Done {
    Quotes(Result<Vec<Quote>, String>),
    History { symbol: String, result: Result<Vec<Bar>, String> },
}

#[derive(Debug, Default)]
pub struct Feed {
    pub quotes: HashMap<String, Quote>,
    /// Unix seconds of the last good quote answer (cached or fetched).
    pub quotes_at: Option<f64>,
    /// The message of the last quote failure, cleared by the next good answer.
    pub quotes_error: Option<String>,
    pub quotes_pending: bool,
    /// A symbol was added since the last quote request, so the next request is due now.
    pub quotes_wanted: bool,
    pub history: HashMap<String, Vec<Bar>>,
    /// Unix seconds of the last good history answer of each symbol.
    pub history_at: HashMap<String, f64>,
    /// The message of the last history failure, per symbol.
    pub history_failed: HashMap<String, String>,
    history_pending: HashSet<String>,
    /// Unix seconds before which no failed fetch is retried.
    pub retry_at: f64,
}

impl Feed {
    /// Whether a quote request is due at `now` (unix seconds) for a non-empty watchlist.
    pub fn quotes_due(&self, now: f64) -> bool {
        if self.quotes_pending || now < self.retry_at {
            return false;
        }
        self.quotes_wanted || self.quotes_at.map_or(true, |at| now - at >= QUOTE_TTL_S)
    }

    /// Whether a history request is due for `symbol` at `now`.
    pub fn history_due(&self, symbol: &str, now: f64) -> bool {
        if self.history_pending.contains(symbol) || now < self.retry_at {
            return false;
        }
        self.history_at.get(symbol).map_or(true, |at| now - at >= HISTORY_TTL_S)
    }

    /// Marks a quote request as started (it is no longer wanted until a symbol is added again).
    pub fn begin_quotes(&mut self) {
        self.quotes_pending = true;
        self.quotes_wanted = false;
    }

    /// Marks a history request for `symbol` as started.
    pub fn begin_history(&mut self, symbol: &str) {
        self.history_pending.insert(symbol.to_string());
    }

    /// Loads quotes from the cache: `raw` is the saved answer, `fetched_at` the unix time it was fetched.
    pub fn load_quotes(&mut self, raw: &str, fetched_at: f64) {
        for q in data::parse_quotes(raw) {
            self.quotes.insert(q.symbol.clone(), q);
        }
        self.quotes_at = Some(fetched_at);
    }

    /// Loads one symbol's history from the cache. Returns false when the saved answer has no bars.
    pub fn load_history(&mut self, symbol: &str, raw: &str, fetched_at: f64) -> bool {
        let bars = data::tail(data::parse_history(raw), data::HISTORY_KEEP);
        if bars.is_empty() {
            return false;
        }
        self.history.insert(symbol.to_string(), bars);
        self.history_at.insert(symbol.to_string(), fetched_at);
        true
    }

    /// Applies a finished job at `now` (unix seconds). A failure keeps the data already held and waits `RETRY_S`.
    pub fn apply(&mut self, done: Done, now: f64) {
        match done {
            Done::Quotes(Ok(list)) => {
                self.quotes_pending = false;
                self.quotes_at = Some(now);
                self.quotes_error = None;
                for q in list {
                    self.quotes.insert(q.symbol.clone(), q);
                }
            }
            Done::Quotes(Err(e)) => {
                self.quotes_pending = false;
                self.retry_at = now + RETRY_S;
                self.quotes_error = Some(e);
            }
            Done::History { symbol, result: Ok(bars) } => {
                self.history_pending.remove(&symbol);
                self.history.insert(symbol.clone(), data::tail(bars, data::HISTORY_KEEP));
                self.history_at.insert(symbol.clone(), now);
                self.history_failed.remove(&symbol);
            }
            Done::History { symbol, result: Err(e) } => {
                self.history_pending.remove(&symbol);
                self.retry_at = now + RETRY_S;
                self.history_failed.insert(symbol, e);
            }
        }
    }

    /// Whether a quote answer is pending.
    pub fn is_quotes_pending(&self) -> bool {
        self.quotes_pending
    }

    /// Whether a history answer for `symbol` is pending.
    pub fn is_history_pending(&self, symbol: &str) -> bool {
        self.history_pending.contains(symbol)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: f64 = 1_000_000.0;

    fn quote(symbol: &str, close: f64) -> Quote {
        Quote { symbol: symbol.into(), stamp: "2026-10-02 22:00:00".into(), open: 100.0, close }
    }

    fn bars(n: usize) -> Vec<Bar> {
        (0..n).map(|i| Bar { date: format!("2026-01-{:02}", i + 1), close: i as f64 }).collect()
    }

    #[test]
    fn a_fresh_feed_asks_for_quotes_at_once() {
        let feed = Feed::default();
        assert!(feed.quotes_due(NOW));
        assert!(feed.history_due("aapl.us", NOW));
    }

    #[test]
    fn quotes_wait_for_their_time_to_live_unless_a_symbol_was_added() {
        let mut feed = Feed::default();
        feed.apply(Done::Quotes(Ok(vec![quote("aapl.us", 110.0)])), NOW);
        assert!(!feed.quotes_due(NOW + 60.0), "fresh");
        assert!(feed.quotes_due(NOW + QUOTE_TTL_S), "stale");
        feed.quotes_wanted = true;
        assert!(feed.quotes_due(NOW + 60.0), "a new symbol is due at once");
    }

    #[test]
    fn a_pending_request_is_never_repeated() {
        let mut feed = Feed::default();
        feed.begin_quotes();
        assert!(!feed.quotes_due(NOW));
        feed.quotes_wanted = true;
        assert!(!feed.quotes_due(NOW), "still pending even when wanted");
        feed.begin_history("aapl.us");
        assert!(!feed.history_due("aapl.us", NOW));
        assert!(feed.history_due("nvda.us", NOW), "other symbols are not blocked");
    }

    #[test]
    fn a_failure_keeps_the_old_quotes_and_waits_before_retrying() {
        let mut feed = Feed::default();
        feed.apply(Done::Quotes(Ok(vec![quote("aapl.us", 110.0)])), NOW);
        feed.begin_quotes();
        feed.apply(Done::Quotes(Err("Quotes are unavailable".into())), NOW + 900.0);
        assert_eq!(feed.quotes.get("aapl.us").map(|q| q.close), Some(110.0), "cached quote still shown");
        assert_eq!(feed.quotes_error.as_deref(), Some("Quotes are unavailable"));
        assert_eq!(feed.quotes_at, Some(NOW), "a failure is not a fresh answer");
        assert!(!feed.quotes_due(NOW + 900.0 + RETRY_S - 1.0));
        assert!(feed.quotes_due(NOW + 900.0 + RETRY_S));
    }

    #[test]
    fn a_good_answer_clears_the_error() {
        let mut feed = Feed::default();
        feed.apply(Done::Quotes(Err("down".into())), NOW);
        feed.apply(Done::Quotes(Ok(vec![quote("aapl.us", 1.0)])), NOW + RETRY_S);
        assert_eq!(feed.quotes_error, None);
        assert_eq!(feed.quotes_at, Some(NOW + RETRY_S));
    }

    #[test]
    fn history_keeps_only_the_newest_sessions_and_its_fetch_time() {
        let mut feed = Feed::default();
        feed.begin_history("aapl.us");
        feed.apply(Done::History { symbol: "aapl.us".into(), result: Ok(bars(150)) }, NOW);
        let kept = &feed.history["aapl.us"];
        assert_eq!(kept.len(), data::HISTORY_KEEP);
        assert_eq!(kept[0].close, 60.0, "the oldest 60 sessions are dropped");
        assert!(!feed.history_due("aapl.us", NOW + HISTORY_TTL_S - 1.0));
        assert!(feed.history_due("aapl.us", NOW + HISTORY_TTL_S));
    }

    #[test]
    fn a_history_failure_is_recorded_per_symbol() {
        let mut feed = Feed::default();
        feed.begin_history("aapl.us");
        feed.apply(Done::History { symbol: "aapl.us".into(), result: Err("no history".into()) }, NOW);
        assert_eq!(feed.history_failed.get("aapl.us").map(String::as_str), Some("no history"));
        assert!(!feed.is_history_pending("aapl.us"));
        assert!(feed.history.get("aapl.us").is_none());
    }

    #[test]
    fn cached_quotes_and_history_load_with_their_age() {
        let mut feed = Feed::default();
        feed.load_quotes("Symbol,Date,Time,Open,High,Low,Close,Volume\nAAPL.US,2026-10-02,22:00:09,100,1,1,105,5\n", NOW - 30.0);
        assert_eq!(feed.quotes["aapl.us"].close, 105.0);
        assert_eq!(feed.quotes_at, Some(NOW - 30.0));
        assert!(feed.load_history("aapl.us", "Date,Open,High,Low,Close,Volume\n2026-10-01,1,1,1,2,1\n", NOW));
        assert!(!feed.load_history("xyz.us", "garbage", NOW), "an unreadable cache is ignored");
        assert_eq!(feed.history["aapl.us"].len(), 1);
    }
}
