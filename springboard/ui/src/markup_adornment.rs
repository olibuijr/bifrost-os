//! Markup adornments: small views placed over a markup canvas at a corner or the centre, such as a caption, a badge or a
//! handle. An adornment is a pure placement model: it turns an anchor, a size and an inset into a frame inside the canvas
//! and answers which adornment is under a point. Adornments are drawn by their owner; the canvas only supplies the frame.
//! Mirrors the reference markup adornment placement (anchored, inset, and topmost-first hit testing).

use crate::Rect;

/// Where an adornment sits in the canvas. Leading and trailing follow the reading direction of the canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Anchor {
    #[default]
    TopLeading,
    TopCenter,
    TopTrailing,
    CenterLeading,
    Center,
    CenterTrailing,
    BottomLeading,
    BottomCenter,
    BottomTrailing,
}

/// One adornment: its anchor, its size in points, and the gap kept between it and the canvas edge.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Adornment {
    pub anchor: Anchor,
    pub width: f32,
    pub height: f32,
    /// distance from the canvas edge for the edge anchors; ignored by the centre anchors
    pub inset: f32,
}

impl Adornment {
    pub fn new(anchor: Anchor, width: f32, height: f32) -> Self {
        Adornment { anchor, width, height, inset: 0.0 }
    }

    /// Sets the inset from the canvas edge.
    pub fn with_inset(mut self, inset: f32) -> Self {
        self.inset = inset;
        self
    }

    /// The frame of this adornment inside `canvas`, in the canvas's own coordinates. A size larger than the canvas is
    /// clipped to the canvas, so the frame never leaves it.
    pub fn frame_in(&self, canvas: Rect) -> Rect {
        let w = self.width.clamp(0.0, canvas.w.max(0.0));
        let h = self.height.clamp(0.0, canvas.h.max(0.0));
        let inset = self.inset.max(0.0);
        let left = canvas.x + inset;
        let right = canvas.x + canvas.w - inset - w;
        let top = canvas.y + inset;
        let bottom = canvas.y + canvas.h - inset - h;
        let mid_x = canvas.x + (canvas.w - w) / 2.0;
        let mid_y = canvas.y + (canvas.h - h) / 2.0;
        let (x, y) = match self.anchor {
            Anchor::TopLeading => (left, top),
            Anchor::TopCenter => (mid_x, top),
            Anchor::TopTrailing => (right, top),
            Anchor::CenterLeading => (left, mid_y),
            Anchor::Center => (mid_x, mid_y),
            Anchor::CenterTrailing => (right, mid_y),
            Anchor::BottomLeading => (left, bottom),
            Anchor::BottomCenter => (mid_x, bottom),
            Anchor::BottomTrailing => (right, bottom),
        };
        let x = x.clamp(canvas.x, (canvas.x + canvas.w - w).max(canvas.x));
        let y = y.clamp(canvas.y, (canvas.y + canvas.h - h).max(canvas.y));
        Rect { x, y, w, h }
    }
}

/// The placed frames of a list of adornments, in the order given.
pub fn place_all(adornments: &[Adornment], canvas: Rect) -> Vec<Rect> {
    adornments.iter().map(|a| a.frame_in(canvas)).collect()
}

/// The index of the adornment under the point `(x, y)`. Later adornments are drawn on top, so the last match wins.
pub fn hit_test(frames: &[Rect], x: f32, y: f32) -> Option<usize> {
    frames.iter().rposition(|f| f.contains(x, y))
}
