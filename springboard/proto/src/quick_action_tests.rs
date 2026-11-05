use super::*;

#[test]
fn a_chosen_quick_action_round_trips_with_its_user_info() {
    let message = ToApp::QuickAction { kind: "compose".into(), user_info: vec![("account".into(), "work".into())] };
    assert_eq!(ToApp::decode(&message.encode()), Ok(message));
    let bare = ToApp::QuickAction { kind: "search".into(), user_info: Vec::new() };
    assert_eq!(ToApp::decode(&bare.encode()), Ok(bare));
}

#[test]
fn a_quick_action_with_too_much_user_info_is_refused() {
    let many: Vec<(String, String)> = (0..=MAX_QUICK_INFO).map(|n| (format!("k{n}"), "v".into())).collect();
    let bytes = ToApp::QuickAction { kind: "x".into(), user_info: many }.encode();
    assert_eq!(ToApp::decode(&bytes), Err(DecodeError::BadValue));
}

#[test]
fn window_messages_round_trip_through_the_codec() {
    let shell = [
        ToShell::WindowOpen { id: 3, title: "Ada Lovelace".into(), w: 480.0, h: 560.0, position: None, flags: 0 },
        ToShell::WindowClose { id: 3 },
        ToShell::WindowTitle { id: 3, title: "Grace".into() },
        ToShell::WindowResize { id: 3, w: 320.0, h: 240.0 },
        ToShell::WindowBuffers { id: 3, w: 4, h: 3, generation: 2 },
        ToShell::WindowFrame { id: 3, buf: 1, damage: vec![[0, 0, 4, 3]] },
        ToShell::WindowFrame { id: 3, buf: 0, damage: vec![] },
    ];
    for m in shell {
        assert_eq!(ToShell::decode(&m.encode()), Ok(m.clone()), "{m:?}");
    }
    let app = [
        ToApp::WindowClosed { id: 3 },
        ToApp::FullScreen { on: true },
        ToApp::WindowConfigure { id: 3, w: 320.0, h: 240.0 },
        ToApp::WindowTouch { id: 3, touch: 1, kind: TouchKind::Move, x: 10.5, y: -2.0, t_ms: 99, source: TouchSource::Pointer, mods: ModifierFlags::OPTION },
        ToApp::WindowRelease { id: 3, buf: 1, generation: 5 },
    ];
    for m in app {
        assert_eq!(ToApp::decode(&m.encode()), Ok(m.clone()), "{m:?}");
    }
}

#[test]
fn key_window_messages_round_trip_through_the_codec() {
    let app = [
        ToApp::WindowFocus { window: 3, focused: true },
        ToApp::WindowFocus { window: 0, focused: false },
        ToApp::WindowInput { window: 3, input: WindowInput::Key(KeyInput::Text("é😀".into())) },
        ToApp::WindowInput { window: 3, input: WindowInput::Key(KeyInput::Code(30)) },
        ToApp::WindowInput { window: 3, input: WindowInput::Command(Command::SelectAll) },
        ToApp::WindowInput { window: 3, input: WindowInput::MenuAction(7) },
        ToApp::WindowInput { window: 3, input: WindowInput::SetMarkedText { text: "にほん".into(), sel_start: 3, sel_len: 3 } },
        ToApp::WindowInput { window: 3, input: WindowInput::UnmarkText { commit: true } },
        ToApp::WindowInput { window: 3, input: WindowInput::DeleteSurrounding { before: 2, after: 0 } },
        ToApp::WindowInput { window: 3, input: WindowInput::AccentHold { letter: 'a' } },
        ToApp::WindowInput {
            window: 3,
            input: WindowInput::Pointer(bifrost_ui::pointer_event::PointerEvent::Wheel {
                x: 12.5,
                y: 40.0,
                dx: 0.0,
                dy: -19.2,
                precise: false,
                phase: bifrost_ui::wheel::Phase::None,
                momentum: bifrost_ui::wheel::Phase::None,
            }),
        },
        ToApp::WindowInput { window: 3, input: WindowInput::Pointer(bifrost_ui::pointer_event::PointerEvent::Hover { x: 1.0, y: 2.0 }) },
        ToApp::WindowInput { window: 3, input: WindowInput::Pointer(bifrost_ui::pointer_event::PointerEvent::HoverExit) },
    ];
    for m in app {
        assert_eq!(ToApp::decode(&m.encode()), Ok(m.clone()), "{m:?}");
    }
}

#[test]
fn window_input_has_the_documented_layout() {
    // tag 0x250, window 3, kind 0 (key), key kind 1 (code), code 30
    let bytes = ToApp::WindowInput { window: 3, input: WindowInput::Key(KeyInput::Code(30)) }.encode();
    assert_eq!(&bytes[..4], &0x250u32.to_le_bytes());
    assert_eq!(&bytes[4..8], &3u32.to_le_bytes());
    assert_eq!(&bytes[8..10], &[0, 1]);
    assert_eq!(&bytes[10..14], &30u32.to_le_bytes());
    // tag 0x24f, window 3, focused
    let focus = ToApp::WindowFocus { window: 3, focused: true }.encode();
    assert_eq!(&focus[..4], &0x24fu32.to_le_bytes());
    assert_eq!(&focus[4..9], &[3, 0, 0, 0, 1]);
}

#[test]
fn window_input_for_the_main_window_uses_the_legacy_messages() {
    assert_eq!(WindowInput::Key(KeyInput::Code(9)).to_app(0), ToApp::Key(KeyInput::Code(9)));
    assert_eq!(WindowInput::Command(Command::Copy).to_app(0), ToApp::Command(Command::Copy));
    assert_eq!(WindowInput::MenuAction(4).to_app(0), ToApp::MenuAction(4));
    assert_eq!(WindowInput::UnmarkText { commit: false }.to_app(0), ToApp::UnmarkText { commit: false });
    assert_eq!(WindowInput::MenuAction(4).to_app(2), ToApp::WindowInput { window: 2, input: WindowInput::MenuAction(4) });
    let hover = bifrost_ui::pointer_event::PointerEvent::Hover { x: 3.0, y: 4.0 };
    assert_eq!(WindowInput::Pointer(hover).to_app(0), ToApp::Pointer(hover));
    assert_eq!(WindowInput::Pointer(hover).to_app(5), ToApp::WindowInput { window: 5, input: WindowInput::Pointer(hover) });
}

#[test]
fn an_accent_hold_goes_to_the_key_window_and_a_bad_code_point_is_refused() {
    assert_eq!(WindowInput::AccentHold { letter: 'e' }.to_app(0), ToApp::AccentHold { letter: 'e' });
    assert_eq!(WindowInput::AccentHold { letter: 'e' }.to_app(2), ToApp::WindowInput { window: 2, input: WindowInput::AccentHold { letter: 'e' } });
    let mut bytes = ToApp::WindowInput { window: 2, input: WindowInput::AccentHold { letter: 'e' } }.encode();
    // kind 7 at byte 8, the code point at 9..13
    assert_eq!(bytes[8], 7);
    assert_eq!(&bytes[9..13], &('e' as u32).to_le_bytes());
    bytes[9..13].copy_from_slice(&0xD800u32.to_le_bytes());
    assert_eq!(ToApp::decode(&bytes), Err(DecodeError::BadValue), "a surrogate is not a letter");
}

#[test]
fn a_window_pointer_event_has_the_pointer_layout_after_kind_six() {
    use bifrost_ui::pointer_event::PointerEvent;
    let ev = PointerEvent::Hover { x: 1.5, y: 2.5 };
    let bytes = ToApp::WindowInput { window: 4, input: WindowInput::Pointer(ev) }.encode();
    assert_eq!(&bytes[..4], &0x250u32.to_le_bytes());
    assert_eq!(&bytes[4..8], &4u32.to_le_bytes());
    // kind 6, then the `ToApp::Pointer` bytes without their tag: kind 1 (hover) and the two floats
    let main = ToApp::Pointer(ev).encode();
    assert_eq!(bytes[8], 6);
    assert_eq!(&bytes[9..], &main[4..]);
    // a NaN location is refused here as it is for the main window
    let mut bad = bytes.clone();
    bad[10..14].copy_from_slice(&f32::NAN.to_le_bytes());
    assert_eq!(ToApp::decode(&bad), Err(DecodeError::BadValue));
}

#[test]
fn a_bad_window_focus_or_window_input_is_refused() {
    let mut focus = ToApp::WindowFocus { window: 1, focused: true }.encode();
    focus[8] = 2;
    assert_eq!(ToApp::decode(&focus), Err(DecodeError::BadValue));
    // unknown input kind
    let mut kind = ToApp::WindowInput { window: 1, input: WindowInput::MenuAction(1) }.encode();
    kind[8] = 9;
    assert_eq!(ToApp::decode(&kind), Err(DecodeError::BadValue));
    // unknown key kind inside a key
    let mut key = ToApp::WindowInput { window: 1, input: WindowInput::Key(KeyInput::Code(1)) }.encode();
    key[9] = 7;
    assert_eq!(ToApp::decode(&key), Err(DecodeError::BadValue));
    // a command number the table does not know
    let mut cmd = ToApp::WindowInput { window: 1, input: WindowInput::Command(Command::Copy) }.encode();
    cmd[9..13].copy_from_slice(&99u32.to_le_bytes());
    assert_eq!(ToApp::decode(&cmd), Err(DecodeError::BadValue));
    // a marked-text selection that ends inside a character
    let mut marked = ToApp::WindowInput { window: 1, input: WindowInput::SetMarkedText { text: "é".into(), sel_start: 0, sel_len: 2 } }.encode();
    // kind 3 at byte 8, the text length at 9..13, "é" at 13..15, then sel_start and sel_len
    marked[15..19].copy_from_slice(&0u32.to_le_bytes());
    marked[19..23].copy_from_slice(&1u32.to_le_bytes());
    assert_eq!(ToApp::decode(&marked), Err(DecodeError::BadValue));
}

#[test]
fn window_open_has_the_documented_layout() {
    let bytes = ToShell::WindowOpen { id: 2, title: "ab".into(), w: 1.0, h: 2.0, position: None, flags: 0 }.encode();
    // tag 0x240, id 2, title length 2 and "ab", then the two floats
    assert_eq!(&bytes[..4], &0x240u32.to_le_bytes());
    assert_eq!(&bytes[4..8], &2u32.to_le_bytes());
    assert_eq!(&bytes[8..12], &2u32.to_le_bytes());
    assert_eq!(&bytes[12..14], b"ab");
}

#[test]
fn window_placement_and_collapse_round_trip() {
    let shell = [
        ToShell::WindowOpen { id: 4, title: "Note".into(), w: 200.0, h: 150.0, position: Some((12.5, 40.0)), flags: window_flags::TITLE_DOUBLE_CLICK },
        ToShell::WindowOpen { id: 5, title: "Plain".into(), w: 1.0, h: 1.0, position: None, flags: 0 },
        ToShell::WindowMove { window: 4, x: -3.0, y: 88.0 },
        ToShell::WindowCollapse { window: 4, collapsed: true },
        ToShell::WindowCollapse { window: 4, collapsed: false },
        ToShell::WindowRaise { window: 0 },
        ToShell::WindowRaise { window: 4 },
    ];
    for m in shell {
        assert_eq!(ToShell::decode(&m.encode()), Ok(m.clone()), "{m:?}");
    }
    let app = [ToApp::WindowMoved { window: 4, x: 10.0, y: 20.0 }, ToApp::WindowTitleDoubleClick { window: 4 }];
    for m in app {
        assert_eq!(ToApp::decode(&m.encode()), Ok(m.clone()), "{m:?}");
    }
}

#[test]
fn window_placement_refuses_a_bad_flag_or_a_non_finite_position() {
    let mut bad = ToShell::WindowCollapse { window: 4, collapsed: true }.encode();
    let n = bad.len();
    bad[n - 1] = 2;
    assert_eq!(ToShell::decode(&bad), Err(DecodeError::BadValue));
    let nan = ToShell::WindowMove { window: 4, x: f32::NAN, y: 0.0 }.encode();
    assert_eq!(ToShell::decode(&nan), Err(DecodeError::BadValue));
    let open = ToShell::WindowOpen { id: 4, title: "a".into(), w: 1.0, h: 1.0, position: Some((f32::INFINITY, 0.0)), flags: 0 }.encode();
    assert_eq!(ToShell::decode(&open), Err(DecodeError::BadValue));
}

#[test]
fn an_overlong_or_non_finite_window_message_is_refused() {
    let long = ToShell::WindowTitle { id: 1, title: "x".repeat(MAX_WINDOW_TITLE + 1) };
    // the encoder cuts the title, so the decoder never sees an overlong one from this crate
    let ToShell::WindowTitle { title, .. } = ToShell::decode(&long.encode()).unwrap() else { panic!() };
    assert_eq!(title.len(), MAX_WINDOW_TITLE);
    let mut bad = ToShell::WindowResize { id: 1, w: 1.0, h: 1.0 }.encode();
    bad[8..12].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(ToShell::decode(&bad).is_err());
    let mut bad_touch = ToApp::WindowTouch { id: 1, touch: 0, kind: TouchKind::Down, x: 0.0, y: 0.0, t_ms: 0, source: TouchSource::Finger, mods: ModifierFlags::EMPTY }.encode();
    bad_touch[12] = 9;
    assert!(ToApp::decode(&bad_touch).is_err());
}

#[test]
fn a_content_size_must_be_finite_positive_and_bounded() {
    let size = ToShell::SetContentSize { w: 230.0, h: 510.0 };
    assert_eq!(u32::from_le_bytes(size.encode()[..4].try_into().unwrap()), 0x252);
    assert_eq!(ToShell::decode(&size.encode()), Ok(size));
    for bad in [f32::NAN, f32::INFINITY, -1.0, 0.0, MAX_CONTENT_PT + 1.0] {
        assert!(ToShell::decode(&ToShell::SetContentSize { w: bad, h: 100.0 }.encode()).is_err(), "width {bad}");
        assert!(ToShell::decode(&ToShell::SetContentLimits { min: Some((100.0, bad)), max: None }.encode()).is_err(), "min height {bad}");
    }
    let limits = ToShell::SetContentLimits { min: Some((230.0, 400.0)), max: Some((800.0, 900.0)) };
    assert_eq!(u32::from_le_bytes(limits.encode()[..4].try_into().unwrap()), 0x253);
    assert_eq!(ToShell::decode(&limits.encode()), Ok(limits));
}
