//! The Weather app's data: the fetches for Open-Meteo forecasts, place search
//! and the Stooq watchlist, the saved locations, the C/F setting and the disk
//! cache.
//!
//! The model (forecast and place parsing, units, skies, summaries, quote rows)
//! lives in `bifrost_core::weather`, which this module re-exports so the app
//! and the shell keep their paths. Open-Meteo needs no key and is served over
//! HTTPS; the crate has a TLS client (`deildu::http`), so the `https://` URLs
//! are used. Every answer is kept under the Weather data directory (the raw
//! JSON per location), so the last forecast still shows with no network.
//! Fetching happens on worker threads (see `Weather`).

pub use bifrost_core::weather::*;

use crate::shell::music::radio::Fetch;
use std::io;
use std::path::{Path, PathBuf};

const BODY_LIMIT: usize = 512 * 1024;
const LOCATIONS_FILE: &str = "locations.tsv";
const SETTINGS_FILE: &str = "settings.txt";
const WATCHLIST_FILE: &str = "watchlist.txt";
pub const MAX_LOCATIONS: usize = 12;
pub const MAX_WATCH: usize = 20;

/// The directory the app keeps its files in.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Store { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn write(&self, name: &str, text: &str) {
        let path = self.dir.join(name);
        let _ = std::fs::create_dir_all(&self.dir);
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(tmp, path);
        }
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.join(name)).unwrap_or_default()
    }

    /// The saved places; Reykjavik when there are none yet.
    pub fn load_locations(&self) -> Vec<Location> {
        let list: Vec<Location> = self
            .read(LOCATIONS_FILE)
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.split('\t').collect();
                Some(Location { name: f.first()?.to_string(), region: f.get(1)?.to_string(), lat: f.get(2)?.parse().ok()?, lon: f.get(3)?.parse().ok()? })
            })
            .collect();
        if list.is_empty() {
            vec![Location { name: "Reykjav\u{ed}k".into(), region: "Iceland".into(), lat: 64.1355, lon: -21.8954 }]
        } else {
            list
        }
    }

    pub fn save_locations(&self, list: &[Location]) {
        let clean = |s: &str| s.replace(['\t', '\n'], " ");
        self.write(LOCATIONS_FILE, &list.iter().map(|l| format!("{}\t{}\t{}\t{}\n", clean(&l.name), clean(&l.region), l.lat, l.lon)).collect::<String>());
    }

    /// (units, index of the selected place)
    pub fn load_settings(&self) -> (Units, usize) {
        let text = self.read(SETTINGS_FILE);
        let get = |k: &str| text.lines().find_map(|l| l.strip_prefix(k)?.strip_prefix('='));
        (Units::from_key(get("units").unwrap_or("C")), get("selected").and_then(|v| v.trim().parse().ok()).unwrap_or(0))
    }

    pub fn save_settings(&self, units: Units, selected: usize) {
        self.write(SETTINGS_FILE, &format!("units={}\nselected={selected}\n", units.key()));
    }

    pub fn load_watchlist(&self) -> Vec<String> {
        let list: Vec<String> = self.read(WATCHLIST_FILE).lines().filter_map(normalise_symbol).collect();
        if list.is_empty() && !self.dir.join(WATCHLIST_FILE).exists() {
            ["aapl.us", "msft.us", "googl.us", "tsla.us"].map(String::from).to_vec()
        } else {
            list
        }
    }

    pub fn save_watchlist(&self, list: &[String]) {
        self.write(WATCHLIST_FILE, &list.iter().map(|s| format!("{s}\n")).collect::<String>());
    }

    pub fn save_summary(&self, s: &Summary) {
        self.write(SUMMARY_FILE, &s.to_line());
    }

    fn forecast_file(loc: &Location) -> String {
        format!("forecast-{}.json", loc.key())
    }

    /// The cached forecast of a place and its age in seconds.
    pub fn load_forecast(&self, loc: &Location) -> Option<(Forecast, u64)> {
        let path = self.dir.join(Self::forecast_file(loc));
        let f = parse_forecast(&std::fs::read_to_string(&path).ok()?)?;
        let age = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map_or(u64::MAX, |d| d.as_secs());
        Some((f, age))
    }

    /// Fetches a place's forecast; a good answer replaces the cached one.
    pub fn fetch_forecast(&self, loc: &Location, net: &dyn Fetch) -> io::Result<Forecast> {
        let body = net.get(&forecast_url(loc), BODY_LIMIT)?;
        let text = String::from_utf8_lossy(&body);
        let f = parse_forecast(&text).ok_or_else(|| io::Error::other("unexpected forecast answer"))?;
        self.write(&Self::forecast_file(loc), &text);
        Ok(f)
    }
}

pub fn search_places(query: &str, net: &dyn Fetch) -> io::Result<Vec<Location>> {
    let body = net.get(&geocode_url(query), BODY_LIMIT)?;
    parse_places(&String::from_utf8_lossy(&body)).ok_or_else(|| io::Error::other("unexpected search answer"))
}

pub fn fetch_quotes(symbols: &[String], net: &dyn Fetch) -> io::Result<Vec<Quote>> {
    let body = net.get(&quote_url(symbols), BODY_LIMIT)?;
    Ok(parse_quotes(&String::from_utf8_lossy(&body)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORECAST: &str = r#"{"current":{"time":"2026-10-04T13:15","temperature_2m":7.4,"apparent_temperature":4.1,"relative_humidity_2m":81,"weather_code":61,"wind_speed_10m":22.5,"is_day":1},
        "hourly":{"time":["2026-10-04T12:00","2026-10-04T13:00","2026-10-04T14:00"],"temperature_2m":[6.0,7.0,8.0],"weather_code":[3,61,2],"is_day":[1,1,0]},
        "daily":{"time":["2026-10-04","2026-10-05"],"weather_code":[61,0],"temperature_2m_max":[8.2,10.0],"temperature_2m_min":[3.0,2.5],"uv_index_max":[2.4,3.0],"sunrise":["2026-10-04T07:41","2026-10-05T07:44"],"sunset":["2026-10-04T18:20","2026-10-05T18:17"]}}"#;

    struct Canned(String);
    impl Fetch for Canned {
        fn get(&self, _: &str, _: usize) -> io::Result<Vec<u8>> {
            Ok(self.0.clone().into_bytes())
        }
    }

    #[test]
    fn store_keeps_places_settings_watchlist_and_forecast() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("target/test-weather-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let st = Store::new(&dir);
        let places = st.load_locations();
        assert_eq!(places.len(), 1, "a default place");
        let more = vec![places[0].clone(), Location { name: "Oslo".into(), region: "Norway".into(), lat: 59.91, lon: 10.75 }];
        st.save_locations(&more);
        assert_eq!(st.load_locations(), more);
        st.save_settings(Units::Fahrenheit, 1);
        assert_eq!(st.load_settings(), (Units::Fahrenheit, 1));
        assert_eq!(st.load_watchlist().len(), 4, "default watchlist");
        st.save_watchlist(&[]);
        assert!(st.load_watchlist().is_empty(), "an emptied list stays empty");
        assert!(st.load_forecast(&more[1]).is_none());
        let f = st.fetch_forecast(&more[1], &Canned(FORECAST.into())).unwrap();
        assert_eq!(st.load_forecast(&more[1]).map(|(c, _)| c), Some(f));
        assert!(st.fetch_forecast(&more[0], &Canned("{}".into())).is_err());
        assert!(st.load_forecast(&more[0]).is_none(), "a bad answer is not cached");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
