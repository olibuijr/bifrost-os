//! Drawing of the sticky notes board on a canvas. Layout is in points (`Ui::pt` turns them into pixels), so the
//! board follows the window size and the display scale. Colours and type come from the shell's tokens. Each run of
//! text is drawn with its own style: weight, slant, outline, baseline, decorations and font features.

use crate::layout::{self, Line};
use crate::model::{self, Board, Colour, Ink, Note, Setting, Style, TextSize, DISCLOSURE_PT, PAD_PT, TITLE_H};
use crate::spell::{Mark, MarkKind};
use crate::swatch;
use bifrost_app::attributed::FontFeatures;
use bifrost_app::controls::Rect;
use bifrost_app::kit::canvas::{Canvas, RectF};
use bifrost_app::kit::rgba;
use bifrost_app::kit::ui::{
    Ui, Weight, BLACK, LABEL, LABEL_2, MAC_BODY_PT, MAC_CALLOUT_PT, MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT, MAC_TITLE3_PT, SYSTEM_BLUE,
    SYSTEM_GREEN, SYSTEM_ORANGE, SYSTEM_PINK, SYSTEM_PURPLE, SYSTEM_RED, SYSTEM_TEAL, SYSTEM_YELLOW,
};
use bifrost_app::kit::Image;
use bifrost_app::widgets::{self, Material};
use std::collections::HashMap;
use std::ops::Range;

/// Corner radius of a note (points): the grouped card radius of the design rules.
pub const RADIUS_PT: f32 = 13.0;
/// The marks of each note, by note id: the misspelled words and the detected items of its text.
pub type Marks = HashMap<u64, Vec<Mark>>;
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
/// How far an italic run leans: the shift of its top edge over its height.
const ITALIC_SLANT: f32 = 0.2;
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

/// The fill of a note: its custom colour when it has one, else the token of its preset.
pub fn note_fill(note: &Note) -> u32 {
    match note.rgb {
        Some((r, g, b)) => rgba(r, g, b, 255),
        None => colour_value(note.colour),
    }
}

/// The colour of a text ink.
pub fn ink_value(ink: Ink) -> u32 {
    match ink {
        Ink::Black => BLACK,
        Ink::Red => SYSTEM_RED,
        Ink::Blue => SYSTEM_BLUE,
        Ink::Green => SYSTEM_GREEN,
        Ink::Purple => SYSTEM_PURPLE,
    }
}

/// Body text size (points) of a note's size.
pub fn body_pt(size: TextSize) -> f32 {
    match size {
        TextSize::Small => MAC_CALLOUT_PT,
        TextSize::Medium => MAC_BODY_PT,
        TextSize::Large => MAC_TITLE3_PT,
    }
}

/// The weight of a run: bold runs are heavier.
pub fn weight_of(style: &Style) -> Weight {
    if style.bold {
        Weight::Bold
    } else {
        Weight::Regular
    }
}

/// The font features of a run: kerning and ligatures set as its Font > Kern and Font > Ligature say. Empty when both
/// are the font's own.
pub fn features(style: &Style) -> FontFeatures {
    let mut f = FontFeatures::new();
    match style.kern {
        Setting::Default => {}
        Setting::Off => {
            f.off("kern");
        }
        Setting::All => {
            f.on("kern");
        }
    }
    match style.ligature {
        Setting::Default => {}
        Setting::Off => {
            f.off("liga").off("clig");
        }
        Setting::All => {
            f.on("liga").on("dlig");
        }
    }
    f
}

/// The width (px) of `text` drawn in `style` at `pt` points.
fn run_width(ui: &Ui, text: &str, pt: f32, style: &Style) -> f32 {
    let f = features(style);
    if f.is_empty() {
        ui.measure(text, pt, weight_of(style))
    } else {
        ui.measure_features(text, pt, weight_of(style), &f)
    }
}

/// How a note's body is set: the text size, the line step and the wrap width, in pixels where noted.
#[derive(Clone, Copy, Debug)]
pub struct BodyMetrics {
    pub pt: f32,
    /// Distance between lines (px).
    pub step: f32,
    /// Wrap width (px).
    pub max: f32,
}

pub fn body_metrics(ui: &Ui, note: &Note) -> BodyMetrics {
    let pt = body_pt(note.style.size);
    BodyMetrics { pt, step: ui.pt(pt * LINE_HEIGHT), max: ui.pt(note.w - 2.0 * PAD_PT) }
}

/// The width (px) of the body text between two byte offsets, each run measured with its own style.
pub fn measure_range(ui: &Ui, note: &Note, m: &BodyMetrics, from: usize, to: usize) -> f32 {
    note.style_runs(from..to).iter().map(|(r, s)| run_width(ui, &note.text[r.clone()], m.pt, s)).sum()
}

/// The wrapped lines of the body.
pub fn body_lines(ui: &Ui, note: &Note, m: &BodyMetrics) -> Vec<Line> {
    layout::lines(&note.text, m.max, &|a, b| measure_range(ui, note, m, a, b))
}

/// The byte offset in the body under the pixel position `(px, py)`, relative to the canvas.
pub fn offset_at(ui: &Ui, note: &Note, px: f32, py: f32) -> usize {
    let m = body_metrics(ui, note);
    let lines = body_lines(ui, note, &m);
    let top = ui.pt(note.y + TITLE_H + PAD_PT);
    let row = ((py - top) / m.step).floor().max(0.0) as usize;
    let line = lines[row.min(lines.len() - 1)];
    layout::index_at(&note.text, &line, px - ui.pt(note.x + PAD_PT), &|a, b| measure_range(ui, note, &m, a, b))
}

/// The caret after moving a line up or down in the wrapped body.
pub fn caret_vertical(ui: &Ui, note: &Note, down: bool) -> usize {
    let m = body_metrics(ui, note);
    let lines = body_lines(ui, note, &m);
    layout::move_vertical(&note.text, &lines, note.caret, down, &|a, b| measure_range(ui, note, &m, a, b))
}

/// Draws the whole board, back to front. `marks` holds the rules under the misspelled and detected text.
pub fn draw(c: &mut dyn Canvas, ui: &Ui, board: &Board, marks: &Marks) {
    let (cw, ch) = c.size();
    if board.notes.is_empty() {
        let img = ui.text("Choose File, New Sticky Note to add a note.", MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
        widgets::blit(c, &img, (cw as i32 - img.w as i32) / 2, (ch as i32 - img.h as i32) / 2);
        return;
    }
    let none: &[Mark] = &[];
    for note in &board.notes {
        let note_marks = marks.get(&note.id).map_or(none, |m| m.as_slice());
        draw_note(c, ui, note, board.focus == Some(note.id), note_marks);
    }
}

fn draw_note(c: &mut dyn Canvas, ui: &Ui, note: &Note, focused: bool, marks: &[Mark]) {
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
    c.fill_rrect(rect, radius, note_fill(note));
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
        draw_body(c, ui, note, focused, marks, x, y);
        draw_grip(c, ui, x, y, w, h);
    }
    c.pop_clip();
    c.pop_opacity();
}

/// Draws one run of text at (`x`, `y`) px: its weight and features, leaning when italic, ringed when outlined.
fn draw_run(c: &mut dyn Canvas, ui: &Ui, note: &Note, m: &BodyMetrics, text: &str, style: &Style, x: f32, y: f32) {
    let f = features(style);
    let render = |colour: u32| {
        if f.is_empty() {
            ui.text(text, m.pt, weight_of(style), colour)
        } else {
            ui.text_features(text, m.pt, weight_of(style), colour, &f)
        }
    };
    let shape = |colour: u32| {
        let img = render(colour);
        if style.italic {
            slant(&img)
        } else {
            img
        }
    };
    let (tx, ty) = (x as i32, y as i32);
    if style.outline {
        // the letters are ringed in the text colour and filled with the note colour
        let ring = shape(ink_value(style.ink));
        let r = (ui.pt(1.0) as i32).max(1);
        for (dx, dy) in [(-r, 0), (r, 0), (0, -r), (0, r)] {
            widgets::blit(c, &ring, tx + dx, ty + dy);
        }
        widgets::blit(c, &shape(note_fill(note)), tx, ty);
    } else {
        widgets::blit(c, &shape(ink_value(style.ink)), tx, ty);
    }
}

/// The image leaning right: each row moves right by a fixed share of the height above it, so the top leans most.
fn slant(img: &Image) -> Image {
    let shear = (img.h as f32 * ITALIC_SLANT).ceil() as usize;
    let mut out = Image::new(img.w + shear, img.h);
    for row in 0..img.h {
        let shift = ((img.h - 1 - row) as f32 * ITALIC_SLANT).round() as usize;
        let start = row * out.w + shift;
        out.px[start..start + img.w].copy_from_slice(&img.px[row * img.w..(row + 1) * img.w]);
    }
    out
}

fn draw_body(c: &mut dyn Canvas, ui: &Ui, note: &Note, focused: bool, marks: &[Mark], x: f32, y: f32) {
    let m = body_metrics(ui, note);
    let text_x = x + ui.pt(PAD_PT);
    let top = y + ui.pt(TITLE_H + PAD_PT);
    let rule = ui.pt(1.0).max(1.0);
    if note.text.is_empty() {
        if focused {
            let img = ui.text("Type a note", m.pt, Weight::Regular, LABEL_2.now());
            widgets::blit(c, &img, text_x as i32, top as i32);
            c.fill_rect(RectF::new(text_x, top, rule, m.step), ink_value(note.current_style().ink));
        }
        return;
    }
    let lines = body_lines(ui, note, &m);
    let links = model::links(&note.text);
    let sel = note.selection();
    let width = |a: usize, b: usize| measure_range(ui, note, &m, a, b);
    for (i, line) in lines.iter().enumerate() {
        let ly = top + m.step * i as f32;
        if focused && !sel.is_empty() {
            let (a, b) = (sel.start.max(line.start), sel.end.min(line.end));
            if a < b {
                c.fill_rect(RectF::new(text_x + width(line.start, a), ly, width(a, b), m.step), selection_fill());
            }
        }
        if line.start == line.end {
            continue;
        }
        // trailing spaces of a line take no decoration
        let visible_end = line.start + note.text[line.start..line.end].trim_end().len();
        for (range, style) in note.style_runs(line.start..line.end) {
            let ink = ink_value(style.ink);
            let run_x = text_x + width(line.start, range.start);
            let run_y = ly + m.step * style.baseline.shift();
            draw_run(c, ui, note, &m, &note.text[range.clone()], &style, run_x, run_y);
            let decorated = range.start..range.end.min(visible_end);
            if decorated.start < decorated.end {
                let run_w = width(decorated.start, decorated.end);
                let dx = text_x + width(line.start, decorated.start);
                if style.underline {
                    c.fill_rect(RectF::new(dx, run_y + m.step * 0.88, run_w, rule), ink);
                }
                if style.strike {
                    c.fill_rect(RectF::new(dx, run_y + m.step * 0.5, run_w, rule), ink);
                }
            }
        }
        for link in &links {
            let (a, b) = (link.start.max(line.start), link.end.min(line.end));
            if a < b {
                c.fill_rect(RectF::new(text_x + width(line.start, a), ly + m.step * 0.88, width(a, b), rule), SYSTEM_BLUE);
            }
        }
        for mark in marks {
            let (a, b) = (mark.range.start.max(line.start), mark.range.end.min(line.end));
            if a >= b {
                continue;
            }
            let (x0, x1) = (text_x + width(line.start, a), text_x + width(line.start, b));
            let y0 = ly + m.step * 0.92;
            match mark.kind {
                MarkKind::Misspelled => squiggle(c, x0, x1, y0, rule, SYSTEM_RED),
                MarkKind::Detected => dotted(c, x0, x1, y0, rule, SYSTEM_TEAL),
            }
        }
    }
    if focused && sel.is_empty() {
        let at = layout::line_of(&lines, note.caret);
        let line = lines[at];
        let cx = text_x + width(line.start, note.caret.clamp(line.start, line.end));
        c.fill_rect(RectF::new(cx, top + m.step * at as f32, rule, m.step), ink_value(note.current_style().ink));
    }
}

/// A wavy rule under misspelled text: short steps that alternate between two heights.
fn squiggle(c: &mut dyn Canvas, x0: f32, x1: f32, y: f32, rule: f32, colour: u32) {
    let step = rule * 2.0;
    let mut x = x0;
    let mut up = false;
    while x < x1 {
        let dy = if up { 0.0 } else { rule };
        c.fill_rect(RectF::new(x, y + dy, step.min(x1 - x), rule), colour);
        x += step;
        up = !up;
    }
}

/// A dotted rule under a detected item (a date, an address or a phone number).
fn dotted(c: &mut dyn Canvas, x0: f32, x1: f32, y: f32, rule: f32, colour: u32) {
    let mut x = x0;
    while x < x1 {
        c.fill_rect(RectF::new(x, y, (rule * 2.0).min(x1 - x), rule), colour);
        x += rule * 4.0;
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

/// The find bar: the search text, the replacement and how many places the search text was found in. The field that
/// takes typing shows the caret.
pub fn draw_find(c: &mut dyn Canvas, ui: &Ui, query: &str, replace: &str, replacing: bool, found: usize) {
    let (cw, ch) = c.size();
    let bar = find_bar_rect(ui, cw, ch);
    widgets::material(c, ui, bar, FIND_RADIUS_PT, Material::Popover);
    let pad = ui.pt(10.0);
    let status = match (query.is_empty(), found) {
        (true, _) => String::new(),
        (false, 0) => "Not found".to_string(),
        (false, 1) => "1 match".to_string(),
        (false, n) => format!("{n} matches"),
    };
    let status_img = ui.text(&status, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
    let (bx, by, bh) = (ui.pt(bar.x), ui.pt(bar.y), ui.pt(bar.h));
    let fields_px = (ui.pt(bar.w) - 4.0 * pad - status_img.w as f32).max(0.0);
    let field_px = fields_px / 2.0;
    let mut x = bx + pad;
    for (text, placeholder, active) in [(query, "Find", !replacing), (replace, "Replace", replacing)] {
        let shown = field_text(text, placeholder, active);
        let colour = if text.is_empty() { LABEL_2.now() } else { LABEL.now() };
        let img = ui.text(&ui.fit(&shown, MAC_BODY_PT, field_px), MAC_BODY_PT, Weight::Regular, colour);
        widgets::blit(c, &img, x as i32, (by + (bh - img.h as f32) / 2.0) as i32);
        x += field_px + pad;
    }
    if !status.is_empty() {
        widgets::blit(c, &status_img, (bx + ui.pt(bar.w) - pad - status_img.w as f32) as i32, (by + (bh - status_img.h as f32) / 2.0) as i32);
    }
}

/// What a bar field shows: the text with a caret while it takes typing, or the placeholder when empty and idle.
pub fn field_text(text: &str, placeholder: &str, active: bool) -> String {
    match (text.is_empty(), active) {
        (true, false) => placeholder.to_string(),
        (_, true) => format!("{text}|"),
        (false, false) => text.to_string(),
    }
}

/// A bottom bar that takes typed text: the text with a caret, a hint at the right, and a swatch when one is given.
fn draw_entry(c: &mut dyn Canvas, ui: &Ui, entry: &str, hint: &str, swatch_colour: Option<u32>) {
    let (cw, ch) = c.size();
    let bar = find_bar_rect(ui, cw, ch);
    widgets::material(c, ui, bar, FIND_RADIUS_PT, Material::Popover);
    let pad = ui.pt(10.0);
    let (bx, by, bh) = (ui.pt(bar.x), ui.pt(bar.y), ui.pt(bar.h));
    let hint_img = ui.text(hint, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
    let swatch_px = ui.pt(16.0);
    let right = bx + ui.pt(bar.w) - pad;
    widgets::blit(c, &hint_img, (right - hint_img.w as f32) as i32, (by + (bh - hint_img.h as f32) / 2.0) as i32);
    let mut taken = hint_img.w as f32 + 2.0 * pad;
    if let Some(colour) = swatch_colour {
        let sx = right - hint_img.w as f32 - pad - swatch_px;
        c.fill_rrect(RectF::new(sx, by + (bh - swatch_px) / 2.0, swatch_px, swatch_px), ui.pt(3.0), colour);
        taken += swatch_px + pad;
    }
    let field_px = (ui.pt(bar.w) - 2.0 * pad - taken).max(0.0);
    let shown = format!("{entry}|");
    let img = ui.text(&ui.fit(&shown, MAC_BODY_PT, field_px), MAC_BODY_PT, Weight::Regular, LABEL.now());
    widgets::blit(c, &img, (bx + pad) as i32, (by + (bh - img.h as f32) / 2.0) as i32);
}

/// The custom colour bar: the typed `#RRGGBB`, a swatch of the colour once it reads as one, and the hint.
pub fn draw_colour_entry(c: &mut dyn Canvas, ui: &Ui, entry: &str) {
    let colour = swatch::parse(entry);
    let hint = match (colour, entry.chars().filter(char::is_ascii_hexdigit).count()) {
        (Some(_), _) => "Return to apply",
        (None, 0) => "Type a hex colour",
        (None, _) => "Not a colour yet",
    };
    draw_entry(c, ui, entry, hint, colour.map(|(r, g, b)| rgba(r, g, b, 255)));
}

/// The link bar: the typed web address and a hint on whether Return will add it.
pub fn draw_link_entry(c: &mut dyn Canvas, ui: &Ui, entry: &str) {
    let hint = match (crate::model::is_link(entry), entry.trim().is_empty()) {
        (true, _) => "Return to add the link",
        (false, true) => "Type a web address",
        (false, false) => "Not a web address yet",
    };
    draw_entry(c, ui, entry, hint, None);
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
    fn a_custom_colour_fills_the_note_instead_of_its_preset() {
        let mut n = Note::new(1, 0.0, 0.0, 0);
        assert_eq!(note_fill(&n), colour_value(Colour::Yellow));
        n.rgb = Some((10, 20, 30));
        assert_eq!(note_fill(&n), rgba(10, 20, 30, 255));
        n.set_preset(Colour::Teal);
        assert_eq!(note_fill(&n), colour_value(Colour::Teal));
    }

    #[test]
    fn a_bar_field_shows_a_caret_when_active_and_its_placeholder_when_empty() {
        assert_eq!(field_text("", "Find", false), "Find");
        assert_eq!(field_text("", "Find", true), "|");
        assert_eq!(field_text("abc", "Find", true), "abc|");
        assert_eq!(field_text("abc", "Find", false), "abc");
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
    fn the_size_sets_the_body_type_and_bold_is_a_weight() {
        assert_eq!(body_pt(TextSize::Large), MAC_TITLE3_PT);
        assert_eq!(body_pt(TextSize::Medium), MAC_BODY_PT);
        assert_eq!(weight_of(&Style { bold: true, ..Style::default() }), Weight::Bold);
        assert_eq!(weight_of(&Style::default()), Weight::Regular);
    }

    #[test]
    fn kerning_and_ligature_settings_become_font_features() {
        let plain = features(&Style::default());
        assert!(plain.is_empty(), "the font's own choice sets nothing");
        let off = features(&Style { kern: Setting::Off, ligature: Setting::Off, ..Style::default() });
        assert_eq!(off.value(*b"kern"), Some(0));
        assert_eq!(off.value(*b"liga"), Some(0));
        let all = features(&Style { kern: Setting::All, ligature: Setting::All, ..Style::default() });
        assert_eq!(all.value(*b"kern"), Some(1));
        assert_eq!(all.value(*b"dlig"), Some(1));
    }

    #[test]
    fn an_italic_run_leans_right_by_a_fifth_of_its_height() {
        let img = Image { w: 1, h: 10, px: vec![7; 10] };
        let slanted = slant(&img);
        assert_eq!((slanted.w, slanted.h), (3, 10), "two columns of room for the lean");
        assert_eq!(slanted.px[2], 7, "the top row moves right by two");
        assert_eq!(slanted.px[0], 0, "the top row no longer starts at the left");
        assert_eq!(slanted.px[9 * 3], 7, "the bottom row stays where it was");
        assert_eq!(slanted.px.iter().filter(|p| **p == 7).count(), 10);
    }
}
