//! Parity sdk-tests-11: headless tests of the notes file store as a model container: fetch (load), insert and update
//! (save), delete, id allocation and the debounced autosave timer. Every test works in its own directory under the
//! cargo test target directory and opens no window.

use notes::notes::store::{self, Autosave, Note};
use std::fs;
use std::path::{Path, PathBuf};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("sdk-tests-11-notes-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn note(id: &str, text: &str) -> Note {
    Note { id: id.into(), text: text.into(), modified: 0 }
}

fn ids(notes: &[Note]) -> Vec<String> {
    let mut ids: Vec<String> = notes.iter().map(|n| n.id.clone()).collect();
    ids.sort();
    ids
}

#[test]
fn a_saved_note_is_fetched_back_with_its_text_and_title() {
    let dir = scratch("fetch");
    store::save(&dir, &note("100", "Groceries\nmilk")).unwrap();
    let fetched = store::load(&dir);
    assert_eq!(fetched.len(), 1);
    assert_eq!(fetched[0].id, "100");
    assert_eq!(fetched[0].text, "Groceries\nmilk");
    assert_eq!(fetched[0].title(), "Groceries");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn each_note_is_one_markdown_file_named_after_its_id() {
    let dir = scratch("file-per-note");
    store::save(&dir, &note("7", "one")).unwrap();
    store::save(&dir, &note("8", "two")).unwrap();
    assert_eq!(fs::read_to_string(dir.join("7.md")).unwrap(), "one");
    assert_eq!(fs::read_to_string(dir.join("8.md")).unwrap(), "two");
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 2, "the atomic write leaves no temporary file behind");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn saving_an_existing_id_updates_the_note_instead_of_adding_one() {
    let dir = scratch("update");
    store::save(&dir, &note("5", "first draft")).unwrap();
    store::save(&dir, &note("5", "second draft")).unwrap();
    let fetched = store::load(&dir);
    assert_eq!(fetched.len(), 1);
    assert_eq!(fetched[0].text, "second draft");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn files_that_are_not_markdown_are_not_notes() {
    let dir = scratch("other-files");
    store::save(&dir, &note("1", "a note")).unwrap();
    fs::write(dir.join("readme.txt"), "not a note").unwrap();
    fs::write(dir.join("picture.png"), [0u8, 1, 2]).unwrap();
    assert_eq!(ids(&store::load(&dir)), ["1"]);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_missing_notes_directory_loads_as_no_notes() {
    let dir = scratch("missing").join("never-created");
    assert!(store::load(&dir).is_empty());
}

#[test]
fn deleting_a_note_removes_its_file_so_it_is_no_longer_fetched() {
    let dir = scratch("delete");
    store::save(&dir, &note("1", "keep")).unwrap();
    store::save(&dir, &note("2", "drop")).unwrap();
    store::delete(&dir, "2");
    assert!(!dir.join("2.md").exists());
    assert_eq!(ids(&store::load(&dir)), ["1"]);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn deleting_an_unknown_id_leaves_the_other_notes_alone() {
    let dir = scratch("delete-unknown");
    store::save(&dir, &note("1", "keep")).unwrap();
    store::delete(&dir, "404");
    assert_eq!(ids(&store::load(&dir)), ["1"]);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_new_id_is_the_creation_time_in_milliseconds() {
    assert_eq!(store::new_id(1_700_000_000_123, &[]), "1700000000123");
    assert_eq!(store::new_id(-5, &[]), "0", "a time before the epoch is taken as zero");
}

#[test]
fn a_new_id_skips_every_id_already_taken() {
    let taken = [note("1000", ""), note("1001", "")];
    assert_eq!(store::new_id(1000, &taken), "1002");
    assert_eq!(store::new_id(999, &taken), "999", "a free id is used as it is");
}

#[test]
fn autosave_is_due_half_a_second_after_the_last_change() {
    let mut autosave = Autosave::default();
    assert!(!autosave.pending());
    autosave.touch(10.0);
    assert!(autosave.pending());
    let due = 10.0 + store::DEBOUNCE_MS;
    assert_eq!(autosave.due(), Some(due));
    assert!(!autosave.fire(due - 1.0, false), "not yet due");
    assert!(autosave.fire(due, false), "due at the debounce interval");
    assert!(!autosave.pending(), "a fired save is not pending any more");
    assert!(!autosave.fire(due + 100.0, false), "nothing left to fire");
}

#[test]
fn every_change_restarts_the_autosave_timer() {
    let mut autosave = Autosave::default();
    autosave.touch(0.0);
    autosave.touch(400.0);
    assert_eq!(autosave.due(), Some(400.0 + store::DEBOUNCE_MS));
    assert!(!autosave.fire(store::DEBOUNCE_MS, false), "the second change pushed the save back");
    assert!(autosave.fire(400.0 + store::DEBOUNCE_MS, false));
}

#[test]
fn autosave_can_be_forced_to_fire_before_it_is_due() {
    let mut autosave = Autosave::default();
    autosave.touch(0.0);
    assert!(autosave.fire(0.0, true), "a forced save fires at once");
    assert!(!autosave.pending());
}
