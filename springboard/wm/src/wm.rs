//! Window-management rules: placement, stacking, focus, move and resize grabs, maximize and minimize.
//! Coordinates are logical output pixels; `area` is the output minus exclusive zones (the menu bar).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect { pub x: i32, pub y: i32, pub w: i32, pub h: i32 }

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self { Rect { x, y, w, h } }
    pub fn contains(&self, x: i32, y: i32) -> bool { x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h }
}

pub type WindowId = u64;

use crate::tiling::{self, Layout, Placement};

/// Which edges a resize grab moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Edges { pub left: bool, pub right: bool, pub top: bool, pub bottom: bool }

/// Offset of each newly placed window from the previous one.
pub const CASCADE: i32 = 28;
/// How much of a window always stays inside the usable area, so it can be grabbed back.
pub const KEEP_VISIBLE: i32 = 48;
/// The smallest frame a resize grab leaves.
pub const MIN_W: i32 = 240;
pub const MIN_H: i32 = 160;

/// The stacking band of a window. A floating window stays above every normal window (its app asked for it: the
/// Calculator's Always on Top); menus, panels and the menu bar are drawn above both and are not modelled here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, PartialOrd, Ord)]
pub enum Level { #[default] Normal, Floating }

struct Window { id: WindowId, rect: Rect, restore: Option<Rect>, tiled: Option<Layout>, minimized: bool, level: Level, full: Option<Saved> }

/// What a window held when it went full screen: the frame, the frame from before its maximize or tile, and its tile.
/// Leaving full screen puts all three back, fitted to the usable area as it is then.
#[derive(Clone, Copy)]
struct Saved { rect: Rect, restore: Option<Rect>, tiled: Option<Layout> }

/// An interactive move or resize in progress; `Wm::motion` applies the pointer to it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Grab {
    Move { id: WindowId, dx: i32, dy: i32 },
    Resize { id: WindowId, edges: Edges, x: i32, y: i32, start: Rect },
}

/// The window stack of one output, bottom first. The stack is kept sorted by level: every normal window is below
/// every floating window, and a window raised or set to a level goes to the top of its own band.
pub struct Wm {
    area: Rect,
    /// The whole output, menu bar included: what a full-screen window fills.
    output: Rect,
    windows: Vec<Window>,
    /// the window the person last raised or made key
    key: Option<WindowId>,
}

impl Wm {
    pub fn new(area: Rect) -> Self { Wm { area, output: area, windows: Vec::new(), key: None } }

    /// The whole output, menu bar included.
    pub fn output(&self) -> Rect { self.output }

    /// The whole output changed (mode, rotation, scale): a full-screen window fills it.
    pub fn set_output(&mut self, output: Rect) {
        self.output = output;
        for w in &mut self.windows {
            if w.full.is_some() { w.rect = output; }
        }
    }

    /// Puts `w` at the top of its level band: after every window of its level or below.
    fn insert_top(&mut self, w: Window) {
        let at = self.windows.iter().position(|o| o.level > w.level).unwrap_or(self.windows.len());
        self.windows.insert(at, w);
    }

    /// Places a new `w` x `h` window: centred, cascaded by the windows already shown, fitted inside the area. It is
    /// raised and focused.
    pub fn map(&mut self, id: WindowId, w: i32, h: i32) -> Rect { self.map_at(id, w, h, None) }

    /// Like `map`, but a window the shell brings back (its toplevel was destroyed while Overview or a minimize hid it)
    /// asks for the position it had, `origin`: the cascade is skipped and the frame is only kept on screen the way a
    /// drag keeps it (`keep_on_screen`), so the window returns exactly where the person left it.
    pub fn map_at(&mut self, id: WindowId, w: i32, h: i32, origin: Option<(i32, i32)>) -> Rect {
        let a = self.area;
        let (w, h) = (w.min(a.w), h.min(a.h));
        let step = CASCADE * self.windows.iter().filter(|w| !w.minimized).count() as i32;
        let (x, y) = match origin {
            // the rule a drag follows: a window may hang off the edges while `KEEP_VISIBLE` of it stays on screen
            Some((ox, oy)) => {
                let r = keep_on_screen(Rect::new(ox, oy, w, h), a);
                (r.x, r.y)
            }
            None => (
                (a.x + (a.w - w) / 2 + step).min(a.x + a.w - w),
                (a.y + (a.h - h) / 2 + step).min(a.y + a.h - h),
            ),
        };
        let rect = Rect::new(x, y, w, h);
        let level = self.level(id).unwrap_or_default();
        self.windows.retain(|w| w.id != id);
        self.insert_top(Window { id, rect, restore: None, tiled: None, minimized: false, level, full: None });
        self.key = Some(id);
        rect
    }

    pub fn unmap(&mut self, id: WindowId) {
        self.windows.retain(|w| w.id != id);
        if self.key == Some(id) { self.key = None; }
    }

    /// Ids bottom first, minimized windows included.
    pub fn stack(&self) -> Vec<WindowId> { self.windows.iter().map(|w| w.id).collect() }

    pub fn rect(&self, id: WindowId) -> Option<Rect> { self.get(id).map(|w| w.rect) }

    pub fn visible(&self, id: WindowId) -> bool { self.get(id).is_some_and(|w| !w.minimized) }

    /// The window with keyboard focus: the one last raised, while it is shown; otherwise the topmost shown window.
    pub fn focused(&self) -> Option<WindowId> {
        self.key.filter(|&k| self.visible(k)).or_else(|| self.windows.iter().rev().find(|w| !w.minimized).map(|w| w.id))
    }

    /// The stacking level of a window.
    pub fn level(&self, id: WindowId) -> Option<Level> { self.get(id).map(|w| w.level) }

    /// Moves a window to `level`, to the top of that band. The keyboard focus does not change.
    pub fn set_level(&mut self, id: WindowId, level: Level) {
        if let Some(i) = self.windows.iter().position(|w| w.id == id) {
            let mut w = self.windows.remove(i);
            w.level = level;
            self.insert_top(w);
        }
    }

    /// The topmost shown window under the point.
    pub fn at(&self, x: i32, y: i32) -> Option<WindowId> {
        self.windows.iter().rev().find(|w| !w.minimized && w.rect.contains(x, y)).map(|w| w.id)
    }

    /// Raises a window to the top of its level band and makes it the key window.
    pub fn raise(&mut self, id: WindowId) {
        if let Some(i) = self.windows.iter().position(|w| w.id == id) {
            let w = self.windows.remove(i);
            self.insert_top(w);
            self.key = Some(id);
        }
    }

    pub fn minimize(&mut self, id: WindowId) {
        if let Some(w) = self.get_mut(id) { w.minimized = true; }
        if self.key == Some(id) { self.key = None; }
    }

    /// Shows a minimized window again, raised and focused.
    pub fn restore(&mut self, id: WindowId) {
        if let Some(w) = self.get_mut(id) { w.minimized = false; }
        self.raise(id);
    }

    /// Maximizes to the usable area, or restores the frame from before; returns the new frame.
    pub fn toggle_maximize(&mut self, id: WindowId) -> Option<Rect> {
        let area = self.area;
        let w = self.get_mut(id)?;
        if w.full.is_some() { return None; }
        match (w.restore.take(), w.tiled.take()) {
            (Some(old), None) => w.rect = old,
            // a snapped window keeps its frame from before the snap and fills the area
            (Some(old), Some(_)) => { w.restore = Some(old); w.rect = area; }
            (None, _) => { w.restore = Some(w.rect); w.rect = area; }
        }
        Some(w.rect)
    }

    /// Starts dragging `id` from the pointer at (`x`, `y`). A maximized window drops back to its old size first,
    /// under the pointer at the same relative position.
    pub fn begin_move(&mut self, id: WindowId, x: i32, y: i32) -> Option<Grab> {
        let area = self.area;
        let w = self.get_mut(id)?;
        if w.full.is_some() { return None; }
        w.tiled = None;
        if let Some(old) = w.restore.take() {
            let rel = (x - w.rect.x) as f32 / w.rect.w.max(1) as f32;
            w.rect = Rect::new(x - (rel * old.w as f32) as i32, area.y, old.w, old.h);
        }
        let r = w.rect;
        self.raise(id);
        Some(Grab::Move { id, dx: x - r.x, dy: y - r.y })
    }

    pub fn begin_resize(&mut self, id: WindowId, edges: Edges, x: i32, y: i32) -> Option<Grab> {
        let w = self.get_mut(id)?;
        if w.full.is_some() { return None; }
        w.restore = None;
        w.tiled = None;
        let start = w.rect;
        self.raise(id);
        Some(Grab::Resize { id, edges, x, y, start })
    }

    /// Applies the pointer at (`x`, `y`) to a grab; returns the window's new frame.
    pub fn motion(&mut self, grab: &Grab, x: i32, y: i32) -> Option<Rect> {
        let area = self.area;
        let (id, rect) = match *grab {
            Grab::Move { id, dx, dy } => {
                let r = self.get(id)?.rect;
                (id, keep_on_screen(Rect::new(x - dx, y - dy, r.w, r.h), area))
            }
            Grab::Resize { id, edges, x: sx, y: sy, start: r } => (id, resized(r, edges, x - sx, y - sy)),
        };
        self.get_mut(id)?.rect = rect;
        Some(rect)
    }

    /// The usable area changed (output mode, menu bar, the Dock coming back after full screen): maximized and tiled
    /// windows fill their part of it, others are pulled back on screen. Returns the windows whose size changed: their
    /// clients must be told (a window maximized while the Dock was away kept the Dock's strip, W3-6).
    pub fn set_area(&mut self, area: Rect) -> Vec<WindowId> {
        self.area = area;
        let output = self.output;
        let mut resized = Vec::new();
        for w in &mut self.windows {
            // a full-screen window fills the output; what it held is fitted to the new area when it leaves
            if w.full.is_some() {
                w.rect = output;
                continue;
            }
            let before = (w.rect.w, w.rect.h);
            w.rect = match (w.restore.is_some(), w.tiled) {
                (_, Some(layout)) => tiling::frame(area, layout),
                (true, None) => area,
                (false, None) => keep_on_screen(w.rect, area),
            };
            if (w.rect.w, w.rect.h) != before {
                resized.push(w.id);
            }
        }
        resized
    }

    /// Tiles `id` into `layout` (a half, a quarter, or maximize): the Fn+Ctrl arrow keys and a drag to an edge. The
    /// frame from before the first tile is kept, so the centre key can restore it. `Maximize` is the maximize state.
    pub fn tile(&mut self, id: WindowId, layout: Layout) -> Option<Rect> {
        let area = self.area;
        let w = self.get_mut(id)?;
        if w.full.is_some() { return None; }
        if w.restore.is_none() { w.restore = Some(w.rect); }
        if layout == Layout::Maximize {
            w.tiled = None;
            w.rect = area;
        } else {
            w.tiled = Some(layout);
            w.rect = tiling::frame(area, layout);
        }
        Some(w.rect)
    }

    /// The layout a window dragged to the pointer (`x`, `y`) would snap into, if any.
    pub fn drop_zone(&self, x: i32, y: i32) -> Option<Layout> {
        tiling::drop_zone(self.area, x, y, tiling::EDGE, tiling::CORNER)
    }

    /// Ctrl+Super+Down: leaves maximized or snapped state and centres the window at its previous size (or its
    /// current size when it was never moved aside). Returns the new frame.
    pub fn restore_centre(&mut self, id: WindowId) -> Option<Rect> {
        let area = self.area;
        let w = self.get_mut(id)?;
        if w.full.is_some() { return None; }
        let size = w.restore.take().unwrap_or(w.rect);
        w.tiled = None;
        let (cw, ch) = (size.w.min(area.w), size.h.min(area.h));
        w.rect = Rect::new(area.x + (area.w - cw) / 2, area.y + (area.h - ch) / 2, cw, ch);
        Some(w.rect)
    }

    /// Whether the window fills the area by the maximize key (a tiled window is not maximized).
    pub fn is_maximized(&self, id: WindowId) -> bool { self.get(id).is_some_and(|w| w.restore.is_some() && w.tiled.is_none()) }

    /// The tiled layout the window is in, if any.
    pub fn snapped(&self, id: WindowId) -> Option<Layout> { self.get(id).and_then(|w| w.tiled) }

    /// The client settled on a frame of its own (it chose its size, or finished a resize).
    pub fn set_frame(&mut self, id: WindowId, rect: Rect) {
        if let Some(w) = self.get_mut(id).filter(|w| w.full.is_none()) { w.rect = rect; w.tiled = None; }
    }

    /// Goes back to the frame from before the last maximize or tile and leaves the tile; a window that never left its
    /// frame stays. Returns the new frame.
    pub fn restore_previous(&mut self, id: WindowId) -> Option<Rect> {
        let w = self.get_mut(id)?;
        if w.full.is_some() { return None; }
        w.tiled = None;
        if let Some(old) = w.restore.take() { w.rect = old; }
        Some(w.rect)
    }

    /// Applies a window-management choice (a tile, the centre, the way back) to `id`; the new frame, or `None` for a
    /// window that is unknown or full screen. The Ctrl+Super arrow keys and the shell's green-button menu both land here.
    pub fn place(&mut self, id: WindowId, placement: Placement) -> Option<Rect> {
        match placement {
            Placement::Layout(layout) => self.tile(id, layout),
            Placement::Centre => self.restore_centre(id),
            Placement::Previous => self.restore_previous(id),
        }
    }

    /// Takes `id` to the whole output, in its own full-screen space. What the window held (its frame, the frame before
    /// a maximize, its tile) is kept for `leave_full_screen`. A window already full screen is left as it is. Returns
    /// the new frame.
    pub fn enter_full_screen(&mut self, id: WindowId) -> Option<Rect> {
        let output = self.output;
        let w = self.get_mut(id)?;
        if w.full.is_none() {
            w.full = Some(Saved { rect: w.rect, restore: w.restore, tiled: w.tiled });
            w.restore = None;
            w.tiled = None;
        }
        w.rect = output;
        Some(w.rect)
    }

    /// Leaves full screen: the window goes back to the frame, maximize or tile it had, fitted to the usable area as it
    /// is now. `None` for a window that is unknown or not full screen.
    pub fn leave_full_screen(&mut self, id: WindowId) -> Option<Rect> {
        let area = self.area;
        let w = self.get_mut(id)?;
        let saved = w.full.take()?;
        w.restore = saved.restore;
        w.tiled = saved.tiled;
        w.rect = match (saved.restore.is_some(), saved.tiled) {
            (_, Some(layout)) => tiling::frame(area, layout),
            (true, None) => area,
            (false, None) => keep_on_screen(saved.rect, area),
        };
        Some(w.rect)
    }

    /// Whether the window fills the output in its own full-screen space.
    pub fn is_full_screen(&self, id: WindowId) -> bool { self.get(id).is_some_and(|w| w.full.is_some()) }

    fn get(&self, id: WindowId) -> Option<&Window> { self.windows.iter().find(|w| w.id == id) }
    fn get_mut(&mut self, id: WindowId) -> Option<&mut Window> { self.windows.iter_mut().find(|w| w.id == id) }
}

/// The titlebar never goes above the area, and `KEEP_VISIBLE` of the window stays inside it on every side.
///
/// Total for every `r` and `a`, including an area narrower than `KEEP_VISIBLE` or an empty one (`i32::clamp` panics when
/// its minimum exceeds its maximum, and a client's layer surface can shrink the area that far).
fn keep_on_screen(r: Rect, a: Rect) -> Rect {
    let lo_x = a.x.saturating_sub(r.w).saturating_add(KEEP_VISIBLE);
    let hi_x = a.x.saturating_add(a.w).saturating_sub(KEEP_VISIBLE).max(lo_x);
    let hi_y = a.y.saturating_add(a.h).saturating_sub(KEEP_VISIBLE).max(a.y);
    Rect { x: r.x.clamp(lo_x, hi_x), y: r.y.clamp(a.y, hi_y), ..r }
}

/// The usable area a layer surface's exclusive zones may leave: `area` inside `output`, at least a quarter of the output
/// wide and high (and at least one pixel), so a client's huge or negative exclusive zone cannot collapse it.
pub fn clamp_area(area: Rect, output: Rect) -> Rect {
    let min_w = (output.w / 4).max(1);
    let min_h = (output.h / 4).max(1);
    let w = area.w.clamp(min_w, output.w.max(min_w));
    let h = area.h.clamp(min_h, output.h.max(min_h));
    let x = area.x.clamp(output.x, output.x.saturating_add(output.w).saturating_sub(w).max(output.x));
    let y = area.y.clamp(output.y, output.y.saturating_add(output.h).saturating_sub(h).max(output.y));
    Rect { x, y, w, h }
}

/// `start` with the grabbed edges moved by (`dx`, `dy`), never smaller than the minimum; the opposite edges stay put.
fn resized(start: Rect, e: Edges, dx: i32, dy: i32) -> Rect {
    let (mut x, mut y, mut w, mut h) = (start.x, start.y, start.w, start.h);
    if e.left { w = (start.w - dx).max(MIN_W); x = start.x + start.w - w; }
    if e.right { w = (start.w + dx).max(MIN_W); }
    if e.top { h = (start.h - dy).max(MIN_H); y = start.y + start.h - h; }
    if e.bottom { h = (start.h + dy).max(MIN_H); }
    Rect::new(x, y, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(0, 28, 1920, 1052); // 1920x1080 output under a 28 px menu bar

    #[test]
    fn keeping_a_window_on_screen_never_panics_on_a_collapsed_area() {
        let r = Rect::new(100, 100, 40, 40);
        for a in [Rect::new(0, 0, 0, 0), Rect::new(0, 28, -90_000, 1000), Rect::new(0, 28, 20, -5), Rect::new(i32::MAX - 10, 0, 100, 100)] {
            let k = keep_on_screen(r, a);
            assert_eq!((k.w, k.h), (r.w, r.h));
        }
        let mut wm = Wm::new(AREA);
        wm.map(1, 800, 600);
        wm.set_area(Rect::new(0, 0, 0, 0));
        wm.set_area(Rect::new(0, 28, -50_000, -50_000));
        wm.map_at(2, 30, 30, Some((10, 10)));
        assert!(wm.rect(2).is_some());
    }

    #[test]
    fn the_usable_area_stays_a_sane_part_of_the_output() {
        let out = Rect::new(0, 0, 1920, 1080);
        let menu_bar = Rect::new(0, 28, 1920, 1052);
        assert_eq!(clamp_area(menu_bar, out), menu_bar, "an ordinary menu bar is untouched");
        let huge_right = clamp_area(Rect::new(0, 0, -98_000, 1080), out);
        assert_eq!((huge_right.w, huge_right.h), (480, 1080), "at least a quarter of the output wide");
        let none = clamp_area(Rect::new(0, 0, 0, 0), out);
        assert_eq!((none.w, none.h), (480, 270));
        let outside = clamp_area(Rect::new(5000, -400, 1920, 1080), out);
        assert!(outside.x + outside.w <= 1920 && outside.y >= 0);
        let empty_output = clamp_area(Rect::new(0, 0, 0, 0), Rect::new(0, 0, 0, 0));
        assert_eq!((empty_output.w, empty_output.h), (1, 1), "never empty, never a panic");
    }

    #[test]
    fn a_new_window_is_centred_in_the_usable_area_and_focused() {
        let mut wm = Wm::new(AREA);
        let r = wm.map(1, 800, 600);
        assert_eq!(r, Rect::new(560, 28 + 226, 800, 600));
        assert_eq!(wm.focused(), Some(1));
    }

    #[test]
    fn further_windows_cascade_and_stay_inside_the_area() {
        let mut wm = Wm::new(AREA);
        let a = wm.map(1, 800, 600);
        let b = wm.map(2, 800, 600);
        assert_eq!((b.x - a.x, b.y - a.y), (CASCADE, CASCADE));
        let big = wm.map(3, 4000, 3000);
        assert_eq!(big, AREA, "a window larger than the area is fitted to it");
        assert_eq!(wm.focused(), Some(3));
    }

    #[test]
    fn a_window_mapped_with_an_origin_returns_there_instead_of_cascading() {
        let mut wm = Wm::new(AREA);
        wm.map(1, 800, 600);
        wm.map(2, 800, 600);
        // two windows left at their own places, brought back in the other order than they were mapped
        assert_eq!(wm.map_at(3, 800, 600, Some((1065, 524))), Rect::new(1065, 524, 800, 600));
        assert_eq!(wm.map_at(4, 400, 300, Some((737, 336))), Rect::new(737, 336, 400, 300));
    }

    #[test]
    fn an_origin_outside_the_usable_area_is_fitted_like_a_drag_fits_it() {
        let mut wm = Wm::new(AREA);
        // KEEP_VISIBLE px stay on screen: x in [a.x - w + 48, a.x + a.w - 48], y in [a.y, a.y + a.h - 48]
        assert_eq!(wm.map_at(1, 800, 600, Some((-300, 0))), Rect::new(-300, 28, 800, 600), "top edge only");
        assert_eq!(wm.map_at(2, 800, 600, Some((-5000, 0))), Rect::new(-752, 28, 800, 600), "left edge");
        assert_eq!(
            wm.map_at(3, 800, 600, Some((5000, 5000))),
            Rect::new(AREA.x + AREA.w - KEEP_VISIBLE, AREA.y + AREA.h - KEEP_VISIBLE, 800, 600),
            "right and bottom edges"
        );
    }

    #[test]
    fn a_window_dragged_partly_off_screen_returns_to_the_same_rect_after_unmap_and_map_at() {
        let mut wm = Wm::new(AREA);
        wm.map(1, 800, 600);
        let grab = wm.begin_move(1, 900, 40).expect("grab");
        // drag so the frame runs past the right and bottom edges
        let dragged = wm.motion(&grab, 1900, 700).expect("moved");
        assert!(dragged.x + dragged.w > AREA.x + AREA.w && dragged.y + dragged.h > AREA.y + AREA.h, "{dragged:?}");
        wm.unmap(1);
        assert_eq!(wm.map_at(1, dragged.w, dragged.h, Some((dragged.x, dragged.y))), dragged);
    }

    #[test]
    fn hit_testing_finds_the_topmost_window_and_raising_changes_it() {
        let mut wm = Wm::new(AREA);
        wm.map(1, 800, 600);
        wm.map(2, 800, 600);
        let (x, y) = (960, 600);
        assert_eq!(wm.at(x, y), Some(2));
        wm.raise(1);
        assert_eq!(wm.at(x, y), Some(1));
        assert_eq!(wm.focused(), Some(1));
        assert_eq!(wm.stack(), vec![2, 1], "bottom first");
    }

    #[test]
    fn a_floating_window_stays_above_the_normal_windows() {
        let mut wm = Wm::new(AREA);
        wm.map(1, 800, 600);
        wm.map(2, 800, 600);
        wm.set_level(1, Level::Floating);
        assert_eq!(wm.stack(), vec![2, 1], "the floating window is on top though it was mapped first");
        assert_eq!(wm.level(1), Some(Level::Floating));
        wm.raise(2);
        assert_eq!(wm.stack(), vec![2, 1], "raising a normal window does not pass a floating one");
        assert_eq!(wm.at(960, 600), Some(1), "hit testing sees the floating window first");
        assert_eq!(wm.focused(), Some(2), "the raised normal window has the keyboard");
    }

    #[test]
    fn floating_windows_keep_their_order_among_themselves_and_drop_back_to_normal() {
        let mut wm = Wm::new(AREA);
        wm.map(1, 400, 300);
        wm.map(2, 400, 300);
        wm.map(3, 400, 300);
        wm.set_level(1, Level::Floating);
        wm.set_level(2, Level::Floating);
        assert_eq!(wm.stack(), vec![3, 1, 2], "floating windows are in the order they were set");
        wm.raise(1);
        assert_eq!(wm.stack(), vec![3, 2, 1], "a raised floating window is the top of its band");
        wm.set_level(1, Level::Normal);
        assert_eq!(wm.stack(), vec![3, 1, 2], "back on the normal level, it goes on top of the normal windows");
        assert_eq!(wm.level(1), Some(Level::Normal));
    }

    #[test]
    fn a_minimized_or_closed_key_window_hands_focus_to_the_topmost_shown_window() {
        let mut wm = Wm::new(AREA);
        wm.map(1, 400, 300);
        wm.map(2, 400, 300);
        wm.set_level(2, Level::Floating);
        assert_eq!(wm.focused(), Some(2), "the mapped window is key");
        wm.minimize(2);
        assert_eq!(wm.focused(), Some(1));
        wm.restore(2);
        assert_eq!(wm.focused(), Some(2));
        wm.unmap(2);
        assert_eq!(wm.focused(), Some(1));
    }

    #[test]
    fn unmapping_the_focused_window_focuses_the_next_one_down() {
        let mut wm = Wm::new(AREA);
        wm.map(1, 400, 300);
        wm.map(2, 400, 300);
        wm.unmap(2);
        assert_eq!(wm.focused(), Some(1));
        wm.unmap(1);
        assert_eq!(wm.focused(), None);
    }

    #[test]
    fn a_move_grab_follows_the_pointer_and_keeps_the_titlebar_reachable() {
        let mut wm = Wm::new(AREA);
        let r = wm.map(1, 800, 600);
        let g = wm.begin_move(1, r.x + 100, r.y + 10).unwrap();
        assert_eq!(wm.motion(&g, r.x + 300, r.y + 210), Some(Rect::new(r.x + 200, r.y + 200, 800, 600)));
        // above the menu bar: the titlebar stops at the top of the usable area
        assert_eq!(wm.motion(&g, r.x + 100, 0).unwrap().y, AREA.y);
        // far off the right edge: a strip stays on screen
        let off = wm.motion(&g, 10_000, r.y + 10).unwrap();
        assert_eq!(off.x, AREA.x + AREA.w - KEEP_VISIBLE);
        assert_eq!(wm.rect(1), Some(off), "the grab moves the window itself");
    }

    #[test]
    fn a_resize_grab_moves_only_its_edges_and_respects_the_minimum() {
        let mut wm = Wm::new(AREA);
        let r = wm.map(1, 800, 600);
        let g = wm.begin_resize(1, Edges { left: true, bottom: true, ..Edges::default() }, r.x, r.y + r.h).unwrap();
        let n = wm.motion(&g, r.x - 50, r.y + r.h + 40).unwrap();
        assert_eq!(n, Rect::new(r.x - 50, r.y, 850, 640), "the right and top edges stay put");
        let tiny = wm.motion(&g, r.x + 790, r.y + 10).unwrap();
        assert_eq!((tiny.w, tiny.h), (MIN_W, MIN_H));
        assert_eq!(tiny.x + tiny.w, r.x + r.w, "shrinking from the left keeps the right edge");
    }

    #[test]
    fn maximize_fills_the_area_and_toggling_restores_the_old_frame() {
        let mut wm = Wm::new(AREA);
        let r = wm.map(1, 800, 600);
        assert_eq!(wm.toggle_maximize(1), Some(AREA));
        assert_eq!(wm.toggle_maximize(1), Some(r));
    }

    #[test]
    fn dragging_a_maximized_window_restores_its_size_under_the_pointer() {
        let mut wm = Wm::new(AREA);
        let r = wm.map(1, 800, 600);
        wm.toggle_maximize(1);
        // grabbed at the middle of the titlebar: the restored frame keeps the pointer at its middle
        let g = wm.begin_move(1, 960, 40).unwrap();
        let n = wm.rect(1).unwrap();
        assert_eq!((n.w, n.h), (r.w, r.h));
        assert_eq!(n.x + n.w / 2, 960);
        let moved = wm.motion(&g, 1000, 300).unwrap();
        assert_eq!((moved.x - n.x, moved.y - n.y), (40, 260));
        assert_eq!(wm.toggle_maximize(1), Some(AREA), "it is a normal window again");
    }

    #[test]
    fn a_new_area_refits_maximized_windows_and_pulls_others_back_on_screen() {
        let mut wm = Wm::new(AREA);
        wm.map(1, 800, 600);
        wm.toggle_maximize(1);
        let r2 = wm.map(2, 800, 600);
        let g = wm.begin_move(2, r2.x, r2.y).unwrap();
        wm.motion(&g, 1800, 900);
        let small = Rect::new(0, 28, 1280, 692);
        wm.set_area(small);
        assert_eq!(wm.rect(1), Some(small));
        let r = wm.rect(2).unwrap();
        assert!(r.x <= small.x + small.w - KEEP_VISIBLE && r.y <= small.y + small.h - KEEP_VISIBLE, "{r:?}");
    }

    #[test]
    fn the_client_may_settle_on_its_own_frame() {
        let mut wm = Wm::new(AREA);
        let r = wm.map(1, 800, 600);
        wm.set_frame(1, Rect { w: 640, ..r });
        assert_eq!(wm.rect(1), Some(Rect { w: 640, ..r }));
        assert!(!wm.is_maximized(1));
        wm.toggle_maximize(1);
        assert!(wm.is_maximized(1));
    }

    #[test]
    fn minimized_windows_are_skipped_by_focus_and_hit_testing() {
        let mut wm = Wm::new(AREA);
        wm.map(1, 800, 600);
        wm.map(2, 800, 600);
        wm.minimize(2);
        assert_eq!(wm.focused(), Some(1));
        assert_eq!(wm.at(960, 600), Some(1));
        assert!(!wm.visible(2));
        wm.restore(2);
        assert_eq!(wm.focused(), Some(2), "restoring raises and focuses");
        assert!(wm.visible(2));
    }

    #[test]
    fn tiling_puts_a_window_on_one_half_and_the_right_half_takes_the_odd_pixel() {
        let mut wm = Wm::new(Rect::new(0, 28, 1921, 1052));
        wm.map(1, 800, 600);
        assert_eq!(wm.tile(1, Layout::Left), Some(Rect::new(0, 28, 960, 1052)));
        assert_eq!(wm.tile(1, Layout::Right), Some(Rect::new(960, 28, 961, 1052)));
        assert_eq!(wm.snapped(1), Some(Layout::Right));
        assert!(!wm.is_maximized(1), "a tiled window is not maximized");
    }

    #[test]
    fn tiling_into_a_quarter_and_maximize_keeps_the_frame_from_before() {
        let mut wm = Wm::new(AREA);
        let r = wm.map(1, 800, 600);
        assert_eq!(wm.tile(1, Layout::BottomRight), Some(Rect::new(960, 28 + 526, 960, 526)));
        assert_eq!(wm.snapped(1), Some(Layout::BottomRight));
        assert_eq!(wm.tile(1, Layout::Maximize), Some(AREA));
        assert!(wm.is_maximized(1), "maximize through tiling is the maximize state");
        assert_eq!(wm.snapped(1), None);
        assert_eq!(wm.toggle_maximize(1), Some(r), "unmaximize returns to the frame before the first tile");
    }

    #[test]
    fn a_drag_to_an_edge_reports_the_layout_it_would_snap_into() {
        let wm = Wm::new(AREA);
        assert_eq!(wm.drop_zone(AREA.x, AREA.y), Some(Layout::TopLeft));
        assert_eq!(wm.drop_zone(960, AREA.y + 2), Some(Layout::Maximize));
        assert_eq!(wm.drop_zone(960, 500), None);
    }

    #[test]
    fn toggling_maximize_from_a_tile_fills_the_area_and_unmaximize_returns_to_the_frame_before_the_tile() {
        let mut wm = Wm::new(AREA);
        let r = wm.map(1, 800, 600);
        wm.tile(1, Layout::Left);
        assert_eq!(wm.toggle_maximize(1), Some(AREA));
        assert!(wm.is_maximized(1));
        assert_eq!(wm.toggle_maximize(1), Some(r));
        assert_eq!(wm.snapped(1), None);
    }

    #[test]
    fn the_centre_key_restores_the_size_from_before_and_centres_it() {
        let mut wm = Wm::new(AREA);
        wm.map(1, 800, 600);
        wm.tile(1, Layout::Right);
        wm.tile(1, Layout::Left);
        assert_eq!(wm.restore_centre(1), Some(Rect::new(560, 28 + 226, 800, 600)));
        assert_eq!(wm.snapped(1), None);
        // a window that was never moved aside is centred at its current size
        wm.set_frame(1, Rect::new(0, 28, 400, 300));
        assert_eq!(wm.restore_centre(1), Some(Rect::new(760, 28 + 376, 400, 300)));
    }

    #[test]
    fn a_tiled_window_follows_a_new_usable_area() {
        let mut wm = Wm::new(AREA);
        wm.map(1, 800, 600);
        wm.tile(1, Layout::Left);
        wm.set_area(Rect::new(0, 28, 1280, 692));
        assert_eq!(wm.rect(1), Some(Rect::new(0, 28, 640, 692)));
    }

    #[test]
    fn the_half_split_is_exact_for_even_and_odd_widths() {
        assert_eq!(tiling::frame(Rect::new(10, 0, 100, 50), Layout::Left), Rect::new(10, 0, 50, 50));
        assert_eq!(tiling::frame(Rect::new(10, 0, 100, 50), Layout::Right), Rect::new(60, 0, 50, 50));
        let odd = Rect::new(0, 0, 7, 1);
        assert_eq!(tiling::frame(odd, Layout::Left).w + tiling::frame(odd, Layout::Right).w, 7);
    }

    /// An output of `w` x `h` with a 28 px menu bar: the usable area under it and the whole output.
    fn screen(w: i32, h: i32) -> (Rect, Rect) { (Rect::new(0, 28, w, h - 28), Rect::new(0, 0, w, h)) }

    #[test]
    fn full_screen_fills_the_whole_output_at_its_origin_for_any_output_size() {
        for (w, h) in [(1024, 768), (1280, 720), (1920, 1080), (2560, 1440), (3840, 2160), (1440, 3200)] {
            let (area, output) = screen(w, h);
            let mut wm = Wm::new(area);
            wm.set_output(output);
            // a window far from the origin, as the lab machine's Music window was
            let r = wm.map(1, w / 3, h / 3);
            assert_ne!((r.x, r.y), (0, 0));
            assert_eq!(wm.enter_full_screen(1), Some(output), "{w}x{h}: the whole output, menu bar included");
            assert_eq!(wm.rect(1), Some(output));
            assert!(wm.is_full_screen(1));
            assert!(!wm.is_maximized(1), "full screen is not the maximize state");
        }
    }

    #[test]
    fn leaving_full_screen_restores_the_previous_frame_exactly() {
        let (area, output) = screen(2560, 1440);
        let mut wm = Wm::new(area);
        wm.set_output(output);
        let r = wm.map(1, 1200, 825);
        wm.enter_full_screen(1);
        assert_eq!(wm.leave_full_screen(1), Some(r));
        assert_eq!(wm.rect(1), Some(r));
        assert!(!wm.is_full_screen(1));
        assert_eq!(wm.leave_full_screen(1), None, "nothing to leave the second time");
    }

    #[test]
    fn a_maximized_or_tiled_window_comes_back_maximized_or_tiled() {
        let (area, output) = screen(1920, 1080);
        let mut wm = Wm::new(area);
        wm.set_output(output);
        let r = wm.map(1, 800, 600);
        wm.toggle_maximize(1);
        wm.enter_full_screen(1);
        assert_eq!(wm.leave_full_screen(1), Some(area));
        assert!(wm.is_maximized(1));
        assert_eq!(wm.toggle_maximize(1), Some(r), "the frame from before the maximize survived the round trip");
        wm.tile(1, Layout::Left);
        wm.enter_full_screen(1);
        assert_eq!(wm.snapped(1), None, "a full-screen window is not tiled");
        assert_eq!(wm.leave_full_screen(1), Some(tiling::frame(area, Layout::Left)));
        assert_eq!(wm.snapped(1), Some(Layout::Left));
    }

    #[test]
    fn a_full_screen_window_cannot_be_moved_resized_tiled_or_maximized() {
        let (area, output) = screen(1920, 1080);
        let mut wm = Wm::new(area);
        wm.set_output(output);
        wm.map(1, 800, 600);
        wm.enter_full_screen(1);
        assert!(wm.begin_move(1, 500, 500).is_none());
        assert!(wm.begin_resize(1, Edges { right: true, ..Edges::default() }, 500, 500).is_none());
        assert_eq!(wm.tile(1, Layout::Left), None);
        assert_eq!(wm.toggle_maximize(1), None);
        assert_eq!(wm.restore_centre(1), None);
        assert_eq!(wm.restore_previous(1), None);
        wm.set_frame(1, Rect::new(10, 10, 100, 100));
        assert_eq!(wm.rect(1), Some(output), "the client does not choose a frame for a full-screen window");
    }

    #[test]
    fn a_full_screen_window_follows_the_output_and_what_it_held_follows_the_area() {
        let (area, output) = screen(1920, 1080);
        let mut wm = Wm::new(area);
        wm.set_output(output);
        wm.map(1, 800, 600);
        wm.toggle_maximize(1);
        wm.enter_full_screen(1);
        let (small_area, small) = screen(1280, 720);
        wm.set_area(small_area);
        wm.set_output(small);
        assert_eq!(wm.rect(1), Some(small), "the new output");
        assert_eq!(wm.leave_full_screen(1), Some(small_area), "a maximized window fills the new area");
    }

    #[test]
    fn a_full_screen_window_is_still_raised_and_focused_like_any_other() {
        let (area, output) = screen(1920, 1080);
        let mut wm = Wm::new(area);
        wm.set_output(output);
        wm.map(1, 800, 600);
        wm.map(2, 800, 600);
        wm.enter_full_screen(1);
        wm.raise(1);
        assert_eq!(wm.focused(), Some(1));
        assert_eq!(wm.at(5, 5), Some(1), "its frame covers the menu bar strip too");
    }

    #[test]
    fn placements_are_the_same_actions_for_the_menu_and_the_keys() {
        let mut wm = Wm::new(AREA);
        let r = wm.map(1, 800, 600);
        assert_eq!(wm.place(1, Placement::Layout(Layout::Right)), Some(Rect::new(960, 28, 960, 1052)));
        assert_eq!(wm.place(1, Placement::Layout(Layout::TopLeft)), Some(Rect::new(0, 28, 960, 526)));
        assert_eq!(wm.place(1, Placement::Previous), Some(r), "the frame from before the first tile");
        assert_eq!(wm.snapped(1), None);
        assert_eq!(wm.place(1, Placement::Previous), Some(r), "already there: stays");
        wm.set_frame(1, Rect::new(0, 28, 400, 300));
        assert_eq!(wm.place(1, Placement::Centre), Some(Rect::new(760, 28 + 376, 400, 300)));
        assert_eq!(wm.place(1, Placement::Layout(Layout::Maximize)), Some(AREA));
        assert!(wm.is_maximized(1));
        assert_eq!(wm.place(9, Placement::Centre), None, "an unknown window");
    }

    /// W3-6: a window filled while the Dock was away (full screen hides it) is resized when the Dock's strip comes back,
    /// and its client is told; a free window that still fits keeps its size.
    #[test]
    fn an_area_change_reports_the_windows_it_resized() {
        let (without_dock, output) = screen(1920, 1080);
        let area = Rect::new(without_dock.x, without_dock.y, without_dock.w, without_dock.h - 100);
        let mut wm = Wm::new(without_dock);
        wm.set_output(output);
        wm.map(1, 800, 600);
        wm.map(2, 800, 600);
        wm.toggle_maximize(1);
        assert_eq!(wm.rect(1), Some(without_dock));
        let resized = wm.set_area(area);
        assert_eq!(resized, vec![1], "only the filled window changed size");
        assert_eq!(wm.rect(1), Some(area));
        assert!(wm.set_area(area).is_empty(), "the same area again resizes nothing");
    }

    /// W3-6: Fill, full screen in and out, then Return to Previous Size comes back to the frame from before the Fill.
    #[test]
    fn previous_size_survives_full_screen_after_a_fill() {
        let (area, output) = screen(1920, 1080);
        let mut wm = Wm::new(area);
        wm.set_output(output);
        let r = wm.map(1, 800, 600);
        wm.toggle_maximize(1);
        wm.enter_full_screen(1);
        assert_eq!(wm.rect(1), Some(output));
        assert_eq!(wm.leave_full_screen(1), Some(area), "it comes back filled");
        assert!(wm.is_maximized(1));
        assert_eq!(wm.place(1, Placement::Previous), Some(r), "and goes back to the frame from before the Fill");
        assert!(!wm.is_maximized(1));
    }
}
