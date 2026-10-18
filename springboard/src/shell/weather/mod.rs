
//! Weather data shared with the Today widget and the lock screen; the app
//! itself is the `weather` package (apps/weather-desktop).
//!
//! The shell keeps the shared summary current with no app window open: `Refresher` asks for a refresh once an
//! hour and again on every wake, and each refresh runs on a worker thread through `data::refresh_summary`.

pub mod data;
pub mod glyph;
pub use glyph::icon;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// The summary is refreshed this often while the shell runs (ms, the shell clock).
pub const REFRESH_MS: f64 = 60.0 * 60_000.0;
/// A cached forecast younger than this is reused by a refresh, so a wake soon after one does not fetch again.
pub const MAX_AGE_S: u64 = 15 * 60;

pub fn data_path() -> String {
    crate::shell::apphost::root().join("appdata/is.olibuijr.weather").to_string_lossy().into_owned()
}

/// When the shared summary is next due. Starts due, so the first frame refreshes it.
#[derive(Debug)]
pub struct Refresher {
    due_at: f64,
    running: Arc<AtomicBool>,
}

impl Default for Refresher {
    fn default() -> Self {
        Refresher { due_at: 0.0, running: Arc::new(AtomicBool::new(false)) }
    }
}

impl Refresher {
    /// True when a refresh is due at `t` (ms). Schedules the next one an hour later.
    pub fn due(&mut self, t: f64) -> bool {
        if t < self.due_at {
            return false;
        }
        self.due_at = t + REFRESH_MS;
        true
    }

    /// The display came back: the next `due` call asks for a refresh.
    pub fn wake(&mut self) {
        self.due_at = 0.0;
    }

    /// Runs one refresh on a worker thread, unless one is still running. The result is saved by
    /// `data::refresh_summary`; the Today widget picks it up on its own refresh.
    pub fn start(&self, dir: PathBuf) -> bool {
        if self.running.swap(true, Ordering::AcqRel) {
            return false;
        }
        let running = self.running.clone();
        let spawned = std::thread::Builder::new().name("weather-refresh".into()).spawn(move || {
            crate::gfx::affinity::background_thread();
            let store = data::Store::new(dir);
            let _ = data::refresh_summary(&store, &crate::shell::music::radio::Http::default(), MAX_AGE_S);
            running.store(false, Ordering::Release);
        });
        if spawned.is_err() {
            self.running.store(false, Ordering::Release);
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_frame_is_due_then_an_hour_apart() {
        let mut r = Refresher::default();
        assert!(r.due(0.0));
        assert!(!r.due(1_000.0));
        assert!(!r.due(REFRESH_MS - 1.0));
        assert!(r.due(REFRESH_MS));
        assert!(!r.due(REFRESH_MS + 1.0));
    }

    #[test]
    fn a_wake_makes_the_refresh_due_at_once() {
        let mut r = Refresher::default();
        assert!(r.due(5_000.0));
        assert!(!r.due(6_000.0));
        r.wake();
        assert!(r.due(6_000.0), "the wake asks for a refresh without waiting for the hour");
        assert!(!r.due(7_000.0), "and the hourly schedule starts again from the wake");
    }

    #[test]
    fn the_cache_limit_is_shorter_than_the_hour() {
        // an hourly refresh must fetch, while a wake soon after a refresh reuses the cache
        assert!(MAX_AGE_S < (REFRESH_MS / 1000.0) as u64);
    }
}
