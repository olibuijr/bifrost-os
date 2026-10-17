//! The `calendar` service: the person's events, for the apps whose manifest declares the `calendars` permission. It is
//! the read side of the event store (events in a date range); the store is the shared calendar file the Calendar app
//! writes, `shell::calendar::store`.
//!
//! Times are local, floating civil times written `YYYY-MM-DDTHH:MM` (no time zone).
//!
//! Methods (params are a JSON object):
//! - `access {}` returns `{status}`: `authorized` when the app declares `calendars`, else `denied`. An app cannot
//!   grant itself anything by asking; the manifest is the declaration.
//! - `events {start, end}` returns `{events: [{id, title, start, end, all_day, location}]}`: every event that overlaps
//!   `[start, end)`, sorted by start then id. Both bounds are required and `end` must be after `start`. An all-day
//!   event spans its whole civil day. A repeating event appears once for each occurrence that overlaps the range, all
//!   with its id; an occurrence that starts the day before the range and runs into it is included.
//!
//! - `save_event {title, start, end, all_day, location}` appends one event to the shared store and returns `{id}`. A
//!   timed event lasts 15 minutes to 24 hours; an all-day event takes the civil day of `start`. The store is read and
//!   rewritten whole, so a save made while the Calendar app is writing can be lost; the Calendar app is the owner.
//!
//! Without the `calendars` permission every method but `access` is an error.
use super::{Shell, SystemService};
use crate::shell::calendar::store::{self, CalendarEvent};
use crate::shell::calendar::Date;
use bifrost_core::json::Json;
use std::path::{Path, PathBuf};

/// The manifest permission that lets an app read events.
pub const PERMISSION: &str = "calendars";
/// The package id of the Calendar app, whose data folder holds the store.
pub const STORE_APP: &str = "is.olibuijr.calendar.desktop";

/// The shared event file: `appdata/<Calendar app id>/events.tsv` under the shell's data directory.
pub fn store_path(data_dir: &Path) -> PathBuf {
    data_dir.join("appdata").join(STORE_APP).join("events.tsv")
}

/// A civil moment as minutes since 1970-01-01 00:00 (floating local time). Dates are validated; hours and minutes
/// must be in range.
fn parse_moment(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    if b.len() != 16 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' {
        return None;
    }
    let num = |s: &str| -> Option<u32> { if s.bytes().all(|c| c.is_ascii_digit()) { s.parse().ok() } else { None } };
    let date = Date { year: num(&text[0..4])? as i64, month: num(&text[5..7])?, day: num(&text[8..10])? };
    let (hour, minute) = (num(&text[11..13])?, num(&text[14..16])?);
    if !date.valid() || hour > 23 || minute > 59 {
        return None;
    }
    Some(date.days() * 1440 + (hour * 60 + minute) as i64)
}

/// The text of a moment from [`parse_moment`].
fn format_moment(moment: i64) -> String {
    let date = Date::from_days(moment.div_euclid(1440));
    let minute = moment.rem_euclid(1440);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}", date.year, date.month, date.day, minute / 60, minute % 60)
}

/// The moment an occurrence of `e` on the civil day `day` (days since 1970-01-01) starts and ends, in the same units as
/// [`parse_moment`]. An all-day event covers its civil day.
fn span_on(e: &CalendarEvent, day: i64) -> (i64, i64) {
    let start = day * 1440;
    if e.all_day {
        (start, start + 1440)
    } else {
        let at = start + e.minute as i64;
        (at, at + e.duration as i64)
    }
}

/// The start and end of every occurrence of `e` that overlaps `[start, end)`, in order. A plain event has one, on its
/// own date. A repeating event has one on each day its rule names. The day before the range is checked too, because an
/// event that starts late in that day can run into the range.
fn occurrences(e: &CalendarEvent, start: i64, end: i64) -> Vec<(i64, i64)> {
    if e.repeat == 0 {
        let (from, to) = span_on(e, e.date.days());
        return if from < end && to > start { vec![(from, to)] } else { Vec::new() };
    }
    let first = (start.div_euclid(1440) - 1).max(e.date.days());
    let last = (end - 1).div_euclid(1440);
    (first..=last)
        .filter(|&day| e.occurs_on(Date::from_days(day)))
        .map(|day| span_on(e, day))
        .filter(|&(from, to)| from < end && to > start)
        .collect()
}

/// The JSON of one occurrence of an event, given its start and end.
fn event_json(e: &CalendarEvent, (start, end): (i64, i64)) -> Json {
    Json::Obj(vec![
        ("id".into(), Json::Num(e.id as f64)),
        ("title".into(), Json::Str(e.title.clone())),
        ("start".into(), Json::Str(format_moment(start))),
        ("end".into(), Json::Str(format_moment(end))),
        ("all_day".into(), Json::Bool(e.all_day)),
        ("location".into(), Json::Str(e.location.clone())),
    ])
}

/// The longest range one `events` call may name, in days. It bounds the occurrences a repeating event expands to.
pub const MAX_RANGE_DAYS: i64 = 3660;

/// The `[start, end)` range named in the params.
fn range_of(params: &Json) -> Result<(i64, i64), String> {
    let text = |key: &str| params.get(key).and_then(Json::as_str).ok_or_else(|| format!("{key} is required (YYYY-MM-DDTHH:MM)"));
    let start = parse_moment(text("start")?).ok_or("start is not a YYYY-MM-DDTHH:MM time")?;
    let end = parse_moment(text("end")?).ok_or("end is not a YYYY-MM-DDTHH:MM time")?;
    if end <= start {
        return Err("end must be after start".into());
    }
    if end - start > MAX_RANGE_DAYS * 1440 {
        return Err(format!("the range may span at most {MAX_RANGE_DAYS} days"));
    }
    Ok((start, end))
}

/// The answer to one method call by `app_id`. `permitted` says whether the app declares the `calendars` permission,
/// and `events` is the store's list (read by the caller only for an `events` call by a permitted app). Pure: no file
/// access here.
pub fn answer(app_id: &str, permitted: bool, method: &str, params: &Json, events: &[CalendarEvent]) -> Result<Json, String> {
    match method {
        "access" => {
            let status = if permitted { "authorized" } else { "denied" };
            Ok(Json::Obj(vec![("status".into(), Json::Str(status.into()))]))
        }
        "events" => {
            if !permitted {
                return Err(format!("{app_id} has no {PERMISSION} permission"));
            }
            let (start, end) = range_of(params)?;
            let mut hits: Vec<(i64, &CalendarEvent, (i64, i64))> = events
                .iter()
                .flat_map(|e| occurrences(e, start, end).into_iter().map(move |span| (span.0, e, span)))
                .collect();
            hits.sort_by_key(|(from, e, _)| (*from, e.id));
            let rows = hits.into_iter().map(|(_, e, span)| event_json(e, span)).collect();
            Ok(Json::Obj(vec![("events".into(), Json::Arr(rows))]))
        }
        _ => Err(format!("calendar has no method {method}")),
    }
}

/// The event a `save_event` call names, with the next free id. Pure: the caller stores the list. The params are
/// `{title, start, end, all_day, location}`: a timed event lasts 15 minutes to 24 hours from `start` to `end`; an
/// all-day event takes the civil day of `start` and may not run past the next midnight.
pub fn new_event(params: &Json, events: &[CalendarEvent]) -> Result<CalendarEvent, String> {
    let text = |key: &str| params.get(key).and_then(Json::as_str).ok_or_else(|| format!("{key} is required"));
    let title = text("title")?.to_string();
    let location = params.get("location").and_then(Json::as_str).unwrap_or("").to_string();
    let all_day = matches!(params.get("all_day"), Some(Json::Bool(true)));
    let (start, end) = range_of(params)?;
    let (minute, duration) = if all_day {
        if end - start > 1440 {
            return Err("an all-day event spans one civil day".into());
        }
        (0, 1440)
    } else {
        let len = end - start;
        if !(15..=1440).contains(&len) {
            return Err("a timed event lasts between 15 minutes and 24 hours".into());
        }
        (start.rem_euclid(1440) as u16, len as u16)
    };
    if events.len() >= store::MAX_EVENTS {
        return Err("the calendar is full".into());
    }
    let event = CalendarEvent {
        id: store::next_id(events),
        date: Date::from_days(start.div_euclid(1440)),
        minute,
        duration,
        all_day,
        title,
        location,
        notes: String::new(),
        repeat: 0,
        alert: 0,
    };
    if !event.valid() {
        return Err("the event has an invalid title or location".into());
    }
    Ok(event)
}

/// Appends the event a `save_event` call names to the shared store and returns `{id}`.
fn save_event(shell: &Shell, params: &Json) -> Result<Json, String> {
    let path = store_path(&shell.data_dir);
    let mut events = store::load(&path).map_err(|e| e.to_string())?;
    let event = new_event(params, &events)?;
    let id = event.id;
    events.push(event);
    store::save(&path, &events).map_err(|e| e.to_string())?;
    Ok(Json::Obj(vec![("id".into(), Json::Num(id as f64))]))
}

pub struct Calendar;

impl SystemService for Calendar {
    fn name(&self) -> &'static str {
        "calendar"
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        let permitted = super::app_has_permission(app_id, PERMISSION);
        if method == "save_event" {
            if !permitted {
                return Err(format!("{app_id} has no {PERMISSION} permission"));
            }
            return save_event(shell, params);
        }
        let events = if method == "events" && permitted {
            store::load(&store_path(&shell.data_dir)).map_err(|e| e.to_string())?
        } else {
            Vec::new()
        };
        answer(app_id, permitted, method, params, &events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: u64, date: (i64, u32, u32), minute: u16, duration: u16, all_day: bool, title: &str) -> CalendarEvent {
        CalendarEvent {
            id,
            date: Date { year: date.0, month: date.1, day: date.2 },
            minute,
            duration,
            all_day,
            title: title.into(),
            location: "Hall".into(),
            notes: String::new(),
            repeat: 0,
            alert: 0,
        }
    }

    fn range(start: &str, end: &str) -> Json {
        Json::Obj(vec![("start".into(), Json::Str(start.into())), ("end".into(), Json::Str(end.into()))])
    }

    fn ids(reply: &Json) -> Vec<u64> {
        reply.get("events").and_then(Json::as_array).unwrap().iter().filter_map(|e| e.get("id").and_then(Json::as_u64)).collect()
    }

    fn day_book() -> Vec<CalendarEvent> {
        vec![
            event(1, (2026, 10, 13), 9 * 60, 60, false, "Standup"),
            event(2, (2026, 10, 12), 23 * 60 + 30, 60, false, "Late call"),
            event(3, (2026, 10, 14), 0, 30, false, "Tomorrow early"),
            event(4, (2026, 10, 13), 8 * 60, 30, false, "Coffee"),
        ]
    }

    #[test]
    fn events_in_a_range_are_the_overlapping_ones_sorted_by_start() {
        let reply = answer("is.example.cal", true, "events", &range("2026-10-13T00:00", "2026-10-14T00:00"), &day_book()).unwrap();
        // event 2 starts the day before but runs into the 13th; event 3 starts exactly at the range end and is left out
        assert_eq!(ids(&reply), vec![2, 4, 1]);
    }

    #[test]
    fn an_event_that_ends_exactly_at_the_range_start_is_left_out() {
        let reply = answer("is.example.cal", true, "events", &range("2026-10-13T10:00", "2026-10-13T11:00"), &day_book()).unwrap();
        assert_eq!(ids(&reply), Vec::<u64>::new());
    }

    #[test]
    fn an_all_day_event_covers_its_whole_civil_day() {
        let book = vec![event(7, (2026, 10, 13), 0, 15, true, "Holiday")];
        let inside = answer("is.example.cal", true, "events", &range("2026-10-13T12:00", "2026-10-13T12:01"), &book).unwrap();
        assert_eq!(ids(&inside), vec![7]);
        let after = answer("is.example.cal", true, "events", &range("2026-10-14T00:00", "2026-10-14T00:30"), &book).unwrap();
        assert_eq!(ids(&after), Vec::<u64>::new());
        let all = inside.get("events").and_then(Json::as_array).unwrap();
        assert_eq!(all[0].get("start").and_then(Json::as_str), Some("2026-10-13T00:00"));
        assert_eq!(all[0].get("end").and_then(Json::as_str), Some("2026-10-14T00:00"));
    }

    #[test]
    fn each_event_carries_its_documented_fields() {
        let reply = answer("is.example.cal", true, "events", &range("2026-10-13T08:00", "2026-10-13T08:10"), &day_book()).unwrap();
        let row = &reply.get("events").and_then(Json::as_array).unwrap()[0];
        assert_eq!(row.get("id").and_then(Json::as_u64), Some(4));
        assert_eq!(row.get("title").and_then(Json::as_str), Some("Coffee"));
        assert_eq!(row.get("start").and_then(Json::as_str), Some("2026-10-13T08:00"));
        assert_eq!(row.get("end").and_then(Json::as_str), Some("2026-10-13T08:30"));
        assert_eq!(row.get("all_day"), Some(&Json::Bool(false)));
        assert_eq!(row.get("location").and_then(Json::as_str), Some("Hall"));
    }

    #[test]
    fn a_range_that_touches_no_event_is_empty() {
        let reply = answer("is.example.cal", true, "events", &range("2026-11-01T00:00", "2026-11-02T00:00"), &day_book()).unwrap();
        assert_eq!(reply.get("events").and_then(Json::as_array).map(|a| a.len()), Some(0));
    }

    #[test]
    fn events_without_the_calendars_permission_is_an_error() {
        let r = range("2026-10-13T00:00", "2026-10-14T00:00");
        assert!(answer("is.example.cal", false, "events", &r, &day_book()).is_err());
    }

    #[test]
    fn access_reports_authorized_only_with_the_permission() {
        let granted = answer("is.example.cal", true, "access", &Json::Null, &[]).unwrap();
        assert_eq!(granted.get("status").and_then(Json::as_str), Some("authorized"));
        let refused = answer("is.example.cal", false, "access", &Json::Null, &[]).unwrap();
        assert_eq!(refused.get("status").and_then(Json::as_str), Some("denied"));
    }

    #[test]
    fn a_range_must_be_valid_and_ordered() {
        let book = day_book();
        assert!(answer("is.example.cal", true, "events", &range("2026-10-14T00:00", "2026-10-13T00:00"), &book).is_err());
        assert!(answer("is.example.cal", true, "events", &range("2026-10-13T00:00", "2026-10-13T00:00"), &book).is_err());
        assert!(answer("is.example.cal", true, "events", &range("2026-13-01T00:00", "2026-10-14T00:00"), &book).is_err());
        assert!(answer("is.example.cal", true, "events", &range("2026-10-13 00:00", "2026-10-14T00:00"), &book).is_err());
        assert!(answer("is.example.cal", true, "events", &range("2026-10-13T24:00", "2026-10-14T00:00"), &book).is_err());
        assert!(answer("is.example.cal", true, "events", &Json::Null, &book).is_err());
    }

    fn starts(reply: &Json) -> Vec<String> {
        let rows = reply.get("events").and_then(Json::as_array).unwrap();
        rows.iter().filter_map(|e| e.get("start").and_then(Json::as_str).map(str::to_string)).collect()
    }

    fn repeating(mut e: CalendarEvent, repeat: u8) -> CalendarEvent {
        e.repeat = repeat;
        e
    }

    #[test]
    fn a_daily_event_appears_once_on_each_day_of_the_range() {
        let daily = repeating(event(5, (2026, 10, 13), 9 * 60, 60, false, "Stand-up"), 1);
        let reply = answer("is.example.cal", true, "events", &range("2026-10-13T00:00", "2026-10-16T00:00"), &[daily]).unwrap();
        assert_eq!(ids(&reply), vec![5, 5, 5]);
        assert_eq!(starts(&reply), vec!["2026-10-13T09:00", "2026-10-14T09:00", "2026-10-15T09:00"]);
    }

    #[test]
    fn a_weekly_event_skips_the_days_between_its_occurrences() {
        let weekly = repeating(event(6, (2026, 10, 13), 12 * 60, 30, false, "Review"), 2);
        let reply = answer("is.example.cal", true, "events", &range("2026-10-13T00:00", "2026-10-28T00:00"), &[weekly]).unwrap();
        assert_eq!(starts(&reply), vec!["2026-10-13T12:00", "2026-10-20T12:00", "2026-10-27T12:00"]);
    }

    #[test]
    fn a_repeating_event_is_not_shown_before_its_first_day() {
        let daily = repeating(event(7, (2026, 10, 13), 9 * 60, 60, false, "Start"), 1);
        let reply = answer("is.example.cal", true, "events", &range("2026-10-10T00:00", "2026-10-13T08:00"), &[daily]).unwrap();
        assert_eq!(ids(&reply), Vec::<u64>::new());
    }

    #[test]
    fn an_occurrence_that_runs_past_midnight_is_found_from_the_day_before() {
        let late = repeating(event(8, (2026, 10, 13), 23 * 60 + 30, 60, false, "Late"), 1);
        let reply = answer("is.example.cal", true, "events", &range("2026-10-14T00:00", "2026-10-14T00:30"), &[late]).unwrap();
        assert_eq!(starts(&reply), vec!["2026-10-13T23:30"]);
        let end = reply.get("events").and_then(Json::as_array).unwrap()[0].get("end").and_then(Json::as_str);
        assert_eq!(end, Some("2026-10-14T00:30"));
    }

    #[test]
    fn a_repeating_all_day_event_covers_each_civil_day_it_repeats_on() {
        let holiday = repeating(event(9, (2026, 10, 13), 0, 15, true, "Holiday"), 1);
        let reply = answer("is.example.cal", true, "events", &range("2026-10-13T12:00", "2026-10-15T00:00"), &[holiday]).unwrap();
        assert_eq!(starts(&reply), vec!["2026-10-13T00:00", "2026-10-14T00:00"]);
    }

    #[test]
    fn a_range_longer_than_the_limit_is_refused() {
        let within = range("2000-01-01T00:00", "2009-12-31T00:00");
        assert!(answer("is.example.cal", true, "events", &within, &[]).is_ok());
        let beyond = range("2000-01-01T00:00", "2011-01-01T00:00");
        assert!(answer("is.example.cal", true, "events", &beyond, &[]).is_err());
    }

    #[test]
    fn an_unknown_method_is_an_error() {
        assert!(answer("is.example.cal", true, "delete", &Json::Null, &[]).is_err());
    }

    #[test]
    fn the_store_is_the_calendar_apps_events_file() {
        let path = store_path(Path::new("/root"));
        assert_eq!(path, PathBuf::from("/root/appdata/is.olibuijr.calendar.desktop/events.tsv"));
    }

    fn save_params(title: &str, start: &str, end: &str, all_day: bool) -> Json {
        Json::Obj(vec![
            ("title".into(), Json::Str(title.into())),
            ("start".into(), Json::Str(start.into())),
            ("end".into(), Json::Str(end.into())),
            ("all_day".into(), Json::Bool(all_day)),
            ("location".into(), Json::Str("Desk".into())),
        ])
    }

    #[test]
    fn a_timed_event_is_saved_with_the_next_id_and_its_civil_fields() {
        let book = day_book();
        let e = new_event(&save_params("Lunch", "2026-10-13T12:30", "2026-10-13T13:15", false), &book).unwrap();
        assert_eq!(e.id, 5);
        assert_eq!(e.date, Date { year: 2026, month: 10, day: 13 });
        assert_eq!(e.minute, 12 * 60 + 30);
        assert_eq!(e.duration, 45);
        assert!(!e.all_day);
        assert_eq!(e.title, "Lunch");
        assert_eq!(e.location, "Desk");
        assert_eq!((e.repeat, e.alert), (0, 0));
    }

    #[test]
    fn a_saved_event_appears_in_the_events_call_for_its_day() {
        let mut book = day_book();
        let e = new_event(&save_params("Lunch", "2026-10-13T12:30", "2026-10-13T13:15", false), &book).unwrap();
        book.push(e);
        let reply = answer("is.example.cal", true, "events", &range("2026-10-13T00:00", "2026-10-14T00:00"), &book).unwrap();
        // event 2 runs into the 13th from the 12th; then 4 at 08:00, 1 at 09:00 and the new event at 12:30
        assert_eq!(ids(&reply), vec![2, 4, 1, 5]);
    }

    #[test]
    fn an_all_day_event_takes_its_civil_day_and_cannot_run_past_it() {
        let ok = new_event(&save_params("Holiday", "2026-10-13T00:00", "2026-10-14T00:00", true), &[]).unwrap();
        assert!(ok.all_day);
        assert_eq!((ok.minute, ok.duration), (0, 1440));
        assert!(new_event(&save_params("Two days", "2026-10-13T00:00", "2026-10-15T00:00", true), &[]).is_err());
    }

    #[test]
    fn a_timed_event_must_last_between_fifteen_minutes_and_a_day() {
        assert!(new_event(&save_params("Blink", "2026-10-13T09:00", "2026-10-13T09:10", false), &[]).is_err());
        assert!(new_event(&save_params("Long", "2026-10-13T09:00", "2026-10-14T10:00", false), &[]).is_err());
    }

    #[test]
    fn a_save_without_a_title_or_with_a_control_character_is_refused() {
        assert!(new_event(&save_params("", "2026-10-13T09:00", "2026-10-13T10:00", false), &[]).is_err());
        assert!(new_event(&save_params("A\nB", "2026-10-13T09:00", "2026-10-13T10:00", false), &[]).is_err());
    }

    #[test]
    fn a_full_calendar_refuses_a_new_event() {
        let full: Vec<CalendarEvent> = (1..=store::MAX_EVENTS as u64).map(|id| event(id, (2026, 10, 13), 0, 60, false, "x")).collect();
        assert!(new_event(&save_params("Extra", "2026-10-13T09:00", "2026-10-13T10:00", false), &full).is_err());
    }

    #[test]
    fn a_saved_event_round_trips_through_the_store_file() {
        let path = std::env::temp_dir().join(format!("bifrost-cal-save-{}.tsv", std::process::id()));
        let e = new_event(&save_params("Lunch", "2026-10-13T12:30", "2026-10-13T13:15", false), &day_book()).unwrap();
        let mut book = day_book();
        book.push(e.clone());
        store::save(&path, &book).unwrap();
        let loaded = store::load(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(loaded.contains(&e));
    }
}
