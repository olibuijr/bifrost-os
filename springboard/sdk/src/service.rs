//! System services: named services in the shell that an app calls with JSON (`springboard/src/shell/services/`).
//!
//! ```ignore
//! let id = service::call(ctx, "system", "ping", &Json::Str("hi".into()));
//! // later, in App::service_reply (or by polling service::take_reply(id)):
//! //   Ok(Json::Str("hi"))
//! ```
//! A reply arrives as `App::service_reply(ctx, id, result)`; when that hook returns false the reply is kept for
//! [`take_reply`]. Events a service pushes arrive as `App::service_event`.
use crate::runtime::Ctx;
use bifrost_core::json::Json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

static NEXT: AtomicU32 = AtomicU32::new(1);
static MAILBOX: Mutex<Option<HashMap<u32, Result<Json, String>>>> = Mutex::new(None);

/// Calls `method` on the shell service `service` with `params`. Returns the request id the reply will carry.
pub fn call(ctx: &Ctx, service: &str, method: &str, params: &Json) -> u32 {
    let request = NEXT.fetch_add(1, Ordering::Relaxed);
    ctx.send_to_shell(crate::proto::ToShell::ServiceCall {
        request,
        service: service.to_string(),
        method: method.to_string(),
        body: params.encode(),
    });
    request
}

/// The reply to `request` if it arrived and no `App::service_reply` hook took it. Each reply is returned once.
pub fn take_reply(request: u32) -> Option<Result<Json, String>> {
    MAILBOX.lock().ok()?.as_mut()?.remove(&request)
}

/// Turns a wire reply into a result: an `ok` body is parsed as JSON, an error body is the message.
pub(crate) fn decode_reply(ok: bool, body: &str) -> Result<Json, String> {
    if ok {
        if body.trim().is_empty() {
            Ok(Json::Null)
        } else {
            bifrost_core::json::parse(body)
        }
    } else {
        Err(body.to_string())
    }
}

/// Keeps a reply no hook took, for [`take_reply`]. At most 256 are kept; the oldest ids go first.
pub(crate) fn keep(request: u32, result: Result<Json, String>) {
    if let Ok(mut g) = MAILBOX.lock() {
        let map = g.get_or_insert_with(HashMap::new);
        if map.len() >= 256 {
            if let Some(oldest) = map.keys().min().copied() {
                map.remove(&oldest);
            }
        }
        map.insert(request, result);
    }
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

    #[test]
    fn a_call_sends_the_service_method_and_json_body_with_a_fresh_id() {
        let ctx = test_ctx();
        let a = call(&ctx, "system", "ping", &Json::Str("hi".into()));
        let b = call(&ctx, "system", "ping", &Json::Null);
        assert_ne!(a, b);
        let sent = ctx.take_sent();
        assert_eq!(sent[0], ToShell::ServiceCall { request: a, service: "system".into(), method: "ping".into(), body: "\"hi\"".into() });
        assert_eq!(sent[1], ToShell::ServiceCall { request: b, service: "system".into(), method: "ping".into(), body: "null".into() });
    }

    #[test]
    fn replies_decode_ok_bodies_as_json_and_errors_as_text() {
        assert_eq!(decode_reply(true, "[1,2]"), Ok(Json::Arr(vec![Json::Num(1.0), Json::Num(2.0)])));
        assert_eq!(decode_reply(true, ""), Ok(Json::Null));
        assert_eq!(decode_reply(false, "no service dock"), Err("no service dock".into()));
        assert!(decode_reply(true, "{bad").is_err());
    }

    #[test]
    fn a_kept_reply_is_taken_once() {
        keep(900_001, Ok(Json::Bool(true)));
        assert_eq!(take_reply(900_001), Some(Ok(Json::Bool(true))));
        assert_eq!(take_reply(900_001), None);
    }
}
