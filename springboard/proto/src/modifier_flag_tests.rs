use super::*;

#[test]
fn modifier_flags_use_the_reference_raw_values() {
    assert_eq!(ModifierFlags::CAPS_LOCK.bits(), 1 << 16);
    assert_eq!(ModifierFlags::SHIFT.bits(), 1 << 17);
    assert_eq!(ModifierFlags::CONTROL.bits(), 1 << 18);
    assert_eq!(ModifierFlags::OPTION.bits(), 1 << 19);
    assert_eq!(ModifierFlags::COMMAND.bits(), 1 << 20);
    assert_eq!(ModifierFlags::from_bits_truncate(u32::MAX), ModifierFlags::ALL, "unknown bits are dropped");
    let both = ModifierFlags::SHIFT | ModifierFlags::COMMAND;
    assert!(both.shift() && both.command() && !both.option() && !both.control() && !both.caps_lock());
    assert!(ModifierFlags::default().is_empty());
}

#[test]
fn chord_bits_and_held_keys_map_to_the_same_flags() {
    assert_eq!(ModifierFlags::from_chord(CHORD_SHIFT | CHORD_META), ModifierFlags::SHIFT | ModifierFlags::COMMAND);
    assert_eq!(ModifierFlags::from_chord(CHORD_CTRL | CHORD_ALT), ModifierFlags::CONTROL | ModifierFlags::OPTION);
    assert_eq!(ModifierFlags::from_held(true, false, false, true, false), ModifierFlags::SHIFT | ModifierFlags::COMMAND);
    assert_eq!(ModifierFlags::from_held(false, false, false, false, true), ModifierFlags::CAPS_LOCK);
}

#[test]
fn a_touch_carries_its_modifiers_and_one_without_them_decodes_as_none_held() {
    let m = ToApp::Touch { kind: TouchKind::Down, id: 0, x: 4.0, y: 5.0, t_ms: 9, source: TouchSource::Pointer, mods: ModifierFlags::SHIFT };
    assert_eq!(ToApp::decode(&m.encode()), Ok(m.clone()));
    // a sender built before the flags stops after `t_ms`
    let mut old = m.encode();
    old.truncate(old.len() - 4);
    let Ok(ToApp::Touch { mods, t_ms, .. }) = ToApp::decode(&old) else { panic!() };
    assert_eq!((mods, t_ms), (ModifierFlags::EMPTY, 9));
    let w = ToApp::WindowTouch { id: 2, touch: 1, kind: TouchKind::Up, x: 1.0, y: 1.0, t_ms: 3, source: TouchSource::Finger, mods: ModifierFlags::COMMAND };
    assert_eq!(ToApp::decode(&w.encode()), Ok(w.clone()));
    let mut old = w.encode();
    old.truncate(old.len() - 4);
    let Ok(ToApp::WindowTouch { mods, .. }) = ToApp::decode(&old) else { panic!() };
    assert!(mods.is_empty());
}
