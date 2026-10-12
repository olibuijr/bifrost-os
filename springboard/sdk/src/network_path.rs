//! Network path for apps: the system's connectivity path (satisfied or not, interface types, expensive) from the
//! shell's `network` service, built on the model in `bifrost_core::net_path`.
//!
//! ```ignore
//! // in an app's event hook, after asking once with request_path(ctx):
//! fn service_reply(&mut self, _ctx: &Ctx, id: u32, result: &Result<Json, String>) -> bool {
//!     if id == self.path_request { self.path = network_path::decode_reply(result.clone()).ok(); return true; }
//!     false
//! }
//! fn service_event(&mut self, _ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
//!     match network_path::path_event(service, event, body) {
//!         Some(Ok(path)) => { self.path = Some(path); true }
//!         _ => false,
//!     }
//! }
//! ```
//! `PathMonitor` (re-exported here) is the same change filter the shell uses: `update` returns the new path only
//! when it differs. The shell pushes `pathUpdate` to every running app when the path changes, so an app needs
//! no polling. Reachability of a named host is not part of this API: the path says whether traffic can leave the
//! machine, and nothing probes a host.
//!
//! The wire form is defined by `springboard::shell::services::network_path` and read back here, so the two sides
//! share one codec.

use crate::runtime::Ctx;
use bifrost_core::json::Json;
use springboard::shell::services::network_path as wire;

pub use bifrost_core::net_path::{IfaceKind, Interface, Path, PathMonitor, Status};

/// The shell service an app calls.
pub const SERVICE: &str = "network";

/// The event the shell pushes when the path changes.
pub const PATH_EVENT: &str = wire::PATH_EVENT;

/// Asks the shell for the current path. The reply arrives as `App::service_reply` with this id (or via
/// `service::take_reply`); read it with [`decode_reply`].
pub fn request_path(ctx: &Ctx) -> u32 {
    crate::service::call(ctx, SERVICE, "path", &Json::Null)
}

/// Reads a reply to [`request_path`]. A service error or a malformed body is an `Err`.
pub fn decode_reply(reply: Result<Json, String>) -> Result<Path, String> {
    wire::path_from_json(&reply?)
}

/// Reads a `pathUpdate` event. Returns `None` for any other service or event, so an app can pass every
/// event it receives through this.
pub fn path_event(service: &str, event: &str, body: &Json) -> Option<Result<Path, String>> {
    if service != SERVICE || event != PATH_EVENT {
        return None;
    }
    Some(wire::path_from_json(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(status: &str, expensive: bool) -> Json {
        bifrost_core::json::parse(&format!(
            r#"{{"status":"{status}","interfaces":[{{"name":"eth0","kind":"wired"}}],"expensive":{expensive},"ipv4":true,"ipv6":false}}"#
        ))
        .unwrap()
    }

    #[test]
    fn a_path_event_from_the_network_service_is_decoded() {
        let got = path_event("network", "pathUpdate", &body("satisfied", false));
        let path = got.expect("network event").expect("valid path");
        assert_eq!(path.status, Status::Satisfied);
        assert_eq!(path.interfaces, vec![Interface { name: "eth0".into(), kind: IfaceKind::Wired }]);
        assert!(!path.expensive && path.ipv4 && !path.ipv6);
    }

    #[test]
    fn other_services_and_events_are_not_ours() {
        assert!(path_event("dock", "pathUpdate", &body("satisfied", false)).is_none());
        assert!(path_event("network", "wifi", &body("satisfied", false)).is_none());
    }

    #[test]
    fn a_malformed_event_body_is_an_error() {
        assert!(path_event("network", "pathUpdate", &Json::Null).unwrap().is_err());
    }

    #[test]
    fn replies_decode_to_a_path_or_an_error() {
        assert_eq!(decode_reply(Ok(body("requiresConnection", true))).unwrap().status, Status::RequiresConnection);
        assert!(decode_reply(Ok(body("satisfied", false))).unwrap().status == Status::Satisfied);
        assert_eq!(decode_reply(Err("no service network".into())), Err("no service network".into()));
    }

    #[test]
    fn the_monitor_reports_only_changes_of_a_decoded_path() {
        let mut m = PathMonitor::new();
        let first = decode_reply(Ok(body("satisfied", false))).unwrap();
        assert!(m.update(first.clone()).is_some());
        assert!(m.update(first).is_none());
    }
}
