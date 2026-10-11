//! Hardware-key chords: Power and Volume Up pressed together within a
//! short window take a screenshot, like an bifrost with no Home button. A lone
//! Power press is held back for that window (or until the key is released,
//! whichever comes first) so the chord never also turns the screen off.

use super::Event;

/// Both keys must go down within this many milliseconds of each other.
pub const WINDOW_MS: f64 = 150.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Power,
    VolumeUp,
    VolumeDown,
}

#[derive(Debug, Default)]
pub struct Chord {
    /// a Power press waiting to see whether Volume Up follows
    pending_power: Option<f64>,
    /// when Volume Up went down, while it is held
    volume_up: Option<f64>,
    /// the current Power press already produced its event
    power_spent: bool,
}

impl Chord {
    /// Feeds a key transition; returns the shell event it resolves to now.
    pub fn key(&mut self, key: Key, down: bool, t: f64) -> Option<Event> {
        match (key, down) {
            (Key::Power, true) => {
                self.power_spent = false;
                if self.volume_up.is_some_and(|v| t - v <= WINDOW_MS) {
                    self.power_spent = true;
                    self.volume_up = None;
                    return Some(Event::Screenshot);
                }
                self.pending_power = Some(t);
                None
            }
            (Key::Power, false) => {
                // released before the window closed: an ordinary short press
                let press = self.pending_power.take().filter(|_| !self.power_spent).map(|_| Event::Power);
                self.power_spent = true;
                press
            }
            (Key::VolumeUp, true) => {
                if self.pending_power.is_some_and(|p| t - p <= WINDOW_MS) {
                    self.pending_power = None;
                    self.power_spent = true;
                    return Some(Event::Screenshot);
                }
                self.volume_up = Some(t);
                None
            }
            (Key::VolumeUp, false) => {
                // not spent on a chord: an ordinary volume step
                self.volume_up.take().map(|_| Event::VolumeUp)
            }
            (Key::VolumeDown, true) => Some(Event::VolumeDown),
            (Key::VolumeDown, false) => None,
        }
    }

    /// When a held-back Power press must be delivered, if one is waiting.
    pub fn deadline(&self) -> Option<f64> {
        self.pending_power.map(|p| p + WINDOW_MS)
    }

    /// Delivers the held-back Power press once its window has closed.
    pub fn due(&mut self, t: f64) -> Option<Event> {
        if self.pending_power.is_some_and(|p| t - p >= WINDOW_MS) {
            self.pending_power = None;
            self.power_spent = true;
            return Some(Event::Power);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_then_volume_up_within_the_window_is_a_screenshot() {
        let mut c = Chord::default();
        assert_eq!(c.key(Key::Power, true, 1000.0), None);
        assert_eq!(c.deadline(), Some(1150.0));
        assert_eq!(c.key(Key::VolumeUp, true, 1080.0), Some(Event::Screenshot));
        assert_eq!(c.deadline(), None, "the power press was consumed");
        assert_eq!(c.key(Key::Power, false, 1300.0), None, "releasing it does nothing more");
        assert_eq!(c.key(Key::VolumeUp, false, 1310.0), None);
        assert_eq!(c.due(2000.0), None);
    }

    #[test]
    fn volume_up_then_power_within_the_window_is_a_screenshot() {
        let mut c = Chord::default();
        assert_eq!(c.key(Key::VolumeUp, true, 500.0), None);
        assert_eq!(c.key(Key::Power, true, 600.0), Some(Event::Screenshot));
        assert_eq!(c.key(Key::Power, false, 700.0), None);
        assert_eq!(c.due(900.0), None);
    }

    #[test]
    fn a_lone_power_press_is_delivered_after_the_window_or_on_release() {
        let mut c = Chord::default();
        assert_eq!(c.key(Key::Power, true, 0.0), None);
        assert_eq!(c.due(100.0), None, "still waiting");
        assert_eq!(c.due(150.0), Some(Event::Power));
        assert_eq!(c.due(160.0), None, "delivered once");
        assert_eq!(c.key(Key::Power, false, 400.0), None);
        // a quick tap resolves on release without waiting
        assert_eq!(c.key(Key::Power, true, 1000.0), None);
        assert_eq!(c.key(Key::Power, false, 1040.0), Some(Event::Power));
        assert_eq!(c.deadline(), None);
    }

    #[test]
    fn keys_too_far_apart_do_not_chord() {
        let mut c = Chord::default();
        c.key(Key::VolumeUp, true, 0.0);
        assert_eq!(c.key(Key::Power, true, 200.0), None, "volume up is stale");
        assert_eq!(c.due(350.0), Some(Event::Power));
        assert_eq!(c.key(Key::VolumeUp, true, 400.0), None, "power already delivered");
        assert_eq!(c.key(Key::VolumeUp, false, 450.0), Some(Event::VolumeUp), "an unspent volume press is a step");
        c.key(Key::Power, false, 460.0);
        assert_eq!(c.key(Key::VolumeDown, true, 500.0), Some(Event::VolumeDown));
        assert_eq!(c.key(Key::Power, true, 510.0), None, "volume down is not part of the chord");
        assert_eq!(c.due(700.0), Some(Event::Power));
    }

    #[test]
    fn plain_volume_presses_are_steps_and_a_chorded_one_is_not() {
        let mut c = Chord::default();
        assert_eq!(c.key(Key::VolumeDown, true, 0.0), Some(Event::VolumeDown));
        assert_eq!(c.key(Key::VolumeDown, false, 50.0), None);
        assert_eq!(c.key(Key::VolumeUp, true, 100.0), None, "volume up waits for its release");
        assert_eq!(c.key(Key::VolumeUp, false, 150.0), Some(Event::VolumeUp));
        c.key(Key::VolumeUp, true, 1000.0);
        assert_eq!(c.key(Key::Power, true, 1050.0), Some(Event::Screenshot));
        assert_eq!(c.key(Key::VolumeUp, false, 1100.0), None, "spent on the chord");
    }

    #[test]
    fn holding_the_chord_fires_once() {
        let mut c = Chord::default();
        c.key(Key::VolumeUp, true, 0.0);
        assert_eq!(c.key(Key::Power, true, 50.0), Some(Event::Screenshot));
        assert_eq!(c.due(1000.0), None);
        assert_eq!(c.key(Key::Power, false, 2000.0), None);
    }
}
