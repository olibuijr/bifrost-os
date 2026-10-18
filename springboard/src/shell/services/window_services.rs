//! The `windows` service: the windows on the desktop, from the shell's real window state. In desktop mode that is the
//! menu bar, the Dock, every app window front to back and the desktop itself; on the home screen it is the one app in
//! front. `bifrost_ui::window_list` holds the record, the list options and the levels; this file fills the records.
//!
//! Window numbers: 1 is the desktop, 2 the menu bar, 3 the Dock, 100 and up the app windows (the app's index in the app
//! list plus 100) and 10000 and up the windows of other desktop programs. The bounds are in points. Native windows of
//! other desktop programs are listed with the title and owner the compositor reports; their bounds are not known to the
//! shell and read as empty.
//!
//! Methods (JSON parameters):
//! - `list {options}` answers the records the options select (`scope`: `all`, `on_screen`, `above` or `below` with a
//!   `reference` window number; `exclude_desktop`), front first.
//! - `info {number}` answers one record.
//! - `level {key}` answers `{level}` for a level key name; `levels {}` answers every name with its level.
//! - `image {number, options}` answers `{width, height, png}` (PNG as base64) for the window. It needs screen
//!   recording authorized for the app, like display capture.
//!
//! A window title is only given to an app that may record the screen, or for the app's own windows.
use super::display_services::require_screen_recording;
use super::privacy_permissions;
use super::{app_pid, Shell, SystemService};
use crate::shell::{apps, desk, AppId, Screen};
use bifrost_core::json::Json;
use bifrost_ui::capture_access::{Media, Status};
use bifrost_ui::wire_json::*;
use bifrost_ui::window_list::*;
use bifrost_ui::Rect;

pub const SERVICE: &str = "windows";
pub const DESKTOP: u32 = 1;
pub const MENU_BAR: u32 = 2;
pub const DOCK: u32 = 3;
pub const APP_BASE: u32 = 100;
pub const NATIVE_BASE: u32 = 10_000;

pub struct WindowServices;

fn shell_record(number: u32, name: &str, kind: ShellWindowKind, bounds: Rect, pid: u32) -> WindowInfo {
    WindowInfo {
        number,
        owner_pid: pid,
        owner_name: "Shell".into(),
        name: Some(name.into()),
        layer: level_of_shell_window(kind),
        bounds,
        alpha: 1.0,
        on_screen: true,
        sharing: SharingState::None,
    }
}

fn app_record(shell: &Shell, app: AppId, bounds: Rect, on_screen: bool) -> Option<WindowInfo> {
    let idx = apps().iter().position(|a| a.id == app)?;
    let pid = match app.parent() {
        AppId::External(slot) => shell.ext.get(slot as usize).and_then(|h| h.as_ref()).and_then(|h| h.pid()),
        _ => None,
    }
    .unwrap_or_else(std::process::id);
    Some(WindowInfo {
        number: APP_BASE + idx as u32,
        owner_pid: pid,
        owner_name: apps()[idx].name.to_string(),
        name: Some(apps()[idx].name.to_string()),
        layer: level_of_shell_window(ShellWindowKind::App),
        bounds,
        alpha: 1.0,
        on_screen,
        sharing: SharingState::ReadOnly,
    })
}

fn pts(pt: f32, x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rect { x: x as f32 / pt, y: y as f32 / pt, w: w as f32 / pt, h: h as f32 / pt }
}

/// The app that owns window `number`, for the numbers in the app range.
fn app_of_number(number: u32) -> Option<AppId> {
    apps().get(number.checked_sub(APP_BASE)? as usize).map(|a| a.id)
}

/// Every window record, front to back, titles included.
pub fn records(shell: &Shell) -> Vec<WindowInfo> {
    let pt = shell.ui.pt(1.0).max(0.01);
    let me = std::process::id();
    let mut out = Vec::new();
    match shell.desk.as_ref() {
        Some(d) => {
            let m = shell.desk_metrics();
            out.push(shell_record(MENU_BAR, "Menu Bar", ShellWindowKind::MenuBar, pts(pt, 0, 0, shell.w, m.menubar), me));
            if shell.dock_is_visible() {
                let r = shell.dock_overlay_rect();
                out.push(shell_record(DOCK, "Dock", ShellWindowKind::Dock, pts(pt, r.x, r.y, r.w, r.h), me));
            }
            for w in d.wins.iter().rev() {
                if let Some(rec) = app_record(shell, w.app, pts(pt, w.r.x, w.r.y, w.r.w, w.r.h), !w.minimized) {
                    out.push(rec);
                }
            }
            #[cfg(feature = "desktop-wayland")]
            for f in &d.native_windows {
                out.push(WindowInfo {
                    number: NATIVE_BASE + f.id,
                    owner_pid: 0,
                    owner_name: f.app_id.clone(),
                    name: Some(f.title.clone()),
                    layer: level_of_shell_window(ShellWindowKind::App),
                    bounds: Rect::default(),
                    alpha: 1.0,
                    on_screen: true,
                    sharing: SharingState::None,
                });
            }
            out.push(shell_record(DESKTOP, "Desktop", ShellWindowKind::Desktop, pts(pt, 0, 0, shell.w, shell.h), me));
        }
        None => {
            if let Screen::App(app) = shell.screen {
                if let Some(rec) = app_record(shell, app, pts(pt, 0, 0, shell.w, shell.h), true) {
                    out.push(rec);
                }
            }
        }
    }
    out
}

/// Whether the app may read the titles of other apps' windows: screen recording is authorized for it.
fn may_read_names(app_id: &str) -> bool {
    privacy_permissions::access(app_id).status(Media::Screen) == Status::Authorized
}

fn visible_to(shell: &Shell, app_id: &str, mut list: Vec<WindowInfo>) -> Vec<WindowInfo> {
    if !may_read_names(app_id) {
        redact_names(&mut list, app_pid(shell, app_id).unwrap_or(u32::MAX));
    }
    list
}

/// The part of a `w` x `h` pixel image inside the rectangle (`x`, `y`, `cw`, `ch`), clamped to the image.
pub fn crop(px: &[u32], w: usize, h: usize, x: i64, y: i64, cw: i64, ch: i64) -> (usize, usize, Vec<u32>) {
    let x0 = x.clamp(0, w as i64) as usize;
    let y0 = y.clamp(0, h as i64) as usize;
    let x1 = (x + cw).clamp(0, w as i64) as usize;
    let y1 = (y + ch).clamp(0, h as i64) as usize;
    let (nw, nh) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
    let mut out = Vec::with_capacity(nw * nh);
    for row in y0..y0 + nh {
        out.extend_from_slice(&px[row * w + x0..row * w + x0 + nw]);
    }
    (nw, nh, out)
}

fn window_image(shell: &mut Shell, number: u32, options: ImageOptions, t: f64) -> Result<(usize, usize, Vec<u32>), String> {
    let rec = records(shell).into_iter().find(|r| r.number == number).ok_or_else(|| format!("windows: no window {number}"))?;
    if !rec.on_screen {
        return Err("windows: the window is not on screen".into());
    }
    let pt = shell.ui.pt(1.0).max(0.01);
    let m = shell.desk_metrics();
    let app = app_of_number(number);
    let title = match app {
        Some(a) if options.ignore_framing => desk::title_of(a, &m) as i64,
        _ => 0,
    };
    let (x, y) = ((rec.bounds.x * pt).round() as i64, (rec.bounds.y * pt).round() as i64);
    let (w, h) = ((rec.bounds.w * pt).round() as i64, (rec.bounds.h * pt).round() as i64);
    // the app's own last frame when the compositor path keeps one, else the window's area of the screen
    let own = app.and_then(|a| shell.desk.as_ref().and_then(|d| d.window_pixels(a)));
    let (iw, ih, mut px) = match own {
        Some((pw, ph, pixels)) => {
            let skip = if (ph as i64) >= h { title } else { 0 };
            crop(&pixels, pw, ph, 0, skip, pw as i64, ph as i64 - skip)
        }
        None => {
            let frame = shell.capture_frame(t);
            crop(&frame.px, frame.w, frame.h, x, y + title, w, h - title)
        }
    };
    if iw == 0 || ih == 0 {
        return Err("windows: the window has no visible area".into());
    }
    if options.opaque {
        make_opaque(&mut px);
    }
    Ok((iw, ih, px))
}

impl SystemService for WindowServices {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, t: f64) -> Result<Json, String> {
        match method {
            "list" => {
                let options = match params.get("options") {
                    Some(j) => WindowListOptions::from_json(j).ok_or("windows: unknown list scope")?,
                    None => WindowListOptions::all(),
                };
                let list = visible_to(shell, app_id, window_list(&records(shell), options));
                Ok(list_to_json(&list))
            }
            "info" => {
                let number = u32_of(params, "number").ok_or("info needs a number")?;
                let list = visible_to(shell, app_id, records(shell));
                list.into_iter().find(|w| w.number == number).map(|w| w.to_json()).ok_or_else(|| format!("windows: no window {number}"))
            }
            "level" => {
                let key = str_of(params, "key").and_then(|k| WindowLevelKey::from_name(&k)).ok_or("level needs a known key")?;
                Ok(obj(vec![("level", num(window_level(key)))]))
            }
            "levels" => Ok(Json::Obj(WindowLevelKey::ALL.iter().map(|k| (k.name().to_string(), num(k.level()))).collect())),
            "image" => {
                let number = u32_of(params, "number").ok_or("image needs a number")?;
                let options = params.get("options").map(ImageOptions::from_json).unwrap_or_default();
                require_screen_recording(app_id)?;
                let (w, h, px) = window_image(shell, number, options, t)?;
                let png = crate::pngenc::encode_rgb(w, h, &px);
                Ok(obj(vec![("width", num(w as f64)), ("height", num(h as f64)), ("png", text(&base64_encode(&png)))]))
            }
            _ => Err(format!("windows has no method {method}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crop_is_clamped_to_the_image() {
        let px: Vec<u32> = (0..12).collect(); // 4 x 3
        assert_eq!(crop(&px, 4, 3, 1, 1, 2, 2), (2, 2, vec![5, 6, 9, 10]));
        assert_eq!(crop(&px, 4, 3, -2, -2, 4, 4), (2, 2, vec![0, 1, 4, 5]));
        assert_eq!(crop(&px, 4, 3, 3, 2, 10, 10), (1, 1, vec![11]));
        assert_eq!(crop(&px, 4, 3, 9, 9, 2, 2).0, 0);
        assert_eq!(crop(&px, 4, 3, 0, 0, -1, 5).2.len(), 0);
    }

    #[test]
    fn app_numbers_start_at_the_app_base() {
        assert!(app_of_number(DESKTOP).is_none());
        assert!(app_of_number(DOCK).is_none());
        assert_eq!(app_of_number(APP_BASE), apps().first().map(|a| a.id));
        assert!(app_of_number(APP_BASE + 100_000).is_none());
    }

    #[test]
    fn points_divide_pixels_by_the_scale() {
        assert_eq!(pts(2.0, 20, 40, 200, 100), Rect { x: 10.0, y: 20.0, w: 100.0, h: 50.0 });
    }

    #[test]
    fn a_shell_record_sits_on_its_level() {
        let r = shell_record(DOCK, "Dock", ShellWindowKind::Dock, Rect::default(), 7);
        assert_eq!(r.layer, window_level(WindowLevelKey::Dock));
        assert_eq!(r.owner_pid, 7);
        assert_eq!(r.sharing, SharingState::None);
    }
}
