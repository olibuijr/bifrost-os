//! Parity tests for the SDK-facing models of the widget kit, the post composer, the search field, the toolbar, the
//! accessibility announcements, the contact card format and the now-playing content types (task sdk-tests-01b).
//! They are headless host tests: pure models with no display, device, network or wall-clock time. Each test names the
//! module it exercises by its full path, so the parity verifier can match it to the matrix row.

// ---- widgetkit: configurable widgets (bifrost_ui::widget_config) ----

use bifrost_ui::widget_config::{ConfigError, Configuration, Parameter, Value};

fn city() -> Parameter {
    Parameter::Choice { key: "city".into(), options: vec!["Reykjavik".into(), "Akureyri".into()], default: 0 }
}

fn sample() -> Configuration {
    Configuration::new(vec![
        city(),
        Parameter::Flag { key: "compact".into(), default: false },
        Parameter::Number { key: "days".into(), min: 1, max: 7, default: 3 },
    ])
    .expect("valid definitions")
}

#[test]
fn headless_widget_config_options_report_their_defaults_until_set() {
    let c = sample();
    assert_eq!(c.value("city"), Some(Value::Choice(0)));
    assert_eq!(c.value("compact"), Some(Value::Flag(false)));
    assert_eq!(c.value("days"), Some(Value::Number(3)));
    assert!(!c.is_set("days"), "the default alone does not count as set");
    assert_eq!(c.value("missing"), None);
}

#[test]
fn headless_widget_config_choice_names_follow_the_chosen_index() {
    let mut c = sample();
    c.set("city", Value::Choice(1)).unwrap();
    assert_eq!(c.value("city"), Some(Value::Choice(1)));
    assert_eq!(c.choice_label("city"), Some("Akureyri"));
    assert!(c.is_set("city"));
    c.reset("city").unwrap();
    assert_eq!(c.choice_label("city"), Some("Reykjavik"), "reset brings the default back");
    assert_eq!(c.choice_label("compact"), None, "a flag has no choice name");
}

#[test]
fn headless_widget_config_numbers_stay_inside_their_range() {
    let mut c = sample();
    assert_eq!(c.set("days", Value::Number(8)), Err(ConfigError::OutOfRange("days".into())));
    assert_eq!(c.set("days", Value::Number(0)), Err(ConfigError::OutOfRange("days".into())));
    assert_eq!(c.value("days"), Some(Value::Number(3)), "a refused value changes nothing");
    c.set("days", Value::Number(7)).unwrap();
    assert_eq!(c.value("days"), Some(Value::Number(7)), "both ends are allowed");
}

#[test]
fn headless_widget_config_refuses_values_of_the_wrong_kind_and_unknown_options() {
    let mut c = sample();
    assert_eq!(c.set("compact", Value::Number(1)), Err(ConfigError::WrongType("compact".into())));
    assert_eq!(c.set("city", Value::Choice(2)), Err(ConfigError::OutOfRange("city".into())));
    assert_eq!(c.set("nope", Value::Flag(true)), Err(ConfigError::UnknownKey("nope".into())));
    assert_eq!(c.reset("nope"), Err(ConfigError::UnknownKey("nope".into())));
}

#[test]
fn headless_widget_config_reset_all_restores_every_default() {
    let mut c = sample();
    c.set("compact", Value::Flag(true)).unwrap();
    c.set("days", Value::Number(5)).unwrap();
    c.reset_all();
    assert_eq!(c.value("compact"), Some(Value::Flag(false)));
    assert_eq!(c.value("days"), Some(Value::Number(3)));
    assert!(!c.is_set("days"));
}

#[test]
fn headless_widget_config_refuses_inconsistent_definitions() {
    let dup = Configuration::new(vec![
        Parameter::Flag { key: "a".into(), default: true },
        Parameter::Flag { key: "a".into(), default: false },
    ]);
    assert_eq!(dup.unwrap_err(), ConfigError::DuplicateKey("a".into()));

    let no_options = Configuration::new(vec![Parameter::Choice { key: "c".into(), options: vec![], default: 0 }]);
    assert_eq!(no_options.unwrap_err(), ConfigError::NoOptions("c".into()));

    let bad_default = Configuration::new(vec![Parameter::Choice { key: "c".into(), options: vec!["x".into()], default: 1 }]);
    assert_eq!(bad_default.unwrap_err(), ConfigError::BadDefault("c".into()));

    let range = Configuration::new(vec![Parameter::Number { key: "n".into(), min: 5, max: 2, default: 3 }]);
    assert_eq!(range.unwrap_err(), ConfigError::BadRange("n".into()));

    let outside = Configuration::new(vec![Parameter::Number { key: "n".into(), min: 1, max: 2, default: 9 }]);
    assert_eq!(outside.unwrap_err(), ConfigError::BadDefault("n".into()));

    let unnamed = Configuration::new(vec![Parameter::Flag { key: String::new(), default: true }]);
    assert_eq!(unnamed.unwrap_err(), ConfigError::EmptyKey);
}

// ---- widgetkit: timeline updates (bifrost_ui::timeline) ----

use bifrost_ui::timeline::{Entry, ReloadPolicy, Timeline};

fn at(date: f64, value: &'static str) -> Entry<&'static str> {
    Entry { date, value }
}

#[test]
fn headless_timeline_shows_the_latest_entry_whose_date_has_passed() {
    let t = Timeline::new(vec![at(10.0, "a"), at(20.0, "b"), at(30.0, "c")], ReloadPolicy::AtEnd);
    assert_eq!(t.entry_at(10.0).map(|e| e.value), Some("a"), "an entry shows from its own date");
    assert_eq!(t.entry_at(19.9).map(|e| e.value), Some("a"));
    assert_eq!(t.entry_at(20.0).map(|e| e.value), Some("b"));
    assert_eq!(t.entry_at(10_000.0).map(|e| e.value), Some("c"), "the last entry stays until a reload");
    assert_eq!(t.entry_at(-5.0).map(|e| e.value), Some("a"), "before the first date the first entry shows");
}

#[test]
fn headless_timeline_keeps_entries_sorted_and_empty_timelines_show_nothing() {
    let t = Timeline::new(vec![at(30.0, "c"), at(10.0, "a"), at(10.0, "b")], ReloadPolicy::Never);
    let order: Vec<&str> = t.entries().iter().map(|e| e.value).collect();
    assert_eq!(order, ["a", "b", "c"], "equal dates keep the order they were given in");
    let empty: Timeline<&str> = Timeline::new(vec![], ReloadPolicy::AtEnd);
    assert!(empty.entry_at(0.0).is_none());
}

#[test]
fn headless_timeline_reload_policies_give_the_next_request_time() {
    let at_end = Timeline::new(vec![at(10.0, "a"), at(50.0, "b")], ReloadPolicy::AtEnd);
    assert_eq!(at_end.next_reload(0.0), Some(50.0), "the last entry's date");
    assert_eq!(at_end.next_reload(70.0), Some(70.0), "a passed date is due now, never in the past");

    let fixed = Timeline::new(vec![at(10.0, "a")], ReloadPolicy::After(300.0));
    assert_eq!(fixed.next_reload(100.0), Some(300.0));
    assert_eq!(fixed.next_reload(400.0), Some(400.0));

    let manual = Timeline::new(vec![at(10.0, "a")], ReloadPolicy::Never);
    assert_eq!(manual.next_reload(100.0), None, "Never waits for the owner");
    assert_eq!(manual.policy(), ReloadPolicy::Never);
}

// ---- widgetkit: smart stacks and relevance (bifrost_ui::relevance) ----

use bifrost_ui::relevance::{self, ClueError, DateKind, Item, Relevance, TimeClue, POINT_CLUE_SECONDS};

fn ranked(date: f64, score: f32, duration: f64) -> Item {
    Item { date, relevance: Some(Relevance { score, duration }) }
}

#[test]
fn headless_smart_stack_ranks_eligible_items_highest_score_first() {
    let items = [ranked(0.0, 1.0, 100.0), ranked(0.0, 3.0, 100.0), ranked(0.0, 2.0, 100.0)];
    assert_eq!(relevance::rank(&items, 10.0), vec![1, 2, 0]);
    let same = [ranked(0.0, 2.0, 100.0), ranked(0.0, 2.0, 100.0)];
    assert_eq!(relevance::rank(&same, 10.0), vec![0, 1], "equal scores keep their given order");
    assert!(relevance::rank(&[], 0.0).is_empty());
}

#[test]
fn headless_smart_stack_items_fall_behind_outside_their_window() {
    let items = [ranked(0.0, 9.0, 10.0), ranked(0.0, 1.0, 100.0), ranked(500.0, 8.0, 10.0), ranked(0.0, 4.0, 100.0)];
    // At t = 50 the first has expired and the third is not due yet; the eligible ones lead.
    assert_eq!(relevance::rank(&items, 50.0), vec![3, 1, 0, 2]);
    let no_hint = [Item { date: 0.0, relevance: None }, ranked(0.0, -1.0, 100.0)];
    assert_eq!(relevance::rank(&no_hint, 10.0), vec![1, 0], "an item without relevance never rotates to the front");
}

#[test]
fn headless_smart_stack_time_clues_move_the_score_inside_the_window() {
    let base = 2.0_f32;
    let scheduled = TimeClue::interval(0.0, 10.0, DateKind::Scheduled).unwrap();
    let informational = TimeClue::interval(0.0, 10.0, DateKind::Informational).unwrap();
    let default = TimeClue::between(0.0, 10.0).unwrap();
    assert_eq!(scheduled.score_at(base, 5.0), Some(3.0));
    assert_eq!(informational.score_at(base, 5.0), Some(1.5));
    assert_eq!(default.score_at(base, 5.0), Some(2.0));
    assert_eq!(scheduled.score_at(base, 10.0), None, "an interval is open at its end");
    let point = TimeClue::at(1000.0, DateKind::Default).unwrap();
    assert_eq!(point.window(), (1000.0, 1000.0 + POINT_CLUE_SECONDS));
    assert!(point.is_active(4599.0) && !point.is_active(4600.0));
}

#[test]
fn headless_smart_stack_refuses_empty_windows_and_non_finite_clues() {
    assert_eq!(TimeClue::interval(100.0, 100.0, DateKind::Default), Err(ClueError::EmptyWindow));
    assert_eq!(TimeClue::range(200.0, 100.0, DateKind::Default), Err(ClueError::EmptyWindow));
    assert_eq!(TimeClue::between(f64::NAN, 10.0), Err(ClueError::NotFinite));
    assert_eq!(TimeClue::at(f64::INFINITY, DateKind::Scheduled), Err(ClueError::NotFinite));
    let instant = TimeClue::range(50.0, 50.0, DateKind::Default).unwrap();
    assert!(instant.is_active(50.0), "a closed range may be a single instant");
}

// ---- widgetkit: deep links from widgets (bifrost_ui::widget_link) ----

use bifrost_ui::widget_link::{Link, LinkError, SCHEME};

#[test]
fn headless_widget_link_splits_the_app_and_its_path() {
    assert_eq!(SCHEME, "bifrost-app://");
    let l = Link::parse("bifrost-app://notes/folder/x").unwrap();
    assert_eq!(l.app(), "notes");
    assert_eq!(l.path(), "/folder/x");
    assert_eq!(l.to_url(), "bifrost-app://notes/folder/x");
    let root = Link::parse("bifrost-app://notes").unwrap();
    assert_eq!(root.path(), "/", "a link with no path goes to the app root");
}

#[test]
fn headless_widget_link_refuses_other_schemes_and_dot_segments() {
    assert_eq!(Link::parse("https://example.com/a"), Err(LinkError::NotAppScheme));
    assert_eq!(Link::parse("bifrost-app:///x"), Err(LinkError::MissingApp));
    assert_eq!(Link::parse("bifrost-app://notes/a/../b"), Err(LinkError::BadPath));
    assert_eq!(Link::new("notes", "./a"), Err(LinkError::BadPath));
    assert_eq!(Link::parse("bifrost-app://notes/a?x=1"), Err(LinkError::BadPath));
    assert_eq!(Link::new("notes", "/my file"), Err(LinkError::BadPath));
}

#[test]
fn headless_widget_link_drops_empty_segments_and_checks_app_identifiers() {
    assert_eq!(Link::new("notes", "a//b/").unwrap().to_url(), "bifrost-app://notes/a/b");
    assert_eq!(Link::new("No Good", "/"), Err(LinkError::BadAppId));
    assert_eq!(Link::new("", "/"), Err(LinkError::BadAppId));
    assert_eq!(Link::new("com.example-app.2", "/").unwrap().app(), "com.example-app.2");
}

// ---- notification center: search view of a widget (bifrost_ui::widget_search) ----

use bifrost_ui::widget_search::WidgetSearch;

fn result_title(s: &String) -> &str {
    s.as_str()
}

fn results(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn headless_widget_search_draws_the_placeholder_only_without_results() {
    let mut s = WidgetSearch::new(result_title).with_placeholder("No notes match");
    assert!(s.rows().is_empty());
    assert_eq!(s.placeholder(), Some("No notes match"));
    s.set_results(results(&["Groceries", "Plan"]));
    assert_eq!(s.placeholder(), None, "results replace the placeholder");
    assert_eq!(s.rows(), ["Groceries", "Plan"], "rows keep the order the search returned");
}

#[test]
fn headless_widget_search_a_new_search_replaces_the_old_results() {
    let mut s = WidgetSearch::new(result_title).with_placeholder("Nothing");
    s.set_results(results(&["a", "b"]));
    s.set_results(results(&["c"]));
    assert_eq!(s.rows(), ["c"]);
    s.clear();
    assert!(s.results().is_empty());
    assert_eq!(s.placeholder(), Some("Nothing"));
}

#[test]
fn headless_widget_search_description_and_placeholder_are_trimmed() {
    let s: WidgetSearch<String> = WidgetSearch::new(result_title).with_description("  Notes  ").with_placeholder("   ");
    assert_eq!(s.description(), Some("Notes"));
    assert_eq!(s.results_placeholder(), None, "blank text clears the placeholder");
    let cleared = s.with_description("");
    assert_eq!(cleared.description(), None);
}
