use super::commands::PhotoCommand;
use super::tests::photos;
use super::*;
use crate::shell::hwkeys::{HwKeys, Shortcut};

/// The evdev code of the Left Meta (Cmd) key.
const KEY_LEFTMETA: u16 = 125;

#[test]
fn select_all_and_deselect_all_work_on_the_grid() {
    let (mut p, _dir, ui) = photos("cmd-select", 3);
    assert!(p.command_enabled(PhotoCommand::SelectAll));
    assert!(!p.command_enabled(PhotoCommand::DeselectAll), "nothing is selected yet");
    p.command(&ui, PhotoCommand::SelectAll, 0.0);
    assert_eq!(p.selection.len(), 3);
    assert!(p.selection.active);
    p.command(&ui, PhotoCommand::DeselectAll, 0.0);
    assert!(p.selection.active && p.selection.is_empty(), "Deselect All keeps the selection mode on");
}

#[test]
fn a_menu_rating_is_one_undoable_step() {
    let (mut p, _dir, ui) = photos("cmd-rate", 2);
    p.open_viewer(0);
    let photo = p.viewing().cloned().unwrap();
    p.command(&ui, PhotoCommand::Rate(4), 0.0);
    assert_eq!(p.rating(&photo), 4);
    assert_eq!(p.command_title(PhotoCommand::Undo), "Undo Rating");
    p.command(&ui, PhotoCommand::Undo, 0.0);
    assert_eq!(p.rating(&photo), 0);
}

#[test]
fn favorite_toggles_its_title_with_the_mark() {
    let (mut p, _dir, ui) = photos("cmd-favourite", 2);
    p.open_viewer(0);
    assert_eq!(p.command_title(PhotoCommand::Favourite), "Favorite");
    p.command(&ui, PhotoCommand::Favourite, 0.0);
    assert!(p.is_favourite(&p.viewing().cloned().unwrap()));
    assert_eq!(p.command_title(PhotoCommand::Favourite), "Unfavorite");
}

#[test]
fn rotate_turns_the_open_photo_and_turning_back_leaves_no_edit() {
    let (mut p, _dir, ui) = photos("cmd-rotate", 1);
    p.open_viewer(0);
    let photo = p.viewing().cloned().unwrap();
    p.command(&ui, PhotoCommand::RotateRight, 0.0);
    assert_eq!(p.recipe(&photo).map(|e| e.turns), Some(1));
    p.command(&ui, PhotoCommand::RotateLeft, 0.0);
    assert!(p.recipe(&photo).is_none(), "an unedited photo stores no edit");
}

#[test]
fn zoom_and_filter_commands_follow_the_grid() {
    let (mut p, _dir, ui) = photos("cmd-view", 3);
    assert_eq!(p.cols(), 5);
    p.command(&ui, PhotoCommand::ZoomIn, 0.0);
    assert_eq!(p.cols(), 3, "Zoom In makes the thumbnails bigger");
    p.command(&ui, PhotoCommand::ZoomOut, 0.0);
    assert_eq!(p.cols(), 5);
    p.command(&ui, PhotoCommand::FavouritesOnly, 0.0);
    assert_eq!(p.search(), "is:favorite");
    assert!(p.command_checked(PhotoCommand::FavouritesOnly));
    p.command(&ui, PhotoCommand::ShowAll, 0.0);
    assert_eq!(p.search(), "");
}

#[test]
fn commands_with_nothing_to_act_on_are_greyed_out() {
    let (mut p, _dir, _ui) = photos("cmd-grey", 2);
    assert!(!p.command_enabled(PhotoCommand::Rate(3)), "no photo is open or selected");
    assert!(!p.command_enabled(PhotoCommand::Print));
    p.open_viewer(0);
    assert!(p.command_enabled(PhotoCommand::Print));
    assert!(!p.command_enabled(PhotoCommand::SelectAll), "the viewer is not the grid");
}

#[test]
fn print_hands_the_open_photo_over_once() {
    let (mut p, _dir, ui) = photos("cmd-print", 1);
    p.open_viewer(0);
    p.command(&ui, PhotoCommand::Print, 0.0);
    let image = p.take_print().expect("the picture decodes");
    assert!(image.w > 0 && image.h > 0);
    assert!(p.take_print().is_none(), "the picture is handed over once");
}

#[test]
fn the_hidden_album_can_be_shown_while_it_is_empty() {
    let (mut p, _dir, ui) = photos("cmd-hidden", 1);
    assert!(!p.menu_albums().contains(&Album::Hidden));
    p.command(&ui, PhotoCommand::ShowHidden, 0.0);
    assert!(p.command_checked(PhotoCommand::ShowHidden));
    assert!(p.menu_albums().contains(&Album::Hidden));
    p.command(&ui, PhotoCommand::ShowHidden, 0.0);
    assert!(!p.menu_albums().contains(&Album::Hidden));
}

#[test]
fn cmd_keys_are_photo_commands_only_while_photos_is_in_front() {
    let mut keys = HwKeys::default();
    assert_eq!(keys.key_front(KEY_LEFTMETA, true, false, true), None);
    assert_eq!(keys.key_front(33, true, false, true), Some(Shortcut::Photo(PhotoCommand::Find)));
    let mut keys = HwKeys::default();
    assert_eq!(keys.key_front(KEY_LEFTMETA, true, false, false), None);
    assert_eq!(keys.key_front(33, true, false, false), None, "Find belongs to Photos alone");
}
