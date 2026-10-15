//! Named notifications between apps, over the shell's `notify_bus` service (`springboard/src/shell/services/notify_bus.rs`).
//! The model is the name-keyed notification centre with integer tokens (`bifrost_core::notify_center`).
//!
//! ```ignore
//! // post a name to every app that holds a token on it (the shell refuses `system.` names)
//! notify_bus::post(ctx, "com.example.refresh");
//! // hear a name: ask for a token, then read the reply in App::service_reply
//! let id = notify_bus::register(ctx, "com.example.refresh", notify_bus::Mode::Event);
//! // App::service_reply: notify_bus::token_from_reply(&result) gives the token
//! // App::service_event: notify_bus::notified(service, event, &body) gives the name that was posted
//! ```
//! Every call returns the request id its reply carries. A token belongs to the app that registered it: suspend,
//! resume, cancel and check fail for another app's token. Gaps: no file descriptor or signal delivery, no
//! `suspensionBehavior`, and a token is not released when its app quits (the app must cancel it).
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;

/// The service name, as the shell registers it.
pub const SERVICE: &str = "notify_bus";
/// The event the shell pushes to the owner of an event token after a post.
pub const EVENT: &str = "notified";
/// Posted by the shell when the scheme changes between Light and Dark.
pub const APPEARANCE_CHANGE: &str = "system.appearance.change";
/// Posted by the shell when the screen locks.
pub const SCREEN_LOCKED: &str = "system.screen.locked";
/// Posted by the shell when the screen unlocks.
pub const SCREEN_UNLOCKED: &str = "system.screen.unlocked";
/// Posted by the shell when the connection goes up or down.
pub const NETWORK_CHANGE: &str = "system.network.change";
/// Posted by the shell when the contacts file changes (the Contacts app saved, or the file appeared or went).
pub const CONTACTS_CHANGE: &str = "system.contacts.change";
/// Posted by the shell when the events file changes (the Calendar app saved, or the file appeared or went).
pub const CALENDAR_CHANGE: &str = "system.calendar.change";
/// Posted by the shell when the local time zone changes.
pub const TIME_ZONE_CHANGE: &str = "system.timezone.change";
/// Posted by the shell when the host name changes.
pub const HOSTNAME_CHANGE: &str = "system.hostname.change";
/// Posted by the shell when a removable volume is mounted.
pub const VOLUME_MOUNT: &str = "system.volume.mount";
/// Posted by the shell when a removable volume is unmounted.
pub const VOLUME_UNMOUNT: &str = "system.volume.unmount";

/// How a token delivers its posts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The shell pushes a `notified` event to the app after each post (the default).
    Event,
    /// The app asks with [`check`]; the mark stays until it does.
    Poll,
}

/// A name the shell delivered to one of this app's tokens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notified {
    pub token: u32,
    pub name: String,
}

fn call(ctx: &Ctx, method: &str, fields: Vec<(&str, Json)>) -> u32 {
    let params = Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect());
    service::call(ctx, SERVICE, method, &params)
}

/// Posts `name` to every live token on it. The reply is `{delivered}`.
pub fn post(ctx: &Ctx, name: &str) -> u32 {
    call(ctx, "post", vec![("name", Json::Str(name.into()))])
}

/// Makes a token on `name`. The reply is `{token}`, read with [`token_from_reply`].
pub fn register(ctx: &Ctx, name: &str, mode: Mode) -> u32 {
    let mode = if mode == Mode::Poll { "poll" } else { "event" };
    call(ctx, "register", vec![("name", Json::Str(name.into())), ("mode", Json::Str(mode.into()))])
}

/// Reads a poll token's mark and clears it. The reply is `{pending}`, read with [`flag_from_reply`].
pub fn check(ctx: &Ctx, token: u32) -> u32 {
    call(ctx, "check", vec![("token", Json::Num(token as f64))])
}

/// Stops delivery to `token` until [`resume`]. The reply is `{ok}`.
pub fn suspend(ctx: &Ctx, token: u32) -> u32 {
    call(ctx, "suspend", vec![("token", Json::Num(token as f64))])
}

/// Restarts delivery to a suspended `token`. The reply is `{ok}`.
pub fn resume(ctx: &Ctx, token: u32) -> u32 {
    call(ctx, "resume", vec![("token", Json::Num(token as f64))])
}

/// Removes `token`. The reply is `{ok}`.
pub fn cancel(ctx: &Ctx, token: u32) -> u32 {
    call(ctx, "cancel", vec![("token", Json::Num(token as f64))])
}

/// Asks whether `token` is still live. The reply is `{valid}`.
pub fn valid(ctx: &Ctx, token: u32) -> u32 {
    call(ctx, "valid", vec![("token", Json::Num(token as f64))])
}

/// Sets the state value of `name` (shared by every token on it). Refused for `system.` names.
pub fn set_state(ctx: &Ctx, name: &str, state: u64) -> u32 {
    call(ctx, "set_state", vec![("name", Json::Str(name.into())), ("state", Json::Num(state as f64))])
}

/// Reads the state value of `name`. The reply is `{state}`, read with [`state_from_reply`].
pub fn state(ctx: &Ctx, name: &str) -> u32 {
    call(ctx, "state", vec![("name", Json::Str(name.into()))])
}

/// Reads the token from a `register` reply.
pub fn token_from_reply(result: &Result<Json, String>) -> Result<u32, String> {
    let body = result.as_ref().map_err(|e| e.clone())?;
    body.get("token")
        .and_then(Json::number)
        .filter(|n| n.fract() == 0.0 && *n >= 1.0 && *n <= u32::MAX as f64)
        .map(|n| n as u32)
        .ok_or_else(|| "notify_bus: the reply has no token".to_string())
}

/// Reads the boolean `key` (`pending`, `ok` or `valid`) from a reply.
pub fn flag_from_reply(result: &Result<Json, String>, key: &str) -> Result<bool, String> {
    let body = result.as_ref().map_err(|e| e.clone())?;
    match body.get(key) {
        Some(Json::Bool(b)) => Ok(*b),
        _ => Err(format!("notify_bus: the reply has no {key}")),
    }
}

/// Reads the state value from a `state` reply.
pub fn state_from_reply(result: &Result<Json, String>) -> Result<u64, String> {
    let body = result.as_ref().map_err(|e| e.clone())?;
    body.get("state")
        .and_then(Json::number)
        .filter(|n| n.fract() == 0.0 && *n >= 0.0)
        .map(|n| n as u64)
        .ok_or_else(|| "notify_bus: the reply has no state".to_string())
}

/// If `service_name`/`event` is a delivered name, decodes its body. None for any other event or a malformed body.
pub fn notified(service_name: &str, event: &str, body: &Json) -> Option<Notified> {
    if service_name != SERVICE || event != EVENT {
        return None;
    }
    let token = body.get("token").and_then(Json::number)?;
    let name = body.str_field("name")?;
    Some(Notified { token: token as u32, name })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(text: &str) -> Result<Json, String> {
        Ok(bifrost_core::json::parse(text).unwrap())
    }

    #[test]
    fn a_register_reply_gives_its_token() {
        assert_eq!(token_from_reply(&reply(r#"{"token":7}"#)), Ok(7));
        assert!(token_from_reply(&reply(r#"{"token":0}"#)).is_err());
        assert!(token_from_reply(&reply(r#"{"token":1.5}"#)).is_err());
        assert_eq!(token_from_reply(&Err("no service notify_bus".into())), Err("no service notify_bus".to_string()));
    }

    #[test]
    fn flags_and_state_are_read_from_their_replies() {
        assert_eq!(flag_from_reply(&reply(r#"{"pending":true}"#), "pending"), Ok(true));
        assert_eq!(flag_from_reply(&reply(r#"{"ok":false}"#), "ok"), Ok(false));
        assert!(flag_from_reply(&reply(r#"{"ok":1}"#), "ok").is_err());
        assert_eq!(state_from_reply(&reply(r#"{"state":42}"#)), Ok(42));
        assert!(state_from_reply(&reply(r#"{"state":-3}"#)).is_err());
    }

    #[test]
    fn a_notified_event_decodes_its_token_and_name() {
        let body = bifrost_core::json::parse(r#"{"token":3,"name":"system.network.change"}"#).unwrap();
        assert_eq!(
            notified(SERVICE, EVENT, &body),
            Some(Notified { token: 3, name: NETWORK_CHANGE.to_string() })
        );
        assert_eq!(notified("dock", EVENT, &body), None);
        assert_eq!(notified(SERVICE, "pathUpdate", &body), None);
        assert_eq!(notified(SERVICE, EVENT, &Json::Null), None);
    }

    #[test]
    fn the_shell_names_start_with_system() {
        for n in [APPEARANCE_CHANGE, SCREEN_LOCKED, SCREEN_UNLOCKED, NETWORK_CHANGE] {
            assert!(n.starts_with("system."));
        }
    }
}
