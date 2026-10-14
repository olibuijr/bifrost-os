//! Parity app-weather: headless tests of the Air quality model behind the Weather app's card. The main pollutant, the
//! advice for each band and the reading of the pollutant concentrations are checked without a network or a window.

use springboard::shell::weather::data::{air_url, parse_air, Air, Location, Pollutant, AIR_API};

fn location() -> Location {
    Location { name: "Akureyri".into(), region: String::new(), lat: 65.6835, lon: -18.1262 }
}

#[test]
fn the_reading_carries_every_pollutant_the_service_sends() {
    let air = parse_air(
        r#"{"current":{"time":"2026-10-04T13:00","european_aqi":34,"pm2_5":7.8,"pm10":12.5,"ozone":61.0,"nitrogen_dioxide":9.0,"sulphur_dioxide":2.0}}"#,
    )
    .unwrap();
    assert_eq!((air.pm25, air.pm10, air.ozone, air.no2, air.so2), (7.8, 12.5, 61.0, 9.0, 2.0));
}

#[test]
fn a_pollutant_the_answer_leaves_out_reads_as_zero() {
    let air = parse_air(r#"{"current":{"european_aqi":34,"pm2_5":7.8}}"#).unwrap();
    assert_eq!((air.pm10, air.ozone, air.no2, air.so2), (0.0, 0.0, 0.0, 0.0));
}

#[test]
fn a_negative_pollutant_reading_is_clamped_to_zero() {
    let air = parse_air(r#"{"current":{"european_aqi":10,"pm2_5":-1.0,"ozone":-3.0}}"#).unwrap();
    assert_eq!((air.pm25, air.ozone), (0.0, 0.0));
}

#[test]
fn the_air_request_asks_for_each_pollutant() {
    let url = air_url(&location());
    assert!(url.starts_with(AIR_API));
    assert!(url.contains("current=european_aqi,pm2_5,pm10,ozone,nitrogen_dioxide,sulphur_dioxide"), "{url}");
}

#[test]
fn the_main_pollutant_is_the_one_furthest_above_its_good_limit() {
    // Ratios against the Good limits: PM2.5 0.78 (7.8 of 10), PM10 1.5 (30 of 20), ozone 0.8 (40 of 50).
    let air = Air { aqi: 61, pm25: 7.8, pm10: 30.0, ozone: 40.0, no2: 10.0, so2: 5.0 };
    assert_eq!(air.main_pollutant(), (Pollutant::Pm10, 30.0));
}

#[test]
fn ozone_can_be_the_main_pollutant_when_it_is_the_furthest_over_its_limit() {
    // Ozone at 150 is three times its limit of 50; the particles are under their limits.
    let air = Air { aqi: 81, pm25: 4.0, pm10: 8.0, ozone: 150.0, no2: 10.0, so2: 5.0 };
    assert_eq!(air.main_pollutant(), (Pollutant::Ozone, 150.0));
    assert_eq!(Pollutant::Ozone.name(), "Ozone");
}

#[test]
fn nitrogen_dioxide_and_sulphur_dioxide_use_their_own_limits() {
    // NO2 at 80 is twice its limit of 40, which beats SO2 at 150 against its limit of 100 (ratio 1.5).
    let no2 = Air { aqi: 41, no2: 80.0, so2: 150.0, ..Air::default() };
    assert_eq!(no2.main_pollutant(), (Pollutant::NitrogenDioxide, 80.0));
    let so2 = Air { aqi: 41, no2: 30.0, so2: 250.0, ..Air::default() };
    assert_eq!(so2.main_pollutant(), (Pollutant::SulphurDioxide, 250.0));
    assert_eq!(Pollutant::NitrogenDioxide.name(), "Nitrogen dioxide");
    assert_eq!(Pollutant::SulphurDioxide.name(), "Sulphur dioxide");
}

#[test]
fn a_tie_keeps_the_pollutant_listed_first() {
    // PM2.5 at 10 and PM10 at 20 are both exactly at their limits: the ratio is 1.0 for each.
    let air = Air { aqi: 21, pm25: 10.0, pm10: 20.0, ..Air::default() };
    assert_eq!(air.main_pollutant(), (Pollutant::Pm25, 10.0));
}

#[test]
fn with_no_pollution_the_main_pollutant_is_particles_at_zero() {
    assert_eq!(Air::default().main_pollutant(), (Pollutant::Pm25, 0.0));
}

#[test]
fn the_advice_follows_the_band() {
    let advice = |aqi: u16| Air { aqi, ..Air::default() }.advice();
    assert_eq!(advice(0), "Enjoy your usual outdoor activities.");
    assert_eq!(advice(20), "Enjoy your usual outdoor activities.");
    assert_eq!(advice(21), "Unusually sensitive people should consider limiting prolonged outdoor exertion.");
    assert_eq!(advice(41), "Sensitive people should limit prolonged outdoor exertion.");
    assert_eq!(advice(61), "Everyone should limit prolonged outdoor exertion.");
    assert_eq!(advice(81), "Everyone should avoid prolonged outdoor exertion.");
    assert_eq!(advice(101), "Everyone should avoid prolonged outdoor exertion.");
}
