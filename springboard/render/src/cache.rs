//! Rendered-text cache, so widgets that redraw every frame (keyboard,
//! pickers, stopwatch digits) rasterise each string once.

use crate::ui::{Ui, Weight};
use crate::gfx::{Image, Surface};
use std::collections::HashMap;

#[derive(Default)]
pub struct TextCache {
    /// style (pt x10, weight, colour) -> text -> image, so a hit never allocates a key
    map: HashMap<(u32, u8, u32), HashMap<String, Image>>,
    count: usize,
    /// `emoji::generation()` the entries were rendered under
    emoji_gen: u32,
    /// `Ui::text_epoch()` the entries were rendered under
    epoch: u64,
}

/// Entries kept before the cache starts over (like the glyph cache).
const MAX_ENTRIES: usize = 512;

impl TextCache {
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
        if !self.map.get(&style).is_some_and(|m| m.contains_key(text)) {
            if self.count >= MAX_ENTRIES {
                self.clear();
            }
            self.map.entry(style).or_default().insert(text.to_string(), ui.text(text, pt, weight, color));
            self.count += 1;
        }
        &self.map[&style][text]
    }

    /// Draws `text` one cached glyph at a time, for strings that change
    /// every frame (clock digits) and would otherwise flood the cache.
    /// Returns the width in px.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_chars(&mut self, s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, x: f32, y: f32) -> f32 {
        let mut pen = x;
        let mut buf = [0u8; 4];
        for c in text.chars() {
            let glyph: &str = c.encode_utf8(&mut buf);
            let advance = ui.measure(glyph, pt, weight);
            let img = self.get(ui, glyph, pt, weight, color);
            // the glyph image is padded equally on both sides of its advance
            let pad = (img.w as f32 - advance.ceil()) / 2.0;
            s.blit(img, (pen - pad).round() as i32, y as i32, 255, 255);
            pen += advance;
        }
        pen - x
    }

    /// Height of a line at `pt`, as `draw_chars` lays it out.
    pub fn line_h(&mut self, ui: &Ui, pt: f32) -> f32 {
        self.get(ui, "0", pt, Weight::Regular, 0xffff_ffff).h as f32
    }

    /// Drops everything cached (after a density or theme change).
    pub fn clear(&mut self) {
        self.map.clear();
        self.count = 0;
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
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
        for i in 0..MAX_ENTRIES + 10 {
            c.get(&ui, &i.to_string(), 12.0, Weight::Regular, WHITE);
        }
        assert!(c.len() <= MAX_ENTRIES);
        assert!(!c.is_empty());
    }
}
