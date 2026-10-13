//! Input events for apps, over the shell's `events` service (`springboard/src/shell/services/event_services.rs`): event
//! creation (mouse, keyboard, scroll wheel), fields, flags, location and timestamps are the model
//! `bifrost_ui::event_tap`, re-exported here; this client posts events, installs session-level event taps (listen-only
//! and filtering), switches them on and off, and reads the event source state tables.
//!
//! Every call needs the `accessibility` permission in the manifest, or the shell fails it. A filtering tap decides by
//! its rules, because the shell cannot wait for an app in the middle of an input event; it still tells the app what it
//! saw as a `tap` event, read with [`tap_event`].
//!
//! ```ignore
//! let ev = input_events::Event::key_down(KeyCombo::parse("cmd+shift+4").unwrap(), 0);
//! input_events::post(ctx, &ev);
//! let id = input_events::tap_create(ctx, &TapSpec::listen(EventMask::of(&[EventType::KeyDown])));
//! // App::service_reply: input_events::decode_tap(result) -> tap id
//! // App::service_event: input_events::tap_event(service, event, body) -> what the tap saw
//! ```
use crate::service;
use crate::Ctx;
use bifrost_core::json::Json;
pub use bifrost_ui::event_tap::*;
use bifrost_ui::wire_json::*;

/// The shell service this client calls.
pub const SERVICE: &str = "events";
/// The event a tap's owner receives for each event the tap saw.
pub const TAP_EVENT: &str = "tap";

fn call(ctx: &Ctx, method: &str, params: Json) -> u32 {
    service::call(ctx, SERVICE, method, &params)
}

/// What a new tap watches and does.
#[derive(Clone, Debug, PartialEq)]
pub struct TapSpec {
    pub mask: EventMask,
    pub options: TapOptions,
    pub placement: TapPlacement,
    pub rules: Vec<FilterRule>,
}

impl TapSpec {
    /// A tap that only watches.
    pub fn listen(mask: EventMask) -> Self {
        TapSpec { mask, options: TapOptions::ListenOnly, placement: TapPlacement::TailAppend, rules: Vec::new() }
    }

    /// A tap that applies `rules` to the events in `mask`.
    pub fn filter(mask: EventMask, rules: Vec<FilterRule>) -> Self {
        TapSpec { mask, options: TapOptions::DefaultTap, placement: TapPlacement::TailAppend, rules }
    }

    /// The same tap, placed before every other tap.
    pub fn at_head(mut self) -> Self {
        self.placement = TapPlacement::HeadInsert;
        self
    }

    fn to_json(&self) -> Json {
        obj(vec![
            ("mask", self.mask.to_json()),
            ("options", text(self.options.name())),
            ("placement", text(if self.placement == TapPlacement::HeadInsert { "head" } else { "tail" })),
            ("rules", Json::Arr(self.rules.iter().map(FilterRule::to_json).collect())),
        ])
    }
}

/// Posts an event into the session stream. Answer: [`decode_post`].
pub fn post(ctx: &Ctx, event: &Event) -> u32 {
    call(ctx, "post", obj(vec![("event", event.to_json())]))
}

/// Installs a tap. Answer: [`decode_tap`].
pub fn tap_create(ctx: &Ctx, spec: &TapSpec) -> u32 {
    call(ctx, "tap_create", spec.to_json())
}

/// Switches a tap on or off. Answer: [`decode_ok`].
pub fn tap_enable(ctx: &Ctx, tap: u32, enabled: bool) -> u32 {
    call(ctx, "tap_enable", obj(vec![("tap", num(tap)), ("enabled", Json::Bool(enabled))]))
}

/// Removes a tap. Answer: [`decode_ok`].
pub fn tap_remove(ctx: &Ctx, tap: u32) -> u32 {
    call(ctx, "tap_remove", obj(vec![("tap", num(tap))]))
}

fn source_name(s: SourceState) -> &'static str {
    match s {
        SourceState::Private => "private",
        SourceState::Session => "session",
        SourceState::Hid => "hid",
    }
}

fn state(ctx: &Ctx, source: SourceState, query: &str, extra: Vec<(&str, Json)>) -> u32 {
    let mut fields = vec![("source", text(source_name(source))), ("query", text(query))];
    fields.extend(extra);
    call(ctx, "state", obj(fields))
}

/// The modifier flags held in an event source's state table. Answer: [`decode_flags`].
pub fn flags_state(ctx: &Ctx, source: SourceState) -> u32 {
    state(ctx, source, "flags", vec![])
}

/// Whether a key (Linux key code) is down in the table. Answer: [`decode_down`].
pub fn key_state(ctx: &Ctx, source: SourceState, key_code: u16) -> u32 {
    state(ctx, source, "key", vec![("key_code", num(key_code))])
}

/// Whether a mouse button is down in the table. Answer: [`decode_down`].
pub fn button_state(ctx: &Ctx, source: SourceState, button: u8) -> u32 {
    state(ctx, source, "button", vec![("button", num(button))])
}

/// Seconds since the last event of a kind. Answer: [`decode_seconds`].
pub fn seconds_since_last(ctx: &Ctx, source: SourceState, kind: EventType) -> u32 {
    state(ctx, source, "since", vec![("kind", text(kind.name()))])
}

/// How many events of a kind the table has seen. Answer: [`decode_count`].
pub fn counter(ctx: &Ctx, source: SourceState, kind: EventType) -> u32 {
    state(ctx, source, "counter", vec![("kind", text(kind.name()))])
}

/// The last pointer location in points. Answer: [`decode_location`].
pub fn location(ctx: &Ctx) -> u32 {
    call(ctx, "location", Json::Null)
}

fn body(result: &Result<Json, String>) -> Result<&Json, String> {
    result.as_ref().map_err(|e| e.clone())
}

/// The answer to [`post`]: whether the event reached the shell (a tap may have dropped it).
pub fn decode_post(result: &Result<Json, String>) -> Result<bool, String> {
    Ok(bool_of(body(result)?, "delivered").unwrap_or(false))
}

/// The answer to [`tap_create`]: the tap's id.
pub fn decode_tap(result: &Result<Json, String>) -> Result<u32, String> {
    u32_of(body(result)?, "tap").ok_or_else(|| "events: the answer has no tap".to_string())
}

/// The answer to [`tap_enable`] and [`tap_remove`]: whether the app's tap existed.
pub fn decode_ok(result: &Result<Json, String>) -> Result<bool, String> {
    Ok(bool_of(body(result)?, "ok").unwrap_or(false))
}

pub fn decode_flags(result: &Result<Json, String>) -> Result<EventFlags, String> {
    u32_of(body(result)?, "flags").map(EventFlags).ok_or_else(|| "events: the answer has no flags".to_string())
}

pub fn decode_down(result: &Result<Json, String>) -> Result<bool, String> {
    bool_of(body(result)?, "down").ok_or_else(|| "events: the answer has no state".to_string())
}

/// `None` when no event of the kind has been seen.
pub fn decode_seconds(result: &Result<Json, String>) -> Result<Option<f64>, String> {
    Ok(f64_of(body(result)?, "seconds"))
}

pub fn decode_count(result: &Result<Json, String>) -> Result<u64, String> {
    f64_of(body(result)?, "count").map(|c| c as u64).ok_or_else(|| "events: the answer has no count".to_string())
}

pub fn decode_location(result: &Result<Json, String>) -> Result<(f32, f32), String> {
    let b = body(result)?;
    Ok((f32_of(b, "x").unwrap_or(0.0), f32_of(b, "y").unwrap_or(0.0)))
}

/// What a tap did with an event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TapNotice {
    pub tap: u32,
    /// `listen`, `pass`, `drop` or `replace`.
    pub action: &'static str,
    pub event: Event,
}

/// The tap event behind a service event; `None` for any other event.
pub fn tap_event(service: &str, event: &str, body: &Json) -> Option<TapNotice> {
    if service != SERVICE || event != TAP_EVENT {
        return None;
    }
    let action = match str_of(body, "action")?.as_str() {
        "listen" => "listen",
        "pass" => "pass",
        "drop" => "drop",
        "replace" => "replace",
        _ => return None,
    };
    Some(TapNotice { tap: u32_of(body, "tap")?, action, event: Event::from_json(body.get("event")?)? })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tap_spec_encodes_what_the_shell_reads() {
        let rule = FilterRule { kind: Some(EventType::KeyDown), key_code: Some(30), flags: None, button: None, action: RuleAction::Drop };
        let spec = TapSpec::filter(EventMask::of(&[EventType::KeyDown]), vec![rule]).at_head();
        let j = spec.to_json();
        assert_eq!(EventMask::from_json(j.get("mask").unwrap()), Some(spec.mask));
        assert_eq!(str_of(&j, "options").as_deref(), Some("filter"));
        assert_eq!(str_of(&j, "placement").as_deref(), Some("head"));
        assert_eq!(FilterRule::from_json(&j.get("rules").unwrap().as_array().unwrap()[0]), Some(rule));
        let listen = TapSpec::listen(EventMask::all()).to_json();
        assert_eq!(str_of(&listen, "options").as_deref(), Some("listen"));
        assert_eq!(str_of(&listen, "placement").as_deref(), Some("tail"));
    }

    #[test]
    fn answers_decode() {
        assert_eq!(decode_tap(&Ok(obj(vec![("tap", num(3u32))]))), Ok(3));
        assert!(decode_tap(&Ok(Json::Null)).is_err());
        assert_eq!(decode_post(&Ok(obj(vec![("delivered", Json::Bool(true))]))), Ok(true));
        assert_eq!(decode_ok(&Ok(obj(vec![("ok", Json::Bool(false))]))), Ok(false));
        assert_eq!(decode_flags(&Ok(obj(vec![("flags", num(EventFlags::SHIFT))]))), Ok(EventFlags(EventFlags::SHIFT)));
        assert_eq!(decode_down(&Ok(obj(vec![("down", Json::Bool(true))]))), Ok(true));
        assert_eq!(decode_seconds(&Ok(obj(vec![("seconds", Json::Null)]))), Ok(None));
        assert_eq!(decode_seconds(&Ok(obj(vec![("seconds", num(1.5))]))), Ok(Some(1.5)));
        assert_eq!(decode_count(&Ok(obj(vec![("count", num(4u32))]))), Ok(4));
        assert_eq!(decode_location(&Ok(obj(vec![("x", num(1.0)), ("y", num(2.0))]))), Ok((1.0, 2.0)));
        assert!(decode_post(&Err("needs accessibility".into())).is_err());
    }

    #[test]
    fn a_tap_event_decodes_and_others_do_not() {
        let e = Event::key(30, true, 5);
        let b = obj(vec![("tap", num(2u32)), ("action", text("drop")), ("event", e.to_json())]);
        let n = tap_event(SERVICE, TAP_EVENT, &b).unwrap();
        assert_eq!((n.tap, n.action, n.event), (2, "drop", e));
        assert!(tap_event(SERVICE, "other", &b).is_none());
        assert!(tap_event("screens", TAP_EVENT, &b).is_none());
        let bad = obj(vec![("tap", num(2u32)), ("action", text("explode")), ("event", e.to_json())]);
        assert!(tap_event(SERVICE, TAP_EVENT, &bad).is_none());
    }
}
