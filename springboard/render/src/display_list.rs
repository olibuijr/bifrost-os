//! `DisplayList`: records `Canvas` calls as `Op`s with their device-space bounds, for the GPU executor
//! (`bifrost-2d`), damage (`bounds_changed`) and culling. Images are captured by id plus an `Arc` copy of the pixels
//! the first time an id is seen in a list, so the executor can upload them once and cache by id.
//!
//! Each drawing op records the state it was drawn in (`states`: a hash of transform, opacity and clips), so a change
//! of opacity or clip that leaves the op itself alone still counts as damage.
//!
//! Owner: wave G1 agent `canvas` (recorder, state stacks, bounds, tests).

use crate::canvas::{
    text_extent, Canvas, CanvasState, ImageRef, MaterialStyle, PathData, PathPaint, PathVerb, RectF,
    Stop, TextRun, Transform2D,
};
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
    /// The hash of the state (transform, opacity, clips) each op was recorded in, same index as `ops`.
    pub states: Vec<u64>,
    /// The recorder's current state while recording.
    state: CanvasState,
}

/// How many ops the damage diff looks ahead for a match after an insertion or a removal.
const RESYNC_WINDOW: usize = 16;

fn union(a: RectF, b: RectF) -> RectF {
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    let x1 = (a.x + a.w).max(b.x + b.w);
    let y1 = (a.y + a.h).max(b.y + b.h);
    RectF::new(x0, y0, x1 - x0, y1 - y0)
}

/// The parts of op `i` that decide whether it is damage: the op, its device bounds and its state hash.
fn key_of(l: &DisplayList, i: usize) -> (&Op, RectF, u64) {
    (&l.ops[i], l.bounds.get(i).copied().unwrap_or_default(), l.states.get(i).copied().unwrap_or(0))
}

/// The local rectangle a shadow touches: the offset rectangle grown by the blur's reach. Matches `Surface::shadow`.
fn shadow_extent(r: RectF, blur: f32, offset: (f32, f32)) -> RectF {
    let sigma = blur.max(0.0) / 2.0;
    let pad = 3.0 * sigma.round() + 2.0 + offset.0.abs().max(offset.1.abs());
    RectF::new(r.x + offset.0 - pad, r.y + offset.1 - pad, r.w + 2.0 * pad, r.h + 2.0 * pad)
}

/// The local rectangle a text run can touch, from its measured extent (a generous fallback for an unknown face).
fn text_extent_rect(run: &TextRun) -> RectF {
    match text_extent(run) {
        Some((w, h)) => RectF::new(run.x - 2.0, run.y - 2.0, w + 8.0, h + 8.0),
        None => {
            let n = run.text.chars().count() as f32;
            RectF::new(run.x - 2.0, run.y - 2.0, n * run.px * 1.5 + 8.0, run.px * 2.0 + 8.0)
        }
    }
}

/// The local rectangle a path can touch: its points (control points included) grown by the stroke half-width.
fn path_extent(path: &PathData, paint: PathPaint) -> RectF {
    let mut pts: Vec<(f32, f32)> = Vec::new();
    for v in &path.verbs {
        match *v {
            PathVerb::Move(x, y) | PathVerb::Line(x, y) => pts.push((x, y)),
            PathVerb::Quad(cx, cy, x, y) => {
                pts.push((cx, cy));
                pts.push((x, y));
            }
            PathVerb::Cubic(c1x, c1y, c2x, c2y, x, y) => {
                pts.push((c1x, c1y));
                pts.push((c2x, c2y));
                pts.push((x, y));
            }
            PathVerb::Close => {}
        }
    }
    let Some(&(x0, y0)) = pts.first() else { return RectF::default() };
    let (mut x1, mut y1, mut x0, mut y0) = (x0, y0, x0, y0);
    for &(x, y) in &pts {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    let hw = match paint {
        PathPaint::Stroke(w) => w.max(0.0) / 2.0,
        _ => 0.0,
    };
    RectF::new(x0 - hw - 1.0, y0 - hw - 1.0, x1 - x0 + 2.0 * hw + 2.0, y1 - y0 + 2.0 * hw + 2.0)
}

impl DisplayList {
    pub fn new(width: u32, height: u32) -> DisplayList {
        DisplayList { width, height, ..DisplayList::default() }
    }

    /// The union of bounds of ops that differ between `self` and `previous` (positional diff with a resync on
    /// equal ops), for damage. A size change damages everything.
    ///
    /// Two ops are equal when the op, its device bounds and its state hash all match, so an op that is the same
    /// but drawn under a different transform, opacity or clip is damaged. After an insertion or a removal the diff
    /// looks up to `RESYNC_WINDOW` ops ahead for the next equal op before it damages both sides.
    pub fn bounds_changed(&self, previous: &DisplayList) -> Option<RectF> {
        if self.width != previous.width || self.height != previous.height {
            return Some(RectF::new(0.0, 0.0, self.width as f32, self.height as f32));
        }
        let same = |i: usize, j: usize| key_of(self, i) == key_of(previous, j);
        let mine = |i: usize| self.bounds.get(i).copied().unwrap_or_default();
        let theirs = |j: usize| previous.bounds.get(j).copied().unwrap_or_default();
        let (na, nb) = (self.ops.len(), previous.ops.len());
        let (mut i, mut j) = (0usize, 0usize);
        let mut damage: Option<RectF> = None;
        let mut add = |r: RectF| {
            if r.w > 0.0 && r.h > 0.0 {
                damage = Some(match damage {
                    Some(d) => union(d, r),
                    None => r,
                });
            }
        };
        while i < na || j < nb {
            if i < na && j < nb {
                if same(i, j) {
                    i += 1;
                    j += 1;
                    continue;
                }
                // the old list had ops between here and the match: they were removed
                if let Some(k) = (j + 1..nb.min(j + RESYNC_WINDOW + 1)).find(|&k| same(i, k)) {
                    for m in j..k {
                        add(theirs(m));
                    }
                    j = k;
                    continue;
                }
                // the new list has ops between here and the match: they were inserted
                if let Some(m) = (i + 1..na.min(i + RESYNC_WINDOW + 1)).find(|&m| same(m, j)) {
                    for k in i..m {
                        add(mine(k));
                    }
                    i = m;
                    continue;
                }
            }
            if i < na {
                add(mine(i));
                i += 1;
            }
            if j < nb {
                add(theirs(j));
                j += 1;
            }
        }
        damage
    }

    /// Replays the list onto any canvas (the CPU fallback draws a recorded layer this way).
    pub fn replay(&self, target: &mut dyn Canvas) {
        for op in &self.ops {
            match op {
                Op::FillRect { r, color } => target.fill_rect(*r, *color),
                Op::FillRRect { r, radius, color } => target.fill_rrect(*r, *radius, *color),
                Op::StrokeRRect { r, radius, width, color } => target.stroke_rrect(*r, *radius, *width, *color),
                Op::LinearGradient { r, radius, start, end, stops } => {
                    target.linear_gradient(*r, *radius, *start, *end, stops)
                }
                Op::Shadow { r, radius, blur, offset, color } => target.shadow(*r, *radius, *blur, *offset, *color),
                Op::Image { id, dst, alpha } => {
                    if let Some(image) = self.captured(*id) {
                        target.image(ImageRef { id: *id, image }, *dst, *alpha);
                    }
                }
                Op::ImageTinted { id, dst, color } => {
                    if let Some(image) = self.captured(*id) {
                        target.image_tinted(ImageRef { id: *id, image }, *dst, *color);
                    }
                }
                Op::Text { run, color } => target.text(run, *color),
                Op::Path { path, paint, color } => target.path(path, *paint, *color),
                Op::Material { r, radius, style } => target.material(*r, *radius, *style),
                Op::PushClip { r, radius } => target.push_clip(*r, *radius),
                Op::PopClip => target.pop_clip(),
                Op::PushTransform(t) => target.push_transform(*t),
                Op::PopTransform => target.pop_transform(),
                Op::PushOpacity(a) => target.push_opacity(*a),
                Op::PopOpacity => target.pop_opacity(),
            }
        }
    }

    /// The captured pixels of an image id, if this list holds them.
    pub fn captured(&self, id: u64) -> Option<&Image> {
        self.images.iter().find(|(k, _)| *k == id).map(|(_, img)| img.as_ref())
    }

    /// Records a drawing op whose local extent is `local`.
    fn push_draw(&mut self, op: Op, local: RectF) {
        let b = self.state.device_bounds(local);
        self.states.push(self.state.key());
        self.bounds.push(b);
        self.ops.push(op);
    }

    /// Records a state op (no bounds).
    fn push_state(&mut self, op: Op) {
        self.states.push(self.state.key());
        self.bounds.push(RectF::default());
        self.ops.push(op);
    }

    fn capture(&mut self, id: u64, image: &Image) {
        if !self.images.iter().any(|(k, _)| *k == id) {
            self.images.push((id, Arc::new(image.clone())));
        }
    }
}

impl Canvas for DisplayList {
    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn fill_rect(&mut self, r: RectF, color: u32) {
        self.push_draw(Op::FillRect { r, color }, r);
    }

    fn fill_rrect(&mut self, r: RectF, radius: f32, color: u32) {
        self.push_draw(Op::FillRRect { r, radius, color }, r);
    }

    fn stroke_rrect(&mut self, r: RectF, radius: f32, width: f32, color: u32) {
        self.push_draw(Op::StrokeRRect { r, radius, width, color }, r);
    }

    fn linear_gradient(&mut self, r: RectF, radius: f32, start: (f32, f32), end: (f32, f32), stops: &[Stop]) {
        self.push_draw(Op::LinearGradient { r, radius, start, end, stops: stops.to_vec() }, r);
    }

    fn shadow(&mut self, r: RectF, radius: f32, blur: f32, offset: (f32, f32), color: u32) {
        let local = shadow_extent(r, blur, offset);
        self.push_draw(Op::Shadow { r, radius, blur, offset, color }, local);
    }

    fn image(&mut self, img: ImageRef<'_>, dst: RectF, alpha: f32) {
        self.capture(img.id, img.image);
        self.push_draw(Op::Image { id: img.id, dst, alpha }, dst);
    }

    fn image_tinted(&mut self, img: ImageRef<'_>, dst: RectF, color: u32) {
        self.capture(img.id, img.image);
        self.push_draw(Op::ImageTinted { id: img.id, dst, color }, dst);
    }

    fn text(&mut self, run: &TextRun, color: u32) {
        let local = text_extent_rect(run);
        self.push_draw(Op::Text { run: run.clone(), color }, local);
    }

    fn path(&mut self, path: &PathData, paint: PathPaint, color: u32) {
        // a path with no points draws nothing; flatten keeps the same geometry the executor will see
        let local = path_extent(path, paint);
        self.push_draw(Op::Path { path: path.clone(), paint, color }, local);
    }

    fn material(&mut self, r: RectF, radius: f32, style: MaterialStyle) {
        self.push_draw(Op::Material { r, radius, style }, r);
    }

    fn push_clip(&mut self, r: RectF, radius: f32) {
        self.state.push_clip(r, radius);
        self.push_state(Op::PushClip { r, radius });
    }

    fn pop_clip(&mut self) {
        self.state.pop_clip();
        self.push_state(Op::PopClip);
    }

    fn push_transform(&mut self, t: Transform2D) {
        self.state.push_transform(t);
        self.push_state(Op::PushTransform(t));
    }

    fn pop_transform(&mut self) {
        self.state.pop_transform();
        self.push_state(Op::PopTransform);
    }

    fn push_opacity(&mut self, alpha: f32) {
        self.state.push_opacity(alpha);
        self.push_state(Op::PushOpacity(alpha));
    }

    fn pop_opacity(&mut self) {
        self.state.pop_opacity();
        self.push_state(Op::PopOpacity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::{register_face, reset_surface_state};
    use crate::gfx::Surface;

    const WHITE: u32 = 0xffff_ffff;
    const BLUE: u32 = 0xffff_0000;

    fn fill(l: &mut DisplayList, r: RectF, c: u32) {
        l.fill_rect(r, c);
    }

    #[test]
    fn bounds_follow_transform_and_clip() {
        let mut l = DisplayList::new(100, 100);
        l.push_transform(Transform2D::translate(10.0, 10.0));
        l.fill_rect(RectF::new(0.0, 0.0, 20.0, 20.0), WHITE);
        assert_eq!(l.bounds[1], RectF::new(10.0, 10.0, 20.0, 20.0));
        l.push_clip(RectF::new(0.0, 0.0, 15.0, 15.0), 0.0);
        l.fill_rect(RectF::new(0.0, 0.0, 20.0, 20.0), WHITE);
        assert_eq!(l.bounds[3], RectF::new(10.0, 10.0, 15.0, 15.0));
        l.pop_clip();
        l.pop_transform();
        l.fill_rect(RectF::new(0.0, 0.0, 5.0, 5.0), WHITE);
        assert_eq!(l.bounds[6], RectF::new(0.0, 0.0, 5.0, 5.0));
    }

    #[test]
    fn state_ops_have_empty_bounds_and_parallel_lists() {
        let mut l = DisplayList::new(10, 10);
        l.push_opacity(0.5);
        fill(&mut l, RectF::new(0.0, 0.0, 4.0, 4.0), WHITE);
        l.pop_opacity();
        assert_eq!(l.ops.len(), 3);
        assert_eq!(l.bounds.len(), 3);
        assert_eq!(l.states.len(), 3);
        assert_eq!(l.bounds[0], RectF::default());
        assert_eq!(l.bounds[2], RectF::default());
        assert_eq!(l.bounds[1], RectF::new(0.0, 0.0, 4.0, 4.0));
    }

    #[test]
    fn identical_lists_have_no_damage() {
        let build = || {
            let mut l = DisplayList::new(50, 50);
            l.fill_rect(RectF::new(1.0, 1.0, 5.0, 5.0), WHITE);
            l.push_clip(RectF::new(0.0, 0.0, 20.0, 20.0), 3.0);
            l.fill_rrect(RectF::new(2.0, 2.0, 9.0, 9.0), 2.0, BLUE);
            l.pop_clip();
            l
        };
        assert_eq!(build().bounds_changed(&build()), None);
    }

    #[test]
    fn a_changed_op_damages_only_its_bounds() {
        let r1 = RectF::new(1.0, 1.0, 5.0, 5.0);
        let r2 = RectF::new(20.0, 20.0, 6.0, 4.0);
        let mut a = DisplayList::new(50, 50);
        a.fill_rect(r1, WHITE);
        a.fill_rect(r2, WHITE);
        let mut b = DisplayList::new(50, 50);
        b.fill_rect(r1, WHITE);
        b.fill_rect(r2, BLUE);
        assert_eq!(b.bounds_changed(&a), Some(r2));
    }

    #[test]
    fn a_transform_change_damages_the_moved_op() {
        let r = RectF::new(0.0, 0.0, 4.0, 4.0);
        let mut a = DisplayList::new(50, 50);
        a.push_transform(Transform2D::IDENTITY);
        a.fill_rect(r, WHITE);
        let mut b = DisplayList::new(50, 50);
        b.push_transform(Transform2D::translate(5.0, 0.0));
        b.fill_rect(r, WHITE);
        let d = b.bounds_changed(&a).expect("moved op is damage");
        assert!(d.x <= 0.0 && d.x + d.w >= 9.0, "{d:?}");
    }

    #[test]
    fn an_opacity_change_damages_an_op_that_is_otherwise_equal() {
        let r = RectF::new(3.0, 3.0, 4.0, 4.0);
        let mut a = DisplayList::new(20, 20);
        a.fill_rect(r, WHITE);
        let mut b = DisplayList::new(20, 20);
        b.push_opacity(0.5);
        b.fill_rect(r, WHITE);
        b.pop_opacity();
        let d = b.bounds_changed(&a).expect("opacity is part of the damage");
        assert!(d.x <= 3.0 && d.x + d.w >= 7.0);
    }

    #[test]
    fn a_size_change_damages_everything_and_insertions_resync() {
        let a = DisplayList::new(10, 10);
        let b = DisplayList::new(20, 10);
        assert_eq!(b.bounds_changed(&a), Some(RectF::new(0.0, 0.0, 20.0, 10.0)));
        // one op inserted before two equal ops: only the inserted op is damage
        let x = RectF::new(0.0, 0.0, 2.0, 2.0);
        let y = RectF::new(8.0, 8.0, 2.0, 2.0);
        let mut old = DisplayList::new(20, 20);
        old.fill_rect(x, WHITE);
        old.fill_rect(y, WHITE);
        let mut new = DisplayList::new(20, 20);
        new.fill_rect(RectF::new(15.0, 0.0, 3.0, 3.0), BLUE);
        new.fill_rect(x, WHITE);
        new.fill_rect(y, WHITE);
        assert_eq!(new.bounds_changed(&old), Some(RectF::new(15.0, 0.0, 3.0, 3.0)));
    }

    #[test]
    fn images_are_captured_once_per_id() {
        let img = Image { w: 2, h: 2, px: vec![WHITE; 4] };
        let mut l = DisplayList::new(10, 10);
        l.image(ImageRef { id: 9, image: &img }, RectF::new(0.0, 0.0, 2.0, 2.0), 1.0);
        l.image_tinted(ImageRef { id: 9, image: &img }, RectF::new(0.0, 0.0, 2.0, 2.0), BLUE);
        assert_eq!(l.images.len(), 1);
        assert_eq!(l.captured(9), Some(&img));
        assert_eq!(l.ops.len(), 2);
    }

    #[test]
    fn replay_into_a_surface_matches_direct_drawing() {
        reset_surface_state();
        let record = |c: &mut dyn Canvas| {
            c.push_transform(Transform2D::translate(2.0, 1.0));
            c.fill_rect(RectF::new(0.0, 0.0, 3.0, 3.0), BLUE);
            c.push_clip(RectF::new(0.0, 0.0, 4.0, 4.0), 0.0);
            c.fill_rect(RectF::new(0.0, 0.0, 10.0, 10.0), WHITE);
            c.pop_clip();
            c.pop_transform();
        };
        let mut list = DisplayList::new(10, 10);
        record(&mut list);
        let mut replayed = vec![0u32; 10 * 10];
        {
            let mut s = Surface { w: 10, h: 10, stride: 10, px: &mut replayed[..], y0: 0 };
            list.replay(&mut s);
        }
        let mut direct = vec![0u32; 10 * 10];
        {
            let mut s = Surface { w: 10, h: 10, stride: 10, px: &mut direct[..], y0: 0 };
            record(&mut s);
        }
        assert_eq!(replayed, direct);
        assert_eq!(direct[1 * 10 + 2], WHITE);
        assert_eq!(direct[1 * 10 + 7], 0);
    }

    #[test]
    fn replay_into_a_list_round_trips_ops_and_bounds() {
        let mut list = DisplayList::new(30, 30);
        list.push_opacity(0.25);
        list.fill_rrect(RectF::new(1.0, 2.0, 8.0, 9.0), 3.0, BLUE);
        list.shadow(RectF::new(5.0, 5.0, 6.0, 6.0), 2.0, 8.0, (1.0, 2.0), WHITE);
        list.pop_opacity();
        let mut copy = DisplayList::new(30, 30);
        list.replay(&mut copy);
        assert_eq!(copy.ops, list.ops);
        assert_eq!(copy.bounds, list.bounds);
        assert_eq!(copy.states, list.states);
    }

    #[test]
    fn unbalanced_pops_do_not_panic_and_leave_the_state_alone() {
        let mut l = DisplayList::new(10, 10);
        l.pop_clip();
        l.pop_transform();
        l.pop_opacity();
        l.fill_rect(RectF::new(0.0, 0.0, 2.0, 2.0), WHITE);
        assert_eq!(l.bounds[3], RectF::new(0.0, 0.0, 2.0, 2.0));
    }

    #[test]
    fn path_and_text_bounds_cover_their_geometry() {
        let path = PathData { verbs: vec![PathVerb::Move(2.0, 2.0), PathVerb::Cubic(0.0, 10.0, 10.0, 10.0, 10.0, 2.0), PathVerb::Close] };
        let mut l = DisplayList::new(30, 30);
        l.path(&path, PathPaint::Stroke(4.0), WHITE);
        let b = l.bounds[0];
        assert!(b.x <= 0.0 && b.y <= 2.0 - 2.0 && b.x + b.w >= 10.0 && b.y + b.h >= 10.0, "{b:?}");
        let data: &'static [u8] = include_bytes!("../../assets/SFUI.ttf");
        let face = register_face(data);
        let run_ = TextRun { face, text: "Hello".into(), px: 16.0, vars: vec![], x: 3.0, y: 4.0 };
        l.text(&run_, WHITE);
        let t = l.bounds[1];
        assert!(t.x <= 3.0 && t.y <= 4.0 && t.w > 30.0 && t.h > 10.0, "{t:?}");
    }
}
