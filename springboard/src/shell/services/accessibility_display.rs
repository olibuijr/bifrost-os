//! The `accessibility_display` service: apps read the Dim Flashing Lights setting, and learn when the user changes it.
//! The setting is `AccessibilitySettings::dim_flashing_lights` (Settings > Accessibility > Dim Flashing Lights, stored as
//! `a11y_dim_flashing_lights`). The shell does not analyse app pictures, so the dimming itself is done by the app with
//! the `bifrost_ui::flashing_lights` model; this service only publishes the flag.
//!
//! Methods (params are ignored): `options()` returns an object with `increase_contrast`, `reduce_transparency`,
//! `differentiate_without_color`, `invert_colors`, `reduce_motion` and `dim_flashing_lights` (all booleans), the
//! Workspace display options; `get()` returns `true` or `false`. The service pushes `changed` to every app with the
//! new flag as its body, once per change (not on the first tick, when no app is running yet).
//!
//! `settings()` returns every accessibility preference in one object (the SDK's `accessibility_settings::Settings`): the
//! display options above, `show_borders`, `animated_images`, `bold_text`, `text_scale`, `display_zoom`, `live_captions`,
//! the preferences the shell has no setting for (always false), `application_accessibility` and `running`, the names of
//! the assistive technologies running now. `settings_changed` carries the same object once per change.
//! `open_feature({"feature": name})` opens Settings on that feature's page and answers `true`; a feature with no page is
//! an error. `feature_supported({"feature": name})` answers whether it has one.
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
/// The method that returns every accessibility preference.
pub const METHOD_SETTINGS: &str = "settings";
/// The method that opens the Settings page of one feature.
pub const METHOD_OPEN_FEATURE: &str = "open_feature";
/// The method that answers whether a feature has a page to open.
pub const METHOD_FEATURE_SUPPORTED: &str = "feature_supported";
/// The event pushed when any preference in `settings` changes.
pub const EVENT_SETTINGS_CHANGED: &str = "settings_changed";

/// What the `settings` object is built from: the accessibility settings plus the two states kept elsewhere in the shell.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsInput {
    pub a11y: crate::shell::accessibility::AccessibilitySettings,
    /// screen zoom (Option+Cmd+8) is magnifying the display
    pub screen_zoom: bool,
    /// Keyboard > Keyboard Navigation: Tab moves focus between every control
    pub keyboard_navigation: bool,
}

/// The names of the assistive technologies running, in the SDK's `Technology::ALL` order.
pub fn running(i: &SettingsInput) -> Vec<&'static str> {
    let a = &i.a11y;
    let mut v = Vec::new();
    if a.screen_reader {
        v.push("screen-reader");
    }
    if a.switch_control || a.switch_scan {
        v.push("switch-control");
    }
    if a.voice_control {
        v.push("voice-control");
    }
    if i.screen_zoom {
        v.push("zoom");
    }
    if i.keyboard_navigation {
        v.push("full-keyboard-access");
    }
    v
}

/// The `settings` object. Preferences the shell has no setting for are present and false, so an app reads one shape.
pub fn settings_json(i: &SettingsInput) -> Json {
    let a = &i.a11y;
    let o = DisplayOptions::from_settings(a);
    let f = |k: &str, v: bool| (k.to_string(), Json::Bool(v));
    let tech = running(i);
    // an app keeps its accessibility data complete while something reads or drives it (keyboard navigation alone does not)
    let reading = tech.iter().any(|t| *t != "full-keyboard-access");
    Json::Obj(vec![
        f("increase_contrast", o.increase_contrast),
        f("reduce_transparency", o.reduce_transparency),
        f("differentiate_without_color", o.differentiate_without_color),
        f("show_borders", a.button_shapes),
        f("invert_colors", o.invert_colors),
        f("reduce_motion", o.reduce_motion),
        f("dim_flashing_lights", o.dim_flashing_lights),
        f("animated_images", !a.reduce_motion),
        f("bold_text", a.bold_text),
        ("text_scale".to_string(), Json::Num(a.text_scale as f64)),
        ("display_zoom".to_string(), Json::Num(a.zoom_factor() as f64)),
        f("non_blinking_insertion_point", false),
        f("reduce_highlighting_effects", false),
        f("horizontal_text_layout", false),
        f("action_slider_alternative", false),
        f("assistive_access", false),
        f("live_captions", a.live_captions),
        f("application_accessibility", reading),
        ("running".to_string(), Json::Arr(tech.into_iter().map(|t| Json::Str(t.to_string())).collect())),
    ])
}

/// The Settings page a feature opens on, by its SDK name (`accessibility_settings::SettingsFeature::name`). Personal
/// Voice, call audio, pointer devices and caption styles have no page here, so they answer `None`.
pub fn feature_page(name: &str) -> Option<crate::shell::pages::PageId> {
    use crate::shell::pages::PageId;
    match name {
        "assistive-touch" => Some(PageId::Accessibility),
        "dwell-control" => Some(PageId::DwellControl),
        _ => None,
    }
}

fn feature_param(params: &Json) -> Result<&str, String> {
    match params.get("feature") {
        Some(Json::Str(s)) => Ok(s.as_str()),
        _ => Err("accessibility_display: give the feature as {\"feature\": name}".to_string()),
    }
}

fn settings_input(shell: &Shell) -> SettingsInput {
    let a11y = shell.settings.as_ref().map_or_else(|| crate::shell::accessibility::AccessibilitySettings::load(&shell.prefs), |st| st.accessibility().clone());
    SettingsInput { a11y, screen_zoom: shell.screen_zoom().on, keyboard_navigation: shell.options().tab_navigation }
}

/// Opens Settings (as a window on the desktop) on `page`.
fn open_feature_page(shell: &mut Shell, page: crate::shell::pages::PageId, t: f64) {
    if shell.desk_active() {
        shell.desk_open(crate::shell::AppId::Settings, t);
    } else {
        shell.open(crate::shell::AppId::Settings, t);
    }
    let ui = &shell.ui;
    if let Some(st) = shell.settings.as_mut() {
        let _ = st.open_page_id(ui, page, t);
    }
    shell.dirty = true;
}

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
    /// The `settings` object last seen by a tick. None until the first tick.
    last_settings: Option<Json>,
}

/// The event to push when the `settings` object is `now` and was `last` on the previous tick; same rules as [`change_event`].
pub fn settings_change_event(last: &mut Option<Json>, now: Json) -> Option<Event> {
    let before = last.replace(now.clone());
    match before {
        Some(b) if b != now => Some(Event { app: None, event: EVENT_SETTINGS_CHANGED.to_string(), body: now }),
        _ => None,
    }
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

    fn call(&mut self, shell: &mut Shell, _app_id: &str, method: &str, params: &Json, t: f64) -> Result<Json, String> {
        match method {
            METHOD_SETTINGS => Ok(settings_json(&settings_input(shell))),
            METHOD_FEATURE_SUPPORTED => Ok(Json::Bool(feature_page(feature_param(params)?).is_some())),
            METHOD_OPEN_FEATURE => {
                let name = feature_param(params)?;
                let page = feature_page(name).ok_or_else(|| format!("accessibility_display: BifrOSt has no Settings page for {name}"))?;
                open_feature_page(shell, page, t);
                Ok(Json::Bool(true))
            }
            METHOD_OPTIONS => Ok(display_options(shell).to_json()),
            METHOD_GET => Ok(Json::Bool(dim_flashing_lights(shell))),
            _ => Err(format!("accessibility_display has no method {method}")),
        }
    }

    fn tick(&mut self, shell: &mut Shell, _t: f64) -> Vec<Event> {
        let now = dim_flashing_lights(shell);
        let mut events: Vec<Event> = change_event(&mut self.last, now).into_iter().collect();
        events.extend(options_change_event(&mut self.last_options, display_options(shell)));
        events.extend(settings_change_event(&mut self.last_settings, settings_json(&settings_input(shell))));
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

    fn input() -> SettingsInput {
        SettingsInput { a11y: crate::shell::accessibility::AccessibilitySettings::DEFAULT, screen_zoom: false, keyboard_navigation: false }
    }

    #[test]
    fn settings_list_the_running_technologies_in_order() {
        let mut i = input();
        assert!(running(&i).is_empty());
        assert_eq!(settings_json(&i).get("application_accessibility"), Some(&Json::Bool(false)));
        i.a11y.voice_control = true;
        i.a11y.screen_reader = true;
        i.screen_zoom = true;
        i.keyboard_navigation = true;
        assert_eq!(running(&i), vec!["screen-reader", "voice-control", "zoom", "full-keyboard-access"]);
        let j = settings_json(&i);
        assert_eq!(j.get("application_accessibility"), Some(&Json::Bool(true)));
        let mut k = input();
        k.keyboard_navigation = true;
        assert_eq!(settings_json(&k).get("application_accessibility"), Some(&Json::Bool(false)), "keyboard navigation alone reads nothing");
    }

    #[test]
    fn settings_carry_every_key_the_sdk_reads() {
        let mut i = input();
        i.a11y.reduce_motion = true;
        i.a11y.button_shapes = true;
        i.a11y.zoom = 1;
        let j = settings_json(&i);
        assert_eq!(j.get("animated_images"), Some(&Json::Bool(false)), "Reduce Motion stops animated images");
        assert_eq!(j.get("show_borders"), Some(&Json::Bool(true)));
        assert_eq!(j.get("differentiate_without_color"), Some(&Json::Bool(true)));
        assert_eq!(j.get("display_zoom"), Some(&Json::Num(1.15f32 as f64)));
        for k in ["non_blinking_insertion_point", "reduce_highlighting_effects", "horizontal_text_layout", "action_slider_alternative", "assistive_access"] {
            assert_eq!(j.get(k), Some(&Json::Bool(false)), "{k} has no setting yet");
        }
    }

    #[test]
    fn only_features_with_a_page_open() {
        use crate::shell::pages::PageId;
        assert_eq!(feature_page("dwell-control"), Some(PageId::DwellControl));
        assert_eq!(feature_page("assistive-touch"), Some(PageId::Accessibility));
        for name in ["personal-voice-allow-apps-to-request", "allow-apps-to-add-audio-to-calls", "assistive-touch-devices", "caption-styles", "x"] {
            assert_eq!(feature_page(name), None, "{name}");
        }
        assert!(feature_param(&Json::Null).is_err());
        assert_eq!(feature_param(&Json::Obj(vec![("feature".into(), Json::Str("dwell-control".into()))])), Ok("dwell-control"));
    }

    #[test]
    fn a_settings_change_reports_the_whole_object_once() {
        let mut last = None;
        let a = settings_json(&input());
        assert_eq!(settings_change_event(&mut last, a.clone()), None);
        let mut i = input();
        i.a11y.screen_reader = true;
        let b = settings_json(&i);
        let e = settings_change_event(&mut last, b.clone()).expect("a change");
        assert_eq!((e.event.as_str(), &e.body), ("settings_changed", &b));
        assert_eq!(settings_change_event(&mut last, b), None);
    }

    #[test]
    fn the_service_is_named_as_the_sdk_expects() {
        assert_eq!(AccessibilityDisplay::default().name(), "accessibility_display");
        assert_eq!(SERVICE, "accessibility_display");
    }
}
