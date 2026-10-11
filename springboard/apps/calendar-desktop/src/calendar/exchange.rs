//! Import / export glue between the event store and ICS: whole-calendar and
//! single-event files, recurring imports materialised into the store, and the
//! rich-event sidecar (`calendar.ics`) that keeps rules, alerts and notes.
use super::ics::{self, IcsCalendar, IcsEvent};
use super::search::{self, Hit};
use super::store::{self, CalendarEvent};
use super::{recur, Calendar, Date};
use std::fs;
use std::path::{Path, PathBuf};

/// Recurring imports are expanded this far around today.
pub const DAYS_BEFORE: i32 = 31;
pub const DAYS_AFTER: i32 = 366;

pub fn rich_path(events_path: &Path) -> PathBuf {
    events_path.with_file_name("calendar.ics")
}

pub fn load_rich(path: &Path) -> IcsCalendar {
    match fs::read(path) {
        Ok(bytes) if bytes.len() <= 8 * 1024 * 1024 => ics::parse(&String::from_utf8_lossy(&bytes)),
        _ => IcsCalendar::default(),
    }
}

/// Dates on which `e` occurs inside `from..=to`. Non-recurring events ignore
/// the window.
pub fn occurrences(e: &IcsEvent, from: Date, to: Date) -> Vec<Date> {
    match &e.rule {
        Some(r) => recur::expand(r, e.start.date, &e.exdates, from, to),
        None => vec![e.start.date],
    }
}

/// Store events for every occurrence of `events` that the store does not
/// already hold (same date, time and title). Ids are assigned from `existing`.
pub fn materialize(events: &[IcsEvent], from: Date, to: Date, existing: &[CalendarEvent]) -> Vec<CalendarEvent> {
    let mut all = existing.to_vec();
    let mut added = vec![];
    for e in events {
        let Some(base) = ics::to_calendar_event(e, 1) else { continue };
        for date in occurrences(e, from, to) {
            if all.len() >= store::MAX_EVENTS {
                return added;
            }
            let dup = all
                .iter()
                .any(|x| x.date == date && x.minute == base.minute && x.all_day == base.all_day && x.title == base.title);
            if dup {
                continue;
            }
            let ev = CalendarEvent { id: store::next_id(&all), date, ..base.clone() };
            all.push(ev.clone());
            added.push(ev);
        }
    }
    added
}

/// Store events plus rich events as one calendar. Store events that merely
/// materialise an occurrence of a rich event are left out so nothing is
/// exported twice.
pub fn build_export(name: &str, store: &[CalendarEvent], rich: &IcsCalendar) -> IcsCalendar {
    let mut events: Vec<IcsEvent> = store
        .iter()
        .filter(|s| {
            !rich.events.iter().any(|r| {
                r.summary == s.title
                    && r.all_day == s.all_day
                    && (s.all_day || r.start.minute == s.minute)
                    && occurrences(r, s.date, s.date).contains(&s.date)
            })
        })
        .map(ics::from_calendar_event)
        .collect();
    events.extend(rich.events.iter().cloned());
    IcsCalendar { name: name.into(), events, timezones: rich.timezones.clone() }
}

impl Calendar {
    /// Imports an .ics file. Returns how many events were added to the store.
    pub fn import_ics(&mut self, text: &str) -> Result<usize, String> {
        if self.storage_blocked {
            return Err("Saved events could not be read. Import is unavailable.".into());
        }
        let cal = ics::parse(text);
        if cal.events.is_empty() {
            return Err("No events found in this file".into());
        }
        let from = self.today.shift_days(-DAYS_BEFORE);
        let to = self.today.shift_days(DAYS_AFTER);
        let added = materialize(&cal.events, from, to, &self.events);
        let mut events = self.events.clone();
        events.extend(added.iter().cloned());
        store::sort(&mut events);
        store::save(&self.path, &events).map_err(|e| e.to_string())?;
        self.events = events;
        self.events_gen = self.events_gen.wrapping_add(1);
        self.dirty = true;
        // Keep what the store cannot hold (rules, alerts) for alerts/export.
        let rp = rich_path(&self.path);
        let mut rich = load_rich(&rp);
        for e in cal.events.iter().filter(|e| e.rule.is_some() || !e.alarms.is_empty()) {
            rich.events.retain(|r| r.uid != e.uid);
            rich.events.push(e.clone());
        }
        for z in cal.timezones {
            if !rich.timezones.iter().any(|(n, _)| *n == z.0) {
                rich.timezones.push(z);
            }
        }
        springboard::shell::fsutil::write_atomic(&rp, &ics::write(&rich, &ics::stamp(self.wall.unix))).map_err(|e| e.to_string())?;
        Ok(added.len())
    }

    /// The whole calendar as an .ics file.
    pub fn export_ics(&self) -> String {
        let rich = load_rich(&rich_path(&self.path));
        let cal = build_export("SpringBoard", &self.events, &rich);
        ics::write(&cal, &ics::stamp(self.wall.unix))
    }

    /// One event as an .ics file.
    pub fn export_event_ics(&self, id: u64) -> Option<String> {
        let e = self.events.iter().find(|e| e.id == id)?;
        let rich = load_rich(&rich_path(&self.path));
        let ev = ics::from_calendar_event(e);
        Some(ics::write_single(&ev, &rich.timezones, &ics::stamp(self.wall.unix)))
    }

    pub fn search(&self, query: &str) -> Vec<Hit> {
        let rich = load_rich(&rich_path(&self.path));
        search::search(query, &self.events, &rich.events)
    }
}

#[cfg(test)]
mod tests {
    use super::super::ics::Local;
    use super::*;

    fn d(month: u32, day: u32) -> Date {
        Date { year: 2026, month, day }
    }
    fn rich(title: &str, rule: &str) -> IcsEvent {
        let mut e = IcsEvent::new("r@x", title, Local::new(d(10, 5), 540), Local::new(d(10, 5), 600));
        e.rule = recur::parse_rrule(rule);
        e
    }
    fn stored(id: u64, date: Date, title: &str) -> CalendarEvent {
        CalendarEvent { id, date, minute: 540, duration: 60, all_day: false, title: title.into(), location: String::new(), notes: String::new(), repeat: 0, alert: 0 }
    }

    #[test]
    fn materialize_expands_rules_within_window() {
        let e = rich("Gym", "FREQ=WEEKLY;COUNT=10");
        let got = materialize(&[e], d(10, 1), d(10, 20), &[]);
        let days: Vec<u32> = got.iter().map(|x| x.date.day).collect();
        assert_eq!(days, vec![5, 12, 19]);
        assert!(got.iter().all(|x| x.valid()));
        let ids: std::collections::HashSet<_> = got.iter().map(|x| x.id).collect();
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn materialize_skips_existing_and_keeps_ids_unique() {
        let e = rich("Gym", "FREQ=DAILY;COUNT=3");
        let existing = vec![stored(5, d(10, 6), "Gym")];
        let got = materialize(&[e], d(10, 1), d(10, 31), &existing);
        let days: Vec<u32> = got.iter().map(|x| x.date.day).collect();
        assert_eq!(days, vec![5, 7]);
        assert!(got.iter().all(|x| x.id != 5));
    }

    #[test]
    fn single_events_ignore_the_window() {
        let mut e = rich("Old", "FREQ=DAILY");
        e.rule = None;
        let got = materialize(&[e], d(12, 1), d(12, 31), &[]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].date, d(10, 5));
    }

    #[test]
    fn export_does_not_duplicate_materialised_occurrences() {
        let r = rich("Gym", "FREQ=WEEKLY;COUNT=10");
        let rich_cal = IcsCalendar { name: String::new(), events: vec![r], timezones: vec![] };
        let store = vec![stored(1, d(10, 12), "Gym"), stored(2, d(10, 13), "Gym"), stored(3, d(10, 6), "Lunch")];
        let out = build_export("Mine", &store, &rich_cal);
        assert_eq!(out.name, "Mine");
        let titles: Vec<(&str, Date)> = out.events.iter().map(|e| (e.summary.as_str(), e.start.date)).collect();
        assert_eq!(titles, vec![("Gym", d(10, 13)), ("Lunch", d(10, 6)), ("Gym", d(10, 5))]);
        let text = ics::write(&out, "20261005T000000Z");
        assert_eq!(ics::parse(&text).events.len(), 3);
    }
}
