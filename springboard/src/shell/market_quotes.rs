//! Stock quotes from Yahoo Finance's public chart endpoint (`/v8/finance/chart`), shared by the Stocks app and the
//! Weather app's watchlist page: symbol rules, request addresses, the parser for the service's JSON answers and the
//! fetcher the requests go through.
//!
//! Privacy (owner rule, privacy-first): a request carries the symbol and nothing else. There is no key, account,
//! cookie or "crumb" (the token the service's own web pages fetch first), and [`QuoteHttp`] sends a generic
//! `User-Agent` (no product or device name), keeps no cookies the service sets and refuses plain HTTP, redirects
//! included. The screens that show quotes credit the source with [`ATTRIBUTION`]: the prices are delayed.
//!
//! One request per symbol gives the latest price, the name, the previous close and a month of daily closes; a longer
//! range gives a chart. Stooq, the source before, answers its quote address with 404 and its history address with a
//! JavaScript browser check (E2E W4-14).

use crate::deildu::http::{Client, Jar, Tls, Transport};
use crate::shell::music::radio::Fetch;
use bifrost_core::json::{self, Json};
use std::io;
use std::sync::{Arc, OnceLock};

/// The chart endpoint; the symbol follows, then the range and the interval.
pub const CHART_API: &str = "https://query1.finance.yahoo.com/v8/finance/chart/";
/// The range a quote request asks for: daily closes enough for a sparkline and the previous close.
pub const QUOTE_RANGE: &str = "1mo";
/// The range of a price chart.
pub const HISTORY_RANGE: &str = "6mo";
/// The largest answer read (a month is a few kilobytes; six months a few tens).
pub const BODY_LIMIT: usize = 1024 * 1024;
/// The source credit every screen that shows these quotes carries (the reference app credits the same source).
pub const ATTRIBUTION: &str = "Quotes from Yahoo Finance \u{b7} delayed";
/// The `User-Agent` a quote request sends: generic, so it names no product, version or device.
pub const USER_AGENT: &str = "Mozilla/5.0";

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

/// The symbol as a screen shows it.
pub fn display_symbol(symbol: &str) -> String {
    symbol.to_uppercase()
}

/// The symbol escaped for a URL path ("^GSPC" is "%5EGSPC").
fn escape(symbol: &str) -> String {
    symbol.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-') { c.to_string() } else { format!("%{:02X}", c as u32) }).collect()
}

/// The address of `symbol`'s chart over `range` in daily bars: the symbol, the range and the interval, nothing else.
pub fn chart_url(symbol: &str, range: &str) -> String {
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

/// One daily bar of a price history: the session date (ISO, `YYYY-MM-DD`) and its close.
#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    pub date: String,
    pub close: f64,
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
    /// The close the change is measured from: the session before the latest one (the price itself with no history).
    pub fn previous_close(&self) -> f64 {
        match self.bars.len() {
            0 => self.price,
            1 => self.bars[0].close,
            n => self.bars[n - 2].close,
        }
    }

    /// When the service priced it, in the exchange's own time: "2026-10-22 16:00".
    pub fn stamp(&self) -> String {
        stamp(self.time + self.offset)
    }

    /// The daily closes, oldest first.
    pub fn closes(&self) -> Vec<f64> {
        self.bars.iter().map(|b| b.close).collect()
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
pub fn iso_date(local: i64) -> String {
    let (y, m, d) = civil(local.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// "2026-10-22 16:00" for local unix seconds.
pub fn stamp(local: i64) -> String {
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

/// The chart of `symbol` at `url` (`quote_url` or `history_url`). A symbol the service does not know is
/// `Ok(Err(message))`; a failed request (no network, the service down) is the outer `Err`.
pub fn fetch_chart(symbol: &str, url: &str, net: &dyn Fetch) -> io::Result<Result<Chart, String>> {
    match net.get(url, BODY_LIMIT) {
        Ok(body) => Ok(parse_chart(&String::from_utf8_lossy(&body))),
        // the service answers an unknown symbol with 404 (and its own error in the body, which the fetch drops)
        Err(e) if e.to_string().contains("status 404") => Ok(Err(format!("{} was not found.", display_symbol(symbol)))),
        Err(e) => Err(e),
    }
}

/// The fetcher for quote requests: HTTPS only (redirects too), the generic [`USER_AGENT`], and no cookies kept or
/// sent, so the service gets the symbol and nothing that tells one person's requests from another's. The client is
/// built on first use.
#[derive(Default)]
pub struct QuoteHttp {
    client: OnceLock<Client>,
    /// the connection maker; `None` is the real one (DNS, TCP and TLS)
    transport: Option<Arc<dyn Transport>>,
}

impl QuoteHttp {
    /// A fetcher over `transport` (tests hand in a canned one).
    pub fn with_transport(transport: Arc<dyn Transport>) -> Self {
        QuoteHttp { client: OnceLock::new(), transport: Some(transport) }
    }

    fn client(&self) -> &Client {
        self.client.get_or_init(|| {
            let transport = self.transport.clone().unwrap_or_else(|| Arc::new(Tls::new()) as Arc<dyn Transport>);
            Client::new(transport, Jar::default()).without_cookies().require_https()
        })
    }
}

impl Fetch for QuoteHttp {
    fn get(&self, url: &str, limit: usize) -> io::Result<Vec<u8>> {
        let r = self.client().get_with_headers(url, &[("User-Agent", USER_AGENT)], limit)?;
        if r.status != 200 {
            return Err(io::Error::other(format!("{url}: status {}", r.status)));
        }
        Ok(r.body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deildu::http::tests::{ok, Fake};

    /// A trimmed chart answer as the service sends it (three sessions, one without a close).
    const CHART: &str = r#"{"chart":{"result":[{"meta":{"currency":"USD","symbol":"AAPL","regularMarketTime":1792699201,
        "gmtoffset":-14400,"regularMarketPrice":348.22,"shortName":"Apple Inc.","longName":"Apple Inc."},
        "timestamp":[1792503000,1792589400,1792600000,1792675800],
        "indicators":{"quote":[{"close":[339.5,343.99,null,348.22]}],"adjclose":[{"adjclose":[339.5,343.99,null,348.22]}]}}],
        "error":null}}"#;

    const UNKNOWN: &str = r#"{"chart":{"result":null,"error":{"code":"Not Found","description":"No data found, symbol may be delisted"}}}"#;

    #[test]
    fn symbols_are_normalised_to_the_service_form() {
        assert_eq!(normalise_symbol(" aapl "), Some("AAPL".to_string()));
        assert_eq!(normalise_symbol("nvda.us"), Some("NVDA".to_string()), "the old Stooq form still reads");
        assert_eq!(normalise_symbol("^spx"), Some("^GSPC".to_string()));
        assert_eq!(normalise_symbol("brk-b"), Some("BRK-B".to_string()));
        assert_eq!(normalise_symbol("vod.l"), Some("VOD.L".to_string()));
        assert_eq!(normalise_symbol(""), None);
        assert_eq!(normalise_symbol("a b"), None);
        assert_eq!(normalise_symbol("waytoolongsymbol"), None);
        assert_eq!(display_symbol("aapl"), "AAPL");
    }

    #[test]
    fn an_address_carries_the_symbol_the_range_and_the_interval_only() {
        assert_eq!(quote_url("AAPL"), "https://query1.finance.yahoo.com/v8/finance/chart/AAPL?range=1mo&interval=1d");
        assert_eq!(history_url("^GSPC"), "https://query1.finance.yahoo.com/v8/finance/chart/%5EGSPC?range=6mo&interval=1d");
        let query = quote_url("AAPL").split_once('?').unwrap().1.to_string();
        let keys: Vec<&str> = query.split('&').map(|p| p.split('=').next().unwrap()).collect();
        assert_eq!(keys, ["range", "interval"], "no crumb, key or identifier");
    }

    #[test]
    fn a_chart_answer_gives_the_price_the_previous_close_the_name_and_the_closes() {
        let chart = parse_chart(CHART).unwrap();
        assert_eq!(chart.symbol, "AAPL");
        assert_eq!(chart.name, "Apple Inc.");
        assert_eq!(chart.closes(), [339.5, 343.99, 348.22], "a session without a close is left out");
        assert_eq!(chart.bars[0].date, "2026-10-20", "dates are the exchange's own days");
        assert_eq!((chart.previous_close(), chart.price), (343.99, 348.22));
        assert_eq!(chart.stamp(), "2026-10-22 16:00");
    }

    #[test]
    fn the_services_own_error_is_the_message() {
        assert_eq!(parse_chart(UNKNOWN), Err("No data found, symbol may be delisted".to_string()));
        assert!(parse_chart("<html>busy</html>").is_err());
    }

    #[test]
    fn civil_dates_follow_the_calendar() {
        assert_eq!(iso_date(0), "1970-01-01");
        assert_eq!(iso_date(951_782_400), "2000-02-29");
        assert_eq!(stamp(1_792_699_201 - 14_400), "2026-10-22 16:00");
    }

    /// The privacy promise, checked on the bytes that leave: the generic User-Agent, no Cookie header even after the
    /// service set one, and nothing in the request line beyond the symbol's address.
    #[test]
    fn a_quote_request_sends_the_symbol_and_nothing_that_identifies_the_person() {
        let set_cookie = "HTTP/1.1 200 OK\r\nSet-Cookie: A3=tracker; Domain=.yahoo.com; Secure\r\nContent-Length: 2\r\n\r\n{}";
        let fake = Fake::new(vec![set_cookie, &ok(CHART)]);
        let net = QuoteHttp::with_transport(fake.clone());
        assert!(net.get(&quote_url("AAPL"), BODY_LIMIT).is_ok());
        let body = net.get(&quote_url("AAPL"), BODY_LIMIT).unwrap();
        assert!(parse_chart(&String::from_utf8_lossy(&body)).is_ok());
        let requests = fake.requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 2);
        for req in &requests {
            let lines: Vec<&str> = req.lines().collect();
            assert_eq!(lines[0], "GET /v8/finance/chart/AAPL?range=1mo&interval=1d HTTP/1.1");
            assert!(lines.contains(&"User-Agent: Mozilla/5.0"), "{req}");
            assert_eq!(lines.iter().filter(|l| l.to_ascii_lowercase().starts_with("user-agent:")).count(), 1, "one generic agent: {req}");
            assert!(!req.to_ascii_lowercase().contains("cookie:"), "no cookie is sent: {req}");
            assert!(!req.to_ascii_lowercase().contains("bifrost"), "no product name: {req}");
            assert!(!req.contains("crumb"), "{req}");
        }
    }

    #[test]
    fn a_quote_request_refuses_plain_http() {
        let fake = Fake::new(vec![]);
        let net = QuoteHttp::with_transport(fake.clone());
        assert!(net.get("http://query1.finance.yahoo.com/v8/finance/chart/AAPL", BODY_LIMIT).is_err());
        assert!(fake.requests.lock().unwrap().is_empty());
    }

    #[test]
    fn an_unknown_symbol_is_a_message_and_an_outage_an_error() {
        struct NotFound;
        impl Fetch for NotFound {
            fn get(&self, url: &str, _limit: usize) -> io::Result<Vec<u8>> {
                Err(io::Error::other(format!("{url}: status 404")))
            }
        }
        struct Down;
        impl Fetch for Down {
            fn get(&self, _url: &str, _limit: usize) -> io::Result<Vec<u8>> {
                Err(io::Error::other("offline"))
            }
        }
        assert_eq!(fetch_chart("NOPE", &quote_url("NOPE"), &NotFound).unwrap(), Err("NOPE was not found.".to_string()));
        assert!(fetch_chart("AAPL", &quote_url("AAPL"), &Down).is_err());
    }
}
