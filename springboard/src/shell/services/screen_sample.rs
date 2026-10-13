//! The `screen` service: the pixel codes of one rectangle of the display, for apps that hold the Screen
//! permission (the Color Meter). The shell draws the frame the way a screenshot does and copies out the
//! rectangle. The SDK side is `bifrost_app::screen_sample`.
//!
//! Methods (apps need the Screen permission for each):
//! - `sample({x, y, w, h})` returns `{w, h, rgb}`, the codes of the rectangle row by row;
//! - `display()` returns `{w, h}`, the size of the frame in display pixels;
//! - `pointer()` returns `{x, y}` where the pointer is in the same pixels, or null when it is off the display or
//!   no window has seen it yet.
//!
//! The pointer is recorded by the pointer code in `bare/wayland.rs` (see [`pointer_moved`] and [`pointer_left`]).

use super::privacy_permissions::access;
use super::{Shell, SystemService};
use crate::gfx::{Image, Surface};
use bifrost_core::json::Json;
use bifrost_ui::capture_access::Media;
use std::sync::Mutex;

/// Largest side of a sampled rectangle, in display pixels. Keeps one reply small.
pub const MAX_SIDE: i32 = 64;

/// The last pointer position in display pixels, `None` once the pointer left every window.
static POINTER: Mutex<Option<(i32, i32)>> = Mutex::new(None);

/// The pointer moved to `(x, y)` in display pixels (fractions are floored).
pub fn pointer_moved(x: f32, y: f32) {
    if let Ok(mut p) = POINTER.lock() {
        *p = Some((x.floor() as i32, y.floor() as i32));
    }
}

/// The pointer left the windows: `pointer()` answers null until it moves again.
pub fn pointer_left() {
    if let Ok(mut p) = POINTER.lock() {
        *p = None;
    }
}

/// The last recorded pointer position, if any.
pub fn pointer_position() -> Option<(i32, i32)> {
    POINTER.lock().ok().and_then(|p| *p)
}

pub struct ScreenSample;

impl SystemService for ScreenSample {
    fn name(&self) -> &'static str {
        "screen"
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, t: f64) -> Result<Json, String> {
        if !matches!(method, "sample" | "display" | "pointer") {
            return Err(format!("screen has no method {method}"));
        }
        if !access(app_id).can_capture(Media::Screen) {
            return Err("the Screen permission is not granted".into());
        }
        match method {
            "display" => return Ok(size_reply(shell.w, shell.h)),
            "pointer" => {
                return Ok(match pointer_position() {
                    Some((x, y)) if x >= 0 && y >= 0 && x < shell.w && y < shell.h => point_reply(x, y),
                    _ => Json::Null,
                });
            }
            _ => {}
        }
        let (x, y, w, h) = rect_of(params)?;
        if w < 1 || h < 1 || w > MAX_SIDE || h > MAX_SIDE || x < 0 || y < 0 || x + w > shell.w || y + h > shell.h {
            return Err("the sampled rectangle is outside the display or too large".into());
        }
        // The same draw a screenshot makes; the frame is not kept.
        let mut img = Image::new(shell.w as usize, shell.h as usize);
        shell.drawn = super::super::Drawn::Nothing;
        shell.draw(&mut Surface { w: img.w, h: img.h, stride: img.w, px: &mut img.px, y0: 0 }, t);
        shell.drawn = super::super::Drawn::Nothing;
        shell.dirty = true;
        let rgb = crop_rgb(&img.px, img.w, (x as usize, y as usize, w as usize, h as usize))
            .ok_or("the sampled rectangle is outside the frame")?;
        Ok(Json::Obj(vec![
            ("w".into(), Json::Num(w as f64)),
            ("h".into(), Json::Num(h as f64)),
            ("rgb".into(), Json::Arr(rgb.into_iter().map(|b| Json::Num(b as f64)).collect())),
        ]))
    }
}

/// The `display` reply: the frame size in display pixels.
pub fn size_reply(w: i32, h: i32) -> Json {
    Json::Obj(vec![("w".into(), Json::Num(w as f64)), ("h".into(), Json::Num(h as f64))])
}

/// The `pointer` reply for a position on the display.
pub fn point_reply(x: i32, y: i32) -> Json {
    Json::Obj(vec![("x".into(), Json::Num(x as f64)), ("y".into(), Json::Num(y as f64))])
}

fn rect_of(params: &Json) -> Result<(i32, i32, i32, i32), String> {
    let n = |key: &str| match params.get(key) {
        Some(Json::Num(v)) if v.fract() == 0.0 => Ok(*v as i32),
        _ => Err(format!("sample needs an integer {key}")),
    };
    Ok((n("x")?, n("y")?, n("w")?, n("h")?))
}

/// The RGB bytes of the rectangle `(x, y, w, h)` of a frame of `stride` pixels per row, row by row.
/// Pixels are packed as the screenshot encoder reads them: red in the low byte. `None` when the rectangle
/// does not fit the buffer.
pub fn crop_rgb(px: &[u32], stride: usize, (x, y, w, h): (usize, usize, usize, usize)) -> Option<Vec<u8>> {
    if w == 0 || h == 0 || x + w > stride {
        return None;
    }
    let rows = px.len() / stride.max(1);
    if y + h > rows {
        return None;
    }
    let mut out = Vec::with_capacity(w * h * 3);
    for row in y..y + h {
        for &p in &px[row * stride + x..row * stride + x + w] {
            out.extend([p as u8, (p >> 8) as u8, (p >> 16) as u8]);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crops_rows_in_order_as_rgb() {
        // 3 wide, 2 high; pixel value encodes its index in the red byte
        let px: Vec<u32> = (0..6u32).collect();
        assert_eq!(crop_rgb(&px, 3, (1, 0, 2, 2)), Some(vec![1, 0, 0, 2, 0, 0, 4, 0, 0, 5, 0, 0]));
    }

    #[test]
    fn refuses_a_rectangle_outside_the_frame() {
        let px = vec![0u32; 6];
        assert_eq!(crop_rgb(&px, 3, (2, 0, 2, 1)), None);
        assert_eq!(crop_rgb(&px, 3, (0, 1, 1, 2)), None);
        assert_eq!(crop_rgb(&px, 3, (0, 0, 0, 1)), None);
    }

    #[test]
    fn size_and_point_replies_carry_their_fields() {
        assert_eq!(size_reply(1920, 1080).get("w"), Some(&Json::Num(1920.0)));
        assert_eq!(size_reply(1920, 1080).get("h"), Some(&Json::Num(1080.0)));
        assert_eq!(point_reply(3, 4).get("x"), Some(&Json::Num(3.0)));
        assert_eq!(point_reply(3, 4).get("y"), Some(&Json::Num(4.0)));
    }

    #[test]
    fn the_pointer_is_recorded_floored_and_cleared_when_it_leaves() {
        pointer_moved(10.7, 20.2);
        assert_eq!(pointer_position(), Some((10, 20)));
        pointer_moved(-0.5, 3.0);
        assert_eq!(pointer_position(), Some((-1, 3)));
        pointer_left();
        assert_eq!(pointer_position(), None);
    }

    #[test]
    fn reads_the_rectangle_from_params() {
        let p = Json::Obj(vec![
            ("x".into(), Json::Num(4.0)),
            ("y".into(), Json::Num(5.0)),
            ("w".into(), Json::Num(9.0)),
            ("h".into(), Json::Num(1.0)),
        ]);
        assert_eq!(rect_of(&p), Ok((4, 5, 9, 1)));
        assert!(rect_of(&Json::Null).is_err());
    }
}
