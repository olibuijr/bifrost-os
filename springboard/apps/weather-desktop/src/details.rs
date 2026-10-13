//! The wording and numbers of the detail tiles and the precipitation cards: wind direction and gusts, dew
//! point, pressure and its trend, visibility, precipitation amounts, the next-hour summary and the sun's
//! position on its arc. Pure functions over the forecast values, so the screen only draws what they return.
//! The unit system follows the temperature setting: Celsius shows metric units, Fahrenheit the imperial set.

use springboard::shell::weather::data::Units;

const HPA_TO_INHG: f64 = 0.029_529_98;
const MM_TO_IN: f64 = 0.039_370_08;
const M_TO_MI: f64 = 0.000_621_371;

/// A precipitation quarter hour counts as wet from this amount (millimetres in 15 minutes).
pub const WET_MM: f64 = 0.05;

/// The compass point the wind comes from: "N", "NE", ... "NW".
pub fn compass_label(degrees: f64) -> &'static str {
    const POINTS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    POINTS[(((degrees.rem_euclid(360.0)) + 22.5) / 45.0).floor() as usize % 8]
}

fn speed(kmh: f64, units: Units) -> String {
    match units {
        Units::Celsius => format!("{} km/h", kmh.round() as i32),
        Units::Fahrenheit => format!("{} mph", (kmh * 0.621_371).round() as i32),
    }
}

/// The note under the wind speed: where it comes from and, when stronger than the wind, the gusts.
pub fn wind_note(direction_deg: f64, gust_kmh: f64, sustained_kmh: f64, units: Units) -> String {
    let from = format!("From the {}", compass_label(direction_deg));
    if gust_kmh > sustained_kmh + 0.5 {
        format!("{from}, gusts {}", speed(gust_kmh, units))
    } else {
        from
    }
}

/// The note under the humidity: the dew point in the temperature unit.
pub fn dew_note(dew_celsius: f64, units: Units) -> String {
    format!("The dew point is {}", units.degrees(dew_celsius))
}

/// Pressure as the tile shows it: hPa, or inHg with two decimals. "--" when the forecast has none.
pub fn pressure_text(hpa: f64, units: Units) -> String {
    if hpa <= 0.0 {
        return "--".into();
    }
    match units {
        Units::Celsius => format!("{} hPa", hpa.round() as i32),
        Units::Fahrenheit => format!("{:.2} inHg", hpa * HPA_TO_INHG),
    }
}

/// The trend note from the change over the next three hours in hPa.
pub fn pressure_note(change_hpa: f64) -> &'static str {
    if change_hpa >= 1.0 {
        "Rising"
    } else if change_hpa <= -1.0 {
        "Falling"
    } else {
        "Steady"
    }
}

/// Visibility in km or miles; whole numbers from 10, one decimal below. "--" when unknown.
pub fn visibility_text(metres: f64, units: Units) -> String {
    if metres <= 0.0 {
        return "--".into();
    }
    let (v, unit) = match units {
        Units::Celsius => (metres / 1000.0, "km"),
        Units::Fahrenheit => (metres * M_TO_MI, "mi"),
    };
    if v >= 10.0 { format!("{} {unit}", v.round() as i32) } else { format!("{v:.1} {unit}") }
}

/// The note under the visibility.
pub fn visibility_note(metres: f64) -> &'static str {
    if metres <= 0.0 {
        "No visibility data"
    } else if metres >= 10_000.0 {
        "Clear view"
    } else if metres >= 4_000.0 {
        "Slightly hazy"
    } else if metres >= 1_000.0 {
        "Hazy"
    } else {
        "Poor visibility"
    }
}

/// A precipitation amount in mm or inches.
pub fn precip_text(mm: f64, units: Units) -> String {
    match units {
        Units::Celsius => if mm >= 10.0 { format!("{} mm", mm.round() as i32) } else { format!("{mm:.1} mm") },
        Units::Fahrenheit => format!("{:.2} in", mm * MM_TO_IN),
    }
}

/// The note under the precipitation tile (which shows the next 24 hours): tomorrow's amount.
pub fn precip_note(tomorrow_mm: f64, units: Units) -> String {
    format!("{} expected tomorrow", precip_text(tomorrow_mm, units))
}

/// The percentage under an hour's glyph; empty below 10 so dry hours stay quiet.
pub fn hour_chance(percent: u8) -> String {
    if percent < 10 { String::new() } else { format!("{}%", (percent as u32 + 2) / 5 * 5) }
}

/// The percentage on a day row; empty below 10.
pub fn day_chance(percent: u8) -> String {
    hour_chance(percent)
}

/// The sentence of the Next Hour card from the precipitation of the coming quarter hours (mm per 15 minutes).
pub fn next_hour_summary(minutes: &[f64]) -> String {
    if minutes.is_empty() {
        return "Minute-by-minute precipitation is not available for this place.".into();
    }
    let wet: Vec<bool> = minutes.iter().map(|&m| m >= WET_MM).collect();
    match (wet[0], wet.iter().position(|&w| w != wet[0])) {
        (false, None) => "No precipitation expected in the next hour.".into(),
        (false, Some(i)) => format!("Precipitation starting in {} minutes.", i * 15),
        (true, None) => "Precipitation for the next hour.".into(),
        (true, Some(i)) => format!("Precipitation stopping in {} minutes.", i * 15),
    }
}

/// Bar heights 0..1 for the Next Hour chart: scaled against 2 mm per quarter hour (heavy rain) at least, so
/// drizzle does not fill the chart.
pub fn next_hour_bars(minutes: &[f64]) -> Vec<f32> {
    let top = minutes.iter().copied().fold(2.0, f64::max);
    minutes.iter().map(|&m| (m / top).clamp(0.0, 1.0) as f32).collect()
}

/// Minutes since midnight of a "06:12" clock text.
pub fn clock_minutes(text: &str) -> Option<u32> {
    let (h, m) = text.split_once(':')?;
    let (h, m): (u32, u32) = (h.trim().parse().ok()?, m.trim().parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// How far the sun is along its arc: 0 at sunrise, 1 at sunset, clamped. None when a time is missing or the
/// sun sets before it rises.
pub fn sun_progress(sunrise: &str, sunset: &str, now_minutes: u32) -> Option<f32> {
    let (rise, set) = (clock_minutes(sunrise)?, clock_minutes(sunset)?);
    (set > rise).then(|| ((now_minutes as f32 - rise as f32) / (set - rise) as f32).clamp(0.0, 1.0))
}

/// The position of the sun on a half-circle arc of `radius` centred on (`cx`, `cy`), for a progress 0..1.
pub fn sun_position(progress: f32, cx: f32, cy: f32, radius: f32) -> (f32, f32) {
    let a = std::f32::consts::PI * (1.0 - progress.clamp(0.0, 1.0));
    (cx + radius * a.cos(), cy - radius * a.sin())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compass_points_cover_the_circle() {
        assert_eq!(compass_label(0.0), "N");
        assert_eq!(compass_label(359.0), "N");
        assert_eq!(compass_label(225.0), "SW");
        assert_eq!(compass_label(-90.0), "W");
        assert_eq!(compass_label(112.4), "E");
        assert_eq!(compass_label(112.6), "SE");
    }

    #[test]
    fn wind_note_names_the_direction_and_only_real_gusts() {
        assert_eq!(wind_note(225.0, 41.0, 22.5, Units::Celsius), "From the SW, gusts 41 km/h");
        assert_eq!(wind_note(225.0, 41.0, 22.5, Units::Fahrenheit), "From the SW, gusts 25 mph");
        assert_eq!(wind_note(90.0, 0.0, 10.0, Units::Celsius), "From the E");
    }

    #[test]
    fn dew_and_pressure_follow_the_unit_system() {
        assert_eq!(dew_note(4.2, Units::Celsius), "The dew point is 4\u{b0}");
        assert_eq!(dew_note(0.0, Units::Fahrenheit), "The dew point is 32\u{b0}");
        assert_eq!(pressure_text(1013.4, Units::Celsius), "1013 hPa");
        assert_eq!(pressure_text(1013.25, Units::Fahrenheit), "29.92 inHg");
        assert_eq!(pressure_text(0.0, Units::Celsius), "--");
        assert_eq!((pressure_note(2.0), pressure_note(-1.5), pressure_note(0.4)), ("Rising", "Falling", "Steady"));
    }

    #[test]
    fn visibility_and_precipitation_amounts() {
        assert_eq!(visibility_text(24_000.0, Units::Celsius), "24 km");
        assert_eq!(visibility_text(9_000.0, Units::Celsius), "9.0 km");
        assert_eq!(visibility_text(16_100.0, Units::Fahrenheit), "10 mi");
        assert_eq!(visibility_text(0.0, Units::Celsius), "--");
        assert_eq!((visibility_note(20_000.0), visibility_note(5_000.0), visibility_note(2_000.0), visibility_note(500.0)), ("Clear view", "Slightly hazy", "Hazy", "Poor visibility"));
        assert_eq!(precip_text(4.5, Units::Celsius), "4.5 mm");
        assert_eq!(precip_text(25.4, Units::Fahrenheit), "1.00 in");
        assert_eq!(precip_note(12.0, Units::Celsius), "12 mm expected tomorrow");
    }

    #[test]
    fn chances_round_to_five_and_hide_below_ten() {
        assert_eq!(hour_chance(0), "");
        assert_eq!(hour_chance(9), "");
        assert_eq!(hour_chance(10), "10%");
        assert_eq!(hour_chance(62), "60%");
        assert_eq!(hour_chance(63), "65%");
        assert_eq!(day_chance(100), "100%");
    }

    #[test]
    fn next_hour_sentence_tells_when_it_starts_or_stops() {
        assert!(next_hour_summary(&[]).contains("not available"));
        assert_eq!(next_hour_summary(&[0.0, 0.0, 0.0, 0.0]), "No precipitation expected in the next hour.");
        assert_eq!(next_hour_summary(&[0.0, 0.0, 0.3, 0.1]), "Precipitation starting in 30 minutes.");
        assert_eq!(next_hour_summary(&[0.4, 0.2, 0.0, 0.0]), "Precipitation stopping in 30 minutes.");
        assert_eq!(next_hour_summary(&[0.4, 0.2, 0.1, 0.1]), "Precipitation for the next hour.");
    }

    #[test]
    fn next_hour_bars_scale_against_at_least_two_millimetres() {
        assert_eq!(next_hour_bars(&[0.0, 1.0, 4.0]), vec![0.0, 0.25, 1.0]);
        assert_eq!(next_hour_bars(&[0.5]), vec![0.25]);
        assert!(next_hour_bars(&[]).is_empty());
    }

    #[test]
    fn sun_progress_runs_from_sunrise_to_sunset() {
        assert_eq!(clock_minutes("06:30"), Some(390));
        assert_eq!(clock_minutes("24:00"), None);
        assert_eq!(clock_minutes(""), None);
        assert_eq!(sun_progress("06:00", "18:00", 12 * 60), Some(0.5));
        assert_eq!(sun_progress("06:00", "18:00", 3 * 60), Some(0.0));
        assert_eq!(sun_progress("06:00", "18:00", 23 * 60), Some(1.0));
        assert_eq!(sun_progress("", "18:00", 600), None);
        assert_eq!(sun_progress("18:00", "06:00", 600), None);
    }

    #[test]
    fn sun_sits_on_the_arc() {
        let (x, y) = sun_position(0.0, 100.0, 50.0, 40.0);
        assert!((x - 60.0).abs() < 1e-3 && (y - 50.0).abs() < 1e-3, "sunrise is on the left end of the arc");
        let (x, y) = sun_position(0.5, 100.0, 50.0, 40.0);
        assert!((x - 100.0).abs() < 1e-3 && (y - 10.0).abs() < 1e-3, "noon is at the top");
    }
}
