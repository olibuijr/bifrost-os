//! The `calendar` service: the person's events, for the apps whose manifest declares the `calendars` permission. It is
//! the read side of the event store (events in a date range); the store is the shared calendar file the Calendar app
//! writes, `shell::calendar::store`.
//!
//! Times are local, floating civil times written `YYYY-MM-DDTHH:MM` (no time zone).
//!
//! Methods (params are a JSON object):
//! - `access {}` returns `{status}`: `not_determined`, `restricted`, `denied`, `write_only` or `authorized` (full
//!   access). The manifest's `calendars` permission only lets the app ask; the person's answer (the access sheet or
//!   Settings > Privacy & Security > App Permissions) decides, and an app that does not declare it is always `denied`.
//! - `request {}` returns the same `{status}` as it stands and, when the person has not answered, queues the access
//!   sheet (read and write). A first `save_event` queues the add-only sheet instead and fails until it is answered.
//! - `events {start, end}` returns `{events: [{id, title, start, end, all_day, location}]}`: every event that overlaps
//!   `[start, end)`, sorted by start then id. Both bounds are required and `end` must be after `start`. An all-day
//!   event spans its whole civil day. A repeating event appears once for each occurrence that overlaps the range, all
//!   with its id; an occurrence that starts the day before the range and runs into it is included.
//!
//! - `save_event {title, start, end, all_day, location}` appends one event to the shared store and returns `{id}`. A
//!   timed event lasts 15 minutes to 24 hours; an all-day event takes the civil day of `start`. The store is read and
//!   rewritten whole, so a save made while the Calendar app is writing can be lost; the Calendar app is the owner.
//!
//! - `update_event {id, title, start, end, all_day, location}` replaces the fields of event `id` and returns `{ok}`. The
//!   repeat rule, alert, notes and account UID are kept; for a repeating event `start` is its first occurrence. An
//!   unknown id is an error.
//! - `delete_event {id}` removes event `id` from the shared store and returns `{ok}`. An unknown id is an error.
//!
//! Without the `calendars` permission, or before the person allows it, every method but `access` and `request` is an
//! error. `save_event` needs add access (add-only or full); `events`, `update_event` and `delete_event` need full
//! access, so an add-only app never reads the person's events.
use super::{Shell, SystemService};
use crate::shell::calendar::store::{self, CalendarEvent};
use crate::shell::calendar::Date;
use bifrost_core::json::Json;
use bifrost_ui::calendar_access::{Access, EntityType, Status};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The manifest permission that lets an app ask to use the calendar.
pub const PERMISSION: &str = "calendars";

thread_local! {
    static ACCESS: RefCell<BTreeMap<String, Access>> = RefCell::new(BTreeMap::new());
}

/// The grant an app holds (a new app has none answered yet).
pub fn access_of(app_id: &str) -> Access {
    ACCESS.with(|a| a.borrow().get(app_id).copied().unwrap_or_default())
}

/// The answer to the access sheet the app is waiting on: `write_only` is the add-only sheet. Only the first answer
/// counts, as the model says.
pub fn answer_prompt(app_id: &str, write_only: bool, granted: bool) -> Status {
    super::privacy_permissions::mark_grants_dirty();
    ACCESS.with(|a| {
        let mut a = a.borrow_mut();
        let access = a.entry(app_id.to_string()).or_default();
        access.request(EntityType::Event, write_only, granted).unwrap_or_else(|_| access.status(EntityType::Event))
    })
}

/// A change the person makes in Settings: on is full access, off is denied. Returns the status that stands.
pub fn change_by_user(app_id: &str, allowed: bool) -> Status {
    super::privacy_permissions::mark_grants_dirty();
    ACCESS.with(|a| a.borrow_mut().entry(app_id.to_string()).or_default().change_events_in_settings(allowed))
}

/// Whether the person lets the app use the calendar (add-only or full), for the Settings switch.
pub fn allowed(app_id: &str) -> bool {
    access_of(app_id).can_write(EntityType::Event)
}

/// Every app with an answer, for the privacy store to save.
pub fn grants_snapshot() -> Vec<(String, Status)> {
    ACCESS.with(|a| a.borrow().iter().map(|(app, access)| (app.clone(), access.status(EntityType::Event))).collect())
}

/// The name an `access` reply gives a status.
pub fn status_name(status: Status) -> &'static str {
    match status {
        Status::NotDetermined => "not_determined",
        Status::Restricted => "restricted",
        Status::Denied => "denied",
        Status::WriteOnly => "write_only",
        Status::FullAccess => "authorized",
    }
}

/// Why the app may not call `method`, or `None` when it may. `declared` says whether its manifest has `calendars` and
/// `status` is the person's answer for it. Pure. A method this service does not know is not refused here.
pub fn refusal(app_id: &str, declared: bool, status: Status, method: &str) -> Option<String> {
    let needs_read = match method {
        "events" | "update_event" | "delete_event" => true,
        "save_event" => false,
        _ => return None,
    };
    if !declared {
        return Some(format!("{app_id} has no {PERMISSION} permission"));
    }
    match status {
        Status::FullAccess => None,
        Status::WriteOnly if !needs_read => None,
        Status::WriteOnly => Some("calendar access is add-only; reading and changing events needs full access".into()),
        Status::NotDetermined => Some("calendar access has not been allowed yet; the person has been asked".into()),
        Status::Denied | Status::Restricted => Some("calendar access is not allowed".into()),
    }
}

/// Queues the access sheet for an app (`write_only` asks to add events only).
fn ask(app_id: &str, write_only: bool) {
    let name = super::super::apphost::installed().iter().find(|a| a.manifest.id == app_id).map_or_else(|| app_id.to_string(), |a| a.manifest.name.clone());
    super::privacy_permissions::queue_prompt(app_id, &name, super::privacy_permissions::Kind::Calendars { write_only }, "");
}

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
    if events.len() >= store::MAX_EVENTS {
        return Err("the calendar is full".into());
    }
    built_event(params, store::next_id(events))
}

/// The event the `{title, start, end, all_day, location}` params describe, with id `id`. Repeat, alert, notes and the
/// account UID are left empty. Validates the fields; pure.
fn built_event(params: &Json, id: u64) -> Result<CalendarEvent, String> {
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
    let event = CalendarEvent {
        id,
        uid: String::new(),
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

/// The `id` an `update_event` or `delete_event` call names.
fn target_id(params: &Json) -> Result<u64, String> {
    params.get("id").and_then(Json::as_u64).ok_or_else(|| "id is required".to_string())
}

/// The list with the event `params.id` names replaced by the fields the `{title, start, end, all_day, location}` params
/// give. The repeat rule, alert, notes and account UID of that event are kept; for a repeating event the new `start`
/// is its first occurrence. Refused when no event has that id. Pure.
pub fn edit_event(params: &Json, events: &[CalendarEvent]) -> Result<Vec<CalendarEvent>, String> {
    let id = target_id(params)?;
    let at = events.iter().position(|e| e.id == id).ok_or("no event has that id")?;
    let fresh = built_event(params, id)?;
    let old = &events[at];
    let kept = CalendarEvent { repeat: old.repeat, alert: old.alert, notes: old.notes.clone(), uid: old.uid.clone(), ..fresh };
    let mut edited = events.to_vec();
    edited[at] = kept;
    Ok(edited)
}

/// The list without the event `params.id` names. Refused when no event has that id. Pure.
pub fn remove_event(params: &Json, events: &[CalendarEvent]) -> Result<Vec<CalendarEvent>, String> {
    let id = target_id(params)?;
    if !events.iter().any(|e| e.id == id) {
        return Err("no event has that id".into());
    }
    Ok(events.iter().filter(|e| e.id != id).cloned().collect())
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

/// Applies an `edit_event` or `remove_event` change to the shared store and writes it back.
fn change_store(shell: &Shell, params: &Json, change: fn(&Json, &[CalendarEvent]) -> Result<Vec<CalendarEvent>, String>) -> Result<Json, String> {
    let path = store_path(&shell.data_dir);
    let events = store::load(&path).map_err(|e| e.to_string())?;
    let next = change(params, &events)?;
    store::save(&path, &next).map_err(|e| e.to_string())?;
    Ok(Json::Obj(vec![("ok".into(), Json::Bool(true))]))
}

pub struct Calendar;

impl SystemService for Calendar {
    fn name(&self) -> &'static str {
        "calendar"
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        let declared = super::app_has_permission(app_id, PERMISSION);
        let status = if declared { access_of(app_id).status(EntityType::Event) } else { Status::Denied };
        let status_reply = Json::Obj(vec![("status".into(), Json::Str(status_name(status).into()))]);
        match method {
            "access" => return Ok(status_reply),
            "request" => {
                if declared && status == Status::NotDetermined {
                    ask(app_id, false);
                }
                return Ok(status_reply);
            }
            _ => {}
        }
        if let Some(why) = refusal(app_id, declared, status, method) {
            // a first save by an undecided app asks for the least: add-only access
            if declared && status == Status::NotDetermined && method == "save_event" {
                ask(app_id, true);
            }
            return Err(why);
        }
        if method == "save_event" {
            return save_event(shell, params);
        }
        if method == "update_event" || method == "delete_event" {
            let change = if method == "update_event" { edit_event } else { remove_event };
            return change_store(shell, params, change);
        }
        let permitted = declared && status == Status::FullAccess;
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
            alert: 0, uid: String::new(),
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
    fn an_undeclared_or_undecided_or_refused_app_may_not_use_the_calendar() {
        for method in ["events", "save_event", "update_event", "delete_event"] {
            assert!(refusal("a", false, Status::FullAccess, method).unwrap().contains("no calendars permission"), "{method}");
            assert!(refusal("a", true, Status::NotDetermined, method).unwrap().contains("not been allowed"), "{method}");
            assert!(refusal("a", true, Status::Denied, method).is_some(), "{method}");
            assert!(refusal("a", true, Status::Restricted, method).is_some(), "{method}");
            assert_eq!(refusal("a", true, Status::FullAccess, method), None, "{method}");
        }
        assert_eq!(refusal("a", false, Status::Denied, "no_such_method"), None, "an unknown method fails later, as before");
    }

    #[test]
    fn an_add_only_app_adds_events_but_never_reads_or_changes_them() {
        assert_eq!(refusal("a", true, Status::WriteOnly, "save_event"), None);
        for method in ["events", "update_event", "delete_event"] {
            assert!(refusal("a", true, Status::WriteOnly, method).unwrap().contains("add-only"), "{method}");
        }
    }

    #[test]
    fn the_persons_answer_is_kept_and_settings_can_change_it() {
        let app = "is.example.cal.answers";
        assert_eq!(access_of(app).status(EntityType::Event), Status::NotDetermined);
        assert!(!allowed(app));
        assert_eq!(answer_prompt(app, true, true), Status::WriteOnly);
        assert!(allowed(app));
        assert!(!access_of(app).can_read(EntityType::Event), "add-only does not read");
        // a later sheet answer does not replace the first one
        assert_eq!(answer_prompt(app, false, false), Status::WriteOnly);
        assert_eq!(change_by_user(app, false), Status::Denied);
        assert!(!allowed(app));
        assert_eq!(change_by_user(app, true), Status::FullAccess);
        assert!(access_of(app).can_read(EntityType::Event));
        assert!(grants_snapshot().contains(&(app.to_string(), Status::FullAccess)));
    }

    #[test]
    fn a_refusal_on_the_sheet_denies_and_status_names_are_stable() {
        let app = "is.example.cal.refused";
        assert_eq!(answer_prompt(app, false, false), Status::Denied);
        assert_eq!(status_name(Status::Denied), "denied");
        assert_eq!(status_name(Status::NotDetermined), "not_determined");
        assert_eq!(status_name(Status::WriteOnly), "write_only");
        assert_eq!(status_name(Status::FullAccess), "authorized");
        assert_eq!(status_name(Status::Restricted), "restricted");
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

    fn id_params(id: u64) -> Json {
        Json::Obj(vec![("id".into(), Json::Num(id as f64))])
    }

    fn edit_params(id: u64, title: &str, start: &str, end: &str) -> Json {
        Json::Obj(vec![
            ("id".into(), Json::Num(id as f64)),
            ("title".into(), Json::Str(title.into())),
            ("start".into(), Json::Str(start.into())),
            ("end".into(), Json::Str(end.into())),
            ("all_day".into(), Json::Bool(false)),
            ("location".into(), Json::Str("Desk".into())),
        ])
    }

    #[test]
    fn an_edit_replaces_the_fields_and_keeps_the_repeat_rule_and_id() {
        let mut book = day_book();
        book[0].repeat = 2;
        book[0].notes = "Bring slides".into();
        let edited = edit_event(&edit_params(1, "Retro", "2026-10-13T15:00", "2026-10-13T16:30"), &book).unwrap();
        assert_eq!(edited.len(), book.len());
        let e = edited.iter().find(|e| e.id == 1).unwrap();
        assert_eq!(e.title, "Retro");
        assert_eq!(e.minute, 15 * 60);
        assert_eq!(e.duration, 90);
        assert_eq!(e.repeat, 2);
        assert_eq!(e.notes, "Bring slides");
        assert_eq!(e.location, "Desk");
        // the other events are untouched
        for other in book.iter().filter(|e| e.id != 1) {
            assert!(edited.contains(other));
        }
    }

    #[test]
    fn an_edit_of_an_unknown_id_or_with_bad_fields_is_refused() {
        let book = day_book();
        assert!(edit_event(&edit_params(99, "Ghost", "2026-10-13T15:00", "2026-10-13T16:00"), &book).is_err());
        assert!(edit_event(&edit_params(1, "Blink", "2026-10-13T15:00", "2026-10-13T15:05"), &book).is_err());
        assert!(edit_event(&Json::Null, &book).is_err());
    }

    #[test]
    fn an_edit_is_allowed_on_a_full_calendar() {
        let full: Vec<CalendarEvent> = (1..=store::MAX_EVENTS as u64).map(|id| event(id, (2026, 10, 13), 0, 60, false, "x")).collect();
        assert!(edit_event(&edit_params(3, "Moved", "2026-10-13T09:00", "2026-10-13T10:00"), &full).is_ok());
    }

    #[test]
    fn a_delete_removes_only_the_named_event() {
        let book = day_book();
        let left = remove_event(&id_params(4), &book).unwrap();
        assert_eq!(left.iter().map(|e| e.id).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert!(remove_event(&id_params(99), &book).is_err());
        assert!(remove_event(&Json::Null, &book).is_err());
    }

    #[test]
    fn a_deleted_event_is_gone_from_the_events_call() {
        let left = remove_event(&id_params(1), &day_book()).unwrap();
        let reply = answer("is.example.cal", true, "events", &range("2026-10-13T00:00", "2026-10-14T00:00"), &left).unwrap();
        // event 2 overlaps the 13th (it runs past midnight) and sorts first; event 1 is the one that is gone
        assert_eq!(ids(&reply), vec![2, 4]);
    }
}
