//! Draws the plot on any `Surface`: the window's plot area, or the buffer an export is drawn into. One routine serves
//! both, so the image that is saved is the one on screen. Colours are the kit's tokens, resolved with `now()`, so the
//! plot follows Light and Dark; the curves keep their own colours.

use crate::model::Model;
use crate::png;
use crate::view::{tick_label, ticks};
use bifrost_app::kit::canvas::{Canvas, PathData, PathPaint, PathVerb, RectF};
use bifrost_app::kit::ui::{CONTENT_BG, LABEL_2, MAC_FOOTNOTE_PT, SEPARATOR_C};
use bifrost_app::kit::{pack, Image, Surface, TextCache, Ui, Weight};

/// The plot's rectangle in device pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

/// The thickness of the axes and of the grid lines, in device pixels.
const AXIS_PX: usize = 2;
/// The width of a curve, in points.
const CURVE_PT: f32 = 2.0;
/// Pixel coordinates are clamped to this, so a point far off the plot cannot overflow the rasteriser.
const PIXEL_LIMIT: f64 = 1.0e6;
/// The gap between a tick label and its axis, in device pixels.
const LABEL_GAP_PX: f64 = 4.0;

/// Draws the plot `model` into `area` of `s`.
pub fn draw(s: &mut Surface, ui: &Ui, labels: &mut TextCache, area: Area, model: &Model) {
    s.fill_rect(area.x, area.y, area.w, area.h, CONTENT_BG.now());
    if area.w == 0 || area.h == 0 {
        return;
    }
    let (w, h) = (area.w as f64, area.h as f64);
    let view = model.view;
    let step_x = view.grid_step(w);
    let step_y = view.grid_step(h);
    let (x0, x1) = view.x_range(w);
    let (y0, y1) = view.y_range(h);
    let (ox, oy) = view.to_px(0.0, 0.0, w, h);
    let grid = SEPARATOR_C.now();
    let axis = LABEL_2.now();
    let text = LABEL_2.now();

    // grid
    for gx in ticks(x0, x1, step_x) {
        let (px, _) = view.to_px(gx, 0.0, w, h);
        if let Some(col) = inside(px, w) {
            s.fill_rect(area.x + col, area.y, 1, area.h, grid);
        }
    }
    for gy in ticks(y0, y1, step_y) {
        let (_, py) = view.to_px(0.0, gy, w, h);
        if let Some(row) = inside(py, h) {
            s.fill_rect(area.x, area.y + row, area.w, 1, grid);
        }
    }

    // axes through the origin, when they are on the plot
    let x_axis_row = inside(oy, h);
    let y_axis_col = inside(ox, w);
    if let Some(row) = x_axis_row {
        s.fill_rect(area.x, area.y + row, area.w, AXIS_PX, axis);
    }
    if let Some(col) = y_axis_col {
        s.fill_rect(area.x + col, area.y, AXIS_PX, area.h, axis);
    }

    // tick values: under the x axis (or at the bottom edge) and beside the y axis (or at the right edge)
    for gx in ticks(x0, x1, step_x) {
        if is_zero(gx, step_x) {
            continue;
        }
        let (px, _) = view.to_px(gx, 0.0, w, h);
        let img = labels.get(ui, &tick_label(gx, step_x), MAC_FOOTNOTE_PT, Weight::Regular, text);
        let (iw, ih) = (img.w as f64, img.h as f64);
        let y = match x_axis_row {
            Some(row) => row as f64 + LABEL_GAP_PX,
            None => h - ih - LABEL_GAP_PX,
        };
        put(s, img, area, px - iw / 2.0, y);
    }
    for gy in ticks(y0, y1, step_y) {
        if is_zero(gy, step_y) {
            continue;
        }
        let (_, py) = view.to_px(0.0, gy, w, h);
        let img = labels.get(ui, &tick_label(gy, step_y), MAC_FOOTNOTE_PT, Weight::Regular, text);
        let (iw, ih) = (img.w as f64, img.h as f64);
        let x = match y_axis_col {
            Some(col) => col as f64 - iw - LABEL_GAP_PX,
            None => w - iw - LABEL_GAP_PX,
        };
        put(s, img, area, x, py - ih / 2.0);
    }

    // axis names: "x" at the right end of the x axis, "y" at the top of the y axis
    let img = labels.get(ui, "x", MAC_FOOTNOTE_PT, Weight::Bold, text);
    let (iw, ih) = (img.w as f64, img.h as f64);
    put(s, img, area, w - iw - LABEL_GAP_PX * 2.0, oy.clamp(0.0, h) - ih - LABEL_GAP_PX);
    let img = labels.get(ui, "y", MAC_FOOTNOTE_PT, Weight::Bold, text);
    put(s, img, area, ox.clamp(0.0, w) + LABEL_GAP_PX * 2.0, LABEL_GAP_PX);

    // curves, clipped to the plot
    let width = pt_to_px(ui, CURVE_PT);
    Canvas::push_clip(s, RectF::new(area.x as f32, area.y as f32, area.w as f32, area.h as f32), 0.0);
    for curve in model.curves(w, h) {
        let (r, g, b) = curve.rgb;
        let colour = pack(r, g, b, 255);
        for line in &curve.lines {
            if line.len() < 2 {
                continue;
            }
            let mut verbs = Vec::with_capacity(line.len());
            for (i, &(x, y)) in line.iter().enumerate() {
                let (px, py) = view.to_px(x, y, w, h);
                let px = area.x as f32 + px.clamp(-PIXEL_LIMIT, PIXEL_LIMIT) as f32;
                let py = area.y as f32 + py.clamp(-PIXEL_LIMIT, PIXEL_LIMIT) as f32;
                verbs.push(if i == 0 { PathVerb::Move(px, py) } else { PathVerb::Line(px, py) });
            }
            Canvas::path(s, &PathData { verbs }, PathPaint::Stroke(width), colour);
        }
    }
    Canvas::pop_clip(s);
}

/// The plot as PNG bytes, `w` by `h` pixels, drawn the way the window shows it.
pub fn export_png(ui: &Ui, labels: &mut TextCache, model: &Model, w: usize, h: usize) -> Vec<u8> {
    let mut px = vec![0u32; w * h];
    {
        let mut s = Surface { w, h, stride: w, px: &mut px, y0: 0 };
        draw(&mut s, ui, labels, Area { x: 0, y: 0, w, h }, model);
    }
    let mut rgba = Vec::with_capacity(w * h * 4);
    for pixel in &px {
        rgba.extend_from_slice(&pixel.to_le_bytes());
    }
    png::encode_rgba(w as u32, h as u32, &rgba)
}

/// Device pixels for a length in points.
fn pt_to_px(ui: &Ui, pt: f32) -> f32 {
    ui.pt(pt).max(1.0)
}

/// `v` as a pixel column or row when it lies in `0..max`.
fn inside(v: f64, max: f64) -> Option<usize> {
    if v >= 0.0 && v < max {
        Some(v as usize)
    } else {
        None
    }
}

fn is_zero(value: f64, step: f64) -> bool {
    value.abs() < step * 1e-6
}

/// Blits a label with its top left at `(x, y)` device pixels relative to `area`, moved inside the area when it would
/// spill out of it.
fn put(s: &mut Surface, img: &Image, area: Area, x: f64, y: f64) {
    let x = x.clamp(0.0, (area.w as f64 - img.w as f64).max(0.0));
    let y = y.clamp(0.0, (area.h as f64 - img.h as f64).max(0.0));
    s.blit(img, (area.x as f64 + x) as i32, (area.y as f64 + y) as i32, 255, 255);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_keeps_only_pixels_on_the_plot() {
        assert_eq!(inside(0.0, 10.0), Some(0));
        assert_eq!(inside(9.9, 10.0), Some(9));
        assert_eq!(inside(10.0, 10.0), None);
        assert_eq!(inside(-0.5, 10.0), None);
    }

    #[test]
    fn zero_is_recognised_for_any_step() {
        assert!(is_zero(0.0, 0.5));
        assert!(is_zero(1e-15, 0.5));
        assert!(!is_zero(0.5, 0.5));
    }
}
