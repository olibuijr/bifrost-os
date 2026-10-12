//! Tests for the service glue: the search index of the shelf, the hand-off from a search result, and the quotation
//! copied to the general pasteboard board. The messages are read with `Ctx::take_sent`.

use super::engine::model::search;
use super::engine::{Block, Book, TocEntry};
use super::quote;
use super::search_index::{self, Entry, SearchIndex, DOMAIN};
use super::store::Span;
use super::*;
use bifrost_app::proto::quicksearch::DELETE_IDS;
use bifrost_app::proto::{Configure, ToShell};
use bifrost_app::quicksearch::{UserActivity, ACTION_TYPE};
use bifrost_app::Ctx;

fn ui() -> Ui {
    springboard::shell::tests::ui()
}

fn ctx() -> Ctx {
    Ctx::testing(ui(), Configure { w: 1200, h: 780, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: false, text_scale: 1.0, reduce_motion: false })
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-books-connect")).join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn entry(path: &str, title: &str, author: &str, format: &'static str) -> Entry {
    Entry { path: PathBuf::from(path), title: title.into(), author: author.into(), format }
}

const SWANS: &str = "Ég sá hvítan svan. Svanurinn synti hægt yfir vatnið! Hann var fallegur.";

fn swans() -> Book {
    Book {
        title: "Svanir".into(),
        author: "A. Tester".into(),
        blocks: vec![Block::Heading(1, "Kafli 1".into()), Block::Para(SWANS.into()), Block::Para("Ný málsgrein.".into())],
        toc: vec![TocEntry { title: "Kafli 1".into(), block: 0, level: 0 }],
        cover: None,
    }
}

/// The span of the first `word` in block `block` of `text`, in chars (the reader's offsets).
fn span_of(text: &str, word: &str, block: usize) -> Span {
    let byte = text.find(word).unwrap();
    Span { block, offset: text[..byte].chars().count(), len: word.chars().count() }
}

#[test]
fn the_shelf_is_indexed_once_and_the_books_that_leave_are_removed() {
    let ctx = ctx();
    let mut index = SearchIndex::default();
    let shelf = vec![entry("/b/one.epub", "One", "A. Author", "EPUB"), entry("/b/two.pdf", "Two", "", "PDF")];
    index.sync(&ctx, &shelf);
    match ctx.take_sent().as_slice() {
        [ToShell::QuickSearchIndex(items)] => {
            assert_eq!(items.len(), 2);
            assert_eq!(items[0].unique_id, "/b/one.epub");
            assert_eq!(items[0].domain, DOMAIN);
            assert_eq!(items[0].title, "One");
            assert_eq!(items[0].keywords, vec!["A. Author".to_string()]);
            assert!(items[1].keywords.is_empty(), "a book with no author has no keyword");
        }
        other => panic!("expected one index message, got {other:?}"),
    }

    // nothing changed: nothing is sent
    index.sync(&ctx, &shelf);
    assert!(ctx.take_sent().is_empty());

    // a probed author re-sends only that book
    let probed = vec![entry("/b/one.epub", "One", "B. Writer", "EPUB"), entry("/b/two.pdf", "Two", "", "PDF")];
    index.sync(&ctx, &probed);
    match ctx.take_sent().as_slice() {
        [ToShell::QuickSearchIndex(items)] => assert_eq!(items.iter().map(|i| i.unique_id.as_str()).collect::<Vec<_>>(), ["/b/one.epub"]),
        other => panic!("expected the changed book only, got {other:?}"),
    }

    // a book that left the shelf is deleted from the index
    index.sync(&ctx, &probed[..1]);
    match ctx.take_sent().as_slice() {
        [ToShell::QuickSearchDelete { kind, values }] => {
            assert_eq!(*kind, DELETE_IDS);
            assert_eq!(values, &["/b/two.pdf".to_string()]);
        }
        other => panic!("expected a delete of the book that left, got {other:?}"),
    }
}

#[test]
fn a_search_result_opens_its_book_only_in_this_apps_domain() {
    let ours = |kind: &str, domain: &str, id: &str| UserActivity { activity_type: kind.into(), unique_identifier: id.into(), domain_identifier: domain.into() };
    assert_eq!(search_index::path_from_activity(&ours(ACTION_TYPE, DOMAIN, "/b/one.epub")), Some(PathBuf::from("/b/one.epub")));
    assert_eq!(search_index::path_from_activity(&ours(ACTION_TYPE, "notes", "/b/one.epub")), None);
    assert_eq!(search_index::path_from_activity(&ours("com.other", DOMAIN, "/b/one.epub")), None);
    assert_eq!(search_index::path_from_activity(&ours(ACTION_TYPE, DOMAIN, "")), None);
}

#[test]
fn a_quotation_is_the_sentence_with_its_citation() {
    let book = swans();
    let span = span_of(SWANS, "Svanurinn", 1);
    assert_eq!(quote::sentence(&book.blocks, span).as_deref(), Some("Svanurinn synti hægt yfir vatnið!"));
    assert_eq!(quote::citation(&book, span), "Svanir, A. Tester, Kafli 1");
    assert_eq!(quote::quotation(&book, span).as_deref(), Some("\u{201c}Svanurinn synti hægt yfir vatnið!\u{201d}\n\u{2014} Svanir, A. Tester, Kafli 1"));
}

#[test]
fn a_quotation_leaves_out_what_is_unknown_and_refuses_spans_outside_the_text() {
    let mut book = swans();
    book.author = String::new();
    book.toc.clear();
    let span = span_of(SWANS, "Svanurinn", 1);
    assert_eq!(quote::citation(&book, span), "Svanir");
    assert_eq!(quote::sentence(&book.blocks, Span { block: 9, offset: 0, len: 1 }), None, "no such block");
    assert_eq!(quote::sentence(&book.blocks, Span { block: 1, offset: 500, len: 3 }), None, "the offset is past the block's text");
}

#[test]
fn a_long_sentence_is_quoted_as_the_match_alone() {
    let long = format!("{}needle{}", "word ".repeat(120), " word".repeat(120));
    let blocks = vec![Block::Para(long)];
    assert_eq!(quote::sentence(&blocks, Span { block: 0, offset: 600, len: 6 }).as_deref(), Some("needle"));
}

#[test]
fn copy_quote_writes_the_quotation_to_the_general_board() {
    let ctx = ctx();
    let text = "\u{201c}Svanurinn\u{201d}\n\u{2014} Svanir";
    quote::copy(&ctx, text);
    match ctx.take_sent().as_slice() {
        [ToShell::ServiceCall { service, method, body, .. }] => {
            assert_eq!(service, "pasteboard");
            assert_eq!(method, "write");
            assert!(body.contains("\"general\""), "the body names the general board: {body}");
            assert!(body.contains("public.utf8-plain-text"), "the item is plain text: {body}");
            assert!(body.contains("Svanir"), "the citation travels with the quote: {body}");
        }
        other => panic!("expected one pasteboard write, got {other:?}"),
    }
}

#[test]
fn copy_quote_follows_the_find_match_on_the_page() {
    let ui = ui();
    let dir = scratch("quote");
    let mut b = Books::with_library_dirs(&ui, 1200, 1920, dir, Vec::new());
    b.show_book(&ui, PathBuf::from("/virtual/swans.epub"), swans());
    assert_eq!(b.quotation(), None, "nothing is quoted before a find");
    {
        let r = b.reader.as_mut().unwrap();
        r.query = "Svanurinn".into();
        r.hits = search(&r.book.blocks, "Svanurinn", 10);
    }
    b.find_step(1, 0.0);
    assert_eq!(b.quotation().as_deref(), Some("\u{201c}Svanurinn synti hægt yfir vatnið!\u{201d}\n\u{2014} Svanir, A. Tester, Kafli 1"));
}

#[test]
fn the_index_entries_of_the_shelf_carry_the_file_kind() {
    let ui = ui();
    let dir = scratch("entries");
    let b = Books::with_library_dirs(&ui, 1200, 1920, dir, Vec::new());
    assert!(b.index_entries().is_empty(), "an empty library indexes nothing");
}
