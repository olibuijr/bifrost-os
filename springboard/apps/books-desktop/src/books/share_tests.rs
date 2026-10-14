//! Headless tests of the rule that opens a book after an import: a lone shared PDF opens, and the other imports only
//! join the shelf. No window, display or file is touched.

use super::share::book_to_open;
use std::path::{Path, PathBuf};

#[test]
fn a_lone_shared_book_opens_at_once() {
    let added = vec![(PathBuf::from("/books/Shared.pdf"), true)];
    assert_eq!(book_to_open(&added), Some(Path::new("/books/Shared.pdf")));
}

#[test]
fn a_book_chosen_in_the_panel_only_joins_the_shelf() {
    let added = vec![(PathBuf::from("/books/Chosen.epub"), false)];
    assert_eq!(book_to_open(&added), None);
}

#[test]
fn several_shared_books_stay_on_the_shelf() {
    let added = vec![
        (PathBuf::from("/books/a.pdf"), true),
        (PathBuf::from("/books/b.pdf"), true),
    ];
    assert_eq!(book_to_open(&added), None, "a batch opens nothing");
}

#[test]
fn nothing_imported_opens_nothing() {
    assert_eq!(book_to_open(&[]), None);
}
