//! Parity tests for the Books library orders (View > Sort By, including Progress) and the shelf keys. Pure logic: no window,
//! no display and no file is touched.

use super::store::{self, SortBy};

/// Orders the titles `(title, progress)` by the sort key of `sort`, the way the library orders its shelf.
fn ordered(sort: SortBy, books: &[(&str, u16)]) -> Vec<String> {
    let mut shown: Vec<(&str, u16)> = books.to_vec();
    shown.sort_by_cached_key(|(title, progress)| store::sort_key(sort, title, "", *progress));
    shown.into_iter().map(|(title, _)| title.to_string()).collect()
}

#[test]
fn sort_by_progress_lists_the_furthest_book_first_and_breaks_ties_by_title() {
    let books = [("Zebra", 800), ("avocado", 0), ("Mango", 800), ("Berry", 250)];
    assert_eq!(ordered(SortBy::Progress, &books), ["Mango", "Zebra", "Berry", "avocado"]);
}

#[test]
fn sort_by_progress_puts_unopened_books_last() {
    let books = [("Alpha", 0), ("Beta", 1), ("Gamma", 0)];
    assert_eq!(ordered(SortBy::Progress, &books), ["Beta", "Alpha", "Gamma"]);
}

#[test]
fn the_sort_menu_offers_four_orders_and_a_stored_progress_choice_is_kept() {
    assert_eq!(SortBy::ALL.len(), 4, "Recent, Title, Author and Progress");
    assert_eq!(SortBy::ALL[3], SortBy::Progress);
    assert_eq!(SortBy::Progress.name(), "Progress");
    let stored = store::LibraryPrefs { sort: SortBy::Progress, ..store::LibraryPrefs::default() };
    assert_eq!(stored.sort, SortBy::ALL[3]);
}

#[test]
fn the_title_and_author_orders_ignore_progress() {
    let books = [("Zebra", 900), ("avocado", 0)];
    assert_eq!(ordered(SortBy::Title, &books), ["avocado", "Zebra"]);
    assert_eq!(ordered(SortBy::Author, &books), ["avocado", "Zebra"], "neither has an author, so the titles decide");
}
