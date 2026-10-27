use super::{app_item_parent, app_level_items, menu_scroll, MENU_ARROW_PT};
use bifrost_proto::{AppMenu, AppMenuItem};

fn format() -> AppMenu {
    AppMenu { title: "Format".into(), items: vec![
        AppMenuItem::submenu("Font"),                 // 0
        AppMenuItem::new(1, "Show Fonts").depth(1),   // 1
        AppMenuItem::submenu("Kern").depth(1),        // 2
        AppMenuItem::new(2, "Use Default").depth(2),  // 3
        AppMenuItem::new(3, "Tighten").depth(2),      // 4
        AppMenuItem::new(4, "Bold").depth(1),         // 5
        AppMenuItem::submenu("Text"),                 // 6
        AppMenuItem::new(5, "Align Left").depth(1),   // 7
        AppMenuItem::separator(),                     // 8
        AppMenuItem::new(6, "Make Plain Text"),       // 9
    ] }
}

#[test]
fn each_level_lists_its_own_items_only() {
    let m = format();
    assert_eq!(app_level_items(&m, None), vec![0, 6, 8, 9]);
    assert_eq!(app_level_items(&m, Some(0)), vec![1, 2, 5], "Font: its items, Kern's header but not Kern's items");
    assert_eq!(app_level_items(&m, Some(2)), vec![3, 4]);
    assert_eq!(app_level_items(&m, Some(6)), vec![7]);
    assert!(app_level_items(&m, Some(1)).is_empty(), "an item that is no submenu has no level");
    assert!(app_level_items(&m, Some(99)).is_empty());
}

#[test]
fn an_item_knows_the_header_it_sits_under() {
    let m = format();
    assert_eq!(app_item_parent(&m, 0), None);
    assert_eq!(app_item_parent(&m, 2), Some(0));
    assert_eq!(app_item_parent(&m, 4), Some(2));
    assert_eq!(app_item_parent(&m, 5), Some(0), "back out of Kern, still in Font");
    assert_eq!(app_item_parent(&m, 7), Some(6));
}

#[test]
fn a_menu_that_fits_does_not_scroll() {
    let sc = menu_scroll(300., 800., 50.);
    assert_eq!((sc.height, sc.offset, sc.up, sc.down), (300., 0., false, false));
}

#[test]
fn a_tall_menu_scrolls_between_its_arrows() {
    let (content, room) = (2000., 600.);
    let top = menu_scroll(content, room, 0.);
    assert_eq!((top.height, top.up, top.down), (room, false, true), "at the top: only the bottom arrow");
    assert_eq!(top.window(), (0., room - MENU_ARROW_PT));
    let mid = menu_scroll(content, room, 700.);
    assert!(mid.up && mid.down);
    let end = menu_scroll(content, room, 1e9);
    assert_eq!((end.offset, end.up, end.down), (content - room, true, false), "clamped to the end: only the top arrow");
    assert_eq!(menu_scroll(content, room, -5.).offset, 0.);
    assert_eq!(menu_scroll(content, room, f32::NAN).offset, 0.);
    // a row under the bottom arrow does not show; revealing it scrolls just enough
    assert!(top.shows(0., 24.));
    assert!(!top.shows(room - 20., 24.));
    let o = top.reveal(content, 900., 24.);
    let after = menu_scroll(content, room, o);
    assert!(after.shows(900., 24.), "revealed at offset {o}");
    // and back up: a row above the top arrow
    let o = after.reveal(content, 100., 24.);
    assert!(menu_scroll(content, room, o).shows(100., 24.), "revealed at offset {o}");
    // the first and last rows scroll all the way, so no arrow covers them
    assert_eq!(after.reveal(content, 5., 24.), 0.);
    assert_eq!(top.reveal(content, content - 29., 24.), content - room);
}

#[test]
fn a_tiny_room_still_leaves_a_row_between_the_arrows() {
    let sc = menu_scroll(1000., 10., 0.);
    let (lo, hi) = sc.window();
    assert!(hi - lo >= 24., "{lo}..{hi}");
}
