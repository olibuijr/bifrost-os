use super::*;

fn metrics() -> Metrics {
    Metrics { w: 2560, h: 1440, title: 52, menubar: 35, min_w: 320, min_h: 220, edge: 6, light: 12, light_gap: 8, light_x: 12, dock_left: 0, dock_right: 0, dock_bottom: 172, pt: 1.25 }
}

fn desktop() -> crate::shell::Shell {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.anim = None;
    sh.screen = crate::shell::Screen::Home;
    if sh.desk.is_none() { sh.desk = Some(Desk::new()); }
    sh
}

/// W4-1: a built-in window never floats, whatever the list says, so raising it always puts it in front of the
/// normal windows of every app.
#[test]
fn a_built_in_window_never_floats() {
    let mut d = Desk::new();
    d.floating = vec![AppId::Terminal, AppId::Settings];
    assert!(!d.is_floating(AppId::Terminal) && !d.is_floating(AppId::Settings));
    d.floating = vec![AppId::External(4)];
    assert!(d.is_floating(AppId::External(4)) && d.is_floating(AppId::ExternalWindow(4, 2)), "an app's windows float with it");
    d.floating = vec![AppId::ExternalWindow(4, 2)];
    assert!(d.is_floating(AppId::ExternalWindow(4, 2)) && !d.is_floating(AppId::External(4)), "one extra window floats alone");
}

/// W4-1: the bands `sync_levels` sorts are the ones `band_top` reads: an extra window of a floating app goes up with
/// it, so a built-in window raised afterwards lands above every normal window, not under a stray floating one.
#[test]
fn a_raised_built_in_window_goes_above_every_normal_window() {
    let m = metrics();
    let mut d = Desk::new();
    d.open(&m, AppId::Terminal, (800, 500));
    d.open(&m, AppId::ExternalWindow(5, 1), (300, 200));
    d.open(&m, AppId::Settings, (900, 700));
    d.open(&m, AppId::External(25), (1000, 700));
    d.open(&m, AppId::External(5), (400, 300));
    d.sync_levels(vec![AppId::External(5)]);
    let order: Vec<AppId> = d.wins.iter().map(|w| w.app).collect();
    assert_eq!(&order[3..], &[AppId::ExternalWindow(5, 1), AppId::External(5)], "both windows of the floating app are on top");
    let t = d.index_of(AppId::Terminal).unwrap();
    let at = d.raise(t);
    assert_eq!(at, 2, "Terminal goes right under the floating band");
    assert!(d.wins[..at].iter().all(|w| !d.is_floating(w.app)), "no floating window is left under it");
    d.sync_levels(Vec::new());
    let t = d.index_of(AppId::Terminal).unwrap();
    assert_eq!(d.raise(t), d.wins.len() - 1, "with nothing floating, a raise is the very front");
    assert_eq!(d.front().map(|w| w.app), Some(AppId::Terminal));
}

/// W4-3, W4-8: a window closed at a tile or the Fill frame is not remembered at that size; a size the person chose is.
#[test]
fn a_tile_or_fill_frame_is_not_remembered_as_the_closed_size() {
    let a = Desk::area(&metrics());
    assert!(!remembered_size(a, (a.w, a.h)), "the Fill frame");
    assert!(!remembered_size(a, (a.w / 2, a.h)), "a half tile is as tall as the work area");
    assert!(!remembered_size(a, (a.w, a.h / 2)), "a top or bottom tile is as wide");
    assert!(remembered_size(a, (1200, 825)), "a default-sized window");
    assert!(remembered_size(a, (a.w * 2 / 3, a.h * 2 / 3)), "the Centre tile is a size of its own");
}

/// W4-8: an app without a manifest `window_size` opens at its default size, even when its last window was closed
/// while it filled the work area.
#[test]
fn a_window_closed_filling_the_screen_opens_again_at_the_default_size() {
    let mut sh = desktop();
    sh.desk_open(AppId::Photos, 0.0);
    let m = sh.desk_metrics();
    let default = { let d = sh.desk.as_ref().unwrap(); let r = d.wins[d.index_of(AppId::Photos).unwrap()].r; (r.w, r.h) };
    {
        let d = sh.desk.as_mut().unwrap();
        let i = d.index_of(AppId::Photos).unwrap();
        d.wins[i].r = Desk::area(&m);
        d.close(i);
    }
    sh.desk_open(AppId::Photos, 10.0);
    let d = sh.desk.as_ref().unwrap();
    let r = d.wins[d.index_of(AppId::Photos).unwrap()].r;
    assert_eq!((r.w, r.h), default, "the default size, not the work area");
}

/// W4-2: a click on the pinned Terminal icon in the Dock opens Terminal in front, with the focus.
#[test]
fn a_dock_click_on_terminal_opens_it_in_front() {
    let mut sh = desktop();
    sh.desk_open(AppId::Settings, 0.0);
    sh.desk_open(AppId::Photos, 1.0);
    {
        let p = &mut sh.desk.as_mut().unwrap().dock_preferences.pinned;
        if !p.iter().any(|k| k == "is.olibuijr.term") { p.push("is.olibuijr.term".into()); }
    }
    let m = sh.desk_metrics();
    let t = 5000.0;
    let (list, bar, icons) = sh.desk_dock(&m);
    let (_, icons) = sh.dock_live(bar, &icons, t / 1000.0);
    let k = list.iter().position(|&e| apps().get(e).is_some_and(|a| a.id == AppId::Terminal)).expect("Terminal is in the Dock");
    let r = icons[k];
    let (x, y) = ((r.x + r.w / 2) as f32, (r.y + r.h / 2) as f32);
    sh.desk_event(Event::Down(x, y), t);
    sh.desk_event(Event::Up(x, y), t + 60.0);
    let d = sh.desk.as_ref().unwrap();
    assert_eq!(d.front().map(|w| w.app), Some(AppId::Terminal), "Terminal is the front window");
    assert_eq!(sh.screen, crate::shell::Screen::App(AppId::Terminal), "and has the focus");
}

/// W4-5: opening a window that sits behind others (an Overview pick, the Dock, the Window menu) brings it to the very
/// front with the focus, and the window server is asked to raise it.
#[test]
fn opening_a_window_behind_others_brings_it_to_the_very_front() {
    let mut sh = desktop();
    sh.desk_open(AppId::Terminal, 0.0);
    sh.desk_open(AppId::Settings, 1.0);
    sh.desk_open(AppId::Photos, 2.0);
    sh.take_raise_front();
    sh.desk_open(AppId::Terminal, 3.0);
    assert_eq!(sh.desk.as_ref().unwrap().wins.last().map(|w| w.app), Some(AppId::Terminal), "last in the stack");
    assert_eq!(sh.screen, crate::shell::Screen::App(AppId::Terminal));
    assert!(sh.take_raise_front(), "the window server raises and focuses it");
}

/// W4-1: raising an app's extra window (the Window menu, the Dock's app menu) asks the window server to raise it too.
#[test]
fn focusing_an_extra_window_asks_the_window_server_to_raise_it() {
    let mut sh = desktop();
    let m = sh.desk_metrics();
    let d = sh.desk.as_mut().unwrap();
    d.open(&m, AppId::ExternalWindow(3, 1), (300, 200));
    d.open(&m, AppId::Settings, (900, 700));
    sh.take_raise_front();
    assert!(sh.desk_focus_extra_window(3, 1));
    assert_eq!(sh.desk.as_ref().unwrap().front().map(|w| w.app), Some(AppId::ExternalWindow(3, 1)));
    assert!(sh.take_raise_front());
}

/// W4-4: Show Desktop (Esc, the hot corner, the Window menu) draws the whole desktop surface afresh, so nothing left
/// under the windows stays on the bare desktop.
#[test]
fn show_desktop_redraws_the_whole_desktop() {
    let mut sh = desktop();
    sh.desk_open(AppId::Settings, 0.0);
    sh.desk_regions = Some(vec![(0, 0, 10, 10)]);
    sh.desk_show_desktop();
    assert!(sh.desk_regions.is_none(), "no partial frame: the regions of the last one are dropped");
    assert!(matches!(sh.drawn, crate::shell::Drawn::Nothing));
    assert!(sh.desk.as_ref().unwrap().wins.iter().all(|w| w.minimized));
}

fn minimized(sh: &crate::shell::Shell, app: AppId) -> bool {
    let d = sh.desk.as_ref().unwrap();
    d.wins[d.index_of(app).unwrap()].minimized
}

/// N13: Show Desktop is its own opposite: a second Show Desktop brings back the windows it put aside, in their
/// order, and a window the person minimized stays in the Dock. The windows aside have no Dock slot.
#[test]
fn show_desktop_again_brings_the_windows_back() {
    let mut sh = desktop();
    sh.desk_open(AppId::Settings, 0.0);
    sh.desk_open(AppId::Photos, 1.0);
    sh.desk_open(AppId::Terminal, 2.0);
    let i = sh.desk.as_ref().unwrap().index_of(AppId::Terminal).unwrap();
    sh.desk_minimize_window(i, 3.0);
    sh.desk_show_desktop();
    assert!(minimized(&sh, AppId::Settings) && minimized(&sh, AppId::Photos));
    let m = sh.desk_metrics();
    let (list, _, _) = sh.desk_dock(&m);
    let slots = list.iter().filter(|&&e| crate::shell::desktop_dock::dock_min_index(e).is_some()).count();
    assert_eq!(slots, 1, "only the window the person minimized has a Dock slot");
    sh.desk_show_desktop();
    assert!(!minimized(&sh, AppId::Settings) && !minimized(&sh, AppId::Photos), "the windows came back");
    assert!(minimized(&sh, AppId::Terminal), "the minimized window stays in the Dock");
    assert_eq!(sh.desk.as_ref().unwrap().front().map(|w| w.app), Some(AppId::Photos), "the front window is in front again");
}

/// N13: after Show Desktop, Window > Bring All to Front brings the windows back.
#[test]
fn bring_all_to_front_brings_back_what_show_desktop_put_aside() {
    let mut sh = desktop();
    sh.desk_open(AppId::Settings, 0.0);
    sh.desk_open(AppId::Photos, 1.0);
    sh.desk_show_desktop();
    sh.desk_bring_all_to_front();
    assert!(!minimized(&sh, AppId::Settings) && !minimized(&sh, AppId::Photos));
    assert_eq!(sh.desk.as_ref().unwrap().front().map(|w| w.app), Some(AppId::Photos));
}

#[cfg(feature = "desktop-wayland")]
fn with_native(sh: &mut crate::shell::Shell, windows: &[(u32, bool)]) {
    sh.desk.as_mut().unwrap().native_windows = windows
        .iter()
        .map(|&(id, active)| crate::bare::wayland::ForeignWindow { id, title: "Film".into(), app_id: format!("x.app{id}"), active, maximized: false, minimized: false })
        .collect();
}

/// The window server reports the toplevels `ids` minimized (or shown again).
#[cfg(feature = "desktop-wayland")]
fn report_minimized(sh: &mut crate::shell::Shell, ids: &[u32], minimized: bool) {
    for w in sh.desk.as_mut().unwrap().native_windows.iter_mut().filter(|w| ids.contains(&w.id)) {
        w.minimized = minimized;
    }
}

/// R3: Show Desktop puts the window server's toplevels (a native app's window) aside with the shell's windows, and
/// a second Show Desktop brings them back, the window that was active last so it is in front again.
#[cfg(feature = "desktop-wayland")]
#[test]
fn show_desktop_puts_native_windows_aside_and_brings_them_back() {
    let mut sh = desktop();
    sh.desk_open(AppId::Settings, 0.0);
    with_native(&mut sh, &[(7, true), (8, false)]);
    sh.desk_show_desktop();
    assert!(minimized(&sh, AppId::Settings));
    assert_eq!(sh.take_native_minimizes(), vec![8, 7], "both native windows are minimized, the active one last");
    report_minimized(&mut sh, &[7, 8], true);
    sh.desk_show_desktop();
    assert!(!minimized(&sh, AppId::Settings), "the shell's window came back");
    assert_eq!(sh.take_native_activations(), vec![8, 7], "the native windows come back, the active one in front");
    assert!(sh.take_native_minimizes().is_empty());
}

/// R3: with only a native window on screen, Show Desktop is not taken for an empty desktop: it puts the window
/// aside, and a native window the person minimized is left alone. A window put aside counts as gone at once, so
/// the toggle works before the window server reports it minimized, and going home twice minimizes it once.
#[cfg(feature = "desktop-wayland")]
#[test]
fn show_desktop_with_only_a_native_window_toggles_it() {
    let mut sh = desktop();
    with_native(&mut sh, &[(7, true), (9, false)]);
    report_minimized(&mut sh, &[9], true);
    sh.desk_show_desktop();
    assert_eq!(sh.take_native_minimizes(), vec![7], "the minimized window is left alone");
    sh.desk_show_desktop();
    assert_eq!(sh.take_native_activations(), vec![7], "only the window Show Desktop put aside comes back");
    sh.desk_put_windows_aside();
    sh.desk_put_windows_aside();
    assert_eq!(sh.take_native_minimizes(), vec![7], "put aside once");
}

/// R3: after Show Desktop, Bring All to Front brings back the shell's windows and the active app's native windows
/// (Window > Bring All to Front acts on the active app); another app's native window stays aside until Show
/// Desktop's toggle brings everything back. The active app is remembered from when the windows went aside, since
/// the window server reports no toplevel active once they are minimized.
#[cfg(feature = "desktop-wayland")]
#[test]
fn bring_all_to_front_brings_back_only_the_active_apps_native_windows() {
    let mut sh = desktop();
    sh.desk_open(AppId::Settings, 0.0);
    with_native(&mut sh, &[(7, true), (8, false)]);
    sh.desk_show_desktop();
    assert_eq!(sh.take_native_minimizes(), vec![8, 7]);
    report_minimized(&mut sh, &[7, 8], true);
    for w in sh.desk.as_mut().unwrap().native_windows.iter_mut() {
        w.active = false;
    }
    sh.desk_bring_all_to_front();
    assert!(!minimized(&sh, AppId::Settings), "the shell's window came back");
    assert_eq!(sh.take_native_activations(), vec![7], "only the active app's native window came back");
    assert_eq!(sh.desk.as_ref().unwrap().foreign_aside, vec![8], "the other app's window stays aside");
    assert!(sh.desk.as_mut().unwrap().bring_back_put_aside());
    assert_eq!(sh.take_native_activations(), vec![8], "the toggle's bring-back restores everything");
}

/// R3: with a shell window's app active (no toplevel active), Bring All to Front brings back the shell's windows
/// and leaves the native windows aside.
#[cfg(feature = "desktop-wayland")]
#[test]
fn bring_all_to_front_for_a_shell_app_leaves_native_windows_aside() {
    let mut sh = desktop();
    sh.desk_open(AppId::Settings, 0.0);
    with_native(&mut sh, &[(7, false)]);
    sh.desk_show_desktop();
    assert_eq!(sh.take_native_minimizes(), vec![7]);
    report_minimized(&mut sh, &[7], true);
    sh.screen = crate::shell::Screen::App(AppId::Settings);
    sh.desk_bring_all_to_front();
    assert!(!minimized(&sh, AppId::Settings));
    assert!(sh.take_native_activations().is_empty());
    assert_eq!(sh.desk.as_ref().unwrap().foreign_aside, vec![7]);
}

/// The window server reports the shell's current native windows (the foreign toplevel state update).
#[cfg(feature = "desktop-wayland")]
fn report_windows(sh: &mut crate::shell::Shell) {
    let d = sh.desk.as_mut().unwrap();
    let windows = d.native_windows.clone();
    d.prune_foreign_aside(&windows);
}

/// R3: a native window put aside that comes back by itself (the app activates it, or the Dock unminimizes it) is
/// no longer aside once the window server reports it shown: the next Show Desktop hides it again instead of taking
/// the bring-back path.
#[cfg(feature = "desktop-wayland")]
#[test]
fn a_native_window_shown_again_is_no_longer_aside() {
    let mut sh = desktop();
    with_native(&mut sh, &[(7, true)]);
    sh.desk_show_desktop();
    assert_eq!(sh.take_native_minimizes(), vec![7]);
    report_minimized(&mut sh, &[7], true);
    report_windows(&mut sh);
    assert_eq!(sh.desk.as_ref().unwrap().foreign_aside, vec![7], "still aside while minimized");
    report_minimized(&mut sh, &[7], false);
    report_windows(&mut sh);
    assert!(sh.desk.as_ref().unwrap().foreign_aside.is_empty(), "shown again: no longer aside");
    sh.desk_show_desktop();
    assert!(sh.take_native_activations().is_empty(), "no bring-back");
    assert_eq!(sh.take_native_minimizes(), vec![7], "the window is put aside again");
}

/// R3: a report that predates the minimize (the window still shown) does not end Show Desktop: the window stays
/// aside and the toggle brings it back.
#[cfg(feature = "desktop-wayland")]
#[test]
fn a_report_before_the_minimize_keeps_the_window_aside() {
    let mut sh = desktop();
    with_native(&mut sh, &[(7, true)]);
    sh.desk_show_desktop();
    assert_eq!(sh.take_native_minimizes(), vec![7]);
    report_windows(&mut sh);
    assert_eq!(sh.desk.as_ref().unwrap().foreign_aside, vec![7]);
    sh.desk_show_desktop();
    assert_eq!(sh.take_native_activations(), vec![7]);
}

/// R3: a native window put aside that closes is dropped from the windows aside, and one the shell activates (the
/// Dock, Overview) is shown again at once.
#[cfg(feature = "desktop-wayland")]
#[test]
fn a_native_window_closed_or_activated_is_no_longer_aside() {
    let mut sh = desktop();
    with_native(&mut sh, &[(7, true), (8, false)]);
    sh.desk_show_desktop();
    assert_eq!(sh.take_native_minimizes(), vec![8, 7]);
    report_minimized(&mut sh, &[7, 8], true);
    sh.desk.as_mut().unwrap().native_windows.retain(|w| w.id != 8);
    report_windows(&mut sh);
    assert_eq!(sh.desk.as_ref().unwrap().foreign_aside, vec![7], "the closed window is dropped");
    sh.desk_activate_foreign(7);
    assert!(sh.desk.as_ref().unwrap().foreign_aside.is_empty(), "the activated window is no longer aside");
    assert_eq!(sh.take_native_activations(), vec![7]);
}

/// N13: going home on the desktop (the assistive Home, a Screen Time block) only ever puts the windows aside; it
/// never brings them back the way a second Show Desktop does.
#[test]
fn putting_the_windows_aside_twice_keeps_them_aside() {
    let mut sh = desktop();
    sh.desk_open(AppId::Settings, 0.0);
    sh.desk_put_windows_aside();
    sh.desk_put_windows_aside();
    assert!(minimized(&sh, AppId::Settings));
}

/// N8: when the Terminal's shell exits, only the Terminal's window closes; the other windows stay on the desktop.
#[test]
fn a_terminal_whose_shell_exits_closes_only_its_window() {
    let mut sh = desktop();
    sh.desk_open(AppId::Settings, 0.0);
    sh.desk_open(AppId::Terminal, 1.0);
    sh.term.clear();
    sh.anim = None;
    sh.tick(100.0);
    let d = sh.desk.as_ref().unwrap();
    assert!(d.index_of(AppId::Terminal).is_none(), "the Terminal's window closed");
    assert!(!minimized(&sh, AppId::Settings), "Settings stays on the desktop");
    assert_eq!(sh.screen, crate::shell::Screen::App(AppId::Settings));
}
