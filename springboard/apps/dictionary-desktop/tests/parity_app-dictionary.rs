//! Parity app-dictionary: headless tests of the Dictionary model through its public API. Lookup history, dictionary
//! scope, stored preferences, find in the definition, cross-reference words, settings row dragging, print and copy text,
//! and the `dict:` and `x-dictionary:` URLs. No test draws a frame or opens a window.

use bifrost_dictionary::model::{
    cycle_scope, definition_text, drop_slot, find_matches, find_spans, folder_url, in_scope, link_runs, preview_order, row_at,
    scope_names, step_match, word_from_url, word_spans, History, Match, Prefs, Run, MAX_STEP, MIN_STEP,
};
use bifrost_ui::stardict::Hit;

fn hit(dictionary: &str, text: &str) -> Hit {
    Hit { dictionary: dictionary.into(), text: text.into() }
}

fn names(prefs: &Prefs) -> Vec<&str> {
    prefs.dicts.iter().map(|d| d.name.as_str()).collect()
}

#[test]
fn history_steps_back_and_forward_through_the_words_looked_up() {
    let mut h = History::default();
    assert!(h.is_empty() && !h.can_back() && !h.can_forward());
    h.visit("alpha");
    h.visit("beta");
    h.visit("gamma");
    assert_eq!(h.current(), Some("gamma"));
    assert_eq!(h.back().as_deref(), Some("beta"));
    assert_eq!(h.back().as_deref(), Some("alpha"));
    assert_eq!(h.back(), None, "Back stops at the first word");
    assert_eq!(h.forward().as_deref(), Some("beta"));
    assert_eq!(h.forward().as_deref(), Some("gamma"));
    assert_eq!(h.forward(), None, "Forward stops at the last word");
}

#[test]
fn looking_up_the_word_on_screen_again_adds_no_history_entry() {
    let mut h = History::default();
    h.visit("alpha");
    h.visit("alpha");
    assert!(!h.can_back(), "one entry, so nothing to go back to");
    h.visit("beta");
    assert_eq!(h.back().as_deref(), Some("alpha"));
    assert_eq!(h.forward().as_deref(), Some("beta"));
}

#[test]
fn a_new_word_after_going_back_drops_the_forward_words() {
    let mut h = History::default();
    h.visit("alpha");
    h.visit("beta");
    h.visit("gamma");
    h.back();
    h.visit("delta");
    assert!(!h.can_forward());
    assert_eq!(h.back().as_deref(), Some("beta"));
    assert_eq!(h.back().as_deref(), Some("alpha"));
}

#[test]
fn clearing_the_history_forgets_every_word() {
    let mut h = History::default();
    h.visit("alpha");
    h.visit("beta");
    h.clear();
    assert!(h.is_empty() && h.current().is_none() && !h.can_back());
}

#[test]
fn definition_text_names_each_dictionary_above_its_definition() {
    assert_eq!(definition_text(&[hit("Alpha", "one"), hit("Beta", "two\nlines")]), "Alpha\none\n\nBeta\ntwo\nlines");
    assert_eq!(definition_text(&[]), "");
}

#[test]
fn folder_url_percent_encodes_unsafe_bytes_and_keeps_slashes() {
    assert_eq!(folder_url("/home/olibuijr/dictionaries"), "file:///home/olibuijr/dictionaries");
    assert_eq!(folder_url("/home/a b/dict"), "file:///home/a%20b/dict");
    assert_eq!(folder_url("/x/\u{e9}"), "file:///x/%C3%A9");
}

#[test]
fn dict_and_x_dictionary_urls_give_the_word_to_look_up() {
    assert_eq!(word_from_url("dict://serendipity").as_deref(), Some("serendipity"));
    assert_eq!(word_from_url("dict:caf%C3%A9").as_deref(), Some("caf\u{e9}"));
    assert_eq!(word_from_url("x-dictionary:r:word").as_deref(), Some("word"));
    assert_eq!(word_from_url("x-dictionary:word").as_deref(), Some("word"));
    assert_eq!(word_from_url("DICT:///Hello%20World").as_deref(), Some("Hello World"));
    assert_eq!(word_from_url("  dict:cat  ").as_deref(), Some("cat"));
}

#[test]
fn urls_without_a_word_or_of_another_scheme_give_nothing() {
    assert_eq!(word_from_url("https://example.com"), None);
    assert_eq!(word_from_url("dict://"), None);
    assert_eq!(word_from_url("x-dictionary:r:"), None);
    assert_eq!(word_from_url("dict:100%").as_deref(), Some("100%"), "a lone percent sign is kept");
}

#[test]
fn stored_preferences_round_trip_through_their_text() {
    let mut p = Prefs::default();
    p.sync(&["Alpha".into(), "Beta".into()]);
    p.set_label(1, "  Bee ");
    p.toggle(0);
    p.bigger();
    let back = Prefs::parse(&p.serialize());
    assert_eq!(p, back);
    assert_eq!(back.dicts[1].shown_name(), "Bee");
    assert!(!back.dicts[0].enabled);
}

#[test]
fn syncing_keeps_known_choices_appends_new_dictionaries_and_drops_missing_ones() {
    let mut p = Prefs::default();
    p.sync(&["A".into(), "B".into()]);
    p.toggle(0);
    assert!(p.sync(&["B".into(), "C".into()]), "a removed and an added dictionary change the list");
    assert_eq!(names(&p), ["B", "C"]);
    assert!(p.is_enabled("C"), "a new dictionary starts enabled");
    assert!(!p.sync(&["B".into(), "C".into()]), "the same installed set changes nothing");
}

#[test]
fn unreadable_preference_lines_are_skipped_and_the_size_is_clamped() {
    let p = Prefs::parse("size=99\nnot a setting\ndict\t1\t\n");
    assert_eq!(p.step, MAX_STEP);
    assert!(p.dicts.is_empty());
    assert_eq!(Prefs::parse("size=-99").step, MIN_STEP);
}

#[test]
fn text_size_steps_clamp_at_both_ends_and_scale_the_base_size() {
    let mut p = Prefs::default();
    for _ in 0..20 {
        p.bigger();
    }
    assert_eq!(p.step, MAX_STEP);
    for _ in 0..20 {
        p.smaller();
    }
    assert_eq!(p.step, MIN_STEP);
    assert!((p.scaled(10.0) - 7.0).abs() < 1e-4, "three steps down scale a 10 point size to 7");
    p.actual_size();
    assert_eq!(p.scaled(13.0), 13.0, "Actual Size is the base size");
}

#[test]
fn arranging_hits_follows_the_stored_order_labels_and_enabled_choices() {
    let mut p = Prefs::default();
    p.sync(&["A".into(), "B".into(), "C".into()]);
    p.move_dict(2, -2);
    p.toggle(1);
    p.set_label(0, "Cee");
    let out = p.arrange(vec![hit("A", "a"), hit("B", "b"), hit("C", "c"), hit("Z", "z")]);
    let shown: Vec<&str> = out.iter().map(|h| h.dictionary.as_str()).collect();
    assert_eq!(shown, ["Cee", "B", "Z"], "A is off, C is relabelled, and Z is shown as installed");
}

#[test]
fn moving_a_dictionary_clamps_at_the_ends_of_the_list() {
    let mut p = Prefs::default();
    p.sync(&["A".into(), "B".into()]);
    p.move_dict(0, -5);
    assert_eq!(names(&p), ["A", "B"]);
    p.move_dict(0, 9);
    assert_eq!(names(&p), ["B", "A"]);
    p.move_dict(7, 1);
    assert_eq!(names(&p), ["B", "A"], "an index past the list moves nothing");
}

#[test]
fn a_drag_places_a_dictionary_at_the_drop_row() {
    let mut p = Prefs::default();
    p.sync(&["A".into(), "B".into(), "C".into()]);
    p.move_to(0, 2);
    assert_eq!(names(&p), ["B", "C", "A"]);
    p.move_to(2, 0);
    assert_eq!(names(&p), ["A", "B", "C"]);
}

#[test]
fn an_unknown_dictionary_counts_as_enabled() {
    let mut p = Prefs::default();
    p.sync(&["B".into()]);
    p.toggle(0);
    assert!(!p.is_enabled("B"));
    assert!(p.is_enabled("unknown"));
}

#[test]
fn renaming_trims_the_label_and_an_empty_label_shows_the_real_name() {
    let mut p = Prefs::default();
    p.sync(&["Alpha".into()]);
    p.set_label(0, "   ");
    assert_eq!(p.dicts[0].shown_name(), "Alpha");
    p.set_label(0, "  Beta  ");
    assert_eq!(p.dicts[0].shown_name(), "Beta");
    p.set_label(9, "ignored");
    assert_eq!(p.dicts[0].label, "Beta");
}

#[test]
fn scope_offers_each_dictionary_that_answered_once_in_order() {
    let hits = vec![hit("A", "1"), hit("B", "2"), hit("A", "3")];
    assert_eq!(scope_names(&hits), ["A", "B"]);
    assert!(scope_names(&[]).is_empty());
}

#[test]
fn scope_cycles_through_all_and_each_dictionary_in_both_directions() {
    let names = vec!["A".to_string(), "B".to_string()];
    let a = cycle_scope(&None, &names, 1);
    assert_eq!(a.as_deref(), Some("A"));
    let b = cycle_scope(&a, &names, 1);
    assert_eq!(b.as_deref(), Some("B"));
    assert_eq!(cycle_scope(&b, &names, 1), None, "Next after the last dictionary is All");
    assert_eq!(cycle_scope(&None, &names, -1).as_deref(), Some("B"), "Previous from All is the last dictionary");
    assert_eq!(cycle_scope(&None, &[], 1), None, "no dictionaries leaves All");
}

#[test]
fn a_scope_keeps_only_the_hits_of_its_dictionary() {
    let hits = vec![hit("A", "1"), hit("B", "2"), hit("A", "3")];
    let only_a = in_scope(&hits, &Some("A".into()));
    assert_eq!(only_a.len(), 2);
    assert!(only_a.iter().all(|h| h.dictionary == "A"));
    assert_eq!(in_scope(&hits, &None).len(), 3);
}

#[test]
fn find_highlights_every_occurrence_in_reading_order() {
    let lines = ["Hello world, world", "nothing", "WORLD again"];
    let found = find_matches(&lines, " world ");
    assert_eq!(
        found,
        [Match { line: 0, start: 6, end: 11 }, Match { line: 0, start: 13, end: 18 }, Match { line: 2, start: 0, end: 5 }]
    );
    assert!(find_matches(&lines, "  ").is_empty(), "a blank query finds nothing");
}

#[test]
fn find_next_and_previous_wrap_around_the_matches() {
    let all = [0usize, 1, 2];
    assert_eq!(step_match(&all, None, 1), Some(0));
    assert_eq!(step_match(&all, None, -1), Some(2));
    assert_eq!(step_match(&all, Some(2), 1), Some(0));
    assert_eq!(step_match(&all, Some(0), -1), Some(2));
    assert_eq!(step_match(&[], None, 1), None, "no matches means nothing to step to");
}

#[test]
fn find_spans_never_overlap_and_a_longer_query_than_the_text_finds_nothing() {
    assert_eq!(find_spans("aaaa", "aa"), [(0, 2), (2, 4)]);
    assert!(find_spans("abc", "").is_empty());
    assert!(find_spans("ab", "abc").is_empty());
}

#[test]
fn find_ignores_case_and_keeps_spans_on_character_boundaries() {
    assert_eq!(find_spans("Caf\u{e9} CAF\u{c9}", "caf\u{e9}"), [(0, 5), (6, 11)]);
    assert_eq!(find_spans("x\u{130}y", "y"), [(3, 4)]);
    let text = "na\u{ef}ve na\u{ef}ve";
    for (s, e) in find_spans(text, "\u{ef}v") {
        assert!(text.is_char_boundary(s) && text.is_char_boundary(e));
    }
}

#[test]
fn word_spans_leave_out_the_punctuation_around_each_word() {
    let text = "  (see \"serendipity\", chance.) -- ok";
    let words: Vec<&str> = word_spans(text).iter().map(|&(s, e)| &text[s..e]).collect();
    assert_eq!(words, ["see", "serendipity", "chance", "ok"]);
    assert!(word_spans("... ---").is_empty());
    assert!(word_spans("").is_empty());
}

#[test]
fn link_runs_mark_the_linked_words_and_cover_the_whole_line() {
    let text = "a fortunate chance, really";
    let runs = link_runs(text, |w| w == "chance" || w == "really");
    assert_eq!(
        runs,
        [
            Run { start: 0, end: 12, link: false },
            Run { start: 12, end: 18, link: true },
            Run { start: 18, end: 20, link: false },
            Run { start: 20, end: 26, link: true },
        ]
    );
    let joined: String = runs.iter().map(|r| &text[r.start..r.end]).collect();
    assert_eq!(joined, text);
}

#[test]
fn a_line_without_links_is_one_plain_run_and_an_empty_line_has_none() {
    let text = "nothing linked here";
    assert_eq!(link_runs(text, |_| false), [Run { start: 0, end: text.len(), link: false }]);
    assert!(link_runs("", |_| true).is_empty());
}

#[test]
fn a_pointer_over_the_settings_rows_picks_the_row_under_it() {
    assert_eq!(row_at(10.0, 40.0, 32.0, 3), None, "above the rows");
    assert_eq!(row_at(40.0, 40.0, 32.0, 3), Some(0));
    assert_eq!(row_at(135.0, 40.0, 32.0, 3), Some(2));
    assert_eq!(row_at(136.0, 40.0, 32.0, 3), None, "below the last row");
}

#[test]
fn a_dragged_row_lands_in_the_slot_under_the_pointer_clamped_to_the_list() {
    assert_eq!(drop_slot(0.0, 40.0, 32.0, 3), 0);
    assert_eq!(drop_slot(75.0, 40.0, 32.0, 3), 1);
    assert_eq!(drop_slot(900.0, 40.0, 32.0, 3), 2);
    assert_eq!(drop_slot(5.0, 40.0, 32.0, 0), 0, "an empty list has slot zero");
}

#[test]
fn a_drag_previews_the_order_with_the_dragged_row_moved() {
    assert_eq!(preview_order(4, 0, 2), [1, 2, 0, 3]);
    assert_eq!(preview_order(4, 3, 0), [3, 0, 1, 2]);
    assert_eq!(preview_order(3, 1, 1), [0, 1, 2]);
    assert_eq!(preview_order(0, 0, 0), Vec::<usize>::new());
}
