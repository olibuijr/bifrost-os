//! Keyboard trackpad mode: touch and hold the space bar and the keys fade
//! out; dragging then moves the text caret (as arrow-key steps) until the
//! finger lifts.

use crate::keyboard::Out;

/// Hold time on the space bar before the keyboard turns into a trackpad.
pub const HOLD_MS: f64 = 450.0;
/// Fade of the key labels in and out.
pub const FADE_MS: f64 = 180.0;
/// Finger travel in points per caret step.
const STEP_X_PT: f32 = 9.0;
const STEP_Y_PT: f32 = 22.0;

#[derive(Default)]
pub struct Trackpad {
    active: bool,
    /// (start, fading in) of the fade in progress
    fade: Option<(f64, bool)>,
    ax: f32,
    ay: f32,
}

impl Trackpad {
    pub fn active(&self) -> bool {
        self.active
    }

    /// True while a fade still needs frames.
    pub fn animating(&self) -> bool {
        self.fade.is_some()
    }

    pub fn start(&mut self, x: f32, y: f32, t: f64) {
        self.active = true;
        self.fade = Some((t, true));
        self.ax = x;
        self.ay = y;
    }

    pub fn end(&mut self, t: f64) {
        if self.active {
            self.active = false;
            self.fade = Some((t, false));
        }
    }

    /// Caret steps for a finger at (x, y); `s` is px per point.
    pub fn moved(&mut self, x: f32, y: f32, s: f32) -> Vec<Out> {
        let mut outs = Vec::new();
        let (sx, sy) = (STEP_X_PT * s, STEP_Y_PT * s);
        while x - self.ax >= sx {
            self.ax += sx;
            outs.push(Out::Named("Right"));
        }
        while self.ax - x >= sx {
            self.ax -= sx;
            outs.push(Out::Named("Left"));
        }
        while y - self.ay >= sy {
            self.ay += sy;
            outs.push(Out::Named("Down"));
        }
        while self.ay - y >= sy {
            self.ay -= sy;
            outs.push(Out::Named("Up"));
        }
        outs
    }

    /// 0 = keys fully visible, 1 = fully faded; retires finished fades.
    pub fn amount(&mut self, t: f64) -> f32 {
        match self.fade {
            Some((t0, on)) => {
                let p = ((t - t0) / FADE_MS).clamp(0.0, 1.0) as f32;
                if p >= 1.0 {
                    self.fade = None;
                }
                if on { p } else { 1.0 - p }
            }
            None => if self.active { 1.0 } else { 0.0 },
        }
    }
}
