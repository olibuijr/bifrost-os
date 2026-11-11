use super::*;
use crate::shell::tests::{desktop_shell, fixture_app};
use crate::shell::{AppId, Screen};

/// A desktop with the fixture app's window open behind another app, its buffers attached and the desktop settled.
fn behind_another_app() -> (crate::shell::Shell, Channel) {
    let mut sh = desktop_shell();
    sh.anim = None;
    let peer = fixture_app(&mut sh);
    let m = sh.desk_metrics();
    sh.desk.as_mut().unwrap().open(&m, AppId::External(0), (600, 400));
    // no installed app is in front (the desktop itself, or a built-in app such as Settings)
    sh.screen = Screen::Home;
    sh.ext_tick(1.0);
    let a = sys::shm_create("behind-a", 48).unwrap();
    let b = sys::shm_create("behind-b", 48).unwrap();
    peer.send(&ToShell::Buffers { w: 4, h: 3, generation: 1 }.encode(), &[a.0, b.0]).unwrap();
    sh.ext_tick(2.0);
    // the first buffers of an app open its window in front, one tick later (the fall-back of a direct app,
    // `take_fallback`), and the front app follows the frontmost window from then on: so a window of a built-in app
    // goes in front of it. Without this the fixture app was in front and no test here was about a window behind
    sh.ext_tick(2.5);
    sh.desk.as_mut().unwrap().front_intent = None;
    sh.desk.as_mut().unwrap().open(&m, AppId::Settings, (300, 200));
    sh.screen = Screen::App(AppId::Settings);
    sh.ext_tick(2.75);
    assert_eq!(sh.screen, Screen::App(AppId::Settings), "the fixture app stays behind");
    sh.dirty = false;
    sh.ext_frame = false;
    (sh, peer)
}

/// E2E7: Files, behind Settings, followed the 24-Hour Time switch 5 to 20 s after the menu bar. The shell told it in the
/// same tick, and it drew again at once, but the pixels of an app that is not in front asked for no frame.
#[test]
fn new_pixels_of_a_window_behind_the_front_app_ask_for_a_frame() {
    let (mut sh, peer) = behind_another_app();
    peer.send(&ToShell::Frame { buf: 0, damage: vec![[0, 0, 4, 3]] }.encode(), &[]).unwrap();
    sh.ext_tick(3.0);
    assert!(sh.ext_frame, "the window is on screen: its new pixels go up now");
    assert!(!sh.dirty, "as a frame of the windows alone");
}

#[test]
fn new_pixels_of_a_minimized_window_ask_for_none() {
    let (mut sh, peer) = behind_another_app();
    for w in sh.desk.as_mut().unwrap().wins.iter_mut().filter(|w| w.app.parent() == AppId::External(0)) {
        w.minimized = true;
    }
    peer.send(&ToShell::Frame { buf: 0, damage: vec![[0, 0, 4, 3]] }.encode(), &[]).unwrap();
    sh.ext_tick(3.0);
    let wins: Vec<String> = sh.desk.as_ref().unwrap().wins.iter().map(|w| format!("{:?} min={}", w.app, w.minimized)).collect();
    assert!(!sh.ext_frame, "nothing of it shows (dirty {}, screen {:?}, running {}, windows {wins:?})", sh.dirty, sh.screen, sh.ext[0].as_ref().is_some_and(|h| h.running()));
}

#[test]
fn a_background_frame_counts_only_for_a_window_the_desktop_shows() {
    assert!(!background_frame_shows(None, 0), "no desktop: only the front app is drawn");
    let mut sh = desktop_shell();
    let m = sh.desk_metrics();
    sh.desk.as_mut().unwrap().open(&m, AppId::External(2), (600, 400));
    assert!(background_frame_shows(sh.desk.as_ref(), 2));
    assert!(!background_frame_shows(sh.desk.as_ref(), 1), "another app's window");
}

/// The clock style and the zone go to every running app in the tick that changes them, in front or not: no app
/// waits for its turn in front, and nothing is sent again while they stay the same.
#[test]
fn a_new_clock_style_reaches_an_app_behind_the_front_one_in_the_same_tick() {
    let (mut sh, peer) = behind_another_app();
    let read = |peer: &Channel| {
        let mut seen = Vec::new();
        while let Ok(Recv::Packet(p, _)) = peer.recv(false) {
            if let Ok(ToApp::DateTime { flags, .. }) = ToApp::decode(&p) {
                seen.push(flags);
            }
        }
        seen
    };
    let _ = read(&peer);
    sh.clock_style.use_24h = !sh.clock_style.use_24h;
    let want = date_time_flags(sh.clock_style.use_24h);
    sh.ext_tick(3.0);
    assert_eq!(read(&peer), vec![want], "told at once, though it is not in front");
    sh.ext_tick(4.0);
    assert_eq!(read(&peer), Vec::<u32>::new(), "and not again");
}
