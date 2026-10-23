//! The watchlist and the price data: plain values and the rules for them (order, selection, limits, change
//! figures and chart scaling). Nothing here draws, touches the disk or talks to the network.

/// The most symbols a watchlist holds.
pub const MAX_SYMBOLS: usize = 30;

/// One quote from the quote service: the latest price against the previous session's close.
#[derive(Clone, Debug, PartialEq)]
pub struct Quote {
    /// in the service's form, e.g. "AAPL" or "^GSPC"
    pub symbol: String,
    /// the company or index name, e.g. "Apple Inc." (empty when the service gave none)
    pub name: String,
    /// when the service priced it, in the exchange's time, e.g. "2026-10-22 16:00"
    pub stamp: String,
    /// the price the change is measured from: the previous session's close
    pub open: f64,
    /// the latest price
    pub close: f64,
    /// a month of daily closes, oldest first, for the sidebar's sparkline
    pub spark: Vec<f64>,
}

impl Quote {
    /// The change since the previous close, in price units.
    pub fn change(&self) -> f64 {
        self.close - self.open
    }

    /// The change since the previous close, in per cent (0 when that close is 0).
    pub fn change_pct(&self) -> f64 {
        if self.open == 0.0 { 0.0 } else { self.change() / self.open * 100.0 }
    }
}

/// One daily bar of a price history: the session date (ISO, `YYYY-MM-DD`) and its close.
#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    pub date: String,
    pub close: f64,
}

/// Why a symbol could not join the watchlist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddError {
    /// The symbol is already in the list.
    Duplicate,
    /// The list has `MAX_SYMBOLS` entries.
    Full,
}

impl AddError {
    /// The sentence the app shows for this error.
    pub fn message(self) -> &'static str {
        match self {
            AddError::Duplicate => "That symbol is already in your watchlist.",
            AddError::Full => "Your watchlist is full. Remove a symbol to add another.",
        }
    }
}

/// The ordered list of symbols and the selected row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Watchlist {
    symbols: Vec<String>,
    selected: usize,
}

impl Watchlist {
    /// A list from saved symbols. Repeats are dropped (the first one stays), and the list stops at `MAX_SYMBOLS`.
    pub fn new(saved: Vec<String>) -> Watchlist {
        let mut symbols: Vec<String> = Vec::new();
        for s in saved {
            if symbols.len() < MAX_SYMBOLS && !s.is_empty() && !symbols.contains(&s) {
                symbols.push(s);
            }
        }
        Watchlist { symbols, selected: 0 }
    }

    pub fn symbols(&self) -> &[String] {
        &self.symbols
    }

    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }

    pub fn contains(&self, symbol: &str) -> bool {
        self.symbols.iter().any(|s| s == symbol)
    }

    /// The selected row's index (0 when the list is empty).
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// The selected symbol, or `None` when the list is empty.
    pub fn selected_symbol(&self) -> Option<&str> {
        self.symbols.get(self.selected).map(String::as_str)
    }

    /// Selects row `index`; an index past the end selects nothing and returns false.
    pub fn select(&mut self, index: usize) -> bool {
        if index < self.symbols.len() {
            self.selected = index;
            true
        } else {
            false
        }
    }

    /// Appends `symbol` (already normalised), selects it and returns its index.
    pub fn add(&mut self, symbol: &str) -> Result<usize, AddError> {
        if self.contains(symbol) {
            return Err(AddError::Duplicate);
        }
        if self.symbols.len() >= MAX_SYMBOLS {
            return Err(AddError::Full);
        }
        self.symbols.push(symbol.to_string());
        self.selected = self.symbols.len() - 1;
        Ok(self.selected)
    }

    /// Removes row `index` and returns its symbol. The selection stays on the same symbol where it still exists,
    /// otherwise it moves to the row now at that place (or the last row).
    pub fn remove(&mut self, index: usize) -> Option<String> {
        if index >= self.symbols.len() {
            return None;
        }
        let gone = self.symbols.remove(index);
        if self.symbols.is_empty() {
            self.selected = 0;
        } else if index < self.selected {
            self.selected -= 1;
        } else if self.selected >= self.symbols.len() {
            self.selected = self.symbols.len() - 1;
        }
        Some(gone)
    }

    /// Swaps row `index` with the row above it. The selection follows the moved symbol. False at the top.
    pub fn move_up(&mut self, index: usize) -> bool {
        if index == 0 || index >= self.symbols.len() {
            return false;
        }
        self.swap_rows(index - 1, index);
        true
    }

    /// Swaps row `index` with the row below it. The selection follows the moved symbol. False at the bottom.
    pub fn move_down(&mut self, index: usize) -> bool {
        if index + 1 >= self.symbols.len() {
            return false;
        }
        self.swap_rows(index, index + 1);
        true
    }

    fn swap_rows(&mut self, a: usize, b: usize) {
        self.symbols.swap(a, b);
        if self.selected == a {
            self.selected = b;
        } else if self.selected == b {
            self.selected = a;
        }
    }
}

/// The low and high of a series, or `None` when it is empty.
pub fn value_range(values: &[f64]) -> Option<(f64, f64)> {
    let first = *values.first()?;
    Some(values.iter().fold((first, first), |(lo, hi), &v| (lo.min(v), hi.max(v))))
}

/// The low and high widened by 4 per cent of their span on each side, so the line does not touch the plot edges.
/// A flat series gets a band of one unit around its value.
pub fn padded_range(lo: f64, hi: f64) -> (f64, f64) {
    if hi <= lo {
        return (lo - 0.5, hi + 0.5);
    }
    let pad = (hi - lo) * 0.04;
    (lo - pad, hi + pad)
}

/// The per cent change from the first close to the last one (`None` with fewer than two bars or a zero first close).
#[cfg(test)]
pub fn period_change_pct(bars: &[Bar]) -> Option<f64> {
    let (first, last) = (bars.first()?, bars.last()?);
    if bars.len() < 2 || first.close == 0.0 {
        return None;
    }
    Some((last.close - first.close) / first.close * 100.0)
}

/// The vertical position of `value` in a plot that runs from `top` (the high) down `height` pixels (the low).
/// `lo` and `hi` are the padded range from `padded_range`. A degenerate range puts the value mid-plot.
pub fn plot_y(value: f64, lo: f64, hi: f64, top: f32, height: f32) -> f32 {
    if hi <= lo {
        return top + height / 2.0;
    }
    let t = ((value - lo) / (hi - lo)) as f32;
    top + height - t * height
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(list: &[&str]) -> Watchlist {
        Watchlist::new(list.iter().map(|x| x.to_string()).collect())
    }

    fn bar(close: f64) -> Bar {
        Bar { date: "2026-10-01".into(), close }
    }

    #[test]
    fn saved_symbols_keep_their_order_and_drop_repeats() {
        let w = s(&["aapl.us", "msft.us", "aapl.us", "", "nvda.us"]);
        assert_eq!(w.symbols(), ["aapl.us", "msft.us", "nvda.us"]);
        assert_eq!(w.selected_symbol(), Some("aapl.us"));
    }

    #[test]
    fn a_saved_list_stops_at_the_limit() {
        let many: Vec<String> = (0..MAX_SYMBOLS + 5).map(|i| format!("s{i}.us")).collect();
        assert_eq!(Watchlist::new(many).len(), MAX_SYMBOLS);
    }

    #[test]
    fn add_appends_selects_the_new_row_and_rejects_repeats() {
        let mut w = s(&["aapl.us"]);
        assert_eq!(w.add("nvda.us"), Ok(1));
        assert_eq!(w.selected_symbol(), Some("nvda.us"));
        assert_eq!(w.add("aapl.us"), Err(AddError::Duplicate));
        assert_eq!(w.len(), 2);
    }

    #[test]
    fn add_refuses_past_the_limit() {
        let mut w = Watchlist::new((0..MAX_SYMBOLS).map(|i| format!("s{i}.us")).collect());
        assert_eq!(w.add("extra.us"), Err(AddError::Full));
        assert_eq!(w.len(), MAX_SYMBOLS);
        assert!(AddError::Full.message().contains("full"));
    }

    #[test]
    fn remove_keeps_the_selected_symbol_when_a_row_above_it_goes() {
        let mut w = s(&["a.us", "b.us", "c.us"]);
        w.select(2);
        assert_eq!(w.remove(0), Some("a.us".to_string()));
        assert_eq!(w.selected_symbol(), Some("c.us"));
    }

    #[test]
    fn remove_moves_the_selection_to_the_row_that_takes_its_place() {
        let mut w = s(&["a.us", "b.us", "c.us"]);
        w.select(1);
        w.remove(1);
        assert_eq!(w.selected_symbol(), Some("c.us"));
        w.remove(1);
        assert_eq!(w.selected_symbol(), Some("a.us"), "the last row when the selected one was last");
        assert_eq!(w.remove(9), None);
    }

    #[test]
    fn removing_the_last_symbol_empties_the_list() {
        let mut w = s(&["a.us"]);
        assert_eq!(w.remove(0), Some("a.us".to_string()));
        assert!(w.is_empty());
        assert_eq!(w.selected_symbol(), None);
        assert!(!w.select(0));
    }

    #[test]
    fn move_up_and_down_swap_rows_and_the_selection_follows() {
        let mut w = s(&["a.us", "b.us", "c.us"]);
        w.select(1);
        assert!(w.move_down(1));
        assert_eq!(w.symbols(), ["a.us", "c.us", "b.us"]);
        assert_eq!(w.selected_symbol(), Some("b.us"));
        assert!(w.move_up(2));
        assert_eq!(w.symbols(), ["a.us", "b.us", "c.us"]);
        assert_eq!(w.selected_symbol(), Some("b.us"));
    }

    #[test]
    fn moves_stop_at_the_ends() {
        let mut w = s(&["a.us", "b.us"]);
        assert!(!w.move_up(0));
        assert!(!w.move_down(1));
        assert!(!w.move_up(7));
        assert_eq!(w.symbols(), ["a.us", "b.us"]);
    }

    #[test]
    fn quote_change_is_against_the_open() {
        let q = Quote { symbol: "AAPL".into(), name: String::new(), stamp: String::new(), open: 100.0, close: 110.0, spark: vec![] };
        assert!((q.change() - 10.0).abs() < 1e-9);
        assert!((q.change_pct() - 10.0).abs() < 1e-9);
        let flat = Quote { open: 0.0, ..q };
        assert_eq!(flat.change_pct(), 0.0);
    }

    #[test]
    fn value_range_and_padding() {
        assert_eq!(value_range(&[]), None);
        assert_eq!(value_range(&[3.0, -1.0, 2.0]), Some((-1.0, 3.0)));
        let (lo, hi) = padded_range(0.0, 100.0);
        assert!((lo + 4.0).abs() < 1e-9 && (hi - 104.0).abs() < 1e-9);
        let (lo, hi) = padded_range(5.0, 5.0);
        assert!(lo < 5.0 && hi > 5.0, "a flat series still gets a band");
    }

    #[test]
    fn period_change_runs_from_the_first_bar_to_the_last() {
        assert_eq!(period_change_pct(&[bar(50.0)]), None);
        assert_eq!(period_change_pct(&[]), None);
        let got = period_change_pct(&[bar(50.0), bar(40.0), bar(60.0)]).unwrap();
        assert!((got - 20.0).abs() < 1e-9);
        assert_eq!(period_change_pct(&[bar(0.0), bar(1.0)]), None);
    }

    #[test]
    fn plot_y_puts_the_high_at_the_top_and_the_low_at_the_bottom() {
        assert_eq!(plot_y(10.0, 0.0, 10.0, 100.0, 50.0), 100.0);
        assert_eq!(plot_y(0.0, 0.0, 10.0, 100.0, 50.0), 150.0);
        assert_eq!(plot_y(5.0, 0.0, 10.0, 100.0, 50.0), 125.0);
        assert_eq!(plot_y(5.0, 5.0, 5.0, 100.0, 50.0), 125.0, "degenerate range is mid-plot");
    }
}
