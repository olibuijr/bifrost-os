//! The pure parts of the forecast screen: the temperature chart and map card, the texts and numbers of the detail tiles
//! (wind, precipitation, visibility, pressure, averages), the Next Hour summary, the hourly strip's columns, the sun
//! arc and wind dial geometry, reordering of the places, `weather:` links, and the Locations note about location
//! access. Nothing here draws or talks to the network: the weather screen draws what these return, and the tests
//! check the numbers by hand.

use bifrost_app::controls::chart::axis_ticks;
use bifrost_app::controls::chart_marks::{Frame, Interpolation, Plot, line_path};
use bifrost_app::controls::geo::{Coordinate, DistanceUnits, format_distance};
use bifrost_app::controls::location_auth::{AuthorizationStatus, LocationError};
use bifrost_app::controls::mapview::{MapCamera, MapType, MapView};
use bifrost_app::controls::weather_alert::WeatherAvailability;
use bifrost_app::controls::weather_attribution::WeatherAttribution;
use bifrost_app::controls::weather_model::Wind;
use springboard::shell::weather::data::{Hour, Units};

/// The data service the forecast comes from, and the page that carries its terms.
pub const DATA_SERVICE: &str = "Open-Meteo";
pub const DATA_LEGAL_URL: &str = "https://open-meteo.com/en/terms";
/// The forecast request asks for no weather alerts, so the place has none from this source. It does ask for the
/// minute forecast (`minutely_15`), which the Next Hour card draws.
pub const DATA_HAS_ALERTS: bool = false;
pub const DATA_HAS_MINUTE: bool = true;

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

// ---- measurements in the unit set of the temperature setting ----
// Celsius shows the metric set (km/h, mm, km, hPa) and Fahrenheit the imperial set (mph, in, mi, inHg).

/// Wind speed as the detail tile shows it: km/h with the Celsius setting, mph with Fahrenheit.
pub fn wind_text(kmh: f64, units: Units) -> String {
    match units {
        Units::Celsius => format!("{} km/h", kmh.round() as i32),
        Units::Fahrenheit => format!("{} mph", Wind::new(kmh, 0.0, None).speed_mph().round() as i32),
    }
}

/// The 16-point compass name of a bearing in degrees clockwise from north: 0 is "N", 315 is "NW".
pub fn compass_point(degrees: f64) -> &'static str {
    const NAMES: [&str; 16] = ["N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW"];
    NAMES[((degrees.rem_euclid(360.0) / 22.5 + 0.5).floor() as usize) % 16]
}

/// The note under the wind speed: the gusts and the bearing the wind comes from, when the service gave them.
pub fn wind_note(gust_kmh: Option<f64>, from_degrees: Option<f64>, units: Units) -> String {
    match (gust_kmh, from_degrees) {
        (Some(g), Some(d)) => format!("Gusts {} \u{b7} From {}", wind_text(g, units), compass_point(d)),
        (Some(g), None) => format!("Gusts {}", wind_text(g, units)),
        (None, Some(d)) => format!("From {}", compass_point(d)),
        (None, None) => "Current speed".to_string(),
    }
}

/// The note under the humidity: the dew point when known.
pub fn humidity_note(dew_point: Option<f64>, units: Units) -> String {
    dew_point.map_or_else(|| "Relative humidity".to_string(), |d| format!("The dew point is {} right now", units.degrees(d)))
}

/// Precipitation: "6.5 mm" or "0.26 in".
pub fn precipitation_text(mm: f64, units: Units) -> String {
    match units {
        Units::Celsius => format!("{mm:.1} mm"),
        Units::Fahrenheit => format!("{:.2} in", mm / 25.4),
    }
}

/// Visibility: "24 km" or "9.0 km", and "15 mi" with the imperial set.
pub fn visibility_text(metres: f64, units: Units) -> String {
    let (value, unit) = match units {
        Units::Celsius => (metres / 1000.0, "km"),
        Units::Fahrenheit => (metres / 1609.344, "mi"),
    };
    if value >= 10.0 { format!("{} {unit}", value.round() as i32) } else { format!("{value:.1} {unit}") }
}

/// The note under the visibility.
pub fn visibility_note(metres: f64) -> &'static str {
    match metres {
        m if m >= 10_000.0 => "Clear view",
        m if m >= 4_000.0 => "Good visibility",
        m if m >= 1_000.0 => "Reduced visibility",
        _ => "Poor visibility",
    }
}

/// Pressure: "1008 hPa" or "29.78 inHg".
pub fn pressure_text(hpa: f64, units: Units) -> String {
    match units {
        Units::Celsius => format!("{} hPa", hpa.round() as i32),
        Units::Fahrenheit => format!("{:.2} inHg", hpa * 0.029_53),
    }
}

/// The change of pressure from now to three hours ahead, in hPa (the last hour given when there are fewer than four).
/// None when there are fewer than two hours or the hours carry no pressure.
pub fn pressure_change(hours: &[Hour]) -> Option<f64> {
    if hours.len() < 2 {
        return None;
    }
    let first = hours.first()?.pressure_hpa?;
    let last = hours.get(3).or(hours.last())?.pressure_hpa?;
    Some(last - first)
}

/// Whether the pressure rises or falls over the next three hours: a change of 1 hPa or more counts. None when the
/// hours carry no pressure.
pub fn pressure_trend(hours: &[Hour]) -> Option<&'static str> {
    Some(match pressure_change(hours)? {
        d if d >= 1.0 => "Rising",
        d if d <= -1.0 => "Falling",
        _ => "Steady",
    })
}

/// The needle of the pressure trend dial, in degrees from straight up: 0 when steady, 90 when rising and -90 when
/// falling, reached at a change of 6 hPa in either direction. None when the change is unknown.
pub fn pressure_dial_angle(hours: &[Hour]) -> Option<f64> {
    pressure_change(hours).map(|d| (d / 6.0).clamp(-1.0, 1.0) * 90.0)
}

/// The note under the pressure.
pub fn pressure_note(hours: &[Hour]) -> String {
    pressure_trend(hours).map_or_else(|| "Mean sea level".to_string(), |t| format!("{t} over the next 3 hours"))
}

/// The Averages tile: the difference between today's high and the mean high of this date, and a note naming that
/// mean. `high` and `average` are in Celsius, so a difference converts by the scale factor alone.
pub fn average_text(high: f64, average: Option<f64>, units: Units) -> (String, String) {
    let Some(average) = average else { return ("--".to_string(), "Climate data is not available".to_string()) };
    let delta = match units {
        Units::Celsius => high - average,
        Units::Fahrenheit => (high - average) * 9.0 / 5.0,
    }
    .round() as i32;
    let value = if delta > 0 { format!("+{delta}\u{b0}") } else { format!("{delta}\u{b0}") };
    let word = match delta {
        d if d > 0 => "Above",
        d if d < 0 => "Below",
        _ => "Same as",
    };
    (value, format!("{word} the 10-year average high of {}", units.degrees(average)))
}

// ---- precipitation chances and the next hour ----

/// Rain of at least this many mm in a 15-minute step counts as wet.
pub const WET_STEP_MM: f64 = 0.1;

/// A chance of precipitation as the strips show it, rounded to ten: "70%". None below 10%, where nothing is shown.
pub fn chance_text(percent: u8) -> Option<String> {
    let rounded = (percent as u32 + 5) / 10 * 10;
    (rounded >= 10).then(|| format!("{}%", rounded.min(100)))
}

/// The line over the Next Hour card for the precipitation of the coming 15-minute steps (the first is the step
/// that is under way now).
pub fn next_hour_summary(minutes: &[f64]) -> String {
    if minutes.is_empty() {
        return "Minute-by-minute rain is not available for this place.".to_string();
    }
    let wet: Vec<bool> = minutes.iter().map(|m| *m >= WET_STEP_MM).collect();
    match (wet[0], wet.iter().position(|w| *w != wet[0])) {
        (false, None) => "No rain expected in the next hour".to_string(),
        (false, Some(i)) => format!("Rain starting in {} minutes", i * 15),
        (true, None) => "Rain for the next hour".to_string(),
        (true, Some(i)) => format!("Rain stopping in {} minutes", i * 15),
    }
}

/// Minutes until rain starts when it is dry now and wet within the hour; None when it is wet now, or stays dry.
pub fn rain_due_in(minutes: &[f64]) -> Option<u32> {
    let first_dry = minutes.first().is_some_and(|m| *m < WET_STEP_MM);
    if !first_dry {
        return None;
    }
    minutes.iter().position(|m| *m >= WET_STEP_MM).map(|i| i as u32 * 15)
}

/// The height of a Next Hour bar as a share of the card's plot, 0 to 1: 2 mm in a step fills it.
pub fn rain_bar(mm: f64) -> f32 {
    ((mm / 2.0) as f32).clamp(0.0, 1.0)
}

// ---- air quality ----

/// The band of the European index, 0 (Good) to 5 (Extremely Poor), matching `Air::category`.
pub fn air_band(aqi: u16) -> usize {
    match aqi {
        0..=20 => 0,
        21..=40 => 1,
        41..=60 => 2,
        61..=80 => 3,
        81..=100 => 4,
        _ => 5,
    }
}

/// Where an index value sits on the card's scale bar, 0 to 1: the bar ends at 120.
pub fn air_position(aqi: u16) -> f32 {
    (aqi as f32 / 120.0).clamp(0.0, 1.0)
}

// ---- the hourly strip ----

/// A column of the hourly strip: an hour, or the sunrise or sunset that falls just after the hour at that index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Hour(usize),
    Sun(usize),
}

/// The strip's columns in order: every hour, with a sun column after the hour that holds a sunrise or sunset.
pub fn strip_columns(hours: &[Hour]) -> Vec<Column> {
    let mut out = Vec::with_capacity(hours.len() + 2);
    for (i, h) in hours.iter().enumerate() {
        out.push(Column::Hour(i));
        if h.sun.is_some() {
            out.push(Column::Sun(i));
        }
    }
    out
}

// ---- the sun and the wind dial ----

/// Minutes after midnight of a clock "06:12", or of the time part of "2026-10-04T06:12".
pub fn clock_minutes(text: &str) -> Option<u32> {
    let clock = if text.len() >= 16 { text.get(11..16)? } else { text };
    let (h, m) = clock.split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// How far the sun is along its day arc, 0 at sunrise to 1 at sunset. None at night, and when a time is missing.
pub fn sun_progress(now: &str, sunrise: &str, sunset: &str) -> Option<f32> {
    let (now, rise, set) = (clock_minutes(now)?, clock_minutes(sunrise)?, clock_minutes(sunset)?);
    (set > rise && now >= rise && now <= set).then(|| (now - rise) as f32 / (set - rise) as f32)
}

/// A point on the day arc: `k` 0 is the left end of the horizon, 1 the right end, 0.5 the top, around centre
/// `(cx, cy)` at `radius`. The y axis points down.
pub fn arc_point(k: f32, cx: f32, cy: f32, radius: f32) -> (f32, f32) {
    let angle = std::f32::consts::PI * (1.0 - k.clamp(0.0, 1.0));
    (cx + radius * angle.cos(), cy - radius * angle.sin())
}

/// A point on the wind dial: `degrees` is a bearing clockwise from north (up), around `(cx, cy)` at `radius`.
pub fn dial_point(degrees: f64, cx: f32, cy: f32, radius: f32) -> (f32, f32) {
    let angle = (degrees as f32).to_radians();
    (cx + radius * angle.sin(), cy - radius * angle.cos())
}

// ---- reordering the places ----

/// Moves the item at `from` to position `to`. False when either index is outside the list or they are the same.
pub fn move_item<T>(list: &mut Vec<T>, from: usize, to: usize) -> bool {
    if from >= list.len() || to >= list.len() || from == to {
        return false;
    }
    let item = list.remove(from);
    list.insert(to, item);
    true
}

/// Where the selected index ends up after the item at `from` moved to `to`.
pub fn follow_selection(selected: usize, from: usize, to: usize) -> usize {
    if selected == from {
        to
    } else if from < selected && to >= selected {
        selected - 1
    } else if from > selected && to <= selected {
        selected + 1
    } else {
        selected
    }
}

/// The row a drag dropped at height `y` lands on, from the rows' `(top, height)`: the row under `y`, the first row
/// above the list and the last one below it.
pub fn drop_index(rows: &[(f32, f32)], y: f32) -> usize {
    rows.iter().position(|&(top, h)| y < top + h).unwrap_or(rows.len().saturating_sub(1))
}

// ---- weather: links ----

/// The place a `weather:` link names: `weather://Akureyri`, `weather:Akureyri` or, with a position,
/// `weather://?lat=65.68&lon=-18.1&name=Akureyri`. The name is decoded; a link with only a position is named by it.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkedPlace {
    pub name: String,
    pub coordinate: Option<(f64, f64)>,
}

/// The longest place name a link may carry.
const MAX_LINK_NAME: usize = 64;

pub fn place_from_url(url: &str) -> Option<LinkedPlace> {
    let url = url.trim();
    let rest = url.get(..8).filter(|p| p.eq_ignore_ascii_case("weather:")).and_then(|_| url.get(8..))?;
    let rest = rest.trim_start_matches('/');
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let param = |key: &str| query.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| percent_decode(v)));
    let coordinate = match (param("lat").and_then(|v| v.trim().parse::<f64>().ok()), param("lon").and_then(|v| v.trim().parse::<f64>().ok())) {
        (Some(lat), Some(lon)) if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) => Some((lat, lon)),
        _ => None,
    };
    let named = param("name").or_else(|| param("q")).unwrap_or_else(|| percent_decode(path.trim_matches('/')));
    let mut name = named.trim().to_string();
    if name.is_empty() {
        let (lat, lon) = coordinate?;
        name = format!("{lat:.2}, {lon:.2}");
    }
    (name.chars().count() <= MAX_LINK_NAME).then_some(LinkedPlace { name, coordinate })
}

/// Percent-decoding of a link component; "+" is a space and a bad escape stays as written.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() && (bytes[i + 1] as char).is_ascii_hexdigit() && (bytes[i + 2] as char).is_ascii_hexdigit() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("00");
                out.push(u8::from_str_radix(hex, 16).unwrap_or(b'?'));
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hour(temp: f64) -> Hour {
        Hour::basic("13", temp, 0, true)
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

    #[test]
    fn wind_follows_the_unit_set_and_names_the_bearing() {
        assert_eq!(wind_text(20.4, Units::Celsius), "20 km/h");
        assert_eq!(wind_text(100.0, Units::Fahrenheit), "62 mph");
        assert_eq!(compass_point(0.0), "N");
        assert_eq!(compass_point(315.0), "NW");
        assert_eq!(compass_point(359.0), "N", "the last sector wraps to north");
        assert_eq!(compass_point(-90.0), "W");
        assert_eq!(compass_point(101.0), "E");
        assert_eq!(wind_note(Some(41.0), Some(315.0), Units::Celsius), "Gusts 41 km/h \u{b7} From NW");
        assert_eq!(wind_note(Some(41.0), None, Units::Celsius), "Gusts 41 km/h");
        assert_eq!(wind_note(None, Some(90.0), Units::Celsius), "From E");
        assert_eq!(wind_note(None, None, Units::Celsius), "Current speed");
    }

    #[test]
    fn humidity_note_gives_the_dew_point_in_the_setting_unit() {
        assert_eq!(humidity_note(Some(4.2), Units::Celsius), "The dew point is 4\u{b0} right now");
        assert_eq!(humidity_note(Some(0.0), Units::Fahrenheit), "The dew point is 32\u{b0} right now");
        assert_eq!(humidity_note(None, Units::Celsius), "Relative humidity");
    }

    #[test]
    fn precipitation_visibility_and_pressure_follow_the_unit_set() {
        assert_eq!(precipitation_text(6.5, Units::Celsius), "6.5 mm");
        assert_eq!(precipitation_text(25.4, Units::Fahrenheit), "1.00 in");
        assert_eq!(visibility_text(24_000.0, Units::Celsius), "24 km");
        assert_eq!(visibility_text(9_000.0, Units::Celsius), "9.0 km");
        assert_eq!(visibility_text(24_000.0, Units::Fahrenheit), "15 mi");
        assert_eq!(visibility_text(3_218.688, Units::Fahrenheit), "2.0 mi");
        assert_eq!(visibility_note(24_000.0), "Clear view");
        assert_eq!(visibility_note(5_000.0), "Good visibility");
        assert_eq!(visibility_note(1_500.0), "Reduced visibility");
        assert_eq!(visibility_note(200.0), "Poor visibility");
        assert_eq!(pressure_text(1008.4, Units::Celsius), "1008 hPa");
        assert_eq!(pressure_text(1013.25, Units::Fahrenheit), "29.92 inHg");
    }

    fn with_pressure(values: &[Option<f64>]) -> Vec<Hour> {
        values.iter().map(|p| Hour { pressure_hpa: *p, ..hour(5.0) }).collect()
    }

    #[test]
    fn pressure_trend_compares_now_with_three_hours_ahead() {
        let rising = with_pressure(&[Some(1000.0), Some(1000.5), Some(1001.0), Some(1002.0), Some(1010.0)]);
        assert_eq!(pressure_trend(&rising), Some("Rising"), "the fourth hour is the one compared");
        assert_eq!(pressure_note(&rising), "Rising over the next 3 hours");
        assert_eq!(pressure_trend(&with_pressure(&[Some(1010.0), Some(1008.0), Some(1005.0), Some(1004.0)])), Some("Falling"));
        assert_eq!(pressure_trend(&with_pressure(&[Some(1010.0), Some(1010.4)])), Some("Steady"), "a short list compares with its last hour");
        assert_eq!(pressure_trend(&with_pressure(&[Some(1010.0)])), None);
        assert_eq!(pressure_trend(&with_pressure(&[None, None])), None);
        assert_eq!(pressure_note(&[]), "Mean sea level");
    }

    #[test]
    fn the_pressure_dial_swings_with_the_three_hour_change() {
        let near = |a: Option<f64>, want: f64| a.is_some_and(|a| (a - want).abs() < 1e-9);
        let rising = with_pressure(&[Some(1000.0), Some(1000.5), Some(1001.0), Some(1002.0), Some(1010.0)]);
        assert_eq!(pressure_change(&rising), Some(2.0));
        assert!(near(pressure_dial_angle(&rising), 30.0), "2 hPa is a third of the way to the rim");
        let falling = with_pressure(&[Some(1010.0), Some(1008.0), Some(1005.0), Some(1004.0)]);
        assert!(near(pressure_dial_angle(&falling), -90.0), "6 hPa or more is the rim");
        let steady = with_pressure(&[Some(1010.0), Some(1010.4)]);
        assert!(near(pressure_dial_angle(&steady), 6.0), "0.4 hPa is 6 degrees off straight up");
        assert_eq!(pressure_dial_angle(&with_pressure(&[Some(1010.0)])), None);
    }

    #[test]
    fn averages_state_the_difference_from_the_ten_year_mean() {
        assert_eq!(average_text(10.0, Some(7.0), Units::Celsius), ("+3\u{b0}".to_string(), "Above the 10-year average high of 7\u{b0}".to_string()));
        assert_eq!(average_text(4.0, Some(7.0), Units::Celsius).0, "-3\u{b0}");
        assert_eq!(average_text(7.2, Some(7.0), Units::Celsius), ("0\u{b0}".to_string(), "Same as the 10-year average high of 7\u{b0}".to_string()));
        // a 5 degree Celsius difference is 9 degrees Fahrenheit; the mean 7 C is 45 F
        assert_eq!(average_text(12.0, Some(7.0), Units::Fahrenheit), ("+9\u{b0}".to_string(), "Above the 10-year average high of 45\u{b0}".to_string()));
        assert_eq!(average_text(4.0, Some(7.0), Units::Celsius).1, "Below the 10-year average high of 7\u{b0}");
        assert_eq!(average_text(10.0, None, Units::Celsius), ("--".to_string(), "Climate data is not available".to_string()));
    }

    #[test]
    fn chances_round_to_ten_and_hide_below_ten() {
        assert_eq!(chance_text(0), None);
        assert_eq!(chance_text(4), None);
        assert_eq!(chance_text(5).as_deref(), Some("10%"));
        assert_eq!(chance_text(73).as_deref(), Some("70%"));
        assert_eq!(chance_text(100).as_deref(), Some("100%"));
    }

    #[test]
    fn the_next_hour_line_says_when_rain_starts_or_stops() {
        assert_eq!(next_hour_summary(&[]), "Minute-by-minute rain is not available for this place.");
        assert_eq!(next_hour_summary(&[0.0, 0.05, 0.0, 0.0]), "No rain expected in the next hour");
        assert_eq!(next_hour_summary(&[0.0, 0.0, 0.3, 0.0]), "Rain starting in 30 minutes");
        assert_eq!(next_hour_summary(&[0.4, 0.2, 0.0, 0.0]), "Rain stopping in 30 minutes");
        assert_eq!(next_hour_summary(&[0.4, 0.2, 0.1, 1.0]), "Rain for the next hour");
    }

    #[test]
    fn rain_is_due_only_when_it_is_dry_now_and_wet_soon() {
        assert_eq!(rain_due_in(&[0.0, 0.0, 0.3, 0.0]), Some(30));
        assert_eq!(rain_due_in(&[0.0, 0.5, 0.0, 0.0]), Some(15));
        assert_eq!(rain_due_in(&[0.4, 0.0, 0.3, 0.0]), None, "it is raining already");
        assert_eq!(rain_due_in(&[0.0, 0.0, 0.0, 0.0]), None);
        assert_eq!(rain_due_in(&[]), None);
    }

    #[test]
    fn rain_bars_fill_at_two_millimetres() {
        assert_eq!(rain_bar(0.0), 0.0);
        assert_eq!(rain_bar(1.0), 0.5);
        assert_eq!(rain_bar(5.0), 1.0);
        assert_eq!(rain_bar(-1.0), 0.0);
    }

    #[test]
    fn air_bands_and_the_scale_position_follow_the_index() {
        assert_eq!([0, 20, 21, 40, 41, 60, 61, 80, 81, 100, 101].map(air_band), [0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5]);
        assert_eq!(air_position(0), 0.0);
        assert_eq!(air_position(60), 0.5);
        assert_eq!(air_position(500), 1.0, "the marker stops at the end of the bar");
    }

    #[test]
    fn a_sun_column_follows_the_hour_that_holds_the_sunrise_or_sunset() {
        use springboard::shell::weather::data::SunEvent;
        let mut hours = vec![hour(1.0), hour(2.0), hour(3.0)];
        hours[1].sun = Some(SunEvent { rise: false, time: "18:20".into() });
        assert_eq!(strip_columns(&hours), vec![Column::Hour(0), Column::Hour(1), Column::Sun(1), Column::Hour(2)]);
        assert_eq!(strip_columns(&[]), vec![]);
    }

    #[test]
    fn clock_minutes_reads_a_clock_and_an_iso_time() {
        assert_eq!(clock_minutes("06:12"), Some(372));
        assert_eq!(clock_minutes("2026-10-04T13:20"), Some(800));
        assert_eq!(clock_minutes("24:00"), None);
        assert_eq!(clock_minutes("noon"), None);
        assert_eq!(clock_minutes(""), None);
    }

    #[test]
    fn the_sun_moves_along_its_arc_between_sunrise_and_sunset() {
        assert_eq!(sun_progress("2026-10-04T07:41", "07:41", "18:21"), Some(0.0));
        assert_eq!(sun_progress("2026-10-04T13:01", "07:41", "18:21"), Some(0.5));
        assert_eq!(sun_progress("2026-10-04T18:21", "07:41", "18:21"), Some(1.0));
        assert_eq!(sun_progress("2026-10-04T05:00", "07:41", "18:21"), None, "before sunrise");
        assert_eq!(sun_progress("2026-10-04T22:00", "07:41", "18:21"), None, "after sunset");
        assert_eq!(sun_progress("2026-10-04T13:00", "", "18:21"), None, "no sunrise time");
        assert_eq!(sun_progress("2026-10-04T13:00", "18:21", "07:41"), None, "a sunset before the sunrise (polar day or night)");
    }

    fn near(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn the_day_arc_runs_left_to_right_over_the_horizon() {
        assert!(near(arc_point(0.0, 100.0, 50.0, 40.0), (60.0, 50.0)));
        assert!(near(arc_point(0.5, 100.0, 50.0, 40.0), (100.0, 10.0)), "noon is the top, above the horizon");
        assert!(near(arc_point(1.0, 100.0, 50.0, 40.0), (140.0, 50.0)));
        assert!(near(arc_point(9.0, 100.0, 50.0, 40.0), (140.0, 50.0)), "k is clamped");
    }

    #[test]
    fn the_wind_dial_is_a_compass_with_north_up() {
        assert!(near(dial_point(0.0, 100.0, 100.0, 30.0), (100.0, 70.0)), "north is up");
        assert!(near(dial_point(90.0, 100.0, 100.0, 30.0), (130.0, 100.0)), "east is right");
        assert!(near(dial_point(180.0, 100.0, 100.0, 30.0), (100.0, 130.0)));
        assert!(near(dial_point(270.0, 100.0, 100.0, 30.0), (70.0, 100.0)));
    }

    #[test]
    fn moving_a_place_keeps_the_selection_on_the_same_place() {
        let mut v = vec!['a', 'b', 'c', 'd'];
        assert!(move_item(&mut v, 0, 2));
        assert_eq!(v, vec!['b', 'c', 'a', 'd']);
        assert!(!move_item(&mut v, 1, 1) && !move_item(&mut v, 4, 0) && !move_item(&mut v, 0, 4));
        assert_eq!(v, vec!['b', 'c', 'a', 'd'], "a refused move changes nothing");
        // 'b' was selected at 1 and 'a' moved from 0 to 2: 'b' is now first
        assert_eq!(follow_selection(1, 0, 2), 0);
        assert_eq!(follow_selection(2, 0, 2), 1);
        assert_eq!(follow_selection(0, 0, 2), 2, "the moved place stays selected");
        assert_eq!(follow_selection(3, 0, 2), 3);
        // moving the other way: [a, b, c, d] with 3 -> 1 gives [a, d, b, c]
        assert_eq!(follow_selection(1, 3, 1), 2);
        assert_eq!(follow_selection(2, 3, 1), 3);
        assert_eq!(follow_selection(0, 3, 1), 0);
        assert_eq!(follow_selection(3, 3, 1), 1);
    }

    #[test]
    fn a_drop_lands_on_the_row_under_the_finger() {
        let rows = [(0.0, 32.0), (32.0, 32.0), (64.0, 32.0)];
        assert_eq!(drop_index(&rows, -10.0), 0);
        assert_eq!(drop_index(&rows, 10.0), 0);
        assert_eq!(drop_index(&rows, 32.0), 1);
        assert_eq!(drop_index(&rows, 95.0), 2);
        assert_eq!(drop_index(&rows, 400.0), 2);
        assert_eq!(drop_index(&[], 10.0), 0);
    }

    #[test]
    fn a_weather_link_names_a_place_or_a_position() {
        let named = |n: &str| LinkedPlace { name: n.into(), coordinate: None };
        assert_eq!(place_from_url("weather://Akureyri"), Some(named("Akureyri")));
        assert_eq!(place_from_url("weather:Akureyri"), Some(named("Akureyri")));
        assert_eq!(place_from_url("WEATHER://S%C3%A3o%20Paulo/"), Some(named("S\u{e3}o Paulo")));
        assert_eq!(place_from_url("weather://New+York"), Some(named("New York")));
        assert_eq!(
            place_from_url("weather://?lat=65.68&lon=-18.1&name=Akureyri"),
            Some(LinkedPlace { name: "Akureyri".into(), coordinate: Some((65.68, -18.1)) })
        );
        assert_eq!(place_from_url("weather://?lat=65.68&lon=-18.1"), Some(LinkedPlace { name: "65.68, -18.10".into(), coordinate: Some((65.68, -18.1)) }));
        assert_eq!(place_from_url("weather://Oslo?lat=95&lon=0"), Some(named("Oslo")), "an impossible position is dropped, the name stays");
        assert_eq!(place_from_url("weather://"), None);
        assert_eq!(place_from_url("weather://?lat=95&lon=0"), None);
        assert_eq!(place_from_url("https://example.com/Oslo"), None);
        assert_eq!(place_from_url(&format!("weather://{}", "x".repeat(65))), None, "an over-long name");
        assert_eq!(percent_decode("100%"), "100%", "a bad escape stays as written");
    }
}
