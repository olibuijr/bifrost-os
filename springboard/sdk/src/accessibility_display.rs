//! The Dim Flashing Lights setting, for apps, over the shell's `accessibility_display` service
//! (`springboard/src/shell/services/accessibility_display.rs`).
//!
//! ```ignore
//! // when the app starts:
//! let id = accessibility_display::get(ctx);
//! // in App::service_reply: accessibility_display::from_reply(result) gives Ok(true) when the user wants dimming
//! // in App::service_event: accessibility_display::changed(service, event, body) gives the new flag after a change
//! // to analyse a picture: flashing_lights(dim) is the bifrost_ui model, dimming only when the setting is on
//! ```
//! The shell does not read app pictures, so the app feeds the luminance samples to the model and draws with
//! `FlashingLights::output`. The model is `bifrost_ui::flashing_lights`, re-exported here.
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;
pub use bifrost_ui::flashing_lights::{FlashingLights, Sample, DIM_FACTOR, MAX_FLASHES_PER_SECOND, SWING};

/// The service name, as the shell registers it.
pub const SERVICE: &str = "accessibility_display";
/// The method that returns the setting.
pub const METHOD_GET: &str = "get";
/// The event the shell pushes when the setting changes. Its body is the new flag.
pub const EVENT_CHANGED: &str = "changed";

/// The method that returns every display accommodation.
pub const METHOD_OPTIONS: &str = "options";
/// The event the shell pushes when any display accommodation changes. Its body is the `options` object.
pub const EVENT_OPTIONS_CHANGED: &str = "options_changed";

/// The display accommodations, named after the Workspace accessibility display flags (`accessibilityDisplayShouldIncreaseContrast`, ...).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayOptions {
    pub increase_contrast: bool,
    pub reduce_transparency: bool,
    pub differentiate_without_color: bool,
    pub invert_colors: bool,
    pub reduce_motion: bool,
    pub dim_flashing_lights: bool,
}

impl DisplayOptions {
    /// Decodes the `options` reply or `options_changed` body. A missing key reads as off; a non-object is an error.
    pub fn from_json(v: &Json) -> Result<Self, String> {
        if !matches!(v, Json::Obj(_)) {
            return Err("accessibility_display: the options are not an object".to_string());
        }
        let flag = |k: &str| matches!(v.get(k), Some(Json::Bool(true)));
        Ok(DisplayOptions {
            increase_contrast: flag("increase_contrast"),
            reduce_transparency: flag("reduce_transparency"),
            differentiate_without_color: flag("differentiate_without_color"),
            invert_colors: flag("invert_colors"),
            reduce_motion: flag("reduce_motion"),
            dim_flashing_lights: flag("dim_flashing_lights"),
        })
    }
}

/// Asks the shell for every display accommodation. Returns the request id the reply carries.
pub fn options(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, METHOD_OPTIONS, &Json::Null)
}

/// Decodes an `options` reply.
pub fn options_from_reply(result: &Result<Json, String>) -> Result<DisplayOptions, String> {
    match result {
        Ok(body) => DisplayOptions::from_json(body),
        Err(e) => Err(e.clone()),
    }
}

/// If `service`/`event` is the options change event, decodes its body. None for any other event.
pub fn options_changed(service_name: &str, event: &str, body: &Json) -> Option<Result<DisplayOptions, String>> {
    (service_name == SERVICE && event == EVENT_OPTIONS_CHANGED).then(|| DisplayOptions::from_json(body))
}

/// Asks the shell for the Dim Flashing Lights setting. Returns the request id the reply carries.
pub fn get(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, METHOD_GET, &Json::Null)
}

/// Decodes a `get` reply (`App::service_reply` or `service::take_reply`). Errors are the shell's message.
pub fn from_reply(result: &Result<Json, String>) -> Result<bool, String> {
    match result {
        Ok(body) => from_json(body),
        Err(e) => Err(e.clone()),
    }
}

/// If `service`/`event` is the setting's change event, decodes its body into the new flag. None for any other event.
pub fn changed(service_name: &str, event: &str, body: &Json) -> Option<Result<bool, String>> {
    (service_name == SERVICE && event == EVENT_CHANGED).then(|| from_json(body))
}

/// A flashing-lights model with dimming set to the user's setting, ready to analyse a picture.
pub fn flashing_lights(dim_on: bool) -> FlashingLights {
    let mut model = FlashingLights::new();
    model.set_dimming_enabled(dim_on);
    model
}

fn from_json(v: &Json) -> Result<bool, String> {
    match v {
        Json::Bool(b) => Ok(*b),
        _ => Err("accessibility_display: the flag is not true or false".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_is_the_flag() {
        assert_eq!(from_reply(&Ok(Json::Bool(true))), Ok(true));
        assert_eq!(from_reply(&Ok(Json::Bool(false))), Ok(false));
        assert!(from_reply(&Ok(Json::Null)).is_err());
        assert_eq!(from_reply(&Err("no service".to_string())), Err("no service".to_string()));
    }

    #[test]
    fn options_decode_every_flag() {
        let body = Json::Obj(vec![("reduce_motion".to_string(), Json::Bool(true)), ("invert_colors".to_string(), Json::Bool(true))]);
        let o = DisplayOptions::from_json(&body).unwrap();
        assert!(o.reduce_motion && o.invert_colors);
        assert!(!o.increase_contrast && !o.reduce_transparency && !o.differentiate_without_color && !o.dim_flashing_lights);
        assert!(DisplayOptions::from_json(&Json::Null).is_err());
        assert_eq!(options_from_reply(&Err("x".to_string())), Err("x".to_string()));
    }

    #[test]
    fn only_the_options_event_is_decoded_as_options() {
        let body = Json::Obj(vec![]);
        assert!(options_changed(SERVICE, EVENT_OPTIONS_CHANGED, &body).is_some());
        assert!(options_changed(SERVICE, EVENT_CHANGED, &body).is_none());
        assert!(options_changed("screens", EVENT_OPTIONS_CHANGED, &body).is_none());
    }

    #[test]
    fn only_this_services_change_event_is_decoded() {
        assert_eq!(changed(SERVICE, EVENT_CHANGED, &Json::Bool(true)), Some(Ok(true)));
        assert_eq!(changed("screens", EVENT_CHANGED, &Json::Bool(true)), None);
        assert_eq!(changed(SERVICE, "other", &Json::Bool(true)), None);
    }

    #[test]
    fn the_model_dims_only_when_the_setting_is_on() {
        // a 10 Hz square wave is flagged (more than 3 flashes in a second): 0.8 dims to 0.8 * 0.6 = 0.48
        let mut on = flashing_lights(true);
        let mut samples = Vec::new();
        for i in 0..20u64 {
            samples.push(Sample { time_ms: i * 100, luminance: if i % 2 == 0 { 1.0 } else { 0.0 } });
        }
        assert!(on.analyse(&samples));
        assert!((on.output(0.8) - 0.48).abs() < 1e-9);
        let mut off = flashing_lights(false);
        assert!(off.analyse(&samples));
        assert_eq!(off.output(0.8), 0.8);
    }
}
