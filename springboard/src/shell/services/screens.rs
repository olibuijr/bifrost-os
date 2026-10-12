//! The `screens` service: the displays an app can read. `list` answers with one object per screen, and the
//! service pushes `changed` (with the same list) to every app when the desktop geometry changes.
//!
//! Each screen is `{"name", "frame": {x,y,w,h}, "visible_frame": {x,y,w,h}, "scale", "main"}`, in global points.
//! The frame is the framebuffer divided by the interface scale; the visible frame leaves out the menu bar and Dock
//! (desktop mode) or the status bar (home). Refresh rates are not reported: the shell does not keep the output mode
//! (gap, see `docs/parity/wire-svc-screens.md`). The geometry model is `bifrost_ui::screen`.
use super::{Event, Shell, SystemService};
use bifrost_core::json::Json;
use bifrost_ui::screen::{Screen, Screens};
use bifrost_ui::Rect;

/// The service name apps call it by, and the event name it pushes.
pub const SERVICE: &str = "screens";
pub const EVENT_CHANGED: &str = "changed";

/// The name of the one display the shell drives.
const DISPLAY_NAME: &str = "Display";

#[derive(Default)]
pub struct Displays {
    /// The geometry last reported, to tell a change from a quiet tick. None until the first tick.
    last: Option<Screens>,
}

impl SystemService for Displays {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, shell: &mut Shell, _app_id: &str, method: &str, _params: &Json, _t: f64) -> Result<Json, String> {
        match method {
            "list" => Ok(encode(&current(shell))),
            _ => Err(format!("screens has no method {method}")),
        }
    }

    fn tick(&mut self, shell: &mut Shell, _t: f64) -> Vec<Event> {
        next_event(&mut self.last, current(shell)).into_iter().collect()
    }
}

/// The displays as the shell draws them now. Desktop mode insets the visible frame by the menu bar and the Dock;
/// home by the status bar.
fn current(shell: &Shell) -> Screens {
    let (top, bottom) = if shell.desk_active() {
        let m = shell.desk_metrics();
        (m.menubar, m.dock_bottom)
    } else {
        (shell.layout.status_h, 0)
    };
    snapshot(shell.w, shell.h, shell.layout.s, top, bottom)
}

/// The one display, from the framebuffer size in pixels, the pixels-per-point scale and the bars in pixels.
pub fn snapshot(w_px: i32, h_px: i32, scale: f32, top_px: i32, bottom_px: i32) -> Screens {
    let s = if scale > 0.0 { scale } else { 1.0 };
    let frame = Rect { x: 0.0, y: 0.0, w: w_px.max(0) as f32 / s, h: h_px.max(0) as f32 / s };
    let visible = Rect {
        x: 0.0,
        y: top_px.max(0) as f32 / s,
        w: frame.w,
        h: (h_px - top_px - bottom_px).max(0) as f32 / s,
    };
    Screens::new(vec![Screen::new(DISPLAY_NAME, frame, s).with_visible_frame(visible)])
}

/// The wire form of a screen list, the reply to `list` and the body of `changed`.
pub fn encode(screens: &Screens) -> Json {
    Json::Arr(
        screens
            .all()
            .iter()
            .map(|s| {
                let is_main = screens.main().is_some_and(|m| std::ptr::eq(m, s));
                Json::Obj(vec![
                    ("name".into(), Json::Str(s.name.clone())),
                    ("frame".into(), rect_json(s.frame)),
                    ("visible_frame".into(), rect_json(s.visible_frame)),
                    ("scale".into(), Json::Num(s.backing_scale as f64)),
                    ("main".into(), Json::Bool(is_main)),
                ])
            })
            .collect(),
    )
}

fn rect_json(r: Rect) -> Json {
    Json::Obj(vec![
        ("x".into(), Json::Num(r.x as f64)),
        ("y".into(), Json::Num(r.y as f64)),
        ("w".into(), Json::Num(r.w as f64)),
        ("h".into(), Json::Num(r.h as f64)),
    ])
}

/// Records `now` as the last geometry and returns the `changed` event when it differs from the one before. The first
/// geometry seen sets the baseline and sends nothing, because apps read `list` when they start.
pub fn next_event(last: &mut Option<Screens>, now: Screens) -> Option<Event> {
    let changed = last.as_ref().is_some_and(|prev| *prev != now);
    let body = encode(&now);
    *last = Some(now);
    changed.then(|| Event { app: None, event: EVENT_CHANGED.into(), body })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(v: &Json, k: &str) -> Json {
        v.get(k).cloned().unwrap_or(Json::Null)
    }

    #[test]
    fn the_display_is_the_framebuffer_in_points_with_bars_removed() {
        // 2880x1800 px at scale 2: 1440x900 pt, a 48 px menu bar (24 pt) and a 120 px Dock zone (60 pt)
        let s = snapshot(2880, 1800, 2.0, 48, 120);
        let screen = &s.all()[0];
        assert_eq!(screen.frame, Rect { x: 0.0, y: 0.0, w: 1440.0, h: 900.0 });
        assert_eq!(screen.visible_frame, Rect { x: 0.0, y: 24.0, w: 1440.0, h: 816.0 });
        assert_eq!(screen.backing_scale, 2.0);
        assert_eq!(s.main().map(|m| m.name.as_str()), Some(DISPLAY_NAME));
    }

    #[test]
    fn a_zero_scale_reads_as_one_pixel_per_point_and_an_oversized_bar_leaves_nothing() {
        let s = snapshot(800, 600, 0.0, 700, 0);
        let screen = &s.all()[0];
        assert_eq!(screen.backing_scale, 1.0);
        assert_eq!(screen.visible_frame.h, 0.0);
    }

    #[test]
    fn the_list_encodes_frames_scale_and_the_main_flag() {
        let j = encode(&snapshot(1920, 1080, 1.0, 24, 0));
        let first = &j.as_array().unwrap()[0];
        assert_eq!(first.str_field("name").as_deref(), Some(DISPLAY_NAME));
        assert_eq!(field(first, "scale"), Json::Num(1.0));
        assert_eq!(field(first, "main"), Json::Bool(true));
        assert_eq!(field(&field(first, "frame"), "w"), Json::Num(1920.0));
        assert_eq!(field(&field(first, "visible_frame"), "y"), Json::Num(24.0));
        assert_eq!(field(&field(first, "visible_frame"), "h"), Json::Num(1056.0));
    }

    #[test]
    fn the_first_geometry_is_a_baseline_and_only_a_later_change_sends_an_event() {
        let mut last = None;
        assert!(next_event(&mut last, snapshot(800, 600, 1.0, 0, 0)).is_none());
        assert!(next_event(&mut last, snapshot(800, 600, 1.0, 0, 0)).is_none());
        let e = next_event(&mut last, snapshot(1024, 768, 1.0, 0, 0)).expect("a resize sends changed");
        assert_eq!(e.app, None);
        assert_eq!(e.event, EVENT_CHANGED);
        assert_eq!(e.body, encode(&snapshot(1024, 768, 1.0, 0, 0)));
        assert!(next_event(&mut last, snapshot(1024, 768, 1.0, 0, 0)).is_none());
    }

    #[test]
    fn a_visible_frame_change_alone_is_a_change() {
        let mut last = None;
        next_event(&mut last, snapshot(800, 600, 1.0, 0, 0));
        assert!(next_event(&mut last, snapshot(800, 600, 1.0, 24, 0)).is_some());
    }
}
