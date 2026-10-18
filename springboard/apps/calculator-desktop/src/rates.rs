//! Live currency rates for Convert: the European Central Bank's daily reference rates, a public source that needs no
//! account. The bank publishes them on working days around 16:00 CET, as one XML file of rates per euro.
//!
//! The app checks for a new file at most once a day (UTC day): the check runs on a worker thread at launch, and the
//! result, the rates and the day of the last check go to `rates.txt` in the app's data folder. Offline, the last copy
//! stays in use and the Convert panel shows its date; with no copy at all, Currency shows as unavailable.

use crate::history::now_secs;
use bifrost_app::url::{URLRequest, URLSession};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Receiver, TryRecvError};

/// The public daily file (reference rates, euro as the base currency).
pub const SOURCE_URL: &str = "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml";
/// Credit line shown with the rates.
pub const SOURCE_NAME: &str = "European Central Bank reference rates";
/// The cache file in the app's data folder.
pub const CACHE_FILE: &str = "rates.txt";
/// The file is about 4 KB; anything much larger is not the rates file.
const MAX_BYTES: usize = 256 * 1024;

/// The rates of one publication day.
#[derive(Clone, Debug, PartialEq)]
pub struct Rates {
    /// The day the bank published the rates, "YYYY-MM-DD".
    pub date: String,
    /// Units of each currency per one euro (the euro itself is not listed).
    pub per_euro: Vec<(String, f64)>,
}

impl Rates {
    /// Units of `code` per one euro; the euro is 1.
    pub fn per_euro(&self, code: &str) -> Option<f64> {
        if code == "EUR" {
            return Some(1.0);
        }
        self.per_euro.iter().find(|(c, _)| c == code).map(|(_, rate)| *rate)
    }

    /// Reads the bank's XML: a `<Cube time="...">` line for the day, then one `<Cube currency="USD" rate="..."/>` line
    /// per currency. None if the day or any rate is missing.
    pub fn parse_xml(xml: &str) -> Option<Rates> {
        let mut date = None;
        let mut per_euro = Vec::new();
        for line in xml.lines() {
            if date.is_none() {
                if let Some(time) = attr(line, "time") {
                    date = Some(time.to_string());
                    continue;
                }
            }
            if let (Some(code), Some(rate)) = (attr(line, "currency"), attr(line, "rate")) {
                let rate: f64 = rate.parse().ok()?;
                if code.len() == 3 && rate.is_finite() && rate > 0.0 {
                    per_euro.push((code.to_string(), rate));
                }
            }
        }
        let date = date?;
        (!per_euro.is_empty()).then_some(Rates { date, per_euro })
    }
}

/// The value of attribute `name` on `line` (single or double quotes), if present.
fn attr<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=");
    let start = line.find(&key)? + key.len();
    let rest = &line[start..];
    let quote = rest.chars().next().filter(|c| *c == '\'' || *c == '"')?;
    let inner = &rest[1..];
    inner.find(quote).map(|end| &inner[..end])
}

/// What the cache file holds: the day of the last check (UTC days since 1970) and the rates in use.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cache {
    pub checked_day: Option<i64>,
    pub rates: Option<Rates>,
}

impl Cache {
    pub fn parse(text: &str) -> Cache {
        let mut checked_day = None;
        let mut date = None;
        let mut per_euro = Vec::new();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('\t') else { continue };
            match key {
                "checked" => checked_day = value.parse().ok(),
                "date" => date = Some(value.to_string()),
                code if code.len() == 3 => {
                    if let Ok(rate) = value.parse::<f64>() {
                        if rate.is_finite() && rate > 0.0 {
                            per_euro.push((code.to_string(), rate));
                        }
                    }
                }
                _ => {}
            }
        }
        let rates = match date {
            Some(date) if !per_euro.is_empty() => Some(Rates { date, per_euro }),
            _ => None,
        };
        Cache { checked_day, rates }
    }

    pub fn encode(&self) -> String {
        let mut out = String::new();
        if let Some(day) = self.checked_day {
            out.push_str(&format!("checked\t{day}\n"));
        }
        if let Some(rates) = &self.rates {
            out.push_str(&format!("date\t{}\n", rates.date));
            for (code, rate) in &rates.per_euro {
                out.push_str(&format!("{code}\t{rate}\n"));
            }
        }
        out
    }

    pub fn load(path: &Path) -> Cache {
        std::fs::read_to_string(path).map(|t| Cache::parse(&t)).unwrap_or_default()
    }

    fn save(&self, path: &Path) {
        let _ = std::fs::write(path, self.encode());
    }
}

/// The current UTC day, as days since 1970.
pub fn today() -> i64 {
    (now_secs() / 86_400) as i64
}

/// Whether a check is due: none has run today.
pub fn due(cache: &Cache, today: i64) -> bool {
    cache.checked_day != Some(today)
}

/// Downloads the file and reads it; the error text is shown nowhere, so it only says why the copy was not replaced.
fn fetch() -> Result<Rates, String> {
    let req = URLRequest::new(SOURCE_URL);
    let cancel = AtomicBool::new(false);
    let mut body = Vec::new();
    let mut too_big = false;
    let resp = URLSession::shared()
        .bytes(&req, &cancel, &mut |_| {}, &mut |chunk| {
            body.extend_from_slice(chunk);
            too_big = body.len() > MAX_BYTES;
            !too_big
        })
        .map_err(|e| e.to_string())?;
    if too_big {
        return Err("rates file is larger than expected".into());
    }
    if resp.status_code != 200 {
        return Err(format!("status {}", resp.status_code));
    }
    let text = String::from_utf8(body).map_err(|_| "rates file is not text".to_string())?;
    Rates::parse_xml(&text).ok_or_else(|| "rates file not recognised".to_string())
}

/// The rates for the Convert panel: the cached copy at once, and a check running in the background when one is due.
pub struct Service {
    cache: Cache,
    rx: Option<Receiver<Result<Rates, String>>>,
}

impl Service {
    /// Loads the cache from `dir` and starts a check on a worker thread if none has run today.
    pub fn start(dir: &Path, today: i64) -> Service {
        let path = dir.join(CACHE_FILE);
        let cache = Cache::load(&path);
        let rx = due(&cache, today).then(|| {
            let (tx, rx) = mpsc::channel();
            let previous = cache.clone();
            std::thread::spawn(move || {
                let result = fetch();
                // a failed check keeps the previous rates, and the day is still recorded so offline launches do not retry
                let rates = match &result {
                    Ok(rates) => Some(rates.clone()),
                    Err(_) => previous.rates,
                };
                Cache { checked_day: Some(today), rates }.save(&path);
                let _ = tx.send(result);
            });
            rx
        });
        Service { cache, rx }
    }

    /// The rates in use, or None when there is no copy.
    pub fn rates(&self) -> Option<Rates> {
        self.cache.rates.clone()
    }

    /// Whether a check is still running.
    pub fn pending(&self) -> bool {
        self.rx.is_some()
    }

    /// Takes the finished check, if any. True when the rates in use changed.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        match rx.try_recv() {
            Ok(Ok(rates)) => {
                self.cache.rates = Some(rates);
                self.rx = None;
                true
            }
            Ok(Err(_)) | Err(TryRecvError::Disconnected) => {
                self.rx = None;
                false
            }
            Err(TryRecvError::Empty) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<gesmes:Envelope xmlns:gesmes="http://www.gesmes.org/xml/2002-08-01">
<Cube>
<Cube time='2026-10-16'>
<Cube currency='USD' rate='1.1650'/>
<Cube currency="ISK" rate="143.20"/>
<Cube currency='JPY' rate='0'/>
</Cube>
</Cube>
</gesmes:Envelope>"#;

    #[test]
    fn the_bank_xml_gives_the_day_and_the_positive_rates() {
        let rates = Rates::parse_xml(SAMPLE).unwrap();
        assert_eq!(rates.date, "2026-10-16");
        assert_eq!(rates.per_euro("USD"), Some(1.165));
        assert_eq!(rates.per_euro("ISK"), Some(143.2));
        assert_eq!(rates.per_euro("JPY"), None, "a zero rate is not used");
        assert_eq!(rates.per_euro("EUR"), Some(1.0));
    }

    #[test]
    fn an_xml_without_a_day_or_rates_is_refused() {
        assert_eq!(Rates::parse_xml("<html>maintenance</html>"), None);
        assert_eq!(Rates::parse_xml("<Cube time='2026-10-16'></Cube>"), None);
    }

    #[test]
    fn the_cache_round_trips_the_check_day_and_the_rates() {
        let cache = Cache { checked_day: Some(20_000), rates: Rates::parse_xml(SAMPLE) };
        assert_eq!(Cache::parse(&cache.encode()), cache);
        assert_eq!(Cache::parse(""), Cache::default());
        assert_eq!(Cache::parse("checked\t5\n").rates, None, "a day alone is not a rates copy");
    }

    #[test]
    fn a_check_is_due_once_per_day() {
        let done = Cache { checked_day: Some(100), rates: None };
        assert!(!due(&done, 100));
        assert!(due(&done, 101));
        assert!(due(&Cache::default(), 100));
    }

    #[test]
    fn attributes_read_single_and_double_quotes_and_skip_longer_names() {
        assert_eq!(attr("<Cube currency='USD' rate='1.2'/>", "rate"), Some("1.2"));
        assert_eq!(attr("<Cube currency=\"USD\"/>", "currency"), Some("USD"));
        assert_eq!(attr("<Cube exchangerate='9'/>", "rate"), None);
    }
}
