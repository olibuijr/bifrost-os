use super::{win_id, FRONT_INTENT_MS};
use crate::bare::wayland::WindowEvent;
use crate::shell::tests::{desktop_shell, fixture_app};
use crate::shell::{AppId, Screen, Shell};

fn front(sh: &Shell) -> (Option<AppId>, Screen) {
    (sh.desk.as_ref().unwrap().front().map(|w| w.app), sh.screen)
}

fn id(app: AppId) -> u32 {
    win_id(app).unwrap()
}

/// Settings in front, Calculator launched: the window server's own focus of Settings (its keyboard enter or an
/// activating configure, not only the hand-back as Search goes) does not lift Settings over the launch.
#[test]
fn a_launch_over_a_built_in_stays_in_front_through_the_window_servers_focus() {
    let mut sh = desktop_shell();
    sh.screen = Screen::Home;
    sh.anim = None;
    let _peer = fixture_app(&mut sh);
    sh.desk_open(AppId::Settings, 0.0);
    sh.desk_open(AppId::External(0), 10.0);
    let want = (Some(AppId::External(0)), Screen::App(AppId::External(0)));
    sh.wayland_window_event(WindowEvent::HandBack(id(AppId::Settings)));
    assert_eq!(front(&sh), want, "the hand-back as Spotlight's keyboard goes");
    sh.wayland_window_event(WindowEvent::ServerFocus(id(AppId::Settings)));
    assert_eq!(front(&sh), want, "a later keyboard enter of the window that was in front");
    sh.desk_intent_tick(10.0 + FRONT_INTENT_MS / 2.0);
    assert_eq!(sh.desk_front_intent(), Some(AppId::External(0)), "the intent holds within its bound");
    sh.desk_intent_tick(10.0 + FRONT_INTENT_MS + 1.0);
    assert_eq!(sh.desk_front_intent(), None, "and ends after it");
    sh.wayland_window_event(WindowEvent::ServerFocus(id(AppId::Settings)));
    assert_eq!(front(&sh), (Some(AppId::Settings), Screen::App(AppId::Settings)), "the window server's later focus is followed again");
}

/// Weather (an external app's window) in front, Settings opened from Search: the keyboard the window server hands
/// back to Weather does not put Weather over Settings, and Settings, drawn already, is activated again.
#[test]
fn a_built_in_opened_over_an_external_app_stays_in_front_and_is_activated_again() {
    let mut sh = desktop_shell();
    sh.screen = Screen::Home;
    sh.anim = None;
    let _peer = fixture_app(&mut sh);
    let m = sh.desk_metrics();
    let weather = AppId::ExternalWindow(0, 3);
    sh.desk.as_mut().unwrap().open(&m, AppId::Settings, (900, 700));
    sh.desk.as_mut().unwrap().open(&m, weather, (600, 400));
    sh.screen = Screen::App(AppId::External(0));
    sh.desk_open(AppId::Settings, 5.0);
    assert!(sh.take_raise_front(), "the open asks for the activation");
    let want = (Some(AppId::Settings), Screen::App(AppId::Settings));
    sh.wayland_window_event(WindowEvent::HandBack(id(weather)));
    assert_eq!(front(&sh), want);
    assert!(sh.take_raise_front(), "the keyboard went to Weather: Settings is activated again");
    sh.wayland_window_event(WindowEvent::ServerFocus(id(weather)));
    assert_eq!(front(&sh), want);
    assert!(sh.take_raise_front(), "each focus the window server gives elsewhere asks again");
    sh.wayland_window_event(WindowEvent::ServerFocus(id(AppId::Settings)));
    assert_eq!(front(&sh), want, "the focus of the intended window itself is followed");
    assert!(!sh.take_raise_front(), "and asks for nothing more");
}

/// Weather in front, Calendar launched: Mail, a third app's window the window server picks as its next toplevel
/// while Calendar has none yet, does not come to the front, nor does Weather after it.
#[test]
fn a_launch_over_an_external_app_ignores_the_window_servers_next_toplevel() {
    let mut sh = desktop_shell();
    sh.screen = Screen::Home;
    sh.anim = None;
    let _weather_peer = fixture_app(&mut sh);
    let _calendar_peer = fixture_app(&mut sh);
    let m = sh.desk_metrics();
    let (mail, weather) = (AppId::Terminal, AppId::ExternalWindow(0, 3));
    sh.desk.as_mut().unwrap().open(&m, mail, (900, 600));
    sh.desk.as_mut().unwrap().open(&m, weather, (600, 400));
    sh.desk_open(AppId::External(1), 20.0);
    let want = (Some(AppId::External(1)), Screen::App(AppId::External(1)));
    sh.wayland_window_event(WindowEvent::ServerFocus(id(mail)));
    assert_eq!(front(&sh), want, "the window server's topmost pick");
    sh.wayland_window_event(WindowEvent::HandBack(id(weather)));
    assert_eq!(front(&sh), want, "the hand-back to the window that was in front");
    let d = sh.desk.as_ref().unwrap();
    assert_eq!(d.wins.iter().rev().nth(1).map(|w| w.app), Some(weather), "the stack below is left as it was");
}

/// The per-tick refocus never takes the screen from the app being brought to the front, and a press (the person's
/// choice) ends the intent at once.
#[test]
fn the_per_tick_refocus_keeps_the_intended_app_and_a_press_ends_it() {
    let mut sh = desktop_shell();
    sh.screen = Screen::Home;
    sh.anim = None;
    let _peer = fixture_app(&mut sh);
    sh.desk_open(AppId::Settings, 0.0);
    sh.desk_open(AppId::External(0), 1.0);
    sh.screen = Screen::App(AppId::Settings);
    sh.desk_sync_extra_windows();
    assert_eq!(sh.screen, Screen::App(AppId::External(0)), "the tick puts the intended app back");
    sh.wayland_window_event(WindowEvent::ServerFocus(id(AppId::Settings)));
    sh.desk_intent_tick(2.0);
    sh.desk_sync_extra_windows();
    assert_eq!(front(&sh), (Some(AppId::External(0)), Screen::App(AppId::External(0))));
    sh.wayland_window_event(WindowEvent::Focus(id(AppId::Settings)));
    assert_eq!(front(&sh), (Some(AppId::Settings), Screen::App(AppId::Settings)), "a press raises the window below");
    assert_eq!(sh.desk_front_intent(), None);
    assert!(sh.desk.as_ref().unwrap().front_intent.is_none());
}

/// The window server reports the intended window's toplevel but another one active: the shell asks again, once per
/// report, and stops once it is active.
#[test]
fn an_inactive_intended_toplevel_is_activated_again() {
    let mut sh = desktop_shell();
    sh.screen = Screen::Home;
    sh.anim = None;
    sh.desk_open(AppId::Settings, 0.0);
    sh.take_raise_front();
    let toplevel = |active| crate::bare::wayland::ForeignWindow { id: 40, title: "Settings".into(), app_id: format!("is.olibuijr.bifrost.{}", id(AppId::Settings)), active, maximized: false, minimized: false };
    let d = sh.desk.as_mut().unwrap();
    d.foreign_snapshot = vec![toplevel(false)];
    d.intent_check = true;
    sh.desk_intent_tick(100.0);
    assert!(sh.take_raise_front(), "mapped but not active: activated again");
    sh.desk_intent_tick(110.0);
    assert!(!sh.take_raise_front(), "once per report");
    let d = sh.desk.as_mut().unwrap();
    d.foreign_snapshot = vec![toplevel(true)];
    d.intent_check = true;
    sh.desk_intent_tick(120.0);
    assert!(!sh.take_raise_front(), "active: nothing to ask");
}
