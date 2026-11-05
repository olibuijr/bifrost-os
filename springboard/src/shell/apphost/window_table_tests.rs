use super::*;

fn cfg() -> Configure {
    Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false }
}

/// Every message the shell has queued for the app so far.
fn sent(peer: &Channel) -> Vec<ToApp> {
    let mut out = Vec::new();
    while let Recv::Packet(p, _) = peer.recv(false).unwrap() {
        out.push(ToApp::decode(&p).unwrap());
    }
    out
}

#[test]
fn the_key_window_gets_the_input_and_the_app_hears_each_focus_change() {
    let (channel, peer) = sys::socketpair_pair().unwrap();
    let mut host = AppHost::with_channel("fixture", channel, None, cfg());
    host.set_key_window(4);
    host.input(WindowInput::Command(bifrost_proto::Command::SelectAll));
    host.key(KeyInput::Code(30));
    host.set_key_window(4);
    host.set_key_window(0);
    host.key(KeyInput::Code(9));
    assert_eq!(
        sent(&peer),
        vec![
            ToApp::WindowFocus { window: 0, focused: false },
            ToApp::WindowFocus { window: 4, focused: true },
            ToApp::WindowInput { window: 4, input: WindowInput::Command(bifrost_proto::Command::SelectAll) },
            ToApp::WindowInput { window: 4, input: WindowInput::Key(KeyInput::Code(30)) },
            ToApp::WindowFocus { window: 4, focused: false },
            ToApp::WindowFocus { window: 0, focused: true },
            ToApp::Key(KeyInput::Code(9)),
        ],
        "a repeated focus sends nothing; the main window's keys keep the legacy message"
    );
}

#[test]
fn input_to_reaches_the_named_window_whichever_is_key() {
    let (channel, peer) = sys::socketpair_pair().unwrap();
    let mut host = AppHost::with_channel("fixture", channel, None, cfg());
    // a dead key's mark is taken back from the window it was shown in after the focus moved on
    host.input_to(3, WindowInput::UnmarkText { commit: false });
    host.input_to(0, WindowInput::UnmarkText { commit: false });
    let got = sent(&peer);
    assert!(got.ends_with(&[
        ToApp::WindowInput { window: 3, input: WindowInput::UnmarkText { commit: false } },
        ToApp::UnmarkText { commit: false },
    ]), "{got:?}");
}

#[test]
fn marked_text_and_accent_holds_follow_the_key_window() {
    let (channel, peer) = sys::socketpair_pair().unwrap();
    let mut host = AppHost::with_channel("fixture", channel, None, cfg());
    host.set_key_window(2);
    host.input(super::super::textinput::to_input(super::super::keyboard::Composition::Preedit { text: "´".into(), sel_start: 2, sel_len: 0 }));
    host.input(WindowInput::AccentHold { letter: 'a' });
    host.set_key_window(0);
    host.input(WindowInput::UnmarkText { commit: true });
    host.input(WindowInput::AccentHold { letter: 'o' });
    let got = sent(&peer);
    assert_eq!(
        got[2..],
        [
            ToApp::WindowInput { window: 2, input: WindowInput::SetMarkedText { text: "´".into(), sel_start: 2, sel_len: 0 } },
            ToApp::WindowInput { window: 2, input: WindowInput::AccentHold { letter: 'a' } },
            ToApp::WindowFocus { window: 2, focused: false },
            ToApp::WindowFocus { window: 0, focused: true },
            ToApp::UnmarkText { commit: true },
            ToApp::AccentHold { letter: 'o' },
        ],
        "an extra key window's composition carries its id; the main window keeps the legacy messages"
    );
}

#[test]
fn a_window_takes_a_position_collapses_and_opts_in_to_the_title_double_click() {
    let (channel, peer) = sys::socketpair_pair().unwrap();
    let mut host = AppHost::with_channel("fixture", channel, None, cfg());
    let mut out = Pumped::default();
    let flags = bifrost_proto::window_flags::TITLE_DOUBLE_CLICK;
    host.handle(ToShell::WindowOpen { id: 4, title: "Note".into(), w: 200.0, h: 150.0, position: Some((12.0, 40.0)), flags }, Vec::new(), &mut out);
    assert_eq!(host.windows.take_pending_move("fixture", 4), Some((12.0, 40.0)), "the requested position is applied once");
    host.handle(ToShell::WindowCollapse { window: 4, collapsed: true }, Vec::new(), &mut out);
    assert!(host.windows.of_app("fixture")[0].collapsed);
    assert!(host.window_title_double_clicked(4));
    assert_eq!(sent(&peer), vec![ToApp::WindowTitleDoubleClick { window: 4 }]);
    assert!(host.window_dragged(4, 100.0, 60.0));
    assert_eq!(sent(&peer), vec![ToApp::WindowMoved { window: 4, x: 100.0, y: 60.0 }]);
    host.handle(ToShell::WindowOpen { id: 5, title: "Plain".into(), w: 10.0, h: 10.0, position: None, flags: 0 }, Vec::new(), &mut out);
    assert!(!host.window_title_double_clicked(5), "an app that did not opt in gets no message");
    assert!(sent(&peer).is_empty());
    let cascade = bifrost_proto::window_flags::CASCADE;
    host.handle(ToShell::WindowOpen { id: 6, title: "Viewer".into(), w: 10.0, h: 10.0, position: None, flags: cascade }, Vec::new(), &mut out);
    host.handle(ToShell::WindowOpen { id: 7, title: "Placed".into(), w: 10.0, h: 10.0, position: Some((1.0, 2.0)), flags: cascade }, Vec::new(), &mut out);
    let cascades = |id: u32| host.windows.of_app("fixture").iter().any(|w| w.id == id && w.cascade);
    assert!(cascades(6), "a window opened with the flag and no position cascades");
    assert!(!cascades(7), "a requested position wins over the cascade");
    assert!(!cascades(5), "a plain window does not cascade");
    let full = bifrost_proto::window_flags::FULL_SIZE;
    host.handle(ToShell::WindowOpen { id: 8, title: "Untitled 2".into(), w: 10.0, h: 10.0, position: None, flags: full }, Vec::new(), &mut out);
    let full_size = |id: u32| host.windows.of_app("fixture").iter().any(|w| w.id == id && w.full_size);
    assert!(full_size(8), "a window opened with the flag draws its own title row");
    assert!(!full_size(5), "a plain window keeps the shell's title bar");
}

#[test]
fn the_window_table_follows_the_messages_the_app_sends() {
    let (channel, _peer) = sys::socketpair_pair().unwrap();
    let mut host = AppHost::with_channel("fixture", channel, None, cfg());
    let mut out = Pumped::default();
    host.handle(ToShell::WindowOpen { id: 1, title: "Ada".into(), w: 480.0, h: 560.0, position: None, flags: 0 }, Vec::new(), &mut out);
    host.handle(ToShell::WindowOpen { id: 2, title: "Grace".into(), w: 480.0, h: 560.0, position: None, flags: 0 }, Vec::new(), &mut out);
    let ids = |h: &AppHost| h.windows.of_app("fixture").iter().map(|w| w.id).collect::<Vec<u32>>();
    assert_eq!(ids(&host), vec![2, 1], "the newest window is in front");
    assert!(out.frame, "a window change asks the desk for a frame");
    host.handle(ToShell::WindowTitle { id: 1, title: "Ada L.".into() }, Vec::new(), &mut out);
    host.handle(ToShell::WindowResize { id: 1, w: 320.0, h: 240.0 }, Vec::new(), &mut out);
    let one = host.windows.of_app("fixture")[1];
    assert_eq!((one.title.as_str(), one.w, one.h), ("Ada L.", 320.0, 240.0));
    host.handle(ToShell::WindowClose { id: 2 }, Vec::new(), &mut out);
    assert_eq!(ids(&host), vec![1]);
}

/// E2E12 2: closing a document app's last document orders its main window out (`WindowClose` id 0) without touching
/// the extra-window table, and `WindowOpen` id 0 asks for it back; each is taken once.
#[test]
fn the_main_window_is_ordered_out_and_back_once_for_the_desk() {
    let (channel, _peer) = sys::socketpair_pair().unwrap();
    let mut host = AppHost::with_channel("fixture", channel, None, cfg());
    let mut out = Pumped::default();
    host.handle(ToShell::WindowOpen { id: 1, title: "Second".into(), w: 300.0, h: 200.0, position: None, flags: 0 }, Vec::new(), &mut out);
    host.handle(ToShell::WindowClose { id: 0 }, Vec::new(), &mut out);
    assert!(out.chrome, "the desk is asked to apply it");
    assert_eq!(host.windows.of_app("fixture").len(), 1, "the extra window is not closed");
    assert_eq!(host.take_main_order(), Some(false));
    assert_eq!(host.take_main_order(), None, "applied once");
    host.handle(ToShell::WindowOpen { id: 0, title: String::new(), w: 0.0, h: 0.0, position: None, flags: 0 }, Vec::new(), &mut out);
    assert_eq!(host.windows.of_app("fixture").len(), 1, "id 0 is not an extra window");
    assert_eq!(host.take_main_order(), Some(true));
}

/// E2E11 5: a document app that moves a document into its main window asks for the main window to take that
/// document's frame (`WindowMove` with window 0); the desk takes the position once. A position that is not a number
/// is refused.
#[test]
fn a_main_window_move_is_kept_once_for_the_desk() {
    let (channel, _peer) = sys::socketpair_pair().unwrap();
    let mut host = AppHost::with_channel("fixture", channel, None, cfg());
    let mut out = Pumped::default();
    host.handle(ToShell::WindowMove { window: 0, x: f32::NAN, y: 10.0 }, Vec::new(), &mut out);
    assert_eq!(host.take_main_move(), None);
    host.handle(ToShell::WindowMove { window: 0, x: 120.0, y: 64.0 }, Vec::new(), &mut out);
    assert!(out.chrome, "the desk is asked to apply it");
    assert_eq!(host.take_main_move(), Some((120.0, 64.0)));
    assert_eq!(host.take_main_move(), None, "applied once");
    assert!(host.windows.of_app("fixture").is_empty(), "the main window is not an extra window");
}

/// The state an app sends again unchanged is free: it asks the shell for no frame at all. Pixels alone ask for a
/// frame of windows (`frame`), not for the shell to draw anything itself (`chrome`).
#[test]
fn identical_state_asks_for_no_frame_and_pixels_alone_are_no_chrome_change() {
    let (mut host, _peer) = super::route_tests::fixture_host();
    let titles = || ToShell::UndoTitles { undo: "Undo Typing".into(), redo: String::new() };
    let mut first = Pumped::default();
    host.handle(titles(), Vec::new(), &mut first);
    assert!(first.frame && first.chrome, "new titles are something the shell draws");
    let mut again = Pumped::default();
    host.handle(titles(), Vec::new(), &mut again);
    assert_eq!(again, Pumped::default(), "the same titles again cost nothing");
    host.handle(ToShell::Materials(Vec::new()), Vec::new(), &mut again);
    host.handle(ToShell::Commands(host.commands), Vec::new(), &mut again);
    assert_eq!(again, Pumped::default(), "nor do the same materials and commands");
    host.handle(ToShell::WindowOpen { id: 1, title: "Ada".into(), w: 480.0, h: 560.0, position: None, flags: 0 }, Vec::new(), &mut again);
    assert!(again.chrome, "a window opening is drawn by the shell");
    let mut same = Pumped::default();
    host.handle(ToShell::WindowTitle { id: 1, title: "Ada".into() }, Vec::new(), &mut same);
    host.handle(ToShell::WindowResize { id: 1, w: 480.0, h: 560.0 }, Vec::new(), &mut same);
    assert_eq!(same, Pumped::default(), "the same title and size again cost nothing");
    host.handle(ToShell::WindowTitle { id: 1, title: "Ada L.".into() }, Vec::new(), &mut same);
    assert!(same.frame && same.chrome, "a new title is drawn");
    // pixels
    let set = |n: &str| vec![sys::shm_create(n, 48).unwrap(), sys::shm_create(n, 48).unwrap()];
    let mut pixels = Pumped::default();
    host.handle(ToShell::Buffers { w: 4, h: 3, generation: 1 }, set("quiet-pixels"), &mut pixels);
    host.handle(ToShell::Frame { buf: 0, damage: vec![[0, 0, 4, 3]] }, Vec::new(), &mut pixels);
    assert!(pixels.frame, "new pixels are a frame");
    assert!(!pixels.chrome, "and only a frame of windows");
}

#[test]
fn a_window_the_table_refuses_is_closed_again_for_the_app() {
    let (channel, peer) = sys::socketpair_pair().unwrap();
    let mut host = AppHost::with_channel("fixture", channel, None, cfg());
    let mut out = Pumped::default();
    let refused = crate::shell::app_window_table::MAX_PER_APP as u32 + 1;
    for id in 1..=refused {
        host.handle(ToShell::WindowOpen { id, title: "w".into(), w: 10.0, h: 10.0, position: None, flags: 0 }, Vec::new(), &mut out);
    }
    assert_eq!(host.windows.of_app("fixture").len(), crate::shell::app_window_table::MAX_PER_APP);
    assert_eq!(sent(&peer), vec![ToApp::WindowClosed { id: refused }]);
}

#[test]
fn a_window_gets_its_own_buffers_and_its_frame_is_given_back_once_blitted() {
    let (channel, peer) = sys::socketpair_pair().unwrap();
    let mut host = AppHost::with_channel("fixture", channel, None, cfg());
    let mut out = Pumped::default();
    host.handle(ToShell::WindowOpen { id: 1, title: "Ada".into(), w: 2.0, h: 2.0, position: None, flags: 0 }, Vec::new(), &mut out);
    let bytes = 2 * 2 * 4;
    let fds = vec![sys::shm_create("test-window-0", bytes).unwrap(), sys::shm_create("test-window-1", bytes).unwrap()];
    host.handle(ToShell::WindowBuffers { id: 1, w: 2, h: 2, generation: 7 }, fds, &mut out);
    assert!(host.window_frame(1).is_none(), "no frame before the first one");
    host.handle(ToShell::WindowFrame { id: 1, buf: 0, damage: vec![[0, 0, 2, 2]] }, Vec::new(), &mut out);
    assert_eq!(host.window_frame(1).map(|(_, w, h)| (w, h)), Some((2, 2)));
    host.handle(ToShell::WindowFrame { id: 1, buf: 1, damage: vec![[0, 0, 2, 2]] }, Vec::new(), &mut out);
    assert!(sent(&peer).is_empty(), "the replaced buffer waits until its frame is blitted");
    host.window_blitted(1);
    assert_eq!(sent(&peer), vec![ToApp::WindowRelease { id: 1, buf: 0, generation: 7 }]);
    host.handle(ToShell::WindowFrame { id: 9, buf: 0, damage: vec![] }, Vec::new(), &mut out);
    assert!(host.window_frame(9).is_none(), "a frame for a window that is not open is dropped");
}

/// MA6: an SDK built before the fix sent a window's buffers and first frame before its `WindowOpen` (the request waited in
/// the app's queue until the end of the turn). The shell dropped them, the app never sent the first frame again, and the
/// window stayed unmapped. They are kept and handled when the window opens.
#[test]
fn buffers_and_a_frame_that_come_before_the_window_open_are_kept_until_it_opens() {
    let (channel, peer) = sys::socketpair_pair().unwrap();
    let mut host = AppHost::with_channel("fixture", channel, None, cfg());
    let mut out = Pumped::default();
    let bytes = 2 * 2 * 4;
    let fds = vec![sys::shm_create("early-window-0", bytes).unwrap(), sys::shm_create("early-window-1", bytes).unwrap()];
    host.handle(ToShell::WindowBuffers { id: 1, w: 2, h: 2, generation: 3 }, fds, &mut out);
    host.handle(ToShell::WindowFrame { id: 1, buf: 0, damage: vec![[0, 0, 2, 2]] }, Vec::new(), &mut out);
    assert!(host.window_frame(1).is_none(), "the window is not open yet");
    host.handle(ToShell::WindowOpen { id: 1, title: "New Message".into(), w: 2.0, h: 2.0, position: None, flags: 0 }, Vec::new(), &mut out);
    assert_eq!(host.window_frame(1).map(|(_, w, h)| (w, h)), Some((2, 2)), "the first frame is there once the window is");
    assert!(host.window_mapped(1));
    assert!(sent(&peer).is_empty(), "the app is not told to drop the window");
    // pixels for a window that never opens do not pile up, and a window the table refuses loses its pixels
    for id in 10..40 {
        host.handle(ToShell::WindowFrame { id, buf: 0, damage: Vec::new() }, Vec::new(), &mut out);
    }
    assert!(host.early.len() <= MAX_EARLY, "{} kept", host.early.len());
    host.handle(ToShell::WindowOpen { id: 39, title: "Late".into(), w: 2.0, h: 2.0, position: None, flags: 0 }, Vec::new(), &mut out);
    assert!(host.early.iter().all(|e| e.window() != 39), "what was kept for a window is used up when it opens");
    host.handle(ToShell::WindowClose { id: 1 }, Vec::new(), &mut out);
    assert!(host.window_frame(1).is_none());
}

/// Wave b2: a closed window's last frame came back. The app had pixels for the window in flight when the shell closed
/// it; they reached a window the table no longer held and were kept as "early" pixels (MA6) for the next window to
/// open under that id, which showed them as its first frame. A window the shell closed keeps nothing for its id.
#[test]
fn pixels_in_flight_for_a_window_the_shell_closed_are_not_the_first_frame_of_the_next_one() {
    let (channel, _peer) = sys::socketpair_pair().unwrap();
    let mut host = AppHost::with_channel("fixture", channel, None, cfg());
    let mut out = Pumped::default();
    let bytes = 2 * 2 * 4;
    let open = |host: &mut AppHost, out: &mut Pumped| host.handle(ToShell::WindowOpen { id: 5, title: "New Message".into(), w: 2.0, h: 2.0, position: None, flags: 0 }, Vec::new(), out);
    open(&mut host, &mut out);
    assert!(host.close_window_from_shell(5));
    // the app drew before it heard of the close
    let fds = vec![sys::shm_create("late-0", bytes).unwrap(), sys::shm_create("late-1", bytes).unwrap()];
    host.handle(ToShell::WindowBuffers { id: 5, w: 2, h: 2, generation: 1 }, fds, &mut out);
    host.handle(ToShell::WindowFrame { id: 5, buf: 0, damage: vec![[0, 0, 2, 2]] }, Vec::new(), &mut out);
    assert!(host.early.is_empty(), "nothing is kept for a window the shell closed: {} kept", host.early.len());
    // the id is used again by a new window: the old frame is not its first
    open(&mut host, &mut out);
    assert!(host.window_open(5));
    assert!(host.window_frame(5).is_none() && !host.window_mapped(5), "the new window has drawn nothing yet");
    // its own pixels are taken as usual
    let fds = vec![sys::shm_create("fresh-0", bytes).unwrap(), sys::shm_create("fresh-1", bytes).unwrap()];
    host.handle(ToShell::WindowBuffers { id: 5, w: 2, h: 2, generation: 2 }, fds, &mut out);
    host.handle(ToShell::WindowFrame { id: 5, buf: 0, damage: vec![[0, 0, 2, 2]] }, Vec::new(), &mut out);
    assert!(host.window_mapped(5));
    // a window the app closed itself is not one the shell closed: pixels that precede its next open are still kept
    host.handle(ToShell::WindowClose { id: 5 }, Vec::new(), &mut out);
    host.handle(ToShell::WindowFrame { id: 5, buf: 0, damage: Vec::new() }, Vec::new(), &mut out);
    assert_eq!(host.early.len(), 1);
}

#[test]
fn the_shell_closing_a_window_drops_it_and_tells_the_app_once() {
    let (channel, peer) = sys::socketpair_pair().unwrap();
    let mut host = AppHost::with_channel("fixture", channel, None, cfg());
    let mut out = Pumped::default();
    host.handle(ToShell::WindowOpen { id: 3, title: "Card".into(), w: 10.0, h: 10.0, position: None, flags: 0 }, Vec::new(), &mut out);
    assert!(host.configure_window(3, 20.0, 30.0));
    assert!(host.touch_window(3, 1, TouchKind::Down, 1.0, 2.0, 5, bifrost_proto::TouchSource::Finger, bifrost_proto::ModifierFlags::EMPTY));
    assert!(host.close_window_from_shell(3));
    assert!(!host.close_window_from_shell(3), "a window closes once");
    assert!(!host.touch_window(3, 1, TouchKind::Down, 1.0, 2.0, 6, bifrost_proto::TouchSource::Finger, bifrost_proto::ModifierFlags::EMPTY), "no touch reaches a closed window");
    assert!(host.windows.of_app("fixture").is_empty());
    assert_eq!(
        sent(&peer),
        vec![
            ToApp::WindowConfigure { id: 3, w: 20.0, h: 30.0 },
            ToApp::WindowTouch { id: 3, touch: 1, kind: TouchKind::Down, x: 1.0, y: 2.0, t_ms: 5, source: bifrost_proto::TouchSource::Finger, mods: bifrost_proto::ModifierFlags::EMPTY },
            ToApp::WindowClosed { id: 3 },
        ]
    );
}

/// E2E-3: a Shift-click or Cmd-click reaches the app with its modifiers, on the main window and on an extra window.
#[test]
fn a_press_reaches_the_app_with_the_modifier_keys_held() {
    use bifrost_proto::{ModifierFlags as F, TouchSource};
    let (channel, peer) = sys::socketpair_pair().unwrap();
    let mut host = AppHost::with_channel("fixture", channel, None, cfg());
    let mut out = Pumped::default();
    host.handle(ToShell::WindowOpen { id: 3, title: "Card".into(), w: 10.0, h: 10.0, position: None, flags: 0 }, Vec::new(), &mut out);
    host.touch(TouchKind::Down, 0, 4.0, 5.0, 7, TouchSource::Pointer, F::SHIFT);
    assert!(host.touch_window(3, 1, TouchKind::Down, 1.0, 2.0, 8, TouchSource::Finger, F::COMMAND));
    assert_eq!(
        sent(&peer),
        vec![
            ToApp::Touch { kind: TouchKind::Down, id: 0, x: 4.0, y: 5.0, t_ms: 7, source: TouchSource::Pointer, mods: F::SHIFT },
            ToApp::WindowTouch { id: 3, touch: 1, kind: TouchKind::Down, x: 1.0, y: 2.0, t_ms: 8, source: TouchSource::Finger, mods: F::COMMAND },
        ]
    );
}

/// E2E: the wheel over an extra window reaches that window with its id, only when the app asked for the wheel and
/// the window is open, and the drag emulated for it is then dropped.
#[test]
fn the_wheel_over_an_extra_window_carries_the_window_id() {
    use bifrost_ui::pointer_event::{PointerEvent, WHEEL};
    use bifrost_ui::wheel::Phase;
    let (channel, peer) = sys::socketpair_pair().unwrap();
    let mut host = AppHost::with_channel("fixture", channel, None, cfg());
    let mut out = Pumped::default();
    host.handle(ToShell::WindowOpen { id: 3, title: "Editor".into(), w: 10.0, h: 10.0, position: None, flags: 0 }, Vec::new(), &mut out);
    let wheel = PointerEvent::Wheel { x: 4.0, y: 5.0, dx: 0.0, dy: -20.0, precise: false, phase: Phase::None, momentum: Phase::None };
    assert!(!host.pointer_window(3, wheel), "an app that did not ask for the wheel keeps the emulated drag");
    host.handle(ToShell::PointerInterest { mask: WHEEL }, Vec::new(), &mut out);
    assert!(host.pointer_window(3, wheel));
    assert!(!host.pointer_window(9, wheel), "no window 9 is open");
    assert!(host.pointer_window(0, wheel), "window 0 is the main window");
    host.swallow_wheel_drag(100.0);
    assert!(host.swallows_touch(100.0) && host.swallows_touch(116.0) && host.swallows_touch(132.0));
    assert!(!host.swallows_touch(140.0), "only the three emulated touches are dropped");
    assert_eq!(
        sent(&peer),
        vec![ToApp::WindowInput { window: 3, input: WindowInput::Pointer(wheel) }, ToApp::Pointer(wheel)]
    );
}

/// A wheel notch an app did not ask for: the drag emulated for it still reaches the app, and only its three touches
/// count as a finger's (they pan on a desktop); the drag is forgotten once it is late.
#[test]
fn the_drag_emulated_for_a_wheel_notch_the_app_did_not_take_pans_like_a_finger() {
    let (channel, _peer) = sys::socketpair_pair().unwrap();
    let mut host = AppHost::with_channel("fixture", channel, None, cfg());
    assert!(!host.pans_touch(0.0), "a plain press is what the pointer made it");
    host.keep_wheel_drag(100.0);
    assert!(host.pans_touch(100.0) && host.pans_touch(116.0) && host.pans_touch(132.0));
    assert!(!host.pans_touch(140.0), "only the three emulated touches pan");
    host.keep_wheel_drag(200.0);
    assert!(!host.pans_touch(200.0 + WHEEL_DRAG_MS + 1.0), "a late touch is a press of its own");
    assert!(!host.pans_touch(500.0));
}

/// The custom actions an app names for its elements are kept cleaned, and a new list replaces the old one.
#[test]
fn custom_actions_are_kept_cleaned_and_replaced_by_the_next_list() {
    use bifrost_ui::ax_action::AxActionSet;
    let (channel, _peer) = sys::socketpair_pair().unwrap();
    let mut host = AppHost::with_channel("fixture", channel, None, cfg());
    let mut out = Pumped::default();
    host.handle(ToShell::AxActions(vec![AxActionSet::new(4, &["Archive", "archive", " "]), AxActionSet::new(5, &[])]), Vec::new(), &mut out);
    assert_eq!(host.ax_actions, vec![AxActionSet::new(4, &["Archive"])]);
    host.handle(ToShell::AxActions(Vec::new()), Vec::new(), &mut out);
    assert!(host.ax_actions.is_empty());
}
