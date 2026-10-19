//! Calendar events for apps (the EventKit reads, and saving, editing and deleting events): ask for the access status, then fetch the events that overlap a
//! range of local civil times.
//!
//! ```ignore
//! let id = calendar::events(ctx, "2026-10-13T00:00", "2026-10-14T00:00");
//! // in App::service_reply, once the reply for `id` arrives:
//! //   let today = calendar::decode_events(&reply);   // Vec<Event>, sorted by start
//! ```
//! An app uses the calendar only when its manifest declares the `calendars` permission and the person has allowed it
//! (the access sheet, or Settings > Privacy & Security > App Permissions); otherwise every call but `access` and
//! `request` is an error.
//! Times are floating local times written `YYYY-MM-DDTHH:MM`. Recurring events appear once, at their base date. The
//! service is `springboard/src/shell/services/calendar.rs`.
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;

/// The service name the shell registers.
pub const SERVICE: &str = "calendar";

/// The access status an app holds. The manifest's `calendars` permission only lets the app ask; the person answers
/// on the access sheet or in Settings > Privacy & Security > App Permissions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Full access: the app reads, adds, changes and deletes events.
    Authorized,
    /// The person refused, or the manifest does not declare `calendars`.
    Denied,
    /// The person has not answered yet; a `request` (or a first `save_event`) shows the sheet.
    NotDetermined,
    /// The app may add events but not read them.
    WriteOnly,
    /// The system blocks calendar access for the app.
    Restricted,
}

/// One event in a range. `start` and `end` are local civil times (`YYYY-MM-DDTHH:MM`); an all-day event starts at
/// midnight and ends at the next midnight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub id: u64,
    pub title: String,
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub location: String,
}

/// Asks for the access status. The reply decodes with [`decode_status`].
pub fn access(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "access", &Json::Null)
}

/// Asks the person for calendar access (the sheet shows when nothing was answered yet). The reply is the status before
/// the answer and decodes with [`decode_status`]; ask again with [`access`] later.
pub fn request(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "request", &Json::Null)
}

/// Fetches the events overlapping `[start, end)`, sorted by start. Both bounds are `YYYY-MM-DDTHH:MM` and `end` must be
/// after `start`. The reply decodes with [`decode_events`].
pub fn events(ctx: &Ctx, start: &str, end: &str) -> u32 {
    let params = Json::Obj(vec![("start".into(), Json::Str(start.into())), ("end".into(), Json::Str(end.into()))]);
    service::call(ctx, SERVICE, "events", &params)
}

/// Saves a new event and returns the request id. `start` and `end` are `YYYY-MM-DDTHH:MM`; a timed event lasts 15
/// minutes to 24 hours, and an all-day event takes the civil day of `start`. The reply decodes with [`decode_saved_id`].
/// Needs the `calendars` permission and the person's consent: a first save shows the sheet and fails until it is answered.
pub fn save_event(ctx: &Ctx, title: &str, start: &str, end: &str, all_day: bool, location: &str) -> u32 {
    service::call(ctx, SERVICE, "save_event", &save_params(title, start, end, all_day, location))
}

/// The parameters of a `save_event` call.
pub fn save_params(title: &str, start: &str, end: &str, all_day: bool, location: &str) -> Json {
    Json::Obj(vec![
        ("title".into(), Json::Str(title.into())),
        ("start".into(), Json::Str(start.into())),
        ("end".into(), Json::Str(end.into())),
        ("all_day".into(), Json::Bool(all_day)),
        ("location".into(), Json::Str(location.into())),
    ])
}

/// Replaces the title, time, all-day flag and location of event `id` (an id from an `events` reply) and returns the request
/// id. The fields are as in [`save_event`]; a repeating event keeps its rule, and its first occurrence moves to `start`.
/// The reply decodes with [`decode_ok`]. Needs the `calendars` permission.
pub fn update_event(ctx: &Ctx, id: u64, title: &str, start: &str, end: &str, all_day: bool, location: &str) -> u32 {
    let params = Json::Obj(vec![
        ("id".into(), Json::Num(id as f64)),
        ("title".into(), Json::Str(title.into())),
        ("start".into(), Json::Str(start.into())),
        ("end".into(), Json::Str(end.into())),
        ("all_day".into(), Json::Bool(all_day)),
        ("location".into(), Json::Str(location.into())),
    ]);
    service::call(ctx, SERVICE, "update_event", &params)
}

/// Deletes event `id` and returns the request id. The reply decodes with [`decode_ok`]. Needs the `calendars` permission.
pub fn delete_event(ctx: &Ctx, id: u64) -> u32 {
    service::call(ctx, SERVICE, "delete_event", &Json::Obj(vec![("id".into(), Json::Num(id as f64))]))
}

/// True for the `{ok: true}` reply of an `update_event` or `delete_event` call.
pub fn decode_ok(reply: &Json) -> bool {
    matches!(reply.get("ok"), Some(Json::Bool(true)))
}

/// The id of the event a `save_event` reply created, or `None` for a reply of another shape.
pub fn decode_saved_id(reply: &Json) -> Option<u64> {
    reply.get("id").and_then(Json::as_u64)
}

/// The status in an `access` reply, or `None` for a reply of another shape.
pub fn decode_status(reply: &Json) -> Option<Status> {
    match reply.get("status").and_then(Json::as_str)? {
        "authorized" => Some(Status::Authorized),
        "denied" => Some(Status::Denied),
        "not_determined" => Some(Status::NotDetermined),
        "write_only" => Some(Status::WriteOnly),
        "restricted" => Some(Status::Restricted),
        _ => None,
    }
}

/// The events in an `events` reply. A row of another shape is skipped; a reply without an event list is empty.
pub fn decode_events(reply: &Json) -> Vec<Event> {
    let rows = reply.get("events").and_then(Json::as_array).unwrap_or(&[]);
    rows.iter().filter_map(decode_event).collect()
}

fn decode_event(v: &Json) -> Option<Event> {
    Some(Event {
        id: v.get("id")?.as_u64()?,
        title: v.get("title")?.as_str()?.to_string(),
        start: v.get("start")?.as_str()?.to_string(),
        end: v.get("end")?.as_str()?.to_string(),
        all_day: matches!(v.get("all_day"), Some(Json::Bool(true))),
        location: v.get("location").and_then(Json::as_str).unwrap_or("").to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: u64, title: &str) -> Json {
        Json::Obj(vec![
            ("id".into(), Json::Num(id as f64)),
            ("title".into(), Json::Str(title.into())),
            ("start".into(), Json::Str("2026-10-13T09:00".into())),
            ("end".into(), Json::Str("2026-10-13T10:00".into())),
            ("all_day".into(), Json::Bool(false)),
            ("location".into(), Json::Str("Hall".into())),
        ])
    }

    fn reply(rows: Vec<Json>) -> Json {
        Json::Obj(vec![("events".into(), Json::Arr(rows))])
    }

    #[test]
    fn a_status_reply_decodes_to_its_status() {
        let granted = Json::Obj(vec![("status".into(), Json::Str("authorized".into()))]);
        assert_eq!(decode_status(&granted), Some(Status::Authorized));
        let refused = Json::Obj(vec![("status".into(), Json::Str("denied".into()))]);
        assert_eq!(decode_status(&refused), Some(Status::Denied));
        for (name, status) in [("not_determined", Status::NotDetermined), ("write_only", Status::WriteOnly), ("restricted", Status::Restricted)] {
            assert_eq!(decode_status(&Json::Obj(vec![("status".into(), Json::Str(name.into()))])), Some(status));
        }
        assert_eq!(decode_status(&Json::Obj(vec![("status".into(), Json::Str("all".into()))])), None);
        assert_eq!(decode_status(&Json::Null), None);
    }

    #[test]
    fn an_events_reply_decodes_to_its_events_in_order() {
        let events = decode_events(&reply(vec![row(4, "Coffee"), row(9, "Standup")]));
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].id, 4);
        assert_eq!(events[0].title, "Coffee");
        assert_eq!(events[0].start, "2026-10-13T09:00");
        assert_eq!(events[0].end, "2026-10-13T10:00");
        assert!(!events[0].all_day);
        assert_eq!(events[1].location, "Hall");
    }

    #[test]
    fn a_malformed_row_is_skipped_and_the_rest_decode() {
        let broken = Json::Obj(vec![("title".into(), Json::Str("No id".into()))]);
        let events = decode_events(&reply(vec![broken, row(2, "Kept")]));
        assert_eq!(events.iter().map(|e| e.id).collect::<Vec<_>>(), vec![2]);
    }

    #[test]
    fn a_reply_of_another_shape_or_an_empty_range_has_no_events() {
        assert!(decode_events(&Json::Null).is_empty());
        assert!(decode_events(&reply(vec![])).is_empty());
    }

    #[test]
    fn a_save_call_names_its_fields_and_the_reply_decodes_to_the_new_id() {
        let params = save_params("Lunch", "2026-10-13T12:30", "2026-10-13T13:15", false, "Desk");
        assert_eq!(params.get("title").and_then(Json::as_str), Some("Lunch"));
        assert_eq!(params.get("start").and_then(Json::as_str), Some("2026-10-13T12:30"));
        assert_eq!(params.get("end").and_then(Json::as_str), Some("2026-10-13T13:15"));
        assert!(matches!(params.get("all_day"), Some(Json::Bool(false))));
        assert_eq!(params.get("location").and_then(Json::as_str), Some("Desk"));
        let reply = Json::Obj(vec![("id".into(), Json::Num(12.0))]);
        assert_eq!(decode_saved_id(&reply), Some(12));
        assert_eq!(decode_saved_id(&Json::Null), None);
    }

    #[test]
    fn an_update_or_delete_reply_decodes_only_when_ok_is_true() {
        assert!(decode_ok(&Json::Obj(vec![("ok".into(), Json::Bool(true))])));
        assert!(!decode_ok(&Json::Obj(vec![("ok".into(), Json::Bool(false))])));
        assert!(!decode_ok(&Json::Null));
    }

    #[test]
    fn an_all_day_flag_and_a_missing_location_decode() {
        let holiday = Json::Obj(vec![
            ("id".into(), Json::Num(3.0)),
            ("title".into(), Json::Str("Holiday".into())),
            ("start".into(), Json::Str("2026-10-13T00:00".into())),
            ("end".into(), Json::Str("2026-10-14T00:00".into())),
            ("all_day".into(), Json::Bool(true)),
        ]);
        let events = decode_events(&reply(vec![holiday]));
        assert!(events[0].all_day);
        assert_eq!(events[0].location, "");
    }
}
