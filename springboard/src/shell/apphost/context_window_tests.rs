use super::*;
use crate::shell::AppId;

/// E2E7: a secondary click in an extra window of an app (a second Files window) went to no one, since only the main
/// window was told. Each window hears its own click, in its own coordinates, with its id.
#[test]
fn a_secondary_click_goes_to_the_window_under_the_pointer() {
    assert_eq!(context_click(AppId::External(3), 40., 30., 2.), Some((3, ToApp::Context { x: 40., y: 30., window: Some(0) })), "the main window: surface pixels, as always");
    assert_eq!(
        context_click(AppId::ExternalWindow(3, 7), 40., 30., 2.),
        Some((3, ToApp::Context { x: 20., y: 15., window: Some(7) })),
        "an extra window: its own points, as its touches are"
    );
    assert_eq!(context_click(AppId::Settings, 40., 30., 2.), None, "a built-in app draws its own menus");
}

/// `ToShell::ContextMenu` names no window: the menu opens for the window the pointer last used, and its point is read
/// in that window's units.
#[test]
fn a_context_menu_opens_for_the_window_the_pointer_last_used() {
    let open = |id: u16| id == 7;
    assert_eq!(context_menu_window(3, 0, open, 2.), (AppId::External(3), 0, 1.), "the main window: the point is in pixels");
    assert_eq!(context_menu_window(3, 7, open, 2.), (AppId::ExternalWindow(3, 7), 7, 2.), "an extra window: the point is in points");
    assert_eq!(context_menu_window(3, 9, open, 2.), (AppId::External(3), 0, 1.), "a window closed since then leaves the main window");
    assert_eq!(context_menu_window(3, 70_000, open, 2.), (AppId::External(3), 0, 1.), "no window has such an id");
}
