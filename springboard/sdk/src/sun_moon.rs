//! Sun and moon events (WeatherKit parity): `SunEvents` and `MoonEvents` (phase is `weather_service::MoonPhase`), computed offline.
//!
//! The sun follows the NOAA solar position algorithm (mean elements, equation of centre, equation of time). Dawn
//! and dusk use depression angles of 18, 12 and 6 degrees (astronomical, nautical, civil) and 0.833 degrees for
//! sunrise and sunset (refraction plus the solar radius). An event that never happens on the day (polar day or
//! night, or no twilight end in a short summer night) is `None`. The moon is a model: the mean synodic month from
//! a known new moon, so phases can be off by a few hours and rise and set times by about an hour.
//!
//! The day is the local mean solar day of the given longitude: it starts at solar midnight, so no time zone is
//! needed and every instant is a `coding::Date` (seconds since 1970). `solar_midnight` is the one that starts
//! the day, so it lies twelve hours before `solar_noon`.
//!
//! ```ignore
//! let ev = SunEvents::new(64.15, -21.94, 2026, 6, 21);
//! assert!(ev.astronomical_dusk.is_none()); // no astronomical night in Reykjavik at midsummer
//! ```

use crate::calendar_system::days_from_civil;
use crate::coding::Date;
use crate::weather_service::{moon, MoonPhase, SYNODIC_DAYS as SYNODIC_MONTH_DAYS};

/// Mean interval between two moon transits: a synodic month longer than the 24 h day by 1/29.53 of a day.
const LUNAR_DAY_SECONDS: f64 = 86_400.0 * (1.0 + 1.0 / SYNODIC_MONTH_DAYS);
/// Moonrise and moonset altitude: refraction, semi-diameter and parallax combined.
const MOON_HORIZON_DEG: f64 = 0.125;

/// Julian day of a Unix time.
fn jd_of(unix: f64) -> f64 {
    unix / 86_400.0 + 2_440_587.5
}

/// Solar declination (degrees), equation of time (minutes) and apparent ecliptic longitude (degrees) at a
/// Julian day.
fn solar_position(jd: f64) -> (f64, f64, f64) {
    let t = (jd - 2_451_545.0) / 36_525.0;
    let l0 = (280.46646 + t * (36_000.76983 + t * 0.0003032)).rem_euclid(360.0);
    let m = 357.52911 + t * (35_999.05029 - 0.0001537 * t);
    let e = 0.016_708_634 - t * (0.000_042_037 + 0.000_000_1267 * t);
    let mr = m.to_radians();
    let c = mr.sin() * (1.914602 - t * (0.004817 + 0.000014 * t))
        + (2.0 * mr).sin() * (0.019993 - 0.000101 * t)
        + (3.0 * mr).sin() * 0.000289;
    let omega = (125.04 - 1934.136 * t).to_radians();
    let lambda = l0 + c - 0.00569 - 0.00478 * omega.sin();
    let eps0 = 23.0 + (26.0 + (21.448 - t * (46.815 + t * (0.00059 - t * 0.001813))) / 60.0) / 60.0;
    let eps = (eps0 + 0.00256 * omega.cos()).to_radians();
    let decl = (eps.sin() * lambda.to_radians().sin()).asin().to_degrees();
    let y = (eps / 2.0).tan().powi(2);
    let l0r = l0.to_radians();
    let eq = 4.0
        * (y * (2.0 * l0r).sin() - 2.0 * e * mr.sin() + 4.0 * e * y * mr.sin() * (2.0 * l0r).cos()
            - 0.5 * y * y * (4.0 * l0r).sin()
            - 1.25 * e * e * (2.0 * mr).sin())
        .to_degrees();
    (decl, eq, lambda)
}

/// Hour angle in degrees at which the sun is `depression` degrees below the horizon, or `None` when it never
/// gets there (or never leaves that depth) on a day with this declination.
fn hour_angle(lat: f64, decl: f64, depression: f64) -> Option<f64> {
    let z = (90.0 + depression).to_radians();
    let (la, de) = (lat.to_radians(), decl.to_radians());
    let c = (z.cos() - la.sin() * de.sin()) / (la.cos() * de.cos());
    (c.abs() <= 1.0).then(|| c.acos().to_degrees())
}

/// Midnight UTC of a calendar day, in Unix seconds.
fn day_start_utc(year: i32, month: u32, day: u32) -> f64 {
    days_from_civil(i64::from(year), i64::from(month), i64::from(day)) as f64 * 86_400.0
}

/// `SunEvents`: the sun's events for one day at one place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunEvents {
    pub astronomical_dawn: Option<Date>,
    pub nautical_dawn: Option<Date>,
    pub civil_dawn: Option<Date>,
    pub sunrise: Option<Date>,
    pub solar_noon: Date,
    pub sunset: Option<Date>,
    pub civil_dusk: Option<Date>,
    pub nautical_dusk: Option<Date>,
    pub astronomical_dusk: Option<Date>,
    /// Solar midnight at the start of the day (twelve hours before `solar_noon`).
    pub solar_midnight: Date,
}

impl SunEvents {
    /// Events for the calendar day `year-month-day` at latitude and longitude in degrees (east positive).
    pub fn new(latitude: f64, longitude: f64, year: i32, month: u32, day: u32) -> SunEvents {
        let base = day_start_utc(year, month, day);
        // Solar noon: refine the equation of time at the noon instant itself.
        let mut noon = base + 43_200.0 - longitude * 240.0;
        for _ in 0..3 {
            let (_, eq, _) = solar_position(jd_of(noon));
            noon = base + 43_200.0 - longitude * 240.0 - eq * 60.0;
        }
        let event = |depression: f64, sign: f64| -> Option<Date> {
            let (decl, _, _) = solar_position(jd_of(noon));
            let mut h = hour_angle(latitude, decl, depression)?;
            for _ in 0..3 {
                let t = noon + sign * h * 240.0;
                let (decl, _, _) = solar_position(jd_of(t));
                h = hour_angle(latitude, decl, depression)?;
            }
            Some(Date(noon + sign * h * 240.0))
        };
        SunEvents {
            astronomical_dawn: event(18.0, -1.0),
            nautical_dawn: event(12.0, -1.0),
            civil_dawn: event(6.0, -1.0),
            sunrise: event(0.833, -1.0),
            solar_noon: Date(noon),
            sunset: event(0.833, 1.0),
            civil_dusk: event(6.0, 1.0),
            nautical_dusk: event(12.0, 1.0),
            astronomical_dusk: event(18.0, 1.0),
            solar_midnight: Date(noon - 43_200.0),
        }
    }
}

/// `MoonEvents`: the moon's phase at solar noon and its rise and set within the day (`None` when none falls
/// inside it, which happens about once a month, or when the moon stays above or below the horizon).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoonEvents {
    pub phase: MoonPhase,
    pub moonrise: Option<Date>,
    pub moonset: Option<Date>,
}

impl MoonEvents {
    pub fn new(latitude: f64, longitude: f64, year: i32, month: u32, day: u32) -> MoonEvents {
        let sun = SunEvents::new(latitude, longitude, year, month, day);
        let start = sun.solar_midnight.0;
        let noon = sun.solar_noon.0;
        let mn = moon(sun.solar_noon.0 as i64);
        let age = mn.age_days;
        // The moon transits at local solar noon at new moon and at midnight at full moon.
        let transit = noon + age / SYNODIC_MONTH_DAYS * 86_400.0;
        // Declination from the moon's ecliptic longitude (the 5 degree orbital tilt is ignored).
        let (_, _, sun_lambda) = solar_position(jd_of(noon));
        let moon_lambda = (sun_lambda + age / SYNODIC_MONTH_DAYS * 360.0).to_radians();
        let eps = 23.44_f64.to_radians();
        let decl = (eps.sin() * moon_lambda.sin()).asin().to_degrees();
        let h = hour_angle(latitude, decl, -MOON_HORIZON_DEG);
        let within = |t: f64| (t >= start && t < start + 86_400.0).then_some(Date(t));
        let pick = |sign: f64| -> Option<Date> {
            let h = h?;
            (-2..=1).find_map(|k| within(transit + f64::from(k) * LUNAR_DAY_SECONDS + sign * h * 240.0))
        };
        MoonEvents { phase: mn.phase, moonrise: pick(-1.0), moonset: pick(1.0) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REYKJAVIK: (f64, f64) = (64.15, -21.94);

    fn minutes(a: Date, b: Date) -> f64 {
        (a.0 - b.0) / 60.0
    }

    /// Unix seconds of a UTC clock time on a calendar day (hours may exceed 24 for the next day).
    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> Date {
        Date(day_start_utc(y, m, d) + f64::from(h) * 3600.0 + f64::from(min) * 60.0)
    }

    #[test]
    fn reykjavik_midsummer_has_the_published_sunrise_and_sunset_and_no_astronomical_dusk() {
        let ev = SunEvents::new(REYKJAVIK.0, REYKJAVIK.1, 2026, 6, 21);
        // Published: sunrise 02:55, sunset 00:04 the next morning (Iceland keeps UTC).
        assert!(minutes(ev.sunrise.unwrap(), at(2026, 6, 21, 2, 55)).abs() <= 3.0);
        assert!(minutes(ev.sunset.unwrap(), at(2026, 6, 21, 24, 4)).abs() <= 3.0);
        assert!(ev.astronomical_dusk.is_none());
        assert!(ev.astronomical_dawn.is_none());
        assert!(ev.nautical_dusk.is_none());
    }

    #[test]
    fn reykjavik_midwinter_has_a_short_day() {
        let ev = SunEvents::new(REYKJAVIK.0, REYKJAVIK.1, 2026, 12, 21);
        // Published: sunrise 11:22, sunset 15:30.
        assert!(minutes(ev.sunrise.unwrap(), at(2026, 12, 21, 11, 22)).abs() <= 3.0);
        assert!(minutes(ev.sunset.unwrap(), at(2026, 12, 21, 15, 30)).abs() <= 3.0);
        let day = minutes(ev.sunset.unwrap(), ev.sunrise.unwrap());
        assert!((day - 248.0).abs() <= 3.0, "{day}");
        // Twilights exist and are ordered.
        let (a, n, c) = (ev.astronomical_dawn.unwrap(), ev.nautical_dawn.unwrap(), ev.civil_dawn.unwrap());
        assert!(a.0 < n.0 && n.0 < c.0 && c.0 < ev.sunrise.unwrap().0);
    }

    #[test]
    fn the_equator_at_an_equinox_has_a_day_of_about_twelve_hours() {
        let ev = SunEvents::new(0.0, 0.0, 2026, 3, 20);
        // Refraction and the solar disc lengthen it by about 7 minutes, so the bound is 8, not 5.
        let day = minutes(ev.sunset.unwrap(), ev.sunrise.unwrap());
        assert!((day - 720.0).abs() <= 8.0, "{day}");
        assert!(minutes(ev.solar_noon, at(2026, 3, 20, 12, 0)).abs() <= 10.0);
        assert!((minutes(ev.solar_noon, ev.solar_midnight) - 720.0).abs() < 1e-6);
    }

    #[test]
    fn above_the_arctic_circle_the_midsummer_sun_does_not_set() {
        let ev = SunEvents::new(70.0, 0.0, 2026, 6, 21);
        assert!(ev.sunrise.is_none() && ev.sunset.is_none());
        let winter = SunEvents::new(70.0, 0.0, 2026, 12, 21);
        assert!(winter.sunrise.is_none() && winter.sunset.is_none());
    }

    #[test]
    fn the_moon_phase_matches_published_2026_new_and_full_moons() {
        // Each date is an eclipse, so the phase is certain: annular solar eclipse (new moon) 2026-02-17,
        // total lunar eclipse (full moon) 2026-03-03, total solar eclipse (new moon) 2026-08-12.
        assert_eq!(moon(at(2026, 2, 17, 12, 0).0 as i64).phase, MoonPhase::New);
        assert_eq!(moon(at(2026, 3, 3, 12, 0).0 as i64).phase, MoonPhase::Full);
        assert_eq!(moon(at(2026, 8, 12, 18, 0).0 as i64).phase, MoonPhase::New);
        // A week after a new moon the moon is at first quarter; a week after a full moon, last quarter.
        assert_eq!(moon(at(2026, 2, 24, 12, 0).0 as i64).phase, MoonPhase::FirstQuarter);
        assert_eq!(moon(at(2026, 3, 10, 12, 0).0 as i64).phase, MoonPhase::LastQuarter);
    }

    #[test]
    fn a_full_moon_rises_near_sunset_and_sets_near_sunrise() {
        let sun = SunEvents::new(0.0, 0.0, 2026, 3, 3);
        let moon = MoonEvents::new(0.0, 0.0, 2026, 3, 3);
        assert_eq!(moon.phase, MoonPhase::Full);
        assert!(minutes(moon.moonrise.unwrap(), sun.sunset.unwrap()).abs() <= 120.0);
        assert!(minutes(moon.moonset.unwrap(), sun.sunrise.unwrap()).abs() <= 120.0);
    }

    #[test]
    fn moon_events_lie_within_the_day() {
        for d in 1..=28 {
            let ev = MoonEvents::new(REYKJAVIK.0, REYKJAVIK.1, 2026, 4, d);
            let start = SunEvents::new(REYKJAVIK.0, REYKJAVIK.1, 2026, 4, d).solar_midnight.0;
            for t in [ev.moonrise, ev.moonset].into_iter().flatten() {
                assert!(t.0 >= start && t.0 < start + 86_400.0);
            }
        }
    }
}
