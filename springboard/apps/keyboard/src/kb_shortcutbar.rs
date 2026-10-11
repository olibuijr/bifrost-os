//! bifrost shortcut bar above the keys: Undo, Redo and Paste glyph buttons on
//! the left of the prediction strip. Bold/Italic/Underline are not offered
//! because the text fields are plain text (the group is hidden, as on bifrost
//! for fields without rich text). Taps map to the `Out::Named` edit commands
//! the focused field already understands.

use springboard::shell::glyphs;
use springboard::gfx::Surface;

/// Buttons, left to right.
pub const BUTTONS: usize = 3;
/// Width of each glyph button in points (a 44 pt tap target).
pub const BUTTON_PT: f32 = 44.0;

/// The edit command a button sends to the focused field.
pub fn command(n: usize) -> &'static str {
    ["Undo", "Redo", "Paste"][n.min(BUTTONS - 1)]
}

/// Left edge in px where the prediction cells start, given the margin and
/// the px-per-point horizontal scale.
pub fn predictions_from(margin: f32, px_per_pt: f32) -> f32 {
    margin + BUTTONS as f32 * BUTTON_PT * px_per_pt
}

/// Draws glyph `n` centred in the box; `r` is its half size in px.
pub fn draw_glyph(s: &mut Surface, n: usize, (cx, cy): (f32, f32), r: f32, t: f32, color: u32) {
    let b = (cx - r * 1.6, cy - r * 1.6, r * 3.2, r * 3.2);
    match n {
        // the system symbol set arrow.uturn.backward / forward: arrowhead, top run, right
        // half turn, short return run (mirrored for redo)
        0 | 1 => {
            let m = if n == 0 { 1.0 } else { -1.0 };
            let p = |x: f32, y: f32| (cx + m * x * r, cy + y * r);
            let seg = |s: &mut Surface, a: (f32, f32), c: (f32, f32)| glyphs::paint(s, b, color, glyphs::segment(a, c, t));
            seg(s, p(-0.9, -0.5), p(0.3, -0.5));
            let (a0, a1) = if n == 0 { (0.0, std::f32::consts::PI) } else { (std::f32::consts::PI, std::f32::consts::TAU) };
            glyphs::paint(s, b, color, glyphs::arc(p(0.3, 0.1), r * 0.6, t, a0, a1));
            seg(s, p(0.3, 0.7), p(-0.3, 0.7));
            seg(s, p(-0.9, -0.5), p(-0.45, -0.95));
            seg(s, p(-0.9, -0.5), p(-0.45, -0.05));
        }
        // the system symbol set doc.on.clipboard: rounded sheet outline with a clip on top
        _ => {
            let (hx, top, bot, cr) = (r * 0.75, cy - r * 0.75, cy + r * 0.95, r * 0.28);
            let (hw, hh) = (hx - cr, (bot - top) / 2.0 - cr);
            let cyb = (top + bot) / 2.0;
            glyphs::paint(s, b, color, move |x, y| {
                let (qx, qy) = ((x - cx).abs() - hw, (y - cyb).abs() - hh);
                let o = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - cr;
                o.abs() - t / 2.0
            });
            glyphs::paint(s, b, color, glyphs::segment((cx - r * 0.2, top - r * 0.1), (cx + r * 0.2, top - r * 0.1), r * 0.55));
        }
    }
}
