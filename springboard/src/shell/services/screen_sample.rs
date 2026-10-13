//! The `screen` service: the pixel codes of one rectangle of the display, for apps that hold the Screen
//! permission (the Color Meter). The shell draws the frame the way a screenshot does and copies out the
//! rectangle. The SDK side is `bifrost_app::screen_sample`.

use super::privacy_permissions::access;
use super::{Shell, SystemService};
use crate::gfx::{Image, Surface};
use bifrost_core::json::Json;
use bifrost_ui::capture_access::Media;

/// Largest side of a sampled rectangle, in display pixels. Keeps one reply small.
pub const MAX_SIDE: i32 = 64;

pub struct ScreenSample;

impl SystemService for ScreenSample {
    fn name(&self) -> &'static str {
        "screen"
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, t: f64) -> Result<Json, String> {
        if method != "sample" {
            return Err(format!("screen has no method {method}"));
        }
        if !access(app_id).can_capture(Media::Screen) {
            return Err("the Screen permission is not granted".into());
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
