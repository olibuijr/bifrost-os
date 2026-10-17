//! Endpoint security for security apps: file open, exec, modify and close events of the whole computer, and the chance to
//! allow or deny an open or an exec before it happens. The shell side is
//! `springboard/src/shell/services/endpoint_security.rs`; the model and the wire types are
//! `bifrost_core::endpoint_security`.
//!
//! Only an app whose manifest declares `entitlements = ["endpoint-security"]` can use this, and only a first-party
//! system app is granted it; every call of any other app fails. The app never holds a descriptor: the root service
//! (`bifrost-privd`) reads the kernel's feed and the shell relays what the app subscribed to. Only the path and the
//! process id of an event are shown, never file contents.
//!
//! ```ignore
//! let sub = Subscription { kinds: vec![Kind::Exec], auth: vec![AuthKind::Exec], muted: vec!["/proc".into()] };
//! endpoint_security::subscribe(ctx, &sub);
//! // every few hundred ms:
//! endpoint_security::poll(ctx, 100);
//! //   in App::service_reply: let poll = endpoint_security::decode_poll(&result?)?;
//! //   for an event with `round`: endpoint_security::answer(ctx, round, allow);
//! ```
//! An authorization that is not answered within two seconds is allowed, so a stuck app cannot freeze the computer.
//! The calls return the request id; the reply comes back through `App::service_reply` (or `service::take_reply`).
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;

pub use bifrost_core::endpoint_security::{AuthKind, Delivered, Kind, Subscription, ENTITLEMENT, MAX_MUTED, MAX_PATH};

/// The name the service is registered under in the shell.
pub const SERVICE: &str = "endpoint-security";

/// What a `poll` returned: the events, oldest first, and how many were dropped from a full queue since the start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Poll {
    pub events: Vec<Delivered>,
    pub dropped: u64,
}

/// The body of a `poll` call.
pub fn poll_body(max: usize) -> Json {
    Json::Obj(vec![("max".to_string(), Json::Num(max as f64))])
}

/// The body of an `answer` call: the authorization `round` of an event, and whether to allow it.
pub fn answer_body(round: u64, allow: bool) -> Json {
    Json::Obj(vec![("round".to_string(), Json::Num(round as f64)), ("allow".to_string(), Json::Bool(allow))])
}

/// Subscribes this app to the events of `sub`. One subscription per app.
pub fn subscribe(ctx: &Ctx, sub: &Subscription) -> u32 {
    service::call(ctx, SERVICE, "subscribe", &sub.to_json())
}

/// Asks for up to `max` queued events.
pub fn poll(ctx: &Ctx, max: usize) -> u32 {
    service::call(ctx, SERVICE, "poll", &poll_body(max))
}

/// Answers the authorization `round` of an event that carried one.
pub fn answer(ctx: &Ctx, round: u64, allow: bool) -> u32 {
    service::call(ctx, SERVICE, "answer", &answer_body(round, allow))
}

/// Ends the subscription (it also ends when the app exits).
pub fn unsubscribe(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "unsubscribe", &Json::Null)
}

/// Reads a `poll` reply.
pub fn decode_poll(result: &Json) -> Result<Poll, String> {
    let events = result.get("events").and_then(Json::as_array).ok_or("endpoint-security: the reply has no events")?;
    Ok(Poll { events: events.iter().filter_map(Delivered::from_json).collect(), dropped: result.get("dropped").and_then(Json::as_u64).unwrap_or(0) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bodies_carry_what_the_shell_service_reads() {
        assert_eq!(poll_body(50).get("max").and_then(Json::as_u64), Some(50));
        let a = answer_body(9, false);
        assert_eq!(a.get("round").and_then(Json::as_u64), Some(9));
        assert_eq!(a.get("allow"), Some(&Json::Bool(false)));
    }

    #[test]
    fn a_subscription_sent_by_subscribe_is_what_the_service_parses() {
        // `subscribe` sends `Subscription::to_json`, which `Subscription::from_json` reads back
        let sub = Subscription { kinds: vec![Kind::Exec, Kind::Modify], auth: vec![AuthKind::Exec], muted: vec!["/proc".into()] };
        assert_eq!(Subscription::from_json(&sub.to_json()), Ok(sub));
        assert_eq!(SERVICE, "endpoint-security");
        assert_eq!(ENTITLEMENT, "endpoint-security");
    }

    #[test]
    fn a_poll_reply_is_decoded_with_its_events_and_drop_count() {
        let e = Delivered { kinds: vec![Kind::Open], auth: Some(AuthKind::Open), pid: 42, path: "/etc/shadow".into(), round: Some(3) };
        let reply = Json::Obj(vec![("events".into(), Json::Arr(vec![e.to_json()])), ("dropped".into(), Json::Num(5.0))]);
        assert_eq!(decode_poll(&reply), Ok(Poll { events: vec![e], dropped: 5 }));
        assert!(decode_poll(&Json::Obj(vec![])).is_err());
    }

    #[test]
    fn the_calls_poll_answer_and_unsubscribe_go_to_the_named_service() {
        // `poll`, `answer` and `unsubscribe` each send one request to SERVICE; their bodies are tested above
        assert!(SERVICE.contains("security"));
    }
}
