//! Books > Settings and the Display panel's font row, driven through the taps the app receives.

use super::engine::{Block, Book};
use super::*;
use springboard::shell::Event;

fn ui() -> Ui {
    springboard::shell::tests::ui()
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-books-settings")).join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn tap(b: &mut Books, ui: &Ui, x: f32, y: f32, t: f64) {
    b.touch(ui, Event::Down(x, y), t);
    b.touch(ui, Event::Up(x, y), t + 30.0);
}

fn centre((x, y, w, h): (f32, f32, f32, f32)) -> (f32, f32) {
    (x + w / 2.0, y + h / 2.0)
}

fn small_book() -> Book {
    Book { title: "Small".into(), author: "A. Tester".into(), blocks: vec![Block::Para("Some words for the page.".into())], toc: vec![], cover: None }
}

fn target_rect(b: &Books, ui: &Ui, want: Target) -> (f32, f32, f32, f32) {
    b.settings_targets(ui).into_iter().find(|(t, _)| *t == want).unwrap_or_else(|| panic!("no {want:?} in the Settings window")).1
}

#[test]
fn done_in_the_settings_window_returns_to_the_library() {
    let ui = ui();
    let mut b = Books::with_library_dirs(&ui, 1200, 1920, scratch("done"), Vec::new());
    b.open_settings();
    assert!(matches!(b.screen, Screen::Settings));
    let (x, y) = centre(target_rect(&b, &ui, Target::SettingsDone));
    tap(&mut b, &ui, x, y, 1000.0);
    assert!(matches!(b.screen, Screen::Library));
}

#[test]
fn the_open_mode_and_the_library_folders_survive_a_restart() {
    let ui = ui();
    let dir = scratch("restart");
    let shelf = dir.join("Shelf");
    std::fs::create_dir_all(&shelf).unwrap();
    {
        let mut b = Books::with_library_dirs(&ui, 1200, 1920, dir.clone(), Vec::new());
        b.set_open_mode(true);
        b.store.folders.push(shelf.clone());
        b.save_now();
    }
    let b = Books::with_library_dirs(&ui, 1200, 1920, dir, Vec::new());
    assert!(b.store.open_continuous);
    assert_eq!(b.store.folders, vec![shelf.clone()]);
    assert!(b.library_dirs().contains(&shelf), "the library scans the folder the user added");
}

#[test]
fn removing_a_user_folder_leaves_the_built_in_ones() {
    let ui = ui();
    let dir = scratch("remove");
    let shelf = dir.join("Shelf");
    let mut b = Books::with_library_dirs(&ui, 1200, 1920, dir.clone(), Vec::new());
    b.store.folders = vec![shelf.clone()];
    let rows = b.folder_rows();
    assert_eq!(rows[0], (dir.clone(), None), "the first folder is built in");
    assert_eq!(rows[1], (shelf.clone(), Some(0)));
    b.remove_folder(0);
    assert_eq!(b.store.folders, vec![shelf], "Remove on a built-in folder does nothing");
    b.remove_folder(1);
    assert!(b.store.folders.is_empty());
    assert_eq!(b.library_dirs(), vec![dir]);
}

#[test]
fn the_font_row_changes_an_epub_and_leaves_a_pdf_alone() {
    let ui = ui();
    let mut b = Books::with_library_dirs(&ui, 1200, 1920, scratch("font"), Vec::new());
    b.font_families = vec!["Literata".to_string()];
    b.show_book(&ui, PathBuf::from("/virtual/scan.pdf"), small_book());
    b.step_family(&ui, 1);
    assert_eq!(b.store.family, "", "a PDF keeps the font its document sets");
    b.show_book(&ui, PathBuf::from("/virtual/novel.epub"), small_book());
    b.step_family(&ui, 1);
    assert_eq!(b.store.family, "Literata");
    b.step_family(&ui, 1);
    assert_eq!(b.store.family, "", "the row wraps back to the system face");
}
