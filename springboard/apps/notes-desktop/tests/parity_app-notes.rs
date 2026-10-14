//! Parity app-notes: headless tests of the attachment browser model (attachments across notes, captions) and of the
//! Notes settings model (text size, dark background, new note start, attachment tile size). No display, no clock.

use notes::notes::attachments::{self, MAX_CAPTION};
use notes::notes::settings::{self, AttachmentSize, NewNoteStart, Settings, TextSize};
use notes::notes::store::Note;
use std::path::{Path, PathBuf};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-app-notes")).join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn note(id: &str, modified: i64) -> Note {
    Note { id: id.to_string(), text: format!("Note {id}"), modified }
}

/// Two notes: "100" at the root with two pictures, "200" in the folder Work with one picture and a drawing.
fn library(dir: &Path) -> Vec<(Option<String>, Note)> {
    std::fs::write(dir.join("100.md"), "Note 100").unwrap();
    std::fs::write(dir.join("100-1.png"), b"x").unwrap();
    std::fs::write(dir.join("100-2.jpg"), b"x").unwrap();
    std::fs::create_dir_all(dir.join("Work")).unwrap();
    std::fs::write(dir.join("Work/200.md"), "Note 200").unwrap();
    std::fs::write(dir.join("Work/200-1.png"), b"x").unwrap();
    std::fs::write(dir.join("Work/200-2.sketch"), b"x").unwrap();
    vec![(None, note("100", 10)), (Some("Work".to_string()), note("200", 20))]
}

#[test]
fn the_browser_lists_newest_note_first_and_keeps_each_notes_folder() {
    let dir = scratch("browse");
    let notes = library(&dir);
    let entries = attachments::browse(&dir, &notes);
    let files: Vec<(&str, Option<&str>, &str)> = entries.iter().map(|e| (e.note_id.as_str(), e.folder.as_deref(), e.file.as_str())).collect();
    assert_eq!(
        files,
        vec![("200", Some("Work"), "200-1.png"), ("200", Some("Work"), "200-2.sketch"), ("100", None, "100-1.png"), ("100", None, "100-2.jpg")],
        "a drawing counts as an attachment and every file is listed once"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_without_a_caption_is_labelled_by_its_file_name() {
    let dir = scratch("label");
    let entries = attachments::browse(&dir, &library(&dir));
    assert!(entries.iter().all(|e| e.caption.is_none()));
    assert_eq!(entries[2].label(), "100-1.png");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn renaming_an_attachment_saves_a_caption_and_leaves_the_file_name_alone() {
    let dir = scratch("rename");
    let notes = library(&dir);
    attachments::set_caption(&dir, "100-1.png", "  Receipt  ").unwrap();
    let entries = attachments::browse(&dir, &notes);
    let renamed = entries.iter().find(|e| e.file == "100-1.png").unwrap();
    assert_eq!(renamed.label(), "Receipt", "the caption is trimmed and shown");
    assert!(dir.join("100-1.png").exists(), "the file on disk keeps its name");
    assert_eq!(attachments::load_captions(&dir).get("100-1.png").map(String::as_str), Some("Receipt"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_blank_caption_removes_the_caption() {
    let dir = scratch("blank");
    attachments::set_caption(&dir, "100-2.jpg", "Trip").unwrap();
    attachments::set_caption(&dir, "100-2.jpg", "   ").unwrap();
    assert!(attachments::load_captions(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_caption_cannot_break_the_caption_file() {
    let dir = scratch("invalid");
    assert!(attachments::set_caption(&dir, "100-1.png", "two\tcolumns").is_err(), "a tab would split the line");
    assert!(attachments::set_caption(&dir, "100-1.png", "line\nbreak").is_err());
    assert!(attachments::set_caption(&dir, "100-1.png", &"x".repeat(MAX_CAPTION + 1)).is_err());
    assert!(attachments::set_caption(&dir, "100-1.png", &"x".repeat(MAX_CAPTION)).is_ok(), "exactly the limit is accepted");
    assert!(attachments::set_caption(&dir, "../100-1.png", "Escape").is_err(), "a path is not a file name");
    assert!(attachments::set_caption(&dir, "", "Empty").is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_damaged_caption_line_is_skipped_and_the_rest_still_load() {
    let dir = scratch("damaged");
    std::fs::write(dir.join(".springboard-notes-captions"), "no tab here\n100-1.png\tOK\n100-2.jpg\t\n").unwrap();
    let captions = attachments::load_captions(&dir);
    assert_eq!(captions.len(), 1);
    assert_eq!(captions.get("100-1.png").map(String::as_str), Some("OK"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_settings_default_to_medium_text_light_paper_and_small_tiles() {
    let s = Settings::default();
    assert_eq!(s.text_size, TextSize::Medium);
    assert!(!s.dark_background);
    assert_eq!(s.new_note_start, NewNoteStart::Title);
    assert_eq!(s.attachment_size, AttachmentSize::Small);
    assert_eq!(settings::parse(""), s, "an empty file is the defaults");
}

#[test]
fn settings_round_trip_through_their_file() {
    let dir = scratch("settings");
    let set = Settings { text_size: TextSize::Large, dark_background: true, new_note_start: NewNoteStart::Checklist, attachment_size: AttachmentSize::Large };
    settings::save(&dir, set).unwrap();
    assert_eq!(settings::load(&dir), set);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unknown_or_damaged_settings_fall_back_to_their_defaults() {
    let s = settings::parse("text_size=huge\ndark_background=yes\nnew_note_start=poem\nattachment_size=large\nzzz=1\n");
    assert_eq!(s.text_size, TextSize::Medium, "an unknown size keeps the default");
    assert!(!s.dark_background, "only 1 turns the dark background on");
    assert_eq!(s.new_note_start, NewNoteStart::Title);
    assert_eq!(s.attachment_size, AttachmentSize::Large, "the known values still load");
}

#[test]
fn text_sizes_are_points_that_grow_with_the_setting() {
    assert!(TextSize::Small.body_pt() < TextSize::Medium.body_pt());
    assert!(TextSize::Medium.body_pt() < TextSize::Large.body_pt());
    assert_eq!(TextSize::Medium.body_pt(), 15.0);
}

#[test]
fn a_new_note_starts_with_the_chosen_prefix() {
    assert_eq!(NewNoteStart::Title.prefix(), "# ");
    assert_eq!(NewNoteStart::Heading.prefix(), "## ");
    assert_eq!(NewNoteStart::Body.prefix(), "");
    assert_eq!(NewNoteStart::Checklist.prefix(), "- [ ] ");
}

#[test]
fn the_attachment_browser_fits_as_many_tiles_as_the_width_allows() {
    assert_eq!(AttachmentSize::Small.columns(400.0, 8.0), 3, "104 pt a tile across 408 pt");
    assert_eq!(AttachmentSize::Large.columns(400.0, 8.0), 2, "168 pt a tile across 408 pt");
    assert_eq!(AttachmentSize::Small.columns(10.0, 8.0), 1, "never fewer than one column");
    assert_eq!(AttachmentSize::Small.columns(0.0, 0.0), 1);
}
