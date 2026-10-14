//! Clipping for drawing code: a stack of pixel rectangles. Pushing a clip intersects it with the clip on top, so
//! nested clips only ever shrink the drawable area, as a rectangular clip of a drawing context does.
//! [`fill_clipped`] fills a rectangle inside the current clip and the surface bounds. Pure: no device and no display.

use crate::gfx::Surface;

/// A pixel rectangle with its origin at the top left of the surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    /// A rectangle. A negative width or height is an empty rectangle.
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    /// Whether the rectangle covers no pixel.
    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    /// The pixels both rectangles cover. The result is empty when they do not overlap.
    pub fn intersect(self, other: Rect) -> Rect {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = (self.x + self.w).min(other.x + other.w);
        let y1 = (self.y + self.h).min(other.y + other.h);
        Rect { x: x0, y: y0, w: (x1 - x0).max(0), h: (y1 - y0).max(0) }
    }

    /// Whether the pixel at (`x`, `y`) lies inside the rectangle.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        !self.is_empty() && x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

/// The clip in force while drawing. The bottom of the stack is the whole surface and is never popped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipStack {
    clips: Vec<Rect>,
}

impl ClipStack {
    /// A stack whose base clip is the whole `width` by `height` surface.
    pub fn new(width: i32, height: i32) -> ClipStack {
        ClipStack { clips: vec![Rect::new(0, 0, width.max(0), height.max(0))] }
    }

    /// Narrows the current clip to its intersection with `rect` and returns the new clip.
    pub fn push(&mut self, rect: Rect) -> Rect {
        let next = self.current().intersect(rect);
        self.clips.push(next);
        next
    }

    /// Removes the innermost clip. Returns `false` when only the base clip is left.
    pub fn pop(&mut self) -> bool {
        if self.clips.len() > 1 {
            self.clips.pop();
            true
        } else {
            false
        }
    }

    /// The clip in force now.
    pub fn current(&self) -> Rect {
        self.clips.last().copied().unwrap_or(Rect::new(0, 0, 0, 0))
    }

    /// The number of clips pushed on top of the base clip.
    pub fn depth(&self) -> usize {
        self.clips.len() - 1
    }

    /// Whether the pixel at (`x`, `y`) may be drawn now.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.current().contains(x, y)
    }
}

/// Fills `rect` with an opaque `color`, drawing only the pixels that lie inside `clip` and inside the surface.
pub fn fill_clipped(surface: &mut Surface, clip: Rect, rect: Rect, color: u32) {
    let surface_bounds = Rect::new(0, 0, surface.w as i32, surface.h as i32);
    let visible = clip.intersect(rect).intersect(surface_bounds);
    if visible.is_empty() {
        return;
    }
    surface.fill_rect(visible.x as usize, visible.y as usize, visible.w as usize, visible.h as usize, color);
}
