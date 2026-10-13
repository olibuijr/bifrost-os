//! Drawing of the sticky notes board on a canvas. Layout is in points (`Ui::pt` turns them into pixels), so the
//! board follows the window size and the display scale. Colours and type come from the shell's tokens.

use crate::model::{Board, Colour, Note, Style, TextSize, DISCLOSURE_PT, GRIP_PT, PAD_PT, TITLE_H};
use bifrost_app::controls::Rect;
use bifrost_app::kit::canvas::{Canvas, RectF};
use bifrost_app::kit::rgba;
use bifrost_app::kit::ui::{
    Ui, Weight, BLACK, LABEL_2, MAC_BODY_PT, MAC_CALLOUT_PT, MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT, MAC_TITLE3_PT, SYSTEM_GREEN,
    SYSTEM_ORANGE, SYSTEM_PINK, SYSTEM_PURPLE, SYSTEM_TEAL, SYSTEM_YELLOW,
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
/// Opacity of a note that is not the focused one (translucent when inactive).
pub const INACTIVE_ALPHA: f32 = 0.82;

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

/// Body text size (points) and weight for a note's style.
pub fn body_type(style: Style) -> (f32, Weight) {
    let pt = match style.size {
        TextSize::Small => MAC_CALLOUT_PT,
        TextSize::Medium => MAC_BODY_PT,
        TextSize::Large => MAC_TITLE3_PT,
    };
    (pt, if style.bold { Weight::Bold } else { Weight::Regular })
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

    if focused {
        c.shadow(rect, radius, ui.pt(8.0), (0.0, ui.pt(3.0)), shadow_colour());
    } else {
        // inactive: the window behind shows through the note's material
        widgets::material(c, ui, frame, RADIUS_PT, Material::Popover);
    }
    c.push_opacity(if focused { 1.0 } else { INACTIVE_ALPHA });
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
    let (pt, weight) = body_type(note.style);
    let text_x = x + ui.pt(PAD_PT);
    let mut text_y = y + ui.pt(TITLE_H + PAD_PT);
    let step = ui.pt(pt * 1.3);
    if note.text.is_empty() {
        if focused {
            let img = ui.text("Type a note", pt, weight, LABEL_2.now());
            widgets::blit(c, &img, text_x as i32, text_y as i32);
        }
        return;
    }
    let max_px = ui.pt(note.w - 2.0 * PAD_PT);
    // a note keeps its own line breaks; each paragraph is wrapped to the width
    for paragraph in note.text.split('\n') {
        let lines = if paragraph.is_empty() { vec![String::new()] } else { ui.wrap(paragraph, pt, weight, max_px) };
        for line in lines {
            if !line.is_empty() {
                let img = ui.text(&line, pt, weight, BLACK);
                widgets::blit(c, &img, text_x as i32, text_y as i32);
            }
            text_y += step;
        }
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
    fn large_bold_text_uses_the_title3_size_and_the_heavy_weight() {
        let (pt, weight) = body_type(Style { bold: true, italic: false, size: TextSize::Large });
        assert_eq!(pt, MAC_TITLE3_PT);
        assert_eq!(weight, Weight::Bold);
        assert_eq!(body_type(Style::default()).0, MAC_BODY_PT);
    }
}
