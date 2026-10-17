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
    /// the longest gap in ms between two presses that still count as a double-click (title-bar zoom)
    pub double_click_ms: u32,
    /// pointer acceleration: a fast mouse move travels further (see `accelerate`)
    pub accel: bool,
}

impl Tuning {
    pub const DEFAULT: Tuning = Tuning {
        mouse_speed: 1.0,
        mouse_scroll: 1.0,
        trackpad_speed: 1.0,
        natural_scroll: false,
        tap_to_click: false,
        secondary: Secondary::TwoFingers,
        swap_buttons: false,
        double_click_ms: crate::shell::mouse_page::DOUBLE_CLICK_DEFAULT_MS,
        accel: true,
    };
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

/// Mouse travel per event (in device counts) above which pointer acceleration starts to add distance.
pub const ACCEL_KNEE: f32 = 2.0;
/// Extra gain per count past the knee.
pub const ACCEL_PER_COUNT: f32 = 0.1;
/// The most the acceleration multiplies a move by.
pub const ACCEL_MAX: f32 = 2.5;

/// The gain pointer acceleration gives a mouse move of `magnitude` counts: 1 up to the knee, then rising with speed.
pub fn accel_gain(magnitude: f32) -> f32 {
    if magnitude <= ACCEL_KNEE {
        1.0
    } else {
        (1.0 + (magnitude - ACCEL_KNEE) * ACCEL_PER_COUNT).min(ACCEL_MAX)
    }
}

/// A mouse move after pointer acceleration: unchanged when acceleration is off, else scaled by the gain of its length.
pub fn accelerate(dx: f32, dy: f32, on: bool) -> (f32, f32) {
    if !on {
        return (dx, dy);
    }
    let g = accel_gain(dx.hypot(dy));
    (dx * g, dy * g)
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
    fn acceleration_is_linear_below_the_knee_and_capped() {
        assert_eq!(accelerate(1.0, 1.0, true), (1.0, 1.0), "slow moves are not sped up");
        assert_eq!(accelerate(10.0, 0.0, false), (10.0, 0.0), "off means the raw travel");
        let (x, _) = accelerate(10.0, 0.0, true);
        assert!(x > 10.0, "a fast move travels further with acceleration on");
        assert!((accel_gain(1000.0) - ACCEL_MAX).abs() < 1e-6, "the gain is capped");
        assert!(accel_gain(5.0) < accel_gain(8.0), "the gain rises with speed");
    }

    #[test]
    fn the_default_double_click_gap_is_the_shell_default() {
        assert_eq!(Tuning::DEFAULT.double_click_ms, crate::shell::mouse_page::DOUBLE_CLICK_DEFAULT_MS);
        assert!(Tuning::DEFAULT.accel);
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
