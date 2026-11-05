use super::*;
use bifrost_proto::ContextItem;

fn items() -> Vec<ContextItem> {
    vec![
        ContextItem::new(1, "Open"),
        ContextItem::new(2, "Get Info").key("cmd+i"),
        ContextItem::separator(),
        ContextItem::new(3, "Paste").enabled(false).key("cmd+v"),
    ]
}

/// A desktop shell with `items()` open as a context menu for a pointer at (x, y).
fn open_at(x: f32, y: f32) -> Shell {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.desktop_context = Some(ContextMenu { slot: 0, x, y, items: items() });
    sh.desktop_menu = Some(Menu::Context);
    sh
}

#[test]
fn the_surface_is_as_tall_as_its_rows_and_separators() {
    let sh = open_at(300., 200.);
    let (_, _, w, h) = sh.desktop_popup_rect().unwrap();
    // edge padding, three rows, one separator, edge padding
    let pt = 2. * MENU_PAD_PT + 3. * MENU_ROW_PT + MENU_SEP_PT;
    assert_eq!(h, sh.ui.pt(pt).ceil() as usize);
    assert_eq!(h, sh.ui.pt(menu_spans(&sh.menu_rows(Menu::Context)).1).ceil() as usize, "the height the rows are drawn with");
    assert!((h as f32) < sh.ui.pt(8. + 36. * 4.), "not a fixed 36 pt per item");
    assert!(w >= sh.ui.pt(264.) as usize, "as wide as the bar menus");
}

#[test]
fn it_opens_at_the_pointer_and_flips_at_the_screen_edges() {
    let sh = open_at(300., 200.);
    let (x, y, _, _) = sh.desktop_popup_rect().unwrap();
    assert_eq!((x, y), (300, 200), "top-left at the pointer");
    // near the bottom of the work area: above the pointer
    let area = crate::shell::desk::Desk::area(&sh.desk_metrics());
    let low = area.y + area.h - 10;
    let sh = open_at(300., low as f32);
    let (x, y, _, h) = sh.desktop_popup_rect().unwrap();
    assert_eq!((x, y + h), (300, low as usize), "its bottom edge is the pointer");
    // near the right edge: left of the pointer
    let right = sh.w - 10;
    let sh = open_at(right as f32, 200.);
    let (x, y, w, _) = sh.desktop_popup_rect().unwrap();
    assert_eq!((x + w, y), (right as usize, 200), "its right edge is the pointer");
}

/// E2E N6: under the window server the popup is a chrome surface cropped to the menu, so its shadow was cut off and
/// a menu over white content had no edge. The surface now has room for the shadow; only the menu takes input.
#[test]
fn the_exported_surface_has_room_for_the_shadow_and_only_the_menu_takes_input() {
    let mut sh = open_at(300., 400.);
    sh.anim = None;
    // the fixture starts on the lock screen, where no chrome is exported
    sh.screen = crate::shell::Screen::Home;
    let (w, h) = (sh.w as usize, sh.h as usize);
    let pixels = vec![0u32; w * h];
    let (px, py, pw, ph) = sh.desktop_popup_rect().unwrap();
    let frames = sh.desktop_chrome_frames(&pixels, w, h);
    let f = frames.iter().find(|f| f.id == 1).expect("an open menu is exported");
    assert_eq!(f.pixels.len(), f.width * f.height);
    assert_eq!(f.input, Some([px as i32 - f.x, py as i32 - f.y, pw as i32, ph as i32]), "the menu's own rectangle, in surface px");
    let reach = sh.ui.pt(crate::shell::desk::SHADOW_REACH_PT) as i32;
    assert!(f.x <= px as i32 - reach && f.y <= py as i32 - reach, "room for the shadow left of and above the menu");
    assert!(f.x + f.width as i32 >= (px + pw) as i32 + reach && f.y + f.height as i32 >= (py + ph) as i32 + reach, "and right of and below it");
    assert!(frames.iter().filter(|f| f.id != 1 && f.id != 2).all(|f| f.input.is_none()), "the other chrome takes the whole surface");
    // W2-24: the Dock's surface has room for a magnified plate at both ends; only the plate drawn now takes input
    if let Some(d) = frames.iter().find(|f| f.id == 2) {
        let plate = sh.dock_overlay_rect();
        assert_eq!(d.input, Some([plate.x - d.x, plate.y - d.y, plate.w, plate.h]), "the plate, in surface px");
        assert!(d.x <= plate.x && d.x + d.width as i32 >= plate.x + plate.w, "the surface holds the plate");
    }
}

/// The shadow does not change what a click does: the menu's rectangle decides, a press on the shadow closes the menu.
#[test]
fn hit_testing_uses_the_menu_rectangle_not_the_shadow() {
    let mut sh = open_at(300., 400.);
    sh.anim = None;
    // the fixture starts on the lock screen, where no chrome is exported
    sh.screen = crate::shell::Screen::Home;
    let (px, py, _, _) = sh.desktop_popup_rect().unwrap();
    let (w, h) = (sh.w as usize, sh.h as usize);
    let pixels = vec![0u32; w * h];
    let f = sh.desktop_chrome_frames(&pixels, w, h).into_iter().find(|f| f.id == 1).unwrap();
    let on_shadow = (px as f32 - 3., py as f32 + 10.);
    assert!(f.x as f32 <= on_shadow.0, "the point is inside the exported surface");
    sh.desktop_bar_event(Event::Down(px as f32 + 5., py as f32 + 5.), 1.);
    assert!(sh.desktop_menu.is_some(), "a press inside the menu keeps it open");
    sh.desktop_bar_event(Event::Down(on_shadow.0, on_shadow.1), 2.);
    assert!(sh.desktop_menu.is_none(), "a press on the shadow, outside the menu, closes it");
}

/// Both draw paths give the menu a shadow around it and a one-pixel edge in the separator colour.
#[test]
fn the_popup_is_drawn_with_a_shadow_and_a_hairline_edge() {
    let sh = open_at(300., 400.);
    let (w, h) = (sh.w as usize, sh.h as usize);
    let (px, py, pw, ph) = sh.desktop_popup_rect().unwrap();
    let mut buf = vec![0u32; w * h];
    sh.draw_desktop_popup(&mut Surface { w, h, stride: w, px: &mut buf, y0: 0 });
    let alpha = |x: usize, y: usize| buf[y * w + x] >> 24;
    let cx = px + pw / 2;
    let reach = sh.ui.pt(crate::shell::desk::SHADOW_REACH_PT) as usize;
    assert!((1..=reach + 2).any(|d| alpha(cx, py + ph + d) > 0), "a shadow below the menu");
    assert!((1..=reach + 2).any(|d| alpha(px - d, py + ph / 2) > 0), "and beside it");
    assert_eq!(alpha(cx, py + ph + 3 * reach), 0, "none beyond its reach");
    let edge = buf[py * w + cx];
    let want = crate::shell::ui::separator_now();
    assert_eq!(edge >> 24, 0xff, "the edge is opaque");
    for shift in [0, 8, 16] {
        let (a, b) = ((edge >> shift & 0xff) as i32, (want >> shift & 0xff) as i32);
        assert!((a - b).abs() <= 3, "the top edge is the separator colour (channel at {shift}: {a} vs {b})");
    }
}

/// Mail's message menu (21 rows) from a right-click low in the list opened upward; wherever an app opens its menu,
/// the whole menu stays on the screen (inside the area under the bar and above the Dock, which may be outside the
/// app's window, as in Golden Gate), and a menu taller than that area scrolls instead of losing rows.
#[test]
fn a_long_app_menu_stays_inside_the_desktop_area_wherever_it_opens_and_scrolls_when_taller() {
    let long = |n: u32| -> Vec<ContextItem> {
        (1..=n).flat_map(|i| if i % 5 == 0 { vec![ContextItem::separator(), ContextItem::new(i, "Move to Trash")] } else { vec![ContextItem::new(i, "Forward as Attachment")] }).collect()
    };
    for (n, height) in [(17, None), (17, Some(700)), (28, Some(500)), (28, Some(420))] {
        let mut sh = open_at(0., 0.);
        if let Some(h) = height {
            sh.h = h;
        }
        let area = crate::shell::desk::Desk::area(&sh.desk_metrics());
        let (top, bottom) = (area.y as usize, (area.y + area.h) as usize);
        for at in [(20., 20.), (300., (bottom - 4) as f32), (sh.w as f32 - 4., (top + 4) as f32), (sh.w as f32 / 2., (top + bottom) as f32 / 2.), (sh.w as f32 + 50., sh.h as f32 + 50.)] {
            sh.desktop_context = Some(ContextMenu { slot: 0, x: at.0, y: at.1, items: long(n) });
            sh.desktop_menu = Some(Menu::Context);
            let (x, y, w, h) = sh.desktop_popup_rect().unwrap();
            let tag = format!("{n} items, screen {}, pointer {at:?}: menu {x},{y} {w}x{h}, area {top}..{bottom}", sh.h);
            assert!(y >= top && y + h <= bottom, "no row is cut at the top or the bottom ({tag})");
            assert!(x + w <= sh.w as usize, "nor at the right ({tag})");
            let rows = sh.menu_rows(Menu::Context);
            let content = sh.ui.pt(menu_spans(&rows).1).ceil() as usize;
            let sc = sh.menu_scroll_for(Menu::Context, &rows);
            if content > bottom - top {
                assert!(sc.down && h < content, "a menu taller than the area scrolls ({tag})");
            } else {
                assert!(!sc.up && !sc.down && h == content, "one that fits shows every row ({tag})");
            }
        }
    }
}

#[test]
fn rows_follow_the_items_and_disabled_items_are_labels() {
    let mut list = vec![ContextItem::separator()];
    list.extend(items());
    list.extend([ContextItem::separator(), ContextItem::separator()]);
    let rows = context_rows(4, &list);
    assert_eq!(rows.len(), 4, "a separator at either end is dropped");
    assert!(matches!(rows[0].action, Some(Action::Context(4, 1))), "a choice goes back to the app with its id");
    assert!(rows[0].key_text().is_none());
    assert_eq!((rows[1].text.as_str(), rows[1].key_text()), ("Get Info", Some("\u{2318}I")));
    assert!(rows[2].is_separator());
    assert_eq!(rows[3].text, "Paste", "the key equivalent is not part of the title");
    assert!(rows[3].action.is_none(), "a disabled item is a label in the secondary ink, not a choice");
    assert_eq!(rows[3].key_text(), Some("\u{2318}V"));
    let doubled = context_rows(0, &[ContextItem::new(1, "A"), ContextItem::separator(), ContextItem::separator(), ContextItem::new(2, "B")]);
    assert_eq!(doubled.len(), 3, "two separators in a row are one");
}

#[test]
fn hovering_a_row_highlights_it_and_the_bar_does_not_take_over_the_menu() {
    let mut sh = open_at(300., 200.);
    let (x, y, _, _) = sh.desktop_popup_rect().unwrap();
    let (spans, _) = menu_spans(&sh.menu_rows(Menu::Context));
    let (top, h) = spans[1];
    sh.desktop_menu_hover(x as f32 + 12., y as f32 + sh.ui.pt(top + h / 2.));
    assert_eq!(sh.menu_row_of(Menu::Context), Some(1));
    sh.desktop_menu_hover(x as f32 + 12., 4.);
    assert_eq!(sh.desktop_menu, Some(Menu::Context), "crossing the bar keeps the context menu");
    assert_eq!(sh.menu_row_of(Menu::Context), None);
}
