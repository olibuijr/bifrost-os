//! Rendered-text cache, so widgets that redraw every frame (keyboard,
//! pickers, stopwatch digits) rasterise each string once.

use crate::ui::{Ui, Weight};
use crate::gfx::{Image, Surface};
use std::collections::HashMap;

/// Style of an entry: pt x10, weight, colour.
type StyleKey = (u32, u8, u32);

/// One generation of entries: a hit never allocates a key.
#[derive(Default)]
struct Generation {
    map: HashMap<StyleKey, HashMap<String, Image>>,
    count: usize,
    bytes: usize,
}

impl Generation {
    fn get(&self, style: StyleKey, text: &str) -> Option<&Image> {
        self.map.get(&style)?.get(text)
    }

    /// Removes and returns an entry, so it can move to the newer generation.
    fn take(&mut self, style: StyleKey, text: &str) -> Option<Image> {
        let img = self.map.get_mut(&style)?.remove(text)?;
        self.count -= 1;
        self.bytes -= img.px.len() * 4;
        Some(img)
    }

    fn insert(&mut self, style: StyleKey, text: &str, img: Image) {
        self.count += 1;
        self.bytes += img.px.len() * 4;
        self.map.entry(style).or_default().insert(text.to_string(), img);
    }
}

/// Pixel bytes kept when the cache is not sized to a screen.
const DEFAULT_BUDGET: usize = 8 * 1024 * 1024;
/// Bounds of a screen-sized budget: the floor keeps small screens useful, the ceiling caps the memory.
const MIN_BUDGET: usize = 4 * 1024 * 1024;
const MAX_BUDGET: usize = 32 * 1024 * 1024;
/// Entries per generation (the cache holds at most twice this).
const GEN_ENTRIES: usize = 256;
/// Pixel bytes per generation at the default budget, before it retires, so large text (the lock clock)
/// cannot fill the cache.
const GEN_BYTES: usize = DEFAULT_BUDGET / 2;

/// The pixel budget for a screen of `w` x `h` px: two screens of RGBA text, within `MIN_BUDGET..=MAX_BUDGET`.
pub fn budget_for_screen(w: usize, h: usize) -> usize {
    (w.saturating_mul(h).saturating_mul(4).saturating_mul(2)).clamp(MIN_BUDGET, MAX_BUDGET)
}

/// Two generations, so a bounded cache keeps the strings in use rather than starting over
/// when it fills: `young` takes new renders, `old` holds the previous generation. A hit in `old` moves
/// the entry to `young`; a full `young` retires `old` and becomes the new `old`. Each generation keeps
/// half of the pixel budget.
pub struct TextCache {
    young: Generation,
    old: Generation,
    /// the most pixel bytes kept across both generations
    budget: usize,
    /// `emoji::generation()` the entries were rendered under
    emoji_gen: u32,
    /// `Ui::text_epoch()` the entries were rendered under
    epoch: u64,
}

impl Default for TextCache {
    fn default() -> Self {
        TextCache::with_budget(DEFAULT_BUDGET)
    }
}

impl TextCache {
    /// An empty cache holding at most `bytes` of pixels.
    pub fn with_budget(bytes: usize) -> Self {
        TextCache { young: Generation::default(), old: Generation::default(), budget: bytes, emoji_gen: 0, epoch: 0 }
    }

    /// An empty cache sized to a `w` x `h` screen (see `budget_for_screen`).
    pub fn for_screen(w: usize, h: usize) -> Self {
        Self::with_budget(budget_for_screen(w, h))
    }

    /// The pixel budget in bytes.
    pub fn budget(&self) -> usize {
        self.budget
    }

    /// Pixel bytes held by the cached images.
    pub fn bytes(&self) -> usize {
        self.young.bytes + self.old.bytes
    }

    /// Pixel bytes one generation may hold before it retires: half the budget.
    fn gen_bytes(&self) -> usize {
        self.budget / 2
    }

    /// `text` at `pt` points in `weight` and `color`, rendering on a miss.
    pub fn get(&mut self, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32) -> &Image {
        // a reloaded emoji font invalidates every cached image that may hold emoji
        let g = crate::emoji::generation();
        let epoch = ui.text_epoch();
        if self.emoji_gen != g || self.epoch != epoch {
            self.emoji_gen = g;
            self.epoch = epoch;
            self.clear();
        }
        let style = ((pt * 10.0) as u32, weight as u8, color);
        if self.young.get(style, text).is_none() {
            let img = match self.old.take(style, text) {
                Some(img) => img,
                None => ui.text(text, pt, weight, color),
            };
            if self.young.count >= GEN_ENTRIES || self.young.bytes >= self.gen_bytes() {
                self.old = std::mem::take(&mut self.young);
            }
            self.young.insert(style, text, img);
        }
        self.young.get(style, text).expect("the entry was just inserted")
    }

    /// Draws `text` one cached glyph at a time, for strings that change
    /// every frame (clock digits) and would otherwise flood the cache.
    /// Returns the width in px.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_chars(&mut self, s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, x: f32, y: f32) -> f32 {
        let mut buf = [0u8; 4];
        for (i, c) in text.char_indices() {
            let glyph: &str = c.encode_utf8(&mut buf);
            let advance = ui.measure(glyph, pt, weight);
            let img = self.get(ui, glyph, pt, weight, color);
            // the pen is the shaped width of the text before this glyph, so kerning matches the whole-string measure
            let pen = x + ui.measure(&text[..i], pt, weight);
            // the glyph image is padded equally on both sides of its advance
            let pad = (img.w as f32 - advance.ceil()) / 2.0;
            s.blit(img, (pen - pad).round() as i32, y as i32, 255, 255);
        }
        ui.measure(text, pt, weight)
    }

    /// Height of a line at `pt`, as `draw_chars` lays it out.
    pub fn line_h(&mut self, ui: &Ui, pt: f32) -> f32 {
        self.get(ui, "0", pt, Weight::Regular, 0xffff_ffff).h as f32
    }

    /// Drops everything cached (after a density or theme change).
    pub fn clear(&mut self) {
        self.young = Generation::default();
        self.old = Generation::default();
    }

    pub fn len(&self) -> usize {
        self.young.count + self.old.count
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::WHITE;

    #[test]
    fn renders_once_per_distinct_request() {
        let _emoji = crate::tests::emoji_lock();
        let ui = crate::tests::ui();
        let mut c = TextCache::default();
        let w = c.get(&ui, "12", 20.0, Weight::Regular, WHITE).w;
        assert_eq!(c.get(&ui, "12", 20.0, Weight::Regular, WHITE).w, w);
        assert_eq!(c.len(), 1);
        c.get(&ui, "12", 20.0, Weight::Bold, WHITE);
        c.get(&ui, "12", 21.0, Weight::Regular, WHITE);
        assert_eq!(c.len(), 3);
        c.clear();
        assert!(c.is_empty());
    }

    #[test]
    fn cached_text_follows_dynamic_type_and_bold_text() {
        let _emoji = crate::tests::emoji_lock();
        let mut ui = crate::tests::ui();
        let mut c = TextCache::default();
        let normal = c.get(&ui, "Settings", 13.0, Weight::Regular, WHITE).w;
        ui.text_scale = 1.5;
        let larger = c.get(&ui, "Settings", 13.0, Weight::Regular, WHITE).w;
        assert!(larger > normal, "Dynamic Type must not serve the old image ({larger} vs {normal})");
        assert_eq!(c.len(), 1, "the cache started over");
        ui.text_scale = 1.0;
        ui.bold = true;
        let bold = c.get(&ui, "Settings", 13.0, Weight::Regular, WHITE).w;
        assert!(bold >= normal);
        assert_eq!(c.get(&ui, "Settings", 13.0, Weight::Regular, WHITE).w, bold);
    }

    #[test]
    fn per_char_drawing_matches_whole_string_width() {
        let _emoji = crate::tests::emoji_lock();
        let ui = crate::tests::ui();
        let mut c = TextCache::default();
        let mut buf = vec![0u32; 600 * 80];
        let mut s = Surface { w: 600, h: 80, stride: 600, px: &mut buf, y0: 0 };
        let w = c.draw_chars(&mut s, &ui, "00:05,04", 30.0, Weight::Regular, WHITE, 10.0, 5.0);
        let whole = ui.measure("00:05,04", 30.0, Weight::Regular);
        assert!((w - whole).abs() < 0.01, "{w} vs {whole}");
        assert_eq!(c.len(), 5, "one image per distinct glyph");
        assert!(buf.contains(&WHITE));
        assert!(c.line_h(&ui, 30.0) > 30.0);
    }

    #[test]
    fn per_char_width_follows_weight() {
        let _emoji = crate::tests::emoji_lock();
        let ui = crate::tests::ui();
        let mut c = TextCache::default();
        let mut buf = vec![0u32; 600 * 80];
        let mut s = Surface { w: 600, h: 80, stride: 600, px: &mut buf, y0: 0 };
        let w = c.draw_chars(&mut s, &ui, "0123", 30.0, Weight::Bold, WHITE, 10.0, 5.0);
        assert!((w - ui.measure("0123", 30.0, Weight::Bold)).abs() < 0.01);
    }

    #[test]
    fn cache_is_bounded() {
        let _emoji = crate::tests::emoji_lock();
        let ui = crate::tests::ui();
        let mut c = TextCache::default();
        for i in 0..2 * GEN_ENTRIES + 10 {
            c.get(&ui, &i.to_string(), 12.0, Weight::Regular, WHITE);
        }
        assert!(c.len() <= 2 * GEN_ENTRIES);
        assert!(!c.is_empty());
    }

    #[test]
    fn a_full_cache_keeps_the_strings_in_use() {
        let _emoji = crate::tests::emoji_lock();
        let ui = crate::tests::ui();
        let mut c = TextCache::default();
        // a hot string is touched between every cold render, so it survives each retirement of a generation
        for i in 0..4 * GEN_ENTRIES {
            c.get(&ui, "hot", 12.0, Weight::Regular, WHITE);
            c.get(&ui, &i.to_string(), 12.0, Weight::Regular, WHITE);
        }
        assert!(c.len() <= 2 * GEN_ENTRIES, "{}", c.len());
        let holds = |g: &Generation, text: &str| g.map.values().any(|m| m.contains_key(text));
        assert!(holds(&c.young, "hot") || holds(&c.old, "hot"), "the hot string was rendered again and again");
    }

    #[test]
    fn large_images_are_bounded_by_bytes() {
        let _emoji = crate::tests::emoji_lock();
        let ui = crate::tests::ui();
        let mut c = TextCache::default();
        for i in 0..40 {
            c.get(&ui, &format!("{i}"), 300.0, Weight::Regular, WHITE);
        }
        assert!(c.young.bytes < GEN_BYTES + GEN_BYTES / 2 && c.old.bytes < GEN_BYTES + GEN_BYTES / 2, "{} {}", c.young.bytes, c.old.bytes);
        assert!(c.len() < 40, "the byte budget retired a generation");
    }

    #[test]
    fn the_byte_budget_bounds_the_cache_and_is_sized_to_the_screen() {
        let _emoji = crate::tests::emoji_lock();
        let ui = crate::tests::ui();
        let mut c = TextCache::with_budget(64 * 1024);
        for i in 0..200 {
            c.get(&ui, &format!("budget {i}"), 13.0, Weight::Regular, WHITE);
            // each generation keeps half the budget, plus the one image that retires it
            assert!(c.bytes() <= 2 * c.budget(), "{} bytes over a {} budget", c.bytes(), c.budget());
        }
        assert!(c.len() >= 1);
        assert!(c.len() < 200, "the budget retired the early strings");
        assert_eq!(budget_for_screen(1920, 1080), 2 * 1920 * 1080 * 4);
        assert_eq!(budget_for_screen(16, 16), MIN_BUDGET, "small screens get the floor");
        assert_eq!(budget_for_screen(7680, 4320), MAX_BUDGET, "large screens get the ceiling");
        assert_eq!(TextCache::for_screen(1920, 1080).budget(), budget_for_screen(1920, 1080));
    }
}
