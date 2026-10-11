//! Altitude from air pressure: the standard atmosphere turns a pressure reading in kilopascal into metres above sea
//! level, and a relative altimeter reports the height change since its first reading.
//!
//! The relative altimeter is exact for changes, because the sea-level pressure cancels out of a difference. An
//! absolute altitude needs the sea-level pressure of the day, which the caller takes from weather data; with the
//! standard 101.325 kPa it is the textbook altitude. The formula holds to about 11 km. The module owns no sensor and
//! no clock: the caller feeds the pressure readings it takes.

/// The standard sea-level pressure in kilopascal.
pub const STANDARD_SEA_LEVEL_KPA: f64 = 101.325;

/// Readings outside this range are not air pressure at any height a device reaches; they are refused.
const MIN_PRESSURE_KPA: f64 = 30.0;
const MAX_PRESSURE_KPA: f64 = 110.0;

fn plausible(pressure_kpa: f64) -> bool {
    pressure_kpa.is_finite() && (MIN_PRESSURE_KPA..=MAX_PRESSURE_KPA).contains(&pressure_kpa)
}

/// The altitude in metres above sea level of a pressure reading in kilopascal, for a given sea-level pressure:
/// `h = 44330 * (1 - (p / p0)^(1 / 5.255))`. Returns `None` for a pressure or sea-level value outside the plausible
/// range.
pub fn absolute_altitude_m(pressure_kpa: f64, sea_level_kpa: f64) -> Option<f64> {
    if !plausible(pressure_kpa) || !plausible(sea_level_kpa) {
        return None;
    }
    Some(44_330.0 * (1.0 - (pressure_kpa / sea_level_kpa).powf(1.0 / 5.255)))
}

/// One altimeter reading: the change in height since the baseline, and the pressure it came from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AltitudeData {
    /// metres above the height of the first accepted reading; negative when the device has gone down
    pub relative_altitude_m: f64,
    pub pressure_kpa: f64,
}

/// Reports the height change since its first accepted pressure reading.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RelativeAltimeter {
    start_kpa: Option<f64>,
}

impl RelativeAltimeter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets the baseline: the next accepted reading becomes the new zero.
    pub fn reset(&mut self) {
        self.start_kpa = None;
    }

    /// Takes a pressure reading in kilopascal. The first accepted reading sets the baseline and reads zero. Returns
    /// `None`, and leaves the baseline alone, for a reading outside the plausible range.
    pub fn sample(&mut self, pressure_kpa: f64) -> Option<AltitudeData> {
        if !plausible(pressure_kpa) {
            return None;
        }
        let start = *self.start_kpa.get_or_insert(pressure_kpa);
        let now = absolute_altitude_m(pressure_kpa, STANDARD_SEA_LEVEL_KPA)?;
        let base = absolute_altitude_m(start, STANDARD_SEA_LEVEL_KPA)?;
        Some(AltitudeData { relative_altitude_m: now - base, pressure_kpa })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_standard_atmosphere_gives_the_textbook_altitudes() {
        // sea level reads zero; the standard pressure at 1000 m is about 89.875 kPa
        assert!(absolute_altitude_m(STANDARD_SEA_LEVEL_KPA, STANDARD_SEA_LEVEL_KPA).unwrap().abs() < 1e-9);
        let at_1000 = absolute_altitude_m(89.875, STANDARD_SEA_LEVEL_KPA).unwrap();
        assert!((at_1000 - 1000.0).abs() < 3.0, "got {at_1000}");
    }

    #[test]
    fn a_higher_pressure_than_the_sea_level_reference_is_below_it() {
        let below = absolute_altitude_m(102.5, STANDARD_SEA_LEVEL_KPA).unwrap();
        assert!(below < 0.0 && below > -200.0, "got {below}");
    }

    #[test]
    fn implausible_pressures_are_refused() {
        assert_eq!(absolute_altitude_m(f64::NAN, STANDARD_SEA_LEVEL_KPA), None);
        assert_eq!(absolute_altitude_m(0.0, STANDARD_SEA_LEVEL_KPA), None);
        assert_eq!(absolute_altitude_m(250.0, STANDARD_SEA_LEVEL_KPA), None);
        assert_eq!(absolute_altitude_m(90.0, f64::INFINITY), None);
    }

    #[test]
    fn the_relative_altimeter_zeroes_at_its_first_reading_and_reads_the_climb() {
        let mut alt = RelativeAltimeter::new();
        let first = alt.sample(STANDARD_SEA_LEVEL_KPA).unwrap();
        assert!(first.relative_altitude_m.abs() < 1e-9);
        // a drop of about 1.3 kPa is roughly 110 m of climb
        let climbed = alt.sample(100.0).unwrap();
        assert!(climbed.relative_altitude_m > 100.0 && climbed.relative_altitude_m < 120.0, "got {}", climbed.relative_altitude_m);
        assert_eq!(climbed.pressure_kpa, 100.0);
    }

    #[test]
    fn a_refused_reading_does_not_move_the_baseline() {
        let mut alt = RelativeAltimeter::new();
        assert_eq!(alt.sample(f64::NAN), None, "nothing accepted yet, so nothing starts");
        alt.sample(99.0).unwrap();
        assert_eq!(alt.sample(500.0), None);
        // the baseline is still the 99 kPa reading, so 99 reads zero
        assert!(alt.sample(99.0).unwrap().relative_altitude_m.abs() < 1e-9);
    }

    #[test]
    fn reset_starts_a_new_baseline() {
        let mut alt = RelativeAltimeter::new();
        alt.sample(100.0).unwrap();
        alt.sample(95.0).unwrap();
        alt.reset();
        assert!(alt.sample(95.0).unwrap().relative_altitude_m.abs() < 1e-9);
    }
}
