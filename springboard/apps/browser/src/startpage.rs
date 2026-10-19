//! Start Page (Browser): what a blank tab shows instead of an empty white page. A centred column under the toolbar with
//! Favorites (the bookmarks as rounded tiles), Frequently Visited (sites from history) and Recently Viewed (the latest
//! pages as a list), each under a title 2 heading. It is drawn natively over the page area, like the tab overview: the
//! tab stays on `about:blank`, so its address, history and session handling are untouched, and Back from a site returns
//! to it. Nothing is fetched: a tile shows the site icon `favicon.rs` already cached on disk, else a monogram on a colour
//! derived from the host. A private window shows a notice instead of the lists. There is no Privacy Report because the
//! Browser does not count blocked trackers yet.
//!
//! The data (`build`), the layout (`layout`, in points, scroll excluded) and the hit test are pure and unit tested here;
//! `State` keeps the scroll and the press, `starttab.rs` connects it to the Browser's input and drawing.
use std::collections::{HashMap, HashSet};

use bifrost_app::controls::prefs_layout::m;
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::kit::ui::system::{SYSTEM_BLUE, SYSTEM_GREEN, SYSTEM_INDIGO, SYSTEM_ORANGE, SYSTEM_PINK, SYSTEM_PURPLE, SYSTEM_RED, SYSTEM_TEAL, UNDER_PAGE_BACKGROUND};
use bifrost_app::kit::ui::{
    Weight, CARD_BG, DISPLAY_SMALL_PT, LABEL, LABEL_2, MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT, MAC_TITLE2_PT, ROW_PRESSED, SEPARATOR_C, SPACE_HUGE, SPACE_L, SPACE_M,
    SPACE_S, SPACE_XL, SPACE_XS, SPACE_XXL, SPACE_XXS, SPACE_XXXL, WHITE,
};
use bifrost_app::kit::{Surface, Ui};
use bifrost_app::webview::WebViewId;
use bifrost_app::TouchKind;

use crate::favicon::{self, Favicons};
use crate::store::{Entry, Store};
use crate::{suggest, tokens};

/// Bookmarks shown as Favorites (two full rows at the widest column).
pub const MAX_FAVORITES: usize = 16;
/// Sites under Frequently Visited (one full row).
pub const MAX_FREQUENT: usize = 8;
/// Pages under Recently Viewed.
pub const MAX_RECENT: usize = 8;
/// A site is frequently visited from this many visits on (all its pages together).
const FREQUENT_MIN_VISITS: u32 = 2;
/// Hues a monogram tile picks from.
pub const PALETTE_LEN: usize = 8;
/// Accessibility ids of the tiles and rows start here (`NODE0 + ordinal`); clear of the tabs and the main menu.
pub const NODE0: u64 = 0x20_0000;

const MAX_COLS: usize = 8;
/// The tile's rounded plate, points, and the gap to the next tile.
const TILE_PT: f32 = 60.0;
const TILE_GAP_PT: f32 = SPACE_XL;
/// A site icon on a tile, points.
const ICON_PT: f32 = 32.0;
/// Space under a tile for its label: the gap and one line of subhead text.
const LABEL_PT: f32 = MAC_SUBHEAD_PT + SPACE_S;
const TILE_H_PT: f32 = TILE_PT + LABEL_PT;
const ROW_GAP_PT: f32 = SPACE_L;
/// A section heading (title 2 line) and the gap under it.
const HEAD_PT: f32 = MAC_TITLE2_PT + SPACE_XS + SPACE_M;
const SECTION_GAP_PT: f32 = SPACE_XXXL + SPACE_S;
const TOP_PT: f32 = SPACE_HUGE + SPACE_XL;
const BOTTOM_PT: f32 = SPACE_XXXL;
/// Side margin the column keeps at narrow widths.
const MARGIN_PT: f32 = SPACE_XXL;
/// One row of Recently Viewed, and the longest host shown at its trailing edge.
const LIST_ROW_PT: f32 = 36.0;
const HOST_MAX_PT: f32 = 150.0;
/// Lines reserved for the empty-Favorites hint and for the private notice.
const HINT_LINES: usize = 2;
const PRIVATE_LINES: usize = 3;
const HINT_H_PT: f32 = HINT_LINES as f32 * (MAC_SUBHEAD_PT + SPACE_XS);
const PRIVATE_H_PT: f32 = HEAD_PT + PRIVATE_LINES as f32 * (MAC_SUBHEAD_PT + SPACE_XS);
/// Alpha of the dimming over a pressed tile.
const PRESS_ALPHA: u8 = 48;

const NO_FAVORITES: &str = "Bookmark a page and it appears here.";
const PRIVATE_TITLE: &str = "Private Browsing Enabled";
const PRIVATE_BODY: &str = "Pages you view in this window are not added to History, and its cookies and website data are deleted when you close it.";

// ---- data ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Favorites,
    Frequent,
    Recent,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Favorites => "Favorites",
            Section::Frequent => "Frequently Visited",
            Section::Recent => "Recently Viewed",
        }
    }
}

/// One tile or row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    pub url: String,
    /// The label: the page or bookmark name, else the host.
    pub title: String,
    /// The host without `www.` ("example.com"); the whole text for an address that is not a web page.
    pub name: String,
    /// The key of the cached site icon (`favicon::host_key`), for web addresses.
    pub host: Option<String>,
}

impl Site {
    fn new(url: &str, title: &str) -> Site {
        let name = crate::host_of(url);
        let title = title.trim();
        Site { url: url.to_string(), title: if title.is_empty() { name.clone() } else { title.to_string() }, name, host: favicon::host_key(url) }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Model {
    pub favorites: Vec<Site>,
    pub frequent: Vec<Site>,
    pub recent: Vec<Site>,
}

/// A tile or row: its section and its place in that section's list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub section: Section,
    pub index: usize,
}

impl Model {
    pub fn site(&self, t: Target) -> Option<&Site> {
        match t.section {
            Section::Favorites => self.favorites.get(t.index),
            Section::Frequent => self.frequent.get(t.index),
            Section::Recent => self.recent.get(t.index),
        }
    }

    /// The position of a target in Favorites, Frequently Visited, Recently Viewed order (its accessibility id offset).
    pub fn ordinal(&self, t: Target) -> usize {
        t.index
            + match t.section {
                Section::Favorites => 0,
                Section::Frequent => self.favorites.len(),
                Section::Recent => self.favorites.len() + self.frequent.len(),
            }
    }

    pub fn by_ordinal(&self, n: usize) -> Option<&Site> {
        let (f, q) = (self.favorites.len(), self.frequent.len());
        if n < f {
            self.favorites.get(n)
        } else if n < f + q {
            self.frequent.get(n - f)
        } else {
            self.recent.get(n - f - q)
        }
    }
}

/// Web pages and files only: history keeps nothing else, and a bookmark of a script or data address is not a tile.
fn listable(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://") || url.starts_with("file://")
}

/// What a site has earned in history: its pages' frecency added up, its visits, and the page that earned most.
struct Tally {
    score: f64,
    visits: u32,
    best: f64,
    url: String,
}

/// The three lists. Favorites are the bookmarks in their saved order. Frequently Visited is one tile per site, the sites
/// with the highest frecency first (visits weighted by how recent they are), at least `FREQUENT_MIN_VISITS` visits, and
/// not a site that is a favorite already; its tile opens the site's best page and is named by the host. Recently Viewed
/// is the history newest first.
pub fn build(store: &Store, now: u64) -> Model {
    let favorites: Vec<Site> = store.bookmarks.iter().filter(|e| listable(&e.url)).take(MAX_FAVORITES).map(|e| Site::new(&e.url, &e.title)).collect();
    let sites: HashMap<String, Tally> = {
        let shown: HashSet<&str> = favorites.iter().filter_map(|s| s.host.as_deref()).collect();
        let mut sites: HashMap<String, Tally> = HashMap::new();
        for e in &store.history {
            let Some(host) = favicon::host_key(&e.url) else { continue };
            if shown.contains(host.as_str()) {
                continue;
            }
            let f = suggest::frecency(e, now);
            let t = sites.entry(host).or_insert_with(|| Tally { score: 0.0, visits: 0, best: -1.0, url: String::new() });
            t.score += f;
            t.visits += e.visits;
            if f > t.best {
                t.best = f;
                t.url = e.url.clone();
            }
        }
        sites
    };
    let mut ranked: Vec<(String, Tally)> = sites.into_iter().filter(|(_, t)| t.visits >= FREQUENT_MIN_VISITS).collect();
    ranked.sort_by(|a, b| b.1.score.total_cmp(&a.1.score).then_with(|| a.0.cmp(&b.0)));
    let frequent: Vec<Site> = ranked.into_iter().take(MAX_FREQUENT).map(|(_, t)| Site::new(&t.url, "")).collect();
    let mut latest: Vec<&Entry> = store.history.iter().filter(|e| listable(&e.url)).collect();
    latest.sort_by(|a, b| b.last.cmp(&a.last).then_with(|| a.url.cmp(&b.url)));
    let recent: Vec<Site> = latest.into_iter().take(MAX_RECENT).map(|e| Site::new(&e.url, &e.title)).collect();
    Model { favorites, frequent, recent }
}

/// A cheap fingerprint of what `build` reads (sizes, visit and time totals, bookmark text length), so the lists are
/// rebuilt only when history or bookmarks changed.
type Stamp = (usize, usize, u64, u64);

fn stamp(store: &Store) -> Stamp {
    let seen = store.history.iter().fold(0u64, |a, e| a.wrapping_add(e.last).wrapping_add(e.visits as u64));
    let marks = store.bookmarks.iter().fold(0u64, |a, e| a.wrapping_add((e.url.len() + e.title.len()) as u64));
    (store.history.len(), store.bookmarks.len(), seen, marks)
}

// ---- monogram ----

/// The letter of a monogram tile: the first letter or digit of the host, upper-case; `?` when there is none.
pub fn initial(name: &str) -> char {
    let name = name.strip_prefix("www.").unwrap_or(name);
    name.chars().find(|c| c.is_alphanumeric()).map_or('?', |c| c.to_uppercase().next().unwrap_or(c))
}

/// The hue of a host's monogram: FNV-1a over the lower-case host without `www.`, so the colour of a site never changes
/// between runs, releases or capitalisation.
pub fn hue_index(name: &str) -> usize {
    let name = name.to_lowercase();
    let name = name.strip_prefix("www.").unwrap_or(&name);
    let mut h: u32 = 0x811c_9dc5;
    for b in name.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    (h % PALETTE_LEN as u32) as usize
}

/// The system colour of hue `i`; white letters stay readable on all of them in both appearances.
fn hue_color(i: usize) -> u32 {
    match i % PALETTE_LEN {
        0 => SYSTEM_RED.now(),
        1 => SYSTEM_ORANGE.now(),
        2 => SYSTEM_GREEN.now(),
        3 => SYSTEM_TEAL.now(),
        4 => SYSTEM_BLUE.now(),
        5 => SYSTEM_INDIGO.now(),
        6 => SYSTEM_PURPLE.now(),
        _ => SYSTEM_PINK.now(),
    }
}

// ---- layout (points, content coordinates: x from the window's left edge, y from the top of the page area, no scroll) ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hint {
    /// Favorites is empty.
    NoFavorites,
    /// A private window: the notice replaces the lists.
    Private,
}

/// What can be pressed: a tile (plate and label) or a row of the list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub target: Target,
    pub rect: Rect,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    /// The content column: left edge and width; `y` is 0 and `h` the content height.
    pub column: Rect,
    /// Tiles per row.
    pub cols: usize,
    /// Each shown heading and the top of its line.
    pub heads: Vec<(Section, f32)>,
    pub slots: Vec<Slot>,
    /// The grouped card behind Recently Viewed.
    pub card: Option<Rect>,
    pub hint: Option<(Hint, f32)>,
    /// Content height including the padding below.
    pub height: f32,
}

impl Layout {
    /// The tile or row under a point of the content.
    pub fn hit(&self, x: f32, y: f32) -> Option<Target> {
        self.slots.iter().find(|s| s.rect.contains(x, y)).map(|s| s.target)
    }
}

/// The rounded plate of a tile slot (the slot is a gap wider so that neighbours touch).
pub fn plate_of(slot: Rect) -> Rect {
    Rect { x: slot.x + TILE_GAP_PT / 2.0, y: slot.y, w: TILE_PT, h: TILE_PT }
}

/// Places `n` tiles of `section` in rows from `y`; returns the bottom of the last row.
fn place_tiles(out: &mut Layout, section: Section, n: usize, y: f32) -> f32 {
    let pitch = TILE_PT + TILE_GAP_PT;
    for i in 0..n {
        let (col, row) = (i % out.cols, i / out.cols);
        let rect = Rect { x: out.column.x + col as f32 * pitch - TILE_GAP_PT / 2.0, y: y + row as f32 * (TILE_H_PT + ROW_GAP_PT), w: pitch, h: TILE_H_PT };
        out.slots.push(Slot { target: Target { section, index: i }, rect });
    }
    let rows = n.div_ceil(out.cols);
    y + rows as f32 * TILE_H_PT + rows.saturating_sub(1) as f32 * ROW_GAP_PT
}

/// The column for a page area `width_pt` wide: at most eight tiles across, centred, narrower (down to one tile) when
/// the window is.
pub fn layout(model: &Model, private: bool, width_pt: f32) -> Layout {
    let widest = MAX_COLS as f32 * TILE_PT + (MAX_COLS - 1) as f32 * TILE_GAP_PT;
    let w = (width_pt - 2.0 * MARGIN_PT).max(TILE_PT).min(widest);
    let cols = (((w + TILE_GAP_PT) / (TILE_PT + TILE_GAP_PT)).floor() as usize).clamp(1, MAX_COLS);
    let mut out = Layout { column: Rect { x: ((width_pt - w) / 2.0).max(0.0), y: 0.0, w, h: 0.0 }, cols, ..Layout::default() };
    let mut y = TOP_PT;
    if private {
        out.hint = Some((Hint::Private, y));
        out.height = y + PRIVATE_H_PT + BOTTOM_PT;
        out.column.h = out.height;
        return out;
    }
    // Favorites always has its heading, so that an empty one can say how to fill it
    out.heads.push((Section::Favorites, y));
    y += HEAD_PT;
    if model.favorites.is_empty() {
        out.hint = Some((Hint::NoFavorites, y));
        y += HINT_H_PT;
    } else {
        y = place_tiles(&mut out, Section::Favorites, model.favorites.len(), y);
    }
    if !model.frequent.is_empty() {
        y += SECTION_GAP_PT;
        out.heads.push((Section::Frequent, y));
        y += HEAD_PT;
        y = place_tiles(&mut out, Section::Frequent, model.frequent.len(), y);
    }
    if !model.recent.is_empty() {
        y += SECTION_GAP_PT;
        out.heads.push((Section::Recent, y));
        y += HEAD_PT;
        out.card = Some(Rect { x: out.column.x, y, w, h: model.recent.len() as f32 * LIST_ROW_PT });
        for i in 0..model.recent.len() {
            let rect = Rect { x: out.column.x, y: y + i as f32 * LIST_ROW_PT, w, h: LIST_ROW_PT };
            out.slots.push(Slot { target: Target { section: Section::Recent, index: i }, rect });
        }
        y += model.recent.len() as f32 * LIST_ROW_PT;
    }
    out.height = y + BOTTOM_PT;
    out.column.h = out.height;
    out
}

/// The scroll offset (points) kept inside the content: 0 at the top, at most the content height minus the view's.
pub fn clamp_scroll(scroll: f32, content_h: f32, view_h: f32) -> f32 {
    if !scroll.is_finite() {
        return 0.0;
    }
    scroll.clamp(0.0, (content_h - view_h).max(0.0))
}

/// A blank tab: no address, or `about:blank`.
pub fn is_blank(url: &str) -> bool {
    url.is_empty() || url == "about:blank"
}

fn inside(area: (i32, i32, u32, u32), x: f32, y: f32) -> bool {
    let (ax, ay, aw, ah) = area;
    x >= ax as f32 && y >= ay as f32 && x < ax as f32 + aw as f32 && y < ay as f32 + ah as f32
}

// ---- state: scroll, press, drawing ----

/// What a touch on the Start Page asks of the Browser.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Not for the Start Page: the caller handles it.
    Pass,
    /// Used up.
    Used,
    /// A tile or row was tapped: load this address in the tab.
    Open(String),
}

#[derive(Default)]
pub struct State {
    stamp: Option<Stamp>,
    private: bool,
    model: Model,
    /// The tab the page was last drawn for; another tab starts at the top.
    view: Option<WebViewId>,
    /// Points scrolled from the top.
    scroll: f32,
    /// The tile or row a press began on.
    pressed: Option<Target>,
    /// Where the press began (y in pixels) and the scroll then.
    drag: Option<(f32, f32)>,
    /// The press moved far enough to be a scroll, not a tap.
    dragged: bool,
}

impl State {
    /// Notes which tab is drawn; a different one starts at the top.
    pub fn show(&mut self, view: WebViewId) {
        if self.view != Some(view) {
            self.view = Some(view);
            self.scroll = 0.0;
            self.pressed = None;
            self.drag = None;
            self.dragged = false;
        }
    }

    /// Rebuilds the lists when history or the bookmarks changed (or the window turned private).
    pub fn refresh(&mut self, store: &Store, private: bool, now: u64) {
        let stamp = stamp(store);
        if self.stamp == Some(stamp) && self.private == private {
            return;
        }
        self.stamp = Some(stamp);
        self.private = private;
        self.model = if private { Model::default() } else { build(store, now) };
    }

    fn layout_for(&self, area: (i32, i32, u32, u32), pt: f32) -> Layout {
        layout(&self.model, self.private, area.2 as f32 / pt)
    }

    /// The wheel or a two-finger scroll over the page area: `dy` pixels, positive moves the content down.
    pub fn wheel(&mut self, dy: f32, area: (i32, i32, u32, u32), pt: f32) {
        if area.2 == 0 || area.3 == 0 || pt <= 0.0 {
            return;
        }
        let lay = self.layout_for(area, pt);
        self.scroll = clamp_scroll(self.scroll - dy / pt, lay.height, area.3 as f32 / pt);
    }

    /// A finger or the pointer at window pixel (`x`, `y`). A press on a tile or row opens it when released on the same
    /// one; a drag past `slop` pixels scrolls instead. Only a press that began inside `area` (the page area in pixels)
    /// is used, so a move or release of any other touch passes.
    pub fn touch(&mut self, kind: TouchKind, x: f32, y: f32, area: (i32, i32, u32, u32), pt: f32, slop: f32) -> Outcome {
        if area.2 == 0 || area.3 == 0 || pt <= 0.0 {
            return Outcome::Pass;
        }
        let lay = self.layout_for(area, pt);
        let content = |x: f32, y: f32, scroll: f32| ((x - area.0 as f32) / pt, (y - area.1 as f32) / pt + scroll);
        match kind {
            TouchKind::Down => {
                if !inside(area, x, y) {
                    return Outcome::Pass;
                }
                let (cx, cy) = content(x, y, self.scroll);
                self.pressed = lay.hit(cx, cy);
                self.drag = Some((y, self.scroll));
                self.dragged = false;
                Outcome::Used
            }
            TouchKind::Move => {
                let Some((y0, scroll0)) = self.drag else { return Outcome::Pass };
                if (y - y0).abs() > slop {
                    self.dragged = true;
                    self.pressed = None;
                }
                if self.dragged {
                    self.scroll = clamp_scroll(scroll0 - (y - y0) / pt, lay.height, area.3 as f32 / pt);
                }
                Outcome::Used
            }
            TouchKind::Up => {
                if self.drag.take().is_none() {
                    return Outcome::Pass;
                }
                let dragged = std::mem::take(&mut self.dragged);
                let pressed = self.pressed.take();
                let (cx, cy) = content(x, y, self.scroll);
                match (pressed, lay.hit(cx, cy)) {
                    (Some(a), Some(b)) if !dragged && a == b => self.model.site(a).map_or(Outcome::Used, |s| Outcome::Open(s.url.clone())),
                    _ => Outcome::Used,
                }
            }
            TouchKind::Cancel => {
                let was = self.drag.take().is_some();
                self.pressed = None;
                self.dragged = false;
                if was {
                    Outcome::Used
                } else {
                    Outcome::Pass
                }
            }
        }
    }

    /// The tiles and rows in view as accessibility buttons (window points), labelled by title with the address as value.
    pub fn nodes(&self, area: (i32, i32, u32, u32), pt: f32) -> Vec<Node> {
        if area.2 == 0 || area.3 == 0 || pt <= 0.0 {
            return Vec::new();
        }
        let lay = self.layout_for(area, pt);
        let (left, top, view_h) = (area.0 as f32 / pt, area.1 as f32 / pt, area.3 as f32 / pt);
        lay.slots
            .iter()
            .filter_map(|slot| {
                let site = self.model.site(slot.target)?;
                let bounds = Rect { x: left + slot.rect.x, y: top + slot.rect.y - self.scroll, w: slot.rect.w, h: slot.rect.h };
                let shown = bounds.y + bounds.h > top && bounds.y < top + view_h;
                shown.then(|| Node {
                    id: NODE0 + self.model.ordinal(slot.target) as u64,
                    role: Role::Button,
                    label: site.title.clone(),
                    value: site.url.clone(),
                    bounds,
                    enabled: true,
                })
            })
            .collect()
    }

    /// The tile or row an accessibility id names.
    pub fn site_of_node(&self, id: u64) -> Option<&Site> {
        self.model.by_ordinal(usize::try_from(id.checked_sub(NODE0)?).ok()?)
    }

    /// Draws the page over `area` (window pixels), clipped to it. Call `show` and `refresh` first.
    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, area: (i32, i32, u32, u32), favicons: &mut Favicons) {
        let (ax, ay, aw, ah) = area;
        let pt = ui.pt(1.0);
        if aw == 0 || ah == 0 || pt <= 0.0 {
            return;
        }
        let lay = self.layout_for(area, pt);
        self.scroll = clamp_scroll(self.scroll, lay.height, ah as f32 / pt);
        let top = ay.max(0) as usize;
        let bottom = ((ay + ah as i32).max(0) as usize).min(s.h);
        if bottom <= top {
            return;
        }
        let mut band = s.rows(top, bottom);
        let s = &mut band;
        s.fill_rect(ax.max(0) as usize, top, aw as usize, bottom - top, UNDER_PAGE_BACKGROUND.now());
        // pixel origin of the content, and the rows of the band
        let origin = (ax as f32, ay as f32 - ui.pt(self.scroll));
        let (lo, hi) = (top as f32, bottom as f32);
        let column_x = origin.0 + ui.pt(lay.column.x);
        for &(section, y) in &lay.heads {
            let head = ui.text(section.title(), MAC_TITLE2_PT, Weight::Bold, LABEL.now());
            s.blit(&head, column_x as i32, (origin.1 + ui.pt(y)) as i32, 255, 255);
        }
        match lay.hint {
            Some((Hint::NoFavorites, y)) => {
                let lines = ui.wrap(NO_FAVORITES, MAC_SUBHEAD_PT, Weight::Regular, ui.pt(lay.column.w));
                for (i, line) in lines.iter().take(HINT_LINES).enumerate() {
                    let img = ui.text(line, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now());
                    s.blit(&img, column_x as i32, (origin.1 + ui.pt(y + i as f32 * (MAC_SUBHEAD_PT + SPACE_XS))) as i32, 255, 255);
                }
            }
            Some((Hint::Private, y)) => {
                let cx = column_x + ui.pt(lay.column.w) / 2.0;
                let head = ui.text(PRIVATE_TITLE, MAC_TITLE2_PT, Weight::Bold, LABEL.now());
                s.blit(&head, (cx - head.w as f32 / 2.0) as i32, (origin.1 + ui.pt(y)) as i32, 255, 255);
                let lines = ui.wrap(PRIVATE_BODY, MAC_SUBHEAD_PT, Weight::Regular, ui.pt(lay.column.w));
                for (i, line) in lines.iter().take(PRIVATE_LINES).enumerate() {
                    let img = ui.text(line, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now());
                    s.blit(&img, (cx - img.w as f32 / 2.0) as i32, (origin.1 + ui.pt(y + HEAD_PT + i as f32 * (MAC_SUBHEAD_PT + SPACE_XS))) as i32, 255, 255);
                }
            }
            None => {}
        }
        if let Some(c) = lay.card {
            s.fill_rrect(origin.0 + ui.pt(c.x), origin.1 + ui.pt(c.y), ui.pt(c.w), ui.pt(c.h), ui.pt(m::CARD_RADIUS), CARD_BG.now());
        }
        let last_row = self.model.recent.len().saturating_sub(1);
        for slot in &lay.slots {
            let Some(site) = self.model.site(slot.target) else { continue };
            let y0 = origin.1 + ui.pt(slot.rect.y);
            if y0 + ui.pt(slot.rect.h) < lo || y0 > hi {
                continue;
            }
            let pressed = self.pressed == Some(slot.target) && !self.dragged;
            if slot.target.section == Section::Recent {
                draw_row(s, ui, favicons, site, slot.rect, origin, pressed);
                if slot.target.index != last_row {
                    // the hairline between rows starts under the title, as in the shared lists
                    let x = origin.0 + ui.pt(slot.rect.x + SPACE_M + favicon::ICON_PT + SPACE_S);
                    let y = origin.1 + ui.pt(slot.rect.y + slot.rect.h);
                    if y >= lo && y < hi {
                        let w = ui.pt(slot.rect.x + slot.rect.w - SPACE_M) + origin.0 - x;
                        s.fill_rect(x.max(0.0) as usize, y as usize, w.max(0.0) as usize, 1, SEPARATOR_C.now());
                    }
                }
            } else {
                draw_tile(s, ui, favicons, site, slot.rect, origin, pressed);
            }
        }
    }
}

/// A tile: the cached site icon on a light plate, or the monogram on the host's colour, with the label centred under it.
fn draw_tile(s: &mut Surface, ui: &Ui, favicons: &mut Favicons, site: &Site, slot: Rect, origin: (f32, f32), pressed: bool) {
    let plate = plate_of(slot);
    let (x, y, side) = (origin.0 + ui.pt(plate.x), origin.1 + ui.pt(plate.y), ui.pt(plate.w));
    let radius = ui.pt(m::CARD_RADIUS);
    let icon_px = ui.pt(ICON_PT).round() as usize;
    match site.host.as_deref().and_then(|h| favicons.get(h, icon_px)) {
        Some(img) => {
            s.fill_rrect(x, y, side, side, radius, CARD_BG.now());
            s.stroke_rrect(x, y, side, side, radius, ui.pt(1.0).max(1.0), SEPARATOR_C.now());
            s.blit(img, (x + (side - img.w as f32) / 2.0) as i32, (y + (side - img.h as f32) / 2.0) as i32, 255, 255);
        }
        None => {
            s.fill_rrect(x, y, side, side, radius, hue_color(hue_index(&site.name)));
            let letter = ui.text(&initial(&site.name).to_string(), DISPLAY_SMALL_PT, Weight::Medium, WHITE);
            s.blit(&letter, (x + (side - letter.w as f32) / 2.0) as i32, (y + (side - letter.h as f32) / 2.0) as i32, 255, 255);
        }
    }
    if pressed {
        s.fill_rrect(x, y, side, side, radius, tokens::scrim(PRESS_ALPHA));
    }
    let room = ui.pt(TILE_PT + TILE_GAP_PT - SPACE_XS);
    let label = ui.text(&ui.fit(&site.title, MAC_SUBHEAD_PT, room), MAC_SUBHEAD_PT, Weight::Regular, LABEL.now());
    s.blit(&label, (x + (side - label.w as f32) / 2.0) as i32, (y + side + ui.pt(SPACE_S)) as i32, 255, 255);
}

/// A row of Recently Viewed on its card: icon (or a small monogram), title, and the host at the trailing edge.
fn draw_row(s: &mut Surface, ui: &Ui, favicons: &mut Favicons, site: &Site, rect: Rect, origin: (f32, f32), pressed: bool) {
    let (x, y, w, h) = (origin.0 + ui.pt(rect.x), origin.1 + ui.pt(rect.y), ui.pt(rect.w), ui.pt(rect.h));
    if pressed {
        s.fill_rrect(x + ui.pt(SPACE_XS), y + ui.pt(SPACE_XXS), w - ui.pt(SPACE_S), h - ui.pt(SPACE_XS), ui.pt(m::SELECTION_RADIUS), ROW_PRESSED.now());
    }
    let side = ui.pt(favicon::ICON_PT);
    let (ix, iy) = (x + ui.pt(SPACE_M), y + (h - side) / 2.0);
    match site.host.as_deref().and_then(|host| favicons.get(host, side.round() as usize)) {
        Some(img) => s.blit(img, ix as i32, iy as i32, 255, 255),
        None => {
            s.fill_rrect(ix, iy, side, side, ui.pt(m::ICON_RADIUS), hue_color(hue_index(&site.name)));
            let letter = ui.text(&initial(&site.name).to_string(), MAC_FOOTNOTE_PT, Weight::Bold, WHITE);
            s.blit(&letter, (ix + (side - letter.w as f32) / 2.0) as i32, (iy + (side - letter.h as f32) / 2.0) as i32, 255, 255);
        }
    }
    let tx = ix + side + ui.pt(SPACE_S);
    let right = x + w - ui.pt(SPACE_M);
    // the host is left out when it is the title already
    let host = (site.title != site.name).then(|| ui.text(&ui.fit(&site.name, MAC_SUBHEAD_PT, ui.pt(HOST_MAX_PT)), MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now()));
    let host_w = host.as_ref().map_or(0.0, |img| img.w as f32 + ui.pt(SPACE_M));
    if let Some(img) = &host {
        s.blit(img, (right - img.w as f32) as i32, (y + (h - img.h as f32) / 2.0) as i32, 255, 255);
    }
    let title = ui.text(&ui.fit(&site.title, MAC_BODY_PT, (right - host_w - tx).max(0.0)), MAC_BODY_PT, Weight::Regular, LABEL.now());
    s.blit(&title, tx as i32, (y + (h - title.h as f32) / 2.0) as i32, 255, 255);
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: (i32, i32, u32, u32) = (0, 100, 1280, 600);

    fn model_with(favorites: usize, frequent: usize, recent: usize) -> Model {
        let site = |tag: &str, i: usize| Site::new(&format!("https://{tag}-{i}.test/"), &format!("{tag} {i}"));
        Model {
            favorites: (0..favorites).map(|i| site("fav", i)).collect(),
            frequent: (0..frequent).map(|i| site("often", i)).collect(),
            recent: (0..recent).map(|i| site("seen", i)).collect(),
        }
    }

    fn titles(sites: &[Site]) -> Vec<&str> {
        sites.iter().map(|s| s.title.as_str()).collect()
    }

    #[test]
    fn favorites_keep_the_bookmark_order_and_unnamed_ones_take_the_host() {
        let mut st = Store::in_memory();
        st.add_bookmark("https://b.test/", "Bee", 1);
        st.add_bookmark("https://www.a.test/x", "  ", 2);
        st.add_bookmark("about:blank", "", 3);
        let m = build(&st, 100);
        assert_eq!(titles(&m.favorites), vec!["Bee", "a.test"]);
        assert_eq!(m.favorites[1].name, "a.test");
        assert_eq!(m.favorites[1].host.as_deref(), Some("www.a.test"));
        assert_eq!(m.favorites[0].url, "https://b.test/");
    }

    #[test]
    fn favorites_are_capped_and_skip_addresses_that_are_not_pages() {
        let mut st = Store::in_memory();
        st.add_bookmark("javascript:void(0)", "Script", 1);
        for i in 0..MAX_FAVORITES + 4 {
            st.add_bookmark(&format!("https://site-{i}.test/"), &format!("Site {i}"), 2 + i as u64);
        }
        let m = build(&st, 100);
        assert_eq!(m.favorites.len(), MAX_FAVORITES);
        assert_eq!(m.favorites[0].title, "Site 0", "the script bookmark is skipped, not counted");
        assert_eq!(m.favorites[MAX_FAVORITES - 1].title, format!("Site {}", MAX_FAVORITES - 1));
        assert!(m.favorites.iter().all(|s| s.url.starts_with("https://")));
    }

    #[test]
    fn frequently_visited_ranks_sites_by_frecency_and_leaves_out_favorites_and_one_offs() {
        let mut st = Store::in_memory();
        let now = 10 * 86_400;
        for _ in 0..3 {
            st.visit("https://a.test/1", "A", now);
        }
        for _ in 0..2 {
            st.visit("https://b.test/", "B", now - 9 * 86_400);
        }
        st.visit("https://c.test/", "C", now);
        for _ in 0..5 {
            st.visit("https://fav.test/", "F", now);
        }
        st.add_bookmark("https://fav.test/", "Fav", 1);
        let m = build(&st, now);
        assert_eq!(titles(&m.frequent), vec!["a.test", "b.test"]);
        assert_eq!(titles(&m.favorites), vec!["Fav"]);
    }

    #[test]
    fn a_frequent_site_adds_up_its_pages_and_opens_the_best_one() {
        let mut st = Store::in_memory();
        let now = 10 * 86_400;
        st.visit("https://a.test/1", "One", now - 86_400);
        st.visit("https://a.test/2", "Two", now);
        let m = build(&st, now);
        assert_eq!(m.frequent.len(), 1);
        assert_eq!(m.frequent[0].url, "https://a.test/2");
        assert_eq!(m.frequent[0].title, "a.test");
    }

    #[test]
    fn frequently_visited_is_capped_and_ties_break_by_host() {
        let mut st = Store::in_memory();
        for i in 0..MAX_FREQUENT + 3 {
            for _ in 0..2 {
                st.visit(&format!("https://h{i:02}.test/"), "", 50);
            }
        }
        let m = build(&st, 50);
        assert_eq!(m.frequent.len(), MAX_FREQUENT);
        assert_eq!(m.frequent[0].name, "h00.test");
        assert_eq!(m.frequent[MAX_FREQUENT - 1].name, format!("h{:02}.test", MAX_FREQUENT - 1));
    }

    #[test]
    fn recently_viewed_is_newest_first_capped_and_titled_by_host_when_untitled() {
        let mut st = Store::in_memory();
        st.visit("https://old.test/", "Old", 5);
        st.visit("https://new.test/", "", 9);
        st.visit("https://mid.test/", "Mid", 7);
        let m = build(&st, 100);
        assert_eq!(titles(&m.recent), vec!["new.test", "Mid", "Old"]);
        for i in 0..MAX_RECENT + 5 {
            st.visit(&format!("https://extra-{i}.test/"), "x", 20 + i as u64);
        }
        let m = build(&st, 100);
        assert_eq!(m.recent.len(), MAX_RECENT);
        assert_eq!(m.recent[0].url, format!("https://extra-{}.test/", MAX_RECENT + 4));
    }

    #[test]
    fn an_empty_profile_builds_empty_lists() {
        assert_eq!(build(&Store::in_memory(), 1), Model::default());
    }

    #[test]
    fn refresh_rebuilds_only_when_history_or_bookmarks_changed() {
        let mut st = Store::in_memory();
        let mut state = State::default();
        state.refresh(&st, false, 10);
        assert!(state.model.recent.is_empty());
        st.visit("https://a.test/", "A", 11);
        state.refresh(&st, false, 12);
        assert_eq!(state.model.recent.len(), 1);
        // the same store with another `now` keeps the lists
        state.model.recent.clear();
        state.refresh(&st, false, 13);
        assert!(state.model.recent.is_empty());
        // turning private empties the lists at once
        state.refresh(&st, true, 14);
        assert_eq!(state.model, Model::default());
    }

    #[test]
    fn monogram_letters_are_upper_case_and_skip_www_and_punctuation() {
        assert_eq!(initial("www.example.com"), 'E');
        assert_eq!(initial("example.com"), 'E');
        assert_eq!(initial("1password.com"), '1');
        assert_eq!(initial("-.é.test"), 'É');
        assert_eq!(initial(""), '?');
        assert_eq!(initial("..."), '?');
    }

    #[test]
    fn monogram_hues_are_stable_across_runs_and_ignore_case_and_www() {
        // pinned: a site must keep its colour from one release to the next
        assert_eq!(hue_index("example.com"), 6);
        assert_eq!(hue_index("b.test"), 1);
        assert_eq!(hue_index("wikipedia.org"), 0);
        assert_eq!(hue_index("WWW.Example.COM"), hue_index("example.com"));
        assert!(["a.test", "news.ycombinator.com", "bifrost.olibuijr.com", ""].iter().all(|h| hue_index(h) < PALETTE_LEN));
    }

    #[test]
    fn the_column_is_at_most_eight_tiles_wide_and_centred() {
        let lay = layout(&model_with(1, 0, 0), false, 1280.0);
        assert_eq!(lay.cols, 8);
        assert_eq!(lay.column.w, 620.0);
        assert_eq!(lay.column.x, 330.0);
        let narrow = layout(&model_with(1, 0, 0), false, 500.0);
        assert_eq!(narrow.cols, 5);
        assert_eq!(narrow.column.x, MARGIN_PT);
        assert_eq!(narrow.column.w, 500.0 - 2.0 * MARGIN_PT);
        let tiny = layout(&model_with(1, 0, 0), false, 40.0);
        assert_eq!(tiny.cols, 1);
        assert!(tiny.column.x >= 0.0 && tiny.column.w >= TILE_PT);
    }

    #[test]
    fn tiles_fill_a_row_then_wrap() {
        let lay = layout(&model_with(9, 0, 0), false, 1280.0);
        assert_eq!(lay.heads, vec![(Section::Favorites, TOP_PT)]);
        let first = lay.slots[0].rect;
        assert_eq!((first.x, first.y), (330.0 - TILE_GAP_PT / 2.0, TOP_PT + HEAD_PT));
        let second = lay.slots[1].rect;
        assert_eq!(second.x - first.x, TILE_PT + TILE_GAP_PT);
        let wrapped = lay.slots[8].rect;
        assert_eq!(wrapped.x, first.x);
        assert_eq!(wrapped.y, first.y + TILE_H_PT + ROW_GAP_PT);
        let plate = plate_of(first);
        assert_eq!((plate.x, plate.w, plate.h), (330.0, TILE_PT, TILE_PT));
    }

    #[test]
    fn empty_sections_are_left_out_and_an_empty_favorites_says_how_to_fill_it() {
        let lay = layout(&model_with(0, 0, 3), false, 1000.0);
        let sections: Vec<Section> = lay.heads.iter().map(|h| h.0).collect();
        assert_eq!(sections, vec![Section::Favorites, Section::Recent]);
        assert_eq!(lay.hint, Some((Hint::NoFavorites, TOP_PT + HEAD_PT)));
        let card = lay.card.unwrap();
        assert_eq!(card.h, 3.0 * LIST_ROW_PT);
        assert_eq!(lay.slots.len(), 3);
        assert_eq!(lay.slots[2].rect.y, card.y + 2.0 * LIST_ROW_PT);
        assert_eq!(lay.height, card.y + card.h + BOTTOM_PT);
        let full = layout(&model_with(2, 2, 2), false, 1000.0);
        assert!(full.hint.is_none());
        let order: Vec<Section> = full.heads.iter().map(|h| h.0).collect();
        assert_eq!(order, vec![Section::Favorites, Section::Frequent, Section::Recent]);
        assert!(full.heads.windows(2).all(|w| w[0].1 < w[1].1));
    }

    #[test]
    fn a_private_window_shows_the_notice_and_no_lists() {
        let lay = layout(&model_with(3, 3, 3), true, 1000.0);
        assert!(lay.heads.is_empty() && lay.slots.is_empty() && lay.card.is_none());
        assert_eq!(lay.hint, Some((Hint::Private, TOP_PT)));
        assert_eq!(lay.height, TOP_PT + PRIVATE_H_PT + BOTTOM_PT);
    }

    #[test]
    fn a_point_hits_the_tile_or_row_under_it_and_nothing_between() {
        let m = model_with(2, 1, 2);
        let lay = layout(&m, false, 1280.0);
        let tile = plate_of(lay.slots[1].rect);
        assert_eq!(lay.hit(tile.x + 5.0, tile.y + 5.0), Some(Target { section: Section::Favorites, index: 1 }));
        // the label below the plate belongs to the tile too
        assert_eq!(lay.hit(tile.x + 5.0, tile.y + TILE_PT + 4.0), Some(Target { section: Section::Favorites, index: 1 }));
        assert_eq!(lay.hit(1.0, 1.0), None);
        let row = lay.slots.last().unwrap().rect;
        assert_eq!(lay.hit(row.x + 4.0, row.y + 4.0), Some(Target { section: Section::Recent, index: 1 }));
        assert_eq!(m.ordinal(Target { section: Section::Recent, index: 1 }), 2 + 1 + 1);
        assert_eq!(m.by_ordinal(3).map(|s| s.title.as_str()), Some("seen 0"));
        assert_eq!(m.by_ordinal(5), None);
    }

    #[test]
    fn scroll_stays_inside_the_content() {
        assert_eq!(clamp_scroll(-5.0, 500.0, 300.0), 0.0);
        assert_eq!(clamp_scroll(250.0, 500.0, 300.0), 200.0);
        assert_eq!(clamp_scroll(80.0, 500.0, 300.0), 80.0);
        assert_eq!(clamp_scroll(40.0, 200.0, 300.0), 0.0);
        assert_eq!(clamp_scroll(f32::NAN, 500.0, 300.0), 0.0);
    }

    fn state_with(model: Model) -> State {
        State { model, ..State::default() }
    }

    #[test]
    fn a_tap_on_a_tile_opens_its_address_and_a_tap_elsewhere_does_nothing() {
        let mut st = state_with(model_with(3, 0, 0));
        let lay = layout(&st.model, false, 1280.0);
        let plate = plate_of(lay.slots[2].rect);
        let (x, y) = (plate.x + 10.0, AREA.1 as f32 + plate.y + 10.0);
        assert_eq!(st.touch(TouchKind::Down, x, y, AREA, 1.0, 6.0), Outcome::Used);
        assert_eq!(st.touch(TouchKind::Up, x, y, AREA, 1.0, 6.0), Outcome::Open("https://fav-2.test/".into()));
        // pressed on a tile, released on empty space
        assert_eq!(st.touch(TouchKind::Down, x, y, AREA, 1.0, 6.0), Outcome::Used);
        assert_eq!(st.touch(TouchKind::Up, 2.0, AREA.1 as f32 + 2.0, AREA, 1.0, 6.0), Outcome::Used);
        // a release with no press, and a press above the page area, are not the Start Page's
        assert_eq!(st.touch(TouchKind::Up, x, y, AREA, 1.0, 6.0), Outcome::Pass);
        assert_eq!(st.touch(TouchKind::Down, x, 10.0, AREA, 1.0, 6.0), Outcome::Pass);
    }

    #[test]
    fn a_drag_scrolls_instead_of_opening() {
        let mut st = state_with(model_with(4, 4, 8));
        let lay = layout(&st.model, false, 1280.0);
        assert!(lay.height > AREA.3 as f32, "the lists must overflow the area for this test");
        let plate = plate_of(lay.slots[0].rect);
        let (x, y) = (plate.x + 10.0, AREA.1 as f32 + plate.y + 10.0);
        assert_eq!(st.touch(TouchKind::Down, x, y, AREA, 1.0, 6.0), Outcome::Used);
        assert_eq!(st.touch(TouchKind::Move, x, y - 50.0, AREA, 1.0, 6.0), Outcome::Used);
        assert_eq!(st.scroll, 50.0);
        assert_eq!(st.touch(TouchKind::Up, x, y - 50.0, AREA, 1.0, 6.0), Outcome::Used);
        // a drag past the end stops at the end
        st.touch(TouchKind::Down, x, 400.0, AREA, 1.0, 6.0);
        st.touch(TouchKind::Move, x, -5000.0, AREA, 1.0, 6.0);
        assert_eq!(st.scroll, lay.height - AREA.3 as f32);
    }

    #[test]
    fn the_wheel_moves_the_content_and_stops_at_both_ends() {
        let mut st = state_with(model_with(4, 4, 8));
        let content = layout(&st.model, false, 1280.0).height;
        st.wheel(-120.0, AREA, 2.0);
        assert_eq!(st.scroll, 60.0);
        st.wheel(10_000.0, AREA, 2.0);
        assert_eq!(st.scroll, 0.0);
        st.wheel(-1_000_000.0, AREA, 2.0);
        assert_eq!(st.scroll, content - AREA.3 as f32 / 2.0);
    }

    #[test]
    fn tiles_in_view_are_published_with_label_address_and_a_stable_id() {
        let mut st = state_with(model_with(2, 1, 1));
        let nodes = st.nodes(AREA, 1.0);
        assert_eq!(nodes.len(), 4);
        assert_eq!(nodes[0].id, NODE0);
        assert_eq!(nodes[3].id, NODE0 + 3);
        assert_eq!(nodes[0].label, "fav 0");
        assert_eq!(nodes[0].value, "https://fav-0.test/");
        assert_eq!(st.site_of_node(NODE0 + 2).map(|s| s.url.as_str()), Some("https://often-0.test/"));
        assert_eq!(st.site_of_node(NODE0 + 9), None);
        assert_eq!(st.site_of_node(5), None);
        // scrolled far past the tiles, none is in view
        st.scroll = 10_000.0;
        assert!(st.nodes(AREA, 1.0).is_empty());
    }

    #[test]
    fn a_blank_tab_is_no_address_or_about_blank() {
        assert!(is_blank(""));
        assert!(is_blank("about:blank"));
        assert!(!is_blank("https://example.com/"));
        assert!(!is_blank("about:home"));
    }
}
