use super::*;

fn open_system() -> Shell {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.screen = Screen::Home;
    sh.anim = None;
    sh.desktop_menu = Some(Menu::System);
    sh
}

#[test]
fn right_opens_the_submenu_of_the_highlighted_row_and_left_goes_back() {
    let mut sh = open_system();
    let at = sh.menu_row_texts(Menu::System).iter().position(|t| t == "Recent Items").unwrap();
    sh.desktop_menu_row = Some((Menu::System, at));
    let _ = sh.desktop_menu_key(106, 1.);
    assert_eq!(sh.desktop_menu, Some(Menu::Recent));
    let _ = sh.desktop_menu_key(105, 2.);
    assert_eq!(sh.desktop_menu, Some(Menu::System));
    // Right on a row without a submenu still moves to the next bar menu
    sh.desktop_menu_row = Some((Menu::System, 0));
    let _ = sh.desktop_menu_key(106, 3.);
    assert_eq!(sh.desktop_menu, Some(Menu::App));
}

#[test]
fn a_click_on_recent_items_opens_the_submenu_and_keeps_a_popup() {
    let mut sh = open_system();
    let rows = sh.menu_rows(Menu::System);
    let at = rows.iter().position(|r| r.text == "Recent Items").unwrap();
    let (x, top, w, _) = sh.desktop_popup_rect().unwrap();
    let (spans, _) = menu_spans(&rows);
    let y = top as f32 + sh.ui.pt(spans[at].0 + spans[at].1 / 2.);
    let fx = sh.desktop_bar_event(Event::Up((x + w / 2) as f32, y), 1.);
    assert!(fx.is_some());
    assert_eq!(sh.desktop_menu, Some(Menu::Recent), "the click opens the submenu instead of closing the menu");
    assert!(sh.desktop_popup_rect().is_some());
}
