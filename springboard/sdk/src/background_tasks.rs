//! Background tasks for apps: the app asks the shell to run a task later, and the shell launches it (hidden) when the
//! request is due and the machine allows it. The model and its JSON codec live in `bifrost_core::background_tasks`;
//! this module is the app side, over the `background` system service (`springboard/src/shell/services/background.rs`).
//!
//! The app declares its task identifiers in `manifest.toml`:
//! ```toml
//! [background]
//! tasks = ["com.example.app.refresh"]
//! ```
//! Then, for example in `App::service_event`, forward the events and finish each task:
//! ```ignore
//! fn service_event(&mut self, ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
//!     match background_tasks::task_event(service, event, body) {
//!         Some(TaskEvent::Started { identifier, .. }) => { /* do the work, then */ background_tasks::completed(ctx, &identifier); true }
//!         Some(TaskEvent::Expiring { identifier }) => { /* stop early and call completed */ true }
//!         None => false,
//!     }
//! }
//! ```
//! Submitting a request whose identifier is not in the manifest is refused by the shell.

pub use bifrost_core::background_tasks::{Conditions, Error, Kind, Request, Scheduler};

use crate::runtime::Ctx;
use bifrost_core::json::Json;

/// The shell service the requests go to.
pub const SERVICE: &str = "background";
/// The event the shell sends when a task starts. Body: `{"identifier", "limit"}`, the limit in seconds.
pub const EVENT_STARTED: &str = "taskStarted";
/// The event the shell sends shortly before the limit. Body: `{"identifier"}`.
pub const EVENT_EXPIRING: &str = "taskExpiring";

/// Asks the shell to run the task `request.identifier` as the request says. Returns the service request id.
pub fn submit(ctx: &Ctx, request: &Request) -> u32 {
    crate::service::call(ctx, SERVICE, "submit", &request.to_json())
}

/// Removes the pending request `identifier`. Returns the service request id.
pub fn cancel(ctx: &Ctx, identifier: &str) -> u32 {
    crate::service::call(ctx, SERVICE, "cancel", &identifier_body(identifier))
}

/// Tells the shell the running task `identifier` is done. The shell then suspends the app, or ends it when the shell
/// launched it for the task. Returns the service request id.
pub fn completed(ctx: &Ctx, identifier: &str) -> u32 {
    crate::service::call(ctx, SERVICE, "completed", &identifier_body(identifier))
}

fn identifier_body(identifier: &str) -> Json {
    Json::Obj(vec![("identifier".to_string(), Json::Str(identifier.to_string()))])
}

/// A task event from the shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskEvent {
    /// The shell started task `identifier`. It has `limit_secs` seconds; call [`completed`] before then.
    Started { identifier: String, limit_secs: u64 },
    /// The time limit is close. Finish now and call [`completed`].
    Expiring { identifier: String },
}

/// The task event for one `ServiceEvent` the app received, or None when it is not from the background service.
pub fn task_event(service: &str, event: &str, body: &Json) -> Option<TaskEvent> {
    if service != SERVICE {
        return None;
    }
    let identifier = body.str_field("identifier").filter(|s| !s.is_empty())?;
    match event {
        EVENT_STARTED => {
            let limit_secs = body.get("limit").and_then(Json::as_u64)?;
            Some(TaskEvent::Started { identifier, limit_secs })
        }
        EVENT_EXPIRING => Some(TaskEvent::Expiring { identifier }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn started_and_expiring_events_are_read() {
        let started = bifrost_core::json::parse(r#"{"identifier":"com.x.refresh","limit":30}"#).unwrap();
        assert_eq!(
            task_event(SERVICE, EVENT_STARTED, &started),
            Some(TaskEvent::Started { identifier: "com.x.refresh".into(), limit_secs: 30 })
        );
        let expiring = bifrost_core::json::parse(r#"{"identifier":"com.x.refresh"}"#).unwrap();
        assert_eq!(task_event(SERVICE, EVENT_EXPIRING, &expiring), Some(TaskEvent::Expiring { identifier: "com.x.refresh".into() }));
    }

    #[test]
    fn other_services_and_events_are_not_task_events() {
        let body = bifrost_core::json::parse(r#"{"identifier":"a","limit":1}"#).unwrap();
        assert_eq!(task_event("power", EVENT_STARTED, &body), None);
        assert_eq!(task_event(SERVICE, "somethingElse", &body), None);
        let no_limit = bifrost_core::json::parse(r#"{"identifier":"a"}"#).unwrap();
        assert_eq!(task_event(SERVICE, EVENT_STARTED, &no_limit), None);
    }
}
