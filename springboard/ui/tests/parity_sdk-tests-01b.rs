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

// ---- social: the compose post sheet (bifrost_ui::compose) ----

use bifrost_ui::compose::{ComposeDraft, ComposeError, ComposeItem, ComposeResult, ComposeSheet, SheetState, MAX_IMAGES, MAX_URLS};

#[test]
fn headless_compose_draft_hands_off_text_then_images_then_links() {
    let mut d = ComposeDraft::new();
    assert!(d.is_empty());
    d.add_url("https://example.com/a").unwrap();
    d.add_image("photo.png").unwrap();
    d.set_initial_text("  Hello  ");
    assert_eq!(d.text(), "  Hello  ", "the text is kept as given");
    assert!(!d.is_empty());
    assert_eq!(
        d.items(),
        vec![
            ComposeItem::Text("  Hello  ".into()),
            ComposeItem::Image("photo.png".into()),
            ComposeItem::Url("https://example.com/a".into()),
        ]
    );
    d.set_initial_text("   ");
    assert_eq!(d.items().len(), 2, "blank text is not handed off");
}

#[test]
fn headless_compose_draft_refuses_bad_links_and_empty_items() {
    let mut d = ComposeDraft::new();
    assert_eq!(d.add_url("   "), Err(ComposeError::Empty));
    assert_eq!(d.add_image(" "), Err(ComposeError::Empty));
    assert_eq!(d.add_url("ftp://example.com"), Err(ComposeError::BadUrl("ftp://example.com".into())));
    assert_eq!(d.add_url("https://"), Err(ComposeError::BadUrl("https://".into())), "a host is required");
    assert_eq!(d.add_url("https://a b.test"), Err(ComposeError::BadUrl("https://a b.test".into())), "no spaces");
    d.add_url("HTTPS://example.com/ok").expect("the scheme ignores case");
    assert_eq!(d.urls(), ["HTTPS://example.com/ok"]);
    d.remove_all_urls();
    assert!(d.urls().is_empty());
}

#[test]
fn headless_compose_draft_images_and_links_stop_at_their_limits() {
    assert_eq!(MAX_IMAGES, 8);
    assert_eq!(MAX_URLS, 8);
    let mut d = ComposeDraft::new();
    for i in 0..MAX_IMAGES {
        d.add_image(&format!("img{i}.png")).unwrap();
    }
    assert_eq!(d.add_image("one-more.png"), Err(ComposeError::Full(MAX_IMAGES)));
    for i in 0..MAX_URLS {
        d.add_url(&format!("https://example.com/{i}")).unwrap();
    }
    assert_eq!(d.add_url("https://example.com/extra"), Err(ComposeError::Full(MAX_URLS)));
    d.remove_all_images();
    assert!(d.images().is_empty());
    assert_eq!(d.urls().len(), MAX_URLS, "removing images leaves the links");
}

#[test]
fn headless_compose_sheet_locks_once_presented_and_reports_one_result() {
    let mut sheet = ComposeSheet::new();
    assert_eq!(sheet.state(), SheetState::Editing);
    sheet.set_initial_text("Draft").unwrap();
    assert!(sheet.present(), "the first presentation shows the sheet");
    assert!(!sheet.present(), "it is not shown twice");
    assert_eq!(sheet.add_image("late.png"), Err(ComposeError::Locked), "only the user changes the post now");
    assert_eq!(sheet.set_initial_text("again"), Err(ComposeError::Locked));
    assert_eq!(sheet.finish(true), Some(ComposeResult::Done));
    assert_eq!(sheet.state(), SheetState::Closed);
    assert_eq!(sheet.result(), Some(ComposeResult::Done));
    assert_eq!(sheet.finish(false), None, "a closed sheet reports nothing more");
    assert_eq!(sheet.draft().text(), "Draft");
}

#[test]
fn headless_compose_sheet_an_empty_or_cancelled_post_reports_cancelled() {
    let mut cancelled = ComposeSheet::new();
    cancelled.set_initial_text("Keep me out").unwrap();
    cancelled.present();
    assert_eq!(cancelled.finish(false), Some(ComposeResult::Cancelled));

    let mut empty = ComposeSheet::new();
    empty.present();
    assert_eq!(empty.finish(true), Some(ComposeResult::Cancelled), "sending nothing is a cancel");
    assert_eq!(ComposeDraft::new().dismiss(true), ComposeResult::Cancelled);
}

// ---- shared app data: search field geometry (bifrost_ui::search) ----

use bifrost_ui::search::{hits_clear, layout as search_layout, GLYPH_PT, INSET_PT};
use bifrost_ui::Rect;

const FIELD: Rect = Rect { x: 100.0, y: 10.0, w: 200.0, h: 28.0 };

#[test]
fn headless_search_field_glyphs_sit_at_the_capsule_edges() {
    let empty = search_layout(FIELD, false);
    assert_eq!(empty.magnifier.x, FIELD.x + INSET_PT, "the magnifier is inset from the leading edge");
    assert_eq!(empty.magnifier.w, GLYPH_PT);
    assert_eq!(empty.magnifier.y + empty.magnifier.h / 2.0, FIELD.y + FIELD.h / 2.0, "glyphs are centred vertically");
    assert!(empty.clear.is_none(), "the clear button shows only with text");
    assert_eq!(empty.text.x, empty.magnifier.x + GLYPH_PT + 6.0, "text starts six points after the magnifier");
}

#[test]
fn headless_search_field_with_text_shows_clear_and_narrows_the_text() {
    let empty = search_layout(FIELD, false);
    let full = search_layout(FIELD, true);
    let clear = full.clear.expect("the clear button is present with text");
    assert_eq!(clear.x + clear.w, FIELD.x + FIELD.w - INSET_PT, "the clear button is inset from the trailing edge");
    assert!(full.text.w < empty.text.w, "the text gives room to the clear button");
    assert!(full.text.x + full.text.w <= clear.x);
    assert!(hits_clear(FIELD, true, 290.0, 24.0), "a press on the button clears the field");
    assert!(hits_clear(FIELD, true, 290.0, 11.0), "the target is padded to the full height");
    assert!(!hits_clear(FIELD, true, 150.0, 24.0), "the text area is not the button");
    assert!(!hits_clear(FIELD, false, 290.0, 24.0), "nothing to clear");
}

// ---- shared app data: toolbars with groups and overflow (bifrost_ui::nav) ----

use bifrost_ui::nav::{Toolbar, ToolbarItem, ToolbarKind};

fn buttons(ids: &[u32]) -> Vec<ToolbarItem> {
    ids.iter().map(|&id| ToolbarItem::button(id, "Action")).collect()
}

#[test]
fn headless_toolbar_lays_every_button_when_the_bar_is_wide_enough() {
    let bar = Toolbar::new(buttons(&[1, 2, 3]));
    let layout = bar.layout(Rect { x: 0.0, y: 0.0, w: 500.0, h: 30.0 }, 40.0, 4.0);
    let ids: Vec<u32> = layout.visible.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, [1, 2, 3]);
    assert!(layout.overflow.is_empty());
    assert!(layout.overflow_button.is_none(), "no overflow button when nothing overflows");
    let first = layout.rect_of(1).expect("button 1 is on the bar");
    assert_eq!(first.x, 0.0);
    assert_eq!(layout.hit(first.x + 1.0, first.y + 1.0), Some(1));
}

#[test]
fn headless_toolbar_trailing_buttons_move_to_the_overflow_menu_when_narrow() {
    let bar = Toolbar::new(buttons(&[1, 2, 3, 4, 5]));
    let layout = bar.layout(Rect { x: 0.0, y: 0.0, w: 150.0, h: 30.0 }, 40.0, 4.0);
    let visible: Vec<u32> = layout.visible.iter().map(|(id, _)| *id).collect();
    assert_eq!(visible.len() + layout.overflow.len(), 5, "every button is either on the bar or in the menu");
    assert!(!layout.overflow.is_empty(), "something overflows");
    assert!(layout.overflow_button.is_some(), "the overflow button is shown");
    for (i, id) in visible.iter().enumerate() {
        assert_eq!(*id, i as u32 + 1, "the bar keeps the leading buttons in order");
    }
}

#[test]
fn headless_toolbar_buttons_of_one_group_share_a_capsule() {
    let items = vec![
        ToolbarItem::button(1, "Bold").in_group(1),
        ToolbarItem::button(2, "Italic").in_group(1),
        ToolbarItem::button(3, "Share").in_group(2),
    ];
    let layout = Toolbar::new(items).layout(Rect { x: 0.0, y: 0.0, w: 500.0, h: 30.0 }, 40.0, 4.0);
    assert_eq!(layout.groups.len(), 2, "one capsule per group on the bar");
    let (_, bold_italic) = layout.groups.iter().find(|(g, _)| *g == 1).expect("group 1 has a capsule");
    assert!(bold_italic.w >= 80.0, "the capsule covers both buttons of the group");
}

#[test]
fn headless_toolbar_validation_reports_only_real_changes() {
    let mut bar = Toolbar::new(vec![ToolbarItem::button(1, "Open"), ToolbarItem::button(2, "Save")]);
    assert!(bar.validate(|id| id != 2), "Save becomes disabled");
    assert!(!bar.items[1].enabled);
    assert!(bar.items[0].enabled);
    assert!(!bar.validate(|id| id != 2), "a second pass with the same answer changes nothing");
    assert_eq!(bar.items[0].kind, ToolbarKind::Button);
}

#[test]
fn headless_toolbar_menu_anchor_sits_below_the_button() {
    let layout = Toolbar::new(buttons(&[7])).layout(Rect { x: 10.0, y: 5.0, w: 300.0, h: 30.0 }, 40.0, 4.0);
    let r = layout.rect_of(7).expect("button 7");
    assert_eq!(layout.menu_anchor(7, 4.0), Some((r.x, r.y + r.h + 4.0)));
    assert_eq!(layout.menu_anchor(99, 4.0), None);
}

// ---- shared events and accessibility integration: fundamentals (bifrost_ui::ax) ----

use bifrost_ui::ax::{role_name, Announcements, Map, Politeness, Priority};
use bifrost_ui::Role;

#[test]
fn headless_accessibility_roles_have_their_spoken_names_and_priorities_parse() {
    assert_eq!(role_name(Role::Button), "button");
    assert_eq!(Priority::from_wire(2), Some(Priority::High));
    assert_eq!(Priority::from_wire(0), Some(Priority::Low));
    assert_eq!(Priority::from_wire(3), None);
    assert_eq!(Priority::default(), Priority::Medium);
}

#[test]
fn headless_accessibility_announcements_collapse_space_skip_repeats_and_speak_assertive_first() {
    let mut queue = Announcements::with_capacity(8);
    assert!(queue.post("  Saved   the note ", Politeness::Polite));
    assert!(!queue.post("Saved the note", Politeness::Polite), "a repeat of the last announcement is dropped");
    assert!(!queue.post("   ", Politeness::Polite), "blank text is dropped");
    assert!(queue.post("Upload failed", Politeness::Assertive));
    let first = queue.take_next().expect("an announcement is waiting");
    assert_eq!(first.text, "Upload failed", "the assertive one is taken first");
    assert_eq!(first.politeness, Politeness::Assertive);
    assert_eq!(queue.take_next().map(|a| a.text), Some("Saved the note".to_string()));
    assert!(queue.is_empty());
}

#[test]
fn headless_accessibility_full_queue_drops_the_oldest_polite_announcement_first() {
    let mut queue = Announcements::with_capacity(2);
    queue.post("first polite", Politeness::Polite);
    queue.post("urgent one", Politeness::Assertive);
    queue.post("urgent two", Politeness::Assertive);
    assert_eq!(queue.len(), 2);
    assert_eq!(queue.take_next().map(|a| a.text), Some("urgent one".to_string()), "assertive announcements are never lost for polite ones");
    assert_eq!(queue.take_next().map(|a| a.text), Some("urgent two".to_string()));
    assert!(queue.take_next().is_none());
}

#[test]
fn headless_accessibility_map_addresses_are_unique_and_list_under_their_scope() {
    let mut map = Map::new();
    let first = map.add("app", "button", "New Note", "", true, None, "7");
    let second = map.add("app", "button", "New Note", "", true, None, "8");
    assert_ne!(first, second, "a duplicate name gets its own address");
    assert_eq!(map.len(), 2);
    let element = map.get(&first).expect("the first element is in the map");
    assert_eq!(element.name, "New Note");
    assert_eq!(element.handle, "7", "the producer's handle is kept for acting on it");
    let listing = map.listing("app");
    assert!(listing.contains("New Note") && listing.contains("enabled"));
    assert!(map.listing("other").is_empty(), "only elements under the scope are listed");
}

// ---- now-playing: content types and metadata (bifrost_ui::media_content) ----

use bifrost_ui::media_content::{optional_text, ContentError, MediaContent, MediaDuration, MediaType};

#[test]
fn headless_now_playing_duration_labels_read_as_a_clock_or_live() {
    assert_eq!(MediaDuration::finite_seconds(225.0).unwrap().label().as_deref(), Some("3:45"));
    assert_eq!(MediaDuration::finite_seconds(3725.0).unwrap().label().as_deref(), Some("1:02:05"));
    assert_eq!(MediaDuration::Live.label().as_deref(), Some("Live"));
    assert_eq!(MediaDuration::Continuous.label(), None);
    assert!(MediaDuration::finite_seconds(0.0).is_err());
    assert!(MediaDuration::finite_seconds(f64::NAN).is_err());
    assert!(MediaDuration::finite_seconds(-1.0).is_err());
    assert_eq!(MediaDuration::finite_seconds(f64::INFINITY), Err(ContentError::InvalidDuration));
}

#[test]
fn headless_now_playing_content_trims_text_and_needs_an_identifier_and_title() {
    let song = MediaContent::new(" track-1 ", " Song ", MediaType::Audio, MediaDuration::Finite(200.0)).unwrap();
    assert_eq!(song.id, "track-1");
    assert_eq!(song.title, "Song");
    assert!(!song.is_explicit);
    assert_eq!(MediaContent::new("  ", "Song", MediaType::Audio, MediaDuration::Live), Err(ContentError::EmptyId));
    assert_eq!(MediaContent::new("id", "  ", MediaType::Video, MediaDuration::Live), Err(ContentError::EmptyTitle));
    assert_eq!(optional_text("  x  "), Some("x".to_string()));
    assert_eq!(optional_text("   "), None);
}

#[test]
fn headless_now_playing_subtitle_joins_artist_and_album_and_skips_what_is_missing() {
    let mut song = MediaContent::new("t", "Song", MediaType::Audio, MediaDuration::Continuous).unwrap();
    assert_eq!(song.subtitle(), "");
    song.artist = Some("Artist".into());
    assert_eq!(song.subtitle(), "Artist");
    song.album = Some("Album".into());
    assert_eq!(song.subtitle(), "Artist - Album");
    song.artist = None;
    assert_eq!(song.subtitle(), "Album");
}

#[test]
fn headless_now_playing_progress_is_a_clamped_fraction_of_finite_items_only() {
    let song = MediaContent::new("t", "Song", MediaType::Audio, MediaDuration::Finite(200.0)).unwrap();
    assert_eq!(song.progress(50.0), Some(0.25));
    assert_eq!(song.progress(500.0), Some(1.0), "past the end is clamped to 1");
    assert_eq!(song.progress(-5.0), Some(0.0), "before the start is clamped to 0");
    let live = MediaContent::new("l", "Radio", MediaType::Audio, MediaDuration::Live).unwrap();
    assert_eq!(live.progress(10.0), None, "a live stream has no end to measure against");
    assert!(!live.duration.is_seekable());
}

// ---- contacts: vCard serialization (bifrost_ui::contact_vcard) ----

use bifrost_ui::contact_book::{Contact, LabeledValue};
use bifrost_ui::contact_vcard::{base64_decode, base64_encode, contacts as parse_cards, serialize, Version};

fn ada() -> Contact {
    Contact {
        given_name: "Ada".into(),
        family_name: "Lovelace".into(),
        phone_numbers: vec![LabeledValue::new(None, "+3545551234".into())],
        email_addresses: vec![LabeledValue::new(Some("work"), "ada@example.test".into())],
        ..Default::default()
    }
}

#[test]
fn headless_vcard_writes_a_card_in_the_requested_version() {
    let v3 = serialize(&[ada()], Version::V3);
    assert!(v3.starts_with("BEGIN:VCARD\r\nVERSION:3.0\r\n"), "3.0 cards open with the version line");
    assert!(v3.contains("FN:Ada Lovelace\r\n"), "the formatted name joins given and family names");
    assert!(v3.contains("END:VCARD"));
    let v4 = serialize(&[ada()], Version::V4);
    assert!(v4.contains("VERSION:4.0\r\n"));
}

#[test]
fn headless_vcard_read_back_keeps_the_name_and_phone() {
    let text = serialize(&[ada()], Version::V4);
    let back = parse_cards(&text).expect("the card parses");
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].given_name, "Ada");
    assert_eq!(back[0].family_name, "Lovelace");
    assert_eq!(back[0].phone_numbers[0].value, "+3545551234");
}

#[test]
fn headless_vcard_several_contacts_make_several_cards() {
    let mut other = ada();
    other.given_name = "Grace".into();
    other.family_name = "Hopper".into();
    let text = serialize(&[ada(), other], Version::V3);
    assert_eq!(text.matches("BEGIN:VCARD").count(), 2);
    let back = parse_cards(&text).unwrap();
    assert_eq!(back.iter().map(|c| c.given_name.as_str()).collect::<Vec<_>>(), ["Ada", "Grace"]);
}

#[test]
fn headless_vcard_photo_bytes_are_base64_both_ways() {
    assert_eq!(base64_encode(b"Man"), "TWFu");
    assert_eq!(base64_encode(b""), "");
    assert_eq!(base64_decode("TWFu"), Some(b"Man".to_vec()));
    assert_eq!(base64_decode("TWFu").map(|b| b.len()), Some(3));
}
