//! The Weather app's data: Open-Meteo forecasts and place search, the saved
//! locations, the C/F setting, a Stooq watchlist and the disk cache.
//!
//! Open-Meteo needs no key. `api.open-meteo.com/v1/forecast` answers with
//! the current conditions plus hourly and daily arrays; the geocoding host
//! turns a typed name into coordinates. Both are served over HTTPS and the
//! crate has a TLS client (`deildu::http`), so the `https://` URLs are used.
//! Every answer is kept under the Weather data directory (the raw JSON per
//! location), so the last forecast still shows with no network. Fetching
//! happens on worker threads (see `Weather`).
//!
//! Stocks-lite uses Stooq's CSV quote endpoint: no key, one request for the
//! whole watchlist, answers `Symbol,Date,Time,Open,High,Low,Close,Volume`.
//! It has no previous close, so the change shown is close against the day's
//! open. When it fails the watchlist still lists the symbols.

use crate::deildu::http::form_encode;
use crate::shell::calendar::Date;
use crate::shell::music::radio::json::{self, Json};
use crate::shell::music::radio::Fetch;
use std::io;
use std::path::{Path, PathBuf};

pub const FORECAST_API: &str = "https://api.open-meteo.com/v1/forecast";
pub const GEOCODE_API: &str = "https://geocoding-api.open-meteo.com/v1/search";
pub const QUOTE_API: &str = "https://stooq.com/q/l/";
const BODY_LIMIT: usize = 512 * 1024;
const LOCATIONS_FILE: &str = "locations.tsv";
const SETTINGS_FILE: &str = "settings.txt";
const WATCHLIST_FILE: &str = "watchlist.txt";
pub const SUMMARY_FILE: &str = "summary.txt";
pub const MAX_LOCATIONS: usize = 12;
pub const MAX_WATCH: usize = 20;
const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// The sky a WMO weather code stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sky {
    Clear,
    PartlyCloudy,
    Cloudy,
    Fog,
    Drizzle,
    Rain,
    Snow,
    Thunder,
}

impl Sky {
    pub fn from_code(code: u8) -> Sky {
        match code {
            0 | 1 => Sky::Clear,
            2 => Sky::PartlyCloudy,
            3 => Sky::Cloudy,
            45 | 48 => Sky::Fog,
            51..=57 => Sky::Drizzle,
            61..=67 | 80..=82 => Sky::Rain,
            71..=77 | 85 | 86 => Sky::Snow,
            95..=99 => Sky::Thunder,
            _ => Sky::Cloudy,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Sky::Clear => "Clear",
            Sky::PartlyCloudy => "Partly Cloudy",
            Sky::Cloudy => "Cloudy",
            Sky::Fog => "Fog",
            Sky::Drizzle => "Drizzle",
            Sky::Rain => "Rain",
            Sky::Snow => "Snow",
            Sky::Thunder => "Thunderstorm",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Units {
    #[default]
    Celsius,
    Fahrenheit,
}

impl Units {
    /// `celsius` in these units, rounded.
    pub fn convert(self, celsius: f64) -> i32 {
        match self {
            Units::Celsius => celsius.round() as i32,
            Units::Fahrenheit => (celsius * 9.0 / 5.0 + 32.0).round() as i32,
        }
    }

    /// "12°"
    pub fn degrees(self, celsius: f64) -> String {
        format!("{}\u{b0}", self.convert(celsius))
    }

    pub fn key(self) -> char {
        match self {
            Units::Celsius => 'C',
            Units::Fahrenheit => 'F',
        }
    }

    pub fn from_key(c: &str) -> Units {
        if c.trim().eq_ignore_ascii_case("F") {
            Units::Fahrenheit
        } else {
            Units::Celsius
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Location {
    pub name: String,
    pub region: String,
    pub lat: f64,
    pub lon: f64,
}

impl Location {
    /// The cache key of this place's forecast.
    pub fn key(&self) -> String {
        format!("{:.2}_{:.2}", self.lat, self.lon)
    }

    /// "Akureyri, Iceland" style second line.
    pub fn subtitle(&self) -> &str {
        &self.region
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Current {
    /// local "2026-10-14T13:15", the time of the current conditions
    pub time: String,
    pub temp: f64,
    pub feels: f64,
    pub code: u8,
    pub day: bool,
    pub humidity: u8,
    pub wind_kmh: f64,
    /// the direction the wind comes from, in degrees clockwise from north (None when the answer has none)
    pub wind_dir: Option<f64>,
    /// the strongest gust of the last hour, km/h
    pub gust_kmh: Option<f64>,
    /// sea-level pressure in hPa
    pub pressure_hpa: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hour {
    /// "13" (local hour of the place)
    pub label: String,
    /// local "2026-10-14T13:00", the same format as the answer's time strings (sorts by time)
    pub time: String,
    pub temp: f64,
    pub code: u8,
    pub day: bool,
    /// chance of precipitation in percent (0 when the answer has none)
    pub chance: u8,
    /// precipitation in the hour, mm
    pub precip_mm: f64,
    /// visibility in metres (None when the answer has none)
    pub visibility_m: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Day {
    /// "Mon"
    pub label: String,
    /// local "2026-10-14"
    pub date: String,
    pub code: u8,
    pub max: f64,
    pub min: f64,
    /// daily peak UV index (0 when the answer has none)
    pub uv: f64,
    /// local "06:12" (empty when the answer has none)
    pub sunrise: String,
    pub sunset: String,
    /// highest chance of precipitation in percent (0 when the answer has none)
    pub chance: u8,
    /// precipitation for the whole day, mm
    pub precip_mm: f64,
}

/// One quarter-hour of the minute forecast: the local start "2026-10-14T13:15" and the precipitation in it, mm.
#[derive(Clone, Debug, PartialEq)]
pub struct Quarter {
    pub time: String,
    pub mm: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Forecast {
    pub current: Current,
    pub hours: Vec<Hour>,
    pub days: Vec<Day>,
    /// the next hour in quarter-hour steps (empty when the answer has none)
    pub quarters: Vec<Quarter>,
    /// the pressure change over the last three hours, hPa (None without hourly pressure)
    pub pressure_trend_hpa: Option<f64>,
}

impl Forecast {
    pub fn summary(&self, place: &str, units: Units) -> Summary {
        let today = self.days.first();
        Summary {
            place: place.to_string(),
            temp: self.current.temp,
            code: self.current.code,
            day: self.current.day,
            max: today.map_or(self.current.temp, |d| d.max),
            min: today.map_or(self.current.temp, |d| d.min),
            units,
        }
    }
}

fn weekday_label(date: &str) -> Option<String> {
    let mut p = date.split('-');
    let d = Date { year: p.next()?.parse().ok()?, month: p.next()?.parse().ok()?, day: p.next()?.parse().ok()? };
    d.valid().then(|| DAYS[d.weekday() % 7].to_string())
}

fn numbers(v: Option<&Json>) -> Option<Vec<f64>> {
    v?.array()?.iter().map(|n| n.number()).collect()
}

/// A whole-number chance from an answer's percentage, 0 to 100 (0 when absent).
fn percent(v: Option<&f64>) -> u8 {
    v.map_or(0, |c| c.round().clamp(0.0, 100.0) as u8)
}

/// The forecast in an Open-Meteo answer: the next 24 hours from the current one, the days, the next hour in
/// quarter-hour steps and the pressure trend. Fields added after a cached answer was saved are optional, so an old
/// cache still parses.
pub fn parse_forecast(text: &str) -> Option<Forecast> {
    let root = json::parse(text)?;
    let cur = root.get("current")?;
    let num = |k: &str| cur.get(k).and_then(Json::number);
    let now = cur.get("time").and_then(Json::string).unwrap_or("");
    let current = Current {
        time: now.to_string(),
        temp: num("temperature_2m")?,
        feels: num("apparent_temperature").or(num("temperature_2m"))?,
        code: num("weather_code")? as u8,
        day: num("is_day").is_none_or(|d| d != 0.0),
        humidity: num("relative_humidity_2m").unwrap_or(0.0).clamp(0.0, 100.0) as u8,
        wind_kmh: num("wind_speed_10m").unwrap_or(0.0),
        wind_dir: num("wind_direction_10m"),
        gust_kmh: num("wind_gusts_10m"),
        pressure_hpa: num("pressure_msl"),
    };
    let hourly = root.get("hourly")?;
    let times = hourly.get("time")?.array()?;
    let (temps, codes) = (numbers(hourly.get("temperature_2m"))?, numbers(hourly.get("weather_code"))?);
    let flags = numbers(hourly.get("is_day")).unwrap_or_default();
    let chances = numbers(hourly.get("precipitation_probability")).unwrap_or_default();
    let rain = numbers(hourly.get("precipitation")).unwrap_or_default();
    let visible = numbers(hourly.get("visibility")).unwrap_or_default();
    let pressure = numbers(hourly.get("pressure_msl")).unwrap_or_default();
    let from = times.iter().position(|t| t.string().is_some_and(|t| t.get(..13).unwrap_or(t) >= now.get(..13).unwrap_or(now))).unwrap_or(0);
    let hours = (from..times.len().min(temps.len()).min(codes.len()))
        .take(24)
        .filter_map(|i| {
            let t = times[i].string()?;
            Some(Hour {
                label: t.get(11..13)?.to_string(),
                time: t.to_string(),
                temp: temps[i],
                code: codes[i] as u8,
                day: flags.get(i).is_none_or(|&d| d != 0.0),
                chance: percent(chances.get(i)),
                precip_mm: rain.get(i).copied().unwrap_or(0.0),
                visibility_m: visible.get(i).copied(),
            })
        })
        .collect();
    let pressure_trend_hpa = pressure.get(from).zip(pressure.get(from.saturating_sub(3))).map(|(now_hpa, before)| now_hpa - before);
    let daily = root.get("daily")?;
    let dates = daily.get("time")?.array()?;
    let (dcodes, max, min) = (numbers(daily.get("weather_code"))?, numbers(daily.get("temperature_2m_max"))?, numbers(daily.get("temperature_2m_min"))?);
    let day_chances = numbers(daily.get("precipitation_probability_max")).unwrap_or_default();
    let day_sums = numbers(daily.get("precipitation_sum")).unwrap_or_default();
    let days = (0..dates.len().min(dcodes.len()).min(max.len()).min(min.len()))
        .take(10)
        .filter_map(|i| {
            let date = dates[i].string()?;
            let clock = |k: &str| daily.get(k).and_then(|v| v.array()?.get(i)?.string()?.get(11..16).map(str::to_string)).unwrap_or_default();
            let uv = daily.get("uv_index_max").and_then(|v| v.array()?.get(i)?.number()).unwrap_or(0.0);
            Some(Day {
                label: weekday_label(date)?,
                date: date.to_string(),
                code: dcodes[i] as u8,
                max: max[i],
                min: min[i],
                uv,
                sunrise: clock("sunrise"),
                sunset: clock("sunset"),
                chance: percent(day_chances.get(i)),
                precip_mm: day_sums.get(i).copied().unwrap_or(0.0),
            })
        })
        .collect();
    let quarters = match root.get("minutely_15") {
        Some(m) => {
            let times = m.get("time").and_then(Json::array).unwrap_or(&[]);
            let mm = numbers(m.get("precipitation")).unwrap_or_default();
            times.iter().zip(mm).filter_map(|(t, mm)| Some(Quarter { time: t.string()?.to_string(), mm })).collect()
        }
        None => Vec::new(),
    };
    Some(Forecast { current, hours, days, quarters, pressure_trend_hpa })
}

pub fn forecast_url(loc: &Location) -> String {
    format!(
        "{FORECAST_API}?latitude={:.4}&longitude={:.4}&current=temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m,wind_direction_10m,wind_gusts_10m,pressure_msl,is_day&hourly=temperature_2m,weather_code,is_day,precipitation_probability,precipitation,visibility,pressure_msl&daily=weather_code,temperature_2m_max,temperature_2m_min,uv_index_max,sunrise,sunset,precipitation_probability_max,precipitation_sum&minutely_15=precipitation&forecast_days=10&timezone=auto",
        loc.lat, loc.lon
    )
}

pub fn geocode_url(query: &str) -> String {
    format!("{GEOCODE_API}?{}&count=6&language=en&format=json", form_encode(&[("name", query.trim())]))
}

/// The places in a geocoding answer (an empty list when nothing matched).
pub fn parse_places(text: &str) -> Option<Vec<Location>> {
    let root = json::parse(text)?;
    let Some(results) = root.get("results").and_then(Json::array) else { return root.get("generationtime_ms").map(|_| vec![]) };
    Some(
        results
            .iter()
            .filter_map(|r| {
                let name = r.get("name")?.string()?.to_string();
                let part = |k: &str| r.get(k).and_then(Json::string).filter(|s| !s.is_empty() && *s != name);
                let region = [part("admin1"), r.get("country").and_then(Json::string)].into_iter().flatten().collect::<Vec<_>>().join(", ");
                Some(Location { name, region, lat: r.get("latitude")?.number()?, lon: r.get("longitude")?.number()? })
            })
            .collect(),
    )
}

/// What the Today widget and the lock screen show of the first place.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub place: String,
    pub temp: f64,
    pub code: u8,
    pub day: bool,
    pub max: f64,
    pub min: f64,
    pub units: Units,
}

impl Summary {
    pub fn to_line(&self) -> String {
        format!("{}\t{}\t{}\t{}\t{}\t{}\t{}", self.place.replace(['\t', '\n'], " "), self.temp, self.code, self.day as u8, self.max, self.min, self.units.key())
    }

    pub fn parse(line: &str) -> Option<Summary> {
        let mut f = line.trim_end().split('\t');
        let place = f.next()?.to_string();
        let mut n = || f.next()?.parse::<f64>().ok();
        let (temp, code, day, max, min) = (n()?, n()?, n()?, n()?, n()?);
        Some(Summary { place, temp, code: code as u8, day: day != 0.0, max, min, units: Units::from_key(f.next().unwrap_or("C")) })
    }

    pub fn sky(&self) -> Sky {
        Sky::from_code(self.code)
    }

    /// "Reykjavik 7° Cloudy" for a lock screen slot.
    pub fn line(&self) -> String {
        format!("{} {} {}", self.place, self.units.degrees(self.temp), self.sky().label())
    }

    /// The summary the app last saved in `dir`.
    pub fn load(dir: &Path) -> Option<Summary> {
        Summary::parse(&std::fs::read_to_string(dir.join(SUMMARY_FILE)).ok()?)
    }
}

/// One watchlist row as the quote endpoint reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct Quote {
    /// as typed on the watchlist, e.g. "aapl.us"
    pub symbol: String,
    pub open: f64,
    pub close: f64,
}

impl Quote {
    pub fn change_pct(&self) -> f64 {
        if self.open == 0.0 { 0.0 } else { (self.close - self.open) / self.open * 100.0 }
    }
}

/// "AAPL" -> "aapl.us"; symbols with a market suffix keep it.
pub fn normalise_symbol(raw: &str) -> Option<String> {
    let s: String = raw.trim().to_lowercase();
    if s.is_empty() || s.len() > 12 || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '^' || c == '-') {
        return None;
    }
    Some(if s.contains('.') || s.starts_with('^') { s } else { format!("{s}.us") })
}

/// "aapl.us" -> "AAPL"
pub fn display_symbol(symbol: &str) -> String {
    symbol.strip_suffix(".us").unwrap_or(symbol).to_uppercase()
}

pub fn quote_url(symbols: &[String]) -> String {
    format!("{QUOTE_API}?s={}&f=sd2t2ohlcv&h&e=csv", symbols.join("+"))
}

/// The rows of a quote CSV that carry prices (`N/D` rows are skipped).
pub fn parse_quotes(csv: &str) -> Vec<Quote> {
    csv.lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.trim().split(',').collect();
            if f.len() < 7 {
                return None;
            }
            Some(Quote { symbol: f[0].to_lowercase(), open: f[3].parse().ok()?, close: f[6].parse().ok()? })
        })
        .collect()
}

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

    #[test]
    fn parses_current_hours_from_now_and_days() {
        let f = parse_forecast(FORECAST).unwrap();
        assert_eq!((f.current.temp, f.current.humidity, f.current.code), (7.4, 81, 61));
        assert_eq!(f.hours.iter().map(|h| h.label.as_str()).collect::<Vec<_>>(), ["13", "14"], "starts at the current hour");
        assert!(f.hours[0].day && !f.hours[1].day);
        assert_eq!(f.days[0].label, "Sun", "2026-10-04 is a Sunday");
        assert_eq!(f.days[1].label, "Mon");
        assert_eq!(f.days[1].max, 10.0);
        assert_eq!((f.days[0].uv, f.days[0].sunrise.as_str(), f.days[1].sunset.as_str()), (2.4, "07:41", "18:17"));
        assert!(parse_forecast("{}").is_none() && parse_forecast("nope").is_none());
    }

    #[test]
    fn units_convert_and_round() {
        assert_eq!(Units::Celsius.degrees(7.4), "7\u{b0}");
        assert_eq!(Units::Fahrenheit.convert(0.0), 32);
        assert_eq!(Units::Fahrenheit.convert(-40.0), -40);
        assert_eq!(Units::from_key("f"), Units::Fahrenheit);
        assert_eq!(Units::from_key("x"), Units::Celsius);
    }

    #[test]
    fn codes_map_to_skies() {
        assert_eq!(Sky::from_code(0), Sky::Clear);
        assert_eq!(Sky::from_code(63), Sky::Rain);
        assert_eq!(Sky::from_code(81), Sky::Rain);
        assert_eq!(Sky::from_code(73), Sky::Snow);
        assert_eq!(Sky::from_code(96), Sky::Thunder);
        assert_eq!(Sky::from_code(250), Sky::Cloudy, "unknown codes are cloud");
    }

    #[test]
    fn places_parse_and_empty_answers_are_not_errors() {
        let text = r#"{"results":[{"name":"Akureyri","admin1":"Northeast","country":"Iceland","latitude":65.68,"longitude":-18.1},{"name":"Bad"}],"generationtime_ms":0.5}"#;
        let p = parse_places(text).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].region, "Northeast, Iceland");
        assert_eq!(parse_places(r#"{"generationtime_ms":0.3}"#), Some(vec![]));
        assert!(parse_places("<html>").is_none());
        assert!(geocode_url("S\u{e3}o Paulo").contains("name=S%"));
    }

    #[test]
    fn summary_round_trips() {
        let s = parse_forecast(FORECAST).unwrap().summary("Reykjav\u{ed}k", Units::Fahrenheit);
        assert_eq!(Summary::parse(&s.to_line()), Some(s.clone()));
        assert_eq!(s.line(), "Reykjav\u{ed}k 45\u{b0} Rain");
        assert!(Summary::parse("junk").is_none());
    }

    #[test]
    fn symbols_normalise_and_quotes_parse() {
        assert_eq!(normalise_symbol(" AAPL "), Some("aapl.us".into()));
        assert_eq!(normalise_symbol("vod.uk"), Some("vod.uk".into()));
        assert_eq!(normalise_symbol("a b"), None);
        assert_eq!(normalise_symbol(""), None);
        assert_eq!(display_symbol("aapl.us"), "AAPL");
        assert_eq!(quote_url(&["a.us".into(), "b.us".into()]), "https://stooq.com/q/l/?s=a.us+b.us&f=sd2t2ohlcv&h&e=csv");
        let q = parse_quotes("Symbol,Date,Time,Open,High,Low,Close,Volume\nAAPL.US,2026-10-02,22:00:09,100.0,111,99,110.0,5\nXYZ.US,N/D,N/D,N/D,N/D,N/D,N/D,N/D\n");
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].symbol, "aapl.us");
        assert!((q[0].change_pct() - 10.0).abs() < 1e-9);
    }

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
