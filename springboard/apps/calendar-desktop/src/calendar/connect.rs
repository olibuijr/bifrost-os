//! Calendar connected to the shell's system services: event alerts are scheduled through `notification-schedule`
//! (one calendar-trigger request per event alarm, replaced on edit and removed on delete), the Up Next widget
//! timeline is kept in `widgets_timeline`, and the Dock tile shows today's date through the `dock` service.
//!
//! The planning is pure (`plan_alerts`, `diff`, `up_next_entries`) so it can be checked without a shell; [`Connect`]
//! holds what was last sent and makes the calls. Calls happen only when the store changed or the local day rolled
//! over. When the shell has no `notification-schedule` service (an older shell answers "no service ..."), the app
//! falls back to posting its own alert notifications from `take_alerts`.

use super::alerts::{alert_calendar_with, due};
use super::extras::Extras;
use super::ics::IcsCalendar;
use super::recur;
use super::store::CalendarEvent;
use super::Date;
use bifrost_app::controls::timeline::ReloadPolicy;
use bifrost_app::dock_tile;
use bifrost_app::notification_schedule::{self, Schedule};
use bifrost_app::widgets_timeline::{self, Entry};
use bifrost_app::Ctx;
use bifrost_core::json::Json;
use std::collections::BTreeMap;

/// Alerts are planned this far ahead. A calendar trigger names a weekday and a time, so the window stays inside one
/// week: the next matching weekday is then the alert's own day.
const WINDOW_SECS: i64 = 6 * 86_400 + 22 * 3_600;
/// Prefix of every alert request identifier.
const ALERT_PREFIX: &str = "calendar-alert:";
/// The widget timeline kind the shell reloads.
pub const UP_NEXT: &str = "up-next";
/// Timed events the widget lists after the current one.
const UP_NEXT_MAX: usize = 4;

/// What the planning reads: the store events, their .ics extras, the clock and the local zone offset (seconds east).
pub struct Snap<'a> {
    pub events: &'a [CalendarEvent],
    pub rich: &'a IcsCalendar,
    /// The calendars and the event extras (custom repeats, alarms, zones), `extras.rs`.
    pub extras: &'a Extras,
    pub now: i64,
    pub offset: i32,
}

/// The local day number (days since the epoch, in the local zone) of Unix time `now`.
pub fn local_day(now: i64, offset: i32) -> i64 {
    (now + offset as i64).div_euclid(86_400)
}

/// The shell's weekday numbering for a calendar trigger: 1 = Sunday .. 7 = Saturday.
fn wire_weekday(day: i64) -> u8 {
    (Date::from_days(day).weekday() + 1) as u8
}

/// The request identifier of one alarm of one event: the event's identifier and its lead time in minutes.
pub fn alert_id(uid: &str, minutes_before: i64) -> String {
    format!("{ALERT_PREFIX}{uid}:{minutes_before}")
}

/// `HH:MM` of Unix time `unix` in the local zone.
fn clock(unix: i64, offset: i32) -> String {
    let s = (unix + offset as i64).rem_euclid(86_400);
    format!("{:02}:{:02}", s / 3600, (s % 3600) / 60)
}

/// The alert requests due in the planning window, keyed by identifier. Each identifier names the next occurrence of
/// one alarm of one event, so an edit replaces it and a series keeps one request per alarm.
pub fn plan_alerts(cal: &IcsCalendar, now: i64, offset: i32) -> BTreeMap<String, Schedule> {
    let mut alerts = due(cal, now, now + WINDOW_SECS, offset);
    alerts.sort_by_key(|a| a.fire);
    let mut out = BTreeMap::new();
    for a in alerts {
        let id = alert_id(&a.uid, (a.start - a.fire) / 60);
        if out.contains_key(&id) {
            continue;
        }
        let local = a.fire + offset as i64;
        let secs = local.rem_euclid(86_400);
        let day = local.div_euclid(86_400);
        let (hour, minute) = ((secs / 3600) as u8, ((secs % 3600) / 60) as u8);
        let s = Schedule::calendar(&id, &a.title, hour, minute, &[wire_weekday(day)], false).body(&a.body).thread("calendar");
        out.insert(id, s);
    }
    out
}

/// What must be sent to move the scheduled requests from `prev` to `next`: the requests to add or replace, and the
/// identifiers to remove (those that no longer have an alert).
pub fn diff(prev: &BTreeMap<String, Schedule>, next: &BTreeMap<String, Schedule>) -> (Vec<Schedule>, Vec<String>) {
    let mut adds = Vec::new();
    for (id, s) in next {
        if prev.get(id) != Some(s) {
            adds.push(s.clone());
        }
    }
    let removes = prev.keys().filter(|id| !next.contains_key(*id)).cloned().collect();
    (adds, removes)
}

/// The Up Next timeline: today's timed events that have not started, in start order. The first entry is shown from
/// `now`; each later entry from the start of the event before it, so the next event is the one on screen.
pub fn up_next_entries(cal: &IcsCalendar, now: i64, offset: i32) -> Vec<Entry> {
    let today = Date::from_days(local_day(now, offset));
    let mut starts: Vec<(i64, String, String)> = Vec::new();
    for e in cal.events.iter().filter(|e| !e.all_day) {
        let dates = match &e.rule {
            Some(r) => recur::expand(r, e.start.date, &e.exdates, today, today),
            None if e.start.date == today && !e.exdates.contains(&today) => vec![today],
            None => vec![],
        };
        for d in dates {
            let start = d.days() * 86_400 + e.start.minute as i64 * 60 - offset as i64;
            if start >= now {
                starts.push((start, e.summary.clone(), e.location.clone()));
            }
        }
    }
    starts.sort();
    if starts.is_empty() {
        return vec![Entry { date: now as f64, title: "Up Next".into(), text: "No more events today".into() }];
    }
    let mut out = Vec::new();
    let mut from = now;
    for (start, title, location) in starts.into_iter().take(UP_NEXT_MAX) {
        let text = if location.is_empty() {
            clock(start, offset)
        } else {
            format!("{} \u{b7} {location}", clock(start, offset))
        };
        out.push(Entry { date: from as f64, title, text });
        from = start;
    }
    out
}

/// The Dock badge: today's day of the month in the local zone.
pub fn badge_text(now: i64, offset: i32) -> String {
    Date::from_days(local_day(now, offset)).day.to_string()
}

/// The unix time of the next local midnight, when the Up Next timeline is due for a reload.
fn next_midnight(now: i64, offset: i32) -> f64 {
    ((local_day(now, offset) + 1) * 86_400 - offset as i64) as f64
}

/// What the calendar last sent to the shell, and whether the shell has the alert service.
#[derive(Default)]
pub struct Connect {
    scheduled: BTreeMap<String, Schedule>,
    up_next: Option<Vec<Entry>>,
    badge: Option<String>,
    synced: Option<(u64, i64)>,
    missing: bool,
}

impl Connect {
    /// True when the store changed (`gen`) or the local day rolled over (`day`) since the last sync.
    pub fn is_stale(&self, gen: u64, day: i64) -> bool {
        self.synced != Some((gen, day))
    }

    /// True when the shell has no `notification-schedule` service, so the app must post its own alerts.
    pub fn fallback(&self) -> bool {
        self.missing
    }

    /// Reads a service reply: an older shell answers "no service notification-schedule" to the first call.
    pub fn on_reply(&mut self, result: &Result<Json, String>) {
        if let Err(msg) = result {
            if msg.starts_with("no service") && msg.contains(notification_schedule::SERVICE) {
                self.missing = true;
            }
        }
    }

    /// Brings the shell up to date with the store: alert requests, the Up Next timeline and the Dock badge. Sends only
    /// what changed since the last call.
    pub fn sync(&mut self, ctx: &Ctx, snap: &Snap, gen: u64, day: i64) {
        self.synced = Some((gen, day));
        let cal = alert_calendar_with(snap.events, snap.rich, snap.extras);
        if !self.missing {
            let next = plan_alerts(&cal, snap.now, snap.offset);
            let (adds, removes) = diff(&self.scheduled, &next);
            for s in &adds {
                notification_schedule::add(ctx, s);
            }
            if !removes.is_empty() {
                let ids: Vec<&str> = removes.iter().map(String::as_str).collect();
                notification_schedule::remove(ctx, &ids);
            }
            self.scheduled = next;
        }
        self.send_up_next(ctx, &cal, snap);
        self.send_badge(ctx, snap);
    }

    /// The shell asked for a new Up Next timeline (its reload date passed): send it again.
    pub fn refresh_up_next(&mut self, ctx: &Ctx, snap: &Snap) {
        let cal = alert_calendar_with(snap.events, snap.rich, snap.extras);
        self.up_next = None;
        self.send_up_next(ctx, &cal, snap);
    }

    fn send_up_next(&mut self, ctx: &Ctx, cal: &IcsCalendar, snap: &Snap) {
        let entries = up_next_entries(cal, snap.now, snap.offset);
        if self.up_next.as_ref() == Some(&entries) {
            return;
        }
        let policy = ReloadPolicy::After(next_midnight(snap.now, snap.offset));
        widgets_timeline::reload(ctx, UP_NEXT, &entries, policy);
        self.up_next = Some(entries);
    }

    fn send_badge(&mut self, ctx: &Ctx, snap: &Snap) {
        let text = badge_text(snap.now, snap.offset);
        if self.badge.as_deref() == Some(text.as_str()) {
            return;
        }
        dock_tile::set_badge(ctx, &text);
        self.badge = Some(text);
    }
}

#[cfg(test)]
mod tests {
    use super::super::alerts::alert_calendar;
    use super::*;
    use bifrost_app::notification_schedule::Trigger;
    use bifrost_app::proto::{Configure, ToShell};

    // 2027-01-15 08:00 UTC is day 20833; the events below are on day 20834 (tomorrow) or day 20833 (today).
    const NOW: i64 = 1_800_000_000;
    const TODAY: i64 = 20_833;
    const TOMORROW: i64 = 20_834;
    const OFF: i32 = 0;

    fn event(id: u64, title: &str, day: i64, minute: u16, alert: u8) -> CalendarEvent {
        CalendarEvent {
            id,
            date: Date::from_days(day),
            minute,
            duration: 60,
            all_day: false,
            title: title.into(),
            location: String::new(),
            notes: String::new(),
            repeat: 0,
            alert,
        }
    }

    fn plan(events: &[CalendarEvent], now: i64) -> BTreeMap<String, Schedule> {
        plan_alerts(&alert_calendar(events, &IcsCalendar::default()), now, OFF)
    }

    fn ctx() -> Ctx {
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(crate::test_ui(), cfg)
    }

    /// (service, method) of each service call the context queued, oldest first.
    fn calls(ctx: &Ctx) -> Vec<(String, String)> {
        ctx.take_sent()
            .into_iter()
            .filter_map(|m| match m {
                ToShell::ServiceCall { service, method, .. } => Some((service, method)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn an_alarm_becomes_a_calendar_trigger_on_its_weekday() {
        // 10:00 tomorrow with an alarm 15 minutes before: fires at 09:45 on that weekday, once.
        let plan = plan(&[event(7, "Standup", TOMORROW, 600, 3)], NOW);
        assert_eq!(plan.len(), 1);
        let s = plan.values().next().unwrap();
        assert!(s.id.starts_with(ALERT_PREFIX) && s.id.ends_with(":15"), "id {}", s.id);
        assert_eq!(s.title, "Standup");
        assert_eq!(s.thread, "calendar");
        let weekday = Date::from_days(TOMORROW).weekday() as u8 + 1;
        assert_eq!(s.trigger, Trigger::Calendar { hour: 9, minute: 45, weekdays: vec![weekday], repeats: false });
    }

    #[test]
    fn an_edit_replaces_its_request_and_a_delete_removes_it() {
        let before = plan(&[event(1, "A", TOMORROW, 600, 3), event(2, "B", TOMORROW, 700, 3)], NOW);
        assert_eq!(before.len(), 2);
        // A moves 30 minutes later and keeps its identifier; B is gone.
        let after = plan(&[event(1, "A moved", TOMORROW, 630, 3)], NOW);
        let (adds, removes) = diff(&before, &after);
        assert_eq!(adds.len(), 1);
        assert_eq!(adds[0].title, "A moved");
        let gone: Vec<String> = before.keys().filter(|k| !after.contains_key(*k)).cloned().collect();
        assert_eq!(gone.len(), 1);
        assert_eq!(removes, gone);
    }

    #[test]
    fn an_unchanged_plan_sends_nothing() {
        let a = plan(&[event(1, "A", TOMORROW, 600, 3)], NOW);
        let (adds, removes) = diff(&a, &a.clone());
        assert!(adds.is_empty() && removes.is_empty());
    }

    #[test]
    fn up_next_lists_today_s_events_that_have_not_started() {
        // Two events today after 08:00 (09:00 and 14:00) and one already past, which is left out.
        let cal = alert_calendar(&[event(1, "Late", TODAY, 840, 0), event(2, "Early", TODAY, 540, 0), event(3, "Past", TODAY, 300, 0)], &IcsCalendar::default());
        let entries = up_next_entries(&cal, NOW, OFF);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].title, "Early");
        assert_eq!(entries[0].date, NOW as f64, "the first entry is shown from now");
        assert_eq!(entries[1].title, "Late");
        assert_eq!(entries[1].date, (TODAY * 86_400 + 540 * 60) as f64, "the next entry starts when the first event starts");
    }

    #[test]
    fn up_next_says_so_when_nothing_is_left_today() {
        let entries = up_next_entries(&IcsCalendar::default(), NOW, OFF);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "No more events today");
    }

    #[test]
    fn the_badge_is_today_s_day_of_the_month() {
        // 2027-01-15 in UTC: the day is 15 at any offset that keeps it on that date.
        assert_eq!(badge_text(NOW, OFF), "15");
    }

    #[test]
    fn only_a_missing_notification_schedule_service_turns_on_the_fallback() {
        let mut c = Connect::default();
        assert!(!c.fallback());
        c.on_reply(&Err("no service widgets_timeline".into()));
        assert!(!c.fallback(), "another service's absence is not this one's");
        c.on_reply(&Ok(Json::Null));
        assert!(!c.fallback());
        c.on_reply(&Err("no service notification-schedule".into()));
        assert!(c.fallback());
    }

    #[test]
    fn a_sync_schedules_replaces_removes_and_refreshes_the_widget_and_badge() {
        let ctx = ctx();
        let mut c = Connect::default();
        let rich = IcsCalendar::default();
        assert!(c.is_stale(1, TODAY));

        let two = [event(1, "A", TOMORROW, 600, 3), event(2, "B", TOMORROW, 700, 3)];
        c.sync(&ctx, &Snap { events: &two, rich: &rich, extras: &Extras::default(), now: NOW, offset: OFF }, 1, TODAY);
        assert!(!c.is_stale(1, TODAY));
        let first = calls(&ctx);
        let adds = first.iter().filter(|(s, m)| s == notification_schedule::SERVICE && m == "add").count();
        assert_eq!(adds, 2);
        assert!(first.contains(&(widgets_timeline::SERVICE.to_string(), "reload".to_string())));
        assert!(first.contains(&(dock_tile::SERVICE.to_string(), "badge".to_string())));

        // The same store again: nothing is sent.
        c.sync(&ctx, &Snap { events: &two, rich: &rich, extras: &Extras::default(), now: NOW, offset: OFF }, 1, TODAY);
        assert!(calls(&ctx).is_empty());

        // B is deleted: one remove, no add.
        let one = [event(1, "A", TOMORROW, 600, 3)];
        c.sync(&ctx, &Snap { events: &one, rich: &rich, extras: &Extras::default(), now: NOW, offset: OFF }, 2, TODAY);
        let del = calls(&ctx);
        assert_eq!(del.iter().filter(|(s, m)| s == notification_schedule::SERVICE && m == "remove").count(), 1);
        assert!(!del.iter().any(|(s, m)| s == notification_schedule::SERVICE && m == "add"));
    }

    #[test]
    fn with_no_alert_service_the_sync_schedules_nothing_but_keeps_the_widget() {
        let ctx = ctx();
        let mut c = Connect::default();
        c.on_reply(&Err("no service notification-schedule".into()));
        let rich = IcsCalendar::default();
        let evs = [event(1, "A", TOMORROW, 600, 3)];
        c.sync(&ctx, &Snap { events: &evs, rich: &rich, extras: &Extras::default(), now: NOW, offset: OFF }, 1, TODAY);
        let sent = calls(&ctx);
        assert!(!sent.iter().any(|(s, _)| s == notification_schedule::SERVICE));
        assert!(sent.iter().any(|(s, _)| s == widgets_timeline::SERVICE));
    }
}
