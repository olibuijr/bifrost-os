use super::*;

fn two_windows() -> (crate::shell::Shell, Metrics) {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.anim = None;
    sh.screen = crate::shell::Screen::Home;
    if sh.desk.is_none() { sh.desk = Some(Desk::new()); }
    let m = sh.desk_metrics();
    let d = sh.desk.as_mut().unwrap();
    d.open(&m, AppId::Settings, (1000, 900));
    d.open(&m, AppId::Photos, (900, 700));
    (sh, m)
}

/// A point on `light` of `app`'s window that the desk hit-tests as that light.
fn light_at(d: &Desk, m: &Metrics, app: AppId, light: Light) -> (f32, f32) {
    let i = d.index_of(app).unwrap();
    let r = d.wins[i].r;
    for y in r.y..r.y + 80 {
        for x in r.x..r.x + 200 {
            if d.pick(m, x as f32, y as f32) == Some((i, Hit::Light(light))) {
                return (x as f32, y as f32);
            }
        }
    }
    panic!("{app:?} has no {light:?} button in front");
}

/// W3-1: a traffic light acts on the window that was pressed, also when the stack changes before the release (the
/// window server handing the focus to another window), as the move and resize grabs do.
#[test]
fn a_traffic_light_acts_on_the_pressed_window_when_another_comes_forward() {
    let (mut sh, m) = two_windows();
    let (x, y) = light_at(sh.desk.as_ref().unwrap(), &m, AppId::Photos, Light::Zoom);
    sh.desk_event(Event::Down(x, y), 0.);
    let d = sh.desk.as_mut().unwrap();
    let s = d.index_of(AppId::Settings).unwrap();
    d.raise(s);
    sh.desk_event(Event::Up(x, y), 16.);
    let d = sh.desk.as_ref().unwrap();
    assert!(d.wins[d.index_of(AppId::Photos).unwrap()].maximized, "the pressed window zooms");
    assert!(!d.wins[d.index_of(AppId::Settings).unwrap()].maximized, "the window that came forward does not");
}

/// W3-6: Bring All to Front raises the active app's shown windows and leaves minimized ones in the Dock.
#[test]
fn bring_all_to_front_raises_the_app_and_never_unminimizes() {
    let (mut sh, m) = two_windows();
    let d = sh.desk.as_mut().unwrap();
    d.open(&m, AppId::Files, (700, 500));
    let f = d.index_of(AppId::Files).unwrap();
    d.wins[f].minimized = true;
    sh.screen = crate::shell::Screen::App(AppId::Settings);
    sh.desk_bring_all_to_front();
    let d = sh.desk.as_ref().unwrap();
    assert!(d.wins[d.index_of(AppId::Files).unwrap()].minimized, "a minimized window stays in the Dock");
    assert_eq!(d.front().map(|w| w.app), Some(AppId::Settings), "the active app's window comes to the front");
    assert!(!d.wins[d.index_of(AppId::Photos).unwrap()].minimized);
}

/// W3-6: a window filled, taken full screen and back keeps the frame from before the Fill as its way back.
#[test]
fn full_screen_from_a_fill_keeps_the_frame_from_before_the_fill() {
    let m = Metrics { w: 2560, h: 1440, title: 38, menubar: 35, min_w: 400, min_h: 275, edge: 8, light: 18, light_gap: 10, light_x: 15, dock_left: 0, dock_right: 0, dock_bottom: 120, pt: 1.25 };
    let mut d = Desk::new();
    d.open(&m, AppId::Photos, (923, 860));
    let before = d.wins[0].r;
    d.toggle_zoom(&m, 0);
    assert!(d.set_full_screen(&m, 0, true));
    assert!(d.set_full_screen(&m, 0, false));
    assert!(d.wins[0].maximized, "it comes back filled");
    assert_eq!(d.wins[0].r, Desk::area(&m));
    d.return_to_previous(0);
    assert_eq!(d.wins[0].r, before, "Return to Previous Size goes back to the frame from before the Fill");
}

/// W3-6: the window server's own full screen (Ctrl+Cmd+F) on a filled window: it reports the full-screen state
/// before the maximize state (`bare::wayland` configure), so the shell keeps its zoom and the frame from before it,
/// and the un-maximized configure of a full-screen window is not taken as an un-zoom.
#[cfg(feature = "desktop-wayland")]
#[test]
fn the_window_servers_full_screen_keeps_the_zoom_and_its_way_back() {
    use crate::bare::wayland::WindowEvent;
    let mut sh = crate::shell::tests::desktop_shell();
    sh.anim = None;
    sh.screen = crate::shell::Screen::Home;
    sh.desk_open(AppId::Settings, 0.);
    let id = win_id(AppId::Settings).unwrap();
    let m = sh.desk_metrics();
    let i = sh.desk.as_ref().unwrap().index_of(AppId::Settings).unwrap();
    let before = sh.desk.as_ref().unwrap().wins[i].r;
    sh.desk.as_mut().unwrap().toggle_zoom(&m, i);
    sh.wayland_window_event(WindowEvent::Maximized(id, true));
    sh.wayland_window_event(WindowEvent::Fullscreen(id, true));
    sh.wayland_window_event(WindowEvent::Maximized(id, false));
    assert!(sh.desk.as_ref().unwrap().wins[i].full);
    sh.wayland_window_event(WindowEvent::Fullscreen(id, false));
    sh.wayland_window_event(WindowEvent::Maximized(id, true));
    let w = sh.desk.as_ref().unwrap().wins[i];
    assert!(!w.full && w.maximized, "back filled");
    sh.desk.as_mut().unwrap().return_to_previous(i);
    assert_eq!(sh.desk.as_ref().unwrap().wins[i].r, before, "the frame from before the Fill, not the full-screen one");
}

#[cfg(feature = "desktop-wayland")]
fn arrange_menu_open(sh: &mut crate::shell::Shell) -> (f32, f32) {
    let m = sh.desk_metrics();
    let (gx, gy) = light_at(sh.desk.as_ref().unwrap(), &m, AppId::Settings, Light::Zoom);
    sh.desktop_pointer(gx, gy, false, 1000.);
    sh.arrange_tick(1000. + ARRANGE_DWELL_MS);
    assert_eq!(sh.desktop_menu, Some(super::super::desktop_bar::Menu::Arrange));
    (gx, gy)
}

/// W3-5: the pointer crossing from the window onto the menu's own surface makes the window server send a Leave
/// (`Hover(-100, -100)`) before the menu's first motion; the menu stays open for it and closes only on a motion
/// that is outside the button, the bridge and the menu.
#[cfg(feature = "desktop-wayland")]
#[test]
fn the_green_button_menu_survives_the_pointer_crossing_onto_its_surface() {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.screen = crate::shell::Screen::Home;
    sh.anim = None;
    sh.desk_open(AppId::Settings, 0.);
    arrange_menu_open(&mut sh);
    sh.desktop_pointer(-100., -100., false, 1600.);
    assert_eq!(sh.desktop_menu, Some(super::super::desktop_bar::Menu::Arrange), "a Leave is not a move away");
    let (px, py, pw, ph) = sh.desktop_popup_rect().unwrap();
    sh.desktop_pointer((px + pw / 2) as f32, (py + ph / 2) as f32, false, 1650.);
    assert_eq!(sh.desktop_menu, Some(super::super::desktop_bar::Menu::Arrange), "the pointer is on the menu");
    sh.desktop_pointer(sh.w as f32 - 4., sh.h as f32 - 4., false, 1700.);
    assert!(sh.desktop_menu.is_none(), "a motion away from it closes it");
}

/// W3-5: a menu closed by a click on its button opens again when the pointer moves on the button and rests there.
#[cfg(feature = "desktop-wayland")]
#[test]
fn resting_on_the_green_button_again_reopens_a_menu_closed_by_a_click() {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.screen = crate::shell::Screen::Home;
    sh.anim = None;
    sh.desk_open(AppId::Settings, 0.);
    let (gx, gy) = arrange_menu_open(&mut sh);
    assert!(sh.desktop_bar_event(Event::Down(gx, gy), 2000.).is_none());
    assert!(sh.desktop_menu.is_none());
    sh.arrange_tick(2000. + 2. * ARRANGE_DWELL_MS);
    assert!(sh.desktop_menu.is_none(), "not again without the pointer moving");
    sh.desktop_pointer(gx, gy, false, 3000.);
    sh.arrange_tick(3000. + ARRANGE_DWELL_MS);
    assert_eq!(sh.desktop_menu, Some(super::super::desktop_bar::Menu::Arrange), "a new rest opens it again");
}
