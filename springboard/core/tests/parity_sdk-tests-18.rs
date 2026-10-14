//! Parity sdk-tests-18: headless tests of controller discovery through the public API of `bifrost_core::gamepad`.
//! Capability bitmaps stand in for an input node, so no device is opened and no thread is started.

use bifrost_core::gamepad::{has_bit, supports_rumble, Capabilities, Controller, Event, Registry};

const KEY_A: u16 = 30;
const BTN_LEFT: u16 = 0x110;
const BTN_SOUTH: u16 = 0x130;
const BTN_START: u16 = 0x13b;
const BTN_TOUCH: u16 = 0x14a;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const FF_RUMBLE: u16 = 0x50;
const FF_PERIODIC: u16 = 0x51;

/// A bitmap sized like a `KEY_MAX` capability read, with the given codes set.
fn bitmap(codes: &[u16]) -> Vec<u8> {
    let mut bits = vec![0u8; 96];
    for &code in codes {
        bits[usize::from(code / 8)] |= 1u8 << (code % 8);
    }
    bits
}

/// A game controller: a face button, a menu button and both left-stick axes.
fn pad() -> Capabilities {
    Capabilities { keys: bitmap(&[BTN_SOUTH, BTN_START]), abs: bitmap(&[ABS_X, ABS_Y]) }
}

#[test]
fn a_face_button_with_both_stick_axes_is_a_controller() {
    let caps = pad();
    assert!(caps.is_game_controller());
    assert!(caps.has_key(BTN_SOUTH));
    assert!(caps.has_abs(ABS_Y));
}

#[test]
fn a_keyboard_and_a_mouse_are_not_controllers() {
    let keyboard = Capabilities { keys: bitmap(&[KEY_A]), abs: Vec::new() };
    assert!(!keyboard.is_game_controller(), "keys without axes is a keyboard");
    let mouse = Capabilities { keys: bitmap(&[BTN_LEFT]), abs: Vec::new() };
    assert!(!mouse.is_game_controller(), "a pointer button excludes the node");
}

#[test]
fn a_touch_panel_with_axes_and_no_face_button_is_not_a_controller() {
    let touch = Capabilities { keys: bitmap(&[BTN_TOUCH]), abs: bitmap(&[ABS_X, ABS_Y]) };
    assert!(!touch.is_game_controller());
}

#[test]
fn a_pad_that_also_reports_a_pointer_button_is_not_a_controller() {
    let odd = Capabilities { keys: bitmap(&[BTN_SOUTH, BTN_LEFT]), abs: bitmap(&[ABS_X, ABS_Y]) };
    assert!(!odd.is_game_controller());
}

#[test]
fn bitmap_bits_are_read_least_significant_first_and_out_of_range_is_clear() {
    let bits = bitmap(&[BTN_SOUTH]);
    assert!(has_bit(&bits, BTN_SOUTH));
    assert!(!has_bit(&bits, BTN_SOUTH + 1));
    assert!(!has_bit(&[], 0), "an empty bitmap sets nothing");
    assert!(!has_bit(&bits, 0x2ff * 2), "a code past the end is clear");
}

#[test]
fn connecting_a_pad_reports_one_event_with_a_trimmed_name() {
    let mut registry = Registry::default();
    let event = registry.connected("/dev/input/event5", "  Pad A  ", &pad());
    assert_eq!(
        event,
        Some(Event::Connected(Controller { id: 1, path: "/dev/input/event5".into(), name: "Pad A".into() }))
    );
    assert_eq!(registry.controllers().len(), 1);
}

#[test]
fn a_node_reported_twice_is_still_one_controller() {
    let mut registry = Registry::default();
    registry.connected("/dev/input/event5", "Pad A", &pad());
    assert_eq!(registry.connected("/dev/input/event5", "Pad A", &pad()), None);
    assert_eq!(registry.controllers().len(), 1);
}

#[test]
fn a_node_that_is_not_a_controller_takes_no_id() {
    let mut registry = Registry::default();
    let keyboard = Capabilities { keys: bitmap(&[KEY_A]), abs: Vec::new() };
    assert_eq!(registry.connected("/dev/input/event1", "Keyboard", &keyboard), None);
    assert!(registry.controllers().is_empty());
    match registry.connected("/dev/input/event2", "Pad", &pad()) {
        Some(Event::Connected(c)) => assert_eq!(c.id, 1, "the first real controller is still id 1"),
        other => panic!("expected a controller, got {other:?}"),
    }
}

#[test]
fn unplugging_reports_the_removed_controller_once() {
    let mut registry = Registry::default();
    registry.connected("/dev/input/event5", "Pad", &pad());
    match registry.disconnected("/dev/input/event5") {
        Some(Event::Disconnected(c)) => assert_eq!((c.id, c.path.as_str()), (1, "/dev/input/event5")),
        other => panic!("expected a disconnect, got {other:?}"),
    }
    assert!(registry.controllers().is_empty());
    assert_eq!(registry.disconnected("/dev/input/event5"), None, "a second unplug reports nothing");
}

#[test]
fn a_replugged_pad_gets_a_new_id_and_ids_are_never_reused() {
    let mut registry = Registry::default();
    registry.connected("/dev/input/event5", "Pad", &pad());
    registry.disconnected("/dev/input/event5");
    match registry.connected("/dev/input/event5", "Pad", &pad()) {
        Some(Event::Connected(c)) => assert_eq!(c.id, 2),
        other => panic!("expected a reconnect, got {other:?}"),
    }
}

#[test]
fn controllers_keep_arrival_order_and_can_be_looked_up_by_id() {
    let mut registry = Registry::default();
    registry.connected("/dev/input/event5", "Pad A", &pad());
    registry.connected("/dev/input/event6", "Pad B", &pad());
    let ids: Vec<u32> = registry.controllers().iter().map(|c| c.id).collect();
    assert_eq!(ids, vec![1, 2]);
    assert_eq!(registry.get(2).map(|c| c.name.as_str()), Some("Pad B"));
    assert_eq!(registry.get(9), None);
    registry.disconnected("/dev/input/event5");
    assert_eq!(registry.get(2).map(|c| c.path.as_str()), Some("/dev/input/event6"));
    assert_eq!(registry.get(1), None, "a removed controller is no longer found");
}

#[test]
fn only_a_rumble_effect_in_the_force_feedback_bitmap_marks_a_motor_pad() {
    assert!(supports_rumble(&bitmap(&[FF_RUMBLE])));
    assert!(!supports_rumble(&bitmap(&[FF_PERIODIC])), "a periodic effect alone is not rumble");
    assert!(!supports_rumble(&[]), "an empty bitmap reports no effects");
}
