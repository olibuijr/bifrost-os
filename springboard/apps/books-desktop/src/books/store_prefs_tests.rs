//! Tests for the Settings window's preferences (folders, open mode, font family) and the Display panel's font row.

use super::store::*;
use std::path::{Path, PathBuf};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-books-prefs")).join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn settings_window_prefs_round_trip() {
    let dir = scratch("roundtrip");
    let file = dir.join("state.tsv");
    let mut s = Store::load(file.clone());
    s.settings = Settings { size: 5, theme: Theme::Sepia, spacing: 0, margin: 2, justify: true };
    s.family = "Literata".to_string();
    s.open_continuous = true;
    s.folders = vec![PathBuf::from("/data/Shelf A"), PathBuf::from("/data/B")];
    s.save();
    let t = Store::load(file);
    assert_eq!(t.family, "Literata");
    assert!(t.open_continuous);
    assert_eq!(t.folders, vec![PathBuf::from("/data/Shelf A"), PathBuf::from("/data/B")]);
    assert_eq!(t.settings, Settings { size: 5, theme: Theme::Sepia, spacing: 0, margin: 2, justify: true });
}

#[test]
fn a_state_file_from_before_the_prefs_loads_with_the_defaults() {
    let dir = scratch("old");
    let file = dir.join("state.tsv");
    std::fs::write(&file, "S\t3\t1\t0\t1\t0\nV\t0\t0\t0\n").unwrap();
    let t = Store::load(file);
    assert_eq!(t.family, "");
    assert!(!t.open_continuous);
    assert!(t.folders.is_empty());
    assert_eq!(t.settings.size, 3);
}

#[test]
fn a_folder_is_listed_once() {
    let dir = scratch("dupes");
    let file = dir.join("state.tsv");
    std::fs::write(&file, "D\t/a\nD\t/a\nD\t/b\n").unwrap();
    assert_eq!(Store::load(file).folders, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
}

#[test]
fn the_family_row_steps_through_the_system_face_and_the_library() {
    let fams: Vec<String> = ["Literata", "Mono Sans"].iter().map(|s| s.to_string()).collect();
    assert_eq!(family_step(&fams, "", 1), "Literata");
    assert_eq!(family_step(&fams, "Literata", 1), "Mono Sans");
    assert_eq!(family_step(&fams, "Mono Sans", 1), "");
    assert_eq!(family_step(&fams, "", -1), "Mono Sans");
    assert_eq!(family_step(&fams, "Gone", 1), "Literata", "a family that is no longer installed counts as the system face");
    assert_eq!(family_step(&[], "", 1), "", "with no library fonts the row stays on the system face");
}

#[test]
fn the_family_choice_is_off_for_pdfs() {
    assert!(family_applies(Path::new("/books/novel.epub")));
    assert!(!family_applies(Path::new("/books/scan.pdf")));
    assert!(!family_applies(Path::new("/books/SCAN.PDF")));
}
