//! Pointer and touchpad choices from Settings (Mouse and Trackpad panes), read by `input` while it
//! turns device events into pointer motion, clicks and scrolling. Settings publishes a fresh `Tuning`
//! whenever the user changes one; the decoders read it on every event, so a change applies at once.

use std::sync::Mutex;

/// How a touchpad makes a secondary (right) click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Secondary {
    /// a press in the bottom-right corner of the panel
    Corner,
    /// a press with two fingers down
    TwoFingers,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    /// multiplier on mouse travel (0.25..=3.0)
    pub mouse_speed: f32,
    /// multiplier on wheel steps (0.25..=3.0)
    pub mouse_scroll: f32,
    /// multiplier on touchpad finger travel (0.25..=3.0)
    pub trackpad_speed: f32,
    /// wheel direction follows the content, like a touch screen
    pub natural_scroll: bool,
    /// a quick tap on a touchpad clicks
    pub tap_to_click: bool,
    pub secondary: Secondary,
    /// the mouse's left and right buttons trade places
    pub swap_buttons: bool,
}

impl Tuning {
    pub const DEFAULT: Tuning = Tuning { mouse_speed: 1.0, mouse_scroll: 1.0, trackpad_speed: 1.0, natural_scroll: false, tap_to_click: false, secondary: Secondary::TwoFingers, swap_buttons: false };
}

static TUNING: Mutex<Tuning> = Mutex::new(Tuning::DEFAULT);

/// Publishes the choices the decoders use from now on.
pub fn set(t: Tuning) {
    if let Ok(mut g) = TUNING.lock() {
        *g = t;
    }
}

/// The choices now in force (the defaults if the lock is poisoned).
pub fn get() -> Tuning {
    TUNING.lock().map(|g| *g).unwrap_or(Tuning::DEFAULT)
}

/// A wheel step after the scroll speed and the natural-direction choice. A non-zero step never
/// rounds away to nothing, so a slow setting still scrolls.
pub fn scroll_step(v: i32, speed: f32, natural: bool) -> i32 {
    if v == 0 {
        return 0;
    }
    let scaled = (v as f32 * speed).round() as i32;
    let scaled = if scaled == 0 { v.signum() } else { scaled };
    if natural { -scaled } else { scaled }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_steps_follow_speed_and_direction() {
        assert_eq!(scroll_step(0, 2.0, true), 0);
        assert_eq!(scroll_step(3, 1.0, false), 3);
        assert_eq!(scroll_step(3, 1.0, true), -3);
        assert_eq!(scroll_step(-2, 2.0, false), -4);
        assert_eq!(scroll_step(1, 0.25, false), 1, "a small step never rounds to zero");
        assert_eq!(scroll_step(-1, 0.25, true), 1);
    }

    #[test]
    fn set_and_get_round_trip() {
        let t = Tuning { tap_to_click: true, ..Tuning::DEFAULT };
        set(t);
        assert_eq!(get(), t);
        set(Tuning::DEFAULT);
        assert_eq!(get(), Tuning::DEFAULT);
    }
}
