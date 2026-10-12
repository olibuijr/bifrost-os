//! Device matching by capability: a filter over the event types and codes a device reports, so a caller can
//! ask for "a keyboard", "a pointer" or "a touch surface" without naming a device. This is the pure model.
//! The system layer reads each device's capability bitmaps and passes them here; no device is opened.
//! Matching is by event type and code, not by HID usage page.

use std::collections::BTreeSet;

/// Event types, numbered as the kernel numbers them.
pub const EV_KEY: u16 = 0x01;
pub const EV_REL: u16 = 0x02;
pub const EV_ABS: u16 = 0x03;
pub const EV_SW: u16 = 0x05;

/// Key and button codes used by the presets.
pub const KEY_ENTER: u16 = 28;
pub const KEY_A: u16 = 30;
pub const BTN_LEFT: u16 = 0x110;

/// Absolute axis codes used by the touch preset.
pub const ABS_MT_POSITION_X: u16 = 0x35;
pub const ABS_MT_POSITION_Y: u16 = 0x36;

fn type_bit(ev: u16) -> u32 {
    if ev < 32 {
        1u32 << ev
    } else {
        0
    }
}

/// The capabilities one device reports: its event types, key codes and absolute axes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    types: u32,
    keys: BTreeSet<u16>,
    axes: BTreeSet<u16>,
}

impl Capabilities {
    pub fn new() -> Capabilities {
        Capabilities::default()
    }

    pub fn with_type(mut self, ev: u16) -> Capabilities {
        self.types |= type_bit(ev);
        self
    }

    /// A key code also declares the key event type.
    pub fn with_key(mut self, code: u16) -> Capabilities {
        self.types |= type_bit(EV_KEY);
        self.keys.insert(code);
        self
    }

    /// An absolute axis also declares the absolute event type.
    pub fn with_axis(mut self, code: u16) -> Capabilities {
        self.types |= type_bit(EV_ABS);
        self.axes.insert(code);
        self
    }

    pub fn has_type(&self, ev: u16) -> bool {
        self.types & type_bit(ev) != 0
    }

    pub fn has_key(&self, code: u16) -> bool {
        self.keys.contains(&code)
    }

    pub fn has_axis(&self, code: u16) -> bool {
        self.axes.contains(&code)
    }
}

/// What a caller asks for: every required type, key and axis must be present, and no excluded type may be.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Criteria {
    types: u32,
    excluded_types: u32,
    keys: BTreeSet<u16>,
    axes: BTreeSet<u16>,
}

impl Criteria {
    /// A device that is a keyboard: it reports the Enter and A keys.
    pub fn keyboard() -> Criteria {
        Criteria::default().require_key(KEY_ENTER).require_key(KEY_A)
    }

    /// A device that is a pointer: it moves relative and has a left button.
    pub fn pointer() -> Criteria {
        Criteria::default().require_type(EV_REL).require_key(BTN_LEFT)
    }

    /// A device that is a touch surface: it reports both multitouch position axes.
    pub fn touch() -> Criteria {
        Criteria::default().require_axis(ABS_MT_POSITION_X).require_axis(ABS_MT_POSITION_Y)
    }

    pub fn require_type(mut self, ev: u16) -> Criteria {
        self.types |= type_bit(ev);
        self
    }

    pub fn exclude_type(mut self, ev: u16) -> Criteria {
        self.excluded_types |= type_bit(ev);
        self
    }

    pub fn require_key(mut self, code: u16) -> Criteria {
        self.keys.insert(code);
        self
    }

    pub fn require_axis(mut self, code: u16) -> Criteria {
        self.axes.insert(code);
        self
    }

    /// Whether one device's capabilities satisfy the criteria.
    pub fn matches(&self, caps: &Capabilities) -> bool {
        caps.types & self.types == self.types
            && caps.types & self.excluded_types == 0
            && self.keys.is_subset(&caps.keys)
            && self.axes.is_subset(&caps.axes)
    }

    /// The positions in `devices` that match, in order.
    pub fn select(&self, devices: &[Capabilities]) -> Vec<usize> {
        devices.iter().enumerate().filter(|(_, c)| self.matches(c)).map(|(i, _)| i).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyboard_device() -> Capabilities {
        Capabilities::new().with_key(KEY_ENTER).with_key(KEY_A)
    }

    #[test]
    fn keyboard_needs_enter_and_a() {
        assert!(Criteria::keyboard().matches(&keyboard_device()));
        let only_a = Capabilities::new().with_key(KEY_A);
        assert!(!Criteria::keyboard().matches(&only_a));
    }

    #[test]
    fn pointer_needs_relative_motion_and_a_left_button() {
        let mouse = Capabilities::new().with_type(EV_REL).with_key(BTN_LEFT);
        assert!(Criteria::pointer().matches(&mouse));
        // Relative motion without a button is not a pointer by this preset.
        let wheel = Capabilities::new().with_type(EV_REL);
        assert!(!Criteria::pointer().matches(&wheel));
    }

    #[test]
    fn touch_needs_both_multitouch_position_axes() {
        let panel = Capabilities::new().with_axis(ABS_MT_POSITION_X).with_axis(ABS_MT_POSITION_Y);
        assert!(Criteria::touch().matches(&panel));
        let one_axis = Capabilities::new().with_axis(ABS_MT_POSITION_X);
        assert!(!Criteria::touch().matches(&one_axis));
    }

    #[test]
    fn excluded_type_rejects_a_device_that_reports_it() {
        let criteria = Criteria::default().exclude_type(EV_ABS);
        let panel = Capabilities::new().with_axis(0);
        assert!(!criteria.matches(&panel));
        assert!(criteria.matches(&keyboard_device()));
    }

    #[test]
    fn select_keeps_the_matching_positions_in_order() {
        let devices = vec![
            keyboard_device(),
            Capabilities::new().with_axis(ABS_MT_POSITION_X).with_axis(ABS_MT_POSITION_Y),
            Capabilities::new().with_key(KEY_ENTER).with_key(KEY_A).with_type(EV_SW),
        ];
        assert_eq!(Criteria::keyboard().select(&devices), vec![0, 2]);
        assert_eq!(Criteria::touch().select(&devices), vec![1]);
    }

    #[test]
    fn a_key_declares_the_key_type_and_out_of_range_types_are_ignored() {
        let caps = Capabilities::new().with_key(KEY_A);
        assert!(caps.has_type(EV_KEY));
        assert!(!caps.has_type(EV_REL));
        // Types above 31 have no bit, so they never match.
        assert!(!Capabilities::new().with_type(40).has_type(40));
        // An empty criteria matches any device, even one with nothing.
        assert!(Criteria::default().matches(&Capabilities::new()));
    }
}
