use super::Menu;
use crate::shell::{AppId, Screen};

#[cfg(feature = "desktop-wayland")]
#[test]
fn a_normal_window_enables_the_halves_center_and_bring_all_to_front() {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.screen = Screen::Home;
    sh.anim = None;
    sh.desk_open(AppId::Settings, 0.0);
    let rows = sh.menu_rows(Menu::Window);
    for want in ["Left Half", "Right Half", "Fill", "Center", "Bring All to Front", "Return to Previous Size"] {
        let row = rows.iter().find(|r| r.text.trim() == want).unwrap_or_else(|| panic!("{want} is in the Window menu"));
        assert!(row.action.is_some(), "{want} is enabled for a normal window");
    }
}

/// E2E9 7: an edited document's window is listed as its title bar reads it, "Untitled – Edited".
#[test]
fn an_edited_document_window_is_listed_with_its_edited_mark() {
    assert_eq!(super::window_menu_title("Untitled", true), "Untitled \u{2013} Edited");
    assert_eq!(super::window_menu_title("notes.txt", false), "notes.txt");
}

#[cfg(feature = "desktop-wayland")]
#[test]
fn with_no_window_the_placements_are_labels() {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.screen = Screen::Home;
    sh.anim = None;
    let rows = sh.menu_rows(Menu::Window);
    for want in ["Left Half", "Right Half", "Fill", "Center", "Bring All to Front"] {
        let row = rows.iter().find(|r| r.text.trim() == want).unwrap_or_else(|| panic!("{want} is in the Window menu"));
        assert!(row.action.is_none(), "{want} has nothing to act on without a window");
    }
}

#[cfg(feature = "desktop-wayland")]
fn native(id: u32, title: &str, app_id: &str, active: bool) -> crate::bare::wayland::ForeignWindow {
    crate::bare::wayland::ForeignWindow { id, title: title.into(), app_id: app_id.into(), active, maximized: false, minimized: false }
}

/// E2E N2 / W3-6: with a native app's window in front, the placements are enabled and act on that window (through
/// the window server, by its foreign toplevel), not on the shell window behind it; with the shell's window in front
/// they act on the shell's window and never send a placement for a native one.
#[cfg(feature = "desktop-wayland")]
#[test]
fn the_placements_act_on_the_front_window_a_native_one_included() {
    use super::Action;
    use crate::bare::window_level_client::Placement;
    use crate::shell::{desk::Tile, Effect};
    let mut sh = crate::shell::tests::desktop_shell();
    sh.screen = Screen::Home;
    sh.anim = None;
    sh.desk_open(AppId::Settings, 0.0);
    sh.desk.as_mut().unwrap().native_windows = vec![native(7, "Film", "x.film", true)];
    let rows = sh.menu_rows(Menu::Window);
    for want in ["Left Half", "Right Half", "Fill", "Center", "Return to Previous Size"] {
        let row = rows.iter().find(|r| r.text.trim() == want).unwrap_or_else(|| panic!("{want} is in the Window menu"));
        assert!(row.action.is_some(), "{want} is enabled for a native window in front");
    }
    let before = sh.desk.as_ref().unwrap().wins.iter().find(|w| w.app == AppId::Settings).unwrap().r;
    let fx = sh.run_menu_action(Action::Tile(Tile::Fill), 1000.0);
    assert!(fx.contains(&Effect::ForeignWindowTile(7, Placement::Fill)), "Fill places the native window: {fx:?}");
    let fx = sh.run_menu_action(Action::Tile(Tile::Left), 1100.0);
    assert!(fx.contains(&Effect::ForeignWindowTile(7, Placement::Left)), "{fx:?}");
    let fx = sh.run_menu_action(Action::RestoreSize, 1200.0);
    assert!(fx.contains(&Effect::ForeignWindowTile(7, Placement::Previous)), "{fx:?}");
    // Ctrl+Cmd+Right, which the window server sends as a `tile right` line, takes the same way
    let fx = sh.hw_shortcut(crate::shell::hwkeys::Shortcut::tile(crate::bare::control::TileKey::Right), 1300.0);
    assert!(fx.contains(&Effect::ForeignWindowTile(7, Placement::Right)), "{fx:?}");
    assert_eq!(sh.desk.as_ref().unwrap().wins.iter().find(|w| w.app == AppId::Settings).unwrap().r, before, "the shell window behind is left alone");
    // the shell's window in front again: no placement goes to the native window
    sh.desk.as_mut().unwrap().native_windows[0].active = false;
    let fx = sh.run_menu_action(Action::Tile(Tile::Left), 1400.0);
    assert!(!fx.iter().any(|e| matches!(e, Effect::ForeignWindowTile(..))), "{fx:?}");
}

/// E2E N2: each app's Window menu lists only that app's windows: a native app in front lists its own windows and
/// neither the shell's nor another native app's; a shell app in front lists none of the native ones.
#[cfg(feature = "desktop-wayland")]
#[test]
fn the_window_menu_lists_only_the_front_apps_windows() {
    use super::Action;
    let mut sh = crate::shell::tests::desktop_shell();
    sh.screen = Screen::Home;
    sh.anim = None;
    sh.desk_open(AppId::Settings, 0.0);
    sh.desk.as_mut().unwrap().native_windows = vec![native(7, "Film A", "x.film", true), native(8, "Film B", "x.film", false), native(9, "Other", "x.other", false)];
    let rows = sh.menu_rows(Menu::Window);
    let foreign = |rows: &[super::Row]| -> Vec<u32> { rows.iter().filter_map(|r| match r.action { Some(Action::Foreign(id)) => Some(id), _ => None }).collect() };
    let shell_windows = |rows: &[super::Row]| rows.iter().filter(|r| matches!(r.action, Some(Action::Open(_)))).count();
    assert_eq!(foreign(&rows), vec![7, 8], "the front native app's two windows only");
    assert_eq!(shell_windows(&rows), 0, "no shell window under a native app");
    sh.desk.as_mut().unwrap().native_windows[0].active = false;
    let rows = sh.menu_rows(Menu::Window);
    assert!(foreign(&rows).is_empty(), "a shell app's menu lists no native window");
    assert!(rows.iter().any(|r| matches!(r.action, Some(Action::Open(AppId::Settings)))), "it lists its own");
}
