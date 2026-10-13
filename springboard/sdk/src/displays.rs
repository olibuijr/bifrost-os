//! Displays for apps, over the shell's `display` service (`springboard/src/shell/services/display_services.rs`): the
//! active and online lists, the main display, bounds and pixel size, modes, refresh rate, rotation, mirroring, the
//! built-in flag, vendor / model / serial, reconfiguration events and (with the person's permission) screen capture.
//! The model is `bifrost_ui::display_info`, re-exported here.
//!
//! Each call returns a request id; the answer arrives in `App::service_reply` and goes through the matching decoder.
//! Changing the mode needs the `display-config` permission in the manifest; capturing the screen needs the person to
//! allow screen recording for the app (the first call shows the sheet and fails).
//!
//! ```ignore
//! let id = displays::list(ctx, displays::Kind::Active);
//! // in App::service_reply: displays::decode_list(result)
//! // in App::service_event: displays::reconfigured(service, event, body)
//! ```
use crate::service;
use crate::Ctx;
use bifrost_core::json::Json;
pub use bifrost_ui::display_info::*;
use bifrost_ui::wire_json::*;

/// The shell service this client calls.
pub const SERVICE: &str = "display";
/// The event the shell pushes after the display list changed.
pub const RECONFIGURED: &str = "reconfigured";

/// Which displays a list holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The displays that are part of the desktop.
    Active,
    /// Every connected display.
    Online,
}

fn call(ctx: &Ctx, method: &str, params: Json) -> u32 {
    service::call(ctx, SERVICE, method, &params)
}

/// The display list. Answer: [`decode_list`].
pub fn list(ctx: &Ctx, kind: Kind) -> u32 {
    let name = if kind == Kind::Active { "active" } else { "online" };
    call(ctx, "list", obj(vec![("kind", text(name))]))
}

/// The main display. Answer: [`decode_one`].
pub fn main(ctx: &Ctx) -> u32 {
    call(ctx, "main", Json::Null)
}

/// One display by id. Answer: [`decode_one`].
pub fn info(ctx: &Ctx, id: DisplayId) -> u32 {
    call(ctx, "info", obj(vec![("id", num(id))]))
}

/// The modes of a display. Answer: [`decode_modes`].
pub fn modes(ctx: &Ctx, id: DisplayId) -> u32 {
    call(ctx, "modes", obj(vec![("id", num(id))]))
}

/// Asks for a display mode (`display-config` permission). Answer: [`decode_mode_change`].
pub fn set_mode(ctx: &Ctx, id: DisplayId, width: u32, height: u32, refresh_hz: f32) -> u32 {
    call(ctx, "set_mode", obj(vec![("id", num(id)), ("width", num(width)), ("height", num(height)), ("refresh_hz", num(refresh_hz))]))
}

/// Captures a display as a PNG (screen recording permission). Answer: [`decode_capture`].
pub fn capture(ctx: &Ctx, id: DisplayId) -> u32 {
    call(ctx, "capture", obj(vec![("id", num(id))]))
}

pub fn decode_list(result: &Result<Json, String>) -> Result<Vec<DisplayInfo>, String> {
    Ok(list_from_json(result.as_ref().map_err(|e| e.clone())?))
}

pub fn decode_one(result: &Result<Json, String>) -> Result<DisplayInfo, String> {
    DisplayInfo::from_json(result.as_ref().map_err(|e| e.clone())?).ok_or_else(|| "displays: unreadable display".to_string())
}

pub fn decode_modes(result: &Result<Json, String>) -> Result<Vec<DisplayMode>, String> {
    let body = result.as_ref().map_err(|e| e.clone())?;
    Ok(body.as_array().map(|a| a.iter().filter_map(DisplayMode::from_json).collect()).unwrap_or_default())
}

/// The answer to [`set_mode`]: `(applied, pending)`. A mode other than the current one is pending until the compositor
/// applies it.
pub fn decode_mode_change(result: &Result<Json, String>) -> Result<(bool, bool), String> {
    let body = result.as_ref().map_err(|e| e.clone())?;
    Ok((bool_of(body, "applied").unwrap_or(false), bool_of(body, "pending").unwrap_or(false)))
}

/// A captured display: its size and the PNG bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Capture {
    pub width: u32,
    pub height: u32,
    pub png: Vec<u8>,
}

pub fn decode_capture(result: &Result<Json, String>) -> Result<Capture, String> {
    let body = result.as_ref().map_err(|e| e.clone())?;
    let png = base64_decode(&str_of(body, "png").ok_or("displays: the capture has no image")?).ok_or("displays: the capture is not base64")?;
    Ok(Capture { width: u32_of(body, "width").unwrap_or(0), height: u32_of(body, "height").unwrap_or(0), png })
}

/// The reconfiguration behind an event: the per-display changes and the new list. `None` for any other event.
pub fn reconfigured(service: &str, event: &str, body: &Json) -> Option<(Vec<(DisplayId, ChangeFlags)>, Vec<DisplayInfo>)> {
    if service != SERVICE || event != RECONFIGURED {
        return None;
    }
    let changes = body
        .get("changes")?
        .as_array()?
        .iter()
        .filter_map(|c| Some((u32_of(c, "id")?, ChangeFlags(u32_of(c, "flags")?))))
        .collect();
    Some((changes, list_from_json(body.get("displays")?)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::Rect;

    fn display() -> DisplayInfo {
        DisplayInfo {
            id: 1,
            name: "Display".into(),
            is_main: true,
            is_builtin: true,
            is_active: true,
            is_online: true,
            is_asleep: false,
            bounds: Rect { x: 0.0, y: 0.0, w: 1440.0, h: 900.0 },
            pixel_width: 2880,
            pixel_height: 1800,
            mode: DisplayMode { width: 1440, height: 900, pixel_width: 2880, pixel_height: 1800, refresh_hz: 60.0, usable: true },
            modes: vec![],
            rotation: Rotation::Deg0,
            mirror_of: None,
            vendor: 0,
            model: 0,
            serial: 0,
            size_mm: (0.0, 0.0),
        }
    }

    #[test]
    fn a_list_and_one_display_decode() {
        let list = Ok(list_to_json(&[display()]));
        assert_eq!(decode_list(&list).unwrap(), vec![display()]);
        assert_eq!(decode_one(&Ok(display().to_json())).unwrap(), display());
        assert!(decode_one(&Ok(Json::Null)).is_err());
        assert!(decode_list(&Err("no".into())).is_err());
    }

    #[test]
    fn modes_and_mode_changes_decode() {
        let m = Ok(Json::Arr(vec![display().mode.to_json()]));
        assert_eq!(decode_modes(&m).unwrap(), vec![display().mode]);
        let change = Ok(obj(vec![("applied", Json::Bool(false)), ("pending", Json::Bool(true))]));
        assert_eq!(decode_mode_change(&change), Ok((false, true)));
    }

    #[test]
    fn a_capture_decodes_its_png() {
        let body = Ok(obj(vec![("width", num(2u32)), ("height", num(1u32)), ("png", text(&base64_encode(&[137, 80, 78, 71])))]));
        let c = decode_capture(&body).unwrap();
        assert_eq!((c.width, c.height, c.png), (2, 1, vec![137, 80, 78, 71]));
        assert!(decode_capture(&Ok(obj(vec![("png", text("!!"))]))).is_err());
    }

    #[test]
    fn a_reconfiguration_event_decodes_and_others_do_not() {
        let body = obj(vec![
            ("changes", Json::Arr(vec![obj(vec![("id", num(1u32)), ("flags", num(ChangeFlags::SET_MODE))])])),
            ("displays", list_to_json(&[display()])),
        ]);
        let (changes, displays) = reconfigured(SERVICE, RECONFIGURED, &body).unwrap();
        assert!(changes[0].1.contains(ChangeFlags::SET_MODE));
        assert_eq!(displays.len(), 1);
        assert!(reconfigured(SERVICE, "other", &body).is_none());
        assert!(reconfigured("screens", RECONFIGURED, &body).is_none());
    }
}
