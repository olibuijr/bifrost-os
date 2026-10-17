//! Weather for apps (`WeatherService`, `Weather`, `CurrentWeather`, `HourWeather`, `DayWeather`, `Wind`, `WeatherMetadata`,
//! `WeatherAttribution`): the forecast of a place the person saved in the Weather app, with the current conditions, the
//! next hours, ten days, air quality, the minutes of rain ahead, the wind in a compass point and a Beaufort force, and
//! the phase of the moon.
//!
//! The service reads the forecast the shell already keeps for the person's saved places (`shell::weather`, refreshed
//! once an hour and on wake). It makes no request of its own, so an app learns nothing about the person's location
//! except the places they chose to keep, and only those places are served: [`WeatherService::weather`] refuses a place
//! the Weather app does not hold ([`Error::NotSaved`]). The data comes from Open-Meteo, whose licence asks for
//! attribution: show [`ATTRIBUTION`] next to the weather.
//!
//! ```ignore
//! let service = weather_service::WeatherService::shared();
//! let place = service.places().into_iter().next().ok_or("no place saved")?;
//! let report = service.weather(&place)?;
//! println!("{} {:.0} C, wind {}", report.condition().label(), report.forecast.current.temp, report.wind().compass);
//! ```
//!
//! Limits: no weather alerts (the data source has none), no per-place history beyond the ten-year mean high
//! (`Forecast::average_high`), and the moon is computed from the mean lunar cycle, so its phase is right to within about
//! half a day and its illumination to a few percent.

use springboard::shell::weather::data::{Forecast, Location, Store};
use std::path::PathBuf;

pub use springboard::shell::weather::data::{Air, Day, Hour, Pollutant, Sky, Units};
pub use springboard::shell::weather::data::{Forecast as WeatherForecast, Location as Place};

/// The licence notice a view that shows this weather carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attribution {
    pub provider: &'static str,
    pub url: &'static str,
    pub license: &'static str,
}

/// Open-Meteo, the source of the forecast, licensed CC BY 4.0.
pub const ATTRIBUTION: Attribution = Attribution { provider: "Open-Meteo.com", url: "https://open-meteo.com/", license: "CC BY 4.0" };

/// A report older than this has missed at least one hourly refresh (the shell refreshes hourly and on wake).
pub const FRESH_S: u64 = 90 * 60;

/// Why no weather was given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// the place is not one the person saved in the Weather app
    NotSaved,
    /// the shell holds no forecast for the place yet (it has not been fetched, or the cache was cleared)
    NoData,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotSaved => write!(f, "the place is not saved in Weather"),
            Error::NoData => write!(f, "there is no forecast for the place yet"),
        }
    }
}

/// The weather of one place, as the shell last fetched it.
#[derive(Clone, Debug, PartialEq)]
pub struct Weather {
    pub place: Location,
    pub forecast: Forecast,
    /// seconds since the forecast was fetched
    pub age_secs: u64,
}

/// `Wind`: speed, direction and gusts, with the compass point and the Beaufort force.
#[derive(Clone, Debug, PartialEq)]
pub struct Wind {
    pub speed_kmh: f64,
    /// degrees clockwise from north that the wind blows from
    pub direction_deg: Option<f64>,
    /// "NW": the compass point of the direction; empty when the data has none
    pub compass: &'static str,
    pub gust_kmh: Option<f64>,
    pub beaufort: u8,
    pub beaufort_text: &'static str,
}

impl Weather {
    /// The sky now.
    pub fn condition(&self) -> Sky {
        Sky::from_code(self.forecast.current.code)
    }

    /// The wind now.
    pub fn wind(&self) -> Wind {
        let c = &self.forecast.current;
        let beaufort = beaufort(c.wind_kmh);
        Wind {
            speed_kmh: c.wind_kmh,
            direction_deg: c.wind_dir,
            compass: c.wind_dir.map_or("", compass_label),
            gust_kmh: c.wind_gust_kmh,
            beaufort,
            beaufort_text: beaufort_label(beaufort),
        }
    }

    /// Whether the forecast is older than [`FRESH_S`].
    pub fn is_stale(&self) -> bool {
        self.age_secs > FRESH_S
    }

    /// Minutes until the next quarter hour with at least 0.1 mm of rain or snow, counted from the first step (0 means it
    /// is falling now). `None` when the minute data shows none or the data source gave no minute steps.
    pub fn minutes_until_precipitation(&self) -> Option<u32> {
        self.forecast.minutes.iter().position(|mm| *mm >= 0.1).map(|i| i as u32 * 15)
    }

    /// The next hours, starting with the current one.
    pub fn hourly(&self) -> &[Hour] {
        &self.forecast.hours
    }

    /// The days, starting with today.
    pub fn daily(&self) -> &[Day] {
        &self.forecast.days
    }

    /// The air quality now, when the shell has it.
    pub fn air(&self) -> Option<&Air> {
        self.forecast.air.as_ref()
    }

    /// Today's sunrise and sunset as local "HH:MM" text, when the data has both.
    pub fn daylight(&self) -> Option<(&str, &str)> {
        let today = self.forecast.days.first()?;
        (!today.sunrise.is_empty() && !today.sunset.is_empty()).then_some((today.sunrise.as_str(), today.sunset.as_str()))
    }

    /// The attribution to show with this weather.
    pub fn attribution(&self) -> Attribution {
        ATTRIBUTION
    }
}

/// The weather service over the shell's weather folder.
#[derive(Clone, Debug)]
pub struct WeatherService {
    store: Store,
}

impl WeatherService {
    /// A service over the weather folder `dir`.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        WeatherService { store: Store::new(dir) }
    }

    /// The service over the folder the shell keeps (`shell::weather::data_path`).
    pub fn shared() -> Self {
        Self::new(springboard::shell::weather::data_path())
    }

    /// The places the person saved in the Weather app; empty until they have saved one.
    pub fn places(&self) -> Vec<Location> {
        if self.store.has_places() {
            self.store.load_locations()
        } else {
            Vec::new()
        }
    }

    /// The weather of `place`, from the forecast the shell holds. Only a saved place is served.
    pub fn weather(&self, place: &Location) -> Result<Weather, Error> {
        let saved = self.places().into_iter().find(|p| p.key() == place.key()).ok_or(Error::NotSaved)?;
        let (forecast, age_secs) = self.store.load_forecast(&saved).ok_or(Error::NoData)?;
        Ok(Weather { place: saved, forecast, age_secs })
    }
}

const COMPASS: [&str; 16] = ["N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW"];

/// The 16-point compass label of a bearing in degrees (wrapped into one turn): 0 is "N", 90 is "E", 315 is "NW".
pub fn compass_label(degrees: f64) -> &'static str {
    if !degrees.is_finite() {
        return "";
    }
    let turn = degrees.rem_euclid(360.0);
    COMPASS[((turn / 22.5) + 0.5).floor() as usize % 16]
}

/// The Beaufort force (0 to 12) of a wind speed in kilometres per hour.
pub fn beaufort(speed_kmh: f64) -> u8 {
    const LIMITS: [f64; 12] = [1.0, 6.0, 12.0, 20.0, 29.0, 39.0, 50.0, 62.0, 75.0, 89.0, 103.0, 118.0];
    if !speed_kmh.is_finite() || speed_kmh < 0.0 {
        return 0;
    }
    LIMITS.iter().take_while(|limit| speed_kmh >= **limit).count() as u8
}

/// The name of a Beaufort force; a force past 12 reads as 12.
pub fn beaufort_label(force: u8) -> &'static str {
    const NAMES: [&str; 13] = [
        "Calm", "Light air", "Light breeze", "Gentle breeze", "Moderate breeze", "Fresh breeze", "Strong breeze", "Near gale", "Gale", "Strong gale", "Storm",
        "Violent storm", "Hurricane force",
    ];
    NAMES[force.min(12) as usize]
}

/// The phases of the moon, eight to a cycle.
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
    pub fn name(self) -> &'static str {
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
}

/// The moon at a moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moon {
    /// days since the last new moon
    pub age_days: f64,
    /// the lit share of the disc, 0 to 1
    pub illumination: f64,
    pub phase: MoonPhase,
}

/// The mean length of the lunar cycle in days.
const SYNODIC_DAYS: f64 = 29.530_588_853;
/// A new moon: 2000-01-06 18:14 UTC, in Unix seconds.
const NEW_MOON_EPOCH: i64 = 947_182_440;

/// The moon at Unix time `unix_secs`, from the mean lunar cycle.
pub fn moon(unix_secs: i64) -> Moon {
    let age_days = ((unix_secs - NEW_MOON_EPOCH) as f64 / 86_400.0).rem_euclid(SYNODIC_DAYS);
    let cycle = age_days / SYNODIC_DAYS;
    let illumination = (1.0 - (std::f64::consts::TAU * cycle).cos()) / 2.0;
    // eight bins, each centred on its phase: the new moon spans the turn of the cycle
    let phase = match ((cycle * 8.0) + 0.5).floor() as u32 % 8 {
        0 => MoonPhase::New,
        1 => MoonPhase::WaxingCrescent,
        2 => MoonPhase::FirstQuarter,
        3 => MoonPhase::WaxingGibbous,
        4 => MoonPhase::Full,
        5 => MoonPhase::WaningGibbous,
        6 => MoonPhase::LastQuarter,
        _ => MoonPhase::WaningCrescent,
    };
    Moon { age_days, illumination, phase }
}

#[cfg(test)]
mod tests {
    use super::*;
    use springboard::shell::music::radio::Fetch;
    use std::io;

    const FORECAST: &str = r#"{"current":{"time":"2026-10-04T13:20","temperature_2m":7.4,"apparent_temperature":4.1,"relative_humidity_2m":81,"weather_code":61,"wind_speed_10m":22.5,"wind_direction_10m":315,"wind_gusts_10m":41.0,"dew_point_2m":4.2,"pressure_msl":1008.4,"is_day":1},
"hourly":{"time":["2026-10-04T12:00","2026-10-04T13:00","2026-10-04T18:00"],"temperature_2m":[6.0,7.0,8.0],"weather_code":[3,61,2],"is_day":[1,1,0]},
"daily":{"time":["2026-10-04","2026-10-05"],"weather_code":[61,0],"temperature_2m_max":[8.2,10.0],"temperature_2m_min":[3.0,2.5],"sunrise":["2026-10-04T07:41","2026-10-05T07:44"],"sunset":["2026-10-04T18:20","2026-10-05T18:17"]},
"minutely_15":{"time":["2026-10-04T13:00","2026-10-04T13:15","2026-10-04T13:30","2026-10-04T13:45"],"precipitation":[0.0,0.0,0.4,0.0]}}"#;

    /// Answers the forecast request and fails every other one (the air quality and the mean high are optional).
    struct Network;

    impl Fetch for Network {
        fn get(&self, url: &str, _limit: usize) -> io::Result<Vec<u8>> {
            if url.starts_with(springboard::shell::weather::data::FORECAST_API) {
                Ok(FORECAST.as_bytes().to_vec())
            } else {
                Err(io::Error::other("offline"))
            }
        }
    }

    fn place(name: &str, lat: f64, lon: f64) -> Location {
        Location { name: name.into(), region: "Iceland".into(), lat, lon }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bifrost-weather-service-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn compass_points_follow_the_sixteen_wind_rose() {
        for (deg, want) in [(0.0, "N"), (11.0, "N"), (12.0, "NNE"), (45.0, "NE"), (90.0, "E"), (180.0, "S"), (270.0, "W"), (315.0, "NW"), (348.0, "NNW"), (349.0, "N"), (360.0, "N"), (-45.0, "NW"), (725.0, "N")] {
            assert_eq!(compass_label(deg), want, "{deg}");
        }
        assert_eq!(compass_label(f64::NAN), "");
    }

    #[test]
    fn beaufort_forces_follow_the_speed_bands() {
        for (kmh, force) in [(0.0, 0), (0.9, 0), (1.0, 1), (5.9, 1), (6.0, 2), (19.9, 3), (20.0, 4), (28.0, 4), (29.0, 5), (49.9, 6), (50.0, 7), (74.9, 8), (75.0, 9), (102.0, 10), (103.0, 11), (118.0, 12), (300.0, 12)] {
            assert_eq!(beaufort(kmh), force, "{kmh}");
        }
        assert_eq!(beaufort(-3.0), 0);
        assert_eq!(beaufort(f64::NAN), 0);
        assert_eq!(beaufort_label(0), "Calm");
        assert_eq!(beaufort_label(7), "Near gale");
        assert_eq!(beaufort_label(40), "Hurricane force");
    }

    #[test]
    fn the_moon_is_new_on_the_epoch_and_full_half_a_cycle_later() {
        let new = moon(NEW_MOON_EPOCH);
        assert!(new.age_days.abs() < 1e-9 && new.illumination < 1e-9);
        assert_eq!(new.phase, MoonPhase::New);
        // the full moon of 2000-01-21 04:40 UTC
        let full = moon(948_429_600);
        assert!((full.age_days - 14.43).abs() < 0.05, "{}", full.age_days);
        assert!(full.illumination > 0.98, "{}", full.illumination);
        assert_eq!(full.phase, MoonPhase::Full);
        assert_eq!(full.phase.name(), "Full Moon");
        // a week after new is the first quarter, half lit; a week after full is the last quarter
        let first = moon(NEW_MOON_EPOCH + (7.4 * 86_400.0) as i64);
        assert_eq!(first.phase, MoonPhase::FirstQuarter);
        assert!((first.illumination - 0.5).abs() < 0.05);
        let last = moon(NEW_MOON_EPOCH + (22.1 * 86_400.0) as i64);
        assert_eq!(last.phase, MoonPhase::LastQuarter);
        // dates before the epoch still land inside one cycle
        let before = moon(NEW_MOON_EPOCH - 86_400);
        assert!(before.age_days > 28.0 && before.age_days < SYNODIC_DAYS);
        assert_eq!(before.phase, MoonPhase::New, "the day before a new moon is new");
    }

    #[test]
    fn the_service_serves_a_saved_places_cached_forecast() {
        let dir = scratch("serve");
        let store = Store::new(&dir);
        let reykjavik = place("Reykjavik", 64.1355, -21.8954);
        store.save_locations(&[reykjavik.clone()]);
        store.fetch_forecast(&reykjavik, &Network).expect("cached");
        let service = WeatherService::new(&dir);
        assert_eq!(service.places(), vec![reykjavik.clone()]);
        let report = service.weather(&reykjavik).expect("weather");
        assert_eq!(report.condition(), Sky::from_code(61));
        let wind = report.wind();
        assert_eq!((wind.compass, wind.beaufort, wind.beaufort_text), ("NW", 4, "Moderate breeze"));
        assert_eq!(wind.gust_kmh, Some(41.0));
        assert!(!report.is_stale());
        assert_eq!(report.hourly().len(), 2);
        assert_eq!(report.daily()[1].max, 10.0);
        assert_eq!(report.daylight(), Some(("07:41", "18:20")));
        assert_eq!(report.minutes_until_precipitation(), Some(15), "steps start at the quarter hour now falls in (13:15), and the 0.4 mm step is the next one");
        assert!(report.air().is_none(), "the air quality host was unreachable");
        assert_eq!(report.attribution().provider, "Open-Meteo.com");
        assert_eq!(report.attribution().license, "CC BY 4.0");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_saved_places_are_served_and_a_place_without_a_forecast_says_so() {
        let dir = scratch("refuse");
        let service = WeatherService::new(&dir);
        let oslo = place("Oslo", 59.91, 10.75);
        assert!(service.places().is_empty(), "no place until the person saves one");
        assert_eq!(service.weather(&oslo), Err(Error::NotSaved));
        Store::new(&dir).save_locations(&[oslo.clone()]);
        assert_eq!(service.weather(&oslo), Err(Error::NoData), "saved, but nothing fetched yet");
        assert_eq!(service.weather(&place("Bergen", 60.39, 5.32)), Err(Error::NotSaved));
        assert_eq!(Error::NoData.to_string(), "there is no forecast for the place yet");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_report_with_no_rain_ahead_or_a_late_forecast_reads_plainly() {
        let mut f = springboard::shell::weather::data::parse_forecast(FORECAST).expect("forecast");
        f.minutes = vec![0.0, 0.05, 0.0];
        f.days[0].sunrise.clear();
        let w = Weather { place: place("X", 1.0, 1.0), forecast: f, age_secs: FRESH_S + 1 };
        assert_eq!(w.minutes_until_precipitation(), None);
        assert!(w.is_stale());
        assert_eq!(w.daylight(), None, "a missing sunrise means no daylight pair");
        let mut calm = w.clone();
        calm.forecast.current.wind_dir = None;
        calm.forecast.current.wind_kmh = 0.0;
        assert_eq!((calm.wind().compass, calm.wind().beaufort_text), ("", "Calm"));
    }
}
