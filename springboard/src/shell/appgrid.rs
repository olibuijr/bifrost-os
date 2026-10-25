//! The Apps grid (F4, or the dock's Apps icon): a capsule header that is also the search field, and under it a
//! floating glass panel with category chips and every app on a paged 7 column grid, page dots, swipe or scroll
//! between pages, Esc or a click outside to close. The wallpaper and the windows stay visible around the panel (the
//! exported surface is masked to the capsule and the panel).
//!
//! Reordering: with a pointer (the desktop), dragging an icon picks it up at once, as the reference's Launchpad does;
//! on a touch screen a press held for `HOLD_MS` picks it up (it lifts under the finger even before it moves), so a
//! plain swipe there still pages. A drag that starts on empty space pages everywhere. While an icon is held the
//! others make room at the slot under the pointer, holding it at the panel's left or right edge turns the page, and
//! the drop keeps the new order.
//!
//! Categories come from each installed app's manifest (`category`); the built-in apps and the few manifests whose
//! category does not say where they belong are mapped in one table below (`CHIPS`, `chip_for_manifest`).
//!
//! The order is kept in the preference `appgrid.order` (comma separated icon ids). Search filters by name and
//! turns reordering off while a query is typed. Motion uses `bifrost_render::motion` (`Kind::OverviewEnter` for
//! opening and closing, `Kind::PanelReveal` for a page settling); both are instant with Reduce Motion.

use super::quicksearch::{mac_glass, panel_glass};
use bifrost_ui::material_request::ShellSurface;
use super::overview::{KeyMods, KEY_BACKSPACE, KEY_DOWN, KEY_ENTER, KEY_ESC, KEY_LEFT, KEY_RIGHT, KEY_UP};
use super::ui::{Weight, FIELD_BG, LABEL, LABEL_2, LABEL_3, MAC_BODY_PT, MAC_CALLOUT_PT, MAC_LARGE_TITLE_PT, SIDEBAR_PLATE, WHITE};
use super::{apphost, apps, AppDef, AppId, Effect, Event, Shell};
use crate::gfx::{scale, Image, Surface};
use bifrost_render::motion::{self, Curve, Kind};
use std::collections::HashMap;

const ORDER_PREF: &str = "appgrid.order";
/// Columns of the grid (the reference's Applications panel).
const COLS: usize = 7;
const MAX_ROWS: usize = 6;
/// The capsule header: 0.37 of the screen wide (at least `CAP_MIN_PT`), `CAP_H_PT` tall, its top at 0.158 of the screen's
/// height (never closer than `CAP_TOP_MIN_PT` to the menu bar).
const CAP_W_FRACTION: f32 = 0.37;
const CAP_MIN_PT: f32 = 320.0;
const CAP_H_PT: f32 = 84.0;
const CAP_TOP_FRACTION: f32 = 0.158;
const CAP_TOP_MIN_PT: f32 = 24.0;
/// The glass panel: 0.6 of the screen wide, 0.59 of its height, `PANEL_GAP_PT` under the capsule.
const PANEL_W_FRACTION: f32 = 0.6;
const PANEL_H_FRACTION: f32 = 0.59;
const PANEL_GAP_PT: f32 = 12.0;
const PANEL_R_PT: f32 = 28.0;
const PANEL_PAD_PT: f32 = 16.0;
/// Chip row: height, gap between chips, text padding, and the gap under the row.
const CHIP_H_PT: f32 = 28.0;
const CHIP_GAP_PT: f32 = 8.0;
const CHIP_PAD_PT: f32 = 14.0;
const CHIP_MAX_GAP_PT: f32 = 24.0;
const CHIPS_BELOW_PT: f32 = 18.0;
/// An icon is this fraction of its cell's width (at most `ICON_MAX_PT`); the name sits `LABEL_BLOCK_PT` under it.
const ICON_FRACTION: f32 = 0.54;
const ICON_MAX_PT: f32 = 80.0;
const LABEL_BLOCK_PT: f32 = 42.0;
/// Room kept under the grid for the page dots, and the bottom edge's distance from the Dock.
const DOTS_BLOCK_PT: f32 = 26.0;
const DOCK_CLEAR_PT: f32 = 20.0;
const MORE_DOT_PT: f32 = 4.0;
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

/// Rows of `cell_h` that fit `avail` px of height.
pub(super) fn grid_rows(avail: f32, cell_h: f32) -> usize {
    ((avail / cell_h.max(1.0)).floor() as usize).clamp(1, MAX_ROWS)
}

/// The category chips, in the order they are shown. An app belongs to exactly one.
const CHIPS: [&str; 7] = ["Utilities", "Productivity & Finance", "Creativity", "Information & Reading", "Social", "Entertainment", "Other"];
const OTHER_CHIP: usize = 6;

/// Manifest `category` values and the chip each one is under.
const MANIFEST_CHIP: &[(&str, usize)] = &[
    ("Utilities", 0),
    ("Productivity", 1),
    ("Finance", 1),
    ("Photo & Video", 2),
    ("Graphics & Design", 2),
    ("Creativity", 2),
    ("Books", 3),
    ("Reference", 3),
    ("Travel", 3),
    ("News", 3),
    ("Social", 4),
    ("Entertainment", 5),
    ("Music", 5),
    ("Games", 5),
];

/// Apps whose manifest category (a work category) is not where people look for them: by app id.
const ID_CHIP: &[(&str, usize)] = &[("is.olibuijr.messages.desktop", 4)];

/// The chip of an installed app from its manifest id and `category`.
pub(super) fn chip_for_manifest(id: &str, category: &str) -> usize {
    if let Some(&(_, c)) = ID_CHIP.iter().find(|(i, _)| *i == id) {
        return c;
    }
    MANIFEST_CHIP.iter().find(|(c, _)| c.eq_ignore_ascii_case(category.trim())).map_or(OTHER_CHIP, |&(_, c)| c)
}

/// The chip of a built-in app.
pub(super) fn builtin_chip(id: AppId) -> usize {
    match id {
        AppId::Settings | AppId::Terminal | AppId::Files | AppId::Magnifier | AppId::AppStore => 0,
        AppId::Photos | AppId::Camera => 2,
        AppId::Browser => 3,
        AppId::Deildu => 5,
        _ => OTHER_CHIP,
    }
}

fn chip_of(def: &AppDef) -> usize {
    match def.id {
        AppId::External(slot) => apphost::installed().get(slot as usize).map_or(OTHER_CHIP, |a| chip_for_manifest(&a.manifest.id, &a.manifest.category)),
        id => builtin_chip(id),
    }
}

/// The chips that have at least one app, in display order, given each app's chip.
pub(super) fn present_chips(app_chips: impl Iterator<Item = usize>) -> Vec<usize> {
    let mut seen = [false; CHIPS.len()];
    for c in app_chips {
        seen[c.min(OTHER_CHIP)] = true;
    }
    (0..CHIPS.len()).filter(|&c| seen[c]).collect()
}

/// Chip rectangles (x, y, w, h) from their text widths: natural width plus padding, spread over `inner` px with at most
/// `max_gap` between them and centred from `x`; when they do not fit, the padding shrinks (down to `min_pad`) first.
#[allow(clippy::too_many_arguments)]
pub(super) fn chip_rects(text_w: &[f32], x: f32, y: f32, inner: f32, h: f32, pad: f32, min_pad: f32, gap: f32, max_gap: f32) -> Vec<(f32, f32, f32, f32)> {
    let n = text_w.len();
    if n == 0 {
        return vec![];
    }
    let sum: f32 = text_w.iter().sum();
    let gaps = gap * (n - 1) as f32;
    let mut pad = pad;
    let need = sum + 2.0 * pad * n as f32 + gaps;
    if need > inner {
        pad = (pad - (need - inner) / (2.0 * n as f32)).max(min_pad);
    }
    let natural = sum + 2.0 * pad * n as f32;
    let gap = if n > 1 { ((inner - natural) / (n - 1) as f32).clamp(gap.min(max_gap), max_gap) } else { 0.0 };
    let total = natural + gap * (n - 1) as f32;
    let mut cx = x + ((inner - total) / 2.0).max(0.0);
    text_w
        .iter()
        .map(|w| {
            let r = (cx, y, w + 2.0 * pad, h);
            cx += w + 2.0 * pad + gap;
            r
        })
        .collect()
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

/// The shown list's positions in the order they are drawn while the icon at position `from` is held over position
/// `to`: the held one sits at `to` and the others close up or make room around it. `to` past the end is the last place.
pub(super) fn display_order(len: usize, from: usize, to: usize) -> Vec<usize> {
    let mut v: Vec<usize> = (0..len).collect();
    if from >= len {
        return v;
    }
    let item = v.remove(from);
    v.insert(to.min(len - 1), item);
    v
}

/// The full order after the icon at position `from` of `shown` (the ids on the grid, a subset of `full` in its order
/// when a category is chosen) was dropped at position `to`: it takes that place among the shown ids, and the ids not
/// shown keep theirs.
pub(super) fn reorder_shown(full: &[String], shown: &[String], from: usize, to: usize) -> Vec<String> {
    if from >= shown.len() {
        return full.to_vec();
    }
    let moved: Vec<&String> = display_order(shown.len(), from, to).into_iter().map(|i| &shown[i]).collect();
    let mut next = moved.into_iter();
    full.iter().map(|id| if shown.contains(id) { next.next().unwrap_or(id).clone() } else { id.clone() }).collect()
}

/// Whether a press on an icon that has moved `moved` px after `held` ms picks the icon up: with a pointer as soon as it
/// drags past the slop, on a touch screen only after the hold (a quicker drag there pages).
pub(super) fn lifts(pointer: bool, held: f64, moved: f32, slop: f32) -> bool {
    if pointer {
        moved > slop || held >= HOLD_MS
    } else {
        held >= HOLD_MS
    }
}

struct Lay {
    pt: f32,
    cols: usize,
    rows: usize,
    cell_w: f32,
    cell_h: f32,
    icon: usize,
    x0: f32,
    y0: f32,
    /// the capsule header, which is also the search field
    search: (f32, f32, f32, f32),
    /// the glass panel under it (x, y, w, h) and its corner radius
    panel: (f32, f32, f32, f32),
    panel_r: f32,
    /// the chips present, as (chip index, rectangle)
    chips: Vec<(usize, (f32, f32, f32, f32))>,
    dots_y: f32,
}

impl Lay {
    /// Pages sit side by side, one panel width apart.
    fn page_w(&self) -> f32 {
        self.panel.2
    }
    fn in_panel(&self, x: f32, y: f32) -> bool {
        let (px, py, pw, ph) = self.panel;
        x >= px && x < px + pw && y >= py && y < py + ph
    }
    fn in_search(&self, x: f32, y: f32) -> bool {
        let (sx, sy, sw, sh) = self.search;
        x >= sx && x < sx + sw && y >= sy && y < sy + sh
    }
    /// The "..." button at the capsule's right end.
    fn more(&self) -> (f32, f32, f32, f32) {
        let (sx, sy, sw, sh) = self.search;
        (sx + sw - sh * 1.1, sy, sh * 1.1, sh)
    }
    fn in_more(&self, x: f32, y: f32) -> bool {
        let (mx, my, mw, mh) = self.more();
        x >= mx && x < mx + mw && y >= my && y < my + mh
    }
    fn chip_at(&self, x: f32, y: f32) -> Option<usize> {
        self.chips.iter().find(|(_, (cx, cy, cw, ch))| x >= *cx && x < cx + cw && y >= *cy && y < cy + ch).map(|(c, _)| *c)
    }
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
    /// the category chip the grid is filtered to (an index into `CHIPS`)
    chip: Option<usize>,
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
    /// every window's minimized flag from before the grid hid them; restored when the grid closes
    pub(super) saved: Vec<(AppId, bool)>,
    /// the other client's window that was active when the grid opened (the app whose menus the bar shows), if any
    bar_foreign: Option<u32>,
    /// how far the grid is shown (0..1) as last drawn: the backdrop fades with it (`overlay_backdrop`)
    veil: f32,
}

/// The dim the open grid lays over the whole desktop behind it (premultiplied black at this alpha, 35%): the windows of
/// other clients, which the grid cannot hide as it hides the shell's, go dim behind it as the reference's desktop
/// does behind its blur (E2E6 6).
const BACKDROP_ALPHA: f32 = 89.0;

/// Lays a black veil of `alpha` (0..255) under the premultiplied pixels `px` (source-over of `px` on the veil): clear
/// pixels become the veil, opaque ones stay as they are.
fn backdrop_under(px: &mut [u32], alpha: u32) {
    if alpha == 0 {
        return;
    }
    for p in px {
        let a = *p >> 24;
        let out = a + (alpha.min(255) * (255 - a) + 127) / 255;
        *p = (*p & 0x00ff_ffff) | (out.min(255) << 24);
    }
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

    /// The grid is on screen but animating out: a toggle then brings it back instead of asking it to close again.
    pub(super) fn closing(&self) -> bool {
        self.open && self.closing.is_some()
    }

    /// While the grid is open (not closing): the app whose menus the bar keeps. `Some(Some(id))` is another client's
    /// window that was active when the grid opened, `Some(None)` none was (a shell window was in front, see
    /// `bar_front`); `None` when the grid is not up and the bar follows the live windows (Lead (a)).
    #[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
    pub(super) fn bar_foreign(&self) -> Option<Option<u32>> {
        (self.open && self.closing.is_none()).then_some(self.bar_foreign)
    }

    /// While the grid is open (not closing): the shell window that was in front when it opened (the grid hid every
    /// window; `saved` holds them back to front with their own minimized flags).
    pub(super) fn bar_front(&self) -> Option<AppId> {
        if !self.open || self.closing.is_some() {
            return None;
        }
        self.saved.iter().rev().find(|(_, minimized)| !*minimized).map(|(app, _)| *app)
    }

    /// The veil's rectangle (x, y, w, h px): between the menu bar and the Dock. On the window server the grid is
    /// exported as an overlay surface of exactly this size, above every window, with the bar and the Dock left to
    /// their own surfaces (`Shell::desktop_chrome_frames`, id 8). None once it is closed.
    #[allow(dead_code)] // the window-server export is the desktop-wayland build's
    pub(super) fn overlay_rect(&self, sh: &Shell) -> Option<(usize, usize, usize, usize)> {
        if !self.open {
            return None;
        }
        sh.overview_veil_rect()
    }

    /// True while the veil fades: opening, or closing until the grid is gone.
    pub(super) fn veil_animating(&self, t: f64) -> bool {
        self.open && (self.closing.is_some() || prog(Kind::OverviewEnter, Curve::EaseOut, self.start, t) < 1.0)
    }

    fn lay(&self, sh: &Shell) -> Lay {
        let m = sh.desk_metrics();
        let pt = m.pt;
        let (w, h) = (m.w as f32, m.h as f32);
        let menubar = m.menubar as f32;
        // the capsule: 0.37 of the width, centred, in the reference's place down the screen
        let cap_w = (w * CAP_W_FRACTION).max(CAP_MIN_PT * pt).min(w * 0.9);
        let cap_h = CAP_H_PT * pt;
        let cap_y = (h * CAP_TOP_FRACTION).max(menubar + CAP_TOP_MIN_PT * pt);
        let search = ((w - cap_w) / 2.0, cap_y, cap_w, cap_h);
        // the panel: 0.6 of the width under it, ending above the Dock; room for seven readable columns at least
        let pw = (w * PANEL_W_FRACTION).max(COLS as f32 * 72.0 * pt).min(w * 0.96);
        let py = cap_y + cap_h + PANEL_GAP_PT * pt;
        let dock = if sh.dock_is_visible() { m.dock_bottom as f32 } else { 16.0 * pt };
        let limit = h - dock - DOCK_CLEAR_PT * pt;
        let ph = (h * PANEL_H_FRACTION).min(limit - py).max(160.0 * pt);
        let panel = ((w - pw) / 2.0, py, pw, ph);
        let pad = PANEL_PAD_PT * pt;
        let inner = pw - 2.0 * pad;
        let cell_w = inner / COLS as f32;
        let icon = ((cell_w * ICON_FRACTION).min(ICON_MAX_PT * pt)) as usize;
        let cell_h = icon as f32 + LABEL_BLOCK_PT * pt;
        // chips across the top, their widths from the text
        let present = present_chips(apps().iter().map(chip_of));
        let widths: Vec<f32> = present.iter().map(|&c| sh.label(CHIPS[c], MAC_BODY_PT, Weight::Regular, WHITE, |im| im.w as f32)).collect();
        let chip_y = py + pad;
        let chip_h = CHIP_H_PT * pt;
        let rects = chip_rects(&widths, panel.0 + pad, chip_y, inner, chip_h, CHIP_PAD_PT * pt, 4.0 * pt, CHIP_GAP_PT * pt, CHIP_MAX_GAP_PT * pt);
        let chips = present.into_iter().zip(rects).collect();
        let top = chip_y + chip_h + CHIPS_BELOW_PT * pt;
        let bottom = py + ph - DOTS_BLOCK_PT * pt;
        let rows = grid_rows(bottom - top, cell_h);
        let (x0, y0) = (panel.0 + pad, top);
        let dots_y = py + ph - DOTS_BLOCK_PT * pt * 0.5 - DOT_PT * pt / 2.0;
        Lay { pt, cols: COLS, rows, cell_w, cell_h, icon, x0, y0, search, panel, panel_r: PANEL_R_PT * pt, chips, dots_y }
    }

    fn arranged(&self) -> Vec<usize> {
        let mut list = arrange(&catalogue(), &self.order, &self.query);
        if let Some(chip) = self.chip {
            let all = apps();
            list.retain(|&i| all.get(i).is_some_and(|a| chip_of(a) == chip));
        }
        list
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
        self.chip = None;
        self.page = 0;
        self.swipe_dx = 0.0;
        self.settle = None;
        self.press = None;
        self.lifted = None;
        self.selected = None;
        self.scroll_acc = 0.0;
        // the bar keeps the app in front: hiding the shell's windows makes the window server activate another client's
        // window, which must not take the menu bar while the grid is up (Lead (a))
        self.bar_foreign = sh.foreign_front_window();
        if self.saved.is_empty() {
            if let Some(d) = sh.desk.as_mut() {
                self.saved = d.wins.iter().map(|w| (w.app, w.minimized)).collect();
                for w in d.wins.iter_mut() {
                    w.minimized = true;
                }
            }
        }
        sh.cc.hide();
        sh.dirty = true;
    }

    pub(super) fn close(&mut self, sh: &mut Shell, t: f64) {
        if !self.open || self.closing.is_some() {
            return;
        }
        if let Some(d) = sh.desk.as_mut() {
            let mut shown = Vec::new();
            for (app, min) in std::mem::take(&mut self.saved) {
                if let Some(w) = d.wins.iter_mut().find(|w| w.app == app) {
                    w.minimized = min;
                    if !min {
                        shown.push(app);
                    }
                }
            }
            // the grid destroyed their toplevels and the window server activated another client's window meanwhile: the
            // windows come back over it in their own order, not only the front one (E2E5 1). When another client's
            // window was in front as the grid opened, they stay where the window server maps them.
            if self.bar_foreign.is_none() {
                d.note_restack(&shown);
            }
        }
        sh.desk_refocus();
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
                let from = self.offset(t) + if fwd { lay.page_w() } else { -lay.page_w() };
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
                if self.query.is_empty() {
                    self.chip = None;
                }
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
                    let dir = if page > self.page { lay.page_w() } else { -lay.page_w() };
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
                    let (px, _, pw, _) = lay.panel;
                    let edge = pw * EDGE_FRACTION;
                    if t - self.edge_at > EDGE_REPEAT_MS {
                        let next = if x < px + edge && self.page > 0 { Some(self.page - 1) } else if x > px + pw - edge && self.page + 1 < pages { Some(self.page + 1) } else { None };
                        if let Some(n) = next {
                            self.edge_at = t;
                            let from = if n > self.page { lay.page_w() } else { -lay.page_w() };
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
                } else if let (Some(item), true) = (p.item, self.query.is_empty() && lifts(sh.form.is_desktop(), t - p.t0, (x - p.x0).hypot(y - p.y0), slop)) {
                    // a pointer drag on an icon, or a press held then moved on a touch screen: lift the icon
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
                    // dropped on a slot: the icon takes that place (an empty slot after the last icon is the last place);
                    // dropped off the grid, nothing moves
                    if let Some(k) = lay.slot_at(x, y) {
                        self.save_order(sh, &list, item, self.page * per + k);
                    }
                } else if p.swiping {
                    let off = self.swipe_dx;
                    let turn = off.abs() > lay.page_w() * 0.15 || p.vel.abs() > FLICK;
                    let toward_next = off < 0.0 || p.vel < -FLICK;
                    let next = if !turn { self.page } else if toward_next { (self.page + 1).min(pages - 1) } else { self.page.saturating_sub(1) };
                    let from = if next > self.page { off + lay.page_w() } else if next < self.page { off - lay.page_w() } else { off };
                    self.set_page(next, pages, from, t);
                } else if (x - p.x0).abs() <= slop && (y - p.y0).abs() <= slop {
                    if let (Some(item), Some(k)) = (p.item, lay.slot_at(x, y)) {
                        if self.page * per + k == item {
                            if let Some(&a) = list.get(item) {
                                self.launch(sh, a, t);
                            }
                        }
                    } else if let Some(d) = p.dot.filter(|_| self.dot_at(&lay, pages, x, y) == p.dot) {
                        let from = (d as f32 - self.page as f32) * lay.page_w();
                        self.set_page(d, pages, from, t);
                    } else if let Some(c) = lay.chip_at(x, y) {
                        // a chip filters to its category; the same chip again shows every app
                        self.chip = if self.chip == Some(c) { None } else { Some(c) };
                        self.page = 0;
                        self.selected = None;
                    } else if lay.in_more(x, y) {
                        // the "..." button clears the search and the category
                        self.query.clear();
                        self.chip = None;
                        self.page = 0;
                        self.selected = None;
                    } else if !lay.in_search(x, y) && !lay.in_panel(x, y) {
                        self.close(sh, t);
                    }
                }
            }
            _ => return None,
        }
        sh.dirty = true;
        Some(vec![])
    }

    /// The alpha mask of the exported surface, which spans `(ox, oy, w, h)` px of the screen: opaque on the capsule and the
    /// panel (rounded as drawn), clear everywhere else so the wallpaper and the windows show around them.
    #[allow(dead_code)] // the window-server export is the desktop-wayland build's
    pub(super) fn overlay_mask(&self, sh: &Shell, (ox, oy, w, h): (usize, usize, usize, usize)) -> Vec<u8> {
        let lay = self.lay(sh);
        let mut mask = vec![0u8; w * h];
        for ((x, y, rw, rh), r) in [(lay.search, lay.search.3 / 2.0), (lay.panel, lay.panel_r)] {
            let (rx, ry) = (x.floor() as i64 - ox as i64, y.floor() as i64 - oy as i64);
            let (rw, rh) = (rw.ceil() as usize, rh.ceil() as usize);
            for (i, a) in crate::gfx::rrect_mask(rw, rh, r).into_iter().enumerate() {
                let (mx, my) = (rx + (i % rw.max(1)) as i64, ry + (i / rw.max(1)) as i64);
                if mx >= 0 && my >= 0 && (mx as usize) < w && (my as usize) < h {
                    let at = my as usize * w + mx as usize;
                    mask[at] = mask[at].max(a);
                }
            }
        }
        mask
    }

    /// Puts the backdrop under the exported surface's masked pixels (`overlay_mask`): the whole veil dims what is behind
    /// it, faded with the grid's opening and closing.
    #[allow(dead_code)] // the window-server export is the desktop-wayland build's
    pub(super) fn overlay_backdrop(&self, px: &mut [u32]) {
        if self.open {
            backdrop_under(px, (BACKDROP_ALPHA * self.veil.clamp(0.0, 1.0)).round() as u32);
        }
    }

    fn dot_at(&self, lay: &Lay, pages: usize, x: f32, y: f32) -> Option<usize> {
        if pages < 2 || (y - lay.dots_y).abs() > 14.0 * lay.pt {
            return None;
        }
        let step = (DOT_PT + DOT_GAP_PT) * lay.pt;
        let x0 = lay.panel.0 + (lay.panel.2 - step * pages as f32 + DOT_GAP_PT * lay.pt) / 2.0;
        let i = ((x - x0) / step).floor();
        (i >= 0.0 && (i as usize) < pages).then_some(i as usize)
    }

    /// Saves the order after the icon at position `from` of the shown list `list` (catalogue indices) was dropped at
    /// position `to`.
    fn save_order(&mut self, sh: &mut Shell, list: &[usize], from: usize, to: usize) {
        let to = to.min(list.len().saturating_sub(1));
        if from == to || from >= list.len() {
            return;
        }
        let all = catalogue();
        let full: Vec<String> = arrange(&all, &self.order, "").into_iter().map(|i| all[i].0.clone()).collect();
        let shown: Vec<String> = list.iter().map(|&i| all[i].0.clone()).collect();
        let full = reorder_shown(&full, &shown, from, to);
        if sh.prefs.set(ORDER_PREF, &full.join(",")).is_err() {
            eprintln!("BifrOSt: could not save the Apps grid order");
        }
        self.order = full;
    }

    /// Makes sure the icon of `app_index` at `size` px is in the cache.
    /// Forgets the icons, so the next draw takes them from the shell's base icons again.
    pub(super) fn clear_icons(&mut self) {
        self.icons.clear();
    }

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
        self.veil = p;
        let a = (255.0 * p) as u32;
        let ui = &sh.ui;
        // the capsule header: glass like the Search field, the store glyph, the query (or its placeholder) and "..."
        let (sx, sy, _, shh) = lay.search;
        mac_glass(s, sx, sy, lay.search.2, shh, shh / 2.0, a);
        let glyph = (shh * 0.34) as usize;
        let mut text_x = sx + shh * 0.62;
        if let Some(store) = apps().iter().position(|d| d.id == AppId::AppStore) {
            if self.ensure_icon(sh, store, glyph) {
                if let Some(img) = self.icons.get(&(store, glyph)) {
                    s.blit(img, text_x as i32, (sy + (shh - glyph as f32) / 2.0) as i32, 255, a);
                }
            }
        }
        text_x += glyph as f32 + shh * 0.2;
        // the chosen category sits in the field as a token, as the query would
        if let Some(c) = self.chip {
            let pad = ui.pt(10.0);
            let w = sh.label(CHIPS[c], MAC_LARGE_TITLE_PT, Weight::Regular, WHITE, |im| {
                let th = im.h as f32 + pad;
                s.fill_rrect(text_x, sy + (shh - th) / 2.0, im.w as f32 + 2.0 * pad, th, ui.pt(8.0), scale(SIDEBAR_PLATE.now(), a));
                s.blit_colored(im, (text_x + pad) as i32, (sy + (shh - im.h as f32) / 2.0) as i32, LABEL.now(), a);
                im.w as f32 + 2.0 * pad
            });
            text_x += w + ui.pt(6.0);
        }
        if self.query.is_empty() && self.chip.is_none() {
            sh.label("Applications", MAC_LARGE_TITLE_PT, Weight::Regular, WHITE, |im| s.blit_colored(im, text_x as i32, (sy + (shh - im.h as f32) / 2.0) as i32, LABEL_2.now(), a));
        } else if !self.query.is_empty() {
            let q = self.query.clone();
            let w = sh.label(&q, MAC_LARGE_TITLE_PT, Weight::Regular, WHITE, |im| {
                s.blit_colored(im, text_x as i32, (sy + (shh - im.h as f32) / 2.0) as i32, LABEL.now(), a);
                im.w as f32
            });
            text_x += w;
        }
        if !self.query.is_empty() || self.chip.is_some() {
            s.fill_rect((text_x + ui.pt(1.0)) as usize, (sy + shh * 0.28) as usize, ui.pt(1.0).max(1.0) as usize, (shh * 0.44) as usize, scale(LABEL.now(), a));
        }
        // "..." at the right end: three dots, clears the search and the category
        let (mx, my, mw, mh) = lay.more();
        let dot = MORE_DOT_PT * lay.pt;
        for i in 0..3 {
            let dx = mx + mw / 2.0 + (i as f32 - 1.0) * dot * 2.4 - dot / 2.0;
            s.fill_rrect(dx, my + (mh - dot) / 2.0, dot, dot, dot / 2.0, scale(LABEL_2.now(), a));
        }
        // the panel: the same translucent material as the field (the search tint, not the menus' nearly opaque one), so the
        // wallpaper shows through it blurred, with the category chips across its top
        let (px, py, pw, ph) = lay.panel;
        panel_glass(s, ShellSurface::AppGridVeil, px, py, pw, ph, lay.panel_r, a);
        for &(c, (cx, cy, cw, ch)) in &lay.chips {
            let on = self.chip == Some(c);
            let plate = if on { scale(SIDEBAR_PLATE.now(), a) } else { scale(FIELD_BG.now(), a * 110 / 255) };
            s.fill_rrect(cx, cy, cw, ch, ch / 2.0, plate);
            sh.label(CHIPS[c], MAC_BODY_PT, Weight::Regular, WHITE, |im| s.blit_colored(im, (cx + (cw - im.w as f32) / 2.0) as i32, (cy + (ch - im.h as f32) / 2.0) as i32, LABEL.now(), a));
        }
        // pages: the current one and its neighbours while they slide
        let list = self.arranged();
        let per = lay.per_page().max(1);
        let pages = self.pages(&lay, list.len());
        self.page = self.page.min(pages - 1);
        // a touch held still on an icon lifts it under the finger once the hold is long enough
        let mut holding = false;
        if let Some(pr) = self.press.filter(|pr| !pr.swiping && self.lifted.is_none() && self.query.is_empty() && !sh.form.is_desktop()) {
            if let Some(item) = pr.item {
                if t - pr.t0 >= HOLD_MS {
                    self.lifted = Some((item, pr.x0, pr.y0));
                    self.edge_at = t;
                } else {
                    holding = true;
                }
            }
        }
        let off = self.offset(t);
        let rise = (1.0 - p) * 14.0 * lay.pt;
        let lifted = self.lifted.map(|l| l.0);
        let hover_slot = self.lifted.and_then(|(_, x, y)| lay.slot_at(x, y));
        // while an icon is held over a slot of this page the others make room for it there
        let order = match (lifted, hover_slot) {
            (Some(item), Some(k)) => display_order(list.len(), item, self.page * per + k),
            (Some(item), None) => display_order(list.len(), item, item),
            _ => (0..list.len()).collect(),
        };
        for pg in self.page.saturating_sub(1)..(self.page + 2).min(pages) {
            let dx = (pg as f32 - self.page as f32) * lay.page_w() + off;
            if dx > lay.page_w() || dx < -lay.page_w() {
                continue;
            }
            let (px, _, pw, _) = lay.panel;
            for k in 0..per {
                let i = pg * per + k;
                let Some(&shown) = order.get(i) else { break };
                let Some(&app) = list.get(shown) else { break };
                let (cx, cy) = lay.cell(k, dx);
                let (cx, cy) = (cx, cy + rise);
                // a page sliding in or out is clipped to the panel: cells wholly outside are not drawn
                if cx + lay.cell_w <= px || cx >= px + pw {
                    continue;
                }
                // the held icon's place: an empty slot with a plate where it will drop
                if lifted == Some(shown) {
                    if pg == self.page && hover_slot == Some(k) {
                        s.fill_rrect(cx + lay.pt * 6.0, cy - lay.pt * 4.0, lay.cell_w - lay.pt * 12.0, lay.cell_h - lay.pt * 6.0, ui.pt(16.0), scale(SIDEBAR_PLATE.now(), a));
                    }
                    continue;
                }
                if self.selected == Some(i) && lifted.is_none() {
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
            let x0 = lay.panel.0 + (lay.panel.2 - step * pages as f32 + DOT_GAP_PT * lay.pt) / 2.0;
            let d = DOT_PT * lay.pt;
            for pg in 0..pages {
                let c = if pg == self.page { LABEL.now() } else { LABEL_3.now() };
                s.fill_rrect(x0 + pg as f32 * step, lay.dots_y, d, d, d / 2.0, scale(c, a));
            }
        }
        if list.is_empty() {
            let msg = format!("No results for \u{201c}{}\u{201d}", self.query);
            sh.label(&msg, MAC_BODY_PT, Weight::Regular, WHITE, |im| s.blit_colored(im, (lay.panel.0 + (lay.panel.2 - im.w as f32) / 2.0) as i32, (lay.y0 + lay.cell_h) as i32, LABEL_2.now(), a));
        }
        p < 1.0 || self.closing.is_some() || self.settle.is_some_and(|(_, st)| prog(Kind::PanelReveal, Curve::EaseOut, st, t) < 1.0) || self.lifted.is_some() || holding
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
        while chars.len() > 1 && sh.label(&text, MAC_CALLOUT_PT, Weight::Regular, WHITE, |im| im.w as f32) > max_w {
            chars.pop();
            text = format!("{}\u{2026}", chars.iter().collect::<String>().trim_end());
        }
        let ly = cy + size as f32 + 8.0 * lay.pt;
        sh.label(&text, MAC_CALLOUT_PT, Weight::Regular, WHITE, |im| s.blit_colored(im, (cx + (lay.cell_w - im.w as f32) / 2.0) as i32, ly as i32, LABEL.now(), alpha));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// E2E6 6: the grid's surface no longer leaves the desktop around it clear (another client's window showed there
    /// as if the grid were not open): clear pixels become the dim veil, the opaque capsule and panel are untouched, an
    /// anti-aliased edge sits over the veil, and a grid fading to nothing lays no veil.
    #[test]
    fn the_backdrop_dims_what_the_grid_leaves_clear() {
        let mut px = vec![0u32, 0xff20_3040, 0x8010_1010];
        backdrop_under(&mut px, 89);
        assert_eq!(px[0], 89 << 24, "clear: premultiplied black at the veil's alpha");
        assert_eq!(px[1], 0xff20_3040, "opaque: as drawn");
        assert_eq!(px[2] & 0x00ff_ffff, 0x0010_1010, "an edge keeps its colour");
        assert!((px[2] >> 24) > 0x80 && (px[2] >> 24) < 0xff, "and gains the veil under it");
        let mut clear = vec![0u32; 4];
        backdrop_under(&mut clear, 0);
        assert!(clear.iter().all(|&p| p == 0));
        let mut grid = AppGrid::default();
        let mut px = vec![0u32; 2];
        grid.overlay_backdrop(&mut px);
        assert!(px.iter().all(|&p| p == 0), "a closed grid exports no veil");
        grid.open = true;
        grid.veil = 1.0;
        grid.overlay_backdrop(&mut px);
        assert!(px.iter().all(|&p| p == (BACKDROP_ALPHA as u32) << 24), "fully open: the whole veil");
    }

    fn all() -> Vec<(String, String)> {
        ["Settings", "Term", "Photos", "Files", "App Store"].iter().map(|n| (n.to_lowercase().replace(' ', "."), n.to_string())).collect()
    }

    #[test]
    fn grid_rows_fit_the_panel_whatever_its_size() {
        assert_eq!(grid_rows(600.0, 150.0), 4);
        assert_eq!(grid_rows(10_000.0, 150.0), MAX_ROWS);
        assert_eq!(grid_rows(10.0, 150.0), 1, "a tiny panel keeps one row");
    }

    #[test]
    fn manifest_categories_map_to_the_chips_and_unknown_ones_are_other() {
        assert_eq!(CHIPS[chip_for_manifest("is.olibuijr.calculator.desktop", "Utilities")], "Utilities");
        assert_eq!(CHIPS[chip_for_manifest("x", "Productivity")], "Productivity & Finance");
        assert_eq!(CHIPS[chip_for_manifest("x", " photo & video ")], "Creativity");
        assert_eq!(CHIPS[chip_for_manifest("x", "Music")], "Entertainment");
        assert_eq!(CHIPS[chip_for_manifest("x", "Books")], "Information & Reading");
        assert_eq!(CHIPS[chip_for_manifest("x", "Something new")], "Other");
        assert_eq!(CHIPS[chip_for_manifest("is.olibuijr.messages.desktop", "Productivity")], "Social", "the id table wins over the category");
        assert!(MANIFEST_CHIP.iter().chain(ID_CHIP).all(|&(_, c)| c < CHIPS.len()));
        assert_eq!(CHIPS[OTHER_CHIP], "Other");
        assert_eq!(CHIPS[builtin_chip(AppId::Settings)], "Utilities");
        assert_eq!(CHIPS[builtin_chip(AppId::Photos)], "Creativity");
    }

    #[test]
    fn only_chips_with_apps_are_shown_in_order() {
        assert_eq!(present_chips([5usize, 0, 5, 2].into_iter()), vec![0, 2, 5]);
        assert!(present_chips(std::iter::empty()).is_empty());
    }

    #[test]
    fn chips_are_centred_spread_and_shrink_to_fit() {
        let widths = [60.0, 100.0, 80.0];
        let r = chip_rects(&widths, 10.0, 5.0, 500.0, 28.0, 14.0, 4.0, 8.0, 24.0);
        assert_eq!(r.len(), 3);
        assert!(r.windows(2).all(|p| p[1].0 >= p[0].0 + p[0].2 + 8.0 - 0.01), "no overlap, at least the gap apart");
        assert!(r.windows(2).all(|p| p[1].0 - (p[0].0 + p[0].2) <= 24.0 + 0.01), "never spread past the maximum gap");
        let (left, right) = (r[0].0 - 10.0, 510.0 - (r[2].0 + r[2].2));
        assert!((left - right).abs() < 0.01, "centred in the row");
        assert_eq!((r[0].1, r[0].3), (5.0, 28.0));
        // too wide for the row: the padding gives way, the chips still do not overlap or leave the row
        let wide = chip_rects(&[200.0, 200.0, 200.0], 0.0, 0.0, 640.0, 28.0, 14.0, 4.0, 8.0, 24.0);
        assert!(wide[0].0 >= 0.0 && wide[2].0 + wide[2].2 <= 640.0 + 0.01, "{wide:?}");
        assert!(wide.windows(2).all(|p| p[1].0 >= p[0].0 + p[0].2));
        assert!(chip_rects(&[], 0.0, 0.0, 100.0, 28.0, 14.0, 4.0, 8.0, 24.0).is_empty());
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

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_dropped_icon_takes_the_place_it_was_dropped_on_forwards_and_backwards() {
        let o = ids(&["a", "b", "c", "d"]);
        assert_eq!(reorder_shown(&o, &o, 3, 1), ids(&["a", "d", "b", "c"]), "d dropped on b's place");
        assert_eq!(reorder_shown(&o, &o, 0, 2), ids(&["b", "c", "a", "d"]), "a dropped on c's place: a is third");
        assert_eq!(reorder_shown(&o, &o, 1, 9), ids(&["a", "c", "d", "b"]), "past the end is the last place");
        assert_eq!(reorder_shown(&o, &o, 2, 2), o, "dropped where it was");
        assert_eq!(reorder_shown(&o, &o, 7, 0), o, "nothing held");
    }

    #[test]
    fn with_a_category_chosen_only_the_shown_icons_move_and_the_rest_keep_their_places() {
        let full = ids(&["a", "x", "b", "y", "c"]);
        let shown = ids(&["a", "b", "c"]);
        assert_eq!(reorder_shown(&full, &shown, 2, 0), ids(&["c", "x", "a", "y", "b"]));
    }

    #[test]
    fn the_other_icons_make_room_where_the_held_one_hovers() {
        assert_eq!(display_order(5, 0, 3), vec![1, 2, 3, 0, 4], "held forwards: the ones between close up");
        assert_eq!(display_order(5, 4, 1), vec![0, 4, 1, 2, 3], "held backwards: the ones from the slot move on");
        assert_eq!(display_order(3, 1, 1), vec![0, 1, 2]);
        assert_eq!(display_order(3, 0, 10), vec![1, 2, 0], "past the end: the last place");
        assert_eq!(display_order(2, 5, 0), vec![0, 1], "an index past the list holds nothing");
    }

    #[test]
    fn a_pointer_drag_lifts_at_once_and_a_touch_only_after_the_hold() {
        let slop = 8.0;
        assert!(lifts(true, 40.0, 20.0, slop), "a mouse drag on an icon picks it up (Launchpad)");
        assert!(!lifts(true, 40.0, 4.0, slop), "a click that barely moves does not");
        assert!(lifts(true, HOLD_MS, 0.0, slop));
        assert!(!lifts(false, 40.0, 300.0, slop), "a quick swipe on a touch screen pages");
        assert!(lifts(false, HOLD_MS + 1.0, 30.0, slop), "a held touch then a drag picks it up");
    }
}

/// The Apps grid as an overlay surface above the windows, and how it closes (E2E L1, L3, OV1).
#[cfg(all(test, feature = "desktop-wayland"))]
mod overlay_tests {
    use super::chip_of;
    use crate::gfx::Surface;
    use crate::shell::hwkeys::Shortcut;
    use crate::shell::tests::desktop_shell;
    use crate::shell::{Event, Screen, Shell};

    fn open_grid() -> Shell {
        let mut sh = desktop_shell();
        // the fixture starts on the lock screen, where nothing opens and no chrome is exported
        sh.anim = None;
        sh.screen = Screen::Home;
        sh.appgrid_toggle(1.0);
        assert!(sh.overview.grid.visible(), "the grid opens");
        sh
    }

    fn frame_ids(sh: &Shell) -> Vec<u32> {
        let (w, h) = (sh.w as usize, sh.h as usize);
        let pixels = vec![0u32; w * h];
        sh.desktop_chrome_frames(&pixels, w, h).iter().map(|f| f.id).collect()
    }

    /// Runs the grid's close animation to its end, as the frames after the close would.
    fn finish(sh: &mut Shell) {
        let (w, h) = (sh.w as usize, sh.h as usize);
        let mut px = vec![0u32; w * h];
        let mut grid = std::mem::take(&mut sh.overview.grid);
        grid.draw(sh, &mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 1e9);
        sh.overview.grid = grid;
    }

    #[test]
    fn the_open_grid_is_an_overlay_surface_between_the_bar_and_the_dock_and_closing_removes_it() {
        let mut sh = open_grid();
        let (w, h) = (sh.w as usize, sh.h as usize);
        let pixels = vec![0u32; w * h];
        let frames = sh.desktop_chrome_frames(&pixels, w, h);
        let f = frames.iter().find(|f| f.id == 8).expect("the open grid is a chrome surface above the windows");
        assert_eq!(f.pixels.len(), f.width * f.height);
        assert_eq!((f.x, f.y as usize, f.width), (0, sh.desktop_bar_height(), w), "the veil: the full width under the menu bar");
        assert!(f.y as usize + f.height <= h - sh.desk_metrics().dock_bottom.max(0) as usize, "and above the Dock");
        assert!(f.input.is_none(), "the whole surface takes the pointer, so a click outside the icons closes it");
        sh.appgrid_toggle(2.0);
        finish(&mut sh);
        assert!(!sh.overview.grid.visible());
        assert!(!frame_ids(&sh).contains(&8), "the frame it closes in drops the surface");
    }

    /// E2E12 4: the exported grid surface, above every toplevel (VEIL rank), carries the dim backdrop over the whole veil
    /// outside the capsule and the panel, so another client's window behind it goes dim with the wallpaper.
    #[test]
    fn the_exported_grid_surface_dims_everything_around_the_capsule_and_panel() {
        let mut sh = open_grid();
        let (w, h) = (sh.w as usize, sh.h as usize);
        let mut px = vec![0u32; w * h];
        let mut grid = std::mem::take(&mut sh.overview.grid);
        grid.draw(&sh, &mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 1e9);
        sh.overview.grid = grid;
        let pixels = vec![0u32; w * h];
        let frames = sh.desktop_chrome_frames(&pixels, w, h);
        let f = frames.iter().find(|f| f.id == crate::shell::chrome_ids::APP_GRID).expect("the grid surface");
        assert!(crate::bare::wayland::chrome_z(f.id) > crate::bare::wayland::chrome_z(crate::shell::chrome_ids::DOCK), "above the windows and the Dock");
        let veil = (super::BACKDROP_ALPHA as u32) << 24;
        for (x, y) in [(0, 0), (f.width - 1, 0), (0, f.height - 1), (f.width - 1, f.height - 1)] {
            assert_eq!(f.pixels[y * f.width + x], veil, "the corner ({x}, {y}) of the veil is dimmed");
        }
    }

    #[test]
    fn the_apps_icon_while_the_grid_animates_out_opens_it_again() {
        let mut sh = open_grid();
        sh.appgrid_toggle(2.0);
        sh.appgrid_toggle(2.1);
        assert!(sh.overview.grid.visible() && !sh.overview.grid.closing(), "a toggle during the close brings the grid back");
        sh.appgrid_toggle(2.2);
        finish(&mut sh);
        assert!(!sh.overview.grid.visible(), "and the next one closes it");
    }

    /// Lead (a): the grid hides the windows, and the window server then activates another client's window; the bar keeps
    /// the app that was in front (its name and menus), the screen stays that app, and Esc with a bar menu open over the
    /// grid closes the menu first, leaving the grid up.
    #[test]
    fn the_bar_keeps_the_front_app_under_the_grid_and_escape_closes_its_menu_first() {
        use crate::shell::apps::AppId;
        use crate::shell::desktop_bar::{FrontApp, Menu};
        use crate::shell::overview::{KeyMods, KEY_ESC};
        let mut sh = desktop_shell();
        sh.anim = None;
        let m = sh.desk_metrics();
        {
            let d = sh.desk.as_mut().unwrap();
            d.wins.clear();
            d.open(&m, AppId::Terminal, (900, 600));
        }
        sh.screen = Screen::App(AppId::Terminal);
        sh.appgrid_toggle(1.0);
        assert!(sh.overview.grid.visible());
        assert!(sh.desk.as_ref().unwrap().front().is_none(), "the grid hid the window");
        assert!(sh.front_app() == FrontApp::Desk(AppId::Terminal), "the bar keeps the app that was in front");
        sh.desk_sync_extra_windows();
        assert_eq!(sh.screen, Screen::App(AppId::Terminal), "the per-tick refocus leaves the screen to that app");
        sh.desktop_menu = Some(Menu::App);
        let mods = KeyMods { ctrl: false, cmd: false, shift: false };
        assert!(!sh.hw_overview_key(KEY_ESC, true, mods, false, 2.0), "Esc is the open menu's, not the grid's");
        sh.hw_shortcut(Shortcut::Back, 2.0);
        assert!(sh.desktop_menu.is_none(), "the first Esc closes the menu");
        assert!(sh.overview.grid.visible() && !sh.overview.grid.closing(), "and the grid stays up");
        sh.hw_shortcut(Shortcut::Back, 3.0);
        finish(&mut sh);
        assert!(!sh.overview.grid.visible(), "the second Esc closes the grid");
        assert_eq!(sh.desk.as_ref().unwrap().front().map(|w| w.app), Some(AppId::Terminal));
        assert!(sh.overview.grid.bar_front().is_none() && sh.overview.grid.bar_foreign().is_none(), "the bar follows the live windows again");
    }

    /// E2E5 1: the grid destroyed the windows' toplevels and the window server activated another client's window; when
    /// the grid closes the window server is told every window that comes back, bottom first, so they all stack over that
    /// window in their order (only the front one came back over FilmCraft). When another client's window was in front as
    /// the grid opened, nothing is restacked over it.
    #[test]
    fn closing_the_grid_brings_every_window_back_in_its_order_over_other_clients() {
        use crate::shell::apps::AppId;
        use crate::shell::desk::win_id;
        let mut sh = desktop_shell();
        sh.anim = None;
        let m = sh.desk_metrics();
        {
            let d = sh.desk.as_mut().unwrap();
            d.wins.clear();
            d.open(&m, AppId::Terminal, (900, 600));
            d.open(&m, AppId::Photos, (900, 600));
            d.open(&m, AppId::Settings, (900, 600));
            let p = d.index_of(AppId::Photos).unwrap();
            d.wins[p].minimized = true;
        }
        sh.screen = Screen::App(AppId::Settings);
        sh.take_wayland_restack();
        sh.appgrid_toggle(1.0);
        assert!(sh.take_wayland_restack().is_empty(), "opening the grid restacks nothing");
        sh.appgrid_toggle(2.0);
        finish(&mut sh);
        let want: Vec<u32> = [AppId::Terminal, AppId::Settings].into_iter().filter_map(win_id).collect();
        assert_eq!(want.len(), 2);
        assert_eq!(sh.take_wayland_restack(), want, "the shown windows, bottom first; the minimized one stays in the Dock");
        // another client's window was in front when the grid opened: the shell's windows are not lifted over it
        sh.desk.as_mut().unwrap().native_windows.push(crate::bare::wayland::ForeignWindow { id: 40, title: "Film".into(), app_id: "x.film".into(), active: true, maximized: false, minimized: false });
        sh.appgrid_toggle(3.0);
        sh.appgrid_toggle(4.0);
        finish(&mut sh);
        assert!(sh.take_wayland_restack().is_empty());
    }

    /// E2E5 5: Overview hides the windows and the window server then activates another client's window; the bar keeps
    /// the app that was in front when Overview opened, as under the Apps grid, until Overview has gone.
    #[test]
    fn the_bar_keeps_the_front_app_under_overview() {
        use crate::shell::apps::AppId;
        use crate::shell::desktop_bar::FrontApp;
        let mut sh = desktop_shell();
        sh.anim = None;
        let m = sh.desk_metrics();
        {
            let d = sh.desk.as_mut().unwrap();
            d.wins.clear();
            d.open(&m, AppId::Terminal, (900, 600));
        }
        sh.screen = Screen::App(AppId::Terminal);
        // a native window behind, inactive while Terminal is in front
        sh.desk.as_mut().unwrap().native_windows.push(crate::bare::wayland::ForeignWindow { id: 41, title: "Film".into(), app_id: "x.film".into(), active: false, maximized: false, minimized: false });
        sh.overview_toggle(1.0);
        assert!(sh.overview.open, "Overview opens");
        assert!(sh.desk.as_ref().unwrap().front().is_none(), "Overview hid the window");
        // the window server activates the native window once the shell's toplevel is gone
        sh.desk.as_mut().unwrap().native_windows[0].active = true;
        assert!(sh.front_app() == FrontApp::Desk(AppId::Terminal), "the bar keeps Terminal, not the activated window");
        sh.desk_sync_extra_windows();
        assert_eq!(sh.screen, Screen::App(AppId::Terminal), "the per-tick refocus leaves the screen to that app");
        assert_eq!(sh.bar_held_front(), Some(AppId::Terminal));
        assert_eq!(sh.bar_held_foreign(), Some(None));
    }

    #[test]
    fn escape_closes_the_grid_before_it_reaches_anything_under_it() {
        let mut sh = open_grid();
        sh.hw_shortcut(Shortcut::Back, 2.0);
        finish(&mut sh);
        assert!(!sh.overview.grid.visible(), "Escape closes the Apps grid");
        assert!(!frame_ids(&sh).contains(&8));
    }

    #[test]
    fn the_dock_icon_closes_it_and_a_click_outside_needs_one_click() {
        let mut sh = open_grid();
        // a press on the Dock falls through, and so does its release: the Dock acts on release
        let d = sh.dock_overlay_rect();
        let (x, y) = ((d.x + d.w / 2) as f32, (d.y + d.h / 2) as f32);
        assert!(sh.overview_event(Event::Down(x, y), 2.0).is_none(), "the press reaches the Dock");
        assert!(sh.overview_event(Event::Up(x, y), 2.1).is_none(), "and so does its release");
        sh.appgrid_toggle(2.2);
        finish(&mut sh);
        assert!(!sh.overview.grid.visible(), "the Apps icon toggles it closed");
        // one click on the empty veil under the menu bar closes it
        let mut sh = open_grid();
        let (cx, cy) = (2.0, sh.desktop_bar_height() as f32 + 2.0);
        assert!(sh.overview_event(Event::Down(cx, cy), 3.0).is_some());
        assert!(sh.overview_event(Event::Up(cx, cy), 3.1).is_some());
        finish(&mut sh);
        assert!(!sh.overview.grid.visible(), "one click outside the icons closes the grid");
    }

    #[test]
    fn the_surface_is_clear_around_the_capsule_and_the_panel_and_the_panel_is_seven_wide() {
        let sh = open_grid();
        let (x, y, w, h) = sh.overview_veil_rect().expect("room for the veil");
        let mask = sh.overview.grid.overlay_mask(&sh, (x, y, w, h));
        assert_eq!(mask.len(), w * h);
        let lay = sh.overview.grid.lay(&sh);
        let at = |px: f32, py: f32| mask[(py as usize - y) * w + px as usize - x];
        assert_eq!(at(x as f32 + 2.0, y as f32 + 2.0), 0, "the wallpaper shows at the veil's corner");
        let (px, py, pw, ph) = lay.panel;
        assert_eq!(at(px + pw / 2.0, py + ph / 2.0), 255, "the panel is opaque");
        let (sx, sy, sw, sh_) = lay.search;
        assert_eq!(at(sx + sw / 2.0, sy + sh_ / 2.0), 255, "so is the capsule");
        assert_eq!(at(sx + sw / 2.0, (sy + sh_ + py) / 2.0), 0, "and the gap between them is clear");
        assert_eq!(at(px + pw + 4.0, py + ph / 2.0), 0, "and the sides of the panel");
        assert_eq!(lay.cols, 7);
        assert!(pw < w as f32 * 0.7 && ph < sh.h as f32 * 0.7, "a floating panel, not a full screen page");
        assert!(sw > pw * 0.5 && sw < pw, "the capsule is narrower than the panel (0.37 of the screen against 0.6)");
    }

    #[test]
    fn a_chip_filters_to_its_category_and_the_panel_is_not_a_click_outside() {
        let mut sh = open_grid();
        let lay = sh.overview.grid.lay(&sh);
        let (chip, (cx, cy, cw, ch)) = lay.chips[0];
        let (x, y) = (cx + cw / 2.0, cy + ch / 2.0);
        assert!(sh.overview_event(Event::Down(x, y), 2.0).is_some());
        assert!(sh.overview_event(Event::Up(x, y), 2.1).is_some());
        assert_eq!(sh.overview.grid.chip, Some(chip));
        let list = sh.overview.grid.arranged();
        assert!(!list.is_empty() && list.iter().all(|&i| chip_of(&crate::shell::apps()[i]) == chip), "only that category's apps remain");
        // an empty spot of the panel keeps it open
        let (px, py, pw, ph) = lay.panel;
        let (ex, ey) = (px + pw - 3.0, py + ph - 3.0);
        assert!(sh.overview_event(Event::Down(ex, ey), 3.0).is_some());
        assert!(sh.overview_event(Event::Up(ex, ey), 3.1).is_some());
        assert!(sh.overview.grid.visible() && !sh.overview.grid.closing(), "a click on the panel does not close the grid");
        // the same chip again shows every app
        assert!(sh.overview_event(Event::Down(x, y), 4.0).is_some());
        assert!(sh.overview_event(Event::Up(x, y), 4.1).is_some());
        assert_eq!(sh.overview.grid.chip, None);
    }

    /// The panel and the capsule are the Search material (a translucent tint over the presenter's blur), not the menus'
    /// nearly opaque flat plate: the wallpaper must show through the panel.
    #[test]
    fn the_panel_is_translucent_glass_not_an_opaque_plate() {
        let mut sh = open_grid();
        let (w, h) = (sh.w as usize, sh.h as usize);
        let mut buf = vec![0u32; w * h];
        let mut grid = std::mem::take(&mut sh.overview.grid);
        let lay = grid.lay(&sh);
        grid.draw(&sh, &mut Surface { w, h, stride: w, px: &mut buf, y0: 0 }, 1e9);
        sh.overview.grid = grid;
        let (px, py, _, ph) = lay.panel;
        // the panel's left padding, level with its middle: no icon, chip or text is there
        let panel = buf[(py + ph / 2.0) as usize * w + (px + 3.0 * lay.pt) as usize] >> 24;
        assert!(panel > 100 && panel < 200, "the panel fill is translucent (alpha {panel}), not the menus' 240");
        let (sx, sy, _, shh) = lay.search;
        let capsule = buf[(sy + shh / 2.0) as usize * w + (sx + 3.0 * lay.pt) as usize] >> 24;
        assert!(capsule > 100 && capsule < 200, "and so is the capsule (alpha {capsule})");
    }

    /// W3-15: a mouse drag on an icon moves it (the others make room) instead of turning the page, the order is
    /// saved, and a drag that starts on empty space still swipes the pages.
    #[test]
    fn a_mouse_drag_on_an_icon_reorders_and_a_drag_on_empty_space_swipes() {
        let mut sh = open_grid();
        sh.form = bifrost_ui::form_factor::FormFactor::Desktop;
        let lay = sh.overview.grid.lay(&sh);
        let before = sh.overview.grid.arranged();
        assert!(before.len() >= 4, "the fixture has a few apps");
        let centre = |k: usize| {
            let (x, y) = lay.cell(k, 0.0);
            (x + lay.cell_w / 2.0, y + lay.icon as f32 / 2.0)
        };
        let ((x0, y0), (x3, y3)) = (centre(0), centre(3));
        assert!(sh.overview_event(Event::Down(x0, y0), 10.0).is_some());
        sh.overview_event(Event::Move(x0 + lay.cell_w / 2.0, y0), 20.0);
        assert!(sh.overview.grid.lifted.is_some(), "a drag on an icon picks it up at once");
        sh.overview_event(Event::Move(x3, y3), 40.0);
        assert_eq!(sh.overview.grid.swipe_dx, 0.0, "the pages do not move under a held icon");
        sh.overview_event(Event::Up(x3, y3), 60.0);
        let after = sh.overview.grid.arranged();
        assert_eq!(after[3], before[0], "the icon takes the slot it was dropped on");
        assert_eq!(&after[..3], &before[1..4], "and the ones it passed close up");
        assert_eq!(sh.overview.grid.page, 0);
        let saved = sh.prefs.get(super::ORDER_PREF).unwrap_or("").to_string();
        assert!(saved.split(',').position(|id| id == super::apps()[before[0]].icon) == Some(3), "the order is saved: {saved}");
        // a drag from the panel's padding, beside the grid, swipes the pages
        let (px, py, _, ph) = lay.panel;
        let (ex, ey) = (px + 3.0 * lay.pt, py + ph / 2.0);
        assert!(sh.overview_event(Event::Down(ex, ey), 100.0).is_some());
        sh.overview_event(Event::Move(ex - 60.0 * lay.pt, ey), 110.0);
        sh.overview_event(Event::Move(ex - 120.0 * lay.pt, ey), 120.0);
        assert!(sh.overview.grid.lifted.is_none(), "no icon is held");
        assert!(sh.overview.grid.swipe_dx < 0.0, "the pages follow the drag");
        sh.overview_event(Event::Up(ex - 120.0 * lay.pt, ey), 140.0);
    }
}
