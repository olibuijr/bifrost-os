//! Due-date alerts as notification triggers. Every alert still to come is handed to the shell as a pending request with
//! a time-interval trigger, so the shell shows it on time whether or not
//! Reminders is running. The app re-plans only when the data changes, when the zone changes, when the earliest planned
//! alert is due, or once an hour; a plan that did not change sends nothing. Pure rules here, the calls in [`apply`].
use crate::model::{self, Reminder};
use bifrost_app::usernotif::{self, Request};
use bifrost_app::Ctx;

/// Alerts one app keeps pending at a time (the shell allows 64); the soonest are planned.
pub const MAX_SCHEDULED: usize = 32;
/// Re-plan at least this often, so the window of planned alerts keeps moving forward.
pub const REFRESH_SECS: i64 = 3600;
/// Longest trigger delay the shell takes, in whole seconds (`u32` milliseconds, about 49 days).
const MAX_DELAY_SECS: i64 = u32::MAX as i64 / 1000;

/// One alert as a notification request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Planned {
    /// Stable per reminder and kind, so a changed alert replaces the old request with the same identifier.
    pub id: String,
    /// Unix second the alert is due.
    pub at: i64,
    pub delay_ms: u32,
    pub title: String,
    pub body: String,
}

/// What changed since the last sync. `reset` clears every pending request of the app first (the app has not synced
/// yet, so it cannot tell what the shell still holds); then `remove` and `add` apply.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Change {
    pub reset: bool,
    pub remove: Vec<String>,
    pub add: Vec<Planned>,
}

#[derive(Default)]
pub struct AlertState {
    synced: bool,
    gen: u32,
    /// Zone offset the last plan was computed under.
    offset: i32,
    /// (request id, due unix) of every request handed to the shell.
    scheduled: Vec<(String, i64)>,
    /// Unix second of the next re-plan.
    next_sync: i64,
}

fn request_id(id: u64, early: bool) -> String {
    format!("reminder-{id}-{}", if early { "early" } else { "due" })
}

/// The alerts after `now` that the shell can take, soonest first, at most [`MAX_SCHEDULED`].
pub fn plan(items: &[Reminder], now: i64, offset_at: model::OffsetAt) -> Vec<Planned> {
    let mut out = Vec::new();
    for r in items.iter().filter(|r| !r.completed) {
        let Some(due) = model::due_unix(r, offset_at) else { continue };
        let mut push = |early: bool, at: i64| {
            if at > now && at - now <= MAX_DELAY_SECS {
                out.push(Planned {
                    id: request_id(r.id, early),
                    at,
                    delay_ms: ((at - now) * 1000) as u32,
                    title: r.title.clone(),
                    body: model::alert_text(r, early),
                });
            }
        };
        push(false, due);
        if let Some(early) = r.early {
            push(true, due - early as i64 * 60);
        }
    }
    out.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)));
    out.truncate(MAX_SCHEDULED);
    out
}

/// Decides what to hand to the shell. `None` when nothing changed and no re-plan is due; `load` is only called when a
/// re-plan runs, so an idle app never reads the store.
pub fn step(
    state: &mut AlertState,
    now: i64,
    gen: u32,
    load: impl FnOnce() -> Vec<Reminder>,
    offset_at: model::OffsetAt,
) -> Option<Change> {
    let offset = offset_at(now);
    if state.synced && state.gen == gen && state.offset == offset && now < state.next_sync {
        return None;
    }
    let first = !state.synced;
    state.synced = true;
    state.gen = gen;
    state.offset = offset;
    if first {
        state.scheduled.clear();
    }
    let planned = plan(&load(), now, offset_at);
    let remove: Vec<String> = state
        .scheduled
        .iter()
        .filter(|(id, _)| !planned.iter().any(|p| &p.id == id))
        .map(|(id, _)| id.clone())
        .collect();
    let add: Vec<Planned> = planned
        .iter()
        .filter(|p| !state.scheduled.iter().any(|(id, at)| *id == p.id && *at == p.at))
        .cloned()
        .collect();
    state.scheduled = planned.iter().map(|p| (p.id.clone(), p.at)).collect();
    state.next_sync = planned.first().map_or(now + REFRESH_SECS, |p| p.at.min(now + REFRESH_SECS));
    if first || !remove.is_empty() || !add.is_empty() {
        Some(Change { reset: first, remove: if first { Vec::new() } else { remove }, add })
    } else {
        None
    }
}

/// Sends a change to the shell: `reset` removes every pending request of the app (an empty id list), otherwise only
/// the removed identifiers. An empty list must never reach `remove_pending` without `reset`, since that removes all.
pub fn apply(ctx: &Ctx, change: &Change) {
    if change.reset {
        usernotif::remove_pending(ctx, &[]);
    } else if !change.remove.is_empty() {
        let ids: Vec<&str> = change.remove.iter().map(String::as_str).collect();
        usernotif::remove_pending(ctx, &ids);
    }
    for p in &change.add {
        usernotif::add(ctx, Request::new(&p.id, &p.title).body(&p.body).after_ms(p.delay_ms));
    }
}

impl AlertState {
    /// Monotonic ms of the next re-plan, for the shell's wake computation; `None` until the first sync.
    pub fn wake(&self, wall_unix: i64, wall_at: f64) -> Option<f64> {
        self.synced.then(|| wall_at + (self.next_sync - wall_unix) as f64 * 1e3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Reminder;

    const BASE: i64 = 20_000 * 86_400;

    fn due_at(id: u64, day: i64, min: u16) -> Reminder {
        let mut r = Reminder::new(id, format!("r{id}"));
        r.due = Some(day);
        r.time = Some(min);
        r
    }

    #[test]
    fn plan_skips_completed_and_past_and_adds_early_alerts() {
        let mut early = due_at(3, 20_000, 720);
        early.early = Some(30);
        let mut done = due_at(4, 20_000, 800);
        done.completed = true;
        let planned = plan(&[early, done, due_at(5, 20_000, 540)], BASE + 600 * 60, &|_| 0);
        let ids: Vec<&str> = planned.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["reminder-3-early", "reminder-3-due"]);
        assert_eq!(planned[0].at, BASE + 690 * 60);
        assert_eq!(planned[0].body, "Due in 30 minutes");
        assert_eq!(planned[0].delay_ms, 90 * 60 * 1000);
        assert_eq!(planned[1].body, "Due 12:00 PM");
    }

    #[test]
    fn plan_keeps_the_soonest_thirty_two() {
        let items: Vec<Reminder> = (1..=40).map(|i| due_at(i, 20_000, 600 + i as u16)).collect();
        let planned = plan(&items, BASE, &|_| 0);
        assert_eq!(planned.len(), MAX_SCHEDULED);
        assert_eq!(planned[0].id, "reminder-1-due");
        assert_eq!(planned[MAX_SCHEDULED - 1].id, "reminder-32-due");
    }

    #[test]
    fn plan_ignores_alerts_beyond_the_trigger_limit() {
        // 60 days ahead is past the 49-day limit of a u32 millisecond delay
        assert!(plan(&[due_at(1, 20_060, 0)], BASE, &|_| 0).is_empty());
    }

    #[test]
    fn first_sync_resets_and_schedules_only_the_future() {
        let now = BASE + 600 * 60;
        let items = vec![due_at(1, 20_000, 540), due_at(2, 20_000, 660)];
        let mut st = AlertState::default();
        assert_eq!(st.wake(now, 0.0), None);
        let c = step(&mut st, now, 0, || items.clone(), &|_| 0).unwrap();
        assert!(c.reset);
        assert!(c.remove.is_empty());
        assert_eq!(c.add.len(), 1);
        assert_eq!(c.add[0].id, "reminder-2-due");
        assert_eq!(c.add[0].delay_ms, 3_600_000);
        assert_eq!(c.add[0].body, "Due 11:00 AM");
        assert_eq!(st.next_sync, BASE + 660 * 60);
        assert_eq!(st.wake(now, 5000.0), Some(5000.0 + 3600.0 * 1000.0));
    }

    #[test]
    fn idle_sync_sends_nothing_and_does_not_read_the_store() {
        let now = BASE + 600 * 60;
        let items = vec![due_at(2, 20_000, 660)];
        let mut st = AlertState::default();
        step(&mut st, now, 0, || items.clone(), &|_| 0);
        let mut loaded = false;
        let quiet = step(&mut st, now + 60, 0, || { loaded = true; items.clone() }, &|_| 0);
        assert!(quiet.is_none());
        assert!(!loaded);
    }

    #[test]
    fn edits_and_deletes_send_only_the_difference() {
        let now = BASE + 600 * 60;
        let mut st = AlertState::default();
        step(&mut st, now, 0, || vec![due_at(1, 20_000, 660), due_at(2, 20_000, 720)], &|_| 0);
        let c = step(&mut st, now, 1, || vec![due_at(2, 20_000, 780)], &|_| 0).unwrap();
        assert!(!c.reset);
        assert_eq!(c.remove, vec!["reminder-1-due".to_string()]);
        assert_eq!(c.add.len(), 1);
        assert_eq!(c.add[0].id, "reminder-2-due");
        assert_eq!(c.add[0].at, BASE + 780 * 60);
    }

    #[test]
    fn the_earliest_alert_wakes_the_next_sync() {
        let now = BASE + 600 * 60;
        let mut st = AlertState::default();
        step(&mut st, now, 0, || vec![due_at(1, 20_000, 601)], &|_| 0);
        assert_eq!(st.next_sync, BASE + 601 * 60);
        // once it is due the plan is empty, so the app re-plans after the refresh interval, not at once
        let mut loaded = false;
        let later = step(&mut st, BASE + 602 * 60, 0, || { loaded = true; vec![] }, &|_| 0);
        assert!(loaded);
        let c = later.unwrap();
        assert_eq!(c.remove, vec!["reminder-1-due".to_string()]);
        assert_eq!(st.next_sync, BASE + 602 * 60 + REFRESH_SECS);
    }

    #[test]
    fn a_zone_change_forces_a_re_plan() {
        let now = BASE + 600 * 60;
        let mut st = AlertState::default();
        step(&mut st, now, 0, || vec![due_at(1, 20_000, 660)], &|_| 0);
        let c = step(&mut st, now + 10, 0, || vec![due_at(1, 20_000, 660)], &|_| 3600);
        assert!(c.is_some(), "a new offset shifts the due moment, so the request is replaced");
    }
}
