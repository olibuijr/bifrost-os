//! Cell grid geometry, colour mapping, a glyph coverage cache and row painting.

use crate::font::{Font, Style};
use crate::gfx::{pack, Image};
use termcore::cell::{attr, Cell};
use termcore::palette::{cell_colors_in, Rgb, Theme, THEMES};
use termcore::term::CursorShape;
use std::collections::HashMap;

/// The the terminal core dark theme: light grey on near-black.
pub const THEME: &Theme = &THEMES[0];

/// Opaque packed pixel for an emulator colour.
pub fn px(c: Rgb) -> u32 {
    pack(c.0, c.1, c.2, 255)
}

/// Pixel size of one cell and where text sits inside it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub cw: i32,
    pub ch: i32,
    /// baseline offset from the cell top
    pub baseline: i32,
    /// inner margin around the grid
    pub pad: i32,
}

impl Metrics {
    pub fn new(font: &Font, px: f32) -> Self {
        let k = px / font.upem;
        let cw = (font.advance(font.glyph('M')) * k).round().max(1.0) as i32;
        let ch = ((font.ascent - font.descent) * k).ceil().max(1.0) as i32;
        Metrics { cw, ch, baseline: (font.ascent * k).round() as i32, pad: cw / 2 }
    }

    /// Columns and rows that fit a `w` x `h` px area (at least 1 x 1).
    pub fn grid(&self, w: i32, h: i32) -> (usize, usize) {
        let fit = |len: i32, cell: i32| ((len - 2 * self.pad) / cell).max(1) as usize;
        (fit(w, self.cw), fit(h, self.ch))
    }

    /// Top-left pixel of cell (`col`, `row`) relative to the area origin.
    pub fn cell_origin(&self, col: usize, row: usize) -> (i32, i32) {
        (self.pad + col as i32 * self.cw, self.pad + row as i32 * self.ch)
    }

    /// Stroke thickness for underline, strike-through and the bar cursor.
    pub fn line(&self) -> i32 {
        (self.ch / 16).max(1)
    }
}

/// A rendered glyph as an alpha coverage mask, offset from the cell origin.
pub struct Glyph {
    pub w: usize,
    pub h: usize,
    pub dx: i32,
    pub dy: i32,
    pub mask: Vec<u8>,
}

/// Renders each (char, bold) of one font and size once and keeps the coverage mask.
pub struct GlyphCache<'a> {
    pub font: Font<'a>,
    px: f32,
    glyphs: HashMap<(char, bool), Glyph>,
}

impl<'a> GlyphCache<'a> {
    pub fn new(font: Font<'a>, px: f32) -> Self {
        GlyphCache { font, px, glyphs: HashMap::new() }
    }

    pub fn get(&mut self, c: char, bold: bool) -> &Glyph {
        let (font, px) = (&self.font, self.px);
        self.glyphs.entry((c, bold)).or_insert_with(|| {
            let embolden = if bold { px * 0.04 } else { 0.0 };
            let img = font.render(&c.to_string(), &Style { px, embolden, color: pack(255, 255, 255, 255), shadow: false });
            let pad = 2 + embolden.ceil() as i32;
            Glyph { w: img.w, h: img.h, dx: -pad, dy: -pad, mask: img.px.iter().map(|p| (p >> 24) as u8).collect() }
        })
    }
}

/// Blends `fg` over `bg` by coverage `a` (opaque pixels).
pub fn blend(bg: u32, fg: u32, a: u8) -> u32 {
    match a {
        0 => bg,
        255 => fg,
        a => {
            let (a, b) = (a as u32, 255 - a as u32);
            let ch = |s: u32| ((((bg >> s) & 255) * b + ((fg >> s) & 255) * a + 127) / 255) << s;
            ch(0) | ch(8) | ch(16) | 0xff00_0000
        }
    }
}

/// Fills a rectangle of `img`, clipped.
fn fill(img: &mut Image, x: i32, y: i32, w: i32, h: i32, c: u32) {
    let (x0, y0) = (x.max(0) as usize, y.max(0) as usize);
    let x1 = ((x + w).max(0) as usize).min(img.w);
    let y1 = ((y + h).max(0) as usize).min(img.h);
    for row in y0..y1 {
        if x0 < x1 {
            img.px[row * img.w + x0..row * img.w + x1].fill(c);
        }
    }
}

/// Where the cursor is drawn on a row, and how.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cursor {
    pub col: usize,
    pub shape: CursorShape,
}

/// Paints screen row `row` of `cells` (padded to `cols` with blanks) into `img`.
/// Glyphs are clipped to the row band so rows can be redrawn independently.
pub fn paint_row(img: &mut Image, cache: &mut GlyphCache, appearance: (&Metrics, &Theme), row: usize, cells: &[Cell], cols: usize, cursor: Option<Cursor>) {
    let (m, theme) = appearance;
    let blank = Cell::default();
    let cell_at = |x: usize| cells.get(x).unwrap_or(&blank);
    let block = |x: usize| cursor.is_some_and(|c| c.col == x && c.shape == CursorShape::Block);
    let (_, top) = m.cell_origin(0, row);
    fill(img, 0, top, img.w as i32, m.ch, px(theme.bg));
    for x in 0..cols {
        let (_, bg) = cell_colors_in(theme, cell_at(x), block(x));
        let (cx, cy) = m.cell_origin(x, row);
        fill(img, cx, cy, m.cw, m.ch, px(bg));
    }
    for x in 0..cols {
        let c = cell_at(x);
        let (fg, _) = cell_colors_in(theme, c, block(x));
        let (cx, cy) = m.cell_origin(x, row);
        let span = if c.attrs & attr::WIDE != 0 { 2 * m.cw } else { m.cw };
        if c.attrs & attr::SPACER == 0 && c.ch != ' ' {
            let g = cache.get(c.ch, c.attrs & attr::BOLD != 0);
            draw_mask(img, g, cx, cy, top, top + m.ch, px(fg));
        }
        let t = m.line();
        if c.attrs & attr::UNDERLINE != 0 {
            fill(img, cx, cy + m.baseline + t, span, t, px(fg));
        }
        if c.attrs & attr::STRIKE != 0 {
            fill(img, cx, cy + m.baseline * 2 / 3, span, t, px(fg));
        }
    }
    if let Some(Cursor { col, shape }) = cursor {
        let (fg, _) = cell_colors_in(theme, cell_at(col), false);
        let (cx, cy) = m.cell_origin(col, row);
        let t = 2 * m.line();
        match shape {
            CursorShape::Block => {}
            CursorShape::Underline => fill(img, cx, cy + m.ch - t, m.cw, t, px(fg)),
            CursorShape::Bar => fill(img, cx, cy, t, m.ch, px(fg)),
        }
    }
}

/// Blends a glyph mask in `fg` at cell origin (`x`, `y`), clipped to rows `y0..y1`.
fn draw_mask(img: &mut Image, g: &Glyph, x: i32, y: i32, y0: i32, y1: i32, fg: u32) {
    for gy in 0..g.h as i32 {
        let py = y + g.dy + gy;
        if py < y0.max(0) || py >= y1.min(img.h as i32) {
            continue;
        }
        for gx in 0..g.w as i32 {
            let pxx = x + g.dx + gx;
            let a = g.mask[(gy * g.w as i32 + gx) as usize];
            if a != 0 && pxx >= 0 && (pxx as usize) < img.w {
                let d = &mut img.px[py as usize * img.w + pxx as usize];
                *d = blend(*d, fg, a);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use termcore::cell::Color;

    fn mono() -> Vec<u8> {
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/SFUIMono.ttf")).expect("assets/SFUIMono.ttf (run tools/pack.py)")
    }

    fn metrics() -> Metrics {
        Metrics { cw: 10, ch: 20, baseline: 15, pad: 5 }
    }

    #[test]
    fn mono_font_cells_are_uniform() {
        let data = mono();
        let f = Font::parse(&data).unwrap();
        let m = Metrics::new(&f, 26.0);
        assert!(m.cw > 10 && m.cw < 20, "{m:?}");
        assert!(m.ch > m.cw && m.baseline < m.ch);
        assert_eq!(f.advance(f.glyph('i')), f.advance(f.glyph('W')));
        assert_eq!(m.pad, m.cw / 2);
    }

    #[test]
    fn grid_fits_inside_padding() {
        let m = metrics();
        assert_eq!(m.grid(1210, 410), (120, 20));
        assert_eq!(m.grid(1209, 409), (119, 19));
        assert_eq!(m.grid(3, 3), (1, 1));
    }

    #[test]
    fn cell_origin_steps_by_cell_size() {
        let m = metrics();
        assert_eq!(m.cell_origin(0, 0), (5, 5));
        assert_eq!(m.cell_origin(3, 2), (35, 45));
    }

    #[test]
    fn colours_map_to_opaque_pixels() {
        assert_eq!(px((1, 2, 3)), 0xff03_0201);
        let red = Cell { fg: Color::Indexed(1), ..Cell::default() };
        assert_eq!(cell_colors_in(THEME, &red, false).0, THEME.ansi[1]);
    }

    #[test]
    fn blend_interpolates_channels() {
        let (black, white) = (pack(0, 0, 0, 255), pack(255, 255, 255, 255));
        assert_eq!(blend(black, white, 0), black);
        assert_eq!(blend(black, white, 255), white);
        assert_eq!(blend(black, white, 128), pack(128, 128, 128, 255));
    }

    #[test]
    fn glyph_cache_renders_each_char_and_weight_once() {
        let data = mono();
        let mut c = GlyphCache::new(Font::parse(&data).unwrap(), 26.0);
        let ink = c.get('A', false).mask.iter().map(|&a| a as u32).sum::<u32>();
        c.get('A', false);
        assert_eq!(c.glyphs.len(), 1);
        let bold = c.get('A', true).mask.iter().map(|&a| a as u32).sum::<u32>();
        assert_eq!(c.glyphs.len(), 2);
        assert!(bold > ink);
    }

    fn painted(cells: &[Cell], cursor: Option<Cursor>) -> (Image, Metrics) {
        let data = mono();
        let f = Font::parse(&data).unwrap();
        let m = Metrics::new(&f, 20.0);
        let mut img = Image::new((2 * m.pad + 4 * m.cw) as usize, (2 * m.pad + 2 * m.ch) as usize);
        let mut cache = GlyphCache::new(f, 20.0);
        paint_row(&mut img, &mut cache, (&m, THEME), 1, cells, 4, cursor);
        (img, m)
    }

    fn cell_pixels(img: &Image, m: &Metrics, col: usize, row: usize) -> Vec<u32> {
        let (x, y) = m.cell_origin(col, row);
        (y..y + m.ch).flat_map(|py| (x..x + m.cw).map(move |pxx| img.px[py as usize * img.w + pxx as usize])).collect()
    }

    #[test]
    fn paint_row_draws_background_and_ink_only_in_its_band() {
        let cells = [Cell { ch: 'H', ..Cell::default() }, Cell { bg: Color::Indexed(4), ..Cell::default() }];
        let (img, m) = painted(&cells, None);
        let h = cell_pixels(&img, &m, 0, 1);
        assert!(h.iter().filter(|&&p| p != px(THEME.bg)).count() > 10, "glyph ink");
        assert!(cell_pixels(&img, &m, 1, 1).iter().all(|&p| p == px(THEME.ansi[4])));
        assert!(cell_pixels(&img, &m, 3, 1).iter().all(|&p| p == px(THEME.bg)), "short rows pad with blanks");
        assert!(cell_pixels(&img, &m, 0, 0).iter().all(|&p| p == 0), "row above untouched");
    }

    #[test]
    fn block_cursor_inverts_and_bar_cursor_draws_a_stroke() {
        let (img, m) = painted(&[], Some(Cursor { col: 2, shape: CursorShape::Block }));
        assert!(cell_pixels(&img, &m, 2, 1).iter().all(|&p| p == px(THEME.fg)));
        let (img, m) = painted(&[], Some(Cursor { col: 2, shape: CursorShape::Bar }));
        let cell = cell_pixels(&img, &m, 2, 1);
        assert_eq!(cell[0], px(THEME.fg));
        assert_eq!(cell[m.cw as usize - 1], px(THEME.bg));
    }

    #[test]
    fn inverse_and_underline_and_wide() {
        let cells = [
            Cell { attrs: attr::INVERSE, ..Cell::default() },
            Cell { attrs: attr::UNDERLINE, ..Cell::default() },
            Cell { ch: 'W', attrs: attr::WIDE, ..Cell::default() },
            Cell { ch: ' ', attrs: attr::SPACER, ..Cell::default() },
        ];
        let (img, m) = painted(&cells, None);
        assert!(cell_pixels(&img, &m, 0, 1).iter().all(|&p| p == px(THEME.fg)));
        let under = cell_pixels(&img, &m, 1, 1);
        let y = (m.baseline + m.line()) as usize;
        assert_eq!(under[y * m.cw as usize], px(THEME.fg));
        assert_eq!(under[0], px(THEME.bg));
        assert!(cell_pixels(&img, &m, 2, 1).iter().any(|&p| p != px(THEME.bg)), "wide char drew its glyph");
    }
}
