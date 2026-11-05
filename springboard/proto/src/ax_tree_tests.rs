use super::*;

#[test]
fn text_input_messages_round_trip() {
    let to_app = [
        ToApp::SetMarkedText { text: "にほん".into(), sel_start: 3, sel_len: 3 },
        ToApp::SetMarkedText { text: String::new(), sel_start: 0, sel_len: 0 },
        ToApp::UnmarkText { commit: true },
        ToApp::UnmarkText { commit: false },
        ToApp::DeleteSurrounding { before: 2, after: 1 },
        ToApp::AccessoryAction(9),
    ];
    for m in to_app {
        assert_eq!(ToApp::decode(&m.encode()), Ok(m));
    }
    let to_shell = [
        ToShell::TextState(TextState { text: "héllo wörld".into(), cursor: 3, anchor: 7, marked: Some((1, 3)) }),
        ToShell::TextState(TextState::default()),
        ToShell::CaretRect { x: 1.0, y: 2.0, w: 0.0, h: 18.0 },
        ToShell::InputTraits(InputTraits { return_key: ReturnKeyType::Continue, content: TextContentKind::Email, flags: tiflags::AUTOCORRECT | tiflags::SECURE }),
        ToShell::InputAccessory(vec![AccessoryItem { id: 1, label: "Done".into(), enabled: true }, AccessoryItem { id: 2, label: "Next".into(), enabled: false }]),
        ToShell::InputAccessory(vec![]),
        ToShell::SuddenTermination { enabled: true },
        ToShell::SuddenTermination { enabled: false },
        ToShell::CloseRequests { on: true },
        ToShell::CloseRequests { on: false },
        ToShell::Activity { options: 0x3 },
        ToShell::Activity { options: 0 },
        ToShell::WindowLevel(WindowLevel::Floating),
        ToShell::WindowLevel(WindowLevel::Normal),
        ToShell::WindowLevelOf { id: 2, level: WindowLevel::Floating },
        ToShell::WindowLevelOf { id: 2, level: WindowLevel::Normal },
    ];
    for m in to_shell {
        assert_eq!(ToShell::decode(&m.encode()), Ok(m));
    }
}

#[test]
fn a_window_level_of_one_window_is_id_then_one_byte() {
    let m = ToShell::WindowLevelOf { id: 3, level: WindowLevel::Floating }.encode();
    assert_eq!(&m[..4], &0x24cu32.to_le_bytes());
    assert_eq!(&m[4..8], &3u32.to_le_bytes());
    assert_eq!(*m.last().unwrap(), 1);
    let mut bad = m.clone();
    *bad.last_mut().unwrap() = 9;
    assert_eq!(ToShell::decode(&bad), Err(DecodeError::BadValue));
}

#[test]
fn window_level_is_one_byte_and_rejects_an_unknown_level() {
    let m = ToShell::WindowLevel(WindowLevel::Floating).encode();
    assert_eq!(m.len(), 4 + 1, "the tag, then the level byte");
    assert_eq!(*m.last().unwrap(), 1);
    let mut bad = m.clone();
    let n = bad.len();
    bad[n - 1] = 7;
    assert_eq!(ToShell::decode(&bad), Err(DecodeError::BadValue));
}

#[test]
fn keyboard_preedit_round_trips_and_rejects_a_bad_selection() {
    for o in [KeyboardOut::Preedit { text: "にほん".into(), sel_start: 3, sel_len: 3 }, KeyboardOut::Unmark { commit: true }] {
        let m = ToShell::KeyboardOut(o);
        assert_eq!(ToShell::decode(&m.encode()), Ok(m));
    }
    let mut p = ToShell::KeyboardOut(KeyboardOut::Preedit { text: "é".into(), sel_start: 0, sel_len: 2 }).encode();
    let n = p.len();
    p[n - 4] = 1;
    assert_eq!(ToShell::decode(&p), Err(DecodeError::BadValue));
}

#[test]
fn text_input_messages_reject_bad_values() {
    // a selection that ends past the text
    let mut p = ToApp::SetMarkedText { text: "ab".into(), sel_start: 0, sel_len: 2 }.encode();
    let n = p.len();
    p[n - 4] = 9;
    assert_eq!(ToApp::decode(&p), Err(DecodeError::BadValue));
    // a selection that cuts a character in half
    let mut p = ToApp::SetMarkedText { text: "é".into(), sel_start: 0, sel_len: 2 }.encode();
    let n = p.len();
    p[n - 4] = 1;
    assert_eq!(ToApp::decode(&p), Err(DecodeError::BadValue));
    let mut p = ToShell::InputTraits(InputTraits::default()).encode();
    p[4] = 99;
    assert_eq!(ToShell::decode(&p), Err(DecodeError::BadValue));
    assert_eq!(ToShell::decode(&ToShell::CaretRect { x: f32::NAN, y: 0., w: 1., h: 1. }.encode()), Err(DecodeError::BadValue));
}

#[test]
fn text_state_window_is_cut_around_the_caret() {
    let text = "é".repeat(MAX_SURROUNDING); // twice the limit in bytes
    let cursor = MAX_SURROUNDING as u32; // a character boundary in the middle
    let m = ToShell::TextState(TextState { text, cursor, anchor: cursor, marked: None });
    let Ok(ToShell::TextState(t)) = ToShell::decode(&m.encode()) else { panic!("decode") };
    assert!(t.text.len() <= MAX_SURROUNDING);
    assert!(t.cursor as usize <= t.text.len());
    assert_eq!(t.cursor, t.anchor);
}

#[test]
fn return_key_types_map_to_the_keyboard_labels() {
    for v in 0..=11u8 {
        let t = ReturnKeyType::from_wire(v).unwrap();
        assert_eq!(t as u8, v);
        let _ = t.basic();
    }
    assert_eq!(ReturnKeyType::from_wire(12), None);
    assert_eq!(ReturnKeyType::Yahoo.basic(), ReturnKey::Search);
    assert_eq!(ReturnKeyType::Continue.basic(), ReturnKey::Next);
}

#[test]
fn controls_tree_round_trips() {
    let list = vec![
        NodeExt { id: 1, parent: None, focused: false, checked: None },
        NodeExt { id: u64::MAX, parent: Some(1 << 40), focused: true, checked: Some(true) },
        NodeExt { id: 7, parent: Some(0), focused: false, checked: Some(false) },
    ];
    assert_eq!(ToShell::decode(&ToShell::ControlsTree(list.clone()).encode()), Ok(ToShell::ControlsTree(list)));
}

#[test]
fn controls_tree_rejects_unknown_flags_and_oversize() {
    let mut p = ToShell::ControlsTree(vec![NodeExt { id: 1, parent: None, focused: false, checked: None }]).encode();
    *p.last_mut().unwrap() = 0x80;
    assert_eq!(ToShell::decode(&p), Err(DecodeError::BadValue));
    let big = ToShell::ControlsTree(vec![NodeExt { id: 1, parent: None, focused: false, checked: None }; 300]).encode();
    let Ok(ToShell::ControlsTree(l)) = ToShell::decode(&big) else { panic!("decode") };
    assert_eq!(l.len(), MAX_NODE_EXT);
}

#[test]
fn ax_set_round_trips() {
    for action in [AxAction::Press, AxAction::Focus, AxAction::Increment, AxAction::Decrement, AxAction::SetValue("héllo".into())] {
        let m = ToApp::AxSet { id: 0x1_0000_0002, action };
        assert_eq!(ToApp::decode(&m.encode()), Ok(m));
    }
    let mut bad = ToApp::AxSet { id: 1, action: AxAction::Press }.encode();
    *bad.last_mut().unwrap() = 9;
    assert_eq!(ToApp::decode(&bad), Err(DecodeError::BadValue));
}
