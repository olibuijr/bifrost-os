//! Parity app-photos: headless tests of the album folder model behind the Photos library. Folders group user albums,
//! an album sits in one folder at most, and the folders survive a round trip through the preference store.

use springboard::shell::photos::folders::{
    album_deleted, clean_name, decode, delete_folder, folder_of, move_album, new_folder, writes, AlbumFolder,
    FolderError, MAX_FOLDERS, MAX_NAME,
};
use std::collections::HashMap;

#[test]
fn a_new_library_has_no_folders() {
    let folders: Vec<AlbumFolder> = Vec::new();
    assert!(decode(|_| None).is_empty());
    assert!(folders.is_empty());
}

#[test]
fn a_new_folder_takes_the_trimmed_name_and_no_albums() {
    let mut folders = Vec::new();
    assert_eq!(new_folder(&mut folders, "  Trips  "), Ok(0));
    assert_eq!(folders, vec![AlbumFolder { name: "Trips".into(), albums: vec![] }]);
}

#[test]
fn a_blank_folder_name_is_refused() {
    let mut folders = Vec::new();
    assert_eq!(new_folder(&mut folders, "   "), Err(FolderError::Empty));
    assert_eq!(new_folder(&mut folders, "\t\n"), Err(FolderError::Empty));
    assert!(folders.is_empty(), "a refused name adds nothing");
}

#[test]
fn control_characters_are_dropped_from_a_folder_name() {
    assert_eq!(clean_name("Sum\nmer\u{7}"), "Summer");
}

#[test]
fn a_folder_name_is_cut_to_the_longest_kept_length() {
    let long = "x".repeat(MAX_NAME + 20);
    assert_eq!(clean_name(&long).chars().count(), MAX_NAME);
}

#[test]
fn folder_names_are_unique_ignoring_case() {
    let mut folders = Vec::new();
    new_folder(&mut folders, "Work").unwrap();
    assert_eq!(new_folder(&mut folders, "wORK"), Err(FolderError::Duplicate));
    assert_eq!(folders.len(), 1);
}

#[test]
fn the_library_holds_no_more_than_the_folder_limit() {
    let mut folders = Vec::new();
    for n in 0..MAX_FOLDERS {
        new_folder(&mut folders, &format!("Folder {n}")).unwrap();
    }
    assert_eq!(new_folder(&mut folders, "One more"), Err(FolderError::Full));
    assert_eq!(folders.len(), MAX_FOLDERS);
}

#[test]
fn a_refusal_message_says_why() {
    assert_eq!(FolderError::Duplicate.to_string(), "a folder with that name exists");
}

#[test]
fn moving_an_album_puts_it_in_the_folder() {
    let mut folders = Vec::new();
    new_folder(&mut folders, "Trips").unwrap();
    assert!(move_album(&mut folders, 0, 2));
    assert_eq!(folders[0].albums, vec![2]);
    assert_eq!(folder_of(&folders, 2), Some(0));
}

#[test]
fn an_album_moved_to_a_second_folder_leaves_the_first() {
    let mut folders = Vec::new();
    new_folder(&mut folders, "A").unwrap();
    new_folder(&mut folders, "B").unwrap();
    move_album(&mut folders, 0, 1);
    move_album(&mut folders, 1, 1);
    assert!(folders[0].albums.is_empty(), "an album sits in one folder at most");
    assert_eq!(folders[1].albums, vec![1]);
}

#[test]
fn moving_into_a_missing_folder_changes_nothing() {
    let mut folders = Vec::new();
    new_folder(&mut folders, "A").unwrap();
    assert!(!move_album(&mut folders, 4, 0));
    assert!(folders[0].albums.is_empty());
}

#[test]
fn an_album_outside_every_folder_has_no_folder() {
    let folders = vec![AlbumFolder { name: "A".into(), albums: vec![0] }];
    assert_eq!(folder_of(&folders, 3), None);
}

#[test]
fn deleting_an_album_takes_it_out_of_its_folder_and_moves_later_slots_down() {
    let mut folders = vec![
        AlbumFolder { name: "A".into(), albums: vec![0, 2] },
        AlbumFolder { name: "B".into(), albums: vec![1, 3] },
    ];
    album_deleted(&mut folders, 1);
    assert_eq!(folders[0].albums, vec![0, 1], "slot 2 moved down to 1");
    assert_eq!(folders[1].albums, vec![2], "slot 1 left, slot 3 moved down to 2");
}

#[test]
fn deleting_a_folder_keeps_the_count_of_the_others() {
    let mut folders = Vec::new();
    new_folder(&mut folders, "A").unwrap();
    new_folder(&mut folders, "B").unwrap();
    assert!(delete_folder(&mut folders, 0));
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].name, "B");
    assert!(!delete_folder(&mut folders, 5));
}

#[test]
fn folders_round_trip_through_the_preference_store() {
    let mut folders = Vec::new();
    new_folder(&mut folders, "Trips").unwrap();
    new_folder(&mut folders, "Family").unwrap();
    move_album(&mut folders, 0, 0);
    move_album(&mut folders, 0, 4);
    let store: HashMap<String, String> = writes(&folders, 0).into_iter().collect();
    let back = decode(|k| store.get(k).map(|s| s.as_str()));
    assert_eq!(back, folders);
}

#[test]
fn a_removed_folder_is_cleared_from_the_store() {
    let mut folders = Vec::new();
    new_folder(&mut folders, "A").unwrap();
    new_folder(&mut folders, "B").unwrap();
    let mut store: HashMap<String, String> = writes(&folders, 0).into_iter().collect();
    delete_folder(&mut folders, 1);
    for (k, v) in writes(&folders, 2) {
        store.insert(k, v);
    }
    let back = decode(|k| store.get(k).map(|s| s.as_str()));
    assert_eq!(back.len(), 1, "the first empty name ends the stored list");
    assert_eq!(back[0].name, "A");
}

#[test]
fn decoding_skips_album_slots_that_do_not_parse() {
    let mut store = HashMap::new();
    store.insert("uf.0.name".to_string(), "Mixed".to_string());
    store.insert("uf.0.albums".to_string(), "1, x,3,,7".to_string());
    let back = decode(|k| store.get(k).map(|s| s.as_str()));
    assert_eq!(back, vec![AlbumFolder { name: "Mixed".into(), albums: vec![1, 3, 7] }]);
}

#[test]
fn the_folder_limit_is_the_number_of_stored_slots() {
    assert_eq!(MAX_FOLDERS, 8, "eight folder slots are stored under uf.0 to uf.7");
}
