//! The app's saved files in its data directory: the watchlist, the last good quotes and one history per symbol, as
//! the small CSV files `data` writes and reads, and the file's age says how old the data is. Writes go to a temporary file first, so a crash never leaves half a file.

use crate::data;
use std::path::PathBuf;

const WATCHLIST_FILE: &str = "watchlist.txt";
const QUOTES_FILE: &str = "quotes.csv";

/// The data directory, cloned into background jobs.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Store {
        Store { dir: dir.into() }
    }

    /// The saved watchlist (empty when there is none).
    pub fn load_watchlist(&self) -> Vec<String> {
        decode_watchlist(&self.read(WATCHLIST_FILE).unwrap_or_default())
    }

    pub fn save_watchlist(&self, symbols: &[String]) {
        self.write(WATCHLIST_FILE, &encode_watchlist(symbols));
    }

    /// The last good quote answer and its age in seconds.
    pub fn load_quotes(&self) -> Option<(String, u64)> {
        self.read_with_age(QUOTES_FILE)
    }

    pub fn save_quotes(&self, raw: &str) {
        self.write(QUOTES_FILE, raw);
    }

    /// The last good history answer of `symbol` and its age in seconds.
    pub fn load_history(&self, symbol: &str) -> Option<(String, u64)> {
        self.read_with_age(&history_file(symbol))
    }

    pub fn save_history(&self, symbol: &str, raw: &str) {
        self.write(&history_file(symbol), raw);
    }

    fn read(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.dir.join(name)).ok()
    }

    fn read_with_age(&self, name: &str) -> Option<(String, u64)> {
        let path = self.dir.join(name);
        let text = std::fs::read_to_string(&path).ok()?;
        let age = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?.elapsed().map_or(0, |d| d.as_secs());
        Some((text, age))
    }

    /// Writes `text` through a temporary file. A failed write keeps the old file, and the app carries on.
    fn write(&self, name: &str, text: &str) {
        let _ = std::fs::create_dir_all(&self.dir);
        let path = self.dir.join(name);
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(tmp, path);
        }
    }
}

/// The watchlist file: one symbol per line.
pub fn encode_watchlist(symbols: &[String]) -> String {
    symbols.iter().map(|s| format!("{s}\n")).collect()
}

/// The symbols of a watchlist file. Lines that are not valid symbols are dropped, so a hand-edited file cannot
/// break the list.
pub fn decode_watchlist(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for symbol in text.lines().filter_map(data::normalise_symbol) {
        if !out.contains(&symbol) {
            out.push(symbol);
        }
    }
    out
}

/// The history file name of a symbol. A caret, which only index symbols carry, becomes an underscore; valid symbols
/// never contain an underscore, so two symbols never share a file.
pub fn history_file(symbol: &str) -> String {
    let safe: String = symbol.chars().map(|c| if c == '^' { '_' } else { c }).collect();
    format!("history-{safe}.csv")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_watchlist_round_trips_in_order() {
        let list = vec!["AAPL".to_string(), "^GSPC".to_string(), "NVDA".to_string()];
        assert_eq!(decode_watchlist(&encode_watchlist(&list)), list);
        assert!(decode_watchlist("").is_empty());
    }

    #[test]
    fn a_hand_edited_watchlist_keeps_only_valid_unique_symbols() {
        let got = decode_watchlist("AAPL\n\n bad symbol \nmsft.us\naapl.us\n");
        assert_eq!(got, ["AAPL", "MSFT"], "the old Stooq form reads as the service's form");
    }

    #[test]
    fn history_files_have_safe_names_per_symbol() {
        assert_eq!(history_file("AAPL"), "history-AAPL.csv");
        assert_eq!(history_file("^GSPC"), "history-_GSPC.csv");
        assert_ne!(history_file("A.US"), history_file("A-US"), "different symbols never share a file");
    }
}
