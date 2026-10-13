//! The `calendar` service: the person's events, for the apps whose manifest declares the `calendars` permission. It is
//! the read side of EventKit (`EKEventStore` events in a date range); the store is the shared calendar file the
//! Calendar app writes, `shell::calendar::store`.
//!
//! Times are local, floating civil times written `YYYY-MM-DDTHH:MM` (no time zone).
//!
//! Methods (params are a JSON object):
//! - `access {}` returns `{status}`: `authorized` when the app declares `calendars`, else `denied`. An app cannot
//!   grant itself anything by asking; the manifest is the declaration.
//! - `events {start, end}` returns `{events: [{id, title, start, end, all_day, location}]}`: every event that overlaps
//!   `[start, end)`, sorted by start then id. Both bounds are required and `end` must be after `start`. An all-day
//!   event spans its whole civil day. Recurring events appear once, at their base date (recurrence expansion is not
//!   in this service yet).
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

/// The moment an event starts and ends, in the same units as [`parse_moment`]. An all-day event covers its civil day.
fn span(e: &CalendarEvent) -> (i64, i64) {
    let start = e.date.days() * 1440;
    if e.all_day {
        (start, start + 1440)
    } else {
        let at = start + e.minute as i64;
        (at, at + e.duration as i64)
    }
}

/// The JSON of one event as the `events` method returns it.
fn event_json(e: &CalendarEvent) -> Json {
    let (start, end) = span(e);
    Json::Obj(vec![
        ("id".into(), Json::Num(e.id as f64)),
        ("title".into(), Json::Str(e.title.clone())),
        ("start".into(), Json::Str(format_moment(start))),
        ("end".into(), Json::Str(format_moment(end))),
        ("all_day".into(), Json::Bool(e.all_day)),
        ("location".into(), Json::Str(e.location.clone())),
    ])
}

/// The `[start, end)` range named in the params.
fn range_of(params: &Json) -> Result<(i64, i64), String> {
    let text = |key: &str| params.get(key).and_then(Json::as_str).ok_or_else(|| format!("{key} is required (YYYY-MM-DDTHH:MM)"));
    let start = parse_moment(text("start")?).ok_or("start is not a YYYY-MM-DDTHH:MM time")?;
    let end = parse_moment(text("end")?).ok_or("end is not a YYYY-MM-DDTHH:MM time")?;
    if end <= start {
        return Err("end must be after start".into());
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
            let mut hits: Vec<(i64, &CalendarEvent)> = events
                .iter()
                .map(|e| (span(e), e))
                .filter(|((from, to), _)| *from < end && *to > start)
                .map(|((from, _), e)| (from, e))
                .collect();
            hits.sort_by_key(|(from, e)| (*from, e.id));
            let rows = hits.into_iter().map(|(_, e)| event_json(e)).collect();
            Ok(Json::Obj(vec![("events".into(), Json::Arr(rows))]))
        }
        _ => Err(format!("calendar has no method {method}")),
    }
}

pub struct Calendar;

impl SystemService for Calendar {
    fn name(&self) -> &'static str {
        "calendar"
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        let permitted = super::app_has_permission(app_id, PERMISSION);
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

    #[test]
    fn an_unknown_method_is_an_error() {
        assert!(answer("is.example.cal", true, "delete", &Json::Null, &[]).is_err());
    }

    #[test]
    fn the_store_is_the_calendar_apps_events_file() {
        let path = store_path(Path::new("/root"));
        assert_eq!(path, PathBuf::from("/root/appdata/is.olibuijr.calendar.desktop/events.tsv"));
    }
}
