//! RunningBoard: process assertions. An app that nothing shows is stopped after a short grace period
//! (`shell::background`, `bifrost_core::occlusion`). An app that is playing audio, downloading or exporting asks this
//! service to keep it running meanwhile.
//!
//! Service methods (`runningboard`, called by the app; the SDK side is `bifrost_app::running_board`):
//! - `take` `{reason, limit?}` -> `{"id": n}`: `reason` is 1 to 64 printable characters, `limit` is in seconds (60 when
//!   omitted, at most 3600). The assertion lapses at its limit, so renew it by taking a new one.
//! - `release` `{id}` -> `{"released": bool}`
//! - `held` -> `{"held": n}`: how many assertions the app holds now.
//!
//! The limits and the bookkeeping are `bifrost_core::process_assertion`; the shell's `Suspender` reads them on every
//! background tick and does not stop an app while it holds one. An app that quits loses its assertions.
use super::{Shell, SystemService};
use crate::shell::apphost;
use bifrost_core::json::Json;

/// The `runningboard` system service.
pub struct RunningBoard;

fn obj(pairs: Vec<(&str, Json)>) -> Json {
    Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// The external app slot of the running installed app `app_id`.
fn slot_of(app_id: &str) -> Result<u8, String> {
    let slot = apphost::slot_of(app_id).ok_or_else(|| format!("{app_id} is not an installed app"))?;
    u8::try_from(slot).map_err(|_| "app slot out of range".to_string())
}

/// The milliseconds clock of the background tick, from the shell's seconds.
fn millis(t: f64) -> u64 {
    (t.max(0.0) * 1000.0) as u64
}

/// The limit in milliseconds that the `limit` parameter (seconds) asks for.
fn limit_ms(params: &Json) -> Result<Option<u64>, String> {
    match params.get("limit") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Num(n)) if n.is_finite() && *n > 0.0 => Ok(Some((*n * 1000.0) as u64)),
        Some(_) => Err("limit must be a positive number of seconds".to_string()),
    }
}

impl SystemService for RunningBoard {
    fn name(&self) -> &'static str {
        "runningboard"
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, t: f64) -> Result<Json, String> {
        let slot = slot_of(app_id)?;
        let now = millis(t);
        match method {
            "take" => {
                let reason = params.str_field("reason").ok_or_else(|| "missing reason".to_string())?;
                let limit = limit_ms(params)?;
                let id = shell.suspender.take_assertion(slot, &reason, limit, now).map_err(|e| e.to_string())?;
                Ok(obj(vec![("id", Json::Num(id as f64))]))
            }
            "release" => {
                let id = params.get("id").and_then(Json::as_u64).and_then(|n| u32::try_from(n).ok()).ok_or_else(|| "missing id".to_string())?;
                Ok(obj(vec![("released", Json::Bool(shell.suspender.release_assertion(slot, id)))]))
            }
            "held" => Ok(obj(vec![("held", Json::Num(shell.suspender.assertions_held(slot, now) as f64))])),
            _ => Err(format!("no method {method}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(n: f64) -> Json {
        Json::Num(n)
    }

    #[test]
    fn the_limit_is_read_in_seconds() {
        assert_eq!(limit_ms(&Json::Null), Ok(None));
        assert_eq!(limit_ms(&obj(vec![("limit", num(90.0))])), Ok(Some(90_000)));
        assert_eq!(limit_ms(&obj(vec![("limit", Json::Null)])), Ok(None));
    }

    #[test]
    fn a_bad_limit_is_refused() {
        assert!(limit_ms(&obj(vec![("limit", num(0.0))])).is_err());
        assert!(limit_ms(&obj(vec![("limit", num(-5.0))])).is_err());
        assert!(limit_ms(&obj(vec![("limit", Json::Str("soon".into()))])).is_err());
        assert!(limit_ms(&obj(vec![("limit", num(f64::NAN))])).is_err());
    }

    #[test]
    fn the_clock_is_milliseconds_and_never_negative() {
        assert_eq!(millis(1.5), 1500);
        assert_eq!(millis(-3.0), 0);
    }

    #[test]
    fn the_service_is_named_runningboard() {
        assert_eq!(RunningBoard.name(), "runningboard");
    }
}
