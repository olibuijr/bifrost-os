//! Mouse Keys: the number keypad moves the pointer, and the 5 key clicks.
//!
//! Pure state. The shell's input loop passes each key event through `press`, which says whether the key is ours
//! (a movement key or the click key, swallowed so it does not type) and whether a press should click. Once a frame
//! `step` returns how far the pointer moves for the time since the last step, so a held key accelerates: it starts
//! slowly and reaches the top speed after `RAMP_MS`. Diagonals are scaled so they are not faster than straight moves.

use std::f32::consts::FRAC_1_SQRT_2;

/// Evdev code of the keypad 5 key, which clicks.
pub const CLICK_KEY: u16 = 76;
/// Pointer speed in points per second when a movement key is first held.
pub const START_PX_PER_S: f32 = 120.0;
/// Pointer speed in points per second after a movement key has been held for `RAMP_MS`.
pub const TOP_PX_PER_S: f32 = 900.0;
/// Milliseconds a movement key must be held to reach `TOP_PX_PER_S`.
pub const RAMP_MS: f64 = 1500.0;
/// How often the loop wakes while the pointer is moving, in milliseconds.
const FRAME_MS: f64 = 16.0;

/// What the input loop does with one key event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// Not a Mouse Keys key: the event goes on to the key path.
    Pass,
    /// A movement key or the release of the click key: swallowed.
    Consumed,
    /// The click key went down: click at the pointer.
    Click,
}

/// The unit direction (x, y, with y growing downwards) a keypad movement key moves in, or None for other keys.
pub fn direction(code: u16) -> Option<(f32, f32)> {
    Some(match code {
        71 => (-1.0, -1.0), // 7
        72 => (0.0, -1.0),  // 8
        73 => (1.0, -1.0),  // 9
        75 => (-1.0, 0.0),  // 4
        77 => (1.0, 0.0),   // 6
        79 => (-1.0, 1.0),  // 1
        80 => (0.0, 1.0),   // 2
        81 => (1.0, 1.0),   // 3
        _ => return None,
    })
}

/// The Mouse Keys state: whether it is on, the movement keys held, and the timing of the current move.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MouseKeys {
    enabled: bool,
    /// movement keys currently held, in the order they went down
    held: Vec<u16>,
    /// when the current run of held movement keys began (ms)
    since: f64,
    /// when `step` last ran (ms)
    last: f64,
}

impl MouseKeys {
    /// A model that is on or off as given.
    pub fn new(enabled: bool) -> Self {
        Self { enabled, held: Vec::new(), since: 0.0, last: 0.0 }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Turning Mouse Keys off forgets held keys, so a later enable starts still.
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        self.held.clear();
    }

    /// One key event at `t` (ms). Movement keys are tracked by press and release; the click key clicks on press.
    pub fn press(&mut self, code: u16, down: bool, t: f64) -> Press {
        if !self.enabled {
            return Press::Pass;
        }
        if code == CLICK_KEY {
            return if down { Press::Click } else { Press::Consumed };
        }
        if direction(code).is_none() {
            return Press::Pass;
        }
        if down {
            if !self.held.contains(&code) {
                if self.held.is_empty() {
                    self.since = t;
                    self.last = t;
                }
                self.held.push(code);
            }
        } else {
            self.held.retain(|&c| c != code);
        }
        Press::Consumed
    }

    /// How far the pointer moves (dx, dy) for the time since the last step, with the held keys' acceleration.
    pub fn step(&mut self, t: f64) -> (f32, f32) {
        if self.held.is_empty() {
            self.last = t;
            return (0.0, 0.0);
        }
        let secs = ((t - self.last).max(0.0) / 1000.0) as f32;
        self.last = t;
        let ramp = ((t - self.since).max(0.0) / RAMP_MS).min(1.0) as f32;
        let speed = START_PX_PER_S + (TOP_PX_PER_S - START_PX_PER_S) * ramp;
        let (mut x, mut y) = (0.0f32, 0.0f32);
        for &code in &self.held {
            if let Some((dx, dy)) = direction(code) {
                x += dx;
                y += dy;
            }
        }
        // two keys at once (a diagonal) move as far as one straight key
        if x != 0.0 && y != 0.0 {
            x *= FRAC_1_SQRT_2;
            y *= FRAC_1_SQRT_2;
        }
        (x * speed * secs, y * speed * secs)
    }

    /// The next time the loop must wake to keep the pointer moving (ms), or None when no movement key is held.
    pub fn deadline(&self) -> Option<f64> {
        (self.enabled && !self.held.is_empty()).then_some(self.last + FRAME_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keypad_directions_cover_the_eight_neighbours() {
        assert_eq!(direction(71), Some((-1.0, -1.0)));
        assert_eq!(direction(80), Some((0.0, 1.0)));
        assert_eq!(direction(77), Some((1.0, 0.0)));
        assert_eq!(direction(30), None);
    }

    #[test]
    fn off_passes_every_key() {
        let mut m = MouseKeys::new(false);
        assert_eq!(m.press(77, true, 0.0), Press::Pass);
        assert_eq!(m.press(CLICK_KEY, true, 0.0), Press::Pass);
        assert_eq!(m.step(100.0), (0.0, 0.0));
    }

    #[test]
    fn movement_keys_are_swallowed_and_other_keys_pass() {
        let mut m = MouseKeys::new(true);
        assert_eq!(m.press(77, true, 0.0), Press::Consumed);
        assert_eq!(m.press(77, false, 10.0), Press::Consumed);
        assert_eq!(m.press(30, true, 10.0), Press::Pass, "letters still type");
    }

    #[test]
    fn the_five_key_clicks_on_press_only() {
        let mut m = MouseKeys::new(true);
        assert_eq!(m.press(CLICK_KEY, true, 0.0), Press::Click);
        assert_eq!(m.press(CLICK_KEY, false, 50.0), Press::Consumed);
    }

    #[test]
    fn a_held_key_accelerates_toward_the_top_speed() {
        let mut m = MouseKeys::new(true);
        m.press(77, true, 0.0);
        let (slow, _) = m.step(100.0);
        let (fast, _) = m.step(100.0 + 1500.0);
        // the step over the ramp covers much more ground per millisecond than the first 100 ms
        assert!(slow > 0.0);
        assert!(fast / 1500.0 > slow / 100.0);
        // past the ramp the speed stops growing: one second at the top speed moves exactly TOP_PX_PER_S
        m.step(10_000.0);
        let (top, _) = m.step(11_000.0);
        assert!((top - TOP_PX_PER_S).abs() < 1e-3, "top step {top}");
    }

    #[test]
    fn a_diagonal_is_not_faster_than_a_straight_move() {
        let mut m = MouseKeys::new(true);
        m.press(77, true, 0.0);
        m.step(0.0);
        let (straight, _) = m.step(100.0);
        m.press(77, false, 100.0);
        m.press(80, true, 100.0);
        m.step(100.0);
        m.press(77, true, 100.0);
        let (dx, dy) = m.step(200.0);
        let diagonal = (dx * dx + dy * dy).sqrt();
        assert!(diagonal <= straight * 1.01, "diagonal {diagonal} straight {straight}");
    }

    #[test]
    fn releasing_every_key_stops_the_pointer_and_clears_the_wakeup() {
        let mut m = MouseKeys::new(true);
        m.press(80, true, 0.0);
        assert!(m.deadline().is_some());
        m.press(80, false, 50.0);
        assert_eq!(m.deadline(), None);
        assert_eq!(m.step(100.0), (0.0, 0.0));
    }

    #[test]
    fn turning_off_drops_held_keys() {
        let mut m = MouseKeys::new(true);
        m.press(80, true, 0.0);
        m.set_enabled(false);
        m.set_enabled(true);
        assert_eq!(m.deadline(), None);
    }
}
