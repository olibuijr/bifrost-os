//! Parity sdk-tests-08: headless tests of the Photos viewer models: the zoom and pan arithmetic, the grid layout and its
//! hit tests, and the viewer's pinch, double tap and single tap. Pure state and geometry; nothing is drawn and no
//! display, clock or input device is used. Times are passed in as milliseconds.

use springboard::shell::photos::grid::{Grid, RowKind};
use springboard::shell::photos::library::Section;
use springboard::shell::photos::viewer::{Action, Viewer};
use springboard::shell::photos::zoom::{self, DOUBLE_TAP_ZOOM, MAX_ZOOM, MIN_ZOOM};

fn section(start: usize, len: usize) -> Section {
    Section { title: "May".into(), start, len }
}

// Zoom and pan arithmetic.

#[test]
fn max_pan_is_half_of_what_overflows_the_viewport() {
    assert_eq!(zoom::max_pan(100.0, 2.0, 100.0), 50.0);
    assert_eq!(zoom::max_pan(100.0, 1.0, 100.0), 0.0, "a picture that fits cannot be panned");
}

#[test]
fn clamp_pan_limits_each_axis_to_its_own_overflow() {
    assert_eq!(zoom::clamp_pan((80.0, -80.0), (100.0, 100.0), 2.0, (100.0, 100.0)), (50.0, -50.0));
    assert_eq!(zoom::clamp_pan((10.0, 20.0), (100.0, 100.0), 2.0, (100.0, 100.0)), (10.0, 20.0));
}

#[test]
fn a_settled_zoom_stays_between_fit_and_the_maximum() {
    assert_eq!(zoom::settle_zoom(0.5), MIN_ZOOM);
    assert_eq!(zoom::settle_zoom(9.0), MAX_ZOOM);
    assert_eq!(zoom::settle_zoom(2.5), 2.5);
}

#[test]
fn a_zoom_inside_the_limits_is_free_and_beyond_them_is_stiff() {
    assert_eq!(zoom::soft_zoom(2.0), 2.0);
    let under = zoom::soft_zoom(0.5);
    assert!((0.7..1.0).contains(&under), "{under}");
    let over = zoom::soft_zoom(10.0);
    assert!(over > MAX_ZOOM && over <= MAX_ZOOM + 1.0, "{over}");
}

#[test]
fn a_pinch_about_the_viewport_centre_keeps_the_pan_still() {
    assert_eq!(zoom::pinch(1.0, (0.0, 0.0), 2.0, (50.0, 50.0), (100.0, 100.0)), (2.0, 0.0, 0.0));
}

#[test]
fn a_pinch_about_the_top_left_moves_the_pan_to_keep_that_point_under_the_fingers() {
    assert_eq!(zoom::pinch(1.0, (0.0, 0.0), 2.0, (0.0, 0.0), (100.0, 100.0)), (2.0, 50.0, 50.0));
}

#[test]
fn a_double_tap_zooms_in_from_fit_and_back_out_from_zoomed() {
    assert_eq!(zoom::double_tap_target(MIN_ZOOM), DOUBLE_TAP_ZOOM);
    assert_eq!(zoom::double_tap_target(DOUBLE_TAP_ZOOM), MIN_ZOOM);
}

#[test]
fn zooming_to_fit_recentres_the_picture() {
    let pan = zoom::zoom_to(MIN_ZOOM, 2.0, (30.0, 40.0), (50.0, 50.0), (100.0, 100.0), (100.0, 100.0));
    assert_eq!(pan, (0.0, 0.0));
}

#[test]
fn the_snap_back_halves_by_the_frame_rate_and_snaps_small_offsets_to_zero() {
    assert_eq!(zoom::decay(0.3, 1.0), 0.0, "under half a pixel is zero");
    assert_eq!(zoom::decay(10.0, 0.0), 10.0, "no time, no movement");
    assert!((zoom::decay(10.0, 1.0 / 60.0) - 7.5).abs() < 1e-3);
    assert!((zoom::decay(10.0, 2.0 / 60.0) - 5.625).abs() < 1e-3);
}

// Grid layout.

#[test]
fn a_grid_lays_out_a_header_then_rows_of_square_cells() {
    // 320 px, 3 columns, 4 px gaps: cells of floor((320 - 8) / 3) = 104 px.
    let grid = Grid::build(&[section(0, 5)], 3, 320.0, 4.0, 30.0);
    assert_eq!(grid.cell, 104.0);
    assert_eq!(grid.height, 246.0);
    assert_eq!(grid.rows().len(), 3);
    assert_eq!(grid.rows()[0].kind, RowKind::Header(0));
    assert_eq!(grid.rows()[1].kind, RowKind::Cells { first: 0, count: 3 });
    assert_eq!(grid.rows()[2].kind, RowKind::Cells { first: 3, count: 2 });
}

#[test]
fn a_cell_rectangle_places_each_photo_by_its_column_and_row() {
    let grid = Grid::build(&[section(0, 5)], 3, 320.0, 4.0, 30.0);
    assert_eq!(grid.cell_rect(0), (0.0, 30.0, 104.0, 104.0));
    assert_eq!(grid.cell_rect(4), (108.0, 138.0, 104.0, 104.0));
}

#[test]
fn a_hit_finds_the_photo_under_a_point_and_misses_the_gaps_and_headers() {
    let grid = Grid::build(&[section(0, 5)], 3, 320.0, 4.0, 30.0);
    assert_eq!(grid.hit(10.0, 40.0), Some(0));
    assert_eq!(grid.hit(110.0, 40.0), Some(1));
    assert_eq!(grid.hit(106.0, 40.0), None, "the gap between two cells");
    assert_eq!(grid.hit(10.0, 10.0), None, "the section header");
    assert_eq!(grid.hit(220.0, 140.0), None, "an empty cell in a short row");
    assert_eq!(grid.hit(110.0, 140.0), Some(4));
}

#[test]
fn visible_returns_the_photos_whose_rows_intersect_the_window() {
    let grid = Grid::build(&[section(0, 5)], 3, 320.0, 4.0, 30.0);
    assert_eq!(grid.visible(0.0, 100.0), vec![0, 1, 2]);
    assert_eq!(grid.visible(140.0, 300.0), vec![3, 4]);
}

#[test]
fn headers_report_each_section_title_row_in_the_window() {
    let grid = Grid::build(&[section(0, 2), section(2, 1)], 3, 320.0, 4.0, 30.0);
    assert_eq!(grid.headers(0.0, 300.0), vec![(0, 0.0), (1, 138.0)]);
    assert_eq!(grid.headers(0.0, 100.0), vec![(0, 0.0)]);
    assert_eq!(grid.height, 276.0);
}

#[test]
fn an_empty_grid_has_no_rows_and_no_height() {
    let grid = Grid::build(&[], 3, 320.0, 4.0, 30.0);
    assert!(grid.rows().is_empty());
    assert_eq!(grid.height, 0.0);
    assert_eq!(grid.visible(0.0, 100.0), Vec::<usize>::new());
}

// Viewer gestures.

#[test]
fn a_viewer_opens_on_its_photo_at_fit() {
    let viewer = Viewer::new(2, 5, 320.0, 480.0);
    assert_eq!(viewer.index, 2);
    assert_eq!(viewer.count, 5);
    assert_eq!(viewer.zoom(), MIN_ZOOM);
    assert_eq!(viewer.pan(), (0.0, 0.0));
    assert!(viewer.chrome, "the bars start shown");
}

#[test]
fn a_pinch_that_ends_zoomed_in_keeps_the_zoom_and_centres_the_pan() {
    let mut viewer = Viewer::new(0, 3, 320.0, 480.0);
    viewer.pinch(2.0, (160.0, 240.0));
    viewer.end_pinch();
    assert_eq!(viewer.zoom(), 2.0);
    assert_eq!(viewer.pan(), (0.0, 0.0));
}

#[test]
fn a_pinch_that_ends_below_fit_settles_back_to_fit() {
    let mut viewer = Viewer::new(0, 3, 320.0, 480.0);
    viewer.pinch(0.5, (160.0, 240.0));
    viewer.end_pinch();
    assert_eq!(viewer.zoom(), MIN_ZOOM);
    assert_eq!(viewer.pan(), (0.0, 0.0));
}

#[test]
fn a_double_tap_within_the_window_zooms_about_the_tapped_point() {
    let mut viewer = Viewer::new(0, 3, 320.0, 480.0);
    viewer.down(100.0, 100.0, 0.0);
    assert_eq!(viewer.up(100.0, 100.0, 50.0), Action::None, "a first tap waits for a second");
    viewer.down(100.0, 100.0, 200.0);
    assert_eq!(viewer.up(100.0, 100.0, 250.0), Action::Zoomed);
    assert_eq!(viewer.zoom(), DOUBLE_TAP_ZOOM);
    assert_eq!(viewer.pan(), (60.0, 140.0));
}

#[test]
fn a_second_double_tap_returns_the_viewer_to_fit() {
    let mut viewer = Viewer::new(0, 3, 320.0, 480.0);
    viewer.down(100.0, 100.0, 0.0);
    viewer.up(100.0, 100.0, 50.0);
    viewer.down(100.0, 100.0, 200.0);
    assert_eq!(viewer.up(100.0, 100.0, 250.0), Action::Zoomed);
    viewer.down(100.0, 100.0, 400.0);
    viewer.up(100.0, 100.0, 450.0);
    viewer.down(100.0, 100.0, 500.0);
    assert_eq!(viewer.up(100.0, 100.0, 520.0), Action::Zoomed);
    assert_eq!(viewer.zoom(), MIN_ZOOM);
    assert_eq!(viewer.pan(), (0.0, 0.0));
}

#[test]
fn taps_further_apart_than_the_double_tap_window_do_not_zoom() {
    let mut viewer = Viewer::new(0, 3, 320.0, 480.0);
    viewer.down(100.0, 100.0, 0.0);
    viewer.up(100.0, 100.0, 0.0);
    viewer.down(100.0, 100.0, 500.0);
    assert_eq!(viewer.up(100.0, 100.0, 510.0), Action::None);
    assert_eq!(viewer.zoom(), MIN_ZOOM);
}

#[test]
fn a_single_tap_toggles_the_bars_once_the_double_tap_window_has_passed() {
    let mut viewer = Viewer::new(0, 3, 320.0, 480.0);
    viewer.down(10.0, 10.0, 0.0);
    assert_eq!(viewer.up(10.0, 10.0, 0.0), Action::None);
    assert_eq!(viewer.tick(299.0), Action::None);
    assert_eq!(viewer.tick(300.0), Action::ToggleChrome);
    assert!(!viewer.chrome);
}

#[test]
fn a_tap_that_moves_is_not_a_tap() {
    let mut viewer = Viewer::new(0, 3, 320.0, 480.0);
    viewer.down(10.0, 10.0, 0.0);
    viewer.moved(200.0, 10.0, 10.0);
    assert_eq!(viewer.up(200.0, 10.0, 20.0), Action::None);
    assert_eq!(viewer.tick(600.0), Action::None);
    assert!(viewer.chrome);
}
