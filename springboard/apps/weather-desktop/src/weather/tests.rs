use super::*;

#[test]
fn the_my_location_hint_follows_the_form_factor() {
    use bifrost_app::form_factor::FormFactor;
    assert_eq!(here_prompt(FormFactor::Tablet), "Tap to find this place");
    assert_eq!(here_prompt(FormFactor::Desktop), "Click to find this place");
}

fn test_ui() -> Ui {
    use bifrost_app::kit::Font;
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
    let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
    let mut ui = Ui::new(Font::parse(data).unwrap(), 1200.0 / 768.0);
    ui.symbols = springboard::symbols::Symbols::load(dir);
    ui
}

/// A network that is always down: the app must keep showing what it has.
struct Offline;

impl Fetch for Offline {
    fn get(&self, _url: &str, _limit: usize) -> std::io::Result<Vec<u8>> {
        Err(std::io::Error::new(std::io::ErrorKind::NotConnected, "offline"))
    }
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("weather-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn place(name: &str, lat: f64) -> Location {
    Location { name: name.into(), region: "Test".into(), lat, lon: -20.0 }
}

fn luma((r, g, b): (u8, u8, u8)) -> u32 {
    r as u32 + g as u32 + b as u32
}

#[test]
fn night_sky_is_darker_than_day_sky() {
    let day = backdrop(Sky::Clear, true);
    let night = backdrop(Sky::Clear, false);
    assert!(luma(night.0) < luma(day.0));
    assert!(luma(night.1) < luma(day.1));
}

#[test]
fn storm_and_rain_skies_ignore_the_time_of_day() {
    assert_eq!(backdrop(Sky::Thunder, true), backdrop(Sky::Thunder, false));
    assert_eq!(backdrop(Sky::Rain, true), backdrop(Sky::Rain, false));
}

#[test]
fn saved_places_settings_and_watchlist_load_on_start() {
    let dir = temp("load");
    let store = Store::new(dir.clone());
    store.save_locations(&[place("Akureyri", 65.68), place("Reykjavik", 64.14)]);
    store.save_settings(Units::Fahrenheit, 1);
    store.save_watchlist(&["aapl.us".to_string()]);

    let ui = test_ui();
    let app = Weather::new(&ui, 1200, 780, dir.clone(), Arc::new(Offline));
    let text = app.describe();
    assert!(text.contains("weather_units=F\n"), "{text}");
    assert!(text.contains("weather_places=2\n"), "{text}");
    assert!(text.contains("weather_place=Reykjavik\n"), "{text}");
    assert!(text.contains("weather_watch=1\n"), "{text}");
    assert!(text.contains("weather_loaded=0\n"), "{text}");
    assert!(app.summary().is_none(), "no cached forecast yet");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn going_offline_with_nothing_cached_shows_the_error_and_coming_back_clears_the_wait() {
    let dir = temp("offline");
    let ui = test_ui();
    let mut app = Weather::new(&ui, 1200, 780, dir.clone(), Arc::new(Offline));
    assert!(app.online, "the app starts online until the shell says otherwise");
    app.set_online(false);
    assert!(!app.online);
    assert!(app.describe().contains("weather_error=The weather could not be loaded.\n"), "{}", app.describe());
    app.set_online(true);
    assert!(app.online);
    app.set_online(true);
    assert!(app.online, "a repeated state changes nothing");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_fetch_is_shown_only_for_its_own_place() {
    let (mut app, _ui, dir) = app_with("errors-per-place", &["Oslo", "Reykjavik"]);
    let (oslo, reykjavik) = (app.places[0].key(), app.places[1].key());
    app.done.lock().unwrap().push(Done::Forecast { key: oslo.clone(), result: Err(WeatherError::Unreachable) });
    app.pump();
    assert!(app.describe().contains("weather_error=The weather service could not be reached.\n"), "{}", app.describe());
    app.selected = 1;
    assert!(app.describe().contains("weather_error=\n"), "Reykjavik did not fail: {}", app.describe());
    app.done.lock().unwrap().push(Done::Forecast { key: reykjavik, result: Err(WeatherError::BadAnswer) });
    app.pump();
    app.selected = 0;
    assert!(app.describe().contains("weather_error=The weather service could not be reached.\n"), "Oslo still failed: {}", app.describe());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_fresh_app_has_no_summary_change_to_report() {
    let dir = temp("summary-flag");
    let ui = test_ui();
    let mut app = Weather::new(&ui, 1200, 780, dir.clone(), Arc::new(Offline));
    assert!(!app.take_summary_changed(), "nothing saved yet");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn first_launch_starts_with_reykjavik_in_celsius() {
    let dir = temp("first");
    let ui = test_ui();
    let app = Weather::new(&ui, 1200, 780, dir.clone(), Arc::new(Offline));
    let text = app.describe();
    assert!(text.contains("weather_units=C\n"), "{text}");
    assert!(text.contains("weather_place=Reykjav"), "{text}");
    assert!(text.contains("weather_places=1\n"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn set_size_updates_the_window_and_marks_it_dirty() {
    let dir = temp("size");
    let ui = test_ui();
    let mut app = Weather::new(&ui, 1200, 780, dir.clone(), Arc::new(Offline));
    app.dirty = false;
    app.set_size(900, 700);
    assert_eq!(app.full_w, 900);
    assert_eq!(app.h, 700);
    assert!(app.dirty);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn places_end_with_the_location_note() {
    let dir = temp("location-note");
    let ui = test_ui();
    let app = Weather::new(&ui, 1200, 780, dir.clone(), Arc::new(Offline));
    match app.place_rows().last() {
        Some((Row::Head(text), _, _)) => assert!(text.starts_with("Location: not asked"), "{text}"),
        _ => panic!("the list should end with the location note"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// An app over `names` as saved places (the first selected), with the window wide enough for the sidebar.
pub(super) fn app_with(tag: &str, names: &[&str]) -> (Weather, Ui, PathBuf) {
    let dir = temp(tag);
    let places: Vec<Location> = names.iter().enumerate().map(|(i, n)| place(n, 60.0 + i as f64)).collect();
    Store::new(dir.clone()).save_locations(&places);
    let ui = test_ui();
    let mut app = Weather::new(&ui, 1200, 780, dir.clone(), Arc::new(Offline));
    app.relayout(&ui);
    (app, ui, dir)
}

#[test]
fn the_drawn_toolbar_sits_where_clicks_land_with_the_sidebar_shown() {
    let (app, ui, _) = app_with("bar-drawn", &["A"]);
    assert!(app.side > 0.0, "the 1200 px window shows the sidebar");
    let drawn = widgets::toolbar_layout_in(&ui, app.bar_bounds(&ui), &app.bar(), app.side / ui.pt(1.0));
    let hit = app.bar_layout(&ui);
    assert_eq!(drawn.visible.len(), hit.visible.len());
    for ((id, d), (hid, h)) in drawn.visible.iter().zip(hit.visible.iter()) {
        assert_eq!(id, hid);
        assert!((d.x - h.x).abs() < 0.01 && (d.w - h.w).abs() < 0.01, "item {id} drawn at {} but hit-tested at {}", d.x, h.x);
    }
}

fn names(app: &Weather) -> Vec<&str> {
    app.places.iter().map(|p| p.name.as_str()).collect()
}

/// A forecast whose minute block has these precipitation values for the steps from 13:15.
fn forecast_with_minutes(minutes: &str) -> Forecast {
    let text = format!(
        r#"{{"current":{{"time":"2026-10-04T13:20","temperature_2m":7.4,"weather_code":3,"is_day":1}},
        "hourly":{{"time":["2026-10-04T13:00"],"temperature_2m":[7.0],"weather_code":[3]}},
        "daily":{{"time":["2026-10-04"],"weather_code":[3],"temperature_2m_max":[8.0],"temperature_2m_min":[2.0]}},
        "minutely_15":{{"time":["2026-10-04T13:15","2026-10-04T13:30","2026-10-04T13:45","2026-10-04T14:00"],"precipitation":{minutes}}}}}"#
    );
    data::parse_forecast(&text).unwrap()
}

#[test]
fn dragging_a_sidebar_row_reorders_the_places_and_keeps_the_selection() {
    let (mut app, ui, dir) = app_with("drag", &["A", "B", "C"]);
    assert!(app.side > 0.0, "the window is wide enough for the sidebar");
    let spans = app.side_row_spans();
    let mid = |i: usize| ui.pt(spans[i].0 + spans[i].1 / 2.0);
    let x = ui.pt(40.0);
    app.touch(&ui, Event::Down(x, mid(0)), 0.0);
    app.touch(&ui, Event::Move(x, mid(1)), 0.1);
    assert!(app.side_drag.is_some_and(|d| d.active), "a move past the threshold becomes a drag");
    app.touch(&ui, Event::Up(x, mid(2)), 0.2);
    assert_eq!(names(&app), ["B", "C", "A"]);
    assert_eq!(app.selected, 2, "A stays selected where it was dropped");
    assert_eq!(app.screen, Screen::Main, "a drop is not a tap on the row");
    let store = Store::new(dir.clone());
    assert_eq!(store.load_locations().iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["B", "C", "A"], "the order is saved");
    assert_eq!(store.load_settings().1, 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_tap_on_a_sidebar_row_still_selects_it() {
    let (mut app, ui, dir) = app_with("tap", &["A", "B", "C"]);
    let spans = app.side_row_spans();
    let (x, y) = (ui.pt(40.0), ui.pt(spans[1].0 + spans[1].1 / 2.0));
    app.touch(&ui, Event::Down(x, y), 0.0);
    app.touch(&ui, Event::Up(x, y), 0.1);
    assert_eq!((app.selected, names(&app)), (1, vec!["A", "B", "C"]));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_delete_key_removes_the_selected_place_but_never_the_last() {
    let (mut app, ui, dir) = app_with("delete", &["A", "B"]);
    app.key(&ui, Out::Named("Delete"), 0.0);
    assert_eq!(names(&app), ["B"]);
    assert_eq!(Store::new(dir.clone()).load_locations().len(), 1, "the removal is saved");
    app.key(&ui, Out::Named("Delete"), 0.1);
    assert_eq!(names(&app), ["B"], "the last place stays");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn delete_does_not_remove_a_place_while_a_field_is_being_typed_in() {
    let (mut app, ui, dir) = app_with("delete-field", &["A", "B"]);
    app.activate(&ui, Target::Places, 0.0, 0.0);
    app.activate(&ui, Target::Field, 0.0, 0.0);
    assert!(app.field.focused);
    app.key(&ui, Out::Named("Delete"), 0.1);
    assert_eq!(names(&app), ["A", "B"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_context_menu_of_a_sidebar_row_offers_notifications_and_remove() {
    let (mut app, ui, dir) = app_with("context", &["A", "B"]);
    let spans = app.side_row_spans();
    let (x, y) = (ui.pt(40.0), ui.pt(spans[1].0 + spans[1].1 / 2.0));
    let items = app.context_items(&ui, x, y);
    assert_eq!(items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["Turn On Notifications", "Remove"]);
    assert!(items.iter().all(|i| i.enabled));
    assert!(app.context_action(&ui, CTX_NOTIFY, 0.0));
    assert!(app.notify_on.contains(&app.places[1].key()), "the menu switched notifications on for the row it opened on");
    let again = app.context_items(&ui, x, y);
    assert_eq!(again[0].label, "Turn Off Notifications");
    assert!(app.context_action(&ui, CTX_REMOVE, 0.0));
    assert_eq!(names(&app), ["A"]);
    assert!(app.notify_on.is_empty(), "a removed place keeps no notification switch");
    assert!(app.context_items(&ui, x, ui.pt(700.0)).is_empty(), "nothing is under the finger below the rows");
    assert!(!app.context_action(&ui, CTX_REMOVE, 0.0), "no menu is open");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_last_place_cannot_be_removed_from_its_context_menu() {
    let (mut app, ui, dir) = app_with("context-last", &["A"]);
    let spans = app.side_row_spans();
    let items = app.context_items(&ui, ui.pt(40.0), ui.pt(spans[0].0 + spans[0].1 / 2.0));
    assert_eq!(items[1].label, "Remove");
    assert!(!items[1].enabled);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_notifications_switch_is_saved_per_place() {
    let (mut app, ui, dir) = app_with("notify", &["A", "B"]);
    let key = app.places[1].key();
    app.activate(&ui, Target::Notify(1), 0.0, 0.0);
    assert!(app.notify_on.contains(&key));
    assert!(notify::load(&dir).contains(&key), "saved");
    let restarted = Weather::new(&ui, 1200, 780, dir.clone(), Arc::new(Offline));
    assert!(restarted.notify_on.contains(&key), "and loaded on the next start");
    app.activate(&ui, Target::Notify(1), 0.0, 0.0);
    assert!(notify::load(&dir).is_empty());
    app.activate(&ui, Target::Notify(9), 0.0, 0.0);
    assert!(app.notify_on.is_empty(), "an index past the list changes nothing");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rain_starting_within_the_hour_posts_one_notice_for_a_place_with_notifications_on() {
    let (mut app, ui, dir) = app_with("rain", &["A", "B"]);
    let (a, b) = (app.places[0].key(), app.places[1].key());
    app.activate(&ui, Target::Notify(0), 0.0, 0.0);
    let soon = forecast_with_minutes("[0.0,0.0,0.4,0.4]");
    assert_eq!(soon.minutes, vec![0.0, 0.0, 0.4, 0.4]);
    for key in [&a, &b] {
        app.done.lock().unwrap().push(Done::Forecast { key: key.clone(), result: Ok(soon.clone()) });
    }
    app.pump();
    let notices = app.take_notices();
    assert_eq!(notices.len(), 1, "only the place with the switch on: {notices:?}");
    assert_eq!(notices[0].title, "Rain soon in A");
    assert_eq!(notices[0].body, "Rain is expected to start in about 30 minutes.");
    app.done.lock().unwrap().push(Done::Forecast { key: a.clone(), result: Ok(soon) });
    app.pump();
    assert!(app.take_notices().is_empty(), "the next refresh does not repeat it");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_sidebar_shows_each_places_sky_and_temperature_once_its_forecast_is_known() {
    let (mut app, _ui, dir) = app_with("rows", &["A", "B"]);
    let row = |app: &Weather, i: usize| match &app.sidebar().items[i + app.here_offset()] {
        SidebarItem::Row { subtitle, trailing, .. } => (subtitle.clone(), trailing.clone()),
        other => panic!("a row, not {other:?}"),
    };
    assert_eq!(row(&app, 0), (Some("Test".to_string()), None), "no forecast yet: the region, no temperature");
    let key = app.places[0].key();
    app.forecasts.insert(key, forecast_with_minutes("[0.0,0.0,0.0,0.0]"));
    assert_eq!(row(&app, 0), (Some("Cloudy".to_string()), Some("7\u{b0}".to_string())));
    app.units = Units::Fahrenheit;
    assert_eq!(row(&app, 0).1.as_deref(), Some("45\u{b0}"), "the temperature follows the unit setting");
    assert_eq!(row(&app, 1).1, None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_weather_link_selects_a_saved_place_or_adds_one_by_position() {
    let (mut app, ui, dir) = app_with("link", &["Akureyri", "Reykjavik"]);
    assert!(app.open_link(&ui, "weather://reykjavik", 0.0), "names match without regard to case");
    assert_eq!(app.selected, 1);
    assert_eq!(names(&app), ["Akureyri", "Reykjavik"]);
    assert!(app.open_link(&ui, "weather://?lat=63.4&lon=-20.3&name=Vestmannaeyjar", 0.1));
    assert_eq!(names(&app), ["Akureyri", "Reykjavik", "Vestmannaeyjar"]);
    assert_eq!(app.selected, 2);
    assert_eq!(Store::new(dir.clone()).load_locations().len(), 3, "the new place is saved");
    assert!(app.open_link(&ui, "weather://?lat=63.4&lon=-20.3&name=Vestmannaeyjar", 0.2));
    assert_eq!(app.places.len(), 3, "the same position is not added twice");
    assert!(!app.open_link(&ui, "https://example.com", 0.3), "other links are not ours");
    assert!(!app.open_link(&ui, "weather://", 0.4));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_weather_link_to_an_unknown_name_is_looked_up_and_the_first_match_shown() {
    let (mut app, ui, dir) = app_with("link-search", &["Akureyri"]);
    assert!(app.open_link(&ui, "weather://Oslo", 0.0), "the lookup starts");
    assert_eq!(names(&app), ["Akureyri"], "nothing is added before the answer");
    let found = Location { name: "Oslo".into(), region: "Norway".into(), lat: 59.91, lon: 10.75 };
    app.done.lock().unwrap().push(Done::Linked { name: "Oslo".into(), result: Ok(vec![found, place("Oslo, Texas", 30.0)]) });
    app.tick(&ui, 1.0);
    assert_eq!(names(&app), ["Akureyri", "Oslo"]);
    assert_eq!(app.selected, 1);
    app.done.lock().unwrap().push(Done::Linked { name: "Nowhere".into(), result: Ok(vec![]) });
    app.tick(&ui, 2.0);
    assert_eq!(names(&app), ["Akureyri", "Oslo"], "no match, no change");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_full_list_ignores_a_link_to_a_new_place() {
    let names_list: Vec<String> = (0..data::MAX_LOCATIONS).map(|i| format!("P{i}")).collect();
    let refs: Vec<&str> = names_list.iter().map(String::as_str).collect();
    let (mut app, ui, dir) = app_with("link-full", &refs);
    assert!(app.open_link(&ui, "weather://?lat=1.5&lon=2.5&name=Extra", 0.0));
    assert_eq!(app.places.len(), data::MAX_LOCATIONS);
    assert_eq!(app.selected, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn control_methods_answer_for_the_selected_or_a_named_place() {
    let (mut app, _ui, dir) = app_with("rpc", &["A", "B"]);
    let none = Json::Obj(vec![]);
    let named = |n: &str| Json::Obj(vec![("place".to_string(), Json::Str(n.to_string()))]);
    let places = app.rpc("weather.places", &none).unwrap();
    assert_eq!(places, Json::Arr(vec![Json::Str("A".into()), Json::Str("B".into())]));
    assert_eq!(app.rpc("weather.current", &none).unwrap_err().code, INTERNAL_ERROR, "no forecast loaded yet");
    assert_eq!(app.rpc("weather.nope", &none).unwrap_err().code, METHOD_NOT_FOUND);
    let key = app.places[1].key();
    app.forecasts.insert(key, forecast_with_minutes("[0.0,0.0,0.0,0.0]"));
    let current = app.rpc("weather.current", &named("b")).unwrap();
    assert_eq!(current.str_field("place").as_deref(), Some("B"));
    assert_eq!(current.str_field("condition").as_deref(), Some("Cloudy"));
    assert_eq!(app.rpc("weather.current", &none).unwrap_err().code, INTERNAL_ERROR, "the selected place A has none");
    assert_eq!(app.rpc("weather.forecast", &named("Z")).unwrap_err().code, INVALID_PARAMS);
    app.selected = 1;
    let forecast = app.rpc("weather.forecast", &none).unwrap();
    assert_eq!(forecast.get("days").and_then(Json::as_array).map(|d| d.len()), Some(1));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_share_button_needs_a_forecast_and_sends_its_text_once() {
    let (mut app, ui, dir) = app_with("share", &["A"]);
    assert!(!app.shareable());
    app.activate(&ui, Target::Share, 0.0, 0.0);
    assert!(!app.take_share(), "nothing to share yet");
    assert_eq!(app.share_text(), None);
    let key = app.place().key();
    app.forecasts.insert(key, forecast_with_minutes("[0.0,0.0,0.0,0.0]"));
    assert!(app.shareable());
    app.activate(&ui, Target::Share, 0.0, 0.0);
    assert!(app.take_share());
    assert!(!app.take_share(), "the request is taken once");
    assert_eq!(app.share_text().as_deref(), Some("A: 7\u{b0} Cloudy, H:8\u{b0} L:2\u{b0}\nWeather data from Open-Meteo"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_toolbar_has_a_share_button_on_the_forecast_page_only() {
    let (mut app, ui, dir) = app_with("share-bar", &["A"]);
    let labels = |app: &Weather| app.bar().items.iter().map(|i| i.label.clone()).collect::<Vec<_>>();
    assert!(labels(&app).contains(&"Share".to_string()));
    assert!(!labels(&app).contains(&"Stocks".to_string()), "the forecast page has no Stocks button");
    app.activate(&ui, Target::Places, 0.0, 0.0);
    assert!(!labels(&app).contains(&"Share".to_string()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_locations_page_has_a_notifications_hit_zone_between_the_temperature_and_remove() {
    let (mut app, ui, dir) = app_with("zone", &["A", "B"]);
    app.activate(&ui, Target::Places, 0.0, 0.0);
    let y = ui.pt(places_list_top_pt() + ROW * (app.here_offset() as f32 + 0.5));
    let left = app.notify_left(&ui);
    assert_eq!(app.hit(&ui, left + ui.pt(10.0), y), Some(Target::Notify(0)));
    assert_eq!(app.hit(&ui, left - ui.pt(10.0), y), Some(Target::Place(0)));
    assert_eq!(app.hit(&ui, app.w as f32 - ui.pt(MARGIN + 10.0), y), Some(Target::Remove(0)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// E2E W4-11: the wheel over the detail pane scrolls the forecast, within its range; the sidebar leaves it alone.
#[test]
fn the_wheel_scrolls_the_forecast_page() {
    let (mut app, ui, dir) = app_with("wheel", &["A"]);
    app.fit_scroll(&ui);
    let (x, y) = (app.side + ui.pt(200.0), 700.0);
    assert!(app.wheel(&ui, x, y, 0.0, -120.0), "a wheel turned down scrolls the page");
    assert!(app.scroll.offset() > 0.0);
    assert!(app.wheel(&ui, x, y, 0.0, 10_000.0));
    assert_eq!(app.scroll.offset(), 0.0, "clamped at the top");
    assert!(!app.wheel(&ui, x, y, 0.0, 120.0), "nothing above the top");
    if app.side > 0.0 {
        assert!(!app.wheel(&ui, 2.0, y, 0.0, -120.0), "the sidebar does not scroll the forecast");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// E2E W4-12: Return in Locations searches, and while the answer is on its way the app asks to be woken for it.
#[test]
fn return_in_locations_searches_and_the_answer_shows_without_more_input() {
    let (mut app, ui, dir) = app_with("enter", &["A"]);
    app.activate(&ui, Target::Places, 0.0, 0.0);
    assert_eq!(app.screen, Screen::Places);
    app.field.set_text(&ui, "London");
    app.field.focused = false;
    app.key(&ui, Out::Enter, 10.0);
    assert!(app.searching, "Return starts the search");
    assert!(app.next_wake(20.0).is_some_and(|w| w <= 20.0 + FETCH_POLL_MS), "polls for the answer");
    // the offline answer lands on the utility queue and a plain tick applies it
    let mut landed = false;
    for i in 0..400 {
        std::thread::sleep(std::time::Duration::from_millis(5));
        app.tick(&ui, 100.0 + i as f64);
        if !app.searching {
            landed = true;
            break;
        }
    }
    assert!(landed, "the search finished");
    assert_eq!(app.note.as_deref(), Some("Search unavailable"));
    let _ = std::fs::remove_dir_all(&dir);
}
