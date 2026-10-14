//! The pure parts of the forecast's temperature chart and map card, and the Locations note about location access.
//! Nothing here draws or talks to the network: the weather screen draws what these return, and the tests check the
//! numbers by hand.

use bifrost_app::controls::chart::axis_ticks;
use bifrost_app::controls::chart_marks::{Frame, Interpolation, Plot, line_path};
use bifrost_app::controls::geo::{Coordinate, DistanceUnits, format_distance};
use bifrost_app::controls::location_auth::{AuthorizationStatus, LocationError};
use bifrost_app::controls::mapview::{MapCamera, MapType, MapView};
use bifrost_app::controls::precipitation::{MinuteForecast, MinuteWeather, Precipitation};
use bifrost_app::controls::weather_alert::WeatherAvailability;
use bifrost_app::controls::weather_attribution::WeatherAttribution;
use bifrost_app::controls::weather_stats::Deviation;
use springboard::shell::weather::data::{Day, Hour, Quarter, Units};

/// The data service the forecast comes from, and the page that carries its terms.
pub const DATA_SERVICE: &str = "Open-Meteo";
pub const DATA_LEGAL_URL: &str = "https://open-meteo.com/en/terms";
/// The forecast request asks for no weather alerts and no minute forecast, so the place has neither from this source.
pub const DATA_HAS_ALERTS: bool = false;
pub const DATA_HAS_MINUTE: bool = false;

/// Zoom of the map card: at this level about two degrees of the world fit across a 400-point card.
pub const MAP_ZOOM: f64 = 8.0;
/// Longest scale bar the map card draws, in points.
pub const SCALE_MAX_PT: f64 = 120.0;

/// The temperature axis of the chart: the lowest and highest value (in Celsius) and the axis values between them.
#[derive(Clone, Debug, PartialEq)]
pub struct TempScale {
    pub lo: f64,
    pub hi: f64,
    pub ticks: Vec<f64>,
}

/// The chart's scale for the hours' temperatures. A flat forecast is widened by half a degree each way so the line
/// is not on the axis. None when no hour has a finite temperature.
pub fn temperature_scale(hours: &[Hour]) -> Option<TempScale> {
    let temps: Vec<f64> = hours.iter().map(|h| h.temp).filter(|t| t.is_finite()).collect();
    if temps.is_empty() {
        return None;
    }
    let lo = temps.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = temps.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let (lo, hi) = if hi - lo < 1.0 { (lo - 0.5, hi + 0.5) } else { (lo, hi) };
    Some(TempScale { lo, hi, ticks: axis_ticks(lo, hi, 3) })
}

/// The chart's data points: x is the hour's position in the list, y is its temperature in Celsius. Hours without a
/// finite temperature are left out.
pub fn temperature_points(hours: &[Hour]) -> Vec<(f64, f64)> {
    hours.iter().enumerate().filter(|(_, h)| h.temp.is_finite()).map(|(i, h)| (i as f64, h.temp)).collect()
}

/// The plot area inside the chart card. `frame` is in points from the card's top left, so its `x` leaves room for
/// the axis labels. The x domain runs from the first hour to the last.
pub fn temperature_plot(scale: &TempScale, hours: usize, frame: Frame) -> Plot {
    let last = hours.saturating_sub(1).max(1) as f64;
    Plot::new(frame, (0.0, last), (scale.lo, scale.hi))
}

/// The line through the hours, in the plot's points: linear, in hour order.
pub fn temperature_line(plot: &Plot, hours: &[Hour]) -> Vec<(f64, f64)> {
    line_path(plot, &temperature_points(hours), Interpolation::Linear)
}

/// The area under the temperature line as a set of columns: one per `step` points across the line, each with the y of
/// the line at that x (the top of the fill). Empty for fewer than two points or a step that is not positive.
pub fn area_columns(line: &[(f64, f64)], step: f64) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    if line.len() < 2 || !(step > 0.0) {
        return out;
    }
    let x1 = line[line.len() - 1].0;
    let mut x = line[0].0;
    let mut seg = 0;
    while x <= x1 + 1e-9 {
        while seg + 2 < line.len() && line[seg + 1].0 < x {
            seg += 1;
        }
        let (ax, ay) = line[seg];
        let (bx, by) = line[seg + 1];
        let t = if bx > ax { ((x - ax) / (bx - ax)).clamp(0.0, 1.0) } else { 0.0 };
        out.push((x, ay + (by - ay) * t));
        x += step;
    }
    out
}

/// A map view centred on a place, at [`MAP_ZOOM`], for a viewport of `w` by `h` points. None for coordinates that
/// are out of range or a viewport that is not positive.
pub fn map_view(lat: f64, lon: f64, w: f64, h: f64) -> Option<MapView> {
    let centre = Coordinate::new(lat, lon)?;
    Some(MapView::new(MapCamera::new(centre, MAP_ZOOM, w, h)?))
}

/// The latitude and longitude lines that cross the view, each with its position in the viewport's points.
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    /// (y in points, latitude) for each latitude line, top to bottom
    pub rows: Vec<(f64, f64)>,
    /// (x in points, longitude) for each longitude line, left to right
    pub cols: Vec<(f64, f64)>,
}

/// The grid lines of a view: about four of each kind, at round values, and only those inside the viewport.
pub fn grid(view: &MapView) -> Grid {
    let cam = view.camera();
    let (w, h) = (cam.viewport_w, cam.viewport_h);
    let top_left = cam.coordinate_at(0.0, 0.0);
    let bottom_right = cam.coordinate_at(w, h);
    let centre = cam.center;
    let mut rows = vec![];
    for lat in axis_ticks(bottom_right.latitude, top_left.latitude, 4) {
        if let Some(c) = Coordinate::new(lat, centre.longitude) {
            let (_, y) = cam.screen_point(c);
            if (0.0..=h).contains(&y) {
                rows.push((y, lat));
            }
        }
    }
    // a higher latitude sits higher up, so the lines come out in reverse order: sort them top to bottom
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut cols = vec![];
    if bottom_right.longitude > top_left.longitude {
        for lon in axis_ticks(top_left.longitude, bottom_right.longitude, 4) {
            if let Some(c) = Coordinate::new(centre.latitude, lon) {
                let (x, _) = cam.screen_point(c);
                if (0.0..=w).contains(&x) {
                    cols.push((x, lon));
                }
            }
        }
    }
    Grid { rows, cols }
}

/// A coordinate as a grid label: "64.5°N" or "21.5°W".
pub fn coordinate_label(value: f64, positive: char, negative: char) -> String {
    let side = if value < 0.0 { negative } else { positive };
    format!("{:.1}\u{b0}{}", value.abs(), side)
}

/// The scale bar's length in the unit system of the temperature setting, such as "2.0 km" or "500 m".
pub fn scale_text(length_m: f64, units: Units) -> String {
    let system = match units {
        Units::Celsius => DistanceUnits::Metric,
        Units::Fahrenheit => DistanceUnits::Imperial,
    };
    format_distance(length_m, system)
}

/// The note under the map: which map type it is and that no tiles are loaded, so the lines are the only map.
pub fn map_note(map_type: MapType) -> String {
    let name = match map_type {
        MapType::Standard => "Standard",
        MapType::Hybrid => "Hybrid",
        MapType::Imagery => "Imagery",
    };
    format!("{name} map with no tiles loaded. The lines are latitude and longitude.")
}

/// The Locations note about location access: the status the app holds, or the plain-language error when a location
/// request failed. Weather reads saved places, so a status other than "on" says that the places are what it uses.
pub fn location_note(status: AuthorizationStatus, error: Option<LocationError>) -> String {
    if let Some(e) = error {
        return match e {
            LocationError::Denied => "Location is turned off for Weather. Weather uses the places you add.".into(),
            LocationError::PromptDeclined => "Location was not allowed. Weather uses the places you add.".into(),
            LocationError::LocationUnknown => "This PC could not find its position. Try again later.".into(),
            LocationError::Network => "Finding the position needs a network connection. Check the connection and try again.".into(),
            _ if e.needs_user_action() => "Location needs your permission before Weather can use it.".into(),
            _ if e.is_transient() => "Location is not available right now. Try again later.".into(),
            _ => "The location service could not finish the request.".into(),
        };
    }
    match status {
        AuthorizationStatus::NotDetermined => "Location: not asked. Weather uses the places you add, not this PC's position.".into(),
        AuthorizationStatus::Restricted => "Location: not allowed for Weather on this PC. Weather uses the places you add.".into(),
        AuthorizationStatus::Denied => "Location: turned off for Weather. Weather uses the places you add.".into(),
        AuthorizationStatus::AuthorizedAlways | AuthorizationStatus::AuthorizedWhenInUse => {
            "Location: on for Weather. Weather uses the places you add.".into()
        }
    }
}

/// The lines under the forecast cards: the alert availability of the place when its source serves no alerts, then the
/// data attribution. Each line is plain text; the attribution is the model's own line.
pub fn footer_lines() -> Vec<String> {
    let mut lines = Vec::new();
    let availability = WeatherAvailability::new(DATA_HAS_ALERTS, DATA_HAS_MINUTE);
    if !availability.has_alerts() {
        lines.push("Weather alerts are not available for this place from the data service.".to_string());
    }
    if let Some(attribution) = WeatherAttribution::new(DATA_SERVICE, Some(DATA_LEGAL_URL), "") {
        lines.push(attribution.line());
    }
    lines
}

/// One column of the hourly strip: a forecast hour (by its index) or a sunrise or sunset marker between two hours.
#[derive(Clone, Debug, PartialEq)]
pub enum HourlyColumn {
    Hour(usize),
    Sun { rise: bool, time: String },
}

/// The hourly strip's columns: each hour, with the sunrise and sunset that fall between it and the next hour (the
/// times compare as text, since the answer's local times share one format). Events after the last hour are left out,
/// because the strip does not reach them.
pub fn hourly_columns(hours: &[Hour], days: &[Day]) -> Vec<HourlyColumn> {
    let mut events: Vec<(String, bool)> = Vec::new();
    for d in days.iter().take(3) {
        for (clock, rise) in [(&d.sunrise, true), (&d.sunset, false)] {
            if !clock.is_empty() {
                events.push((format!("{}T{}", d.date, clock), rise));
            }
        }
    }
    events.sort();
    let mut out = Vec::new();
    for (i, h) in hours.iter().enumerate() {
        out.push(HourlyColumn::Hour(i));
        let Some(next) = hours.get(i + 1) else { continue };
        for (at, rise) in events.iter().filter(|(at, _)| at.as_str() >= h.time.as_str() && at.as_str() < next.time.as_str()) {
            out.push(HourlyColumn::Sun { rise: *rise, time: at.get(11..16).unwrap_or(at.as_str()).to_string() });
        }
    }
    out
}

/// The label of a chance of precipitation, rounded to five percent, or None below ten percent (no label is drawn).
pub fn chance_label(percent: u8) -> Option<String> {
    (percent >= 10).then(|| format!("{}%", (u16::from(percent) + 2) / 5 * 5))
}

/// The next hour as the card shows it: one sentence and the four quarter-hour amounts in millimetres.
#[derive(Clone, Debug, PartialEq)]
pub struct NextHour {
    pub summary: String,
    pub bars: [f64; 4],
}

/// The next hour from the quarter-hour forecast. `now` is the answer's current time ("2026-10-14T13:15"); the four
/// quarters from the one that covers it (the last one that starts at or before it) are used. Snowy skies report
/// snow, anything else rain. None when the forecast has fewer than four quarters from there on.
pub fn next_hour(quarters: &[Quarter], now: &str, snow: bool) -> Option<NextHour> {
    let start = quarters.iter().rposition(|q| q.time.as_str() <= now).unwrap_or(0);
    let window: Vec<&Quarter> = quarters.iter().skip(start).take(4).collect();
    if window.len() < 4 {
        return None;
    }
    let form = if snow { Precipitation::Snow } else { Precipitation::Rain };
    let minutes = window
        .iter()
        .enumerate()
        .map(|(k, q)| {
            let wet = q.mm > 0.0;
            let kind = if wet { form } else { Precipitation::None };
            MinuteWeather::new(k as i64 * 900, kind, if wet { 1.0 } else { 0.0 }, q.mm * 4.0)
        })
        .collect();
    let forecast = MinuteForecast::new(minutes);
    let bars = [window[0].mm, window[1].mm, window[2].mm, window[3].mm];
    Some(NextHour { summary: forecast.summary(0), bars })
}

/// The pressure tile: the reading in hPa or inHg (the temperature setting's pair) and the trend over three hours,
/// "Rising", "Falling" or "Steady" (a change of a whole hPa or more counts as a trend).
pub fn pressure_text(hpa: f64, trend_hpa: Option<f64>, units: Units) -> (String, String) {
    let value = match units {
        Units::Celsius => format!("{:.0} hPa", hpa),
        Units::Fahrenheit => format!("{:.2} inHg", hpa * 0.029_529_983),
    };
    let note = match trend_hpa {
        Some(d) if d >= 1.0 => "Rising",
        Some(d) if d <= -1.0 => "Falling",
        Some(_) => "Steady",
        None => "Trend not available",
    };
    (value, note.to_string())
}

/// The precipitation tile: the day's total in mm or inches, and tomorrow's total as the note.
pub fn precipitation_text(today_mm: f64, tomorrow_mm: Option<f64>, units: Units) -> (String, String) {
    let amount = |mm: f64| match units {
        Units::Celsius => format!("{mm:.1} mm"),
        Units::Fahrenheit => format!("{:.2} in", mm / 25.4),
    };
    let note = match tomorrow_mm {
        Some(mm) if mm > 0.0 => format!("Tomorrow: {}", amount(mm)),
        Some(_) => "No rain tomorrow".to_string(),
        None => "Tomorrow not available".to_string(),
    };
    (amount(today_mm), note)
}

/// The visibility tile: the distance in km or mi (the temperature setting's pair) and a word for it.
pub fn visibility_text(metres: Option<f64>, units: Units) -> (String, String) {
    let Some(m) = metres.filter(|m| m.is_finite() && *m >= 0.0) else {
        return ("\u{2013}".to_string(), "Not available".to_string());
    };
    let system = match units {
        Units::Celsius => DistanceUnits::Metric,
        Units::Fahrenheit => DistanceUnits::Imperial,
    };
    let note = match m {
        m if m < 1_000.0 => "Fog",
        m if m < 4_000.0 => "Poor visibility",
        m if m < 10_000.0 => "Moderate visibility",
        _ => "Clear view",
    };
    (format_distance(m, system), note.to_string())
}

/// The averages tile: the average high for the date in the setting's unit, and how today's high compares with the years
/// around it (`compare`: above or below their interquartile range, or typical). None when there are no years to compare.
pub fn average_text(average_c: f64, units: Units, deviation: Option<Deviation>) -> (String, String) {
    let note = match deviation {
        Some(Deviation::Above) => "Warmer than usual for the date",
        Some(Deviation::Below) => "Cooler than usual for the date",
        Some(Deviation::Normal) => "Typical for the date",
        None => "Not available",
    };
    (units.degrees(average_c), note.to_string())
}

/// The point a compass needle reaches at `radius` from its centre, for a bearing in degrees clockwise from north
/// (screen y grows downwards, so north is up).
pub fn bearing_offset(degrees: f64, radius: f64) -> (f64, f64) {
    let r = degrees.to_radians();
    (radius * r.sin(), -radius * r.cos())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hour(temp: f64) -> Hour {
        Hour { label: "13".into(), time: "2026-10-14T13:00".into(), temp, code: 0, day: true, chance: 0, precip_mm: 0.0, visibility_m: None }
    }

    fn day(date: &str, rise: &str, set: &str) -> Day {
        Day { label: "Wed".into(), date: date.into(), code: 0, max: 5.0, min: 1.0, uv: 0.0, sunrise: rise.into(), sunset: set.into(), chance: 0, precip_mm: 0.0 }
    }

    fn at(time: &str, temp: f64) -> Hour {
        Hour { time: time.into(), ..hour(temp) }
    }

    #[test]
    fn sun_markers_sit_between_the_hours_they_fall_in() {
        // hours 05:00, 06:00, 07:00; sunrise 06:12 falls between 06:00 and 07:00
        let hours = [at("2026-10-14T05:00", 1.0), at("2026-10-14T06:00", 1.0), at("2026-10-14T07:00", 1.0)];
        let cols = hourly_columns(&hours, &[day("2026-10-14", "06:12", "18:30")]);
        assert_eq!(
            cols,
            vec![
                HourlyColumn::Hour(0),
                HourlyColumn::Hour(1),
                HourlyColumn::Sun { rise: true, time: "06:12".into() },
                HourlyColumn::Hour(2),
            ]
        );
    }

    #[test]
    fn a_sunset_after_the_last_hour_is_left_out() {
        let hours = [at("2026-10-14T17:00", 1.0), at("2026-10-14T18:00", 1.0)];
        let cols = hourly_columns(&hours, &[day("2026-10-14", "06:12", "18:30")]);
        assert_eq!(cols, vec![HourlyColumn::Hour(0), HourlyColumn::Hour(1)]);
        assert!(hourly_columns(&[], &[day("2026-10-14", "06:12", "18:30")]).is_empty());
    }

    #[test]
    fn chances_under_ten_percent_are_not_labelled_and_others_round_to_five() {
        assert_eq!(chance_label(9), None);
        assert_eq!(chance_label(10).as_deref(), Some("10%"));
        assert_eq!(chance_label(42).as_deref(), Some("40%"));
        assert_eq!(chance_label(43).as_deref(), Some("45%"));
        assert_eq!(chance_label(100).as_deref(), Some("100%"));
    }

    fn quarter(time: &str, mm: f64) -> Quarter {
        Quarter { time: time.into(), mm }
    }

    #[test]
    fn next_hour_takes_four_quarters_from_now_and_describes_the_change() {
        let q = [
            quarter("2026-10-14T13:00", 0.0),
            quarter("2026-10-14T13:15", 0.0),
            quarter("2026-10-14T13:30", 0.4),
            quarter("2026-10-14T13:45", 0.6),
            quarter("2026-10-14T14:00", 0.0),
        ];
        let next = next_hour(&q, "2026-10-14T13:15", false).unwrap();
        assert_eq!(next.bars, [0.0, 0.4, 0.6, 0.0]);
        assert_eq!(next.summary, "Rain starts in 15 min");
    }

    #[test]
    fn a_dry_hour_and_a_short_forecast_read_plainly() {
        let dry = [quarter("2026-10-14T13:00", 0.0), quarter("2026-10-14T13:15", 0.0), quarter("2026-10-14T13:30", 0.0), quarter("2026-10-14T13:45", 0.0)];
        assert_eq!(next_hour(&dry, "2026-10-14T13:00", false).unwrap().summary, "No precipitation in the next hour");
        assert!(next_hour(&dry[..2], "2026-10-14T13:00", false).is_none());
        assert!(next_hour(&[], "2026-10-14T13:00", false).is_none());
    }

    #[test]
    fn pressure_and_precipitation_read_in_the_setting_units() {
        assert_eq!(pressure_text(1013.4, Some(2.0), Units::Celsius), ("1013 hPa".into(), "Rising".into()));
        assert_eq!(pressure_text(1013.0, Some(-1.5), Units::Celsius).1, "Falling");
        assert_eq!(pressure_text(1013.0, Some(0.4), Units::Celsius).1, "Steady");
        assert_eq!(pressure_text(1013.0, None, Units::Celsius).1, "Trend not available");
        assert_eq!(pressure_text(1013.0, None, Units::Fahrenheit).0, "29.92 inHg");
        assert_eq!(precipitation_text(2.4, Some(0.0), Units::Celsius), ("2.4 mm".into(), "No rain tomorrow".into()));
        assert_eq!(precipitation_text(0.0, Some(5.0), Units::Celsius).1, "Tomorrow: 5.0 mm");
        assert_eq!(precipitation_text(25.4, None, Units::Fahrenheit).0, "1.00 in");
    }

    #[test]
    fn visibility_follows_the_setting_and_names_fog() {
        assert_eq!(visibility_text(Some(800.0), Units::Celsius), ("800 m".into(), "Fog".into()));
        let (far, note) = visibility_text(Some(24_000.0), Units::Celsius);
        assert!(far.ends_with(" km") && far.starts_with("24"), "{far}");
        assert_eq!(note, "Clear view");
        assert!(visibility_text(Some(16_000.0), Units::Fahrenheit).0.ends_with(" mi"));
        assert_eq!(visibility_text(None, Units::Celsius).1, "Not available");
    }

    #[test]
    fn the_average_tile_names_the_comparison_for_the_date() {
        assert_eq!(average_text(7.4, Units::Celsius, Some(Deviation::Above)), ("7\u{b0}".to_string(), "Warmer than usual for the date".to_string()));
        assert_eq!(average_text(7.4, Units::Celsius, Some(Deviation::Normal)).1, "Typical for the date");
        assert_eq!(average_text(7.4, Units::Celsius, None).1, "Not available");
    }

    #[test]
    fn the_needle_points_toward_where_the_wind_comes_from() {
        let (x, y) = bearing_offset(0.0, 10.0);
        assert!(x.abs() < 1e-9 && (y + 10.0).abs() < 1e-9, "north is up");
        let (x, y) = bearing_offset(90.0, 10.0);
        assert!((x - 10.0).abs() < 1e-9 && y.abs() < 1e-9, "east is right");
    }

    #[test]
    fn scale_covers_the_hours_and_ticks_are_round() {
        // lo 1.0, hi 5.8: span 4.8, raw step 1.6 -> 2; the values 2 and 4 lie inside the range
        let s = temperature_scale(&[hour(3.2), hour(5.8), hour(1.0)]).unwrap();
        assert_eq!((s.lo, s.hi), (1.0, 5.8));
        assert_eq!(s.ticks, vec![2.0, 4.0]);
    }

    #[test]
    fn a_flat_forecast_is_widened_and_an_empty_one_has_no_scale() {
        let s = temperature_scale(&[hour(5.0), hour(5.0)]).unwrap();
        assert_eq!((s.lo, s.hi), (4.5, 5.5));
        assert!(temperature_scale(&[]).is_none());
        assert!(temperature_scale(&[hour(f64::NAN)]).is_none());
    }

    #[test]
    fn points_keep_the_hour_position_and_skip_missing_temperatures() {
        assert_eq!(temperature_points(&[hour(1.0), hour(f64::NAN), hour(3.0)]), vec![(0.0, 1.0), (2.0, 3.0)]);
    }

    #[test]
    fn area_columns_follow_the_line_at_each_step() {
        // two segments: (0,10)-(10,20) then (10,20)-(20,0); a step of 5 gives x = 0, 5, 10, 15, 20
        let line = [(0.0, 10.0), (10.0, 20.0), (20.0, 0.0)];
        assert_eq!(area_columns(&line, 5.0), vec![(0.0, 10.0), (5.0, 15.0), (10.0, 20.0), (15.0, 10.0), (20.0, 0.0)]);
        // one point, or a step that is not positive, gives no area
        assert!(area_columns(&[(0.0, 1.0)], 1.0).is_empty());
        assert!(area_columns(&line, 0.0).is_empty());
    }

    #[test]
    fn the_line_maps_into_the_plot_frame() {
        // frame 100 wide and 50 high at (10, 20): x 0..2 maps to 10..110, y 1..3 maps up from 70 to 20
        let frame = Frame::new(10.0, 20.0, 100.0, 50.0);
        let scale = TempScale { lo: 1.0, hi: 3.0, ticks: vec![] };
        let plot = temperature_plot(&scale, 3, frame);
        let line = temperature_line(&plot, &[hour(1.0), hour(2.0), hour(3.0)]);
        assert_eq!(line, vec![(10.0, 70.0), (60.0, 45.0), (110.0, 20.0)]);
    }

    #[test]
    fn map_view_is_centred_on_the_place_at_the_card_zoom() {
        let v = map_view(64.0, -21.0, 400.0, 200.0).unwrap();
        assert_eq!(v.camera().zoom, MAP_ZOOM);
        assert_eq!((v.camera().center.latitude, v.camera().center.longitude), (64.0, -21.0));
        assert_eq!(v.map_type, MapType::Standard);
        assert!(map_view(91.0, 0.0, 400.0, 200.0).is_none());
        assert!(map_view(64.0, 0.0, 0.0, 200.0).is_none());
    }

    #[test]
    fn grid_lines_are_inside_the_view_and_ordered_on_screen() {
        let v = map_view(64.0, -21.0, 400.0, 200.0).unwrap();
        let g = grid(&v);
        assert!(!g.rows.is_empty() && !g.cols.is_empty());
        for &(y, _) in &g.rows {
            assert!((0.0..=200.0).contains(&y));
        }
        for pair in g.rows.windows(2) {
            assert!(pair[0].0 < pair[1].0, "a higher latitude sits higher up, so it comes first");
            assert!(pair[0].1 > pair[1].1);
        }
        for pair in g.cols.windows(2) {
            assert!(pair[0].0 < pair[1].0, "a larger longitude sits further right");
        }
    }

    #[test]
    fn labels_name_the_hemisphere() {
        assert_eq!(coordinate_label(64.5, 'N', 'S'), "64.5\u{b0}N");
        assert_eq!(coordinate_label(-21.5, 'E', 'W'), "21.5\u{b0}W");
    }

    #[test]
    fn scale_text_follows_the_temperature_setting() {
        assert_eq!(scale_text(2000.0, Units::Celsius), "2.0 km");
        assert_eq!(scale_text(500.0, Units::Celsius), "500 m");
    }

    #[test]
    fn location_note_says_what_weather_uses_in_each_status() {
        assert!(location_note(AuthorizationStatus::NotDetermined, None).contains("not asked"));
        assert!(location_note(AuthorizationStatus::Denied, None).contains("turned off"));
        assert!(location_note(AuthorizationStatus::AuthorizedWhenInUse, None).starts_with("Location: on"));
        assert!(location_note(AuthorizationStatus::Denied, Some(LocationError::Network)).contains("network"));
        assert!(location_note(AuthorizationStatus::Denied, Some(LocationError::PromptDeclined)).contains("not allowed"));
    }

    #[test]
    fn map_note_names_the_map_type() {
        assert!(map_note(MapType::Hybrid).starts_with("Hybrid map"));
    }

    #[test]
    fn footer_says_alerts_are_unavailable_then_credits_the_service() {
        let lines = footer_lines();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "Weather alerts are not available for this place from the data service.");
        assert_eq!(lines[1], "Weather data from Open-Meteo");
    }

}
