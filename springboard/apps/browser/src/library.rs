//! Bookmarks UX (Browser): the Cmd+D popover that names a bookmark, the Favorites bar under the toolbar (View > Show
//! Favorites Bar, Cmd+Shift+B) with an overflow chevron menu, and the Bookmarks / History panel (Cmd+Option+B,
//! Cmd+Y) with search, history grouped by day, Delete to remove and Return or a double click to open. The data is
//! `store.rs`; this module only holds view state, layout, drawing and the pure row building (unit tested).
use bifrost_app::controls::prefs_layout::m;
use bifrost_app::controls::{Command, Rect, TextField};
use bifrost_app::kit::ui::{Weight, CAPSULE_SHADOW, CARD_BG, DESTRUCTIVE, FIELD_BG, LABEL, LABEL_2, MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT, ROW_PRESSED, SEGMENT_ON, SEPARATOR_C, TINT, WHITE};
use bifrost_app::kit::{Surface, Ui};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::controls::bookmark_import;
use bifrost_app::{glyphs, widgets, Ctx, KeyInput};
use std::fs;
use std::path::{Path, PathBuf};

use crate::store::{Entry, Store};

const PREF_FAVBAR: &str = "browser.favorites_bar";

/// Favorites bar height, points.
pub const FAV_BAR_PT: f32 = 28.0;
const FAV_BTN_MAX_PT: f32 = 150.0;
const FAV_PAD_PT: f32 = 8.0;
const FAV_GAP_PT: f32 = 2.0;
const CHEVRON_PT: f32 = 24.0;
const MENU_W_PT: f32 = 260.0;
const MENU_ROW_PT: f32 = 24.0;
const MENU_MAX_ROWS: usize = 14;

const PANEL_W_PT: f32 = 520.0;
/// Longest host shown at the trailing edge of a row (points).
const HOST_MAX_PT: f32 = 150.0;
const PANEL_MAX_H_PT: f32 = 480.0;
const PAD_PT: f32 = 14.0;
const ROW_PT: f32 = 26.0;
const DOUBLE_TAP_S: f64 = 0.4;

const POP_W_PT: f32 = 300.0;
const POP_H_PT: f32 = 112.0;
const ARROW_PT: f32 = 6.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    #[default]
    Bookmarks,
    History,
}

/// What the owner must do after an input.
#[derive(Debug, PartialEq, Eq)]
pub enum Act {
    /// Nothing for the owner (the input was used).
    None,
    /// The input is not for the library; the caller handles it.
    Pass,
    /// Load this URL in the current tab.
    Open(String),
}

/// A row of the panel list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Header(String),
    Item { url: String, title: String },
}

struct Pop {
    url: String,
    field: TextField,
    /// Horizontal centre the arrow points at, points.
    anchor: f32,
}

#[derive(Default)]
pub struct Library {
    /// View > Show Favorites Bar.
    pub favbar: bool,
    pub open: bool,
    pub mode: Mode,
    query: TextField,
    sel: Option<usize>,
    first: usize,
    last_tap: Option<(usize, f64)>,
    overflow: bool,
    pop: Option<Pop>,
}

// ---- pure helpers ----

/// Civil date (year, month 1-12, day 1-31) of a day count since 1970-01-01 (proleptic Gregorian).
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if mo <= 2 { y + 1 } else { y }, mo, d)
}

/// Day group title: Today, Yesterday, else "Monday, October 5, 2026" (UTC days; the engine gives no time zone).
pub fn day_label(last: u64, now: u64) -> String {
    let (day, today) = ((last / 86_400) as i64, (now / 86_400) as i64);
    match today - day {
        0 => return "Today".into(),
        1 => return "Yesterday".into(),
        _ => {}
    }
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    let (y, mo, d) = civil_from_days(day);
    format!("{}, {} {}, {}", DAYS[((day + 4).rem_euclid(7)) as usize], MONTHS[mo as usize - 1], d, y)
}

pub fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}

/// Adds the links of a bookmark export (the HTML form browsers write) to the store. Returns how many were added.
pub fn import_html(store: &mut Store, html: &str, at: u64) -> usize {
    let links: Vec<(String, String)> = bookmark_import::bookmarks(&bookmark_import::parse(html)).into_iter().map(|b| (b.url, b.title)).collect();
    store.import_bookmarks(&links, at)
}

/// The newest `*.html` file in `dir` whose name contains "bookmark" (what a browser saves its bookmark export as).
pub fn newest_export(dir: &Path) -> Option<PathBuf> {
    fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| is_export_name(p))
        .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
}

fn is_export_name(p: &Path) -> bool {
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
    name.contains("bookmark") && (name.ends_with(".html") || name.ends_with(".htm"))
}

fn label(e: &Entry) -> String {
    if e.title.trim().is_empty() {
        e.url.clone()
    } else {
        e.title.clone()
    }
}

/// Case-insensitive match of every space-separated word against title and URL.
pub fn matches(e: &Entry, query: &str) -> bool {
    let hay = format!("{} {}", e.title, e.url).to_lowercase();
    query.to_lowercase().split_whitespace().all(|w| hay.contains(w))
}

/// Bookmarks in saved order, or history newest first under day headers; filtered by `query`.
pub fn rows(store: &Store, mode: Mode, query: &str, now: u64) -> Vec<Row> {
    match mode {
        Mode::Bookmarks => store.bookmarks.iter().filter(|e| matches(e, query)).map(|e| Row::Item { url: e.url.clone(), title: label(e) }).collect(),
        Mode::History => {
            let mut items: Vec<&Entry> = store.history.iter().filter(|e| matches(e, query)).collect();
            items.sort_by(|a, b| b.last.cmp(&a.last));
            let mut out = Vec::new();
            let mut cur = String::new();
            for e in items {
                let l = day_label(e.last, now);
                if l != cur {
                    out.push(Row::Header(l.clone()));
                    cur = l;
                }
                out.push(Row::Item { url: e.url.clone(), title: label(e) });
            }
            out
        }
    }
}

fn is_item(r: &Row) -> bool {
    matches!(r, Row::Item { .. })
}

/// Next selectable row from `from` in direction `dir` (+1/-1), staying put at the ends.
fn step(rows: &[Row], from: Option<usize>, dir: i32) -> Option<usize> {
    let n = rows.len() as i32;
    let mut i = match from {
        Some(i) => i as i32 + dir,
        None if dir > 0 => 0,
        None => n - 1,
    };
    while (0..n).contains(&i) {
        if is_item(&rows[i as usize]) {
            return Some(i as usize);
        }
        i += dir;
    }
    from.or_else(|| rows.iter().position(is_item))
}

struct Geo {
    panel: Rect,
    seg: [Rect; 2],
    done: Rect,
    search: Rect,
    list: Rect,
    foot_y: f32,
}

impl Library {
    pub fn load(&mut self, ctx: &Ctx) {
        let d = ctx.defaults();
        if d.contains(PREF_FAVBAR) {
            self.favbar = d.bool(PREF_FAVBAR);
        }
    }

    pub fn save(&self, ctx: &Ctx) {
        ctx.defaults_mut().set_bool(PREF_FAVBAR, self.favbar);
    }

    /// Height the Favorites bar takes from the content, points.
    pub fn bar_height(&self) -> f32 {
        if self.favbar {
            FAV_BAR_PT
        } else {
            0.0
        }
    }

    /// Something of the library is on screen above the page (panel, popover or overflow menu).
    pub fn overlay(&self) -> bool {
        self.open || self.pop.is_some() || self.overflow
    }

    /// The panel or the popover takes typed keys.
    pub fn captures_keys(&self) -> bool {
        self.open || self.pop.is_some()
    }

    pub fn show_panel(&mut self, mode: Mode) {
        self.open = true;
        self.mode = mode;
        self.query.clear();
        self.sel = None;
        self.first = 0;
        self.last_tap = None;
        self.overflow = false;
    }

    pub fn close_all(&mut self, store: &mut Store) {
        self.open = false;
        self.overflow = false;
        self.close_pop(store);
    }

    /// Cmd+D: bookmark the page if it is not one yet and show the naming popover (Remove is in it).
    pub fn bookmark_page(&mut self, store: &mut Store, url: &str, title: &str, now: u64, anchor: f32) {
        if !store.add_bookmark(url, title, now) {
            return;
        }
        let name = store.bookmark(url).map(label).unwrap_or_default();
        let mut field = TextField::default();
        field.push_str(&name);
        field.command(Command::SelectAll, "");
        self.overflow = false;
        self.pop = Some(Pop { url: url.to_string(), field, anchor });
    }

    fn close_pop(&mut self, store: &mut Store) {
        if let Some(p) = self.pop.take() {
            let name = p.field.to_string();
            if !name.trim().is_empty() {
                store.rename_bookmark(&p.url, name.trim());
            }
        }
    }

    fn rows(&self, store: &Store, now: u64) -> Vec<Row> {
        rows(store, self.mode, &self.query.to_string(), now)
    }

    fn remove_selected(&mut self, store: &mut Store, now: u64) {
        let rs = self.rows(store, now);
        let Some(Row::Item { url, .. }) = self.sel.and_then(|i| rs.get(i)) else { return };
        match self.mode {
            Mode::Bookmarks => store.remove_bookmark(url),
            Mode::History => store.remove_history(url),
        };
        let rs = self.rows(store, now);
        // keep the selection near where it was
        let at = self.sel.unwrap_or(0).min(rs.len().saturating_sub(1));
        self.sel = (at..rs.len()).find(|&i| is_item(&rs[i])).or_else(|| (0..at.min(rs.len())).rev().find(|&i| is_item(&rs[i])));
    }

    fn open_selected(&mut self, store: &Store, now: u64) -> Act {
        let rs = self.rows(store, now);
        match self.sel.and_then(|i| rs.get(i)) {
            Some(Row::Item { url, .. }) => {
                self.open = false;
                Act::Open(url.clone())
            }
            _ => Act::None,
        }
    }

    fn follow(&mut self, cap: usize) {
        if let Some(i) = self.sel {
            if i < self.first {
                self.first = i;
            } else if cap > 0 && i >= self.first + cap {
                self.first = i + 1 - cap;
            }
        }
    }

    /// Keys while the panel or popover is up. Escape closes, Return opens / confirms, Delete removes the selected
    /// row, Backspace edits the search, arrows move. Returns `Pass` when only the overflow menu is up.
    pub fn key(&mut self, k: &KeyInput, store: &mut Store, now: u64, hpt: f32) -> Act {
        if let Some(p) = &mut self.pop {
            match k {
                KeyInput::Text(s) => p.field.push_str(s),
                KeyInput::Code(8) => p.field.pop(),
                KeyInput::Code(46) => p.field.delete(),
                KeyInput::Code(37) => p.field.move_caret(false),
                KeyInput::Code(39) => p.field.move_caret(true),
                KeyInput::Code(13) | KeyInput::Code(27) | KeyInput::Code(1) => self.close_pop(store),
                _ => {}
            }
            return Act::None;
        }
        if !self.open {
            if self.overflow && matches!(k, KeyInput::Code(27) | KeyInput::Code(1)) {
                self.overflow = false;
                return Act::None;
            }
            return Act::Pass;
        }
        let cap = (Self::geo(hpt, PANEL_W_PT).list.h / ROW_PT) as usize;
        match k {
            KeyInput::Text(s) => {
                self.query.push_str(s);
                self.sel = step(&self.rows(store, now), None, 1);
                self.first = 0;
            }
            KeyInput::Code(8) => {
                self.query.pop();
                self.sel = step(&self.rows(store, now), None, 1);
                self.first = 0;
            }
            KeyInput::Code(46) => self.remove_selected(store, now),
            KeyInput::Code(37) => self.query.move_caret(false),
            KeyInput::Code(39) => self.query.move_caret(true),
            KeyInput::Code(40) => {
                self.sel = step(&self.rows(store, now), self.sel, 1);
                self.follow(cap);
            }
            KeyInput::Code(38) => {
                self.sel = step(&self.rows(store, now), self.sel, -1);
                self.follow(cap);
            }
            KeyInput::Code(13) => return self.open_selected(store, now),
            KeyInput::Code(27) | KeyInput::Code(1) => self.open = false,
            _ => {}
        }
        Act::None
    }

    // ---- geometry (points) ----

    fn geo(hpt: f32, wpt: f32) -> Geo {
        let h = (hpt - UNIFIED_BAR_PT - 16.0).clamp(160.0, PANEL_MAX_H_PT);
        let panel = Rect { x: ((wpt - PANEL_W_PT) / 2.0).max(8.0), y: UNIFIED_BAR_PT, w: PANEL_W_PT.min(wpt - 16.0), h };
        let seg_w = 100.0;
        let seg_y = panel.y + 12.0;
        let seg = [
            Rect { x: panel.x + PAD_PT, y: seg_y, w: seg_w, h: 24.0 },
            Rect { x: panel.x + PAD_PT + seg_w, y: seg_y, w: seg_w, h: 24.0 },
        ];
        let done = Rect { x: panel.x + panel.w - PAD_PT - 56.0, y: seg_y, w: 56.0, h: 24.0 };
        let search = Rect { x: panel.x + PAD_PT, y: seg_y + 32.0, w: panel.w - 2.0 * PAD_PT, h: 28.0 };
        let list = Rect { x: panel.x, y: search.y + search.h + 8.0, w: panel.w, h: (panel.y + panel.h - 28.0 - (search.y + search.h + 8.0)).max(ROW_PT) };
        Geo { panel, seg, done, search, list, foot_y: panel.y + panel.h - 14.0 }
    }

    fn pop_rect(&self, wpt: f32) -> Option<Rect> {
        let p = self.pop.as_ref()?;
        Some(Rect { x: (p.anchor - POP_W_PT / 2.0).clamp(8.0, (wpt - POP_W_PT - 8.0).max(8.0)), y: UNIFIED_BAR_PT + ARROW_PT, w: POP_W_PT, h: POP_H_PT })
    }

    fn pop_buttons(r: Rect) -> (Rect, Rect, Rect) {
        let field = Rect { x: r.x + 62.0, y: r.y + 40.0, w: r.w - 62.0 - 16.0, h: 28.0 };
        let remove = Rect { x: r.x + 16.0, y: r.y + r.h - 34.0, w: 80.0, h: 24.0 };
        let done = Rect { x: r.x + r.w - 16.0 - 60.0, y: r.y + r.h - 34.0, w: 60.0, h: 24.0 };
        (field, remove, done)
    }

    /// Favorites bar buttons `(bookmark index, rect)` and, when some do not fit, `(first hidden index, chevron)`.
    fn fav_layout(&self, ui: &Ui, wpt: f32, store: &Store, top: f32) -> (Vec<(usize, Rect)>, Option<(usize, Rect)>) {
        let px = ui.pt(1.0);
        let widths: Vec<f32> = store
            .bookmarks
            .iter()
            .map(|e| ui.measure(&ui.fit(&label(e), MAC_SUBHEAD_PT, ui.pt(FAV_BTN_MAX_PT - 2.0 * FAV_PAD_PT)), MAC_SUBHEAD_PT, Weight::Regular) / px + 2.0 * FAV_PAD_PT)
            .collect();
        let right = wpt - 12.0;
        let total: f32 = widths.iter().map(|w| w + FAV_GAP_PT).sum();
        let fits = 12.0 + total <= right;
        let limit = if fits { right } else { right - CHEVRON_PT - 4.0 };
        let (mut x, mut out) = (12.0, Vec::new());
        let mut hidden = None;
        for (i, w) in widths.iter().enumerate() {
            if x + w > limit {
                hidden = Some(i);
                break;
            }
            out.push((i, Rect { x, y: top + 3.0, w: *w, h: FAV_BAR_PT - 6.0 }));
            x += w + FAV_GAP_PT;
        }
        let chev = hidden.map(|i| (i, Rect { x: right - CHEVRON_PT, y: top + 3.0, w: CHEVRON_PT, h: FAV_BAR_PT - 6.0 }));
        (out, chev)
    }

    fn menu_rect(&self, wpt: f32, top: f32, hidden: usize, total: usize) -> Rect {
        let n = (total - hidden).min(MENU_MAX_ROWS) as f32;
        Rect { x: wpt - 12.0 - MENU_W_PT, y: top + FAV_BAR_PT, w: MENU_W_PT, h: n * MENU_ROW_PT + 8.0 }
    }

    // ---- touch ----

    /// True when a touch at window pixels belongs to the library (panel, popover, menu, or the Favorites bar band).
    pub fn wants_touch(&self, ui: &Ui, y: f32, top: f32) -> bool {
        if self.overlay() {
            return true;
        }
        let py = y / ui.pt(1.0);
        self.favbar && py >= top && py < top + FAV_BAR_PT
    }

    /// A finger lifting at window pixels `(x, y)`; `top` is where the Favorites bar starts (points).
    pub fn touch_up(&mut self, ui: &Ui, w: u32, h: u32, x: f32, y: f32, store: &mut Store, top: f32, t: f64, now: u64) -> Act {
        let (px, py) = (x / ui.pt(1.0), y / ui.pt(1.0));
        let (wpt, hpt) = (w as f32 / ui.pt(1.0), h as f32 / ui.pt(1.0));
        if let Some(r) = self.pop_rect(wpt) {
            let (_, remove, done) = Self::pop_buttons(r);
            if remove.contains(px, py) {
                if let Some(p) = self.pop.take() {
                    store.remove_bookmark(&p.url);
                }
            } else if done.contains(px, py) || !r.contains(px, py) {
                self.close_pop(store);
            }
            return Act::None;
        }
        if self.open {
            let g = Self::geo(hpt, wpt);
            if !g.panel.contains(px, py) || g.done.contains(px, py) {
                self.open = false;
                return Act::None;
            }
            for (i, mode) in [Mode::Bookmarks, Mode::History].into_iter().enumerate() {
                if g.seg[i].contains(px, py) && self.mode != mode {
                    self.mode = mode;
                    self.sel = None;
                    self.first = 0;
                }
            }
            if g.list.contains(px, py) {
                let rs = self.rows(store, now);
                let i = self.first + ((py - g.list.y) / ROW_PT) as usize;
                if rs.get(i).is_some_and(is_item) {
                    self.sel = Some(i);
                    let dbl = self.last_tap.is_some_and(|(j, t0)| j == i && t - t0 < DOUBLE_TAP_S);
                    self.last_tap = Some((i, t));
                    if dbl {
                        return self.open_selected(store, now);
                    }
                }
            }
            return Act::None;
        }
        let (buttons, chev) = self.fav_layout(ui, wpt, store, top);
        if self.overflow {
            if let Some((hidden, _)) = chev {
                let r = self.menu_rect(wpt, top, hidden, store.bookmarks.len());
                if r.contains(px, py) {
                    let i = hidden + ((py - r.y - 4.0).max(0.0) / MENU_ROW_PT) as usize;
                    self.overflow = false;
                    if let Some(b) = store.bookmarks.get(i).filter(|_| i < hidden + MENU_MAX_ROWS) {
                        return Act::Open(b.url.clone());
                    }
                    return Act::None;
                }
            }
            self.overflow = false;
        }
        if let Some((_, r)) = chev {
            if r.contains(px, py) {
                self.overflow = !self.overflow;
                return Act::None;
            }
        }
        match buttons.iter().find(|(_, r)| r.contains(px, py)) {
            Some((i, _)) => Act::Open(store.bookmarks[*i].url.clone()),
            None => Act::None,
        }
    }

    // ---- drawing ----

    fn txt(s: &mut Surface, ui: &Ui, t: &str, pt: f32, wt: Weight, color: u32, x: f32, cy: f32) {
        let img = ui.text(t, pt, wt, color);
        s.blit(&img, ui.pt(x) as i32, ui.pt(cy) as i32 - img.h as i32 / 2, 255, 255);
    }

    fn card(s: &mut Surface, ui: &Ui, r: Rect) {
        let rad = ui.pt(m::CARD_RADIUS);
        for (grow, off) in [(10.0, 4.0), (5.0, 2.0)] {
            s.fill_rrect(ui.pt(r.x - grow), ui.pt(r.y - grow + off), ui.pt(r.w + 2.0 * grow), ui.pt(r.h + 2.0 * grow), rad + ui.pt(grow), CAPSULE_SHADOW.now());
        }
        s.fill_rrect(ui.pt(r.x) - 1.0, ui.pt(r.y) - 1.0, ui.pt(r.w) + 2.0, ui.pt(r.h) + 2.0, rad, SEPARATOR_C.now());
        s.fill_rrect(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h), rad, CARD_BG.now());
    }

    /// The Favorites bar at `top` points.
    pub fn draw_bar(&self, s: &mut Surface, ui: &Ui, wpt: f32, top: f32, store: &Store) {
        if !self.favbar {
            return;
        }
        let (y, h) = (ui.pt(top) as usize, ui.pt(FAV_BAR_PT).ceil() as usize);
        s.fill_rect(0, y, s.w, h, CARD_BG.now());
        s.fill_rect(0, y + h - 1, s.w, 1, SEPARATOR_C.now());
        if store.bookmarks.is_empty() {
            Self::txt(s, ui, "Bookmarks you add with Cmd+D appear here.", MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), 12.0, top + FAV_BAR_PT / 2.0);
            return;
        }
        let (buttons, chev) = self.fav_layout(ui, wpt, store, top);
        for (i, r) in buttons {
            let t = ui.fit(&label(&store.bookmarks[i]), MAC_SUBHEAD_PT, ui.pt(FAV_BTN_MAX_PT - 2.0 * FAV_PAD_PT));
            Self::txt(s, ui, &t, MAC_SUBHEAD_PT, Weight::Regular, LABEL.now(), r.x + FAV_PAD_PT, r.y + r.h / 2.0);
        }
        if let Some((_, r)) = chev {
            if self.overflow {
                s.fill_rrect(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h), ui.pt(m::SELECTION_RADIUS), ROW_PRESSED.now());
            }
            glyphs::glyph(s, ui, "chevron.right", r, LABEL_2.now());
        }
    }

    /// The overflow menu, the Bookmarks / History panel and the naming popover.
    pub fn draw_overlays(&self, s: &mut Surface, ui: &Ui, w: u32, h: u32, top: f32, store: &Store, now: u64) {
        let (wpt, hpt) = (w as f32 / ui.pt(1.0), h as f32 / ui.pt(1.0));
        if self.overflow && self.favbar {
            if let (_, Some((hidden, _))) = self.fav_layout(ui, wpt, store, top) {
                let r = self.menu_rect(wpt, top, hidden, store.bookmarks.len());
                Self::card(s, ui, r);
                for (k, e) in store.bookmarks.iter().skip(hidden).take(MENU_MAX_ROWS).enumerate() {
                    let t = ui.fit(&label(e), MAC_SUBHEAD_PT, ui.pt(MENU_W_PT - 24.0));
                    Self::txt(s, ui, &t, MAC_SUBHEAD_PT, Weight::Regular, LABEL.now(), r.x + 12.0, r.y + 4.0 + k as f32 * MENU_ROW_PT + MENU_ROW_PT / 2.0);
                }
            }
        }
        if self.open {
            self.draw_panel(s, ui, wpt, hpt, store, now);
        }
        if let (Some(p), Some(r)) = (&self.pop, self.pop_rect(wpt)) {
            Self::card(s, ui, r);
            // arrow pointing up at the address field
            let ax = p.anchor.clamp(r.x + 20.0, r.x + r.w - 20.0);
            for k in 0..ARROW_PT as usize {
                let half = ARROW_PT - k as f32;
                let y = ui.pt(r.y) - ui.pt(ARROW_PT) + ui.pt(k as f32 + 1.0) - 1.0;
                s.fill_rect(ui.pt(ax - half) as usize, y.max(0.0) as usize, ui.pt(half * 2.0) as usize, ui.pt(1.0).ceil() as usize, CARD_BG.now());
            }
            let (field, remove, done) = Self::pop_buttons(r);
            Self::txt(s, ui, "Added to Bookmarks", MAC_BODY_PT, Weight::Semibold, LABEL.now(), r.x + 16.0, r.y + 22.0);
            Self::txt(s, ui, "Name", MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), r.x + 16.0, field.y + field.h / 2.0);
            widgets::text_field(s, ui, &p.field, field, "Title", true);
            Self::txt(s, ui, "Remove", MAC_SUBHEAD_PT, Weight::Regular, DESTRUCTIVE, remove.x, remove.y + remove.h / 2.0);
            Self::txt(s, ui, "Done", MAC_SUBHEAD_PT, Weight::Semibold, TINT, done.x + 14.0, done.y + done.h / 2.0);
        }
    }

    fn draw_panel(&self, s: &mut Surface, ui: &Ui, wpt: f32, hpt: f32, store: &Store, now: u64) {
        let g = Self::geo(hpt, wpt);
        Self::card(s, ui, g.panel);
        let track = Rect { x: g.seg[0].x, y: g.seg[0].y, w: g.seg[1].x + g.seg[1].w - g.seg[0].x, h: g.seg[0].h };
        s.fill_rrect(ui.pt(track.x), ui.pt(track.y), ui.pt(track.w), ui.pt(track.h), ui.pt(m::SELECTION_RADIUS), FIELD_BG.now());
        for (i, (r, name)) in g.seg.iter().zip(["Bookmarks", "History"]).enumerate() {
            let on = (i == 0) == (self.mode == Mode::Bookmarks);
            if on {
                s.fill_rrect(ui.pt(r.x + 2.0), ui.pt(r.y + 2.0), ui.pt(r.w - 4.0), ui.pt(r.h - 4.0), ui.pt(m::SELECTION_RADIUS), SEGMENT_ON.now());
            }
            let img = ui.text(name, MAC_SUBHEAD_PT, if on { Weight::Semibold } else { Weight::Regular }, LABEL.now());
            s.blit(&img, ui.pt(r.x + r.w / 2.0) as i32 - img.w as i32 / 2, ui.pt(r.y + r.h / 2.0) as i32 - img.h as i32 / 2, 255, 255);
        }
        Self::txt(s, ui, "Done", MAC_BODY_PT, Weight::Semibold, TINT, g.done.x + 8.0, g.done.y + g.done.h / 2.0);
        widgets::text_field(s, ui, &self.query, g.search, "Search", true);

        let rs = self.rows(store, now);
        if rs.is_empty() {
            let msg = match (self.mode, self.query.to_string().is_empty()) {
                (_, false) => "No Results",
                (Mode::Bookmarks, true) => "No Bookmarks",
                (Mode::History, true) => "No History",
            };
            let img = ui.text(msg, MAC_BODY_PT, Weight::Regular, LABEL_2.now());
            s.blit(&img, ui.pt(g.list.x + g.list.w / 2.0) as i32 - img.w as i32 / 2, ui.pt(g.list.y + 40.0) as i32, 255, 255);
        }
        let cap = (g.list.h / ROW_PT) as usize;
        for (k, row) in rs.iter().skip(self.first).take(cap).enumerate() {
            let i = self.first + k;
            let (y, cy) = (g.list.y + k as f32 * ROW_PT, g.list.y + k as f32 * ROW_PT + ROW_PT / 2.0);
            match row {
                Row::Header(t) => Self::txt(s, ui, t, MAC_FOOTNOTE_PT, Weight::Semibold, LABEL_2.now(), g.list.x + PAD_PT, cy + 4.0),
                Row::Item { url, title } => {
                    let on = self.sel == Some(i);
                    if on {
                        s.fill_rrect(ui.pt(g.list.x + 6.0), ui.pt(y + 1.0), ui.pt(g.list.w - 12.0), ui.pt(ROW_PT - 2.0), ui.pt(m::SELECTION_RADIUS), TINT);
                    }
                    let (c1, c2) = if on { (WHITE, WHITE) } else { (LABEL.now(), LABEL_2.now()) };
                    let sub = host(url);
                    let sub_px = ui.measure(sub, MAC_FOOTNOTE_PT, Weight::Regular).min(ui.pt(HOST_MAX_PT));
                    let room = ui.pt(g.list.w - 2.0 * PAD_PT - 16.0) - sub_px;
                    let t = ui.fit(title, MAC_SUBHEAD_PT, room.max(0.0));
                    Self::txt(s, ui, &t, MAC_SUBHEAD_PT, Weight::Regular, c1, g.list.x + PAD_PT + 4.0, cy);
                    let st = ui.fit(sub, MAC_FOOTNOTE_PT, ui.pt(HOST_MAX_PT));
                    let img = ui.text(&st, MAC_FOOTNOTE_PT, Weight::Regular, c2);
                    s.blit(&img, ui.pt(g.list.x + g.list.w - PAD_PT - 4.0) as i32 - img.w as i32, ui.pt(cy) as i32 - img.h as i32 / 2, 255, 255);
                }
            }
        }
        let hint = "Return opens. Delete removes.";
        let img = ui.text(hint, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
        s.blit(&img, ui.pt(g.panel.x + g.panel.w / 2.0) as i32 - img.w as i32 / 2, ui.pt(g.foot_y) as i32 - img.h as i32 / 2, 255, 255);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(url: &str, title: &str, last: u64) -> Entry {
        Entry { url: url.into(), title: title.into(), visits: 1, last }
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_000), (2024, 10, 4));
        // 2026-10-11 is day 20_737
        assert_eq!(civil_from_days(20_737), (2026, 10, 11));
    }

    #[test]
    fn day_labels() {
        let now = 20_737 * 86_400 + 3600;
        assert_eq!(day_label(now - 10, now), "Today");
        assert_eq!(day_label(now - 86_400, now), "Yesterday");
        assert_eq!(day_label(20_735 * 86_400, now), "Friday, October 9, 2026");
    }

    #[test]
    fn history_groups_by_day_newest_first() {
        let now = 20_737 * 86_400 + 100;
        let mut st = Store::in_memory();
        st.history = vec![e("https://old.test/", "Old", now - 5 * 86_400), e("https://a.test/", "A", now - 50), e("https://b.test/", "B", now - 10)];
        let rs = rows(&st, Mode::History, "", now);
        assert_eq!(rs[0], Row::Header("Today".into()));
        assert_eq!(rs[1], Row::Item { url: "https://b.test/".into(), title: "B".into() });
        assert!(matches!(rs[3], Row::Header(_)));
        assert_eq!(rs.len(), 5);
        assert_eq!(rows(&st, Mode::History, "old", now).len(), 2);
    }

    #[test]
    fn search_matches_title_and_url_words() {
        let st = {
            let mut s = Store::in_memory();
            s.bookmarks = vec![e("https://rust-lang.org/", "Rust Programming", 1), e("https://example.com/", "Example", 1)];
            s
        };
        assert_eq!(rows(&st, Mode::Bookmarks, "rust prog", 0).len(), 1);
        assert_eq!(rows(&st, Mode::Bookmarks, "EXAMPLE.com", 0).len(), 1);
        assert_eq!(rows(&st, Mode::Bookmarks, "", 0).len(), 2);
    }

    #[test]
    fn selection_skips_headers() {
        let rs = vec![Row::Header("Today".into()), Row::Item { url: "a".into(), title: "a".into() }, Row::Header("Yesterday".into()), Row::Item { url: "b".into(), title: "b".into() }];
        assert_eq!(step(&rs, None, 1), Some(1));
        assert_eq!(step(&rs, Some(1), 1), Some(3));
        assert_eq!(step(&rs, Some(3), 1), Some(3));
        assert_eq!(step(&rs, Some(3), -1), Some(1));
        assert_eq!(host("https://a.test:8080/x?y"), "a.test:8080");
    }

    #[test]
    fn delete_key_removes_the_selected_bookmark() {
        let mut st = Store::in_memory();
        st.add_bookmark("https://a.test/", "A", 1);
        st.add_bookmark("https://b.test/", "B", 2);
        let mut lib = Library::default();
        lib.show_panel(Mode::Bookmarks);
        lib.key(&KeyInput::Code(40), &mut st, 10, 600.0);
        assert_eq!(lib.key(&KeyInput::Code(46), &mut st, 10, 600.0), Act::None);
        assert_eq!(st.bookmarks.len(), 1);
        assert_eq!(st.bookmarks[0].url, "https://b.test/");
        assert_eq!(lib.key(&KeyInput::Code(13), &mut st, 10, 600.0), Act::Open("https://b.test/".into()));
    }
}

#[cfg(test)]
mod export_tests {
    use super::*;

    #[test]
    fn export_names_are_bookmark_html_files() {
        assert!(is_export_name(Path::new("/d/Bookmarks.html")));
        assert!(is_export_name(Path::new("/d/firefox-bookmarks-2026.htm")));
        assert!(!is_export_name(Path::new("/d/notes.html")));
        assert!(!is_export_name(Path::new("/d/bookmarks.zip")));
    }

    #[test]
    fn import_html_adds_the_export_links() {
        let html = "<DL><p><DT><H3>Work</H3><DL><p><DT><A HREF=\"https://a.test/\">A</A></DL></DL>";
        let mut store = Store::in_memory();
        assert_eq!(import_html(&mut store, html, 9), 1);
        assert_eq!(store.bookmark("https://a.test/").map(|b| b.title.clone()), Some("A".to_string()));
        assert_eq!(import_html(&mut store, html, 10), 0, "a second import adds nothing");
    }
}
