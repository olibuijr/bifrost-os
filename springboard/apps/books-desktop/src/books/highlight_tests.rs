//! Tests for the find-in-book and highlight wiring: Find Next and Previous order, whole-word matching,
//! and the highlights kept in the store file.

use super::engine::model::search;
use super::engine::Block;
use super::store::{Span, Store};
use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-books-highlight")).join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn find_steps_wrap_at_both_ends() {
    // no hit visited yet: next takes the first, previous the last
    assert_eq!(step_index(None, 1, 3), Some(0));
    assert_eq!(step_index(None, -1, 3), Some(2));
    // then it walks and wraps
    assert_eq!(step_index(Some(0), 1, 3), Some(1));
    assert_eq!(step_index(Some(2), 1, 3), Some(0));
    assert_eq!(step_index(Some(0), -1, 3), Some(2));
    assert_eq!(step_index(Some(1), -1, 3), Some(0));
    // a single hit stays put, and no hit gives nothing
    assert_eq!(step_index(Some(0), 1, 1), Some(0));
    assert_eq!(step_index(None, 1, 0), None);
    assert_eq!(step_index(Some(0), -1, 0), None);
}

#[test]
fn whole_word_keeps_only_matches_on_word_edges() {
    let blocks = vec![Block::Para("the cat concatenates cat.".into()), Block::Heading(1, "Cats".into())];
    let all = search(&blocks, "cat", 100);
    // "cat" inside "concatenates" is a hit too: offsets 4, 11 and 21
    assert_eq!(all.iter().map(|h| h.offset).collect::<Vec<_>>(), [4, 11, 21]);
    let whole = keep_whole_words(&blocks, "cat", all);
    assert_eq!(whole.iter().map(|h| (h.block, h.offset)).collect::<Vec<_>>(), [(0, 4), (0, 21)]);
    // a plural is not a whole word match
    let plural = keep_whole_words(&blocks, "cat", search(&blocks, "cats", 100));
    assert_eq!(plural.iter().map(|h| h.block).collect::<Vec<_>>(), [1]);
}

#[test]
fn whole_word_ignores_case_and_non_letter_neighbours() {
    let blocks = vec![Block::Para("CAT, Cat-like cat".into())];
    let whole = keep_whole_words(&blocks, "cat", search(&blocks, "cat", 100));
    assert_eq!(whole.iter().map(|h| h.offset).collect::<Vec<_>>(), [0, 5, 14]);
}

#[test]
fn highlights_toggle_and_survive_a_save() {
    let dir = scratch("roundtrip");
    let file = dir.join("state.tsv");
    let book = Path::new("/a/one.pdf");
    let mut s = Store::load(file.clone());
    s.touch(book);
    assert!(s.toggle_highlight(book, Span { block: 1, offset: 0, len: 2 }));
    assert!(s.toggle_highlight(book, Span { block: 0, offset: 3, len: 4 }));
    // the same span again removes it
    assert!(!s.toggle_highlight(book, Span { block: 1, offset: 0, len: 2 }));
    assert!(s.toggle_highlight(book, Span { block: 1, offset: 0, len: 2 }));
    // an empty span is never stored
    assert!(!s.toggle_highlight(book, Span { block: 2, offset: 0, len: 0 }));
    s.save();
    let t = Store::load(file);
    let one = t.entry(book).unwrap();
    assert_eq!(one.highlights, vec![Span { block: 0, offset: 3, len: 4 }, Span { block: 1, offset: 0, len: 2 }]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn highlight_line_before_its_book_line_loads_into_one_entry() {
    let dir = scratch("order");
    let file = dir.join("state.tsv");
    std::fs::write(&file, "H\t/a/book.pdf\t2\t5\t7\nB\t/a/book.pdf\t3\t4\t9\t\t250\n").unwrap();
    let s = Store::load(file);
    assert_eq!(s.entries.len(), 1);
    let e = s.entry(Path::new("/a/book.pdf")).unwrap();
    assert_eq!(e.highlights, vec![Span { block: 2, offset: 5, len: 7 }]);
    assert_eq!(e.pos.block, 3);
    assert_eq!(e.progress, 250);
    assert_eq!(e.opened, 9);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn state_without_highlights_loads_empty() {
    let dir = scratch("old");
    let file = dir.join("state.tsv");
    std::fs::write(&file, "B\t/a/old.epub\t1\t2\t3\t\t40\n").unwrap();
    let s = Store::load(file);
    assert!(s.entry(Path::new("/a/old.epub")).unwrap().highlights.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}
