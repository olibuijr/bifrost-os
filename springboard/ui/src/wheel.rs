//! Scroll wheel events (scrollWheel): `scrollingDeltaX/Y`, `hasPreciseScrollingDeltas`, `phase` and
//! `momentumPhase`. Pure model: the shell feeds raw wheel notches or touchpad pixel deltas and gets the event to
//! deliver to `scrollWheel(with:)`.

/// Pixels one wheel line is worth when a device reports lines (a notched mouse wheel).
pub const LINE_HEIGHT: f32 = 10.0;

/// `Event.Phase`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    None,
    Began,
    Changed,
    Ended,
    Cancelled,
    MayBegin,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollEvent {
    pub scrolling_delta_x: f32,
    pub scrolling_delta_y: f32,
    pub has_precise_scrolling_deltas: bool,
    pub phase: Phase,
    pub momentum_phase: Phase,
}

/// Raw input from the compositor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Raw {
    /// Wheel notches: not precise, a fixed number of lines per notch.
    Lines(f32, f32),
    /// Touchpad pixels: precise.
    Pixels(f32, f32),
    /// Fingers lifted from the touchpad (ends the gesture phase; momentum may follow).
    FingersUp,
    /// A momentum step after the lift, in pixels.
    Momentum(f32, f32),
    /// Momentum ran out.
    MomentumEnd,
}

/// Turns raw input into events with a consistent phase sequence: Began, Changed..., Ended, then momentum
/// Began, Changed..., Ended. A notched wheel has no phases.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sequencer {
    touching: bool,
    coasting: bool,
}

impl Sequencer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, raw: Raw) -> Option<ScrollEvent> {
        match raw {
            Raw::Lines(x, y) => Some(ScrollEvent {
                scrolling_delta_x: x * LINE_HEIGHT,
                scrolling_delta_y: y * LINE_HEIGHT,
                has_precise_scrolling_deltas: false,
                phase: Phase::None,
                momentum_phase: Phase::None,
            }),
            Raw::Pixels(x, y) => {
                let phase = if self.touching { Phase::Changed } else { Phase::Began };
                self.touching = true;
                self.coasting = false;
                Some(precise(x, y, phase, Phase::None))
            }
            Raw::FingersUp => {
                if !self.touching {
                    return None;
                }
                self.touching = false;
                Some(precise(0., 0., Phase::Ended, Phase::None))
            }
            Raw::Momentum(x, y) => {
                let mp = if self.coasting { Phase::Changed } else { Phase::Began };
                self.coasting = true;
                Some(precise(x, y, Phase::None, mp))
            }
            Raw::MomentumEnd => {
                if !self.coasting {
                    return None;
                }
                self.coasting = false;
                Some(precise(0., 0., Phase::None, Phase::Ended))
            }
        }
    }
}

fn precise(x: f32, y: f32, phase: Phase, momentum_phase: Phase) -> ScrollEvent {
    ScrollEvent { scrolling_delta_x: x, scrolling_delta_y: y, has_precise_scrolling_deltas: true, phase, momentum_phase }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notched_wheel_is_imprecise_lines_without_phases() {
        let e = Sequencer::new().feed(Raw::Lines(0., -3.)).unwrap();
        assert_eq!(e.scrolling_delta_y, -30.);
        assert!(!e.has_precise_scrolling_deltas);
        assert_eq!((e.phase, e.momentum_phase), (Phase::None, Phase::None));
    }

    #[test]
    fn touchpad_sequence_began_changed_ended_then_momentum() {
        let mut s = Sequencer::new();
        let ph = |e: Option<ScrollEvent>| e.map(|e| (e.phase, e.momentum_phase));
        assert_eq!(ph(s.feed(Raw::Pixels(0., 5.))), Some((Phase::Began, Phase::None)));
        assert_eq!(ph(s.feed(Raw::Pixels(0., 6.))), Some((Phase::Changed, Phase::None)));
        assert_eq!(ph(s.feed(Raw::FingersUp)), Some((Phase::Ended, Phase::None)));
        assert_eq!(ph(s.feed(Raw::Momentum(0., 4.))), Some((Phase::None, Phase::Began)));
        assert_eq!(ph(s.feed(Raw::Momentum(0., 2.))), Some((Phase::None, Phase::Changed)));
        assert_eq!(ph(s.feed(Raw::MomentumEnd)), Some((Phase::None, Phase::Ended)));
        assert_eq!(ph(s.feed(Raw::MomentumEnd)), None, "nothing left to end");
        assert_eq!(ph(s.feed(Raw::FingersUp)), None);
    }

    #[test]
    fn a_new_touch_stops_momentum_and_begins_again() {
        let mut s = Sequencer::new();
        s.feed(Raw::Pixels(0., 1.));
        s.feed(Raw::FingersUp);
        s.feed(Raw::Momentum(0., 1.));
        let e = s.feed(Raw::Pixels(0., 1.)).unwrap();
        assert_eq!(e.phase, Phase::Began);
        assert!(e.has_precise_scrolling_deltas);
        assert_eq!(s.feed(Raw::MomentumEnd), None);
    }
}
