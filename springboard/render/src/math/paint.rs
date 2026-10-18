//! Draws a [`Layout`] through a [`Canvas`]: each glyph as a one-character text run, each rule as a filled rectangle.

use crate::canvas::{with_font, Canvas, RectF, TextRun};

use super::Layout;
use super::Glyph;

/// The padding `render_placed` puts around a rendered run: the pen starts this far inside the image.
const RUN_PAD: f32 = 2.0;

/// Top-left corner of the text run that draws `g` with its baseline at `origin + (g.x, g.y)`, where
/// `ascent` is the face ascent at the glyph size. The run's image has its baseline `RUN_PAD + ascent` below
/// its top edge, and its pen starts `RUN_PAD` in from the left.
pub fn run_origin(g: &Glyph, origin: (f32, f32), ascent: f32) -> (f32, f32) {
    (origin.0 + g.x - RUN_PAD, origin.1 + g.y - RUN_PAD - ascent)
}

/// Paints `layout` with its baseline origin at `origin` (device pixels, y down), in `face` and `color`.
pub fn paint(layout: &Layout, canvas: &mut dyn Canvas, origin: (f32, f32), face: u64, color: u32) {
    for g in &layout.glyphs {
        let ascent = with_font(face, |f| f.line_metrics(g.size).ascent).unwrap_or(g.size * 0.8);
        let (x, y) = run_origin(g, origin, ascent);
        let run = TextRun { face, text: g.ch.to_string(), px: g.size, vars: Vec::new(), x, y };
        canvas.text(&run, color);
    }
    for r in &layout.rules {
        canvas.fill_rect(RectF::new(origin.0 + r.x, origin.1 + r.y, r.w, r.h), color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_origin_puts_the_baseline_on_the_glyph_baseline() {
        let g = Glyph { ch: 'x', x: 10.0, y: -4.0, size: 16.0 };
        let (x, y) = run_origin(&g, (100.0, 50.0), 12.0);
        assert_eq!((x, y), (108.0, 50.0 - 4.0 - 2.0 - 12.0));
        // The baseline of the run image is y + pad + ascent, which is the glyph baseline.
        assert_eq!(y + RUN_PAD + 12.0, 50.0 - 4.0);
    }
}
