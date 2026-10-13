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

/// How often an open app checks whether a fetch is due (seconds).
const CHECK_S: f64 = 60.0;

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

    /// Applies finished jobs and asks for what is due. True when the screen needs a redraw.
    pub fn tick(&mut self, t: f64) -> bool {
        if self.pump() {
            self.dirty = true;
        }
        self.maintain(t);
        self.dirty
    }

    /// When the loop should next call `tick`: the next check, or the field's caret blink while it is focused.
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        let mut due = Some(self.next_check.max(t));
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
            self.note = Some("Use a symbol such as AAPL, or ^SPX for an index.".to_string());
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
                self.scrolling = view::in_list(ui, self.size, y);
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
            Hit::Up(i) => self.move_row(i, true),
            Hit::Down(i) => self.move_row(i, false),
            Hit::Remove(i) => self.remove(i, t),
        }
    }

    /// Loads the selected symbol's cached chart when it is not in memory yet, and asks for a fresh one when due.
    fn show_selected(&mut self, t: f64) {
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
                Ok((raw, quotes)) => {
                    store.save_quotes(&raw);
                    Ok(quotes)
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
