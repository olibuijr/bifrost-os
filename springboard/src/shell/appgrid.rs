//! The Apps grid (F4, or the dock's Apps icon): every app on a paged grid with a search field, page dots,
//! swipe or scroll between pages, press-and-hold then drag to reorder, Esc to close.
//!
//! The order is kept in the preference `appgrid.order` (comma separated icon ids). Search filters by name and
//! turns reordering off while a query is typed. Motion uses `bifrost_render::motion` (`Kind::OverviewEnter` for
//! opening and closing, `Kind::PanelReveal` for a page settling); both are instant with Reduce Motion.

use super::control_center::desk_glass;
use super::overview::{KeyMods, KEY_BACKSPACE, KEY_DOWN, KEY_ENTER, KEY_ESC, KEY_LEFT, KEY_RIGHT, KEY_UP};
use super::ui::{Weight, FIELD_BG, LABEL, LABEL_2, LABEL_3, MAC_BODY_PT, MAC_SUBHEAD_PT, SIDEBAR_PLATE, WHITE};
use super::{apps, Effect, Event, Shell};
use crate::gfx::{rgba, scale, Image, Surface};
use bifrost_render::motion::{self, Curve, Kind};
use std::collections::HashMap;

const ORDER_PREF: &str = "appgrid.order";
const ICON_PT: f32 = 80.0;
const CELL_W_PT: f32 = 136.0;
const LABEL_H_PT: f32 = 34.0;
const SEARCH_W_PT: f32 = 300.0;
const SEARCH_H_PT: f32 = 30.0;
const DOT_PT: f32 = 7.0;
const DOT_GAP_PT: f32 = 14.0;
const SLOP_PT: f32 = 8.0;
/// A press held this long (ms) before it moves lifts the icon for reordering.
const HOLD_MS: f64 = 350.0;
/// A flick faster than this (px per ms) turns the page whatever the distance.
const FLICK: f32 = 0.5;
const EDGE_FRACTION: f32 = 0.06;
const EDGE_REPEAT_MS: f64 = 700.0;
const MAX_QUERY: usize = 64;

/// Columns and rows that fit `w` x `h` for cells of `cell_w` x `cell_h`.
pub(super) fn grid_dims(w: f32, h: f32, cell_w: f32, cell_h: f32) -> (usize, usize) {
    let cols = ((w * 0.84 / cell_w.max(1.0)).floor() as usize).clamp(3, 9);
    let rows = ((h / cell_h.max(1.0)).floor() as usize).clamp(1, 6);
    (cols, rows)
}

/// `apps` (icon id, name) in the saved order, unknown ids after the saved ones in catalogue order; a query keeps
/// the names that contain it, case-insensitively.
pub(super) fn arrange(all: &[(String, String)], order: &[String], query: &str) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..all.len()).collect();
    idx.sort_by_key(|&i| order.iter().position(|o| *o == all[i].0).unwrap_or(usize::MAX));
    let q = query.trim().to_lowercase();
    if !q.is_empty() {
        idx.retain(|&i| all[i].1.to_lowercase().contains(&q));
    }
    idx
}

/// Moves `id` so it sits where `before` is in `order` (the full, unfiltered list of ids).
pub(super) fn reorder(order: &mut Vec<String>, id: &str, before: &str) {
    let Some(from) = order.iter().position(|o| o == id) else { return };
    let item = order.remove(from);
    let at = order.iter().position(|o| o == before).unwrap_or(order.len());
    order.insert(at, item);
}

struct Lay {
    w: f32,
    h: f32,
    pt: f32,
    cols: usize,
    rows: usize,
    cell_w: f32,
    cell_h: f32,
    icon: usize,
    x0: f32,
    y0: f32,
    search: (f32, f32, f32, f32),
    dots_y: f32,
}

impl Lay {
    fn per_page(&self) -> usize {
        self.cols * self.rows
    }
    /// The cell of slot `k` on a page whose left edge is at `dx`.
    fn cell(&self, k: usize, dx: f32) -> (f32, f32) {
        (self.x0 + dx + (k % self.cols) as f32 * self.cell_w, self.y0 + (k / self.cols) as f32 * self.cell_h)
    }
    fn slot_at(&self, x: f32, y: f32) -> Option<usize> {
        let (gx, gy) = (x - self.x0, y - self.y0);
        if gx < 0.0 || gy < 0.0 {
            return None;
        }
        let (c, r) = ((gx / self.cell_w) as usize, (gy / self.cell_h) as usize);
        (c < self.cols && r < self.rows).then_some(r * self.cols + c)
    }
}

#[derive(Clone, Copy)]
struct Press {
    /// the index into the arranged list
    item: Option<usize>,
    x0: f32,
    y0: f32,
    t0: f64,
    last_x: f32,
    last_t: f64,
    vel: f32,
    swiping: bool,
    dot: Option<usize>,
}

#[derive(Default)]
pub(super) struct AppGrid {
    open: bool,
    start: f64,
    closing: Option<f64>,
    query: String,
    page: usize,
    order: Vec<String>,
    loaded: bool,
    press: Option<Press>,
    /// the pages are dragged this far (px) from rest
    swipe_dx: f32,
    /// a settling page: the offset it started from and when
    settle: Option<(f32, f64)>,
    /// the lifted icon (an index into the arranged list) and where the pointer is
    lifted: Option<(usize, f32, f32)>,
    edge_at: f64,
    selected: Option<usize>,
    scroll_acc: f32,
    icons: HashMap<(usize, usize), Image>,
}

fn catalogue() -> Vec<(String, String)> {
    apps().iter().map(|a| (a.icon.to_string(), a.name.to_string())).collect()
}

fn prog(kind: Kind, curve: Curve, start: f64, t: f64) -> f32 {
    let d = motion::duration(kind);
    if d <= 0.0 {
        return 1.0;
    }
    motion::ease(curve, (((t - start) / 1000.0) as f32 / d).clamp(0.0, 1.0))
}

impl AppGrid {
    pub(super) fn visible(&self) -> bool {
        self.open
    }

    fn lay(&self, sh: &Shell) -> Lay {
        let m = sh.desk_metrics();
        let pt = m.pt;
        let (w, h) = (m.w as f32, m.h as f32);
        let icon = (ICON_PT * pt) as usize;
        let (cell_w, cell_h) = (CELL_W_PT * pt, icon as f32 + LABEL_H_PT * pt + 22.0 * pt);
        let search_w = (SEARCH_W_PT * pt).min(w * 0.8);
        let search_h = SEARCH_H_PT * pt;
        let search = ((w - search_w) / 2.0, m.menubar as f32 + 24.0 * pt, search_w, search_h);
        let top = search.1 + search_h + 36.0 * pt;
        let bottom = h - if sh.dock_is_visible() { m.dock_bottom as f32 } else { 16.0 * pt } - 52.0 * pt;
        let (cols, rows) = grid_dims(w, (bottom - top).max(cell_h), cell_w, cell_h);
        let gw = cols as f32 * cell_w;
        let gh = rows as f32 * cell_h;
        let x0 = (w - gw) / 2.0;
        let y0 = top + ((bottom - top - gh) / 2.0).max(0.0);
        Lay { w, h, pt, cols, rows, cell_w, cell_h, icon, x0, y0, search, dots_y: y0 + gh + 22.0 * pt }
    }

    fn arranged(&self) -> Vec<usize> {
        arrange(&catalogue(), &self.order, &self.query)
    }

    fn pages(&self, lay: &Lay, n: usize) -> usize {
        n.div_ceil(lay.per_page().max(1)).max(1)
    }

    pub(super) fn open(&mut self, sh: &mut Shell, t: f64) {
        if !self.loaded {
            self.order = sh.prefs.get(ORDER_PREF).map(|s| s.split(',').filter(|x| !x.is_empty()).map(str::to_string).collect()).unwrap_or_default();
            self.loaded = true;
        }
        self.open = true;
        self.start = t;
        self.closing = None;
        self.query.clear();
        self.page = 0;
        self.swipe_dx = 0.0;
        self.settle = None;
        self.press = None;
        self.lifted = None;
        self.selected = None;
        self.scroll_acc = 0.0;
        sh.cc.hide();
        sh.dirty = true;
    }

    pub(super) fn close(&mut self, sh: &mut Shell, t: f64) {
        if !self.open || self.closing.is_some() {
            return;
        }
        self.press = None;
        self.lifted = None;
        if motion::duration(Kind::OverviewEnter) <= 0.0 {
            self.open = false;
            self.icons.clear();
        } else {
            self.closing = Some(t);
        }
        sh.dirty = true;
    }

    fn launch(&mut self, sh: &mut Shell, app_index: usize, t: f64) {
        let Some(app) = apps().get(app_index).map(|a| a.id) else { return };
        self.close(sh, t);
        sh.desk_open(app, t);
    }

    fn set_page(&mut self, page: usize, n_pages: usize, from_offset: f32, t: f64) {
        self.page = page.min(n_pages - 1);
        self.swipe_dx = 0.0;
        self.settle = Some((from_offset, t));
    }

    /// Where the pages sit right now: the finger's offset, or a settling page's.
    fn offset(&self, t: f64) -> f32 {
        match self.settle {
            Some((from, st)) => from * (1.0 - prog(Kind::PanelReveal, Curve::EaseOut, st, t)),
            None => self.swipe_dx,
        }
    }

    pub(super) fn type_text(&mut self, text: &str) {
        for c in text.chars().filter(|c| !c.is_control()) {
            if self.query.chars().count() < MAX_QUERY {
                self.query.push(c);
            }
        }
        self.page = 0;
        self.selected = None;
    }

    pub(super) fn scroll(&mut self, sh: &mut Shell, dx: f32, dy: f32, t: f64) {
        if self.closing.is_some() {
            return;
        }
        let lay = self.lay(sh);
        let n = self.pages(&lay, self.arranged().len());
        // a wheel's vertical notches page too; one page per ~60 px
        let d = if dx.abs() >= dy.abs() { dx } else { dy };
        self.scroll_acc += d;
        let step = 60.0 * lay.pt;
        while self.scroll_acc.abs() >= step {
            let fwd = self.scroll_acc > 0.0;
            self.scroll_acc -= step * if fwd { 1.0 } else { -1.0 };
            let next = if fwd { (self.page + 1).min(n - 1) } else { self.page.saturating_sub(1) };
            if next != self.page {
                let from = self.offset(t) + if fwd { lay.w } else { -lay.w };
                self.set_page(next, n, from, t);
            }
        }
    }

    pub(super) fn key(&mut self, sh: &mut Shell, code: u16, _mods: KeyMods, t: f64) -> bool {
        if self.closing.is_some() {
            return true;
        }
        let lay = self.lay(sh);
        let list = self.arranged();
        let n = list.len();
        match code {
            KEY_ESC => self.close(sh, t),
            KEY_BACKSPACE => {
                self.query.pop();
                self.page = 0;
                self.selected = None;
            }
            KEY_ENTER => {
                if let Some(&a) = self.selected.and_then(|i| list.get(i)).or(list.first()) {
                    self.launch(sh, a, t);
                }
            }
            KEY_LEFT | KEY_RIGHT | KEY_UP | KEY_DOWN if n > 0 => {
                let cur = self.selected.unwrap_or(self.page * lay.per_page());
                let next = match code {
                    KEY_LEFT => cur.saturating_sub(1),
                    KEY_RIGHT => (cur + 1).min(n - 1),
                    KEY_UP => cur.saturating_sub(lay.cols),
                    _ => (cur + lay.cols).min(n - 1),
                };
                let first = self.selected.is_none();
                self.selected = Some(if first { cur } else { next });
                let page = self.selected.unwrap_or(0) / lay.per_page().max(1);
                if page != self.page {
                    let dir = if page > self.page { lay.w } else { -lay.w };
                    let from = self.offset(t) + dir;
                    let pages = self.pages(&lay, n);
                    self.set_page(page, pages, from, t);
                }
            }
            _ => return false,
        }
        sh.dirty = true;
        true
    }

    pub(super) fn event(&mut self, sh: &mut Shell, ev: Event, t: f64) -> Option<Vec<Effect>> {
        if self.closing.is_some() {
            return Some(vec![]);
        }
        let lay = self.lay(sh);
        let list = self.arranged();
        let per = lay.per_page().max(1);
        let pages = self.pages(&lay, list.len());
        let slop = SLOP_PT * lay.pt;
        match ev {
            Event::Down(x, y) => {
                let item = lay.slot_at(x, y).map(|k| self.page * per + k).filter(|&i| i < list.len());
                let dot = self.dot_at(&lay, pages, x, y);
                self.press = Some(Press { item, x0: x, y0: y, t0: t, last_x: x, last_t: t, vel: 0.0, swiping: false, dot });
                self.settle = None;
            }
            Event::Move(x, y) => {
                let Some(mut p) = self.press else { return Some(vec![]) };
                if self.lifted.is_some() {
                    self.lifted = self.lifted.map(|(i, _, _)| (i, x, y));
                    // dwelling at a side edge turns the page, one per repeat interval
                    let edge = lay.w * EDGE_FRACTION;
                    if t - self.edge_at > EDGE_REPEAT_MS {
                        let next = if x < edge && self.page > 0 { Some(self.page - 1) } else if x > lay.w - edge && self.page + 1 < pages { Some(self.page + 1) } else { None };
                        if let Some(n) = next {
                            self.edge_at = t;
                            let from = if n > self.page { lay.w } else { -lay.w };
                            self.set_page(n, pages, from, t);
                        }
                    }
                } else if p.swiping {
                    let dt = (t - p.last_t).max(1.0) as f32;
                    p.vel = (x - p.last_x) / dt;
                    p.last_x = x;
                    p.last_t = t;
                    self.swipe_dx = x - p.x0;
                    // the first and last page resist
                    if (self.page == 0 && self.swipe_dx > 0.0) || (self.page + 1 >= pages && self.swipe_dx < 0.0) {
                        self.swipe_dx /= 3.0;
                    }
                    self.press = Some(p);
                } else if let (Some(item), true) = (p.item, t - p.t0 >= HOLD_MS && self.query.is_empty()) {
                    // held still, then moved: lift the icon
                    self.lifted = Some((item, x, y));
                    self.edge_at = t;
                } else if (x - p.x0).abs() > slop && (x - p.x0).abs() > (y - p.y0).abs() {
                    p.swiping = true;
                    p.last_x = x;
                    p.last_t = t;
                    self.press = Some(p);
                }
            }
            Event::Up(x, y) => {
                let Some(p) = self.press.take() else { return Some(vec![]) };
                if let Some((item, _, _)) = self.lifted.take() {
                    if let Some(k) = lay.slot_at(x, y) {
                        let target = self.page * per + k;
                        if let (Some(&from), Some(&to)) = (list.get(item), list.get(target.min(list.len().saturating_sub(1)))) {
                            self.save_order(sh, from, to);
                        }
                    }
                } else if p.swiping {
                    let off = self.swipe_dx;
                    let turn = off.abs() > lay.w * 0.15 || p.vel.abs() > FLICK;
                    let toward_next = off < 0.0 || p.vel < -FLICK;
                    let next = if !turn { self.page } else if toward_next { (self.page + 1).min(pages - 1) } else { self.page.saturating_sub(1) };
                    let from = if next > self.page { off + lay.w } else if next < self.page { off - lay.w } else { off };
                    self.set_page(next, pages, from, t);
                } else if (x - p.x0).abs() <= slop && (y - p.y0).abs() <= slop {
                    if let (Some(item), Some(k)) = (p.item, lay.slot_at(x, y)) {
                        if self.page * per + k == item {
                            if let Some(&a) = list.get(item) {
                                self.launch(sh, a, t);
                            }
                        }
                    } else if let Some(d) = p.dot.filter(|_| self.dot_at(&lay, pages, x, y) == p.dot) {
                        let from = (d as f32 - self.page as f32) * lay.w;
                        self.set_page(d, pages, from, t);
                    } else if !self.in_search(&lay, x, y) {
                        self.close(sh, t);
                    }
                }
            }
            _ => return None,
        }
        sh.dirty = true;
        Some(vec![])
    }

    fn in_search(&self, lay: &Lay, x: f32, y: f32) -> bool {
        let (sx, sy, sw, sh) = lay.search;
        x >= sx && x < sx + sw && y >= sy && y < sy + sh
    }

    fn dot_at(&self, lay: &Lay, pages: usize, x: f32, y: f32) -> Option<usize> {
        if pages < 2 || (y - lay.dots_y).abs() > 14.0 * lay.pt {
            return None;
        }
        let step = (DOT_PT + DOT_GAP_PT) * lay.pt;
        let x0 = (lay.w - step * pages as f32 + DOT_GAP_PT * lay.pt) / 2.0;
        let i = ((x - x0) / step).floor();
        (i >= 0.0 && (i as usize) < pages).then_some(i as usize)
    }

    /// Saves the order after the app at catalogue index `from` was dropped where `to` is.
    fn save_order(&mut self, sh: &mut Shell, from: usize, to: usize) {
        if from == to {
            return;
        }
        let all = catalogue();
        let mut full: Vec<String> = arrange(&all, &self.order, "").into_iter().map(|i| all[i].0.clone()).collect();
        reorder(&mut full, &all[from].0, &all[to].0);
        if sh.prefs.set(ORDER_PREF, &full.join(",")).is_err() {
            eprintln!("BifrOSt: could not save the Apps grid order");
        }
        self.order = full;
    }

    /// Makes sure the icon of `app_index` at `size` px is in the cache.
    fn ensure_icon(&mut self, sh: &Shell, app_index: usize, size: usize) -> bool {
        if self.icons.contains_key(&(app_index, size)) {
            return true;
        }
        let Some(base) = sh.base_icons.get(app_index) else { return false };
        if self.icons.len() > 256 {
            self.icons.clear();
        }
        self.icons.insert((app_index, size), if base.w == size { base.clone() } else { base.resized(size, size) });
        true
    }

    /// Draws the grid; true while it animates.
    pub(super) fn draw(&mut self, sh: &Shell, s: &mut Surface, t: f64) -> bool {
        if !self.open {
            return false;
        }
        let lay = self.lay(sh);
        let opening = prog(Kind::OverviewEnter, Curve::EaseOut, self.start, t);
        let p = match self.closing {
            Some(c) => {
                let k = prog(Kind::OverviewEnter, Curve::EaseInOut, c, t);
                if k >= 1.0 {
                    self.open = false;
                    self.closing = None;
                    self.icons.clear();
                    return false;
                }
                1.0 - k
            }
            None => opening,
        };
        let a = (255.0 * p) as u32;
        let ui = &sh.ui;
        let m = sh.desk_metrics();
        let veil_h = lay.h - m.menubar as f32 - if sh.dock_is_visible() { m.dock_bottom as f32 } else { 0.0 };
        desk_glass(s, (0.0, m.menubar as f32, lay.w, veil_h.max(1.0)), 0.0, (235.0 * p) as u32);
        s.fill_rrect(0.0, m.menubar as f32, lay.w, veil_h.max(1.0), 0.0, rgba(0, 0, 0, (50.0 * p) as u8));
        // search field: a capsule
        let (sx, sy, sw, shh) = lay.search;
        s.fill_rrect(sx, sy, sw, shh, shh / 2.0, scale(FIELD_BG.now(), a));
        let text_x = sx + shh * 0.9;
        if self.query.is_empty() {
            sh.label("Search", MAC_BODY_PT, Weight::Regular, WHITE, |im| s.blit_colored(im, text_x as i32, (sy + (shh - im.h as f32) / 2.0) as i32, LABEL_2.now(), a));
        } else {
            let q = self.query.clone();
            let w = sh.label(&q, MAC_BODY_PT, Weight::Regular, WHITE, |im| {
                s.blit_colored(im, text_x as i32, (sy + (shh - im.h as f32) / 2.0) as i32, LABEL.now(), a);
                im.w as f32
            });
            s.fill_rect((text_x + w + ui.pt(1.0)) as usize, (sy + shh * 0.22) as usize, ui.pt(1.0).max(1.0) as usize, (shh * 0.56) as usize, scale(LABEL.now(), a));
        }
        // pages: the current one and its neighbours while they slide
        let list = self.arranged();
        let per = lay.per_page().max(1);
        let pages = self.pages(&lay, list.len());
        self.page = self.page.min(pages - 1);
        let off = self.offset(t);
        let rise = (1.0 - p) * 14.0 * lay.pt;
        let lifted = self.lifted.map(|l| l.0);
        let hover_slot = self.lifted.and_then(|(_, x, y)| lay.slot_at(x, y));
        for pg in self.page.saturating_sub(1)..(self.page + 2).min(pages) {
            let dx = (pg as f32 - self.page as f32) * lay.w + off;
            if dx > lay.w || dx < -lay.w {
                continue;
            }
            for k in 0..per {
                let i = pg * per + k;
                let Some(&app) = list.get(i) else { break };
                if lifted == Some(i) {
                    continue;
                }
                let (cx, cy) = lay.cell(k, dx);
                let (cx, cy) = (cx, cy + rise);
                if self.selected == Some(i) || (pg == self.page && hover_slot == Some(k) && lifted.is_some()) {
                    s.fill_rrect(cx + lay.pt * 6.0, cy - lay.pt * 4.0, lay.cell_w - lay.pt * 12.0, lay.cell_h - lay.pt * 6.0, ui.pt(16.0), scale(SIDEBAR_PLATE.now(), a));
                }
                self.draw_item(sh, s, app, cx, cy, &lay, a, 1.0);
            }
        }
        if let Some((item, x, y)) = self.lifted {
            if let Some(&app) = list.get(item) {
                let k = 1.12;
                self.draw_item(sh, s, app, x - lay.cell_w * k / 2.0, y - lay.icon as f32 * k / 2.0, &lay, 230, k);
            }
        }
        // page dots
        if pages > 1 {
            let step = (DOT_PT + DOT_GAP_PT) * lay.pt;
            let x0 = (lay.w - step * pages as f32 + DOT_GAP_PT * lay.pt) / 2.0;
            let d = DOT_PT * lay.pt;
            for pg in 0..pages {
                let c = if pg == self.page { LABEL.now() } else { LABEL_3.now() };
                s.fill_rrect(x0 + pg as f32 * step, lay.dots_y, d, d, d / 2.0, scale(c, a));
            }
        }
        if list.is_empty() {
            let msg = format!("No results for \u{201c}{}\u{201d}", self.query);
            sh.label(&msg, MAC_BODY_PT, Weight::Regular, WHITE, |im| s.blit_colored(im, ((lay.w - im.w as f32) / 2.0) as i32, (lay.y0 + lay.cell_h) as i32, LABEL_2.now(), a));
        }
        p < 1.0 || self.closing.is_some() || self.settle.is_some_and(|(_, st)| prog(Kind::PanelReveal, Curve::EaseOut, st, t) < 1.0) || self.lifted.is_some()
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_item(&mut self, sh: &Shell, s: &mut Surface, app: usize, cx: f32, cy: f32, lay: &Lay, alpha: u32, k: f32) {
        let size = (lay.icon as f32 * k) as usize;
        let ix = cx + (lay.cell_w - size as f32) / 2.0;
        if self.ensure_icon(sh, app, size) {
            if let Some(img) = self.icons.get(&(app, size)) {
                s.blit(img, ix as i32, cy as i32, 255, alpha);
            }
        }
        let name = apps().get(app).map_or("", |a| a.name);
        let max_w = lay.cell_w - 8.0 * lay.pt;
        // shorten with an ellipsis until it fits the cell
        let mut chars: Vec<char> = name.chars().collect();
        let mut text = name.to_string();
        while chars.len() > 1 && sh.label(&text, MAC_SUBHEAD_PT, Weight::Regular, WHITE, |im| im.w as f32) > max_w {
            chars.pop();
            text = format!("{}\u{2026}", chars.iter().collect::<String>().trim_end());
        }
        let ly = cy + size as f32 + 8.0 * lay.pt;
        sh.label(&text, MAC_SUBHEAD_PT, Weight::Regular, WHITE, |im| s.blit_colored(im, (cx + (lay.cell_w - im.w as f32) / 2.0) as i32, ly as i32, LABEL.now(), alpha));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<(String, String)> {
        ["Settings", "Term", "Photos", "Files", "App Store"].iter().map(|n| (n.to_lowercase().replace(' ', "."), n.to_string())).collect()
    }

    #[test]
    fn grid_fits_the_screen_whatever_its_size() {
        let (c, r) = grid_dims(1920.0, 600.0, 200.0, 180.0);
        assert!(c >= 3 && c <= 9 && r >= 1 && r <= 6);
        assert_eq!(grid_dims(100.0, 10.0, 200.0, 180.0), (3, 1), "tiny screens keep a usable grid");
    }

    #[test]
    fn saved_order_comes_first_and_unknown_apps_follow() {
        let a = all();
        let order = vec!["files".to_string(), "settings".to_string()];
        let idx = arrange(&a, &order, "");
        assert_eq!(&idx[..2], &[3, 0]);
        assert_eq!(idx.len(), 5);
    }

    #[test]
    fn search_filters_by_name_ignoring_case() {
        let a = all();
        assert_eq!(arrange(&a, &[], "STO"), vec![4]);
        assert!(arrange(&a, &[], "zzz").is_empty());
    }

    #[test]
    fn reorder_puts_the_app_where_the_target_was() {
        let mut o: Vec<String> = ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect();
        reorder(&mut o, "d", "b");
        assert_eq!(o, ["a", "d", "b", "c"]);
        reorder(&mut o, "a", "c");
        assert_eq!(o, ["d", "b", "a", "c"]);
    }
}
