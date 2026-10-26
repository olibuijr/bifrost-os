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

/// Stacks this app's window at `level` (the reference window level: a floating window stays above the normal windows and below
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

/// Resizes this app's main window to a content size of `w` x `h` points (the reference window's set-content-size call, or
/// `setFrame:display:animate:` with the animation). The shell keeps the window's top-left corner, holds the size to the
/// content limits ([`set_content_limits`], else the manifest's `min_size` / `max_size`) and the screen's work area, and
/// animates the change like a zoom. The app then gets the new size as an ordinary configure, so it lays out from
/// `ctx.cfg` as it does after the person resizes the window. A zoomed or full-screen window keeps its frame and goes
/// back to this size when it leaves that state. A size that is not finite and positive is not sent.
///
/// The window's first size comes from the manifest (`window_size = "WxH"`, the preferred initial content size); an app
/// whose size depends on its state (Calculator's keypads) sends the size it wants when that state changes.
pub fn set_content_size(ctx: &Ctx, w: f32, h: f32) {
    if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 {
        ctx.send_to_shell(crate::proto::ToShell::SetContentSize { w: w.min(crate::proto::MAX_CONTENT_PT), h: h.min(crate::proto::MAX_CONTENT_PT) });
    }
}

/// Sets this app's main window content limits in points (the reference window's content minimum and maximum sizes): the person
/// cannot resize the window outside them, and [`set_content_size`] is held to them. `None` uses the manifest's
/// `min_size` / `max_size`. The limits hold for as long as the app runs. A limit that is not finite and positive is
/// sent as `None`.
pub fn set_content_limits(ctx: &Ctx, min: Option<(f32, f32)>, max: Option<(f32, f32)>) {
    let ok = |s: Option<(f32, f32)>| {
        s.filter(|&(w, h)| w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0)
            .map(|(w, h)| (w.min(crate::proto::MAX_CONTENT_PT), h.min(crate::proto::MAX_CONTENT_PT)))
    };
    ctx.send_to_shell(crate::proto::ToShell::SetContentLimits { min: ok(min), max: ok(max) });
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
    fn content_size_requests_go_to_the_shell_and_bad_sizes_stay_home() {
        use crate::proto::ToShell;
        let ctx = crate::runtime::test_ctx();
        set_content_size(&ctx, 230.0, 510.0);
        set_content_size(&ctx, f32::NAN, 510.0);
        set_content_size(&ctx, 0.0, 510.0);
        set_content_limits(&ctx, Some((230.0, 400.0)), Some((f32::INFINITY, 900.0)));
        assert_eq!(
            ctx.take_sent(),
            vec![ToShell::SetContentSize { w: 230.0, h: 510.0 }, ToShell::SetContentLimits { min: Some((230.0, 400.0)), max: None }]
        );
    }

    #[test]
    fn the_levels_are_the_models() {
        assert!(window_level(WindowLevelKey::Floating) > window_level(WindowLevelKey::Normal));
    }
}
