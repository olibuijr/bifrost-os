//! The quote and price-history service: symbol rules, request addresses and the parsers for the service's CSV
//! answers. The service is Stooq (https://stooq.com), a free public source of delayed quotes and daily history.
//! The network is passed in as a `Fetch`, so the parsers and the fetch functions can be checked without it.

use crate::model::{Bar, Quote};
use springboard::shell::music::radio::Fetch;
use std::io;

/// The latest quotes for several symbols in one CSV answer.
pub const QUOTE_API: &str = "https://stooq.com/q/l/";
/// The daily history of one symbol as CSV, oldest session first.
pub const HISTORY_API: &str = "https://stooq.com/q/d/l/";
/// The largest answer the app reads (a quote page is a few hundred bytes; a history is a few hundred kilobytes).
pub const BODY_LIMIT: usize = 1024 * 1024;
/// The sessions the chart keeps (about four months of trading days).
pub const HISTORY_KEEP: usize = 90;

/// "AAPL" becomes "aapl.us"; a symbol with a market suffix or an index caret keeps its own form. Returns `None` for
/// an empty or malformed symbol.
pub fn normalise_symbol(raw: &str) -> Option<String> {
    let s = raw.trim().to_lowercase();
    if s.is_empty() || s.len() > 12 || !s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '^' | '-')) {
        return None;
    }
    Some(if s.contains('.') || s.starts_with('^') { s } else { format!("{s}.us") })
}

/// "aapl.us" becomes "AAPL" for the screen.
pub fn display_symbol(symbol: &str) -> String {
    symbol.strip_suffix(".us").unwrap_or(symbol).to_uppercase()
}

/// The address of the latest quotes for `symbols`.
pub fn quote_url(symbols: &[String]) -> String {
    format!("{QUOTE_API}?s={}&f=sd2t2ohlcv&h&e=csv", symbols.join("+"))
}

/// The address of the daily history of one symbol.
pub fn history_url(symbol: &str) -> String {
    format!("{HISTORY_API}?s={symbol}&i=d")
}

/// The quote rows of a quote CSV (`Symbol,Date,Time,Open,High,Low,Close,Volume`). Rows the service could not price
/// (`N/D`) are skipped.
pub fn parse_quotes(csv: &str) -> Vec<Quote> {
    csv.lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.trim().split(',').collect();
            if f.len() < 7 || f[1].is_empty() {
                return None;
            }
            Some(Quote {
                symbol: f[0].to_lowercase(),
                stamp: format!("{} {}", f[1], f[2]),
                open: f[3].parse().ok()?,
                close: f[6].parse().ok()?,
            })
        })
        .collect()
}

/// The bars of a history CSV (`Date,Open,High,Low,Close,Volume`), oldest first. Rows without a date or a close are
/// skipped.
pub fn parse_history(csv: &str) -> Vec<Bar> {
    let mut bars: Vec<Bar> = csv
        .lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.trim().split(',').collect();
            if f.len() < 5 || !is_iso_date(f[0]) {
                return None;
            }
            Some(Bar { date: f[0].to_string(), close: f[4].parse().ok()? })
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

/// Fetches the latest quotes for `symbols`. Returns the raw answer (to cache) and the parsed rows. An answer with no
/// priced rows is an error, so an outage page is never shown as an empty list.
pub fn fetch_quotes(symbols: &[String], net: &dyn Fetch) -> io::Result<(String, Vec<Quote>)> {
    let body = String::from_utf8_lossy(&net.get(&quote_url(symbols), BODY_LIMIT)?).into_owned();
    let quotes = parse_quotes(&body);
    if quotes.is_empty() {
        return Err(io::Error::other("no quotes in the answer"));
    }
    Ok((body, quotes))
}

/// Fetches the daily history of `symbol`. Returns the raw answer (to cache) and the bars, oldest first.
pub fn fetch_history(symbol: &str, net: &dyn Fetch) -> io::Result<(String, Vec<Bar>)> {
    let body = String::from_utf8_lossy(&net.get(&history_url(symbol), BODY_LIMIT)?).into_owned();
    let bars = parse_history(&body);
    if bars.is_empty() {
        return Err(io::Error::other("no history in the answer"));
    }
    Ok((body, bars))
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUOTES: &str = "Symbol,Date,Time,Open,High,Low,Close,Volume\n\
        AAPL.US,2026-10-02,22:00:09,100.0,111,99,110.0,5\n\
        XYZ.US,N/D,N/D,N/D,N/D,N/D,N/D,N/D\n\
        MSFT.US,2026-10-02,22:00:10,400.5,401,398,399.5,9\n";

    const HISTORY: &str = "Date,Open,High,Low,Close,Volume\n\
        2026-10-03,12,13,11,12.5,100\n\
        2026-10-01,10,11,9,10.5,100\n\
        bad row\n\
        2026-10-02,11,12,10,,100\n\
        2026-10-02,11,12,10,11.25,100\n";

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

    #[test]
    fn symbols_are_normalised_to_the_service_form() {
        assert_eq!(normalise_symbol(" AAPL "), Some("aapl.us".to_string()));
        assert_eq!(normalise_symbol("nvda.us"), Some("nvda.us".to_string()));
        assert_eq!(normalise_symbol("^spx"), Some("^spx".to_string()));
        assert_eq!(normalise_symbol(""), None);
        assert_eq!(normalise_symbol("a b"), None);
        assert_eq!(normalise_symbol("waytoolongsymbol"), None);
        assert_eq!(display_symbol("aapl.us"), "AAPL");
        assert_eq!(display_symbol("^spx"), "^SPX");
    }

    #[test]
    fn addresses_name_the_symbols_and_the_csv_form() {
        assert_eq!(quote_url(&["a.us".into(), "b.us".into()]), "https://stooq.com/q/l/?s=a.us+b.us&f=sd2t2ohlcv&h&e=csv");
        assert_eq!(history_url("aapl.us"), "https://stooq.com/q/d/l/?s=aapl.us&i=d");
    }

    #[test]
    fn quote_rows_parse_with_their_stamp_and_unpriced_rows_are_skipped() {
        let q = parse_quotes(QUOTES);
        assert_eq!(q.len(), 2);
        assert_eq!(q[0].symbol, "aapl.us");
        assert_eq!(q[0].stamp, "2026-10-02 22:00:09");
        assert!((q[0].change_pct() - 10.0).abs() < 1e-9);
        assert_eq!(q[1].close, 399.5);
    }

    #[test]
    fn history_parses_oldest_first_and_skips_bad_rows() {
        let bars = parse_history(HISTORY);
        let dates: Vec<&str> = bars.iter().map(|b| b.date.as_str()).collect();
        assert_eq!(dates, ["2026-10-01", "2026-10-02", "2026-10-03"]);
        assert_eq!(bars[1].close, 11.25, "the row with an empty close is skipped");
    }

    #[test]
    fn tail_keeps_the_newest_bars() {
        let bars = parse_history(HISTORY);
        let got = tail(bars.clone(), 2);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].date, "2026-10-02");
        assert_eq!(tail(bars.clone(), 10).len(), 3);
        assert_eq!(tail(Vec::new(), 3).len(), 0);
    }

    #[test]
    fn a_good_answer_is_returned_with_its_raw_text() {
        let (raw, quotes) = fetch_quotes(&["aapl.us".into()], &Canned(QUOTES)).unwrap();
        assert_eq!(raw, QUOTES);
        assert_eq!(quotes.len(), 2);
        let (_, bars) = fetch_history("aapl.us", &Canned(HISTORY)).unwrap();
        assert_eq!(bars.len(), 3);
    }

    #[test]
    fn an_answer_with_nothing_priced_is_an_error_not_an_empty_list() {
        assert!(fetch_quotes(&["xyz.us".into()], &Canned("Symbol,Date\nXYZ.US,N/D,N/D,N/D,N/D,N/D,N/D,N/D\n")).is_err());
        assert!(fetch_history("xyz.us", &Canned("<html>limit reached</html>")).is_err());
    }

    #[test]
    fn a_network_failure_is_passed_on() {
        assert!(fetch_quotes(&["aapl.us".into()], &Down).is_err());
        assert!(fetch_history("aapl.us", &Down).is_err());
    }
}
