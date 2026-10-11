//! Caps lock and the Settings > General > Keyboard switches that shape typing:
//! Enable Caps Lock, Auto-Capitalisation (kept in `keyboard.rs`) and the "."
//! Shortcut (double space types a period). Also the bifrost shift-key glyph:
//! an outlined arrow (off), a filled arrow (shift) and a filled arrow over an
//! underline bar (caps lock).

use super::glyphs::paint;
use crate::gfx::Surface;

/// Typing switches from Settings; both default on like bifrost.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KbPrefs {
    /// double-tapping shift locks caps
    pub enable_caps_lock: bool,
    /// "." Shortcut: a double space types ". "
    pub period_shortcut: bool,
}

impl Default for KbPrefs {
    fn default() -> Self {
        KbPrefs { enable_caps_lock: true, period_shortcut: true }
    }
}

/// What the shift key shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShiftState {
    Off,
    On,
    Locked,
}

impl ShiftState {
    pub fn of(shift: bool, caps: bool) -> Self {
        if caps {
            ShiftState::Locked
        } else if shift {
            ShiftState::On
        } else {
            ShiftState::Off
        }
    }
}

/// Signed distance to a closed polygon (negative inside).
fn poly(pts: &[(f32, f32)], px: f32, py: f32) -> f32 {
    let (mut d2, mut inside) = (f32::MAX, false);
    let n = pts.len();
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let (ex, ey) = (b.0 - a.0, b.1 - a.1);
        let (wx, wy) = (px - a.0, py - a.1);
        let h = ((wx * ex + wy * ey) / (ex * ex + ey * ey)).clamp(0.0, 1.0);
        let (dx, dy) = (wx - ex * h, wy - ey * h);
        d2 = d2.min(dx * dx + dy * dy);
        if (a.1 > py) != (b.1 > py) && px < (b.0 - a.0) * (py - a.1) / (b.1 - a.1) + a.0 {
            inside = !inside;
        }
    }
    if inside { -d2.sqrt() } else { d2.sqrt() }
}

/// The shift key glyph, `h` px tall, centred on `c`; `t` is the stroke width.
pub fn shift_glyph(s: &mut Surface, (cx, cy): (f32, f32), h: f32, t: f32, state: ShiftState, color: u32) {
    let lift = if state == ShiftState::Locked { -0.08 } else { 0.0 };
    let arrow: Vec<(f32, f32)> = [(0.0, -0.5), (0.5, 0.04), (0.2, 0.04), (0.2, 0.36), (-0.2, 0.36), (-0.2, 0.04), (-0.5, 0.04)]
        .iter()
        .map(|&(x, y)| (cx + x * h, cy + (y + lift) * h))
        .collect();
    let pad = t + 2.0;
    let rect = (cx - 0.5 * h - pad, cy - 0.5 * h - pad, h + 2.0 * pad, h * 1.1 + 2.0 * pad);
    match state {
        ShiftState::Off => paint(s, rect, color, |x, y| poly(&arrow, x, y).abs() - t / 2.0),
        _ => paint(s, rect, color, |x, y| poly(&arrow, x, y)),
    }
    if state == ShiftState::Locked {
        let (y0, y1) = (cy + 0.46 * h, cy + 0.56 * h);
        let bar = [(cx - 0.2 * h, y0), (cx + 0.2 * h, y0), (cx + 0.2 * h, y1), (cx - 0.2 * h, y1)];
        paint(s, rect, color, |x, y| poly(&bar, x, y));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_follows_flags() {
        assert_eq!(ShiftState::of(false, false), ShiftState::Off);
        assert_eq!(ShiftState::of(true, false), ShiftState::On);
        assert_eq!(ShiftState::of(true, true), ShiftState::Locked);
    }

    #[test]
    fn polygon_distance_signs() {
        let sq = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        assert!(poly(&sq, 5.0, 5.0) < 0.0);
        assert!(poly(&sq, 15.0, 5.0) > 0.0);
    }
}
