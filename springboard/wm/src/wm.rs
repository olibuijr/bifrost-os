//! Window-management rules: placement, stacking, focus, move and resize grabs, maximize and minimize.
//! Coordinates are logical output pixels; `area` is the output minus exclusive zones (the menu bar).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect { pub x: i32, pub y: i32, pub w: i32, pub h: i32 }

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self { Rect { x, y, w, h } }
    pub fn contains(&self, x: i32, y: i32) -> bool { x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h }
}

pub type WindowId = u64;

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

struct Window { id: WindowId, rect: Rect, restore: Option<Rect>, minimized: bool }

/// An interactive move or resize in progress; `Wm::motion` applies the pointer to it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Grab {
    Move { id: WindowId, dx: i32, dy: i32 },
    Resize { id: WindowId, edges: Edges, x: i32, y: i32, start: Rect },
}

/// The window stack of one output, bottom first.
pub struct Wm { area: Rect, windows: Vec<Window> }

impl Wm {
    pub fn new(area: Rect) -> Self { Wm { area, windows: Vec::new() } }

    /// Places a new `w` x `h` window: centred, cascaded by the windows already shown, fitted inside the area. It is
    /// raised and focused.
    pub fn map(&mut self, id: WindowId, w: i32, h: i32) -> Rect {
        let a = self.area;
        let (w, h) = (w.min(a.w), h.min(a.h));
        let step = CASCADE * self.windows.iter().filter(|w| !w.minimized).count() as i32;
        let x = (a.x + (a.w - w) / 2 + step).min(a.x + a.w - w);
        let y = (a.y + (a.h - h) / 2 + step).min(a.y + a.h - h);
        let rect = Rect::new(x, y, w, h);
        self.windows.retain(|w| w.id != id);
        self.windows.push(Window { id, rect, restore: None, minimized: false });
        rect
    }

    pub fn unmap(&mut self, id: WindowId) { self.windows.retain(|w| w.id != id); }

    /// Ids bottom first, minimized windows included.
    pub fn stack(&self) -> Vec<WindowId> { self.windows.iter().map(|w| w.id).collect() }

    pub fn rect(&self, id: WindowId) -> Option<Rect> { self.get(id).map(|w| w.rect) }

    pub fn visible(&self, id: WindowId) -> bool { self.get(id).is_some_and(|w| !w.minimized) }

    /// The topmost shown window: it has keyboard focus.
    pub fn focused(&self) -> Option<WindowId> { self.windows.iter().rev().find(|w| !w.minimized).map(|w| w.id) }

    /// The topmost shown window under the point.
    pub fn at(&self, x: i32, y: i32) -> Option<WindowId> {
        self.windows.iter().rev().find(|w| !w.minimized && w.rect.contains(x, y)).map(|w| w.id)
    }

    pub fn raise(&mut self, id: WindowId) {
        if let Some(i) = self.windows.iter().position(|w| w.id == id) {
            let w = self.windows.remove(i);
            self.windows.push(w);
        }
    }

    pub fn minimize(&mut self, id: WindowId) {
        if let Some(w) = self.get_mut(id) { w.minimized = true; }
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
        match w.restore.take() {
            Some(old) => w.rect = old,
            None => { w.restore = Some(w.rect); w.rect = area; }
        }
        Some(w.rect)
    }

    /// Starts dragging `id` from the pointer at (`x`, `y`). A maximized window drops back to its old size first,
    /// under the pointer at the same relative position.
    pub fn begin_move(&mut self, id: WindowId, x: i32, y: i32) -> Option<Grab> {
        let area = self.area;
        let w = self.get_mut(id)?;
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
        w.restore = None;
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

    /// The usable area changed (output mode, menu bar): maximized windows fill it, others are pulled back on screen.
    pub fn set_area(&mut self, area: Rect) {
        self.area = area;
        for w in &mut self.windows {
            w.rect = if w.restore.is_some() { area } else { keep_on_screen(w.rect, area) };
        }
    }

    fn get(&self, id: WindowId) -> Option<&Window> { self.windows.iter().find(|w| w.id == id) }
    fn get_mut(&mut self, id: WindowId) -> Option<&mut Window> { self.windows.iter_mut().find(|w| w.id == id) }
}

/// The titlebar never goes above the area, and `KEEP_VISIBLE` of the window stays inside it on every side.
fn keep_on_screen(r: Rect, a: Rect) -> Rect {
    let x = r.x.clamp(a.x - r.w + KEEP_VISIBLE, a.x + a.w - KEEP_VISIBLE);
    let y = r.y.clamp(a.y, (a.y + a.h - KEEP_VISIBLE).max(a.y));
    Rect { x, y, ..r }
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
}
