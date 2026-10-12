//! Gamepad input model: the physical elements of a controller and the state they add up to, fed from raw
//! evdev `(type, code, value)` records. It follows the extended gamepad profile of the framework that apps use
//! for controllers: digital buttons, two sticks with clicks, two triggers and a direction pad.
//!
//! Pure: no device, no thread. Discovery of which input node is a controller lives in `bifrost_core::gamepad`,
//! and the event loop decides when to feed this model. The decoder in `springboard/src/bare/input.rs` still maps
//! ABS_X/ABS_Y as a touchscreen's pointer, so wiring this in is a separate step (see `docs/parity/game-controller.md`).
//!
//! Codes are the Linux gamepad codes (`BTN_SOUTH`..`BTN_THUMBR`, `ABS_X`..`ABS_RZ`, `ABS_HAT0X`/`ABS_HAT0Y`). Values
//! are normalised here: sticks to -1..=1 over a centred range, triggers to 0..=1, the direction pad to -1, 0 or 1 per
//! axis. The dead zone is cut out and the rest rescaled, so the output has no step at its edge.

/// `EV_KEY`: button records.
pub const EV_KEY: u16 = 0x01;
/// `EV_ABS`: absolute axis records.
pub const EV_ABS: u16 = 0x03;

const BTN_SOUTH: u16 = 0x130;
const BTN_EAST: u16 = 0x131;
const BTN_NORTH: u16 = 0x133;
const BTN_WEST: u16 = 0x134;
const BTN_TL: u16 = 0x136;
const BTN_TR: u16 = 0x137;
const BTN_TL2: u16 = 0x138;
const BTN_TR2: u16 = 0x139;
const BTN_SELECT: u16 = 0x13a;
const BTN_START: u16 = 0x13b;
const BTN_MODE: u16 = 0x13c;
const BTN_THUMBL: u16 = 0x13d;
const BTN_THUMBR: u16 = 0x13e;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_Z: u16 = 0x02;
const ABS_RX: u16 = 0x03;
const ABS_RY: u16 = 0x04;
const ABS_RZ: u16 = 0x05;
const ABS_HAT0X: u16 = 0x10;
const ABS_HAT0Y: u16 = 0x11;

/// A digital element: the face buttons, shoulders, trigger clicks, stick clicks and the menu buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Button {
    South = 0,
    East = 1,
    North = 2,
    West = 3,
    LeftShoulder = 4,
    RightShoulder = 5,
    LeftTrigger = 6,
    RightTrigger = 7,
    LeftStick = 8,
    RightStick = 9,
    Select = 10,
    Start = 11,
    Home = 12,
}

impl Button {
    pub const ALL: [Button; 13] = [
        Button::South,
        Button::East,
        Button::North,
        Button::West,
        Button::LeftShoulder,
        Button::RightShoulder,
        Button::LeftTrigger,
        Button::RightTrigger,
        Button::LeftStick,
        Button::RightStick,
        Button::Select,
        Button::Start,
        Button::Home,
    ];

    fn bit(self) -> u32 {
        1u32 << (self as u32)
    }
}

/// An analogue element. Sticks are centred; triggers rest at the bottom of their range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Axis {
    LeftX = 0,
    LeftY = 1,
    RightX = 2,
    RightY = 3,
    LeftTrigger = 4,
    RightTrigger = 5,
}

impl Axis {
    pub const ALL: [Axis; 6] = [Axis::LeftX, Axis::LeftY, Axis::RightX, Axis::RightY, Axis::LeftTrigger, Axis::RightTrigger];
}

/// What one raw record means on a gamepad. Codes that are not gamepad elements map to `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Element {
    Button(Button),
    Axis(Axis),
    DpadX,
    DpadY,
}

/// The element a raw record reports, or `None` for anything a gamepad does not use.
pub fn element(ty: u16, code: u16) -> Option<Element> {
    match (ty, code) {
        (EV_KEY, BTN_SOUTH) => Some(Element::Button(Button::South)),
        (EV_KEY, BTN_EAST) => Some(Element::Button(Button::East)),
        (EV_KEY, BTN_NORTH) => Some(Element::Button(Button::North)),
        (EV_KEY, BTN_WEST) => Some(Element::Button(Button::West)),
        (EV_KEY, BTN_TL) => Some(Element::Button(Button::LeftShoulder)),
        (EV_KEY, BTN_TR) => Some(Element::Button(Button::RightShoulder)),
        (EV_KEY, BTN_TL2) => Some(Element::Button(Button::LeftTrigger)),
        (EV_KEY, BTN_TR2) => Some(Element::Button(Button::RightTrigger)),
        (EV_KEY, BTN_THUMBL) => Some(Element::Button(Button::LeftStick)),
        (EV_KEY, BTN_THUMBR) => Some(Element::Button(Button::RightStick)),
        (EV_KEY, BTN_SELECT) => Some(Element::Button(Button::Select)),
        (EV_KEY, BTN_START) => Some(Element::Button(Button::Start)),
        (EV_KEY, BTN_MODE) => Some(Element::Button(Button::Home)),
        (EV_ABS, ABS_X) => Some(Element::Axis(Axis::LeftX)),
        (EV_ABS, ABS_Y) => Some(Element::Axis(Axis::LeftY)),
        (EV_ABS, ABS_RX) => Some(Element::Axis(Axis::RightX)),
        (EV_ABS, ABS_RY) => Some(Element::Axis(Axis::RightY)),
        (EV_ABS, ABS_Z) => Some(Element::Axis(Axis::LeftTrigger)),
        (EV_ABS, ABS_RZ) => Some(Element::Axis(Axis::RightTrigger)),
        (EV_ABS, ABS_HAT0X) => Some(Element::DpadX),
        (EV_ABS, ABS_HAT0Y) => Some(Element::DpadY),
        _ => None,
    }
}

/// The device range of an axis (the `EVIOCGABS` minimum and maximum) and its dead zone, as a fraction of the
/// travel. A centred axis maps `min` to -1, the centre to 0 and `max` to 1; a unipolar one maps `min` to 0 and
/// `max` to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisRange {
    pub min: i32,
    pub max: i32,
    pub centred: bool,
    pub deadzone: f32,
}

impl AxisRange {
    pub const fn stick(min: i32, max: i32, deadzone: f32) -> Self {
        AxisRange { min, max, centred: true, deadzone }
    }

    pub const fn trigger(min: i32, max: i32, deadzone: f32) -> Self {
        AxisRange { min, max, centred: false, deadzone }
    }

    /// The raw value as -1..=1 (centred) or 0..=1 (unipolar), with the dead zone removed.
    pub fn normalize(self, raw: i32) -> f32 {
        let span = i64::from(self.max) - i64::from(self.min);
        if span <= 0 {
            return 0.0;
        }
        let travel = (i64::from(raw) - i64::from(self.min)) as f32 / span as f32;
        let (low, v) = if self.centred { (-1.0, travel * 2.0 - 1.0) } else { (0.0, travel) };
        let v = v.clamp(low, 1.0);
        let dz = self.deadzone.clamp(0.0, 0.99);
        let mag = v.abs();
        if mag <= dz {
            0.0
        } else {
            v.signum() * (mag - dz) / (1.0 - dz)
        }
    }
}

/// A change to the state, reported once per edge or per moved value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Change {
    Button { button: Button, pressed: bool },
    Axis { axis: Axis, value: f32 },
    /// The direction pad, each component -1, 0 or 1 (x to the right, y down as the hat reports them).
    Dpad { x: i8, y: i8 },
}

/// The state of one controller: buttons as a bit set, axes as normalised values and the direction pad.
#[derive(Clone, Debug, PartialEq)]
pub struct Gamepad {
    buttons: u32,
    axes: [f32; 6],
    dpad: [i8; 2],
    ranges: [AxisRange; 6],
}

impl Default for Gamepad {
    fn default() -> Self {
        Self::new()
    }
}

impl Gamepad {
    /// The usual ranges of a standard pad: sticks signed 16-bit with a 10 % dead zone, triggers 0..=255.
    pub fn new() -> Self {
        let stick = AxisRange::stick(-32768, 32767, 0.1);
        let trigger = AxisRange::trigger(0, 255, 0.05);
        Gamepad {
            buttons: 0,
            axes: [0.0; 6],
            dpad: [0; 2],
            ranges: [stick, stick, stick, stick, trigger, trigger],
        }
    }

    /// Replaces the range of one axis, for a controller that reports another travel.
    pub fn set_range(&mut self, axis: Axis, range: AxisRange) {
        self.ranges[axis as usize] = range;
    }

    /// Feeds one raw record. Returns the change it caused, or `None` when the state did not move: key repeats
    /// (value 2), a button that is already in that state and an axis that reports the same normalised value.
    pub fn feed(&mut self, ty: u16, code: u16, value: i32) -> Option<Change> {
        match element(ty, code)? {
            Element::Button(button) => {
                if ty == EV_KEY && value == 2 {
                    return None;
                }
                let pressed = value != 0;
                if (self.buttons & button.bit() != 0) == pressed {
                    return None;
                }
                if pressed {
                    self.buttons |= button.bit();
                } else {
                    self.buttons &= !button.bit();
                }
                Some(Change::Button { button, pressed })
            }
            Element::Axis(axis) => {
                let i = axis as usize;
                let value = self.ranges[i].normalize(value);
                if value == self.axes[i] {
                    return None;
                }
                self.axes[i] = value;
                Some(Change::Axis { axis, value })
            }
            Element::DpadX => self.set_dpad(0, value),
            Element::DpadY => self.set_dpad(1, value),
        }
    }

    fn set_dpad(&mut self, slot: usize, value: i32) -> Option<Change> {
        let v = value.clamp(-1, 1) as i8;
        if self.dpad[slot] == v {
            return None;
        }
        self.dpad[slot] = v;
        Some(Change::Dpad { x: self.dpad[0], y: self.dpad[1] })
    }

    pub fn is_pressed(&self, button: Button) -> bool {
        self.buttons & button.bit() != 0
    }

    pub fn axis(&self, axis: Axis) -> f32 {
        self.axes[axis as usize]
    }

    /// The direction pad as (x, y), each -1, 0 or 1.
    pub fn dpad(&self) -> (i8, i8) {
        (self.dpad[0], self.dpad[1])
    }
}

use std::collections::VecDeque;

/// Most changes kept for polling. A reader that falls behind loses the oldest ones, not the newest.
pub const QUEUE_LIMIT: usize = 256;

/// The input stream an app reads from one controller: every change is handed to the callbacks as it is fed, and kept
/// in a queue for polling. Repeats and unchanged values reach neither. [`ControllerInput::capture`] copies the current
/// state, so an app can compare a snapshot with later state.
pub struct ControllerInput {
    pad: Gamepad,
    queue: VecDeque<Change>,
    listeners: Vec<Box<dyn FnMut(&Change)>>,
}

impl Default for ControllerInput {
    fn default() -> Self {
        Self::new()
    }
}

impl ControllerInput {
    pub fn new() -> Self {
        Self::with_pad(Gamepad::new())
    }

    /// Starts from an existing state, such as one with axis ranges already set.
    pub fn with_pad(pad: Gamepad) -> Self {
        ControllerInput { pad, queue: VecDeque::new(), listeners: Vec::new() }
    }

    /// Feeds one raw record. Runs the callbacks and queues the change, and returns it.
    pub fn feed(&mut self, ty: u16, code: u16, value: i32) -> Option<Change> {
        let change = self.pad.feed(ty, code, value)?;
        for listener in self.listeners.iter_mut() {
            listener(&change);
        }
        if self.queue.len() == QUEUE_LIMIT {
            self.queue.pop_front();
        }
        self.queue.push_back(change);
        Some(change)
    }

    /// The oldest change not yet polled.
    pub fn poll(&mut self) -> Option<Change> {
        self.queue.pop_front()
    }

    /// How many changes wait to be polled.
    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    /// Adds a callback that runs for every change, in the order the changes were fed.
    pub fn on_change<F: FnMut(&Change) + 'static>(&mut self, callback: F) {
        self.listeners.push(Box::new(callback));
    }

    /// A copy of the current state.
    pub fn capture(&self) -> Gamepad {
        self.pad.clone()
    }

    pub fn gamepad(&self) -> &Gamepad {
        &self.pad
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn element_maps_gamepad_codes_and_ignores_the_rest() {
        assert_eq!(element(EV_KEY, BTN_SOUTH), Some(Element::Button(Button::South)));
        assert_eq!(element(EV_KEY, BTN_START), Some(Element::Button(Button::Start)));
        assert_eq!(element(EV_KEY, BTN_MODE), Some(Element::Button(Button::Home)));
        assert_eq!(element(EV_ABS, ABS_X), Some(Element::Axis(Axis::LeftX)));
        assert_eq!(element(EV_ABS, ABS_RZ), Some(Element::Axis(Axis::RightTrigger)));
        assert_eq!(element(EV_ABS, ABS_HAT0Y), Some(Element::DpadY));
        // a left click (BTN_LEFT) is a pointer button, and the same code on another record type is not a button
        assert_eq!(element(EV_KEY, 0x110), None);
        assert_eq!(element(EV_KEY, ABS_X), None);
        assert_eq!(element(2, 0), None);
    }

    #[test]
    fn buttons_report_each_edge_once_and_ignore_repeats() {
        let mut pad = Gamepad::new();
        assert_eq!(pad.feed(EV_KEY, BTN_SOUTH, 1), Some(Change::Button { button: Button::South, pressed: true }));
        assert!(pad.is_pressed(Button::South));
        assert_eq!(pad.feed(EV_KEY, BTN_SOUTH, 1), None);
        assert_eq!(pad.feed(EV_KEY, BTN_SOUTH, 2), None);
        assert_eq!(pad.feed(EV_KEY, BTN_SOUTH, 0), Some(Change::Button { button: Button::South, pressed: false }));
        assert!(!pad.is_pressed(Button::South));
        assert!(!pad.is_pressed(Button::East));
    }

    #[test]
    fn stick_is_centred_and_rescaled_past_its_dead_zone() {
        let mut pad = Gamepad::new();
        pad.set_range(Axis::LeftX, AxisRange::stick(0, 200, 0.2));
        // the centre is rest, both ends are full deflection
        assert_eq!(pad.feed(EV_ABS, ABS_X, 0), Some(Change::Axis { axis: Axis::LeftX, value: -1.0 }));
        assert_eq!(pad.feed(EV_ABS, ABS_X, 200), Some(Change::Axis { axis: Axis::LeftX, value: 1.0 }));
        assert_eq!(pad.feed(EV_ABS, ABS_X, 100), Some(Change::Axis { axis: Axis::LeftX, value: 0.0 }));
        // v = 0.5 (raw 150) becomes (0.5 - 0.2) / 0.8 = 0.375
        pad.feed(EV_ABS, ABS_X, 150);
        assert!(close(pad.axis(Axis::LeftX), 0.375));
        // just past the dead zone the output is near 0, with no step at the edge: v = 0.21 gives 0.0125
        pad.feed(EV_ABS, ABS_X, 121);
        assert!(close(pad.axis(Axis::LeftX), 0.0125));
    }

    #[test]
    fn trigger_is_unipolar_and_zero_inside_its_dead_zone() {
        let mut pad = Gamepad::new();
        assert_eq!(pad.feed(EV_ABS, ABS_Z, 255), Some(Change::Axis { axis: Axis::LeftTrigger, value: 1.0 }));
        // 6 / 255 is under the 5 % dead zone
        pad.feed(EV_ABS, ABS_Z, 6);
        assert_eq!(pad.axis(Axis::LeftTrigger), 0.0);
        // 128 / 255 is about 0.50, rescaled over the 0.95 that lies past the dead zone
        pad.feed(EV_ABS, ABS_Z, 128);
        assert!(close(pad.axis(Axis::LeftTrigger), (128.0 / 255.0 - 0.05) / 0.95));
    }

    #[test]
    fn axis_reports_only_when_the_normalised_value_moves() {
        let mut pad = Gamepad::new();
        // raw moves inside the dead zone normalise to the same 0 as at rest, so they are not reported
        assert_eq!(pad.feed(EV_ABS, ABS_RX, 1000), None);
        assert_eq!(pad.feed(EV_ABS, ABS_RX, 1200), None);
        assert_eq!(pad.axis(Axis::RightX), 0.0);
        // past the dead zone the value moves and is reported, and a repeat of the same raw value is not
        assert!(pad.feed(EV_ABS, ABS_RX, 16000).is_some());
        assert_eq!(pad.feed(EV_ABS, ABS_RX, 16000), None);
    }

    #[test]
    fn dpad_reports_each_direction_change() {
        let mut pad = Gamepad::new();
        assert_eq!(pad.feed(EV_ABS, ABS_HAT0X, -1), Some(Change::Dpad { x: -1, y: 0 }));
        assert_eq!(pad.feed(EV_ABS, ABS_HAT0Y, 1), Some(Change::Dpad { x: -1, y: 1 }));
        assert_eq!(pad.feed(EV_ABS, ABS_HAT0X, -1), None);
        assert_eq!(pad.feed(EV_ABS, ABS_HAT0X, 0), Some(Change::Dpad { x: 0, y: 1 }));
        assert_eq!(pad.dpad(), (0, 1));
        // a hat value out of range is clamped, not stored raw
        assert_eq!(pad.feed(EV_ABS, ABS_HAT0Y, 5), None);
    }

    #[test]
    fn callbacks_run_once_per_change_in_feed_order() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let mut input = ControllerInput::new();
        let sink = Rc::clone(&seen);
        input.on_change(move |c| sink.borrow_mut().push(*c));
        input.feed(EV_KEY, BTN_SOUTH, 1);
        // a repeat and a second press of a held button change nothing and reach no callback
        input.feed(EV_KEY, BTN_SOUTH, 2);
        input.feed(EV_KEY, BTN_SOUTH, 1);
        input.feed(EV_KEY, BTN_SOUTH, 0);
        let got = seen.borrow().clone();
        assert_eq!(
            got,
            vec![
                Change::Button { button: Button::South, pressed: true },
                Change::Button { button: Button::South, pressed: false },
            ]
        );
    }

    #[test]
    fn polling_returns_queued_changes_in_order_then_nothing() {
        let mut input = ControllerInput::new();
        assert_eq!(input.feed(EV_ABS, ABS_HAT0X, -1), Some(Change::Dpad { x: -1, y: 0 }));
        assert_eq!(input.feed(EV_ABS, ABS_HAT0Y, 1), Some(Change::Dpad { x: -1, y: 1 }));
        assert_eq!(input.pending(), 2);
        assert_eq!(input.poll(), Some(Change::Dpad { x: -1, y: 0 }));
        assert_eq!(input.poll(), Some(Change::Dpad { x: -1, y: 1 }));
        assert_eq!(input.poll(), None);
        assert_eq!(input.pending(), 0);
    }

    #[test]
    fn values_inside_the_dead_zone_are_neither_queued_nor_called_back() {
        let calls = Rc::new(RefCell::new(0u32));
        let mut input = ControllerInput::new();
        let count = Rc::clone(&calls);
        input.on_change(move |_| *count.borrow_mut() += 1);
        // raw 1000 and 1200 normalise to the rest value 0 of the stick, so the state does not move
        assert_eq!(input.feed(EV_ABS, ABS_RX, 1000), None);
        assert_eq!(input.feed(EV_ABS, ABS_RX, 1200), None);
        assert_eq!(input.pending(), 0);
        assert_eq!(*calls.borrow(), 0);
        assert!(input.feed(EV_ABS, ABS_RX, 16000).is_some());
        assert_eq!(*calls.borrow(), 1);
    }

    #[test]
    fn capture_is_a_snapshot_that_later_changes_do_not_move() {
        let mut input = ControllerInput::new();
        input.feed(EV_KEY, BTN_SOUTH, 1);
        let snapshot = input.capture();
        input.feed(EV_KEY, BTN_SOUTH, 0);
        assert!(snapshot.is_pressed(Button::South));
        assert!(!input.gamepad().is_pressed(Button::South));
    }

    #[test]
    fn a_full_queue_drops_the_oldest_changes() {
        let mut input = ControllerInput::new();
        // 300 presses and releases alternate, so every record is a change; change i is a press when i is even
        for i in 0..300 {
            let value = if i % 2 == 0 { 1 } else { 0 };
            assert!(input.feed(EV_KEY, BTN_SOUTH, value).is_some());
        }
        assert_eq!(input.pending(), QUEUE_LIMIT);
        // the 44 oldest (changes 0 to 43) were dropped, so the first one left is change 44, a press
        assert_eq!(input.poll(), Some(Change::Button { button: Button::South, pressed: true }));
    }
}
