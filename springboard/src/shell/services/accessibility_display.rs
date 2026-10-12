//! The `accessibility_display` service: apps read the Dim Flashing Lights setting, and learn when the user changes it.
//! The setting is `AccessibilitySettings::dim_flashing_lights` (Settings > Accessibility > Dim Flashing Lights, stored as
//! `a11y_dim_flashing_lights`). The shell does not analyse app pictures, so the dimming itself is done by the app with
//! the `bifrost_ui::flashing_lights` model; this service only publishes the flag.
//!
//! Methods (params are ignored): `get()` returns `true` or `false`. The service pushes `changed` to every app with the
//! new flag as its body, once per change (not on the first tick, when no app is running yet).
use super::{Event, Shell, SystemService};
use bifrost_core::json::Json;

/// The service name apps call it by.
pub const SERVICE: &str = "accessibility_display";
/// The method that returns the setting.
pub const METHOD_GET: &str = "get";
/// The event pushed when the setting changes. Its body is the new flag.
pub const EVENT_CHANGED: &str = "changed";

#[derive(Default)]
pub struct AccessibilityDisplay {
    /// The flag last seen by a tick. None until the first tick.
    last: Option<bool>,
}

/// The event to push when the flag is `now` and was `last` on the previous tick. A change makes one event; the first
/// tick and a quiet tick make none. `last` is updated to `now`.
pub fn change_event(last: &mut Option<bool>, now: bool) -> Option<Event> {
    let before = last.replace(now);
    match before {
        Some(b) if b != now => Some(Event { app: None, event: EVENT_CHANGED.to_string(), body: Json::Bool(now) }),
        _ => None,
    }
}

impl SystemService for AccessibilityDisplay {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, shell: &mut Shell, _app_id: &str, method: &str, _params: &Json, _t: f64) -> Result<Json, String> {
        match method {
            METHOD_GET => Ok(Json::Bool(shell.accessibility().dim_flashing_lights)),
            _ => Err(format!("accessibility_display has no method {method}")),
        }
    }

    fn tick(&mut self, shell: &mut Shell, _t: f64) -> Vec<Event> {
        let now = shell.accessibility().dim_flashing_lights;
        change_event(&mut self.last, now).into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_tick_reports_nothing() {
        let mut last = None;
        assert_eq!(change_event(&mut last, false), None);
        assert_eq!(last, Some(false));
    }

    #[test]
    fn a_quiet_tick_reports_nothing() {
        let mut last = Some(true);
        assert_eq!(change_event(&mut last, true), None);
        assert_eq!(last, Some(true));
    }

    #[test]
    fn a_change_reports_the_new_flag_once() {
        let mut last = Some(false);
        let e = change_event(&mut last, true).expect("a change");
        assert_eq!(e.app, None, "every app hears it");
        assert_eq!(e.event, "changed");
        assert_eq!(e.body, Json::Bool(true));
        assert_eq!(change_event(&mut last, true), None, "the same flag again is quiet");
        let off = change_event(&mut last, false).expect("switched off");
        assert_eq!(off.body, Json::Bool(false));
    }

    #[test]
    fn the_service_is_named_as_the_sdk_expects() {
        assert_eq!(AccessibilityDisplay::default().name(), "accessibility_display");
        assert_eq!(SERVICE, "accessibility_display");
    }
}
