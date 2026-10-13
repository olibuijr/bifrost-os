//! The `accessibility_display` service: apps read the Dim Flashing Lights setting, and learn when the user changes it.
//! The setting is `AccessibilitySettings::dim_flashing_lights` (Settings > Accessibility > Dim Flashing Lights, stored as
//! `a11y_dim_flashing_lights`). The shell does not analyse app pictures, so the dimming itself is done by the app with
//! the `bifrost_ui::flashing_lights` model; this service only publishes the flag.
//!
//! Methods (params are ignored): `options()` returns an object with `increase_contrast`, `reduce_transparency`,
//! `differentiate_without_color`, `invert_colors`, `reduce_motion` and `dim_flashing_lights` (all booleans), the
//! Workspace display options; `get()` returns `true` or `false`. The service pushes `changed` to every app with the
//! new flag as its body, once per change (not on the first tick, when no app is running yet).
use super::{Event, Shell, SystemService};
use bifrost_core::json::Json;

/// The service name apps call it by.
pub const SERVICE: &str = "accessibility_display";
/// The method that returns the setting.
pub const METHOD_GET: &str = "get";
/// The event pushed when the setting changes. Its body is the new flag.
pub const EVENT_CHANGED: &str = "changed";
/// The method that returns every display accommodation.
pub const METHOD_OPTIONS: &str = "options";
/// The event pushed when any display accommodation changes. Its body is the same object `options` returns.
pub const EVENT_OPTIONS_CHANGED: &str = "options_changed";

/// The display accommodations apps can read, named after the Workspace accessibility display flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayOptions {
    pub increase_contrast: bool,
    pub reduce_transparency: bool,
    /// Button Shapes: controls are told apart by shape, not by colour alone.
    pub differentiate_without_color: bool,
    /// The shell has no colour inversion, so this is always false.
    pub invert_colors: bool,
    pub reduce_motion: bool,
    pub dim_flashing_lights: bool,
}

impl DisplayOptions {
    pub fn from_settings(s: &crate::shell::accessibility::AccessibilitySettings) -> Self {
        DisplayOptions {
            increase_contrast: s.increase_contrast,
            reduce_transparency: s.reduce_transparency,
            differentiate_without_color: s.button_shapes,
            invert_colors: false,
            reduce_motion: s.reduce_motion,
            dim_flashing_lights: s.dim_flashing_lights,
        }
    }

    pub fn to_json(&self) -> Json {
        let f = |k: &str, v: bool| (k.to_string(), Json::Bool(v));
        Json::Obj(vec![
            f("increase_contrast", self.increase_contrast),
            f("reduce_transparency", self.reduce_transparency),
            f("differentiate_without_color", self.differentiate_without_color),
            f("invert_colors", self.invert_colors),
            f("reduce_motion", self.reduce_motion),
            f("dim_flashing_lights", self.dim_flashing_lights),
        ])
    }
}

/// The event to push when the options are `now` and were `last` on the previous tick; same rules as [`change_event`].
pub fn options_change_event(last: &mut Option<DisplayOptions>, now: DisplayOptions) -> Option<Event> {
    let before = last.replace(now);
    match before {
        Some(b) if b != now => Some(Event { app: None, event: EVENT_OPTIONS_CHANGED.to_string(), body: now.to_json() }),
        _ => None,
    }
}

fn display_options(shell: &Shell) -> DisplayOptions {
    shell.settings.as_ref().map_or_else(
        || DisplayOptions::from_settings(&crate::shell::accessibility::AccessibilitySettings::load(&shell.prefs)),
        |st| DisplayOptions::from_settings(&st.accessibility()),
    )
}

#[derive(Default)]
pub struct AccessibilityDisplay {
    /// The flag last seen by a tick. None until the first tick.
    last: Option<bool>,
    /// The options last seen by a tick. None until the first tick.
    last_options: Option<DisplayOptions>,
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

/// The current Dim Flashing Lights flag: the open Settings window's copy when there is one, else the saved pref.
fn dim_flashing_lights(shell: &Shell) -> bool {
    shell.settings.as_ref().map_or_else(
        || crate::shell::accessibility::AccessibilitySettings::load(&shell.prefs).dim_flashing_lights,
        |st| st.accessibility().dim_flashing_lights,
    )
}

impl SystemService for AccessibilityDisplay {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, shell: &mut Shell, _app_id: &str, method: &str, _params: &Json, _t: f64) -> Result<Json, String> {
        match method {
            METHOD_OPTIONS => Ok(display_options(shell).to_json()),
            METHOD_GET => Ok(Json::Bool(dim_flashing_lights(shell))),
            _ => Err(format!("accessibility_display has no method {method}")),
        }
    }

    fn tick(&mut self, shell: &mut Shell, _t: f64) -> Vec<Event> {
        let now = dim_flashing_lights(shell);
        let mut events: Vec<Event> = change_event(&mut self.last, now).into_iter().collect();
        events.extend(options_change_event(&mut self.last_options, display_options(shell)));
        events
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
    fn options_carry_every_display_setting() {
        let o = DisplayOptions { increase_contrast: true, reduce_motion: true, ..Default::default() };
        let j = o.to_json();
        assert_eq!(j.get("increase_contrast"), Some(&Json::Bool(true)));
        assert_eq!(j.get("reduce_transparency"), Some(&Json::Bool(false)));
        assert_eq!(j.get("differentiate_without_color"), Some(&Json::Bool(false)));
        assert_eq!(j.get("invert_colors"), Some(&Json::Bool(false)));
        assert_eq!(j.get("reduce_motion"), Some(&Json::Bool(true)));
        assert_eq!(j.get("dim_flashing_lights"), Some(&Json::Bool(false)));
    }

    #[test]
    fn button_shapes_is_differentiate_without_color() {
        let mut s = crate::shell::accessibility::AccessibilitySettings::DEFAULT;
        s.button_shapes = true;
        assert!(DisplayOptions::from_settings(&s).differentiate_without_color);
    }

    #[test]
    fn an_option_change_reports_the_whole_snapshot_once() {
        let mut last = None;
        let base = DisplayOptions::default();
        assert_eq!(options_change_event(&mut last, base), None);
        assert_eq!(options_change_event(&mut last, base), None);
        let next = DisplayOptions { reduce_transparency: true, ..base };
        let e = options_change_event(&mut last, next).expect("a change");
        assert_eq!(e.event, "options_changed");
        assert_eq!(e.body, next.to_json());
        assert_eq!(options_change_event(&mut last, next), None);
    }

    #[test]
    fn the_service_is_named_as_the_sdk_expects() {
        assert_eq!(AccessibilityDisplay::default().name(), "accessibility_display");
        assert_eq!(SERVICE, "accessibility_display");
    }
}
