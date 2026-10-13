//! Distances and times for display: the Kilometres / Miles setting, the duration text of a route, and the scale bar.

pub use bifrost_app::controls::geo::{format_distance, DistanceUnits};

/// The Units setting of the View menu: follow the region, or force one system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitsSetting {
    Automatic,
    Kilometres,
    Miles,
}

impl UnitsSetting {
    pub const ALL: [UnitsSetting; 3] = [UnitsSetting::Automatic, UnitsSetting::Kilometres, UnitsSetting::Miles];

    pub fn label(self) -> &'static str {
        match self {
            UnitsSetting::Automatic => "Automatic",
            UnitsSetting::Kilometres => "Kilometres",
            UnitsSetting::Miles => "Miles",
        }
    }

    /// The value stored in the preferences.
    pub fn key(self) -> &'static str {
        match self {
            UnitsSetting::Automatic => "automatic",
            UnitsSetting::Kilometres => "kilometres",
            UnitsSetting::Miles => "miles",
        }
    }

    /// An unknown or missing value is Automatic.
    pub fn from_key(key: &str) -> UnitsSetting {
        Self::ALL.into_iter().find(|u| u.key() == key).unwrap_or(UnitsSetting::Automatic)
    }

    /// The unit system to draw with. `region_is_metric` is the region's own answer (used by Automatic).
    pub fn resolve(self, region_is_metric: bool) -> DistanceUnits {
        match self {
            UnitsSetting::Kilometres => DistanceUnits::Metric,
            UnitsSetting::Miles => DistanceUnits::Imperial,
            UnitsSetting::Automatic if region_is_metric => DistanceUnits::Metric,
            UnitsSetting::Automatic => DistanceUnits::Imperial,
        }
    }
}

/// A travel time: "under 1 min", "7 min", "1 h 05 min".
pub fn format_duration(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0.0 {
        return "unknown".into();
    }
    let minutes = (seconds / 60.0).round() as u64;
    match minutes {
        0 => "under 1 min".into(),
        1..=59 => format!("{minutes} min"),
        _ => format!("{} h {:02} min", minutes / 60, minutes % 60),
    }
}

/// A scale bar: how long to draw it (screen points) and its label.
#[derive(Clone, Debug, PartialEq)]
pub struct ScaleBar {
    pub length_pt: f64,
    pub label: String,
}

/// The longest "nice" distance (1, 2 or 5 times a power of ten) that fits in `max_pt` points when one point is
/// `metres_per_pt` metres. `None` when the numbers are not usable.
pub fn scale_bar(metres_per_pt: f64, max_pt: f64, units: DistanceUnits) -> Option<ScaleBar> {
    if !(metres_per_pt.is_finite() && metres_per_pt > 0.0 && max_pt.is_finite() && max_pt > 0.0) {
        return None;
    }
    let max_m = metres_per_pt * max_pt;
    // the unit to count in: kilometres or metres; miles or feet
    let (per_unit, big, small, small_m) = match units {
        DistanceUnits::Metric => (1000.0, "km", "m", 1.0),
        DistanceUnits::Imperial => (1609.344, "mi", "ft", 0.3048),
    };
    let (unit_m, name) = if max_m >= per_unit { (per_unit, big) } else { (small_m, small) };
    let max_units = max_m / unit_m;
    let magnitude = 10f64.powf(max_units.log10().floor());
    let nice = [5.0, 2.0, 1.0].into_iter().map(|m| m * magnitude).find(|v| *v <= max_units)?;
    let label = if nice >= 1.0 { format!("{nice:.0} {name}") } else { format!("{nice} {name}") };
    Some(ScaleBar { length_pt: nice * unit_m / metres_per_pt, label })
}

/// Metres covered by one screen point at a latitude and zoom (Web Mercator, 256-point tiles).
pub fn metres_per_pt(lat: f64, zoom: f64) -> f64 {
    const EQUATOR_M: f64 = 40_075_016.686;
    lat.to_radians().cos() * EQUATOR_M / crate::tiles::world_px(zoom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_round_trips_and_automatic_follows_the_region() {
        for u in UnitsSetting::ALL {
            assert_eq!(UnitsSetting::from_key(u.key()), u);
        }
        assert_eq!(UnitsSetting::from_key("nonsense"), UnitsSetting::Automatic);
        assert_eq!(UnitsSetting::Automatic.resolve(true), DistanceUnits::Metric);
        assert_eq!(UnitsSetting::Automatic.resolve(false), DistanceUnits::Imperial);
        assert_eq!(UnitsSetting::Miles.resolve(true), DistanceUnits::Imperial);
        assert_eq!(UnitsSetting::Kilometres.resolve(false), DistanceUnits::Metric);
    }

    #[test]
    fn distances_use_the_chosen_system() {
        assert_eq!(format_distance(1500.0, DistanceUnits::Metric), "1.5 km");
        assert_eq!(format_distance(1609.344, DistanceUnits::Imperial), "1.0 mi");
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(format_duration(20.0), "under 1 min");
        assert_eq!(format_duration(61.0), "1 min");
        assert_eq!(format_duration(3600.0 + 300.0), "1 h 05 min");
        assert_eq!(format_duration(f64::NAN), "unknown");
        assert_eq!(format_duration(-3.0), "unknown");
    }

    #[test]
    fn the_scale_bar_picks_a_round_length_that_fits() {
        // 10 m per point and 100 points of room: 1000 m fits exactly, so 1 km
        let bar = scale_bar(10.0, 100.0, DistanceUnits::Metric).unwrap();
        assert_eq!(bar.label, "1 km");
        assert!((bar.length_pt - 100.0).abs() < 1e-9);
        // 3 m per point and 100 points: 300 m of room, so 200 m
        let bar = scale_bar(3.0, 100.0, DistanceUnits::Metric).unwrap();
        assert_eq!(bar.label, "200 m");
        assert!(bar.length_pt <= 100.0);
        // 20 m per point: 2000 m of room, 1 mi is 1609 m and fits
        let bar = scale_bar(20.0, 100.0, DistanceUnits::Imperial).unwrap();
        assert_eq!(bar.label, "1 mi");
        assert!(bar.length_pt <= 100.0 + 1e-9);
        assert!(scale_bar(0.0, 100.0, DistanceUnits::Metric).is_none());
        assert!(scale_bar(1.0, f64::NAN, DistanceUnits::Metric).is_none());
    }

    #[test]
    fn metres_per_point_halves_with_each_zoom_level_and_shrinks_away_from_the_equator() {
        let a = metres_per_pt(0.0, 3.0);
        let b = metres_per_pt(0.0, 4.0);
        assert!((a / b - 2.0).abs() < 1e-9);
        assert!((metres_per_pt(0.0, 0.0) - 40_075_016.686 / 256.0).abs() < 1e-6);
        assert!(metres_per_pt(64.0, 3.0) < a);
    }
}
