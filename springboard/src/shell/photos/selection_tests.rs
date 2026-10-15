//! The grid's selection: a Shift tap selects the run from the last photo tapped, and Image > Duplicate copies
//! every selected photo.

use super::commands::PhotoCommand;
use super::tests::{photos, pump_until};
use super::*;

fn paths(p: &Photos) -> Vec<PathBuf> {
    p.photos().iter().map(|q| q.path.clone()).collect()
}

#[test]
fn a_shift_tap_selects_the_run_from_the_last_tap() {
    let (mut p, _dir, ui) = photos("select-run", 5);
    p.activate(&ui, Target::Select, 0.0);
    let view = paths(&p);
    p.activate(&ui, Target::Cell(1), 0.0);
    p.set_shift(true);
    p.activate(&ui, Target::Cell(3), 0.0);
    p.set_shift(false);
    let mut want = vec![view[1].clone(), view[2].clone(), view[3].clone()];
    want.sort();
    assert_eq!(p.selection.paths(), want);
    p.activate(&ui, Target::Cell(0), 0.0);
    assert!(p.selection.contains(&view[0]), "a plain tap still toggles one photo");
    assert_eq!(p.selection.len(), 4);
}

#[test]
fn a_shift_tap_with_no_earlier_tap_toggles_one_photo() {
    let (mut p, _dir, ui) = photos("select-first", 3);
    p.activate(&ui, Target::Select, 0.0);
    let view = paths(&p);
    p.set_shift(true);
    p.activate(&ui, Target::Cell(2), 0.0);
    assert_eq!(p.selection.paths(), vec![view[2].clone()]);
}

#[test]
fn leaving_selection_forgets_the_last_tap() {
    let (mut p, _dir, ui) = photos("select-anchor", 4);
    p.activate(&ui, Target::Select, 0.0);
    p.activate(&ui, Target::Cell(0), 0.0);
    p.activate(&ui, Target::Select, 0.0);
    assert!(!p.selection.active);
    p.activate(&ui, Target::Select, 0.0);
    let view = paths(&p);
    p.set_shift(true);
    p.activate(&ui, Target::Cell(2), 0.0);
    assert_eq!(p.selection.paths(), vec![view[2].clone()], "the old tap is not an anchor any more");
}

#[test]
fn duplicate_copies_every_selected_photo() {
    let (mut p, _dir, ui) = photos("select-duplicate", 3);
    assert!(!p.command_enabled(PhotoCommand::Duplicate), "nothing is selected yet");
    p.activate(&ui, Target::Select, 0.0);
    p.activate(&ui, Target::Cell(0), 0.0);
    p.activate(&ui, Target::Cell(2), 0.0);
    assert!(p.command_enabled(PhotoCommand::Duplicate));
    p.command(&ui, PhotoCommand::Duplicate, 0.0);
    pump_until(&mut p, &ui, |p| p.all.len() == 5);
}
