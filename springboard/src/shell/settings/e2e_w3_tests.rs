use super::*;
use bifrost_ui::prefs_layout::m;

fn info() -> Info {
    Info { battery: 42, wifi: "Home".into(), wifi_on: true, model: "Midget".into(), brightness: 0.5, ..Info::default() }
}

/// The desktop split layout (sidebar + detail).
fn split(ui: &Ui) -> (Settings, Prefs) {
    let st = Settings::new(ui, &info(), &Prefs::memory(), 1200, 1920, Rc::new(vec![]));
    assert!(st.sidebar_list().is_some(), "1200 px at this scale is wide enough for the split layout");
    (st, Prefs::memory())
}

fn settle(st: &mut Settings, ui: &Ui) {
    st.tick(ui, 1e9);
}

fn tap(st: &mut Settings, ui: &Ui, prefs: &mut Prefs, x: f32, y: f32) -> Vec<Effect> {
    let mut fx = st.touch(ui, prefs, Event::Down(x, y), 0.0);
    fx.extend(st.touch(ui, prefs, Event::Up(x, y), 10.0));
    settle(st, ui);
    fx
}

/// The centre of the back (`false`) or forward (`true`) half of the toolbar pill, in window px.
fn pill(st: &Settings, ui: &Ui, forward: bool) -> (f32, f32) {
    let k = if forward { 0.75 } else { 0.25 };
    (st.pane_x() + ui.pt(m::TOOLBAR_X + m::TOOLBAR_PILL_W * k), ui.pt(m::TOOLBAR_Y + m::TOOLBAR_PILL_H / 2.0))
}

/// W3-3: a page opened in the split layout (a deep link, a Search hit) had no back target, so the pill and Esc did
/// nothing; it now goes on its pane, and the pill walks back and forward through the history.
#[test]
fn the_split_layout_pill_goes_back_and_forward() {
    let ui = crate::shell::tests::ui();
    let (mut st, mut prefs) = split(&ui);
    st.open_page(&ui, "About", 0.0);
    settle(&mut st, &ui);
    assert_eq!(st.stack(), vec![PageId::General, PageId::About], "About goes on its pane");
    let nav = st.list().expect("list").nav();
    assert!(nav.has_back() && !nav.has_forward(), "only the back half is enabled");
    let (bx, by) = pill(&st, &ui, false);
    tap(&mut st, &ui, &mut prefs, bx, by);
    assert_eq!(st.stack(), vec![PageId::General], "the back half pops About");
    assert!(st.can_go_forward() && st.list().expect("list").nav().has_forward(), "About is ahead now");
    assert!(!st.list().expect("list").nav().has_back(), "General is the first page: nothing behind it");
    let (fx, fy) = pill(&st, &ui, true);
    tap(&mut st, &ui, &mut prefs, fx, fy);
    assert_eq!(st.stack(), vec![PageId::General, PageId::About], "the forward half opens About again");
    assert!(!st.can_go_forward());
    // a tap on the dimmed forward half does nothing
    tap(&mut st, &ui, &mut prefs, fx, fy);
    assert_eq!(st.page(), PageId::About);
}

/// W3-3: Esc pops a sub-page in the split layout, and is not used at the pane (so the shell leaves it alone).
#[test]
fn escape_pops_a_split_sub_page_and_stops_at_its_pane() {
    let ui = crate::shell::tests::ui();
    let (mut st, _) = split(&ui);
    st.open_page(&ui, "Software Update", 0.0);
    settle(&mut st, &ui);
    assert_eq!(st.stack(), vec![PageId::General, PageId::SoftwareUpdate]);
    assert!(st.back_page(0.0));
    settle(&mut st, &ui);
    assert_eq!(st.stack(), vec![PageId::General]);
    assert!(!st.back_page(0.0), "nothing to pop at the pane");
}

/// A new navigation (a sidebar row, a deep link) clears what was ahead.
#[test]
fn a_new_page_clears_the_forward_history() {
    let ui = crate::shell::tests::ui();
    let (mut st, _) = split(&ui);
    st.open_page(&ui, "About", 0.0);
    settle(&mut st, &ui);
    st.back_page(0.0);
    settle(&mut st, &ui);
    assert!(st.can_go_forward());
    st.open_page(&ui, "Bluetooth", 0.0);
    settle(&mut st, &ui);
    assert!(!st.can_go_forward());
    assert!(!st.forward_page(&ui, 0.0));
}

/// The single column stacks the same way, on the root.
#[test]
fn the_single_column_stacks_a_deep_link_on_its_pane() {
    let ui = crate::shell::tests::ui();
    let (mut st, _) = split(&ui);
    st.allow_split(&ui, false);
    st.open_page(&ui, "About", 0.0);
    settle(&mut st, &ui);
    assert_eq!(st.stack(), vec![PageId::Root, PageId::General, PageId::About]);
}

/// W3-13: with a pointer a click on the empty middle of a switch row did flip the switch; now only the switch or its
/// label does. A finger (the tablet form) still flips it from anywhere on the row.
#[test]
fn a_pointer_flips_a_switch_only_on_the_switch_or_its_label() {
    use bifrost_ui::form_factor::FormFactor;
    let ui = crate::shell::tests::ui();
    let (mut st, mut prefs) = split(&ui);
    st.open_page(&ui, "Focus", 0.0);
    settle(&mut st, &ui);
    let (g, i) = st.list().expect("list").find("Do Not Disturb").expect("the switch row");
    let (rx, ry, rw, rh) = st.list().expect("list").row_rect(&ui, g, i);
    let (px, mid) = (st.pane_x(), ry + rh / 2.0);
    let middle = px + rx + rw * 0.55;
    let switch = px + rx + rw - ui.pt(16.0 + 19.0);
    let before = st.options().dnd;
    super::super::pages::set_test_form(FormFactor::Desktop);
    tap(&mut st, &ui, &mut prefs, middle, mid);
    assert_eq!(st.options().dnd, before, "a click on the row's empty middle does nothing");
    tap(&mut st, &ui, &mut prefs, switch, mid);
    assert_eq!(st.options().dnd, !before, "a click on the switch flips it");
    super::super::pages::set_test_form(FormFactor::Tablet);
    tap(&mut st, &ui, &mut prefs, middle, mid);
    assert_eq!(st.options().dnd, before, "a finger flips it from anywhere on the row");
}

/// The sidebar's draw reads the hits `requery` kept for the query instead of matching the index every frame, and
/// they are the hits a fresh match finds.
#[test]
fn search_hits_are_kept_per_query() {
    let ui = crate::shell::tests::ui();
    let (mut st, _) = split(&ui);
    st.search.active = true;
    st.key(&ui, Out::Text("display".into()), 0.0);
    assert_eq!(st.hits.0, st.search.query(), "kept for the typed query");
    assert!(!st.hits.1.is_empty());
    assert_eq!(st.hit_pages(), st.match_hit_pages());
    st.key(&ui, Out::Text("x".into()), 0.0);
    assert_eq!(st.hits.0, st.search.query(), "a new query replaces them");
    assert_eq!(st.hit_pages(), st.match_hit_pages());
}

/// N3: Return opened the first match even when another hit was highlighted (opened by Down or a tap).
#[test]
fn return_in_search_opens_the_highlighted_hit() {
    let ui = crate::shell::tests::ui();
    let (mut st, _) = split(&ui);
    st.search.active = true;
    st.key(&ui, Out::Text("display".into()), 0.0);
    let hits = st.hit_pages();
    let first = *hits.first().expect("a match");
    let second = *hits.iter().find(|p| **p != first).expect("hits on two pages");
    st.open_page_id(&ui, first, 0.0);
    settle(&mut st, &ui);
    st.key(&ui, Out::Named("Down"), 0.0);
    settle(&mut st, &ui);
    assert_eq!(st.page(), second, "Down opens the next hit");
    assert_eq!(st.side_selected(), second, "and the sidebar fills it");
    st.key(&ui, Out::Named("Up"), 0.0);
    settle(&mut st, &ui);
    assert_eq!(st.page(), first, "Up goes back");
    st.key(&ui, Out::Named("Down"), 0.0);
    settle(&mut st, &ui);
    // Return ends the search field's editing (the keyboard goes away), so it comes last
    st.key(&ui, Out::Enter, 0.0);
    settle(&mut st, &ui);
    assert_eq!(st.page(), second, "Return opens the highlighted hit, not the first");
}

/// A pane that was open before the query is no highlight: Return opens the first hit even when that pane is a hit.
#[test]
fn a_new_query_highlights_its_first_hit() {
    let ui = crate::shell::tests::ui();
    let (mut st, _) = split(&ui);
    st.search.active = true;
    st.key(&ui, Out::Text("display".into()), 0.0);
    let hits = st.hit_pages();
    let second = *hits.iter().find(|p| **p != hits[0]).expect("hits on two pages");
    st.open_page_id(&ui, second, 0.0);
    settle(&mut st, &ui);
    st.key(&ui, Out::Text("s".into()), 0.0);
    assert_eq!(st.highlighted_hit(&st.hit_pages()), None, "a new query clears the highlight");
}

/// N17: the wheel scrolls the list under the pointer itself (the shell then drops the drag emulated for it, which
/// pressed and saved the slider under the pointer).
#[test]
fn the_wheel_scrolls_the_list_under_the_pointer() {
    let ui = crate::shell::tests::ui();
    let (mut st, _) = split(&ui);
    st.set_size(&ui, 1200, 700);
    assert!(st.wheel(10.0, -40.0), "the sidebar takes the wheel");
    assert_eq!(st.sidebar_list().expect("split").offset(), 40.0);
    st.open_page_id(&ui, PageId::Appearance, 0.0);
    settle(&mut st, &ui);
    let before = st.list().expect("list").offset();
    assert!(st.wheel(st.sidebar_px() as f32 + 10.0, -40.0), "the page takes the wheel");
    assert!(st.list().expect("list").offset() > before, "and scrolls down");
    assert_eq!(st.sidebar_list().expect("split").offset(), 40.0, "the sidebar stays");
}

/// A mouse drag in the sidebar or a pane does not scroll it on a desktop; a finger does, and a tablet pans on either.
#[test]
fn a_mouse_drag_does_not_scroll_the_sidebar_or_a_pane_on_a_desktop() {
    use bifrost_ui::form_factor::FormFactor::{Desktop, Tablet};
    use bifrost_ui::pan::{with_touch, TouchSource::{Finger, Pointer}};
    let ui = crate::shell::tests::ui();
    let drag = |x: f32, form, source, pane: bool| {
        let (mut st, mut prefs) = split(&ui);
        st.set_size(&ui, 1200, 700);
        if pane {
            st.open_page_id(&ui, PageId::Appearance, 0.0);
            settle(&mut st, &ui);
        }
        let x = if pane { st.sidebar_px() as f32 + x } else { x };
        with_touch(form, source, || {
            st.touch(&ui, &mut prefs, Event::Down(x, 600.0), 0.0);
            st.touch(&ui, &mut prefs, Event::Move(x, 500.0), 16.0);
            st.touch(&ui, &mut prefs, Event::Move(x, 200.0), 32.0);
            st.touch(&ui, &mut prefs, Event::Up(x, 200.0), 48.0);
        });
        if pane { st.list().expect("list").offset() } else { st.sidebar_list().expect("split").offset() }
    };
    for pane in [false, true] {
        assert_eq!(drag(10.0, Desktop, Pointer, pane), 0.0, "a mouse drag does not scroll (pane: {pane})");
        assert!(drag(10.0, Desktop, Finger, pane) > 0.0, "a finger drag scrolls (pane: {pane})");
        assert!(drag(10.0, Tablet, Pointer, pane) > 0.0, "a tablet pans on any drag (pane: {pane})");
    }
}

/// N4: a window resize rebuilt the sidebar from its top.
#[test]
fn a_resize_keeps_the_sidebar_scroll() {
    let ui = crate::shell::tests::ui();
    let (mut st, _) = split(&ui);
    st.set_size(&ui, 1200, 700);
    st.sidebar.as_mut().expect("split").jump_to(150.0);
    let before = st.sidebar_list().expect("split").offset();
    assert!(before > 0.0, "the sidebar scrolls at this height");
    st.set_size(&ui, 1210, 680);
    assert_eq!(st.sidebar_list().expect("split").offset(), before);
}

/// N12: a new Sidebar icon size moved the sidebar rows only at the next relayout.
#[test]
fn the_sidebar_icon_size_applies_at_once() {
    let ui = crate::shell::tests::ui();
    let keep = bifrost_render::ui::sidebar_icon_size();
    bifrost_render::ui::set_sidebar_icon_size(1);
    // the desktop sidebar has Appearance as a pane of its own (the tablet keeps it under Display & Brightness)
    let desk = Info { display_mode: Some((2560, 1440, 60)), ..info() };
    let mut st = Settings::new(&ui, &desk, &Prefs::memory(), 1200, 1920, Rc::new(vec![]));
    let mut prefs = Prefs::memory();
    st.open_page_id(&ui, PageId::Appearance, 0.0);
    settle(&mut st, &ui);
    let (_, before) = st.sidebar_centre(&ui, "Appearance").expect("the Appearance row");
    let (x, y) = st.row_centre(&ui, "Sidebar icon size").expect("the size row");
    tap(&mut st, &ui, &mut prefs, x, y);
    st.activate(&ui, &mut prefs, Hit::Choice(2), 0.0);
    settle(&mut st, &ui);
    let (_, after) = st.sidebar_centre(&ui, "Appearance").expect("the Appearance row");
    bifrost_render::ui::set_sidebar_icon_size(keep);
    assert!(after > before, "Large rows move the row down at once: {before} -> {after}");
}

#[test]
fn arrow_keys_step_over_the_hits_of_the_highlighted_page() {
    let hits = [PageId::Display, PageId::Display, PageId::NightShift, PageId::About];
    assert_eq!(step_hit(&hits, None, true), Some(PageId::NightShift), "Down from the default highlight on the first");
    assert_eq!(step_hit(&hits, None, false), Some(PageId::About), "Up with nothing chosen opens the last hit");
    assert_eq!(step_hit(&[], None, true), None);
    assert_eq!(step_hit(&[], None, false), None);
    assert_eq!(step_hit(&hits, Some(0), true), Some(PageId::NightShift));
    assert_eq!(step_hit(&hits, Some(2), false), Some(PageId::Display));
    assert_eq!(step_hit(&hits, Some(3), true), None);
    assert_eq!(step_hit(&hits, Some(0), false), None);
}

/// E2E13 5: after Return opened Bluetooth from Search the back half of the pill did nothing; it returns to the pane
/// that was open before, and the first hit is highlighted before Return.
#[test]
fn back_returns_from_a_search_opened_pane_to_the_previous_one() {
    let ui = crate::shell::tests::ui();
    let (mut st, mut prefs) = split(&ui);
    st.open_page(&ui, "Sound", 0.0);
    settle(&mut st, &ui);
    assert_eq!(st.stack(), vec![PageId::Sounds], "Sound is a pane of its own");
    assert!(!st.list().expect("list").nav().has_back(), "a pane opened by itself has nothing behind it");
    st.search.active = true;
    st.key(&ui, Out::Text("bluet".into()), 0.0);
    let first = *st.hit_pages().first().expect("a match");
    assert_eq!(first, PageId::Bluetooth);
    assert_eq!(st.side_selected(), first, "the first hit is highlighted before Return");
    st.key(&ui, Out::Enter, 0.0);
    settle(&mut st, &ui);
    assert_eq!(st.stack(), vec![PageId::Bluetooth]);
    assert!(st.list().expect("list").nav().has_back(), "the pill's back half is enabled");
    let (bx, by) = pill(&st, &ui, false);
    tap(&mut st, &ui, &mut prefs, bx, by);
    assert_eq!(st.stack(), vec![PageId::Sounds], "Back returns to the pane open before the search");
    assert!(!st.list().expect("list").nav().has_back(), "and nothing is behind that one");
    assert!(!st.back_page(0.0));
    let hits = st.hit_pages();
    assert!(!hits.contains(&PageId::Sounds));
    assert_eq!(st.highlighted_hit(&hits), None, "the Bluetooth hit is not shown, so it is not highlighted");
    assert!(!hits.contains(&st.side_selected()), "no hit row is highlighted beside Sound");
}

/// The sidebar has the keyboard when the window opens: Up and Down move the selection over its rows and open each
/// pane (the ends stay put), and the keys that type go nowhere.
#[test]
fn up_and_down_move_the_sidebar_selection() {
    let ui = crate::shell::tests::ui();
    let (mut st, _) = split(&ui);
    let pages = st.sidebar_list().expect("sidebar").push_pages();
    assert!(pages.len() >= 3, "the sidebar has rows to step over: {pages:?}");
    st.select(&ui, pages[0], 0.0);
    settle(&mut st, &ui);
    assert!(!st.search.active && !st.text_focused(), "the sidebar, not the search field, has the keyboard");
    st.key(&ui, Out::Named("Down"), 0.0);
    settle(&mut st, &ui);
    assert_eq!(st.side_selected(), pages[1], "Down selects the next row");
    assert_eq!(st.stack().first(), Some(&pages[1]), "and opens its pane");
    st.key(&ui, Out::Named("Down"), 0.0);
    settle(&mut st, &ui);
    assert_eq!(st.side_selected(), pages[2]);
    st.key(&ui, Out::Named("Up"), 0.0);
    settle(&mut st, &ui);
    assert_eq!(st.side_selected(), pages[1], "Up goes back one");
    st.key(&ui, Out::Named("Up"), 0.0);
    st.key(&ui, Out::Named("Up"), 0.0);
    settle(&mut st, &ui);
    assert_eq!(st.side_selected(), pages[0], "the first row stays selected");
    assert_eq!(st.stack().first(), Some(&pages[0]));
    let last = *pages.last().unwrap();
    st.select(&ui, last, 0.0);
    settle(&mut st, &ui);
    st.key(&ui, Out::Named("Down"), 0.0);
    assert_eq!(st.side_selected(), last, "the last row stays selected");
    st.key(&ui, Out::Text("x".into()), 0.0);
    assert_eq!(st.search.query(), "", "typing with the sidebar focused does not reach the search field");
}

/// Tab moves the keyboard from the sidebar through the detail pane's controls to the search field and back, each stop with
/// its focus ring; Shift+Tab goes the other way round.
#[test]
fn tab_cycles_focus_through_the_sidebar_the_pane_and_the_search_field() {
    let ui = crate::shell::tests::ui();
    let (mut st, _) = split(&ui);
    st.select(&ui, PageId::Display, 0.0);
    settle(&mut st, &ui);
    let controls = st.list().expect("list").focusable();
    assert!(controls.len() >= 3, "Display has a slider, links and a switch to focus: {controls:?}");
    assert!(!st.side_ring, "no ring before a key is used");
    // Tab from the sidebar: the pane's first control, with the ring on its row
    st.key(&ui, Out::Named("Tab"), 0.0);
    assert_eq!(st.valid_pane_focus(), Some(controls[0]), "Tab from the sidebar focuses the pane's first control");
    assert_eq!(st.list().unwrap().focus(), Some(controls[0]), "its row shows the ring");
    assert!(!st.side_ring && !st.search.active);
    // Tab steps through the controls in order
    for c in &controls[1..] {
        st.key(&ui, Out::Named("Tab"), 0.0);
        assert_eq!(st.valid_pane_focus(), Some(*c));
    }
    // past the last control: the search field (its own ring), the pane's ring gone
    st.key(&ui, Out::Named("Tab"), 0.0);
    assert!(st.search.active && st.text_focused(), "Tab from the last control focuses the search field");
    assert_eq!(st.valid_pane_focus(), None);
    assert_eq!(st.list().unwrap().focus(), None, "the pane's ring is gone");
    st.key(&ui, Out::Text("disp".into()), 0.0);
    assert_eq!(st.search.query(), "disp", "typing goes to the field");
    st.key(&ui, Out::Named("Tab"), 0.0);
    assert!(!st.search.active && !st.text_focused(), "Tab from the field returns to the sidebar");
    assert!(st.side_ring, "the sidebar's selected row shows the ring");
    assert_eq!(st.search.query(), "disp", "the query is kept");
    // Shift+Tab: back to the field, then to the pane's last control, up to the first, and out to the sidebar
    st.key(&ui, Out::Named("BackTab"), 0.0);
    assert!(st.search.active, "Shift+Tab from the sidebar focuses the search field");
    st.key(&ui, Out::Named("BackTab"), 0.0);
    assert_eq!(st.valid_pane_focus(), controls.last().copied(), "Shift+Tab from the field focuses the pane's last control");
    for c in controls[..controls.len() - 1].iter().rev() {
        st.key(&ui, Out::Named("BackTab"), 0.0);
        assert_eq!(st.valid_pane_focus(), Some(*c));
    }
    st.key(&ui, Out::Named("BackTab"), 0.0);
    assert!(st.side_ring && st.valid_pane_focus().is_none(), "Shift+Tab from the first control returns to the sidebar");
}

/// Space and Return press the focused control, Down and Up step between controls, Left and Right nudge a slider, and a touch
/// takes the focus away.
#[test]
fn the_pane_control_with_the_focus_is_pressed_by_space_and_return() {
    let ui = crate::shell::tests::ui();
    let (mut st, mut prefs) = split(&ui);
    st.select(&ui, PageId::Display, 0.0);
    settle(&mut st, &ui);
    let (bg, bi) = st.list().unwrap().find("Bold Text").expect("the Bold Text row");
    let controls = st.list().unwrap().focusable();
    let n = controls.len();
    for _ in 0..=n {
        if st.valid_pane_focus() == Some((bg, bi)) {
            break;
        }
        st.key_with_prefs(&ui, &mut prefs, Out::Named("Tab"), 0.0);
    }
    assert_eq!(st.valid_pane_focus(), Some((bg, bi)), "Tab reached the switch");
    assert!(!st.options().bold_text);
    st.key_with_prefs(&ui, &mut prefs, Out::Text(" ".into()), 1.0);
    assert!(st.options().bold_text, "Space flips the focused switch");
    assert_eq!(st.valid_pane_focus(), Some((bg, bi)), "the focus stays on the row");
    assert_eq!(st.list().unwrap().focus(), Some((bg, bi)), "and the rebuilt page still shows the ring");
    st.key_with_prefs(&ui, &mut prefs, Out::Enter, 2.0);
    assert!(!st.options().bold_text, "Return flips it back");
    // Up and Down step between controls without leaving the pane
    let k = controls.iter().position(|c| *c == (bg, bi)).unwrap();
    st.key_with_prefs(&ui, &mut prefs, Out::Named("Up"), 3.0);
    assert_eq!(st.valid_pane_focus(), Some(controls[k - 1]));
    assert_eq!(st.side_selected(), PageId::Display, "Up in the pane does not move the sidebar's selection");
    st.key_with_prefs(&ui, &mut prefs, Out::Named("Down"), 3.0);
    assert_eq!(st.valid_pane_focus(), Some((bg, bi)));
    // Left and Right nudge the brightness slider (the first control)
    for _ in 0..n {
        if st.valid_pane_focus() == Some(controls[0]) {
            break;
        }
        st.key_with_prefs(&ui, &mut prefs, Out::Named("Up"), 4.0);
    }
    assert_eq!(st.valid_pane_focus(), Some(controls[0]));
    let before = st.brightness();
    let (key, expected) = if before > 0.5 { ("Left", before - 0.1) } else { ("Right", before + 0.1) };
    let fx = st.key_with_prefs(&ui, &mut prefs, Out::Named(key), 5.0);
    assert!((st.brightness() - expected).abs() < 0.01, "{key} moves the slider a tenth: {before} -> {}", st.brightness());
    assert!(fx.contains(&Effect::Brightness(st.brightness())));
    // a touch takes the focus away
    let x = st.side_w as f32 + 40.0;
    tap(&mut st, &ui, &mut prefs, x, 400.0);
    assert_eq!(st.valid_pane_focus(), None, "a touch removes the keyboard focus from the pane");
    assert_eq!(st.list().unwrap().focus(), None);
}

/// The ring has a row to surround, a touch takes it away, and arrows over search results step the hits from the sidebar.
#[test]
fn a_touch_clears_the_ring_and_arrows_step_search_hits_from_the_sidebar() {
    let ui = crate::shell::tests::ui();
    let (mut st, mut prefs) = split(&ui);
    st.key(&ui, Out::Named("Down"), 0.0);
    assert!(st.side_ring);
    let (_, y, w, h) = st.sidebar_list().unwrap().selected_plate(&ui).expect("the selected row has a plate");
    assert!(w > 0.0 && h > 0.0 && y + h > 0.0 && y < st.h as f32, "the ring has a row to surround: y {y} h {h}");
    let x = st.side_w as f32 + 40.0;
    tap(&mut st, &ui, &mut prefs, x, 400.0);
    assert!(!st.side_ring, "a touch removes the keyboard ring");
    st.search.active = true;
    st.key(&ui, Out::Text("display".into()), 0.0);
    let hits = st.hit_pages();
    let last = *hits.last().expect("a match");
    st.key(&ui, Out::Named("Tab"), 0.0);
    assert!(!st.search.active);
    st.key(&ui, Out::Named("Up"), 0.0);
    settle(&mut st, &ui);
    assert_eq!(st.side_selected(), last, "Up over the results opens the last hit");
}

/// Up in Search with no hit chosen yet opens the last hit (the first one's highlight is only a cue).
#[test]
fn up_with_no_hit_chosen_opens_the_last_hit() {
    let ui = crate::shell::tests::ui();
    let (mut st, _) = split(&ui);
    st.search.active = true;
    st.key(&ui, Out::Text("display".into()), 0.0);
    let hits = st.hit_pages();
    let last = *hits.last().expect("a match");
    assert_eq!(st.side_selected(), hits[0], "the first hit shows highlighted");
    st.key(&ui, Out::Named("Up"), 0.0);
    settle(&mut st, &ui);
    assert_eq!(st.side_selected(), last, "Up opened the last hit");
}

/// Esc walks back over a search-opened pane too, and a sidebar row starts a new history.
#[test]
fn a_sidebar_row_clears_the_pane_history() {
    let ui = crate::shell::tests::ui();
    let (mut st, _) = split(&ui);
    st.open_page(&ui, "Sound", 0.0);
    settle(&mut st, &ui);
    st.search.active = true;
    st.key(&ui, Out::Text("bluet".into()), 0.0);
    st.key(&ui, Out::Enter, 0.0);
    settle(&mut st, &ui);
    assert!(st.back_page(0.0), "Esc goes back to Sound");
    settle(&mut st, &ui);
    assert_eq!(st.stack(), vec![PageId::Sounds]);
    st.open_page_id(&ui, PageId::Bluetooth, 0.0);
    settle(&mut st, &ui);
    assert_eq!(st.stack(), vec![PageId::Bluetooth], "the hit opens again over Sound");
    assert!(st.list().expect("list").nav().has_back());
    st.select(&ui, PageId::General, 0.0);
    settle(&mut st, &ui);
    assert!(!st.back_page(0.0), "a sidebar row has nothing behind it");
}

/// W3-11: Return in the search field did nothing; it opens the first match.
#[test]
fn return_in_search_opens_the_first_match() {
    let ui = crate::shell::tests::ui();
    let (mut st, _) = split(&ui);
    st.search.active = true;
    st.key(&ui, Out::Text("about".into()), 0.0);
    let first = settings_search::first_hit(st.index.as_deref().unwrap_or(&[]), "about").expect("a match");
    st.key(&ui, Out::Enter, 0.0);
    settle(&mut st, &ui);
    assert_eq!(st.page(), first, "Return opened the first match");
    assert_eq!(first, PageId::About);
}

/// The Output volume slider's value on the open Sound page.
fn sound_slider(st: &Settings) -> f32 {
    let spec = st.list().expect("list").spec();
    let g = spec.groups.iter().find(|g| g.header.as_deref() == Some("OUTPUT VOLUME")).expect("the output volume group");
    match g.items[0].body {
        super::super::list::Body::Slider { value, .. } => value,
        _ => panic!("not a slider"),
    }
}

/// W3-9: Sound showed the saved "volume" pref (0) while the system played at 0.644; it shows the live level, and
/// follows it while open.
#[test]
fn sound_shows_and_follows_the_live_output_level() {
    let ui = crate::shell::tests::ui();
    let (mut st, _) = split(&ui);
    st.set_audio(&ui, 0.644, false);
    st.open_page(&ui, "Sound", 0.0);
    settle(&mut st, &ui);
    assert_eq!(st.page(), PageId::Sounds);
    assert!((sound_slider(&st) - 0.644).abs() < 1e-4, "the live level");
    st.set_audio(&ui, 0.25, false);
    assert!((sound_slider(&st) - 0.25).abs() < 1e-4, "a level set by the keys or Control Center shows at once");
    let spec = st.list().expect("list").spec();
    assert!(spec.groups.iter().any(|g| g.header.as_deref() == Some("OUTPUT") && !g.items.is_empty()), "the output devices are listed");
    assert!(spec.groups[0].items.iter().any(|i| i.label == "Play sound effects through"));
}

/// W3-12: the desktop sidebar was one flat list; it follows the Golden Gate groups, with Appearance and Keyboard as
/// panes of their own.
#[test]
fn the_desktop_sidebar_follows_the_golden_gate_groups() {
    let ui = crate::shell::tests::ui();
    let desk = Info { display_mode: Some((2560, 1440, 60)), ..info() };
    let mut st = Settings::new(&ui, &desk, &Prefs::memory(), 1200, 1920, Rc::new(vec![]));
    let groups: Vec<Vec<String>> = st.sidebar_list().expect("split").spec().groups.iter().map(|g| g.items.iter().map(|i| i.label.clone()).collect()).collect();
    assert_eq!(groups[0], ["BifrOSt Account"]);
    assert_eq!(groups[1][..2], ["Wi-Fi", "Bluetooth"], "connections first; no Airplane Mode on a desktop");
    assert_eq!(groups[2], ["General", "Sharing", "Accessibility", "Appearance", "Desktop & Dock", "Overview", "Displays", "Menu Bar", "Control Center", "Assistant", "Search", "Wallpaper", "Screen Saver"]);
    assert_eq!(groups[3], ["Notifications", "Sound", "Focus", "Screen Time"]);
    assert_eq!(groups[4][..2], ["Lock Screen", "Privacy & Security"]);
    assert_eq!(groups[4].last().map(String::as_str), Some("Users & Groups"));
    assert_eq!(groups[5], ["Passwords", "Internet Accounts"]);
    assert_eq!(groups[6], ["Keyboard", "Keyboard Shortcuts", "Mouse", "Trackpad", "Printers & Scanners"]);
    assert_eq!(groups.last().expect("groups"), &["Developer"]);
    // Appearance and Keyboard open as first-level panes (their sidebar row selected), not under Displays or General
    st.open_page(&ui, "Appearance", 0.0);
    settle(&mut st, &ui);
    assert_eq!(st.stack(), vec![PageId::Appearance]);
    st.open_page(&ui, "Keyboard", 0.0);
    settle(&mut st, &ui);
    assert_eq!(st.stack(), vec![PageId::Keyboard]);
    // the tablet's root list keeps its own order
    let tablet = Settings::new(&ui, &info(), &Prefs::memory(), 1200, 1920, Rc::new(vec![]));
    let first = tablet.sidebar_list().expect("split").spec().groups[1].items[0].label.clone();
    assert_eq!(first, "Airplane Mode");
}

/// W3-3: a Search result in the sidebar that is a page inside a pane (Night Shift) opens on its pane, with a back
/// target, instead of as a first page with none.
#[test]
fn a_sidebar_search_hit_inside_a_pane_has_a_back_target() {
    let ui = crate::shell::tests::ui();
    let (mut st, mut prefs) = split(&ui);
    st.search.active = true;
    st.key(&ui, Out::Text("night shift".into()), 0.0);
    let label = st.sidebar_list().expect("split").spec().groups[0].items[0].label.clone();
    let want = settings_search::first_hit(st.index.as_deref().unwrap_or(&[]), "night shift").expect("a match");
    assert_eq!(want, PageId::NightShift);
    let (x, y) = st.sidebar_centre(&ui, &label).expect("the result row");
    tap(&mut st, &ui, &mut prefs, x, y);
    assert_eq!(st.page(), PageId::NightShift);
    assert!(st.stack().len() >= 2, "Night Shift sits on its pane: {:?}", st.stack());
    assert!(st.list().expect("list").nav().has_back());
    assert!(st.back_page(0.0), "Esc goes back to the pane");
}
#[test]
fn dock_changes_from_a_filled_settings_window_use_the_output_size() {
    use crate::shell::{AppId, Event, desktop_dock::{Change, Position}, desk::Desk};
    let mut sh = crate::shell::tests::desktop_shell();
    sh.desk_open(AppId::Settings, 1000.0);
    let m = sh.desk_metrics();
    let index = sh.desk.as_ref().unwrap().index_of(AppId::Settings).unwrap();
    sh.desk.as_mut().unwrap().toggle_zoom(&m, index);
    for change in [Change::Position(Position::Left), Change::Position(Position::Right), Change::Position(Position::Bottom), Change::Autohide] {
        let m = sh.desk_metrics();
        let body = sh.desk.as_ref().unwrap().wins[index].body(&m);
        // This queue is consumed during a real Settings content event, inside with_size before the fix.
        sh.settings.as_mut().unwrap().dock_out.push(change);
        sh.desk_event(Event::Down((body.x + body.w / 2) as f32, (body.y + body.h / 2) as f32), 2000.0);
        let expected = Desk::area(&sh.desk_metrics());
        assert_eq!(sh.desk.as_ref().unwrap().wins[index].r, expected);
    }
}
