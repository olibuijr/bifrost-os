//! Rendered-text cache, so widgets that redraw every frame (keyboard,
//! pickers, stopwatch digits) rasterise each string once.

use crate::ui::{Ui, Weight};
use crate::gfx::{Image, Surface};
use std::collections::HashMap;

/// One cached string: its image and the tick of its last use.
struct Entry {
    img: Image,
    tick: u64,
}

/// Style key: (pt x10, weight, colour).
type Style = (u32, u8, u32);

pub struct TextCache {
    /// style -> text -> image, so a hit never allocates a key
    map: HashMap<Style, HashMap<String, Entry>>,
    count: usize,
    /// pixel bytes held by the cached images
    bytes: usize,
    /// the most pixel bytes kept; the least recently used strings go first
    budget: usize,
    /// advances on every lookup, so the smallest tick is the least recently used
    tick: u64,
    /// `emoji::generation()` the entries were rendered under
    emoji_gen: u32,
    /// `Ui::text_epoch()` the entries were rendered under
    epoch: u64,
}

/// Entries kept before the least recently used go (like the glyph cache).
const MAX_ENTRIES: usize = 512;
/// Pixel bytes kept when the cache is not sized to a screen.
const DEFAULT_BUDGET: usize = 8 * 1024 * 1024;
/// Bounds of a screen-sized budget: the floor keeps small screens useful, the ceiling caps the memory.
const MIN_BUDGET: usize = 4 * 1024 * 1024;
const MAX_BUDGET: usize = 32 * 1024 * 1024;

/// The pixel budget for a screen of `w` x `h` px: two screens of RGBA text, within `MIN_BUDGET..=MAX_BUDGET`.
pub fn budget_for_screen(w: usize, h: usize) -> usize {
    (w.saturating_mul(h).saturating_mul(4).saturating_mul(2)).clamp(MIN_BUDGET, MAX_BUDGET)
}

impl Default for TextCache {
    fn default() -> Self {
        TextCache::with_budget(DEFAULT_BUDGET)
    }
}

impl TextCache {
    /// An empty cache holding at most `bytes` of pixels.
    pub fn with_budget(bytes: usize) -> Self {
        TextCache { map: HashMap::new(), count: 0, bytes: 0, budget: bytes, tick: 0, emoji_gen: 0, epoch: 0 }
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
        self.bytes
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
        let style: Style = ((pt * 10.0) as u32, weight as u8, color);
        self.tick += 1;
        let tick = self.tick;
        let hit = self.map.get_mut(&style).and_then(|m| m.get_mut(text)).map(|e| e.tick = tick).is_some();
        if !hit {
            let img = ui.text(text, pt, weight, color);
            self.bytes += img.px.len() * 4;
            self.count += 1;
            self.map.entry(style).or_default().insert(text.to_string(), Entry { img, tick });
            self.evict_to_limits(style, text);
        }
        &self.map[&style][text].img
    }

    /// Drops the least recently used strings until both limits hold. The string just inserted is never dropped.
    fn evict_to_limits(&mut self, keep_style: Style, keep_text: &str) {
        while self.count > 1 && (self.count > MAX_ENTRIES || self.bytes > self.budget) {
            // the oldest entry other than the one just inserted
            let mut oldest: Option<(u64, Style, &str)> = None;
            for (style, m) in &self.map {
                for (text, e) in m {
                    if *style == keep_style && text == keep_text {
                        continue;
                    }
                    if oldest.map_or(true, |(t, _, _)| e.tick < t) {
                        oldest = Some((e.tick, *style, text.as_str()));
                    }
                }
            }
            let Some((_, style, text)) = oldest else { break };
            let text = text.to_string();
            let Some(m) = self.map.get_mut(&style) else { break };
            if let Some(e) = m.remove(&text) {
                self.bytes -= e.img.px.len() * 4;
                self.count -= 1;
            }
            if m.is_empty() {
                self.map.remove(&style);
            }
        }
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
        self.bytes = 0;
    }

    pub fn len(&self) -> usize {
        self.count
    }

    /// True when some style caches `text`.
    #[cfg(test)]
    fn holds(&self, text: &str) -> bool {
        self.map.values().any(|m| m.contains_key(text))
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

    #[test]
    fn eviction_drops_the_least_recently_used_string_not_everything() {
        let _emoji = crate::tests::emoji_lock();
        let ui = crate::tests::ui();
        let mut c = TextCache::default();
        c.get(&ui, "keep", 12.0, Weight::Regular, WHITE);
        for i in 0..MAX_ENTRIES - 1 {
            c.get(&ui, &format!("fill{i}"), 12.0, Weight::Regular, WHITE);
            // touching "keep" makes it the most recently used string every time
            c.get(&ui, "keep", 12.0, Weight::Regular, WHITE);
        }
        assert_eq!(c.len(), MAX_ENTRIES);
        c.get(&ui, "one more", 12.0, Weight::Regular, WHITE);
        assert_eq!(c.len(), MAX_ENTRIES, "one entry went, the rest stayed");
        assert!(c.holds("keep"), "the recently used string survives");
        assert!(!c.holds("fill0"), "the oldest string went first");
    }

    #[test]
    fn the_byte_budget_bounds_the_cache_and_is_sized_to_the_screen() {
        let _emoji = crate::tests::emoji_lock();
        let ui = crate::tests::ui();
        let mut c = TextCache::with_budget(64 * 1024);
        for i in 0..200 {
            c.get(&ui, &format!("budget {i}"), 13.0, Weight::Regular, WHITE);
            assert!(c.bytes() <= c.budget() || c.len() == 1, "{} bytes over a {} budget", c.bytes(), c.budget());
        }
        assert!(c.len() >= 1);
        assert_eq!(budget_for_screen(1920, 1080), 2 * 1920 * 1080 * 4);
        assert_eq!(budget_for_screen(16, 16), MIN_BUDGET, "small screens get the floor");
        assert_eq!(budget_for_screen(7680, 4320), MAX_BUDGET, "large screens get the ceiling");
        assert_eq!(TextCache::for_screen(1920, 1080).budget(), budget_for_screen(1920, 1080));
    }
}
