use super::*;

fn loc(name: &str, lat: f64, lon: f64) -> Location {
    Location { name: name.into(), region: "Iceland".into(), lat, lon }
}

#[test]
fn wind_follows_the_temperature_setting_unit_system() {
    assert_eq!(wind_text(20.4, Units::Celsius), "20 km/h");
    assert_eq!(wind_text(100.0, Units::Fahrenheit), "62 mph");
}

#[test]
fn distance_is_metric_or_imperial_and_omitted_for_the_same_place() {
    let reykjavik = loc("Reykjavik", 64.1466, -21.9426);
    let paris = loc("Paris", 48.8566, 2.3522);
    let metric = away_text(&reykjavik, &paris, Units::Celsius).unwrap();
    assert!(metric.ends_with(" km away"), "{metric}");
    assert!(!metric.starts_with("0"), "{metric}");
    let imperial = away_text(&reykjavik, &paris, Units::Fahrenheit).unwrap();
    assert!(imperial.ends_with(" mi away"), "{imperial}");
    assert_eq!(away_text(&reykjavik, &reykjavik, Units::Celsius), None);
    assert_eq!(away_text(&reykjavik, &loc("Off", 95.0, 0.0), Units::Celsius), None);
}

#[test]
fn moon_tile_shows_new_and_full_moon_at_the_cycle_marks() {
    let reference_new_moon = 947_182_440.0;
    assert_eq!(moon_text(reference_new_moon), ("0%".to_string(), "New Moon".to_string()));
    let half = reference_new_moon + bifrost_app::controls::weather_model::SYNODIC_MONTH_DAYS / 2.0 * 86_400.0;
    assert_eq!(moon_text(half), ("100%".to_string(), "Full Moon".to_string()));
}

#[test]
fn my_location_is_named_and_its_coordinates_rounded_to_two_decimals() {
    let fix = Fix {
        coordinate: bifrost_app::core_location::Coordinate::new(64.146_57, -21.942_61),
        altitude: None,
        horizontal_accuracy: 100.0,
        timestamp: 0.0,
        source: "manual".into(),
        city: String::new(),
        country: String::new(),
    };
    let here = here_place(&fix);
    assert_eq!(here.name, "My Location");
    assert_eq!(here.region, "64.15, -21.94", "the label is the rounded position");
    assert_eq!((here.lat, here.lon), (64.15, -21.94), "the forecast is asked for the rounded position too");
}

#[test]
fn my_location_leads_the_sidebar_until_location_is_refused() {
    let (mut app, _ui, dir) = super::tests::app_with("here-row", &["A", "B"]);
    assert_eq!(app.here_offset(), 1, "listed while location is not asked, so the person can pick it");
    assert_eq!(app.sidebar().items.len(), 3);
    app.stop_here(LocationError::Denied);
    assert_eq!(app.here_offset(), 0, "gone once location is off for Weather");
    assert_eq!(app.sidebar().items.len(), 2);
    assert!(!app.here_shown());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn subtitle_gets_distance_only_for_other_places() {
    let here = loc("Here", 64.1466, -21.9426);
    let other = loc("There", 65.6835, -18.1262);
    assert_eq!(here.subtitle(), "Iceland");
    assert!(away_text(&here, &other, Units::Celsius).is_some());
    assert!(away_text(&here, &here, Units::Celsius).is_none());
}
