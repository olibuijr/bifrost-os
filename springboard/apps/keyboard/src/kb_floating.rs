//! Floating/undock/split support for the keyboard: the long-press menu on the
//! dismiss key (Dock / Split / Floating), the pinch gesture and the bottom
//! grabber that drags the floating keyboard. Pure state; `keyboard.rs` owns
//! the mode and applies what this returns.

use crate::keyboard::Mode;

/// Pinch ratio (current finger distance / start distance) that switches mode.
pub const PINCH_IN: f32 = 0.8;
pub const PINCH_OUT: f32 = 1.25;
/// Menu row height and width in points.
pub const ROW_PT: f32 = 44.0;
pub const MENU_W_PT: f32 = 200.0;
/// Grabber band below the floating keys and its pill size in points.
pub const GRABBER_BAND_PT: f32 = 18.0;
pub const GRABBER_W_PT: f32 = 64.0;
pub const GRABBER_H_PT: f32 = 5.0;

pub const ITEMS: [(&str, Mode); 3] = [("Dock", Mode::Docked), ("Split", Mode::Split), ("Floating", Mode::Floating)];

/// The dismiss-key menu: top-left in keyboard coordinates plus the row the
/// finger is on.
#[derive(Clone, Copy, Debug)]
pub struct Menu {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub row_h: f32,
    pub sel: usize,
}

impl Menu {
    /// Menu anchored above the key at (`kx`, `ky`) width `kw`, right-aligned
    /// to it and kept inside `screen_w`; `s` is px per point.
    pub fn new(kx: f32, ky: f32, kw: f32, screen_w: f32, s: f32, mode: Mode) -> Menu {
        let (w, row_h) = (MENU_W_PT * s, ROW_PT * s);
        let x = (kx + kw - w).clamp(0.0, (screen_w - w).max(0.0));
        let y = ky - 8.0 * s - row_h * ITEMS.len() as f32;
        let sel = ITEMS.iter().position(|(_, m)| *m == mode).unwrap_or(0);
        Menu { x, y, w, row_h, sel }
    }

    pub fn rect(&self) -> (f32, f32, f32, f32) {
        (self.x, self.y, self.w, self.row_h * ITEMS.len() as f32)
    }

    /// Follows the finger vertically.
    pub fn track(&mut self, y: f32) {
        let i = ((y - self.y) / self.row_h).floor().max(0.0) as usize;
        self.sel = i.min(ITEMS.len() - 1);
    }

    pub fn chosen(&self) -> Mode {
        ITEMS[self.sel].1
    }
}

/// Two-finger pinch on the keyboard.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pinch {
    start: Option<f32>,
}

impl Pinch {
    /// Feed the distance between the two fingers; returns the mode to switch
    /// to once the ratio crosses a threshold (once per gesture).
    pub fn update(&mut self, dist: f32, mode: Mode) -> Option<Mode> {
        let start = *self.start.get_or_insert(dist);
        if start <= 0.0 {
            return None;
        }
        let r = dist / start;
        let next = if r <= PINCH_IN && mode != Mode::Floating {
            Mode::Floating
        } else if r >= PINCH_OUT && mode == Mode::Floating {
            Mode::Docked
        } else {
            return None;
        };
        self.start = Some(dist);
        Some(next)
    }

    pub fn end(&mut self) {
        self.start = None;
    }
}

/// Whether (x, y) is on the floating keyboard's grabber band, given the
/// keyboard's x extent and the bottom of its last key row.
pub fn on_grabber(x: f32, y: f32, lo: f32, hi: f32, keys_bottom: f32, s: f32) -> bool {
    x >= lo && x <= hi && y >= keys_bottom && y <= keys_bottom + GRABBER_BAND_PT * s
}

/// Pill rect (x, y, w, h) for the grabber, centred in the keyboard extent.
pub fn grabber_rect(lo: f32, hi: f32, keys_bottom: f32, s: f32) -> (f32, f32, f32, f32) {
    let (w, h) = (GRABBER_W_PT * s, GRABBER_H_PT * s);
    ((lo + hi - w) / 2.0, keys_bottom + (GRABBER_BAND_PT * s - h) / 2.0, w, h)
}

/// Drags the floating keyboard: new left edge after moving by `dx`.
pub fn drag_x(float_x: f32, dx: f32, screen_w: f32, scale: f32) -> f32 {
    (float_x + dx).clamp(0.0, screen_w * (1.0 - scale))
}
