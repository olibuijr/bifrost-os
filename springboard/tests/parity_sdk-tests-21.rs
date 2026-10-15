//! Parity sdk-tests-21: headless tests of the Weather data module through its public API. Places, geocoding, the
//! forecast request and its answer, the cache and the current conditions are checked with a fixed network and a
//! scratch store under the crate's target directory. No test opens a connection, a window or a clock.

use std::io;
use std::path::PathBuf;
use std::sync::Mutex;

use springboard::shell::music::radio::Fetch;
use springboard::shell::weather::data::{
    forecast_url, geocode_url, parse_forecast, parse_places, search_places, Current, Location, Sky, Store, SunEvent,
    AIR_API, FORECAST_API, GEOCODE_API,
};

const FORECAST: &str = r#"{"current":{"time":"2026-10-04T13:20","temperature_2m":7.4,"apparent_temperature":4.1,"relative_humidity_2m":81,"weather_code":61,"wind_speed_10m":22.5,"wind_direction_10m":315,"wind_gusts_10m":41.0,"dew_point_2m":4.2,"pressure_msl":1008.4,"is_day":1},
"hourly":{"time":["2026-10-04T12:00","2026-10-04T13:00","2026-10-04T18:00"],"temperature_2m":[6.0,7.0,8.0],"weather_code":[3,61,2],"is_day":[1,1,0],"precipitation_probability":[20,70,null],"precipitation":[0.1,1.4,0.0],"visibility":[24000,9000,15000],"pressure_msl":[1009.0,1008.4,1004.0]},
"daily":{"time":["2026-10-04","2026-10-05"],"weather_code":[61,0],"temperature_2m_max":[8.2,10.0],"temperature_2m_min":[3.0,2.5],"uv_index_max":[2.4,3.0],"sunrise":["2026-10-04T07:41","2026-10-05T07:44"],"sunset":["2026-10-04T18:20","2026-10-05T18:17"],"precipitation_sum":[6.5,0.0],"snowfall_sum":[0.0,2.5],"precipitation_probability_max":[85,null]}}"#;

const MINIMAL: &str = r#"{"current":{"time":"2026-10-04T13:00","temperature_2m":5.0,"weather_code":3},"hourly":{"time":["2026-10-04T13:00"],"temperature_2m":[5.0],"weather_code":[3]},"daily":{"time":["2026-10-04"],"weather_code":[3],"temperature_2m_max":[6.0],"temperature_2m_min":[1.0]}}"#;

const PLACES: &str = r#"{"results":[{"name":"Akureyri","admin1":"Northeast","country":"Iceland","latitude":65.68,"longitude":-18.1},{"name":"Akureyri","admin1":"Akureyri","country":"Iceland","latitude":65.7,"longitude":-18.0}],"generationtime_ms":0.4}"#;

const AIR_BODY: &str = r#"{"current":{"european_aqi":34,"pm2_5":7.8}}"#;

/// A network with fixed answers. The first route whose prefix matches a URL answers with its body; a route with no
/// body, or no matching route, fails. Every URL asked is recorded.
struct Network {
    routes: Vec<(&'static str, Option<&'static str>)>,
    asked: Mutex<Vec<String>>,
}

impl Network {
    fn new(routes: Vec<(&'static str, Option<&'static str>)>) -> Self {
        Network { routes, asked: Mutex::new(Vec::new()) }
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl Fetch for Network {
    fn get(&self, url: &str, _limit: usize) -> io::Result<Vec<u8>> {
        self.asked.lock().unwrap().push(url.to_string());
        match self.routes.iter().find(|(prefix, _)| url.starts_with(*prefix)) {
            Some((_, Some(body))) => Ok(body.as_bytes().to_vec()),
            _ => Err(io::Error::other(format!("no answer for {url}"))),
        }
    }
}

/// An answer with `hours` hourly entries from midnight of 2026-10-04 and `days` daily entries from that day.
fn forecast_with(hours: usize, days: usize) -> String {
    let times = (0..hours).map(|h| format!("\"2026-10-04T{:02}:00\"", h % 24)).collect::<Vec<_>>().join(",");
    let temps = (0..hours).map(|h| format!("{h}.0")).collect::<Vec<_>>().join(",");
    let codes = vec!["3"; hours].join(",");
    let dates = (0..days).map(|d| format!("\"2026-10-{:02}\"", 4 + d)).collect::<Vec<_>>().join(",");
    let ones = vec!["1"; days].join(",");
    let highs = (0..days).map(|d| format!("{d}.0")).collect::<Vec<_>>().join(",");
    format!(
        r#"{{"current":{{"time":"2026-10-04T13:20","temperature_2m":7.4,"weather_code":3}},"hourly":{{"time":[{times}],"temperature_2m":[{temps}],"weather_code":[{codes}]}},"daily":{{"time":[{dates}],"weather_code":[{ones}],"temperature_2m_max":[{highs}],"temperature_2m_min":[{highs}]}}}}"#
    )
}

fn reykjavik() -> Location {
    Location { name: "Reykjav\u{ed}k".into(), region: "Iceland".into(), lat: 64.1355, lon: -21.8954 }
}

fn oslo() -> Location {
    Location { name: "Oslo".into(), region: "Norway".into(), lat: 59.91, lon: 10.75 }
}

/// A store in a directory of its own under the crate's target directory, emptied before use.
fn scratch(name: &str) -> Store {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("parity-sdk-tests-21-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    Store::new(dir)
}

#[test]
fn a_place_keeps_its_name_region_and_coordinates() {
    let place = Location { name: "Akureyri".into(), region: "Northeast, Iceland".into(), lat: 65.6835, lon: -18.1262 };
    assert_eq!(place.name, "Akureyri");
    assert_eq!(place.subtitle(), "Northeast, Iceland");
    assert_eq!((place.lat, place.lon), (65.6835, -18.1262));
    assert_eq!(place.clone(), place);
}

#[test]
fn a_place_key_rounds_its_coordinates_to_two_decimals() {
    assert_eq!(oslo().key(), "59.91_10.75");
    let akureyri = Location { name: "Akureyri".into(), region: String::new(), lat: 65.6835, lon: -18.1262 };
    assert_eq!(akureyri.key(), "65.68_-18.13");
}

#[test]
fn two_names_at_the_same_coordinates_share_one_forecast_key() {
    let airport = Location { name: "Oslo Airport".into(), ..oslo() };
    assert_eq!(airport.key(), oslo().key());
    assert_ne!(airport, oslo(), "the names still tell the places apart");
}

#[test]
fn reykjavik_is_the_only_place_until_another_is_saved() {
    let store = scratch("default-place");
    assert_eq!(store.load_locations(), vec![reykjavik()]);
}

#[test]
fn saved_places_come_back_in_order_with_their_coordinates() {
    let store = scratch("saved-places");
    let places = vec![reykjavik(), oslo()];
    store.save_locations(&places);
    assert_eq!(store.load_locations(), places);
}

#[test]
fn a_tab_or_line_break_in_a_place_name_is_saved_as_a_space() {
    let store = scratch("tab-name");
    store.save_locations(&[Location { name: "Ak\tur\neyri".into(), ..oslo() }]);
    assert_eq!(store.load_locations()[0].name, "Ak ur eyri");
}

#[test]
fn the_place_search_asks_the_geocoder_for_six_english_matches() {
    let url = geocode_url("  Akureyri, Iceland ");
    assert!(url.starts_with(GEOCODE_API));
    assert_eq!(url, format!("{GEOCODE_API}?name=Akureyri%2C+Iceland&count=6&language=en&format=json"));
}

#[test]
fn a_non_ascii_place_name_is_percent_encoded_as_utf8() {
    assert!(geocode_url("Reykjav\u{ed}k").ends_with("name=Reykjav%C3%ADk&count=6&language=en&format=json"));
}

#[test]
fn a_place_answer_gives_each_place_its_region_from_admin_area_and_country() {
    let places = parse_places(PLACES).unwrap();
    assert_eq!(places.len(), 2);
    assert_eq!(places[0], Location { name: "Akureyri".into(), region: "Northeast, Iceland".into(), lat: 65.68, lon: -18.1 });
    assert_eq!(places[1].region, "Iceland", "an admin area that repeats the name is left out");
}

#[test]
fn a_result_without_coordinates_is_left_out_of_the_places() {
    let text = r#"{"results":[{"name":"Nowhere","country":"Iceland"}],"generationtime_ms":0.2}"#;
    assert_eq!(parse_places(text), Some(vec![]));
}

#[test]
fn an_answer_with_no_matches_is_an_empty_list_and_a_foreign_answer_is_not_one() {
    assert_eq!(parse_places(r#"{"generationtime_ms":0.3}"#), Some(vec![]));
    assert_eq!(parse_places("{}"), None);
    assert_eq!(parse_places("<html>"), None);
}

#[test]
fn a_local_place_search_returns_the_places_the_geocoder_sends() {
    let net = Network::new(vec![(GEOCODE_API, Some(PLACES))]);
    let places = search_places("  Akureyri ", &net).unwrap();
    assert_eq!(places.len(), 2);
    assert_eq!(net.asked(), vec![geocode_url("Akureyri")], "the query is trimmed before it is sent");
}

#[test]
fn a_search_that_cannot_reach_the_geocoder_is_an_error_not_an_empty_list() {
    let net = Network::new(vec![]);
    assert!(search_places("Oslo", &net).is_err());
}

#[test]
fn a_search_answer_that_is_not_a_place_list_is_an_error() {
    let net = Network::new(vec![(GEOCODE_API, Some("<html>"))]);
    let err = search_places("Oslo", &net).unwrap_err();
    assert_eq!(err.to_string(), "unexpected search answer");
}

#[test]
fn the_forecast_request_names_the_place_and_asks_for_every_field_the_screen_shows() {
    let url = forecast_url(&reykjavik());
    assert!(url.starts_with(&format!("{FORECAST_API}?latitude=64.1355&longitude=-21.8954&current=")), "{url}");
    assert!(url.ends_with("&timezone=auto"), "{url}");
    for field in ["wind_direction_10m", "wind_gusts_10m", "uv_index_max", "sunrise", "sunset"] {
        assert!(url.contains(field), "{field} in {url}");
    }
}

#[test]
fn a_fetched_forecast_is_cached_per_place_and_read_back_without_a_network() {
    let store = scratch("cache");
    let net = Network::new(vec![(FORECAST_API, Some(FORECAST)), (AIR_API, Some(AIR_BODY))]);
    let fetched = store.fetch_forecast(&reykjavik(), &net).unwrap();
    assert_eq!(fetched.current.temp, 7.4);
    assert_eq!(fetched.air.as_ref().map(|a| a.aqi), Some(34));
    let (cached, _age) = store.load_forecast(&reykjavik()).unwrap();
    assert_eq!(cached, fetched, "the cache returns what the fetch returned");
    assert!(store.load_forecast(&oslo()).is_none(), "another place has no cached forecast");
}

#[test]
fn a_forecast_still_arrives_when_the_air_quality_host_is_down() {
    let store = scratch("no-air");
    let net = Network::new(vec![(FORECAST_API, Some(FORECAST))]);
    let forecast = store.fetch_forecast(&reykjavik(), &net).unwrap();
    assert_eq!((forecast.current.code, forecast.air.is_none()), (61, true));
}

#[test]
fn an_answer_that_is_not_a_forecast_is_an_error_and_is_not_cached() {
    let store = scratch("bad-answer");
    let net = Network::new(vec![(FORECAST_API, Some("{}"))]);
    assert!(store.fetch_forecast(&reykjavik(), &net).is_err());
    assert!(store.load_forecast(&reykjavik()).is_none());
}

#[test]
fn a_forecast_request_that_fails_on_the_network_is_an_error() {
    let store = scratch("offline");
    assert!(store.fetch_forecast(&reykjavik(), &Network::new(vec![])).is_err());
}

#[test]
fn a_wmo_code_stands_for_its_sky_and_an_unlisted_code_is_cloud() {
    assert_eq!(Sky::from_code(0), Sky::Clear);
    assert_eq!(Sky::from_code(2), Sky::PartlyCloudy);
    assert_eq!(Sky::from_code(3), Sky::Cloudy);
    assert_eq!(Sky::from_code(48), Sky::Fog);
    assert_eq!(Sky::from_code(53), Sky::Drizzle);
    assert_eq!(Sky::from_code(65), Sky::Rain);
    assert_eq!(Sky::from_code(75), Sky::Snow);
    assert_eq!(Sky::from_code(82), Sky::Rain);
    assert_eq!(Sky::from_code(99), Sky::Thunder);
    assert_eq!(Sky::from_code(4), Sky::Cloudy);
}

#[test]
fn a_sky_has_a_readable_label() {
    assert_eq!((Sky::PartlyCloudy.label(), Sky::Thunder.label()), ("Partly Cloudy", "Thunderstorm"));
}

#[test]
fn the_current_conditions_come_from_the_current_block() {
    let f = parse_forecast(FORECAST).unwrap();
    let expected = Current {
        temp: 7.4,
        feels: 4.1,
        code: 61,
        day: true,
        humidity: 81,
        wind_kmh: 22.5,
        wind_dir: Some(315.0),
        wind_gust_kmh: Some(41.0),
        dew_point: Some(4.2),
        pressure_hpa: Some(1008.4),
    };
    assert_eq!(f.current, expected);
    assert_eq!(Sky::from_code(f.current.code), Sky::Rain);
}

#[test]
fn a_current_block_without_apparent_temperature_feels_like_the_air_temperature() {
    let f = parse_forecast(MINIMAL).unwrap();
    assert_eq!((f.current.temp, f.current.feels, f.current.code), (5.0, 5.0, 3));
}

#[test]
fn a_reading_marked_as_night_is_not_daytime() {
    let night = FORECAST.replace("\"is_day\":1}", "\"is_day\":0}");
    assert!(!parse_forecast(&night).unwrap().current.day);
}

#[test]
fn a_current_humidity_is_kept_within_zero_to_one_hundred_percent() {
    let high = FORECAST.replace("\"relative_humidity_2m\":81", "\"relative_humidity_2m\":150");
    assert_eq!(parse_forecast(&high).unwrap().current.humidity, 100);
    let low = FORECAST.replace("\"relative_humidity_2m\":81", "\"relative_humidity_2m\":-5");
    assert_eq!(parse_forecast(&low).unwrap().current.humidity, 0);
}

#[test]
fn a_current_block_without_a_temperature_is_not_a_forecast() {
    let text = MINIMAL.replace("\"temperature_2m\":5.0,", "");
    assert!(parse_forecast(&text).is_none());
}

#[test]
fn the_wind_direction_is_one_bearing_between_zero_and_three_hundred_sixty() {
    let east = FORECAST.replace("\"wind_direction_10m\":315", "\"wind_direction_10m\":365");
    assert_eq!(parse_forecast(&east).unwrap().current.wind_dir, Some(5.0));
    let west = FORECAST.replace("\"wind_direction_10m\":315", "\"wind_direction_10m\":-90");
    assert_eq!(parse_forecast(&west).unwrap().current.wind_dir, Some(270.0));
}

#[test]
fn a_missing_wind_reading_is_calm_and_has_no_direction_or_gust() {
    let text = FORECAST
        .replace("\"wind_speed_10m\":22.5,", "")
        .replace("\"wind_direction_10m\":315,", "")
        .replace("\"wind_gusts_10m\":41.0,", "");
    let current = parse_forecast(&text).unwrap().current;
    assert_eq!((current.wind_kmh, current.wind_dir, current.wind_gust_kmh), (0.0, None, None));
}

#[test]
fn the_uv_index_of_each_day_is_its_daily_peak() {
    let f = parse_forecast(FORECAST).unwrap();
    assert_eq!((f.days[0].uv, f.days[1].uv), (2.4, 3.0));
}

#[test]
fn a_day_without_a_uv_index_reads_as_zero() {
    let f = parse_forecast(MINIMAL).unwrap();
    assert_eq!(f.days[0].uv, 0.0);
}

#[test]
fn the_hours_start_at_the_current_hour_and_cover_the_next_day() {
    let f = parse_forecast(&forecast_with(40, 1)).unwrap();
    assert_eq!(f.hours.len(), 24);
    assert_eq!((f.hours[0].label.as_str(), f.hours[23].label.as_str()), ("13", "12"));
}

#[test]
fn the_days_cover_ten_days_and_skip_a_date_that_does_not_exist() {
    assert_eq!(parse_forecast(&forecast_with(0, 12)).unwrap().days.len(), 10);
    let text = FORECAST.replace("\"time\":[\"2026-10-04\",\"2026-10-05\"]", "\"time\":[\"2026-10-04\",\"2026-13-05\"]");
    assert_eq!(parse_forecast(&text).unwrap().days.len(), 1);
}

#[test]
fn an_hour_reports_its_temperature_code_and_day_flag() {
    let f = parse_forecast(FORECAST).unwrap();
    assert_eq!((f.hours[0].temp, f.hours[0].code, f.hours[0].day), (7.0, 61, true));
    assert_eq!((f.hours[1].temp, f.hours[1].code, f.hours[1].day), (8.0, 2, false));
}

#[test]
fn an_hour_carries_its_chance_and_amount_of_rain_its_visibility_and_pressure() {
    let f = parse_forecast(FORECAST).unwrap();
    let now = &f.hours[0];
    assert_eq!((now.precip_prob, now.precip_mm, now.visibility_m, now.pressure_hpa), (70, 1.4, Some(9000.0), Some(1008.4)));
    let masked = FORECAST.replace("[24000,9000,15000]", "[24000,-1,15000]");
    assert_eq!(parse_forecast(&masked).unwrap().hours[0].visibility_m, None, "a negative visibility is unknown");
}

#[test]
fn a_day_reports_its_rain_snow_and_chance_of_precipitation() {
    let f = parse_forecast(FORECAST).unwrap();
    assert_eq!((f.days[0].precip_sum, f.days[0].precip_prob, f.days[0].snow_sum), (6.5, 85, 0.0));
    assert_eq!((f.days[1].precip_prob, f.days[1].snow_sum), (0, 2.5), "a null chance reads as zero");
}

#[test]
fn each_day_has_its_sunrise_and_sunset_clock_times() {
    let f = parse_forecast(FORECAST).unwrap();
    assert_eq!((f.days[0].sunrise.as_str(), f.days[0].sunset.as_str()), ("07:41", "18:20"));
    assert_eq!((f.days[1].sunrise.as_str(), f.days[1].sunset.as_str()), ("07:44", "18:17"));
}

#[test]
fn a_day_without_sun_times_has_empty_sun_fields() {
    let f = parse_forecast(MINIMAL).unwrap();
    assert_eq!((f.days[0].sunrise.as_str(), f.days[0].sunset.as_str()), ("", ""));
}

#[test]
fn a_sunset_is_placed_in_the_hour_it_happens() {
    let f = parse_forecast(FORECAST).unwrap();
    assert_eq!(f.hours[0].sun, None);
    assert_eq!(f.hours[1].sun, Some(SunEvent { rise: false, time: "18:20".into() }));
}

#[test]
fn a_sunrise_and_a_sunset_in_one_hour_report_the_sunrise() {
    let text = FORECAST.replace("\"sunrise\":[\"2026-10-04T07:41\"", "\"sunrise\":[\"2026-10-04T18:05\"");
    let f = parse_forecast(&text).unwrap();
    assert_eq!(f.hours[1].sun, Some(SunEvent { rise: true, time: "18:05".into() }));
}
