//! Parity sdk-gaps-04: headless tests of the pane Help menu entries, the scene hand-off to an app-owned renderer and the
//! photo adjustment records. Nothing here opens a window or draws; the scene tests only read the viewport in pixels.

use bifrost_ui::help_menu::{HelpEntry, PaneHelpMenu};
use bifrost_ui::photo_adjustment::{AdjustmentData, EditError, EditSession};
use bifrost_ui::scene_handoff::{SceneHandoff, Viewport};

// Help menu entries of a settings pane.

#[test]
fn an_entry_needs_a_title_and_a_blank_anchor_is_refused() {
    assert_eq!(HelpEntry::new("  ", None), None);
    assert_eq!(HelpEntry::new("Privacy help", Some(" ")), None);
    let e = HelpEntry::new("  Privacy help ", Some(" Privacy_Camera ")).unwrap();
    assert_eq!(e.title, "Privacy help");
    assert_eq!(e.anchor.as_deref(), Some("Privacy_Camera"));
    assert_eq!(HelpEntry::new("Network help", None).unwrap().anchor, None);
}

#[test]
fn a_new_help_menu_has_no_pane_entries() {
    let menu = PaneHelpMenu::new();
    assert!(menu.is_empty());
    assert_eq!(menu.entries().count(), 0);
}

#[test]
fn selecting_a_pane_adds_its_entries_and_unselecting_removes_them() {
    let mut menu = PaneHelpMenu::new();
    assert!(menu.update("displays", vec![HelpEntry::new("Display help", None).unwrap()]));
    assert_eq!(menu.entries().count(), 1);
    assert!(menu.remove("displays"));
    assert!(!menu.remove("displays"), "a pane with no entries has nothing to remove");
    assert!(menu.is_empty());
}

#[test]
fn updating_a_pane_replaces_its_entries_and_moves_it_last() {
    let mut menu = PaneHelpMenu::new();
    menu.update("displays", vec![HelpEntry::new("Display help", None).unwrap()]);
    menu.update("security", vec![HelpEntry::new("Security help", None).unwrap()]);
    menu.update("displays", vec![HelpEntry::new("Arrangement help", None).unwrap()]);
    let titles: Vec<&str> = menu.entries().map(|e| e.title.as_str()).collect();
    assert_eq!(titles, vec!["Security help", "Arrangement help"]);
}

#[test]
fn a_blank_pane_identifier_is_refused_without_change() {
    let mut menu = PaneHelpMenu::new();
    assert!(!menu.update("  ", vec![HelpEntry::new("Help", None).unwrap()]));
    assert!(menu.is_empty());
}

#[test]
fn entry_titled_finds_the_first_match_ignoring_case() {
    let mut menu = PaneHelpMenu::new();
    menu.update("displays", vec![HelpEntry::new("Display help", Some("displays_brightness")).unwrap()]);
    menu.update("sound", vec![HelpEntry::new("Display help", None).unwrap()]);
    let found = menu.entry_titled(" DISPLAY HELP ").unwrap();
    assert_eq!(found.anchor.as_deref(), Some("displays_brightness"));
    assert_eq!(menu.entry_titled("Sound help"), None);
}

// Scene hand-off to an app-owned renderer.

#[test]
fn a_hand_off_needs_a_positive_finite_scale() {
    assert!(SceneHandoff::new(100.0, 50.0, 0.0).is_none());
    assert!(SceneHandoff::new(100.0, 50.0, -2.0).is_none());
    assert!(SceneHandoff::new(100.0, 50.0, f32::NAN).is_none());
    assert_eq!(SceneHandoff::new(100.0, 50.0, 2.0).unwrap().scale(), 2.0);
}

#[test]
fn set_scale_refuses_values_that_are_not_positive_and_finite() {
    let mut h = SceneHandoff::new(100.0, 50.0, 1.0).unwrap();
    assert!(!h.set_scale(0.0));
    assert!(!h.set_scale(f32::INFINITY));
    assert_eq!(h.scale(), 1.0);
    assert!(h.set_scale(2.0));
    assert_eq!(h.scale(), 2.0);
}

#[test]
fn no_render_pass_exists_before_the_scene_has_been_updated() {
    let mut h = SceneHandoff::new(100.0, 50.0, 1.0).unwrap();
    assert_eq!(h.render_pass(), None);
    h.present();
    assert_eq!(h.render_pass(), None, "presented but not yet updated");
    assert!(h.update(0.0).is_some());
    assert!(h.render_pass().is_some());
}

#[test]
fn the_viewport_is_the_scene_size_in_device_pixels() {
    let mut h = SceneHandoff::new(320.0, 200.0, 2.0).unwrap();
    h.present();
    h.update(0.0);
    let pass = h.render_pass().unwrap();
    assert_eq!(pass.viewport, Viewport { x: 0, y: 0, width: 640, height: 400 });
}

#[test]
fn device_pixels_round_to_the_nearest_pixel() {
    let mut h = SceneHandoff::new(10.4, 10.6, 1.0).unwrap();
    h.present();
    h.update(0.0);
    let pass = h.render_pass().unwrap();
    assert_eq!((pass.viewport.width, pass.viewport.height), (10, 11));
}

#[test]
fn a_zero_sized_scene_has_no_render_pass() {
    let mut h = SceneHandoff::new(0.0, 200.0, 1.0).unwrap();
    h.present();
    h.update(0.0);
    assert_eq!(h.render_pass(), None);
}

#[test]
fn the_render_pass_reports_the_scene_time_after_the_last_update() {
    let mut h = SceneHandoff::new(10.0, 10.0, 1.0).unwrap();
    h.present();
    h.update(1.0);
    h.update(1.05);
    let pass = h.render_pass().unwrap();
    assert!((pass.scene_time - 0.05).abs() < 1e-9);
}

#[test]
fn dismissing_the_scene_refuses_further_render_passes() {
    let mut h = SceneHandoff::new(10.0, 10.0, 1.0).unwrap();
    h.present();
    h.update(0.0);
    assert!(h.dismiss());
    assert_eq!(h.render_pass(), None);
    assert_eq!(h.update(1.0), None, "a dismissed scene runs no steps");
}

#[test]
fn presenting_again_needs_a_new_update_before_a_render_pass() {
    let mut h = SceneHandoff::new(10.0, 10.0, 1.0).unwrap();
    h.present();
    h.update(0.0);
    h.dismiss();
    h.present();
    assert_eq!(h.render_pass(), None);
}

#[test]
fn resize_reports_whether_the_scene_changed_size() {
    let mut h = SceneHandoff::new(10.0, 10.0, 1.0).unwrap();
    assert!(!h.resize(10.0, 10.0));
    assert!(h.resize(20.0, 10.0));
    h.present();
    h.update(0.0);
    assert_eq!(h.render_pass().unwrap().viewport.width, 20);
}

// Adjustment records of edited photos.

fn record(identifier: &str) -> AdjustmentData {
    AdjustmentData::new(identifier, "1.0", vec![1, 2, 3]).unwrap()
}

#[test]
fn a_record_needs_a_format_identifier_and_a_version() {
    assert_eq!(AdjustmentData::new(" ", "1.0", vec![]), None);
    assert_eq!(AdjustmentData::new("crop.editor", "", vec![]), None);
    let r = AdjustmentData::new(" crop.editor ", " 2 ", vec![9]).unwrap();
    assert_eq!(r.format_identifier(), "crop.editor");
    assert_eq!(r.format_version(), "2");
    assert_eq!(r.data(), [9u8].as_slice());
}

#[test]
fn a_session_needs_a_format_identifier() {
    assert!(EditSession::begin("  ", None).is_none());
    assert!(EditSession::begin("tone.editor", None).is_some());
}

#[test]
fn an_untouched_photo_keeps_the_record_it_had() {
    let session = EditSession::begin("tone.editor", Some(record("tone.editor"))).unwrap();
    assert_eq!(session.input(), Some(&record("tone.editor")));
    assert_eq!(session.result(), Some(&record("tone.editor")));
    assert!(session.has_adjustments());
}

#[test]
fn a_record_from_another_editor_format_is_not_offered_as_the_input() {
    let session = EditSession::begin("tone.editor", Some(record("crop.editor"))).unwrap();
    assert_eq!(session.input(), None);
    assert!(!session.has_adjustments());
}

#[test]
fn a_committed_record_replaces_the_input() {
    let mut session = EditSession::begin("tone.editor", Some(record("tone.editor"))).unwrap();
    let next = AdjustmentData::new("tone.editor", "1.0", vec![7]).unwrap();
    assert_eq!(session.commit(next.clone()), Ok(()));
    assert_eq!(session.result(), Some(&next));
}

#[test]
fn a_record_from_another_format_is_refused_and_changes_nothing() {
    let mut session = EditSession::begin("tone.editor", None).unwrap();
    assert_eq!(session.commit(record("crop.editor")), Err(EditError::FormatMismatch));
    assert_eq!(session.result(), None);
}

#[test]
fn reverting_returns_the_photo_to_its_original() {
    let mut session = EditSession::begin("tone.editor", Some(record("tone.editor"))).unwrap();
    session.commit(record("tone.editor")).unwrap();
    session.revert();
    assert_eq!(session.result(), None);
    assert!(!session.has_adjustments());
}

#[test]
fn a_commit_after_a_revert_makes_the_edit_again() {
    let mut session = EditSession::begin("tone.editor", None).unwrap();
    session.revert();
    session.commit(record("tone.editor")).unwrap();
    assert_eq!(session.result(), Some(&record("tone.editor")));
}
