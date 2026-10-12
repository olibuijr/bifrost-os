//! Per-app usage metrics as a pure model: time in the foreground and in the background, the first-draw time of each
//! launch as a histogram, and how each foreground run ended, counted by cause. Every timestamp is a count of
//! milliseconds from a fixed origin that the caller supplies, so nothing here reads a clock. The shell and the SDK do
//! not feed this model yet. The per-day Settings usage records in `shell/screentime` are the only per-app time store.

use std::collections::BTreeMap;

/// Whether an app is in front of the user or behind other windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Foreground,
    Background,
}

/// How a foreground run ended. The discriminant is the index of the count in `Totals::endings`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    Normal = 0,
    MemoryLimit = 1,
    InvalidAccess = 2,
    Abnormal = 3,
    IllegalInstruction = 4,
    Watchdog = 5,
}

/// Upper bounds of the first-draw histogram buckets, in milliseconds. Bucket 0 holds durations below the first bound,
/// bucket `i` holds durations from the bound before it up to the bound at `i`, and the last bucket has no upper limit.
pub const FIRST_DRAW_BOUNDS_MS: [u64; 6] = [250, 500, 1000, 2000, 4000, 8000];

/// The histogram bucket for a first-draw duration in milliseconds.
pub fn first_draw_bucket(ms: u64) -> usize {
    FIRST_DRAW_BOUNDS_MS
        .iter()
        .position(|&bound| ms < bound)
        .unwrap_or(FIRST_DRAW_BOUNDS_MS.len())
}

/// The totals kept for one app.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub foreground_ms: u64,
    pub background_ms: u64,
    /// Foreground run endings, indexed by `Ending as usize`.
    pub endings: [u32; 6],
    /// First-draw counts per histogram bucket, see `FIRST_DRAW_BOUNDS_MS`.
    pub first_draw: [u32; 7],
}

#[derive(Debug, Default)]
struct AppState {
    totals: Totals,
    open: Option<(Phase, u64)>,
    launch_began: Option<u64>,
}

/// The metrics of every app that has reported to the model.
#[derive(Debug, Default)]
pub struct Metrics {
    apps: BTreeMap<String, AppState>,
}

fn bump(count: &mut u32) {
    *count = count.saturating_add(1);
}

fn bank(totals: &mut Totals, phase: Phase, ms: u64) {
    match phase {
        Phase::Foreground => totals.foreground_ms = totals.foreground_ms.saturating_add(ms),
        Phase::Background => totals.background_ms = totals.background_ms.saturating_add(ms),
    }
}

impl Metrics {
    pub fn new() -> Metrics {
        Metrics::default()
    }

    /// Moves `app` into `phase` at `now` and banks the time of the phase it leaves. Asking for the phase that is already
    /// open changes nothing. A `now` earlier than the open phase's start banks zero time.
    pub fn set_phase(&mut self, app: &str, phase: Phase, now: u64) {
        let state = self.apps.entry(app.to_string()).or_default();
        if let Some((open, since)) = state.open {
            if open == phase {
                return;
            }
            bank(&mut state.totals, open, now.saturating_sub(since));
        }
        state.open = Some((phase, now));
    }

    /// Ends the open phase of `app` at `now`, as when the app quits. An app with no open phase is left alone.
    pub fn close(&mut self, app: &str, now: u64) {
        if let Some(state) = self.apps.get_mut(app) {
            if let Some((phase, since)) = state.open.take() {
                bank(&mut state.totals, phase, now.saturating_sub(since));
            }
        }
    }

    /// Starts the launch clock for `app`.
    pub fn begin_launch(&mut self, app: &str, now: u64) {
        self.apps.entry(app.to_string()).or_default().launch_began = Some(now);
    }

    /// Records the first draw of the launch in progress and returns its duration in milliseconds. Returns `None` when
    /// no launch was begun, so only the first draw after `begin_launch` is counted.
    pub fn first_draw(&mut self, app: &str, now: u64) -> Option<u64> {
        let state = self.apps.get_mut(app)?;
        let began = state.launch_began.take()?;
        let ms = now.saturating_sub(began);
        bump(&mut state.totals.first_draw[first_draw_bucket(ms)]);
        Some(ms)
    }

    /// Counts how a foreground run of `app` ended.
    pub fn record_ending(&mut self, app: &str, ending: Ending) {
        let state = self.apps.entry(app.to_string()).or_default();
        bump(&mut state.totals.endings[ending as usize]);
    }

    /// The totals of `app`, or `None` if it has never reported.
    pub fn totals(&self, app: &str) -> Option<&Totals> {
        self.apps.get(app).map(|state| &state.totals)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_bank_their_time() {
        let mut m = Metrics::new();
        m.set_phase("notes", Phase::Foreground, 0);
        m.set_phase("notes", Phase::Background, 1000);
        m.set_phase("notes", Phase::Foreground, 3000);
        m.close("notes", 3500);
        let t = m.totals("notes").unwrap();
        // foreground 0..1000 and 3000..3500, background 1000..3000
        assert_eq!(t.foreground_ms, 1500);
        assert_eq!(t.background_ms, 2000);
    }

    #[test]
    fn repeating_the_open_phase_keeps_its_start() {
        let mut m = Metrics::new();
        m.set_phase("notes", Phase::Foreground, 0);
        m.set_phase("notes", Phase::Foreground, 400);
        m.close("notes", 1000);
        assert_eq!(m.totals("notes").unwrap().foreground_ms, 1000);
    }

    #[test]
    fn a_clock_that_moves_back_banks_nothing_for_the_left_phase() {
        let mut m = Metrics::new();
        m.set_phase("mail", Phase::Foreground, 1000);
        m.set_phase("mail", Phase::Background, 900);
        m.close("mail", 1200);
        let t = m.totals("mail").unwrap();
        assert_eq!(t.foreground_ms, 0);
        assert_eq!(t.background_ms, 300);
    }

    #[test]
    fn close_without_an_open_phase_changes_nothing() {
        let mut m = Metrics::new();
        m.close("ghost", 5000);
        assert!(m.totals("ghost").is_none());
        m.set_phase("mail", Phase::Background, 0);
        m.close("mail", 200);
        m.close("mail", 900);
        let t = m.totals("mail").unwrap();
        assert_eq!(t.background_ms, 200);
        assert_eq!(t.foreground_ms, 0);
    }

    #[test]
    fn first_draw_is_timed_from_the_launch_and_counted_once() {
        let mut m = Metrics::new();
        assert_eq!(m.first_draw("notes", 50), None);
        m.begin_launch("notes", 100);
        assert_eq!(m.first_draw("notes", 350), Some(250));
        assert_eq!(m.first_draw("notes", 400), None);
        assert_eq!(m.totals("notes").unwrap().first_draw, [0, 1, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn first_draw_buckets_have_hand_worked_edges() {
        assert_eq!(first_draw_bucket(0), 0);
        assert_eq!(first_draw_bucket(249), 0);
        assert_eq!(first_draw_bucket(250), 1);
        assert_eq!(first_draw_bucket(7999), 5);
        assert_eq!(first_draw_bucket(8000), 6);
    }

    #[test]
    fn endings_are_counted_by_cause() {
        let mut m = Metrics::new();
        m.record_ending("mail", Ending::Normal);
        m.record_ending("mail", Ending::Watchdog);
        m.record_ending("mail", Ending::Watchdog);
        m.record_ending("mail", Ending::MemoryLimit);
        assert_eq!(m.totals("mail").unwrap().endings, [1, 1, 0, 0, 0, 2]);
    }
}
