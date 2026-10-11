//! Painting: text helpers over the shell's `TextCache`, and the painter for the client core's
//! `DrawList`s; their coordinates are surface pixels.

use crate::gfx::{rgba, Image, Surface};
use crate::shell::cache::TextCache;
use crate::shell::ui::{Ui, Weight};
use std::collections::HashMap;
use wowstream_client::draw::{Color, DrawList};

/// Straight float RGBA to a premultiplied pixel.
fn pixel(c: Color) -> u32 {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    rgba(b(c[0]), b(c[1]), b(c[2]), b(c[3]))
}

/// Text left-aligned at `x`, vertically centred on `cy`; returns its width.
pub fn text(s: &mut Surface, cache: &mut TextCache, ui: &Ui, t: &str, pt: f32, weight: Weight, color: u32, x: f32, cy: f32) -> f32 {
    let img = cache.get(ui, t, pt, weight, color);
    s.blit(img, x.round() as i32, (cy - img.h as f32 / 2.0).round() as i32, 255, 255);
    img.w as f32
}

/// Text centred on (`cx`, `cy`).
pub fn text_c(s: &mut Surface, cache: &mut TextCache, ui: &Ui, t: &str, pt: f32, weight: Weight, color: u32, cx: f32, cy: f32) {
    let img = cache.get(ui, t, pt, weight, color);
    s.blit(img, (cx - img.w as f32 / 2.0).round() as i32, (cy - img.h as f32 / 2.0).round() as i32, 255, 255);
}

/// Text right-aligned to `right`.
pub fn text_r(s: &mut Surface, cache: &mut TextCache, ui: &Ui, t: &str, pt: f32, weight: Weight, color: u32, right: f32, cy: f32) {
    let img = cache.get(ui, t, pt, weight, color);
    s.blit(img, (right - img.w as f32).round() as i32, (cy - img.h as f32 / 2.0).round() as i32, 255, 255);
}

/// Draws the core's lists: for each list, its shapes then its labels, lists in order, so a later
/// list is always on top of an earlier one.
#[derive(Default)]
pub struct Painter {
    /// Rasterised label images.
    texts: HashMap<(String, u32, u32), Image>,
    scale: u32,
}

impl Painter {
    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, list: &DrawList) {
        if self.scale != ui.s.to_bits() || self.texts.len() > 256 {
            self.texts.clear();
            self.scale = ui.s.to_bits();
        }
        for shape in &list.shapes {
            let r = shape.rect;
            if shape.fill[3] > 0.0 {
                s.fill_rrect(r.x, r.y, r.w, r.h, shape.radius, pixel(shape.fill));
            }
            if shape.stroke[3] > 0.0 && shape.stroke_w > 0.0 {
                s.stroke_rrect(r.x, r.y, r.w, r.h, shape.radius, shape.stroke_w, pixel(shape.stroke));
            }
        }
        for label in &list.labels {
            let color = pixel(label.color);
            let key = (label.text.clone(), (label.size * 10.0) as u32, color);
            // `chrome` ignores Dynamic Type; its size is in points, ours in pixels
            let img = self.texts.entry(key).or_insert_with(|| ui.chrome(&label.text, label.size / ui.s, Weight::Regular, color));
            let x = if label.left { label.cx } else { label.cx - img.w as f32 / 2.0 };
            s.blit(img, x.round() as i32, (label.cy - img.h as f32 / 2.0).round() as i32, 255, 255);
        }
    }
}
