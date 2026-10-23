//! The Stocks screen's state and input: the watchlist, the feed of quotes and histories, the add field and the list's
//! scroll position. Fetches run on the global utility queue; their results wait in a mailbox that `tick` drains on
//! the app loop, so the UI thread never waits on the network or the disk. Drawing lives in `view`.

use crate::data;
use crate::feed::{Done, Feed};
use crate::model::Watchlist;
use crate::store::Store;
use crate::view::{self, Hit};
use bifrost_app::dispatch::{DispatchQueue, Qos};
use bifrost_app::kit::ui::{MAC_BODY_PT, SPACE_M};
use bifrost_app::kit::{Event, Scroller, TextCache, Ui};
use springboard::shell::keyboard::Out;
use springboard::shell::music::radio::Fetch;
use springboard::shell::textfield::TextField;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use bifrost_app::ContextItem;

/// The ids of a row's context menu.
const CTX_UP: u32 = 1;
const CTX_DOWN: u32 = 2;
const CTX_REMOVE: u32 = 3;

/// How often an open app checks whether a fetch is due (seconds).
const CHECK_S: f64 = 60.0;
/// How often a fetch in flight is looked for, in milliseconds.
const FETCH_POLL_MS: f64 = 150.0;

/// Unix seconds now.
pub fn now_unix() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// Hands a finished job to the app loop.
fn post(done: &Arc<Mutex<Vec<Done>>>, result: Done) {
    done.lock().unwrap_or_else(|e| e.into_inner()).push(result);
    DispatchQueue::main().async_(|| {});
}

pub struct Stocks {
    pub list: Watchlist,
    pub feed: Feed,
    /// A line under the add field: why an add failed, or what changed.
    pub note: Option<String>,
    pub field: TextField,
    pub scroll: Scroller,
    /// Rendered text, kept between frames.
    pub labels: TextCache,
    /// The window size in pixels.
    pub size: (usize, usize),
    /// The hit under a press that has not been released yet.
    pub pressed: Option<Hit>,
    /// A press in the list that may turn into a scroll.
    scrolling: bool,
    pub dirty: bool,
    store: Store,
    net: Arc<dyn Fetch>,
    done: Arc<Mutex<Vec<Done>>>,
    next_check: f64,
    /// The row a context menu was opened on, until its answer comes back.
    context_row: Option<usize>,
}

impl Stocks {
    /// The screen with the saved watchlist and quotes. Nothing is fetched yet; `start` does that.
    pub fn new(ui: &Ui, dir: PathBuf, net: Arc<dyn Fetch>) -> Stocks {
        let store = Store::new(dir);
        let list = Watchlist::new(store.load_watchlist());
        let now = now_unix();
        let mut feed = Feed::default();
        if let Some((raw, age)) = store.load_quotes() {
            feed.load_quotes(&raw, now - age as f64);
        }
        feed.quotes_wanted = list.symbols().iter().any(|s| !feed.quotes.contains_key(s));
        let mut field = TextField::new(ui, "Add symbol, e.g. AAPL", MAC_BODY_PT, false, 16);
        field.focused = false;
        Stocks {
            list,
            feed,
            note: None,
            field,
            scroll: Scroller::new(),
            labels: TextCache::default(),
            size: (0, 0),
            pressed: None,
            scrolling: false,
            dirty: true,
            store,
            net,
            done: Arc::new(Mutex::new(Vec::new())),
            next_check: 0.0,
            context_row: None,
        }
    }

    /// Called when the app starts: loads the selected chart from the cache and asks for whatever is due.
    pub fn start(&mut self, t: f64) {
        self.show_selected(t);
        self.maintain(t);
    }

    pub fn set_size(&mut self, w: usize, h: usize) {
        if self.size != (w, h) {
            self.size = (w, h);
            self.dirty = true;
        }
    }

    /// Whether a quote or history answer is still out, or one waits in the mailbox.
    pub fn fetching(&self) -> bool {
        self.feed.is_quotes_pending()
            || self.list.symbols().iter().any(|s| self.feed.is_history_pending(s))
            || !self.done.lock().map_or(true, |d| d.is_empty())
    }

    /// Applies finished jobs and asks for what is due. True when the screen needs a redraw.
    pub fn tick(&mut self, t: f64) -> bool {
        if self.pump() {
            self.dirty = true;
        }
        self.maintain(t);
        self.dirty
    }

    /// When the loop should next call `tick`: the next check, the field's caret blink while it is focused, and soon
    /// while a fetch is out, so its answer shows when it lands (the worker's main-queue wake does not always reach
    /// the loop; before, a new symbol's price showed only after the next input).
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        let mut due = Some(self.next_check.max(t));
        if self.fetching() {
            due = due.map(|d| d.min(t + FETCH_POLL_MS));
        }
        if self.field.focused {
            if let Some(blink) = self.field.next_blink(t) {
                due = Some(due.map_or(blink, |d| d.min(blink)));
            }
        }
        due
    }

    /// Adds a typed symbol to the watchlist, selects it and asks for its quote and chart. False (with a note) when
    /// the text is not a symbol, the symbol is already listed, or the list is full.
    pub fn add(&mut self, ui: &Ui, raw: &str, t: f64) -> bool {
        let Some(symbol) = data::normalise_symbol(raw) else {
            self.note = Some("Use a symbol such as AAPL, or ^GSPC for an index.".to_string());
            return false;
        };
        if let Err(e) = self.list.add(&symbol) {
            self.note = Some(e.message().to_string());
            return false;
        }
        self.store.save_watchlist(self.list.symbols());
        self.note = Some(format!("Added {}.", data::display_symbol(&symbol)));
        self.feed.quotes_wanted = true;
        self.field.set_text(ui, "");
        self.dirty = true;
        self.show_selected(t);
        self.maintain(t);
        true
    }

    /// Removes row `index` from the watchlist.
    pub fn remove(&mut self, index: usize, t: f64) {
        if let Some(symbol) = self.list.remove(index) {
            self.store.save_watchlist(self.list.symbols());
            self.note = Some(format!("Removed {}.", data::display_symbol(&symbol)));
            self.show_selected(t);
            self.dirty = true;
        }
    }

    /// Moves row `index` one place up (`up`) or down, and saves the new order.
    pub fn move_row(&mut self, index: usize, up: bool) {
        let moved = if up { self.list.move_up(index) } else { self.list.move_down(index) };
        if moved {
            self.store.save_watchlist(self.list.symbols());
            self.dirty = true;
        }
    }

    /// Selects row `index` and shows its chart.
    pub fn select(&mut self, index: usize, t: f64) {
        if self.list.select(index) {
            self.dirty = true;
            self.show_selected(t);
        }
    }

    /// Keyboard input for the add field: text editing, Return adds the symbol, Escape leaves the field.
    pub fn key(&mut self, ui: &Ui, out: Out, t: f64) {
        self.dirty = true;
        if !self.field.focused {
            // Delete (or Backspace) removes the selected symbol, as Remove from Watchlist does
            if matches!(out, Out::Backspace | Out::Named("Delete")) && !self.list.is_empty() {
                self.remove(self.list.selected(), t);
            }
            return;
        }
        match out {
            Out::Hide => self.field.focused = false,
            Out::Enter => {
                let raw = self.field.text().to_string();
                self.add(ui, &raw, t);
            }
            other => {
                self.field.edit(ui, &other, t);
            }
        }
    }

    /// Pointer input in pixels. A press on a row, a button or the field acts on release, unless the press moved into
    /// a scroll of the list.
    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) {
        self.dirty = true;
        match ev {
            Event::Down(x, y) => {
                self.pressed = self.hit(ui, x, y);
                self.scrolling = view::in_list(ui, self.size, x, y);
                if self.scrolling {
                    self.scroll.down(y, t);
                }
            }
            Event::Move(_, y) => {
                if self.scrolling {
                    self.scroll.moved(y, t);
                    if self.scroll.dragging() {
                        self.pressed = None;
                    }
                }
            }
            Event::Up(x, y) => {
                let tap = if self.scrolling { self.scroll.up(y, t) } else { true };
                self.scrolling = false;
                let pressed = self.pressed.take();
                if tap {
                    if let Some(target) = pressed.filter(|p| self.hit(ui, x, y) == Some(*p)) {
                        self.activate(ui, target, x, t);
                    }
                }
            }
            _ => {}
        }
    }

    /// The control under a pixel position.
    pub fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Hit> {
        view::hit(self, ui, x, y)
    }

    fn activate(&mut self, ui: &Ui, target: Hit, x: f32, t: f64) {
        match target {
            Hit::Field => {
                let field = view::layout(ui, self.size).field;
                self.field.focused = true;
                self.field.tap(ui, x - field.0 - ui.pt(SPACE_M), t);
                self.dirty = true;
            }
            Hit::Row(i) => self.select(i, t),
        }
    }

    /// A secondary click: the menu of the row under it (Move Up, Move Down, Remove from Watchlist), which also selects
    /// the row. The answer comes back through `context_action`. Empty off the rows.
    pub fn context_items(&mut self, ui: &Ui, x: f32, y: f32, t: f64) -> Vec<ContextItem> {
        let Some(Hit::Row(i)) = self.hit(ui, x, y) else { return Vec::new() };
        self.select(i, t);
        self.context_row = Some(i);
        let item = |id: u32, label: &str, enabled: bool| ContextItem { id, label: label.to_string(), enabled };
        vec![
            item(CTX_UP, "Move Up", i > 0),
            item(CTX_DOWN, "Move Down", i + 1 < self.list.len()),
            item(CTX_REMOVE, "Remove from Watchlist", true),
        ]
    }

    /// The choice from a row's context menu; true when something changed.
    pub fn context_action(&mut self, id: u32, t: f64) -> bool {
        let Some(i) = self.context_row.take() else { return false };
        match id {
            CTX_UP => self.move_row(i, true),
            CTX_DOWN => self.move_row(i, false),
            CTX_REMOVE => self.remove(i, t),
            _ => return false,
        }
        self.dirty = true;
        true
    }

    /// Loads the selected symbol's cached chart when it is not in memory yet, and asks for a fresh one when due.
    fn show_selected(&mut self, _t: f64) {
        let Some(symbol) = self.list.selected_symbol().map(str::to_string) else { return };
        if !self.feed.history.contains_key(&symbol) {
            if let Some((raw, age)) = self.store.load_history(&symbol) {
                self.feed.load_history(&symbol, &raw, now_unix() - age as f64);
            }
        }
        self.request_history(&symbol);
    }

    /// Starts the fetches that are due: the quotes of the whole list and the history of the selected symbol.
    fn maintain(&mut self, t: f64) {
        let now = now_unix();
        if !self.list.is_empty() && self.feed.quotes_due(now) {
            self.spawn_quotes();
        }
        if let Some(symbol) = self.list.selected_symbol().map(str::to_string) {
            self.request_history(&symbol);
        }
        self.next_check = t + CHECK_S;
    }

    fn request_history(&mut self, symbol: &str) {
        if self.feed.history_due(symbol, now_unix()) {
            self.spawn_history(symbol);
        }
    }

    fn spawn_quotes(&mut self) {
        let symbols = self.list.symbols().to_vec();
        self.feed.begin_quotes();
        let (net, store, done) = (self.net.clone(), self.store.clone(), self.done.clone());
        DispatchQueue::global(Qos::Utility).async_(move || {
            let result = match data::fetch_quotes(&symbols, &*net) {
                Ok((raw, quotes, failed)) => {
                    if !quotes.is_empty() {
                        store.save_quotes(&raw);
                    }
                    Ok((quotes, failed))
                }
                Err(_) => Err("Quotes are unavailable. Check the connection and try again.".to_string()),
            };
            post(&done, Done::Quotes(result));
        });
    }

    fn spawn_history(&mut self, symbol: &str) {
        self.feed.begin_history(symbol);
        let symbol = symbol.to_string();
        let (net, store, done) = (self.net.clone(), self.store.clone(), self.done.clone());
        DispatchQueue::global(Qos::Utility).async_(move || {
            let result = match data::fetch_history(&symbol, &*net) {
                Ok((raw, bars)) => {
                    store.save_history(&symbol, &raw);
                    Ok(bars)
                }
                Err(_) => Err("Price history is unavailable right now.".to_string()),
            };
            post(&done, Done::History { symbol, result });
        });
    }

    /// Applies the finished jobs. True when there were any.
    fn pump(&mut self) -> bool {
        let done = std::mem::take(&mut *self.done.lock().unwrap_or_else(|e| e.into_inner()));
        let changed = !done.is_empty();
        let now = now_unix();
        for d in done {
            self.feed.apply(d, now);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Offline;
    impl Fetch for Offline {
        fn get(&self, _url: &str, _limit: usize) -> std::io::Result<Vec<u8>> {
            Err(std::io::Error::other("offline"))
        }
    }

    fn app(tag: &str, symbols: &[&str]) -> (Stocks, Ui, PathBuf) {
        let dir = std::env::temp_dir().join(format!("stocks-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Store::new(dir.clone()).save_watchlist(&symbols.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let ui = crate::test_ui();
        let mut s = Stocks::new(&ui, dir.clone(), Arc::new(Offline));
        s.set_size(1875, 1219);
        (s, ui, dir)
    }

    fn row_centre(s: &Stocks, ui: &Ui, i: usize) -> (f32, f32) {
        let l = view::layout(ui, s.size);
        (l.list.0 + l.list.2 / 2.0, l.list.1 + (i as f32 + 0.5) * l.row_px)
    }

    /// The old Stooq symbols read as the new service's form.
    #[test]
    fn a_saved_stooq_watchlist_loads_in_the_new_form() {
        let (s, _ui, dir) = app("legacy", &["aapl.us", "^spx"]);
        assert_eq!(s.list.symbols(), ["AAPL", "^GSPC"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Rows are reordered and removed from their context menu, and Delete removes the selected row.
    #[test]
    fn the_row_menu_moves_and_removes_and_delete_removes_the_selection() {
        let (mut s, ui, dir) = app("menu", &["AAPL", "MSFT", "NVDA"]);
        let (x, y) = row_centre(&s, &ui, 1);
        let items = s.context_items(&ui, x, y, 0.0);
        assert_eq!(items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["Move Up", "Move Down", "Remove from Watchlist"]);
        assert_eq!(s.list.selected_symbol(), Some("MSFT"), "the menu selects its row");
        assert!(s.context_action(CTX_UP, 0.0));
        assert_eq!(s.list.symbols(), ["MSFT", "AAPL", "NVDA"]);
        let (x, y) = row_centre(&s, &ui, 2);
        assert!(!s.context_items(&ui, x, y, 0.0)[1].enabled, "the last row cannot move down");
        assert!(s.context_action(CTX_REMOVE, 0.0));
        assert_eq!(s.list.symbols(), ["MSFT", "AAPL"]);
        assert!(s.context_items(&ui, ui.pt(500.0), y, 0.0).is_empty(), "no menu off the rows");
        s.select(0, 0.0);
        s.key(&ui, Out::Named("Delete"), 0.0);
        assert_eq!(s.list.symbols(), ["AAPL"]);
        assert_eq!(Store::new(dir.clone()).load_watchlist(), ["AAPL"], "the removal is saved");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// While a fetch is out the app wakes soon to show its answer; an offline answer ends in a visible error.
    #[test]
    fn a_fetch_in_flight_wakes_the_app_and_its_failure_is_shown() {
        let (mut s, _ui, dir) = app("wake", &["AAPL"]);
        s.start(0.0);
        assert!(s.fetching());
        assert!(s.next_wake(10.0).is_some_and(|w| w <= 10.0 + FETCH_POLL_MS));
        let mut done = false;
        for i in 0..400 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            s.tick(20.0 + i as f64);
            if !s.fetching() {
                done = true;
                break;
            }
        }
        assert!(done, "the offline answers came back");
        assert!(s.feed.quotes_error.is_some());
        assert_eq!(view::row_status(&s.feed, "AAPL"), view::ROW_UNAVAILABLE);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
