//! The window list for apps, over the shell's `windows` service (`springboard/src/shell/services/window_services.rs`): the
//! windows on the desktop front to back with the list options (on screen only, above or below a window, without
//! desktop elements), each window's number, owner, bounds, layer, alpha, title and sharing state, the window levels by
//! key, and a window image with the person's permission. The model is `bifrost_ui::window_list`, re-exported here.
//!
//! A window title is only given for the app's own windows, unless the person allowed screen recording for the app.
//!
//! ```ignore
//! let id = windows::list(ctx, WindowListOptions::all());
//! // in App::service_reply: windows::decode_list(result)
//! ```
use crate::service;
use crate::Ctx;
use bifrost_core::json::Json;
pub use bifrost_ui::window_list::*;
use bifrost_ui::wire_json::*;

/// The stacking level of this app's window: `Normal`, or `Floating` to stay above the normal windows.
pub use crate::proto::WindowLevel as Level;

/// The shell service this client calls.
pub const SERVICE: &str = "windows";

fn call(ctx: &Ctx, method: &str, params: Json) -> u32 {
    service::call(ctx, SERVICE, method, &params)
}

/// The windows the options select. Answer: [`decode_list`].
pub fn list(ctx: &Ctx, options: WindowListOptions) -> u32 {
    call(ctx, "list", obj(vec![("options", options.to_json())]))
}

/// One window by number. Answer: [`decode_one`].
pub fn info(ctx: &Ctx, number: u32) -> u32 {
    call(ctx, "info", obj(vec![("number", num(number))]))
}

/// The image of a window as a PNG (screen recording permission). Answer: [`decode_image`].
pub fn image(ctx: &Ctx, number: u32, options: ImageOptions) -> u32 {
    call(ctx, "image", obj(vec![("number", num(number)), ("options", options.to_json())]))
}

/// Stacks this app's window at `level` (`NSWindow.level`: a floating window stays above the normal windows and below
/// menus and panels). The shell keeps the level for as long as the app runs; it is `Normal` until the app asks.
pub fn set_level(ctx: &Ctx, level: Level) {
    ctx.send_to_shell(crate::proto::ToShell::WindowLevel(level));
}

/// Stacks one of this app's extra windows (opened with `Ctx::open_window`) at `level`, so a floating window stays above
/// the normal windows while the app's main window stays where it is. The main window uses [`set_level`]. The shell
/// keeps the level for as long as the window is open.
pub fn set_window_level(ctx: &Ctx, window: crate::app_windows::WindowId, level: Level) {
    ctx.send_to_shell(crate::proto::ToShell::WindowLevelOf { id: window.0, level });
}

pub fn decode_list(result: &Result<Json, String>) -> Result<Vec<WindowInfo>, String> {
    Ok(list_from_json(result.as_ref().map_err(|e| e.clone())?))
}

pub fn decode_one(result: &Result<Json, String>) -> Result<WindowInfo, String> {
    WindowInfo::from_json(result.as_ref().map_err(|e| e.clone())?).ok_or_else(|| "windows: unreadable window record".to_string())
}

/// A captured window: its size and the PNG bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowImage {
    pub width: u32,
    pub height: u32,
    pub png: Vec<u8>,
}

pub fn decode_image(result: &Result<Json, String>) -> Result<WindowImage, String> {
    let body = result.as_ref().map_err(|e| e.clone())?;
    let png = base64_decode(&str_of(body, "png").ok_or("windows: the image is missing")?).ok_or("windows: the image is not base64")?;
    Ok(WindowImage { width: u32_of(body, "width").unwrap_or(0), height: u32_of(body, "height").unwrap_or(0), png })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::Rect;

    fn window() -> WindowInfo {
        WindowInfo {
            number: 100,
            owner_pid: 4,
            owner_name: "Notes".into(),
            name: Some("Notes".into()),
            layer: 0,
            bounds: Rect { x: 10.0, y: 20.0, w: 300.0, h: 200.0 },
            alpha: 1.0,
            on_screen: true,
            sharing: SharingState::ReadOnly,
        }
    }

    #[test]
    fn records_decode_from_the_wire() {
        assert_eq!(decode_one(&Ok(window().to_json())).unwrap(), window());
        assert_eq!(decode_list(&Ok(list_to_json(&[window()]))).unwrap(), vec![window()]);
        assert!(decode_one(&Err("no".into())).is_err());
    }

    #[test]
    fn an_image_decodes_its_png() {
        let body = Ok(obj(vec![("width", num(3u32)), ("height", num(2u32)), ("png", text(&base64_encode(&[1, 2, 3])))]));
        assert_eq!(decode_image(&body).unwrap(), WindowImage { width: 3, height: 2, png: vec![1, 2, 3] });
        assert!(decode_image(&Ok(Json::Null)).is_err());
    }

    #[test]
    fn set_level_sends_the_level_to_the_shell() {
        let ctx = crate::runtime::test_ctx();
        set_level(&ctx, Level::Floating);
        set_level(&ctx, Level::Normal);
        assert_eq!(ctx.take_sent(), vec![crate::proto::ToShell::WindowLevel(Level::Floating), crate::proto::ToShell::WindowLevel(Level::Normal)]);
    }

    #[test]
    fn set_window_level_sends_the_level_of_one_extra_window() {
        let ctx = crate::runtime::test_ctx();
        set_window_level(&ctx, crate::app_windows::WindowId(2), Level::Floating);
        assert_eq!(ctx.take_sent(), vec![crate::proto::ToShell::WindowLevelOf { id: 2, level: Level::Floating }]);
    }

    #[test]
    fn the_levels_are_the_models() {
        assert!(window_level(WindowLevelKey::Floating) > window_level(WindowLevelKey::Normal));
    }
}
