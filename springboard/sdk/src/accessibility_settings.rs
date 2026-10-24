//! Every accessibility preference an app may read, in one struct, plus a way to open the Settings page of one feature.
//! Over the shell's `accessibility_display` service (`springboard/src/shell/services/accessibility_display.rs`).
//!
//! ```ignore
//! // when the app starts:
//! let id = accessibility_settings::get(ctx);
//! // in App::service_reply: accessibility_settings::from_reply(result) gives the Settings
//! // in App::service_event: accessibility_settings::changed(service, event, body) gives the new Settings after a change
//! if settings.is_running(Technology::ScreenReader) { /* describe the chart in words */ }
//! // a button that sends the user to the Dwell Control page:
//! accessibility_settings::open_feature(ctx, SettingsFeature::DwellControl);
//! ```
//!
//! The reference framework answers these as free functions (one per preference, each with its own change notification),
//! lists the running assistive technologies, and opens a settings feature with a completion that carries an error. Here one
//! `settings` call answers them all and one `settings_changed` event follows any change; `open_feature` answers whether the
//! page opened. Preferences the shell has no setting for read as their neutral value and are listed in `UNSUPPORTED`.
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;

/// The service name, as the shell registers it.
pub const SERVICE: &str = "accessibility_display";
/// The method that returns every preference as one object.
pub const METHOD_SETTINGS: &str = "settings";
/// The method that opens the Settings page of a feature. Params: `{"feature": <name>}`.
pub const METHOD_OPEN_FEATURE: &str = "open_feature";
/// The method that answers whether a feature has a page to open. Params: `{"feature": <name>}`.
pub const METHOD_FEATURE_SUPPORTED: &str = "feature_supported";
/// The event the shell pushes when any preference changes. Its body is the `settings` object.
pub const EVENT_SETTINGS_CHANGED: &str = "settings_changed";

/// The preferences the shell has no setting for yet; they always read as the value in [`Settings::default`].
pub const UNSUPPORTED: &[&str] = &["non_blinking_insertion_point", "reduce_highlighting_effects", "horizontal_text_layout", "action_slider_alternative", "assistive_access"];

/// An assistive technology that may be running (the reference's technology constants, neutral names).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Technology {
    /// the screen reader (Settings > Accessibility > Screen Reader)
    ScreenReader,
    SwitchControl,
    VoiceControl,
    /// screen zoom (Option+Cmd+8), not Display Zoom
    Zoom,
    /// Speak Selection and the other spoken-content features
    SpeakScreen,
    /// the keyboard drives every control (Keyboard > Keyboard Navigation)
    FullKeyboardAccess,
    /// text under the pointer shown or spoken larger
    HoverText,
    /// an automation client (an agent driving the element map)
    Automation,
}

impl Technology {
    pub const ALL: [Technology; 8] = [
        Technology::ScreenReader,
        Technology::SwitchControl,
        Technology::VoiceControl,
        Technology::Zoom,
        Technology::SpeakScreen,
        Technology::FullKeyboardAccess,
        Technology::HoverText,
        Technology::Automation,
    ];

    /// The name on the wire.
    pub fn name(self) -> &'static str {
        match self {
            Technology::ScreenReader => "screen-reader",
            Technology::SwitchControl => "switch-control",
            Technology::VoiceControl => "voice-control",
            Technology::Zoom => "zoom",
            Technology::SpeakScreen => "speak-screen",
            Technology::FullKeyboardAccess => "full-keyboard-access",
            Technology::HoverText => "hover-text",
            Technology::Automation => "automation",
        }
    }

    pub fn from_name(s: &str) -> Option<Technology> {
        Technology::ALL.into_iter().find(|t| t.name() == s)
    }
}

/// A feature whose Settings page an app may open. The discriminants are the reference's raw values (measured from its
/// bridge metadata, see docs/bifrost-ui.md "Accessibility settings for apps").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsFeature {
    PersonalVoiceAllowAppsToRequestToUse = 1,
    AllowAppsToAddAudioToCalls = 2,
    AssistiveTouch = 3,
    AssistiveTouchDevices = 4,
    DwellControl = 5,
    CaptionStyles = 6,
}

impl SettingsFeature {
    pub const ALL: [SettingsFeature; 6] = [
        SettingsFeature::PersonalVoiceAllowAppsToRequestToUse,
        SettingsFeature::AllowAppsToAddAudioToCalls,
        SettingsFeature::AssistiveTouch,
        SettingsFeature::AssistiveTouchDevices,
        SettingsFeature::DwellControl,
        SettingsFeature::CaptionStyles,
    ];

    /// The name on the wire.
    pub fn name(self) -> &'static str {
        match self {
            SettingsFeature::PersonalVoiceAllowAppsToRequestToUse => "personal-voice-allow-apps-to-request",
            SettingsFeature::AllowAppsToAddAudioToCalls => "allow-apps-to-add-audio-to-calls",
            SettingsFeature::AssistiveTouch => "assistive-touch",
            SettingsFeature::AssistiveTouchDevices => "assistive-touch-devices",
            SettingsFeature::DwellControl => "dwell-control",
            SettingsFeature::CaptionStyles => "caption-styles",
        }
    }

    pub fn from_name(s: &str) -> Option<SettingsFeature> {
        SettingsFeature::ALL.into_iter().find(|f| f.name() == s)
    }

    pub fn raw(self) -> i64 {
        self as i64
    }
}

/// Every accessibility preference an app can read.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub increase_contrast: bool,
    pub reduce_transparency: bool,
    /// Button Shapes: controls are told apart by shape, not colour alone
    pub differentiate_without_color: bool,
    /// controls draw a visible border (the shell's Button Shapes setting answers it too)
    pub show_borders: bool,
    pub invert_colors: bool,
    pub reduce_motion: bool,
    pub dim_flashing_lights: bool,
    /// animated images play; the shell has no setting to stop them, so this is on unless Reduce Motion is on
    pub animated_images: bool,
    pub bold_text: bool,
    /// the user's text size as a factor of the default (1.0)
    pub text_scale: f32,
    /// Display Zoom: the geometry factor apps receive (1.0 Standard, 1.15 Larger)
    pub display_zoom: f32,
    /// the caret does not blink (not settable yet, see `UNSUPPORTED`)
    pub non_blinking_insertion_point: bool,
    pub reduce_highlighting_effects: bool,
    pub horizontal_text_layout: bool,
    pub action_slider_alternative: bool,
    pub assistive_access: bool,
    /// Live Captions is on
    pub live_captions: bool,
    /// some assistive technology reads or drives apps, so they should keep their accessibility data complete
    pub application_accessibility: bool,
    /// the assistive technologies running now, in `Technology::ALL` order
    pub running: Vec<Technology>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            increase_contrast: false,
            reduce_transparency: false,
            differentiate_without_color: false,
            show_borders: false,
            invert_colors: false,
            reduce_motion: false,
            dim_flashing_lights: false,
            animated_images: true,
            bold_text: false,
            text_scale: 1.0,
            display_zoom: 1.0,
            non_blinking_insertion_point: false,
            reduce_highlighting_effects: false,
            horizontal_text_layout: false,
            action_slider_alternative: false,
            assistive_access: false,
            live_captions: false,
            application_accessibility: false,
            running: Vec::new(),
        }
    }
}

impl Settings {
    /// Whether `t` is running.
    pub fn is_running(&self, t: Technology) -> bool {
        self.running.contains(&t)
    }

    /// Decodes the `settings` reply or `settings_changed` body. A missing key keeps its default; unknown technology
    /// names are skipped; a non-object is an error.
    pub fn from_json(v: &Json) -> Result<Self, String> {
        if !matches!(v, Json::Obj(_)) {
            return Err("accessibility_display: the settings are not an object".to_string());
        }
        let d = Settings::default();
        let flag = |k: &str, def: bool| match v.get(k) {
            Some(Json::Bool(b)) => *b,
            _ => def,
        };
        let num = |k: &str, def: f32| match v.get(k) {
            Some(Json::Num(n)) if n.is_finite() && *n > 0.0 => *n as f32,
            _ => def,
        };
        let mut running = Vec::new();
        if let Some(Json::Arr(list)) = v.get("running") {
            for t in list {
                if let Json::Str(s) = t {
                    if let Some(t) = Technology::from_name(s).filter(|t| !running.contains(t)) {
                        running.push(t);
                    }
                }
            }
        }
        running.sort_by_key(|t| Technology::ALL.iter().position(|a| a == t));
        Ok(Settings {
            increase_contrast: flag("increase_contrast", d.increase_contrast),
            reduce_transparency: flag("reduce_transparency", d.reduce_transparency),
            differentiate_without_color: flag("differentiate_without_color", d.differentiate_without_color),
            show_borders: flag("show_borders", d.show_borders),
            invert_colors: flag("invert_colors", d.invert_colors),
            reduce_motion: flag("reduce_motion", d.reduce_motion),
            dim_flashing_lights: flag("dim_flashing_lights", d.dim_flashing_lights),
            animated_images: flag("animated_images", d.animated_images),
            bold_text: flag("bold_text", d.bold_text),
            text_scale: num("text_scale", d.text_scale),
            display_zoom: num("display_zoom", d.display_zoom),
            non_blinking_insertion_point: flag("non_blinking_insertion_point", d.non_blinking_insertion_point),
            reduce_highlighting_effects: flag("reduce_highlighting_effects", d.reduce_highlighting_effects),
            horizontal_text_layout: flag("horizontal_text_layout", d.horizontal_text_layout),
            action_slider_alternative: flag("action_slider_alternative", d.action_slider_alternative),
            assistive_access: flag("assistive_access", d.assistive_access),
            live_captions: flag("live_captions", d.live_captions),
            application_accessibility: flag("application_accessibility", d.application_accessibility),
            running,
        })
    }
}

/// Asks the shell for every preference. Returns the request id the reply carries.
pub fn get(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, METHOD_SETTINGS, &Json::Null)
}

/// Decodes a `settings` reply (`App::service_reply` or `service::take_reply`).
pub fn from_reply(result: &Result<Json, String>) -> Result<Settings, String> {
    match result {
        Ok(body) => Settings::from_json(body),
        Err(e) => Err(e.clone()),
    }
}

/// If `service`/`event` is the settings change event, decodes its body. None for any other event.
pub fn changed(service_name: &str, event: &str, body: &Json) -> Option<Result<Settings, String>> {
    (service_name == SERVICE && event == EVENT_SETTINGS_CHANGED).then(|| Settings::from_json(body))
}

fn feature_params(feature: SettingsFeature) -> Json {
    Json::Obj(vec![("feature".to_string(), Json::Str(feature.name().to_string()))])
}

/// Opens Settings on the page of `feature`. The reply ([`opened_from_reply`]) is `Ok(())` when the page opened and an
/// error naming the feature when BifrOSt has no page for it. Returns the request id.
pub fn open_feature(ctx: &Ctx, feature: SettingsFeature) -> u32 {
    service::call(ctx, SERVICE, METHOD_OPEN_FEATURE, &feature_params(feature))
}

/// Asks whether `feature` has a page [`open_feature`] can open. The reply decodes with [`supported_from_reply`].
pub fn feature_supported(ctx: &Ctx, feature: SettingsFeature) -> u32 {
    service::call(ctx, SERVICE, METHOD_FEATURE_SUPPORTED, &feature_params(feature))
}

/// Decodes an `open_feature` reply.
pub fn opened_from_reply(result: &Result<Json, String>) -> Result<(), String> {
    match result {
        Ok(Json::Bool(true)) => Ok(()),
        Ok(_) => Err("accessibility_display: the page did not open".to_string()),
        Err(e) => Err(e.clone()),
    }
}

/// Decodes a `feature_supported` reply.
pub fn supported_from_reply(result: &Result<Json, String>) -> Result<bool, String> {
    match result {
        Ok(Json::Bool(b)) => Ok(*b),
        Ok(_) => Err("accessibility_display: the answer is not true or false".to_string()),
        Err(e) => Err(e.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_key_keeps_its_default() {
        let s = Settings::from_json(&Json::Obj(vec![])).unwrap();
        assert_eq!(s, Settings::default());
        assert!(s.animated_images, "animated images play by default");
        assert!(Settings::from_json(&Json::Null).is_err());
    }

    #[test]
    fn settings_decode_flags_numbers_and_the_running_technologies() {
        let body = Json::Obj(vec![
            ("reduce_motion".into(), Json::Bool(true)),
            ("animated_images".into(), Json::Bool(false)),
            ("text_scale".into(), Json::Num(1.3)),
            ("display_zoom".into(), Json::Num(-2.0)),
            ("running".into(), Json::Arr(vec![Json::Str("zoom".into()), Json::Str("screen-reader".into()), Json::Str("telepathy".into()), Json::Str("zoom".into())])),
        ]);
        let s = Settings::from_json(&body).unwrap();
        assert!(s.reduce_motion && !s.animated_images);
        assert!((s.text_scale - 1.3).abs() < 1e-6);
        assert_eq!(s.display_zoom, 1.0, "a nonsense factor keeps the default");
        assert_eq!(s.running, vec![Technology::ScreenReader, Technology::Zoom], "known names, once each, in ALL order");
        assert!(s.is_running(Technology::Zoom) && !s.is_running(Technology::VoiceControl));
    }

    #[test]
    fn feature_raw_values_and_names_round_trip() {
        let raws: Vec<i64> = SettingsFeature::ALL.iter().map(|f| f.raw()).collect();
        assert_eq!(raws, vec![1, 2, 3, 4, 5, 6]);
        for f in SettingsFeature::ALL {
            assert_eq!(SettingsFeature::from_name(f.name()), Some(f));
        }
        for t in Technology::ALL {
            assert_eq!(Technology::from_name(t.name()), Some(t));
        }
        assert_eq!(SettingsFeature::from_name("nope"), None);
    }

    #[test]
    fn replies_and_events_decode() {
        assert_eq!(opened_from_reply(&Ok(Json::Bool(true))), Ok(()));
        assert!(opened_from_reply(&Ok(Json::Bool(false))).is_err());
        assert_eq!(opened_from_reply(&Err("no page".into())), Err("no page".into()));
        assert_eq!(supported_from_reply(&Ok(Json::Bool(false))), Ok(false));
        assert!(supported_from_reply(&Ok(Json::Null)).is_err());
        assert!(changed(SERVICE, EVENT_SETTINGS_CHANGED, &Json::Obj(vec![])).is_some());
        assert!(changed(SERVICE, "options_changed", &Json::Obj(vec![])).is_none());
        assert!(changed("screens", EVENT_SETTINGS_CHANGED, &Json::Obj(vec![])).is_none());
    }
}
