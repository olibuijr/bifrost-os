//! The News app: an RSS/Atom reader with a few default Icelandic feeds
//! (RUV, mbl.is, Visir). A tab strip picks All or one feed, the list shows
//! source, date and headline, and a tap opens a reader page with the
//! article text the feed carries. Feeds and pictures load on the shared
//! dispatch pool (`bifrost_app::dispatch`), cached on disk so the last headlines show offline, and the UI
//! thread only reads memory.

use bifrost_app::dispatch::{DispatchQueue, Qos};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::reading_list::ReadingList;
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::TextField;
use bifrost_app::KeyInput;
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets;
use bifrost_app::kit::ui::{SPACE_HUGE, SPACE_L, SPACE_M, SPACE_S, SPACE_XL};
use bifrost_app::kit::ui::{MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_LARGE_TITLE_PT, MAC_SUBHEAD_PT, MAC_TITLE1_PT};
use crate::feed::{self, Item};
use crate::reader_size;
use crate::services;
use crate::wire;
use bifrost_app::kit::ui::{home_zone, CARD_BG, CONTENT_BG, LABEL, LABEL_2, SEPARATOR_C};
use bifrost_app::kit::{Event, Surface, TextCache, Ui, Weight};
use bifrost_app::translation::TranslationMemory;
use springboard::deildu::scroll::Scroller;
use springboard::shell::music::radio::Fetch;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const TITLE_H: f32 = 52.0;
const INSET: f32 = 16.0;
/// the unified toolbar's bottom edge: the article page starts here
fn nav_bottom_pt() -> f32 { UNIFIED_BAR_PT }
const MARGIN: f32 = 24.0;
fn list_top_pt() -> f32 { nav_bottom_pt() + TITLE_H }
/// the source list (News' sidebar: Today, then one row per feed)
const SIDEBAR_W: f32 = 220.0;
/// the source list's rows start below the traffic lights
const SIDEBAR_SCROLL: f32 = -(UNIFIED_BAR_PT + 4.0);
const SIDEBAR_ROW: u32 = 100;
pub const BAR_BACK: u32 = 1;
pub const BAR_REFRESH: u32 = 2;
/// Saves the open article to the Reading List, or removes it.
pub const BAR_READ: u32 = 3;
/// The toolbar's search field: it filters the list by headline as the user types.
pub const BAR_SEARCH: u32 = 4;
/// The search field's width, and the clear button's hit width at its trailing end (points).
const SEARCH_W: f32 = 200.0;
const SEARCH_CLEAR_PT: f32 = 28.0;
/// Tab 0 is Today, tab 1 the Reading List, and tab n + 2 is feed n.
const READING_TAB: usize = 1;
const FEED_TAB: usize = 2;
/// The rich link card under an article: height at 1x.
const LINK_CARD_H: f32 = 64.0;
const ROW: f32 = 112.0;
/// the picture beside a headline, and its corner radius
const THUMB_W: f32 = 120.0;
const THUMB_H: f32 = 80.0;
const THUMB_R: f32 = bifrost_app::controls::prefs_layout::m::SELECTION_RADIUS;
/// at most this many pictures load at once
const PIC_JOBS: usize = 4;
const PIC_CACHE: usize = 240;
const RADIUS: f32 = bifrost_app::controls::prefs_layout::m::CARD_RADIUS;
const LINE: f32 = 24.0;
const REFRESH_MS: f64 = 10.0 * 60_000.0;
const RETRY_MS: f64 = 30_000.0;
const FEED_LIMIT: usize = 2_000_000;

/// The feeds a fresh install reads: (name, address).
pub const DEFAULT_FEEDS: &[(&str, &str)] = &[
    ("RUV", "https://www.ruv.is/rss/frettir"),
    ("mbl.is", "https://www.mbl.is/feeds/fp/"),
    ("Visir", "https://www.visir.is/rss/allt"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Back,
    Refresh,
    /// a source-list row: 0 is Today (all feeds), n is feed n - 1
    Feed(usize),
    Row(usize),
    /// the toolbar's Reading List button
    Read,
    /// the toolbar's search field: focuses it
    Search,
    /// the search field's clear button: empties the query
    ClearSearch,
}

enum Done {
    Feed { idx: usize, result: Result<Vec<Item>, String> },
    Picture { key: String, img: Option<springboard::gfx::Image> },
}

pub struct News {
    w: i32,
    h: i32,
    dir: PathBuf,
    fetcher: Arc<dyn Fetch>,
    /// results of finished jobs, taken by `pump` on the UI thread
    done: Arc<Mutex<Vec<Done>>>,
    feeds: Vec<(String, String)>,
    items: Vec<Vec<Item>>,
    /// 0 is All, n is feed n - 1
    tab: usize,
    list: Vec<Item>,
    tried: HashMap<usize, f64>,
    pending: HashSet<usize>,
    error: Option<String>,
    /// the open article and its wrapped title and body
    open: Option<(Item, Vec<String>, Vec<String>)>,
    scroll: Scroller,
    pressed: Option<Target>,
    scrolling: bool,
    labels: TextCache,
    dirty: bool,
    /// pictures by `pic_key` (None = the article has none, or it failed)
    pics: HashMap<String, Option<Arc<springboard::gfx::Image>>>,
    pic_pending: HashSet<String>,
    /// article ids whose picture load was skipped because the job queue was full; their rows retry
    pic_skipped: HashSet<String>,
    /// each visible row drawn once (text and picture), so scrolling only moves pixels
    rows: HashMap<usize, springboard::gfx::Image>,
    /// the card colour the cached rows were drawn with; a scheme change redraws them
    row_bg: u32,
    /// the toolbar as last drawn, for hits and the published controls
    bar: Option<ToolbarLayout>,
    /// the saved pages, kept in the app's data directory
    reading: ReadingList,
    /// the open article's facts line (language, words, reading time)
    facts: String,
    /// whether the shell reports a usable network path (true until it says otherwise); no feed is fetched while false
    online: bool,
    /// the headlines changed since the widget timeline was last sent
    top_changed: bool,
    /// the result of the last Translate command for the open article (a reason it could not run, or nothing)
    notice: Option<String>,
    /// the reader's text size step (see `reader_size`), kept across articles
    text_step: usize,
    /// the toolbar search query: the list keeps the headlines that contain it
    query: TextField,
    /// whether the search field has keyboard focus (typing then goes to the query)
    search_focus: bool,
}

/// `c` at the nearest end of the Light/Dark fade: pictures are rounded onto it once per scheme.
fn snap(c: bifrost_app::kit::appearance::Pair) -> u32 {
    if LABEL.now() == LABEL.dark { c.dark } else { c.light }
}

fn pic_key(item: &Item, w: usize, h: usize) -> String {
    format!("{w}x{h} {:x} {}", snap(CARD_BG), if item.link.is_empty() { &item.image } else { &item.link })
}

fn slug(i: usize) -> String {
    format!("feed{i}.xml")
}

/// Runs `job` on the Utility lane and parks its result in `done`; the main-queue hop wakes the loop,
/// and `pump` takes the result on the UI thread.
fn spawn(done: &Arc<Mutex<Vec<Done>>>, job: impl FnOnce() -> Done + Send + 'static) {
    let done = done.clone();
    DispatchQueue::global(Qos::Utility).async_(move || {
        let result = job();
        done.lock().unwrap().push(result);
        DispatchQueue::main().async_(|| {});
    });
}

impl News {
    pub fn new(_ui: &Ui, w: i32, h: i32, dir: PathBuf, fetcher: Arc<dyn Fetch>) -> Self {
        let feeds: Vec<(String, String)> = DEFAULT_FEEDS.iter().map(|(n, u)| (n.to_string(), u.to_string())).collect();
        let items = feeds
            .iter()
            .enumerate()
            .map(|(i, (name, _))| std::fs::read(dir.join(slug(i))).ok().map(|b| feed::parse(&String::from_utf8_lossy(&b), name)).unwrap_or_default())
            .collect();
        let reading = wire::load_reading_list(&dir);
        let mut n = News {
            w,
            h,
            dir,
            fetcher,
            done: Arc::new(Mutex::new(Vec::new())),
            feeds,
            items,
            tab: 0,
            list: vec![],
            tried: HashMap::new(),
            pending: HashSet::new(),
            error: None,
            open: None,
            scroll: Scroller::new(),
            pressed: None,
            scrolling: false,
            labels: TextCache::default(),
            dirty: true,
            pics: HashMap::new(),
            pic_pending: HashSet::new(),
            pic_skipped: HashSet::new(),
            rows: HashMap::new(),
            row_bg: 0,
            bar: None,
            reading,
            facts: String::new(),
            online: true,
            top_changed: true,
            notice: None,
            text_step: reader_size::DEFAULT_STEP,
            query: TextField::new(),
            search_focus: false,
        };
        n.rebuild();
        n
    }

    pub fn set_size(&mut self, w: i32, h: i32) {
        if (self.w, self.h) != (w, h) {
            self.w = w;
            self.h = h;
            self.rows.clear();
            self.dirty = true;
        }
    }

    /// The cached feeds, one list per feed, in feed order.
    pub fn items(&self) -> &[Vec<Item>] {
        &self.items
    }

    /// Records the shell's network path. Going online refetches every feed at once; going offline stops fetches
    /// and picture loads, and the cached headlines and saved pages stay readable.
    pub fn set_online(&mut self, online: bool) {
        if self.online == online {
            return;
        }
        self.online = online;
        if online {
            self.tried.clear();
            self.rows.clear();
        }
        self.dirty = true;
    }

    /// True once after the headlines changed (a feed arrived or was reloaded); the caller then sends the timeline.
    pub fn take_top_changed(&mut self) -> bool {
        std::mem::take(&mut self.top_changed)
    }

    /// Rebuilds the visible list: the chosen feed, or today's items from all feeds.
    fn rebuild(&mut self) {
        self.rows.clear();
        self.list = match self.tab {
            // Today: the day's headlines from every feed, newest first
            0 => feed::today_list(&self.items, (services::now_secs() / 86_400.0).floor() as i64),
            READING_TAB => wire::rows(&self.reading),
            tab => self.items.get(tab - FEED_TAB).cloned().unwrap_or_default(),
        };
        wire::search_rows(&mut self.list, &self.query);
    }

    // ---- fetching ----

    pub fn pump(&mut self) -> bool {
        let done = std::mem::take(&mut *self.done.lock().unwrap());
        let changed = !done.is_empty();
        let mut feed_changed = false;
        for d in done {
            let (idx, result) = match d {
                Done::Feed { idx, result } => (idx, result),
                Done::Picture { key, img } => {
                    self.pic_pending.remove(&key);
                    if self.pics.len() >= PIC_CACHE {
                        self.pics.clear();
                    }
                    // redraw only the rows showing this article, not every visible row (that was an
                    // 80-110 ms frame each time a picture arrived)
                    let ident = key.splitn(3, ' ').nth(2).unwrap_or("").to_string();
                    // a finished job frees a queue slot: rows that were skipped earlier retry too
                    let mut idents: HashSet<String> = std::mem::take(&mut self.pic_skipped);
                    idents.insert(ident);
                    let hit: Vec<usize> = self.list.iter().enumerate().filter(|(_, it)| idents.contains(if it.link.is_empty() { &it.image } else { &it.link })).map(|(i, _)| i).collect();
                    for i in hit {
                        self.rows.remove(&i);
                    }
                    self.pics.insert(key, img.map(Arc::new));
                    self.dirty = true;
                    continue;
                }
            };
            self.pending.remove(&idx);
            feed_changed = true;
            match result {
                Ok(items) => {
                    self.items[idx] = items;
                    self.error = None;
                }
                Err(e) => self.error = Some(e),
            }
        }
        if feed_changed {
            self.rebuild();
            self.dirty = true;
            self.top_changed = true;
        }
        changed
    }

    /// The picture for `item` at `w` x `h` px: Some(None) when it has none, None while it loads
    /// (the load is started here).
    fn picture(&mut self, item: &Item, w: usize, h: usize, r: usize, bg: u32) -> Option<Option<Arc<springboard::gfx::Image>>> {
        let key = pic_key(item, w, h);
        if let Some(p) = self.pics.get(&key) {
            return Some(p.clone());
        }
        if item.image.is_empty() && item.link.is_empty() {
            return Some(None);
        }
        if !self.online {
            // no request while offline; the picture loads once the path is back (set_online clears the rows)
            return None;
        }
        if !self.pic_pending.contains(&key) {
            if self.pic_pending.len() >= PIC_JOBS {
                self.pic_skipped.insert(key.splitn(3, ' ').nth(2).unwrap_or("").to_string());
            } else {
                self.pic_pending.insert(key.clone());
                let (fetcher, image, link) = (self.fetcher.clone(), item.image.clone(), item.link.clone());
                spawn(&self.done, move || Done::Picture { img: crate::pictures::load(&*fetcher, &image, &link, w, h, r, bg), key });
            }
        }
        None
    }

    /// One list row as an image: the cell, source and date, up to three lines of headline and the picture.
    fn render_row(&mut self, ui: &Ui, item: &Item, last: bool) -> springboard::gfx::Image {
        let (w, h) = (self.card_w(ui), ui.pt(ROW));
        let mut img = springboard::gfx::Image::new(w as usize, h as usize);
        let (tw, th) = (ui.pt(THUMB_W) as usize, ui.pt(THUMB_H) as usize);
        let pic = self.picture(item, tw, th, ui.pt(THUMB_R) as usize, snap(CARD_BG));
        let has_pic = !matches!(pic, Some(None));
        {
            let mut c = Surface { w: img.w, h: img.h, stride: img.w, px: &mut img.px, y0: 0 };
            c.fill_rect(0, 0, w as usize, h as usize, CARD_BG.now());
            if !last {
                c.fill_rect(ui.pt(INSET) as usize, h as usize - 1, (w - ui.pt(INSET)) as usize, 1, SEPARATOR_C.now());
            }
            let text_w = if has_pic { w - ui.pt(2.0 * INSET + THUMB_W + 12.0) } else { w - ui.pt(2.0 * INSET) };
            let (px, py) = (w - ui.pt(INSET) - tw as f32, (h - th as f32) / 2.0);
            match &pic {
                Some(Some(p)) => c.blit(p, px.round() as i32, py.round() as i32, 255, 255),
                // loading: a placeholder of the size the picture will take
                None => c.fill_rrect(px, py, tw as f32, th as f32, ui.pt(THUMB_R), SEPARATOR_C.now()),
                Some(None) => {}
            }
            let source = ui.fit(&item.source, MAC_FOOTNOTE_PT, text_w * 0.5);
            self.put(&mut c, ui, &source, MAC_FOOTNOTE_PT, Weight::Semibold, LABEL_2.now(), false, ui.pt(INSET), ui.pt(SPACE_XL));
            let sw = ui.measure(&source, MAC_FOOTNOTE_PT, Weight::Semibold);
            let date = ui.fit(&item.date, MAC_FOOTNOTE_PT, (text_w - sw - ui.pt(SPACE_S)).max(0.0));
            self.put(&mut c, ui, &date, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), false, ui.pt(INSET) + sw + ui.pt(SPACE_S), ui.pt(SPACE_XL));
            let lines = ui.wrap(&item.title, MAC_BODY_PT, Weight::Semibold, text_w);
            for (n, line) in lines.iter().take(3).enumerate() {
                let line = if n == 2 && lines.len() > 3 { format!("{}\u{2026}", ui.fit(line, MAC_BODY_PT, text_w - ui.pt(SPACE_XL))) } else { line.clone() };
                self.put(&mut c, ui, &line, MAC_BODY_PT, Weight::Semibold, LABEL.now(), false, ui.pt(INSET), ui.pt(42.0 + 21.0 * n as f32));
            }
        }
        img
    }

    /// How long a feed waits before its next fetch: a retry while it is empty, else the refresh interval.
    fn feed_wait(&self, idx: usize) -> f64 {
        if self.items[idx].is_empty() { RETRY_MS } else { REFRESH_MS }
    }

    fn maintain(&mut self, t: f64) {
        if !self.online {
            return;
        }
        for idx in 0..self.feeds.len() {
            if self.pending.contains(&idx) {
                continue;
            }
            if t - self.tried.get(&idx).copied().unwrap_or(f64::NEG_INFINITY) < self.feed_wait(idx) {
                continue;
            }
            self.tried.insert(idx, t);
            self.pending.insert(idx);
            let (name, url) = self.feeds[idx].clone();
            let (fetcher, path) = (self.fetcher.clone(), self.dir.join(slug(idx)));
            spawn(&self.done, move || {
                let result = fetcher.get(&url, FEED_LIMIT).map_err(|e| format!("{name}: {e}")).and_then(|bytes| {
                    let items = feed::parse(&String::from_utf8_lossy(&bytes), &name);
                    if items.is_empty() {
                        return Err(format!("{name}: no articles in the feed"));
                    }
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    let _ = std::fs::write(&path, &bytes);
                    Ok(items)
                });
                Done::Feed { idx, result }
            });
        }
    }

    // ---- layout and input ----

    fn bottom(&self, ui: &Ui) -> f32 {
        self.h as f32 - home_zone(ui)
    }

    /// the content column's left edge in px
    fn left(&self, ui: &Ui) -> f32 {
        ui.pt(SIDEBAR_W + MARGIN)
    }

    fn card_w(&self, ui: &Ui) -> f32 {
        (self.w as f32 - ui.pt(SIDEBAR_W + 2.0 * MARGIN)).max(ui.pt(THUMB_W + 2.0 * INSET))
    }

    /// The source list: Today (every feed), then the feeds under Sources; the open one is selected.
    fn sidebar(&self) -> SourceList {
        let mut items = vec![
            SidebarItem::row(SIDEBAR_ROW, "Today").with_symbol("doc"),
            SidebarItem::row(SIDEBAR_ROW + READING_TAB as u32, "Reading List").with_symbol("bookmark"),
            SidebarItem::header("Sources"),
        ];
        items.extend(self.feeds.iter().enumerate().map(|(i, (name, _))| SidebarItem::row(SIDEBAR_ROW + (FEED_TAB + i) as u32, name).with_symbol("globe")));
        let mut list = SourceList::new(items, 32.0, 30.0);
        list.set_selected(Some(SIDEBAR_ROW + self.tab as u32));
        list
    }

    fn sidebar_bounds(&self, ui: &Ui) -> Rect {
        Rect { x: 0.0, y: 0.0, w: SIDEBAR_W, h: self.h as f32 / ui.pt(1.0) }
    }

    /// The unified toolbar: Back while an article is open, then Refresh at the trailing end.
    fn toolbar(&self) -> Toolbar {
        let mut items = vec![];
        if self.open.is_some() {
            items.push(ToolbarItem::icon(BAR_BACK, "Back", "chevron.left"));
        }
        items.push(ToolbarItem::flexible_space());
        if let Some((item, ..)) = &self.open {
            let label = if self.reading.contains(&item.link) { "Remove from Reading List" } else { "Add to Reading List" };
            items.push(ToolbarItem::button(BAR_READ, label).enabled(ReadingList::supports(&item.link)));
        }
        items.push(ToolbarItem::button(BAR_REFRESH, if self.pending.is_empty() { "Refresh" } else { "Refreshing\u{2026}" }).enabled(self.pending.is_empty()));
        items.push(ToolbarItem::custom(BAR_SEARCH, "Search", SEARCH_W));
        Toolbar::new(items)
    }

    fn bar_bounds(&self, ui: &Ui) -> Rect {
        let x = SIDEBAR_W + 12.0;
        let h = widgets::CAPSULE_PT;
        Rect { x, y: (UNIFIED_BAR_PT - h) / 2.0, w: (self.w as f32 / ui.pt(1.0) - x - 16.0).max(0.0), h }
    }

    /// The toolbar buttons as controls, so the shell does not move the window from them and they can be pressed.
    pub fn controls(&self, ui: &Ui) -> Vec<Node> {
        let bar = self.toolbar();
        let mut nodes: Vec<Node> = self
            .bar
            .iter()
            .flat_map(|p| p.visible.iter())
            .filter_map(|(id, r)| {
                let search = *id == BAR_SEARCH;
                let role = if search { Role::TextField } else { Role::Button };
                let value = if search { self.query.to_string() } else { String::new() };
                bar.items.iter().find(|i| i.id == *id).map(|i| Node { id: *id as u64, role, label: i.label.clone(), value, bounds: *r, enabled: i.enabled })
            })
            .collect();
        // the source list's rows: Today, then each feed (the ids match `sidebar` and `activate_control`)
        let list = self.sidebar();
        let bounds = self.sidebar_bounds(ui);
        let names = ["Today", "Reading List"].into_iter().map(str::to_string).chain(self.feeds.iter().map(|(name, _)| name.clone()));
        for (tab, name) in names.enumerate() {
            // the list is Today, the Reading List, a header, then the feeds
            let index = if tab < FEED_TAB { tab } else { tab + 1 };
            nodes.push(Node { id: (SIDEBAR_ROW + tab as u32) as u64, role: Role::Row, label: name, value: String::new(), bounds: list.item_rect(index, bounds, SIDEBAR_SCROLL), enabled: true });
        }
        nodes
    }

    /// Presses control `id` (accessibility or the keyboard); false when it is not on the bar.
    pub fn activate_control(&mut self, ui: &Ui, id: u64) -> bool {
        let target = match id as u32 {
            BAR_BACK if self.open.is_some() => Target::Back,
            BAR_REFRESH => Target::Refresh,
            BAR_READ if self.open.is_some() => Target::Read,
            BAR_SEARCH => Target::Search,
            n if n >= SIDEBAR_ROW && (n - SIDEBAR_ROW) as usize <= READING_TAB + self.feeds.len() => Target::Feed((n - SIDEBAR_ROW) as usize),
            _ => return false,
        };
        self.activate(ui, target);
        true
    }

    fn content_h(&self, ui: &Ui) -> f32 {
        match &self.open {
            Some((_, title, body)) => self.hero_h(ui) + ui.pt(24.0 + 36.0 * title.len() as f32 + 40.0 + LINE * body.len().max(2) as f32 + 80.0),
            None => ui.pt(ROW * self.list.len() as f32 + 24.0),
        }
    }

    /// The article page's picture: full card width, 16:9, plus the gap under it.
    fn hero_size(&self, ui: &Ui) -> (usize, usize) {
        let w = self.card_w(ui) as usize;
        (w, w * 9 / 16)
    }

    fn hero_h(&self, ui: &Ui) -> f32 {
        match &self.open {
            Some((item, ..)) if !matches!(self.pics.get(&pic_key(item, self.hero_size(ui).0, self.hero_size(ui).1)), Some(None)) => self.hero_size(ui).1 as f32 + ui.pt(SPACE_S),
            _ => 0.0,
        }
    }

    fn fit_scroll(&mut self, ui: &Ui) {
        let top = if self.open.is_some() { nav_bottom_pt() } else { list_top_pt() };
        let view = (self.bottom(ui) - ui.pt(top)).max(0.0);
        let max = (self.content_h(ui) - view).max(0.0);
        self.scroll.set_max(max);
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let (px, py) = (x / ui.pt(1.0), y / ui.pt(1.0));
        if px < SIDEBAR_W {
            let id = self.sidebar().hit(self.sidebar_bounds(ui), SIDEBAR_SCROLL, px, py)?;
            return Some(Target::Feed((id - SIDEBAR_ROW) as usize));
        }
        if py < nav_bottom_pt() {
            return match self.bar.as_ref()?.hit(px, py)? {
                BAR_BACK => Some(Target::Back),
                BAR_REFRESH if self.pending.is_empty() => Some(Target::Refresh),
                BAR_READ => Some(Target::Read),
                BAR_SEARCH => {
                    let field = self.bar.as_ref()?.rect_of(BAR_SEARCH)?;
                    // the clear button sits at the trailing end, and only while there is a query
                    if !self.query.is_empty() && px >= field.right() - SEARCH_CLEAR_PT {
                        Some(Target::ClearSearch)
                    } else {
                        Some(Target::Search)
                    }
                }
                _ => None,
            };
        }
        if self.open.is_some() || y >= self.bottom(ui) || y < ui.pt(list_top_pt()) {
            return None;
        }
        if x < self.left(ui) || x > self.left(ui) + self.card_w(ui) {
            return None;
        }
        let i = ((y - ui.pt(list_top_pt()) + self.scroll.offset()) / ui.pt(ROW)) as usize;
        (i < self.list.len()).then_some(Target::Row(i))
    }

    /// Whether an article is open, so the Translate menu item is enabled.
    pub fn can_translate(&self) -> bool {
        self.open.is_some()
    }

    /// Whether an open article can be shared: its page must have a web address.
    pub fn share_link(&self) -> Option<String> {
        self.open.as_ref().map(|(item, ..)| item.link.clone()).filter(|link| ReadingList::supports(link))
    }

    /// Whether View > Larger Text (`larger`) or Smaller Text can act now: an article is open and the size can move.
    pub fn can_resize(&self, larger: bool) -> bool {
        let room = if larger { reader_size::can_grow(self.text_step) } else { reader_size::can_shrink(self.text_step) };
        self.open.is_some() && room
    }

    /// The body text size at the current step.
    fn body_pt(&self) -> f32 {
        MAC_BODY_PT * reader_size::factor(self.text_step)
    }

    /// Larger Text or Smaller Text for the open article: the body is wrapped again at the new size.
    pub fn resize_text(&mut self, ui: &Ui, larger: bool) {
        if !self.can_resize(larger) {
            return;
        }
        self.text_step = if larger { reader_size::larger(self.text_step) } else { reader_size::smaller(self.text_step) };
        let Some((item, title, body)) = self.open.take() else { return };
        let width = self.card_w(ui);
        let body = ui.wrap(&body.join(" "), self.body_pt(), Weight::Regular, width);
        self.open = Some((item, title, body));
        self.dirty = true;
    }

    /// Translate command: translates the open article into the other of English and Icelandic. On success the
    /// article shows the translation; otherwise the notice says why (for example, the language pair is not installed).
    pub fn translate(&mut self, ui: &Ui) {
        let Some(item) = self.open.as_ref().map(|(item, ..)| item.clone()) else { return };
        let width = self.card_w(ui);
        match wire::translate_article(&item, &wire::installed(), TranslationMemory::new()) {
            Ok((title, body)) => {
                let title = ui.wrap(&title, MAC_TITLE1_PT, Weight::Bold, width);
                let body = ui.wrap(&body, self.body_pt(), Weight::Regular, width);
                self.open = Some((item, title, body));
                self.notice = None;
            }
            Err(error) => self.notice = Some(format!("{}  {}", error.error_description(), error.failure_reason())),
        }
        self.dirty = true;
    }

    /// Whether the search field has keyboard focus; the shell shows the keyboard for it.
    pub fn search_focused(&self) -> bool {
        self.search_focus
    }

    /// A key press for the search field while it has focus: text is added, Backspace removes the last character,
    /// Return leaves the field, and Escape clears the query and leaves it. Other keys are not used here.
    pub fn key(&mut self, k: KeyInput) -> bool {
        if !self.search_focus {
            return false;
        }
        match k {
            KeyInput::Text(s) => {
                let clean: String = s.chars().filter(|c| !c.is_control()).collect();
                if clean.is_empty() {
                    return true;
                }
                self.query.push_str(&clean);
            }
            KeyInput::Code(8) => self.query.pop(),
            KeyInput::Code(13) => self.search_focus = false,
            KeyInput::Code(1) => {
                self.query.clear();
                self.search_focus = false;
            }
            KeyInput::Code(_) => return false,
        }
        self.rebuild();
        self.scroll.jump_top();
        self.dirty = true;
        true
    }

    fn activate(&mut self, ui: &Ui, target: Target) {
        self.notice = None;
        // any press outside the search field leaves it
        if !matches!(target, Target::Search | Target::ClearSearch) {
            self.search_focus = false;
        }
        if target == Target::Search {
            self.search_focus = true;
            self.dirty = true;
            return;
        }
        match target {
            Target::Search => {}
            Target::ClearSearch => {
                self.query.clear();
                self.search_focus = true;
                self.rebuild();
            }
            Target::Back => {
                self.open = None;
            }
            Target::Refresh => self.tried.clear(),
            Target::Read => {
                if let Some(item) = self.open.as_ref().map(|(item, ..)| item.clone()) {
                    wire::toggle(&mut self.reading, &item);
                    wire::save_reading_list(&self.dir, &self.reading);
                    if self.tab == READING_TAB {
                        self.rebuild();
                    }
                }
            }
            Target::Feed(i) => {
                self.open = None;
                self.tab = i;
                self.rebuild();
            }
            Target::Row(i) => {
                if let Some(item) = self.list.get(i).cloned() {
                    self.facts = wire::facts(&item.body);
                    let width = self.card_w(ui);
                    let title = ui.wrap(&item.title, MAC_TITLE1_PT, Weight::Bold, width);
                    let text = if item.body.is_empty() { "No preview in this feed.".to_string() } else { item.body.clone() };
                    let body = ui.wrap(&text, self.body_pt(), Weight::Regular, width);
                    self.open = Some((item, title, body));
                }
            }
        }
        self.scroll.jump_top();
        self.fit_scroll(ui);
        self.dirty = true;
    }

    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) {
        let (x, y) = match ev {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            _ => return,
        };
        self.dirty = true;
        match ev {
            Event::Down(..) => {
                self.pressed = self.hit(ui, x, y);
                let top = if self.open.is_some() { nav_bottom_pt() } else { list_top_pt() };
                if x >= ui.pt(SIDEBAR_W) && y >= ui.pt(top) && y < self.bottom(ui) {
                    self.scrolling = true;
                    self.scroll.down(y, t);
                }
            }
            Event::Move(..) => {
                if self.scrolling {
                    self.scroll.moved(y, t);
                    if self.scroll.dragging() {
                        self.pressed = None;
                    }
                }
            }
            Event::Up(..) => {
                let tap = if std::mem::take(&mut self.scrolling) { self.scroll.up(y, t) } else { true };
                let pressed = self.pressed.take();
                if tap {
                    if let Some(target) = pressed.filter(|p| self.hit(ui, x, y) == Some(*p)) {
                        self.activate(ui, target);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        self.maintain(t);
        self.fit_scroll(ui);
        let was = self.scroll.animating();
        let moving = self.scroll.tick(t);
        self.dirty |= was || moving;
        self.dirty
    }

    /// Sleeps until the next feed is due, or until the scroll settles. Nothing polls while a fetch or a
    /// picture is in flight: its job wakes the loop through the main queue when it finishes.
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        if self.scroll.animating() {
            return Some(t);
        }
        if !self.online {
            return None;
        }
        (0..self.feeds.len())
            .filter(|idx| !self.pending.contains(idx))
            .map(|idx| self.tried.get(&idx).map_or(t, |at| (at + self.feed_wait(idx)).max(t)))
            .reduce(f64::min)
    }

    // ---- drawing ----

    #[allow(clippy::too_many_arguments)]
    fn put(&mut self, s: &mut Surface, ui: &Ui, text: &str, size: f32, weight: Weight, color: u32, right: bool, x: f32, mid: f32) {
        let img = self.labels.get(ui, text, size, weight, color);
        let left = if right { x - img.w as f32 } else { x };
        s.blit(img, left.round() as i32, (mid - img.h as f32 / 2.0).round() as i32, 255, 255);
    }

    /// The sidebar and the unified toolbar over the content column.
    fn draw_chrome(&mut self, s: &mut Surface, ui: &Ui) {
        let side = ui.pt(SIDEBAR_W) as usize;
        s.fill_rect(side, 0, self.w as usize - side.min(self.w as usize), ui.pt(nav_bottom_pt()) as usize, CONTENT_BG.now());
        widgets::source_list(s, ui, self.sidebar_bounds(ui), &self.sidebar(), SIDEBAR_SCROLL, None);
        s.fill_rect(side.saturating_sub(1), 0, 1, self.h as usize, SEPARATOR_C.now());
        let band = Rect { x: SIDEBAR_W, y: 0.0, w: self.w as f32 / ui.pt(1.0) - SIDEBAR_W, h: UNIFIED_BAR_PT };
        widgets::toolbar_band(s, ui, band);
        let pressed = match self.pressed {
            Some(Target::Back) => Some(BAR_BACK),
            Some(Target::Refresh) => Some(BAR_REFRESH),
            Some(Target::Read) => Some(BAR_READ),
            _ => None,
        };
        self.bar = Some(widgets::toolbar(s, ui, self.bar_bounds(ui), &self.toolbar(), pressed));
        if let Some(r) = self.bar.as_ref().and_then(|b| b.rect_of(BAR_SEARCH)) {
            widgets::search_field(s, ui, &self.query, Rect { x: r.x, y: r.y + 4.0, w: r.w, h: r.h - 8.0 }, "Search", self.search_focus);
        }
    }

    fn draw_list(&mut self, s: &mut Surface, ui: &Ui) {
        let (top, bottom) = (ui.pt(list_top_pt()) as usize, self.bottom(ui) as usize);
        let (x, w) = (self.left(ui), self.card_w(ui));
        if self.row_bg != CARD_BG.now() {
            self.row_bg = CARD_BG.now();
            self.rows.clear();
        }
        let list = std::mem::take(&mut self.list);
        {
            let mut c = s.rows(top, bottom);
            let oy = ui.pt(list_top_pt()) - self.scroll.offset();
            if list.is_empty() {
                let msg = match (&self.error, self.pending.is_empty()) {
                    (_, false) => "Loading headlines\u{2026}".to_string(),
                    _ if !self.online && self.tab != READING_TAB => "You are offline".to_string(),
                    (Some(e), true) => e.clone(),
                    _ if !self.query.is_empty() => "No matching articles".to_string(),
                    _ if self.tab == READING_TAB => "Nothing saved yet".to_string(),
                    _ => "No headlines".to_string(),
                };
                let msg = ui.fit(&msg, MAC_SUBHEAD_PT, w);
                self.put(&mut c, ui, &msg, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), false, x, oy + ui.pt(SPACE_HUGE));
            }
            let mut fresh = 0;
            for (i, item) in list.iter().enumerate() {
                let y = oy + i as f32 * ui.pt(ROW);
                if y + ui.pt(ROW) < top as f32 || y > bottom as f32 {
                    continue;
                }
                if !self.rows.contains_key(&i) {
                    // at most one new row per frame (each wraps text and rasterises it); the rest
                    // show an empty cell for a frame or two
                    if fresh >= 1 {
                        c.fill_rect(x as usize, y.max(0.0) as usize, w as usize, ui.pt(ROW) as usize, CARD_BG.now());
                        self.dirty = true;
                        continue;
                    }
                    fresh += 1;
                    let row = self.render_row(ui, item, i + 1 == list.len());
                    self.rows.insert(i, row);
                }
                // rows are opaque: a row copy, not a per-pixel blend
                c.copy(self.rows[&i].view(), x.round() as i32, y.round() as i32);
            }
            // a frame that drew no new row prepares one just past the screen edge (below, then above),
            // so rows scrolled into view are usually ready
            if fresh == 0 {
                let vis0 = (self.scroll.offset() / ui.pt(ROW)) as usize;
                let vis1 = vis0 + ((bottom - top) as f32 / ui.pt(ROW)).ceil() as usize;
                let next = (vis1..(vis1 + 3).min(list.len())).chain((vis0.saturating_sub(2)..vis0).rev()).find(|i| !self.rows.contains_key(i));
                if let Some(i) = next {
                    let row = self.render_row(ui, &list[i], i + 1 == list.len());
                    self.rows.insert(i, row);
                    self.dirty = true;
                }
            }
            // keep only rows near the screen
            let first = ((self.scroll.offset() / ui.pt(ROW)) as usize).saturating_sub(8);
            let visible = ((bottom - top) as f32 / ui.pt(ROW)) as usize + 16;
            self.rows.retain(|&i, _| i >= first && i < first + visible);
        }
        self.list = list;
        s.fill_rect(0, 0, self.w as usize, top, CONTENT_BG.now());
        self.draw_chrome(s, ui);
        let title = match self.tab {
            0 => "Today".to_string(),
            READING_TAB => "Reading List".to_string(),
            tab => self.feeds[tab - FEED_TAB].0.clone(),
        };
        let title = ui.fit(&title, MAC_LARGE_TITLE_PT, w);
        // compliance-allow: LAY-07 the title is on the list column's leading edge; widgets::large_title centres on the readable column
        self.put(s, ui, &title, MAC_LARGE_TITLE_PT, Weight::Bold, LABEL.now(), false, x, ui.pt(nav_bottom_pt() + TITLE_H / 2.0));
    }

    fn draw_article(&mut self, s: &mut Surface, ui: &Ui) {
        let (top, bottom) = (ui.pt(nav_bottom_pt()) as usize, self.bottom(ui) as usize);
        let x = self.left(ui);
        let Some((item, title, body)) = self.open.take() else { return };
        {
            let mut c = s.rows(top, bottom);
            let mut y = ui.pt(nav_bottom_pt()) - self.scroll.offset() + ui.pt(SPACE_L);
            let (hw, hh) = self.hero_size(ui);
            match self.picture(&item, hw, hh, ui.pt(RADIUS) as usize, snap(CONTENT_BG)) {
                Some(Some(p)) => {
                    c.blit(&p, x.round() as i32, y.round() as i32, 255, 255);
                    y += hh as f32 + ui.pt(SPACE_S);
                }
                None => {
                    c.fill_rrect(x, y, hw as f32, hh as f32, ui.pt(RADIUS), CARD_BG.now());
                    y += hh as f32 + ui.pt(SPACE_S);
                }
                Some(None) => {}
            }
            y += ui.pt(SPACE_S);
            for line in &title {
                self.put(&mut c, ui, line, MAC_TITLE1_PT, Weight::Bold, LABEL.now(), false, x, y + ui.pt(SPACE_XL));
                y += ui.pt(36.0);
            }
            let source = format!("{}  {}", item.source, item.date);
            let meta = match (source.trim(), self.facts.as_str()) {
                ("", facts) => facts.to_string(),
                (source, "") => source.to_string(),
                (source, facts) => format!("{source}  \u{b7}  {facts}"),
            };
            let meta = ui.fit(&meta, MAC_FOOTNOTE_PT, self.card_w(ui));
            self.put(&mut c, ui, &meta, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), false, x, y + ui.pt(SPACE_M));
            y += ui.pt(SPACE_HUGE);
            if let Some(notice) = self.notice.clone() {
                let line = ui.fit(&notice, MAC_FOOTNOTE_PT, self.card_w(ui));
                self.put(&mut c, ui, &line, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), false, x, y + ui.pt(LINE / 2.0));
                y += ui.pt(LINE);
            }
            let (body_size, line_h) = (self.body_pt(), ui.pt(LINE) * reader_size::factor(self.text_step));
            for line in &body {
                if y + line_h >= top as f32 && y <= bottom as f32 {
                    self.put(&mut c, ui, line, body_size, Weight::Regular, LABEL.now(), false, x, y + line_h / 2.0);
                }
                y += line_h;
            }
            // the link card: the article's own page, titled from the feed
            if let Some(card) = wire::link_card(&item) {
                let (cw, top_y) = (self.card_w(ui), y + ui.pt(SPACE_M));
                c.fill_rrect(x, top_y, cw, ui.pt(LINK_CARD_H), ui.pt(RADIUS), CARD_BG.now());
                let text_w = cw - ui.pt(SPACE_L) * 2.0;
                let card_title = ui.fit(&card.title, MAC_BODY_PT, text_w);
                let site = ui.fit(&card.subtitle, MAC_FOOTNOTE_PT, text_w);
                self.put(&mut c, ui, &card_title, MAC_BODY_PT, Weight::Bold, LABEL.now(), false, x + ui.pt(SPACE_L), top_y + ui.pt(LINK_CARD_H * 0.36));
                self.put(&mut c, ui, &site, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), false, x + ui.pt(SPACE_L), top_y + ui.pt(LINK_CARD_H * 0.72));
            }
        }
        self.open = Some((item, title, body));
        s.fill_rect(0, 0, self.w as usize, top, CONTENT_BG.now());
        self.draw_chrome(s, ui);
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, _t: f64) {
        self.dirty = false;
        s.fill_rect(0, 0, self.w as usize, self.h as usize, CONTENT_BG.now());
        if self.open.is_some() {
            self.draw_article(s, ui);
        } else {
            self.draw_list(s, ui);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_feeds_are_icelandic() {
        assert!(DEFAULT_FEEDS.iter().any(|(_, u)| u.contains("ruv.is")));
        assert!(DEFAULT_FEEDS.iter().any(|(_, u)| u.contains("mbl.is")));
    }
}
