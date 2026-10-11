//! Engine recovery policy for the Browser, as pure functions the owners of `sdk/src/webview.rs` and `main.rs` call.
//! Declared from `session.rs` (`#[path]`), so no edit to `main.rs` is needed to compile it.
//!
//! Wiring (for the performance and features tracks):
//!   * every `PING_EVERY_S` send `ping N` to the engine; on `pong N` call `Watchdog::pong`. `Watchdog::check(now)`
//!     returns `Action::KillWedged` when no pong came for `WEDGE_AFTER_S` while the engine is alive: kill the
//!     process, then restart it exactly as for an exited engine.
//!   * on every engine start call `StormGuard::allow_restart(now)`; false means show `error_page(...)` instead of
//!     restarting (max `MAX_RESTARTS` per `WINDOW_S`).
//!   * on every engine crash or kill write `crash_line(...)` to the unified log (`Logger`, category `browser.engine`).
#![allow(dead_code)]

use std::collections::VecDeque;

pub const PING_EVERY_S: f64 = 2.0;
/// Missed heartbeats before the engine counts as wedged (a page-heavy first frame can take a few seconds).
pub const WEDGE_AFTER_S: f64 = 20.0;
pub const MAX_RESTARTS: usize = 3;
pub const WINDOW_S: f64 = 60.0;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Action {
    None,
    SendPing,
    KillWedged,
}

/// Heartbeat state for one engine process.
#[derive(Debug, Default)]
pub struct Watchdog {
    last_ping: Option<f64>,
    last_pong: Option<f64>,
    seq: u64,
}

impl Watchdog {
    /// Forget everything; call when a new engine starts. The start counts as the first sign of life.
    pub fn reset(&mut self, now: f64) {
        *self = Watchdog { last_ping: None, last_pong: Some(now), seq: 0 };
    }
    /// The sequence number of the ping to send now, if one is due.
    pub fn due(&mut self, now: f64) -> Option<u64> {
        if self.last_ping.is_some_and(|t| now - t < PING_EVERY_S) {
            return None;
        }
        self.last_ping = Some(now);
        self.seq += 1;
        Some(self.seq)
    }
    pub fn pong(&mut self, now: f64) {
        self.last_pong = Some(now);
    }
    /// What to do at `now`: nothing, ping, or kill a wedged engine.
    pub fn check(&mut self, now: f64) -> Action {
        if self.last_pong.is_some_and(|t| now - t > WEDGE_AFTER_S) {
            return Action::KillWedged;
        }
        if self.due(now).is_some() { Action::SendPing } else { Action::None }
    }
}

/// Restart-storm limiter: at most `MAX_RESTARTS` restarts in any `WINDOW_S`.
#[derive(Debug, Default)]
pub struct StormGuard {
    starts: VecDeque<f64>,
}

impl StormGuard {
    /// Records a restart attempt at `now`; false when it would be the one too many.
    pub fn allow_restart(&mut self, now: f64) -> bool {
        while self.starts.front().is_some_and(|t| now - t > WINDOW_S) {
            self.starts.pop_front();
        }
        if self.starts.len() >= MAX_RESTARTS {
            return false;
        }
        self.starts.push_back(now);
        true
    }
    /// The user pressed Reload on the error page: start counting afresh.
    pub fn clear(&mut self) {
        self.starts.clear();
    }
    pub fn restarts_in_window(&self) -> usize {
        self.starts.len()
    }
}

/// One unified-log line for an engine crash.
pub fn crash_line(tab_url: &str, reason: &str, restarts: usize) -> String {
    let url: String = tab_url.chars().filter(|c| !c.is_control()).take(300).collect();
    let reason: String = reason.chars().map(|c| if c.is_control() { ' ' } else { c }).take(300).collect();
    format!("engine crash tab_url={url} reason={reason} restarts_in_window={restarts}")
}

/// HTML for the page shown when the engine keeps crashing.
pub fn error_page(last_url: &str, reason: &str) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    format!(
        "<!doctype html><meta charset=utf-8><title>Browser engine stopped</title>\
         <body style=\"font:13px -apple-system,sans-serif;text-align:center;padding:15vh 2em;color:#888\">\
         <h2 style=\"font-size:17px\">The web engine stopped repeatedly</h2>\
         <p>It was restarted {MAX_RESTARTS} times in the last minute, so it was not started again.</p>\
         <p>{}</p><p>Page: {}</p><p>Choose Reload to try again.</p>",
        esc(reason),
        esc(last_url)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storm_allows_three_per_minute() {
        let mut g = StormGuard::default();
        assert!(g.allow_restart(0.0) && g.allow_restart(1.0) && g.allow_restart(2.0));
        assert!(!g.allow_restart(3.0));
        assert!(g.allow_restart(61.5));
        g.clear();
        assert_eq!(g.restarts_in_window(), 0);
    }

    #[test]
    fn watchdog_pings_then_kills() {
        let mut w = Watchdog::default();
        w.reset(0.0);
        assert_eq!(w.check(0.1), Action::SendPing);
        assert_eq!(w.check(1.0), Action::None);
        assert_eq!(w.check(2.2), Action::SendPing);
        w.pong(5.0);
        assert_ne!(w.check(22.0), Action::KillWedged);
        assert_eq!(w.check(25.5), Action::KillWedged);
    }

    #[test]
    fn restart_resets_the_watchdog_and_clear_forgives_the_storm() {
        let mut w = Watchdog::default();
        w.reset(100.0);
        assert_eq!(w.check(105.0), Action::SendPing);
        assert_eq!(w.check(121.0), Action::KillWedged);
        w.reset(122.0);
        assert_ne!(w.check(123.0), Action::KillWedged);
        let mut g = StormGuard::default();
        for t in 0..3 {
            assert!(g.allow_restart(t as f64));
        }
        g.clear();
        assert!(g.allow_restart(4.0));
    }

    #[test]
    fn crash_line_is_one_line_and_escaped_page() {
        assert!(!crash_line("http://a\n/b", "x\ny", 2).contains('\n'));
        assert!(!error_page("<script>", "a&b").contains("<script>"));
    }
}
