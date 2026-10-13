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
    /// advance widths (`Ui::measure`) per (pt bits, weight) -> text -> px
    widths: HashMap<(u32, u8), HashMap<String, f32>>,
    /// fitted strings (`Ui::fit`) per (pt bits, max px bits) -> text -> fitted text
    fits: HashMap<(u32, u32), HashMap<String, String>>,
    /// widths and fits held, bounded like the images
    metrics: usize,
}

/// Entries kept before the cache starts over (like the glyph cache).
const MAX_ENTRIES: usize = 512;
/// Widths and fits kept before they start over; they are cheap, so there are more of them.
const MAX_METRICS: usize = 2048;

impl TextCache {
    /// Drops the entries rendered under another emoji font or Dynamic Type, bold or contrast setting.
    fn sync(&mut self, ui: &Ui) {
        // a reloaded emoji font invalidates every cached image that may hold emoji
        let g = crate::emoji::generation();
        let epoch = ui.text_epoch();
        if self.emoji_gen != g || self.epoch != epoch {
            self.emoji_gen = g;
            self.epoch = epoch;
            self.clear();
        }
    }

    /// `Ui::measure` of `text`, memoised: a bar or a menu that lays out its labels every frame shapes each one once.
    pub fn width(&mut self, ui: &Ui, text: &str, pt: f32, weight: Weight) -> f32 {
        self.sync(ui);
        let style = (pt.to_bits(), weight as u8);
        if let Some(&w) = self.widths.get(&style).and_then(|m| m.get(text)) {
            return w;
        }
        let w = ui.measure(text, pt, weight);
        self.room_for_metric();
        self.widths.entry(style).or_default().insert(text.to_string(), w);
        self.metrics += 1;
        w
    }

    /// `Ui::fit` of `text` into `max_px`, memoised like `width`.
    pub fn fit(&mut self, ui: &Ui, text: &str, pt: f32, max_px: f32) -> String {
        self.sync(ui);
        let style = (pt.to_bits(), max_px.to_bits());
        if let Some(s) = self.fits.get(&style).and_then(|m| m.get(text)) {
            return s.clone();
        }
        let s = ui.fit(text, pt, max_px);
        self.room_for_metric();
        self.fits.entry(style).or_default().insert(text.to_string(), s.clone());
        self.metrics += 1;
        s
    }

    /// Starts the widths and fits over once they fill up.
    fn room_for_metric(&mut self) {
        if self.metrics >= MAX_METRICS {
            self.widths.clear();
            self.fits.clear();
            self.metrics = 0;
        }
    }

    /// `text` at `pt` points in `weight` and `color`, rendering on a miss.
    pub fn get(&mut self, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32) -> &Image {
        self.sync(ui);
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
        self.map.clear();
        self.count = 0;
        self.widths.clear();
        self.fits.clear();
        self.metrics = 0;
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
    fn widths_and_fits_match_the_ui_and_are_remembered() {
        let _emoji = crate::tests::emoji_lock();
        let ui = crate::tests::ui();
        let mut c = TextCache::default();
        let w = c.width(&ui, "Wi-Fi", 13.0, Weight::Semibold);
        assert_eq!(w, ui.measure("Wi-Fi", 13.0, Weight::Semibold));
        assert_eq!(c.width(&ui, "Wi-Fi", 13.0, Weight::Semibold), w);
        assert_eq!(c.metrics, 1, "the second width is a hit");
        let fit = c.fit(&ui, "a very long network name indeed", 13.0, 40.0);
        assert_eq!(fit, ui.fit("a very long network name indeed", 13.0, 40.0));
        assert_eq!(c.fit(&ui, "a very long network name indeed", 13.0, 40.0), fit);
        assert_eq!(c.metrics, 3);
        assert_eq!(c.len(), 0, "widths and fits leave the images alone");
    }

    #[test]
    fn widths_follow_dynamic_type_and_the_metric_cache_is_bounded() {
        let _emoji = crate::tests::emoji_lock();
        let mut ui = crate::tests::ui();
        let mut c = TextCache::default();
        let normal = c.width(&ui, "Settings", 13.0, Weight::Regular);
        ui.text_scale = 1.5;
        let larger = c.width(&ui, "Settings", 13.0, Weight::Regular);
        assert!(larger > normal, "Dynamic Type must not serve the old width ({larger} vs {normal})");
        for i in 0..MAX_METRICS + 10 {
            c.width(&ui, &i.to_string(), 12.0, Weight::Regular);
        }
        assert!(c.metrics <= MAX_METRICS);
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
