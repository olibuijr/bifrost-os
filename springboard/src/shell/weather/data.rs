//! The Weather app's data: Open-Meteo forecasts and place search, the saved
//! locations, the C/F setting, a Stooq watchlist and the disk cache.
//!
//! Open-Meteo needs no key. `api.open-meteo.com/v1/forecast` answers with
//! the current conditions plus hourly and daily arrays; the geocoding host
//! turns a typed name into coordinates. Both are served over HTTPS and the
//! crate has a TLS client (`deildu::http`), so the `https://` URLs are used.
//! Every answer is kept under the Weather data directory (the raw JSON per
//! location), so the last forecast still shows with no network. Two further
//! Open-Meteo hosts fill in the rest: the air-quality host (`european_aqi`,
//! `pm2_5`, asked on every refresh) and the archive host (the daily highs of
//! this date over the last ten years, asked once per place and date), each
//! kept in its own small file next to the forecast. Fetching happens on
//! worker threads (see `Weather`).
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
pub const AIR_API: &str = "https://air-quality-api.open-meteo.com/v1/air-quality";
pub const ARCHIVE_API: &str = "https://archive-api.open-meteo.com/v1/archive";
const BODY_LIMIT: usize = 512 * 1024;
const LOCATIONS_FILE: &str = "locations.tsv";
const SETTINGS_FILE: &str = "settings.txt";
const WATCHLIST_FILE: &str = "watchlist.txt";
pub const SUMMARY_FILE: &str = "summary.txt";
/// The next hours of the summary's place, written beside it for Notification Center's Weather card.
pub const GLANCE_FILE: &str = "glance.txt";
/// How many upcoming hours the glance keeps.
pub const GLANCE_HOURS: usize = 6;
/// A glance older than this is not shown: its hour labels would no longer match the clock.
pub const GLANCE_MAX_AGE_S: u64 = 3 * 3600;
/// How many past years the "average high for this date" looks at.
pub const AVERAGE_YEARS: i32 = 10;
/// The fewest past years that make an average worth showing.
const AVERAGE_MIN_SAMPLES: usize = 3;
/// The minute forecast shown is this many 15-minute steps: the next hour.
pub const MINUTE_STEPS: usize = 4;
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
    pub temp: f64,
    pub feels: f64,
    pub code: u8,
    pub day: bool,
    pub humidity: u8,
    pub wind_kmh: f64,
    /// bearing the wind blows from, degrees clockwise from north (None when the answer has none)
    pub wind_dir: Option<f64>,
    pub wind_gust_kmh: Option<f64>,
    /// dew point, Celsius
    pub dew_point: Option<f64>,
    /// mean sea level pressure, hPa
    pub pressure_hpa: Option<f64>,
}

/// The sun crossing the horizon during an hour.
#[derive(Clone, Debug, PartialEq)]
pub struct SunEvent {
    pub rise: bool,
    /// local "06:12"
    pub time: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hour {
    /// "13" (local hour of the place)
    pub label: String,
    pub temp: f64,
    pub code: u8,
    pub day: bool,
    /// chance of precipitation, percent (0 when the answer has none)
    pub precip_prob: u8,
    /// precipitation in the hour, mm
    pub precip_mm: f64,
    /// visibility, metres
    pub visibility_m: Option<f64>,
    /// mean sea level pressure, hPa
    pub pressure_hpa: Option<f64>,
    /// the sunrise or sunset that falls in this hour
    pub sun: Option<SunEvent>,
}

impl Hour {
    /// An hour with only the basics; the extra measurements are absent.
    pub fn basic(label: &str, temp: f64, code: u8, day: bool) -> Hour {
        Hour { label: label.into(), temp, code, day, precip_prob: 0, precip_mm: 0.0, visibility_m: None, pressure_hpa: None, sun: None }
    }
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
    /// precipitation total of the day, mm
    pub precip_sum: f64,
    /// highest chance of precipitation in the day, percent (0 when the answer has none)
    pub precip_prob: u8,
    /// snowfall total of the day, centimetres (0 when the answer has none)
    pub snow_sum: f64,
}

/// Air quality now, on the European index, with the concentrations the reading is made from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Air {
    pub aqi: u16,
    /// fine particles (PM2.5), micrograms per cubic metre
    pub pm25: f64,
    /// coarse particles (PM10), micrograms per cubic metre
    pub pm10: f64,
    /// ozone, micrograms per cubic metre
    pub ozone: f64,
    /// nitrogen dioxide, micrograms per cubic metre
    pub no2: f64,
    /// sulphur dioxide, micrograms per cubic metre
    pub so2: f64,
}

/// A pollutant that the air quality reading can name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pollutant {
    Pm25,
    Pm10,
    Ozone,
    NitrogenDioxide,
    SulphurDioxide,
}

impl Pollutant {
    /// Every pollutant the reading carries, in the order that breaks a tie between two equal ratios.
    pub const ALL: [Pollutant; 5] = [Pollutant::Pm25, Pollutant::Pm10, Pollutant::Ozone, Pollutant::NitrogenDioxide, Pollutant::SulphurDioxide];

    /// The name shown to the reader.
    pub fn name(self) -> &'static str {
        match self {
            Pollutant::Pm25 => "PM2.5",
            Pollutant::Pm10 => "PM10",
            Pollutant::Ozone => "Ozone",
            Pollutant::NitrogenDioxide => "Nitrogen dioxide",
            Pollutant::SulphurDioxide => "Sulphur dioxide",
        }
    }

    /// The top of the Good band of the European index for this pollutant, micrograms per cubic metre. A concentration
    /// at this limit reads as the edge of good air.
    fn good_limit(self) -> f64 {
        match self {
            Pollutant::Pm25 => 10.0,
            Pollutant::Pm10 => 20.0,
            Pollutant::Ozone => 50.0,
            Pollutant::NitrogenDioxide => 40.0,
            Pollutant::SulphurDioxide => 100.0,
        }
    }
}

impl Air {
    /// The European Air Quality Index band: 0 to 20 Good, then Fair, Moderate, Poor, Very Poor, Extremely Poor.
    pub fn category(&self) -> &'static str {
        match self.aqi {
            0..=20 => "Good",
            21..=40 => "Fair",
            41..=60 => "Moderate",
            61..=80 => "Poor",
            81..=100 => "Very Poor",
            _ => "Extremely Poor",
        }
    }

    /// The advice that goes with the band, in the words a reader can act on.
    pub fn advice(&self) -> &'static str {
        match self.aqi {
            0..=20 => "Enjoy your usual outdoor activities.",
            21..=40 => "Unusually sensitive people should consider limiting prolonged outdoor exertion.",
            41..=60 => "Sensitive people should limit prolonged outdoor exertion.",
            61..=80 => "Everyone should limit prolonged outdoor exertion.",
            _ => "Everyone should avoid prolonged outdoor exertion.",
        }
    }

    /// The concentration of one pollutant, micrograms per cubic metre.
    pub fn concentration(&self, pollutant: Pollutant) -> f64 {
        match pollutant {
            Pollutant::Pm25 => self.pm25,
            Pollutant::Pm10 => self.pm10,
            Pollutant::Ozone => self.ozone,
            Pollutant::NitrogenDioxide => self.no2,
            Pollutant::SulphurDioxide => self.so2,
        }
    }

    /// The main pollutant: the one whose concentration is furthest above its Good limit, with that concentration.
    /// A tie keeps the pollutant listed first in `Pollutant::ALL`.
    pub fn main_pollutant(&self) -> (Pollutant, f64) {
        let ratio = |p: Pollutant| self.concentration(p) / p.good_limit();
        let mut best = Pollutant::Pm25;
        for p in Pollutant::ALL {
            if ratio(p) > ratio(best) {
                best = p;
            }
        }
        (best, self.concentration(best))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Forecast {
    pub current: Current,
    pub hours: Vec<Hour>,
    pub days: Vec<Day>,
    /// the place's local date "2026-10-04" and time "2026-10-04T13:15" of the current conditions
    pub date: String,
    pub time: String,
    /// precipitation in mm for each of the next 15-minute steps (empty when the service has no minute data)
    pub minutes: Vec<f64>,
    /// air quality now, from its own endpoint (None until fetched)
    pub air: Option<Air>,
    /// the mean daily high of this date over the last ten years, Celsius, from the archive endpoint
    pub average_high: Option<f64>,
}

impl Forecast {
    /// The next hours the glance keeps, starting with the current one.
    pub fn glance(&self) -> &[Hour] {
        &self.hours[..self.hours.len().min(GLANCE_HOURS)]
    }

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

/// The first `n` bytes of `s` ("2026-10-04T13:15" -> "2026-10-04" for 10), or all of it when it is shorter.
fn head(s: &str, n: usize) -> &str {
    s.get(..n).unwrap_or(s)
}

/// `time` ("2026-10-04T13:20") rounded down to its 15-minute step ("2026-10-04T13:15").
fn quarter_floor(time: &str) -> Option<String> {
    let minute: u32 = time.get(14..16)?.parse().ok()?;
    Some(format!("{}{:02}", time.get(..14)?, minute / 15 * 15))
}

/// The precipitation of the next 15-minute steps in the `minutely_15` block, starting at the step `now` falls in.
fn parse_minutes(root: &Json, now: &str) -> Vec<f64> {
    let (Some(block), Some(start)) = (root.get("minutely_15"), quarter_floor(now)) else { return vec![] };
    let (Some(times), Some(values)) = (block.get("time").and_then(Json::array), block.get("precipitation").and_then(Json::array)) else { return vec![] };
    let Some(from) = times.iter().position(|t| t.string().is_some_and(|t| head(t, 16) >= start.as_str())) else { return vec![] };
    (from..times.len().min(values.len())).take(MINUTE_STEPS).map(|i| values[i].number().unwrap_or(0.0).max(0.0)).collect()
}

/// The sunrise or sunset that falls in the hour starting at `hour` ("2026-10-04T07:00").
fn sun_in_hour(hour: &str, rises: &[String], sets: &[String]) -> Option<SunEvent> {
    let key = hour.get(..13)?;
    let find = |list: &[String]| list.iter().find(|s| s.get(..13) == Some(key)).and_then(|s| s.get(11..16)).map(str::to_string);
    find(rises).map(|time| SunEvent { rise: true, time }).or_else(|| find(sets).map(|time| SunEvent { rise: false, time }))
}

/// The forecast in an Open-Meteo answer: the next 24 hours from the current
/// one and the days. Air quality and the climate average come from their own
/// endpoints and are left empty here.
pub fn parse_forecast(text: &str) -> Option<Forecast> {
    let root = json::parse(text)?;
    let cur = root.get("current")?;
    let num = |k: &str| cur.get(k).and_then(Json::number);
    let current = Current {
        temp: num("temperature_2m")?,
        feels: num("apparent_temperature").or(num("temperature_2m"))?,
        code: num("weather_code")? as u8,
        day: num("is_day").is_none_or(|d| d != 0.0),
        humidity: num("relative_humidity_2m").unwrap_or(0.0).clamp(0.0, 100.0) as u8,
        wind_kmh: num("wind_speed_10m").unwrap_or(0.0),
        wind_dir: num("wind_direction_10m").map(|d| d.rem_euclid(360.0)),
        wind_gust_kmh: num("wind_gusts_10m"),
        dew_point: num("dew_point_2m"),
        pressure_hpa: num("pressure_msl"),
    };
    let now = cur.get("time").and_then(Json::string).unwrap_or("");
    let hourly = root.get("hourly")?;
    let times = hourly.get("time")?.array()?;
    let (temps, codes) = (numbers(hourly.get("temperature_2m"))?, numbers(hourly.get("weather_code"))?);
    let flags = numbers(hourly.get("is_day")).unwrap_or_default();
    let daily = root.get("daily")?;
    let strings = |k: &str| -> Vec<String> { daily.get(k).and_then(Json::array).map(|a| a.iter().filter_map(|v| v.string().map(str::to_string)).collect()).unwrap_or_default() };
    let (rises, sets) = (strings("sunrise"), strings("sunset"));
    let from = times.iter().position(|t| t.string().is_some_and(|t| t.get(..13).unwrap_or(t) >= now.get(..13).unwrap_or(now))).unwrap_or(0);
    let hourly_at = |k: &str, i: usize| hourly.get(k).and_then(|v| v.array()?.get(i)?.number());
    let hours = (from..times.len().min(temps.len()).min(codes.len()))
        .take(24)
        .filter_map(|i| {
            let t = times[i].string()?;
            Some(Hour {
                label: t.get(11..13)?.to_string(),
                temp: temps[i],
                code: codes[i] as u8,
                day: flags.get(i).is_none_or(|&d| d != 0.0),
                precip_prob: hourly_at("precipitation_probability", i).unwrap_or(0.0).clamp(0.0, 100.0) as u8,
                precip_mm: hourly_at("precipitation", i).unwrap_or(0.0).max(0.0),
                visibility_m: hourly_at("visibility", i).filter(|v| *v >= 0.0),
                pressure_hpa: hourly_at("pressure_msl", i),
                sun: sun_in_hour(t, &rises, &sets),
            })
        })
        .collect();
    let dates = daily.get("time")?.array()?;
    let (dcodes, max, min) = (numbers(daily.get("weather_code"))?, numbers(daily.get("temperature_2m_max"))?, numbers(daily.get("temperature_2m_min"))?);
    let days = (0..dates.len().min(dcodes.len()).min(max.len()).min(min.len()))
        .take(10)
        .filter_map(|i| {
            let clock = |k: &str| daily.get(k).and_then(|v| v.array()?.get(i)?.string()?.get(11..16).map(str::to_string)).unwrap_or_default();
            let daily_at = |k: &str| daily.get(k).and_then(|v| v.array()?.get(i)?.number());
            Some(Day {
                label: weekday_label(dates[i].string()?)?,
                code: dcodes[i] as u8,
                max: max[i],
                min: min[i],
                uv: daily_at("uv_index_max").unwrap_or(0.0),
                sunrise: clock("sunrise"),
                sunset: clock("sunset"),
                precip_sum: daily_at("precipitation_sum").unwrap_or(0.0).max(0.0),
                precip_prob: daily_at("precipitation_probability_max").unwrap_or(0.0).clamp(0.0, 100.0) as u8,
                snow_sum: daily_at("snowfall_sum").unwrap_or(0.0).max(0.0),
            })
        })
        .collect();
    Some(Forecast { current, hours, days, date: head(now, 10).to_string(), time: now.to_string(), minutes: parse_minutes(&root, now), air: None, average_high: None })
}

pub fn forecast_url(loc: &Location) -> String {
    format!(
        "{FORECAST_API}?latitude={:.4}&longitude={:.4}&current=temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m,wind_direction_10m,wind_gusts_10m,dew_point_2m,pressure_msl,is_day&hourly=temperature_2m,weather_code,is_day,precipitation_probability,precipitation,visibility,pressure_msl&daily=weather_code,temperature_2m_max,temperature_2m_min,uv_index_max,sunrise,sunset,precipitation_sum,snowfall_sum,precipitation_probability_max&minutely_15=precipitation&forecast_minutely_15=8&forecast_days=10&timezone=auto",
        loc.lat, loc.lon
    )
}

pub fn air_url(loc: &Location) -> String {
    format!(
        "{AIR_API}?latitude={:.4}&longitude={:.4}&current=european_aqi,pm2_5,pm10,ozone,nitrogen_dioxide,sulphur_dioxide&timezone=auto",
        loc.lat, loc.lon
    )
}

/// The air quality in an air-quality answer. A pollutant the answer leaves out reads as zero.
pub fn parse_air(text: &str) -> Option<Air> {
    let root = json::parse(text)?;
    let cur = root.get("current")?;
    let aqi = cur.get("european_aqi").and_then(Json::number).filter(|v| *v >= 0.0)?;
    let reading = |key: &str| cur.get(key).and_then(Json::number).unwrap_or(0.0).max(0.0);
    Some(Air {
        aqi: aqi.round().min(u16::MAX as f64) as u16,
        pm25: reading("pm2_5"),
        pm10: reading("pm10"),
        ozone: reading("ozone"),
        no2: reading("nitrogen_dioxide"),
        so2: reading("sulphur_dioxide"),
    })
}

/// The first and last day the archive request covers and the month and day ("10-04") of local date "2026-10-04":
/// the same calendar day from `AVERAGE_YEARS` years ago to last year. 29 February is asked as 28 February, which
/// exists in every year.
fn average_window(date: &str) -> Option<(String, String, String)> {
    let year: i32 = date.get(..4)?.parse().ok()?;
    let md = date.get(5..10)?;
    let ask = if md == "02-29" { "02-28" } else { md };
    Some((format!("{}-{ask}", year - AVERAGE_YEARS), format!("{}-{ask}", year - 1), md.to_string()))
}

/// The archive request for the daily highs of the last ten years, ending last year on the same date.
pub fn average_url(loc: &Location, date: &str) -> Option<String> {
    let (start, end, _) = average_window(date)?;
    Some(format!("{ARCHIVE_API}?latitude={:.4}&longitude={:.4}&start_date={start}&end_date={end}&daily=temperature_2m_max&timezone=auto", loc.lat, loc.lon))
}

/// The mean of the daily highs that fall on month and day `md` ("10-04") in an archive answer. None with fewer than
/// three such days, which is too little to call an average.
pub fn parse_average_high(text: &str, md: &str) -> Option<f64> {
    let root = json::parse(text)?;
    let daily = root.get("daily")?;
    let dates = daily.get("time")?.array()?;
    let highs = daily.get("temperature_2m_max")?.array()?;
    let samples: Vec<f64> = dates
        .iter()
        .zip(highs)
        .filter(|(d, _)| d.string().and_then(|d| d.get(5..10)) == Some(md))
        .filter_map(|(_, h)| h.number())
        .filter(|h| h.is_finite())
        .collect();
    (samples.len() >= AVERAGE_MIN_SAMPLES).then(|| samples.iter().sum::<f64>() / samples.len() as f64)
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

fn glance_text(place: &str, hours: &[Hour]) -> String {
    let mut text = format!("{}\n", place.replace(['\t', '\n'], " "));
    for h in hours {
        text.push_str(&format!("{}\t{}\t{}\t{}\n", h.label.replace(['\t', '\n'], " "), h.temp, h.code, h.day as u8));
    }
    text
}

fn parse_glance(text: &str, place: &str) -> Vec<Hour> {
    let mut lines = text.lines();
    if lines.next() != Some(place.replace(['\t', '\n'], " ").as_str()) {
        return Vec::new();
    }
    lines
        .filter_map(|l| {
            let mut f = l.split('\t');
            let label = f.next()?;
            let (temp, code, day) = (f.next()?.parse::<f64>().ok()?, f.next()?.parse::<u8>().ok()?, f.next()?.parse::<u8>().ok()?);
            Some(Hour::basic(label, temp, code, day != 0))
        })
        .take(GLANCE_HOURS)
        .collect()
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

    /// True once the app has saved its place list.
    pub fn has_places(&self) -> bool {
        self.dir.join(LOCATIONS_FILE).is_file()
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

    /// Saves the next `hours` of `place` beside the summary: the place on the first line (so a glance of another
    /// place is never shown under this summary), then one tab-separated hour per line.
    pub fn save_glance(&self, place: &str, hours: &[Hour]) {
        self.write(GLANCE_FILE, &glance_text(place, hours));
    }

    /// The saved glance for `place`: empty when there is none, when it is for another place, or when it is older
    /// than `GLANCE_MAX_AGE_S`.
    pub fn load_glance(&self, place: &str) -> Vec<Hour> {
        let path = self.dir.join(GLANCE_FILE);
        let age = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map_or(u64::MAX, |d| d.as_secs());
        if age > GLANCE_MAX_AGE_S {
            return Vec::new();
        }
        parse_glance(&self.read(GLANCE_FILE), place)
    }

    fn forecast_file(loc: &Location) -> String {
        format!("forecast-{}.json", loc.key())
    }

    fn air_file(loc: &Location) -> String {
        format!("air-{}.json", loc.key())
    }

    fn average_file(loc: &Location) -> String {
        format!("average-{}.txt", loc.key())
    }

    /// The saved mean high ("MM-DD<TAB>celsius") when it is for the month and day of `date`.
    fn load_average(&self, loc: &Location, date: &str) -> Option<f64> {
        let text = self.read(&Self::average_file(loc));
        let (md, value) = text.trim().split_once('\t')?;
        (Some(md) == date.get(5..10)).then(|| value.parse().ok()).flatten()
    }

    /// The cached forecast of a place and its age in seconds, with the cached air quality and climate average.
    pub fn load_forecast(&self, loc: &Location) -> Option<(Forecast, u64)> {
        let path = self.dir.join(Self::forecast_file(loc));
        let mut f = parse_forecast(&std::fs::read_to_string(&path).ok()?)?;
        f.air = parse_air(&self.read(&Self::air_file(loc)));
        f.average_high = self.load_average(loc, &f.date);
        let age = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map_or(u64::MAX, |d| d.as_secs());
        Some((f, age))
    }

    /// Fetches a place's forecast; a good answer replaces the cached one. The air quality and the ten-year average
    /// high come from their own endpoints after it; when one is down, the forecast still returns, with the value
    /// the cache last held.
    pub fn fetch_forecast(&self, loc: &Location, net: &dyn Fetch) -> io::Result<Forecast> {
        let body = net.get(&forecast_url(loc), BODY_LIMIT)?;
        let text = String::from_utf8_lossy(&body);
        let mut f = parse_forecast(&text).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unexpected forecast answer"))?;
        self.write(&Self::forecast_file(loc), &text);
        if let Ok(body) = net.get(&air_url(loc), BODY_LIMIT) {
            let text = String::from_utf8_lossy(&body);
            if parse_air(&text).is_some() {
                self.write(&Self::air_file(loc), &text);
            }
        }
        f.air = parse_air(&self.read(&Self::air_file(loc)));
        // the average of a date does not change within the day, so it is asked once per place and date
        if self.load_average(loc, &f.date).is_none() {
            self.fetch_average(loc, &f.date, net);
        }
        f.average_high = self.load_average(loc, &f.date);
        Ok(f)
    }

    /// Asks the archive for the daily highs of this date over the last ten years and saves their mean.
    fn fetch_average(&self, loc: &Location, date: &str, net: &dyn Fetch) {
        let Some(url) = average_url(loc, date) else { return };
        let Some(md) = date.get(5..10) else { return };
        let Ok(body) = net.get(&url, BODY_LIMIT) else { return };
        if let Some(mean) = parse_average_high(&String::from_utf8_lossy(&body), md) {
            self.write(&Self::average_file(loc), &format!("{md}\t{mean}\n"));
        }
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

/// The summary the weather service keeps current for the Today widget and the lock screen. It needs no app
/// window: it reads the selected place and units the app saved, reuses the cached forecast while it is younger
/// than `max_age_s`, otherwise fetches a new one, and saves the summary for the readers.
pub fn refresh_summary(store: &Store, net: &dyn Fetch, max_age_s: u64) -> io::Result<Summary> {
    // a place is only fetched once the weather app has saved one: the default place is not asked for on its own
    if !store.has_places() {
        return Err(io::Error::other("no place saved yet"));
    }
    let (units, selected) = store.load_settings();
    let places = store.load_locations();
    let loc = places.get(selected).or(places.first()).ok_or_else(|| io::Error::other("no place saved"))?;
    let forecast = match store.load_forecast(loc) {
        Some((f, age)) if age < max_age_s => f,
        _ => store.fetch_forecast(loc, net)?,
    };
    let summary = forecast.summary(&loc.name, units);
    store.save_summary(&summary);
    store.save_glance(&summary.place, forecast.glance());
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORECAST: &str = r#"{"current":{"time":"2026-10-04T13:20","temperature_2m":7.4,"apparent_temperature":4.1,"relative_humidity_2m":81,"weather_code":61,"wind_speed_10m":22.5,"wind_direction_10m":315,"wind_gusts_10m":41.0,"dew_point_2m":4.2,"pressure_msl":1008.4,"is_day":1},
        "hourly":{"time":["2026-10-04T12:00","2026-10-04T13:00","2026-10-04T18:00"],"temperature_2m":[6.0,7.0,8.0],"weather_code":[3,61,2],"is_day":[1,1,0],"precipitation_probability":[20,70,null],"precipitation":[0.1,1.4,0.0],"visibility":[24000,9000,15000],"pressure_msl":[1009.0,1008.4,1004.0]},
        "daily":{"time":["2026-10-04","2026-10-05"],"weather_code":[61,0],"temperature_2m_max":[8.2,10.0],"temperature_2m_min":[3.0,2.5],"uv_index_max":[2.4,3.0],"sunrise":["2026-10-04T07:41","2026-10-05T07:44"],"sunset":["2026-10-04T18:20","2026-10-05T18:17"],"precipitation_sum":[6.5,0.0],"snowfall_sum":[0.0,2.5],"precipitation_probability_max":[85,null]},
        "minutely_15":{"time":["2026-10-04T13:00","2026-10-04T13:15","2026-10-04T13:30","2026-10-04T13:45","2026-10-04T14:00","2026-10-04T14:15"],"precipitation":[0.0,0.4,0.6,0.0,0.0,0.0]}}"#;

    #[test]
    fn parses_current_hours_from_now_and_days() {
        let f = parse_forecast(FORECAST).unwrap();
        assert_eq!((f.current.temp, f.current.humidity, f.current.code), (7.4, 81, 61));
        assert_eq!(f.hours.iter().map(|h| h.label.as_str()).collect::<Vec<_>>(), ["13", "18"], "starts at the current hour");
        assert!(f.hours[0].day && !f.hours[1].day);
        assert_eq!(f.days[0].label, "Sun", "2026-10-04 is a Sunday");
        assert_eq!(f.days[1].label, "Mon");
        assert_eq!(f.days[1].max, 10.0);
        assert_eq!((f.days[0].uv, f.days[0].sunrise.as_str(), f.days[1].sunset.as_str()), (2.4, "07:41", "18:17"));
        assert_eq!((f.date.as_str(), f.time.as_str()), ("2026-10-04", "2026-10-04T13:20"));
        assert!(parse_forecast("{}").is_none() && parse_forecast("nope").is_none());
    }

    #[test]
    fn parses_wind_dew_point_pressure_and_the_hourly_extras() {
        let f = parse_forecast(FORECAST).unwrap();
        assert_eq!((f.current.wind_dir, f.current.wind_gust_kmh), (Some(315.0), Some(41.0)));
        assert_eq!((f.current.dew_point, f.current.pressure_hpa), (Some(4.2), Some(1008.4)));
        let now = &f.hours[0];
        assert_eq!((now.precip_prob, now.precip_mm, now.visibility_m, now.pressure_hpa), (70, 1.4, Some(9000.0), Some(1008.4)));
        assert_eq!(f.hours[1].precip_prob, 0, "a null chance is zero");
        assert_eq!((f.days[0].precip_sum, f.days[0].precip_prob), (6.5, 85));
        assert_eq!((f.days[0].snow_sum, f.days[1].snow_sum), (0.0, 2.5), "snowfall is kept per day");
        assert_eq!(f.days[1].precip_prob, 0);
    }

    #[test]
    fn an_answer_without_the_extras_still_parses() {
        let text = r#"{"current":{"time":"2026-10-04T13:15","temperature_2m":7.0,"weather_code":1},
            "hourly":{"time":["2026-10-04T13:00"],"temperature_2m":[7.0],"weather_code":[1]},
            "daily":{"time":["2026-10-04"],"weather_code":[1],"temperature_2m_max":[8.0],"temperature_2m_min":[2.0]}}"#;
        let f = parse_forecast(text).unwrap();
        assert_eq!((f.current.wind_dir, f.current.pressure_hpa, f.current.dew_point), (None, None, None));
        assert_eq!(f.hours[0], Hour::basic("13", 7.0, 1, true));
        assert!(f.minutes.is_empty() && f.air.is_none() && f.average_high.is_none());
    }

    #[test]
    fn the_sun_event_lands_in_the_hour_it_happens() {
        let f = parse_forecast(FORECAST).unwrap();
        assert_eq!(f.hours[0].sun, None);
        assert_eq!(f.hours[1].sun, Some(SunEvent { rise: false, time: "18:20".into() }), "sunset 18:20 is in the 18:00 hour");
        let rises = ["2026-10-05T07:44".to_string()];
        assert_eq!(sun_in_hour("2026-10-05T07:00", &rises, &[]), Some(SunEvent { rise: true, time: "07:44".into() }));
        assert_eq!(sun_in_hour("2026-10-05T08:00", &rises, &[]), None);
    }

    #[test]
    fn the_minute_forecast_starts_at_the_current_quarter_hour() {
        let f = parse_forecast(FORECAST).unwrap();
        // 13:20 is in the 13:15 step; the next four steps follow
        assert_eq!(f.minutes, vec![0.4, 0.6, 0.0, 0.0]);
        assert_eq!(quarter_floor("2026-10-04T13:59").as_deref(), Some("2026-10-04T13:45"));
        assert_eq!(quarter_floor("2026-10-04T13:00").as_deref(), Some("2026-10-04T13:00"));
        assert_eq!(quarter_floor("bad"), None);
    }

    #[test]
    fn the_forecast_url_asks_for_every_field_the_screen_shows() {
        let url = forecast_url(&Location { name: "A".into(), region: String::new(), lat: 64.1, lon: -21.9 });
        for field in ["wind_direction_10m", "wind_gusts_10m", "dew_point_2m", "pressure_msl", "precipitation_probability", "visibility", "precipitation_sum", "snowfall_sum", "precipitation_probability_max", "minutely_15=precipitation"] {
            assert!(url.contains(field), "{field} in {url}");
        }
    }

    #[test]
    fn air_quality_parses_and_is_banded_on_the_european_index() {
        let air = parse_air(r#"{"current":{"time":"2026-10-04T13:00","european_aqi":34,"pm2_5":7.8}}"#).unwrap();
        assert_eq!((air.aqi, air.pm25, air.category()), (34, 7.8, "Fair"));
        for (aqi, name) in [(0, "Good"), (20, "Good"), (21, "Fair"), (41, "Moderate"), (61, "Poor"), (81, "Very Poor"), (101, "Extremely Poor")] {
            assert_eq!(Air { aqi, ..Air::default() }.category(), name, "{aqi}");
        }
        assert!(parse_air(r#"{"current":{"pm2_5":7.8}}"#).is_none(), "no index, no reading");
        assert!(parse_air("nope").is_none());
        let loc = Location { name: "A".into(), region: String::new(), lat: 64.1, lon: -21.9 };
        assert!(air_url(&loc).starts_with(AIR_API) && air_url(&loc).contains("european_aqi,pm2_5"));
    }

    #[test]
    fn the_average_window_covers_ten_years_ending_last_year() {
        assert_eq!(average_window("2026-10-04"), Some(("2016-10-04".into(), "2025-10-04".into(), "10-04".into())));
        assert_eq!(average_window("2028-02-29"), Some(("2018-02-28".into(), "2027-02-28".into(), "02-29".into())), "29 February is asked as the 28th");
        assert_eq!(average_window("junk"), None);
        let loc = Location { name: "A".into(), region: String::new(), lat: 64.1, lon: -21.9 };
        let url = average_url(&loc, "2026-10-04").unwrap();
        assert!(url.starts_with(ARCHIVE_API) && url.contains("start_date=2016-10-04&end_date=2025-10-04") && url.contains("daily=temperature_2m_max"), "{url}");
    }

    #[test]
    fn the_average_high_is_the_mean_of_the_same_calendar_day() {
        let archive = r#"{"daily":{"time":["2023-10-04","2023-10-05","2024-10-04","2024-10-05","2025-10-04","2025-10-05"],"temperature_2m_max":[6.0,20.0,9.0,20.0,null,20.0]}}"#;
        assert_eq!(parse_average_high(archive, "10-04"), None, "two usable years is too few");
        let more = r#"{"daily":{"time":["2022-10-04","2023-10-04","2024-10-04","2025-10-04","2025-10-05"],"temperature_2m_max":[5.0,6.0,9.0,8.0,30.0]}}"#;
        assert_eq!(parse_average_high(more, "10-04"), Some(7.0));
        assert_eq!(parse_average_high(more, "11-01"), None);
        assert_eq!(parse_average_high("nope", "10-04"), None);
    }

    #[test]
    fn the_store_fills_air_and_average_from_their_endpoints_and_keeps_them() {
        use std::sync::atomic::{AtomicU32, Ordering};
        // a network that answers by endpoint and counts the archive asks
        struct ByUrl(AtomicU32);
        impl Fetch for ByUrl {
            fn get(&self, url: &str, _: usize) -> io::Result<Vec<u8>> {
                let body = if url.starts_with(AIR_API) {
                    r#"{"current":{"european_aqi":55,"pm2_5":12.0}}"#.to_string()
                } else if url.starts_with(ARCHIVE_API) {
                    self.0.fetch_add(1, Ordering::SeqCst);
                    r#"{"daily":{"time":["2022-10-04","2023-10-04","2024-10-04","2025-10-04"],"temperature_2m_max":[5.0,6.0,9.0,8.0]}}"#.to_string()
                } else {
                    FORECAST.to_string()
                };
                Ok(body.into_bytes())
            }
        }
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("target/test-weather-extra-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let st = Store::new(&dir);
        let place = Location { name: "Oslo".into(), region: "Norway".into(), lat: 59.91, lon: 10.75 };
        let net = ByUrl(AtomicU32::new(0));
        let f = st.fetch_forecast(&place, &net).unwrap();
        assert_eq!(f.air.as_ref().map(|a| a.aqi), Some(55));
        assert_eq!(f.average_high, Some(7.0));
        let again = st.fetch_forecast(&place, &net).unwrap();
        assert_eq!(net.0.load(Ordering::SeqCst), 1, "the archive is asked once per place and date");
        assert_eq!(again.average_high, Some(7.0));
        let (cached, _) = st.load_forecast(&place).unwrap();
        assert_eq!((cached.air, cached.average_high), (f.air, f.average_high), "a cold start shows them offline");
        let _ = std::fs::remove_dir_all(&dir);
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

    #[test]
    fn the_service_refreshes_the_summary_for_the_selected_place() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("target/test-weather-service-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let st = Store::new(&dir);
        // before the app has saved a place the service asks for nothing
        assert!(refresh_summary(&st, &Canned("{}".into()), 900).is_err());
        assert!(!st.has_places());
        let oslo = Location { name: "Oslo".into(), region: "Norway".into(), lat: 59.91, lon: 10.75 };
        st.save_locations(&[oslo.clone()]);
        assert!(st.has_places());
        st.save_settings(Units::Celsius, 0);
        // no cache yet: the service fetches, saves the summary and the readers see it
        let fresh = refresh_summary(&st, &Canned(FORECAST.into()), 900).unwrap();
        assert_eq!(fresh.place, "Oslo");
        assert_eq!(Summary::load(&dir), Some(fresh.clone()));
        // a cache younger than the limit is used and the network is not asked
        let cached = refresh_summary(&st, &Canned("{}".into()), 900).unwrap();
        assert_eq!(cached, fresh);
        // a zero age limit always asks the network, and a failed ask keeps the saved summary
        assert!(refresh_summary(&st, &Canned("{}".into()), 0).is_err());
        assert_eq!(Summary::load(&dir), Some(fresh));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_glance_round_trips_for_its_place_only() {
        let dir = std::env::temp_dir().join(format!("bifrost-glance-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let st = Store::new(&dir);
        let f = parse_forecast(FORECAST).unwrap();
        assert_eq!(f.glance().len(), 2, "the hours the forecast has, up to the limit");
        assert!(st.load_glance("Oslo").is_empty(), "nothing saved, nothing shown");
        st.save_glance("Oslo", f.glance());
        let back = st.load_glance("Oslo");
        let basics = |hs: &[Hour]| hs.iter().map(|h| (h.label.clone(), h.temp, h.code, h.day)).collect::<Vec<_>>();
        assert_eq!(basics(&back), basics(f.glance()));
        assert!(back[0].day && !back[1].day);
        assert!(st.load_glance("Bergen").is_empty(), "another place's hours are never shown");
        // a glance older than the limit is dropped
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(GLANCE_MAX_AGE_S + 60);
        std::fs::File::options().write(true).open(dir.join(GLANCE_FILE)).unwrap().set_modified(old).unwrap();
        assert!(st.load_glance("Oslo").is_empty(), "stale hours are not shown");
        // a damaged line is skipped, not invented
        assert_eq!(parse_glance("Oslo\n13\t7\t61\t1\nbad\n14\tx\t3\t1\n", "Oslo").len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refresh_summary_saves_the_glance_beside_the_summary() {
        let dir = std::env::temp_dir().join(format!("bifrost-glance-refresh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let st = Store::new(&dir);
        st.save_locations(&[Location { name: "Oslo".into(), region: "Norway".into(), lat: 59.91, lon: 10.75 }]);
        let sum = refresh_summary(&st, &Canned(FORECAST.into()), 900).unwrap();
        let hours = st.load_glance(&sum.place);
        assert_eq!(hours.iter().map(|h| h.label.as_str()).collect::<Vec<_>>(), ["13", "18"]);
        assert_eq!(hours[0].temp, 7.0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
