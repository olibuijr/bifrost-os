//! System services that apps call by name: `ToShell::ServiceCall { service, method, body }` is answered with
//! `ToApp::ServiceReply`, and a service may push `ToApp::ServiceEvent`s. The SDK side is `bifrost_app::service`.
//!
//! Adding a service: write `services/<name>.rs` with a type that implements [`SystemService`], add `pub mod <name>;`
//! below and one line to [`all`]. Nothing in the wire protocol changes. A service gets the whole [`Shell`] on every
//! call and tick, so it can read and change any shell state; it must answer at once (no blocking work on the shell
//! thread: hand slow work to a thread and report it later as an event).
use super::Shell;
use bifrost_core::json::Json;

pub mod cursor_service;
pub mod system;

/// A named service in the shell.
pub trait SystemService {
    /// The name apps call it by, for example `"dock"`.
    fn name(&self) -> &'static str;
    /// Handles `method` called by the app `app_id` with JSON `params`. Returns the result or an error message.
    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, t: f64) -> Result<Json, String>;
    /// Runs on every shell tick; returns events to deliver.
    fn tick(&mut self, _shell: &mut Shell, _t: f64) -> Vec<Event> {
        Vec::new()
    }
}

/// An event a service sends: to one app (`app` is its id) or to every running app (`None`).
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub app: Option<String>,
    pub event: String,
    pub body: Json,
}

/// Every service, in registration order. One line per service.
pub fn all() -> Vec<Box<dyn SystemService>> {
    vec![
        Box::new(system::System),
        Box::new(cursor_service::CursorService),
    ]
}

/// The names of the registered services.
pub fn names(list: &[Box<dyn SystemService>]) -> Vec<&'static str> {
    list.iter().map(|s| s.name()).collect()
}

impl Shell {
    /// Runs one service call for the app `app_id`. An unknown service or method is an error, never a panic.
    pub(super) fn service_call(&mut self, app_id: &str, service: &str, method: &str, body: &str, t: f64) -> Result<Json, String> {
        let params = if body.trim().is_empty() { Json::Null } else { bifrost_core::json::parse(body)? };
        let mut list = std::mem::take(&mut self.services);
        let r = match list.iter_mut().find(|s| s.name() == service) {
            Some(s) => s.call(self, app_id, method, &params, t),
            None => Err(format!("no service {service}")),
        };
        // a service registered during the call (none should) is kept after the original list
        list.append(&mut self.services);
        self.services = list;
        r
    }

    /// Ticks every service and sends the events they return to the running apps they name.
    pub(super) fn services_tick(&mut self, t: f64) {
        let mut list = std::mem::take(&mut self.services);
        let mut out = Vec::new();
        for s in list.iter_mut() {
            for e in s.tick(self, t) {
                out.push((s.name(), e));
            }
        }
        list.append(&mut self.services);
        self.services = list;
        if out.is_empty() {
            return;
        }
        let apps = super::apphost::installed();
        for (service, e) in out {
            let m = bifrost_proto::ToApp::ServiceEvent { service: service.to_string(), event: e.event.clone(), body: e.body.encode() };
            for slot in 0..self.ext.len() {
                let id = apps.get(slot).map(|a| a.manifest.id.as_str());
                if e.app.is_none() || e.app.as_deref() == id {
                    self.ext_send(slot, &m);
                }
            }
        }
    }
}
