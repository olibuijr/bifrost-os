//! The quote and price-history service: symbol rules, request addresses, the parser for the service's JSON answers
//! and the app's own CSV cache form. The service is Yahoo Finance's public chart endpoint (`/v8/finance/chart`): no
//! key, no account and no cookie are sent, only the symbol (the reference app credits the same source). One request
//! per symbol gives the latest price, the name, the previous close and a month of daily closes (the sparkline); a
//! longer range gives the chart. Stooq, the source before, now answers its quote address with 404 and its history
//! address with a JavaScript browser check (E2E W4-14).
//! The network is passed in as a `Fetch`, so the parsers and the fetch functions can be checked without it.

use crate::model::{Bar, Quote};
use bifrost_core::json::{self, Json};
use springboard::shell::music::radio::Fetch;
use std::io;

/// The chart endpoint; the symbol follows, then the range and the interval.
pub const CHART_API: &str = "https://query1.finance.yahoo.com/v8/finance/chart/";
/// The range a quote request asks for: daily closes enough for the sidebar's sparkline and the previous close.
pub const QUOTE_RANGE: &str = "1mo";
/// The range of the chart.
pub const HISTORY_RANGE: &str = "6mo";
/// The largest answer the app reads (a month is a few kilobytes; six months a few tens).
pub const BODY_LIMIT: usize = 1024 * 1024;
/// The sessions the chart keeps (about four months of trading days).
pub const HISTORY_KEEP: usize = 90;
/// The header of the cached quotes file.
const QUOTES_HEADER: &str = "Symbol,Date,Time,Previous,Close,Name,Spark";
/// The header of a cached history file.
const HISTORY_HEADER: &str = "Date,Close";

/// A typed symbol in the service's form: upper case, "AAPL", "^GSPC", "BRK-B", "MAREL.IC". The ".us" suffix of the
/// old Stooq form is dropped and "^SPX" becomes "^GSPC". Returns `None` for an empty or malformed symbol.
pub fn normalise_symbol(raw: &str) -> Option<String> {
    let s = raw.trim().to_uppercase();
    let s = s.strip_suffix(".US").unwrap_or(&s).to_string();
    if s.is_empty() || s.len() > 12 || !s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '^' | '-' | '=')) {
        return None;
    }
    Some(if s == "^SPX" { "^GSPC".to_string() } else { s })
}

/// The symbol as the screen shows it.
pub fn display_symbol(symbol: &str) -> String {
    symbol.to_uppercase()
}

/// The symbol escaped for a URL path ("^GSPC" is "%5EGSPC").
fn escape(symbol: &str) -> String {
    symbol.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-') { c.to_string() } else { format!("%{:02X}", c as u32) }).collect()
}

/// The address of `symbol`'s chart over `range` in daily bars.
fn chart_url(symbol: &str, range: &str) -> String {
    format!("{CHART_API}{}?range={range}&interval=1d", escape(symbol))
}

/// The address of the latest quote (and a month of closes) of one symbol.
pub fn quote_url(symbol: &str) -> String {
    chart_url(symbol, QUOTE_RANGE)
}

/// The address of the daily history of one symbol.
pub fn history_url(symbol: &str) -> String {
    chart_url(symbol, HISTORY_RANGE)
}

/// One answer of the chart endpoint: the meta block and the daily closes.
#[derive(Clone, Debug, PartialEq)]
pub struct Chart {
    pub symbol: String,
    pub name: String,
    pub price: f64,
    /// Unix seconds of the price, and the exchange's offset from UTC in seconds.
    pub time: i64,
    pub offset: i64,
    /// Daily bars, oldest first (sessions without a close are left out).
    pub bars: Vec<Bar>,
}

impl Chart {
    /// The quote: the price against the close before the latest session.
    pub fn quote(&self) -> Quote {
        let closes: Vec<f64> = self.bars.iter().map(|b| b.close).collect();
        let previous = if closes.len() >= 2 { closes[closes.len() - 2] } else { closes.first().copied().unwrap_or(self.price) };
        Quote {
            symbol: self.symbol.clone(),
            name: self.name.clone(),
            stamp: stamp(self.time + self.offset),
            open: previous,
            close: self.price,
            spark: closes,
        }
    }
}

/// Days since 1970-01-01 to a civil (year, month, day).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// "2026-10-22" for local unix seconds.
fn iso_date(local: i64) -> String {
    let (y, m, d) = civil(local.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// "2026-10-22 16:00" for local unix seconds.
fn stamp(local: i64) -> String {
    let secs = local.rem_euclid(86_400);
    format!("{} {:02}:{:02}", iso_date(local), secs / 3600, secs / 60 % 60)
}

/// Reads one chart answer. The service's own error ("No data found, symbol may be delisted") is the `Err` text.
pub fn parse_chart(text: &str) -> Result<Chart, String> {
    let root = json::parse(text).map_err(|_| "The quote service sent an answer it could not read.".to_string())?;
    let chart = root.get("chart").ok_or("The quote service sent an unexpected answer.")?;
    if let Some(e) = chart.get("error").filter(|e| **e != Json::Null) {
        return Err(e.str_field("description").unwrap_or_else(|| "The symbol was not found.".to_string()));
    }
    let result = chart.get("result").and_then(Json::as_array).and_then(|r| r.first()).ok_or("The symbol was not found.")?;
    let meta = result.get("meta").ok_or("The quote service sent an unexpected answer.")?;
    let num = |j: &Json, k: &str| j.get(k).and_then(Json::number);
    let price = num(meta, "regularMarketPrice").ok_or("The symbol has no price.")?;
    let symbol = meta.str_field("symbol").and_then(|s| normalise_symbol(&s)).ok_or("The quote service sent an unexpected answer.")?;
    let name = meta.str_field("shortName").or_else(|| meta.str_field("longName")).unwrap_or_default();
    let offset = num(meta, "gmtoffset").unwrap_or(0.0) as i64;
    let time = num(meta, "regularMarketTime").unwrap_or(0.0) as i64;
    let stamps = result.get("timestamp").and_then(Json::as_array).unwrap_or(&[]);
    let closes = result
        .get("indicators")
        .and_then(|i| i.get("quote"))
        .and_then(Json::as_array)
        .and_then(|q| q.first())
        .and_then(|q| q.get("close"))
        .and_then(Json::as_array)
        .unwrap_or(&[]);
    let bars = stamps
        .iter()
        .zip(closes)
        .filter_map(|(t, c)| Some(Bar { date: iso_date(t.number()? as i64 + offset), close: c.number()? }))
        .collect();
    Ok(Chart { symbol, name, price, time, offset, bars })
}

/// The cached quotes file: one row per quote, the name with its commas taken out and the sparkline's closes joined
/// by semicolons.
pub fn quotes_csv(quotes: &[Quote]) -> String {
    let mut out = format!("{QUOTES_HEADER}\n");
    for q in quotes {
        let (date, time) = q.stamp.split_once(' ').unwrap_or((q.stamp.as_str(), ""));
        let spark: Vec<String> = q.spark.iter().map(|v| format!("{v}")).collect();
        out.push_str(&format!("{},{date},{time},{},{},{},{}\n", q.symbol, q.open, q.close, q.name.replace([',', '\n'], " "), spark.join(";")));
    }
    out
}

/// The quote rows of a cached quotes file. Unreadable rows are skipped.
pub fn parse_quotes(csv: &str) -> Vec<Quote> {
    csv.lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.trim().split(',').collect();
            if f.len() < 5 || f[1].is_empty() {
                return None;
            }
            Some(Quote {
                symbol: normalise_symbol(f[0])?,
                name: f.get(5).map_or(String::new(), |s| s.to_string()),
                stamp: format!("{} {}", f[1], f[2]).trim().to_string(),
                open: f[3].parse().ok()?,
                close: f[4].parse().ok()?,
                spark: f.get(6).map_or(Vec::new(), |s| s.split(';').filter_map(|v| v.parse().ok()).collect()),
            })
        })
        .collect()
}

/// The cached history file of one symbol.
pub fn history_csv(bars: &[Bar]) -> String {
    let mut out = format!("{HISTORY_HEADER}\n");
    for b in bars {
        out.push_str(&format!("{},{}\n", b.date, b.close));
    }
    out
}

/// The bars of a cached history file (`Date,Close`), oldest first. Rows without a date or a close are skipped.
pub fn parse_history(csv: &str) -> Vec<Bar> {
    let mut bars: Vec<Bar> = csv
        .lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.trim().split(',').collect();
            if f.len() < 2 || !is_iso_date(f[0]) {
                return None;
            }
            Some(Bar { date: f[0].to_string(), close: f[1].parse().ok()? })
        })
        .collect();
    bars.sort_by(|a, b| a.date.cmp(&b.date));
    bars
}

/// The last `keep` bars of a history.
pub fn tail(mut bars: Vec<Bar>, keep: usize) -> Vec<Bar> {
    let cut = bars.len().saturating_sub(keep);
    bars.drain(..cut);
    bars
}

fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10 && b[4] == b'-' && b[7] == b'-' && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// The chart of one symbol over `range`. A symbol the service does not know is `Ok(Err(message))`; a failed request
/// (no network, the service down) is the outer `Err`.
fn fetch_chart(symbol: &str, url: &str, net: &dyn Fetch) -> io::Result<Result<Chart, String>> {
    match net.get(url, BODY_LIMIT) {
        Ok(body) => Ok(parse_chart(&String::from_utf8_lossy(&body))),
        // the service answers an unknown symbol with 404 (and its own error in the body, which the fetch drops)
        Err(e) if e.to_string().contains("status 404") => Ok(Err(format!("{} was not found.", display_symbol(symbol)))),
        Err(e) => Err(e),
    }
}

/// What a quote request brought back: the cache text, the quotes, and the symbols the service did not know with why.
pub type QuoteAnswer = (String, Vec<Quote>, Vec<(String, String)>);

/// Fetches the latest quote of each of `symbols`. A symbol the service does not know is listed with its message, so
/// the row can say so instead of waiting. When no request got through at all the result is an error, so an outage
/// never looks like a list of unknown symbols.
pub fn fetch_quotes(symbols: &[String], net: &dyn Fetch) -> io::Result<QuoteAnswer> {
    let (mut quotes, mut failed, mut down) = (Vec::new(), Vec::new(), None);
    for symbol in symbols {
        match fetch_chart(symbol, &quote_url(symbol), net) {
            Ok(Ok(chart)) => {
                let mut q = chart.quote();
                q.symbol = symbol.clone();
                quotes.push(q);
            }
            Ok(Err(message)) => failed.push((symbol.clone(), message)),
            Err(e) => down = Some(e),
        }
    }
    if quotes.is_empty() && failed.is_empty() {
        return Err(down.unwrap_or_else(|| io::Error::other("no symbols")));
    }
    Ok((quotes_csv(&quotes), quotes, failed))
}

/// Fetches the daily history of `symbol`. Returns the cache text and the bars, oldest first.
pub fn fetch_history(symbol: &str, net: &dyn Fetch) -> io::Result<(String, Vec<Bar>)> {
    let chart = fetch_chart(symbol, &history_url(symbol), net)?.map_err(io::Error::other)?;
    if chart.bars.is_empty() {
        return Err(io::Error::other("no history in the answer"));
    }
    Ok((history_csv(&chart.bars), chart.bars))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trimmed chart answer as the service sends it (three sessions, one without a close).
    const CHART: &str = r#"{"chart":{"result":[{"meta":{"currency":"USD","symbol":"AAPL","regularMarketTime":1792699201,
        "gmtoffset":-14400,"regularMarketPrice":348.22,"shortName":"Apple Inc.","longName":"Apple Inc."},
        "timestamp":[1792503000,1792589400,1792600000,1792675800],
        "indicators":{"quote":[{"close":[339.5,343.99,null,348.22]}],"adjclose":[{"adjclose":[339.5,343.99,null,348.22]}]}}],
        "error":null}}"#;

    const UNKNOWN: &str = r#"{"chart":{"result":null,"error":{"code":"Not Found","description":"No data found, symbol may be delisted"}}}"#;

    struct Canned(&'static str);
    impl Fetch for Canned {
        fn get(&self, _url: &str, _limit: usize) -> io::Result<Vec<u8>> {
            Ok(self.0.as_bytes().to_vec())
        }
    }

    struct Down;
    impl Fetch for Down {
        fn get(&self, _url: &str, _limit: usize) -> io::Result<Vec<u8>> {
            Err(io::Error::other("offline"))
        }
    }

    /// Knows AAPL only; anything else is the 404 the service sends for an unknown symbol.
    struct OnlyApple;
    impl Fetch for OnlyApple {
        fn get(&self, url: &str, _limit: usize) -> io::Result<Vec<u8>> {
            if url.contains("/chart/AAPL?") { Ok(CHART.as_bytes().to_vec()) } else { Err(io::Error::other(format!("{url}: status 404"))) }
        }
    }

    #[test]
    fn symbols_are_normalised_to_the_service_form() {
        assert_eq!(normalise_symbol(" aapl "), Some("AAPL".to_string()));
        assert_eq!(normalise_symbol("nvda.us"), Some("NVDA".to_string()), "the old Stooq form still reads");
        assert_eq!(normalise_symbol("^spx"), Some("^GSPC".to_string()));
        assert_eq!(normalise_symbol("brk-b"), Some("BRK-B".to_string()));
        assert_eq!(normalise_symbol("marel.ic"), Some("MAREL.IC".to_string()));
        assert_eq!(normalise_symbol(""), None);
        assert_eq!(normalise_symbol("a b"), None);
        assert_eq!(normalise_symbol("waytoolongsymbol"), None);
        assert_eq!(display_symbol("AAPL"), "AAPL");
    }

    #[test]
    fn addresses_name_the_symbol_the_range_and_daily_bars() {
        assert_eq!(quote_url("AAPL"), "https://query1.finance.yahoo.com/v8/finance/chart/AAPL?range=1mo&interval=1d");
        assert_eq!(history_url("^GSPC"), "https://query1.finance.yahoo.com/v8/finance/chart/%5EGSPC?range=6mo&interval=1d");
        assert!(!quote_url("AAPL").contains("key"), "no key or account goes with a request");
    }

    #[test]
    fn a_chart_answer_gives_the_price_the_previous_close_the_name_and_the_closes() {
        let chart = parse_chart(CHART).unwrap();
        assert_eq!(chart.symbol, "AAPL");
        assert_eq!(chart.name, "Apple Inc.");
        assert_eq!(chart.bars.iter().map(|b| b.close).collect::<Vec<_>>(), [339.5, 343.99, 348.22], "a session without a close is left out");
        assert_eq!(chart.bars[0].date, "2026-10-20", "dates are the exchange's own days");
        let q = chart.quote();
        assert_eq!((q.open, q.close), (343.99, 348.22), "the change is against the previous close");
        assert!((q.change_pct() - 1.23).abs() < 0.01);
        assert_eq!(q.stamp, "2026-10-22 16:00");
        assert_eq!(q.spark.len(), 3);
    }

    #[test]
    fn the_services_own_error_is_the_message() {
        assert_eq!(parse_chart(UNKNOWN), Err("No data found, symbol may be delisted".to_string()));
        assert!(parse_chart("<html>busy</html>").is_err());
    }

    #[test]
    fn the_quote_cache_round_trips() {
        let q = parse_chart(CHART).unwrap().quote();
        let csv = quotes_csv(&[q.clone()]);
        assert!(csv.starts_with(QUOTES_HEADER));
        assert_eq!(parse_quotes(&csv), vec![q]);
        assert!(parse_quotes("Symbol\nbad\n").is_empty());
    }

    #[test]
    fn history_round_trips_oldest_first_and_skips_bad_rows() {
        let bars = parse_chart(CHART).unwrap().bars;
        assert_eq!(parse_history(&history_csv(&bars)), bars);
        let messy = "Date,Close\n2026-10-03,12.5\nbad row\n2026-10-01,10.5\n2026-10-02,\n";
        let dates: Vec<String> = parse_history(messy).into_iter().map(|b| b.date).collect();
        assert_eq!(dates, ["2026-10-01", "2026-10-03"]);
    }

    #[test]
    fn tail_keeps_the_newest_bars() {
        let bars: Vec<Bar> = (1..=3).map(|d| Bar { date: format!("2026-10-0{d}"), close: d as f64 }).collect();
        let got = tail(bars.clone(), 2);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].date, "2026-10-02");
        assert_eq!(tail(bars, 10).len(), 3);
        assert_eq!(tail(Vec::new(), 3).len(), 0);
    }

    #[test]
    fn a_good_answer_is_returned_with_its_cache_text() {
        let (raw, quotes, failed) = fetch_quotes(&["AAPL".into()], &Canned(CHART)).unwrap();
        assert_eq!(parse_quotes(&raw), quotes);
        assert_eq!(quotes[0].close, 348.22);
        assert!(failed.is_empty());
        let (raw, bars) = fetch_history("AAPL", &Canned(CHART)).unwrap();
        assert_eq!(bars.len(), 3);
        assert_eq!(parse_history(&raw), bars);
    }

    #[test]
    fn an_unknown_symbol_is_named_not_waited_for() {
        let (_, quotes, failed) = fetch_quotes(&["AAPL".into(), "NOPE".into()], &OnlyApple).unwrap();
        assert_eq!(quotes.len(), 1);
        assert_eq!(failed, vec![("NOPE".to_string(), "NOPE was not found.".to_string())]);
        let (_, quotes, failed) = fetch_quotes(&["NOPE".into()], &Canned(UNKNOWN)).unwrap();
        assert!(quotes.is_empty());
        assert_eq!(failed[0].1, "No data found, symbol may be delisted");
        assert!(fetch_history("NOPE", &OnlyApple).is_err());
    }

    #[test]
    fn a_network_failure_is_passed_on() {
        assert!(fetch_quotes(&["AAPL".into()], &Down).is_err());
        assert!(fetch_history("AAPL", &Down).is_err());
    }

    #[test]
    fn civil_dates_follow_the_calendar() {
        assert_eq!(iso_date(0), "1970-01-01");
        assert_eq!(iso_date(951_782_400), "2000-02-29");
        assert_eq!(stamp(1_792_699_201 - 14_400), "2026-10-22 16:00");
    }
}
