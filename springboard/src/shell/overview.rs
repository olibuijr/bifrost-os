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
use super::ui::{Weight, CARD_BG, FIELD_BG, LABEL, LABEL_2, MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT, SELECTION, TINT, WHITE};
use super::{apps, AppId, Effect, Event, Shell};
use crate::gfx::{rgba, Image, Surface};
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

struct Tile {
    app: AppId,
    from: FRect,
    to: FRect,
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
    sources: HashMap<AppId, Image>,
    scaled: HashMap<(AppId, usize, usize), Image>,
    size: (i32, i32),
    pub(super) grid: AppGrid,
    /// a press that began on the menu bar or the Dock fell through, so its release must too (the Dock acts on release)
    pass_up: bool,
    /// whether the last desktop frame had Overview or the Apps grid on screen: a change repaints the frame, so the
    /// overlay surface the grid is exported as appears and goes in step with it
    pub(super) was_visible: bool,
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
        let sizes: Vec<(f32, f32)> = wins.iter().map(|w| (w.r.w as f32, w.r.h as f32)).collect();
        let to = grid_layout(&sizes, g.area, TILE_GAP_PT * g.pt);
        self.tiles = wins.iter().zip(to).map(|(w, to)| Tile {
            app: w.app,
            from: FRect { x: w.r.x as f32, y: w.r.y as f32, w: w.r.w as f32, h: w.r.h as f32 },
            to,
        }).collect();
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
        let Some(d) = sh.desk.as_mut() else { return };
        self.spaces.sync(&d.wins);
        self.saved = d.wins.iter().map(|w| (w.app, w.minimized)).collect();
        for w in d.wins.iter_mut() {
            w.minimized = true;
        }
        self.open = true;
        self.start = t;
        self.closing = None;
        self.chosen = None;
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
                        self.sources.insert(w.app, Image { w: pw, h: ph, px: px[..pw * ph].to_vec() });
                    }
                }
            }
        }
        self.layout(sh);
        sh.dirty = true;
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
        match self.chosen.take() {
            Some(app) => sh.desk_open(app, t),
            None => sh.desk_refocus(),
        }
        self.sources.clear();
        self.scaled.clear();
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
                let app = self.selected.and_then(|i| self.tiles.get(i)).map(|t| t.app);
                self.close(sh, app, t);
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
                    let app = self.tiles.get(d.tile).map(|t| t.app);
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
                            Target::Tile(i) => {
                                let app = self.tiles.get(i).map(|t| t.app);
                                self.close(sh, app, t);
                            }
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
    fn thumb(&mut self, app: AppId, w: usize, h: usize) -> Option<&Image> {
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
        for (i, r) in rects.iter().enumerate() {
            let y = r.y + slide_y;
            let rad = ui.pt(8.0);
            s.fill_rrect(r.x, y, r.w, r.h, rad, crate::gfx::scale(CARD_BG.now(), alpha));
            for a in self.spaces.windows_on(i, &wins) {
                if let Some(w) = wins.iter().find(|w| w.app == a) {
                    let k = r.w / g.w.max(1.0);
                    let wr = FRect { x: r.x + w.r.x as f32 * k, y: y + w.r.y as f32 * k, w: (w.r.w as f32 * k).max(2.0), h: (w.r.h as f32 * k).max(2.0) };
                    s.fill_rrect(wr.x, wr.y, wr.w.min(r.x + r.w - wr.x).max(1.0), wr.h.min(y + r.h - wr.y).max(1.0), ui.pt(1.5), crate::gfx::scale(FIELD_BG.now(), alpha));
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
            let (app, from, to) = (self.tiles[i].app, self.tiles[i].from, self.tiles[i].to);
            let r = FRect::lerp(from, to, p);
            self.draw_tile(sh, s, app, r, to.w / from.w.max(1.0), alpha, hover_tile == Some(i), self.selected == Some(i));
            if hover_tile == Some(i) || self.selected == Some(i) {
                title = Some((i, r));
            }
        }
        if let Some((tile, dx, dy)) = self.drag.as_ref().filter(|d| d.moved && d.tile < n).map(|d| (d.tile, d.x, d.y)) {
            let (app, to, from) = (self.tiles[tile].app, self.tiles[tile].to, self.tiles[tile].from);
            let k = 0.6;
            let r = FRect { x: dx - to.w * k / 2.0, y: dy - to.h * k / 2.0, w: to.w * k, h: to.h * k };
            self.draw_tile(sh, s, app, r, k * to.w / from.w.max(1.0), 220, true, false);
        }
        // the title shows under the tile the pointer (or the keyboard) is on
        if let Some((i, r)) = title {
            if p >= 1.0 && dragging.is_none() {
                let name = apps().iter().find(|a| a.id == self.tiles[i].app).map_or("", |a| a.name);
                let (tw, th) = sh.label(name, MAC_SUBHEAD_PT, Weight::Semibold, WHITE, |im| (im.w as f32, im.h as f32));
                let (pw, ph) = (tw + ui.pt(20.0), th + ui.pt(8.0));
                let (px, py) = (r.cx() - pw / 2.0, r.y + r.h + ui.pt(8.0));
                desk_glass(s, ShellSurface::OverviewPill, (px, py, pw, ph), ph / 2.0, 255);
                sh.label(name, MAC_SUBHEAD_PT, Weight::Semibold, WHITE, |im| s.blit_colored(im, (px + (pw - im.w as f32) / 2.0) as i32, (py + (ph - im.h as f32) / 2.0) as i32, LABEL.now(), 255));
            }
        }
        p < 1.0 && self.closing.is_none() || self.closing.is_some()
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_tile(&mut self, sh: &Shell, s: &mut Surface, app: AppId, r: FRect, scale_k: f32, alpha: u32, hover: bool, selected: bool) {
        let ui = &sh.ui;
        let rad = (ui.pt(bifrost_ui::prefs_layout::m::WINDOW_RADIUS) * scale_k.clamp(0.0, 1.0)).max(ui.pt(4.0));
        // soft shadow
        for (k, a) in [(3.0, 14u8), (1.5, 20u8)] {
            let gx = ui.pt(k * 2.0);
            s.fill_rrect(r.x - gx, r.y - gx + ui.pt(4.0), r.w + 2.0 * gx, r.h + 2.0 * gx, rad + gx, rgba(0, 0, 0, ((a as u32 * alpha) / 255) as u8));
        }
        let (iw, ih) = (r.w.round().max(1.0) as usize, r.h.round().max(1.0) as usize);
        match self.thumb(app, iw, ih) {
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
    }

    pub fn appgrid_toggle(&mut self, t: f64) {
        if !self.desk_active() || self.is_locked() {
            return;
        }
        let mut ov = std::mem::take(&mut self.overview);
        if ov.open {
            ov.close(self, None, t);
        }
        if ov.grid.visible() {
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

    /// Wheel or two-finger scroll while the Apps grid is open pages through it; true when it was taken.
    pub fn overview_scroll(&mut self, dx: f32, dy: f32, t: f64) -> bool {
        if !self.desk_active() || !self.overview.grid.visible() {
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
}
