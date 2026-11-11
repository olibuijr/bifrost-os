use super::*;
use bifrost_proto::{sys::Recv, ToApp, ToShell, TouchKind};

/// A desktop with the fixture app's main window in front and its extra window 1 beside it, not overlapping.
fn two_windows() -> (Shell, bifrost_proto::sys::Channel, AppId) {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.anim = None;
    let peer = crate::shell::tests::fixture_app(&mut sh);
    let (main, extra) = (AppId::External(0), AppId::ExternalWindow(0, 1));
    let m = sh.desk_metrics();
    let mut out = apphost::Pumped::default();
    sh.ext[0].as_mut().unwrap().handle(ToShell::WindowOpen { id: 1, title: "Second".into(), w: 300., h: 200., position: None, flags: 0 }, Vec::new(), &mut out);
    let d = sh.desk.as_mut().unwrap();
    d.wins.clear();
    d.open(&m, extra, (300, 200));
    d.open(&m, main, (300, 200));
    let area = Desk::area(&m);
    for (app, x) in [(extra, area.x + 20), (main, area.x + 420)] {
        let i = d.index_of(app).unwrap();
        (d.wins[i].r.x, d.wins[i].r.y) = (x, area.y + 40);
    }
    let front = d.index_of(main).unwrap();
    d.raise(front);
    sh.screen = Screen::App(main);
    sh.sync_key_windows();
    (sh, peer, extra)
}

/// What the app heard about its windows' focus and touches since the last call.
fn heard(peer: &bifrost_proto::sys::Channel) -> Vec<ToApp> {
    let mut out = Vec::new();
    while let Ok(Recv::Packet(p, _)) = peer.recv(false) {
        match ToApp::decode(&p) {
            Ok(m @ (ToApp::WindowFocus { .. } | ToApp::WindowTouch { .. } | ToApp::Touch { .. })) => out.push(m),
            _ => {}
        }
    }
    out
}

/// E2E7: the first click on a window that is not active must act in it (a toolbar's back button), not only bring it
/// forward. The shell raises the window, makes it its app's key window and then delivers the press and the release.
#[test]
fn the_click_that_activates_a_window_reaches_it_as_a_press_and_a_release() {
    let (mut sh, peer, extra) = two_windows();
    let _ = heard(&peer);
    let m = sh.desk_metrics();
    let b = sh.desk.as_ref().unwrap().wins.iter().find(|w| w.app == extra).unwrap().body(&m);
    // near the window's left edge: clear of the other window, whatever width the desk gave this one
    let (x, y) = ((b.x + 30) as f32, (b.y + b.h / 2) as f32);
    assert!(sh.desk_event(Event::Down(x, y), 1000.).is_some());
    assert!(sh.desk_event(Event::Up(x, y), 1060.).is_some());
    assert_eq!(sh.desk.as_ref().unwrap().front().map(|w| w.app), Some(extra), "the pressed window came forward");
    let got = heard(&peer);
    assert_eq!(got.len(), 4, "{got:?}");
    assert_eq!(got[0], ToApp::WindowFocus { window: 0, focused: false });
    assert_eq!(got[1], ToApp::WindowFocus { window: 1, focused: true }, "it is the key window before its press arrives");
    assert!(matches!(got[2], ToApp::WindowTouch { id: 1, kind: TouchKind::Down, .. }), "the activating press is delivered: {:?}", got[2]);
    assert!(matches!(got[3], ToApp::WindowTouch { id: 1, kind: TouchKind::Up, .. }), "and its release: {:?}", got[3]);
    assert_eq!(sh.ext[0].as_ref().unwrap().pointer_window, 1, "a menu the app opens on this press is this window's");
}

/// A press on the title bar is the shell's (it moves the window): the app hears no touch for it.
#[test]
fn a_press_on_the_title_bar_of_an_inactive_window_only_brings_it_forward() {
    let (mut sh, peer, extra) = two_windows();
    let _ = heard(&peer);
    let m = sh.desk_metrics();
    let r = sh.desk.as_ref().unwrap().wins.iter().find(|w| w.app == extra).unwrap().r;
    // past the traffic lights, and clear of the other window
    let (x, y) = ((r.x + 250) as f32, (r.y + m.title / 2) as f32);
    sh.desk_event(Event::Down(x, y), 1000.);
    sh.desk_event(Event::Up(x, y), 1060.);
    assert_eq!(sh.desk.as_ref().unwrap().front().map(|w| w.app), Some(extra));
    assert!(heard(&peer).iter().all(|m| !matches!(m, ToApp::WindowTouch { .. } | ToApp::Touch { .. })), "no touch for a title bar press");
}
