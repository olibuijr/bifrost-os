//! Racing wheel input model: the steering wheel, the throttle and brake pedals and a sequential gear shifter, fed
//! from raw evdev records. The steering axis is centred and maps to -1..=1; the pedals map to 0..=1. Each gear
//! button press moves the gear one step, so gear 0 is neutral and the shifter stops at the top gear.
//!
//! Pure: no device and no force feedback. Paddles and a separate hat are not modelled yet.
//! Ranges come from `AxisRange`, as they do for a gamepad, so a wheel with another travel sets its own.

use crate::gamepad::{AxisRange, EV_ABS, EV_KEY};

/// `ABS_WHEEL`: the steering wheel angle.
const ABS_WHEEL: u16 = 0x08;
/// `ABS_GAS`: the throttle pedal.
const ABS_GAS: u16 = 0x09;
/// `ABS_BRAKE`: the brake pedal.
const ABS_BRAKE: u16 = 0x0a;
/// `BTN_GEAR_DOWN`: shift one gear down.
const BTN_GEAR_DOWN: u16 = 0x150;
/// `BTN_GEAR_UP`: shift one gear up.
const BTN_GEAR_UP: u16 = 0x151;

const STEERING: usize = 0;
const THROTTLE: usize = 1;
const BRAKE: usize = 2;

/// A change to the wheel state, reported once per moved value or per gear step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WheelChange {
    Steering(f32),
    Throttle(f32),
    Brake(f32),
    /// The gear now engaged: 0 is neutral, 1..=top gear forward.
    Gear(i8),
}

/// The state of one racing wheel.
#[derive(Clone, Debug, PartialEq)]
pub struct RacingWheel {
    values: [f32; 3],
    ranges: [AxisRange; 3],
    gear: i8,
    top_gear: i8,
}

impl RacingWheel {
    /// A wheel with a shifter of `top_gear` forward gears (at least one). The steering range is a 16-bit centred
    /// axis with no dead zone; both pedals run 0..=255 with no dead zone. The caller sets the real ranges with
    /// `set_range`.
    pub fn new(top_gear: i8) -> Self {
        let steering = AxisRange::stick(-32768, 32767, 0.0);
        let pedal = AxisRange::trigger(0, 255, 0.0);
        RacingWheel {
            values: [0.0; 3],
            ranges: [steering, pedal, pedal],
            gear: 0,
            top_gear: top_gear.max(1),
        }
    }

    /// Replaces the steering range. Use a centred range for the wheel and a unipolar range for each pedal.
    pub fn set_steering_range(&mut self, range: AxisRange) {
        self.ranges[STEERING] = range;
    }

    /// Replaces the range of the throttle or the brake. Pedal ranges are unipolar.
    pub fn set_pedal_range(&mut self, brake: bool, range: AxisRange) {
        self.ranges[if brake { BRAKE } else { THROTTLE }] = range;
    }

    /// Feeds one raw record. Returns the change it caused, or `None` for codes that are not wheel inputs, values
    /// that do not move, key releases and key repeats (value 2) on the gear buttons.
    pub fn feed(&mut self, ty: u16, code: u16, value: i32) -> Option<WheelChange> {
        match (ty, code) {
            (EV_ABS, ABS_WHEEL) => self.axis(STEERING, value).map(WheelChange::Steering),
            (EV_ABS, ABS_GAS) => self.axis(THROTTLE, value).map(WheelChange::Throttle),
            (EV_ABS, ABS_BRAKE) => self.axis(BRAKE, value).map(WheelChange::Brake),
            (EV_KEY, BTN_GEAR_UP) if value == 1 => self.shift(1),
            (EV_KEY, BTN_GEAR_DOWN) if value == 1 => self.shift(-1),
            _ => None,
        }
    }

    fn axis(&mut self, slot: usize, raw: i32) -> Option<f32> {
        let v = self.ranges[slot].normalize(raw);
        if v == self.values[slot] {
            return None;
        }
        self.values[slot] = v;
        Some(v)
    }

    fn shift(&mut self, step: i8) -> Option<WheelChange> {
        let next = (self.gear + step).clamp(0, self.top_gear);
        if next == self.gear {
            return None;
        }
        self.gear = next;
        Some(WheelChange::Gear(next))
    }

    /// The steering position, -1 (full left) to 1 (full right), 0 at centre.
    pub fn steering(&self) -> f32 {
        self.values[STEERING]
    }

    /// The throttle position, 0 to 1.
    pub fn throttle(&self) -> f32 {
        self.values[THROTTLE]
    }

    /// The brake position, 0 to 1.
    pub fn brake(&self) -> f32 {
        self.values[BRAKE]
    }

    /// The engaged gear: 0 is neutral.
    pub fn gear(&self) -> i8 {
        self.gear
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn steering_maps_the_device_range_to_minus_one_through_one() {
        let mut w = RacingWheel::new(4);
        w.set_steering_range(AxisRange::stick(0, 1000, 0.0));
        // Raw 0 is full left, raw 500 is centre, raw 1000 is full right.
        assert_eq!(w.feed(EV_ABS, ABS_WHEEL, 0), Some(WheelChange::Steering(-1.0)));
        assert_eq!(w.feed(EV_ABS, ABS_WHEEL, 500), Some(WheelChange::Steering(0.0)));
        assert_eq!(w.feed(EV_ABS, ABS_WHEEL, 1000), Some(WheelChange::Steering(1.0)));
        // The same value again does not move the state.
        assert_eq!(w.feed(EV_ABS, ABS_WHEEL, 1000), None);
        assert!(near(w.steering(), 1.0));
    }

    #[test]
    fn throttle_maps_the_pedal_range_to_zero_through_one() {
        let mut w = RacingWheel::new(4);
        w.set_pedal_range(false, AxisRange::trigger(0, 200, 0.0));
        // 100 of 200 is half travel.
        assert_eq!(w.feed(EV_ABS, ABS_GAS, 100), Some(WheelChange::Throttle(0.5)));
        assert_eq!(w.feed(EV_ABS, ABS_GAS, 200), Some(WheelChange::Throttle(1.0)));
        assert!(near(w.throttle(), 1.0));
        assert!(near(w.brake(), 0.0));
    }

    #[test]
    fn brake_past_the_top_of_its_range_clamps_to_one() {
        let mut w = RacingWheel::new(4);
        w.set_pedal_range(true, AxisRange::trigger(0, 200, 0.0));
        // 300 is 1.5 of the travel and clamps to 1.0.
        assert_eq!(w.feed(EV_ABS, ABS_BRAKE, 300), Some(WheelChange::Brake(1.0)));
        assert!(near(w.brake(), 1.0));
    }

    #[test]
    fn gear_up_steps_to_the_top_gear_and_stops_there() {
        let mut w = RacingWheel::new(3);
        assert_eq!(w.feed(EV_KEY, BTN_GEAR_UP, 1), Some(WheelChange::Gear(1)));
        assert_eq!(w.feed(EV_KEY, BTN_GEAR_UP, 1), Some(WheelChange::Gear(2)));
        assert_eq!(w.feed(EV_KEY, BTN_GEAR_UP, 1), Some(WheelChange::Gear(3)));
        // Already in top gear: no change.
        assert_eq!(w.feed(EV_KEY, BTN_GEAR_UP, 1), None);
        assert_eq!(w.gear(), 3);
    }

    #[test]
    fn gear_down_stops_at_neutral_and_ignores_releases_and_repeats() {
        let mut w = RacingWheel::new(3);
        w.feed(EV_KEY, BTN_GEAR_UP, 1);
        // A release (0) and a repeat (2) do not shift.
        assert_eq!(w.feed(EV_KEY, BTN_GEAR_UP, 0), None);
        assert_eq!(w.feed(EV_KEY, BTN_GEAR_UP, 2), None);
        assert_eq!(w.feed(EV_KEY, BTN_GEAR_DOWN, 1), Some(WheelChange::Gear(0)));
        // Neutral already: down does nothing.
        assert_eq!(w.feed(EV_KEY, BTN_GEAR_DOWN, 1), None);
        assert_eq!(w.gear(), 0);
    }

    #[test]
    fn unrelated_codes_are_ignored() {
        let mut w = RacingWheel::new(2);
        assert_eq!(w.feed(EV_ABS, 0x00, 500), None);
        assert_eq!(w.feed(EV_KEY, 0x130, 1), None);
    }
}
