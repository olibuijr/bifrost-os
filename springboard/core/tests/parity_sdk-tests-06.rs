//! Parity tests for the Weather data model (`bifrost_core::weather`) and the CSV reader
//! (`bifrost_core::csv`). Every check is pure: fixed JSON and CSV text in, concrete values out. No network,
//! no clock and no files.

use bifrost_core::csv::parse_rows;
use bifrost_core::weather::{
    display_symbol, forecast_url, geocode_url, normalise_symbol, parse_forecast, parse_places, parse_quotes, quote_url,
    Location, Quote, Sky, Summary, Units,
};

const FORECAST: &str = r#"{"current":{"time":"2026-10-04T13:15","temperature_2m":7.4,"apparent_temperature":4.1,"relative_humidity_2m":81,"weather_code":61,"wind_speed_10m":22.5,"is_day":1},
    "hourly":{"time":["2026-10-04T12:00","2026-10-04T13:00","2026-10-04T14:00"],"temperature_2m":[6.0,7.0,8.0],"weather_code":[3,61,2],"is_day":[1,1,0]},
    "daily":{"time":["2026-10-04","2026-10-05"],"weather_code":[61,0],"temperature_2m_max":[8.2,10.0],"temperature_2m_min":[3.0,2.5],"uv_index_max":[2.4,3.0],"sunrise":["2026-10-04T07:41","2026-10-05T07:44"],"sunset":["2026-10-04T18:20","2026-10-05T18:17"]}}"#;

const MINIMAL: &str = r#"{"current":{"time":"2026-10-04T13:15","temperature_2m":7.4,"weather_code":0},
    "hourly":{"time":["2026-10-04T13:00"],"temperature_2m":[7.0],"weather_code":[0]},
    "daily":{"time":["2026-10-04"],"weather_code":[0],"temperature_2m_max":[8.0],"temperature_2m_min":[3.0]}}"#;

#[test]
fn location_key_and_subtitle_describe_a_saved_place() {
    let place = Location { name: "Reykjav\u{ed}k".into(), region: "Iceland".into(), lat: 64.1, lon: -21.9 };
    assert_eq!(place.key(), "64.10_-21.90", "the cache key keeps two decimals of each coordinate");
    assert_eq!(place.subtitle(), "Iceland");
    let copy = place.clone();
    assert_eq!(copy, place, "a saved place compares by its fields");
}

#[test]
fn units_convert_round_format_and_parse_their_key() {
    assert_eq!(Units::default(), Units::Celsius);
    assert_eq!(Units::Celsius.convert(7.4), 7);
    assert_eq!(Units::Celsius.degrees(7.4), "7\u{b0}");
    assert_eq!(Units::Fahrenheit.convert(0.0), 32);
    assert_eq!(Units::Fahrenheit.convert(-40.0), -40, "the scales meet at -40");
    assert_eq!(Units::Fahrenheit.degrees(10.0), "50\u{b0}");
    assert_eq!((Units::Celsius.key(), Units::Fahrenheit.key()), ('C', 'F'));
    assert_eq!(Units::from_key("f"), Units::Fahrenheit);
    assert_eq!(Units::from_key(" F "), Units::Fahrenheit, "the key ignores spaces and case");
    assert_eq!(Units::from_key("x"), Units::Celsius, "anything but F is Celsius");
}

#[test]
fn wmo_codes_map_to_skies_and_labels() {
    assert_eq!(Sky::from_code(0), Sky::Clear);
    assert_eq!(Sky::from_code(1), Sky::Clear);
    assert_eq!(Sky::from_code(2), Sky::PartlyCloudy);
    assert_eq!(Sky::from_code(3), Sky::Cloudy);
    assert_eq!(Sky::from_code(48), Sky::Fog);
    assert_eq!(Sky::from_code(53), Sky::Drizzle);
    assert_eq!(Sky::from_code(63), Sky::Rain);
    assert_eq!(Sky::from_code(81), Sky::Rain);
    assert_eq!(Sky::from_code(73), Sky::Snow);
    assert_eq!(Sky::from_code(86), Sky::Snow);
    assert_eq!(Sky::from_code(96), Sky::Thunder);
    assert_eq!(Sky::from_code(250), Sky::Cloudy, "unknown codes are cloud");
    assert_eq!(Sky::Thunder.label(), "Thunderstorm");
    assert_eq!(Sky::PartlyCloudy.label(), "Partly Cloudy");
}

#[test]
fn forecast_reports_current_conditions_with_wind_and_humidity() {
    let f = parse_forecast(FORECAST).expect("a complete answer parses");
    assert_eq!((f.current.temp, f.current.feels), (7.4, 4.1), "apparent temperature is kept");
    assert_eq!((f.current.code, f.current.humidity), (61, 81));
    assert_eq!(f.current.wind_kmh, 22.5, "the wind is speed only, in km/h");
    assert!(f.current.day, "is_day 1 is daytime");
    assert_eq!(Sky::from_code(f.current.code), Sky::Rain);
}

#[test]
fn forecast_hours_start_at_the_current_hour() {
    let f = parse_forecast(FORECAST).expect("parses");
    assert_eq!(f.hours.iter().map(|h| h.label.as_str()).collect::<Vec<_>>(), ["13", "14"], "the 12:00 hour is before now");
    assert_eq!((f.hours[0].temp, f.hours[0].code), (7.0, 61));
    assert!(f.hours[0].day && !f.hours[1].day, "is_day flags carry over per hour");
}

#[test]
fn forecast_days_carry_range_uv_and_sun_times() {
    let f = parse_forecast(FORECAST).expect("parses");
    assert_eq!(f.days.len(), 2);
    assert_eq!(f.days[0].label, "Sun", "2026-10-04 is a Sunday");
    assert_eq!(f.days[1].label, "Mon", "2026-10-05 is a Monday");
    assert_eq!((f.days[1].max, f.days[1].min), (10.0, 2.5));
    assert_eq!(f.days[0].uv, 2.4, "the daily UV peak is kept");
    assert_eq!((f.days[0].sunrise.as_str(), f.days[1].sunset.as_str()), ("07:41", "18:17"), "sun times are local HH:MM");
}

#[test]
fn forecast_summary_uses_today_range_and_units() {
    let f = parse_forecast(FORECAST).expect("parses");
    let s = f.summary("Reykjav\u{ed}k", Units::Fahrenheit);
    assert_eq!(s.place, "Reykjav\u{ed}k");
    assert_eq!((s.temp, s.code, s.day, s.max, s.min), (7.4, 61, true, 8.2, 3.0));
    assert_eq!(s.units, Units::Fahrenheit);
}

#[test]
fn optional_forecast_fields_fall_back_to_defaults() {
    let f = parse_forecast(MINIMAL).expect("the required fields are enough");
    assert_eq!(f.current.feels, 7.4, "without apparent temperature the air temperature is used");
    assert_eq!((f.current.humidity, f.current.wind_kmh), (0, 0.0));
    assert!(f.current.day, "without is_day it is daytime");
    assert_eq!((f.days[0].uv, f.days[0].sunrise.as_str(), f.days[0].sunset.as_str()), (0.0, "", ""));
    assert!(f.hours[0].day);
}

#[test]
fn forecast_rejects_incomplete_or_invalid_answers() {
    assert!(parse_forecast("{}").is_none(), "no current block");
    assert!(parse_forecast("nope").is_none(), "not JSON");
    let no_daily = MINIMAL.replace("\"daily\"", "\"other\"");
    assert!(parse_forecast(&no_daily).is_none(), "a missing daily block is an error");
}

#[test]
fn forecast_drops_days_whose_date_is_not_real() {
    let bad_day = MINIMAL.replace("2026-10-04\"]", "2026-02-30\"]");
    let f = parse_forecast(&bad_day).expect("the hours still parse");
    assert!(f.days.is_empty(), "2026-02-30 does not exist, so that day is dropped");
    let leap = MINIMAL.replace("2026-10-04\"]", "2028-02-29\"]");
    assert_eq!(parse_forecast(&leap).expect("parses").days[0].label, "Tue", "2028-02-29 is a Tuesday");
}

#[test]
fn forecast_url_asks_for_every_field_the_parser_reads() {
    let url = forecast_url(&Location { name: "Oslo".into(), region: "Norway".into(), lat: 59.91, lon: 10.75 });
    assert!(url.starts_with("https://api.open-meteo.com/v1/forecast?latitude=59.9100&longitude=10.7500&"), "{url}");
    for field in ["apparent_temperature", "wind_speed_10m", "uv_index_max", "sunrise", "sunset", "timezone=auto"] {
        assert!(url.contains(field), "the URL asks for {field}");
    }
}

#[test]
fn geocode_url_encodes_the_typed_name() {
    assert_eq!(
        geocode_url("S\u{e3}o Paulo"),
        "https://geocoding-api.open-meteo.com/v1/search?name=S%C3%A3o+Paulo&count=6&language=en&format=json"
    );
    assert_eq!(geocode_url("  Oslo "), "https://geocoding-api.open-meteo.com/v1/search?name=Oslo&count=6&language=en&format=json");
    assert!(geocode_url("a&b=c").contains("name=a%26b%3Dc&"), "reserved characters are escaped");
}

#[test]
fn place_answers_become_locations_with_region_lines() {
    let text = r#"{"results":[{"name":"Akureyri","admin1":"Northeast","country":"Iceland","latitude":65.68,"longitude":-18.1},{"name":"Bad"}],"generationtime_ms":0.5}"#;
    let places = parse_places(text).expect("parses");
    assert_eq!(places.len(), 1, "a result without coordinates is skipped");
    assert_eq!(places[0], Location { name: "Akureyri".into(), region: "Northeast, Iceland".into(), lat: 65.68, lon: -18.1 });
}

#[test]
fn a_region_does_not_repeat_the_place_name() {
    let text = r#"{"results":[{"name":"Oslo","admin1":"Oslo","country":"Norway","latitude":59.91,"longitude":10.75}]}"#;
    let places = parse_places(text).expect("parses");
    assert_eq!(places[0].region, "Norway", "admin1 equal to the name is left out");
}

#[test]
fn an_empty_search_is_an_empty_list_and_junk_is_an_error() {
    assert_eq!(parse_places(r#"{"generationtime_ms":0.3}"#), Some(vec![]));
    assert!(parse_places("<html>").is_none());
}

#[test]
fn summary_line_round_trips_and_formats_for_the_lock_screen() {
    let s = parse_forecast(FORECAST).expect("parses").summary("Reykjav\u{ed}k", Units::Fahrenheit);
    assert_eq!(s.to_line(), "Reykjav\u{ed}k\t7.4\t61\t1\t8.2\t3\tF");
    assert_eq!(Summary::parse(&s.to_line()), Some(s.clone()));
    assert_eq!(s.line(), "Reykjav\u{ed}k 45\u{b0} Rain");
    assert_eq!(s.sky(), Sky::Rain);
}

#[test]
fn summary_parse_defaults_the_units_and_rejects_short_lines() {
    let s = Summary::parse("Oslo\t1\t2\t0\t3\t-1").expect("a line without units parses");
    assert_eq!(s.units, Units::Celsius);
    assert!(!s.day);
    assert_eq!((s.temp, s.min), (1.0, -1.0));
    assert!(Summary::parse("junk").is_none());
    assert!(Summary::parse("Oslo\t1\t2").is_none(), "a short line is not a summary");
}

#[test]
fn symbols_normalise_and_display_without_the_market_suffix() {
    assert_eq!(normalise_symbol(" AAPL "), Some("aapl.us".into()));
    assert_eq!(normalise_symbol("vod.uk"), Some("vod.uk".into()), "a suffix is kept");
    assert_eq!(normalise_symbol("^ftse"), Some("^ftse".into()), "an index keeps its caret");
    assert_eq!(normalise_symbol("a b"), None, "spaces are not allowed");
    assert_eq!(normalise_symbol(""), None);
    assert_eq!(normalise_symbol("abcdefghijklm"), None, "more than 12 characters is not a symbol");
    assert_eq!(display_symbol("aapl.us"), "AAPL");
    assert_eq!(display_symbol("vod.uk"), "VOD.UK");
}

#[test]
fn quote_url_joins_the_watchlist_symbols() {
    assert_eq!(quote_url(&["a.us".into(), "b.us".into()]), "https://stooq.com/q/l/?s=a.us+b.us&f=sd2t2ohlcv&h&e=csv");
}

#[test]
fn quote_rows_keep_priced_lines_and_skip_unknown_symbols() {
    let csv = "Symbol,Date,Time,Open,High,Low,Close,Volume\nAAPL.US,2026-10-02,22:00:09,100.0,111,99,110.0,5\nXYZ.US,N/D,N/D,N/D,N/D,N/D,N/D,N/D\n";
    let quotes = parse_quotes(csv);
    assert_eq!(quotes.len(), 1, "the N/D row has no prices");
    assert_eq!(quotes[0].symbol, "aapl.us");
    assert!((quotes[0].change_pct() - 10.0).abs() < 1e-9, "change is close against open");
}

#[test]
fn a_quote_with_no_open_price_has_no_change() {
    let q = Quote { symbol: "x.us".into(), open: 0.0, close: 5.0 };
    assert_eq!(q.change_pct(), 0.0);
}

#[test]
fn csv_splits_plain_records_with_lf_and_crlf() {
    assert_eq!(parse_rows("a,b,c\n1,2,3\n"), vec![vec!["a", "b", "c"], vec!["1", "2", "3"]]);
    assert_eq!(parse_rows("a,b\r\n1,2\r\n"), vec![vec!["a", "b"], vec!["1", "2"]], "CRLF ends a record too");
}

#[test]
fn csv_quoted_fields_keep_commas_and_doubled_quotes() {
    assert_eq!(
        parse_rows("name,note\n\"Doe, J\",\"said \"\"hi\"\"\"\n"),
        vec![vec!["name", "note"], vec!["Doe, J", "said \"hi\""]]
    );
}

#[test]
fn csv_quoted_fields_keep_line_breaks() {
    assert_eq!(parse_rows("x,\"line1\nline2\"\n"), vec![vec!["x", "line1\nline2"]]);
}

#[test]
fn csv_empty_fields_and_blank_lines_are_records() {
    assert_eq!(parse_rows("a,,c\n"), vec![vec!["a", "", "c"]], "an empty field is an empty cell");
    assert_eq!(parse_rows("a\n\nb"), vec![vec!["a"], vec![""], vec!["b"]], "a blank line in the middle is one empty field");
    assert_eq!(parse_rows("\"\""), vec![vec![""]], "a quoted empty field at the end still makes a record");
}

#[test]
fn csv_without_a_final_line_break_and_empty_input() {
    assert_eq!(parse_rows("a,b"), vec![vec!["a", "b"]]);
    assert!(parse_rows("").is_empty());
}

#[test]
fn csv_keeps_a_quote_inside_an_unquoted_field() {
    assert_eq!(parse_rows("a\"b,c"), vec![vec!["a\"b", "c"]]);
}
