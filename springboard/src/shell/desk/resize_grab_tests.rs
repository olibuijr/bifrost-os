use super::*;

/// E2E-12: a corner drag resizes the window that was pressed, also when another window comes forward during the
/// drag (the window server handed the focus to Settings behind, and Settings took the drag's frame).
#[test]
fn a_resize_follows_the_pressed_window_when_another_comes_forward() {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.anim = None;
    sh.screen = crate::shell::Screen::Home;
    if sh.desk.is_none() { sh.desk = Some(Desk::new()); }
    let m = sh.desk_metrics();
    let d = sh.desk.as_mut().unwrap();
    d.open(&m, AppId::Settings, (1000, 900));
    d.open(&m, AppId::Photos, (900, 700));
    let settings_before = { let r = d.wins[d.index_of(AppId::Settings).unwrap()].r; (r.x, r.y, r.w, r.h) };
    let p = d.wins[d.index_of(AppId::Photos).unwrap()].r;
    let (x, y) = ((p.x + p.w - 1) as f32, (p.y + p.h - 1) as f32);
    sh.desk_event(Event::Down(x, y), 0.);
    let d = sh.desk.as_mut().unwrap();
    let s = d.index_of(AppId::Settings).unwrap();
    d.raise(s);
    sh.desk_event(Event::Move(x - 40., y - 30.), 16.);
    sh.desk_event(Event::Up(x - 40., y - 30.), 32.);
    let d = sh.desk.as_ref().unwrap();
    let r = d.wins[d.index_of(AppId::Settings).unwrap()].r;
    assert_eq!((r.x, r.y, r.w, r.h), settings_before, "the window behind is not resized");
    let r = d.wins[d.index_of(AppId::Photos).unwrap()].r;
    assert_eq!((r.w, r.h), (p.w - 40, p.h - 30), "the pressed window follows the pointer");
}

/// N1: the drag the Wayland pointer emulates for a wheel notch over a window's edge (the bottom one on Midget; every
/// edge alike) leaves the window's size alone; a real press there, after the wheel, still resizes it.
#[test]
fn the_wheel_never_starts_or_drives_a_resize() {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.anim = None;
    sh.screen = crate::shell::Screen::Home;
    if sh.desk.is_none() { sh.desk = Some(Desk::new()); }
    let m = sh.desk_metrics();
    let d = sh.desk.as_mut().unwrap();
    d.open(&m, AppId::Photos, (900, 700));
    let p = d.wins[d.index_of(AppId::Photos).unwrap()].r;
    let (x, y) = ((p.x + p.w - 1) as f32, (p.y + p.h / 2) as f32);
    assert!(matches!(sh.desk.as_ref().unwrap().pick(&m, x, y), Some((_, Hit::Edge(_)))), "the press is on the right edge");
    for n in 0..15 {
        let t = n as f64 * 100.;
        sh.desk_wheel(t);
        sh.desk_event(Event::Down(x, y), t);
        sh.desk_event(Event::Move(x - 9., y), t + 16.);
        sh.desk_event(Event::Up(x - 9., y), t + 32.);
    }
    let d = sh.desk.as_ref().unwrap();
    let r = d.wins[d.index_of(AppId::Photos).unwrap()].r;
    assert_eq!((r.w, r.h), (p.w, p.h), "fifteen wheel notches leave the window's size alone");
    sh.desk_event(Event::Down(x, y), 5000.);
    sh.desk_event(Event::Move(x - 30., y), 5016.);
    sh.desk_event(Event::Up(x - 30., y), 5032.);
    let d = sh.desk.as_ref().unwrap();
    let r = d.wins[d.index_of(AppId::Photos).unwrap()].r;
    assert_eq!(r.w, p.w - 30, "a press long after the wheel still resizes");
}

/// A wheel notch Settings took drops its emulated drag in `Shell::event`, so the desk never sees that press: the
/// mark must not outlive it, or the real title-bar press that follows within 250 ms is taken for the wheel's.
#[test]
fn a_title_bar_press_right_after_a_notch_over_settings_drags_the_window() {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.anim = None;
    sh.screen = crate::shell::Screen::Home;
    if sh.desk.is_none() { sh.desk = Some(Desk::new()); }
    let m = sh.desk_metrics();
    let d = sh.desk.as_mut().unwrap();
    d.open(&m, AppId::Settings, (1000, 800));
    let s = d.wins[d.index_of(AppId::Settings).unwrap()].r;
    let (x, y) = ((s.x + s.w / 2) as f32, (s.y + m.title / 2) as f32);
    assert!(matches!(sh.desk.as_ref().unwrap().pick(&m, x, y), Some((_, Hit::Title))), "the press is on the title bar");
    // the notch, as bin/shell.rs routes it: the desk is told, then Settings takes it (`settings_wheel`)
    sh.desk_wheel(1000.);
    sh.swallow_wheel_drag(1000.);
    let (cx, cy) = ((s.x + s.w / 2) as f32, (s.y + s.h / 2) as f32);
    assert!(sh.event(Event::Down(cx, cy), 1000.).is_empty());
    assert!(sh.event(Event::Move(cx, cy - 9.), 1016.).is_empty());
    assert!(sh.event(Event::Up(cx, cy - 9.), 1032.).is_empty());
    // a real press 100 ms later, well inside the wheel's 250 ms
    sh.desk_event(Event::Down(x, y), 1100.);
    sh.desk_event(Event::Move(x + 40., y + 30.), 1116.);
    sh.desk_event(Event::Up(x + 40., y + 30.), 1132.);
    let d = sh.desk.as_ref().unwrap();
    let r = d.wins[d.index_of(AppId::Settings).unwrap()].r;
    assert_eq!((r.x, r.y), (s.x + 40, s.y + 30), "the title-bar press drags the window");
    assert_eq!((r.w, r.h), (s.w, s.h));
}

/// E2E-12: Settings never goes below the width of its sidebar layout, whether by a drag, the window server's size or
/// the size it is reopened at.
#[test]
fn settings_keeps_its_split_layout_width() {
    let m = Metrics { w: 2560, h: 1440, title: 38, menubar: 35, min_w: 400, min_h: 275, edge: 8, light: 18, light_gap: 10, light_x: 15, dock_left: 0, dock_right: 0, dock_bottom: 120, pt: 1.25 };
    let (min_w, _, _, _) = size_range(AppId::Settings, &m);
    assert!(min_w as f32 >= super::super::settings::SPLIT_MIN_PT * m.pt, "{min_w} px is the split layout");
    assert_eq!(clamp_size(AppId::Settings, &m, (400, 549)).0, min_w, "the 400x549 of E2E-12 is held to the minimum");
    let mut d = Desk::new();
    d.open(&m, AppId::Settings, (923, 860));
    let start = d.wins[0].r;
    d.resize_to(&m, 0, R | B, start, -800, -500);
    assert_eq!(d.wins[0].r.w, min_w);
    assert!(clamp_size(AppId::Photos, &m, (100, 100)) == (m.min_w, m.min_h + m.title), "other windows keep the desktop minimum");
}
