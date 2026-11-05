use super::{reveal_band, revealed_win, win_id, Grab, Light, Reveal, UNIFIED_BAR_PT};
use crate::bare::wayland::WindowEvent;
use crate::shell::tests::desktop_shell;
use crate::shell::{chrome_ids, AppId, Event, Screen, Shell};

fn full_screen_settings() -> Shell {
    let mut sh = desktop_shell();
    sh.screen = Screen::Home;
    sh.anim = None;
    sh.desk_open(AppId::Settings, 0.0);
    sh.wayland_window_event(WindowEvent::Fullscreen(win_id(AppId::Settings).unwrap(), true));
    assert!(sh.desk.as_ref().unwrap().front_full());
    sh
}

#[test]
fn the_toolbar_comes_down_with_the_menu_bar_and_keeps_it_while_the_pointer_is_on_it() {
    let mut sh = full_screen_settings();
    let bar = sh.desktop_bar_height() as f32;
    assert!(sh.desk_reveal().is_none(), "bar hidden: the toolbar is at the top with no lights");
    sh.desktop_pointer(300.0, 2.0, false, 10.0);
    let (i, rv) = sh.desk_reveal().expect("the bar is back, with the toolbar under it");
    let m = sh.desk_metrics();
    assert_eq!(rv, Reveal { top: bar as i32, band: (UNIFIED_BAR_PT * m.pt).round() as i32 }, "directly under the bar");
    assert_eq!(rv.band, reveal_band(&sh.desk.as_ref().unwrap().wins[i], &m));
    // down from the bar onto the revealed toolbar: both stay
    sh.desktop_pointer(300.0, bar + rv.band as f32 / 2.0, false, 20.0);
    assert!(!sh.desk_bar_hidden() && sh.desk_reveal().is_some(), "on the toolbar the bar stays");
    // past the toolbar: both go
    sh.desktop_pointer(300.0, bar + rv.band as f32 + 40.0, false, 30.0);
    assert!(sh.desk_bar_hidden() && sh.desk_reveal().is_none());
    // up from below onto the toolbar's rows: only the top edge brings the bar back
    sh.desktop_pointer(300.0, bar + rv.band as f32 / 2.0, false, 40.0);
    assert!(sh.desk_bar_hidden() && sh.desk_reveal().is_none());
}

#[test]
fn the_pointer_leaving_the_screen_does_not_latch_the_bar_for_a_later_move_into_the_band() {
    let mut sh = full_screen_settings();
    let bar = sh.desktop_bar_height() as f32;
    let band = sh.desk_full_band().unwrap().1 as f32;
    // the window server's pointer-leave hover
    sh.desktop_pointer(-100.0, -100.0, false, 10.0);
    assert!(sh.desk_bar_hidden() && sh.desk_reveal().is_none(), "leaving is not the top edge");
    // back onto the toolbar's rows without reaching the top edge: the bar stays hidden
    sh.desktop_pointer(300.0, bar + band / 2.0, false, 20.0);
    assert!(sh.desk_bar_hidden() && sh.desk_reveal().is_none(), "only the top edge brings the bar back");
    // a leave while the bar is shown leaves it as it was
    sh.desktop_pointer(300.0, 2.0, false, 30.0);
    sh.desktop_pointer(-100.0, -100.0, false, 40.0);
    assert!(!sh.desk_bar_hidden() && sh.desk_reveal().is_some());
}

#[test]
fn a_bar_kept_by_an_open_menu_or_a_recording_reveals_no_toolbar_under_it() {
    let mut sh = full_screen_settings();
    let (w, h) = (sh.w as usize, sh.h as usize);
    // a Dock menu opened with the pointer far below the top: the bar comes back, the toolbar stays at the top
    sh.desktop_pointer(300.0, (h / 2) as f32, false, 10.0);
    sh.desktop_menu = Some(crate::shell::desktop_bar::Menu::Dock);
    assert!(!sh.desk_bar_hidden(), "the open menu keeps the bar");
    assert!(sh.desk_reveal().is_none(), "but the pointer did not reach the top edge: no band");
    let px = vec![0u32; w * h];
    assert!(sh.desktop_chrome_frames(&px, w, h).iter().all(|f| f.id != chrome_ids::REVEAL));
    sh.desktop_menu = None;
    assert!(sh.desk_bar_hidden());
    // a screen recording keeps the bar for its indicator, again without the band (so app frames can draw the
    // windows alone, `ext_windows_only_ok`)
    let dir = crate::shell::photos::library::tests::scratch("reveal-recording");
    let area = crate::shell::screenrec::Area { x: 0, y: 0, w: 4, h: 2 };
    sh.rec = Some(crate::shell::screenrec::Session::start(dir.join("a.avi"), crate::shell::screenrec::Encoder::Avi, area, 10, 0.0).unwrap());
    assert!(!sh.desk_bar_hidden(), "the recording keeps the bar");
    assert!(sh.desk_reveal().is_none(), "but reveals no toolbar under it");
    // the pointer rule still reveals it
    sh.desktop_pointer(300.0, 2.0, false, 20.0);
    assert!(sh.desk_reveal().is_some(), "the top edge reveals the toolbar, recording or not");
    if let Some(r) = sh.rec.take() {
        let _ = r.finish();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_revealed_toolbar_ends_in_the_separator_hairline() {
    let mut sh = full_screen_settings();
    sh.desktop_pointer(300.0, 2.0, false, 10.0);
    let (_, rv) = sh.desk_reveal().unwrap();
    let (w, h) = (sh.w as usize, sh.h as usize);
    let mut px = vec![0u32; w * h];
    let mut s = crate::gfx::Surface { w, h, stride: w, px: &mut px, y0: 0 };
    sh.draw_desk_reveal(&mut s);
    let sep = crate::shell::ui::SEPARATOR_C.now();
    let last = (rv.top + rv.band - 1) as usize;
    assert!([w / 2, w - 1].iter().all(|&x| px[last * w + x] == sep), "a hairline across the band's last row");
    assert_ne!(px[(last - 1) * w + w / 2], sep, "on that row only");
}

#[test]
fn the_revealed_toolbar_is_a_surface_of_its_own_under_the_bar_and_goes_with_it() {
    let mut sh = full_screen_settings();
    sh.desktop_pointer(300.0, 2.0, false, 10.0);
    let (_, rv) = sh.desk_reveal().unwrap();
    let covered = sh.desk_regions_now().unwrap().iter().any(|&(x, y, w, h)| x <= 0 && y <= rv.top && x + w >= sh.w && y + h >= rv.top + rv.band);
    assert!(covered, "the band is redrawn with the bar");
    let (w, h) = (sh.w as usize, sh.h as usize);
    let px = vec![0u32; w * h];
    let f = sh.desktop_chrome_frames(&px, w, h).into_iter().find(|f| f.id == chrome_ids::REVEAL).expect("a surface for the band");
    assert_eq!((f.x, f.y, f.width, f.height), (0, rv.top, w, rv.band as usize));
    sh.desktop_pointer(300.0, (h / 2) as f32, false, 20.0);
    assert!(sh.desktop_chrome_frames(&px, w, h).iter().all(|f| f.id != chrome_ids::REVEAL), "no band once the bar hides");
}

#[test]
fn a_press_on_the_revealed_toolbar_reaches_the_app_at_the_row_it_shows_and_the_green_light_leaves_full_screen() {
    let mut sh = full_screen_settings();
    sh.desktop_pointer(300.0, 2.0, false, 10.0);
    let (i, rv) = sh.desk_reveal().unwrap();
    let m = sh.desk_metrics();
    let w = sh.desk.as_ref().unwrap().wins[i];
    let (x, y) = ((m.w / 2) as f32, (rv.top + rv.band / 2) as f32);
    sh.event(Event::Down(x, y), 100.0);
    let dy = match sh.desk.as_ref().unwrap().grab { Some(Grab::Content(AppId::Settings, dy)) => dy, _ => panic!("the press is the app's") };
    assert_eq!(dy, rv.top as f32, "at the toolbar row the band shows, the bar's height up");
    sh.event(Event::Up(x, y), 110.0);
    let lr = revealed_win(&w, rv).light_rect(&m, 2);
    assert!(lr.y >= rv.top, "the lights are below the bar");
    let (gx, gy) = ((lr.x + lr.w / 2) as f32, (lr.y + lr.h / 2) as f32);
    sh.desktop_pointer(gx, gy, false, 200.0);
    sh.event(Event::Down(gx, gy), 210.0);
    assert!(matches!(sh.desk.as_ref().unwrap().grab, Some(Grab::Light(AppId::Settings, Light::Zoom))));
    sh.event(Event::Up(gx, gy), 220.0);
    assert!(!sh.desk.as_ref().unwrap().wins[i].full, "the green light takes the window out of full screen");
}
