//! What Weather hands to other programs: the answers of its control methods (`weather.current` and
//! `weather.forecast`, read by scripts and the assistant) and the text the Share button sends. Nothing here reads
//! the screen or the network; it describes a forecast the app already holds, in the unit set of the temperature
//! setting, so a reader sees the numbers the window shows.

use crate::insights;
use bifrost_core::json::Json;
use springboard::shell::weather::data::{Forecast, Sky, Units};

/// The control methods the app serves: `weather.places` lists the saved place names; `weather.current` and
/// `weather.forecast` answer for the selected place, or for the saved place named by their `place` parameter.
pub const METHODS: [&str; 3] = ["weather.places", "weather.current", "weather.forecast"];

fn text(s: impl Into<String>) -> Json {
    Json::Str(s.into())
}

fn whole(v: f64) -> Json {
    Json::Num(v.round())
}

fn tenth(v: f64) -> Json {
    Json::Num((v * 10.0).round() / 10.0)
}

fn optional(v: Option<f64>, f: impl Fn(f64) -> Json) -> Json {
    v.map_or(Json::Null, f)
}

fn object(fields: Vec<(&str, Json)>) -> Json {
    Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn degrees(celsius: f64, units: Units) -> Json {
    Json::Num(units.convert(celsius) as f64)
}

/// The current conditions of `place`: temperatures in the setting's unit (`units` says which), the sky in words,
/// the wind in the unit set of the setting, and the rest in metric units as the service reports them.
pub fn current_json(place: &str, f: &Forecast, units: Units) -> Json {
    let c = &f.current;
    let today = f.days.first();
    object(vec![
        ("place", text(place)),
        ("units", text(units.key().to_string())),
        ("time", text(f.time.clone())),
        ("temperature", degrees(c.temp, units)),
        ("feels_like", degrees(c.feels, units)),
        ("condition", text(Sky::from_code(c.code).label())),
        ("high", today.map_or(Json::Null, |d| degrees(d.max, units))),
        ("low", today.map_or(Json::Null, |d| degrees(d.min, units))),
        ("humidity_percent", Json::Num(c.humidity as f64)),
        ("wind", text(insights::wind_text(c.wind_kmh, units))),
        ("wind_from", c.wind_dir.map_or(Json::Null, |d| text(insights::compass_point(d)))),
        ("wind_gust_kmh", optional(c.wind_gust_kmh, whole)),
        ("dew_point", optional(c.dew_point, |d| degrees(d, units))),
        ("pressure_hpa", optional(c.pressure_hpa, whole)),
        ("precipitation_today_mm", optional(today.map(|d| d.precip_sum), tenth)),
        ("air_quality", f.air.as_ref().map_or(Json::Null, |a| object(vec![("european_aqi", Json::Num(a.aqi as f64)), ("category", text(a.category()))]))),
    ])
}

/// The hours and days of the forecast of `place`, after the current conditions.
pub fn forecast_json(place: &str, f: &Forecast, units: Units) -> Json {
    let chance = |p: u8| Json::Num(p as f64);
    let hours = f
        .hours
        .iter()
        .map(|h| {
            object(vec![
                ("hour", text(h.label.clone())),
                ("temperature", degrees(h.temp, units)),
                ("condition", text(Sky::from_code(h.code).label())),
                ("precipitation_chance_percent", chance(h.precip_prob)),
            ])
        })
        .collect();
    let days = f
        .days
        .iter()
        .map(|d| {
            object(vec![
                ("day", text(d.label.clone())),
                ("high", degrees(d.max, units)),
                ("low", degrees(d.min, units)),
                ("condition", text(Sky::from_code(d.code).label())),
                ("precipitation_chance_percent", chance(d.precip_prob)),
                ("precipitation_mm", tenth(d.precip_sum)),
            ])
        })
        .collect();
    object(vec![("place", text(place)), ("units", text(units.key().to_string())), ("current", current_json(place, f, units)), ("hours", Json::Arr(hours)), ("days", Json::Arr(days))])
}

/// The text the Share button sends: "Reykjavik: 7° Rain, H:8° L:3°" and the data source. Short enough for a share item.
pub fn share_text(place: &str, f: &Forecast, units: Units) -> String {
    let sky = Sky::from_code(f.current.code).label();
    let range = f.days.first().map(|d| format!(", H:{} L:{}", units.degrees(d.max), units.degrees(d.min))).unwrap_or_default();
    format!("{place}: {} {sky}{range}\nWeather data from {}", units.degrees(f.current.temp), insights::DATA_SERVICE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use springboard::shell::weather::data::{parse_forecast, Air};

    const ANSWER: &str = r#"{"current":{"time":"2026-10-04T13:20","temperature_2m":7.4,"apparent_temperature":4.1,"relative_humidity_2m":81,"weather_code":61,"wind_speed_10m":22.5,"wind_direction_10m":315,"wind_gusts_10m":41.0,"dew_point_2m":4.2,"pressure_msl":1008.4,"is_day":1},
        "hourly":{"time":["2026-10-04T13:00","2026-10-04T14:00"],"temperature_2m":[7.0,8.0],"weather_code":[61,2],"precipitation_probability":[70,30]},
        "daily":{"time":["2026-10-04","2026-10-05"],"weather_code":[61,0],"temperature_2m_max":[8.2,10.0],"temperature_2m_min":[3.0,2.5],"precipitation_sum":[6.54,0.0],"precipitation_probability_max":[85,10]}}"#;

    fn forecast() -> Forecast {
        let mut f = parse_forecast(ANSWER).unwrap();
        f.air = Some(Air { aqi: 34, pm25: 7.8, ..Air::default() });
        f
    }

    #[test]
    fn current_reports_what_the_window_shows() {
        let j = current_json("Reykjav\u{ed}k", &forecast(), Units::Celsius);
        assert_eq!(j.str_field("place").as_deref(), Some("Reykjav\u{ed}k"));
        assert_eq!(j.str_field("condition").as_deref(), Some("Rain"));
        assert_eq!(j.str_field("wind").as_deref(), Some("23 km/h"));
        assert_eq!(j.str_field("wind_from").as_deref(), Some("NW"));
        assert_eq!(j.get("temperature").and_then(Json::number), Some(7.0));
        assert_eq!(j.get("high").and_then(Json::number), Some(8.0));
        assert_eq!(j.get("low").and_then(Json::number), Some(3.0));
        assert_eq!(j.get("humidity_percent").and_then(Json::number), Some(81.0));
        assert_eq!(j.get("pressure_hpa").and_then(Json::number), Some(1008.0));
        assert_eq!(j.get("precipitation_today_mm").and_then(Json::number), Some(6.5));
        assert_eq!(j.get("air_quality").and_then(|a| a.str_field("category")).as_deref(), Some("Fair"));
    }

    #[test]
    fn temperatures_follow_the_unit_setting() {
        let j = current_json("A", &forecast(), Units::Fahrenheit);
        assert_eq!(j.str_field("units").as_deref(), Some("F"));
        assert_eq!(j.get("temperature").and_then(Json::number), Some(45.0));
        assert_eq!(j.str_field("wind").as_deref(), Some("14 mph"));
    }

    #[test]
    fn missing_measurements_are_null_not_zero() {
        let bare = r#"{"current":{"time":"2026-10-04T13:15","temperature_2m":7.0,"weather_code":1},
            "hourly":{"time":["2026-10-04T13:00"],"temperature_2m":[7.0],"weather_code":[1]},
            "daily":{"time":["2026-10-04"],"weather_code":[1],"temperature_2m_max":[8.0],"temperature_2m_min":[2.0]}}"#;
        let j = current_json("A", &parse_forecast(bare).unwrap(), Units::Celsius);
        for key in ["wind_from", "wind_gust_kmh", "dew_point", "pressure_hpa", "air_quality"] {
            assert_eq!(j.get(key), Some(&Json::Null), "{key}");
        }
    }

    #[test]
    fn the_forecast_lists_hours_and_days_with_their_chances() {
        let j = forecast_json("A", &forecast(), Units::Celsius);
        let hours = j.get("hours").and_then(Json::as_array).unwrap();
        assert_eq!(hours.len(), 2);
        assert_eq!(hours[0].str_field("hour").as_deref(), Some("13"));
        assert_eq!(hours[0].get("precipitation_chance_percent").and_then(Json::number), Some(70.0));
        let days = j.get("days").and_then(Json::as_array).unwrap();
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].str_field("day").as_deref(), Some("Sun"));
        assert_eq!(days[0].get("high").and_then(Json::number), Some(8.0));
        assert_eq!(days[0].get("precipitation_chance_percent").and_then(Json::number), Some(85.0));
        assert_eq!(j.get("current").and_then(|c| c.str_field("place")).as_deref(), Some("A"));
    }

    #[test]
    fn the_reply_is_valid_json_text_within_the_request_limit() {
        let text = forecast_json("A", &forecast(), Units::Celsius).encode();
        assert!(bifrost_core::json::parse(&text).is_ok());
        assert!(text.len() < 64 * 1024);
    }

    #[test]
    fn the_share_text_has_the_place_the_sky_the_range_and_the_source() {
        assert_eq!(share_text("Akureyri", &forecast(), Units::Celsius), "Akureyri: 7\u{b0} Rain, H:8\u{b0} L:3\u{b0}\nWeather data from Open-Meteo");
        assert!(share_text("Akureyri", &forecast(), Units::Celsius).len() <= 512, "a share item holds at most 512 characters");
    }
}
