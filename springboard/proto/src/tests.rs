use super::*;

fn tag_of(p: &[u8]) -> u32 {
    u32::from_le_bytes(p[..4].try_into().unwrap())
}

/// Each rich message owns one tag and decodes back to itself; the 0x1d0 block is shared by both directions, so a
/// tag must decode as the variant that encodes to it (ToShell 0x1d0/0x1d2, ToApp 0x1d1..0x1d3).
#[test]
fn distributed_and_service_messages_round_trip_on_their_tags() {
    let post = ToShell::DistPost { name: "n".into(), user_info: vec![("k".into(), "v".into())] };
    assert_eq!(tag_of(&post.encode()), 0x1d0);
    assert_eq!(ToShell::decode(&post.encode()), Ok(post));

    let call = ToShell::ServiceCall { request: 7, service: "dock".into(), method: "badge".into(), body: "null".into() };
    assert_eq!(tag_of(&call.encode()), 0x1d2);
    assert_eq!(ToShell::decode(&call.encode()), Ok(call));

    let dist = ToApp::DistNotify { name: "n".into(), user_info: vec![] };
    assert_eq!(tag_of(&dist.encode()), 0x1d1);
    assert_eq!(ToApp::decode(&dist.encode()), Ok(dist));

    let reply = ToApp::ServiceReply { request: 7, ok: false, body: "no such method".into() };
    assert_eq!(tag_of(&reply.encode()), 0x1d2);
    assert_eq!(ToApp::decode(&reply.encode()), Ok(reply));

    let event = ToApp::ServiceEvent { service: "now-playing".into(), event: "command".into(), body: "{}".into() };
    assert_eq!(tag_of(&event.encode()), 0x1d3);
    assert_eq!(ToApp::decode(&event.encode()), Ok(event));

    let ask = ToApp::SelectionRequest { request: 9 };
    assert_eq!(tag_of(&ask.encode()), 0x1d4);
    assert_eq!(ToApp::decode(&ask.encode()), Ok(ask));

    let answer = ToShell::SelectionReply { request: 9, text: "second".into() };
    assert_eq!(tag_of(&answer.encode()), 0x1d4);
    assert_eq!(ToShell::decode(&answer.encode()), Ok(answer));
}

#[test]
fn text_state_gives_the_selection_and_the_word_at_the_caret() {
    let t = |cursor: u32, anchor: u32| TextState { text: "the second line, isn't it".into(), cursor, anchor, marked: None };
    assert_eq!(t(10, 4).selected_text(), "second", "the anchor may come after the caret");
    assert_eq!(t(4, 10).selected_text(), "second");
    assert_eq!(t(4, 4).selected_text(), "", "a caret selects nothing");
    assert_eq!(t(40, 4).selected_text(), "", "out of range");
    assert_eq!(t(6, 6).word_at_cursor(), "second", "inside a word");
    assert_eq!(t(4, 4).word_at_cursor(), "second", "at its start");
    assert_eq!(t(10, 10).word_at_cursor(), "second", "at its end, before the space");
    assert_eq!(t(15, 15).word_at_cursor(), "line", "punctuation is not part of the word");
    assert_eq!(t(19, 19).word_at_cursor(), "isn't", "an apostrophe inside a word is");
    assert_eq!(t(99, 99).word_at_cursor(), "");
    let blank = TextState { text: "a  b".into(), cursor: 2, anchor: 2, marked: None };
    assert_eq!(blank.word_at_cursor(), "", "between two spaces");
    let wide = TextState { text: "þú ert".into(), cursor: 1, anchor: 1, marked: None };
    assert_eq!(wide.word_at_cursor(), "", "inside a character");
    let wide = TextState { text: "þú ert".into(), cursor: 2, anchor: 2, marked: None };
    assert_eq!(wide.word_at_cursor(), "þú");
}

/// E2E10: an editor that keeps its own text reports a window of it around the caret that always decodes.
#[test]
fn text_state_around_keeps_a_window_at_the_caret_that_decodes() {
    let short = TextState::around("the second line", 6, 4);
    assert_eq!((short.text.as_str(), short.cursor, short.anchor), ("the second line", 6, 4));
    assert_eq!(short.word_at_cursor(), "second");
    // a byte inside "þ" moves back to it; offsets past the end stop at the end
    let wide = TextState::around("þú ert", 1, 99);
    assert_eq!((wide.cursor, wide.anchor), (0, 8));
    // a long text: the window holds the caret's word, and the message decodes
    let long = format!("{} orange {}", "é".repeat(MAX_SURROUNDING), "ü".repeat(MAX_SURROUNDING));
    let at = long.find("orange").unwrap();
    let s = TextState::around(&long, at + 2, at);
    assert!(s.text.len() <= MAX_SURROUNDING);
    assert_eq!(s.word_at_cursor(), "orange");
    assert_eq!(s.selected_text(), "or");
    let m = ToShell::TextState(s);
    assert_eq!(ToShell::decode(&m.encode()), Ok(m));
}

/// A selection longer than `MAX_SURROUNDING` characters is cut on the way out, so the reply always decodes.
#[test]
fn a_long_selection_reply_is_cut_and_still_decodes() {
    let long = ToShell::SelectionReply { request: 1, text: "é".repeat(MAX_SURROUNDING + 10) };
    let Ok(ToShell::SelectionReply { request: 1, text }) = ToShell::decode(&long.encode()) else { panic!("decodes") };
    assert_eq!(text.chars().count(), MAX_SURROUNDING);
}

#[test]
fn keychain_messages_round_trip() {
    let add = |v| KeychainOp::Add { service: "svc".into(), account: "me".into(), value: v };
    for m in [
        ToShell::Keychain { request: 1, op: add(vec![1, 2, 3]) },
        ToShell::Keychain { request: 2, op: KeychainOp::Get { service: "s".into(), account: "a".into() } },
        ToShell::Keychain { request: 3, op: KeychainOp::Update { service: "s".into(), account: "a".into(), value: vec![] } },
        ToShell::Keychain { request: 4, op: KeychainOp::Delete { service: "s".into(), account: "a".into() } },
    ] {
        assert_eq!(ToShell::decode(&m.encode()).unwrap(), m);
    }
    let r = ToApp::KeychainReply { request: 9, status: KeychainStatus::Duplicate, value: vec![7; 5] };
    assert_eq!(ToApp::decode(&r.encode()).unwrap(), r);
    let big = ToShell::Keychain { request: 1, op: add(vec![0; MAX_KEYCHAIN_VALUE + 100]) };
    let Ok(ToShell::Keychain { op: KeychainOp::Add { value, .. }, .. }) = ToShell::decode(&big.encode()) else { panic!("decodes") };
    assert_eq!(value.len(), MAX_KEYCHAIN_VALUE + 1, "cut to one over the limit, which the store refuses");
}

#[test]
fn document_messages_round_trip_and_are_bounded() {
    let g: GrantId = [7; 16];
    for m in [
        ToShell::DocOpen { request: 1, mode: DocMode::Write, title: "Open".into(), filters: vec!["public.text".into(), "md".into()] },
        ToShell::DocOpenEx { request: 5, mode: DocMode::Read, title: "Choose".into(), filters: vec!["public.image".into()], options: docopt::MULTIPLE | docopt::DIRECTORIES | docopt::CREATE_DIRS },
        ToShell::DocSave { request: 2, suggested_name: "notes.md".into(), filters: vec![] },
        ToShell::DocCommit { request: 2 },
        ToShell::DocAbort { request: 3 },
        ToShell::DocReopen { request: 4, grant: g },
        ToShell::DocRelease { grant: g },
        ToShell::DocSaveGrant { request: 5, grant: g },
        ToShell::DocGrantPath { request: 6, path: "/home/a/Notes/b.txt".into() },
    ] {
        assert_eq!(ToShell::decode(&m.encode()), Ok(m));
    }
    for m in [
        ToApp::DocGranted { request: 1, grant: g, mode: DocMode::Read, name: "a.txt".into(), size: 1 << 40 },
        ToApp::DocError { request: 9, code: DocErrorCode::Stale },
        ToApp::DocBatch { request: 5, count: 3 },
    ] {
        assert_eq!(ToApp::decode(&m.encode()), Ok(m));
    }
    // the encoder drops empty and oversized filters and keeps at most MAX_DOC_FILTERS; long titles are cut
    let many: Vec<String> = (0..40).map(|i| format!("ext{i}")).chain(["".into(), "x".repeat(65)]).collect();
    let ToShell::DocOpen { filters, title, .. } =
        ToShell::decode(&ToShell::DocOpen { request: 0, mode: DocMode::Read, title: "t".repeat(1000), filters: many }.encode()).unwrap()
    else {
        panic!()
    };
    assert_eq!((filters.len(), title.chars().count()), (MAX_DOC_FILTERS, MAX_DOC_TEXT));
    // a hand-built packet that breaks the bounds is refused
    let mut bad = Vec::new();
    u32_(&mut bad, tag::DOC_OPEN);
    u32_(&mut bad, 0);
    bad.push(0);
    str_(&mut bad, "t");
    u32_(&mut bad, 17);
    assert_eq!(ToShell::decode(&bad), Err(DecodeError::BadValue));
    let mut bad_mode = ToShell::DocOpen { request: 0, mode: DocMode::Read, title: String::new(), filters: vec![] }.encode();
    bad_mode[8] = 2;
    assert_eq!(ToShell::decode(&bad_mode), Err(DecodeError::BadValue));
    let mut too_many = ToApp::DocBatch { request: 1, count: 1 }.encode();
    too_many[8..12].copy_from_slice(&(MAX_DOC_BATCH + 1).to_le_bytes());
    assert_eq!(ToApp::decode(&too_many), Err(DecodeError::BadValue));
    let mut bad_code = ToApp::DocError { request: 0, code: DocErrorCode::Denied }.encode();
    bad_code[8] = 6;
    assert_eq!(ToApp::decode(&bad_code), Err(DecodeError::BadValue));
    assert_eq!(ToShell::decode(&ToShell::DocRelease { grant: g }.encode()[..10]), Err(DecodeError::Truncated));
}

#[test]
fn chords_round_trip_as_codes_and_plain_codes_are_not_chords() {
    let k = KeyInput::chord(49, CHORD_META | CHORD_SHIFT);
    assert_eq!(k.as_chord(), Some((49, CHORD_META | CHORD_SHIFT)));
    assert_eq!(KeyInput::Code(49).as_chord(), None, "a plain key is not a chord");
    assert_eq!(KeyInput::Text("n".into()).as_chord(), None);
    // a chord is an ordinary Code on the wire, so an SDK 1 app decodes it without error
    let m = ToApp::Key(k.clone()).encode();
    assert_eq!(ToApp::decode(&m).unwrap(), ToApp::Key(k));
}

#[test]
fn controls_are_bounded_and_invalid_commands_rejected() {
    let nodes=(0..200).map(|id|Node{id,role:Role::Button,label:"x".repeat(200),value:"y".repeat(200),bounds:UiRect{x:0.,y:0.,w:44.,h:44.},enabled:true}).collect();
    let packet=ToShell::Controls(nodes).encode();assert!(packet.len()<=MAX_PACKET);
    let ToShell::Controls(decoded)=ToShell::decode(&packet).unwrap() else{panic!("wrong message")};assert!(!decoded.is_empty());assert!(decoded.len()<200);
    let mut packet=ToApp::Command(Command::Cut).encode();packet[4..8].copy_from_slice(&99u32.to_le_bytes());assert_eq!(ToApp::decode(&packet),Err(DecodeError::BadValue));
}

#[test]
fn semantic_control_roles_round_trip_with_stable_wire_values() {
    let roles = [Role::Button, Role::TextField, Role::List, Role::Row, Role::Toggle,
        Role::Checkbox, Role::Slider, Role::SegmentedControl, Role::StaticText,
        Role::MenuItem, Role::Window, Role::ProgressIndicator];
    for (i, role) in roles.into_iter().enumerate() {
        assert_eq!(role as u32, i as u32, "append-only role wire value");
        let node = Node { id: i as u64, role, label: format!("role-{i}"), value: String::new(),
            bounds: UiRect { x: 0., y: 0., w: 40., h: 28. }, enabled: true };
        assert_eq!(ToShell::decode(&ToShell::Controls(vec![node.clone()]).encode()),
            Ok(ToShell::Controls(vec![node])));
    }
    let node = Node { id: 0, role: Role::Button, label: String::new(), value: String::new(),
        bounds: UiRect { x: 0., y: 0., w: 1., h: 1. }, enabled: true };
    let mut invalid = ToShell::Controls(vec![node]).encode();
    invalid[16..20].copy_from_slice(&99u32.to_le_bytes());
    assert_eq!(ToShell::decode(&invalid), Err(DecodeError::BadValue));
}

/// E2E12 2: the shell tells an app that was activated with its main window closed (tag 0x254), and an app asks for
/// its main window to be ordered out with `WindowClose` id 0.
#[test]
fn reopen_and_ordering_out_the_main_window_have_their_tags() {
    let bytes = ToApp::Reopen.encode();
    assert_eq!(&bytes[..4], &0x254u32.to_le_bytes());
    assert_eq!(ToApp::decode(&bytes), Ok(ToApp::Reopen));
    let close = ToShell::WindowClose { id: 0 };
    assert_eq!(ToShell::decode(&close.encode()), Ok(close));
}

/// An app that handles the close control itself announces it (tag 0x256) and then hears `CloseRequest` (tag 0x255).
#[test]
fn close_requests_have_their_tags() {
    let bytes = ToApp::CloseRequest.encode();
    assert_eq!(&bytes[..4], &0x255u32.to_le_bytes());
    assert_eq!(ToApp::decode(&bytes), Ok(ToApp::CloseRequest));
    let opt_in = ToShell::CloseRequests { on: true }.encode();
    assert_eq!(&opt_in[..4], &0x256u32.to_le_bytes());
    assert_eq!(ToShell::decode(&opt_in), Ok(ToShell::CloseRequests { on: true }));
    assert_eq!(ToShell::decode(&ToShell::CloseRequests { on: false }.encode()), Ok(ToShell::CloseRequests { on: false }));
    // the close control of an extra window carries the window
    let extra = ToApp::WindowCloseRequest { id: 3 }.encode();
    assert_eq!(&extra[..4], &0x257u32.to_le_bytes());
    assert_eq!(ToApp::decode(&extra), Ok(ToApp::WindowCloseRequest { id: 3 }));
}

#[test]
fn to_app_round_trips() {
    let cfg = Configure { w: 1200, h: 1920, scale: 1.5625, safe_top: 24.0, safe_bottom: 20.0, dark: true, text_scale: 1.1, reduce_motion: false };
    let all = vec![
        ToApp::Configure(cfg),
        ToApp::Touch { kind: TouchKind::Down, id: 3, x: 10.5, y: -2.0, t_ms: 123456, source: TouchSource::Finger, mods: ModifierFlags::EMPTY },
        ToApp::Touch { kind: TouchKind::Down, id: 3, x: 1.0, y: 2.0, t_ms: 7, source: TouchSource::Pointer, mods: ModifierFlags::SHIFT | ModifierFlags::COMMAND },
        ToApp::Touch { kind: TouchKind::Cancel, id: 0, x: 0.0, y: 0.0, t_ms: 0, source: TouchSource::Finger, mods: ModifierFlags::EMPTY },
        ToApp::Key(KeyInput::Text("héllo ✓".into())),
        ToApp::Key(KeyInput::Code(66)),
        ToApp::Release { buf: 1, generation: 5 },
        ToApp::Command(Command::Paste),
        ToApp::KeyboardConfig(KeyboardConfig::default()),
        ToApp::KeyboardConfig(KeyboardConfig { layout: KeyboardLayout::Floating, return_key: ReturnKey::Send, flags: kbflags::PREDICTIVE | kbflags::SHOWN, languages: 3, language: 1 }),
        ToApp::Activate(u64::MAX),
        ToApp::Reopen,
        ToApp::CloseRequest,
        ToApp::QuitRequest,
        ToApp::Resume,
        ToApp::Suspend,
        ToApp::Quit,
        ToApp::Open { url: "https://x.is/?a=1".into() },
        ToApp::Clipboard { text: String::new() },
        ToApp::Appearance { flags: appearance::INCREASE_CONTRAST | appearance::SHOW_BORDERS | 0x8000_0000 },
        ToApp::InputSource { japanese: true },
        ToApp::InputSource { japanese: false },
        ToApp::AccentHold { letter: 'é' },
    ];
    for m in all {
        assert_eq!(ToApp::decode(&m.encode()), Ok(m.clone()), "{m:?}");
    }
}

#[test]
fn to_shell_round_trips() {
    let all = vec![
        ToShell::Hello { sdk_version: SDK_VERSION, app_id: "is.olibuijr.calculator".into() },
        ToShell::Buffers { w: 1200, h: 1920, generation: 2 },
        ToShell::Frame { buf: 1, damage: vec![[0, 0, 1200, 1920], [5, 6, 7, 8]] },
        ToShell::Frame { buf: 0, damage: vec![] },
        ToShell::WantsKeyboard { on: true },
        ToShell::KeyboardOut(KeyboardOut::Text("é😀".into())),
        ToShell::KeyboardOut(KeyboardOut::Backspace),
        ToShell::KeyboardOut(KeyboardOut::Enter),
        ToShell::KeyboardOut(KeyboardOut::Hide),
        ToShell::KeyboardOut(KeyboardOut::Named("Escape".into())),
        ToShell::KeyboardOut(KeyboardOut::Height(824)),
        ToShell::KeyboardOut(KeyboardOut::Click(2)),
        ToShell::KeyboardOut(KeyboardOut::Language(1)),
        ToShell::Commands(Commands(15)),
        ToShell::UndoTitles { undo: "Undo Typing".into(), redo: String::new() },
        ToShell::Controls(vec![Node{id:u64::MAX,role:Role::TextField,label:"Search".into(),value:"café".into(),bounds:UiRect{x:1.,y:2.,w:100.,h:44.},enabled:true}]),
        ToShell::RemoteSharing{on:true},
        ToShell::RemoteSharing{on:false},
        ToShell::Haptic,
        ToShell::SetBadge { label: "12".into() },
        ToShell::SetBadge { label: String::new() },
        ToShell::SetFullScreen { on: true },
        ToShell::SetFullScreen { on: false },
        ToShell::SetContentSize { w: 230.0, h: 510.0 },
        ToShell::SetContentLimits { min: Some((230.0, 400.0)), max: None },
        ToShell::SetContentLimits { min: None, max: Some((1200.0, 900.0)) },
        ToShell::SetContentLimits { min: None, max: None },
        ToShell::Materials(vec![Material { kind: MaterialKind::Sidebar, rect: UiRect { x: 0., y: 0., w: 220., h: 600. }, radius: 17., state: MaterialState::FollowsWindowActiveState, blending: BlendingMode::BehindWindow, emphasized: false, corners: None },
            Material { kind: MaterialKind::Titlebar, rect: UiRect { x: 220., y: 0., w: 580., h: 52. }, radius: 0., state: MaterialState::Active, blending: BlendingMode::WithinWindow, emphasized: true, corners: Some(Corners { tl: 1., tr: 2., br: 3., bl: 4. }) }]),
        ToShell::Materials(MaterialKind::ALL.iter().map(|&kind| Material { kind, rect: UiRect { x: 1., y: 2., w: 3., h: 4. }, radius: 5., state: MaterialState::Inactive, blending: BlendingMode::BehindWindow, emphasized: false, corners: None }).collect()),
        ToShell::Materials(vec![]),
        ToShell::OpenUrl { url: "https://bifrost.olibuijr.com".into() },
        ToShell::PlaySystemSound { name: "Bell".into() },
        ToShell::Notify { title: "T".into(), body: "B ü".into() },
        ToShell::Log { text: "hi".into() },
        ToShell::SetClipboard { text: "1,5".into() },
    ];
    for m in all {
        assert_eq!(ToShell::decode(&m.encode()), Ok(m.clone()), "{m:?}");
    }
}

#[test]
fn exact_layout() {
    assert_eq!(ToApp::Release { buf: 1, generation: 2 }.encode(), [4, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0]);
    assert_eq!(ToShell::Haptic.encode(), [5, 1, 0, 0]);
    assert_eq!(ToApp::Key(KeyInput::Code(7)).encode(), [3, 0, 0, 0, 1, 7, 0, 0, 0]);
}

#[test]
fn rejects_bad_input() {
    assert_eq!(ToApp::decode(&[]), Err(DecodeError::Truncated));
    assert_eq!(ToApp::decode(&[99, 0, 0, 0]), Err(DecodeError::UnknownTag(99)));
    assert_eq!(ToApp::decode(&[4, 0, 0, 0, 1]), Err(DecodeError::Truncated));
    assert_eq!(ToApp::decode(&[8, 0, 0, 0, 2, 0, 0, 0, 0xff, 0xfe]), Err(DecodeError::BadUtf8));
    assert_eq!(ToShell::decode(&[0x03, 1, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]), Err(DecodeError::BadValue));
    // keyboard enums reject unknown values and a truncated Text
    assert_eq!(ToApp::decode(&[15, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0, 0]), Err(DecodeError::BadValue));
    assert_eq!(ToApp::decode(&[15, 0, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0]), Err(DecodeError::BadValue));
    assert_eq!(ToShell::decode(&[0x0f, 1, 0, 0, 99]), Err(DecodeError::BadValue));
    assert_eq!(ToShell::decode(&[0x0f, 1, 0, 0, 0, 5, 0, 0, 0]), Err(DecodeError::Truncated));
    assert_eq!(ToApp::decode(&[2, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), Err(DecodeError::BadValue));
}

#[test]
fn long_text_is_clamped_on_a_char_boundary() {
    let m = ToShell::Log { text: "é".repeat(MAX_PACKET) };
    let b = m.encode();
    assert!(b.len() <= MAX_PACKET);
    assert!(matches!(ToShell::decode(&b), Ok(ToShell::Log { .. })));
}
