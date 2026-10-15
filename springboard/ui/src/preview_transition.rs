//! Geometry of the preview panel's transitions. On open the panel grows out of the on-screen frame of the item's
//! thumbnail, and on close it shrinks back into it. Full-screen mode fills the bounds less a margin. Progress runs
//! from 0 (the source frame) to 1 (the target frame) and is clamped, so an animation that overshoots lands on the
//! target. Nothing here draws or animates; the shell asks for the rectangle at each frame.

use crate::Rect;

/// Linear interpolation between two rectangles. `t` is clamped to 0..=1.
pub fn lerp_rect(from: Rect, to: Rect, t: f32) -> Rect {
    let t = t.clamp(0.0, 1.0);
    let mix = |a: f32, b: f32| a + (b - a) * t;
    Rect { x: mix(from.x, to.x), y: mix(from.y, to.y), w: mix(from.w, to.w), h: mix(from.h, to.h) }
}

/// The panel's own frame and whether full-screen mode is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelFrame {
    windowed: Rect,
    full_screen: bool,
}

impl PanelFrame {
    /// A panel in its windowed frame, not full screen.
    pub fn new(windowed: Rect) -> Self {
        PanelFrame { windowed, full_screen: false }
    }

    /// Whether full-screen mode is on.
    pub fn is_full_screen(&self) -> bool {
        self.full_screen
    }

    /// Switches full-screen mode and returns the new state.
    pub fn toggle_full_screen(&mut self) -> bool {
        self.full_screen = !self.full_screen;
        self.full_screen
    }

    /// The rectangle the panel rests at inside `bounds`: the windowed frame, or in full-screen mode the bounds inset
    /// by `margin` on every side. A margin larger than half the bounds leaves an empty rectangle, never a negative one.
    pub fn target(&self, bounds: Rect, margin: f32) -> Rect {
        if self.full_screen {
            Rect {
                x: bounds.x + margin,
                y: bounds.y + margin,
                w: (bounds.w - 2.0 * margin).max(0.0),
                h: (bounds.h - 2.0 * margin).max(0.0),
            }
        } else {
            self.windowed
        }
    }

    /// The rectangle at `progress` of an opening (0 at `source`, 1 at the target) or, with progress running
    /// backwards, of a closing.
    pub fn transition(&self, source: Rect, bounds: Rect, margin: f32, progress: f32) -> Rect {
        lerp_rect(source, self.target(bounds, margin), progress)
    }
}
