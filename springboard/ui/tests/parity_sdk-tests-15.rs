//! Parity sdk-tests-15: headless model tests for the Today widget protocol (`widget`, `widget_list`), the Picture in
//! Picture controller and placement (`pip_window`) and the onscreen entity annotations (`onscreen_entity`). Nothing here
//! draws or opens a window: each test drives the pure state machines and geometry through their public API.

use bifrost_ui::onscreen_entity::{AnnotationError, EntityRef, OnscreenContext, ViewAnnotation};
use bifrost_ui::pip_window::{self, Controller as PipController, Corner, Event, State};
use bifrost_ui::widget::{self, Controller as WidgetController, DisplayMode, Insets, Limits};
use bifrost_ui::widget_list::WidgetList;
use bifrost_ui::Rect;

fn near(a: (f32, f32), b: (f32, f32)) -> bool {
    (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01
}

fn ids(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
}

fn note(id: &str) -> EntityRef {
    EntityRef { type_name: "Note".to_string(), id: id.to_string() }
}

// --- widget: display modes, margins and the content flag -----------------------------------------------------------

#[test]
fn headless_the_more_button_switches_a_widget_between_compact_and_expanded() {
    assert_eq!(DisplayMode::default(), DisplayMode::Compact);
    assert_eq!(DisplayMode::Compact.toggled(), DisplayMode::Expanded);
    assert_eq!(DisplayMode::Expanded.toggled(), DisplayMode::Compact);
}

#[test]
fn a_widget_keeps_the_proposed_margins_unless_it_returns_its_own() {
    let proposed = Insets::uniform(8.0);
    assert_eq!(widget::margin_insets(proposed, None), proposed);
    let custom = Insets { top: 0.0, left: 4.0, bottom: 12.0, right: 4.0 };
    assert_eq!(widget::margin_insets(proposed, Some(custom)), custom);
    assert_eq!(Insets::uniform(3.5), Insets { top: 3.5, left: 3.5, bottom: 3.5, right: 3.5 });
}

#[test]
fn each_display_mode_reports_its_own_height_at_the_column_width() {
    let limits = Limits::new(342.0, 110.0, 340.0);
    assert!(near(limits.max_size(DisplayMode::Compact), (342.0, 110.0)));
    assert!(near(limits.max_size(DisplayMode::Expanded), (342.0, 340.0)));
}

#[test]
fn an_expanded_height_is_raised_to_the_compact_height_when_it_is_shorter() {
    let limits = Limits::new(300.0, 120.0, 80.0);
    assert_eq!(limits.expanded_h, 120.0);
    assert!(near(limits.max_size(DisplayMode::Expanded), (300.0, 120.0)));
}

#[test]
fn a_widget_that_never_reported_has_content_and_is_shown() {
    let c = WidgetController::default();
    assert!(c.has_content("is.example.weather"));
    assert_eq!(c.visible(&ids(&["is.example.weather"])), vec!["is.example.weather"]);
}

#[test]
fn an_empty_widget_is_left_out_and_the_others_keep_their_order() {
    let mut c = WidgetController::default();
    c.set_has_content("is.example.b", false);
    let order = ids(&["is.example.a", "is.example.b", "is.example.c"]);
    assert_eq!(c.visible(&order), vec!["is.example.a", "is.example.c"]);
    assert!(!c.has_content("is.example.b"));
}

#[test]
fn a_widget_reporting_content_again_is_shown_in_its_original_place() {
    let mut c = WidgetController::default();
    let order = ids(&["is.example.a", "is.example.b", "is.example.c"]);
    c.set_has_content("is.example.b", false);
    c.set_has_content("is.example.b", true);
    assert_eq!(c.visible(&order), vec!["is.example.a", "is.example.b", "is.example.c"]);
}

#[test]
fn a_report_for_one_widget_leaves_the_others_alone() {
    let mut c = WidgetController::default();
    c.set_has_content("is.example.b", false);
    assert!(c.has_content("is.example.a"));
    assert!(c.has_content("is.example.c"));
}

#[test]
fn forgetting_a_removed_app_makes_its_widget_visible_again() {
    let mut c = WidgetController::default();
    c.set_has_content("is.example.b", false);
    c.forget("is.example.b");
    assert!(c.has_content("is.example.b"));
    assert_eq!(c.visible(&ids(&["is.example.b"])), vec!["is.example.b"]);
}

// --- widget_list: rows, visible count, editing ---------------------------------------------------------------------

fn rows(n: usize, row_h: f32) -> WidgetList<String> {
    let mut l = WidgetList::new(row_h);
    l.set_contents((0..n).map(|i| format!("row {i}")).collect());
    l
}

#[test]
fn a_row_is_found_by_its_content_and_content_is_read_by_row() {
    let l = rows(3, 44.0);
    assert_eq!(l.row_for(&"row 1".to_string()), Some(1));
    assert_eq!(l.row_for(&"missing".to_string()), None);
    assert_eq!(l.item_at(2).map(String::as_str), Some("row 2"));
    assert_eq!(l.item_at(3), None);
}

#[test]
fn the_rows_that_fit_the_height_are_shown_and_never_more_than_the_content() {
    assert_eq!(rows(10, 44.0).visible_rows(132.0), 3, "132 / 44 = 3 rows fit");
    assert_eq!(rows(10, 44.0).visible_rows(1000.0), 10, "capped at the content count");
    assert_eq!(rows(2, 44.0).visible_rows(1000.0), 2);
    assert_eq!(rows(10, 44.0).visible_rows(20.0), 0, "less than one row of room");
}

#[test]
fn a_row_height_of_zero_or_less_shows_every_row() {
    assert_eq!(rows(4, 0.0).visible_rows(10.0), 4);
    assert_eq!(rows(4, -1.0).visible_rows(0.0), 4);
}

#[test]
fn the_minimum_row_count_beats_a_short_height_but_not_the_content() {
    let mut l = rows(10, 44.0);
    l.minimum_visible_rows = 4;
    assert_eq!(l.visible_rows(44.0), 4);
    let mut short = rows(2, 44.0);
    short.minimum_visible_rows = 4;
    assert_eq!(short.visible_rows(44.0), 2);
}

#[test]
fn the_add_button_needs_both_editing_and_the_list_asking_for_it() {
    let mut l = rows(1, 44.0);
    assert!(!l.add_button_visible());
    l.editing = true;
    assert!(!l.add_button_visible());
    l.shows_add_button_when_editing = true;
    assert!(l.add_button_visible());
    l.editing = false;
    assert!(!l.add_button_visible(), "the button leaves with editing");
}

#[test]
fn content_is_removed_only_while_the_list_is_editing() {
    let mut l = rows(3, 44.0);
    assert_eq!(l.delete(0), None, "not editing: nothing is removed");
    assert_eq!(l.contents().len(), 3);
    l.editing = true;
    assert_eq!(l.delete(1), Some("row 1".to_string()));
    assert_eq!(l.contents(), &["row 0".to_string(), "row 2".to_string()]);
    assert_eq!(l.delete(5), None, "a row past the end");
}

#[test]
fn deleting_a_row_keeps_the_order_of_the_rows_that_remain() {
    let mut l = rows(4, 44.0);
    l.editing = true;
    assert_eq!(l.delete(0), Some("row 0".to_string()));
    assert_eq!(l.delete(0), Some("row 1".to_string()));
    assert_eq!(l.row_for(&"row 3".to_string()), Some(1));
    assert_eq!(l.visible_rows(1000.0), 2);
}

// --- pip_window: the controller's life cycle -----------------------------------------------------------------------

fn screen() -> Rect {
    Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 }
}

fn active_pip() -> PipController {
    let mut c = PipController::new();
    c.set_possible(true);
    c.start();
    c.finish_start();
    c
}

#[test]
fn headless_a_suspension_hides_the_window_without_ending_it() {
    let mut c = active_pip();
    c.suspend(true);
    assert!(c.is_suspended());
    assert!(c.is_active(), "the window is still the active one");
    assert_eq!(c.stop(), Some(Event::WillStop), "it can still be stopped while suspended");
    assert_eq!(c.finish_stop(), Some(Event::DidStop));
}

#[test]
fn losing_the_possibility_while_starting_takes_the_window_down() {
    let mut c = PipController::new();
    c.set_possible(true);
    assert_eq!(c.start(), Some(Event::WillStart));
    assert_eq!(c.set_possible(false), Some(Event::DidStop));
    assert_eq!(c.state(), State::Inline);
    assert_eq!(c.finish_start(), None, "the start was abandoned");
}

#[test]
fn losing_the_possibility_while_stopping_leaves_the_stop_to_finish() {
    let mut c = active_pip();
    assert_eq!(c.stop(), Some(Event::WillStop));
    assert_eq!(c.set_possible(false), None);
    assert_eq!(c.state(), State::Stopping);
    assert_eq!(c.finish_stop(), Some(Event::DidStop));
    assert_eq!(c.state(), State::Inline);
}

#[test]
fn finishing_a_stop_needs_a_stopping_window() {
    let mut c = PipController::new();
    c.set_possible(true);
    assert_eq!(c.finish_stop(), None, "inline");
    c.start();
    assert_eq!(c.finish_stop(), None, "still starting");
    assert_eq!(c.stop(), None, "a starting window is not stopped");
}

#[test]
fn a_new_controller_starts_inline_with_the_linear_flag_off() {
    let c = PipController::default();
    assert_eq!(c.state(), State::Inline);
    assert!(!c.is_possible() && !c.is_suspended() && !c.is_active());
    assert!(!c.requires_linear_playback);
    assert!(!c.start_automatically);
}

#[test]
fn automatic_start_follows_the_background_only_while_the_flag_is_on() {
    let mut c = PipController::new();
    c.set_possible(true);
    c.start_automatically = true;
    c.suspend(true);
    assert_eq!(c.enter_background(true), None, "suspended by the system");
    c.suspend(false);
    assert_eq!(c.enter_background(true), Some(Event::WillStart));
    assert_eq!(c.enter_background(true), None, "already starting");
}

// --- pip_window: size and placement -------------------------------------------------------------------------------

#[test]
fn a_screen_with_no_height_gives_an_empty_window() {
    assert!(near(pip_window::window_size(0.0, 0.0, 16.0 / 9.0), (0.0, 0.0)));
}

#[test]
fn a_non_finite_screen_width_counts_as_no_width() {
    assert!(near(pip_window::window_size(f32::INFINITY, 1080.0, 16.0 / 9.0), (240.0, 135.0)));
}

#[test]
fn a_tall_video_is_limited_by_half_the_screen_height() {
    // A quarter of 1920 is 480, but 480 / (9 / 16) = 853 points tall is over the 500 limit, so the width drops to 281.25.
    assert!(near(pip_window::window_size(1920.0, 1000.0, 9.0 / 16.0), (281.25, 500.0)));
}

#[test]
fn the_default_corner_is_bottom_right() {
    assert_eq!(Corner::default(), Corner::BottomRight);
    let r = pip_window::place(screen(), (480.0, 270.0), Corner::default());
    assert_eq!(r, Rect { x: 1424.0, y: 794.0, w: 480.0, h: 270.0 });
}

#[test]
fn the_window_keeps_the_edge_margin_inside_a_screen_with_a_menu_bar() {
    let usable = Rect { x: 0.0, y: 24.0, w: 1920.0, h: 1056.0 };
    let top_left = pip_window::place(usable, (480.0, 270.0), Corner::TopLeft);
    assert_eq!((top_left.x, top_left.y), (16.0, 40.0), "16 points below the menu bar");
    let bottom_right = pip_window::place(usable, (480.0, 270.0), Corner::BottomRight);
    assert_eq!((bottom_right.x, bottom_right.y), (1424.0, 794.0));
}

#[test]
fn a_dropped_window_snaps_to_the_corner_nearest_the_drop_point() {
    let usable = Rect { x: 0.0, y: 24.0, w: 1920.0, h: 1056.0 };
    assert_eq!(pip_window::nearest_corner(usable, 100.0, 40.0), Corner::TopLeft);
    assert_eq!(pip_window::nearest_corner(usable, 1800.0, 40.0), Corner::TopRight);
    assert_eq!(pip_window::nearest_corner(usable, 100.0, 1000.0), Corner::BottomLeft);
    assert_eq!(pip_window::nearest_corner(usable, 1800.0, 1000.0), Corner::BottomRight);
}

// --- onscreen_entity: annotations per view ------------------------------------------------------------------------

#[test]
fn the_annotations_of_visible_views_come_in_view_number_order_with_selection() {
    let mut ctx = OnscreenContext::new();
    ctx.annotate(20, note("b"), true).unwrap();
    ctx.annotate(10, note("a"), false).unwrap();
    assert_eq!(
        ctx.view_annotations(),
        vec![
            ViewAnnotation { entity: note("a"), is_selected: false },
            ViewAnnotation { entity: note("b"), is_selected: true },
        ]
    );
}

#[test]
fn a_hidden_view_is_left_out_of_the_query_until_it_is_shown_again() {
    let mut ctx = OnscreenContext::new();
    ctx.annotate(1, note("a"), false).unwrap();
    ctx.annotate(2, note("b"), false).unwrap();
    assert!(ctx.set_visible(2, false));
    assert_eq!(ctx.view_annotations().len(), 1);
    assert!(ctx.set_visible(2, true));
    assert_eq!(ctx.view_annotations().len(), 2);
}

#[test]
fn annotating_a_hidden_view_again_makes_it_visible_with_the_new_entity() {
    let mut ctx = OnscreenContext::new();
    ctx.annotate(4, note("old"), false).unwrap();
    assert!(ctx.set_visible(4, false));
    ctx.annotate(4, note("new"), true).unwrap();
    assert_eq!(ctx.view_annotations(), vec![ViewAnnotation { entity: note("new"), is_selected: true }]);
}

#[test]
fn visibility_and_selection_of_an_unannotated_view_are_refused() {
    let mut ctx = OnscreenContext::new();
    assert!(!ctx.set_visible(9, false));
    assert!(!ctx.set_selected(9, true));
    assert!(!ctx.remove(9));
}

#[test]
fn removing_a_hidden_view_drops_its_annotation_for_good() {
    let mut ctx = OnscreenContext::new();
    ctx.annotate(1, note("a"), false).unwrap();
    ctx.annotate(2, note("b"), false).unwrap();
    assert!(ctx.set_visible(1, false));
    assert!(ctx.remove(1));
    assert!(!ctx.set_visible(1, true), "nothing to show any more");
    assert_eq!(ctx.view_annotations().iter().map(|a| a.entity.id.as_str()).collect::<Vec<_>>(), ["b"]);
}

#[test]
fn two_views_may_show_the_same_entity() {
    let mut ctx = OnscreenContext::new();
    ctx.annotate(1, note("same"), false).unwrap();
    ctx.annotate(2, note("same"), true).unwrap();
    let got = ctx.view_annotations();
    assert_eq!(got.len(), 2);
    assert!(!got[0].is_selected && got[1].is_selected);
}

#[test]
fn a_blank_entity_type_or_identifier_is_refused_with_its_own_error() {
    let mut ctx = OnscreenContext::new();
    let blank_type = EntityRef { type_name: " ".to_string(), id: "x".to_string() };
    assert_eq!(ctx.annotate(1, blank_type, false), Err(AnnotationError::EmptyType));
    assert_eq!(ctx.annotate(1, note("  "), false), Err(AnnotationError::EmptyId));
    assert!(ctx.view_annotations().is_empty(), "a refused annotation leaves nothing behind");
}
