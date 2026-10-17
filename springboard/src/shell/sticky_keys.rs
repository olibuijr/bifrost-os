//! Sticky keys: a modifier tapped once latches for the next key press; tapped twice it locks until tapped again.
//!
//! Pure state. `HwKeys` (`hwkeys.rs`) owns one `StickyKeys`: a modifier key press calls `tap_modifier`, and a
//! non-modifier key press calls `take_for_key`, which counts the returned modifiers as held until that key is
//! released. Every shortcut and typing decision then sees them like a physical hold.

/// The modifiers sticky keys can hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    Shift,
    Ctrl,
    Alt,
    Super,
}

const ALL: [Modifier; 4] = [Modifier::Shift, Modifier::Ctrl, Modifier::Alt, Modifier::Super];

impl Modifier {
    /// The modifier a Linux input key code (evdev `KEY_*`) stands for, both sides of each pair. None for any other key.
    pub fn of_code(code: u16) -> Option<Modifier> {
        match code {
            42 | 54 => Some(Modifier::Shift),
            29 | 97 => Some(Modifier::Ctrl),
            56 | 100 => Some(Modifier::Alt),
            125 | 126 => Some(Modifier::Super),
            _ => None,
        }
    }

    /// The left-hand key code of this modifier (the one the shell synthesises for a latched modifier).
    pub fn code(self) -> u16 {
        match self {
            Modifier::Shift => 42,
            Modifier::Ctrl => 29,
            Modifier::Alt => 56,
            Modifier::Super => 125,
        }
    }
}

/// Where one modifier stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Latch {
    Off,
    /// Applies to the next key press, then clears.
    Once,
    /// Applies to every key press until tapped again.
    Locked,
}

/// The sticky-keys state for all modifiers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StickyKeys {
    enabled: bool,
    latch: [Latch; 4],
}

impl Default for StickyKeys {
    fn default() -> Self {
        Self::new(false)
    }
}

impl StickyKeys {
    pub fn new(enabled: bool) -> Self {
        Self { enabled, latch: [Latch::Off; 4] }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Turning sticky keys off releases every modifier.
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            self.latch = [Latch::Off; 4];
        }
    }

    pub fn latch_of(&self, m: Modifier) -> Latch {
        self.latch[index(m)]
    }

    /// A modifier key tap: Off to Once, Once to Locked, Locked to Off. Ignored while sticky keys are off.
    pub fn tap_modifier(&mut self, m: Modifier) {
        if !self.enabled {
            return;
        }
        let slot = &mut self.latch[index(m)];
        *slot = match *slot {
            Latch::Off => Latch::Once,
            Latch::Once => Latch::Locked,
            Latch::Locked => Latch::Off,
        };
    }

    /// Called for a non-modifier key press. Returns the modifiers that apply to it, in a fixed order, and clears the
    /// one-shot latches. Locked modifiers stay in place.
    pub fn take_for_key(&mut self) -> Vec<Modifier> {
        if !self.enabled {
            return Vec::new();
        }
        let mut held = Vec::new();
        for (i, m) in ALL.iter().enumerate() {
            match self.latch[i] {
                Latch::Off => {}
                Latch::Once => {
                    held.push(*m);
                    self.latch[i] = Latch::Off;
                }
                Latch::Locked => held.push(*m),
            }
        }
        held
    }
}

fn index(m: Modifier) -> usize {
    ALL.iter().position(|x| *x == m).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_tap_applies_to_one_key() {
        let mut s = StickyKeys::new(true);
        s.tap_modifier(Modifier::Shift);
        assert_eq!(s.take_for_key(), vec![Modifier::Shift]);
        assert_eq!(s.take_for_key(), Vec::<Modifier>::new());
        assert_eq!(s.latch_of(Modifier::Shift), Latch::Off);
    }

    #[test]
    fn double_tap_locks_until_third_tap() {
        let mut s = StickyKeys::new(true);
        s.tap_modifier(Modifier::Ctrl);
        s.tap_modifier(Modifier::Ctrl);
        assert_eq!(s.take_for_key(), vec![Modifier::Ctrl]);
        assert_eq!(s.take_for_key(), vec![Modifier::Ctrl]);
        s.tap_modifier(Modifier::Ctrl);
        assert_eq!(s.take_for_key(), Vec::<Modifier>::new());
    }

    #[test]
    fn several_modifiers_come_back_in_fixed_order() {
        let mut s = StickyKeys::new(true);
        s.tap_modifier(Modifier::Super);
        s.tap_modifier(Modifier::Alt);
        assert_eq!(s.take_for_key(), vec![Modifier::Alt, Modifier::Super]);
    }

    #[test]
    fn taps_are_ignored_while_disabled() {
        let mut s = StickyKeys::default();
        s.tap_modifier(Modifier::Shift);
        assert_eq!(s.latch_of(Modifier::Shift), Latch::Off);
        assert!(s.take_for_key().is_empty());
    }

    #[test]
    fn key_codes_map_both_sides_to_one_modifier() {
        assert_eq!(Modifier::of_code(42), Some(Modifier::Shift));
        assert_eq!(Modifier::of_code(54), Some(Modifier::Shift));
        assert_eq!(Modifier::of_code(29), Some(Modifier::Ctrl));
        assert_eq!(Modifier::of_code(97), Some(Modifier::Ctrl));
        assert_eq!(Modifier::of_code(56), Some(Modifier::Alt));
        assert_eq!(Modifier::of_code(100), Some(Modifier::Alt));
        assert_eq!(Modifier::of_code(126), Some(Modifier::Super));
        assert_eq!(Modifier::of_code(30), None, "the A key is not a modifier");
        for m in ALL {
            assert_eq!(Modifier::of_code(m.code()), Some(m));
        }
    }

    #[test]
    fn disabling_releases_locks() {
        let mut s = StickyKeys::new(true);
        s.tap_modifier(Modifier::Shift);
        s.tap_modifier(Modifier::Shift);
        s.set_enabled(false);
        s.set_enabled(true);
        assert!(s.take_for_key().is_empty());
    }
}
