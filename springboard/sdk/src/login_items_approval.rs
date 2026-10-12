//! Typed client for the shell's `login-items` service: an app asks to open at login, reads its status, and hears when the
//! user allows it. The user approves a request in Settings > General > Login Items. Until then `register` leaves the app
//! `RequiresApproval` (the reference's `SMAppService.Status.requiresApproval`); when the user allows it the shell sends a
//! `status` event with `Enabled`, and a request withdrawn with `unregister` sends `NotRegistered`.
//!
//! The calls are asynchronous over the service channel (`service::call`). Their replies arrive as
//! `App::service_reply`, and the status events as `App::service_event`:
//!
//! ```ignore
//! // in App::service_reply(ctx, request, result):
//! if let Some(status) = login_items_approval::status_from_reply(result) { /* show it */ }
//! // in App::service_event(ctx, service, event, body):
//! if let Some(status) = login_items_approval::status_from_event(service, event, body) { /* Enabled after approval */ }
//! ```
//! The same status values as the main-app `service_management` handle; the wire names are `not_registered`, `enabled`,
//! `requires_approval` and `not_found`.
use crate::proto::LoginStatus as Status;
use crate::service;
use crate::Ctx;
use bifrost_core::json::Json;

/// The shell service this client talks to.
pub const SERVICE: &str = "login-items";
/// The event a status change arrives as.
pub const EVENT_STATUS: &str = "status";

/// Asks the shell for the caller's status. Returns the request id the reply carries.
pub fn status(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "status", &Json::Null)
}

/// Asks to open at login. The reply is `RequiresApproval` until the user allows it.
pub fn register(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "register", &Json::Null)
}

/// Removes the caller from the login items and from the pending requests.
pub fn unregister(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "unregister", &Json::Null)
}

/// The wire name of a status.
pub fn status_name(status: Status) -> &'static str {
    match status {
        Status::NotRegistered => "not_registered",
        Status::Enabled => "enabled",
        Status::RequiresApproval => "requires_approval",
        Status::NotFound => "not_found",
    }
}

/// The status a wire name stands for; an unknown name is `None`.
pub fn parse_status(name: &str) -> Option<Status> {
    Some(match name {
        "not_registered" => Status::NotRegistered,
        "enabled" => Status::Enabled,
        "requires_approval" => Status::RequiresApproval,
        "not_found" => Status::NotFound,
        _ => return None,
    })
}

/// The status in a `{"status": "..."}` body.
pub fn status_from_body(body: &Json) -> Option<Status> {
    parse_status(body.get("status")?.as_str()?)
}

/// The status in a call reply, or `None` for an error reply or a body without a status.
pub fn status_from_reply(result: &Result<Json, String>) -> Option<Status> {
    status_from_body(result.as_ref().ok()?)
}

/// The status in a service event from this service, or `None` for any other event.
pub fn status_from_event(service: &str, event: &str, body: &Json) -> Option<Status> {
    if service != SERVICE || event != EVENT_STATUS {
        return None;
    }
    status_from_body(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::ToShell;

    fn test_ctx() -> Ctx {
        use crate::kit::{Font, Ui};
        use crate::proto::Configure;
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
    }

    fn obj(status: &str) -> Json {
        Json::Obj(vec![("status".into(), Json::Str(status.into()))])
    }

    #[test]
    fn the_calls_send_the_login_items_service_methods() {
        let ctx = test_ctx();
        let a = status(&ctx);
        let b = register(&ctx);
        let c = unregister(&ctx);
        let sent = ctx.take_sent();
        assert_eq!(sent[0], ToShell::ServiceCall { request: a, service: "login-items".into(), method: "status".into(), body: "null".into() });
        assert_eq!(sent[1], ToShell::ServiceCall { request: b, service: "login-items".into(), method: "register".into(), body: "null".into() });
        assert_eq!(sent[2], ToShell::ServiceCall { request: c, service: "login-items".into(), method: "unregister".into(), body: "null".into() });
    }

    #[test]
    fn status_names_round_trip() {
        for s in [Status::NotRegistered, Status::Enabled, Status::RequiresApproval, Status::NotFound] {
            assert_eq!(parse_status(status_name(s)), Some(s));
        }
        assert_eq!(parse_status("approved"), None);
    }

    #[test]
    fn a_reply_carries_the_status_and_an_error_carries_none() {
        assert_eq!(status_from_reply(&Ok(obj("requires_approval"))), Some(Status::RequiresApproval));
        assert_eq!(status_from_reply(&Ok(obj("bogus"))), None);
        assert_eq!(status_from_reply(&Err("no service login-items".into())), None);
        assert_eq!(status_from_reply(&Ok(Json::Null)), None);
    }

    #[test]
    fn only_this_services_status_events_are_read() {
        assert_eq!(status_from_event("login-items", "status", &obj("enabled")), Some(Status::Enabled));
        assert_eq!(status_from_event("dock", "status", &obj("enabled")), None);
        assert_eq!(status_from_event("login-items", "other", &obj("enabled")), None);
    }
}
