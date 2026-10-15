//! Weather models that need no network: the moon phase at a moment, wind (speed, gust and the compass direction it
//! comes from) and a typed weather error that apps can match on. The types mirror the official weather framework's
//! `MoonPhase`, `Wind` (`CompassDirection`, `gust`, `direction`) and `WeatherError` (`permissionDenied`, `unknown`).
//! The moon phase comes from the mean lunation, not from an ephemeris, so it can be a day off near a quarter.
//! Times are passed in as Unix seconds (UTC); nothing here reads the clock.

use std::error::Error;
use std::f64::consts::PI;
use std::fmt;
use std::io;

/// Length of one synodic month (new moon to the next new moon), in days.
pub const SYNODIC_MONTH_DAYS: f64 = 29.530_588_853;
/// Julian day of a reference new moon (2000-01-06, 18:14 UTC).
const REFERENCE_NEW_MOON_JD: f64 = 2_451_550.26;
/// Julian day of the Unix epoch, 1970-01-01 00:00 UTC.
const UNIX_EPOCH_JD: f64 = 2_440_587.5;
const SECONDS_PER_DAY: f64 = 86_400.0;
/// Kilometres per hour to miles per hour.
const KMH_TO_MPH: f64 = 0.621_371;

/// The eight phases of the moon. Each phase covers one eighth of the cycle, centred on its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoonPhase {
    New,
    WaxingCrescent,
    FirstQuarter,
    WaxingGibbous,
    Full,
    WaningGibbous,
    LastQuarter,
    WaningCrescent,
}

impl MoonPhase {
    /// Every phase in cycle order, starting at the new moon.
    pub const ALL: [MoonPhase; 8] = [
        MoonPhase::New,
        MoonPhase::WaxingCrescent,
        MoonPhase::FirstQuarter,
        MoonPhase::WaxingGibbous,
        MoonPhase::Full,
        MoonPhase::WaningGibbous,
        MoonPhase::LastQuarter,
        MoonPhase::WaningCrescent,
    ];

    /// The phase after `age` days since a new moon. Ages outside one cycle wrap around.
    pub fn from_age_days(age: f64) -> MoonPhase {
        let fraction = age.rem_euclid(SYNODIC_MONTH_DAYS) / SYNODIC_MONTH_DAYS;
        let index = ((fraction * 8.0 + 0.5).floor() as usize) % 8;
        MoonPhase::ALL[index]
    }

    /// The phase at a Unix time in seconds (UTC).
    pub fn at_unix(seconds: f64) -> MoonPhase {
        MoonPhase::from_age_days(age_days(seconds))
    }

    /// The English name shown to people, such as "Waxing Gibbous".
    pub fn label(self) -> &'static str {
        match self {
            MoonPhase::New => "New Moon",
            MoonPhase::WaxingCrescent => "Waxing Crescent",
            MoonPhase::FirstQuarter => "First Quarter",
            MoonPhase::WaxingGibbous => "Waxing Gibbous",
            MoonPhase::Full => "Full Moon",
            MoonPhase::WaningGibbous => "Waning Gibbous",
            MoonPhase::LastQuarter => "Last Quarter",
            MoonPhase::WaningCrescent => "Waning Crescent",
        }
    }

    /// True while the lit part grows (new moon to full moon).
    pub fn is_waxing(self) -> bool {
        matches!(
            self,
            MoonPhase::WaxingCrescent | MoonPhase::FirstQuarter | MoonPhase::WaxingGibbous
        )
    }
}

/// Days since the reference new moon at a Unix time in seconds, between 0 and one synodic month.
pub fn age_days(unix_seconds: f64) -> f64 {
    let jd = unix_seconds / SECONDS_PER_DAY + UNIX_EPOCH_JD;
    (jd - REFERENCE_NEW_MOON_JD).rem_euclid(SYNODIC_MONTH_DAYS)
}

/// The fraction of the moon's disk that is lit at a Unix time: 0 at the new moon, 1 at the full moon.
pub fn illuminated_fraction(unix_seconds: f64) -> f64 {
    let cycle = age_days(unix_seconds) / SYNODIC_MONTH_DAYS;
    (1.0 - (2.0 * PI * cycle).cos()) / 2.0
}

/// The eight compass points a wind direction is reported in: cardinal and intercardinal directions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompassDirection {
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
}

impl CompassDirection {
    /// The point nearest to `degrees`, measured clockwise from north. Each point covers 45 degrees centred on it.
    pub fn from_degrees(degrees: f64) -> CompassDirection {
        let index = (((degrees.rem_euclid(360.0)) + 22.5) / 45.0).floor() as usize % 8;
        [
            CompassDirection::North,
            CompassDirection::NorthEast,
            CompassDirection::East,
            CompassDirection::SouthEast,
            CompassDirection::South,
            CompassDirection::SouthWest,
            CompassDirection::West,
            CompassDirection::NorthWest,
        ][index]
    }

    /// The English abbreviation: "N", "NE", "E" and so on.
    pub fn abbreviation(self) -> &'static str {
        match self {
            CompassDirection::North => "N",
            CompassDirection::NorthEast => "NE",
            CompassDirection::East => "E",
            CompassDirection::SouthEast => "SE",
            CompassDirection::South => "S",
            CompassDirection::SouthWest => "SW",
            CompassDirection::West => "W",
            CompassDirection::NorthWest => "NW",
        }
    }
}

/// Wind at a place: the sustained speed, the direction it comes from and an optional gust, all in metric units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wind {
    /// Sustained speed in kilometres per hour.
    pub speed_kmh: f64,
    /// The direction the wind comes from, in degrees clockwise from north, always within 0..360.
    pub direction_deg: f64,
    /// A sudden increase in speed in kilometres per hour, when the source reports one.
    pub gust_kmh: Option<f64>,
}

impl Wind {
    /// A wind reading. The direction is folded into 0..360 so `350 + 20` reads as 10.
    pub fn new(speed_kmh: f64, direction_deg: f64, gust_kmh: Option<f64>) -> Wind {
        Wind {
            speed_kmh,
            direction_deg: direction_deg.rem_euclid(360.0),
            gust_kmh,
        }
    }

    /// The compass point the wind comes from.
    pub fn compass(&self) -> CompassDirection {
        CompassDirection::from_degrees(self.direction_deg)
    }

    /// The sustained speed in miles per hour.
    pub fn speed_mph(&self) -> f64 {
        self.speed_kmh * KMH_TO_MPH
    }

    /// The gust as a multiple of the sustained speed, or `None` with no gust or no wind.
    pub fn gust_factor(&self) -> Option<f64> {
        match self.gust_kmh {
            Some(gust) if self.speed_kmh > 0.0 => Some(gust / self.speed_kmh),
            _ => None,
        }
    }
}

/// A weather failure an app can match on. Other causes fall into `Unknown`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeatherError {
    /// The user or the system refused access to weather data.
    PermissionDenied,
    /// The forecast service could not be reached: no connection, a refused connection, or a timeout.
    Unreachable,
    /// The service answered, but the answer could not be read as weather data.
    BadAnswer,
    /// Anything else.
    Unknown,
}

impl WeatherError {
    /// The error for a failed read. A refused permission stays `PermissionDenied`, a network failure is `Unreachable`,
    /// an unreadable answer is `BadAnswer`, and every other kind is `Unknown`.
    pub fn from_io(err: &io::Error) -> WeatherError {
        match err.kind() {
            io::ErrorKind::PermissionDenied => WeatherError::PermissionDenied,
            io::ErrorKind::NotConnected
            | io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::TimedOut
            | io::ErrorKind::HostUnreachable
            | io::ErrorKind::NetworkUnreachable
            | io::ErrorKind::NetworkDown
            | io::ErrorKind::UnexpectedEof => WeatherError::Unreachable,
            io::ErrorKind::InvalidData => WeatherError::BadAnswer,
            _ => WeatherError::Unknown,
        }
    }

    /// True when trying the same request again later may succeed. A refused permission never does by itself.
    pub fn is_retryable(self) -> bool {
        !matches!(self, WeatherError::PermissionDenied)
    }

    /// What went wrong, in one sentence.
    pub fn description(self) -> &'static str {
        match self {
            WeatherError::PermissionDenied => "Weather data is not allowed for this app.",
            WeatherError::Unreachable => "The weather service could not be reached.",
            WeatherError::BadAnswer => "The weather service sent an answer that could not be read.",
            WeatherError::Unknown => "The weather could not be loaded.",
        }
    }

    /// What the person can do about it.
    pub fn recovery_suggestion(self) -> &'static str {
        match self {
            WeatherError::PermissionDenied => "Allow weather access in Privacy settings and try again.",
            WeatherError::Unreachable => "Check the connection and try again.",
            WeatherError::BadAnswer => "Try again later.",
            WeatherError::Unknown => "Check the connection and try again.",
        }
    }
}

impl fmt::Display for WeatherError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(WeatherError::description(*self))
    }
}

impl Error for WeatherError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn unix_of_reference() -> f64 {
        (REFERENCE_NEW_MOON_JD - UNIX_EPOCH_JD) * SECONDS_PER_DAY
    }

    #[test]
    fn reference_new_moon_is_new() {
        let t = unix_of_reference();
        assert!(age_days(t) < 1e-6 || age_days(t) > SYNODIC_MONTH_DAYS - 1e-6);
        assert_eq!(MoonPhase::at_unix(t), MoonPhase::New);
        assert!(illuminated_fraction(t) < 1e-6);
    }

    #[test]
    fn half_cycle_is_full_moon_and_fully_lit() {
        let t = unix_of_reference() + (SYNODIC_MONTH_DAYS / 2.0) * SECONDS_PER_DAY;
        assert_eq!(MoonPhase::at_unix(t), MoonPhase::Full);
        assert!((illuminated_fraction(t) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn quarters_fall_at_quarter_and_three_quarter_cycle() {
        assert_eq!(MoonPhase::from_age_days(SYNODIC_MONTH_DAYS / 4.0), MoonPhase::FirstQuarter);
        assert_eq!(MoonPhase::from_age_days(SYNODIC_MONTH_DAYS * 0.75), MoonPhase::LastQuarter);
        assert!(MoonPhase::FirstQuarter.is_waxing());
        assert!(!MoonPhase::LastQuarter.is_waxing());
    }

    #[test]
    fn ages_outside_one_cycle_wrap_around() {
        // One day before a new moon is still in the last eighth of the cycle, which rounds into the new moon bin.
        assert_eq!(MoonPhase::from_age_days(-1.0), MoonPhase::New);
        assert_eq!(MoonPhase::from_age_days(SYNODIC_MONTH_DAYS + 0.1), MoonPhase::New);
        assert_eq!(
            MoonPhase::from_age_days(SYNODIC_MONTH_DAYS * 0.5 + SYNODIC_MONTH_DAYS * 3.0),
            MoonPhase::Full
        );
    }

    #[test]
    fn eight_bins_cover_the_cycle_in_order() {
        for (k, expected) in MoonPhase::ALL.iter().enumerate() {
            let age = SYNODIC_MONTH_DAYS * k as f64 / 8.0;
            assert_eq!(MoonPhase::from_age_days(age), *expected, "bin {k}");
        }
        let labels: Vec<&str> = MoonPhase::ALL.iter().map(|p| p.label()).collect();
        assert_eq!(labels.len(), 8);
        assert!(labels.iter().all(|l| !l.is_empty()));
    }

    #[test]
    fn compass_points_split_at_22_5_degree_edges() {
        assert_eq!(CompassDirection::from_degrees(0.0), CompassDirection::North);
        assert_eq!(CompassDirection::from_degrees(22.4), CompassDirection::North);
        assert_eq!(CompassDirection::from_degrees(22.6), CompassDirection::NorthEast);
        assert_eq!(CompassDirection::from_degrees(90.0), CompassDirection::East);
        assert_eq!(CompassDirection::from_degrees(225.0), CompassDirection::SouthWest);
        assert_eq!(CompassDirection::from_degrees(-10.0), CompassDirection::North);
        assert_eq!(CompassDirection::from_degrees(359.0), CompassDirection::North);
        assert_eq!(CompassDirection::SouthWest.abbreviation(), "SW");
    }

    #[test]
    fn wind_folds_direction_and_reports_gust_factor() {
        let wind = Wind::new(20.0, 350.0 + 20.0, Some(35.0));
        assert!((wind.direction_deg - 10.0).abs() < 1e-9);
        assert_eq!(wind.compass(), CompassDirection::North);
        assert!((wind.gust_factor().unwrap() - 1.75).abs() < 1e-9);
        assert!((Wind::new(100.0, 0.0, None).speed_mph() - 62.1371).abs() < 1e-3);
        assert_eq!(Wind::new(20.0, 0.0, None).gust_factor(), None);
        assert_eq!(Wind::new(0.0, 0.0, Some(5.0)).gust_factor(), None);
    }

    #[test]
    fn weather_error_maps_permission_and_everything_else() {
        let denied = io::Error::new(io::ErrorKind::PermissionDenied, "no");
        let other = io::Error::other("timeout");
        assert_eq!(WeatherError::from_io(&denied), WeatherError::PermissionDenied);
        assert_eq!(WeatherError::from_io(&other), WeatherError::Unknown);
        assert_ne!(
            WeatherError::PermissionDenied.description(),
            WeatherError::Unknown.description()
        );
        assert_eq!(
            WeatherError::Unknown.to_string(),
            WeatherError::Unknown.description()
        );
        let as_error: &dyn Error = &WeatherError::Unknown;
        assert!(as_error.source().is_none());
    }
}
