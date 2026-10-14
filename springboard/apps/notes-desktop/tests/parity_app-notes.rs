//! Parity tests for Notes (reference app: Notes on macOS 27). These cover the model paths that the app UI calls:
//! attachments that travel with their note, the Group by Date preference, and the date sections of the list.

use notes::notes::attach;
use notes::notes::folders;
use notes::notes::grouping;
use notes::notes::store::{self, Note, Prefs, SortKey};
use std::path::PathBuf;

/// A clean directory under the crate's test target dir (never the system temp dir).
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("parity-app-notes-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 2026-10-15 12:00 UTC.
const NOW: i64 = 1_792_065_600;
const DAY: i64 = 86_400;

#[test]
fn attach_file_stores_any_type_and_moves_with_the_note() {
    let dir = scratch("attach-move");
    folders::create(&dir, "Work").unwrap();
    std::fs::write(dir.join("300.md"), "Plan\n").unwrap();
    let stored = attach::import_file(&dir, "300", "Budget (final).XLSX", b"sheet").unwrap();
    assert_eq!(stored, "300-1.xlsx");
    assert_eq!(attach::file_line("Budget (final).XLSX", &stored), "[Budget final.XLSX](300-1.xlsx)");
    assert_eq!(folders::attachments(&dir, "300"), vec!["300-1.xlsx".to_string()]);

    folders::move_note(&dir, "300", None, Some("Work")).unwrap();
    assert!(dir.join("Work/300-1.xlsx").exists(), "the file moves with its note");
    assert!(!dir.join("300-1.xlsx").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn group_by_date_is_saved_with_the_other_list_settings() {
    let dir = scratch("prefs");
    assert!(!store::load_prefs(&dir).group_by_date, "off by default");
    let saved = Prefs { sort: SortKey::Title, checked_to_bottom: false, group_by_date: true };
    store::save_prefs(&dir, saved).unwrap();
    assert_eq!(store::load_prefs(&dir), saved);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn date_sections_follow_the_list_order() {
    let note = |modified: i64| Note { id: modified.to_string(), text: String::new(), modified };
    let notes = vec![note(NOW), note(NOW - 3_600), note(NOW - DAY), note(NOW - 40 * DAY), note(NOW - 400 * DAY)];
    let sections = grouping::group(&notes, &[0, 1, 2, 3, 4], NOW, 0);
    let titles: Vec<(&str, usize)> = sections.iter().map(|s| (s.title.as_str(), s.notes.len())).collect();
    assert_eq!(titles, vec![("Today", 2), ("Yesterday", 1), ("September", 1), ("2025", 1)]);
}
