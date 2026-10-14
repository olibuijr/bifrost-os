//! Headless tests for the UI models behind the parity rows: the app-side search query, the search index query and
//! its acknowledged writes, the split view, the popover placer, the pop-up and menu model, the navigation stack and
//! the accessibility element map and announcement queue. Each one is pure logic: it runs on the host with no display,
//! no window and no clock (every time is passed in), and asserts the exact results.

use bifrost_ui::ax::{slug, Announcement, Announcements, Map, Politeness, Resolve};
use bifrost_ui::geometry::Edge;
use bifrost_ui::menu::{popup_origin, Menu, MenuItem};
use bifrost_ui::nav::{NavigationPath, NavigationStack};
use bifrost_ui::quicksearch_query::{Error as QueryError, IndexDelegate, Item, QueryContext, SearchableIndex, SearchQuery, UserQuery, UserQueryContext};
use bifrost_ui::searchquery::{Context, Error, Field, Query, Record};
use bifrost_ui::split::{place_popover, visible_rows, SplitView};
use bifrost_ui::Rect;

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

fn hits(q: &Query, records: &[Record]) -> Vec<(usize, u32)> {
    bifrost_ui::searchquery::run(q, records).expect("the query runs").iter().map(|h| (h.index, h.score)).collect()
}

fn rec(title: &str, description: &str, keywords: &[&str]) -> Record {
    Record { title: title.into(), description: description.into(), keywords: keywords.iter().map(|k| k.to_string()).collect() }
}

/// Four records: a title match, a keyword match, a description match and a short title.
fn records() -> Vec<Record> {
    vec![rec("Milk", "oat", &["shop"]), rec("Bread", "", &["shop"]), rec("Tea", "milk tea", &[]), rec("Mil", "", &[])]
}

// --- App-side search query (bifrost_ui::searchquery) ------------------------------------------------------------------

#[test]
fn a_word_matches_the_start_of_a_word_and_the_title_outranks_the_keywords() {
    // "mil" finds "Milk" and "Mil" in titles (weight 3) and "milk" in a description (weight 1)
    assert_eq!(hits(&Query::new("mil"), &records()), [(0, 3), (3, 3), (2, 1)]);
    // "shop" is only in keywords (weight 2)
    assert_eq!(hits(&Query::new("shop"), &records()), [(0, 2), (1, 2)]);
}

#[test]
fn every_word_of_the_query_must_match() {
    // "milk" scores 3 in the title and "shop" 2 in the keywords; Bread and Tea each miss one word
    assert_eq!(hits(&Query::new("milk shop"), &records()), [(0, 5)]);
}

#[test]
fn quoted_text_is_a_phrase_whose_last_word_may_be_a_prefix() {
    assert_eq!(hits(&Query::new("\"milk tea\""), &records()), [(2, 1)]);
    assert_eq!(hits(&Query::new("\"milk te\""), &records()), [(2, 1)]);
    assert!(hits(&Query::new("\"tea milk\""), &records()).is_empty(), "the words must keep their order");
}

#[test]
fn punctuation_and_case_are_ignored() {
    assert_eq!(hits(&Query::new("MILK!"), &records()), [(0, 3), (2, 1)]);
}

#[test]
fn filter_queries_must_match_but_do_not_add_to_the_score() {
    let ctx = Context { filter_queries: vec!["shop".into()], ..Context::default() };
    assert_eq!(hits(&Query::with_context("mil", ctx), &records()), [(0, 3)]);
}

#[test]
fn restricting_the_fields_removes_matches_outside_them() {
    assert!(hits(&Query::new("shop").fields(&[Field::Title]), &records()).is_empty());
    assert_eq!(hits(&Query::new("shop").fields(&[Field::Keywords]), &records()), [(0, 2), (1, 2)]);
}

#[test]
fn an_empty_query_or_a_query_with_no_fields_is_refused_with_its_code() {
    assert_eq!(bifrost_ui::searchquery::run(&Query::new("  "), &records()), Err(Error::EmptyQueryString));
    assert_eq!(bifrost_ui::searchquery::run(&Query::new("milk").fields(&[]), &records()), Err(Error::NoFields));
    assert_eq!(Error::EmptyQueryString.code(), 1);
    assert_eq!(Error::NoFields.code(), 2);
    assert_eq!(Error::DOMAIN, "bifrost.quicksearch.query");
}

#[test]
fn ties_keep_the_order_of_the_records() {
    let twins = [rec("Milk", "", &[]), rec("Milk", "", &[])];
    assert_eq!(hits(&Query::new("milk"), &twins), [(0, 3), (1, 3)]);
}

// --- Search index query (bifrost_ui::quicksearch_query) ---------------------------------------------------------------

fn indexed() -> Vec<Item> {
    vec![
        Item::new("a", "notes").text("title", "Milk").text("keywords", "shop").text("description", "oat"),
        Item::new("b", "notes").text("title", "Bread").text("keywords", "milk"),
        Item::new("c", "notes").text("title", "Tea").text("description", "milk tea"),
    ]
}

fn ids(items: &[Item]) -> Vec<&str> {
    items.iter().map(|i| i.id.as_str()).collect()
}

#[test]
fn a_user_query_ranks_title_then_keyword_then_description_matches() {
    let q = UserQuery::new("mil", UserQueryContext::default());
    let (results, _) = q.run(&indexed());
    assert_eq!(ids(&results), ["a", "b", "c"]);
}

#[test]
fn every_typed_word_of_a_user_query_must_match() {
    let (results, _) = UserQuery::new("milk shop", UserQueryContext::default()).run(&indexed());
    assert_eq!(ids(&results), ["a"]);
}

#[test]
fn a_user_query_caps_its_results_and_its_suggestions() {
    let capped = UserQueryContext { max_result_count: 2, max_suggestion_count: 0 };
    let (results, suggestions) = UserQuery::new("mil", capped).run(&indexed());
    assert_eq!(ids(&results), ["a", "b"]);
    assert!(suggestions.is_empty());
}

#[test]
fn suggestions_are_titles_that_continue_the_typed_text() {
    let (_, suggestions) = UserQuery::new("mi", UserQueryContext::default()).run(&indexed());
    let titles: Vec<&str> = suggestions.iter().map(|s| s.localized_attribute_set_title.as_str()).collect();
    assert_eq!(titles, ["Milk"]);
}

#[test]
fn a_user_query_ignores_case_and_diacritics() {
    let items = [Item::new("m", "notes").text("title", "Mjólk")];
    let (results, _) = UserQuery::new("MJOLK", UserQueryContext::default()).run(&items);
    assert_eq!(ids(&results), ["m"]);
}

#[test]
fn a_blank_user_query_finds_nothing() {
    let (results, suggestions) = UserQuery::new("   ", UserQueryContext::default()).run(&indexed());
    assert!(results.is_empty() && suggestions.is_empty());
}

#[test]
fn a_query_string_with_a_field_test_matches_the_attribute_exactly() {
    let items = indexed();
    let mut q = SearchQuery::new("title == \"milk\"c", QueryContext::default());
    let mut found: Vec<String> = Vec::new();
    let mut done = None;
    q.start(&items, 0.0, |batch| found.extend(batch.iter().map(|i| i.id.clone())), |e| done = Some(e));
    assert_eq!(found, ["a"]);
    assert_eq!(done, Some(None), "the query completes without an error");
    assert_eq!(q.found_item_count(), 1);
}

#[test]
fn a_cancelled_query_reports_cancellation_and_finds_nothing() {
    let items = indexed();
    let mut q = SearchQuery::new("title == \"milk\"c", QueryContext::default());
    q.cancel();
    assert!(q.is_cancelled());
    let mut called = false;
    let mut done = None;
    q.start(&items, 0.0, |_| called = true, |e| done = Some(e));
    assert!(!called);
    assert_eq!(done, Some(Some(QueryError::Cancelled)));
    assert_eq!(QueryError::Cancelled.code(), 2);
    assert_eq!(QueryError::DOMAIN, "bifrost.quicksearch.query");
}

#[test]
fn the_searchable_index_evicts_the_oldest_items_past_its_capacity() {
    let mut index = SearchableIndex::new("notes", Some("complete"), 2);
    index.index_searchable_items(vec![Item::new("a", "notes")]).unwrap();
    index.index_searchable_items(vec![Item::new("b", "notes")]).unwrap();
    index.index_searchable_items(vec![Item::new("c", "notes")]).unwrap();
    assert_eq!(ids(index.items()), ["b", "c"]);
    assert_eq!(index.take_evicted(), ["a"]);
    assert!(index.take_evicted().is_empty(), "taking drains the list");
    assert_eq!(index.protection_class.as_deref(), Some("complete"));
}

#[test]
fn indexing_an_identifier_again_replaces_the_item() {
    let mut index = SearchableIndex::new("notes", None, 2);
    index.index_searchable_items(vec![Item::new("a", "notes"), Item::new("b", "notes")]).unwrap();
    index.index_searchable_items(vec![Item::new("a", "other")]).unwrap();
    assert_eq!(index.items().len(), 2);
    assert_eq!(index.items().iter().find(|i| i.id == "a").map(|i| i.domain.as_str()), Some("other"));
    assert!(index.take_evicted().is_empty());
}

#[test]
fn a_disconnected_index_refuses_writes_and_keeps_its_batch_for_a_retry() {
    let mut index = SearchableIndex::new("notes", None, 4);
    index.begin_batch().unwrap();
    index.index_searchable_items(vec![Item::new("a", "notes")]).unwrap();
    index.set_connected(false);
    assert_eq!(index.end_batch(b"v1"), Err(QueryError::RemoteConnection));
    assert_eq!(index.fetch_last_client_state(), Err(QueryError::RemoteConnection));
    assert_eq!(index.index_searchable_items(vec![]), Err(QueryError::RemoteConnection));
    index.set_connected(true);
    assert!(index.items().is_empty(), "nothing is committed while disconnected");
    index.end_batch(b"v1").unwrap();
    assert_eq!(ids(index.items()), ["a"]);
    assert_eq!(index.fetch_last_client_state(), Ok(b"v1".to_vec()));
}

#[test]
fn batches_must_open_and_close_in_order() {
    let mut index = SearchableIndex::new("notes", None, 4);
    assert_eq!(index.end_batch(b""), Err(QueryError::BatchState));
    index.begin_batch().unwrap();
    assert_eq!(index.begin_batch(), Err(QueryError::BatchState));
}

#[test]
fn deleting_by_identifier_removes_only_those_items() {
    let mut index = SearchableIndex::new("notes", None, 4);
    index.index_searchable_items(vec![Item::new("a", "n"), Item::new("b", "n"), Item::new("c", "n")]).unwrap();
    index.delete_searchable_items(&["a".to_string(), "c".to_string()]).unwrap();
    assert_eq!(ids(index.items()), ["b"]);
}

struct Rebuild {
    everything: bool,
    asked: Vec<String>,
}

impl IndexDelegate for Rebuild {
    fn reindex_all_searchable_items(&mut self, index: &mut SearchableIndex) {
        self.everything = true;
        index.index_searchable_items(vec![Item::new("z", "notes")]).unwrap();
    }
    fn reindex_searchable_items(&mut self, _index: &mut SearchableIndex, identifiers: &[String]) {
        self.asked = identifiers.to_vec();
    }
}

#[test]
fn a_lost_index_empties_and_asks_the_delegate_to_rebuild_it() {
    let mut index = SearchableIndex::new("notes", None, 4);
    index.index_searchable_items(vec![Item::new("a", "notes")]).unwrap();
    let mut delegate = Rebuild { everything: false, asked: vec![] };
    index.index_lost(&mut delegate);
    assert!(delegate.everything);
    assert_eq!(ids(index.items()), ["z"]);
    assert_eq!(index.fetch_last_client_state(), Ok(vec![]));
}

#[test]
fn corrupt_items_are_removed_and_only_those_identifiers_are_reindexed() {
    let mut index = SearchableIndex::new("notes", None, 4);
    index.index_searchable_items(vec![Item::new("a", "notes"), Item::new("b", "notes")]).unwrap();
    let mut delegate = Rebuild { everything: false, asked: vec![] };
    index.items_corrupt(&["a".to_string()], &mut delegate);
    assert_eq!(ids(index.items()), ["b"]);
    assert_eq!(delegate.asked, ["a"]);
    assert!(!delegate.everything);
}

// --- Split view and popover placement -------------------------------------------------------------------------------

#[test]
fn the_divider_respects_both_minimum_widths() {
    let mut s = SplitView::new(1000.0, 300.0, 200.0, 400.0);
    assert_eq!(s.leading(), 300.0);
    s.drag_to(10.0);
    assert_eq!(s.leading(), 200.0, "the leading pane keeps its minimum");
    s.drag_to(5000.0);
    assert_eq!(s.leading(), 600.0, "the trailing pane keeps its minimum");
    assert_eq!(SplitView::new(1000.0, 900.0, 200.0, 400.0).leading(), 600.0);
}

#[test]
fn resizing_the_window_keeps_the_leading_width_as_far_as_the_minimums_allow() {
    let mut s = SplitView::new(1000.0, 300.0, 200.0, 400.0);
    s.set_total(500.0);
    assert_eq!(s.leading(), 200.0);
}

#[test]
fn a_collapsed_leading_pane_has_no_width_and_dragging_reopens_it() {
    let mut s = SplitView::new(1000.0, 300.0, 200.0, 400.0);
    assert!(s.toggle(), "toggling a shown pane collapses it");
    assert!(s.is_collapsed());
    assert_eq!(s.leading(), 0.0);
    assert!(!s.toggle());
    s.set_collapsed(true);
    s.drag_to(400.0);
    assert!(!s.is_collapsed());
    assert_eq!(s.leading(), 400.0);
}

#[test]
fn the_split_view_lays_out_leading_divider_and_trailing_panes() {
    let s = SplitView::new(1000.0, 300.0, 200.0, 400.0);
    let (lead, divider, trail) = s.rects(rect(0.0, 0.0, 1000.0, 500.0), 1.0);
    assert_eq!(lead, rect(0.0, 0.0, 300.0, 500.0));
    assert_eq!(divider, rect(300.0, 0.0, 1.0, 500.0));
    assert_eq!(trail, rect(301.0, 0.0, 699.0, 500.0));
    let mut hidden = s;
    hidden.set_collapsed(true);
    let (lead, divider, trail) = hidden.rects(rect(0.0, 0.0, 1000.0, 500.0), 1.0);
    assert_eq!((lead.w, divider.w, trail.x, trail.w), (0.0, 0.0, 0.0, 1000.0));
}

#[test]
fn the_divider_is_hit_within_its_slop_only_while_shown() {
    let s = SplitView::new(1000.0, 300.0, 200.0, 400.0);
    assert!(s.hits_divider(302.0, 4.0));
    assert!(!s.hits_divider(320.0, 4.0));
    let mut hidden = s;
    hidden.set_collapsed(true);
    assert!(!hidden.hits_divider(300.0, 4.0));
}

#[test]
fn a_popover_goes_below_its_anchor_when_there_is_room() {
    let anchor = rect(400.0, 300.0, 100.0, 20.0);
    let (r, edge) = place_popover(anchor, (200.0, 100.0), rect(0.0, 0.0, 1000.0, 800.0), Edge::Bottom, 8.0);
    assert_eq!(edge, Edge::Bottom);
    assert_eq!(r, rect(350.0, 328.0, 200.0, 100.0));
}

#[test]
fn a_popover_flips_to_the_opposite_edge_when_the_preferred_side_has_no_room() {
    let anchor = rect(400.0, 760.0, 100.0, 20.0);
    let (r, edge) = place_popover(anchor, (200.0, 100.0), rect(0.0, 0.0, 1000.0, 800.0), Edge::Bottom, 8.0);
    assert_eq!(edge, Edge::Top);
    assert_eq!(r, rect(350.0, 652.0, 200.0, 100.0));
}

#[test]
fn a_popover_on_the_right_edge_opens_to_the_left() {
    let anchor = rect(950.0, 300.0, 20.0, 20.0);
    let (r, edge) = place_popover(anchor, (200.0, 100.0), rect(0.0, 0.0, 1000.0, 800.0), Edge::Right, 8.0);
    assert_eq!(edge, Edge::Left);
    assert_eq!(r, rect(742.0, 260.0, 200.0, 100.0));
}

#[test]
fn a_popover_stays_on_screen() {
    let anchor = rect(0.0, 300.0, 10.0, 20.0);
    let (r, _) = place_popover(anchor, (200.0, 100.0), rect(0.0, 0.0, 1000.0, 800.0), Edge::Bottom, 8.0);
    assert_eq!(r.x, 0.0);
}

#[test]
fn the_rows_a_table_keeps_cells_for_follow_the_viewport() {
    assert_eq!(visible_rows(0.0, 100.0, 20.0, 50, 1), 0..6);
    assert_eq!(visible_rows(200.0, 100.0, 20.0, 50, 1), 9..16);
    assert_eq!(visible_rows(0.0, 100.0, 20.0, 3, 1), 0..3);
    assert_eq!(visible_rows(0.0, 100.0, 0.0, 50, 1), 0..0);
    assert!(visible_rows(900.0, 100.0, 20.0, 10, 0).is_empty());
}

// --- Pop-up and menu model --------------------------------------------------------------------------------------------

fn sample_menu() -> Menu {
    Menu::new(vec![MenuItem::new(1, "Open"), MenuItem::separator(), MenuItem::new(2, "Save").enabled(false), MenuItem::new(3, "Close")])
}

#[test]
fn arrow_keys_skip_separators_and_disabled_rows_and_wrap() {
    let mut m = sample_menu();
    assert_eq!(m.step(1), Some(0));
    assert_eq!(m.step(1), Some(3), "the separator and the disabled row are skipped");
    assert_eq!(m.step(1), Some(0), "the step wraps to the top");
    assert_eq!(m.step(-1), Some(3));
    assert_eq!(m.activate(), Some(3));
}

#[test]
fn first_and_last_pick_the_ends_of_the_menu() {
    let mut m = sample_menu();
    assert_eq!(m.first(), Some(0));
    assert_eq!(m.last(), Some(3));
}

#[test]
fn a_menu_with_nothing_selectable_has_no_highlight() {
    assert_eq!(Menu::new(vec![MenuItem::separator()]).step(1), None);
    assert_eq!(Menu::new(vec![]).step(1), None);
    assert_eq!(Menu::new(vec![MenuItem::new(1, "Open").enabled(false)]).activate(), None);
}

#[test]
fn a_pop_up_menu_checks_and_highlights_its_selected_row() {
    let m = Menu::popup(vec![MenuItem::new(1, "A"), MenuItem::new(2, "B"), MenuItem::new(3, "C")], 1);
    assert_eq!(m.selected(), Some(1));
    assert_eq!(m.highlighted, Some(1));
    let disabled = Menu::popup(vec![MenuItem::new(1, "A").enabled(false), MenuItem::new(2, "B")], 0);
    assert_eq!(disabled.selected(), Some(0));
    assert_eq!(disabled.highlighted, None, "a disabled row is not highlighted");
}

#[test]
fn typing_selects_the_next_row_whose_title_starts_with_the_typed_text() {
    let mut m = Menu::new(vec![MenuItem::new(1, "Almond"), MenuItem::new(2, "Apricot"), MenuItem::new(3, "Banana"), MenuItem::new(4, "Blueberry")]);
    assert_eq!(m.type_select('b', 0.0), Some(2));
    assert_eq!(m.type_select('l', 0.5), Some(3), "the typed prefix is kept within one second");
    assert_eq!(m.type_select('z', 5.0), None, "a pause starts a new prefix; nothing starts with z");
    assert_eq!(m.highlighted, Some(3), "a miss leaves the highlight where it was");
}

#[test]
fn repeating_one_letter_cycles_through_the_rows_that_start_with_it() {
    let mut m = Menu::new(vec![MenuItem::new(1, "Almond"), MenuItem::new(2, "Apricot"), MenuItem::new(3, "Avocado")]);
    assert_eq!(m.type_select('a', 0.0), Some(0));
    assert_eq!(m.type_select('a', 0.2), Some(1));
    assert_eq!(m.type_select('a', 0.4), Some(2));
    assert_eq!(m.type_select('a', 0.6), Some(0), "the cycle wraps");
}

#[test]
fn the_menu_lays_out_rows_and_hits_only_selectable_ones() {
    let m = Menu::new(vec![MenuItem::new(1, "A"), MenuItem::separator(), MenuItem::new(2, "B")]);
    let (rows, height) = m.rows(20.0, 5.0, 4.0);
    assert_eq!(rows, [(4.0, 20.0), (24.0, 5.0), (29.0, 20.0)]);
    assert_eq!(height, 53.0);
    assert_eq!(m.hit(10.0, 20.0, 5.0, 4.0), Some(0));
    assert_eq!(m.hit(25.0, 20.0, 5.0, 4.0), None, "over the separator");
    assert_eq!(m.hit(2.0, 20.0, 5.0, 4.0), None, "over the padding");
    assert_eq!(m.hit(35.0, 20.0, 5.0, 4.0), Some(2));
}

#[test]
fn a_pop_up_menu_slides_inside_the_screen() {
    let bounds = rect(0.0, 0.0, 1000.0, 800.0);
    assert_eq!(popup_origin(rect(100.0, 50.0, 80.0, 24.0), 120.0, 200.0, 30.0, bounds), (100.0, 20.0));
    assert_eq!(popup_origin(rect(950.0, 10.0, 40.0, 24.0), 120.0, 200.0, 30.0, bounds), (880.0, 0.0));
}

// --- Navigation stack and path ----------------------------------------------------------------------------------------

#[test]
fn pushing_and_popping_screens_tracks_the_titles_and_the_back_title() {
    let mut nav = NavigationStack::new("Root", 1);
    assert!(!nav.can_go_back());
    nav.push("Detail", 2);
    nav.push("Edit", 3);
    assert_eq!(nav.depth(), 3);
    assert_eq!(nav.path(), ["Root", "Detail", "Edit"]);
    assert_eq!(nav.title(), "Edit");
    assert_eq!(nav.back_title(), Some("Detail"));
    assert_eq!(*nav.top(), 3);
    assert_eq!(nav.pop(), Some(("Edit".to_string(), 3)));
    assert_eq!(nav.pop_to_root(), 1);
    assert_eq!(nav.depth(), 1);
    assert_eq!(nav.pop(), None, "the root can never be popped");
    assert_eq!(nav.back_title(), None);
}

#[test]
fn popping_to_an_index_keeps_the_screens_up_to_it() {
    let mut nav = NavigationStack::new("Root", 0);
    nav.push("A", 1);
    nav.push("B", 2);
    nav.push("C", 3);
    assert_eq!(nav.pop_to(1), 2);
    assert_eq!(nav.path(), ["Root", "A"]);
    *nav.top_mut() = 9;
    assert_eq!(*nav.top(), 9);
}

#[test]
fn a_navigation_path_rebuilds_the_stack_from_its_destinations() {
    let mut path = NavigationPath::new();
    assert!(path.is_empty());
    path.append(10);
    path.append(20);
    assert_eq!(path.count(), 2);
    assert_eq!(path.last(), Some(&20));
    let mut nav = NavigationStack::new("Root", 0);
    nav.push("Stale", 99);
    let pushed = nav.apply_path(&path, |d| (format!("Item {d}"), d * 2));
    assert_eq!(pushed, 2);
    assert_eq!(nav.path(), ["Root", "Item 10", "Item 20"]);
    assert_eq!(*nav.top(), 40);
    assert_eq!(path.remove_last(1), 1);
    assert_eq!(path.count(), 1);
}

// --- Accessibility element map and announcements ----------------------------------------------------------------------

#[test]
fn accessibility_names_slug_to_lowercase_words_joined_by_dashes() {
    assert_eq!(slug("New Note"), "new-note");
    assert_eq!(slug("  New   Note!! "), "new-note");
    assert_eq!(slug("Mjólk Ö"), "mjolk-o");
    assert_eq!(slug("Settings > General"), "settings-general");
}

#[test]
fn a_repeated_name_under_one_parent_gets_a_numbered_address() {
    let mut m = Map::new();
    assert_eq!(m.add("os/settings", "button", "New Note", "", true, None, "n1"), "os/settings/new-note");
    assert_eq!(m.add("os/settings", "button", "New Note", "", true, None, "n2"), "os/settings/new-note-2");
    assert_eq!(m.add("os/settings", "button", "New Note", "", true, None, "n3"), "os/settings/new-note-3");
    assert_eq!(m.add("os/settings", "toggle", "", "", true, None, "t"), "os/settings/toggle", "an unnamed element takes its role");
    assert_eq!(m.get("os/settings/new-note").map(|e| e.handle.as_str()), Some("n1"));
    assert_eq!(m.len(), 4);
}

#[test]
fn everything_under_a_prefix_is_listed_in_the_order_added() {
    let mut m = Map::new();
    m.add("os/settings", "button", "New Note", "", true, None, "");
    m.add("os/settings", "button", "New Note", "", true, None, "");
    m.add("os/other", "button", "Other", "", true, None, "");
    let under: Vec<&str> = m.under("os/settings/").map(|e| e.address.as_str()).collect();
    assert_eq!(under, ["os/settings/new-note", "os/settings/new-note-2"]);
    assert_eq!(m.under("").count(), 3, "an empty prefix is the whole map");
    assert_eq!(m.under("os/missing").count(), 0);
}

#[test]
fn an_agent_query_resolves_by_address_by_path_or_by_name() {
    let mut m = Map::new();
    m.add("os/settings/general", "row", "Software Update", "", true, None, "su");
    m.add("os/settings", "button", "New Note", "", true, None, "");
    assert!(matches!(m.resolve("os/settings/general/software-update"), Resolve::One(e) if e.name == "Software Update"));
    assert!(matches!(m.resolve("software update"), Resolve::One(e) if e.address == "os/settings/general/software-update"));
    assert!(matches!(m.resolve("Settings > General > Software Update"), Resolve::One(e) if e.handle == "su"));
    assert_eq!(m.resolve("nothing like this"), Resolve::None);
}

#[test]
fn a_query_that_matches_two_elements_is_ambiguous_and_lists_both() {
    let mut m = Map::new();
    m.add("a/x", "button", "Save", "", true, None, "");
    m.add("b/x", "button", "Save", "", true, None, "");
    assert_eq!(m.resolve("save"), Resolve::Ambiguous(vec!["a/x/save", "b/x/save"]));
}

#[test]
fn custom_actions_are_named_once_per_element_without_regard_to_case() {
    let mut m = Map::new();
    let address = m.add("os/files", "row", "Notes", "", true, None, "");
    assert!(m.add_custom_action(&address, "Move up"));
    assert!(!m.add_custom_action(&address, "move   UP"), "the same name in another case is refused");
    assert!(!m.add_custom_action(&address, "   "), "a blank name is refused");
    assert!(!m.add_custom_action("os/files/missing", "Archive"), "an element that is not there takes nothing");
    assert_eq!(m.custom_actions(&address).collect::<Vec<_>>(), ["Move up"]);
    assert!(m.has_custom_action(&address, "MOVE UP"));
}

#[test]
fn assertive_announcements_are_spoken_first_and_repeats_are_dropped() {
    let mut a = Announcements::with_capacity(4);
    assert!(a.post("  Saved   file ", Politeness::Polite));
    assert!(!a.post("Saved file", Politeness::Polite), "a repeat of the last announcement is dropped");
    assert!(!a.post("   ", Politeness::Polite), "blank text is dropped");
    assert!(a.post("Error", Politeness::Assertive));
    assert_eq!(a.take_next(), Some(Announcement { text: "Error".into(), politeness: Politeness::Assertive }));
    assert_eq!(a.take_next(), Some(Announcement { text: "Saved file".into(), politeness: Politeness::Polite }));
    assert_eq!(a.take_next(), None);
}

#[test]
fn a_full_queue_drops_the_oldest_polite_announcement_and_keeps_the_assertive_one() {
    let mut a = Announcements::with_capacity(2);
    a.post("one", Politeness::Polite);
    a.post("two", Politeness::Assertive);
    a.post("three", Politeness::Polite);
    assert_eq!(a.len(), 2);
    assert_eq!(a.take_next().map(|x| x.text), Some("two".to_string()));
    assert_eq!(a.take_next().map(|x| x.text), Some("three".to_string()));
    a.post("four", Politeness::Polite);
    a.clear();
    assert!(a.is_empty());
}
