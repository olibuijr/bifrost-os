//! `DisplayList`: records `Canvas` calls as `Op`s with their device-space bounds, for the GPU executor
//! (`bifrost-2d`), damage (`bounds_changed`) and culling. Images are captured by id plus an `Arc` copy of the pixels
//! the first time an id is seen in a list, so the executor can upload them once and cache by id.
//!
//! Owner: wave G1 agent `canvas` (recorder, state stacks, bounds, tests).

use crate::canvas::{Canvas, ImageRef, MaterialStyle, PathData, PathPaint, RectF, Stop, TextRun, Transform2D};
use crate::gfx::Image;
use std::sync::Arc;

/// One recorded drawing operation, in the transform/clip/opacity state current when it was recorded.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    FillRect { r: RectF, color: u32 },
    FillRRect { r: RectF, radius: f32, color: u32 },
    StrokeRRect { r: RectF, radius: f32, width: f32, color: u32 },
    LinearGradient { r: RectF, radius: f32, start: (f32, f32), end: (f32, f32), stops: Vec<Stop> },
    Shadow { r: RectF, radius: f32, blur: f32, offset: (f32, f32), color: u32 },
    Image { id: u64, dst: RectF, alpha: f32 },
    ImageTinted { id: u64, dst: RectF, color: u32 },
    Text { run: TextRun, color: u32 },
    Path { path: PathData, paint: PathPaint, color: u32 },
    Material { r: RectF, radius: f32, style: MaterialStyle },
    PushClip { r: RectF, radius: f32 },
    PopClip,
    PushTransform(Transform2D),
    PopTransform,
    PushOpacity(f32),
    PopOpacity,
}

/// A recorded frame (or layer) of drawing.
#[derive(Debug, Clone, Default)]
pub struct DisplayList {
    pub width: u32,
    pub height: u32,
    pub ops: Vec<Op>,
    /// Device-space bounds of each op (same index as `ops`), after transform and clip; state ops get empty bounds.
    pub bounds: Vec<RectF>,
    /// Pixels of every image id used, captured once per list.
    pub images: Vec<(u64, Arc<Image>)>,
}

impl DisplayList {
    pub fn new(width: u32, height: u32) -> DisplayList {
        DisplayList { width, height, ..DisplayList::default() }
    }

    /// The union of bounds of ops that differ between `self` and `previous` (positional diff with a resync on
    /// equal ops), for damage. A size change damages everything.
    pub fn bounds_changed(&self, previous: &DisplayList) -> Option<RectF> {
        let _ = previous;
        todo!("canvas")
    }

    /// Replays the list onto any canvas (the CPU fallback draws a recorded layer this way).
    pub fn replay(&self, target: &mut dyn Canvas) {
        let _ = target;
        todo!("canvas")
    }
}

impl Canvas for DisplayList {
    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    fn fill_rect(&mut self, r: RectF, color: u32) {
        let _ = (r, color);
        todo!("canvas")
    }
    fn fill_rrect(&mut self, r: RectF, radius: f32, color: u32) {
        let _ = (r, radius, color);
        todo!("canvas")
    }
    fn stroke_rrect(&mut self, r: RectF, radius: f32, width: f32, color: u32) {
        let _ = (r, radius, width, color);
        todo!("canvas")
    }
    fn linear_gradient(&mut self, r: RectF, radius: f32, start: (f32, f32), end: (f32, f32), stops: &[Stop]) {
        let _ = (r, radius, start, end, stops);
        todo!("canvas")
    }
    fn shadow(&mut self, r: RectF, radius: f32, blur: f32, offset: (f32, f32), color: u32) {
        let _ = (r, radius, blur, offset, color);
        todo!("canvas")
    }
    fn image(&mut self, img: ImageRef<'_>, dst: RectF, alpha: f32) {
        let _ = (img, dst, alpha);
        todo!("canvas")
    }
    fn image_tinted(&mut self, img: ImageRef<'_>, dst: RectF, color: u32) {
        let _ = (img, dst, color);
        todo!("canvas")
    }
    fn text(&mut self, run: &TextRun, color: u32) {
        let _ = (run, color);
        todo!("canvas")
    }
    fn path(&mut self, path: &PathData, paint: PathPaint, color: u32) {
        let _ = (path, paint, color);
        todo!("canvas")
    }
    fn material(&mut self, r: RectF, radius: f32, style: MaterialStyle) {
        let _ = (r, radius, style);
        todo!("canvas")
    }
    fn push_clip(&mut self, r: RectF, radius: f32) {
        let _ = (r, radius);
        todo!("canvas")
    }
    fn pop_clip(&mut self) {
        todo!("canvas")
    }
    fn push_transform(&mut self, t: Transform2D) {
        let _ = t;
        todo!("canvas")
    }
    fn pop_transform(&mut self) {
        todo!("canvas")
    }
    fn push_opacity(&mut self, alpha: f32) {
        let _ = alpha;
        todo!("canvas")
    }
    fn pop_opacity(&mut self) {
        todo!("canvas")
    }
}
