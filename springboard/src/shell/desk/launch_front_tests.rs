use crate::shell::tests::{desktop_shell, fixture_app, hand_back_after_launch};
use crate::shell::{AppId, Screen};

/// E2E6 1: a Dock click on an app that is not running yet opens it in front, and the keyboard the window server
/// hands back to the window it still knows in front does not put that window over the launching app.
#[test]
fn a_dock_launch_stays_in_front_through_a_keyboard_hand_back() {
    let mut sh = desktop_shell();
    sh.screen = Screen::Home;
    sh.anim = None;
    let _peer = fixture_app(&mut sh);
    sh.desk_open(AppId::Settings, 0.0);
    sh.desk_dock_tap(AppId::External(0), 1.0);
    assert_eq!(hand_back_after_launch(&mut sh), (Some(AppId::External(0)), Screen::App(AppId::External(0))));
}

/// The guard holds only while every window above is still launching: a drawn window above (a built-in) leaves the
/// hand-back as it was, a raise of the window it names.
#[test]
fn a_hand_back_with_a_drawn_window_above_still_raises() {
    let mut sh = desktop_shell();
    sh.screen = Screen::Home;
    sh.anim = None;
    let _peer = fixture_app(&mut sh);
    sh.desk_open(AppId::Settings, 0.0);
    sh.desk_open(AppId::External(0), 1.0);
    let m = sh.desk_metrics();
    sh.desk.as_mut().unwrap().open(&m, AppId::Magnifier, (600, 400));
    let i = sh.desk.as_ref().unwrap().index_of(AppId::Settings).unwrap();
    assert!(!sh.desk_launching_above(i), "the built-in above draws its own content");
    assert_eq!(hand_back_after_launch(&mut sh), (Some(AppId::Settings), Screen::App(AppId::Settings)));
}
