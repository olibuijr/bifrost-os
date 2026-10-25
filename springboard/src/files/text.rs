//! The text preview: a file's lines set in SF Mono, drawn only where they
//! are on screen, with each rendered line cached while it stays nearby.

use crate::font::{Font, Style};
use crate::gfx::{Image, Surface};
use crate::shell::ui::{Ui, SECONDARY, WHITE};
use std::cell::RefCell;
use std::collections::HashMap;

/// Point size of the preview text.
pub const PT: f32 = 13.0;
/// Line pitch as a multiple of the point size.
const PITCH: f32 = 1.35;
const PAD_PT: f32 = 16.0;
/// Rendered lines kept around once the view scrolls on.
const CACHE_LINES: usize = 400;

/// The keys to drop from a full cache: all but the `keep` nearest to `near`, farthest first.
fn far_lines(keys: impl Iterator<Item = usize>, near: usize, keep: usize) -> Vec<usize> {
    let mut keys: Vec<usize> = keys.collect();
    keys.sort_by_key(|k| std::cmp::Reverse(k.abs_diff(near)));
    let drop = keys.len().saturating_sub(keep);
    keys.truncate(drop);
    keys
}

pub struct TextView {
    lines: Vec<String>,
    truncated: bool,
    cache: RefCell<HashMap<usize, Image>>,
}

impl TextView {
    pub fn new(text: &str, truncated: bool) -> Self {
        TextView { lines: text.lines().map(str::to_string).collect(), truncated, cache: RefCell::new(HashMap::new()) }
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn line_h(ui: &Ui) -> f32 {
        (ui.pt(PT) * PITCH).round()
    }

    /// Height of all the lines (plus a note when the file was cut short).
    pub fn content_h(&self, ui: &Ui) -> f32 {
        (self.lines.len() + self.truncated as usize) as f32 * Self::line_h(ui) + 2.0 * ui.pt(PAD_PT)
    }

    /// The range of lines visible with `offset` px scrolled in a viewport `h` px tall.
    pub fn visible(&self, ui: &Ui, offset: f32, h: f32) -> std::ops::Range<usize> {
        let lh = Self::line_h(ui);
        let first = ((offset - ui.pt(PAD_PT)) / lh).floor().max(0.0) as usize;
        let last = (((offset + h - ui.pt(PAD_PT)) / lh).ceil().max(0.0) as usize).min(self.lines.len());
        first.min(last)..last
    }

    /// Renders line `i` into the cache if it is not there. A full cache keeps the `CACHE_LINES / 2` lines nearest `i`.
    fn ensure_rendered(&self, font: &Font, ui: &Ui, i: usize) {
        let mut cache = self.cache.borrow_mut();
        if !cache.contains_key(&i) {
            if cache.len() >= CACHE_LINES {
                let drop = far_lines(cache.keys().copied(), i, CACHE_LINES / 2);
                for k in drop {
                    cache.remove(&k);
                }
            }
            let st = Style { px: ui.pt(PT), embolden: 0.0, color: WHITE, shadow: false };
            cache.insert(i, font.render(&self.lines[i], &st));
        }
    }

    /// Draws the lines into the band `top..bottom` (px) scrolled by `offset`,
    /// `ox` px to the right (for the push transition).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&self, s: &mut Surface, font: &Font, ui: &Ui, ox: f32, top: f32, bottom: f32, offset: f32) {
        let lh = Self::line_h(ui);
        let x = (ox + ui.pt(PAD_PT)) as i32;
        let base = top + ui.pt(PAD_PT) - offset;
        for i in self.visible(ui, offset, bottom - top) {
            let y = base + i as f32 * lh;
            if y + lh < top || y > bottom {
                continue;
            }
            self.ensure_rendered(font, ui, i);
            let cache = self.cache.borrow();
            let img = &cache[&i];
            let visible = img.crop(0, 0, img.w.min((s.w as f32 - x as f32).max(0.0) as usize), img.h);
            s.blit(&visible, x, (y + (lh - img.h as f32) / 2.0) as i32, 255, 255);
        }
        if self.truncated {
            let y = base + self.lines.len() as f32 * lh;
            if y < bottom && y + lh > top {
                let st = Style { px: ui.pt(PT), embolden: 0.0, color: SECONDARY, shadow: false };
                let note = font.render("— preview ends here, the file is longer —", &st);
                s.blit(&note, x, (y + (lh - note.h as f32) / 2.0) as i32, 255, 255);
            }
        }
    }

    pub fn cached(&self) -> usize {
        self.cache.borrow().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mono() -> Font<'static> {
        let data: &'static [u8] = crate::test_support::sfui_mono();
        Font::parse(data).unwrap()
    }

    #[test]
    fn a_full_line_cache_drops_the_lines_farthest_from_the_view() {
        let keys = 0..CACHE_LINES;
        let dropped = far_lines(keys, 300, CACHE_LINES / 2);
        assert_eq!(dropped.len(), CACHE_LINES / 2);
        // the 200 nearest to line 300 are lines 200..400, so the first 200 lines go
        assert!(dropped.iter().all(|k| *k < 200), "only the far lines go");
        assert!(far_lines(0..3, 1, 10).is_empty(), "a cache under the keep count drops nothing");
    }

    #[test]
    fn visible_range_and_lazy_rendering() {
        let ui = crate::shell::tests::ui();
        let text: String = (0..1000).map(|i| format!("line {i}\n")).collect();
        let v = TextView::new(&text, true);
        assert_eq!(v.len(), 1000);
        assert!(!v.is_empty());
        let lh = TextView::line_h(&ui);
        assert_eq!(v.visible(&ui, 0.0, 10.0 * lh), 0..10, "the top padding pushes line 10 off a ten-line viewport");
        let r = v.visible(&ui, 100.0 * lh + ui.pt(PAD_PT), 10.0 * lh);
        assert_eq!(r, 100..110);
        assert_eq!(v.visible(&ui, 1e6, 100.0), 1000..1000);
        assert_eq!(v.content_h(&ui), 1001.0 * lh + 2.0 * ui.pt(PAD_PT));
        let font = mono();
        let mut buf = vec![0u32; 600 * 400];
        v.draw(&mut Surface { w: 600, h: 400, stride: 600, px: &mut buf, y0: 0 }, &font, &ui, 0.0, 50.0, 400.0, 0.0);
        assert!(buf.contains(&WHITE));
        assert!(v.cached() > 5 && v.cached() < 30, "only the visible lines are rendered: {}", v.cached());
        for k in 0..30 {
            v.draw(&mut Surface { w: 600, h: 400, stride: 600, px: &mut buf, y0: 0 }, &font, &ui, 0.0, 50.0, 400.0, k as f32 * 20.0 * lh);
        }
        assert!(v.cached() <= CACHE_LINES, "the cache is bounded");
        let end = v.content_h(&ui) - 350.0;
        v.draw(&mut Surface { w: 600, h: 400, stride: 600, px: &mut buf, y0: 0 }, &font, &ui, 0.0, 50.0, 400.0, end);
        assert!(buf.contains(&SECONDARY), "the truncation note shows at the end");
    }
}
