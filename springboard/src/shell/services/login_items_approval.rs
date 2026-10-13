//! The `login-items` service: an app registers to open at login, and the user allows it in Settings > General > Login
//! Items. Until then the registration is `requires_approval`; every status change goes back to the app as a `status`
//! event with body `{"status": "..."}`, and the calls answer with the same body.
//!
//! Methods: `status` (the caller's current status), `register` (asks to open at login), `unregister` (removes the caller
//! from the list and from the pending requests). The caller is always the app the connection belongs to.
//! `open_settings` shows Settings on the Login Items page (`openSystemSettingsLoginItems()`). A reply that failed carries
//! `"error"` (the `SMAppServiceErrorDomain` case name) and `"code"` (its `kSMError*` value) next to the status.
//! Status names: `not_registered`, `enabled`, `requires_approval`, `not_found` (the SDK client is
//! `bifrost_app::login_items_approval`).
use super::{Event, Shell, SystemService};
use crate::shell::extras::Registry;
use bifrost_core::json::Json;
use bifrost_proto::{LoginOp, LoginStatus};

/// The event name for a status change.
pub const EVENT_STATUS: &str = "status";

pub struct LoginItemsApproval;

/// The wire name of a status.
pub fn status_name(status: LoginStatus) -> &'static str {
    match status {
        LoginStatus::NotRegistered => "not_registered",
        LoginStatus::Enabled => "enabled",
        LoginStatus::RequiresApproval => "requires_approval",
        LoginStatus::NotFound => "not_found",
    }
}

/// The body that carries a status: `{"status": "<name>"}`.
pub fn status_body(status: LoginStatus) -> Json {
    Json::Obj(vec![("status".into(), Json::Str(status_name(status).into()))])
}

/// The error a call gets, as `(name, kSMError code)`, judged from the status before the call: registering an app that is
/// already registered or waiting for approval is `AlreadyRegistered`; removing an app that is not registered is `JobNotFound`.
pub fn call_error(before: LoginStatus, method: &str) -> Option<(&'static str, i64)> {
    match (method, before) {
        ("register", LoginStatus::Enabled | LoginStatus::RequiresApproval) => Some(("already_registered", 11)),
        ("unregister", LoginStatus::NotRegistered | LoginStatus::NotFound) => Some(("job_not_found", 5)),
        _ => None,
    }
}

/// `status_body` plus the `error` and `code` members when the call failed.
pub fn reply_body(status: LoginStatus, error: Option<(&'static str, i64)>) -> Json {
    let mut body = vec![("status".to_string(), Json::Str(status_name(status).into()))];
    if let Some((name, code)) = error {
        body.push(("error".into(), Json::Str(name.into())));
        body.push(("code".into(), Json::Num(code as f64)));
    }
    Json::Obj(body)
}

/// Moves `app` one place earlier in the login order, so it starts before the app above it. False when `app` is first
/// or not in the list (nothing changes then).
pub fn move_earlier(list: &mut [String], app: &str) -> bool {
    match list.iter().position(|a| a == app) {
        Some(i) if i > 0 => {
            list.swap(i - 1, i);
            true
        }
        _ => false,
    }
}

/// What `method` does for `app` in `reg`: the status to answer with and whether a list changed.
pub fn apply(reg: &mut Registry, app: &str, method: &str) -> Result<(LoginStatus, bool), String> {
    let op = match method {
        "status" => LoginOp::Query,
        "register" => LoginOp::Register,
        "unregister" => LoginOp::Unregister,
        _ => return Err(format!("login-items has no method {method}")),
    };
    Ok(reg.login_op(app, op))
}

impl SystemService for LoginItemsApproval {
    fn name(&self) -> &'static str {
        "login-items"
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, _params: &Json, _t: f64) -> Result<Json, String> {
        if method == "open_settings" {
            let _ = shell.open_settings_page("Login Items & Extensions", _t);
            shell.dirty = true;
            return Ok(status_body(shell.extras.login_status(app_id)));
        }
        let before = shell.extras.login_status(app_id);
        let (status, changed) = apply(&mut shell.extras, app_id, method)?;
        if changed {
            shell.login_save();
            shell.dirty = true;
        }
        Ok(reply_body(status, call_error(before, method)))
    }

    fn tick(&mut self, shell: &mut Shell, _t: f64) -> Vec<Event> {
        shell.extras.take_changes().into_iter().map(|(app, status)| Event { app: Some(app), event: EVENT_STATUS.into(), body: status_body(status) }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_names_match_the_sdk_wire_names() {
        assert_eq!(status_name(LoginStatus::NotRegistered), "not_registered");
        assert_eq!(status_name(LoginStatus::Enabled), "enabled");
        assert_eq!(status_name(LoginStatus::RequiresApproval), "requires_approval");
        assert_eq!(status_name(LoginStatus::NotFound), "not_found");
        assert_eq!(status_body(LoginStatus::Enabled).get("status").and_then(Json::as_str), Some("enabled"));
    }

    #[test]
    fn register_waits_for_approval_and_status_reports_it() {
        let mut reg = Registry::default();
        assert_eq!(apply(&mut reg, "a", "status"), Ok((LoginStatus::NotRegistered, false)));
        assert_eq!(apply(&mut reg, "a", "register"), Ok((LoginStatus::RequiresApproval, true)));
        assert_eq!(apply(&mut reg, "a", "status"), Ok((LoginStatus::RequiresApproval, false)));
        assert!(reg.approve("a"));
        assert_eq!(apply(&mut reg, "a", "status"), Ok((LoginStatus::Enabled, false)));
        assert_eq!(apply(&mut reg, "a", "unregister"), Ok((LoginStatus::NotRegistered, true)));
    }

    #[test]
    fn registering_twice_is_already_registered_with_the_apple_code() {
        assert_eq!(call_error(LoginStatus::Enabled, "register"), Some(("already_registered", 11)));
        assert_eq!(call_error(LoginStatus::RequiresApproval, "register"), Some(("already_registered", 11)));
        assert_eq!(call_error(LoginStatus::NotRegistered, "register"), None);
    }

    #[test]
    fn unregistering_an_unknown_item_is_job_not_found() {
        assert_eq!(call_error(LoginStatus::NotRegistered, "unregister"), Some(("job_not_found", 5)));
        assert_eq!(call_error(LoginStatus::Enabled, "unregister"), None);
        assert_eq!(call_error(LoginStatus::Enabled, "status"), None);
    }

    #[test]
    fn a_failed_reply_carries_error_and_code_next_to_the_status() {
        let body = reply_body(LoginStatus::Enabled, Some(("already_registered", 11)));
        assert_eq!(body.get("status").and_then(Json::as_str), Some("enabled"));
        assert_eq!(body.get("error").and_then(Json::as_str), Some("already_registered"));
        assert!(body.get("code").is_some());
        assert!(reply_body(LoginStatus::Enabled, None).get("error").is_none());
    }

    #[test]
    fn move_earlier_swaps_with_the_app_above() {
        let mut list: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        assert!(move_earlier(&mut list, "b"));
        assert_eq!(list, ["b", "a", "c"]);
        assert!(!move_earlier(&mut list, "b"), "the first app cannot move earlier");
        assert!(!move_earlier(&mut list, "zz"), "an app that is not listed does not move");
        assert_eq!(list, ["b", "a", "c"]);
    }

    #[test]
    fn an_unknown_method_is_an_error_not_a_change() {
        let mut reg = Registry::default();
        assert!(apply(&mut reg, "a", "approve").is_err());
        assert!(reg.pending_ids().is_empty());
        assert!(reg.take_changes().is_empty());
    }
}
