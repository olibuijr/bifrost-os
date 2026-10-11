//! Draws a laid-out [`Page`] into a surface: rectangles, text (rendered
//! once per string and cached), underlines, slanted italics, scaled
//! pictures and the selection highlight. Layout units are CSS px, which
//! are the shell's points; `Viewport::scale` turns them into screen px.

use super::css::Color;
use super::layout::{Item, Page};
use crate::gfx::{pack, Image, Surface};
use crate::shell::cache::TextCache;
use crate::shell::ui::{Ui, Weight};
use std::collections::{HashMap, HashSet};

/// Highlight behind selected text, on the white page.
pub const SELECT_BG: Color = Color::rgb(184, 212, 255);
const PLACEHOLDER: u32 = pack(228, 228, 230, 255);
const SCALED_KEEP: usize = 16;
const SLANT: f32 = 0.2;

/// Decoded pictures of one page, plus their screen-size versions.
#[derive(Default)]
pub struct Images {
    raw: HashMap<String, Image>,
    scaled: HashMap<(String, usize, usize), Image>,
    failed: HashSet<String>,
}

impl Images {
    pub fn insert(&mut self, src: &str, img: Image) {
        self.scaled.retain(|(s, ..), _| s != src);
        self.raw.insert(src.to_string(), img);
    }

    pub fn fail(&mut self, src: &str) {
        self.failed.insert(src.to_string());
    }

    pub fn has(&self, src: &str) -> bool {
        self.raw.contains_key(src) || self.failed.contains(src)
    }

    pub fn size(&self, src: &str) -> Option<(usize, usize)> {
        self.raw.get(src).map(|i| (i.w, i.h))
    }

    pub fn len(&self) -> usize {
        self.raw.len()
    }

    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    /// The picture at exactly `w` x `h` px, scaled once and kept.
    pub fn scaled(&mut self, src: &str, w: usize, h: usize) -> Option<&Image> {
        let raw = self.raw.get(src)?;
        let key = (src.to_string(), w.max(1), h.max(1));
        if !self.scaled.contains_key(&key) {
            if self.scaled.len() >= SCALED_KEEP {
                self.scaled.clear();
            }
            let factor = (raw.w / key.1).min(raw.h / key.2).max(1);
            let img = if factor > 1 { crate::imgdec::box_down(raw, factor).resized(key.1, key.2) } else { raw.resized(key.1, key.2) };
            self.scaled.insert(key.clone(), img);
        }
        self.scaled.get(&key)
    }
}

/// Which text is drawn highlighted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Highlight {
    None,
    Block(usize),
    All,
}

impl Highlight {
    fn covers(self, block: usize) -> bool {
        match self {
            Highlight::None => false,
            Highlight::Block(b) => b == block,
            Highlight::All => block != super::layout::NO_BLOCK,
        }
    }
}

/// Where the page sits on the screen, in px.
#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub x: f32,
    pub top: f32,
    pub w: f32,
    pub h: f32,
    /// how far the page is scrolled, px
    pub scroll: f32,
    /// px per CSS px
    pub scale: f32,
}

#[derive(Default)]
pub struct Painter {
    text: TextCache,
    slanted: HashMap<(String, u32, bool, u32), Image>,
}

/// Fills a rectangle in px, clipped to the screen and to `top..bottom`.
fn fill(s: &mut Surface, x: f32, y: f32, w: f32, h: f32, color: u32, top: f32, bottom: f32) {
    let (x0, x1) = (x.max(0.0), (x + w).min(s.w as f32));
    let (y0, y1) = (y.max(top), (y + h).min(bottom));
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    s.fill_rect(x0.round() as usize, y0.round() as usize, ((x1 - x0).round() as usize).max(1), ((y1 - y0).round() as usize).max(1), color);
}

/// Leans text to the right by shearing each row.
pub fn slant(img: &Image) -> Image {
    let shift = (img.h as f32 * SLANT).ceil() as usize;
    let mut out = Image::new(img.w + shift, img.h);
    for y in 0..img.h {
        let off = ((img.h - 1 - y) as f32 * SLANT).round() as usize;
        out.px[y * out.w + off..y * out.w + off + img.w].copy_from_slice(&img.px[y * img.w..(y + 1) * img.w]);
    }
    out
}

impl Painter {
    /// Draws `page` through `vp`: first the page background over the whole
    /// viewport, then its items. Anything the page puts above `vp.top` is
    /// left for the chrome to cover.
    pub fn paint(&mut self, s: &mut Surface, ui: &Ui, images: &mut Images, page: &Page, vp: &Viewport, highlight: Highlight) {
        let (top, bottom, sc) = (vp.top, vp.top + vp.h, vp.scale);
        let bg = page.background.unwrap_or(Color::WHITE).pack();
        fill(s, vp.x, top, vp.w, vp.h, bg, top, bottom);
        for item in &page.items {
            match item {
                Item::Rect { x, y, w, h, color } => {
                    fill(s, vp.x + x * sc, top + y * sc - vp.scroll, w * sc, h * sc, color.pack(), top, bottom);
                }
                Item::Text { x, y, w, h, size, text, bold, italic, underline, color, block, .. } => {
                    let (sx, sy) = (vp.x + x * sc, top + y * sc - vp.scroll);
                    if sy + h * sc < top || sy > bottom {
                        continue;
                    }
                    if highlight.covers(*block) {
                        fill(s, sx, sy, (w + size * 0.25) * sc, h * sc, SELECT_BG.pack(), top, bottom);
                    }
                    let packed = color.pack();
                    let weight = if *bold { Weight::Bold } else { Weight::Regular };
                    let img = self.text.get(ui, text, *size, weight, packed);
                    if *italic {
                        let key = (text.clone(), (*size * 10.0) as u32, *bold, packed);
                        let slanted = self.slanted.entry(key).or_insert_with(|| slant(img));
                        s.blit(slanted, sx.round() as i32, sy.round() as i32, 255, 255);
                    } else {
                        s.blit(img, sx.round() as i32, sy.round() as i32, 255, 255);
                    }
                    if *underline {
                        fill(s, sx, sy + h * sc * 0.9, w * sc, sc.max(1.0), packed, top, bottom);
                    }
                }
                Item::Image { x, y, w, h, src, .. } => {
                    let (sx, sy) = (vp.x + x * sc, top + y * sc - vp.scroll);
                    let (dw, dh) = ((w * sc).round().max(1.0) as usize, (h * sc).round().max(1.0) as usize);
                    if sy + dh as f32 > top && sy < bottom {
                        match images.scaled(src, dw, dh) {
                            Some(img) => s.blit(img, sx.round() as i32, sy.round() as i32, 255, 255),
                            None => fill(s, sx, sy, dw as f32, dh as f32, PLACEHOLDER, top, bottom),
                        }
                    }
                }
            }
        }
        if self.slanted.len() > 512 {
            self.slanted.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::browser::layout::NO_BLOCK;

    fn ui() -> Ui {
        crate::shell::tests::ui()
    }

    fn surface(w: usize, h: usize) -> (Vec<u32>, usize, usize) {
        (vec![0u32; w * h], w, h)
    }

    fn page(items: Vec<Item>) -> Page {
        Page { width: 100.0, height: 100.0, items, ..Page::default() }
    }

    fn vp(scroll: f32) -> Viewport {
        Viewport { x: 0.0, top: 20.0, w: 100.0, h: 60.0, scroll, scale: 1.0 }
    }

    const RED: Color = Color::rgb(255, 0, 0);

    #[test]
    fn rectangles_land_at_the_scrolled_position_and_clip_to_the_viewport() {
        let (mut px, w, h) = surface(100, 100);
        let mut s = Surface { w, h, stride: w, px: &mut px, y0: 0 };
        let p = page(vec![Item::Rect { x: 10.0, y: 10.0, w: 20.0, h: 100.0, color: RED }]);
        let mut painter = Painter::default();
        painter.paint(&mut s, &ui(), &mut Images::default(), &p, &vp(5.0), Highlight::None);
        let at = |x: usize, y: usize| px[y * 100 + x];
        assert_eq!(at(15, 25), RED.pack(), "page y 10 at scroll 5 is screen y 25");
        assert_eq!(at(15, 22), Color::WHITE.pack(), "above the rectangle is page background");
        assert_eq!(at(15, 85), 0, "nothing below the viewport");
        assert_eq!(at(15, 10), 0, "nothing above it either");
        assert_eq!(at(40, 40), Color::WHITE.pack());
    }

    #[test]
    fn the_page_background_fills_the_viewport() {
        let (mut px, w, h) = surface(100, 100);
        let mut s = Surface { w, h, stride: w, px: &mut px, y0: 0 };
        let mut p = page(vec![]);
        p.background = Some(Color::rgb(1, 2, 3));
        Painter::default().paint(&mut s, &ui(), &mut Images::default(), &p, &vp(0.0), Highlight::None);
        assert_eq!(px[50 * 100 + 50], Color::rgb(1, 2, 3).pack());
        assert_eq!(px[90 * 100 + 50], 0);
    }

    fn word(block: usize, italic: bool, underline: bool) -> Item {
        Item::Text { x: 5.0, y: 5.0, w: 60.0, h: 24.0, size: 20.0, text: "Hello".into(), bold: false, italic, underline, color: Color::BLACK, link: None, block }
    }

    #[test]
    fn text_draws_dark_pixels_and_underlines_and_highlights() {
        let ink = |items: Vec<Item>, hl: Highlight| {
            let (mut px, w, h) = surface(100, 100);
            let mut s = Surface { w, h, stride: w, px: &mut px, y0: 0 };
            Painter::default().paint(&mut s, &ui(), &mut Images::default(), &page(items), &vp(0.0), hl);
            px
        };
        let plain = ink(vec![word(0, false, false)], Highlight::None);
        let dark = plain.iter().filter(|&&p| p != Color::WHITE.pack() && p != 0 && (p & 0xff) < 100).count();
        assert!(dark > 20, "glyphs inked {dark} pixels");
        let under = ink(vec![word(0, false, true)], Highlight::None);
        assert!(under.iter().zip(&plain).any(|(a, b)| a != b), "underline adds pixels");
        let marked = ink(vec![word(0, false, false)], Highlight::Block(0));
        assert!(marked.contains(&SELECT_BG.pack()));
        assert!(!ink(vec![word(1, false, false)], Highlight::Block(0)).contains(&SELECT_BG.pack()));
        assert!(ink(vec![word(0, false, false)], Highlight::All).contains(&SELECT_BG.pack()));
        assert!(!ink(vec![word(NO_BLOCK, false, false)], Highlight::All).contains(&SELECT_BG.pack()), "list markers are never selected");
        let italic = ink(vec![word(0, true, false)], Highlight::None);
        assert!(italic != plain, "italics are slanted");
    }

    #[test]
    fn slant_shears_rows_progressively() {
        let mut img = Image::new(4, 10);
        img.px.iter_mut().for_each(|p| *p = 0xff00_00ff);
        let out = slant(&img);
        assert_eq!(out.w, 4 + 2);
        assert_eq!(out.px[0], 0, "the top row starts shifted right");
        assert_eq!(out.px[1 * out.w - 1], 0xff00_00ff);
        assert_eq!(out.px[9 * out.w], 0xff00_00ff, "the bottom row stays put");
    }

    #[test]
    fn pictures_scale_to_their_box_and_missing_ones_show_a_placeholder() {
        let mut imgs = Images::default();
        let mut src = Image::new(8, 4);
        src.px.iter_mut().for_each(|p| *p = 0xff00_ff00);
        imgs.insert("a.png", src);
        assert_eq!(imgs.size("a.png"), Some((8, 4)));
        assert!(imgs.has("a.png") && !imgs.has("b.png"));
        imgs.fail("b.png");
        assert!(imgs.has("b.png"));
        let items = vec![
            Item::Image { x: 0.0, y: 0.0, w: 20.0, h: 10.0, src: "a.png".into(), link: None },
            Item::Image { x: 0.0, y: 20.0, w: 20.0, h: 10.0, src: "b.png".into(), link: None },
        ];
        let (mut px, w, h) = surface(100, 100);
        let mut s = Surface { w, h, stride: w, px: &mut px, y0: 0 };
        Painter::default().paint(&mut s, &ui(), &mut imgs, &page(items), &vp(0.0), Highlight::None);
        assert_eq!(px[25 * 100 + 10], 0xff00_ff00);
        assert_eq!(px[45 * 100 + 10], PLACEHOLDER);
        assert_eq!(imgs.scaled("a.png", 20, 10).map(|i| (i.w, i.h)), Some((20, 10)));
        assert!(imgs.scaled("zzz", 5, 5).is_none());
    }
}
