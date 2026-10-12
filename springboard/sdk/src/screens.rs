//! Displays for apps, over the shell's `screens` service (`springboard/src/shell/services/screens.rs`).
//!
//! ```ignore
//! // when the app starts, and again whenever the list may have changed:
//! let id = screens::list(ctx);
//! // in App::service_reply: screens::from_reply(result) gives the Screens
//! // in App::service_event: screens::changed(service, event, body) gives the new Screens after a change
//! ```
//! The model (frames, visible frames, backing conversion, `main`, `deepest`, `containing`) is
//! `bifrost_ui::screen`, re-exported here. Refresh rates are not reported yet.
use crate::service;
use bifrost_core::json::Json;
use bifrost_ui::Rect;
pub use bifrost_ui::screen::{Align, Screen, Screens};

/// The service name, as the shell registers it.
pub const SERVICE: &str = "screens";
/// The method that returns the screen list.
pub const METHOD_LIST: &str = "list";
/// The event the shell pushes when the geometry changes. Its body is the new list.
pub const EVENT_CHANGED: &str = "changed";

/// Asks the shell for the screen list. Returns the request id the reply carries.
pub fn list(ctx: &crate::runtime::Ctx) -> u32 {
    service::call(ctx, SERVICE, METHOD_LIST, &Json::Null)
}

/// Decodes a `list` reply (`App::service_reply` or `service::take_reply`). Errors are the shell's message.
pub fn from_reply(result: &Result<Json, String>) -> Result<Screens, String> {
    match result {
        Ok(body) => from_json(body),
        Err(e) => Err(e.clone()),
    }
}

/// If `service`/`event` is the screens change event, decodes its body into the new screen list. None for any other event.
pub fn changed(service_name: &str, event: &str, body: &Json) -> Option<Result<Screens, String>> {
    (service_name == SERVICE && event == EVENT_CHANGED).then(|| from_json(body))
}

/// Decodes a screen list from its wire form. The screen flagged `main` (the first one if several are) becomes the main
/// screen; with none flagged the first screen is main, as `Screens::new` chooses.
pub fn from_json(v: &Json) -> Result<Screens, String> {
    let items = v.as_array().ok_or("screens: the list is not an array")?;
    let mut screens = Vec::with_capacity(items.len());
    let mut main = None;
    for (i, item) in items.iter().enumerate() {
        if main.is_none() && matches!(item.get("main"), Some(Json::Bool(true))) {
            main = Some(i);
        }
        screens.push(decode_screen(item)?);
    }
    let mut out = Screens::new(screens);
    if let Some(i) = main {
        out.set_main(i);
    }
    Ok(out)
}

fn decode_screen(v: &Json) -> Result<Screen, String> {
    let name = v.str_field("name").ok_or("screens: a screen has no name")?;
    let frame = decode_rect(v.get("frame"), "frame")?;
    let visible = decode_rect(v.get("visible_frame"), "visible_frame")?;
    let scale = v.get("scale").and_then(Json::number).ok_or("screens: a screen has no scale")?;
    Ok(Screen::new(&name, frame, scale as f32).with_visible_frame(visible))
}

fn decode_rect(v: Option<&Json>, what: &str) -> Result<Rect, String> {
    let v = v.ok_or_else(|| format!("screens: a screen has no {what}"))?;
    let num = |k: &str| {
        v.get(k).and_then(Json::number).map(|n| n as f32).ok_or_else(|| format!("screens: {what} has no {k}"))
    };
    Ok(Rect { x: num("x")?, y: num("y")?, w: num("w")?, h: num("h")? })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Json {
        bifrost_core::json::parse(
            r#"[{"name":"Display","frame":{"x":0,"y":0,"w":1440,"h":900},"visible_frame":{"x":0,"y":24,"w":1440,"h":816},"scale":2,"main":false},
                {"name":"Second","frame":{"x":1440,"y":0,"w":1920,"h":1080},"visible_frame":{"x":1440,"y":0,"w":1920,"h":1080},"scale":1,"main":true}]"#,
        )
        .unwrap()
    }

    #[test]
    fn a_list_decodes_frames_scale_and_the_flagged_main_screen() {
        let s = from_json(&sample()).unwrap();
        assert_eq!(s.all().len(), 2);
        assert_eq!(s.all()[0].frame, Rect { x: 0.0, y: 0.0, w: 1440.0, h: 900.0 });
        assert_eq!(s.all()[0].visible_frame, Rect { x: 0.0, y: 24.0, w: 1440.0, h: 816.0 });
        assert_eq!(s.all()[0].backing_scale, 2.0);
        assert_eq!(s.main().map(|m| m.name.as_str()), Some("Second"));
        assert_eq!(s.containing(1500.0, 10.0).map(|m| m.name.as_str()), Some("Second"));
    }

    #[test]
    fn without_a_main_flag_the_first_screen_is_main() {
        let j = bifrost_core::json::parse(r#"[{"name":"A","frame":{"x":0,"y":0,"w":10,"h":10},"visible_frame":{"x":0,"y":0,"w":10,"h":10},"scale":1}]"#).unwrap();
        assert_eq!(from_json(&j).unwrap().main().map(|m| m.name.as_str()), Some("A"));
    }

    #[test]
    fn a_visible_frame_outside_the_screen_is_clipped_to_empty() {
        let j = bifrost_core::json::parse(r#"[{"name":"A","frame":{"x":0,"y":0,"w":10,"h":10},"visible_frame":{"x":50,"y":50,"w":10,"h":10},"scale":1,"main":true}]"#).unwrap();
        assert_eq!(from_json(&j).unwrap().all()[0].visible_frame.w, 0.0);
    }

    #[test]
    fn malformed_lists_are_errors_not_panics() {
        assert!(from_json(&Json::Null).is_err());
        assert!(from_json(&bifrost_core::json::parse(r#"[{"frame":{}}]"#).unwrap()).is_err());
        assert!(from_json(&bifrost_core::json::parse(r#"[{"name":"A","frame":{"x":0,"y":0,"w":1},"visible_frame":{"x":0,"y":0,"w":1,"h":1},"scale":1}]"#).unwrap()).is_err());
    }

    #[test]
    fn a_reply_error_is_passed_on_and_an_ok_reply_decodes() {
        assert_eq!(from_reply(&Err("no service screens".into())).err().as_deref(), Some("no service screens"));
        assert_eq!(from_reply(&Ok(sample())).unwrap().all().len(), 2);
    }

    #[test]
    fn only_the_screens_change_event_is_decoded() {
        assert!(changed(SERVICE, EVENT_CHANGED, &sample()).unwrap().is_ok());
        assert!(changed("dock", EVENT_CHANGED, &sample()).is_none());
        assert!(changed(SERVICE, "other", &sample()).is_none());
    }
}
