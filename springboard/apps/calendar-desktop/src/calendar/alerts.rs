//! Event alerts: pure computation of which alarms fall in a time window, and
//! the `AlertState` the app polls to post due ones as notifications.
use super::exchange;
use super::ics::{IcsCalendar, IcsEvent};
use super::store::CalendarEvent;
use super::{recur, Calendar, Date};
use springboard::shell::clock::Wall;

/// How far ahead one scan looks; the state rescans when it runs out.
const SCAN_SECS: i64 = 3 * 86_400;
const REPEAT_RULES: [&str; 6] = ["", "FREQ=DAILY", "FREQ=WEEKLY", "FREQ=WEEKLY;INTERVAL=2", "FREQ=MONTHLY", "FREQ=YEARLY"];
/// Minutes before the start for each `CalendarEvent::alert` choice.
const ALERT_MINUTES: [Option<u32>; 7] = [None, Some(0), Some(5), Some(15), Some(30), Some(60), Some(1440)];

/// Store events plus rich events as one calendar whose store events carry
/// their alert and repeat choices, which the .ics conversion leaves out.
pub fn alert_calendar(store: &[CalendarEvent], rich: &IcsCalendar) -> IcsCalendar {
    let mut cal = exchange::build_export("", store, rich);
    for e in &mut cal.events {
        let id = e.uid.strip_suffix("@springboard.tab").and_then(|i| i.parse::<u64>().ok());
        let Some(s) = id.and_then(|id| store.iter().find(|s| s.id == id)) else { continue };
        e.alarms = ALERT_MINUTES.get(s.alert as usize).copied().flatten().into_iter().collect();
        e.rule = REPEAT_RULES.get(s.repeat as usize).and_then(|r| recur::parse_rrule(r));
    }
    cal
}

#[derive(Default)]
pub struct AlertState {
    primed: bool,
    refresh: bool,
    /// Unix second up to which alerts were already handled.
    last: i64,
    next: Option<i64>,
    gen: u32,
    offset: i32,
}

/// What the app should post now; split out so the rule is host-testable.
/// The calendar is built only when an alert is due, the events changed
/// (`gen`), the zone changed or the scan window ran out.
pub fn step(state: &mut AlertState, now: i64, gen: u32, offset: i32, load: impl FnOnce() -> IcsCalendar) -> Vec<Alert> {
    if !state.primed {
        state.primed = true;
        state.refresh = true;
        state.last = now;
        state.offset = offset;
    }
    if state.offset != offset {
        state.offset = offset;
        state.refresh = true;
    }
    let due_now = state.next.is_some_and(|n| n <= now);
    if !(due_now || state.refresh || gen != state.gen) {
        return vec![];
    }
    state.gen = gen;
    state.refresh = false;
    let cal = load();
    let from = state.last;
    state.last = now;
    state.next = Some(due(&cal, now, now + SCAN_SECS, offset).first().map_or(now + SCAN_SECS, |a| a.fire));
    due(&cal, from, now, offset)
}

impl AlertState {
    /// Monotonic ms of the next alert (or rescan), for the app's wake computation.
    pub fn wake(&self, wall: &Wall) -> Option<f64> {
        self.next.map(|n| wall.mono(n))
    }
}

impl Calendar {
    /// Alerts that came due since the last call; the app posts each as a notification.
    pub fn take_alerts(&mut self, t: f64) -> Vec<Alert> {
        let (events, path) = (&self.events, &self.path);
        let load = || alert_calendar(events, &exchange::load_rich(&exchange::rich_path(path)));
        step(&mut self.alerts, self.wall.now(t), self.events_gen, self.wall.offset, load)
    }

    /// Monotonic ms of the next alert.
    pub fn alert_wake(&self) -> Option<f64> {
        self.alerts.wake(&self.wall)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alert {
    pub fire: i64,
    pub start: i64,
    pub uid: String,
    pub title: String,
    pub body: String,
}

fn utc_offset_secs(e: &IcsEvent, zones: &[(String, i32)], device: i32) -> i32 {
    match e.tzid.as_deref() {
        Some("UTC") => 0,
        Some(tz) => zones.iter().find(|(n, _)| n == tz).map_or(device, |&(_, m)| m * 60),
        None => device,
    }
}

fn date_of(unix: i64, offset: i32) -> Date {
    Date::from_days((unix + offset as i64).div_euclid(86_400))
}

/// Alerts with `from < fire <= to`, ordered by fire time. `device_offset`
/// (seconds east of UTC) applies to floating and all-day events.
pub fn due(cal: &IcsCalendar, from: i64, to: i64, device_offset: i32) -> Vec<Alert> {
    let mut out = vec![];
    if to <= from {
        return out;
    }
    for e in cal.events.iter().filter(|e| !e.alarms.is_empty()) {
        let off = utc_offset_secs(e, &cal.timezones, device_offset);
        let longest = *e.alarms.iter().max().unwrap_or(&0) as i64 * 60;
        let lo = date_of(from, off).shift_days(-1);
        let hi = date_of(to + longest, off).shift_days(1);
        let dates = match &e.rule {
            Some(r) => recur::expand(r, e.start.date, &e.exdates, lo, hi),
            None if e.start.date >= lo && e.start.date <= hi && !e.exdates.contains(&e.start.date) => {
                vec![e.start.date]
            }
            None => vec![],
        };
        for date in dates {
            let start = date.days() * 86_400 + e.start.minute as i64 * 60 - off as i64;
            for &m in &e.alarms {
                let fire = start - m as i64 * 60;
                if fire > from && fire <= to {
                    out.push(Alert {
                        fire,
                        start,
                        uid: e.uid.clone(),
                        title: e.summary.clone(),
                        body: body(e, m),
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| (a.fire, &a.uid).cmp(&(b.fire, &b.uid)));
    out
}

fn body(e: &IcsEvent, before: u32) -> String {
    let when = if e.all_day {
        "All day".to_string()
    } else if before == 0 {
        "Now".to_string()
    } else if before % 1440 == 0 {
        format!("In {} day{}", before / 1440, if before == 1440 { "" } else { "s" })
    } else if before % 60 == 0 {
        format!("In {} hour{}", before / 60, if before == 60 { "" } else { "s" })
    } else {
        format!("In {before} minutes")
    };
    if e.location.is_empty() {
        when
    } else {
        format!("{when} - {}", e.location)
    }
}

#[cfg(test)]
mod tests {
    use super::super::ics::Local;
    use super::*;

    fn d(month: u32, day: u32) -> Date {
        Date { year: 2026, month, day }
    }
    fn unix(date: Date, h: i64, m: i64, off: i32) -> i64 {
        date.days() * 86_400 + h * 3600 + m * 60 - off as i64
    }
    fn cal(e: IcsEvent) -> IcsCalendar {
        IcsCalendar { name: String::new(), events: vec![e], timezones: vec![] }
    }
    fn event(alarms: Vec<u32>) -> IcsEvent {
        let mut e = IcsEvent::new("u", "Dentist", Local::new(d(10, 5), 600), Local::new(d(10, 5), 660));
        e.alarms = alarms;
        e
    }

    #[test]
    fn single_alert_in_window() {
        let c = cal(event(vec![15]));
        let fire = unix(d(10, 5), 9, 45, 0);
        let got = due(&c, fire - 60, fire, 0);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].fire, fire);
        assert_eq!(got[0].start, unix(d(10, 5), 10, 0, 0));
        assert_eq!(got[0].body, "In 15 minutes");
        // window is (from, to]: already fired alerts are not repeated
        assert!(due(&c, fire, fire + 60, 0).is_empty());
    }

    #[test]
    fn device_offset_applies_to_floating_events() {
        let c = cal(event(vec![0]));
        let fire = unix(d(10, 5), 10, 0, 3600);
        assert_eq!(due(&c, fire - 1, fire, 3600).len(), 1);
        assert!(due(&c, fire - 1, fire, 0).is_empty());
    }

    #[test]
    fn utc_and_named_zones() {
        let mut e = event(vec![0]);
        e.tzid = Some("UTC".into());
        let fire = unix(d(10, 5), 10, 0, 0);
        assert_eq!(due(&cal(e.clone()), fire - 1, fire, 7200).len(), 1);
        e.tzid = Some("Zone/X".into());
        let c = IcsCalendar { name: String::new(), events: vec![e], timezones: vec![("Zone/X".into(), -300)] };
        let fire = unix(d(10, 5), 10, 0, -300 * 60);
        assert_eq!(due(&c, fire - 1, fire, 0).len(), 1);
    }

    #[test]
    fn recurring_events_alert_each_occurrence_except_exceptions() {
        let mut e = event(vec![10]);
        e.rule = recur::parse_rrule("FREQ=DAILY;COUNT=4");
        e.exdates = vec![d(10, 6)];
        let c = cal(e);
        let from = unix(d(10, 4), 0, 0, 0);
        let got = due(&c, from, unix(d(10, 9), 0, 0, 0), 0);
        let fires: Vec<i64> = got.iter().map(|a| a.fire).collect();
        let want: Vec<i64> = [5, 7, 8].iter().map(|&day| unix(d(10, day), 9, 50, 0)).collect();
        assert_eq!(fires, want);
    }

    #[test]
    fn day_before_alert_crosses_midnight() {
        let c = cal(event(vec![1440, 15]));
        let got = due(&c, unix(d(10, 3), 0, 0, 0), unix(d(10, 6), 0, 0, 0), 0);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].fire, unix(d(10, 4), 10, 0, 0));
        assert_eq!(got[0].body, "In 1 day");
    }

    #[test]
    fn location_and_all_day_text() {
        let mut e = event(vec![60]);
        e.location = "Room 4".into();
        assert_eq!(body(&e, 60), "In 1 hour - Room 4");
        e.all_day = true;
        e.location.clear();
        assert_eq!(body(&e, 0), "All day");
    }

    fn stored(id: u64, alert: u8, repeat: u8) -> CalendarEvent {
        CalendarEvent {
            id,
            date: d(10, 5),
            minute: 600,
            duration: 60,
            all_day: false,
            title: "Gym".into(),
            location: String::new(),
            notes: String::new(),
            repeat,
            alert,
        }
    }

    #[test]
    fn store_alert_and_repeat_choices_become_alarms_and_rules() {
        let c = alert_calendar(&[stored(7, 3, 2)], &IcsCalendar::default());
        assert_eq!(c.events.len(), 1);
        assert_eq!(c.events[0].alarms, vec![15]);
        assert!(c.events[0].rule.is_some());
        let none = alert_calendar(&[stored(8, 0, 0)], &IcsCalendar::default());
        assert!(none.events[0].alarms.is_empty() && none.events[0].rule.is_none());
    }

    #[test]
    fn a_stored_event_alert_fires_once_and_the_state_stays_idle_between() {
        let mut st = AlertState::default();
        let fire = unix(d(10, 5), 9, 45, 0);
        let load = || alert_calendar(&[stored(7, 3, 0)], &IcsCalendar::default());
        assert!(step(&mut st, fire - 3600, 0, 0, load).is_empty(), "priming never fires the past");
        assert_eq!(st.next, Some(fire));
        let mut loaded = false;
        let quiet = step(&mut st, fire - 60, 0, 0, || {
            loaded = true;
            IcsCalendar::default()
        });
        assert!(quiet.is_empty() && !loaded, "no rebuild before the next alert");
        let got = step(&mut st, fire + 2, 0, 0, load);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].title, "Gym");
        assert_eq!(got[0].body, "In 15 minutes");
        assert!(step(&mut st, fire + 60, 0, 0, load).is_empty());
    }

    #[test]
    fn saved_events_and_a_zone_change_trigger_a_rescan() {
        let mut st = AlertState::default();
        let now = unix(d(10, 5), 0, 0, 0);
        step(&mut st, now, 0, 0, IcsCalendar::default);
        assert_eq!(st.next, Some(now + SCAN_SECS), "empty calendars rescan at the horizon");
        let load = || alert_calendar(&[stored(7, 1, 0)], &IcsCalendar::default());
        step(&mut st, now + 10, 1, 0, load);
        assert_eq!(st.next, Some(unix(d(10, 5), 10, 0, 0)));
        step(&mut st, now + 20, 1, 3600, load);
        assert_eq!(st.next, Some(unix(d(10, 5), 10, 0, 3600)));
    }

    #[test]
    fn events_without_alarms_and_empty_windows_are_ignored() {
        assert!(due(&cal(event(vec![])), 0, i64::MAX / 4, 0).is_empty());
        assert!(due(&cal(event(vec![5])), 100, 100, 0).is_empty());
    }
}
