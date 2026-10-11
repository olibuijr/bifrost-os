//! Browser: tabs, an address bar with suggestions from bookmarks and
//! history, back/forward/reload, bookmarks, history, reader mode and
//! tap-to-follow links, on a small HTML + CSS engine of its own.
//!
//! The engine is split by stage: `html` parses, `css` styles, `layout`
//! flows the styled tree into a display list, `paint` draws it. `fetch`
//! loads pages, style sheets and pictures on worker threads over the
//! Deildu HTTP client (HTTP/1.1 over rustls, so https works), `reader`
//! extracts articles, `store` keeps bookmarks, history and the open tabs,
//! `nav` turns typed text into addresses and renders the `about:` pages.
//! This file is the app itself: tab state, touch handling, chrome drawing.

pub mod budget;
pub mod chrome;
pub mod css;
pub mod display;
pub mod dom;
pub mod fetch;
pub mod html;
pub mod icon;
pub mod images;
pub mod js;
pub mod layout;
pub mod layout2;
pub mod nav;
pub mod paint;
pub mod reader;
pub mod store;
pub mod tabmem;
pub mod tiles;
pub mod style;
pub mod doc;
pub mod resources;

pub use icon::icon;

use super::cache::TextCache;
use super::clock::Wall;
use super::edit::{Action, EditTarget};
use super::editmenu::EditMenu;
use super::keyboard::{self, Keyboard, Out};
use super::textfield::TextField;
use super::ui::{home_zone, Ui, Weight, BLACK, CELL, SECONDARY, WHITE};
use super::{clipboard, Event};
use crate::deildu::http::Transport;
use crate::deildu::scroll::Scroller;
use crate::gfx::Surface;
use crate::scene::STATUS_PT;
use css::{Rule, Styled};
use reader::Article;
use fetch::{Fetched, Loader, Msg, SheetSrc};
use layout::{Metrics, Page};
use paint::{Highlight, Images, Painter, Viewport};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

/// Where the browser's bookmarks, history and session live.
pub fn data_dir() -> String {
    crate::shell::apphost::root().join("Browser").to_string_lossy().into_owned()
}
use super::ui::TINT as ACCENT;
use super::ui::WEB_BROWSER_BAR as BAR;
const TAB_IDLE: u32 = BLACK;
use super::ui::WEB_BROWSER_BAR as TAB_ACTIVE;
use super::ui::WEB_BROWSER_FIELD as FIELD;
use super::ui::TERTIARY_LABEL as DISABLED;

const TAB_BAR_PT: f32 = 33.0;
const TOOLBAR_PT: f32 = 50.0;
const BUTTON_PT: f32 = 40.0;
const FIELD_PT: f32 = 36.0;
const TAB_MIN_PT: f32 = 76.0;
const FIELD_LEFT_PT: f32 = 151.0;
const FIELD_RIGHT_PT: f32 = 152.0;
const FIELD_INSET_PT: f32 = 38.0;
// button centres measured on the bifrost mini (Browser, bifrost 15): from the left / from the right edge
const LEFT_CENTRES_PT: [f32; 3] = [35.0, 77.0, 116.0];
const RIGHT_CENTRES_PT: [f32; 3] = [33.0, 77.0, 120.0];
const ROW_PT: f32 = 52.0;
/// toast radius; the URL field is a capsule
const RADIUS_PT: f32 = 13.0;
const MAX_TABS: usize = 12;
const MAX_RULES: usize = 6000;
const MAX_IMAGES: usize = 48;
const CACHED_PAGES: usize = 4;
const LONG_PRESS_MS: f64 = 450.0;
const TOAST_MS: f64 = 1600.0;
const SUGGESTIONS: usize = 6;

// --- measuring text for layout ---------------------------------------------

type Widths = RefCell<HashMap<(String, u32, bool), f32>>;

/// Layout's view of the shell font: widths in CSS px, memoised.
struct Measurer<'a> {
    ui: &'a Ui,
    cache: &'a Widths,
}

impl Metrics for Measurer<'_> {
    fn width(&self, text: &str, size: f32, bold: bool) -> f32 {
        let key = (text.to_string(), (size * 10.0) as u32, bold);
        if let Some(&w) = self.cache.borrow().get(&key) {
            return w;
        }
        let w = self.ui.measure(text, size, if bold { Weight::Bold } else { Weight::Regular }) / self.ui.s;
        let mut cache = self.cache.borrow_mut();
        if cache.len() > 60_000 {
            cache.clear();
        }
        cache.insert(key, w);
        w
    }
}

fn ua_rules() -> &'static [Rule] {
    static UA: OnceLock<Vec<Rule>> = OnceLock::new();
    UA.get_or_init(|| css::parse_stylesheet(css::USER_AGENT))
}

fn reader_rules() -> &'static [Rule] {
    static READER: OnceLock<Vec<Rule>> = OnceLock::new();
    READER.get_or_init(|| css::parse_stylesheet(reader::READER_CSS))
}

// --- a loaded page ----------------------------------------------------------

struct Source {
    rules: Vec<Rule>,
    /// the address of a linked sheet still on its way
    pending: Option<String>,
}

/// Everything about a page that is on screen (or parked for Back).
struct PageView {
    url: String,
    title: String,
    dom: Vec<html::Node>,
    sources: Vec<Source>,
    sizes: HashMap<String, (usize, usize)>,
    images: Images,
    requested: HashSet<String>,
    tree: Vec<Styled>,
    page: Page,
    laid_w: f32,
    article: Option<Article>,
    reader_on: bool,
    restyle: bool,
    reflow: bool,
    failed: bool,
}

impl PageView {
    fn restyle(&mut self) {
        let mut rules: Vec<Rule> = ua_rules().to_vec();
        match (&self.article, self.reader_on) {
            (Some(a), true) => {
                rules.extend_from_slice(reader_rules());
                self.tree = css::style_tree(&reader::document(a), &rules);
            }
            _ => {
                for s in &self.sources {
                    if rules.len() < MAX_RULES {
                        rules.extend(s.rules.iter().cloned());
                    }
                }
                self.tree = css::style_tree(&self.dom, &rules);
            }
        }
    }

    /// Restyles and relays out what changed; true when the page did.
    fn refresh(&mut self, m: &dyn Metrics, width: f32) -> bool {
        if !(self.restyle || self.reflow || (self.laid_w - width).abs() > 0.5) {
            return false;
        }
        if self.restyle {
            self.restyle();
        }
        self.page = layout::layout(&self.tree, m, width, &self.sizes);
        self.laid_w = width;
        self.restyle = false;
        self.reflow = false;
        true
    }
}

#[derive(Clone, Debug)]
struct Entry {
    url: String,
    title: String,
    scroll: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Sel {
    Block(usize),
    All,
}

struct Tab {
    id: u64,
    /// bumped by every navigation; answers carrying an older number are stale
    nav: u64,
    entries: Vec<Entry>,
    pos: usize,
    view: Option<PageView>,
    /// pages left behind, newest last, restored instantly by Back and Forward
    cache: Vec<PageView>,
    loading: Option<String>,
    loading_since: f64,
    restore: Option<f32>,
    scroll: Scroller,
    selection: Option<Sel>,
}

impl Tab {
    fn new(id: u64, url: &str) -> Tab {
        Tab {
            id,
            nav: 0,
            entries: vec![Entry { url: url.to_string(), title: String::new(), scroll: 0.0 }],
            pos: 0,
            view: None,
            cache: vec![],
            loading: None,
            loading_since: 0.0,
            restore: None,
            scroll: Scroller::new(),
            selection: None,
        }
    }

    /// The address this tab shows or is going to.
    fn url(&self) -> &str {
        self.loading.as_deref().or(self.view.as_ref().map(|v| v.url.as_str())).unwrap_or(&self.entries[self.pos].url)
    }

    fn title(&self) -> String {
        match (&self.loading, &self.view) {
            (Some(u), _) => nav::display_url(u),
            (None, Some(v)) => v.title.clone(),
            (None, None) => match self.entries[self.pos].title.as_str() {
                "" => "New Tab".to_string(),
                t => t.to_string(),
            },
        }
    }
}

/// Something a touch can land on.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Tab(usize),
    CloseTab(usize),
    NewTab,
    Back,
    Forward,
    Url,
    Reload,
    Star,
    Reader,
    Library,
    /// the tab overview button, a card of the overview, its close badge, its + button
    Overview,
    Card(usize),
    CardClose(usize),
    CardNew,
    CardsDone,
    /// row 0 is the typed text itself, then the suggestions
    Row(usize),
    /// empty space below the suggestions
    Dismiss,
    Content,
    Keyboard,
}

pub struct Browser {
    w: i32,
    h: i32,
    dir: PathBuf,
    wall: Wall,
    lib: store::Library,
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    loader: Loader,
    widths: Widths,
    painter: Painter,
    labels: TextCache,
    url: TextField,
    editing: bool,
    /// the Browser tab overview grid is showing
    overview: bool,
    keyboard: Keyboard,
    kb_touch: bool,
    field_touch: bool,
    suggestions: Vec<store::Suggestion>,
    pressed: Option<Target>,
    /// a finger down on the page: where and when
    press: Option<(f32, f32, f64)>,
    scrolling: bool,
    long_fired: bool,
    menu_down: Option<(f32, f32)>,
    toast: Option<(String, f64)>,
    dirty: bool,
}

fn within(x: f32, y: f32, (rx, ry, rw, rh): (f32, f32, f32, f32)) -> bool {
    x >= rx && x < rx + rw && y >= ry && y < ry + rh
}

impl Browser {
    /// Opens the browser: bookmarks and history from `dir`, the tabs of the
    /// last session (the active one loads now, the rest when selected).
    pub fn new(ui: &Ui, w: i32, h: i32, dir: PathBuf, transport: Arc<dyn Transport>, wall: Wall, t: f64) -> Browser {
        let (urls, active) = store::load_session(&dir);
        let mut b = Browser {
            w,
            h,
            lib: store::Library::load(&dir),
            dir,
            wall,
            tabs: vec![],
            active: 0,
            next_id: 1,
            loader: Loader::new(transport),
            widths: RefCell::new(HashMap::new()),
            painter: Painter::default(),
            labels: TextCache::default(),
            url: TextField::new(ui, "Search or enter address", 16.0, false, 2000),
            editing: false,
            overview: false,
            keyboard: Keyboard::new_hidden(w as f32, ui.s),
            kb_touch: false,
            field_touch: false,
            suggestions: vec![],
            pressed: None,
            press: None,
            scrolling: false,
            long_fired: false,
            menu_down: None,
            toast: None,
            dirty: true,
        };
        b.url.focused = false;
        let urls = if urls.is_empty() { vec![nav::HOME.to_string()] } else { urls };
        for u in urls.iter().take(MAX_TABS) {
            let id = b.next_id;
            b.next_id += 1;
            b.tabs.push(Tab::new(id, u));
        }
        b.active = active.min(b.tabs.len() - 1);
        b.ensure_loaded(ui, b.active, t);
        b
    }

    pub fn set_wall(&mut self, wall: Wall) {
        self.wall = wall;
    }

    pub fn needs_frame(&self) -> bool {
        self.dirty
    }

    pub fn keyboard_shown(&self) -> bool {
        self.keyboard.shown()
    }

    /// key=value lines for remote inspection.
    pub fn describe(&self) -> String {
        let tab = &self.tabs[self.active];
        format!(
            "browser_tabs={}\nbrowser_active={}\nbrowser_url={}\nbrowser_title={}\nbrowser_loading={}\nbrowser_reader={}\nbrowser_editing={}\nbrowser_bookmarks={}\nbrowser_history={}\n",
            self.tabs.len(),
            self.active,
            tab.url(),
            tab.title(),
            tab.loading.is_some(),
            tab.view.as_ref().is_some_and(|v| v.reader_on),
            self.editing,
            self.lib.bookmarks.len(),
            self.lib.history.len()
        )
    }

    /// Writes the open tabs now (the app is being closed for good).
    pub fn flush(&mut self) {
        self.save_session();
    }

    fn save_session(&self) {
        let urls: Vec<String> = self.tabs.iter().map(|t| t.entries[t.pos].url.clone()).collect();
        store::save_session(&self.dir, &urls, self.active);
    }

    // --- geometry (px) ---

    fn tool_top(&self, ui: &Ui) -> f32 {
        ui.pt(STATUS_PT)
    }

    fn tab_top(&self, ui: &Ui) -> f32 {
        self.tool_top(ui) + ui.pt(TOOLBAR_PT)
    }

    fn content_top(&self, ui: &Ui) -> f32 {
        self.tab_top(ui) + ui.pt(TAB_BAR_PT)
    }

    /// Bottom of the page area, above the home gesture zone.
    fn page_bottom(&self, ui: &Ui) -> f32 {
        self.h as f32 - home_zone(ui)
    }

    fn kb_top(&self, ui: &Ui) -> f32 {
        self.page_bottom(ui) - self.keyboard.h()
    }

    /// The viewport width in CSS px.
    fn css_w(&self, ui: &Ui) -> f32 {
        self.w as f32 / ui.s
    }

    fn tab_w(&self, ui: &Ui) -> f32 {
        (self.w as f32 / self.tabs.len() as f32).max(ui.pt(TAB_MIN_PT))
    }

    /// Slots 0..=2 count from the left, -1..=-3 from the right; 10 and 11 are
    /// the reader and reload buttons inside the address field.
    fn button_rect(&self, ui: &Ui, slot: i32) -> (f32, f32, f32, f32) {
        let b = ui.pt(BUTTON_PT);
        let cx = match slot {
            0..=2 => ui.pt(LEFT_CENTRES_PT[slot as usize]),
            -3..=-1 => self.w as f32 - ui.pt(RIGHT_CENTRES_PT[(-slot - 1) as usize]),
            10 => self.url_rect(ui).0 + ui.pt(FIELD_INSET_PT / 2.0 - 1.0),
            _ => {
                let (x, _, w, _) = self.url_rect(ui);
                x + w - ui.pt(FIELD_INSET_PT / 2.0 - 1.0)
            }
        };
        (cx - b / 2.0, self.tool_top(ui), b, ui.pt(TOOLBAR_PT))
    }

    fn url_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let x0 = ui.pt(FIELD_LEFT_PT);
        let x1 = self.w as f32 - ui.pt(FIELD_RIGHT_PT);
        let h = ui.pt(FIELD_PT);
        (x0, self.tool_top(ui) + ui.pt(4.0), x1 - x0, h)
    }

    fn url_frame(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (x, y, w, h) = self.url_rect(ui);
        (x + ui.pt(FIELD_INSET_PT), y, w - ui.pt(2.0 * FIELD_INSET_PT), h)
    }

    fn row_rect(&self, ui: &Ui, i: usize) -> (f32, f32, f32, f32) {
        (0.0, self.content_top(ui) + ui.pt(ROW_PT) * i as f32, self.w as f32, ui.pt(ROW_PT))
    }

    /// The rows of the suggestion list: the typed text first, then matches.
    fn row_count(&self) -> usize {
        usize::from(!self.url.text().trim().is_empty()) + self.suggestions.len()
    }

    /// Card `i` of the overview grid: two columns on a portrait window, three when wide.
    fn card_rect(&self, ui: &Ui, i: usize) -> (f32, f32, f32, f32) {
        let cols = if self.w > self.h { 3 } else { 2 };
        let (m, gap) = (ui.pt(20.0), ui.pt(16.0));
        let cw = (self.w as f32 - 2.0 * m - gap * (cols - 1) as f32) / cols as f32;
        let ch = cw * 0.75;
        let (col, row) = (i % cols, i / cols);
        (m + col as f32 * (cw + gap), self.tool_top(ui) + ui.pt(16.0) + row as f32 * (ch + ui.pt(36.0)), cw, ch)
    }

    fn overview_bar(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let h = ui.pt(TOOLBAR_PT);
        (0.0, self.page_bottom(ui) - h, self.w as f32, h)
    }

    fn hit_overview(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let bar = self.overview_bar(ui);
        if within(x, y, bar) {
            let b = ui.pt(BUTTON_PT) * 1.6;
            if x < b + ui.pt(8.0) {
                return Some(Target::CardNew);
            }
            return (x >= self.w as f32 - b - ui.pt(8.0)).then_some(Target::CardsDone);
        }
        for i in 0..self.tabs.len() {
            let r = self.card_rect(ui, i);
            if within(x, y, r) {
                let close = (r.0 + r.2 - ui.pt(36.0), r.1, ui.pt(36.0), ui.pt(36.0));
                return Some(if within(x, y, close) { Target::CardClose(i) } else { Target::Card(i) });
            }
        }
        None
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        if self.overview {
            return self.hit_overview(ui, x, y);
        }
        if self.keyboard.shown() && y >= self.kb_top(ui) {
            return Some(Target::Keyboard);
        }
        if y < self.tool_top(ui) {
            return None;
        }
        if y < self.tab_top(ui) {
            let slots = [(0, Target::Library), (1, Target::Back), (2, Target::Forward), (-1, Target::Overview), (-2, Target::NewTab), (-3, Target::Star), (10, Target::Reader), (11, Target::Reload)];
            for (slot, target) in slots {
                if within(x, y, self.button_rect(ui, slot)) {
                    return Some(target);
                }
            }
            return within(x, y, self.url_rect(ui)).then_some(Target::Url);
        }
        if y < self.content_top(ui) {
            let tw = self.tab_w(ui);
            let i = (x / tw) as usize;
            if i >= self.tabs.len() {
                return None;
            }
            return Some(if x < i as f32 * tw + ui.pt(36.0) { Target::CloseTab(i) } else { Target::Tab(i) });
        }
        if self.editing {
            let rows = self.row_count();
            for i in 0..rows {
                if within(x, y, self.row_rect(ui, i)) {
                    return Some(Target::Row(i));
                }
            }
            return Some(Target::Dismiss);
        }
        (y < self.page_bottom(ui)).then_some(Target::Content)
    }

    // --- tabs and navigation ---

    fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    fn ensure_loaded(&mut self, ui: &Ui, ti: usize, t: f64) {
        let tab = &self.tabs[ti];
        if tab.view.is_none() && tab.loading.is_none() {
            let url = tab.entries[tab.pos].url.clone();
            let restore = Some(tab.entries[tab.pos].scroll);
            self.load(ui, ti, &url, restore, t);
        }
    }

    fn new_tab(&mut self, ui: &Ui, url: &str, t: f64) {
        if self.tabs.len() >= MAX_TABS {
            self.toast("Too many tabs", t);
            return;
        }
        self.unfocus(ui, t);
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(Tab::new(id, url));
        self.active = self.tabs.len() - 1;
        self.ensure_loaded(ui, self.active, t);
        self.sync_url(ui);
        self.save_session();
    }

    fn switch_tab(&mut self, ui: &Ui, i: usize, t: f64) {
        if i >= self.tabs.len() {
            return;
        }
        self.unfocus(ui, t);
        self.active = i;
        self.ensure_loaded(ui, i, t);
        self.process_dirty(ui);
        self.sync_url(ui);
        self.save_session();
    }

    fn close_tab(&mut self, ui: &Ui, i: usize, t: f64) {
        if i >= self.tabs.len() {
            return;
        }
        self.unfocus(ui, t);
        self.loader.cancel(self.tabs[i].id);
        self.tabs.remove(i);
        if self.tabs.is_empty() {
            let id = self.next_id;
            self.next_id += 1;
            self.tabs.push(Tab::new(id, nav::HOME));
            self.active = 0;
        } else if i < self.active {
            self.active -= 1;
        } else if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
        self.ensure_loaded(ui, self.active, t);
        self.process_dirty(ui);
        self.sync_url(ui);
        self.save_session();
    }

    /// Opens `url` in the active tab as a new history entry.
    fn navigate(&mut self, ui: &Ui, url: &str, t: f64) {
        let ti = self.active;
        {
            let tab = &mut self.tabs[ti];
            let off = tab.scroll.offset();
            let title = tab.view.as_ref().map(|v| v.title.clone());
            if let Some(e) = tab.entries.get_mut(tab.pos) {
                e.scroll = off;
                if let Some(title) = title {
                    e.title = title;
                }
            }
            tab.entries.truncate(tab.pos + 1);
            tab.entries.push(Entry { url: url.to_string(), title: String::new(), scroll: 0.0 });
            tab.pos = tab.entries.len() - 1;
        }
        self.load(ui, ti, url, None, t);
    }

    /// Steps through the tab's history by `delta` (-1 Back, +1 Forward).
    fn go(&mut self, ui: &Ui, delta: i32, t: f64) {
        let ti = self.active;
        let target = self.tabs[ti].pos as i32 + delta;
        if target < 0 || target as usize >= self.tabs[ti].entries.len() {
            return;
        }
        let url = {
            let tab = &mut self.tabs[ti];
            let off = tab.scroll.offset();
            let title = tab.view.as_ref().map(|v| v.title.clone());
            if let Some(e) = tab.entries.get_mut(tab.pos) {
                e.scroll = off;
                if let Some(title) = title {
                    e.title = title;
                }
            }
            tab.pos = target as usize;
            tab.entries[tab.pos].url.clone()
        };
        let restore = self.tabs[ti].entries[self.tabs[ti].pos].scroll;
        let cached = if nav::is_about(&url) { None } else { self.tabs[ti].cache.iter().rposition(|v| v.url == url) };
        match cached {
            Some(idx) => {
                let tab = &mut self.tabs[ti];
                tab.nav += 1;
                tab.loading = None;
                tab.selection = None;
                self.loader.cancel(tab.id);
                let view = tab.cache.remove(idx);
                if let Some(old) = tab.view.take() {
                    tab.cache.push(old);
                }
                tab.view = Some(view);
                self.process_dirty(ui);
                self.fit_scroll(ui);
                self.tabs[ti].scroll.jump_to(restore);
                self.sync_url(ui);
                self.save_session();
            }
            None => self.load(ui, ti, &url, Some(restore), t),
        }
    }

    fn reload_or_stop(&mut self, ui: &Ui, t: f64) {
        let ti = self.active;
        if self.tabs[ti].loading.is_some() {
            let tab = &mut self.tabs[ti];
            tab.nav += 1;
            tab.loading = None;
            self.loader.cancel(tab.id);
            self.sync_url(ui);
            return;
        }
        let url = self.tabs[ti].view.as_ref().map(|v| v.url.clone()).unwrap_or_else(|| self.tabs[ti].entries[self.tabs[ti].pos].url.clone());
        let off = self.tabs[ti].scroll.offset();
        self.load(ui, ti, &url, Some(off), t);
    }

    fn apply_action(&mut self, a: &nav::Action) {
        match a {
            nav::Action::RemoveBookmark(u) => self.lib.remove_bookmark(u),
            nav::Action::RemoveHistory(u) => self.lib.remove_history(u),
            nav::Action::ClearHistory => self.lib.clear_history(),
        }
    }

    /// Starts loading `url` into tab `ti`, which keeps showing its current page meanwhile.
    fn load(&mut self, ui: &Ui, ti: usize, url: &str, restore: Option<f32>, t: f64) {
        if let Some(action) = nav::action(url) {
            self.apply_action(&action);
            let next = action.next();
            let tab = &mut self.tabs[ti];
            let pos = tab.pos;
            tab.entries[pos].url = next.to_string();
            return self.load(ui, ti, next, None, t);
        }
        let (id, nav_no) = {
            let tab = &mut self.tabs[ti];
            tab.nav += 1;
            tab.restore = restore;
            tab.selection = None;
            (tab.id, tab.nav)
        };
        self.loader.cancel(id);
        if nav::is_about(url) {
            self.tabs[ti].loading = None;
            let f = match nav::about_page(url, &self.lib) {
                Some((_, page)) => fetch::fetched(url, html::parse(&page)),
                None => fetch::error_page(url, "There is no such built-in page."),
            };
            self.install(ui, ti, f, restore, t);
            return;
        }
        let tab = &mut self.tabs[ti];
        tab.loading = Some(url.to_string());
        tab.loading_since = t;
        self.loader.page(id, nav_no, url);
        self.sync_url(ui);
        self.dirty = true;
    }

    /// Makes `f` the page of tab `ti`.
    fn install(&mut self, ui: &Ui, ti: usize, f: Fetched, restore: Option<f32>, t: f64) {
        let title = html::title(&f.dom).unwrap_or_else(|| nav::display_url(&f.url));
        let mut images = Images::default();
        let mut sizes = HashMap::new();
        if let Some(img) = f.image {
            sizes.insert(f.url.clone(), (img.w, img.h));
            images.insert(&f.url, img);
        }
        let (tab_id, nav_no) = (self.tabs[ti].id, self.tabs[ti].nav);
        let mut sources = Vec::with_capacity(f.sheets.len());
        for (i, s) in f.sheets.into_iter().enumerate() {
            match s {
                SheetSrc::Inline(rules) => sources.push(Source { rules, pending: None }),
                SheetSrc::Link(u) => {
                    self.loader.sheet(tab_id, nav_no, i, &u);
                    sources.push(Source { rules: vec![], pending: Some(u) });
                }
            }
        }
        let mut view = PageView {
            url: f.url.clone(),
            title: title.clone(),
            dom: f.dom,
            sources,
            sizes,
            images,
            requested: HashSet::new(),
            tree: vec![],
            page: Page::default(),
            laid_w: 0.0,
            article: None,
            reader_on: false,
            restyle: true,
            reflow: true,
            failed: f.failed,
        };
        view.refresh(&Measurer { ui, cache: &self.widths }, self.css_w(ui));
        {
            let tab = &mut self.tabs[ti];
            if let Some(old) = tab.view.take() {
                if old.url != view.url {
                    tab.cache.push(old);
                    while tab.cache.len() > CACHED_PAGES {
                        tab.cache.remove(0);
                    }
                }
            }
            tab.loading = None;
            tab.selection = None;
            let pos = tab.pos;
            tab.entries[pos].url = f.url.clone();
            tab.entries[pos].title = title.clone();
            tab.view = Some(view);
        }
        if !f.failed {
            self.lib.visit(&f.url, &title, self.wall.now(t));
        }
        self.request_images(ti);
        if ti == self.active {
            self.fit_scroll(ui);
            self.tabs[ti].scroll.jump_to(restore.unwrap_or(0.0));
            self.sync_url(ui);
        }
        self.save_session();
        self.dirty = true;
    }

    /// Asks for the pictures of tab `ti`'s page that nobody has asked for yet.
    fn request_images(&mut self, ti: usize) {
        let tab = &mut self.tabs[ti];
        let (id, nav_no) = (tab.id, tab.nav);
        let Some(v) = tab.view.as_mut() else { return };
        let wanted: Vec<String> = v.page.images.iter().filter(|s| !v.requested.contains(*s) && !v.images.has(s)).cloned().collect();
        for src in wanted {
            if v.requested.len() >= MAX_IMAGES {
                break;
            }
            v.requested.insert(src.clone());
            match nav::resolve(&v.url, &src) {
                Some(abs) => self.loader.image(id, nav_no, &src, &abs),
                None => v.images.fail(&src),
            }
        }
    }

    fn on_msg(&mut self, ui: &Ui, msg: Msg, t: f64) {
        let (tab_id, nav_no) = match &msg {
            Msg::Page { tab, nav, .. } | Msg::Sheet { tab, nav, .. } | Msg::Image { tab, nav, .. } => (*tab, *nav),
        };
        let Some(ti) = self.tabs.iter().position(|x| x.id == tab_id) else { return };
        if self.tabs[ti].nav != nav_no {
            return;
        }
        match msg {
            Msg::Page { result, .. } => {
                let url = self.tabs[ti].loading.clone().unwrap_or_default();
                let restore = self.tabs[ti].restore.take();
                let f = match result {
                    Ok(f) => f,
                    Err(e) => fetch::error_page(&url, &e),
                };
                self.install(ui, ti, f, restore, t);
            }
            Msg::Sheet { slot, rules, .. } => {
                if let Some(v) = self.tabs[ti].view.as_mut() {
                    if let Some(s) = v.sources.get_mut(slot) {
                        s.rules = rules;
                        s.pending = None;
                        v.restyle = true;
                    }
                }
            }
            Msg::Image { src, image, .. } => {
                if let Some(v) = self.tabs[ti].view.as_mut() {
                    match image {
                        Some(img) => {
                            v.sizes.insert(src.clone(), (img.w, img.h));
                            v.images.insert(&src, img);
                            v.reflow = true;
                        }
                        None => v.images.fail(&src),
                    }
                }
            }
        }
        self.dirty = true;
    }

    /// Restyles and relays out the active page if something changed.
    fn process_dirty(&mut self, ui: &Ui) -> bool {
        let css_w = self.css_w(ui);
        let ti = self.active;
        let changed = match self.tabs[ti].view.as_mut() {
            Some(v) => v.refresh(&Measurer { ui, cache: &self.widths }, css_w),
            None => false,
        };
        if changed {
            self.request_images(ti);
            self.fit_scroll(ui);
        }
        changed
    }

    fn fit_scroll(&mut self, ui: &Ui) {
        let view_h = self.page_bottom(ui) - self.content_top(ui);
        let tab = &mut self.tabs[self.active];
        let max = tab.view.as_ref().map_or(0.0, |v| v.page.height * ui.s - view_h);
        tab.scroll.set_max(max);
    }

    fn toggle_reader(&mut self, ui: &Ui, t: f64) {
        let ti = self.active;
        let Some(v) = self.tabs[ti].view.as_mut() else { return };
        if v.reader_on {
            v.reader_on = false;
        } else {
            if v.article.is_none() && !nav::is_about(&v.url) {
                v.article = reader::extract(&v.dom);
            }
            if v.article.is_none() {
                self.toast("No article to show in Reader", t);
                return;
            }
            v.reader_on = true;
        }
        v.restyle = true;
        self.tabs[ti].selection = None;
        self.process_dirty(ui);
        self.tabs[ti].scroll.jump_top();
        self.dirty = true;
    }

    fn toggle_bookmark(&mut self, t: f64) {
        let Some(v) = self.tabs[self.active].view.as_ref() else { return };
        if nav::is_about(&v.url) || v.failed {
            return;
        }
        let (title, url) = (v.title.clone(), v.url.clone());
        let on = self.lib.toggle_bookmark(&title, &url);
        self.toast(if on { "Bookmark added" } else { "Bookmark removed" }, t);
    }

    fn toast(&mut self, text: &str, t: f64) {
        self.toast = Some((text.to_string(), t + TOAST_MS));
        self.dirty = true;
    }

    // --- the address bar ---

    /// What the bar shows when it is not being edited.
    fn shown_url(&self) -> String {
        let u = self.tab().url();
        if nav::is_about(u) {
            String::new()
        } else {
            nav::display_url(u)
        }
    }

    fn sync_url(&mut self, ui: &Ui) {
        if !self.editing {
            let text = self.shown_url();
            self.url.set_text(ui, &text);
        }
        self.dirty = true;
    }

    fn focus_url(&mut self, ui: &Ui, t: f64) {
        if self.editing {
            return;
        }
        self.editing = true;
        let u = self.tab().url();
        let full = if nav::is_about(u) { String::new() } else { u.to_string() };
        self.url.set_text(ui, &full);
        self.url.focused = true;
        let n = self.url.len();
        self.url.select(ui, (0, n));
        self.keyboard.set_shown(true, t);
        self.refresh_suggestions();
        self.dirty = true;
    }

    fn unfocus(&mut self, ui: &Ui, t: f64) {
        if !self.editing {
            return;
        }
        self.editing = false;
        self.url.focused = false;
        self.keyboard.set_shown(false, t);
        self.suggestions.clear();
        self.sync_url(ui);
    }

    fn refresh_suggestions(&mut self) {
        self.suggestions = self.lib.suggest(self.url.text(), SUGGESTIONS);
    }

    fn submit(&mut self, ui: &Ui, t: f64) {
        let target = nav::normalize_input(self.url.text());
        self.unfocus(ui, t);
        if let Some(u) = target {
            self.navigate(ui, &u, t);
        }
    }

    /// Typed input for the address bar.
    pub fn key(&mut self, ui: &Ui, out: Out, t: f64) {
        self.dirty = true;
        if !self.editing {
            return;
        }
        match out {
            Out::Hide => self.unfocus(ui, t),
            Out::Enter => self.submit(ui, t),
            out => {
                self.url.edit(ui, &out, t);
                self.refresh_suggestions();
            }
        }
    }

    // --- selection ---

    fn page_point(&self, ui: &Ui, x: f32, y: f32) -> (f32, f32) {
        (x / ui.s, (y - self.content_top(ui) + self.tab().scroll.offset()) / ui.s)
    }

    /// The Copy / Select All bubble over the selected text.
    fn selection_menu(&self, ui: &Ui) -> Option<EditMenu> {
        let tab = self.tab();
        let sel = tab.selection?;
        let v = tab.view.as_ref()?;
        let (top, bottom) = (self.content_top(ui), self.page_bottom(ui));
        let anchor = match sel {
            Sel::Block(b) => {
                let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for it in &v.page.items {
                    if let layout::Item::Text { x, y, w, h, block, .. } = it {
                        if *block == b {
                            x0 = x0.min(*x);
                            y0 = y0.min(*y);
                            x1 = x1.max(x + w);
                            y1 = y1.max(y + h);
                        }
                    }
                }
                if x0 > x1 {
                    return None;
                }
                let (sy0, sy1) = ((top + y0 * ui.s - tab.scroll.offset()).clamp(top, bottom), (top + y1 * ui.s - tab.scroll.offset()).clamp(top, bottom));
                (x0 * ui.s, sy0, (x1 - x0) * ui.s, sy1 - sy0)
            }
            Sel::All => (self.w as f32 / 2.0 - 1.0, top + ui.pt(8.0), 2.0, 2.0),
        };
        let actions: &[Action] = if sel == Sel::All { &[Action::Copy] } else { &[Action::Copy, Action::SelectAll] };
        Some(EditMenu::new(ui, actions, anchor, self.w as f32, top))
    }

    fn run_menu_action(&mut self, action: Action, t: f64) {
        let ti = self.active;
        let text = match (self.tabs[ti].selection, self.tabs[ti].view.as_ref()) {
            (Some(Sel::Block(b)), Some(v)) => v.page.blocks.get(b).cloned(),
            (Some(Sel::All), Some(v)) => Some(v.page.text()),
            _ => None,
        };
        match action {
            Action::Copy => {
                if let Some(text) = text.filter(|t| !t.is_empty()) {
                    clipboard::set_text(&text);
                    self.toast("Copied", t);
                }
                self.tabs[ti].selection = None;
            }
            Action::SelectAll => self.tabs[ti].selection = Some(Sel::All),
            _ => {}
        }
        self.dirty = true;
    }

    // --- touch ---

    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) {
        let (x, y) = match ev {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            _ => return,
        };
        self.dirty = true;
        match ev {
            Event::Down(..) => self.down(ui, ev, x, y, t),
            Event::Move(..) => self.moved(ui, ev, x, y, t),
            _ => self.up(ui, ev, x, y, t),
        }
    }

    fn down(&mut self, ui: &Ui, ev: Event, x: f32, y: f32, t: f64) {
        self.long_fired = false;
        self.press = None;
        self.scrolling = false;
        self.menu_down = None;
        if let Some(menu) = self.selection_menu(ui) {
            if menu.contains(ui, x, y) {
                self.menu_down = Some((x, y));
                return;
            }
        }
        if self.editing && self.url.touch(ui, ev, self.url_frame(ui), self.w as f32, t) {
            self.field_touch = true;
            return;
        }
        self.pressed = self.hit(ui, x, y);
        match self.pressed {
            Some(Target::Keyboard) => {
                self.kb_touch = true;
                let top = self.kb_top(ui);
                self.keyboard.down(x, y - top, t);
            }
            Some(Target::Content) => {
                self.scrolling = true;
                self.press = Some((x, y, t));
                let ti = self.active;
                self.tabs[ti].scroll.down(y, t);
            }
            _ => {}
        }
    }

    fn moved(&mut self, ui: &Ui, ev: Event, x: f32, y: f32, t: f64) {
        if self.menu_down.is_some() {
            return;
        }
        if self.field_touch {
            self.url.touch(ui, ev, self.url_frame(ui), self.w as f32, t);
        } else if self.kb_touch {
            let top = self.kb_top(ui);
            self.keyboard.moved(x, y - top);
        } else if self.scrolling {
            let ti = self.active;
            self.tabs[ti].scroll.moved(y, t);
            if self.tabs[ti].scroll.dragging() {
                self.pressed = None;
                self.press = None;
            }
        }
    }

    fn up(&mut self, ui: &Ui, ev: Event, x: f32, y: f32, t: f64) {
        if let Some((x0, y0)) = self.menu_down.take() {
            if let Some(mut menu) = self.selection_menu(ui) {
                menu.press(x0, y0);
                if let Some(action) = menu.release(x, y) {
                    self.run_menu_action(action, t);
                }
            }
            return;
        }
        if std::mem::take(&mut self.field_touch) {
            self.url.touch(ui, ev, self.url_frame(ui), self.w as f32, t);
            self.refresh_suggestions();
            return;
        }
        if std::mem::take(&mut self.kb_touch) {
            self.pressed = None;
            let top = self.kb_top(ui);
            if let Some(out) = self.keyboard.up(x, y - top, t) {
                self.key(ui, out, t);
            }
            return;
        }
        let ti = self.active;
        let tap = if std::mem::take(&mut self.scrolling) { self.tabs[ti].scroll.up(y, t) } else { true };
        self.press = None;
        let pressed = self.pressed.take();
        if std::mem::take(&mut self.long_fired) || !tap {
            return;
        }
        if let Some(target) = pressed.filter(|p| self.hit(ui, x, y) == Some(*p)) {
            self.activate(ui, target, x, y, t);
        }
    }

    fn activate(&mut self, ui: &Ui, target: Target, x: f32, y: f32, t: f64) {
        if !matches!(target, Target::Url | Target::Keyboard | Target::Row(_) | Target::Dismiss) {
            self.unfocus(ui, t);
        }
        match target {
            Target::Tab(i) => self.switch_tab(ui, i, t),
            Target::CloseTab(i) => self.close_tab(ui, i, t),
            Target::NewTab => self.new_tab(ui, nav::HOME, t),
            Target::Back => self.go(ui, -1, t),
            Target::Forward => self.go(ui, 1, t),
            Target::Url => self.focus_url(ui, t),
            Target::Reload => self.reload_or_stop(ui, t),
            Target::Star => self.toggle_bookmark(t),
            Target::Reader => self.toggle_reader(ui, t),
            Target::Overview => self.overview = true,
            Target::Card(i) => {
                self.overview = false;
                self.switch_tab(ui, i, t);
            }
            Target::CardClose(i) => {
                self.close_tab(ui, i, t);
                self.overview = self.tabs.len() > 1 || self.tabs[0].url() != nav::HOME;
            }
            Target::CardNew => {
                self.overview = false;
                self.new_tab(ui, nav::HOME, t);
            }
            Target::CardsDone => self.overview = false,
            Target::Library => {
                self.unfocus(ui, t);
                self.navigate(ui, nav::BOOKMARKS, t);
            }
            Target::Row(i) => {
                let typed = !self.url.text().trim().is_empty();
                let url = if typed && i == 0 {
                    nav::normalize_input(self.url.text())
                } else {
                    self.suggestions.get(i - usize::from(typed)).map(|s| s.url.clone())
                };
                self.unfocus(ui, t);
                if let Some(u) = url {
                    self.navigate(ui, &u, t);
                }
            }
            Target::Dismiss => self.unfocus(ui, t),
            Target::Content => self.tap_content(ui, x, y, t),
            Target::Keyboard => {}
        }
    }

    fn tap_content(&mut self, ui: &Ui, x: f32, y: f32, t: f64) {
        let ti = self.active;
        if self.tabs[ti].selection.take().is_some() {
            return;
        }
        let (px, py) = self.page_point(ui, x, y);
        let href = {
            let Some(v) = self.tabs[ti].view.as_ref() else { return };
            v.page.link_at(px, py).and_then(|l| nav::resolve(&v.url, &v.page.links[l]))
        };
        if let Some(url) = href {
            self.navigate(ui, &url, t);
        }
    }

    // --- frame loop ---

    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        for msg in self.loader.poll() {
            self.on_msg(ui, msg, t);
        }
        if self.process_dirty(ui) {
            self.dirty = true;
        }
        let keyboard_was_animating = self.keyboard.animating();
        for out in self.keyboard.tick(t) {
            self.key(ui, out, t);
        }
        self.fit_scroll(ui);
        let ti = self.active;
        let was_moving = self.tabs[ti].scroll.animating();
        let moving = self.tabs[ti].scroll.tick(t);
        if self.editing && self.url.tick(ui, t) {
            self.dirty = true;
        }
        if let Some((x, y, t0)) = self.press {
            if !self.long_fired && !self.tabs[ti].scroll.dragging() && t - t0 >= crate::shell::accessibility::long_press_ms(LONG_PRESS_MS) {
                self.long_fired = true;
                let (px, py) = self.page_point(ui, x, y);
                let block = self.tabs[ti].view.as_ref().and_then(|v| v.page.block_at(px, py, 6.0));
                if let Some(b) = block {
                    self.tabs[ti].selection = Some(Sel::Block(b));
                    self.dirty = true;
                }
            }
        }
        if self.toast.as_ref().is_some_and(|(_, until)| t >= *until) {
            self.toast = None;
            self.dirty = true;
        }
        self.dirty |= was_moving || moving || keyboard_was_animating || self.keyboard.shown() || self.tabs[ti].loading.is_some();
        self.dirty
    }

    /// When the app next needs a frame (animation, loading, a pending long press).
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        let tab = self.tab();
        if self.keyboard.animating() || tab.scroll.animating() {
            return Some(t);
        }
        let mut due: Option<f64> = None;
        let mut soon = |at: f64| due = Some(due.map_or(at, |d| d.min(at)));
        if tab.loading.is_some() || self.loader.busy() {
            soon(t + 30.0);
        }
        if let Some((_, _, t0)) = self.press.filter(|_| !self.long_fired) {
            soon(t0 + crate::shell::accessibility::long_press_ms(LONG_PRESS_MS));
        }
        if let Some((_, until)) = &self.toast {
            soon(*until);
        }
        if self.editing {
            if let Some(w) = self.url.next_wake(t) {
                soon(w);
            }
        }
        due
    }

    // --- drawing ---

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        self.dirty = false;
        let (w, h) = (self.w as usize, self.h as usize);
        s.fill_rect(0, 0, w, h, BLACK);
        let (top, bottom) = (self.content_top(ui), self.page_bottom(ui));
        let ti = self.active;
        let highlight = match self.tabs[ti].selection {
            Some(Sel::Block(b)) => Highlight::Block(b),
            Some(Sel::All) => Highlight::All,
            None => Highlight::None,
        };
        let scroll = self.tabs[ti].scroll.offset();
        match self.tabs[ti].view.as_mut() {
            Some(v) => {
                let vp = Viewport { x: 0.0, top, w: self.w as f32, h: bottom - top, scroll, scale: ui.s };
                self.painter.paint(s, ui, &mut v.images, &v.page, &vp, highlight);
            }
            None => {
                let label = self.labels.get(ui, &format!("Loading {}", nav::display_url(self.tabs[ti].url())), 17.0, Weight::Regular, SECONDARY);
                let (lw, lh) = (label.w as i32, label.h as i32);
                s.blit(label, (self.w - lw) / 2, ((top + bottom) / 2.0) as i32 - lh / 2, 255, 255);
            }
        }
        if let Some(menu) = self.selection_menu(ui) {
            menu.draw(s, ui);
        }
        if self.overview {
            self.draw_overview(s, ui);
            return;
        }
        self.draw_tab_bar(s, ui);
        self.draw_toolbar(s, ui, t);
        if self.editing {
            self.draw_suggestions(s, ui);
        }
        if self.keyboard.shown() || self.keyboard.animating() {
            let kb_top = self.kb_top(ui);
            self.keyboard.draw(s, ui, kb_top, t);
            s.fill_rect(0, (kb_top + self.keyboard.h() + self.keyboard.offset(t)) as usize, w, h, keyboard::BG);
        }
        if let Some((text, _)) = &self.toast {
            let img = self.labels.get(ui, text, 14.0, Weight::Medium, WHITE);
            let (bw, bh) = (img.w as f32 + ui.pt(32.0), ui.pt(38.0));
            let (bx, by) = ((self.w as f32 - bw) / 2.0, bottom - bh - ui.pt(24.0) - if self.keyboard.shown() { self.keyboard.h() } else { 0.0 });
            s.fill_rrect(bx, by, bw, bh, ui.pt(RADIUS_PT), BAR);
            s.blit(img, (bx + ui.pt(16.0)) as i32, (by + (bh - img.h as f32) / 2.0) as i32, 255, 255);
        }
    }

    fn draw_tab_bar(&mut self, s: &mut Surface, ui: &Ui) {
        let (top, bar_h) = (self.tab_top(ui), ui.pt(TAB_BAR_PT));
        s.fill_rect(0, top as usize, self.w as usize, bar_h as usize, TAB_IDLE);
        let tw = self.tab_w(ui);
        let titles: Vec<String> = self.tabs.iter().map(Tab::title).collect();
        for (i, title) in titles.iter().enumerate() {
            let x = i as f32 * tw;
            let active = i == self.active;
            if active {
                s.fill_rect(x as usize, top as usize, tw.ceil() as usize, bar_h as usize, TAB_ACTIVE);
            }
            let text = ui.fit(if title.is_empty() { "New Tab" } else { title }, 15.0, tw - ui.pt(96.0));
            let color = if active { WHITE } else { SECONDARY };
            let img = self.labels.get(ui, &text, 15.0, if active { Weight::Semibold } else { Weight::Medium }, color);
            s.blit(img, (x + (tw - img.w as f32) / 2.0) as i32, (top + (bar_h - img.h as f32) / 2.0) as i32, 255, 255);
            if active {
                s.fill_rrect(x + ui.pt(17.0), top + bar_h / 2.0 - ui.pt(5.5), ui.pt(11.0), ui.pt(11.0), ui.pt(2.5), SECONDARY);
                chrome::cross(s, (x + ui.pt(22.5), top + bar_h / 2.0), ui.pt(2.6), ui.pt(1.3), TAB_ACTIVE);
            }
        }
    }

    fn draw_toolbar(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        let (top, th) = (self.tool_top(ui), ui.pt(TOOLBAR_PT));
        s.fill_rect(0, 0, self.w as usize, (top + th) as usize, BAR);
        let tab = &self.tabs[self.active];
        let (can_back, can_forward) = (tab.pos > 0, tab.pos + 1 < tab.entries.len());
        let loading = tab.loading.is_some();
        let (bookmarkable, bookmarked, reader_on) = match &tab.view {
            Some(v) => (!nav::is_about(&v.url) && !v.failed, self.lib.is_bookmarked(&v.url), v.reader_on),
            None => (false, false, false),
        };
        let progress = loading.then(|| {
            let secs = ((t - tab.loading_since) / 1000.0).max(0.0) as f32;
            0.1 + 0.85 * (1.0 - (-secs / 2.5).exp())
        });
        let centre = |r: (f32, f32, f32, f32)| (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0);
        let (r, st) = (ui.pt(8.5), ui.pt(2.0));
        chrome::chevron(s, centre(self.button_rect(ui, 1)), r, st, if can_back { ACCENT } else { DISABLED }, true);
        chrome::chevron(s, centre(self.button_rect(ui, 2)), r, st, if can_forward { ACCENT } else { DISABLED }, false);
        let reload_at = centre(self.button_rect(ui, 11));
        if loading {
            chrome::cross(s, reload_at, ui.pt(6.0), ui.pt(1.7), WHITE);
        } else {
            chrome::reload(s, reload_at, ui.pt(7.5), ui.pt(1.7), WHITE);
        }
        chrome::star(s, centre(self.button_rect(ui, -3)), ui.pt(9.0), ui.pt(1.7), if bookmarkable { ACCENT } else { DISABLED }, bookmarked);
        let (rx, ry) = centre(self.button_rect(ui, 10));
        let page = if reader_on { ACCENT } else { WHITE };
        s.fill_rrect(rx - ui.pt(5.0), ry - ui.pt(7.0), ui.pt(10.0), ui.pt(14.0), ui.pt(2.0), page);
        s.fill_rrect(rx - ui.pt(4.0), ry - ui.pt(6.0), ui.pt(8.0), ui.pt(12.0), ui.pt(1.2), FIELD);
        for dy in [-3.0, 0.0, 3.0] {
            s.fill_rect((rx - ui.pt(2.5)) as usize, (ry + ui.pt(dy)) as usize, ui.pt(5.0) as usize, ui.pt(1.0).max(1.0) as usize, page);
        }
        chrome::squares(s, centre(self.button_rect(ui, -1)), ui.pt(9.0), ui.pt(1.7), ACCENT, self.tabs.len());
        chrome::plus(s, centre(self.button_rect(ui, -2)), ui.pt(9.0), ui.pt(1.8), ACCENT);
        chrome::book(s, centre(self.button_rect(ui, 0)), ui.pt(9.0), ui.pt(1.8), ACCENT);
        let (fx, fy, fw, fh) = self.url_rect(ui);
        s.fill_rrect(fx, fy, fw, fh, 1.0e4, FIELD);
        self.url.draw(s, ui, self.url_frame(ui), t);
        if let Some(p) = progress {
            s.fill_rect(0, (top + th - ui.pt(2.0)) as usize, (self.w as f32 * p) as usize, ui.pt(2.0).ceil() as usize, ACCENT);
        }
    }

    /// The tab overview: a grid of cards (title over address) with close badges.
    fn draw_overview(&mut self, s: &mut Surface, ui: &Ui) {
        s.fill_rect(0, 0, self.w as usize, self.h as usize, TAB_IDLE);
        let infos: Vec<(String, String)> = self.tabs.iter().map(|t| (t.title(), nav::display_url(t.url()))).collect();
        for (i, (title, addr)) in infos.iter().enumerate() {
            let (x, y, w, h) = self.card_rect(ui, i);
            s.fill_rrect(x, y, w, h, ui.pt(RADIUS_PT), if i == self.active { TAB_ACTIVE } else { CELL });
            let a = self.labels.get(ui, &ui.fit(if title.is_empty() { "New Tab" } else { title }, 15.0, w - ui.pt(48.0)), 15.0, Weight::Semibold, WHITE);
            s.blit(a, (x + ui.pt(12.0)) as i32, (y + ui.pt(12.0)) as i32, 255, 255);
            let b = self.labels.get(ui, &ui.fit(addr, 12.0, w - ui.pt(24.0)), 12.0, Weight::Regular, SECONDARY);
            s.blit(b, (x + ui.pt(12.0)) as i32, (y + ui.pt(36.0)) as i32, 255, 255);
            chrome::cross(s, (x + w - ui.pt(18.0), y + ui.pt(18.0)), ui.pt(5.0), ui.pt(1.6), SECONDARY);
        }
        let (bx, by, bw, bh) = self.overview_bar(ui);
        s.fill_rect(bx as usize, by as usize, bw as usize, bh as usize, BAR);
        chrome::plus(s, (ui.pt(BUTTON_PT), by + bh / 2.0), ui.pt(8.0), ui.pt(2.0), ACCENT);
        let done = self.labels.get(ui, "Done", 17.0, Weight::Semibold, ACCENT);
        s.blit(done, (self.w as f32 - ui.pt(BUTTON_PT) - done.w as f32 / 2.0) as i32, (by + (bh - done.h as f32) / 2.0) as i32, 255, 255);
    }

    fn draw_suggestions(&mut self, s: &mut Surface, ui: &Ui) {
        let top = self.content_top(ui) as usize;
        s.fill_rect(0, top, self.w as usize, (self.page_bottom(ui) as usize).saturating_sub(top), BLACK);
        let typed = self.url.text().trim().to_string();
        let mut rows: Vec<(String, String)> = vec![];
        if !typed.is_empty() {
            let is_url = nav::normalize_input(&typed).is_some_and(|u| !u.starts_with(nav::search_url("").as_str()));
            rows.push((typed.clone(), if is_url { "Go to address".into() } else { "Search the web".into() }));
        }
        rows.extend(self.suggestions.iter().map(|g| (if g.title.is_empty() { g.url.clone() } else { g.title.clone() }, nav::display_url(&g.url))));
        let m = ui.pt(20.0);
        for (i, (title, sub)) in rows.iter().enumerate() {
            let (rx, ry, rw, rh) = self.row_rect(ui, i);
            s.fill_rect(rx as usize, ry as usize, rw as usize, rh as usize, if i % 2 == 0 { CELL } else { BAR });
            let (t1, t2) = (ui.fit(title, 16.0, rw - 2.0 * m), ui.fit(sub, 12.0, rw - 2.0 * m));
            let a = self.labels.get(ui, &t1, 16.0, Weight::Regular, WHITE);
            s.blit(a, (rx + m) as i32, (ry + ui.pt(8.0)) as i32, 255, 255);
            let b = self.labels.get(ui, &t2, 12.0, Weight::Regular, SECONDARY);
            s.blit(b, (rx + m) as i32, (ry + rh - ui.pt(8.0) - b.h as f32) as i32, 255, 255);
        }
    }
}

#[cfg(test)]
mod tests;
