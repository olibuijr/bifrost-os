//! Parity tests for the shell side of the widget kit, the Today View and the Notification Centre widget store (task
//! sdk-tests-01b). They are headless host tests: the pure layout models and the widget store, with no display, device,
//! network or wall-clock time. Every test names the module it exercises by its full path.

use bifrost_ui::relevance::{DateKind, TimeClue};
use bifrost_ui::timeline::{Entry, ReloadPolicy};
use bifrost_ui::widget_config::{Parameter, Value};
use bifrost_ui::widget_link::Link;
use springboard::layout::{COLS, ROWS};
use springboard::shell::homeconf::{find_spot, fits, resize, Placed, Size};
use springboard::shell::services::widgets_timeline::{Hint, Store, Text};
use springboard::shell::today::{layout as today_layout, COL_PT, GAP_PT, GAP_X_PT, SEARCH_H_PT, SEARCH_TOP_PT};
use springboard::shell::widgets::{Kind, MEDIUM_PT, SMALL_PT};

// ---- widgetkit: widget families and sizes (springboard::shell::widgets, springboard::shell::homeconf) ----

#[test]
fn headless_widget_families_are_small_and_medium_only() {
    assert_eq!(SMALL_PT, 155.0);
    assert_eq!(MEDIUM_PT, 329.0);
    assert_eq!(Size::Small.cells(), (2, 2));
    assert_eq!(Size::Medium.cells(), (4, 2));
    assert_eq!(Size::from_key("medium"), Some(Size::Medium));
    assert_eq!(Size::from_key("large"), None, "there is no large family");
    assert_eq!(Size::Small.toggled(), Size::Medium);
    assert_eq!(Size::Medium.label(), "Medium");
}

#[test]
fn headless_widget_families_kinds_size_in_points_by_their_family() {
    assert!(Kind::Clock.medium() && Kind::Deildu.medium() && Kind::Terminal.medium());
    assert!(!Kind::Battery.medium());
    assert_eq!(Kind::Clock.size(1.0), (329, 155), "a medium tile spans the column");
    assert_eq!(Kind::Battery.size(2.0), (310, 310), "a small tile at two pixels a point");
    assert_eq!(Kind::from_key("stack"), Some(Kind::Stack));
    assert_eq!(Kind::from_key("nope"), None);
}

// ---- widgetkit: home-screen placement and gallery (springboard::shell::homeconf, widget store) ----

#[test]
fn headless_widget_placement_finds_the_first_free_spot_on_the_grid() {
    assert_eq!(COLS, 5);
    assert_eq!(ROWS, 6);
    let first = find_spot(&[], 0, Kind::Clock, Size::Small).expect("an empty page has room");
    assert_eq!((first.col, first.row, first.page), (0, 0, 0));

    let medium_at_origin = Placed { kind: Kind::Clock, size: Size::Medium, page: 0, col: 0, row: 0 };
    let next = find_spot(&[medium_at_origin], 0, Kind::Battery, Size::Small).expect("room below the medium tile");
    assert_eq!((next.col, next.row), (0, 2), "a medium tile covers columns 0 to 3 of rows 0 and 1");
}

#[test]
fn headless_widget_placement_refuses_spots_off_the_grid_or_over_another_widget() {
    let small_at_edge = Placed { kind: Kind::Battery, size: Size::Small, page: 0, col: COLS - 1, row: 0 };
    assert!(!fits(&[], &small_at_edge), "a small tile needs two columns");
    let on_next_page = Placed { page: 16, ..Placed { kind: Kind::Battery, size: Size::Small, page: 0, col: 0, row: 0 } };
    assert!(!fits(&[], &on_next_page), "pages stop at MAX_PAGES");
    let existing = Placed { kind: Kind::Wifi, size: Size::Small, page: 0, col: 0, row: 0 };
    let overlapping = Placed { kind: Kind::Battery, size: Size::Small, page: 0, col: 1, row: 1 };
    assert!(!fits(&[existing], &overlapping));
    assert!(existing.covers(1, 1) && !existing.covers(2, 0));
}

#[test]
fn headless_widget_placement_resize_switches_size_where_it_fits() {
    let mut widgets = vec![Placed { kind: Kind::Battery, size: Size::Small, page: 0, col: 0, row: 0 }];
    assert!(resize(&mut widgets, 0), "a lone small tile grows to medium");
    assert_eq!(widgets[0].size, Size::Medium);
    assert!(resize(&mut widgets, 0), "and shrinks back");
    assert_eq!(widgets[0].size, Size::Small);

    let mut crowded = vec![
        Placed { kind: Kind::Battery, size: Size::Small, page: 0, col: 0, row: 0 },
        Placed { kind: Kind::Wifi, size: Size::Small, page: 0, col: 2, row: 0 },
    ];
    assert!(!resize(&mut crowded, 0), "a medium tile would run into its neighbour");
    assert_eq!(crowded[0].size, Size::Small, "a refused resize changes nothing");
    assert!(!resize(&mut crowded, 9), "no widget at that index");
}

#[test]
fn headless_widget_placement_store_adds_removes_and_enlarges_a_widget() {
    let mut store = Store::default();
    assert_eq!(store.reload_ranked("clock-app", "clock", vec![entry(0.0, "Now", "12:00")], ReloadPolicy::Never, None), Ok(1));
    assert_eq!(store.rows(10).len(), 1, "a reloaded widget starts placed");

    assert!(store.set_placed("clock-app", "clock", false));
    assert!(store.rows(10).is_empty(), "a widget taken off the column is not in Notification Centre");
    assert!(store.set_placed("clock-app", "clock", true));

    assert!(store.set_large("clock-app", "clock", true));
    assert!(store.rows(10)[0].large, "Large makes the row taller");

    let listed = store.listing(10);
    assert_eq!(listed.len(), 1);
    assert!(listed[0].placed && listed[0].large && !listed[0].configurable);
    assert_eq!(listed[0].title, "Now", "the gallery names the entry on screen");

    assert!(!store.set_placed("clock-app", "unknown", true), "only reloaded widgets can be placed");
    assert!(store.remove("clock-app", "clock"));
    assert!(store.listing(10).is_empty());
}

// ---- widgetkit: Today View layout (springboard::shell::today) ----

#[test]
fn headless_today_view_flows_medium_and_small_tiles_into_the_column() {
    assert_eq!(COL_PT, MEDIUM_PT);
    assert_eq!(GAP_X_PT, COL_PT - 2.0 * SMALL_PT);
    let placed = today_layout(1.0, 400, &[(Kind::Clock, true), (Kind::Battery, true), (Kind::Wifi, true)]);
    assert_eq!(placed.len(), 3);
    let top = (SEARCH_TOP_PT + SEARCH_H_PT + GAP_PT).round() as i32;
    assert_eq!(placed[0].rect.w, 329);
    assert_eq!(placed[0].rect.h, 155);
    assert_eq!(placed[0].rect.x, 35, "the column is centred: (400 - 329) / 2");
    assert_eq!(placed[0].rect.y, top);
    assert_eq!(placed[1].rect.x, placed[0].rect.x, "a small tile starts the left side of the next row");
    assert_eq!(placed[2].rect.x, placed[0].rect.x + 329 - 155, "the second small tile sits to the right");
    assert_eq!(placed[1].rect.y, placed[2].rect.y);
    assert!(placed.iter().all(|p| p.on));
}

#[test]
fn headless_today_view_switched_off_widgets_follow_a_section_gap() {
    let placed = today_layout(1.0, 400, &[(Kind::Clock, true), (Kind::Deildu, false)]);
    assert_eq!(placed.len(), 2);
    assert!(!placed[1].on, "the off tile is marked off");
    // Clock: 84 + 155 + 16 = 255 for the next row, then the section gap of 16 before the off tile.
    assert_eq!(placed[1].rect.y, 271);
}

// ---- widgetkit: smart stacks and relevance in the store (springboard::shell::services::widgets_timeline) ----

#[test]
fn headless_smart_stack_store_orders_rows_by_relevance_then_given_order() {
    let mut store = Store::default();
    store.reload_ranked("news", "a", vec![entry(0.0, "A", "")], ReloadPolicy::Never, Some(Hint::Entry { score: 1.0, duration: 100.0 })).unwrap();
    store.reload_ranked("news", "b", vec![entry(0.0, "B", "")], ReloadPolicy::Never, Some(Hint::Entry { score: 5.0, duration: 100.0 })).unwrap();
    store.reload_ranked("news", "c", vec![entry(0.0, "C", "")], ReloadPolicy::Never, None).unwrap();
    let titles: Vec<String> = store.rows(10).into_iter().map(|r| r.title).collect();
    assert_eq!(titles, ["B", "A", "C"], "eligible scores first, highest first; the widget without a hint comes last");
}

#[test]
fn headless_smart_stack_clue_hint_keeps_a_widget_eligible_while_the_clue_is_active() {
    let mut store = Store::default();
    let clue = TimeClue::interval(0.0, 50.0, DateKind::Scheduled).unwrap();
    store.reload_ranked("trains", "next", vec![entry(0.0, "Next", "")], ReloadPolicy::Never, Some(Hint::Clue { score: 2.0, clue })).unwrap();
    store.reload_ranked("news", "top", vec![entry(0.0, "Top", "")], ReloadPolicy::Never, Some(Hint::Entry { score: 2.5, duration: 100.0 })).unwrap();
    let during: Vec<String> = store.rows(10).into_iter().map(|r| r.title).collect();
    assert_eq!(during, ["Next", "Top"], "the scheduled hint adds 1.0 to 2.0, which is 3.0 and outranks 2.5");
}

// ---- widgetkit: configurable widgets in the store (springboard::shell::services::widgets_timeline) ----

#[test]
fn headless_widget_config_store_defines_steps_and_refuses_options() {
    let mut store = Store::default();
    store.reload_ranked("weather", "forecast", vec![entry(0.0, "Sun", "")], ReloadPolicy::Never, None).unwrap();
    let params = vec![
        Parameter::Flag { key: "compact".into(), default: false },
        Parameter::Number { key: "days".into(), min: 1, max: 7, default: 3 },
        Parameter::Choice { key: "unit".into(), options: vec!["C".into(), "F".into()], default: 0 },
    ];
    assert_eq!(store.configure("weather", "forecast", params), Ok(3));
    assert!(store.listing(10)[0].configurable);

    assert!(store.step_option("weather", "forecast", "compact", 1));
    assert_eq!(store.configuration("weather", "forecast").unwrap().value("compact"), Some(Value::Flag(true)));
    assert!(store.step_option("weather", "forecast", "days", 10));
    assert_eq!(store.configuration("weather", "forecast").unwrap().value("days"), Some(Value::Number(7)), "numbers stop at the range end");
    assert!(store.step_option("weather", "forecast", "unit", -1));
    assert_eq!(store.configuration("weather", "forecast").unwrap().choice_label("unit"), Some("F"), "choices wrap around");

    assert!(!store.step_option("weather", "forecast", "missing", 1));
    assert!(store.set_option("weather", "forecast", "days", Value::Number(9)).is_err());
    assert!(store.set_option("weather", "other", "days", Value::Number(2)).is_err(), "no options for that widget");
}

// ---- notification center: widget search view in the store (springboard::shell::services::widgets_timeline) ----

#[test]
fn headless_widget_search_store_keeps_results_and_the_description() {
    let mut store = Store::default();
    store.reload_ranked("notes", "recent", vec![entry(0.0, "Notes", "")], ReloadPolicy::Never, None).unwrap();
    assert_eq!(store.set_search("notes", "recent", "Notes", "No notes match", vec!["Groceries".into(), "Plan".into()]), Ok(2));
    let view = store.search_view("notes", "recent").expect("the search view is set");
    assert_eq!(view.rows(), ["Groceries", "Plan"]);
    assert_eq!(view.placeholder(), None);
    assert_eq!(view.description(), Some("Notes"));
    assert_eq!(store.listing(10)[0].description.as_deref(), Some("Notes"));

    assert_eq!(store.set_search("notes", "recent", " ", "No notes match", vec![]), Ok(0));
    assert_eq!(store.search_view("notes", "recent").unwrap().placeholder(), Some("No notes match"), "no results draws the placeholder");
}

// ---- widgetkit: deep links on a widget in the store (springboard::shell::services::widgets_timeline) ----

#[test]
fn headless_widget_link_store_keeps_the_link_per_widget() {
    let mut store = Store::default();
    store.reload_ranked("trips", "today", vec![entry(0.0, "Trip", "")], ReloadPolicy::Never, None).unwrap();
    assert!(store.link("trips", "today").is_none());
    let link = Link::parse("bifrost-app://trips/today").unwrap();
    assert!(store.set_link("trips", "today", Some(link.clone())));
    assert_eq!(store.link("trips", "today"), Some(&link));
    assert!(store.set_link("trips", "today", None));
    assert!(store.link("trips", "today").is_none());
    assert!(!store.set_link("trips", "missing", None), "only reloaded widgets carry a link");
}

// ---- helpers ----

fn entry(date: f64, title: &str, text: &str) -> Entry<Text> {
    Entry { date, value: Text::new(title, text) }
}

// ---- now-playing: playback position and transport (springboard::shell::services::now_playing) ----

use bifrost_ui::media_session::RemoteCommand;
use springboard::shell::music::nowplaying::Target as Media;
use springboard::shell::services::now_playing::{extrapolate, scrub_target_ms, transport_for};

#[test]
fn headless_now_playing_position_moves_on_while_playing_and_holds_when_paused() {
    assert_eq!(extrapolate(1000, true, 250, Some(5000)), 1250, "the published position plus the time since it was set");
    assert_eq!(extrapolate(4900, true, 500, Some(5000)), 5000, "never past the length");
    assert_eq!(extrapolate(1000, false, 250, Some(5000)), 1000, "a paused session holds its position");
    assert_eq!(extrapolate(10, true, 5, None), 15, "without a length there is no cap");
}

#[test]
fn headless_now_playing_scrub_lands_on_the_fraction_of_the_length() {
    assert_eq!(scrub_target_ms(200_000, 0.5), 100_000.0);
    assert_eq!(scrub_target_ms(1000, 2.0), 1000.0, "the fraction is clamped to the bar");
    assert_eq!(scrub_target_ms(1000, -1.0), 0.0);
}

#[test]
fn headless_now_playing_transport_maps_play_pause_and_skips_to_remote_commands() {
    assert_eq!(transport_for(Media::Play, true), Some(RemoteCommand::Pause));
    assert_eq!(transport_for(Media::Play, false), Some(RemoteCommand::Play));
    assert_eq!(transport_for(Media::Prev, false), Some(RemoteCommand::Previous));
    assert_eq!(transport_for(Media::Next, true), Some(RemoteCommand::Next));
    assert_eq!(transport_for(Media::Volume, true), None, "volume is not a transport command");
    assert_eq!(transport_for(Media::Scrubber, true), None, "a scrub is a seek, not a command");
}

// ---- uniform type identifiers: open and share targets by type (springboard::shell::sharing) ----

use bifrost_proto::Payload;
use springboard::shell::sharing::{handles, Candidate};

#[test]
fn headless_open_targets_match_by_the_type_the_item_conforms_to() {
    let photo_handles = vec!["public.png:owner".to_string(), "public.jpeg".to_string()];
    let photos = Candidate { id: "photos", name: "Photos", handles: &photo_handles, schemes: &[] };
    let png = Payload::File { grant: [1; 16], name: "a.png".into() };
    assert!(handles(&photos, &png), "a PNG opens in an app that handles public.png");
    assert!(!handles(&photos, &Payload::Text("hi".into())), "text is not an image");
    assert!(handles(&photos, &Payload::Url("file:///home/a/photo.jpeg".into())), "a file URL is typed by its extension");
    assert!(!handles(&photos, &Payload::Url("mailto:someone@example.test".into())), "a non-file URL has no type");
}

#[test]
fn headless_open_targets_web_links_match_the_handled_scheme() {
    let browser_handles = vec!["public.html:owner".to_string()];
    let browser_schemes = vec!["https".to_string()];
    let browser = Candidate { id: "browser", name: "Browser", handles: &browser_handles, schemes: &browser_schemes };
    assert!(handles(&browser, &Payload::Url("https://example.com/x".into())));
    assert!(!handles(&browser, &Payload::Url("ftp://example.com/x".into())));
}
