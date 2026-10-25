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
    downloads_in(None)
}

/// The Downloads folder under `home` (the document broker's home, which grants are made relative to), else under
/// `home_dir`. The open stack takes its home from the broker, so a shell whose `$HOME` differs from the account's
/// still lists files the broker can grant instead of showing every one in Files.
pub(super) fn downloads_in(home: Option<&Path>) -> PathBuf {
    home.filter(|h| !h.as_os_str().is_empty()).map_or_else(home_dir, Path::to_path_buf).join("Downloads")
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

/// The open stack's keyboard selection, as `desktop_menu_row` holds it for `Menu::Stack`: the items are 0.. (newest
/// first) and Open in Files is `cells.len()`.
fn target_of(l: &Layout, i: usize) -> Hit {
    if i >= l.cells.len() {
        Hit::Open
    } else {
        Hit::Item(i)
    }
}

/// The selection after an arrow key (`code`: 103 Up, 108 Down, 105 Left, 106 Right), from `cur` (`target_of`). With
/// nothing selected the first arrow selects the newest item. The fan runs upward: Up to the next older item, Open in
/// Files above the oldest; Down back. The grid moves by cell and by row; Down from the last row, or Right from the
/// last item, reaches Open in Files, and Up or Left from it the last item. It stops at the ends.
pub(super) fn step(l: &Layout, cur: Option<usize>, code: u16) -> Option<usize> {
    let n = l.cells.len();
    let Some(i) = cur.filter(|&i| i <= n) else { return Some(0) };
    let next = match l.style {
        Style::Fan => match code {
            103 => i + 1,
            108 => i.saturating_sub(1),
            _ => i,
        },
        Style::Grid if i == n => match code {
            103 | 105 => n.saturating_sub(1),
            _ => i,
        },
        Style::Grid => {
            // the columns: the cells on the first row
            let cols = l.cells.iter().take_while(|c| c.1 == l.cells[0].1).count().max(1);
            match code {
                105 => i.saturating_sub(1),
                106 => i + 1,
                103 if i >= cols => i - cols,
                108 if i + cols < n => i + cols,
                // Down on the last row reaches Open in Files; on the row above a shorter last row, its last item
                108 if i / cols == (n - 1) / cols => n,
                108 => n - 1,
                _ => i,
            }
        }
    };
    Some(next.min(n))
}

/// Type to select: the next item after `cur` (wrapping) whose name starts with `c`, ignoring case; None when no name
/// does.
pub(super) fn type_select(names: &[&str], cur: Option<usize>, c: char) -> Option<usize> {
    let n = names.len();
    let start = cur.map_or(0, |i| i + 1);
    (0..n).map(|k| (start + k) % n).find(|&i| names[i].chars().next().is_some_and(|f| f.to_lowercase().eq(c.to_lowercase())))
}

/// The modifier keys (Ctrl, Shift, Alt, Meta, Caps Lock): pressed alone they leave the stack open, since a chord's
/// modifier comes before its key.
const MODIFIER_KEYS: [u16; 9] = [29, 97, 42, 54, 56, 100, 125, 126, 58];

/// What a key does while the stack is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Key {
    /// Esc: closes the stack
    Close,
    /// an arrow: moves the selection (`step`)
    Move,
    /// Return or Enter: opens the selected item
    Choose,
    /// a letter or digit: selects by name (`type_select`)
    Type(char),
    /// a modifier alone: the stack stays open and the key goes on
    Wait,
    /// any other key, and every key with Ctrl, Alt or Cmd held: the stack closes and the key goes on (a shortcut,
    /// typing in the app in front)
    Pass,
}

/// The use of the key `code` while the stack is open; `chord` is true with Ctrl, Alt or Cmd held.
pub(super) fn key_use(code: u16, chord: bool) -> Key {
    if MODIFIER_KEYS.contains(&code) {
        return Key::Wait;
    }
    if chord {
        return Key::Pass;
    }
    match code {
        1 => Key::Close,
        103 | 105 | 106 | 108 => Key::Move,
        28 | 96 => Key::Choose,
        _ => super::evkeys::char_for(code, false).filter(|c| c.is_alphanumeric()).map_or(Key::Pass, Key::Type),
    }
}

/// A fan item's icon in its rectangle `cell` (the icon is its right end), `size` px square.
fn fan_icon((x, y, w, _): R, size: f32) -> R {
    (x + w - size, y, size, size)
}

/// The fan's Open in Files button in its rectangle `open`, `b` px square.
fn fan_open((x, y, w, h): R, b: f32) -> R {
    (x + w - b, y, b, h)
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
        let items = recent_items(&self.stack_downloads(), POPOVER_MAX);
        let Some(d) = self.desk.as_mut() else { return };
        d.stack_items = items;
        d.stack_icon = Some((icon.x as f32, icon.y as f32, icon.w as f32, icon.h as f32));
        self.desktop_menu = Some(super::desktop_bar::Menu::Stack);
        self.desktop_menu_row = None;
        self.dirty = true;
    }

    /// The Downloads folder the open stack lists: under the document broker's home, the one grants are made relative to
    /// (`downloads_in`).
    fn stack_downloads(&mut self) -> PathBuf {
        let home = self.doc_broker().map(|b| b.opener.home().to_path_buf());
        downloads_in(home.as_deref())
    }

    /// Closes the open stack and drops its keyboard selection.
    fn stack_close(&mut self) {
        self.desktop_menu = None;
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

    /// A fan name capsule ending just left of its icon `r`: the name as shown (one line, cut to the room) and the
    /// capsule's rectangle (px).
    fn fan_capsule(&self, r: R, text: &str) -> (String, R) {
        use super::ui::{Weight, MAC_CALLOUT_PT};
        let ui = &self.ui;
        let room = ui.pt(FAN_LABEL_PT - 12.0);
        let text = ui.wrap_capped(text, MAC_CALLOUT_PT, Weight::Regular, room - ui.pt(16.0), 1).into_iter().next().unwrap_or_default();
        let (cw, ch) = (self.measure_chrome(&text, MAC_CALLOUT_PT, Weight::Regular) + ui.pt(16.0), ui.pt(22.0));
        (text, (r.0 - ui.pt(6.0) - cw, r.1 + (r.3 - ch) / 2.0, cw, ch))
    }

    /// The parts of the open fan that take the pointer (px): each item's icon and name capsule, and Open in Files's
    /// button and capsule. The room between them is clear (drawn transparent) and passes the pointer on.
    fn fan_targets(&self, l: &Layout) -> Vec<(Hit, R)> {
        let size = self.ui.pt(FAN_ICON_PT);
        let items = self.desk.as_ref().map_or(&[][..], |d| &d.stack_items[..]);
        let mut out = Vec::with_capacity(2 * l.cells.len() + 2);
        for (i, (&cell, item)) in l.cells.iter().zip(items).enumerate() {
            let icon = fan_icon(cell, size);
            out.push((Hit::Item(i), icon));
            out.push((Hit::Item(i), self.fan_capsule(icon, &item.name).1));
        }
        let open = fan_open(l.open, self.ui.pt(FAN_OPEN_PT));
        out.push((Hit::Open, open));
        out.push((Hit::Open, self.fan_capsule(open, "Open in Files").1));
        out
    }

    /// The item or Open in Files of the open stack under (`x`, `y`) screen px: the grid's cells, the fan's parts.
    fn stack_hit(&self, l: &Layout, x: f32, y: f32) -> Option<Hit> {
        match l.style {
            Style::Grid => hit(l, x, y),
            Style::Fan => self.fan_targets(l).into_iter().find(|&(_, r)| inside(r, x, y)).map(|(h, _)| h),
        }
    }

    /// Whether the open stack takes the pointer at (`x`, `y`) screen px: anywhere on the grid's panel (its gaps keep
    /// it open), on the fan's icons and capsules only.
    pub(super) fn stack_takes(&self, x: f32, y: f32) -> bool {
        let Some(l) = self.stack_layout() else { return false };
        match l.style {
            Style::Grid => inside(l.panel, x, y),
            Style::Fan => self.stack_hit(&l, x, y).is_some(),
        }
    }

    /// Whether (`x`, `y`) is on the stack's own Dock icon, at rest or as drawn now: a press there closes the stack and
    /// is used up, so it does not open again.
    pub(super) fn stack_on_icon(&self, x: f32, y: f32) -> bool {
        if self.desk.as_ref().and_then(|d| d.stack_icon).is_some_and(|r| inside(r, x, y)) {
            return true;
        }
        let m = self.desk_metrics();
        let (list, _, rects) = self.desk_dock(&m);
        list.iter().position(|&e| e == DOCK_STACK).and_then(|i| rects.get(i)).is_some_and(|r| inside((r.x as f32, r.y as f32, r.w as f32, r.h as f32), x, y))
    }

    /// The input region of the popup surface at screen (`x0`, `y0`) while the stack opens as a fan: its parts
    /// (`fan_targets`) in surface px. Empty otherwise (the popup's own rect is used).
    pub(super) fn stack_input_shape(&self, x0: i32, y0: i32) -> Vec<[i32; 4]> {
        if self.desktop_menu != Some(super::desktop_bar::Menu::Stack) {
            return Vec::new();
        }
        let Some(l) = self.stack_layout().filter(|l| l.style == Style::Fan) else { return Vec::new() };
        self.fan_targets(&l)
            .into_iter()
            .map(|(_, (x, y, w, h))| {
                let (left, top) = (x.floor() as i32, y.floor() as i32);
                [left - x0, top - y0, (x + w).ceil() as i32 - left, (y + h).ceil() as i32 - top]
            })
            .collect()
    }

    /// A release on the open stack at (`x`, `y`) screen px: an item opens with its default app, Open in Files opens the
    /// folder; either closes the stack. A release on the grid's gaps keeps it open.
    pub(super) fn stack_click(&mut self, x: f32, y: f32, t: f64) {
        let Some(l) = self.stack_layout() else { return };
        if let Some(target) = self.stack_hit(&l, x, y) {
            self.stack_choose(target, t);
        }
    }

    /// Opens `target` of the open stack and closes it.
    fn stack_choose(&mut self, target: Hit, t: f64) {
        self.stack_close();
        match target {
            Hit::Open => {
                let folder = self.stack_downloads();
                self.stack_reveal(&folder, t);
            }
            Hit::Item(i) => {
                if let Some(item) = self.desk.as_ref().and_then(|d| d.stack_items.get(i).cloned()) {
                    self.stack_open_item(&item, t);
                }
            }
        }
    }

    /// A key while the stack is open (`key_use`): Esc closes it, the arrows move its selection, Return opens the
    /// selected item, a letter selects by name. A modifier alone waits; any other key closes the stack and goes on
    /// (None), as a shortcut or typing for the app in front.
    pub(super) fn stack_key(&mut self, code: u16, t: f64) -> Option<Vec<super::Effect>> {
        use super::desktop_bar::Menu;
        use bifrost_proto::ModifierFlags as F;
        let chord = [F::CONTROL, F::OPTION, F::COMMAND].into_iter().any(|f| self.key_mods.contains(f));
        let cur = self.menu_row_of(Menu::Stack);
        match key_use(code, chord) {
            Key::Wait => return None,
            Key::Pass => {
                self.stack_close();
                return None;
            }
            Key::Close => self.stack_close(),
            Key::Move => {
                if let Some(l) = self.stack_layout() {
                    self.desktop_menu_row = step(&l, cur, code).map(|i| (Menu::Stack, i));
                    self.dirty = true;
                }
            }
            Key::Choose => {
                if let (Some(i), Some(l)) = (cur, self.stack_layout()) {
                    self.stack_choose(target_of(&l, i), t);
                }
            }
            Key::Type(c) => {
                let shown = self.stack_layout().map_or(0, |l| l.cells.len());
                let pick = self.desk.as_ref().and_then(|d| {
                    let names: Vec<&str> = d.stack_items.iter().take(shown).map(|i| i.name.as_str()).collect();
                    type_select(&names, cur, c)
                });
                if let Some(i) = pick {
                    self.desktop_menu_row = Some((Menu::Stack, i));
                    self.dirty = true;
                }
            }
        }
        Some(vec![])
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
        let folder = item.path.parent().map_or_else(|| self.stack_downloads(), Path::to_path_buf);
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
    /// (`draw_menu_level` drew it); the fan has no panel: its icons and name capsules float over the desktop. The
    /// keyboard selection (`stack_key`) is drawn on the menu highlight.
    pub(super) fn draw_stack_popover(&self, s: &mut Surface) {
        use super::ui::{Weight, LABEL, LINK, MAC_BODY_PT, MAC_CALLOUT_PT, MAC_FOOTNOTE_PT, MAC_HEADLINE_PT, MAC_SUBHEAD_PT, MENU_HIGHLIGHT, MENU_HIGHLIGHT_TEXT, MENU_SOLID, SECONDARY, SEPARATOR_C};
        use bifrost_ui::prefs_layout::m;
        let (Some(l), Some(d)) = (self.stack_layout(), self.desk.as_ref()) else { return };
        let ui = &self.ui;
        let edge = SEPARATOR_C.now();
        let sel = self.menu_row_of(super::desktop_bar::Menu::Stack).map(|i| target_of(&l, i));
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
                let radius = ui.pt(m::SELECTION_RADIUS);
                for (i, (cell, item)) in l.cells.iter().zip(&d.stack_items).enumerate() {
                    let (cx, cy, cw, ch) = *cell;
                    let chosen = sel == Some(Hit::Item(i));
                    if chosen {
                        let inset = ui.pt(2.0);
                        s.fill_rrect(cx + inset, cy + inset, cw - 2.0 * inset, ch - 2.0 * inset, radius, MENU_HIGHLIGHT);
                    }
                    let top = cy + ui.pt(4.0);
                    icon(s, (cx + (cw - size) / 2.0, top, size, size), item);
                    let text_c = if chosen { MENU_HIGHLIGHT_TEXT } else { LABEL.now() };
                    for (k, text) in ui.wrap_capped(&item.name, MAC_SUBHEAD_PT, Weight::Regular, cw - ui.pt(6.0), 2).iter().enumerate() {
                        self.label(text, MAC_SUBHEAD_PT, Weight::Regular, text_c, |im| {
                            s.blit(im, (cx + (cw - im.w as f32) / 2.0) as i32, (top + size + ui.pt(4.0) + line * k as f32) as i32, 255, 255)
                        });
                    }
                }
                let (ox, oy, ow, oh) = l.open;
                s.fill_rect(ox as usize, oy as usize, ow as usize, 1, edge);
                let open_c = if sel == Some(Hit::Open) {
                    let inset = ui.pt(4.0);
                    s.fill_rrect(ox - inset, oy + inset, ow + 2.0 * inset, oh - 2.0 * inset, radius, MENU_HIGHLIGHT);
                    MENU_HIGHLIGHT_TEXT
                } else {
                    LINK
                };
                self.label("Open in Files", MAC_BODY_PT, Weight::Regular, open_c, |im| s.blit(im, ox as i32, (oy + oh / 2.0 - im.h as f32 / 2.0) as i32, 255, 255));
            }
            Style::Fan => {
                if bifrost_render::glass::gpu() {
                    let (x, y, w, h) = l.panel;
                    s.fill_rect(x.max(0.0) as usize, y.max(0.0) as usize, w as usize, h as usize, 0);
                }
                // a name in a capsule ending just left of its icon `r`, centred on it (`fan_capsule`, which the pointer
                // takes too); the selected one on the menu highlight
                let capsule = |s: &mut Surface, r: R, text: &str, color: u32, chosen: bool| {
                    let (text, (cx, cy, cw, ch)) = self.fan_capsule(r, text);
                    let (fill, color) = if chosen { (MENU_HIGHLIGHT, MENU_HIGHLIGHT_TEXT) } else { (MENU_SOLID.now(), color) };
                    s.fill_rrect(cx, cy, cw, ch, ch / 2.0, fill);
                    self.label(&text, MAC_CALLOUT_PT, Weight::Regular, color, |im| s.blit(im, (cx + ui.pt(8.0)) as i32, (cy + (ch - im.h as f32) / 2.0) as i32, 255, 255));
                };
                let size = ui.pt(FAN_ICON_PT);
                for (i, (cell, item)) in l.cells.iter().zip(&d.stack_items).enumerate() {
                    let r = fan_icon(*cell, size);
                    icon(s, r, item);
                    capsule(s, r, &item.name, LABEL.now(), sel == Some(Hit::Item(i)));
                }
                let b = ui.pt(FAN_OPEN_PT);
                let r = fan_open(l.open, b);
                s.fill_rrect(r.0, r.1, b, b, b / 2.0, MENU_SOLID.now());
                if let Some(im) = ui.symbols.render(crate::symbols::Symbol::ChevronRight, ui.pt(13.0), LINK) {
                    s.blit(&im, (r.0 + (b - im.w as f32) / 2.0) as i32, (r.1 + (b - im.h as f32) / 2.0) as i32, 255, 255);
                }
                capsule(s, r, "Open in Files", LINK, sel == Some(Hit::Open));
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

    /// The fan's arrows run up its items to Open in Files and back; the grid's move by cell and by row, Down from the
    /// last row reaching Open in Files. The first arrow selects the newest item, and the ends hold.
    #[test]
    fn the_arrows_walk_the_fan_and_the_grid() {
        let (room, icon) = screen();
        let fan = layout(3, Side::Above, icon, room, PT);
        assert_eq!(fan.style, Style::Fan);
        assert_eq!(step(&fan, None, 103), Some(0), "the first arrow selects the newest item");
        assert_eq!(step(&fan, None, 108), Some(0));
        assert_eq!(step(&fan, Some(0), 103), Some(1), "Up goes to the older item above");
        assert_eq!(step(&fan, Some(2), 103), Some(3), "Open in Files is above the oldest");
        assert_eq!(target_of(&fan, 3), Hit::Open);
        assert_eq!(step(&fan, Some(3), 103), Some(3), "the top holds");
        assert_eq!(step(&fan, Some(1), 108), Some(0));
        assert_eq!(step(&fan, Some(0), 108), Some(0), "the bottom holds");
        assert_eq!(step(&fan, Some(1), 105), Some(1), "the fan has no sideways");
        // 20 items: 5 columns, 4 rows
        let grid = layout(20, Side::Above, icon, room, PT);
        assert_eq!(grid.style, Style::Grid);
        assert_eq!(step(&grid, Some(0), 106), Some(1));
        assert_eq!(step(&grid, Some(1), 105), Some(0));
        assert_eq!(step(&grid, Some(0), 105), Some(0));
        assert_eq!(step(&grid, Some(2), 108), Some(7), "Down is the next row");
        assert_eq!(step(&grid, Some(7), 103), Some(2));
        assert_eq!(step(&grid, Some(2), 103), Some(2), "the top row holds");
        assert_eq!(step(&grid, Some(17), 108), Some(20), "Down from the last row is Open in Files");
        assert_eq!(step(&grid, Some(19), 106), Some(20));
        assert_eq!(step(&grid, Some(20), 103), Some(19), "Up from Open in Files is the last item");
        assert_eq!(step(&grid, Some(20), 108), Some(20));
        // 7 items in 3 columns: Down from the middle row's end goes to the short last row's last item
        let short = layout(7, Side::Right, (8.0 * PT, 400.0 * PT, 48.0 * PT, 48.0 * PT), room, PT);
        assert_eq!(short.style, Style::Grid);
        assert_eq!(step(&short, Some(5), 108), Some(6));
        assert_eq!(step(&short, Some(4), 108), Some(6));
        assert_eq!(step(&short, Some(6), 108), Some(7), "the last row reaches Open in Files");
        // no items: Open in Files is all there is
        let empty = layout(0, Side::Right, (8.0 * PT, 400.0 * PT, 48.0 * PT, 48.0 * PT), room, PT);
        assert_eq!(step(&empty, None, 108), Some(0));
        assert_eq!(target_of(&empty, 0), Hit::Open);
        assert_eq!(step(&empty, Some(0), 103), Some(0));
    }

    #[test]
    fn typing_selects_the_next_item_by_its_first_letter() {
        let names = ["Report.pdf", "photo.png", "readme.txt", "2026 plan.key"];
        assert_eq!(type_select(&names, None, 'r'), Some(0), "case does not matter");
        assert_eq!(type_select(&names, Some(0), 'r'), Some(2), "the same letter again moves on");
        assert_eq!(type_select(&names, Some(2), 'r'), Some(0), "and wraps");
        assert_eq!(type_select(&names, Some(4), 'p'), Some(1), "from Open in Files too");
        assert_eq!(type_select(&names, None, '2'), Some(3));
        assert_eq!(type_select(&names, Some(1), 'z'), None, "no match keeps the selection");
        assert_eq!(type_select(&[], None, 'a'), None);
    }

    /// The stack keeps Esc, its arrows, Return and typing; a modifier alone waits for its key; every other key, and
    /// every chord, closes the stack and goes on.
    #[test]
    fn the_stack_takes_only_its_own_keys() {
        assert_eq!(key_use(1, false), Key::Close);
        for code in [103, 105, 106, 108] {
            assert_eq!(key_use(code, false), Key::Move);
        }
        assert_eq!(key_use(28, false), Key::Choose);
        assert_eq!(key_use(96, false), Key::Choose, "keypad Enter");
        assert_eq!(key_use(30, false), Key::Type('a'));
        assert_eq!(key_use(2, false), Key::Type('1'));
        for code in [29, 42, 56, 125] {
            assert_eq!(key_use(code, false), Key::Wait, "modifier {code}");
            assert_eq!(key_use(code, true), Key::Wait);
        }
        assert_eq!(key_use(57, false), Key::Pass, "Space");
        assert_eq!(key_use(15, false), Key::Pass, "Tab");
        assert_eq!(key_use(59, false), Key::Pass, "F1");
        assert_eq!(key_use(30, true), Key::Pass, "Cmd+A is the app's");
        assert_eq!(key_use(1, true), Key::Pass);
        assert_eq!(key_use(103, true), Key::Pass);
    }

    /// The stack lists Downloads under the broker's home, whatever `$HOME` says, so the files it lists can be granted.
    #[test]
    fn the_stack_takes_its_downloads_from_the_brokers_home() {
        assert_eq!(downloads_in(Some(Path::new("/home/ola"))), PathBuf::from("/home/ola/Downloads"));
        assert_eq!(downloads_in(Some(Path::new(""))), downloads_dir(), "an empty home counts as none");
        assert_eq!(downloads_in(None), home_dir().join("Downloads"));
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
        // a press outside closes it and goes on to what is under it, as in the reference
        sh.stack_open(icon);
        assert!(sh.desktop_bar_event(Event::Down(4.0, sh_h as f32 / 2.0), 40.0).is_none(), "the press is not used up");
        assert_eq!(sh.desktop_menu, None);
        // a press on its own Dock icon only closes it
        sh.stack_open(icon);
        let (ix, iy) = (icon.x as f32 + icon.w as f32 / 2.0, icon.y as f32 + icon.h as f32 / 2.0);
        assert!(sh.desktop_bar_event(Event::Down(ix, iy), 50.0).is_some());
        assert_eq!(sh.desktop_menu, None);
    }

    /// A desktop shell on a 1280 x 800 pt screen with the stack open over its Dock icon, listing `n` items.
    fn open_stack(n: usize) -> (crate::shell::Shell, Rect) {
        let mut sh = crate::shell::tests::desktop_shell();
        sh.w = sh.ui.pt(1280.0).round() as i32;
        sh.h = sh.ui.pt(800.0).round() as i32;
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
        sh.desk.as_mut().unwrap().stack_items = items(n);
        (sh, icon)
    }

    /// The fan clears its bounding box: only its icons and capsules take the pointer, on the popup's input region as
    /// in the shell's own hit test, and a press in the clear room closes the stack and goes on.
    #[test]
    fn the_fan_takes_the_pointer_on_its_parts_only() {
        let (mut sh, _) = open_stack(3);
        let l = sh.stack_layout().unwrap();
        assert_eq!(l.style, Style::Fan);
        let size = sh.ui.pt(FAN_ICON_PT);
        let (ix, iy, iw, ih) = fan_icon(l.cells[1], size);
        assert!(sh.stack_takes(ix + iw / 2.0, iy + ih / 2.0), "an item's icon");
        let (cx, cy, _, ch) = sh.fan_capsule(fan_icon(l.cells[1], size), "file 1.pdf").1;
        assert!(sh.stack_takes(cx + 2.0, cy + ch / 2.0), "its name capsule");
        let (ox, oy, ow, oh) = fan_open(l.open, sh.ui.pt(FAN_OPEN_PT));
        assert!(sh.stack_takes(ox + ow / 2.0, oy + oh / 2.0), "Open in Files");
        // the far left of the newest item's row is inside the fan's box but clear of its capsule
        let gap = (l.cells[0].0 + 1.0, l.cells[0].1 + l.cells[0].3 / 2.0);
        assert!(inside(l.panel, gap.0, gap.1) && !sh.stack_takes(gap.0, gap.1));
        // the popup surface's input region is those parts, in surface px
        let (sx, sy) = (l.panel.0 as i32 - 10, l.panel.1 as i32 - 10);
        let shape = sh.stack_input_shape(sx, sy);
        assert_eq!(shape.len(), 2 * 3 + 2, "an icon and a capsule per item and for Open in Files");
        let covers = |px: f32, py: f32| shape.iter().any(|r| inside((r[0] as f32, r[1] as f32, r[2] as f32, r[3] as f32), px - sx as f32, py - sy as f32));
        assert!(covers(ix + iw / 2.0, iy + ih / 2.0) && !covers(gap.0, gap.1));
        // the grid keeps the popup's own rect
        sh.desk.as_mut().unwrap().stack_items = items(20);
        assert!(sh.stack_input_shape(0, 0).is_empty());
        let g = sh.stack_layout().unwrap();
        assert!(sh.stack_takes(g.panel.0 + 2.0, g.panel.1 + 2.0), "the grid's title band keeps it open");
        // back to the fan: a press in its clear room closes it and is not used up
        sh.desk.as_mut().unwrap().stack_items = items(3);
        assert!(sh.desktop_bar_event(Event::Down(gap.0, gap.1), 10.0).is_none());
        assert_eq!(sh.desktop_menu, None);
        assert!(sh.stack_input_shape(0, 0).is_empty(), "no stack, no shape");
    }

    /// Esc closes the stack; the arrows and typing select, Return opens the selection; any other key and every chord
    /// closes it and goes on (`hw_menu_key` answers None), so shortcuts and the app in front still get their keys.
    #[test]
    fn the_open_stack_lets_other_keys_through() {
        use crate::shell::desktop_bar::Menu;
        let (mut sh, _) = open_stack(3);
        assert_eq!(sh.desktop_menu_key(103, 1.0), Some(vec![]));
        assert_eq!(sh.menu_row_of(Menu::Stack), Some(0), "the first arrow selects the newest item");
        assert_eq!(sh.desktop_menu_key(103, 2.0), Some(vec![]));
        assert_eq!(sh.menu_row_of(Menu::Stack), Some(1));
        // typing: names are "file 0.pdf".. so f moves to the next one
        assert_eq!(sh.desktop_menu_key(33, 3.0), Some(vec![]));
        assert_eq!(sh.menu_row_of(Menu::Stack), Some(2));
        assert_eq!(sh.desktop_menu, Some(Menu::Stack));
        // the selection draws
        let (bw, bh) = (sh.w as usize, sh.h as usize);
        let mut buf = vec![0u32; bw * bh];
        sh.draw_desktop_popup(&mut Surface { w: bw, h: bh, stride: bw, px: &mut buf, y0: 0 });
        assert!(buf.iter().any(|&p| p != 0));
        // a modifier alone keeps it open and goes on
        assert_eq!(sh.hw_menu_key(125, true, 4.0), None);
        assert_eq!(sh.desktop_menu, Some(Menu::Stack));
        // Space closes it and goes on
        assert_eq!(sh.hw_menu_key(57, true, 5.0), None);
        assert_eq!(sh.desktop_menu, None);
        assert_eq!(sh.menu_row_of(Menu::Stack), None, "the selection goes with it");
        // a chord closes it and goes on, even on a letter the stack would type
        sh.stack_open(sh.desk.as_ref().unwrap().stack_icon.map(|(x, y, w, h)| Rect { x: x as i32, y: y as i32, w: w as i32, h: h as i32 }).unwrap());
        sh.desk.as_mut().unwrap().stack_items = items(3);
        sh.set_key_mods(bifrost_proto::ModifierFlags::COMMAND);
        assert_eq!(sh.hw_menu_key(33, true, 6.0), None);
        assert_eq!(sh.desktop_menu, None);
        sh.set_key_mods(bifrost_proto::ModifierFlags::EMPTY);
        // Return opens the selection (item 0 is a folder: it opens in Files) and closes the stack
        sh.stack_open(sh.desk.as_ref().unwrap().stack_icon.map(|(x, y, w, h)| Rect { x: x as i32, y: y as i32, w: w as i32, h: h as i32 }).unwrap());
        sh.desk.as_mut().unwrap().stack_items = items(3);
        assert_eq!(sh.desktop_menu_key(28, 7.0), Some(vec![]), "Return with nothing selected waits");
        assert_eq!(sh.desktop_menu, Some(Menu::Stack));
        sh.desktop_menu_key(108, 8.0);
        assert_eq!(sh.desktop_menu_key(28, 9.0), Some(vec![]));
        assert_eq!(sh.desktop_menu, None);
        // Esc closes it
        sh.stack_open(sh.desk.as_ref().unwrap().stack_icon.map(|(x, y, w, h)| Rect { x: x as i32, y: y as i32, w: w as i32, h: h as i32 }).unwrap());
        assert_eq!(sh.desktop_menu_key(1, 10.0), Some(vec![]));
        assert_eq!(sh.desktop_menu, None);
    }

    /// A picture whose default app is the built-in Photos opens in its viewer, outside the library too; a built-in
    /// app that takes no file answers false, so the stack shows the file in Files.
    #[test]
    fn a_file_for_a_built_in_app_opens_in_it() {
        let mut sh = crate::shell::tests::desktop_shell();
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-stack-builtin")).join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Downloads")).unwrap();
        sh.data_dir = dir.clone();
        sh.photo_dirs = vec![dir.join("Pictures")];
        let png = dir.join("Downloads/shot.png");
        std::fs::write(&png, crate::cg::imageio::tests::png_bytes(4, 4, [10, 20, 30, 255])).unwrap();
        assert!(sh.open_path_with("bifrost.photos", &png, 1.0), "Photos opens it");
        assert_eq!(sh.photos.as_ref().and_then(|p| p.viewing()).map(|p| p.path.clone()), Some(png.clone()));
        assert!(!sh.open_path_with("bifrost.photos", &dir.join("Downloads/missing.png"), 2.0));
        assert!(!sh.open_path_with("no.such.app", &png, 3.0));
        let terminal = crate::shell::apps::apps().iter().find(|a| a.id == crate::shell::apps::AppId::Terminal).map(|a| a.icon).unwrap();
        assert!(!sh.open_path_with(terminal, &png, 4.0), "a built-in that takes no file");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
