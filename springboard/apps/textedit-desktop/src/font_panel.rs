//! Format > Font > Show Fonts (Cmd+T) and Show Colors (Shift+Cmd+C): the font panel (`NSFontPanel`), with columns for the
//! family, the typeface and the size, and the colour panel (`NSColorPanel`) with the document's palette. Both float over
//! the page like the app's other panels, apply a choice to the selection at once (as the shared panels do), and close
//! with Done or Esc. The family column lists every family a document can use (`text::families`: the three base families,
//! then the installed ones), scrolls with its arrows, Up and Down, and jumps to a family by typing its name.
//!
//! Layout and hit testing are pure (tested here); drawing goes through `Canvas`.

use crate::text::{families, COLORS, SIZES};
use crate::format_bar::face_label;
use crate::view::{blit, text_color};
use bifrost_app::controls::Rect;
use bifrost_app::kit::canvas::{Canvas, RectF};
use bifrost_app::kit::ui::{Ui, Weight, DETAIL_BG, FIELD_BG, LABEL, LABEL_3, LINK, MAC_BODY_PT, MAC_FOOTNOTE_PT, SELECTION, SEPARATOR_C, WHITE};

/// Family rows shown at once.
pub const ROWS: usize = 9;
pub const ROW_H: f32 = 22.0;
const PAD: f32 = 16.0;
const TITLE_H: f32 = 28.0;
const HEAD_H: f32 = 20.0;
const BUTTONS_H: f32 = 44.0;
const BUTTON_W: f32 = 72.0;
const BUTTON_H: f32 = 26.0;
const GAP: f32 = 8.0;
const MAX_W: f32 = 560.0;
const TOP_GAP: f32 = 12.0;
/// Typing within this many ms continues the family name being typed.
const TYPE_SELECT_MS: f64 = 1000.0;
/// One colour swatch of the colour panel, and the gap between swatches.
const SWATCH: f32 = 36.0;
const SWATCH_GAP: f32 = 10.0;
const RADIUS: f32 = 6.0;

/// Where the font panel's parts are, in window points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontsLayout {
    pub frame: Rect,
    pub title_y: f32,
    pub family: Rect,
    pub face: Rect,
    pub size: Rect,
    /// the family column's scroll arrows, in its header
    pub up: Rect,
    pub down: Rect,
    pub done: Rect,
}

/// The font panel for a window `width` points wide whose page starts at `body_top`.
pub fn fonts_layout(width: f32, body_top: f32) -> FontsLayout {
    let w = (width - 2.0 * PAD).clamp(0.0, MAX_W);
    let x = ((width - w) / 2.0).max(0.0);
    let y = body_top + TOP_GAP;
    let inner = (w - 2.0 * PAD - 2.0 * GAP).max(0.0);
    let cols_y = y + PAD + TITLE_H + HEAD_H;
    let cols_h = ROW_H * ROWS as f32;
    let (fw, tw) = (inner * 0.5, inner * 0.3);
    let family = Rect { x: x + PAD, y: cols_y, w: fw, h: cols_h };
    let face = Rect { x: family.x + fw + GAP, y: cols_y, w: tw, h: cols_h };
    let size = Rect { x: face.x + tw + GAP, y: cols_y, w: (inner - fw - tw).max(0.0), h: cols_h };
    let up = Rect { x: family.x + family.w - 2.0 * HEAD_H, y: cols_y - HEAD_H, w: HEAD_H, h: HEAD_H };
    let down = Rect { x: family.x + family.w - HEAD_H, y: cols_y - HEAD_H, w: HEAD_H, h: HEAD_H };
    let h = PAD + TITLE_H + HEAD_H + cols_h + BUTTONS_H;
    let done = Rect { x: x + w - PAD - BUTTON_W, y: cols_y + cols_h + (BUTTONS_H - BUTTON_H) / 2.0, w: BUTTON_W, h: BUTTON_H };
    FontsLayout { frame: Rect { x, y, w, h }, title_y: y + PAD, family, face, size, up, down, done }
}

/// What a press on the font panel chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontsHit {
    /// a family (index into `families()`), a typeface (0 Regular, 1 Bold, 2 Italic, 3 Bold Italic), a size (index into `SIZES`)
    Family(u8),
    Face(u8),
    Size(u8),
    Up,
    Down,
    Done,
}

/// The font panel while it is open: the first family row shown, and the name being typed to jump to a family.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fonts {
    pub top: usize,
    typed: String,
    typed_at: f64,
}

impl Fonts {
    /// The panel opened on the family `current` (the selection's), scrolled to show it.
    pub fn new(current: u8) -> Self {
        let mut f = Fonts::default();
        f.reveal(current as usize);
        f
    }

    fn max_top() -> usize {
        families().len().saturating_sub(ROWS)
    }

    /// Scrolls so family `i` is in view.
    pub fn reveal(&mut self, i: usize) {
        if i < self.top {
            self.top = i;
        } else if i >= self.top + ROWS {
            self.top = i + 1 - ROWS;
        }
        self.top = self.top.min(Self::max_top());
    }

    /// Scrolls the family column by `rows` (negative is up).
    pub fn scroll(&mut self, rows: i32) {
        self.top = (self.top as i64 + rows as i64).clamp(0, Self::max_top() as i64) as usize;
    }

    /// Type-select: the first family whose name starts with what was typed (keys within a second continue the name).
    pub fn type_select(&mut self, s: &str, t: f64) -> Option<u8> {
        if t - self.typed_at > TYPE_SELECT_MS {
            self.typed.clear();
        }
        self.typed_at = t;
        self.typed.push_str(&s.to_lowercase());
        let i = families().iter().position(|f| f.to_lowercase().starts_with(&self.typed))?;
        self.reveal(i);
        Some(i as u8)
    }
}

/// What is under (x, y) on the font panel.
pub fn fonts_hit(l: &FontsLayout, panel: &Fonts, x: f32, y: f32) -> Option<FontsHit> {
    let row = |r: &Rect| ((y - r.y) / ROW_H) as usize;
    if l.done.contains(x, y) {
        return Some(FontsHit::Done);
    }
    if l.up.contains(x, y) {
        return Some(FontsHit::Up);
    }
    if l.down.contains(x, y) {
        return Some(FontsHit::Down);
    }
    if l.family.contains(x, y) {
        let i = panel.top + row(&l.family);
        return (i < families().len()).then_some(FontsHit::Family(i as u8));
    }
    if l.face.contains(x, y) {
        let i = row(&l.face);
        return (i < 4).then_some(FontsHit::Face(i as u8));
    }
    if l.size.contains(x, y) {
        let i = row(&l.size);
        return (i < SIZES.len()).then_some(FontsHit::Size(i as u8));
    }
    None
}

/// The colour panel's parts, in window points.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorsLayout {
    pub frame: Rect,
    pub title_y: f32,
    /// one swatch per palette colour (index into `COLORS`)
    pub swatches: Vec<Rect>,
    pub done: Rect,
}

pub fn colors_layout(width: f32, body_top: f32) -> ColorsLayout {
    let per_row = 4usize;
    let rows = COLORS.len().div_ceil(per_row);
    let w = (PAD * 2.0 + per_row as f32 * SWATCH + (per_row - 1) as f32 * SWATCH_GAP + 2.0 * PAD).min((width - 2.0 * PAD).max(0.0));
    let x = ((width - w) / 2.0).max(0.0);
    let y = body_top + TOP_GAP;
    let grid_w = per_row as f32 * SWATCH + (per_row - 1) as f32 * SWATCH_GAP;
    let gx = x + (w - grid_w) / 2.0;
    let gy = y + PAD + TITLE_H;
    let swatches = (0..COLORS.len())
        .map(|i| Rect { x: gx + (i % per_row) as f32 * (SWATCH + SWATCH_GAP), y: gy + (i / per_row) as f32 * (SWATCH + SWATCH_GAP), w: SWATCH, h: SWATCH })
        .collect();
    let grid_h = rows as f32 * SWATCH + (rows - 1) as f32 * SWATCH_GAP;
    let done = Rect { x: x + w - PAD - BUTTON_W, y: gy + grid_h + (BUTTONS_H - BUTTON_H) / 2.0, w: BUTTON_W, h: BUTTON_H };
    ColorsLayout { frame: Rect { x, y, w, h: PAD + TITLE_H + grid_h + BUTTONS_H }, title_y: y + PAD, swatches, done }
}

/// What a press on the colour panel chose: a palette colour (`Some(index)`), Done (`None`), or nothing.
pub fn colors_hit(l: &ColorsLayout, x: f32, y: f32) -> Option<Option<u8>> {
    if l.done.contains(x, y) {
        return Some(None);
    }
    l.swatches.iter().position(|r| r.contains(x, y)).map(|i| Some(i as u8))
}

fn px(ui: &Ui, r: Rect) -> RectF {
    RectF::new(ui.pt(r.x).floor(), ui.pt(r.y).floor(), ui.pt(r.w).ceil(), ui.pt(r.h).ceil())
}

/// `text` at the left of `r` (`inset` points in), centred vertically, cut to fit.
fn text_in(c: &mut dyn Canvas, ui: &Ui, r: Rect, inset: f32, text: &str, pt: f32, weight: Weight, color: u32) {
    let fitted = ui.fit(text, pt, (ui.pt(r.w) - 2.0 * ui.pt(inset)).max(0.0));
    let img = ui.text(&fitted, pt, weight, color);
    let p = px(ui, r);
    blit(c, &img, p.x + ui.pt(inset), p.y + (p.h - img.h as f32) / 2.0);
}

fn frame(c: &mut dyn Canvas, ui: &Ui, r: Rect) {
    let f = px(ui, r);
    c.fill_rrect(f, ui.pt(RADIUS), DETAIL_BG.now());
    c.stroke_rrect(f, ui.pt(RADIUS), ui.pt(1.0).max(1.0), SEPARATOR_C.now());
}

fn done_button(c: &mut dyn Canvas, ui: &Ui, r: Rect) {
    c.fill_rrect(px(ui, r), ui.pt(RADIUS), LINK);
    let img = ui.text("Done", MAC_BODY_PT, Weight::Bold, WHITE);
    let p = px(ui, r);
    blit(c, &img, p.x + (p.w - img.w as f32) / 2.0, p.y + (p.h - img.h as f32) / 2.0);
}

/// One column: its rows, the chosen one on a selection plate.
fn column(c: &mut dyn Canvas, ui: &Ui, r: Rect, rows: &[&str], chosen: Option<usize>) {
    c.fill_rrect(px(ui, r), ui.pt(RADIUS), FIELD_BG.now());
    for (i, label) in rows.iter().enumerate() {
        let row = Rect { x: r.x + 2.0, y: r.y + ROW_H * i as f32, w: r.w - 4.0, h: ROW_H };
        let on = chosen == Some(i);
        if on {
            c.fill_rrect(px(ui, Rect { y: row.y + 1.0, h: row.h - 2.0, ..row }), ui.pt(4.0), SELECTION.now());
        }
        text_in(c, ui, row, 6.0, label, MAC_BODY_PT, Weight::Regular, if on { WHITE } else { LABEL.now() });
    }
}

/// Draws the font panel. `font`, `face` and `size` are the selection's family index, typeface (0 to 3) and point size.
pub fn draw_fonts(c: &mut dyn Canvas, ui: &Ui, l: &FontsLayout, panel: &Fonts, font: u8, face: u8, size: f32) {
    frame(c, ui, l.frame);
    text_in(c, ui, Rect { x: l.frame.x + PAD, y: l.title_y, w: l.frame.w - 2.0 * PAD, h: TITLE_H }, 0.0, "Fonts", MAC_BODY_PT, Weight::Bold, LABEL.now());
    for (r, head) in [(l.family, "Family"), (l.face, "Typeface"), (l.size, "Size")] {
        text_in(c, ui, Rect { y: r.y - HEAD_H, h: HEAD_H, ..r }, 4.0, head, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_3.now());
    }
    let all = families();
    let can_up = panel.top > 0;
    let can_down = panel.top + ROWS < all.len();
    for (r, glyph, on) in [(l.up, "\u{25b2}", can_up), (l.down, "\u{25bc}", can_down)] {
        let img = ui.text(glyph, MAC_FOOTNOTE_PT, Weight::Regular, if on { LINK } else { LABEL_3.now() });
        let p = px(ui, r);
        blit(c, &img, p.x + (p.w - img.w as f32) / 2.0, p.y + (p.h - img.h as f32) / 2.0);
    }
    let shown: Vec<&str> = all.iter().skip(panel.top).take(ROWS).map(String::as_str).collect();
    let chosen_family = (font as usize).checked_sub(panel.top).filter(|i| *i < ROWS);
    column(c, ui, l.family, &shown, chosen_family);
    let faces: Vec<&str> = (0..4u8).map(|i| face_label(i & 1 != 0, i & 2 != 0)).collect();
    column(c, ui, l.face, &faces, Some(face as usize));
    let sizes: Vec<String> = SIZES.iter().map(|s| format!("{s}")).collect();
    let size_rows: Vec<&str> = sizes.iter().map(String::as_str).collect();
    column(c, ui, l.size, &size_rows, SIZES.iter().position(|s| *s == size));
    done_button(c, ui, l.done);
}

/// Draws the colour panel; `current` is the selection's colour (index into `COLORS`), ringed.
pub fn draw_colors(c: &mut dyn Canvas, ui: &Ui, l: &ColorsLayout, current: u8) {
    frame(c, ui, l.frame);
    text_in(c, ui, Rect { x: l.frame.x + PAD, y: l.title_y, w: l.frame.w - 2.0 * PAD, h: TITLE_H }, 0.0, "Colors", MAC_BODY_PT, Weight::Bold, LABEL.now());
    for (i, r) in l.swatches.iter().enumerate() {
        c.fill_rrect(px(ui, *r), ui.pt(RADIUS), text_color(i as u8));
        c.stroke_rrect(px(ui, *r), ui.pt(RADIUS), ui.pt(1.0).max(1.0), SEPARATOR_C.now());
        if i == current as usize {
            let ring = Rect { x: r.x - 3.0, y: r.y - 3.0, w: r.w + 6.0, h: r.h + 6.0 };
            c.stroke_rrect(px(ui, ring), ui.pt(RADIUS + 2.0), ui.pt(2.0), LINK);
        }
    }
    done_button(c, ui, l.done);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_font_panel_columns_sit_side_by_side_inside_the_frame_and_answer_presses() {
        let l = fonts_layout(900.0, 100.0);
        assert!(l.family.x + l.family.w <= l.face.x && l.face.x + l.face.w <= l.size.x);
        assert!(l.size.x + l.size.w <= l.frame.x + l.frame.w && l.done.y + l.done.h <= l.frame.y + l.frame.h);
        let p = Fonts::new(0);
        let mid = |r: Rect, i: usize| (r.x + 4.0, r.y + ROW_H * (i as f32 + 0.5));
        let (x, y) = mid(l.family, 1);
        assert_eq!(fonts_hit(&l, &p, x, y), Some(FontsHit::Family(1)));
        let (x, y) = mid(l.face, 3);
        assert_eq!(fonts_hit(&l, &p, x, y), Some(FontsHit::Face(3)));
        let (x, y) = mid(l.size, 4);
        assert_eq!(fonts_hit(&l, &p, x, y), Some(FontsHit::Size(4)));
        let (x, y) = mid(l.face, 5);
        assert_eq!(fonts_hit(&l, &p, x, y), None, "four typefaces");
        assert_eq!(fonts_hit(&l, &p, l.done.x + 1.0, l.done.y + 1.0), Some(FontsHit::Done));
        assert_eq!(fonts_hit(&l, &p, l.up.x + 1.0, l.up.y + 1.0), Some(FontsHit::Up));
    }

    #[test]
    fn type_select_jumps_to_a_family_and_a_pause_starts_over() {
        let mut p = Fonts::new(0);
        assert_eq!(p.type_select("t", 0.0), Some(1), "Times");
        assert_eq!(p.type_select("i", 100.0), Some(1), "\"ti\" is still Times");
        assert_eq!(p.type_select("c", 5000.0), Some(2), "after a pause \"c\" is Courier");
        assert_eq!(p.type_select("q", 5100.0), None, "no family starts with \"cq\"");
        p.scroll(-5);
        assert_eq!(p.top, 0);
    }

    #[test]
    fn the_colour_panel_has_a_swatch_per_palette_colour() {
        let l = colors_layout(900.0, 100.0);
        assert_eq!(l.swatches.len(), COLORS.len());
        let s = l.swatches[5];
        assert_eq!(colors_hit(&l, s.x + 2.0, s.y + 2.0), Some(Some(5)));
        assert_eq!(colors_hit(&l, l.done.x + 1.0, l.done.y + 1.0), Some(None));
        assert_eq!(colors_hit(&l, l.frame.x + 1.0, l.frame.y + 1.0), None);
        assert!(l.swatches.iter().all(|r| r.x >= l.frame.x && r.x + r.w <= l.frame.x + l.frame.w));
    }
}
