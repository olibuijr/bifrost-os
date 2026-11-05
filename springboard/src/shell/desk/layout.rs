//! Pure window geometry: rectangles, tiling, the green-button menu, cascade, the Dock row and window shapes.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x as f32 && x < (self.x + self.w) as f32 && y >= self.y as f32 && y < (self.y + self.h) as f32
    }
}

/// Where a window can be tiled in the work area (Window menu, Move & Resize, and edge snapping).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tile {
    Left, Right, Top, Bottom, TopLeft, TopRight, BottomLeft, BottomRight, Fill, Center,
}

/// A choice of the green-button menu (hover on the green traffic light), for one window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(any(test, feature = "desktop-wayland"))]
pub enum Arrange {
    /// Fill, Centre and the halves and quarters of Move & Resize.
    Tile(Tile),
    /// Return to Previous Size.
    Previous,
    /// Enter (`true`) or Exit Full Screen.
    FullScreen(bool),
    /// This window on the left half, the window behind it on the right half.
    SideBySide,
}

/// The open green-button menu: the window it is for, and where it opens (screen px, under the button).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArrangeMenu {
    pub app: AppId,
    pub x: f32,
    pub y: f32,
}

/// How long (ms) the pointer rests on a green button before its menu opens.
pub const ARRANGE_DWELL_MS: f64 = 500.0;
/// The gap (pt) between a green button and the menu under it, and how far above the menu's top (pt, measured from the
/// anchor) the pointer may stray on the way in before the menu closes.
#[cfg(feature = "desktop-wayland")]
pub const ARRANGE_GAP_PT: f32 = 4.0;
#[cfg(feature = "desktop-wayland")]
pub const ARRANGE_BRIDGE_PT: f32 = 24.0;

/// One line of the green-button menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(any(test, feature = "desktop-wayland"))]
pub enum ArrangeLine {
    Item(&'static str, Arrange),
    Header(&'static str),
    Separator,
}

/// The lines of the green-button menu for a window: `full` when it is full screen (the only choice then is to leave
/// it), `others` when another window could share the screen.
#[cfg(any(test, feature = "desktop-wayland"))]
pub fn arrange_lines(full: bool, others: bool) -> Vec<ArrangeLine> {
    use ArrangeLine::*;
    if full {
        return vec![Item("Exit Full Screen", Arrange::FullScreen(false))];
    }
    let mut v = vec![
        Item("Fill", Arrange::Tile(Tile::Fill)),
        Item("Center", Arrange::Tile(Tile::Center)),
        Item("Return to Previous Size", Arrange::Previous),
        Separator,
        Header("Move & Resize"),
        Item("Left", Arrange::Tile(Tile::Left)),
        Item("Right", Arrange::Tile(Tile::Right)),
        Item("Top", Arrange::Tile(Tile::Top)),
        Item("Bottom", Arrange::Tile(Tile::Bottom)),
        Item("Top Left", Arrange::Tile(Tile::TopLeft)),
        Item("Top Right", Arrange::Tile(Tile::TopRight)),
        Item("Bottom Left", Arrange::Tile(Tile::BottomLeft)),
        Item("Bottom Right", Arrange::Tile(Tile::BottomRight)),
    ];
    if others {
        v.extend([Separator, Item("Tile with Next Window", Arrange::SideBySide)]);
    }
    v.extend([Separator, Item("Enter Full Screen", Arrange::FullScreen(true))]);
    v
}

/// What the window server is asked for a tile (`bifrost_window_level_v1` `tile`): the same actions the window
/// server's Ctrl+Super arrow keys perform.
#[cfg(feature = "desktop-wayland")]
pub fn placement_of(tile: Tile) -> crate::bare::window_level_client::Placement {
    use crate::bare::window_level_client::Placement as P;
    match tile {
        Tile::Fill => P::Fill,
        Tile::Left => P::Left,
        Tile::Right => P::Right,
        Tile::Top => P::Top,
        Tile::Bottom => P::Bottom,
        Tile::TopLeft => P::TopLeft,
        Tile::TopRight => P::TopRight,
        Tile::BottomLeft => P::BottomLeft,
        Tile::BottomRight => P::BottomRight,
        Tile::Center => P::Centre,
    }
}

/// The rectangle of `tile` inside the work area `a`; Centre keeps two thirds of each side.
/// How far a window moves up to clear the on-screen keyboard whose top edge is `kb_top`: its bottom edge stops
/// at the keyboard, and its top never goes above the menu bar. Zero when it already clears the keyboard.
pub fn kb_lift(r: Rect, kb_top: i32, menubar: i32) -> i32 {
    let over = r.y + r.h - kb_top;
    if over <= 0 {
        return 0;
    }
    over.min((r.y - menubar).max(0))
}

/// The position a window's toplevel asks to map at: where the window is (`x`, `y`) when the window server placed it
/// before (the toplevel was destroyed while the window was hidden and comes back), none for a window never placed, which
/// the window server cascades as a new window.
#[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
pub fn origin_hint(placed: &std::collections::HashSet<AppId>, app: AppId, x: i32, y: i32) -> Option<(i32, i32)> {
    placed.contains(&app).then_some((x, y))
}

/// The top-left corner (px) of a window of `size` cascaded from `from`, another window of the same app
/// (`window_flags::CASCADE`, the reference toolkit's cascade call), kept inside the work area `a`. From the main window
/// (`from_main`) it goes beside the main window's trailing edge when it fits there, and otherwise over the main window's
/// trailing side one `step` down, so the leading columns (a sidebar, a message list) stay in view. A window about as
/// wide as the main window (less than a `step` narrower) has no trailing side to sit on: that put a second document
/// straight over the first, one step down (E2E11 5), so it cascades one `step` down and right instead. From another
/// cascaded window it goes one `step` down and right, as each new document window does.
pub fn cascade_origin(a: Rect, from: Rect, from_main: bool, size: (i32, i32), step: i32) -> (i32, i32) {
    let (w, h) = size;
    let (x, y) = if !from_main || from.w - w < step {
        (from.x + step, from.y + step)
    } else if from.x + from.w + w <= a.x + a.w {
        (from.x + from.w, from.y)
    } else {
        (from.x + from.w - w, from.y + step)
    };
    (x.clamp(a.x, (a.x + a.w - w).max(a.x)), y.clamp(a.y, (a.y + a.h - h).max(a.y)))
}

/// Moves `r` to the position (`x`, `y`) the window server reports for its toplevel; the size is left alone (the
/// configure decides it). True when the position changed, so the frame the presenter holds must be sent again.
#[cfg(feature = "desktop-wayland")]
pub fn frame_follows(r: &mut Rect, x: i32, y: i32) -> bool {
    if (r.x, r.y) == (x, y) {
        return false;
    }
    r.x = x;
    r.y = y;
    true
}

pub fn tile_rect(a: Rect, tile: Tile) -> Rect {
    let (hw, hh) = (a.w / 2, a.h / 2);
    let (rw, rh) = (a.w - hw, a.h - hh);
    let r = |x, y, w, h| Rect { x: a.x + x, y: a.y + y, w, h };
    match tile {
        Tile::Left => r(0, 0, hw, a.h),
        Tile::Right => r(hw, 0, rw, a.h),
        Tile::Top => r(0, 0, a.w, hh),
        Tile::Bottom => r(0, hh, a.w, rh),
        Tile::TopLeft => r(0, 0, hw, hh),
        Tile::TopRight => r(hw, 0, rw, hh),
        Tile::BottomLeft => r(0, hh, hw, rh),
        Tile::BottomRight => r(hw, hh, rw, rh),
        Tile::Fill => a,
        Tile::Center => r(a.w / 6, a.h / 6, a.w * 2 / 3, a.h * 2 / 3),
    }
}

/// Split view: two windows share the work area side by side with a `divider` px gap between them. `lead_permille`
/// (0..=1000) is the leading window's share of the usable width; each side keeps at least `min_w` px, so the ratio
/// gives way on a narrow area. The rectangles always cover the area minus the divider.
#[cfg_attr(not(test), allow(dead_code))] // wired by the split view entry (next round)
pub fn split_view_rects(a: Rect, lead_permille: i32, divider: i32, min_w: i32) -> (Rect, Rect) {
    let usable = (a.w - divider).max(0);
    let lo = min_w.max(0).min(usable / 2);
    let lead = (usable * lead_permille.clamp(0, 1000) / 1000).clamp(lo, usable - lo);
    let trail = usable - lead;
    (Rect { x: a.x, y: a.y, w: lead, h: a.h }, Rect { x: a.x + lead + divider, y: a.y, w: trail, h: a.h })
}

/// The tile a pointer at (`px`,`py`) snaps to while dragging a title bar: left or right edge = half,
/// top edge = fill, corners = quarters. The band is `band` px wide (derive it from `ui.pt`).
pub fn snap_zone(a: Rect, px: i32, py: i32, band: i32) -> Option<Tile> {
    let (l, r, t) = (px <= a.x + band, px >= a.x + a.w - band, py <= a.y + band);
    let b = py >= a.y + a.h - band;
    match (l, r, t, b) {
        (true, _, true, _) => Some(Tile::TopLeft),
        (_, true, true, _) => Some(Tile::TopRight),
        (true, _, _, true) => Some(Tile::BottomLeft),
        (_, true, _, true) => Some(Tile::BottomRight),
        (true, ..) => Some(Tile::Left),
        (_, true, ..) => Some(Tile::Right),
        (_, _, true, _) => Some(Tile::Fill),
        _ => None,
    }
}

/// The parts of a window of `width` x `height` px that its pixels cover (alpha above zero), as rects in window px:
/// a run of covered pixels per row, and rows with the same runs merged into one rect. A borderless window takes the
/// pointer only there (`WindowFrame::input`). More than `MAX_SHAPE_RECTS` rects (a noisy picture) falls back to one
/// bounding box per run of rows, so the region stays small.
#[cfg(any(test, feature = "desktop-wayland"))]
pub fn alpha_shape(px: &[u32], width: usize, height: usize) -> Vec<[i32; 4]> {
    let mut out: Vec<[i32; 4]> = Vec::new();
    // the rects of the row band still growing: (x0, x1) runs and the row they started on
    let mut open: Vec<(usize, usize)> = Vec::new();
    let mut start = 0usize;
    let flush = |out: &mut Vec<[i32; 4]>, runs: &[(usize, usize)], y0: usize, y1: usize| {
        for &(a, b) in runs { out.push([a as i32, y0 as i32, (b - a) as i32, (y1 - y0) as i32]); }
    };
    for y in 0..=height {
        let runs: Vec<(usize, usize)> = if y == height { Vec::new() } else {
            let row = &px[y * width..(y + 1) * width];
            let mut v = Vec::new();
            let mut x = 0;
            while x < width {
                if row[x] >> 24 == 0 { x += 1; continue; }
                let a = x;
                while x < width && row[x] >> 24 != 0 { x += 1; }
                v.push((a, x));
            }
            v
        };
        if runs != open {
            flush(&mut out, &open, start, y);
            open = runs;
            start = y;
        }
    }
    if out.len() > MAX_SHAPE_RECTS {
        // one box per band of rows that has any pixel
        let mut boxes: Vec<[i32; 4]> = Vec::new();
        for r in out {
            match boxes.last_mut() {
                Some(b) if b[1] + b[3] == r[1] || (b[1] == r[1]) => {
                    let (x0, x1) = (b[0].min(r[0]), (b[0] + b[2]).max(r[0] + r[2]));
                    let (y0, y1) = (b[1].min(r[1]), (b[1] + b[3]).max(r[1] + r[3]));
                    *b = [x0, y0, x1 - x0, y1 - y0];
                }
                _ => boxes.push(r),
            }
        }
        return boxes;
    }
    out
}

/// Whether window-local (`x`, `y`) is inside one of `shape`'s rects.
pub fn shape_contains(shape: &[[i32; 4]], x: f32, y: f32) -> bool {
    shape.iter().any(|r| x >= r[0] as f32 && x < (r[0] + r[2]) as f32 && y >= r[1] as f32 && y < (r[1] + r[3]) as f32)
}

/// Where a press moves window `w` instead of reaching the app: above `drag_height` (window px), except in the
/// rectangles returned (window px). A full-size app's empty toolbar band drags; its traffic lights and the controls
/// it publishes (`App::controls`) do not, as the native toolkit lets the empty part of a unified toolbar move the window.
/// A full-screen window covers the menu bar; the bar comes back while the pointer is in its band at the top. Once it is
/// back, `keep` px more below the bar keep it (the window's title bar or toolbar revealed under it, `reveal_band`), so the
/// pointer can go down from the bar to that toolbar without both sliding away under it.
pub fn bar_hidden(full: bool, pointer_y: Option<f32>, bar_h: f32, keep: f32) -> bool {
    full && pointer_y.map_or(true, |y| y >= bar_h + keep.max(0.0))
}

#[cfg(test)]
pub fn dock_rects(m: &Metrics, icon: i32, pad: i32, gap: i32, n: usize, sep_after: Option<usize>, sep: i32) -> (Rect, Vec<Rect>) {
    let seps: Vec<usize> = sep_after.into_iter().collect();
    dock_rects_seps(m, icon, pad, gap, n, &seps, sep)
}

pub(super) fn dock_span(icon: i32, pad: i32, gap: i32, n: usize, seps: &[usize], sep: i32) -> i32 {
    n as i32 * icon + (n as i32 - 1).max(0) * gap + seps.len() as i32 * sep + 2 * pad
}

pub fn dock_rects_seps(m: &Metrics, icon: i32, pad: i32, gap: i32, n: usize, seps: &[usize], sep: i32) -> (Rect, Vec<Rect>) {
    let total = dock_span(icon, pad, gap, n, seps, sep);
    let x0 = (m.w - total) / 2;
    let y0 = m.h - icon - 2 * pad - (m.pt * DOCK_MARGIN_PT).round() as i32;
    let bar = Rect { x: x0, y: y0, w: total, h: icon + 2 * pad };
    let mut x = x0 + pad;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        out.push(Rect { x, y: y0 + pad, w: icon, h: icon });
        x += icon + gap;
        if seps.contains(&i) {
            x += sep;
        }
    }
    (bar, out)
}
