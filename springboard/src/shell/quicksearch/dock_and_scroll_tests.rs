use super::*;
use crate::shell::keyboard::Out;
use crate::shell::Event;

fn ui() -> Ui {
    crate::shell::tests::ui()
}

/// A desktop shell with Search open and "se" typed (several kinds of result).
fn typed() -> Shell {
    let mut sh = crate::shell::tests::desktop_shell();
    sh.anim = None;
    sh.screen = Screen::Home;
    sh.open_quicksearch_hw(1000.0);
    sh.quicksearch_tick(2000.0);
    assert!(sh.quicksearch.is_shown());
    sh.quicksearch.key(&sh.ui, sh.h, Out::Text("se".into()), 2100.0);
    sh
}

/// The Dock top that leaves room for exactly `rows` result rows under the chips.
fn dock_for_rows(sh: &Shell, rows: f32) -> f32 {
    let qs = &sh.quicksearch;
    let m = qs.m();
    qs.px(&sh.ui, m.card_top + m.field) + qs.list_fixed(&sh.ui) + rows * qs.px(&sh.ui, m.row) + sh.ui.pt(MAC_DOCK_MARGIN_PT)
}

fn row_h(sh: &Shell) -> f32 {
    sh.quicksearch.px(&sh.ui, sh.quicksearch.m().row)
}

#[test]
fn the_dock_limits_the_card_on_the_desktop_and_the_screen_otherwise() {
    let mut sh = crate::shell::tests::desktop_shell();
    let top = sh.quicksearch_dock_top().expect("the Dock stands along the bottom and shows");
    assert!(top > 0.0 && top < sh.h as f32, "the Dock's top is on the screen");
    sh.desk.as_mut().unwrap().dock_preferences.position = crate::shell::desktop_dock::Position::Left;
    assert_eq!(sh.quicksearch_dock_top(), None, "a Dock on a side does not limit the height");
}

#[test]
fn the_results_card_stays_above_the_dock_however_many_results() {
    let mut sh = typed();
    let dock = sh.quicksearch_dock_top().unwrap();
    let h = sh.h;
    let margin = sh.ui.pt(MAC_DOCK_MARGIN_PT);
    // from the real Dock to one that leaves four rows, three rows and one row
    for floor in [dock, dock_for_rows(&sh, 4.0), dock_for_rows(&sh, 3.0), dock_for_rows(&sh, 1.0)] {
        sh.quicksearch.set_dock_top(&sh.ui, h, Some(floor));
        let (_, cy, _, ch) = sh.quicksearch.card(&sh.ui, sh.w);
        assert!(cy + ch <= floor - margin + 0.5, "the card ends {} px above the Dock's top {floor}", floor - cy - ch);
        for (_, (_, ry, _, rh)) in sh.quicksearch.row_rects(&sh.ui, sh.w) {
            assert!(ry + rh <= cy + ch, "a row shows only inside the card");
        }
    }
    // the Dock hiding gives the screen's bottom back
    sh.quicksearch.set_dock_top(&sh.ui, h, None);
    let (_, cy, _, ch) = sh.quicksearch.card(&sh.ui, sh.w);
    assert!(cy + ch <= h as f32 - margin + 0.5);
}

#[test]
fn rows_beyond_the_card_scroll_with_the_wheel_and_do_not_tap_through() {
    let mut sh = typed();
    let floor = dock_for_rows(&sh, 3.0);
    sh.quicksearch.set_dock_top(&sh.ui, sh.h, Some(floor));
    let row = row_h(&sh);
    let qs = &sh.quicksearch;
    assert!(qs.rows_h(&sh.ui) > qs.viewport_h(&sh.ui) + row, "more results than the card shows");
    assert!((qs.viewport_h(&sh.ui) - 3.0 * row).abs() < 1.0, "the card shows three rows");
    assert!(qs.row_rects(&sh.ui, sh.w).len() < qs.rows().len(), "rows below the card's edge cannot be hit");
    assert_eq!(qs.scroll, 0.0);
    // a wheel turned down (negative content travel) scrolls the list
    assert!(sh.quicksearch_wheel(-row, 3000.0));
    assert!((sh.quicksearch.scroll - row).abs() < 0.5);
    // the drag emulated for the notch lands on a row and must not open it
    let (_, (x, y, w, rh)) = sh.quicksearch.row_rects(&sh.ui, sh.w)[0];
    sh.event(Event::Down(x + w / 2.0, y + rh / 2.0), 3000.0);
    sh.event(Event::Move(x + w / 2.0, y + rh / 2.0 - 3.0), 3016.0);
    sh.event(Event::Up(x + w / 2.0, y + rh / 2.0 - 3.0), 3032.0);
    assert!(sh.quicksearch.is_shown(), "the emulated drag is dropped");
    // far past the end stops at the last row
    sh.quicksearch_wheel(-100_000.0, 4000.0);
    let qs = &sh.quicksearch;
    assert!((qs.scroll - qs.max_scroll(&sh.ui)).abs() < 0.5);
    let last = qs.row_rects(&sh.ui, sh.w).last().unwrap().0;
    assert_eq!(last, (qs.sections.len() - 1, qs.sections.last().unwrap().rows.len() - 1), "the last result shows at the end");
    // and back to the top
    sh.quicksearch_wheel(100_000.0, 5000.0);
    assert_eq!(sh.quicksearch.scroll, 0.0);
}

#[test]
fn the_arrow_keys_keep_the_selected_row_in_view() {
    let mut sh = typed();
    let floor = dock_for_rows(&sh, 3.0);
    sh.quicksearch.set_dock_top(&sh.ui, sh.h, Some(floor));
    let (all, row) = (sh.quicksearch.rows().len(), row_h(&sh));
    for i in 0..all {
        sh.quicksearch.key(&sh.ui, sh.h, Out::Named("Down"), 3000.0 + i as f64);
        let qs = &sh.quicksearch;
        let at = qs.selected.expect("Down selects a row");
        let rect = qs.row_rects(&sh.ui, sh.w).into_iter().find(|(a, _)| *a == at).expect("the selected row is in view").1;
        assert!((rect.3 - row).abs() < 0.5, "the selected row shows whole");
    }
    assert!(sh.quicksearch.scroll > 0.0, "the list followed the selection down");
    for i in 0..all {
        sh.quicksearch.key(&sh.ui, sh.h, Out::Named("Up"), 4000.0 + i as f64);
        let qs = &sh.quicksearch;
        let at = qs.selected.expect("Up keeps a row selected");
        assert!(qs.row_rects(&sh.ui, sh.w).iter().any(|(a, r)| *a == at && (r.3 - row).abs() < 0.5), "the selected row shows whole");
    }
    assert_eq!(sh.quicksearch.scroll, 0.0, "the first row is back at the top");
}

/// Two rows (an app, then a Terminal search) in two sections, as "calc" lists them.
fn two_rows(q: &mut QuickSearch, u: &Ui) -> (Hit, Hit) {
    let (a, b) = (Hit::App(AppId::Settings), Hit::Terminal("calc".into()));
    let section = |title: &str, hit: &Hit| Section { header: u.text(title, 20.0, Weight::Semibold, WHITE), rows: vec![(hit.clone(), u.text("row", 13.0, Weight::Regular, WHITE))], subs: vec![None] };
    q.sections = vec![section("Top Hit", &a), section("Search", &b)];
    (a, b)
}

#[test]
fn down_then_return_opens_the_second_result() {
    let u = ui();
    for mac in [true, false] {
        let mut q = QuickSearch::new(&u, 800.0, 1.0);
        q.mac = mac;
        let (first, second) = two_rows(&mut q, &u);
        assert_eq!(q.key(&u, 800, Out::Enter, 0.0), Some(first.clone()), "no arrow yet: Return opens the Top Hit");
        two_rows(&mut q, &u);
        assert_eq!(q.key(&u, 800, Out::Named("Down"), 0.0), None);
        if !mac {
            // the bifrost has no pre-highlighted row: the first Down takes the first one
            assert_eq!(q.selected, Some((0, 0)));
            q.key(&u, 800, Out::Named("Down"), 0.0);
        }
        assert_eq!(q.selected, Some((1, 0)), "Down left the Top Hit for the second row (mac {mac})");
        assert_eq!(q.key(&u, 800, Out::Enter, 0.0), Some(second), "Return opens the highlighted row");
        q.key(&u, 800, Out::Named("Up"), 0.0);
        assert_eq!(q.selected, Some((0, 0)));
    }
}

#[test]
fn a_rebuild_for_the_same_query_keeps_the_arrow_selection() {
    let mut sh = typed();
    assert!(sh.quicksearch.rows().len() > 2);
    sh.quicksearch.key(&sh.ui, sh.h, Out::Named("Down"), 3000.0);
    let at = sh.quicksearch.selected.expect("Down selects a row");
    assert_ne!(at, (0, 0), "the Top Hit is already highlighted, so Down goes past it");
    let hit = sh.quicksearch.selected_hit();
    sh.quicksearch.search(&sh.ui, sh.h);
    assert_eq!(sh.quicksearch.selected, Some(at), "late rows or a new Dock do not reset the highlight");
    assert_eq!(sh.quicksearch.selected_hit(), hit);
    sh.quicksearch.key(&sh.ui, sh.h, Out::Text("t".into()), 3100.0);
    assert_eq!(sh.quicksearch.selected, None, "a new query highlights the Top Hit again");
}

#[test]
fn cmd_down_and_up_jump_between_categories() {
    let u = ui();
    let mut q = QuickSearch::new(&u, 800.0, 1.0);
    let (_, second) = two_rows(&mut q, &u);
    assert!(q.jump_section(&u, 1));
    assert_eq!(q.selected_hit(), Some(second));
    assert!(q.jump_section(&u, 1));
    assert_eq!(q.selected, Some((1, 0)), "clamped at the last category");
    assert!(q.jump_section(&u, -1));
    assert_eq!(q.selected, Some((0, 0)));
    q.sections.clear();
    assert!(!q.jump_section(&u, 1), "nothing to jump through");
}

#[test]
fn a_new_query_scrolls_back_to_the_top() {
    let mut sh = typed();
    let floor = dock_for_rows(&sh, 3.0);
    sh.quicksearch.set_dock_top(&sh.ui, sh.h, Some(floor));
    sh.quicksearch_wheel(-200.0, 3000.0);
    assert!(sh.quicksearch.scroll > 0.0);
    sh.quicksearch.key(&sh.ui, sh.h, Out::Text("t".into()), 3100.0);
    assert_eq!(sh.quicksearch.scroll, 0.0);
}

/// E2E6 1: an app opened from Search on the desktop is the front app. Search held the keyboard exclusively, so as it
/// closes the window server hands the keyboard back to the window it still knows in front (the launched app has no
/// toplevel until it draws); that hand-back must not lift Settings over the app (Calculator opened under Settings).
#[cfg(feature = "desktop-wayland")]
#[test]
fn an_app_opened_from_search_on_the_desktop_stays_in_front_when_search_closes() {
    let mut sh = super::super::tests::desktop_shell();
    sh.screen = Screen::Home;
    sh.anim = None;
    let _peer = super::super::tests::fixture_app(&mut sh);
    sh.desk_open(AppId::Settings, 0.0);
    let _ = sh.quicksearch_act(Hit::App(AppId::External(0)), 1.0);
    assert!(!sh.quicksearch.active(), "Search closes as the app opens");
    assert_eq!(super::super::tests::hand_back_after_launch(&mut sh), (Some(AppId::External(0)), Screen::App(AppId::External(0))));
}
