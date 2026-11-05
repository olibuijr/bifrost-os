use super::*;
use bifrost_proto::{AppMenu, AppMenuItem};

fn with_menus(menus: Vec<AppMenu>) -> (Shell, bifrost_proto::sys::Channel) {
    // a landscape desktop, so a menu and two submenus fit side by side
    let mut sh = crate::shell::tests::shell_sized(1920, 1200);
    sh.desk = Some(crate::shell::desk::Desk::new());
    sh.form = bifrost_ui::form_factor::FormFactor::Desktop;
    sh.anim = None;
    let (mine, theirs) = bifrost_proto::sys::socketpair_pair().unwrap();
    let mut host = super::super::apphost::AppHost::with_channel("fixture", mine, None, bifrost_proto::Configure { w: 100, h: 100, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false });
    host.app_menus = menus;
    sh.ext.push(Some(host));
    sh.ext_fg = Some(0);
    sh.screen = Screen::App(AppId::External(0));
    (sh, theirs)
}

fn format(families: usize) -> AppMenu {
    let mut items = vec![AppMenuItem::submenu("Font"), AppMenuItem::new(1, "Show Fonts").depth(1), AppMenuItem::submenu("Kern").depth(1), AppMenuItem::new(2, "Use Default").depth(2), AppMenuItem::separator().depth(1)];
    items.extend((0..families).map(|i| AppMenuItem::new(100 + i as u32, &format!("Family {i}")).depth(1)));
    items.extend([AppMenuItem::submenu("Text"), AppMenuItem::new(3, "Align Left").depth(1), AppMenuItem::separator(), AppMenuItem::new(4, "Make Plain Text")]);
    AppMenu { title: "Format".into(), items }
}

fn texts(sh: &Shell, menu: Menu) -> Vec<String> {
    sh.menu_rows(menu).into_iter().map(|r| r.text).collect()
}

#[test]
fn nested_submenus_cascade_beside_their_rows_and_close_one_level_at_a_time() {
    let (mut sh, _app) = with_menus(vec![format(3)]);
    assert_eq!(texts(&sh, Menu::Custom(0)), ["Font", "Text", "", "Make Plain Text"]);
    sh.desktop_menu = Some(Menu::Custom(0));
    sh.desktop_menu_row = Some((Menu::Custom(0), 0));
    let _ = sh.desktop_menu_key(106, 1.);
    assert_eq!(sh.desktop_menu, Some(Menu::AppSub(0, 0)), "Right opens Font");
    assert_eq!(texts(&sh, Menu::AppSub(0, 0)), ["Show Fonts", "Kern", "", "Family 0", "Family 1", "Family 2"], "no row back: Format stays open beside it");
    assert_eq!(sh.menu_row_of(Menu::AppSub(0, 0)), Some(0), "the keyboard lands on Font's first item");
    let kern = 1;
    sh.desktop_menu_row = Some((Menu::AppSub(0, 0), kern));
    let _ = sh.desktop_menu_key(106, 2.);
    assert_eq!(sh.desktop_menu, Some(Menu::AppSub(0, 2)), "and Kern inside it");
    assert_eq!(texts(&sh, Menu::AppSub(0, 2)), ["Use Default"]);
    assert_eq!(sh.bar_anchor(Menu::AppSub(0, 2)), Menu::Custom(0), "a nested submenu still hangs from Format's bar item");
    assert_eq!(sh.menu_chain(), vec![Menu::Custom(0), Menu::AppSub(0, 0), Menu::AppSub(0, 2)]);
    let levels = sh.menu_level_rects();
    assert_eq!(levels.len(), 3, "Format, Font and Kern are all open");
    // Format stays under its bar item; each submenu cascades beside its parent, its first row level with the header
    let format_x = sh.menu_buttons().into_iter().find(|b| b.0 == Menu::Custom(0)).unwrap().1;
    assert_eq!(levels[0].1 .0, (format_x as usize).min((sh.w as usize).saturating_sub(levels[0].1 .2)));
    let overlap = sh.ui.pt(FLYOUT_OVERLAP_PT) as usize;
    for k in 1..3 {
        let ((px, _, pw, _), (cx, _, cw, _), right) = (levels[k - 1].1, levels[k].1, levels[k].2);
        if right { assert_eq!(cx + overlap, px + pw, "level {k} opens right of its parent"); } else { assert_eq!(cx + cw, px + overlap, "level {k} opens left of its parent"); }
    }
    let (spans, _) = menu_spans(&sh.menu_rows(Menu::Custom(0)));
    assert_eq!(levels[1].1 .1 as i32, levels[0].1 .1 as i32 + sh.ui.pt(spans[0].0) as i32 - sh.ui.pt(MENU_PAD_PT) as i32, "Font's first row is level with its header");
    assert_eq!(sh.desktop_popup_rect(), Some(levels[2].1), "the open menu is the deepest level");
    let _ = sh.desktop_menu_key(105, 3.);
    assert_eq!(sh.desktop_menu, Some(Menu::AppSub(0, 0)), "Left closes Kern");
    assert_eq!(sh.menu_row_of(Menu::AppSub(0, 0)), Some(kern), "Kern's row in Font stays highlighted");
    let _ = sh.desktop_menu_key(1, 4.);
    assert_eq!(sh.desktop_menu, Some(Menu::Custom(0)), "Esc closes one level");
    assert_eq!(sh.menu_row_of(Menu::Custom(0)), Some(0), "back on Font's row");
    let _ = sh.desktop_menu_key(1, 5.);
    assert_eq!(sh.desktop_menu, None, "Esc on the menu itself closes it");
}

/// Opens Format, then Font beside its row by the keyboard, and returns the two levels.
fn format_and_font(sh: &mut Shell) -> ((usize, usize, usize, usize), (usize, usize, usize, usize), bool) {
    sh.desktop_menu = Some(Menu::Custom(0));
    sh.desktop_menu_row = Some((Menu::Custom(0), 0));
    let _ = sh.desktop_menu_key(106, 1.);
    let levels = sh.menu_level_rects();
    assert_eq!(levels.len(), 2);
    (levels[0].1, levels[1].1, levels[1].2)
}

/// The centre of row `i` of `menu` drawn at `rect`, in screen px.
fn row_point(sh: &Shell, menu: Menu, rect: (usize, usize, usize, usize), i: usize) -> (f32, f32) {
    let (spans, _) = menu_spans(&sh.menu_rows(menu));
    (rect.0 as f32 + rect.2 as f32 / 2., rect.1 as f32 + sh.ui.pt(spans[i].0 + spans[i].1 / 2.))
}

#[test]
fn hovering_a_header_opens_its_submenu_after_the_delay_and_the_parent_stays_highlighted() {
    let (mut sh, _app) = with_menus(vec![format(3)]);
    sh.desktop_menu = Some(Menu::Custom(0));
    let format = sh.desktop_popup_rect().unwrap();
    let (x, y) = row_point(&sh, Menu::Custom(0), format, 0);
    sh.desktop_menu_hover_at(x, y, 1000.);
    assert_eq!(sh.menu_row_of(Menu::Custom(0)), Some(0), "the header row is highlighted at once");
    assert_eq!(sh.desktop_menu, Some(Menu::Custom(0)), "the submenu waits for the delay");
    assert_eq!(sh.desktop_flyout.due(), Some(1000. + FLYOUT_OPEN_MS), "the shell wakes for it");
    sh.desktop_menu_flyout_tick(1000. + FLYOUT_OPEN_MS - 1.);
    assert_eq!(sh.desktop_menu, Some(Menu::Custom(0)));
    sh.desktop_menu_flyout_tick(1000. + FLYOUT_OPEN_MS);
    assert_eq!(sh.desktop_menu, Some(Menu::AppSub(0, 0)), "resting on Font opens it");
    assert_eq!(sh.desktop_flyout.due(), None);
    assert_eq!(sh.menu_level_rects().len(), 2, "Format stays open beside it");
    // a pointer that only passes over a header does not open it
    let (tx, ty) = row_point(&sh, Menu::Custom(0), format, 1);
    sh.desktop_menu_hover_at(tx + sh.ui.pt(60.), ty, 2000.);
    sh.desktop_menu_hover_at(tx + sh.ui.pt(60.), row_point(&sh, Menu::Custom(0), format, 3).1, 2050.);
    sh.desktop_menu_flyout_tick(2050. + FLYOUT_OPEN_MS);
    assert_eq!(sh.desktop_menu, Some(Menu::Custom(0)), "Font closed and Text never opened");
    assert_eq!(sh.menu_row_of(Menu::Custom(0)), Some(3), "Make Plain Text is highlighted");
}

#[test]
fn a_diagonal_move_toward_the_submenu_keeps_it_open_and_a_rest_lets_the_row_take_over() {
    let (mut sh, _app) = with_menus(vec![format(3)]);
    let (format, font, right) = format_and_font(&mut sh);
    let (_, hy) = row_point(&sh, Menu::Custom(0), format, 0);
    // from the far side of the header row, heading down toward the submenu's near edge
    let start = if right { (format.0 as f32 + sh.ui.pt(20.), hy) } else { ((format.0 + format.2) as f32 - sh.ui.pt(20.), hy) };
    sh.desktop_menu_hover_at(start.0, start.1, 100.);
    assert_eq!(sh.desktop_menu, Some(Menu::AppSub(0, 0)), "back on Font's row: Font stays open");
    let text_y = row_point(&sh, Menu::Custom(0), format, 1).1;
    let near = if right { (font.0 as f32 - 2., text_y) } else { ((font.0 + font.2) as f32 + 2., text_y) };
    let step = (start.0 + (near.0 - start.0) * 0.9, text_y);
    assert!(in_safe_triangle(start, step, (font.0 as f32, font.1 as f32, font.2 as f32, font.3 as f32), right), "the move heads for Font");
    sh.desktop_menu_hover_at(step.0, step.1, 120.);
    assert_eq!(sh.desktop_menu, Some(Menu::AppSub(0, 0)), "crossing Text on the way does not switch the submenu");
    assert_eq!(sh.menu_level_rects().len(), 2);
    sh.desktop_menu_flyout_tick(120. + FLYOUT_SAFE_MS - 1.);
    assert_eq!(sh.desktop_menu, Some(Menu::AppSub(0, 0)));
    // stopping there lets Text take over: Font closes, Text is highlighted and opens after its own delay
    sh.desktop_menu_flyout_tick(120. + FLYOUT_SAFE_MS);
    assert_eq!(sh.desktop_menu, Some(Menu::Custom(0)), "a rest on Text closes Font");
    assert_eq!(sh.menu_row_of(Menu::Custom(0)), Some(1));
    sh.desktop_menu_flyout_tick(120. + FLYOUT_SAFE_MS + FLYOUT_OPEN_MS);
    assert_eq!(sh.desktop_menu, Some(Menu::AppSub(0, 8)), "and Text opens");
}

#[test]
fn moving_away_from_the_submenu_switches_rows_at_once() {
    let (mut sh, _app) = with_menus(vec![format(3)]);
    let (format, _, right) = format_and_font(&mut sh);
    let (_, hy) = row_point(&sh, Menu::Custom(0), format, 0);
    // near the edge away from Font, then straight down: away from the submenu
    let far = if right { format.0 as f32 + sh.ui.pt(8.) } else { (format.0 + format.2) as f32 - sh.ui.pt(8.) };
    sh.desktop_menu_hover_at(far, hy, 100.);
    let (_, py) = row_point(&sh, Menu::Custom(0), format, 3);
    let back = if right { far - sh.ui.pt(2.) } else { far + sh.ui.pt(2.) };
    sh.desktop_menu_hover_at(back, py, 110.);
    assert_eq!(sh.desktop_menu, Some(Menu::Custom(0)), "Font closes");
    assert_eq!(sh.menu_row_of(Menu::Custom(0)), Some(3), "Make Plain Text is highlighted");
}

#[test]
fn a_pointer_leave_between_the_menu_surfaces_closes_nothing() {
    let (mut sh, _app) = with_menus(vec![format(3)]);
    let (_, font, _) = format_and_font(&mut sh);
    let (x, y) = row_point(&sh, Menu::AppSub(0, 0), font, 0);
    sh.desktop_menu_hover_at(x, y, 100.);
    sh.desktop_menu_hover_at(-100., -100., 110.);
    assert_eq!(sh.desktop_menu, Some(Menu::AppSub(0, 0)));
    assert_eq!(sh.menu_row_of(Menu::AppSub(0, 0)), Some(0), "the highlight stays");
    assert_eq!(sh.menu_level_rects().len(), 2);
}

#[test]
fn a_click_on_a_parent_row_or_a_header_works_while_a_submenu_is_open() {
    let (mut sh, app) = with_menus(vec![format(3)]);
    let (format, _, _) = format_and_font(&mut sh);
    // a click on Text, another header, opens Text in place of Font
    let (tx, ty) = row_point(&sh, Menu::Custom(0), format, 1);
    let _ = sh.desktop_bar_event(Event::Up(tx, ty), 2.);
    assert_eq!(sh.desktop_menu, Some(Menu::AppSub(0, 8)), "the click opens Text at once");
    assert_eq!(sh.menu_chain(), vec![Menu::Custom(0), Menu::AppSub(0, 8)]);
    // a press inside either level keeps the menus open
    let _ = sh.desktop_bar_event(Event::Down(tx, ty), 3.);
    assert_eq!(sh.desktop_menu, Some(Menu::AppSub(0, 8)));
    // a click on Make Plain Text in Format reaches the app
    let (px, py) = row_point(&sh, Menu::Custom(0), format, 3);
    let _ = sh.desktop_bar_event(Event::Up(px, py), 4.);
    assert_eq!(sh.desktop_menu, None, "choosing an item closes every level");
    let bifrost_proto::sys::Recv::Packet(packet, _) = app.recv(false).unwrap() else { panic!("no menu action") };
    assert_eq!(bifrost_proto::ToApp::decode(&packet), Ok(bifrost_proto::ToApp::MenuAction(4)));
}

#[test]
fn each_submenu_is_an_overlay_surface_of_its_own_that_takes_the_pointer_over_the_submenu() {
    let (mut sh, _app) = with_menus(vec![format(3)]);
    let (_, font, _) = format_and_font(&mut sh);
    let (w, h) = (sh.w as usize, sh.h as usize);
    let pixels = vec![0xff10_2030u32; w * h];
    let frames = sh.desktop_chrome_frames(&pixels, w, h);
    let menu = frames.iter().find(|f| f.id == 1).expect("the menu's surface");
    let sub = frames.iter().find(|f| f.id == FLYOUT_CHROME_ID).expect("the submenu's surface");
    assert!(frames.iter().all(|f| f.id != FLYOUT_CHROME_ID + 1), "one surface per open submenu");
    let [ix, iy, iw, ih] = sub.input.expect("only the submenu takes the pointer");
    assert_eq!(((sub.x + ix) as usize, (sub.y + iy) as usize, iw as usize, ih as usize), font);
    // where the submenu's surface lies over the menu's, the menu's surface is clear
    let (mx, my) = ((sub.x - menu.x).max(0) as usize, (sub.y - menu.y).max(0) as usize);
    if mx < menu.width && my < menu.height { assert_eq!(menu.pixels[my * menu.width + mx], 0); }
    assert!(menu.pixels.iter().any(|&p| p != 0), "the rest of the menu shows");
    // closing the submenu drops its surface
    let _ = sh.desktop_menu_key(105, 2.);
    assert!(sh.desktop_chrome_frames(&pixels, w, h).iter().all(|f| f.id != FLYOUT_CHROME_ID));
}

#[test]
fn a_click_on_a_submenu_item_reaches_the_app() {
    let (mut sh, app) = with_menus(vec![format(3)]);
    sh.desktop_menu = Some(Menu::AppSub(0, 0));
    let rows = sh.menu_rows(Menu::AppSub(0, 0));
    let at = rows.iter().position(|r| r.text == "Family 1").unwrap();
    let (x, top, w, _) = sh.desktop_popup_rect().unwrap();
    let (spans, _) = menu_spans(&rows);
    let y = top as f32 + sh.ui.pt(spans[at].0 + spans[at].1 / 2.);
    let _ = sh.desktop_bar_event(Event::Up((x + w / 2) as f32, y), 1.);
    assert_eq!(sh.desktop_menu, None, "choosing an item closes the menu");
    let bifrost_proto::sys::Recv::Packet(packet, _) = app.recv(false).unwrap() else { panic!("no menu action") };
    assert_eq!(bifrost_proto::ToApp::decode(&packet), Ok(bifrost_proto::ToApp::MenuAction(101)));
}

#[test]
fn a_disabled_submenu_does_not_open() {
    let (sh, _app) = with_menus(vec![AppMenu { title: "Playback".into(), items: vec![AppMenuItem::submenu("Speed").enabled(false), AppMenuItem::new(1, "1x").depth(1)] }]);
    let rows = sh.menu_rows(Menu::Custom(0));
    assert_eq!(rows.len(), 1);
    assert!(rows[0].action.is_none());
}

#[test]
fn help_search_finds_items_inside_submenus() {
    let (sh, _app) = with_menus(vec![format(3)]);
    let entries = sh.help_entries();
    let hit = entries.iter().find(|(_, _, item)| item.title == "Use Default").expect("a nested item is searched");
    assert_eq!(hit.0, Menu::AppSub(0, 2));
    assert!(hit.2.menu.contains("Kern"), "{}", hit.2.menu);
    assert_eq!(hit.1, 0, "Use Default is Kern's first row: a cascading submenu has no row back");
}

#[test]
fn a_menu_taller_than_the_screen_scrolls_and_keeps_its_rows_reachable() {
    let (mut sh, _app) = with_menus(vec![format(300)]);
    let menu = Menu::AppSub(0, 0);
    sh.desktop_menu = Some(menu);
    let (x, top, w, h) = sh.desktop_popup_rect().unwrap();
    assert!(top + h <= sh.h as usize, "the popup stays on the screen: {top}+{h} > {}", sh.h);
    let rows = sh.menu_rows(menu);
    let sc = sh.menu_scroll_for(menu, &rows);
    assert!(sc.down && !sc.up, "a bottom arrow only");
    // the last family is reachable by the keyboard: the highlight scrolls the menu
    let last = rows.len() - 1;
    sh.desktop_menu_row = Some((menu, last - 1));
    let _ = sh.desktop_menu_key(108, 1.);
    assert_eq!(sh.menu_row_of(menu), Some(last));
    let sc = sh.menu_scroll_for(menu, &rows);
    let (spans, _) = menu_spans(&rows);
    assert!(sc.up && sc.shows(spans[last].0, spans[last].1), "the last row shows after Down");
    // a click lands on the row drawn under the pointer, not the one the offset hides
    let y = top as f32 + sh.ui.pt(spans[last].0 - sc.offset + spans[last].1 / 2.);
    assert_eq!(sh.menu_row_at(menu, y - top as f32), Some(last));
    // the wheel scrolls back up, and resting on the top arrow scrolls further each frame
    assert!(sh.desktop_menu_wheel((x + w / 2) as f32, (top + h / 2) as f32, 240.));
    let after = sh.menu_scroll_for(menu, &rows).offset;
    assert!(after < sc.offset, "{after} < {}", sc.offset);
    sh.desktop_menu_hover((x + w / 2) as f32, top as f32 + sh.ui.pt(MENU_ARROW_PT / 2.));
    assert_eq!(sh.desktop_menu_scroll.arrow, -1);
    assert!(sh.menu_autoscrolling());
    sh.desktop_menu_autoscroll(10.);
    sh.desktop_menu_autoscroll(60.);
    assert!(sh.menu_scroll_for(menu, &rows).offset < after, "resting on the arrow scrolls");
    for k in 0..2000 {
        sh.desktop_menu_autoscroll(100. + k as f64 * 50.);
    }
    assert_eq!(sh.menu_scroll_for(menu, &rows).offset, 0.);
    assert_eq!(sh.desktop_menu_scroll.arrow, 0, "it stops at the top");
    assert!(!sh.menu_autoscrolling());
}
