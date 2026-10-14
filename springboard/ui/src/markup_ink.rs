//! Ink for markup canvases: the pointer model behind the pen, marker, pencil and eraser of the markup editor.
//!
//! A pointer press starts a stroke, moves extend it and a release commits it. The eraser has no stroke of its own:
//! while it moves it removes every committed stroke its path touches, and a gesture that removes nothing leaves no
//! undo step. Points are kept inside the canvas, as the editor keeps them inside the capture. Non-finite pointer
//! positions are ignored, so a bad event never reaches the stored strokes.
//!
//! This is the pure model. Drawing the strokes is left to the caller. The shell editor in `springboard/src/shell/markup.rs`
//! still keeps its own stroke list, so this canvas is the model apps and the shell can share, not yet the shell's editor.

/// How many committed edits undo keeps. Undo never holds more steps than this.
pub const HISTORY_LIMIT: usize = 100;

/// The tools of the markup editor. Each draws with a width in canvas points, and an eraser removes within its width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InkTool {
    Pen,
    Marker,
    Pencil,
    Eraser,
}

impl InkTool {
    /// Stroke width in canvas points. For the eraser this is the width of the eraser path.
    pub fn width(self) -> f32 {
        match self {
            InkTool::Pen => 3.0,
            InkTool::Marker => 14.0,
            InkTool::Pencil => 2.0,
            InkTool::Eraser => 16.0,
        }
    }

    /// Opacity of a stroke the tool makes, from 0 (clear) to 255 (solid).
    pub fn opacity(self) -> u8 {
        match self {
            InkTool::Pen => 255,
            InkTool::Marker => 110,
            InkTool::Pencil => 190,
            InkTool::Eraser => 255,
        }
    }
}

/// A committed stroke: the tool and colour it was drawn with, and its points in canvas points.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub tool: InkTool,
    /// Colour as 0xRRGGBB. The tool's opacity applies when the stroke is drawn.
    pub color: u32,
    pub points: Vec<(f32, f32)>,
}

/// Why a canvas cannot be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InkError {
    /// A width or height is not finite and above zero.
    InvalidSize,
}

/// A canvas of strokes with one current tool and colour, and undo for each committed change.
#[derive(Clone, Debug, PartialEq)]
pub struct InkCanvas {
    width: f32,
    height: f32,
    strokes: Vec<Stroke>,
    active: Option<Stroke>,
    tool: InkTool,
    color: u32,
    undo: Vec<Vec<Stroke>>,
}

impl InkCanvas {
    /// An empty canvas with the pen in black. The size must be finite and above zero.
    pub fn new(width: f32, height: f32) -> Result<InkCanvas, InkError> {
        if !(width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0) {
            return Err(InkError::InvalidSize);
        }
        Ok(InkCanvas { width, height, strokes: Vec::new(), active: None, tool: InkTool::Pen, color: 0, undo: Vec::new() })
    }

    /// The canvas size in points.
    pub fn size(&self) -> (f32, f32) {
        (self.width, self.height)
    }

    pub fn tool(&self) -> InkTool {
        self.tool
    }

    /// Changes the tool. A gesture in progress keeps the tool it started with.
    pub fn set_tool(&mut self, tool: InkTool) {
        self.tool = tool;
    }

    pub fn color(&self) -> u32 {
        self.color
    }

    /// Sets the colour. Only the low 24 bits are kept, so the value is always 0xRRGGBB.
    pub fn set_color(&mut self, color: u32) {
        self.color = color & 0x00ff_ffff;
    }

    /// The committed strokes, oldest first.
    pub fn strokes(&self) -> &[Stroke] {
        &self.strokes
    }

    /// The stroke being drawn, if a pointer is down on a pen or the eraser.
    pub fn active(&self) -> Option<&Stroke> {
        self.active.as_ref()
    }

    pub fn can_undo(&self) -> bool {
        self.active.is_none() && !self.undo.is_empty()
    }

    /// Starts a gesture at the point. A press while a gesture is already running is ignored.
    pub fn pointer_down(&mut self, x: f32, y: f32) {
        if self.active.is_some() || !(x.is_finite() && y.is_finite()) {
            return;
        }
        let point = self.clamp(x, y);
        self.undo.push(self.strokes.clone());
        self.active = Some(Stroke { tool: self.tool, color: self.color, points: vec![point] });
        if self.active.as_ref().is_some_and(|stroke| stroke.tool == InkTool::Eraser) {
            self.erase_last_segment();
        }
    }

    /// Extends the gesture to the point. Without a gesture in progress this does nothing.
    pub fn pointer_move(&mut self, x: f32, y: f32) {
        if self.active.is_none() || !(x.is_finite() && y.is_finite()) {
            return;
        }
        let point = self.clamp(x, y);
        if let Some(stroke) = self.active.as_mut() {
            stroke.points.push(point);
        }
        if self.tool == InkTool::Eraser {
            self.erase_last_segment();
        }
    }

    /// Ends the gesture. A pen stroke is committed. An eraser gesture that removed nothing is taken back out of the
    /// undo history. Returns true when the strokes changed.
    pub fn pointer_up(&mut self) -> bool {
        let Some(stroke) = self.active.take() else {
            return false;
        };
        if stroke.tool == InkTool::Eraser {
            let removed_any = self.undo.last().is_some_and(|before| before.len() != self.strokes.len());
            if !removed_any {
                self.undo.pop();
            } else {
                self.trim_undo();
            }
            return removed_any;
        }
        self.strokes.push(stroke);
        self.trim_undo();
        true
    }

    /// Restores the strokes from before the last committed change. A gesture in progress is not undone.
    pub fn undo(&mut self) -> bool {
        if self.active.is_some() {
            return false;
        }
        match self.undo.pop() {
            Some(before) => {
                self.strokes = before;
                true
            }
            None => false,
        }
    }

    fn clamp(&self, x: f32, y: f32) -> (f32, f32) {
        (x.clamp(0.0, self.width), y.clamp(0.0, self.height))
    }

    fn trim_undo(&mut self) {
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
    }

    /// Removes every committed stroke that comes within the eraser's reach of the last segment of the eraser path.
    fn erase_last_segment(&mut self) {
        let Some(eraser) = self.active.as_ref() else {
            return;
        };
        let (a, b) = match eraser.points.as_slice() {
            [.., a, b] => (*a, *b),
            [only] => (*only, *only),
            [] => return,
        };
        let reach = InkTool::Eraser.width() / 2.0;
        self.strokes.retain(|stroke| {
            let limit = reach + stroke.tool.width() / 2.0;
            !stroke_touches(stroke, a, b, limit)
        });
    }
}

/// True when some segment of the stroke comes within `limit` of the segment `a` to `b`.
fn stroke_touches(stroke: &Stroke, a: (f32, f32), b: (f32, f32), limit: f32) -> bool {
    let points = stroke.points.as_slice();
    let limit_sq = limit * limit;
    match points {
        [] => false,
        [only] => segment_gap_sq(*only, *only, a, b) <= limit_sq,
        _ => points.windows(2).any(|pair| segment_gap_sq(pair[0], pair[1], a, b) <= limit_sq),
    }
}

/// The squared distance between two segments, zero when they cross.
fn segment_gap_sq(p1: (f32, f32), p2: (f32, f32), q1: (f32, f32), q2: (f32, f32)) -> f32 {
    if segments_cross(p1, p2, q1, q2) {
        return 0.0;
    }
    point_segment_sq(p1, q1, q2).min(point_segment_sq(p2, q1, q2)).min(point_segment_sq(q1, p1, p2)).min(point_segment_sq(q2, p1, p2))
}

fn orientation(a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> f32 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

fn segments_cross(p1: (f32, f32), p2: (f32, f32), q1: (f32, f32), q2: (f32, f32)) -> bool {
    let d1 = orientation(q1, q2, p1);
    let d2 = orientation(q1, q2, p2);
    let d3 = orientation(p1, p2, q1);
    let d4 = orientation(p1, p2, q2);
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

/// The squared distance from `p` to the segment `a` to `b`.
fn point_segment_sq(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length_sq = dx * dx + dy * dy;
    let t = if length_sq == 0.0 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length_sq).clamp(0.0, 1.0) };
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    (p.0 - cx) * (p.0 - cx) + (p.1 - cy) * (p.1 - cy)
}
