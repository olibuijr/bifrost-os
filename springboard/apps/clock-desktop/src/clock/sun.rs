//! Where the sun is: its declination and the longitude it stands over (the low-precision solar position of the
//! Astronomical Almanac, good to about a hundredth of a degree for 1950 to 2050), the day and night terminator the
//! World Clock map shades, and sunrise and sunset for a city. Pure maths; angles are degrees unless a name says radians.

/// Unix time of the J2000.0 epoch, 2000-01-01 12:00 UTC.
const J2000_UNIX: f64 = 946_728_000.0;
/// The sun's altitude at sunrise and sunset: its radius and the refraction at the horizon.
const HORIZON_DEG: f64 = -0.833;

/// The sun at one instant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sun {
    /// Declination: the latitude the sun stands over.
    pub dec: f64,
    /// The longitude the sun stands over (-180 to 180): local solar noon there.
    pub lon: f64,
    /// The equation of time in minutes (apparent minus mean solar time).
    pub eot_min: f64,
}

fn wrap180(deg: f64) -> f64 {
    (deg + 180.0).rem_euclid(360.0) - 180.0
}

/// The sun at Unix time `unix` (seconds).
pub fn sun_at(unix: f64) -> Sun {
    let n = (unix - J2000_UNIX) / 86_400.0;
    let l = (280.460 + 0.985_647_4 * n).rem_euclid(360.0);
    let g = (357.528 + 0.985_600_3 * n).rem_euclid(360.0).to_radians();
    let lambda = (l + 1.915 * g.sin() + 0.020 * (2.0 * g).sin()).to_radians();
    let eps = (23.439 - 0.000_000_4 * n).to_radians();
    let dec = (eps.sin() * lambda.sin()).asin().to_degrees();
    let ra = (eps.cos() * lambda.sin()).atan2(lambda.cos()).to_degrees();
    let eot_deg = wrap180(l - ra);
    let utc_hours = (unix / 3600.0).rem_euclid(24.0);
    // local apparent time is 12:00 where the sun stands: UTC + equation of time + longitude / 15 = 12 h
    let lon = wrap180(180.0 - 15.0 * utc_hours - eot_deg);
    Sun { dec, lon, eot_min: 4.0 * eot_deg }
}

/// The sun's altitude above the horizon at (`lat`, `lon`).
pub fn altitude(sun: &Sun, lat: f64, lon: f64) -> f64 {
    let (lat, dec, h) = (lat.to_radians(), sun.dec.to_radians(), (lon - sun.lon).to_radians());
    (lat.sin() * dec.sin() + lat.cos() * dec.cos() * h.cos()).asin().to_degrees()
}

/// The latitude of the terminator (sun on the horizon) at longitude `lon`. Near an equinox, where the terminator runs
/// almost along a meridian, the declination is held a hair off zero so the line stays defined.
pub fn terminator_lat(sun: &Sun, lon: f64) -> f64 {
    let dec = if sun.dec.abs() < 1e-3 { 1e-3f64.copysign(if sun.dec == 0.0 { 1.0 } else { sun.dec }) } else { sun.dec };
    let h = (lon - sun.lon).to_radians();
    (-h.cos() / dec.to_radians().tan()).atan().to_degrees()
}

/// True when the night side lies south of the terminator (northern summer): the south pole is in the dark.
pub fn night_is_south(sun: &Sun) -> bool {
    sun.dec >= 0.0
}

/// A city's day: sunrise and sunset, or the sun staying up or down all day.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Daylight {
    /// Unix times of sunrise and sunset.
    Times(i64, i64),
    MidnightSun,
    PolarNight,
}

/// Sunrise and sunset on the civil day that holds `unix` at (`lat`, `lon`), for a city `offset` seconds ahead of UTC.
/// The sun is taken at that day's solar noon, which is within a minute or two of the exact times.
pub fn daylight(unix: i64, offset: i32, lat: f64, lon: f64) -> Daylight {
    let local_midnight = (unix + offset as i64).div_euclid(86_400) * 86_400 - offset as i64;
    // solar noon of that day: 12:00 at the longitude, corrected by the equation of time
    let mut noon = local_midnight as f64 + offset as f64 + (12.0 * 3600.0) - lon / 15.0 * 3600.0;
    // the local day can start a day off the UTC day at the date line; keep the noon inside it
    while noon < local_midnight as f64 {
        noon += 86_400.0;
    }
    while noon >= local_midnight as f64 + 86_400.0 {
        noon -= 86_400.0;
    }
    let sun = sun_at(noon);
    let noon = noon - sun.eot_min * 60.0;
    let (phi, dec) = (lat.to_radians(), sun.dec.to_radians());
    let cos_h = (HORIZON_DEG.to_radians().sin() - phi.sin() * dec.sin()) / (phi.cos() * dec.cos());
    if cos_h <= -1.0 {
        return Daylight::MidnightSun;
    }
    if cos_h >= 1.0 {
        return Daylight::PolarNight;
    }
    let half = cos_h.acos().to_degrees() / 15.0 * 3600.0;
    Daylight::Times((noon - half).round() as i64, (noon + half).round() as i64)
}

/// Latitude and longitude of a World Clock city (`tz::CITIES` names).
pub fn coords(name: &str) -> Option<(f64, f64)> {
    COORDS.iter().find(|(n, _, _)| *n == name).map(|&(_, lat, lon)| (lat, lon))
}

const COORDS: &[(&str, f64, f64)] = &[
    ("Reykjavik", 64.15, -21.94),
    ("London", 51.51, -0.13),
    ("Paris", 48.86, 2.35),
    ("Moscow", 55.76, 37.62),
    ("Dubai", 25.20, 55.27),
    ("New York", 40.71, -74.01),
    ("San Francisco", 37.77, -122.42),
    ("Tokyo", 35.68, 139.69),
    ("Sydney", -33.87, 151.21),
    ("Lisbon", 38.72, -9.14),
    ("Dublin", 53.35, -6.26),
    ("Berlin", 52.52, 13.40),
    ("Madrid", 40.42, -3.70),
    ("Rome", 41.90, 12.50),
    ("Amsterdam", 52.37, 4.90),
    ("Brussels", 50.85, 4.35),
    ("Vienna", 48.21, 16.37),
    ("Zurich", 47.38, 8.54),
    ("Stockholm", 59.33, 18.07),
    ("Oslo", 59.91, 10.75),
    ("Copenhagen", 55.68, 12.57),
    ("Warsaw", 52.23, 21.01),
    ("Prague", 50.08, 14.44),
    ("Helsinki", 60.17, 24.94),
    ("Athens", 37.98, 23.73),
    ("Johannesburg", -26.20, 28.05),
    ("Istanbul", 41.01, 28.98),
    ("Nairobi", -1.29, 36.82),
    ("Mumbai", 19.08, 72.88),
    ("Bangkok", 13.76, 100.50),
    ("Singapore", 1.35, 103.82),
    ("Hong Kong", 22.32, 114.17),
    ("Beijing", 39.90, 116.41),
    ("Perth", -31.95, 115.86),
    ("Seoul", 37.57, 126.98),
    ("Honolulu", 21.31, -157.86),
    ("Anchorage", 61.22, -149.90),
    ("Los Angeles", 34.05, -118.24),
    ("Denver", 39.74, -104.99),
    ("Chicago", 41.88, -87.63),
    ("Mexico City", 19.43, -99.13),
    ("Toronto", 43.65, -79.38),
    ("Buenos Aires", -34.60, -58.38),
    ("Sao Paulo", -23.55, -46.63),
    ("Cairo", 30.04, 31.24),
    ("Lagos", 6.52, 3.38),
    ("Casablanca", 33.57, -7.59),
    ("Cape Town", -33.92, 18.42),
    ("Tehran", 35.69, 51.39),
    ("Karachi", 24.86, 67.01),
    ("Delhi", 28.61, 77.21),
    ("Dhaka", 23.81, 90.41),
    ("Jakarta", -6.21, 106.85),
    ("Manila", 14.60, 120.98),
    ("Taipei", 25.03, 121.57),
    ("Auckland", -36.85, 174.76),
    ("Melbourne", -37.81, 144.96),
    ("Brisbane", -27.47, 153.03),
    ("Vancouver", 49.28, -123.12),
    ("Miami", 25.76, -80.19),
    ("Bogota", 4.71, -74.07),
    ("Lima", -12.05, -77.04),
    ("Santiago", -33.45, -70.67),
    ("Kyiv", 50.45, 30.52),
    ("Budapest", 47.50, 19.04),
    ("Riga", 56.95, 24.11),
];

#[cfg(test)]
mod tests {
    use super::*;
    use springboard::shell::clock::tz::{days_from_civil, CITIES};

    fn unix(y: i64, m: u32, d: u32, h: i64, min: i64) -> f64 {
        (days_from_civil(y, m, d) * 86_400 + h * 3600 + min * 60) as f64
    }

    #[test]
    fn every_city_has_coordinates() {
        for city in CITIES {
            let (lat, lon) = coords(city.name).unwrap_or_else(|| panic!("{} has no coordinates", city.name));
            assert!((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon), "{}", city.name);
        }
    }

    #[test]
    fn declination_follows_the_solstices_and_equinoxes() {
        assert!((sun_at(unix(2026, 6, 21, 12, 0)).dec - 23.44).abs() < 0.1);
        assert!((sun_at(unix(2026, 12, 21, 12, 0)).dec + 23.44).abs() < 0.1);
        assert!(sun_at(unix(2026, 3, 20, 15, 0)).dec.abs() < 0.3);
        assert!(sun_at(unix(2026, 9, 23, 0, 0)).dec.abs() < 0.3);
    }

    #[test]
    fn the_sun_stands_over_greenwich_near_noon_utc() {
        // early November the sun runs about 16 minutes fast: solar noon at Greenwich is near 11:44 UTC
        let s = sun_at(unix(2026, 11, 3, 11, 44));
        assert!(s.lon.abs() < 0.5, "{}", s.lon);
        assert!((s.eot_min - 16.4).abs() < 0.5, "{}", s.eot_min);
        // twelve hours later it is over the date line
        assert!(sun_at(unix(2026, 11, 3, 23, 44)).lon.abs() > 179.0);
        // an hour later it has moved 15 degrees west
        let later = sun_at(unix(2026, 11, 3, 12, 44));
        assert!((later.lon - (s.lon - 15.0)).abs() < 0.1);
    }

    #[test]
    fn the_terminator_is_where_the_sun_is_on_the_horizon() {
        for &(y, m, d) in &[(2026, 6, 21), (2026, 10, 25), (2026, 12, 21), (2027, 3, 20)] {
            let sun = sun_at(unix(y, m, d, 7, 30));
            for lon in (-180..=180).step_by(15) {
                let lat = terminator_lat(&sun, lon as f64);
                assert!(altitude(&sun, lat, lon as f64).abs() < 0.05, "{y}-{m}-{d} lon {lon}");
            }
            // the subsolar point is day, the far side of the night pole is night
            assert!(altitude(&sun, sun.dec, sun.lon) > 89.9);
            let pole = if night_is_south(&sun) { -89.0 } else { 89.0 };
            assert!(altitude(&sun, pole, sun.lon + 180.0) < 0.0);
        }
    }

    #[test]
    fn the_terminator_is_defined_at_the_equinox() {
        let sun = Sun { dec: 0.0, lon: 0.0, eot_min: 0.0 };
        let lat = terminator_lat(&sun, 45.0);
        assert!(lat.is_finite() && lat.abs() > 89.0);
        assert!(terminator_lat(&sun, 90.0).abs() < 1.0); // the meridian 90 degrees from the sun
    }

    #[test]
    fn sunrise_and_sunset_match_published_times() {
        let hm = |t: i64, offset: i32| {
            let s = (t + offset as i64).rem_euclid(86_400);
            (s / 3600, s / 60 % 60)
        };
        let near = |(h, m): (i64, i64), (eh, em): (i64, i64)| ((h * 60 + m) - (eh * 60 + em)).abs() <= 3;
        // London, 21 June 2026 (BST): sunrise 04:43, sunset 21:21
        let Daylight::Times(rise, set) = daylight(unix(2026, 6, 21, 12, 0) as i64, 3600, 51.51, -0.13) else { panic!() };
        assert!(near(hm(rise, 3600), (4, 43)), "{:?}", hm(rise, 3600));
        assert!(near(hm(set, 3600), (21, 21)), "{:?}", hm(set, 3600));
        // Sydney, 21 December 2026 (AEDT): sunrise 05:41, sunset 20:05
        let Daylight::Times(rise, set) = daylight(unix(2026, 12, 21, 2, 0) as i64, 39_600, -33.87, 151.21) else { panic!() };
        assert!(near(hm(rise, 39_600), (5, 41)), "{:?}", hm(rise, 39_600));
        assert!(near(hm(set, 39_600), (20, 5)), "{:?}", hm(set, 39_600));
        assert!(rise < set);
    }

    #[test]
    fn the_poles_have_midnight_sun_and_polar_night() {
        assert_eq!(daylight(unix(2026, 6, 21, 12, 0) as i64, 0, 78.2, 15.6), Daylight::MidnightSun);
        assert_eq!(daylight(unix(2026, 12, 21, 12, 0) as i64, 0, 78.2, 15.6), Daylight::PolarNight);
    }

    #[test]
    fn sunrise_falls_on_the_local_day_across_the_date_line() {
        // Auckland at 23:00 local on 1 March is 10:00 UTC the same day; its sunrise is that morning, local time
        let offset = 13 * 3600;
        let t = unix(2026, 3, 1, 10, 0) as i64;
        let Daylight::Times(rise, set) = daylight(t, offset, -36.85, 174.76) else { panic!() };
        let day = |x: i64| (x + offset as i64).div_euclid(86_400);
        assert_eq!(day(rise), day(t));
        assert_eq!(day(set), day(t));
    }
}
