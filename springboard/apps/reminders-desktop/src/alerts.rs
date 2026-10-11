//! Alerts: due reminders post a notification through the shell's existing
//! notification system, also while the app is closed. The shell keeps one
//! `AlertState`; the data is read from the store file only when an alert is
//! due or the app saved (generation changed).
use crate::model;

#[derive(Default)]
pub struct AlertState {
    primed: bool,
    refresh: bool,
    /// Unix second up to which alerts were already handled.
    last: i64,
    next: Option<i64>,
    gen: u32,
    /// Zone offset the cached `next` was computed under.
    offset: i32,
}

/// What `tick` decided; split out so the rule is host-testable.
pub fn step(
    state: &mut AlertState,
    now: i64,
    gen: u32,
    load: impl FnOnce() -> Vec<model::Reminder>,
    offset_at: model::OffsetAt,
) -> Vec<(u64, String, String, i64)> {
    let offset = offset_at(now);
    if !state.primed {
        state.primed = true;
        state.refresh = true;
        state.last = now;
        state.offset = offset;
    }
    if state.offset != offset {
        // the zone changed (travel, daylight saving): the cached next alert is stale
        state.offset = offset;
        state.next = None;
        state.refresh = true;
    }
    let due_now = state.next.is_some_and(|n| n <= now);
    if !(due_now || state.refresh || gen != state.gen) {
        return vec![];
    }
    state.gen = gen;
    state.refresh = false;
    let items = load();
    let from = state.last;
    state.last = now;
    state.next = model::next_alert(&items, now, offset_at);
    model::due_alerts(&items, from, now, offset_at)
        .into_iter()
        .filter_map(|(id, at, early)| {
            let r = items.iter().find(|r| r.id == id)?;
            Some((id, r.title.clone(), model::alert_text(r, early), at))
        })
        .collect()
}

impl AlertState {
    /// Monotonic ms of the next alert, for the shell's wake computation.
    pub fn wake(&self, wall_unix: i64, wall_at: f64) -> Option<f64> {
        self.next.map(|n| wall_at + (n - wall_unix) as f64 * 1e3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::Reminder;
    fn due_at(id: u64, day: i64, min: u16) -> Reminder {
        let mut r = Reminder::new(id, format!("r{id}"));
        r.due = Some(day);
        r.time = Some(min);
        r
    }
    #[test]
    fn first_call_primes_without_firing_the_past() {
        let mut st = AlertState::default();
        let base = 20_000 * 86_400;
        let items = vec![due_at(1, 20_000, 540)];
        let fired = step(&mut st, base + 600 * 60, 0, || items.clone(), &|_| 0);
        assert!(fired.is_empty());
        assert_eq!(st.next, None);
    }
    #[test]
    fn fires_once_when_the_moment_passes_and_stays_idle_between() {
        let mut st = AlertState::default();
        let base = 20_000 * 86_400;
        let mut items = vec![due_at(1, 20_000, 600)];
        items[0].early = Some(30);
        assert!(step(&mut st, base + 500 * 60, 0, || items.clone(), &|_| 0).is_empty());
        assert_eq!(st.next, Some(base + 570 * 60));
        let mut loaded = false;
        let quiet = step(&mut st, base + 560 * 60, 0, || { loaded = true; vec![] }, &|_| 0);
        assert!(quiet.is_empty() && !loaded, "no disk read before the next alert");
        let early = step(&mut st, base + 570 * 60 + 5, 0, || items.clone(), &|_| 0);
        assert_eq!(early.len(), 1);
        assert_eq!(early[0].2, "Due in 30 minutes");
        let main = step(&mut st, base + 600 * 60 + 1, 0, || items.clone(), &|_| 0);
        assert_eq!(main.len(), 1);
        assert_eq!(main[0].1, "r1");
        assert_eq!(main[0].2, "Due 10:00 AM");
        assert!(step(&mut st, base + 700 * 60, 0, || items.clone(), &|_| 0).is_empty());
        assert_eq!(st.next, None);
    }
    #[test]
    fn an_offset_change_recomputes_the_cached_next_alert() {
        let mut st = AlertState::default();
        let base = 20_000 * 86_400;
        let items = vec![due_at(1, 20_000, 600)];
        step(&mut st, base, 0, || items.clone(), &|_| 0);
        assert_eq!(st.next, Some(base + 600 * 60));
        let mut loaded = false;
        step(&mut st, base + 10, 0, || { loaded = true; items.clone() }, &|_| 3600);
        assert!(loaded);
        assert_eq!(st.next, Some(base + 600 * 60 - 3600));
    }
    #[test]
    fn app_saves_trigger_a_refresh_and_wake_is_monotonic() {
        let mut st = AlertState::default();
        let base = 20_000 * 86_400;
        step(&mut st, base, 0, Vec::new, &|_| 0);
        let items = vec![due_at(1, 20_000, 60)];
        step(&mut st, base + 10, 1, || items.clone(), &|_| 0);
        assert_eq!(st.next, Some(base + 3600));
        assert_eq!(st.wake(base + 10, 5000.0), Some(5000.0 + 3590.0 * 1000.0));
    }
}
