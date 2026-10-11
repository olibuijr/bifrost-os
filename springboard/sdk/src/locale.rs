//! Foundation locale parity: `Locale`, `NumberFormatter`, `DateFormatter`, `RelativeDateTimeFormatter`,
//! `ByteCountFormatter`, `MeasurementFormatter` basics, `ListFormatter`, and `NativeLocalizedString`
//! (`localized!`) over `Localizable.strings` tables. Built-in data for `en` and `is`; any other
//! language falls back to `en`. Pure functions, no I/O except `Bundle::load`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Is,
}

/// `Locale`: a language plus an optional region (`en_US`, `is_IS`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Locale {
    pub language: String,
    pub region: Option<String>,
}

impl Locale {
    pub fn new(identifier: &str) -> Self {
        let id = identifier.split(['.', '@']).next().unwrap_or("");
        let mut it = id.split(['_', '-']);
        let language = it.next().unwrap_or("").to_ascii_lowercase();
        let region = it.next().filter(|r| !r.is_empty()).map(|r| r.to_ascii_uppercase());
        if language.is_empty() || language == "c" || language == "posix" {
            return Locale { language: "en".into(), region: Some("US".into()) };
        }
        Locale { language, region }
    }

    /// `Locale.current`: `LC_ALL`, `LC_MESSAGES`, `LANG` in that order; the shell's region preference
    /// (`BIFROST_REGION`) replaces the region.
    pub fn current() -> Self {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let id = get("LC_ALL").or_else(|| get("LC_MESSAGES")).or_else(|| get("LANG")).unwrap_or_default();
        Self::from_env(&id, get("BIFROST_REGION").as_deref())
    }

    pub fn from_env(id: &str, region_pref: Option<&str>) -> Self {
        let mut l = Locale::new(id);
        if let Some(r) = region_pref.filter(|r| !r.is_empty()) {
            l.region = Some(r.to_ascii_uppercase());
        }
        l
    }

    pub fn identifier(&self) -> String {
        match &self.region {
            Some(r) => format!("{}_{}", self.language, r),
            None => self.language.clone(),
        }
    }

    pub fn lang(&self) -> Lang {
        if self.language == "is" { Lang::Is } else { Lang::En }
    }

    pub fn decimal_separator(&self) -> char {
        if self.lang() == Lang::Is { ',' } else { '.' }
    }

    pub fn grouping_separator(&self) -> char {
        if self.lang() == Lang::Is { '.' } else { ',' }
    }

    /// `usesMetricSystem`: false only for the US.
    pub fn uses_metric_system(&self) -> bool {
        self.region.as_deref() != Some("US")
    }

    /// Default currency for the region.
    pub fn currency_code(&self) -> &'static str {
        match self.region.as_deref() {
            Some("IS") => "ISK",
            Some("GB") => "GBP",
            Some("DE" | "FR" | "ES" | "IT" | "NL" | "FI" | "IE" | "AT") => "EUR",
            _ => if self.lang() == Lang::Is { "ISK" } else { "USD" },
        }
    }

    fn uses_24h(&self) -> bool {
        self.lang() == Lang::Is || !matches!(self.region.as_deref(), Some("US" | "CA" | "AU") | None)
    }
}

impl Default for Locale {
    fn default() -> Self {
        Locale::current()
    }
}

// ---------------------------------------------------------------- NumberFormatter

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberStyle {
    Decimal,
    Percent,
    Currency,
}

#[derive(Clone, Debug)]
pub struct NumberFormatter {
    pub locale: Locale,
    pub style: NumberStyle,
    pub uses_grouping: bool,
    pub min_fraction: usize,
    pub max_fraction: usize,
    pub currency_code: Option<String>,
}

impl NumberFormatter {
    pub fn new(locale: Locale, style: NumberStyle) -> Self {
        let (min_fraction, max_fraction) = match style {
            NumberStyle::Decimal => (0, 3),
            NumberStyle::Percent => (0, 0),
            NumberStyle::Currency => (2, 2),
        };
        NumberFormatter { locale, style, uses_grouping: true, min_fraction, max_fraction, currency_code: None }
    }

    pub fn string(&self, v: f64) -> String {
        let code = self.currency_code.clone().unwrap_or_else(|| self.locale.currency_code().into());
        let (min, max) = if self.style == NumberStyle::Currency && matches!(code.as_str(), "ISK" | "JPY") && self.max_fraction == 2 {
            (0, 0)
        } else {
            (self.min_fraction, self.max_fraction)
        };
        let v = if self.style == NumberStyle::Percent { v * 100.0 } else { v };
        let mut s = format!("{:.*}", max, v.abs());
        let neg = v < 0.0 && s.chars().any(|c| c.is_ascii_digit() && c != '0');
        if let Some(dot) = s.find('.') {
            let keep = dot + 1 + min;
            while s.len() > keep && s.ends_with('0') {
                s.pop();
            }
            if s.ends_with('.') {
                s.pop();
            }
        }
        let (int, frac) = s.split_once('.').unwrap_or((s.as_str(), ""));
        let mut body = String::new();
        for (i, c) in int.chars().enumerate() {
            if self.uses_grouping && i > 0 && (int.len() - i) % 3 == 0 {
                body.push(self.locale.grouping_separator());
            }
            body.push(c);
        }
        if !frac.is_empty() {
            body.push(self.locale.decimal_separator());
            body.push_str(frac);
        }
        let is = self.locale.lang() == Lang::Is;
        let sign = if neg { "-" } else { "" };
        match self.style {
            NumberStyle::Decimal => format!("{sign}{body}"),
            NumberStyle::Percent => if is { format!("{sign}{body}\u{a0}%") } else { format!("{sign}{body}%") },
            NumberStyle::Currency => {
                let sym = match code.as_str() {
                    "USD" => "$",
                    "EUR" => "\u{20ac}",
                    "GBP" => "\u{a3}",
                    "ISK" => "kr.",
                    "JPY" => "\u{a5}",
                    other => other,
                };
                if is { format!("{sign}{body}\u{a0}{sym}") } else { format!("{sign}{sym}{body}") }
            }
        }
    }
}

// ---------------------------------------------------------------- DateFormatter

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateStyle {
    None,
    Short,
    Medium,
    Long,
    Full,
}

const MONTHS_EN: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const MONTHS_IS: [&str; 12] = ["janúar", "febrúar", "mars", "apríl", "maí", "júní", "júlí", "ágúst", "september", "október", "nóvember", "desember"];
const SHORT_IS: [&str; 12] = ["jan.", "feb.", "mar.", "apr.", "maí", "jún.", "júl.", "ágú.", "sep.", "okt.", "nóv.", "des."];
const DAYS_EN: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const DAYS_IS: [&str; 7] = ["sunnudagur", "mánudagur", "þriðjudagur", "miðvikudagur", "fimmtudagur", "föstudagur", "laugardagur"];
const SHORTDAY_IS: [&str; 7] = ["sun.", "mán.", "þri.", "mið.", "fim.", "fös.", "lau."];

struct Civil {
    y: i64,
    mo: usize,
    d: i64,
    wd: usize,
    h: i64,
    mi: i64,
    s: i64,
}

fn civil(unix: i64, tz_offset: i64) -> Civil {
    let t = unix + tz_offset;
    let days = t.div_euclid(86_400);
    let rem = t.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(mo <= 2);
    Civil { y, mo: (mo - 1) as usize, d, wd: (days + 4).rem_euclid(7) as usize, h: rem / 3600, mi: rem / 60 % 60, s: rem % 60 }
}

#[derive(Clone, Debug)]
pub struct DateFormatter {
    pub locale: Locale,
    pub date_style: DateStyle,
    pub time_style: DateStyle,
    /// seconds east of UTC (`TimeZone.secondsFromGMT`)
    pub tz_offset: i64,
    pattern: Option<String>,
}

impl DateFormatter {
    pub fn new(locale: Locale) -> Self {
        DateFormatter { locale, date_style: DateStyle::None, time_style: DateStyle::None, tz_offset: 0, pattern: None }
    }

    /// `dateFormat`: a CLDR pattern (`yyyy-MM-dd HH:mm`, quotes for literals).
    pub fn set_date_format(&mut self, p: &str) {
        self.pattern = Some(p.into());
    }

    /// `setLocalizedDateFormatFromTemplate`: letters pick the fields (y, M/MMM/MMMM, d, E/EEEE, j/H/h, m, s);
    /// order and punctuation come from the locale.
    pub fn set_localized_date_format_from_template(&mut self, t: &str) {
        let has = |c: char| t.contains(c);
        let count = |c: char| t.chars().filter(|&x| x == c).count();
        let is = self.locale.lang() == Lang::Is;
        let month = match count('M') {
            0 => None,
            1 => Some("M"),
            2 => Some("MM"),
            3 => Some("MMM"),
            _ => Some("MMMM"),
        };
        let wk = match count('E') {
            0 => None,
            1..=3 => Some("EEE"),
            _ => Some("EEEE"),
        };
        let h24 = has('H') || (has('j') && self.locale.uses_24h());
        let mut time = String::new();
        if has('j') || has('H') || has('h') {
            time.push_str(if h24 { "HH" } else { "h" });
            if has('m') {
                time.push_str(":mm");
            }
            if has('s') {
                time.push_str(":ss");
            }
            if !h24 {
                time.push_str(" a");
            }
        } else if has('m') {
            time.push_str(if has('s') { "mm:ss" } else { "mm" });
        }
        let textual = month.is_some_and(|m| m.len() >= 3);
        let mut date = String::new();
        let push_sep = |d: &mut String| {
            if !d.is_empty() && !d.ends_with(' ') {
                d.push(' ');
            }
        };
        if is {
            if let Some(w) = wk {
                date.push_str(w);
                date.push(' ');
            }
            if has('d') {
                date.push_str("d.");
            }
            if let Some(m) = month {
                push_sep(&mut date);
                date.push_str(m);
                if !textual && has('y') {
                    date.push('.');
                }
            }
            if has('y') {
                push_sep(&mut date);
                date.push('y');
            }
        } else if textual {
            if let Some(w) = wk {
                date.push_str(w);
                date.push_str(", ");
            }
            date.push_str(month.unwrap_or(""));
            if has('d') {
                date.push_str(" d");
            }
            if has('y') {
                date.push_str(if has('d') { ", y" } else { " y" });
            }
        } else {
            let mut parts = vec![];
            if let Some(m) = month {
                parts.push(m);
            }
            if has('d') {
                parts.push("d");
            }
            if has('y') {
                parts.push("y");
            }
            if let Some(w) = wk {
                date.push_str(w);
                date.push_str(", ");
            }
            date.push_str(&parts.join("/"));
        }
        let date = date.trim().to_string();
        self.pattern = Some(match (date.is_empty(), time.is_empty()) {
            (false, false) => format!("{date}{}{time}", if is { " 'kl.' " } else { " 'at' " }),
            (false, true) => date,
            _ => time,
        });
    }

    fn style_pattern(&self) -> String {
        let is = self.locale.lang() == Lang::Is;
        let h24 = self.locale.uses_24h();
        let d = match (self.date_style, is) {
            (DateStyle::None, _) => "",
            (DateStyle::Short, false) => "M/d/yy",
            (DateStyle::Medium, false) => "MMM d, y",
            (DateStyle::Long, false) => "MMMM d, y",
            (DateStyle::Full, false) => "EEEE, MMMM d, y",
            (DateStyle::Short, true) => "d.M.y",
            (DateStyle::Medium, true) => "d. MMM y",
            (DateStyle::Long, true) => "d. MMMM y",
            (DateStyle::Full, true) => "EEEE, d. MMMM y",
        };
        let t = match (self.time_style, h24) {
            (DateStyle::None, _) => "",
            (DateStyle::Short, true) => "HH:mm",
            (DateStyle::Short, false) => "h:mm a",
            (_, true) => "HH:mm:ss",
            (_, false) => "h:mm:ss a",
        };
        match (d.is_empty(), t.is_empty()) {
            (false, false) => {
                let glue = if is {
                    " 'kl.' "
                } else if matches!(self.date_style, DateStyle::Long | DateStyle::Full) {
                    " 'at' "
                } else {
                    ", "
                };
                format!("{d}{glue}{t}")
            }
            (false, true) => d.into(),
            _ => t.into(),
        }
    }

    pub fn string(&self, unix: i64) -> String {
        let c = civil(unix, self.tz_offset);
        let is = self.locale.lang() == Lang::Is;
        let pat = self.pattern.clone().unwrap_or_else(|| self.style_pattern());
        let chars: Vec<char> = pat.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            let ch = chars[i];
            if ch == '\'' {
                i += 1;
                if chars.get(i) == Some(&'\'') {
                    out.push('\'');
                    i += 1;
                    continue;
                }
                while i < chars.len() && chars[i] != '\'' {
                    out.push(chars[i]);
                    i += 1;
                }
                i += 1;
                continue;
            }
            if !ch.is_ascii_alphabetic() {
                out.push(ch);
                i += 1;
                continue;
            }
            let mut n = 1;
            while chars.get(i + n) == Some(&ch) {
                n += 1;
            }
            i += n;
            let h12 = if c.h % 12 == 0 { 12 } else { c.h % 12 };
            let two = |v: i64| format!("{v:02}");
            let padded = |v: i64| if n >= 2 { two(v) } else { v.to_string() };
            match ch {
                'y' => out.push_str(&if n == 2 { two(c.y % 100) } else { c.y.to_string() }),
                'M' => out.push_str(&match n {
                    1 => (c.mo + 1).to_string(),
                    2 => two(c.mo as i64 + 1),
                    3 => if is { SHORT_IS[c.mo].into() } else { MONTHS_EN[c.mo][..3].to_string() },
                    _ => (if is { MONTHS_IS[c.mo] } else { MONTHS_EN[c.mo] }).to_string(),
                }),
                'd' => out.push_str(&padded(c.d)),
                'E' => out.push_str(&if n >= 4 {
                    (if is { DAYS_IS[c.wd] } else { DAYS_EN[c.wd] }).to_string()
                } else if is {
                    SHORTDAY_IS[c.wd].into()
                } else {
                    DAYS_EN[c.wd][..3].to_string()
                }),
                'H' => out.push_str(&padded(c.h)),
                'h' => out.push_str(&padded(h12)),
                'm' => out.push_str(&two(c.mi)),
                's' => out.push_str(&two(c.s)),
                'a' => out.push_str(if c.h < 12 { "AM" } else { "PM" }),
                other => out.extend(std::iter::repeat(other).take(n)),
            }
        }
        out
    }
}

// ---------------------------------------------------------------- RelativeDateTimeFormatter

/// `RelativeDateTimeFormatter` (named style): `delta_seconds` is `then - now`.
pub fn relative_date_time(locale: &Locale, delta_seconds: i64) -> String {
    let a = delta_seconds.unsigned_abs();
    let is = locale.lang() == Lang::Is;
    if a < 5 {
        return if is { "núna" } else { "now" }.into();
    }
    let (n, k) = match a {
        0..=59 => (a, 0),
        60..=3599 => (a / 60, 1),
        3600..=86_399 => (a / 3600, 2),
        86_400..=2_591_999 => (a / 86_400, 3),
        2_592_000..=31_535_999 => (a / 2_592_000, 4),
        _ => (a / 31_536_000, 5),
    };
    let past = delta_seconds < 0;
    if !is {
        let unit = ["second", "minute", "hour", "day", "month", "year"][k];
        let s = if n == 1 { "" } else { "s" };
        return if past { format!("{n} {unit}{s} ago") } else { format!("in {n} {unit}{s}") };
    }
    let one = n % 10 == 1 && n % 100 != 11;
    // past takes the dative ("fyrir 2 mínútum síðan"), future the accusative ("eftir 2 mínútur")
    let forms: [[&str; 4]; 6] = [
        ["sekúndu", "sekúndum", "sekúndu", "sekúndur"],
        ["mínútu", "mínútum", "mínútu", "mínútur"],
        ["klukkustund", "klukkustundum", "klukkustund", "klukkustundir"],
        ["degi", "dögum", "dag", "daga"],
        ["mánuði", "mánuðum", "mánuð", "mánuði"],
        ["ári", "árum", "ár", "ár"],
    ];
    let f = forms[k];
    if past {
        format!("fyrir {n} {} síðan", if one { f[0] } else { f[1] })
    } else {
        format!("eftir {n} {}", if one { f[2] } else { f[3] })
    }
}

// ---------------------------------------------------------------- ByteCountFormatter

/// Decimal units as Golden Gate shows them: "1,5 MB" in Icelandic, "1.5 MB" in English.
pub fn byte_count(locale: &Locale, bytes: u64) -> String {
    let is = locale.lang() == Lang::Is;
    if bytes < 1000 {
        return match (bytes, is) {
            (1, false) => "1 byte".into(),
            (1, true) => "1 bæti".into(),
            (n, false) => format!("{n} bytes"),
            (n, true) => format!("{n} bæti"),
        };
    }
    let units = ["KB", "MB", "GB", "TB", "PB"];
    let (mut v, mut u) = (bytes as f64, 0);
    while v >= 999.5 && u < units.len() {
        v /= 1000.0;
        u += 1;
    }
    let mut nf = NumberFormatter::new(locale.clone(), NumberStyle::Decimal);
    nf.uses_grouping = false;
    (nf.min_fraction, nf.max_fraction) = if u > 1 && v < 99.95 { (1, 1) } else { (0, 0) };
    format!("{} {}", nf.string(v), units[u - 1])
}

// ---------------------------------------------------------------- Measurement

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Meters,
    Kilometers,
    Miles,
    Feet,
    Grams,
    Kilograms,
    Pounds,
    Celsius,
    Fahrenheit,
    Kelvin,
}

impl Unit {
    pub fn symbol(self) -> &'static str {
        match self {
            Unit::Meters => "m",
            Unit::Kilometers => "km",
            Unit::Miles => "mi",
            Unit::Feet => "ft",
            Unit::Grams => "g",
            Unit::Kilograms => "kg",
            Unit::Pounds => "lb",
            Unit::Celsius => "\u{b0}C",
            Unit::Fahrenheit => "\u{b0}F",
            Unit::Kelvin => "K",
        }
    }
    fn dimension(self) -> u8 {
        match self {
            Unit::Meters | Unit::Kilometers | Unit::Miles | Unit::Feet => 0,
            Unit::Grams | Unit::Kilograms | Unit::Pounds => 1,
            _ => 2,
        }
    }
    /// to the base unit (metre, gram, kelvin)
    fn to_base(self, v: f64) -> f64 {
        match self {
            Unit::Meters | Unit::Grams | Unit::Kelvin => v,
            Unit::Kilometers | Unit::Kilograms => v * 1000.0,
            Unit::Miles => v * 1609.344,
            Unit::Feet => v * 0.3048,
            Unit::Pounds => v * 453.592_37,
            Unit::Celsius => v + 273.15,
            Unit::Fahrenheit => (v - 32.0) * 5.0 / 9.0 + 273.15,
        }
    }
    fn from_base(self, b: f64) -> f64 {
        match self {
            Unit::Meters | Unit::Grams | Unit::Kelvin => b,
            Unit::Kilometers | Unit::Kilograms => b / 1000.0,
            Unit::Miles => b / 1609.344,
            Unit::Feet => b / 0.3048,
            Unit::Pounds => b / 453.592_37,
            Unit::Celsius => b - 273.15,
            Unit::Fahrenheit => (b - 273.15) * 9.0 / 5.0 + 32.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measurement {
    pub value: f64,
    pub unit: Unit,
}

impl Measurement {
    pub fn new(value: f64, unit: Unit) -> Self {
        Measurement { value, unit }
    }
    /// `converted(to:)`; None across dimensions.
    pub fn converted(self, to: Unit) -> Option<Measurement> {
        (self.unit.dimension() == to.dimension()).then(|| Measurement { value: to.from_base(self.unit.to_base(self.value)), unit: to })
    }
}

/// `MeasurementFormatter` (short unit style, up to 2 fraction digits).
pub fn measurement(locale: &Locale, m: Measurement) -> String {
    let mut nf = NumberFormatter::new(locale.clone(), NumberStyle::Decimal);
    nf.max_fraction = 2;
    let tight = matches!(m.unit, Unit::Celsius | Unit::Fahrenheit) && locale.lang() == Lang::En;
    format!("{}{}{}", nf.string(m.value), if tight { "" } else { "\u{a0}" }, m.unit.symbol())
}

// ---------------------------------------------------------------- ListFormatter

/// `ListFormatter.string(from:)`: "a, b, and c" (en), "a, b og c" (is).
pub fn list(locale: &Locale, items: &[&str]) -> String {
    let is = locale.lang() == Lang::Is;
    let and = if is { "og" } else { "and" };
    match items {
        [] => String::new(),
        [a] => (*a).into(),
        [a, b] => format!("{a} {and} {b}"),
        [init @ .., last] => format!("{}{} {and} {last}", init.join(", "), if is { "" } else { "," }),
    }
}

// ---------------------------------------------------------------- NativeLocalizedString

/// Parses `Localizable.strings`: `"key" = "value";` with `//` and `/* */` comments and `\" \\ \n \t` escapes.
pub fn parse_strings(text: &str) -> HashMap<String, String> {
    let c: Vec<char> = text.chars().collect();
    let (mut i, mut out) = (0, HashMap::new());
    let mut toks: Vec<String> = vec![];
    while i < c.len() {
        match c[i] {
            '/' if c.get(i + 1) == Some(&'/') => {
                while i < c.len() && c[i] != '\n' {
                    i += 1;
                }
            }
            '/' if c.get(i + 1) == Some(&'*') => {
                i += 2;
                while i + 1 < c.len() && !(c[i] == '*' && c[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
            }
            '"' => {
                i += 1;
                let mut s = String::new();
                while i < c.len() && c[i] != '"' {
                    if c[i] == '\\' && i + 1 < c.len() {
                        i += 1;
                        s.push(match c[i] {
                            'n' => '\n',
                            't' => '\t',
                            o => o,
                        });
                    } else {
                        s.push(c[i]);
                    }
                    i += 1;
                }
                i += 1;
                toks.push(s);
            }
            ';' => {
                if toks.len() == 2 {
                    out.insert(toks[0].clone(), toks[1].clone());
                }
                toks.clear();
                i += 1;
            }
            _ => i += 1,
        }
    }
    out
}

/// `Bundle`: the app's `Localizable.strings` tables, most specific first.
#[derive(Clone, Debug, Default)]
pub struct Bundle {
    tables: Vec<HashMap<String, String>>,
}

impl Bundle {
    /// Reads `<dir>/<lang>.lproj/Localizable.strings` for the locale's language, then `en` as fallback.
    pub fn load(dir: &Path, locale: &Locale) -> Self {
        let mut tables = vec![];
        for lang in [locale.language.as_str(), "en"] {
            if let Ok(t) = std::fs::read_to_string(dir.join(format!("{lang}.lproj/Localizable.strings"))) {
                tables.push(parse_strings(&t));
            }
        }
        Bundle { tables }
    }

    pub fn from_tables(tables: Vec<HashMap<String, String>>) -> Self {
        Bundle { tables }
    }

    /// The translation, or the key itself when no table has it.
    pub fn string(&self, key: &str) -> String {
        self.tables.iter().find_map(|t| t.get(key)).cloned().unwrap_or_else(|| key.into())
    }
}

static MAIN: Mutex<Option<Bundle>> = Mutex::new(None);

/// `Bundle.main` for this process; call once at launch with the app package directory.
pub fn set_main_bundle(b: Bundle) {
    if let Ok(mut g) = MAIN.lock() {
        *g = Some(b);
    }
}

/// Backs `localized!`; the comment is for translators only.
pub fn localized(key: &str, _comment: &str) -> String {
    MAIN.lock().ok().and_then(|g| g.as_ref().map(|b| b.string(key))).unwrap_or_else(|| key.into())
}

/// `NativeLocalizedString(key, comment:)`.
#[macro_export]
macro_rules! localized {
    ($key:expr, $comment:expr) => {
        $crate::locale::localized($key, $comment)
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    fn en() -> Locale {
        Locale::new("en_US.UTF-8")
    }
    fn is() -> Locale {
        Locale::new("is_IS")
    }

    #[test]
    fn locale_from_env_and_region_pref() {
        assert_eq!(en().identifier(), "en_US");
        assert_eq!(Locale::from_env("C", None).identifier(), "en_US");
        assert_eq!(Locale::from_env("is_IS.UTF-8", Some("de")).identifier(), "is_DE");
        assert!(!en().uses_metric_system() && is().uses_metric_system());
    }

    #[test]
    fn numbers() {
        let mut nf = NumberFormatter::new(en(), NumberStyle::Decimal);
        assert_eq!(nf.string(1234567.891), "1,234,567.891");
        assert_eq!(NumberFormatter::new(is(), NumberStyle::Decimal).string(-1234.5), "-1.234,5");
        nf.uses_grouping = false;
        assert_eq!(nf.string(1234.0), "1234");
        assert_eq!(NumberFormatter::new(en(), NumberStyle::Percent).string(0.256), "26%");
        assert_eq!(NumberFormatter::new(is(), NumberStyle::Percent).string(0.5), "50\u{a0}%");
        assert_eq!(NumberFormatter::new(en(), NumberStyle::Currency).string(1234.5), "$1,234.50");
        assert_eq!(NumberFormatter::new(is(), NumberStyle::Currency).string(1234.5), "1.234\u{a0}kr."); // half-even, NumberFormatter.roundingMode default
    }

    #[test]
    fn dates() {
        let t = 1_791_727_509; // 2026-10-11 14:05:09 UTC, a Sunday
        let mut f = DateFormatter::new(en());
        f.date_style = DateStyle::Full;
        assert_eq!(f.string(t), "Sunday, October 11, 2026");
        f.date_style = DateStyle::Short;
        f.time_style = DateStyle::Short;
        assert_eq!(f.string(t), "10/11/26, 2:05 PM");
        let mut g = DateFormatter::new(is());
        g.date_style = DateStyle::Long;
        g.time_style = DateStyle::Medium;
        assert_eq!(g.string(t), "11. október 2026 kl. 14:05:09");
        g.date_style = DateStyle::Full;
        g.time_style = DateStyle::None;
        assert_eq!(g.string(t), "sunnudagur, 11. október 2026");
        g.set_localized_date_format_from_template("yMMMd");
        assert_eq!(g.string(t), "11. okt. 2026");
        f.set_localized_date_format_from_template("yMMMMd");
        assert_eq!(f.string(t), "October 11, 2026");
        f.set_date_format("yyyy-MM-dd'T'HH:mm");
        assert_eq!(f.string(t), "2026-10-11T14:05");
    }

    #[test]
    fn relative_bytes_measure_list() {
        assert_eq!(relative_date_time(&en(), -90), "1 minute ago");
        assert_eq!(relative_date_time(&is(), -7200), "fyrir 2 klukkustundum síðan");
        assert_eq!(relative_date_time(&is(), 172_800), "eftir 2 daga");
        assert_eq!(byte_count(&en(), 1_500_000), "1.5 MB");
        assert_eq!(byte_count(&is(), 1_500_000), "1,5 MB");
        assert_eq!(byte_count(&is(), 12), "12 bæti");
        let m = Measurement::new(100.0, Unit::Celsius).converted(Unit::Fahrenheit).unwrap();
        assert_eq!(measurement(&en(), m), "212\u{b0}F");
        assert_eq!(measurement(&is(), Measurement::new(2.5, Unit::Kilometers)), "2,5\u{a0}km");
        assert!(Measurement::new(1.0, Unit::Meters).converted(Unit::Grams).is_none());
        assert_eq!(list(&en(), &["a", "b", "c"]), "a, b, and c");
        assert_eq!(list(&is(), &["a", "b", "c"]), "a, b og c");
    }

    #[test]
    fn localizable_strings() {
        let t = parse_strings("/* c */ \"Hello\" = \"Halló\"; // x\n\"Quote\" = \"a \\\"b\\\"\\n\";");
        assert_eq!(t["Hello"], "Halló");
        assert_eq!(t["Quote"], "a \"b\"\n");
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("is.lproj")).unwrap();
        std::fs::write(dir.path().join("is.lproj/Localizable.strings"), "\"Open\" = \"Opna\";").unwrap();
        let b = Bundle::load(dir.path(), &is());
        assert_eq!((b.string("Open").as_str(), b.string("Missing").as_str()), ("Opna", "Missing"));
        set_main_bundle(b);
        assert_eq!(crate::localized!("Open", "menu"), "Opna");
        assert_eq!(crate::localized!("Nope", ""), "Nope");
    }
}
