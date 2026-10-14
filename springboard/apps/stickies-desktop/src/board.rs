//! Drawing of the sticky notes board on a canvas. Layout is in points (`Ui::pt` turns them into pixels), so the
//! board follows the window size and the display scale. Colours and type come from the shell's tokens.

use crate::layout::{self, Line};
use crate::model::{self, Board, Colour, Ink, Note, Style, TextSize, DISCLOSURE_PT, PAD_PT, TITLE_H};
use bifrost_app::controls::Rect;
use bifrost_app::kit::canvas::{Canvas, RectF};
use bifrost_app::kit::rgba;
use bifrost_app::kit::ui::{
    Ui, Weight, BLACK, LABEL, LABEL_2, MAC_BODY_PT, MAC_CALLOUT_PT, MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT, MAC_TITLE3_PT, SYSTEM_BLUE,
    SYSTEM_GREEN, SYSTEM_ORANGE, SYSTEM_PINK, SYSTEM_PURPLE, SYSTEM_RED, SYSTEM_TEAL, SYSTEM_YELLOW,
};
use bifrost_app::widgets::{self, Material};

/// Corner radius of a note (points): the grouped card radius of the design rules.
pub const RADIUS_PT: f32 = 13.0;
/// Shade over the title bar of a note, so the bar reads apart from the body.
fn title_shade() -> u32 {
    rgba(0, 0, 0, 38)
}
/// Shadow under the focused note.
fn shadow_colour() -> u32 {
    rgba(0, 0, 0, 60)
}
/// The fill behind selected text.
fn selection_fill() -> u32 {
    rgba(10, 132, 255, 90)
}
/// Opacity of a note that is not the focused one (translucent when inactive).
pub const INACTIVE_ALPHA: f32 = 0.82;
/// Line height of the body as a multiple of its text size.
const LINE_HEIGHT: f32 = 1.3;
/// Size of the find bar at the bottom of the board (points).
const FIND_W_PT: f32 = 320.0;
const FIND_H_PT: f32 = 30.0;
const FIND_BOTTOM_PT: f32 = 16.0;
const FIND_RADIUS_PT: f32 = 6.0;

pub fn colour_value(c: Colour) -> u32 {
    match c {
        Colour::Yellow => SYSTEM_YELLOW,
        Colour::Orange => SYSTEM_ORANGE,
        Colour::Green => SYSTEM_GREEN,
        Colour::Teal => SYSTEM_TEAL,
        Colour::Pink => SYSTEM_PINK,
        Colour::Purple => SYSTEM_PURPLE,
    }
}

/// The colour of a note's text.
pub fn ink_value(ink: Ink) -> u32 {
    match ink {
        Ink::Black => BLACK,
        Ink::Red => SYSTEM_RED,
        Ink::Blue => SYSTEM_BLUE,
        Ink::Green => SYSTEM_GREEN,
        Ink::Purple => SYSTEM_PURPLE,
    }
}

/// Body text size (points) and weight for a note's style.
pub fn body_type(style: Style) -> (f32, Weight) {
    let pt = match style.size {
        TextSize::Small => MAC_CALLOUT_PT,
        TextSize::Medium => MAC_BODY_PT,
        TextSize::Large => MAC_TITLE3_PT,
    };
    (pt, if style.bold { Weight::Bold } else { Weight::Regular })
}

/// How a note's body is set: the text size and weight, the line step and the wrap width, in pixels where noted.
#[derive(Clone, Copy, Debug)]
pub struct BodyMetrics {
    pub pt: f32,
    pub weight: Weight,
    /// Distance between lines (px).
    pub step: f32,
    /// Wrap width (px).
    pub max: f32,
}

pub fn body_metrics(ui: &Ui, note: &Note) -> BodyMetrics {
    let (pt, weight) = body_type(note.style);
    BodyMetrics { pt, weight, step: ui.pt(pt * LINE_HEIGHT), max: ui.pt(note.w - 2.0 * PAD_PT) }
}

/// The wrapped lines of the body.
pub fn body_lines(ui: &Ui, note: &Note, m: &BodyMetrics) -> Vec<Line> {
    layout::lines(&note.text, m.max, &|s| ui.measure(s, m.pt, m.weight))
}

/// The byte offset in the body under the pixel position `(px, py)`, relative to the canvas.
pub fn offset_at(ui: &Ui, note: &Note, px: f32, py: f32) -> usize {
    let m = body_metrics(ui, note);
    let lines = body_lines(ui, note, &m);
    let top = ui.pt(note.y + TITLE_H + PAD_PT);
    let row = ((py - top) / m.step).floor().max(0.0) as usize;
    let line = lines[row.min(lines.len() - 1)];
    layout::index_at(&note.text, &line, px - ui.pt(note.x + PAD_PT), &|s| ui.measure(s, m.pt, m.weight))
}

/// The caret after moving a line up or down in the wrapped body.
pub fn caret_vertical(ui: &Ui, note: &Note, down: bool) -> usize {
    let m = body_metrics(ui, note);
    let lines = body_lines(ui, note, &m);
    layout::move_vertical(&note.text, &lines, note.caret, down, &|s| ui.measure(s, m.pt, m.weight))
}

/// Draws the whole board, back to front. `width` and `height` are the canvas size in pixels.
pub fn draw(c: &mut dyn Canvas, ui: &Ui, board: &Board) {
    let (cw, ch) = c.size();
    if board.notes.is_empty() {
        let img = ui.text("Choose File, New Sticky Note to add a note.", MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
        widgets::blit(c, &img, (cw as i32 - img.w as i32) / 2, (ch as i32 - img.h as i32) / 2);
        return;
    }
    for note in &board.notes {
        draw_note(c, ui, note, board.focus == Some(note.id));
    }
}

fn draw_note(c: &mut dyn Canvas, ui: &Ui, note: &Note, focused: bool) {
    let frame = Rect { x: note.x, y: note.y, w: note.w, h: note.frame_h() };
    let (x, y, w, h) = (ui.pt(note.x), ui.pt(note.y), ui.pt(note.w), ui.pt(frame.h));
    let radius = ui.pt(RADIUS_PT);
    let rect = RectF::new(x, y, w, h);
    let see_through = !focused && note.translucent;

    if focused {
        c.shadow(rect, radius, ui.pt(8.0), (0.0, ui.pt(3.0)), shadow_colour());
    } else if see_through {
        // inactive: the window behind shows through the note's material
        widgets::material(c, ui, frame, RADIUS_PT, Material::Popover);
    }
    c.push_opacity(if see_through { INACTIVE_ALPHA } else { 1.0 });
    c.fill_rrect(rect, radius, colour_value(note.colour));
    c.push_clip(rect, radius);
    c.fill_rect(RectF::new(x, y, w, ui.pt(TITLE_H)), title_shade());

    // disclosure: collapsed notes show a right-pointing mark, expanded ones a downward one
    let disclosure = if note.collapsed { ">" } else { "v" };
    let img = ui.text(disclosure, MAC_SUBHEAD_PT, Weight::Bold, BLACK);
    widgets::blit(c, &img, (x + ui.pt(DISCLOSURE_PT / 2.0) - img.w as f32 / 2.0) as i32, (y + (ui.pt(TITLE_H) - img.h as f32) / 2.0) as i32);

    let title_px = ui.pt(note.w - DISCLOSURE_PT - PAD_PT);
    let title = ui.fit(&note.title(), MAC_SUBHEAD_PT, title_px);
    if !title.is_empty() {
        let img = ui.text(&title, MAC_SUBHEAD_PT, Weight::Semibold, BLACK);
        widgets::blit(c, &img, (x + ui.pt(DISCLOSURE_PT)) as i32, (y + (ui.pt(TITLE_H) - img.h as f32) / 2.0) as i32);
    }

    if !note.collapsed {
        draw_body(c, ui, note, focused, x, y);
        draw_grip(c, ui, x, y, w, h);
    }
    c.pop_clip();
    c.pop_opacity();
}

fn draw_body(c: &mut dyn Canvas, ui: &Ui, note: &Note, focused: bool, x: f32, y: f32) {
    let m = body_metrics(ui, note);
    let ink = ink_value(note.style.ink);
    let text_x = x + ui.pt(PAD_PT);
    let top = y + ui.pt(TITLE_H + PAD_PT);
    let rule = ui.pt(1.0).max(1.0);
    if note.text.is_empty() {
        if focused {
            let img = ui.text("Type a note", m.pt, m.weight, LABEL_2.now());
            widgets::blit(c, &img, text_x as i32, top as i32);
            c.fill_rect(RectF::new(text_x, top, rule, m.step), ink);
        }
        return;
    }
    let lines = body_lines(ui, note, &m);
    let links = model::links(&note.text);
    let sel = note.selection();
    let width_of = |from: usize, to: usize| ui.measure(&note.text[from..to], m.pt, m.weight);
    for (i, line) in lines.iter().enumerate() {
        let ly = top + m.step * i as f32;
        if focused && !sel.is_empty() {
            let (a, b) = (sel.start.max(line.start), sel.end.min(line.end));
            if a < b {
                c.fill_rect(RectF::new(text_x + width_of(line.start, a), ly, width_of(a, b), m.step), selection_fill());
            }
        }
        if line.start == line.end {
            continue;
        }
        let shift = (m.step * note.style.baseline.shift()) as i32;
        let (tx, ty) = (text_x as i32, ly as i32 + shift);
        let img = if note.style.outline {
            // outline: the letters are ringed in the text colour and filled with the note colour
            let ring = ui.text(&note.text[line.start..line.end], m.pt, m.weight, ink);
            let r = (ui.pt(1.0) as i32).max(1);
            for (dx, dy) in [(-r, 0), (r, 0), (0, -r), (0, r)] {
                widgets::blit(c, &ring, tx + dx, ty + dy);
            }
            ui.text(&note.text[line.start..line.end], m.pt, m.weight, colour_value(note.colour))
        } else {
            ui.text(&note.text[line.start..line.end], m.pt, m.weight, ink)
        };
        widgets::blit(c, &img, tx, ty);
        let shown = note.text[line.start..line.end].trim_end().len();
        let line_w = width_of(line.start, line.start + shown);
        if note.style.underline {
            c.fill_rect(RectF::new(text_x, ly + m.step * 0.88, line_w, rule), ink);
        }
        if note.style.strike {
            c.fill_rect(RectF::new(text_x, ly + m.step * 0.5, line_w, rule), ink);
        }
        for link in &links {
            let (a, b) = (link.start.max(line.start), link.end.min(line.end));
            if a < b {
                c.fill_rect(RectF::new(text_x + width_of(line.start, a), ly + m.step * 0.88, width_of(a, b), rule), SYSTEM_BLUE);
            }
        }
    }
    if focused && sel.is_empty() {
        let line = lines[layout::line_of(&lines, note.caret)];
        let at = lines.iter().position(|l| *l == line).unwrap_or(0);
        let cx = text_x + width_of(line.start, note.caret.clamp(line.start, line.end));
        c.fill_rect(RectF::new(cx, top + m.step * at as f32, rule, m.step), ink);
    }
}

/// Three diagonal dots in the bottom right corner: the resize grip.
fn draw_grip(c: &mut dyn Canvas, ui: &Ui, x: f32, y: f32, w: f32, h: f32) {
    let colour = LABEL_2.now();
    for i in 0..3 {
        let off = 4.0 + 4.0 * i as f32; // points in from the right and bottom edges
        c.fill_rrect(RectF::new(x + w - ui.pt(off), y + h - ui.pt(off), ui.pt(2.0), ui.pt(2.0)), ui.pt(1.0), colour);
    }
}

/// The find bar's rectangle in points, for a canvas of `width` by `height` pixels.
pub fn find_bar_rect(ui: &Ui, width: u32, height: u32) -> Rect {
    let scale = ui.pt(1.0).max(0.001);
    let (w, h) = (width as f32 / scale, height as f32 / scale);
    let bar_w = FIND_W_PT.min(w - 2.0 * FIND_BOTTOM_PT).max(0.0);
    Rect { x: (w - bar_w) / 2.0, y: h - FIND_BOTTOM_PT - FIND_H_PT, w: bar_w, h: FIND_H_PT }
}

/// The find bar: the search text and how many places it was found in.
pub fn draw_find(c: &mut dyn Canvas, ui: &Ui, query: &str, replace: Option<&str>, replace_active: bool, found: usize) {
    let (cw, ch) = c.size();
    let bar = find_bar_rect(ui, cw, ch);
    widgets::material(c, ui, bar, FIND_RADIUS_PT, Material::Popover);
    let pad = ui.pt(10.0);
    let caret = if replace.is_some() && replace_active { "" } else { "|" };
    let shown = if query.is_empty() { "Find".to_string() } else { format!("{query}{caret}") };
    // with the replace field open the find text comes first and the replacement follows it
    let shown = match replace {
        Some(with) => format!("{shown}  >  {}{}", if with.is_empty() && !replace_active { "Replace" } else { with }, if replace_active { "|" } else { "" }),
        None => shown,
    };
    let colour = if query.is_empty() { LABEL_2.now() } else { LABEL.now() };
    let status = match (query.is_empty(), found) {
        (true, _) => String::new(),
        (false, 0) => "Not found".to_string(),
        (false, 1) => "1 match".to_string(),
        (false, n) => format!("{n} matches"),
    };
    let status_img = ui.text(&status, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
    let field_px = ui.pt(bar.w) - 3.0 * pad - status_img.w as f32;
    let img = ui.text(&ui.fit(&shown, MAC_BODY_PT, field_px.max(0.0)), MAC_BODY_PT, Weight::Regular, colour);
    let (bx, by, bh) = (ui.pt(bar.x), ui.pt(bar.y), ui.pt(bar.h));
    widgets::blit(c, &img, (bx + pad) as i32, (by + (bh - img.h as f32) / 2.0) as i32);
    if !status.is_empty() {
        widgets::blit(c, &status_img, (bx + ui.pt(bar.w) - pad - status_img.w as f32) as i32, (by + (bh - status_img.h as f32) / 2.0) as i32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_colour_has_a_token_value() {
        let values: Vec<u32> = Colour::ALL.iter().map(|c| colour_value(*c)).collect();
        for (i, a) in values.iter().enumerate() {
            for b in &values[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn every_ink_has_its_own_value() {
        let values: Vec<u32> = Ink::ALL.iter().map(|i| ink_value(*i)).collect();
        for (i, a) in values.iter().enumerate() {
            for b in &values[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn large_bold_text_uses_the_title3_size_and_the_heavy_weight() {
        let (pt, weight) = body_type(Style { bold: true, size: TextSize::Large, ..Style::default() });
        assert_eq!(pt, MAC_TITLE3_PT);
        assert_eq!(weight, Weight::Bold);
        assert_eq!(body_type(Style::default()).0, MAC_BODY_PT);
    }
}
