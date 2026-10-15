//! Parity sdk-rest-09: headless tests for keyboard and mouse as game input, and for gamepad state in web content. Pure
//! models fed from raw evdev records; no device, no window and no page is involved.

use bifrost_ui::game_input::{key_name, Change, Frame, KeyboardProfile, MouseButton, MouseProfile, EV_KEY, EV_REL};
use bifrost_ui::gamepad::Gamepad;
use bifrost_ui::web_gamepad::{snapshot, WebGamepadError, MAX_SLOTS, MAPPING, STANDARD_AXES, STANDARD_BUTTONS};

const KEY_A: u16 = 30;
const KEY_W: u16 = 17;
const KEY_SPACE: u16 = 57;
const BTN_LEFT: u16 = 0x110;
const BTN_RIGHT: u16 = 0x111;
const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;
const REL_WHEEL: u16 = 0x08;
const REL_HWHEEL: u16 = 0x06;
const EV_ABS: u16 = 0x03;
const ABS_X: u16 = 0x00;
const ABS_Z: u16 = 0x02;
const ABS_HAT0X: u16 = 0x10;
const ABS_HAT0Y: u16 = 0x11;
const BTN_SOUTH: u16 = 0x130;
const BTN_START: u16 = 0x13b;
const BTN_MODE: u16 = 0x13c;

#[test]
fn keyboard_reports_each_key_edge_once() {
    let mut keys = KeyboardProfile::new();
    assert_eq!(keys.feed(EV_KEY, KEY_W, 1), Some(Change::Key { code: KEY_W, pressed: true }));
    assert!(keys.is_pressed(KEY_W));
    assert_eq!(keys.feed(EV_KEY, KEY_W, 1), None, "a key already down reports no second press");
    assert_eq!(keys.feed(EV_KEY, KEY_W, 0), Some(Change::Key { code: KEY_W, pressed: false }));
    assert!(!keys.is_pressed(KEY_W));
}

#[test]
fn keyboard_ignores_repeats_unknown_values_and_other_record_types() {
    let mut keys = KeyboardProfile::new();
    assert_eq!(keys.feed(EV_KEY, KEY_A, 2), None, "value 2 is an auto-repeat, not a change");
    assert_eq!(keys.feed(EV_KEY, KEY_A, 7), None, "only 0 and 1 are key states");
    assert_eq!(keys.feed(EV_REL, REL_X, 4), None, "a relative record is not a key");
    assert_eq!(keys.feed(EV_KEY, 0x110, 1), None, "a code above the keyboard range is not a key");
    assert_eq!(keys.held().count(), 0);
}

#[test]
fn keyboard_release_all_frees_every_held_key() {
    let mut keys = KeyboardProfile::new();
    keys.feed(EV_KEY, KEY_SPACE, 1);
    keys.feed(EV_KEY, KEY_A, 1);
    let released = keys.release_all();
    assert_eq!(
        released,
        vec![Change::Key { code: KEY_A, pressed: false }, Change::Key { code: KEY_SPACE, pressed: false }],
        "one release per held key, in ascending code order"
    );
    assert_eq!(keys.held().count(), 0);
    assert!(keys.release_all().is_empty());
}

#[test]
fn key_names_cover_the_keys_games_bind() {
    assert_eq!(key_name(KEY_W), Some("w"));
    assert_eq!(key_name(KEY_SPACE), Some("space"));
    assert_eq!(key_name(103), Some("up"));
    assert_eq!(key_name(0xfe), None);
}

#[test]
fn mouse_buttons_map_their_codes_both_ways() {
    assert_eq!(MouseButton::from_code(BTN_LEFT), Some(MouseButton::Left));
    assert_eq!(MouseButton::Right.code(), BTN_RIGHT);
    assert_eq!(MouseButton::from_code(KEY_A), None, "a keyboard key is not a mouse button");
}

#[test]
fn mouse_button_edges_are_reported_once() {
    let mut mouse = MouseProfile::new();
    assert_eq!(mouse.feed(EV_KEY, BTN_LEFT, 1), Some(Change::Button { button: MouseButton::Left, pressed: true }));
    assert!(mouse.is_pressed(MouseButton::Left));
    assert_eq!(mouse.feed(EV_KEY, BTN_LEFT, 1), None);
    assert_eq!(mouse.feed(EV_KEY, BTN_LEFT, 0), Some(Change::Button { button: MouseButton::Left, pressed: false }));
}

#[test]
fn mouse_motion_sums_within_a_frame_and_take_frame_starts_the_next() {
    let mut mouse = MouseProfile::new();
    assert_eq!(mouse.feed(EV_REL, REL_X, 5), Some(Change::Motion { dx: 5, dy: 0 }));
    mouse.feed(EV_REL, REL_X, -2);
    mouse.feed(EV_REL, REL_Y, 7);
    assert_eq!(mouse.take_frame(), Frame { dx: 3, dy: 7, wheel_x: 0, wheel_y: 0 });
    assert_eq!(mouse.take_frame(), Frame::default(), "the next frame starts at zero");
}

#[test]
fn mouse_zero_motion_is_not_a_change() {
    let mut mouse = MouseProfile::new();
    assert_eq!(mouse.feed(EV_REL, REL_X, 0), None);
    assert_eq!(mouse.take_frame(), Frame::default());
}

#[test]
fn mouse_wheel_steps_report_the_axis_that_moved() {
    let mut mouse = MouseProfile::new();
    assert_eq!(mouse.feed(EV_REL, REL_WHEEL, 1), Some(Change::Wheel { x: 0, y: 1 }));
    assert_eq!(mouse.feed(EV_REL, REL_HWHEEL, -2), Some(Change::Wheel { x: -2, y: 0 }));
    let frame = mouse.take_frame();
    assert_eq!((frame.wheel_x, frame.wheel_y), (-2, 1));
}

#[test]
fn mouse_release_all_frees_every_held_button() {
    let mut mouse = MouseProfile::new();
    mouse.feed(EV_KEY, BTN_RIGHT, 1);
    mouse.feed(EV_KEY, BTN_LEFT, 1);
    let released = mouse.release_all();
    assert_eq!(
        released,
        vec![
            Change::Button { button: MouseButton::Left, pressed: false },
            Change::Button { button: MouseButton::Right, pressed: false },
        ]
    );
    assert!(!mouse.is_pressed(MouseButton::Right));
}

fn pad_with(records: &[(u16, u16, i32)]) -> Gamepad {
    let mut pad = Gamepad::new();
    for &(ty, code, value) in records {
        pad.feed(ty, code, value);
    }
    pad
}

#[test]
fn web_snapshot_has_the_standard_shape_and_mapping_name() {
    let pad = Gamepad::new();
    let snap = snapshot(0, "Controller", &pad).unwrap();
    assert_eq!(snap.mapping, MAPPING);
    assert_eq!(snap.mapping, "standard");
    assert_eq!(snap.buttons.len(), STANDARD_BUTTONS);
    assert_eq!(snap.axes.len(), STANDARD_AXES);
    assert_eq!(snap.index, 0);
    assert_eq!(snap.id, "Controller");
}

#[test]
fn web_snapshot_puts_the_face_and_menu_buttons_at_their_standard_indices() {
    let pad = pad_with(&[(EV_KEY, BTN_SOUTH, 1), (EV_KEY, BTN_START, 1), (EV_KEY, BTN_MODE, 1)]);
    let snap = snapshot(1, "pad", &pad).unwrap();
    assert!(snap.buttons[0].pressed, "south is standard button 0");
    assert_eq!(snap.buttons[0].value, 1.0);
    assert!(snap.buttons[9].pressed, "start is standard button 9");
    assert!(snap.buttons[16].pressed, "home is standard button 16");
    assert!(!snap.buttons[1].pressed, "a button that is up stays unpressed");
}

#[test]
fn web_snapshot_maps_the_sticks_to_the_first_four_axes() {
    let pad = pad_with(&[(EV_ABS, ABS_X, 32767)]);
    let snap = snapshot(0, "pad", &pad).unwrap();
    assert!(snap.axes[0] > 0.99, "left X at full deflection reads near 1");
    assert_eq!(snap.axes[1], 0.0, "a centred left Y reads 0");
    assert_eq!(snap.axes[2], 0.0);
    assert_eq!(snap.axes[3], 0.0);
}

#[test]
fn web_snapshot_reports_triggers_as_button_values() {
    let light = pad_with(&[(EV_ABS, ABS_Z, 20)]);
    let snap = snapshot(0, "pad", &light).unwrap();
    assert!(!snap.buttons[6].pressed, "a light pull is under the press threshold");
    assert!(snap.buttons[6].value > 0.0 && snap.buttons[6].value < 0.5);

    let hard = pad_with(&[(EV_ABS, ABS_Z, 200)]);
    let snap = snapshot(0, "pad", &hard).unwrap();
    assert!(snap.buttons[6].pressed, "a pull past the threshold is pressed");
    assert!(snap.buttons[6].value >= 0.5);
}

#[test]
fn web_snapshot_puts_the_direction_pad_on_buttons_twelve_to_fifteen() {
    let pad = pad_with(&[(EV_ABS, ABS_HAT0X, -1), (EV_ABS, ABS_HAT0Y, -1)]);
    let snap = snapshot(0, "pad", &pad).unwrap();
    assert!(snap.buttons[12].pressed, "up is standard button 12");
    assert!(snap.buttons[14].pressed, "left is standard button 14");
    assert!(!snap.buttons[13].pressed, "down is not pressed while up is held");
    assert!(!snap.buttons[15].pressed, "right is not pressed while left is held");
}

#[test]
fn web_snapshot_refuses_a_slot_the_page_cannot_see() {
    let pad = Gamepad::new();
    assert_eq!(
        snapshot(MAX_SLOTS, "pad", &pad),
        Err(WebGamepadError::SlotOutOfRange { index: MAX_SLOTS, max: MAX_SLOTS })
    );
    assert_eq!(
        WebGamepadError::SlotOutOfRange { index: 4, max: 4 }.to_string(),
        "gamepad slot 4 is out of range 0 to 3"
    );
}

#[test]
fn web_snapshot_refuses_an_empty_name() {
    let pad = Gamepad::new();
    assert_eq!(snapshot(0, "", &pad), Err(WebGamepadError::EmptyId));
    assert_eq!(WebGamepadError::EmptyId.to_string(), "gamepad id is empty");
}
