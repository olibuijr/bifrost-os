//! The Weather data model: Open-Meteo forecast and place-search answers, the
//! saved places, the Stooq quote rows and the one-line summary the Today
//! widget reads. Everything here is parsing and formatting; the network
//! fetches and the files live in the Weather app's data module.
//!
//! Open-Meteo needs no key. `api.open-meteo.com/v1/forecast` answers with the
//! current conditions plus hourly and daily arrays; the geocoding host turns a
//! typed name into coordinates. Stooq's CSV quote endpoint answers
//! `Symbol,Date,Time,Open,High,Low,Close,Volume`, with `N/D` for symbols it
//! does not know.

use crate::csv;
use crate::json::{self, Json};
use std::path::Path;

pub const FORECAST_API: &str = "https://api.open-meteo.com/v1/forecast";
pub const GEOCODE_API: &str = "https://geocoding-api.open-meteo.com/v1/search";
pub const QUOTE_API: &str = "https://stooq.com/q/l/";
pub const SUMMARY_FILE: &str = "summary.txt";
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

/// A saved place: a name, a region line and a coordinate in decimal degrees.
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
    pub temp: f64,
    pub feels: f64,
    pub code: u8,
    pub day: bool,
    pub humidity: u8,
    pub wind_kmh: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hour {
    /// "13" (local hour of the place)
    pub label: String,
    pub temp: f64,
    pub code: u8,
    pub day: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Day {
    /// "Mon"
    pub label: String,
    pub code: u8,
    pub max: f64,
    pub min: f64,
    /// daily peak UV index (0 when the answer has none)
    pub uv: f64,
    /// local "06:12" (empty when the answer has none)
    pub sunrise: String,
    pub sunset: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Forecast {
    pub current: Current,
    pub hours: Vec<Hour>,
    pub days: Vec<Day>,
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

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn valid_date(year: i64, month: u32, day: u32) -> bool {
    let len = match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return false,
    };
    (1..=len).contains(&day)
}

/// Sunday is 0 (Sakamoto's method). The date must be valid.
fn weekday_index(year: i64, month: u32, day: u32) -> usize {
    const OFFSET: [i64; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    (y + y / 4 - y / 100 + y / 400 + OFFSET[month as usize - 1] + day as i64).rem_euclid(7) as usize
}

/// The three-letter weekday of an ISO "YYYY-MM-DD" date, or `None` when the date is not a real day.
fn weekday_label(date: &str) -> Option<String> {
    let mut p = date.split('-');
    let year = p.next()?.parse::<i64>().ok()?;
    let month = p.next()?.parse::<u32>().ok()?;
    let day = p.next()?.parse::<u32>().ok()?;
    valid_date(year, month, day).then(|| DAYS[weekday_index(year, month, day)].to_string())
}

fn numbers(v: Option<&Json>) -> Option<Vec<f64>> {
    v?.array()?.iter().map(|n| n.number()).collect()
}

/// The forecast in an Open-Meteo answer: the next 24 hours from the current
/// one and the days.
pub fn parse_forecast(text: &str) -> Option<Forecast> {
    let root = json::parse(text).ok()?;
    let cur = root.get("current")?;
    let num = |k: &str| cur.get(k).and_then(Json::number);
    let current = Current {
        temp: num("temperature_2m")?,
        feels: num("apparent_temperature").or(num("temperature_2m"))?,
        code: num("weather_code")? as u8,
        day: num("is_day").is_none_or(|d| d != 0.0),
        humidity: num("relative_humidity_2m").unwrap_or(0.0).clamp(0.0, 100.0) as u8,
        wind_kmh: num("wind_speed_10m").unwrap_or(0.0),
    };
    let now = cur.get("time").and_then(Json::string).unwrap_or("");
    let hourly = root.get("hourly")?;
    let times = hourly.get("time")?.array()?;
    let (temps, codes) = (numbers(hourly.get("temperature_2m"))?, numbers(hourly.get("weather_code"))?);
    let flags = numbers(hourly.get("is_day")).unwrap_or_default();
    let from = times.iter().position(|t| t.string().is_some_and(|t| t.get(..13).unwrap_or(t) >= now.get(..13).unwrap_or(now))).unwrap_or(0);
    let hours = (from..times.len().min(temps.len()).min(codes.len()))
        .take(24)
        .filter_map(|i| {
            let t = times[i].string()?;
            Some(Hour { label: t.get(11..13)?.to_string(), temp: temps[i], code: codes[i] as u8, day: flags.get(i).is_none_or(|&d| d != 0.0) })
        })
        .collect();
    let daily = root.get("daily")?;
    let dates = daily.get("time")?.array()?;
    let (dcodes, max, min) = (numbers(daily.get("weather_code"))?, numbers(daily.get("temperature_2m_max"))?, numbers(daily.get("temperature_2m_min"))?);
    let days = (0..dates.len().min(dcodes.len()).min(max.len()).min(min.len()))
        .take(10)
        .filter_map(|i| {
            let clock = |k: &str| daily.get(k).and_then(|v| v.array()?.get(i)?.string()?.get(11..16).map(str::to_string)).unwrap_or_default();
            let uv = daily.get("uv_index_max").and_then(|v| v.array()?.get(i)?.number()).unwrap_or(0.0);
            Some(Day { label: weekday_label(dates[i].string()?)?, code: dcodes[i] as u8, max: max[i], min: min[i], uv, sunrise: clock("sunrise"), sunset: clock("sunset") })
        })
        .collect();
    Some(Forecast { current, hours, days })
}

pub fn forecast_url(loc: &Location) -> String {
    format!(
        "{FORECAST_API}?latitude={:.4}&longitude={:.4}&current=temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m,is_day&hourly=temperature_2m,weather_code,is_day&daily=weather_code,temperature_2m_max,temperature_2m_min,uv_index_max,sunrise,sunset&forecast_days=10&timezone=auto",
        loc.lat, loc.lon
    )
}

/// application/x-www-form-urlencoded encoding of one value: spaces become `+`, and every byte outside the unreserved
/// set becomes `%XX`.
fn form_value(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn geocode_url(query: &str) -> String {
    format!("{GEOCODE_API}?name={}&count=6&language=en&format=json", form_value(query.trim()))
}

/// The places in a geocoding answer (an empty list when nothing matched).
pub fn parse_places(text: &str) -> Option<Vec<Location>> {
    let root = json::parse(text).ok()?;
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
pub fn parse_quotes(text: &str) -> Vec<Quote> {
    csv::parse_rows(text)
        .iter()
        .skip(1)
        .filter_map(|f| {
            if f.len() < 7 {
                return None;
            }
            Some(Quote { symbol: f[0].to_lowercase(), open: f[3].trim().parse().ok()?, close: f[6].trim().parse().ok()? })
        })
        .collect()
}
