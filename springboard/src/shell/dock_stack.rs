//! The Dock's Downloads stack: a Dock entry just left of the Trash that shows the newest files of the account's
//! Downloads folder as a fanned stack. A click opens the stack above the Dock (`Menu::Stack`) the Automatic way: a fan
//! of the newest items with Open in Files at its top when they are few (`FAN_MAX`) and the Dock is at the bottom, else a
//! grid popover on the menu surface with Open in Files at its foot. Clicking an item opens it with its default app.
//!
//! The folder is listed on a timer (every `REFRESH_S` seconds, at most `MAX_SCAN` entries read), never per frame, so a
//! large folder cannot stall drawing; the open stack lists it once when it opens. Hidden files are left out.
// the open stack is drawn and clicked by the desktop-wayland menus only; other builds keep the layout for its tests
#![cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
use super::desk::{Desk, Rect};
use crate::gfx::{rgba, Surface};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The Dock list entry of the Downloads stack. Minimized windows take the entries from `DOCK_MIN_BASE` (usize::MAX - 1)
/// down to `usize::MAX - 4096`, so this one sits just below them and `dock_min_index` does not read it as a window.
pub(super) const DOCK_STACK: usize = usize::MAX - 4097;
/// How many of the newest files the stack fans out.
pub(super) const SHOWN: usize = 3;
/// Seconds between two listings of the Downloads folder.
const REFRESH_S: f64 = 10.0;
/// Entries read from the folder per listing at most.
const MAX_SCAN: usize = 2048;

/// The account's home folder: the installed account's home, else `$HOME`, else `/root` (the shell runs as root).
pub(super) fn home_dir() -> PathBuf {
    #[cfg(feature = "desktop-wayland")]
    let account = crate::bare::app_policy::Account::installed().ok().map(|a| a.home);
    #[cfg(not(feature = "desktop-wayland"))]
    let account = None;
    pick_home(account, std::env::var_os("HOME").map(PathBuf::from))
}

/// The home folder from its sources, best first: the installed account's, else `$HOME` when it is set and not empty,
/// else `/root`. A shell without `$HOME` (the session service does not set it) still finds the account's folders.
pub(super) fn pick_home(account: Option<PathBuf>, env: Option<PathBuf>) -> PathBuf {
    account
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| env.filter(|p| !p.as_os_str().is_empty()))
        .unwrap_or_else(|| "/root".into())
}

/// The account's Downloads folder.
pub(super) fn downloads_dir() -> PathBuf {
    home_dir().join("Downloads")
}

/// The names of the newest `limit` visible entries of `dir`, newest first (ties by name). An unreadable folder is empty.
pub(super) fn recent_files(dir: &Path, limit: usize) -> Vec<String> {
    recent_items(dir, limit).into_iter().map(|item| item.name).collect()
}

/// One entry of the open stack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Item {
    pub name: String,
    pub path: PathBuf,
    pub dir: bool,
}

/// The newest `limit` visible entries of `dir`, newest first (ties by name), at most `MAX_SCAN` read. An unreadable
/// folder is empty.
pub(super) fn recent_items(dir: &Path, limit: usize) -> Vec<Item> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<(SystemTime, Item)> = read
        .take(MAX_SCAN)
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let meta = entry.metadata().ok();
            let when = meta.as_ref().and_then(|m| m.modified().ok()).unwrap_or(SystemTime::UNIX_EPOCH);
            // a link to a folder opens as the folder
            let dir = meta.is_some_and(|m| m.is_dir()) || entry.path().is_dir();
            Some((when, Item { name, path: entry.path(), dir }))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    found.into_iter().take(limit).map(|(_, item)| item).collect()
}

/// Items the open stack lists at most (a grid shows as many rows of them as fit above the Dock).
pub(super) const POPOVER_MAX: usize = 48;
/// Automatic: a fan for up to this many items, a grid for more.
pub(super) const FAN_MAX: usize = 10;
/// Gap (pt) between the open stack and its Dock icon, and the least room it keeps from the screen's edges.
const GAP_PT: f32 = 8.0;
const EDGE_PT: f32 = 8.0;
/// The grid: cell size, icon size and the panel's padding, title band and Open in Files band (pt).
pub(super) const GRID_CELL_W_PT: f32 = 84.0;
pub(super) const GRID_CELL_H_PT: f32 = 90.0;
pub(super) const GRID_ICON_PT: f32 = 52.0;
pub(super) const GRID_PAD_PT: f32 = 12.0;
pub(super) const GRID_HEADER_PT: f32 = 34.0;
pub(super) const GRID_FOOTER_PT: f32 = 36.0;
const GRID_COLS_MIN: usize = 3;
const GRID_COLS_MAX: usize = 6;
/// The fan: icon size, the rise from one item to the next, how far the top bends right, the room left of an icon for
/// its name, and the Open in Files button (pt).
pub(super) const FAN_ICON_PT: f32 = 44.0;
const FAN_STEP_PT: f32 = 52.0;
const FAN_SWEEP_PT: f32 = 56.0;
pub(super) const FAN_LABEL_PT: f32 = 190.0;
pub(super) const FAN_OPEN_PT: f32 = 32.0;

/// How the open stack is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Style {
    Fan,
    Grid,
}

/// Where the open stack sits from its Dock icon: above a bottom Dock, beside a side Dock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Side {
    Above,
    /// right of a Dock on the left edge
    Right,
    /// left of a Dock on the right edge
    Left,
}

/// A rectangle in screen px (x, y, w, h).
pub(super) type R = (f32, f32, f32, f32);

/// The open stack's geometry in screen px: the popup (the fan's bounding box), each shown item's rectangle (newest
/// first; a fan item's rectangle runs from its name's room on the left to its icon on the right), and Open in Files.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Layout {
    pub style: Style,
    pub panel: R,
    pub cells: Vec<R>,
    pub open: R,
}

/// What a click on the open stack lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hit {
    Item(usize),
    Open,
}

fn inside((x, y, w, h): R, px: f32, py: f32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

/// The open stack for `n` items from the Dock icon `icon`, kept inside `room` (the screen under the menu bar), `pt` px
/// per point. Automatic: a fan above a bottom Dock for up to `FAN_MAX` items when it fits, a grid otherwise.
pub(super) fn layout(n: usize, side: Side, icon: R, room: R, pt: f32) -> Layout {
    if side == Side::Above && n <= FAN_MAX {
        if let Some(l) = fan(n, icon, room, pt) {
            return l;
        }
    }
    grid(n, side, icon, room, pt)
}

/// The grid popover: about square (3 to 6 columns), as many rows as fit the room; the rest stays in Files.
fn grid(n: usize, side: Side, icon: R, room: R, pt: f32) -> Layout {
    let cols = ((n.max(1) as f32).sqrt().ceil() as usize).clamp(GRID_COLS_MIN, GRID_COLS_MAX);
    let (cw, ch, pad, head, foot) = (GRID_CELL_W_PT * pt, GRID_CELL_H_PT * pt, GRID_PAD_PT * pt, GRID_HEADER_PT * pt, GRID_FOOTER_PT * pt);
    let (gap, edge) = (GAP_PT * pt, EDGE_PT * pt);
    let (top, bottom) = (room.1 + edge, room.1 + room.3 - edge);
    // above the Dock the panel ends over the icon; beside it the panel may use the whole height
    let limit = if side == Side::Above { icon.1 - gap } else { bottom };
    let fit_rows = ((limit - top - head - foot) / ch).floor().max(1.0) as usize;
    let rows = n.div_ceil(cols).clamp(1, fit_rows);
    let shown = n.min(rows * cols);
    let (w, h) = (2.0 * pad + cols as f32 * cw, head + rows as f32 * ch + foot);
    let (lo_x, hi_x) = (room.0 + edge, (room.0 + room.2 - edge - w).max(room.0 + edge));
    let (lo_y, hi_y) = (top, (bottom - h).max(top));
    let (x, y) = match side {
        Side::Above => ((icon.0 + icon.2 / 2.0 - w / 2.0).clamp(lo_x, hi_x), (icon.1 - gap - h).max(top)),
        Side::Right => ((icon.0 + icon.2 + gap).clamp(lo_x, hi_x), (icon.1 + icon.3 / 2.0 - h / 2.0).clamp(lo_y, hi_y)),
        Side::Left => ((icon.0 - gap - w).clamp(lo_x, hi_x), (icon.1 + icon.3 / 2.0 - h / 2.0).clamp(lo_y, hi_y)),
    };
    let cells = (0..shown).map(|i| (x + pad + (i % cols) as f32 * cw, y + head + (i / cols) as f32 * ch, cw, ch)).collect();
    Layout { style: Style::Grid, panel: (x, y, w, h), cells, open: (x + pad, y + h - foot, w - 2.0 * pad, foot) }
}

/// The fan: the newest item just over the icon, each older one a step higher and bent a little further right, Open in
/// Files at the top. None when it does not fit the room (the grid is used instead).
fn fan(n: usize, icon: R, room: R, pt: f32) -> Option<Layout> {
    let (size, step, label, button) = (FAN_ICON_PT * pt, FAN_STEP_PT * pt, FAN_LABEL_PT * pt, FAN_OPEN_PT * pt);
    let (gap, edge) = (GAP_PT * pt, EDGE_PT * pt);
    let (cx, bottom) = (icon.0 + icon.2 / 2.0, icon.1 - gap);
    // slot i's centre: the newest item is slot 0, Open in Files the slot after the oldest item
    let centre = |i: usize| (cx + FAN_SWEEP_PT * pt * (i as f32 / FAN_MAX as f32).powi(2), bottom - step * (i as f32 + 0.5));
    let cell = |i: usize, side: f32| {
        let (x, y) = centre(i);
        (x - side / 2.0 - label, y - side / 2.0, label + side, side)
    };
    let cells: Vec<R> = (0..n).map(|i| cell(i, size)).collect();
    let open = cell(n, button);
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(x, y, w, h) in cells.iter().chain(std::iter::once(&open)) {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + w), y1.max(y + h));
    }
    let fits = x0 >= room.0 + edge && x1 <= room.0 + room.2 - edge && y0 >= room.1 + edge;
    fits.then_some(Layout { style: Style::Fan, panel: (x0, y0, x1 - x0, y1 - y0), cells, open })
}

/// The item or Open in Files under (`x`, `y`); None on the panel's gaps and outside it.
pub(super) fn hit(l: &Layout, x: f32, y: f32) -> Option<Hit> {
    if inside(l.open, x, y) {
        return Some(Hit::Open);
    }
    l.cells.iter().position(|&r| inside(r, x, y)).map(Hit::Item)
}

/// The short kind shown on a file's page: its extension in capitals, at most four letters; none without one.
pub(super) fn badge(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty() && !ext.is_empty() && ext.chars().all(char::is_alphanumeric)).then(|| ext.chars().take(4).collect::<String>().to_uppercase())
}

/// Offsets (px) of `n` fanned tiles from the front one (index 0, at the origin) to the back one: each further tile
/// steps right by `step` and up by half of it.
pub(super) fn fan_offsets(n: usize, step: i32) -> Vec<(i32, i32)> {
    (0..n as i32).map(|i| (i * step, -(i * step) / 2)).collect()
}

impl Desk {
    /// Re-lists the Downloads folder when the last listing is `REFRESH_S` old. Returns whether the listed names changed.
    pub(super) fn stack_refresh(&mut self, now: f64) -> bool {
        if self.stack_listed.is_some_and(|at| now - at < REFRESH_S) {
            return false;
        }
        self.stack_listed = Some(now);
        let next = recent_files(&downloads_dir(), SHOWN);
        if next == self.stack_files {
            return false;
        }
        self.stack_files = next;
        true
    }
}

#[cfg(feature = "desktop-wayland")]
impl super::Shell {
    /// A click on the Dock's Downloads stack: lists the folder's newest items and opens the stack above (or beside) the
    /// Dock icon (px), on the menu layer, as `Menu::Stack`. It hangs from the icon's resting place in the row (the
    /// magnification eases off once the pointer moves onto the stack); `pressed` is the icon as pressed, for a row
    /// without the stack.
    pub(super) fn stack_open(&mut self, pressed: Rect) {
        let m = self.desk_metrics();
        let (list, _, rects) = self.desk_dock(&m);
        let icon = list.iter().position(|&e| e == DOCK_STACK).and_then(|i| rects.get(i).copied()).unwrap_or(pressed);
        let items = recent_items(&downloads_dir(), POPOVER_MAX);
        let Some(d) = self.desk.as_mut() else { return };
        d.stack_items = items;
        d.stack_icon = Some((icon.x as f32, icon.y as f32, icon.w as f32, icon.h as f32));
        self.desktop_menu = Some(super::desktop_bar::Menu::Stack);
        self.desktop_menu_row = None;
        self.dirty = true;
    }

    /// The open stack's geometry for the current screen and Dock position.
    pub(super) fn stack_layout(&self) -> Option<Layout> {
        use super::desktop_dock::Position;
        let d = self.desk.as_ref()?;
        let icon = d.stack_icon?;
        let side = match self.dock_preferences().position {
            Position::Bottom => Side::Above,
            Position::Left => Side::Right,
            Position::Right => Side::Left,
        };
        let top = self.desktop_bar_height() as f32;
        let room = (0.0, top, self.w as f32, (self.h as f32 - top).max(0.0));
        Some(layout(d.stack_items.len(), side, icon, room, self.ui.pt(1.0)))
    }

    /// The open stack's popup rectangle (px), as `root_popup_rect` gives it for `Menu::Stack`.
    pub(super) fn stack_popup_rect(&self) -> Option<(usize, usize, usize, usize)> {
        let (x, y, w, h) = self.stack_layout()?.panel;
        Some((x.max(0.0) as usize, y.max(0.0) as usize, w.ceil().max(1.0) as usize, h.ceil().max(1.0) as usize))
    }

    /// A release on the open stack at (`x`, `y`) screen px: an item opens with its default app, Open in Files opens the
    /// folder; either closes the stack. A release on the panel's gaps keeps it open.
    pub(super) fn stack_click(&mut self, x: f32, y: f32, t: f64) {
        let Some(l) = self.stack_layout() else { return };
        let Some(target) = hit(&l, x, y) else { return };
        self.desktop_menu = None;
        self.dirty = true;
        match target {
            Hit::Open => self.stack_reveal(&downloads_dir(), t),
            Hit::Item(i) => {
                if let Some(item) = self.desk.as_ref().and_then(|d| d.stack_items.get(i).cloned()) {
                    self.stack_open_item(&item, t);
                }
            }
        }
    }

    /// Opens `folder` in a Files window.
    fn stack_reveal(&mut self, folder: &Path, t: f64) {
        if let Err(e) = self.desktop_open_folder(super::workspace::FILES_APP, &folder.to_string_lossy(), t) {
            eprintln!("BifrOSt Downloads: {e}");
        }
    }

    /// Opens a stack item the way Files opens it: a folder in Files; a file, after the malware gate, with the app its type
    /// opens with (a package archive on the install sheet). A suspicious file, or one no app opens, is shown in its folder
    /// in Files, whose open asks on the gate's card; a known threat gets the refusal card.
    fn stack_open_item(&mut self, item: &Item, t: f64) {
        use super::malware_gate::{check, rules_dir, Gate, MalwareAlert};
        if item.dir {
            return self.stack_reveal(&item.path, t);
        }
        let folder = item.path.parent().map_or_else(downloads_dir, Path::to_path_buf);
        match check(&rules_dir(), std::slice::from_ref(&item.path)) {
            Gate::Open => {}
            gate @ Gate::Refuse { .. } => {
                self.malware_alert = MalwareAlert::new(&self.ui, gate, None);
                self.dirty = true;
                self.drawn = super::Drawn::Nothing;
                return;
            }
            Gate::Ask { .. } => return self.stack_reveal(&folder, t),
        }
        if super::pacman_update::is_package_file(&item.name) && self.package_open_files(std::slice::from_ref(&item.path)) {
            return;
        }
        let opened = self.default_app_for_file(&item.name).is_some_and(|key| self.open_path_with(&key, &item.path, t));
        if !opened {
            self.stack_reveal(&folder, t);
        }
    }

    /// Draws the open stack into the popup `(x, y, w, h)` (px). The grid is drawn on the menu material
    /// (`draw_menu_level` drew it); the fan has no panel: its icons and name capsules float over the desktop.
    pub(super) fn draw_stack_popover(&self, s: &mut Surface) {
        use super::ui::{Weight, LABEL, LINK, MAC_BODY_PT, MAC_CALLOUT_PT, MAC_FOOTNOTE_PT, MAC_HEADLINE_PT, MAC_SUBHEAD_PT, MENU_SOLID, SECONDARY, SEPARATOR_C};
        let (Some(l), Some(d)) = (self.stack_layout(), self.desk.as_ref()) else { return };
        let ui = &self.ui;
        let edge = SEPARATOR_C.now();
        // an item's icon: a folder, or a page with the file's kind on it
        let icon = |s: &mut Surface, r: R, item: &Item| {
            if item.dir {
                draw_stack(s, &Rect { x: r.0.round() as i32, y: r.1.round() as i32, w: r.2.round() as i32, h: r.3.round() as i32 }, &[]);
                return;
            }
            draw_page(s, r, edge);
            if let Some(kind) = badge(&item.name) {
                self.label(&kind, MAC_FOOTNOTE_PT, Weight::Semibold, SECONDARY, |im| {
                    s.blit(im, (r.0 + (r.2 - im.w as f32) / 2.0) as i32, (r.1 + r.3 * 0.72 - im.h as f32 / 2.0) as i32, 255, 255)
                });
            }
        };
        match l.style {
            Style::Grid => {
                let (x, y, w, _) = l.panel;
                let pad = ui.pt(GRID_PAD_PT);
                self.label("Downloads", MAC_HEADLINE_PT, Weight::Semibold, LABEL.now(), |im| {
                    s.blit(im, (x + pad) as i32, (y + ui.pt(GRID_HEADER_PT) / 2.0 - im.h as f32 / 2.0) as i32, 255, 255)
                });
                if l.cells.is_empty() {
                    self.label("No Items", MAC_BODY_PT, Weight::Regular, SECONDARY, |im| {
                        s.blit(im, (x + (w - im.w as f32) / 2.0) as i32, (y + ui.pt(GRID_HEADER_PT + GRID_CELL_H_PT / 2.0) - im.h as f32 / 2.0) as i32, 255, 255)
                    });
                }
                let size = ui.pt(GRID_ICON_PT);
                // the name's line pitch: subhead type with a little leading
                let line = ui.pt(MAC_SUBHEAD_PT + 3.0);
                for (cell, item) in l.cells.iter().zip(&d.stack_items) {
                    let (cx, cy, cw, _) = *cell;
                    let top = cy + ui.pt(4.0);
                    icon(s, (cx + (cw - size) / 2.0, top, size, size), item);
                    for (k, text) in ui.wrap_capped(&item.name, MAC_SUBHEAD_PT, Weight::Regular, cw - ui.pt(6.0), 2).iter().enumerate() {
                        self.label(text, MAC_SUBHEAD_PT, Weight::Regular, LABEL.now(), |im| {
                            s.blit(im, (cx + (cw - im.w as f32) / 2.0) as i32, (top + size + ui.pt(4.0) + line * k as f32) as i32, 255, 255)
                        });
                    }
                }
                let (ox, oy, ow, oh) = l.open;
                s.fill_rect(ox as usize, oy as usize, ow as usize, 1, edge);
                self.label("Open in Files", MAC_BODY_PT, Weight::Regular, LINK, |im| s.blit(im, ox as i32, (oy + oh / 2.0 - im.h as f32 / 2.0) as i32, 255, 255));
            }
            Style::Fan => {
                if bifrost_render::glass::gpu() {
                    let (x, y, w, h) = l.panel;
                    s.fill_rect(x.max(0.0) as usize, y.max(0.0) as usize, w as usize, h as usize, 0);
                }
                // a name in a capsule ending just left of its icon `r`, centred on it
                let capsule = |s: &mut Surface, r: R, text: &str, color: u32| {
                    let room = ui.pt(FAN_LABEL_PT - 12.0);
                    let text = ui.wrap_capped(text, MAC_CALLOUT_PT, Weight::Regular, room - ui.pt(16.0), 1).into_iter().next().unwrap_or_default();
                    let (cw, ch) = (self.measure_chrome(&text, MAC_CALLOUT_PT, Weight::Regular) + ui.pt(16.0), ui.pt(22.0));
                    let (cx, cy) = (r.0 - ui.pt(6.0) - cw, r.1 + (r.3 - ch) / 2.0);
                    s.fill_rrect(cx, cy, cw, ch, ch / 2.0, MENU_SOLID.now());
                    self.label(&text, MAC_CALLOUT_PT, Weight::Regular, color, |im| s.blit(im, (cx + ui.pt(8.0)) as i32, (cy + (ch - im.h as f32) / 2.0) as i32, 255, 255));
                };
                let size = ui.pt(FAN_ICON_PT);
                for (cell, item) in l.cells.iter().zip(&d.stack_items) {
                    let (cx, cy, cw, _) = *cell;
                    let r = (cx + cw - size, cy, size, size);
                    icon(s, r, item);
                    capsule(s, r, &item.name, LABEL.now());
                }
                let (ox, oy, ow, oh) = l.open;
                let b = ui.pt(FAN_OPEN_PT);
                let r = (ox + ow - b, oy, b, oh);
                s.fill_rrect(r.0, r.1, b, b, b / 2.0, MENU_SOLID.now());
                if let Some(im) = ui.symbols.render(crate::symbols::Symbol::ChevronRight, ui.pt(13.0), LINK) {
                    s.blit(&im, (r.0 + (b - im.w as f32) / 2.0) as i32, (r.1 + (b - im.h as f32) / 2.0) as i32, 255, 255);
                }
                capsule(s, r, "Open in Files", LINK);
            }
        }
    }
}

/// The folder's back and front and a file tile's paper.
fn stack_colours() -> (u32, u32, u32) {
    (rgba(88, 150, 232, 255), rgba(116, 172, 244, 255), rgba(246, 247, 250, 255))
}

/// A file's page in `r` (px): the stack's pale tile with its edge in `edge`. The kind badge is the caller's.
pub(super) fn draw_page(s: &mut Surface, (x, y, w, h): R, edge: u32) {
    let (pw, ph) = (w * 0.74, h * 0.92);
    let (px, py) = (x + (w - pw) / 2.0, y + (h - ph) / 2.0);
    s.fill_rrect(px, py, pw, ph, pw * 0.1, stack_colours().2);
    s.stroke_rrect(px, py, pw, ph, pw * 0.1, (w / 64.0).max(1.0), edge);
}

/// Draws the Downloads stack in the Dock slot `r`: a blue folder with up to `SHOWN` pale file tiles fanned out behind
/// its front flap. An empty folder shows the flap alone (the open stack draws a folder item that way).
pub(super) fn draw_stack(s: &mut Surface, r: &Rect, files: &[String]) {
    let (x, y, w, h) = (r.x as f32, r.y as f32, r.w as f32, r.h as f32);
    let (back, front, tile) = stack_colours();
    let tiles = files.len().min(SHOWN);
    let (tw, th) = (w * 0.52, h * 0.52);
    let (x0, y0) = (x + w * 0.22, y + h * 0.14);
    let step = (w * 0.09).round() as i32;
    // the back of the folder, then the tiles from the back forward, then the front flap over their feet
    s.fill_rrect(x, y + h * 0.08, w, h * 0.9, w * 0.16, back);
    for (dx, dy) in fan_offsets(tiles, step).into_iter().rev() {
        s.fill_rrect(x0 + dx as f32, y0 + dy as f32, tw, th, tw * 0.12, tile);
    }
    s.fill_rrect(x, y + h * 0.36, w, h * 0.62, w * 0.16, front);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_home_comes_from_the_account_before_the_environment() {
        let p = |s: &str| Some(PathBuf::from(s));
        assert_eq!(pick_home(p("/home/ola"), p("/root")), PathBuf::from("/home/ola"), "the account wins");
        assert_eq!(pick_home(None, p("/home/x")), PathBuf::from("/home/x"));
        assert_eq!(pick_home(None, None), PathBuf::from("/root"), "no HOME at all is not the filesystem root");
        assert_eq!(pick_home(p(""), p("")), PathBuf::from("/root"), "empty values count as unset");
    }

    #[test]
    fn the_stack_entry_is_not_a_minimized_window() {
        assert_eq!(crate::shell::desktop_dock::dock_min_index(DOCK_STACK), None);
        assert_ne!(DOCK_STACK, crate::shell::desk::DOCK_TRASH);
    }

    #[test]
    fn fan_steps_up_and_right_from_the_front_tile() {
        assert_eq!(fan_offsets(0, 10), Vec::<(i32, i32)>::new());
        assert_eq!(fan_offsets(3, 10), vec![(0, 0), (10, -5), (20, -10)]);
    }

    #[test]
    fn recent_files_are_newest_first_and_skip_hidden_ones() {
        let dir = std::env::temp_dir().join(format!("bifrost-stack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        let new = std::time::UNIX_EPOCH + std::time::Duration::from_secs(2_000_000);
        std::fs::write(dir.join("old.pdf"), b"a").unwrap();
        std::fs::write(dir.join("new.zip"), b"b").unwrap();
        std::fs::write(dir.join(".hidden"), b"c").unwrap();
        std::fs::File::options().write(true).open(dir.join("old.pdf")).unwrap().set_modified(old).unwrap();
        std::fs::File::options().write(true).open(dir.join("new.zip")).unwrap().set_modified(new).unwrap();
        assert_eq!(recent_files(&dir, 5), vec!["new.zip".to_string(), "old.pdf".to_string()]);
        assert_eq!(recent_files(&dir, 1), vec!["new.zip".to_string()]);
        assert!(recent_files(&dir.join("missing"), 3).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn recent_items_know_their_folders_and_paths() {
        let dir = std::env::temp_dir().join(format!("bifrost-stack-items-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Album")).unwrap();
        std::fs::write(dir.join("notes.txt"), b"a").unwrap();
        let items = recent_items(&dir, 5);
        assert_eq!(items.len(), 2);
        let album = items.iter().find(|i| i.name == "Album").unwrap();
        assert!(album.dir && album.path == dir.join("Album"));
        let notes = items.iter().find(|i| i.name == "notes.txt").unwrap();
        assert!(!notes.dir && notes.path == dir.join("notes.txt"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_page_shows_its_kind() {
        assert_eq!(badge("report.pdf").as_deref(), Some("PDF"));
        assert_eq!(badge("song.flac").as_deref(), Some("FLAC"));
        assert_eq!(badge("archive.tar.zstd").as_deref(), Some("ZSTD"));
        assert_eq!(badge("movie.webmx1").as_deref(), Some("WEBM"), "at most four letters");
        assert_eq!(badge("README"), None);
        assert_eq!(badge(".bashrc"), None, "a dot file has no kind");
        assert_eq!(badge("odd.ex-t"), None);
    }

    const PT: f32 = 2.0;
    /// A 1280 x 800 pt screen at 2 px per pt, the menu bar 28 pt, and the stack's icon (48 pt) near the right end of a
    /// bottom Dock.
    fn screen() -> (R, R) {
        let room = (0.0, 28.0 * PT, 1280.0 * PT, (800.0 - 28.0) * PT);
        let icon = (1100.0 * PT, 744.0 * PT, 48.0 * PT, 48.0 * PT);
        (room, icon)
    }

    fn within(outer: R, inner: R) -> bool {
        inner.0 >= outer.0 - 0.01 && inner.1 >= outer.1 - 0.01 && inner.0 + inner.2 <= outer.0 + outer.2 + 0.01 && inner.1 + inner.3 <= outer.1 + outer.3 + 0.01
    }

    fn overlap(a: R, b: R) -> bool {
        a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
    }

    fn centre(r: R) -> (f32, f32) {
        (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0)
    }

    /// Automatic, as the reference's default: a fan for a few items over a bottom Dock, a grid for more, and a grid
    /// beside a side Dock.
    #[test]
    fn the_stack_fans_a_few_items_and_grids_many() {
        let (room, icon) = screen();
        for n in [0, 1, 5, FAN_MAX] {
            assert_eq!(layout(n, Side::Above, icon, room, PT).style, Style::Fan, "{n} items");
        }
        assert_eq!(layout(FAN_MAX + 1, Side::Above, icon, room, PT).style, Style::Grid);
        assert_eq!(layout(3, Side::Right, (8.0, 700.0, 96.0, 96.0), room, PT).style, Style::Grid, "a side Dock opens a grid");
    }

    #[test]
    fn the_fan_rises_from_the_icon_with_open_in_files_on_top() {
        let (room, icon) = screen();
        let l = layout(4, Side::Above, icon, room, PT);
        assert_eq!(l.cells.len(), 4);
        assert!(l.panel.1 + l.panel.3 <= icon.1, "the fan stays above its icon");
        for pair in l.cells.windows(2) {
            assert!(pair[1].1 + pair[1].3 <= pair[0].1 + 0.01, "each older item is higher: {pair:?}");
            assert!(pair[1].0 >= pair[0].0, "and bends right, not left");
        }
        assert!(l.open.1 + l.open.3 <= l.cells[3].1 + 0.01, "Open in Files is above the oldest item");
        // the newest item's icon (the right end of its rectangle) sits over the Dock icon
        let first = l.cells[0];
        assert!((first.0 + first.2 - FAN_ICON_PT * PT / 2.0 - centre(icon).0).abs() < 1.0);
        assert!(l.cells.iter().chain([&l.open]).all(|&r| within(l.panel, r) && within(room, r)));
        // a screen too short for the fan opens the grid instead, still above the icon
        let short = (0.0, 28.0 * PT, 1280.0 * PT, 300.0 * PT);
        let low = (1100.0 * PT, 270.0 * PT, 48.0 * PT, 48.0 * PT);
        let g = layout(FAN_MAX, Side::Above, low, short, PT);
        assert_eq!(g.style, Style::Grid);
        assert!(g.panel.1 + g.panel.3 <= low.1 && g.panel.1 >= short.1);
    }

    #[test]
    fn the_grid_sits_over_its_icon_inside_the_screen_and_keeps_to_the_room() {
        let (room, icon) = screen();
        let l = layout(20, Side::Above, icon, room, PT);
        assert_eq!(l.style, Style::Grid);
        assert_eq!(l.cells.len(), 20, "every item fits a tall screen");
        assert!(within(room, l.panel), "on the screen: {:?}", l.panel);
        assert!(l.panel.1 + l.panel.3 <= icon.1, "above the icon");
        assert!(l.panel.0 <= centre(icon).0 && centre(icon).0 <= l.panel.0 + l.panel.2, "over the icon (pushed in from the edge)");
        for (i, &a) in l.cells.iter().enumerate() {
            assert!(within(l.panel, a));
            assert!(!overlap(a, l.open), "the cells are clear of Open in Files");
            assert!(l.cells[i + 1..].iter().all(|&b| !overlap(a, b)), "cell {i} overlaps another");
        }
        // 20 items: 5 columns (about square), newest first along the top row
        assert_eq!(l.cells[1].1, l.cells[0].1);
        assert!(l.cells[1].0 > l.cells[0].0);
        assert!(l.cells[5].1 > l.cells[0].1 && l.cells[5].0 == l.cells[0].0);
        // a short screen shows the rows that fit; the rest stays in Files
        let short = (0.0, 28.0 * PT, 1280.0 * PT, 400.0 * PT);
        let low = (600.0 * PT, 370.0 * PT, 48.0 * PT, 48.0 * PT);
        let s = layout(POPOVER_MAX, Side::Above, low, short, PT);
        assert!(s.cells.len() < POPOVER_MAX && !s.cells.is_empty());
        assert!(s.panel.1 >= short.1 && s.panel.1 + s.panel.3 <= low.1);
        // beside a side Dock: right of a left Dock, left of a right Dock, both on the screen
        let left = layout(12, Side::Right, (8.0 * PT, 400.0 * PT, 48.0 * PT, 48.0 * PT), room, PT);
        assert!(left.panel.0 >= 56.0 * PT && within(room, left.panel));
        let right = layout(12, Side::Left, (1224.0 * PT, 400.0 * PT, 48.0 * PT, 48.0 * PT), room, PT);
        assert!(right.panel.0 + right.panel.2 <= 1224.0 * PT && within(room, right.panel));
    }

    #[test]
    fn a_click_lands_on_its_item_or_open_in_files_and_gaps_take_none() {
        let (room, icon) = screen();
        for n in [3, 20] {
            let l = layout(n, Side::Above, icon, room, PT);
            for (i, &r) in l.cells.iter().enumerate() {
                let (x, y) = centre(r);
                assert_eq!(hit(&l, x, y), Some(Hit::Item(i)), "{:?} item {i}", l.style);
            }
            let (x, y) = centre(l.open);
            assert_eq!(hit(&l, x, y), Some(Hit::Open));
            assert_eq!(hit(&l, l.panel.0 - 1.0, l.panel.1 - 1.0), None, "outside");
        }
        // the grid's title band is part of the panel but no item
        let g = layout(20, Side::Above, icon, room, PT);
        assert_eq!(hit(&g, g.panel.0 + 2.0, g.panel.1 + 2.0), None);
    }
}

/// The open stack on the desktop: it opens from the Dock icon as `Menu::Stack`, its popup is the layout's, it draws,
/// and Esc or a press outside closes it while a press on its gaps does not.
#[cfg(all(test, feature = "desktop-wayland"))]
mod shell_tests {
    use super::*;
    use crate::shell::desktop_bar::Menu;
    use crate::shell::Event;

    fn items(n: usize) -> Vec<Item> {
        (0..n).map(|i| Item { name: format!("file {i}.pdf"), path: PathBuf::from(format!("/nonexistent/file {i}.pdf")), dir: i == 0 }).collect()
    }

    #[test]
    fn the_stack_opens_over_its_icon_draws_and_closes() {
        let mut sh = crate::shell::tests::desktop_shell();
        // a 1280 x 800 pt screen at the test shell's scale
        let (sw, sh_h) = (sh.ui.pt(1280.0).round() as i32, sh.ui.pt(800.0).round() as i32);
        sh.w = sw;
        sh.h = sh_h;
        sh.screen = crate::shell::Screen::Home;
        sh.anim = None;
        {
            let d = sh.desk.as_mut().unwrap();
            d.wins.clear();
            d.dock_preferences.autohide = false;
            d.dock_preferences.magnification = false;
        }
        let m = sh.desk_metrics();
        let (list, _, rects) = sh.desk_dock(&m);
        let icon = rects[list.iter().position(|&e| e == DOCK_STACK).unwrap()];
        sh.stack_open(icon);
        assert_eq!(sh.desktop_menu, Some(Menu::Stack));
        assert_eq!(sh.desk.as_ref().unwrap().stack_icon, Some((icon.x as f32, icon.y as f32, icon.w as f32, icon.h as f32)));
        for (n, style) in [(3, Style::Fan), (20, Style::Grid)] {
            sh.desk.as_mut().unwrap().stack_items = items(n);
            let l = sh.stack_layout().unwrap();
            assert_eq!(l.style, style);
            assert!(l.panel.1 + l.panel.3 <= icon.y as f32, "above the Dock icon");
            let (x, y, w, h) = sh.desktop_popup_rect().expect("the stack is the open popup");
            assert_eq!((x, y), (l.panel.0 as usize, l.panel.1 as usize));
            assert!(w as f32 >= l.panel.2 && h as f32 >= l.panel.3);
            let (bw, bh) = (sw as usize, sh_h as usize);
            let mut buf = vec![0u32; bw * bh];
            sh.draw_desktop_popup(&mut Surface { w: bw, h: bh, stride: bw, px: &mut buf, y0: 0 });
            assert!(buf.iter().any(|&p| p != 0), "{style:?} draws");
        }
        // a press and release on the grid's title band keeps it open; Esc closes it
        let l = sh.stack_layout().unwrap();
        sh.desktop_bar_event(Event::Down(l.panel.0 + 4.0, l.panel.1 + 4.0), 10.0);
        sh.desktop_bar_event(Event::Up(l.panel.0 + 4.0, l.panel.1 + 4.0), 20.0);
        assert_eq!(sh.desktop_menu, Some(Menu::Stack));
        sh.desktop_menu_key(1, 30.0);
        assert_eq!(sh.desktop_menu, None);
        // a press outside closes it too, and is used up
        sh.stack_open(icon);
        assert!(sh.desktop_bar_event(Event::Down(4.0, sh_h as f32 / 2.0), 40.0).is_some());
        assert_eq!(sh.desktop_menu, None);
    }
}
