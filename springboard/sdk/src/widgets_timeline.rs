//! Client for the shell's `widgets_timeline` service (`springboard/src/shell/services/widgets_timeline.rs`): an app
//! gives the shell a widget timeline (dated entries and a reload policy), reads the entry on screen, and starts and
//! ends live activities. The shell draws the current entry in Notification Centre and sends `reload` when the
//! timeline's reload date passes.
//!
//! ```ignore
//! use bifrost_app::widgets_timeline::{self as wt, Entry, ReloadPolicy};
//! wt::reload(ctx, "next-event", &[Entry { date: start, title: "Next".into(), text: "Standup".into() }], ReloadPolicy::AtEnd);
//! // in App::service_event:
//! //   if let Some(wt::Event::Reload { kind }) = wt::parse_event(service, event, body) { /* build and reload again */ }
//! ```
//! Requests return the request id the reply carries (see [`crate::service`]). Dates are unix seconds.
use crate::controls::live_activity::Dismissal;
pub use crate::controls::timeline::ReloadPolicy;
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;

/// The service name the shell registers.
pub const SERVICE: &str = "widgets_timeline";
/// The event the shell sends when a timeline's reload date has passed. Its body is `{"kind": "..."}`.
pub const EVENT_RELOAD: &str = "reload";

/// One dated entry of a widget timeline: shown from `date` until the next entry's date.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub date: f64,
    pub title: String,
    pub text: String,
}

/// What a widget event means to the app.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The shell asks for a new timeline for `kind`.
    Reload { kind: String },
}

/// Decodes a service event: `Some` for this service's events the SDK knows, `None` for any other.
pub fn parse_event(service: &str, event: &str, body: &Json) -> Option<Event> {
    if service != SERVICE || event != EVENT_RELOAD {
        return None;
    }
    let kind = body.str_field("kind")?;
    Some(Event::Reload { kind })
}

/// Replaces the timeline `kind` with `entries` under `policy`.
pub fn reload(ctx: &Ctx, kind: &str, entries: &[Entry], policy: ReloadPolicy) -> u32 {
    let (method, body) = reload_call(kind, entries, policy);
    service::call(ctx, SERVICE, method, &body)
}

/// Drops the timeline `kind`.
pub fn remove(ctx: &Ctx, kind: &str) -> u32 {
    service::call(ctx, SERVICE, "remove", &obj(&[("kind", Json::Str(kind.into()))]))
}

/// Asks for the entry on screen for `kind` now. The reply is `{date, title, text}` or null.
pub fn current(ctx: &Ctx, kind: &str) -> u32 {
    service::call(ctx, SERVICE, "current", &obj(&[("kind", Json::Str(kind.into()))]))
}

/// Starts a live activity showing `title` and `text`. `stale_at` marks when its content is out of date; `start_at`
/// makes it pending until then. The reply is the activity id.
pub fn start_activity(ctx: &Ctx, title: &str, text: &str, stale_at: Option<u64>, start_at: Option<u64>) -> u32 {
    let mut pairs = content_pairs(title, text, stale_at);
    if let Some(t) = start_at {
        pairs.push(("start_at".into(), Json::Num(t as f64)));
    }
    service::call(ctx, SERVICE, "start_activity", &Json::Obj(pairs))
}

/// Replaces the content of a live activity this app started.
pub fn update_activity(ctx: &Ctx, id: u32, title: &str, text: &str, stale_at: Option<u64>) -> u32 {
    let mut pairs = content_pairs(title, text, stale_at);
    pairs.insert(0, ("id".into(), Json::Num(id as f64)));
    service::call(ctx, SERVICE, "update_activity", &Json::Obj(pairs))
}

/// Ends a live activity this app started. `final_content` replaces its content first; `dismissal` says when it leaves
/// the screen.
pub fn end_activity(ctx: &Ctx, id: u32, final_content: Option<(&str, &str)>, dismissal: Dismissal) -> u32 {
    service::call(ctx, SERVICE, "end_activity", &end_body(id, final_content, dismissal))
}

/// Lists this app's live activities on screen: `[{id, state, title, text}]`.
pub fn activities(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "activities", &Json::Null)
}

/// The method name and JSON body of a `reload` call. Split out so the encoding can be checked without a context.
fn reload_call(kind: &str, entries: &[Entry], policy: ReloadPolicy) -> (&'static str, Json) {
    let (name, reload_at) = match policy {
        ReloadPolicy::AtEnd => ("at_end", None),
        ReloadPolicy::Never => ("never", None),
        ReloadPolicy::After(t) => ("after", Some(t)),
    };
    let mut pairs = vec![
        ("kind".into(), Json::Str(kind.into())),
        ("policy".into(), Json::Str(name.into())),
        ("entries".into(), Json::Arr(entries.iter().map(entry_json).collect())),
    ];
    if let Some(t) = reload_at {
        pairs.push(("reload_at".into(), Json::Num(t)));
    }
    ("reload", Json::Obj(pairs))
}

/// The JSON body of an `end_activity` call.
fn end_body(id: u32, final_content: Option<(&str, &str)>, dismissal: Dismissal) -> Json {
    let mut pairs = vec![("id".into(), Json::Num(id as f64))];
    if let Some((title, text)) = final_content {
        pairs.push(("title".into(), Json::Str(title.into())));
        pairs.push(("text".into(), Json::Str(text.into())));
    }
    let (name, after) = match dismissal {
        Dismissal::Default => ("default", None),
        Dismissal::Immediate => ("immediate", None),
        Dismissal::After(t) => ("after", Some(t)),
    };
    pairs.push(("dismissal".into(), Json::Str(name.into())));
    if let Some(t) = after {
        pairs.push(("after".into(), Json::Num(t as f64)));
    }
    Json::Obj(pairs)
}

fn entry_json(e: &Entry) -> Json {
    obj(&[("date", Json::Num(e.date)), ("title", Json::Str(e.title.clone())), ("text", Json::Str(e.text.clone()))])
}

fn content_pairs(title: &str, text: &str, stale_at: Option<u64>) -> Vec<(String, Json)> {
    let mut pairs = vec![("title".into(), Json::Str(title.into())), ("text".into(), Json::Str(text.into()))];
    if let Some(t) = stale_at {
        pairs.push(("stale_at".into(), Json::Num(t as f64)));
    }
    pairs
}

fn obj(pairs: &[(&str, Json)]) -> Json {
    Json::Obj(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reload_call_encodes_the_entries_and_the_policy() {
        let e = Entry { date: 10.0, title: "Next".into(), text: "Standup".into() };
        let (method, body) = reload_call("next", std::slice::from_ref(&e), ReloadPolicy::After(300.0));
        assert_eq!(method, "reload");
        assert_eq!(body.str_field("kind").as_deref(), Some("next"));
        assert_eq!(body.str_field("policy").as_deref(), Some("after"));
        assert_eq!(body.get("reload_at").and_then(Json::number), Some(300.0));
        let first = &body.get("entries").and_then(Json::array).unwrap()[0];
        assert_eq!(first.get("date").and_then(Json::number), Some(10.0));
        assert_eq!(first.str_field("text").as_deref(), Some("Standup"));
    }

    #[test]
    fn the_end_of_timeline_and_never_policies_send_no_reload_date() {
        let (_, body) = reload_call("k", &[], ReloadPolicy::AtEnd);
        assert_eq!(body.str_field("policy").as_deref(), Some("at_end"));
        assert!(body.get("reload_at").is_none());
        let (_, body) = reload_call("k", &[], ReloadPolicy::Never);
        assert_eq!(body.str_field("policy").as_deref(), Some("never"));
    }

    #[test]
    fn the_reload_event_is_decoded_for_this_service_only() {
        let body = obj(&[("kind", Json::Str("next".into()))]);
        assert_eq!(parse_event(SERVICE, EVENT_RELOAD, &body), Some(Event::Reload { kind: "next".into() }));
        assert_eq!(parse_event("dock", EVENT_RELOAD, &body), None, "another service's event is not ours");
        assert_eq!(parse_event(SERVICE, "other", &body), None);
        assert_eq!(parse_event(SERVICE, EVENT_RELOAD, &Json::Null), None, "a reload event names its kind");
    }

    #[test]
    fn an_end_call_carries_the_final_content_and_the_dismissal_time() {
        let body = end_body(7, Some(("Ride", "Done")), Dismissal::After(42));
        assert_eq!(body.get("id").and_then(Json::number), Some(7.0));
        assert_eq!(body.str_field("title").as_deref(), Some("Ride"));
        assert_eq!(body.str_field("dismissal").as_deref(), Some("after"));
        assert_eq!(body.get("after").and_then(Json::number), Some(42.0));
        let plain = end_body(8, None, Dismissal::Immediate);
        assert_eq!(plain.str_field("dismissal").as_deref(), Some("immediate"));
        assert!(plain.get("title").is_none() && plain.get("after").is_none());
    }
}
