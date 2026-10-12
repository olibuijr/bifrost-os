//! The `login-items` service: an app registers to open at login, and the user allows it in Settings > General > Login
//! Items. Until then the registration is `requires_approval`; every status change goes back to the app as a `status`
//! event with body `{"status": "..."}`, and the calls answer with the same body.
//!
//! Methods: `status` (the caller's current status), `register` (asks to open at login), `unregister` (removes the caller
//! from the list and from the pending requests). The caller is always the app the connection belongs to.
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
        let (status, changed) = apply(&mut shell.extras, app_id, method)?;
        if changed {
            shell.login_save();
            shell.dirty = true;
        }
        Ok(status_body(status))
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
    fn an_unknown_method_is_an_error_not_a_change() {
        let mut reg = Registry::default();
        assert!(apply(&mut reg, "a", "approve").is_err());
        assert!(reg.pending_ids().is_empty());
        assert!(reg.take_changes().is_empty());
    }
}
