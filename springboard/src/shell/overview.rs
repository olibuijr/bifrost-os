//! Overview: every window of the current desktop laid out in a non-overlapping grid (Ctrl+Up, F3), a strip of
//! desktops across the top with a "+" to add one, and Ctrl+Left / Ctrl+Right to switch desktops with a slide.
//!
//! The desktop has no real virtual workspaces, so a desktop here is a tag on each window (`Spaces`): the windows
//! of the other desktops are minimized and remembered. While Overview is open the real windows are hidden too
//! (they are separate surfaces under the shell's layer) and their last drawn frames are shown as tiles.
//!
//! Motion follows the shared contract: `bifrost_render::motion::{ease, duration, Curve, Kind}`; the clock is the
//! shell's `t` in milliseconds, the durations are seconds (0 with Reduce Motion, so everything then snaps).

use super::appgrid::AppGrid;
use super::control_center::desk_glass;
use bifrost_ui::material_request::ShellSurface;
use super::desk::Metrics;
use super::ui::{Weight, CARD_BG, FIELD_BG, LABEL, LABEL_2, MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT, SELECTION, TINT, WHITE, WINDOW_FRAME_BLURRED};
use super::window_shots::{self, Source};
use super::{apps, AppId, Effect, Event, Shell};
use crate::gfx::{pack, rgba, Image, Surface, View};
use bifrost_render::motion::{self, Curve, Kind};
use std::collections::{HashMap, HashSet};

pub const KEY_ESC: u16 = 1;
pub const KEY_BACKSPACE: u16 = 14;
pub const KEY_TAB: u16 = 15;
pub const KEY_ENTER: u16 = 28;
pub const KEY_GRAVE: u16 = 41;
pub const KEY_F3: u16 = 61;
pub const KEY_F4: u16 = 62;
pub const KEY_UP: u16 = 103;
pub const KEY_LEFT: u16 = 105;
pub const KEY_RIGHT: u16 = 106;
pub const KEY_DOWN: u16 = 108;

/// The modifier state a key event arrives with.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct KeyMods {
    pub ctrl: bool,
    pub cmd: bool,
    pub shift: bool,
}

/// At most this many desktops.
const MAX_DESKTOPS: usize = 8;
/// Strip thumbnail height, gaps and the window area padding, in pt.
const THUMB_H_PT: f32 = 76.0;
const STRIP_GAP_PT: f32 = 14.0;
const TILE_GAP_PT: f32 = 26.0;
const AREA_PAD_PT: f32 = 28.0;
const DRAG_SLOP_PT: f32 = 8.0;
const DOCK_CLEAR_PT: f32 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl FRect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
    fn lerp(a: FRect, b: FRect, k: f32) -> FRect {
        let l = |p: f32, q: f32| p + (q - p) * k;
        FRect { x: l(a.x, b.x), y: l(a.y, b.y), w: l(a.w, b.w), h: l(a.h, b.h) }
    }
    fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
    fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }
    #[cfg(test)]
    fn overlaps(&self, o: &FRect) -> bool {
        self.x < o.x + o.w - 0.01 && o.x < self.x + self.w - 0.01 && self.y < o.y + o.h - 0.01 && o.y < self.y + self.h - 0.01
    }
}

/// Lays `sizes` (natural window sizes) out in `area` without overlap: the column count that gives the largest
/// total tile area, every tile scaled down (never up) to its cell with its aspect kept, rows centred.
pub(super) fn grid_layout(sizes: &[(f32, f32)], area: FRect, gap: f32) -> Vec<FRect> {
    let n = sizes.len();
    if n == 0 {
        return vec![];
    }
    let fit = |cols: usize| -> (f32, f32, f32, usize) {
        let rows = n.div_ceil(cols);
        let cw = ((area.w - gap * (cols as f32 - 1.0)) / cols as f32).max(1.0);
        let ch = ((area.h - gap * (rows as f32 - 1.0)) / rows as f32).max(1.0);
        let score: f32 = sizes.iter().map(|&(w, h)| {
            let k = (cw / w.max(1.0)).min(ch / h.max(1.0)).min(1.0);
            w * k * h * k
        }).sum();
        (cw, ch, score, rows)
    };
    let mut best = (1usize, f32::MIN);
    for cols in 1..=n {
        let (_, _, score, _) = fit(cols);
        if score > best.1 + 0.5 {
            best = (cols, score);
        }
    }
    let cols = best.0;
    let (cw, ch, _, rows) = fit(cols);
    let mut out = Vec::with_capacity(n);
    let used_h = rows as f32 * ch + gap * (rows as f32 - 1.0);
    let top = area.y + ((area.h - used_h) / 2.0).max(0.0);
    for (i, &(w, h)) in sizes.iter().enumerate() {
        let (row, col) = (i / cols, i % cols);
        let in_row = if row + 1 == rows { n - row * cols } else { cols };
        let row_w = in_row as f32 * cw + gap * (in_row as f32 - 1.0);
        let left = area.x + ((area.w - row_w) / 2.0).max(0.0);
        let k = (cw / w.max(1.0)).min(ch / h.max(1.0)).min(1.0);
        let (tw, th) = (w * k, h * k);
        let (cell_x, cell_y) = (left + col as f32 * (cw + gap), top + row as f32 * (ch + gap));
        out.push(FRect { x: cell_x + (cw - tw) / 2.0, y: cell_y + (ch - th) / 2.0, w: tw, h: th });
    }
    out
}

/// Which desktop each window is on. Windows of other desktops are minimized; `user_min` keeps what the person had
/// minimized so coming back restores exactly that.
#[derive(Default)]
pub(super) struct Spaces {
    count: usize,
    current: usize,
    of: HashMap<AppId, usize>,
    away: HashSet<AppId>,
    user_min: HashMap<AppId, bool>,
}

impl Spaces {
    fn ensure(&mut self) {
        if self.count == 0 {
            self.count = 1;
        }
        self.current = self.current.min(self.count - 1);
    }

    /// Forgets closed windows; a window of another desktop that was brought forward (dock, Search) joins this one.
    fn sync(&mut self, wins: &[super::desk::Win]) {
        self.ensure();
        self.of.retain(|a, _| wins.iter().any(|w| w.app == *a));
        self.away.retain(|a| wins.iter().any(|w| w.app == *a));
        self.user_min.retain(|a, _| self.away.contains(a));
        for w in wins {
            if self.away.contains(&w.app) && !w.minimized {
                self.of.insert(w.app, self.current);
                self.away.remove(&w.app);
                self.user_min.remove(&w.app);
            }
        }
    }

    /// Hides the windows of other desktops and shows the ones of this desktop.
    fn apply(&mut self, wins: &mut [super::desk::Win]) {
        self.ensure();
        for w in wins.iter_mut() {
            let sp = *self.of.entry(w.app).or_insert(self.current);
            if sp == self.current {
                if self.away.remove(&w.app) {
                    w.minimized = self.user_min.remove(&w.app).unwrap_or(false);
                }
            } else {
                if self.away.insert(w.app) {
                    self.user_min.insert(w.app, w.minimized);
                }
                w.minimized = true;
            }
        }
    }

    fn windows_on(&self, i: usize, wins: &[super::desk::Win]) -> Vec<AppId> {
        wins.iter().filter(|w| self.of.get(&w.app).copied().unwrap_or(self.current) == i).map(|w| w.app).collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Tile(usize),
    Desk(usize),
    Add,
    Back,
}

/// What a tile stands for: a window of the shell's desk, or a window of another client that the window server lists
/// (its pixels come from a capture, `window_shots`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum TileKey {
    App(AppId),
    Foreign(u32),
}

struct Tile {
    key: TileKey,
    /// the window's title (windows of other clients; empty for the desk's)
    title: String,
    from: FRect,
    to: FRect,
}

/// The size a window of another client takes in the tile layout before its tile is fitted: the aspect of its picture
/// (16:10 until there is one), at most 60 % of the screen each way. Its real size is not known to the shell.
pub(super) fn foreign_natural(picture: Option<(usize, usize)>, screen: (f32, f32)) -> (f32, f32) {
    let aspect = picture.filter(|p| p.0 > 0 && p.1 > 0).map_or(1.6, |p| p.0 as f32 / p.1 as f32).clamp(0.2, 5.0);
    let (max_w, max_h) = ((screen.0 * 0.6).max(1.0), (screen.1 * 0.6).max(1.0));
    let w = max_w.min(max_h * aspect);
    (w, w / aspect)
}

struct Drag {
    tile: usize,
    x: f32,
    y: f32,
    moved: bool,
}

#[derive(Default)]
pub(super) struct Overview {
    pub(super) open: bool,
    start: f64,
    closing: Option<f64>,
    chosen: Option<AppId>,
    /// a window of another client the person picked: it is activated when Overview has closed
    chosen_foreign: Option<u32>,
    /// every window's minimized flag from before Overview hid them
    saved: Vec<(AppId, bool)>,
    tiles: Vec<Tile>,
    hover: Option<Target>,
    selected: Option<usize>,
    press: Option<(Target, f32, f32)>,
    drag: Option<Drag>,
    pub(super) spaces: Spaces,
    /// a desktop switch in flight: direction (+1 forward) and start time
    slide: Option<(i32, f64)>,
    sources: HashMap<TileKey, Image>,
    /// the wallpaper reduced to a desktop thumbnail of the strip, by thumbnail size and the wallpaper it was made from
    wall: Option<(WallKey, Image)>,
    scaled: HashMap<(TileKey, usize, usize), Image>,
    /// the captured windows of other clients, scaled to their tiles, by window, size and picture serial
    fscaled: HashMap<(u32, usize, usize, u32), Image>,
    /// the windows of other clients the layout was made for, with the size of the picture it used (0 when none yet)
    foreign_sig: Vec<(u32, usize, usize)>,
    size: (i32, i32),
    pub(super) grid: AppGrid,
    /// a press that began on the menu bar or the Dock fell through, so its release must too (the Dock acts on release)
    pass_up: bool,
    /// the window that had the focus when Overview opened: a window of another client that was active, else the desk's
    /// front window. Leaving without choosing a tile gives the focus back to it, not to whatever the window server
    /// finds when the overlay goes
    foreign_before: Option<u32>,
    front_before: Option<AppId>,
    /// whether the last desktop frame had Overview or the Apps grid on screen: a change repaints the frame, so the
    /// overlay surface the grid is exported as appears and goes in step with it
    pub(super) was_visible: bool,
}

/// Who gets the focus when Overview is left without a choice.
#[derive(Debug, PartialEq, Eq)]
enum Focus {
    /// a window of another client, by the window server's id
    Foreign(u32),
    /// a window of the desk
    App(AppId),
    /// nobody in particular: the desk's front window takes it
    Refocus,
}

/// The window that had the focus before Overview opened, when it is still there: `before` is the foreign window that
/// was active and the desk's front window, `foreign` the windows of other clients now, `shown` says whether a desk
/// window is on screen.
fn leave_focus(before: (Option<u32>, Option<AppId>), foreign: &[(u32, String)], shown: impl Fn(AppId) -> bool) -> Focus {
    match before {
        (Some(id), _) if foreign.iter().any(|(w, _)| *w == id) => Focus::Foreign(id),
        (_, Some(app)) if shown(app) => Focus::App(app),
        _ => Focus::Refocus,
    }
}

/// Thumbnail size, wallpaper width, height and address: what a cached desktop thumbnail was made from.
type WallKey = (usize, usize, usize, usize, usize);

/// The whole wallpaper reduced to `w` by `h` px (opaque), each pixel the mean of a 3 by 3 spread of source samples so
/// fine detail does not shimmer. None when there is no wallpaper to reduce.
fn wallpaper_thumb(wp: View, w: usize, h: usize) -> Option<Image> {
    if w == 0 || h == 0 || wp.w == 0 || wp.h == 0 || wp.px.len() < wp.w * wp.h {
        return None;
    }
    const N: usize = 3;
    let at = |t: usize, n: usize, k: usize, src: usize| (((t * N + k) * src) / (n * N)).min(src - 1);
    let mut px = Vec::with_capacity(w * h);
    for ty in 0..h {
        for tx in 0..w {
            let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
            for ky in 0..N {
                let row = at(ty, h, ky, wp.h) * wp.w;
                for kx in 0..N {
                    let p = wp.px[row + at(tx, w, kx, wp.w)];
                    r += p & 0xff;
                    g += (p >> 8) & 0xff;
                    b += (p >> 16) & 0xff;
                }
            }
            let n = (N * N) as u32;
            px.push(pack((r / n) as u8, (g / n) as u8, (b / n) as u8, 255));
        }
    }
    Some(Image { w, h, px })
}

fn dur(kind: Kind) -> f32 {
    motion::duration(kind)
}

/// 0..1 eased progress of an animation that began at `start` (ms).
fn progress(kind: Kind, curve: Curve, start: f64, t: f64) -> f32 {
    let d = dur(kind);
    if d <= 0.0 {
        return 1.0;
    }
    motion::ease(curve, (((t - start) / 1000.0) as f32 / d).clamp(0.0, 1.0))
}

struct Geo {
    w: f32,
    h: f32,
    pt: f32,
    area: FRect,
    thumb: (f32, f32),
    strip_y: f32,
    strip_h: f32,
}

impl Overview {
    /// Forgets the scaled app icons and the grid's, after the icons were drawn again.
    pub(super) fn clear_icons(&mut self) {
        self.scaled.clear();
        self.grid.clear_icons();
    }

    /// Whether the user minimized this window, as opposed to Overview or another desktop hiding it: only those
    /// windows get a dock slot.
    pub(super) fn user_minimized(&self, w: &super::desk::Win) -> bool {
        if self.open {
            return self.saved.iter().find(|(a, _)| *a == w.app).map_or(w.minimized, |(_, m)| *m);
        }
        if self.spaces.away.contains(&w.app) {
            return false;
        }
        w.minimized
    }

    fn geo(&self, sh: &Shell) -> Geo {
        let m: Metrics = sh.desk_metrics();
        let pt = m.pt;
        let (w, h) = (m.w as f32, m.h as f32);
        let thumb_h = THUMB_H_PT * pt;
        let thumb = (thumb_h * w / h.max(1.0), thumb_h);
        let strip_y = m.menubar as f32 + 10.0 * pt;
        let strip_h = thumb_h + 8.0 * pt;
        let top = strip_y + strip_h + 16.0 * pt;
        let bottom = h - if sh.dock_is_visible() { m.dock_bottom as f32 } else { DOCK_CLEAR_PT * pt };
        let pad = AREA_PAD_PT * pt;
        let area = FRect { x: pad, y: top, w: (w - 2.0 * pad).max(1.0), h: (bottom - top - pad / 2.0).max(1.0) };
        Geo { w, h, pt, area, thumb, strip_y, strip_h }
    }

    fn desk_rects(&self, g: &Geo) -> Vec<FRect> {
        let n = self.spaces.count.max(1);
        let gap = STRIP_GAP_PT * g.pt;
        let add_w = g.thumb.1 * 0.6;
        let total = n as f32 * g.thumb.0 + (n as f32) * gap + add_w;
        let x0 = ((g.w - total) / 2.0).max(g.pt * 8.0);
        (0..n).map(|i| FRect { x: x0 + i as f32 * (g.thumb.0 + gap), y: g.strip_y, w: g.thumb.0, h: g.thumb.1 }).collect()
    }

    fn add_rect(&self, g: &Geo) -> FRect {
        let rs = self.desk_rects(g);
        let last = rs.last().copied().unwrap_or(FRect { x: 0.0, y: g.strip_y, w: 0.0, h: g.thumb.1 });
        let add_w = g.thumb.1 * 0.6;
        FRect { x: last.x + last.w + STRIP_GAP_PT * g.pt, y: g.strip_y + (g.thumb.1 - add_w) / 2.0, w: add_w, h: add_w }
    }

    /// Tiles for the windows of the current desktop, in stacking order (back to front).
    fn layout(&mut self, sh: &Shell) {
        let g = self.geo(sh);
        let Some(d) = sh.desk.as_ref() else { return };
        let cur = self.spaces.current;
        let wins: Vec<&super::desk::Win> = d.wins.iter().filter(|w| self.spaces.of.get(&w.app).copied().unwrap_or(cur) == cur).collect();
        // the desk's windows, then the windows of other clients (they belong to every desktop)
        let mut entries: Vec<(TileKey, String, FRect, (f32, f32))> = wins
            .iter()
            .map(|w| {
                let from = FRect { x: w.r.x as f32, y: w.r.y as f32, w: w.r.w as f32, h: w.r.h as f32 };
                (TileKey::App(w.app), String::new(), from, (w.r.w as f32, w.r.h as f32))
            })
            .collect();
        let mut sig = Vec::new();
        for (id, title) in sh.mission_foreign_windows() {
            let picture = sh.shots.shot(Source::Foreign(id)).map(|s| (s.image.w, s.image.h));
            let (nw, nh) = foreign_natural(picture, (sh.w as f32, sh.h as f32));
            sig.push((id, picture.map_or(0, |p| p.0), picture.map_or(0, |p| p.1)));
            let from = FRect { x: (sh.w as f32 - nw) / 2.0, y: (sh.h as f32 - nh) / 2.0, w: nw, h: nh };
            entries.push((TileKey::Foreign(id), title, from, (nw, nh)));
        }
        self.foreign_sig = sig;
        let sizes: Vec<(f32, f32)> = entries.iter().map(|e| e.3).collect();
        let to = grid_layout(&sizes, g.area, TILE_GAP_PT * g.pt);
        self.tiles = entries.into_iter().zip(to).map(|((key, title, from, _), to)| Tile { key, title, from, to }).collect();
        self.size = (sh.w, sh.h);
        self.selected = self.selected.filter(|&i| i < self.tiles.len());
    }

    fn target_at(&self, g: &Geo, x: f32, y: f32) -> Target {
        if self.add_rect(g).contains(x, y) && self.spaces.count < MAX_DESKTOPS {
            return Target::Add;
        }
        if let Some(i) = self.desk_rects(g).iter().position(|r| r.contains(x, y)) {
            return Target::Desk(i);
        }
        // the front-most tile wins
        if let Some(i) = self.tiles.iter().rposition(|t| t.to.contains(x, y)) {
            return Target::Tile(i);
        }
        Target::Back
    }

    pub(super) fn visible(&self) -> bool {
        self.open || self.slide.is_some() || self.grid.visible()
    }

    /// The overlay surface Overview is exported as on the window server (x, y, w, h px): the veil between the menu
    /// bar and the Dock, above every window. Listed only while Overview is open, which includes its enter and leave
    /// animations (`open` turns false in `finish`, the frame the windows come back), so closing removes the surface.
    #[allow(dead_code)] // the window-server export is the desktop-wayland build's
    pub(super) fn overlay_rect(&self, sh: &Shell) -> Option<(usize, usize, usize, usize)> {
        if self.open {
            sh.overview_veil_rect()
        } else {
            None
        }
    }

    /// The overlay surface of a desktop switch's sweep (same rectangle): present only while the sweep runs and Mission
    /// Control is not open (it then has its own surface). It takes no pointer and no keys.
    #[allow(dead_code)] // the window-server export is the desktop-wayland build's
    pub(super) fn slide_overlay_rect(&self, sh: &Shell) -> Option<(usize, usize, usize, usize)> {
        if !self.open && self.slide.is_some() {
            sh.overview_veil_rect()
        } else {
            None
        }
    }

    /// Tiles in the current layout: (app, final rectangle), back to front.
    #[cfg(test)]
    fn tile_rects(&self) -> Vec<(AppId, FRect)> {
        self.tiles.iter().filter_map(|t| match t.key { TileKey::App(app) => Some((app, t.to)), TileKey::Foreign(_) => None }).collect()
    }

    fn prog(&self, t: f64) -> f32 {
        match self.closing {
            Some(c) => 1.0 - progress(Kind::OverviewEnter, Curve::EaseInOut, c, t),
            None => progress(Kind::OverviewEnter, Curve::EaseOut, self.start, t),
        }
    }

    fn open_now(&mut self, sh: &mut Shell, t: f64) {
        if self.open {
            return;
        }
        let foreign = sh.foreign_front_window();
        let Some(d) = sh.desk.as_mut() else { return };
        self.spaces.sync(&d.wins);
        self.foreign_before = foreign;
        self.front_before = d.front().map(|w| w.app);
        self.saved = d.wins.iter().map(|w| (w.app, w.minimized)).collect();
        for w in d.wins.iter_mut() {
            w.minimized = true;
        }
        self.open = true;
        // the wallpaper may have moved on (a live look) since the thumbnail was made
        self.wall = None;
        self.start = t;
        self.closing = None;
        self.chosen = None;
        self.chosen_foreign = None;
        self.hover = None;
        self.press = None;
        self.drag = None;
        self.selected = None;
        self.sources.clear();
        self.scaled.clear();
        if let Some(d) = sh.desk.as_ref() {
            for w in &d.wins {
                if let Some((pw, ph, px)) = d.window_pixels(w.app) {
                    if pw > 0 && ph > 0 && px.len() >= pw * ph {
                        self.sources.insert(TileKey::App(w.app), Image { w: pw, h: ph, px: px[..pw * ph].to_vec() });
                    }
                }
            }
        }
        self.layout(sh);
        self.request_shots(sh, t, 0.0);
        sh.dirty = true;
    }

    /// Asks the window server for a fresh picture of every window of another client that has a tile, at most one request
    /// per window every `gap_ms` (a window that did not change costs nothing: it answers `unchanged`).
    fn request_shots(&self, sh: &mut Shell, t: f64, gap_ms: f64) {
        for tile in &self.tiles {
            if let TileKey::Foreign(id) = tile.key {
                sh.shots.request(Source::Foreign(id), window_shots::bound_for_tile(tile.to.w, tile.to.h), t, gap_ms);
            }
        }
    }

    /// Whether any window of another client has a tile.
    fn has_foreign_tiles(&self) -> bool {
        self.tiles.iter().any(|t| matches!(t.key, TileKey::Foreign(_)))
    }

    /// Leaves Overview, focusing `chosen` when given. The windows come back when the animation has finished.
    fn close(&mut self, sh: &mut Shell, chosen: Option<AppId>, t: f64) {
        if !self.open || self.closing.is_some() {
            return;
        }
        self.chosen = chosen;
        self.drag = None;
        self.press = None;
        if dur(Kind::OverviewEnter) <= 0.0 {
            self.finish(sh, t);
        } else {
            self.closing = Some(t);
        }
        sh.dirty = true;
    }

    fn finish(&mut self, sh: &mut Shell, t: f64) {
        self.open = false;
        self.closing = None;
        if let Some(d) = sh.desk.as_mut() {
            for (app, min) in std::mem::take(&mut self.saved) {
                if let Some(w) = d.wins.iter_mut().find(|w| w.app == app) {
                    w.minimized = min;
                }
            }
            self.spaces.apply(&mut d.wins);
        }
        let before = (self.foreign_before.take(), self.front_before.take());
        match (self.chosen_foreign.take(), self.chosen.take()) {
            (Some(id), _) => sh.desk_activate_foreign(id),
            (None, Some(app)) => sh.desk_open(app, t),
            (None, None) => match leave_focus(before, &sh.mission_foreign_windows(), |app| sh.desk.as_ref().is_some_and(|d| d.index_of(app).is_some_and(|i| !d.wins[i].minimized))) {
                Focus::Foreign(id) => sh.desk_activate_foreign(id),
                Focus::App(app) => sh.desk_open(app, t),
                Focus::Refocus => sh.desk_refocus(),
            },
        }
        self.sources.clear();
        self.scaled.clear();
        self.fscaled.clear();
        sh.dirty = true;
    }

    fn add_desktop(&mut self) -> Option<usize> {
        self.spaces.ensure();
        if self.spaces.count >= MAX_DESKTOPS {
            return None;
        }
        self.spaces.count += 1;
        Some(self.spaces.count - 1)
    }

    /// Makes desktop `target` the current one. Inside Overview only the tiles change; outside, windows swap and
    /// the slide plays.
    fn go_to(&mut self, sh: &mut Shell, target: usize, t: f64) {
        self.spaces.ensure();
        let target = target.min(self.spaces.count - 1);
        let from = self.spaces.current;
        if target == from {
            return;
        }
        if let Some(d) = sh.desk.as_ref() {
            self.spaces.sync(&d.wins);
        }
        self.spaces.current = target;
        if self.open {
            self.selected = None;
            self.layout(sh);
        } else {
            if let Some(d) = sh.desk.as_mut() {
                self.spaces.apply(&mut d.wins);
            }
            self.slide = Some((if target > from { 1 } else { -1 }, t));
            sh.desk_refocus();
        }
        sh.dirty = true;
    }

    fn move_window(&mut self, sh: &mut Shell, app: AppId, desk: usize) {
        self.spaces.of.insert(app, desk);
        self.layout(sh);
        sh.dirty = true;
    }

    fn navigate(&mut self, dx: f32, dy: f32) {
        if self.tiles.is_empty() {
            return;
        }
        let Some(cur) = self.selected else {
            self.selected = Some(self.tiles.len() - 1);
            return;
        };
        let c = self.tiles[cur].to;
        let mut best: Option<(usize, f32)> = None;
        for (i, t) in self.tiles.iter().enumerate() {
            if i == cur {
                continue;
            }
            let (ox, oy) = (t.to.cx() - c.cx(), t.to.cy() - c.cy());
            let along = ox * dx + oy * dy;
            if along <= 1.0 {
                continue;
            }
            let across = (ox * dy).abs() + (oy * dx).abs();
            let score = along + 2.0 * across;
            if best.map_or(true, |b| score < b.1) {
                best = Some((i, score));
            }
        }
        if let Some((i, _)) = best {
            self.selected = Some(i);
        }
    }

    fn key(&mut self, sh: &mut Shell, code: u16, mods: KeyMods, t: f64) -> bool {
        if self.closing.is_some() {
            return true;
        }
        match code {
            KEY_ESC => self.close(sh, None, t),
            KEY_ENTER => {
                self.pick(sh, self.selected, t);
            }
            KEY_TAB => {
                if !self.tiles.is_empty() {
                    let n = self.tiles.len();
                    self.selected = Some(match self.selected {
                        None => 0,
                        Some(i) if mods.shift => (i + n - 1) % n,
                        Some(i) => (i + 1) % n,
                    });
                }
            }
            KEY_LEFT => self.navigate(-1.0, 0.0),
            KEY_RIGHT => self.navigate(1.0, 0.0),
            KEY_UP => self.navigate(0.0, -1.0),
            KEY_DOWN => self.navigate(0.0, 1.0),
            _ => return false,
        }
        sh.dirty = true;
        true
    }

    /// Leaves Overview for tile `i` (none: for no window): a desk window is opened, a window of another client is
    /// activated.
    fn pick(&mut self, sh: &mut Shell, i: Option<usize>, t: f64) {
        match i.and_then(|i| self.tiles.get(i)).map(|tile| tile.key) {
            Some(TileKey::App(app)) => self.close(sh, Some(app), t),
            Some(TileKey::Foreign(id)) => {
                self.chosen_foreign = Some(id);
                self.close(sh, None, t);
            }
            None => self.close(sh, None, t),
        }
    }

    fn event(&mut self, sh: &mut Shell, ev: Event, t: f64) -> Option<Vec<Effect>> {
        if self.closing.is_some() {
            return Some(vec![]);
        }
        let g = self.geo(sh);
        match ev {
            Event::Down(x, y) => {
                let tg = self.target_at(&g, x, y);
                self.press = Some((tg, x, y));
                self.drag = None;
            }
            Event::Move(x, y) => {
                let tg = self.target_at(&g, x, y);
                self.hover = Some(tg);
                if let Some((Target::Tile(i), x0, y0)) = self.press {
                    let moved = self.drag.as_ref().is_some_and(|d| d.moved) || (x - x0).abs() + (y - y0).abs() > DRAG_SLOP_PT * g.pt;
                    self.drag = Some(Drag { tile: i, x, y, moved });
                }
            }
            Event::Up(x, y) => {
                let drag = self.drag.take();
                let press = self.press.take();
                let tg = self.target_at(&g, x, y);
                if let Some(d) = drag.filter(|d| d.moved) {
                    // only the desk's own windows move between desktops
                    let app = self.tiles.get(d.tile).and_then(|t| match t.key { TileKey::App(app) => Some(app), TileKey::Foreign(_) => None });
                    if let Some(app) = app {
                        match tg {
                            Target::Desk(i) => self.move_window(sh, app, i),
                            Target::Add => {
                                if let Some(i) = self.add_desktop() {
                                    self.move_window(sh, app, i);
                                }
                            }
                            _ => {}
                        }
                    }
                } else if let Some((pt, _, _)) = press {
                    if pt == tg {
                        match tg {
                            Target::Tile(i) => self.pick(sh, Some(i), t),
                            Target::Desk(i) => self.go_to(sh, i, t),
                            Target::Add => {
                                if let Some(i) = self.add_desktop() {
                                    self.go_to(sh, i, t);
                                }
                            }
                            Target::Back => self.close(sh, None, t),
                        }
                    }
                }
            }
            _ => return None,
        }
        sh.dirty = true;
        Some(vec![])
    }

    /// Scaled tile image for `app` at `w`x`h`, cached; None when the window was never drawn through the cache.
    fn thumb(&mut self, app: TileKey, w: usize, h: usize) -> Option<&Image> {
        if w == 0 || h == 0 {
            return None;
        }
        if !self.scaled.contains_key(&(app, w, h)) {
            let img = self.sources.get(&app)?.resized(w, h);
            // keep the cache small: a resize animation makes many sizes
            if self.scaled.len() > 24 {
                self.scaled.clear();
            }
            self.scaled.insert((app, w, h), img);
        }
        self.scaled.get(&(app, w, h))
    }

    /// Makes the desktop thumbnail of the strip, `thumb` px, from the wallpaper the desktop shows now; kept until the
    /// size or the wallpaper changes.
    fn ensure_wall(&mut self, sh: &Shell, thumb: (f32, f32)) {
        let (w, h) = (thumb.0.round().max(1.0) as usize, thumb.1.round().max(1.0) as usize);
        let wp = sh.wallpaper_frame();
        let key = (w, h, wp.w, wp.h, wp.px.as_ptr() as usize);
        if self.wall.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        self.wall = wallpaper_thumb(wp, w, h).map(|img| (key, img));
    }

    /// Draws everything; true while something animates (the caller keeps the frames coming).
    fn draw(&mut self, sh: &Shell, s: &mut Surface, t: f64) -> bool {
        let mut animating = false;
        if self.open {
            animating |= self.draw_overview(sh, s, t);
        }
        if let Some((dir, start)) = self.slide {
            animating |= draw_slide(sh, s, dir, start, self.spaces.current, t);
        }
        animating |= self.grid.draw(sh, s, t);
        animating
    }

    fn draw_overview(&mut self, sh: &Shell, s: &mut Surface, t: f64) -> bool {
        let p = self.prog(t);
        let g = self.geo(sh);
        let ui = &sh.ui;
        let m = sh.desk_metrics();
        // the veil: the presenter's blur under a tint, below the menu bar and above the dock
        let veil_h = g.h - m.menubar as f32 - if sh.dock_is_visible() { m.dock_bottom as f32 } else { 0.0 };
        desk_glass(s, ShellSurface::OverviewVeil, (0.0, m.menubar as f32, g.w, veil_h.max(1.0)), 0.0, (215.0 * p) as u32);
        s.fill_rrect(0.0, m.menubar as f32, g.w, veil_h.max(1.0), 0.0, rgba(0, 0, 0, (60.0 * p) as u8));
        // the desktops strip slides down from the menu bar
        let slide_y = -(1.0 - p) * (g.strip_h + 10.0 * g.pt);
        let alpha = (255.0 * p) as u32;
        let cur = self.spaces.current;
        let wins: Vec<super::desk::Win> = sh.desk.as_ref().map(|d| d.wins.clone()).unwrap_or_default();
        let rects = self.desk_rects(&g);
        let drop_desk = match (&self.drag, self.hover) {
            (Some(d), Some(Target::Desk(i))) if d.moved => Some(i),
            _ => None,
        };
        self.ensure_wall(sh, g.thumb);
        for (i, r) in rects.iter().enumerate() {
            let y = r.y + slide_y;
            let rad = ui.pt(8.0);
            // the desktop's picture: the wallpaper with each window's frame on it
            match &self.wall {
                Some((_, img)) => super::switcher::draw_scaled(s, img, crate::layout::Rect { x: r.x.round() as i32, y: y.round() as i32, w: img.w as i32, h: img.h as i32 }, rad, alpha),
                None => s.fill_rrect(r.x, y, r.w, r.h, rad, crate::gfx::scale(CARD_BG.now(), alpha)),
            }
            for a in self.spaces.windows_on(i, &wins) {
                if let Some(w) = wins.iter().find(|w| w.app == a) {
                    let k = r.w / g.w.max(1.0);
                    let wr = FRect { x: r.x + w.r.x as f32 * k, y: y + w.r.y as f32 * k, w: (w.r.w as f32 * k).max(2.0), h: (w.r.h as f32 * k).max(2.0) };
                    s.fill_rrect(wr.x, wr.y, wr.w.min(r.x + r.w - wr.x).max(1.0), wr.h.min(y + r.h - wr.y).max(1.0), ui.pt(1.5), crate::gfx::scale(WINDOW_FRAME_BLURRED.now(), alpha));
                }
            }
            if i == cur {
                s.stroke_rrect(r.x - 1.0, y - 1.0, r.w + 2.0, r.h + 2.0, rad + 1.0, ui.pt(2.0), crate::gfx::scale(TINT, alpha));
            } else if drop_desk == Some(i) {
                s.stroke_rrect(r.x - 1.0, y - 1.0, r.w + 2.0, r.h + 2.0, rad + 1.0, ui.pt(2.0), crate::gfx::scale(SELECTION.now(), alpha));
            }
            let name = format!("Desktop {}", i + 1);
            sh.label(&name, MAC_FOOTNOTE_PT, Weight::Semibold, WHITE, |im| {
                s.blit_colored(im, (r.x + (r.w - im.w as f32) / 2.0) as i32, (y + r.h - im.h as f32 - ui.pt(5.0)) as i32, LABEL_2.now(), alpha);
            });
        }
        if self.spaces.count < MAX_DESKTOPS {
            let a = self.add_rect(&g);
            let y = a.y + slide_y;
            s.fill_rrect(a.x, y, a.w, a.h, a.w / 2.0, crate::gfx::scale(FIELD_BG.now(), alpha));
            let (arm, th) = (a.w * 0.34, ui.pt(2.0));
            let c = crate::gfx::scale(LABEL.now(), alpha);
            s.fill_rrect(a.x + a.w / 2.0 - arm, y + a.h / 2.0 - th / 2.0, 2.0 * arm, th, th / 2.0, c);
            s.fill_rrect(a.x + a.w / 2.0 - th / 2.0, y + a.h / 2.0 - arm, th, 2.0 * arm, th / 2.0, c);
        }
        // tiles
        let hover_tile = match self.hover {
            Some(Target::Tile(i)) => Some(i),
            _ => None,
        };
        let dragging = self.drag.as_ref().filter(|d| d.moved).map(|d| d.tile);
        let n = self.tiles.len();
        let mut title: Option<(usize, FRect)> = None;
        for i in 0..n {
            if dragging == Some(i) {
                continue;
            }
            let (app, from, to) = (self.tiles[i].key, self.tiles[i].from, self.tiles[i].to);
            let r = FRect::lerp(from, to, p);
            self.draw_tile(sh, s, app, r, to.w / from.w.max(1.0), alpha, hover_tile == Some(i), self.selected == Some(i));
            if hover_tile == Some(i) || self.selected == Some(i) {
                title = Some((i, r));
            }
        }
        if let Some((tile, dx, dy)) = self.drag.as_ref().filter(|d| d.moved && d.tile < n).map(|d| (d.tile, d.x, d.y)) {
            let (app, to, from) = (self.tiles[tile].key, self.tiles[tile].to, self.tiles[tile].from);
            let k = 0.6;
            let r = FRect { x: dx - to.w * k / 2.0, y: dy - to.h * k / 2.0, w: to.w * k, h: to.h * k };
            self.draw_tile(sh, s, app, r, k * to.w / from.w.max(1.0), 220, true, false);
        }
        // the title shows under the tile the pointer (or the keyboard) is on
        if let Some((i, r)) = title {
            if p >= 1.0 && dragging.is_none() {
                let name: &str = match self.tiles[i].key {
                    TileKey::App(app) => apps().iter().find(|a| a.id == app).map_or("", |a| a.name),
                    TileKey::Foreign(_) => self.tiles[i].title.as_str(),
                };
                let (tw, th) = sh.label(name, MAC_SUBHEAD_PT, Weight::Semibold, WHITE, |im| (im.w as f32, im.h as f32));
                let (pw, ph) = (tw + ui.pt(20.0), th + ui.pt(8.0));
                let (px, py) = (r.cx() - pw / 2.0, r.y + r.h + ui.pt(8.0));
                desk_glass(s, ShellSurface::OverviewPill, (px, py, pw, ph), ph / 2.0, 255);
                sh.label(name, MAC_SUBHEAD_PT, Weight::Semibold, WHITE, |im| s.blit_colored(im, (px + (pw - im.w as f32) / 2.0) as i32, (py + (ph - im.h as f32) / 2.0) as i32, LABEL.now(), 255));
            }
        }
        p < 1.0 && self.closing.is_none() || self.closing.is_some()
    }

    /// The captured picture of window `id` of another client in its tile `r` (scaled once per picture), or, while there is
    /// none (the window server cannot capture, or the first answer is on its way), a plate with the window's title.
    #[allow(clippy::too_many_arguments)]
    fn draw_foreign_picture(&mut self, sh: &Shell, s: &mut Surface, id: u32, r: FRect, rad: f32, (iw, ih): (usize, usize), alpha: u32) {
        if let Some(shot) = sh.shots.shot(Source::Foreign(id)).filter(|shot| shot.image.w > 0 && shot.image.h > 0) {
            let ck = (id, iw, ih, shot.serial);
            if !self.fscaled.contains_key(&ck) {
                // keep the cache small: a resize animation makes many sizes
                if self.fscaled.len() > 24 {
                    self.fscaled.clear();
                }
                self.fscaled.insert(ck, shot.image.resized(iw, ih));
            }
            if let Some(img) = self.fscaled.get(&ck) {
                s.blit(img, r.x.round() as i32, r.y.round() as i32, 255, alpha);
                return;
            }
        }
        s.fill_rrect(r.x, r.y, r.w, r.h, rad, crate::gfx::scale(CARD_BG.now(), alpha));
        let title = self.tiles.iter().find(|t| t.key == TileKey::Foreign(id)).map(|t| t.title.clone()).unwrap_or_default();
        if !title.is_empty() {
            sh.label(&title, MAC_SUBHEAD_PT, Weight::Semibold, WHITE, |im| {
                s.blit_colored(im, (r.x + (r.w - im.w as f32) / 2.0) as i32, (r.y + (r.h - im.h as f32) / 2.0) as i32, LABEL_2.now(), alpha);
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_tile(&mut self, sh: &Shell, s: &mut Surface, key: TileKey, r: FRect, scale_k: f32, alpha: u32, hover: bool, selected: bool) {
        let ui = &sh.ui;
        let rad = (ui.pt(bifrost_ui::prefs_layout::m::WINDOW_RADIUS) * scale_k.clamp(0.0, 1.0)).max(ui.pt(4.0));
        // soft shadow
        for (k, a) in [(3.0, 14u8), (1.5, 20u8)] {
            let gx = ui.pt(k * 2.0);
            s.fill_rrect(r.x - gx, r.y - gx + ui.pt(4.0), r.w + 2.0 * gx, r.h + 2.0 * gx, rad + gx, rgba(0, 0, 0, ((a as u32 * alpha) / 255) as u8));
        }
        let (iw, ih) = (r.w.round().max(1.0) as usize, r.h.round().max(1.0) as usize);
        let app = match key {
            TileKey::App(app) => app,
            TileKey::Foreign(id) => {
                self.draw_foreign_picture(sh, s, id, r, rad, (iw, ih), alpha);
                if hover || selected {
                    let c = if selected { TINT } else { SELECTION.now() };
                    s.stroke_rrect(r.x - 2.0, r.y - 2.0, r.w + 4.0, r.h + 4.0, rad + 2.0, ui.pt(3.0), crate::gfx::scale(c, alpha));
                }
                return;
            }
        };
        match self.thumb(key, iw, ih) {
            Some(img) => s.blit(img, r.x.round() as i32, r.y.round() as i32, 255, alpha),
            None => {
                s.fill_rrect(r.x, r.y, r.w, r.h, rad, crate::gfx::scale(CARD_BG.now(), alpha));
                // no frame to show: the app icon on a plate
                if let Some(i) = apps().iter().position(|a| a.id == app) {
                    if let Some(base) = sh.base_icons.get(i) {
                        let side = (r.w.min(r.h) * 0.4).max(8.0) as usize;
                        let img = base.resized(side, side);
                        s.blit(&img, (r.x + (r.w - side as f32) / 2.0) as i32, (r.y + (r.h - side as f32) / 2.0) as i32, 255, alpha);
                    }
                }
            }
        }
        if hover || selected {
            let c = if selected { TINT } else { SELECTION.now() };
            s.stroke_rrect(r.x - 2.0, r.y - 2.0, r.w + 4.0, r.h + 4.0, rad + 2.0, ui.pt(3.0), crate::gfx::scale(c, alpha));
        }
    }
}

/// The desktop switch: a veil sweeps across in the direction of travel and the new desktop's name shows.
/// Reduce Motion makes the duration 0, so nothing is drawn.
fn draw_slide(sh: &Shell, s: &mut Surface, dir: i32, start: f64, current: usize, t: f64) -> bool {
    let d = dur(Kind::PanelReveal);
    if d <= 0.0 {
        return false;
    }
    let k = (((t - start) / 1000.0) as f32 / d).clamp(0.0, 1.0);
    if k >= 1.0 {
        return false;
    }
    let e = motion::ease(Curve::EaseInOut, k);
    let m = sh.desk_metrics();
    let (w, top) = (m.w as f32, m.menubar as f32);
    let h = m.h as f32 - top - if sh.dock_is_visible() { m.dock_bottom as f32 } else { 0.0 };
    // the veil enters from the far side and leaves on the near one
    let x = -(dir as f32) * w * (1.0 - 2.0 * e);
    let fade = (1.0 - (2.0 * e - 1.0).abs()).clamp(0.0, 1.0);
    desk_glass(s, ShellSurface::OverviewVeil, (x.max(0.0), top, (w - x.abs()).max(1.0), h.max(1.0)), 0.0, (200.0 * fade) as u32);
    let name = format!("Desktop {}", current + 1);
    let a = (255.0 * fade) as u32;
    sh.label(&name, super::ui::MAC_TITLE2_PT, Weight::Semibold, WHITE, |im| {
        s.blit_colored(im, ((w - im.w as f32) / 2.0) as i32, (top + (h - im.h as f32) / 2.0) as i32, LABEL.now(), a);
    });
    true
}

impl Shell {
    /// The veil's rectangle (x, y, w, h px): the full width between the menu bar and the Dock. The menu bar and the Dock stay
    /// their own surfaces, above or beside it, and stay usable. None when the output has no room for it.
    pub(super) fn overview_veil_rect(&self) -> Option<(usize, usize, usize, usize)> {
        let m = self.desk_metrics();
        let top = m.menubar.max(0);
        let bottom = m.h - if self.dock_is_visible() { m.dock_bottom } else { 0 };
        (m.w > 0 && bottom > top).then(|| (0, top as usize, m.w as usize, (bottom - top) as usize))
    }

    /// The pointer moved over the veil without a button while Overview is open: the tile under it is highlighted. True
    /// when the point is on the veil (the surface under it must not see the pointer); the menu bar and the Dock are not.
    pub fn overview_hover(&mut self, x: f32, y: f32) -> bool {
        if !self.desk_active() || self.is_locked() || !self.overview.open || self.overview.closing.is_some() {
            return false;
        }
        let Some((vx, vy, vw, vh)) = self.overview_veil_rect() else { return false };
        if x < vx as f32 || x >= (vx + vw) as f32 || y < vy as f32 || y >= (vy + vh) as f32 {
            return false;
        }
        let tg = self.overview.target_at(&self.overview.geo(self), x, y);
        if self.overview.hover != Some(tg) {
            self.overview.hover = Some(tg);
            self.dirty = true;
        }
        true
    }

    /// True while Overview, the Apps grid or a desktop switch is on screen.
    pub(super) fn overview_active(&self) -> bool {
        self.desk_active() && self.overview.visible()
    }

    /// The content-layer regions of a GPU desktop frame: the whole screen while Overview or the Apps grid shows.
    pub(super) fn overview_regions(&self, regions: Option<Vec<(i32, i32, i32, i32)>>) -> Option<Vec<(i32, i32, i32, i32)>> {
        if self.overview_active() && bifrost_render::glass::gpu() {
            Some(vec![(0, 0, self.w, self.h)])
        } else {
            regions
        }
    }

    /// Overview is open: asks for fresh pictures of the windows of other clients, at most 5 times a second each.
    pub(super) fn overview_refresh_shots(&mut self, t: f64) {
        if !self.overview.open || self.overview.closing.is_some() || !self.shots.available() {
            return;
        }
        let ov = std::mem::take(&mut self.overview);
        ov.request_shots(self, t, window_shots::MISSION_REFRESH_MS);
        self.overview = ov;
    }

    /// Whether Overview is open with a tile for a window of another client (it then wakes to refresh the tiles).
    pub(super) fn overview_wants_shots(&self) -> bool {
        self.overview.open && self.overview.closing.is_none() && self.shots.available() && self.overview.has_foreign_tiles()
    }

    /// A picture arrived: when the set of windows or a picture's shape changed since the tiles were laid out, lays them out
    /// again (the first picture of a window gives its tile the window's real aspect). Otherwise the tile is just redrawn.
    pub(super) fn overview_shots_changed(&mut self) {
        if !self.overview.open || self.overview.closing.is_some() {
            return;
        }
        let now: Vec<(u32, usize, usize)> = self
            .mission_foreign_windows()
            .into_iter()
            .map(|(id, _)| {
                let pic = self.shots.shot(Source::Foreign(id)).map(|s| (s.image.w, s.image.h));
                (id, pic.map_or(0, |p| p.0), pic.map_or(0, |p| p.1))
            })
            .collect();
        if now != self.overview.foreign_sig {
            let mut ov = std::mem::take(&mut self.overview);
            ov.layout(self);
            self.overview = ov;
        }
        self.dirty = true;
    }

    /// Pointer events while Overview or the Apps grid is open; `None` when it is not.
    pub(super) fn overview_event(&mut self, ev: Event, t: f64) -> Option<Vec<Effect>> {
        if !self.desk_active() || self.is_locked() || !(self.overview.open || self.overview.grid.visible()) {
            return None;
        }
        // the menu bar and the dock stay usable while Overview is open: their presses fall through, and so does the
        // release that ends one (the Dock acts on release: the Apps icon closes the grid with it, E2E L3)
        match ev {
            Event::Down(x, y) => {
                let d = self.dock_overlay_rect();
                let in_dock = x >= d.x as f32 && x < (d.x + d.w) as f32 && y >= d.y as f32 && y < (d.y + d.h) as f32;
                let through = y < self.desktop_bar_height() as f32 || in_dock;
                self.overview.pass_up = through;
                if through {
                    return None;
                }
            }
            Event::Up(..) if std::mem::take(&mut self.overview.pass_up) => return None,
            _ => {}
        }
        let mut ov = std::mem::take(&mut self.overview);
        let r = if ov.grid.visible() { ov.grid.event(self, ev, t) } else { ov.event(self, ev, t) };
        self.overview = ov;
        self.dirty = true;
        r
    }

    /// Draws the overlays on top of the desktop; finishes a close animation that has run out.
    pub(super) fn draw_overview(&mut self, s: &mut Surface, t: f64) {
        if !self.desk_active() || !self.overview.visible() {
            return;
        }
        let mut ov = std::mem::take(&mut self.overview);
        if let Some(c) = ov.closing {
            if progress(Kind::OverviewEnter, Curve::EaseInOut, c, t) >= 1.0 {
                ov.finish(self, t);
            }
        }
        if ov.slide.is_some_and(|(_, st)| dur(Kind::PanelReveal) <= 0.0 || (t - st) / 1000.0 >= dur(Kind::PanelReveal) as f64) {
            ov.slide = None;
        }
        let animating = ov.draw(self, s, t);
        self.overview = ov;
        if animating {
            self.dirty = true;
        }
    }

    pub fn overview_toggle(&mut self, t: f64) {
        if !self.desk_active() || self.is_locked() {
            return;
        }
        let mut ov = std::mem::take(&mut self.overview);
        if ov.grid.visible() {
            ov.grid.close(self, t);
        }
        if ov.open {
            ov.close(self, None, t);
        } else {
            self.cc.hide();
            ov.open_now(self, t);
        }
        self.overview = ov;
        self.dirty = true;
    }

    pub fn appgrid_toggle(&mut self, t: f64) {
        if !self.desk_active() || self.is_locked() {
            return;
        }
        let mut ov = std::mem::take(&mut self.overview);
        if ov.open {
            ov.close(self, None, t);
        }
        // a grid that is still animating out opens again (a close that has been asked for once cannot be asked for twice,
        // so the Dock's Apps icon would do nothing during it)
        if ov.grid.visible() && !ov.grid.closing() {
            ov.grid.close(self, t);
        } else {
            ov.grid.open(self, t);
        }
        self.overview = ov;
        self.dirty = true;
    }

    /// Opens the launcher when it is closed (the game controller's Home button); unlike `appgrid_toggle` it never closes it.
    pub fn appgrid_show(&mut self, t: f64) {
        if !self.overview.grid.visible() {
            self.appgrid_toggle(t);
        }
    }

    /// Ctrl+Left / Ctrl+Right: the neighbouring desktop, with a slide.
    pub fn switch_desktop(&mut self, delta: i32, t: f64) {
        if !self.desk_active() || self.is_locked() {
            return;
        }
        let mut ov = std::mem::take(&mut self.overview);
        ov.spaces.ensure();
        let target = (ov.spaces.current as i32 + delta).clamp(0, ov.spaces.count as i32 - 1) as usize;
        ov.go_to(self, target, t);
        self.overview = ov;
    }

    /// Hardware keys for Overview, the Apps grid and desktop switching; true when consumed. Call it for key
    /// presses before the other shortcut handling. Cmd+Tab and Cmd+` are in `switcher.rs` (`switcher_key`).
    pub fn overview_key(&mut self, code: u16, down: bool, mods: KeyMods, t: f64) -> bool {
        if !self.desk_active() || self.is_locked() || !down {
            return false;
        }
        match code {
            KEY_F3 => {
                self.overview_toggle(t);
                return true;
            }
            KEY_UP if mods.ctrl && !self.overview.grid.visible() => {
                self.overview_toggle(t);
                return true;
            }
            KEY_F4 => {
                self.appgrid_toggle(t);
                return true;
            }
            KEY_LEFT | KEY_RIGHT if mods.ctrl => {
                self.switch_desktop(if code == KEY_LEFT { -1 } else { 1 }, t);
                return true;
            }
            _ => {}
        }
        if !(self.overview.open || self.overview.grid.visible()) {
            return false;
        }
        let mut ov = std::mem::take(&mut self.overview);
        let used = if ov.grid.visible() { ov.grid.key(self, code, mods, t) } else { ov.key(self, code, mods, t) };
        self.overview = ov;
        self.dirty = true;
        // a modal layer swallows the rest
        let _ = used;
        true
    }

    /// Esc while Overview or the Apps grid is open closes it, before the key reaches anything under it; true when taken.
    pub(super) fn overview_escape(&mut self, t: f64) -> bool {
        if !(self.overview.open || self.overview.grid.visible()) {
            return false;
        }
        self.overview_key(KEY_ESC, true, KeyMods::default(), t)
    }

    /// Typing while the Apps grid is open: text goes to its search field, Backspace, Return, Esc and the arrows are its
    /// keys, and nothing else gets through (the grid is modal, and its overlay surface holds the keyboard); true when
    /// the grid took it.
    pub(super) fn overview_typed(&mut self, out: &super::keyboard::Out, t: f64) -> bool {
        use super::keyboard::Out;
        if !self.desk_active() || self.is_locked() || !self.overview.grid.visible() {
            return false;
        }
        let code = match out.clone() {
            Out::Text(text) => {
                self.overview_type(&text);
                return true;
            }
            Out::Backspace => KEY_BACKSPACE,
            Out::Enter => KEY_ENTER,
            Out::Named("Escape") => KEY_ESC,
            Out::Named("Up") => KEY_UP,
            Out::Named("Down") => KEY_DOWN,
            Out::Named("Left") => KEY_LEFT,
            Out::Named("Right") => KEY_RIGHT,
            _ => return true,
        };
        self.overview_key(code, true, KeyMods::default(), t);
        true
    }

    /// Typed text while the Apps grid is open goes to its search field; true when it was taken.
    pub fn overview_type(&mut self, text: &str) -> bool {
        if !self.desk_active() || !self.overview.grid.visible() {
            return false;
        }
        self.overview.grid.type_text(text);
        self.dirty = true;
        true
    }

    /// Wheel or two-finger scroll while the Apps grid is open pages through it (`dx`, `dy` positive: the next page, so the
    /// wheel turned down or a swipe to the left); while Overview is open the wheel is swallowed (nothing under the
    /// veil scrolls). True when it was taken.
    pub fn overview_scroll(&mut self, dx: f32, dy: f32, t: f64) -> bool {
        if !self.desk_active() || self.is_locked() {
            return false;
        }
        if self.overview.open && !self.overview.grid.visible() {
            return true;
        }
        if !self.overview.grid.visible() {
            return false;
        }
        let mut ov = std::mem::take(&mut self.overview);
        ov.grid.scroll(self, dx, dy, t);
        self.overview = ov;
        self.dirty = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The desktop thumbnail is the wallpaper reduced, not a plate: each half keeps its colour, and no wallpaper gives none.
    #[test]
    fn the_desktop_thumbnail_is_the_wallpaper_reduced() {
        let (red, blue) = (crate::gfx::pack(200, 0, 0, 255), crate::gfx::pack(0, 0, 200, 255));
        let px: Vec<u32> = (0..8 * 4).map(|i| if i % 8 < 4 { red } else { blue }).collect();
        let img = wallpaper_thumb(View { w: 8, h: 4, px: &px }, 2, 1).expect("a thumbnail");
        assert_eq!((img.w, img.h, img.px.len()), (2, 1, 2));
        assert_eq!(img.px, vec![red, blue], "each half of the picture keeps its colour, fully opaque");
        assert!(wallpaper_thumb(View { w: 0, h: 0, px: &[] }, 2, 1).is_none());
        assert!(wallpaper_thumb(View { w: 8, h: 4, px: &px }, 0, 1).is_none());
    }

    fn area() -> FRect {
        FRect { x: 0.0, y: 0.0, w: 1600.0, h: 800.0 }
    }

    #[test]
    fn tiles_never_overlap_and_stay_inside_the_area() {
        for n in 1..=12 {
            let sizes: Vec<(f32, f32)> = (0..n).map(|i| (600.0 + 40.0 * i as f32, 400.0 + 25.0 * (i % 3) as f32)).collect();
            let tiles = grid_layout(&sizes, area(), 20.0);
            assert_eq!(tiles.len(), n);
            for (i, a) in tiles.iter().enumerate() {
                assert!(a.x >= -0.01 && a.y >= -0.01 && a.x + a.w <= 1600.01 && a.y + a.h <= 800.01, "tile {i} of {n} outside");
                for b in &tiles[i + 1..] {
                    assert!(!a.overlaps(b), "{n} windows overlap");
                }
            }
        }
    }

    #[test]
    fn tiles_keep_their_aspect_and_never_grow() {
        let tiles = grid_layout(&[(400.0, 300.0), (1200.0, 700.0)], area(), 20.0);
        assert!((tiles[0].w - 400.0).abs() < 0.01 && (tiles[0].h - 300.0).abs() < 0.01);
        assert!((tiles[1].w / tiles[1].h - 1200.0 / 700.0).abs() < 0.01);
    }

    #[test]
    fn no_windows_no_tiles() {
        assert!(grid_layout(&[], area(), 10.0).is_empty());
    }

    #[test]
    fn the_last_row_is_centred() {
        let sizes = vec![(500.0, 400.0); 5];
        let t = grid_layout(&sizes, area(), 20.0);
        let mid = |r: &FRect| r.cx();
        // whatever the column count, the tiles as a group are centred on the area
        let left = t.iter().map(|r| r.x).fold(f32::MAX, f32::min);
        let right = t.iter().map(|r| r.x + r.w).fold(f32::MIN, f32::max);
        assert!(((left + right) / 2.0 - 800.0).abs() < 1.0);
        assert!(mid(&t[0]) > 0.0);
    }

    /// Ctrl+Up and F3 open Overview on the desktop, the same chord closes it, and Esc closes it too. The Apps grid
    /// takes only its navigation keys, so a letter reaches `hw_type`.
    #[test]
    fn ctrl_up_and_f3_toggle_mission_control_on_the_desktop() {
        let mut sh = crate::shell::tests::desktop_shell();
        sh.screen_on = true;
        sh.screen = crate::shell::Screen::Home;
        assert!(!sh.hw_overview_key(KEY_UP, true, KeyMods { ctrl: false, cmd: false, shift: false }, false, 1000.0), "a plain Up is the app's");
        assert!(!sh.overview.open);
        assert!(sh.hw_overview_key(KEY_UP, true, KeyMods { ctrl: true, cmd: false, shift: false }, false, 1000.0), "Ctrl+Up opens it");
        assert!(sh.overview.open);
        assert!(!sh.hw_overview_key(KEY_UP, false, KeyMods { ctrl: true, cmd: false, shift: false }, false, 1100.0), "the release passes");
        assert!(sh.hw_overview_key(KEY_F3, true, KeyMods { ctrl: false, cmd: false, shift: false }, false, 2000.0), "F3 closes it");
        assert!(sh.overview.closing.is_some() || !sh.overview.open);
        let mut sh = crate::shell::tests::desktop_shell();
        sh.screen_on = true;
        sh.screen = crate::shell::Screen::Home;
        assert!(sh.hw_overview_key(KEY_F3, true, KeyMods { ctrl: false, cmd: false, shift: false }, false, 1000.0), "F3 opens it");
        assert!(sh.overview.open);
        assert!(sh.hw_overview_key(KEY_ESC, true, KeyMods { ctrl: false, cmd: false, shift: false }, false, 2000.0), "Esc is taken while it is open");
        assert!(!sh.hw_overview_key(29, true, KeyMods { ctrl: true, cmd: false, shift: false }, false, 2100.0), "a modifier press passes");
    }

    /// Overview belongs to the desktop: the tablet form never opens it, and Alt or Cmd chords are not its keys.
    #[test]
    fn mission_control_keys_do_nothing_on_a_tablet_or_with_alt() {
        let mut sh = crate::shell::tests::shell();
        sh.screen_on = true;
        assert!(!sh.hw_overview_key(KEY_F3, true, KeyMods { ctrl: false, cmd: false, shift: false }, false, 1000.0));
        let mut sh = crate::shell::tests::desktop_shell();
        sh.screen_on = true;
        sh.screen = crate::shell::Screen::Home;
        assert!(!sh.hw_overview_key(KEY_UP, true, KeyMods { ctrl: true, cmd: false, shift: false }, true, 1000.0), "Ctrl+Alt+Up is the screen reader's");
        assert!(!sh.hw_overview_key(KEY_F3, true, KeyMods { ctrl: false, cmd: true, shift: false }, false, 1000.0), "Cmd+F3 is not Overview");
        assert!(!sh.overview.open);
    }

    // ---- Overview as an overlay surface (E2E round 3, MC1) ----

    const NO_MODS: KeyMods = KeyMods { ctrl: false, cmd: false, shift: false };
    const APPS_FOR_TILES: [AppId; 6] = [AppId::Settings, AppId::AppStore, AppId::Terminal, AppId::Deildu, AppId::Photos, AppId::Magnifier];

    /// A desktop shell on an output of `w` x `h` px at `s` px per point, past the lock screen, with `n` windows open.
    fn desk_with(n: usize, w: i32, h: i32, s: f32) -> Shell {
        let mut sh = crate::shell::tests::desktop_shell();
        // the fixture starts on the lock screen, where nothing opens and no chrome is exported
        sh.anim = None;
        sh.screen_on = true;
        sh.screen = crate::shell::Screen::Home;
        sh.w = w;
        sh.h = h;
        sh.ui.s = s;
        let m = sh.desk_metrics();
        let d = sh.desk.as_mut().unwrap();
        d.wins.clear();
        for app in APPS_FOR_TILES.into_iter().take(n) {
            d.open(&m, app, (m.w * 6 / 10, m.h * 5 / 10));
        }
        sh
    }

    /// Runs whatever Overview and the grid animate to their ends, as the frames after the close would.
    fn settle(sh: &mut Shell) {
        let (w, h) = (sh.w as usize, sh.h as usize);
        let mut px = vec![0u32; w * h];
        sh.draw_overview(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 1e9);
    }

    #[test]
    fn the_layout_scales_with_the_area_it_is_given() {
        let sizes = [(900.0, 600.0), (700.0, 500.0), (1100.0, 640.0), (500.0, 500.0)];
        let a = FRect { x: 40.0, y: 120.0, w: 1500.0, h: 700.0 };
        let one = grid_layout(&sizes, a, 26.0);
        let k = 1.5;
        let big: Vec<(f32, f32)> = sizes.iter().map(|&(w, h)| (w * k, h * k)).collect();
        let two = grid_layout(&big, FRect { x: a.x * k, y: a.y * k, w: a.w * k, h: a.h * k }, 26.0 * k);
        assert_eq!(one.len(), two.len());
        for (p, q) in one.iter().zip(&two) {
            for (a, b) in [(p.x, q.x), (p.y, q.y), (p.w, q.w), (p.h, q.h)] {
                assert!((a * k - b).abs() < 0.5, "the same arrangement at another scale: {p:?} vs {q:?}");
            }
        }
    }

    /// Whatever the output size, aspect ratio or scale, every window of the desktop gets a tile, the tiles never overlap, and
    /// all of them lie in the area under the desktops strip and above the Dock.
    #[test]
    fn tiles_fit_any_output_without_overlap() {
        for (w, h, s) in [(1200, 1920, 1.0), (1920, 1080, 1.0), (1366, 768, 1.0), (2560, 1080, 1.25), (3840, 2160, 2.0), (800, 1280, 0.75)] {
            for n in 1..=APPS_FOR_TILES.len() {
                let mut sh = desk_with(n, w, h, s);
                sh.overview_toggle(1000.0);
                assert!(sh.overview.open, "{w}x{h} opens");
                let tiles = sh.overview.tile_rects();
                assert_eq!(tiles.len(), n, "{w}x{h} at {s}: a tile per window");
                let g = sh.overview.geo(&sh);
                let (_, vy, _, vh) = sh.overview_veil_rect().expect("the output has room for the veil");
                assert!(g.area.y >= vy as f32 && g.area.y + g.area.h <= (vy + vh) as f32 + 0.5, "{w}x{h}: the tile area is inside the veil");
                for (i, (_, a)) in tiles.iter().enumerate() {
                    assert!(a.x >= g.area.x - 0.5 && a.x + a.w <= g.area.x + g.area.w + 0.5, "{w}x{h} tile {i} of {n} outside in x");
                    assert!(a.y >= g.area.y - 0.5 && a.y + a.h <= g.area.y + g.area.h + 0.5, "{w}x{h} tile {i} of {n} outside in y");
                    for (_, b) in &tiles[i + 1..] {
                        assert!(!a.overlaps(b), "{w}x{h} at {s}: {n} tiles overlap");
                    }
                }
            }
        }
    }

    #[test]
    fn f4_opens_and_closes_the_apps_grid_and_mission_control_swallows_other_keys() {
        let mut sh = desk_with(2, 1920, 1080, 1.0);
        assert!(sh.hw_overview_key(KEY_F4, true, NO_MODS, false, 1000.0), "F4 opens the Apps grid");
        assert!(sh.overview.grid.visible());
        assert!(sh.hw_overview_key(KEY_F4, true, NO_MODS, false, 2000.0), "and closes it");
        settle(&mut sh);
        assert!(!sh.overview.grid.visible());
        assert!(!sh.hw_overview_key(KEY_F4, true, KeyMods { ctrl: true, ..NO_MODS }, false, 3000.0), "Ctrl+F4 is not the grid's");
        assert!(!sh.hw_overview_key(KEY_F4, true, NO_MODS, true, 3000.0), "Alt+F4 closes a window");
        // Overview is modal: a letter is used up, a modifier passes, Esc closes it
        assert!(sh.hw_overview_key(KEY_F3, true, NO_MODS, false, 4000.0));
        assert!(sh.overview.open);
        assert!(sh.hw_overview_key(30, true, NO_MODS, false, 4100.0), "a letter does not reach the app under it");
        assert!(!sh.hw_overview_key(29, true, KeyMods { ctrl: true, ..NO_MODS }, false, 4200.0), "Ctrl passes");
        assert!(sh.hw_overview_key(KEY_ESC, true, NO_MODS, false, 5000.0), "Esc closes it");
        settle(&mut sh);
        assert!(!sh.overview.open, "closed once its animation ends");
        // F3 and Ctrl+Up close it, too
        for (code, mods) in [(KEY_F3, NO_MODS), (KEY_UP, KeyMods { ctrl: true, ..NO_MODS })] {
            assert!(sh.hw_overview_key(code, true, mods, false, 6000.0));
            assert!(sh.overview.open);
            assert!(sh.hw_overview_key(code, true, mods, false, 7000.0));
            settle(&mut sh);
            assert!(!sh.overview.open, "the chord that opened it closes it");
        }
    }

    #[test]
    fn the_wheel_pages_the_apps_grid_and_mission_control_swallows_it() {
        let mut sh = desk_with(1, 1920, 1080, 1.0);
        assert!(!sh.overview_scroll(0.0, 100.0, 1000.0), "nothing open: the wheel is the window's");
        sh.appgrid_toggle(1000.0);
        assert!(sh.overview_scroll(0.0, 100.0, 1100.0), "the grid takes the wheel");
        assert!(sh.overview_scroll(100.0, 0.0, 1200.0), "and a sideways swipe");
        sh.appgrid_toggle(2000.0);
        settle(&mut sh);
        sh.overview_toggle(3000.0);
        assert!(sh.overview.open);
        assert!(sh.overview_scroll(0.0, 100.0, 3100.0), "nothing under the veil scrolls");
    }

    #[test]
    fn clicking_a_window_raises_it_and_closes_mission_control_and_empty_space_just_closes() {
        let mut sh = desk_with(2, 1920, 1080, 1.0);
        // click the window that is not in front
        let front = sh.desk.as_ref().unwrap().front().map(|w| w.app).unwrap();
        let other = if front == AppId::Settings { AppId::AppStore } else { AppId::Settings };
        sh.overview_toggle(1000.0);
        let tile = sh.overview.tile_rects().into_iter().find(|(a, _)| *a == other).expect("a tile for each window").1;
        let (cx, cy) = (tile.x + tile.w / 2.0, tile.y + tile.h / 2.0);
        // the pointer over the tile highlights it
        assert!(sh.overview_hover(cx, cy), "the veil takes the hover");
        assert!(matches!(sh.overview.hover, Some(Target::Tile(_))));
        assert!(!sh.overview_hover(2.0, 2.0), "the menu bar is not the veil's");
        assert!(sh.overview_event(Event::Down(cx, cy), 2000.0).is_some());
        assert!(sh.overview_event(Event::Up(cx, cy), 2010.0).is_some());
        assert!(sh.overview.closing.is_some() || !sh.overview.open, "the click closes it");
        settle(&mut sh);
        assert!(!sh.overview.open);
        assert_eq!(sh.desk.as_ref().unwrap().front().map(|w| w.app), Some(other), "and raises the window that was clicked");
        // a click on empty space (the area's bottom left corner holds no tile) only closes it
        sh.overview_toggle(3000.0);
        let g = sh.overview.geo(&sh);
        let (ex, ey) = (2.0, g.area.y + g.area.h - 2.0);
        assert!(matches!(sh.overview.target_at(&g, ex, ey), Target::Back), "the corner of the area is empty");
        assert!(sh.overview_event(Event::Down(ex, ey), 4000.0).is_some());
        assert!(sh.overview_event(Event::Up(ex, ey), 4010.0).is_some());
        assert!(sh.overview.closing.is_some() || !sh.overview.open);
        settle(&mut sh);
        assert!(!sh.overview.open);
        assert_eq!(sh.desk.as_ref().unwrap().front().map(|w| w.app), Some(other), "the stack is as it was");
    }

    #[cfg(feature = "desktop-wayland")]
    fn frame_ids(sh: &Shell) -> Vec<u32> {
        let (w, h) = (sh.w as usize, sh.h as usize);
        let pixels = vec![0u32; w * h];
        sh.desktop_chrome_frames(&pixels, w, h).iter().map(|f| f.id).collect()
    }

    /// Overview is chrome surface 9 (above the windows) only while it is open.
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn the_surface_is_listed_only_while_mission_control_is_open() {
        let mut sh = desk_with(2, 1920, 1080, 1.0);
        let (w, h) = (sh.w as usize, sh.h as usize);
        assert!(!frame_ids(&sh).contains(&9), "closed: no surface");
        sh.overview_toggle(1000.0);
        let pixels = vec![0u32; w * h];
        let frames = sh.desktop_chrome_frames(&pixels, w, h);
        let f = frames.iter().find(|f| f.id == 9).expect("the open Overview is a chrome surface above the windows");
        assert_eq!(f.pixels.len(), f.width * f.height);
        assert_eq!((f.x, f.y as usize, f.width), (0, sh.desktop_bar_height(), w), "the full width under the menu bar");
        assert!(f.y as usize + f.height <= h - sh.desk_metrics().dock_bottom.max(0) as usize, "and above the Dock");
        assert!(f.input.is_none(), "the whole surface takes the pointer, so a click on empty space closes it");
        assert!(!frames.iter().any(|f| f.id == 10), "no sweep surface while it is open");
        // closing: still listed while it animates out, gone the frame it ends in
        sh.hw_overview_key(KEY_ESC, true, NO_MODS, false, 2000.0);
        if sh.overview.closing.is_some() {
            assert!(frame_ids(&sh).contains(&9), "listed while it animates out");
        }
        settle(&mut sh);
        assert!(!frame_ids(&sh).contains(&9), "the frame it closes in drops the surface");
    }

    /// A desktop switch's sweep is surface 10 with an empty input region, and only while it runs.
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn a_desktop_switch_sweeps_on_a_surface_that_takes_no_input() {
        let mut sh = desk_with(1, 1920, 1080, 1.0);
        sh.overview.spaces.count = 2;
        sh.switch_desktop(1, 1000.0);
        assert!(sh.overview.slide.is_some(), "the sweep runs");
        let (w, h) = (sh.w as usize, sh.h as usize);
        let pixels = vec![0u32; w * h];
        let frames = sh.desktop_chrome_frames(&pixels, w, h);
        let f = frames.iter().find(|f| f.id == 10).expect("the sweep is a surface above the windows");
        assert_eq!(f.input, Some([0, 0, 0, 0]), "an empty input region: the pointer reaches what is below");
        assert!(!frames.iter().any(|f| f.id == 9), "it is not Overview");
        settle(&mut sh);
        assert!(!frame_ids(&sh).contains(&10), "gone when the sweep ends");
    }

    // ---- windows of other clients: tiles from window captures ----------------------------------------------------------

    /// Overview for a desk with `n` windows and the windows `foreign` of other clients, window capture offered.
    #[cfg(feature = "desktop-wayland")]
    fn desk_with_foreign(n: usize, foreign: &[(u32, &str)]) -> Shell {
        let mut sh = desk_with(n, 1920, 1080, 1.0);
        sh.shots.set_available(true);
        sh.desk.as_mut().unwrap().native_windows = foreign
            .iter()
            .map(|(id, title)| crate::bare::wayland::ForeignWindow { id: *id, title: (*title).into(), app_id: format!("x.app{id}"), active: false, maximized: false })
            .collect();
        sh
    }

    #[cfg(feature = "desktop-wayland")]
    fn foreign_tiles(sh: &Shell) -> Vec<(u32, FRect)> {
        sh.overview.tiles.iter().filter_map(|t| match t.key { TileKey::Foreign(id) => Some((id, t.to)), TileKey::App(_) => None }).collect()
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn every_window_of_another_client_gets_a_tile_beside_the_desks() {
        let mut sh = desk_with_foreign(2, &[(7, "Files"), (8, "Browser")]);
        sh.overview_toggle(1000.0);
        assert!(sh.overview.open);
        assert_eq!(sh.overview.tiles.len(), 4, "two desk windows and two windows of other clients");
        assert_eq!(sh.overview.tile_rects().len(), 2, "tile_rects is the desk's");
        let foreign = foreign_tiles(&sh);
        assert_eq!(foreign.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![7, 8]);
        let g = sh.overview.geo(&sh);
        for (i, (_, a)) in foreign.iter().enumerate() {
            assert!(a.x >= g.area.x - 0.5 && a.x + a.w <= g.area.x + g.area.w + 0.5 && a.y >= g.area.y - 0.5 && a.y + a.h <= g.area.y + g.area.h + 0.5, "tile {i} inside the area");
        }
        let all: Vec<FRect> = sh.overview.tiles.iter().map(|t| t.to).collect();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert!(!a.overlaps(b), "tiles never overlap");
            }
        }
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn a_window_listed_twice_has_one_tile() {
        let mut sh = desk_with_foreign(1, &[(7, "Files")]);
        sh.desk.as_mut().unwrap().linux_apps.push(crate::shell::desk::LinuxDockApp::for_test("x.app7", vec![crate::bare::wayland::ForeignWindow { id: 7, title: "Files".into(), app_id: "x.app7".into(), active: false, maximized: false }]));
        sh.overview_toggle(1000.0);
        assert_eq!(foreign_tiles(&sh).len(), 1);
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn opening_asks_for_each_windows_picture_at_the_tile_size() {
        let mut sh = desk_with_foreign(1, &[(7, "Files")]);
        sh.overview_toggle(1000.0);
        let reqs = sh.take_capture_requests();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].source, Source::Foreign(7));
        let tile = foreign_tiles(&sh)[0].1;
        assert_eq!(reqs[0].max, window_shots::bound_for_tile(tile.w, tile.h));
        assert_eq!(reqs[0].since, 0, "nothing held yet");
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn the_tiles_refresh_at_most_five_times_a_second_while_open() {
        let mut sh = desk_with_foreign(0, &[(7, "Files")]);
        sh.overview_toggle(1000.0);
        let first = sh.take_capture_requests();
        sh.window_capture_results(vec![(first[0].token, window_shots::Outcome::Ready { image: Image { w: 80, h: 50, px: vec![0xff00_ff00; 80 * 50] }, serial: 2 })], 1010.0);
        for t in [1050.0, 1100.0, 1190.0] {
            sh.shots_tick(t);
            assert!(sh.take_capture_requests().is_empty(), "{t}: too soon after the last ask");
        }
        sh.shots_tick(1200.0);
        let again = sh.take_capture_requests();
        assert_eq!(again.len(), 1, "200 ms after the last ask it asks again");
        assert_eq!(again[0].since, 2, "quoting the picture it holds, so an idle window costs nothing");
        assert_eq!(sh.shots_wake(1200.0), Some(1200.0 + window_shots::MISSION_REFRESH_MS), "the loop wakes to refresh");
        // closed: no more asking and no wake
        sh.window_capture_results(vec![(again[0].token, window_shots::Outcome::Unchanged { serial: 2 })], 1210.0);
        sh.overview_toggle(1300.0);
        settle(&mut sh);
        sh.shots_tick(5000.0);
        assert!(sh.take_capture_requests().is_empty() && sh.shots_wake(5000.0).is_none());
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn the_first_picture_gives_its_tile_the_windows_real_aspect() {
        let mut sh = desk_with_foreign(0, &[(7, "Files")]);
        sh.overview_toggle(1000.0);
        let before = foreign_tiles(&sh)[0].1;
        assert!((before.w / before.h - 1.6).abs() < 0.05, "16:10 until there is a picture: {before:?}");
        let token = sh.take_capture_requests()[0].token;
        sh.window_capture_results(vec![(token, window_shots::Outcome::Ready { image: Image { w: 300, h: 400, px: vec![0xff11_2233; 300 * 400] }, serial: 1 })], 1050.0);
        let after = foreign_tiles(&sh)[0].1;
        assert!((after.w / after.h - 0.75).abs() < 0.02, "a tall window gets a tall tile: {after:?}");
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn a_tile_without_a_picture_still_draws_and_a_picture_replaces_the_plate() {
        let mut sh = desk_with_foreign(0, &[(7, "Files")]);
        sh.overview_toggle(1000.0);
        let (w, h) = (sh.w as usize, sh.h as usize);
        let draw = |sh: &mut Shell| -> Vec<u32> {
            let mut px = vec![0u32; w * h];
            sh.draw_overview(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 1e9 / 2.0);
            px
        };
        // the plate (the fallback: the window server cannot capture, or the answer is on its way)
        let plate = draw(&mut sh);
        let tile = foreign_tiles(&sh)[0].1;
        let (cx, cy) = ((tile.x + tile.w / 2.0) as usize, (tile.y + tile.h * 0.9) as usize);
        let token = sh.take_capture_requests()[0].token;
        sh.window_capture_results(vec![(token, window_shots::Outcome::Ready { image: Image { w: 160, h: 100, px: vec![crate::gfx::pack(250, 10, 10, 255); 160 * 100] }, serial: 1 })], 1500.0);
        let tile = foreign_tiles(&sh)[0].1;
        let (cx2, cy2) = ((tile.x + tile.w / 2.0) as usize, (tile.y + tile.h * 0.9) as usize);
        let picture = draw(&mut sh);
        assert_ne!(plate[cy * w + cx], picture[cy2 * w + cx2], "the picture is drawn where the plate was");
        assert_eq!(picture[cy2 * w + cx2] & 255, 250, "the window's own red, not the plate");
    }

    #[test]
    fn leaving_without_a_choice_gives_the_focus_back_to_the_window_that_had_it() {
        let gone = |_: AppId| false;
        let there = |_: AppId| true;
        let others = vec![(7u32, "Files".to_string())];
        assert_eq!(leave_focus((Some(7), Some(AppId::Settings)), &others, there), Focus::Foreign(7), "a focused window of another client first");
        assert_eq!(leave_focus((Some(9), Some(AppId::Settings)), &others, there), Focus::App(AppId::Settings), "it closed meanwhile: the desk's front window");
        assert_eq!(leave_focus((None, Some(AppId::Settings)), &others, there), Focus::App(AppId::Settings));
        assert_eq!(leave_focus((None, Some(AppId::Settings)), &others, gone), Focus::Refocus, "that window is gone or hidden");
        assert_eq!(leave_focus((None, None), &others, there), Focus::Refocus, "an empty desk has nobody to return to");
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn closing_mission_control_keeps_the_window_that_was_in_front() {
        let mut sh = desk_with_foreign(2, &[(7, "Notes")]);
        let front = sh.desk.as_ref().unwrap().front().map(|w| w.app).expect("a window is in front");
        sh.overview_toggle(1000.0);
        assert!(sh.overview.open);
        sh.overview_toggle(2000.0);
        settle(&mut sh);
        assert!(!sh.overview.open, "Overview closed");
        assert_eq!(sh.desk.as_ref().unwrap().front().map(|w| w.app), Some(front), "the same window is in front");
        assert_eq!(sh.screen, crate::shell::Screen::App(front.parent()), "and has the focus");
        assert!(sh.take_raise_front(), "the shell raises it above the windows of other clients");
        assert!(sh.take_native_activations().is_empty(), "the window of another client is not the one raised");
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn closing_mission_control_returns_to_the_window_of_another_client_that_was_active() {
        let mut sh = desk_with_foreign(1, &[(7, "Notes")]);
        sh.desk.as_mut().unwrap().native_windows[0].active = true;
        sh.overview_toggle(1000.0);
        sh.overview_toggle(2000.0);
        settle(&mut sh);
        assert_eq!(sh.take_native_activations(), vec![7], "the window server is asked to focus it again");
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn choosing_a_window_of_another_client_activates_it() {
        let mut sh = desk_with_foreign(1, &[(7, "Files")]);
        sh.overview_toggle(1000.0);
        let tile = foreign_tiles(&sh)[0].1;
        let (cx, cy) = (tile.x + tile.w / 2.0, tile.y + tile.h / 2.0);
        assert!(sh.overview_event(Event::Down(cx, cy), 2000.0).is_some());
        assert!(sh.overview_event(Event::Up(cx, cy), 2010.0).is_some());
        settle(&mut sh);
        assert!(!sh.overview.open, "Overview closed");
        assert_eq!(sh.take_native_activations(), vec![7], "the window server is asked to raise and focus window 7");
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn a_window_of_another_client_does_not_move_to_another_desktop() {
        let mut sh = desk_with_foreign(1, &[(7, "Files")]);
        sh.overview.spaces.count = 2;
        sh.overview_toggle(1000.0);
        let idx = sh.overview.tiles.iter().position(|t| t.key == TileKey::Foreign(7)).unwrap();
        let g = sh.overview.geo(&sh);
        let desk2 = sh.overview.desk_rects(&g)[1];
        sh.overview.press = Some((Target::Tile(idx), 10.0, 10.0));
        sh.overview.drag = Some(Drag { tile: idx, x: desk2.x + 1.0, y: desk2.y + 1.0, moved: true });
        sh.overview.hover = Some(Target::Desk(1));
        sh.overview_event(Event::Up(desk2.x + 1.0, desk2.y + 1.0), 2000.0);
        assert!(sh.overview.open && sh.overview.closing.is_none(), "dropping it on a desktop does nothing");
        assert_eq!(foreign_tiles(&sh).len(), 1);
    }

    #[test]
    fn the_natural_size_of_a_window_of_another_client_follows_its_picture() {
        let screen = (1920.0, 1080.0);
        let (w, h) = foreign_natural(None, screen);
        assert!((w / h - 1.6).abs() < 1e-3 && w <= 1920.0 * 0.6 + 0.5 && h <= 1080.0 * 0.6 + 0.5);
        let (w, h) = foreign_natural(Some((300, 400)), screen);
        assert!((w / h - 0.75).abs() < 1e-3 && h <= 1080.0 * 0.6 + 0.5, "{w}x{h}");
        let (w, h) = foreign_natural(Some((4000, 100)), screen);
        assert!(w <= 1920.0 * 0.6 + 0.5 && h > 0.0, "an extreme aspect is clamped: {w}x{h}");
        assert_eq!(foreign_natural(Some((0, 0)), screen), foreign_natural(None, screen), "an empty picture is none");
    }
}
