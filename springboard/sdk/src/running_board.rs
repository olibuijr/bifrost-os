//! Process assertions for apps (RunningBoard): ask the shell to keep the app running while nothing shows it, for
//! example while audio plays or a download runs. The shell otherwise stops an app that has been unseen and unfocused
//! for a short while (`bifrost_core::occlusion`). The model and its limits are `bifrost_core::process_assertion`; this
//! module is the app side, over the `runningboard` system service (`springboard/src/shell/services/running_board.rs`).
//!
//! An assertion has a reason and a time limit (60 s unless the app asks for another, 1 hour at most). Renew by taking
//! a new one before the old lapses, and release it when the work is done:
//!
//! ```ignore
//! let request = running_board::take(ctx, "Exporting video", Some(120));
//! // the reply carries the id: running_board::assertion_id(&reply)
//! running_board::release(ctx, id);
//! ```

pub use bifrost_core::process_assertion::{Refused, DEFAULT_LIMIT_MS, MAX_LIMIT_MS, MAX_PER_APP, MAX_REASON};

use crate::runtime::Ctx;
use bifrost_core::json::Json;

/// The shell service the calls go to.
pub const SERVICE: &str = "runningboard";

/// The body of a `take` call: the reason and, when given, the limit in seconds.
pub fn take_body(reason: &str, limit_secs: Option<u64>) -> Json {
    let mut pairs = vec![("reason".to_string(), Json::Str(reason.to_string()))];
    if let Some(s) = limit_secs {
        pairs.push(("limit".to_string(), Json::Num(s as f64)));
    }
    Json::Obj(pairs)
}

/// The body of a `release` call.
pub fn release_body(id: u32) -> Json {
    Json::Obj(vec![("id".to_string(), Json::Num(id as f64))])
}

/// Asks the shell to keep this app running for `limit_secs` seconds (the default when `None`). Returns the service
/// request id; the reply is `{"id": n}`, read with [`assertion_id`].
pub fn take(ctx: &Ctx, reason: &str, limit_secs: Option<u64>) -> u32 {
    crate::service::call(ctx, SERVICE, "take", &take_body(reason, limit_secs))
}

/// Drops the assertion `id`. Returns the service request id; the reply is `{"released": bool}`.
pub fn release(ctx: &Ctx, id: u32) -> u32 {
    crate::service::call(ctx, SERVICE, "release", &release_body(id))
}

/// Asks how many assertions this app holds. Returns the service request id; the reply is `{"held": n}`.
pub fn held(ctx: &Ctx) -> u32 {
    crate::service::call(ctx, SERVICE, "held", &Json::Null)
}

/// The assertion id in a reply to [`take`].
pub fn assertion_id(reply: &Json) -> Option<u32> {
    reply.get("id").and_then(Json::as_u64).and_then(|n| u32::try_from(n).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_take_body_carries_the_reason_and_the_limit() {
        let b = take_body("Download", Some(90));
        assert_eq!(b.str_field("reason").as_deref(), Some("Download"));
        assert_eq!(b.get("limit").and_then(Json::as_u64), Some(90));
        assert!(take_body("x", None).get("limit").is_none());
    }

    #[test]
    fn the_id_is_read_from_a_reply() {
        let reply = Json::Obj(vec![("id".to_string(), Json::Num(7.0))]);
        assert_eq!(assertion_id(&reply), Some(7));
        assert_eq!(assertion_id(&Json::Null), None);
        assert_eq!(release_body(3).get("id").and_then(Json::as_u64), Some(3));
    }
}
