//! The pure parts of the forecast's temperature chart and map card, and the Locations note about location access.
//! Nothing here draws or talks to the network: the weather screen draws what these return, and the tests check the
//! numbers by hand.

use bifrost_app::controls::chart::axis_ticks;
use bifrost_app::controls::chart_marks::{Frame, Interpolation, Plot, line_path};
use bifrost_app::controls::geo::{Coordinate, DistanceUnits, format_distance};
use bifrost_app::controls::location_auth::{AuthorizationStatus, LocationError};
use bifrost_app::controls::mapview::{MapCamera, MapType, MapView};
use springboard::shell::weather::data::{Hour, Units};

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

#[cfg(test)]
mod tests {
    use super::*;

    fn hour(temp: f64) -> Hour {
        Hour { label: "13".into(), temp, code: 0, day: true }
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
}
