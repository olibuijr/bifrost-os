//! A small drawing surface for thumbnails: a pixel buffer, the UI font at a scale chosen so that a page of
//! `logical_w` points fills the buffer, and a few drawing calls in points. Fonts come from the shell's assets
//! directory (`BIFROST_ASSETS`, else the installed paths, else the source tree's `assets`); with no font the canvas
//! cannot be made and a provider answers with the type icon.

use bifrost_render::font::Font;
use bifrost_render::gfx::{pack, Surface};
use bifrost_render::ui::{Ui, Weight};
use bifrost_ui::thumb_service::Pixels;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Paper and ink colours shared by the providers.
pub const PAPER: u32 = pack(250, 250, 247, 255);
pub const INK: u32 = pack(40, 40, 44, 255);
pub const MUTED: u32 = pack(120, 120, 128, 255);
pub const EDGE: u32 = pack(200, 200, 204, 255);
pub const ACCENT: u32 = pack(10, 132, 255, 255);
pub const RED: u32 = pack(255, 59, 48, 255);

/// The font file, read once. `None` when no assets directory has it.
fn font_bytes() -> Option<&'static [u8]> {
    static BYTES: OnceLock<Option<&'static [u8]>> = OnceLock::new();
    *BYTES.get_or_init(|| {
        // the override, the package-owned assets, the legacy directory, then the source tree
        let mut dirs: Vec<PathBuf> = bifrost_core::system_assets::candidates();
        dirs.push(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/assets")));
        dirs.iter().find_map(|d| std::fs::read(d.join(crate::first_frame_cache::MENU_FONT)).ok()).map(|v| &*Box::leak(v.into_boxed_slice()))
    })
}

/// Whether the UI font can be loaded here (tests that draw text skip when it cannot).
pub fn have_font() -> bool {
    font_bytes().is_some_and(|b| Font::parse(b).is_some())
}

/// A UI at `s` pixels per point, or `None` without the font.
pub fn make_ui(s: f32) -> Option<Ui> {
    Some(Ui::new(Font::parse(font_bytes()?)?, s))
}

pub struct Canvas {
    pub ui: Ui,
    pub w: usize,
    pub h: usize,
    /// Pixels per point.
    pub s: f32,
    px: Vec<u32>,
}

impl Canvas {
    /// A transparent canvas of `w` x `h` pixels where `logical_w` points span the width. `None` without a font or
    /// with an empty size.
    pub fn new(w: usize, h: usize, logical_w: f32) -> Option<Canvas> {
        if w == 0 || h == 0 || logical_w <= 0.0 {
            return None;
        }
        let s = w as f32 / logical_w;
        Some(Canvas { ui: make_ui(s)?, w, h, s, px: vec![0; w * h] })
    }

    /// A canvas over an existing picture, where `logical_w` points span its width: what is drawn lands on top of the
    /// picture. `None` without a font or with a non-positive width.
    pub fn over(pixels: Pixels, logical_w: f32) -> Option<Canvas> {
        if logical_w <= 0.0 {
            return None;
        }
        let s = pixels.w as f32 / logical_w;
        Some(Canvas { ui: make_ui(s)?, w: pixels.w, h: pixels.h, s, px: pixels.px })
    }

    pub fn surface(&mut self) -> Surface<'_> {
        Surface { w: self.w, h: self.h, stride: self.w, px: &mut self.px, y0: 0 }
    }

    /// A filled rounded rectangle, in points.
    pub fn rrect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, color: u32) {
        let s = self.s;
        self.surface().fill_rrect(x * s, y * s, w * s, h * s, r * s, color);
    }

    /// A filled rectangle, in points.
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: u32) {
        let s = self.s;
        let (x0, y0) = ((x * s).round().max(0.0) as usize, (y * s).round().max(0.0) as usize);
        let (x1, y1) = (((x + w) * s).round().max(0.0) as usize, ((y + h) * s).round().max(0.0) as usize);
        let (x1, y1) = (x1.min(self.w), y1.min(self.h));
        if x1 > x0 && y1 > y0 {
            self.surface().fill_rect(x0, y0, x1 - x0, y1 - y0, color);
        }
    }

    /// The page: paper with a hairline edge, filling the whole canvas.
    pub fn page(&mut self, radius_pt: f32) {
        let (w, h) = (self.w as f32 / self.s, self.h as f32 / self.s);
        self.rrect(0.0, 0.0, w, h, radius_pt, EDGE);
        let hair = 0.5;
        self.rrect(hair, hair, w - 2.0 * hair, h - 2.0 * hair, (radius_pt - hair).max(0.0), PAPER);
    }

    /// Width in points of `text` at `pt` points.
    pub fn width(&self, text: &str, pt: f32, weight: Weight) -> f32 {
        self.ui.measure(text, pt, weight) / self.s
    }

    /// `text` with its top-left at (`x`, `y`) points, cut with an ellipsis to `max_w` points.
    pub fn text(&mut self, text: &str, pt: f32, weight: Weight, color: u32, x: f32, y: f32, max_w: f32) {
        if text.is_empty() {
            return;
        }
        let shown = self.ui.fit(text, pt, max_w * self.s);
        let img = self.ui.text(&shown, pt, weight, bifrost_render::ui::WHITE);
        // the rendered image carries two pixels of padding on every side
        let (ix, iy) = ((x * self.s).round() as i32 - 2, (y * self.s).round() as i32 - 2);
        self.surface().blit_colored(&img, ix, iy, color, 255);
    }

    /// `text` centred on `cx` points.
    pub fn text_centered(&mut self, text: &str, pt: f32, weight: Weight, color: u32, cx: f32, y: f32, max_w: f32) {
        let w = self.width(text, pt, weight).min(max_w);
        self.text(text, pt, weight, color, cx - w / 2.0, y, max_w);
    }

    pub fn into_pixels(self) -> Option<Pixels> {
        Pixels::new(self.w, self.h, self.px)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_canvas_scales_points_to_pixels() {
        if !have_font() {
            return;
        }
        let c = Canvas::new(200, 100, 100.0).unwrap();
        assert_eq!(c.s, 2.0);
        assert!(Canvas::new(0, 10, 100.0).is_none());
        assert!(Canvas::new(10, 10, 0.0).is_none());
    }

    #[test]
    fn a_page_fills_the_canvas_with_paper() {
        if !have_font() {
            return;
        }
        let mut c = Canvas::new(60, 80, 60.0).unwrap();
        c.page(3.0);
        let p = c.into_pixels().unwrap();
        assert_eq!(p.px[40 * 60 + 30], PAPER);
        assert_eq!(p.px[0], 0, "the rounded corner stays transparent");
    }

    #[test]
    fn text_marks_pixels() {
        if !have_font() {
            return;
        }
        let mut c = Canvas::new(120, 40, 120.0).unwrap();
        c.text("Hi", 14.0, Weight::Regular, INK, 4.0, 4.0, 100.0);
        let p = c.into_pixels().unwrap();
        assert!(p.px.iter().any(|&v| v != 0));
    }
}
