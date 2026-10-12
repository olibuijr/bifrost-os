//! `Canvas`: the one drawing vocabulary of BifrOSt (docs/graphics-architecture.md, "Canvas, DisplayList").
//!
//! `Surface` implements it on the CPU with its existing methods, so all current drawing keeps working;
//! `display_list::DisplayList` records it for the GPU executor (`bifrost-2d`). Coordinates are f32 device pixels with
//! the origin at the top left; colours are packed premultiplied RGBA8 (`gfx::pack`).
//!
//! State (transform, clip, opacity) lives in `CanvasState`. `DisplayList` owns one; `Surface` keeps one per thread,
//! because `Surface` has no room for it without changing every struct literal that builds one (see
//! `docs/graphics/g1-canvas.md`). A draw with the identity transform, no clip and full opacity goes straight to the existing `Surface`
//! methods. Anything else is drawn into a scratch buffer and composited with the clip coverage and the opacity; a
//! transform that is not a translation samples the scratch buffer through the inverse transform (nearest neighbour).
//!
//! Owner: wave G1 agent `canvas`. The trait itself is the contract: change it only with the lead.

use crate::font::{Font, Style};
use crate::gfx::{over, rrect_coverage, scale, Image, Surface};
use crate::gradient::linear_t;
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

/// A rectangle in device pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl RectF {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> RectF {
        RectF { x, y, w, h }
    }
}

/// A 2D affine transform `[a c tx; b d ty]`: `x' = a x + c y + tx`, `y' = b x + d y + ty`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform2D {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Transform2D {
    pub const IDENTITY: Transform2D = Transform2D { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 };

    pub fn translate(tx: f32, ty: f32) -> Transform2D {
        Transform2D { tx, ty, ..Transform2D::IDENTITY }
    }

    pub fn scale(sx: f32, sy: f32) -> Transform2D {
        Transform2D { a: sx, d: sy, ..Transform2D::IDENTITY }
    }

    /// `self · o`: `o` is applied to a point first, then `self`.
    pub fn mul(self, o: Transform2D) -> Transform2D {
        Transform2D {
            a: self.a * o.a + self.c * o.b,
            b: self.b * o.a + self.d * o.b,
            c: self.a * o.c + self.c * o.d,
            d: self.b * o.c + self.d * o.d,
            tx: self.a * o.tx + self.c * o.ty + self.tx,
            ty: self.b * o.tx + self.d * o.ty + self.ty,
        }
    }

    pub fn apply(self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.tx, self.b * x + self.d * y + self.ty)
    }

    /// The inverse, or `None` when the transform is singular.
    pub fn inverse(self) -> Option<Transform2D> {
        let det = self.a * self.d - self.b * self.c;
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Some(Transform2D { a, b, c, d, tx: -(a * self.tx + c * self.ty), ty: -(b * self.tx + d * self.ty) })
    }

    /// True when the linear part is the identity, so the transform only moves things.
    pub fn is_translation(self) -> bool {
        self.a == 1.0 && self.b == 0.0 && self.c == 0.0 && self.d == 1.0
    }
}

/// The bounding box of a rectangle after a transform.
pub fn bbox_of(t: Transform2D, r: RectF) -> RectF {
    let pts = [t.apply(r.x, r.y), t.apply(r.x + r.w, r.y), t.apply(r.x, r.y + r.h), t.apply(r.x + r.w, r.y + r.h)];
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (x, y) in pts {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    RectF::new(x0, y0, x1 - x0, y1 - y0)
}

/// The overlap of two rectangles; zero width or height when they do not meet.
pub fn intersect(a: RectF, b: RectF) -> RectF {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.w).min(b.x + b.w);
    let y1 = (a.y + a.h).min(b.y + b.h);
    RectF::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

/// A colour stop of a gradient: position 0..1 and packed colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stop {
    pub at: f32,
    pub color: u32,
}

/// An image the GPU side can cache: `id` is stable for the same pixels (a content hash or an owner-assigned id that
/// changes when the pixels change).
#[derive(Debug, Clone, Copy)]
pub struct ImageRef<'a> {
    pub id: u64,
    pub image: &'a Image,
}

/// A run of text in one face and size, positioned at its top-left (`x`, `y`), as `font::Font::render` draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    /// Identifies the face (the address of its font data, stable for the process lifetime). Register the data with
    /// `register_face` to get the id.
    pub face: u64,
    pub text: String,
    pub px: f32,
    /// Variation axis values (weight and so on), as `font::Var` tags and values.
    pub vars: Vec<([u8; 4], f32)>,
    pub x: f32,
    pub y: f32,
}

/// A path made of move, line, quadratic and cubic segments, in device pixels.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PathData {
    pub verbs: Vec<PathVerb>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathVerb {
    Move(f32, f32),
    Line(f32, f32),
    Quad(f32, f32, f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}

/// How a path is painted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathPaint {
    FillNonZero,
    FillEvenOdd,
    /// Stroke with this width (round joins and caps).
    Stroke(f32),
}

/// A behind-content material (glass). The window server blurs what is behind it; on the CPU path `Surface` uses its
/// own `glass_rrect`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialStyle {
    pub blur: f32,
    pub tint: u32,
    pub saturation: f32,
}

/// The drawing vocabulary. Every method draws in the current transform, clipped to the current clip, at the
/// current opacity.
pub trait Canvas {
    /// The target size in device pixels.
    fn size(&self) -> (u32, u32);

    fn fill_rect(&mut self, r: RectF, color: u32);
    fn fill_rrect(&mut self, r: RectF, radius: f32, color: u32);
    /// A rounded-rectangle outline of width `width` drawn inside `r`.
    fn stroke_rrect(&mut self, r: RectF, radius: f32, width: f32, color: u32);
    /// A linear gradient filling the rounded rectangle, from `start` to `end` (device pixels).
    fn linear_gradient(&mut self, r: RectF, radius: f32, start: (f32, f32), end: (f32, f32), stops: &[Stop]);
    /// A soft shadow of a rounded rectangle: Gaussian with standard deviation `blur / 2`, offset by `offset`.
    fn shadow(&mut self, r: RectF, radius: f32, blur: f32, offset: (f32, f32), color: u32);
    /// An image scaled into `dst`, multiplied by `alpha` (0..1).
    fn image(&mut self, img: ImageRef<'_>, dst: RectF, alpha: f32);
    /// An image used as a coverage mask and filled with `color` (symbols, pre-rendered text).
    fn image_tinted(&mut self, img: ImageRef<'_>, dst: RectF, color: u32);
    fn text(&mut self, run: &TextRun, color: u32);
    fn path(&mut self, path: &PathData, paint: PathPaint, color: u32);
    fn material(&mut self, r: RectF, radius: f32, style: MaterialStyle);

    /// Intersects the clip with a rounded rectangle until the matching `pop_clip`.
    fn push_clip(&mut self, r: RectF, radius: f32);
    fn pop_clip(&mut self);
    /// Concatenates a transform until the matching `pop_transform`.
    fn push_transform(&mut self, t: Transform2D);
    fn pop_transform(&mut self);
    /// Multiplies opacity until the matching `pop_opacity`.
    fn push_opacity(&mut self, alpha: f32);
    fn pop_opacity(&mut self);
}

// ---------------------------------------------------------------------------------------------------------------------
// State: transform, clip and opacity stacks, shared by `DisplayList` and `Surface`.
// ---------------------------------------------------------------------------------------------------------------------

/// One clip in the stack: a rounded rectangle in the space of the transform current when it was pushed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipEntry {
    pub r: RectF,
    /// The corner radius, clamped to half the smaller side.
    pub radius: f32,
    /// The transform in effect when the clip was pushed.
    pub t: Transform2D,
    pub(crate) inv: Option<Transform2D>,
    /// Device-space bounding box of the clip.
    pub(crate) bbox: RectF,
}

/// The current transform, the clip stack and the opacity, with the stacks that restore them.
#[derive(Debug, Clone, PartialEq)]
pub struct CanvasState {
    t: Transform2D,
    transforms: Vec<Transform2D>,
    clips: Vec<ClipEntry>,
    alpha: f32,
    alphas: Vec<f32>,
}

impl Default for CanvasState {
    fn default() -> Self {
        CanvasState { t: Transform2D::IDENTITY, transforms: Vec::new(), clips: Vec::new(), alpha: 1.0, alphas: Vec::new() }
    }
}

impl CanvasState {
    /// Whether every stack is back at its start (no clip, transform or opacity pushed).
    pub fn is_empty(&self) -> bool {
        self.transforms.is_empty() && self.clips.is_empty() && self.alphas.is_empty()
    }

    pub fn transform(&self) -> Transform2D {
        self.t
    }

    pub fn alpha(&self) -> f32 {
        self.alpha
    }

    pub fn clips(&self) -> &[ClipEntry] {
        &self.clips
    }

    pub fn push_transform(&mut self, t: Transform2D) {
        self.transforms.push(self.t);
        self.t = self.t.mul(t);
    }

    /// Restores the transform saved by the matching push. An unbalanced pop keeps the current transform.
    pub fn pop_transform(&mut self) {
        if let Some(t) = self.transforms.pop() {
            self.t = t;
        }
    }

    pub fn push_clip(&mut self, r: RectF, radius: f32) {
        let inv = self.t.inverse();
        let bbox = if inv.is_some() { bbox_of(self.t, r) } else { RectF::default() };
        let radius = radius.max(0.0).min(r.w.max(0.0) / 2.0).min(r.h.max(0.0) / 2.0);
        self.clips.push(ClipEntry { r, radius, t: self.t, inv, bbox });
    }

    pub fn pop_clip(&mut self) {
        self.clips.pop();
    }

    pub fn push_opacity(&mut self, alpha: f32) {
        self.alphas.push(self.alpha);
        let a = if alpha.is_nan() { 0.0 } else { alpha.clamp(0.0, 1.0) };
        self.alpha *= a;
    }

    pub fn pop_opacity(&mut self) {
        if let Some(a) = self.alphas.pop() {
            self.alpha = a;
        }
    }

    /// The device rectangle every clip allows, or `None` with no clip.
    pub fn clip_box(&self) -> Option<RectF> {
        self.clips.iter().map(|c| c.bbox).reduce(intersect)
    }

    /// The device-space bounds of a local rectangle drawn now: its transformed box, cut to the clips.
    pub fn device_bounds(&self, local: RectF) -> RectF {
        let b = bbox_of(self.t, local);
        match self.clip_box() {
            Some(cb) => intersect(b, cb),
            None => b,
        }
    }

    /// Coverage 0..1 of every clip at the device point (x, y): the product of the clips' anti-aliased masks.
    pub fn clip_coverage(&self, x: f32, y: f32) -> f32 {
        let mut c = 1.0f32;
        for k in &self.clips {
            let Some(inv) = k.inv else { return 0.0 };
            if k.r.w <= 0.0 || k.r.h <= 0.0 {
                return 0.0;
            }
            let (lx, ly) = inv.apply(x, y);
            c *= rrect_coverage(lx - k.r.x, ly - k.r.y, k.r.w, k.r.h, k.radius);
            if c <= 0.0 {
                return 0.0;
            }
        }
        c
    }

    /// A hash of everything that changes how the next drawing op looks: transform, opacity and clips.
    pub fn key(&self) -> u64 {
        let mut h = DefaultHasher::new();
        for v in [self.t.a, self.t.b, self.t.c, self.t.d, self.t.tx, self.t.ty, self.alpha] {
            v.to_bits().hash(&mut h);
        }
        self.clips.len().hash(&mut h);
        for c in &self.clips {
            for v in [c.r.x, c.r.y, c.r.w, c.r.h, c.radius] {
                v.to_bits().hash(&mut h);
            }
            for v in [c.t.a, c.t.b, c.t.c, c.t.d, c.t.tx, c.t.ty] {
                v.to_bits().hash(&mut h);
            }
        }
        h.finish()
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Fonts: a process-wide face registry (faces are static font data) and a per-thread parsed-font cache.
// ---------------------------------------------------------------------------------------------------------------------

static FACES: Mutex<Vec<(u64, &'static [u8])>> = Mutex::new(Vec::new());

thread_local! {
    static FONTS: RefCell<HashMap<u64, Font<'static>>> = RefCell::new(HashMap::new());
    /// Canvas state of each `Surface` being drawn on this thread, keyed by its pixel buffer's address, so a scratch
    /// or band surface never sees another surface's clips and transforms. An entry goes when its stacks are empty.
    static SURFACE_STATE: RefCell<HashMap<usize, CanvasState>> = RefCell::new(HashMap::new());
}

/// Registers font data and returns the face id a `TextRun` uses: the address of the data. Registering the same data
/// again returns the same id.
pub fn register_face(data: &'static [u8]) -> u64 {
    let id = data.as_ptr() as usize as u64;
    let mut faces = FACES.lock().unwrap_or_else(|e| e.into_inner());
    if !faces.iter().any(|(k, _)| *k == id) {
        faces.push((id, data));
    }
    id
}

/// The data registered under a face id.
pub fn face_data(face: u64) -> Option<&'static [u8]> {
    let faces = FACES.lock().unwrap_or_else(|e| e.into_inner());
    faces.iter().find(|(k, _)| *k == face).map(|(_, d)| *d)
}

/// Runs `f` on the parsed font of a registered face (the GPU text path lays out runs with the same font the CPU path
/// draws with). `None` when the face is unknown or its data does not parse.
pub fn with_face<R>(face: u64, f: impl FnOnce(&Font<'static>) -> R) -> Option<R> {
    with_font(face, f)
}

/// Runs `f` on the parsed font of a registered face. `None` when the face is unknown or its data does not parse.
pub(crate) fn with_font<R>(face: u64, f: impl FnOnce(&Font<'static>) -> R) -> Option<R> {
    let data = face_data(face)?;
    FONTS.with(|c| {
        let mut m = c.borrow_mut();
        if !m.contains_key(&face) {
            let parsed = Font::parse(data)?;
            m.insert(face, parsed);
        }
        let font = m.get(&face)?;
        Some(f(font))
    })
}

/// The rendered image of a text run in `color` (premultiplied), with its top-left at the run's origin.
pub(crate) fn render_text(run: &TextRun, color: u32) -> Option<Image> {
    if run.text.is_empty() {
        return None;
    }
    let st = Style { px: run.px, embolden: 0.0, color, shadow: false };
    with_font(run.face, |f| f.render_var(&run.text, &st, &run.vars))
}

/// The advance width and the ascent-plus-descent height of a run, in px, as `render_text` lays it out.
pub(crate) fn text_extent(run: &TextRun) -> Option<(f32, f32)> {
    with_font(run.face, |f| {
        let lm = f.line_metrics(run.px);
        (f.measure_var(&run.text, run.px, &run.vars), lm.ascent.abs() + lm.descent.abs())
    })
}

/// The identity of a surface's canvas state: the address of its first pixel.
fn surface_key(s: &Surface<'_>) -> usize {
    s.px.as_ptr() as usize
}

/// The canvas state of `s` on this thread.
pub(crate) fn surface_state(s: &Surface<'_>) -> CanvasState {
    SURFACE_STATE.with(|c| c.borrow().get(&surface_key(s)).cloned().unwrap_or_default())
}

/// Changes the canvas state of `s`; the entry is dropped once every stack is empty again.
fn edit_surface_state(s: &Surface<'_>, f: impl FnOnce(&mut CanvasState)) {
    let key = surface_key(s);
    SURFACE_STATE.with(|c| {
        let mut map = c.borrow_mut();
        let st = map.entry(key).or_default();
        f(st);
        if st.is_empty() {
            map.remove(&key);
        }
    });
}

/// Resets every `Surface` canvas state on this thread (for tests and for callers that abandon a half-finished frame).
pub fn reset_surface_state() {
    SURFACE_STATE.with(|c| c.borrow_mut().clear());
}

// ---------------------------------------------------------------------------------------------------------------------
// Pixel helpers.
// ---------------------------------------------------------------------------------------------------------------------

/// Channel-wise linear interpolation of two premultiplied colours.
fn mix_px(a: u32, b: u32, k: f32) -> u32 {
    let mut out = 0u32;
    for ch in 0..4 {
        let sh = ch * 8;
        let x = ((a >> sh) & 255) as f32;
        let y = ((b >> sh) & 255) as f32;
        let v = (x + (y - x) * k).round().clamp(0.0, 255.0) as u32;
        out |= v << sh;
    }
    out
}

/// The colour of a gradient at offset `t`. `stops` must not be empty.
fn ramp(stops: &[Stop], t: f32) -> u32 {
    let first = stops[0];
    let last = stops[stops.len() - 1];
    if t <= first.at {
        return first.color;
    }
    if t >= last.at {
        return last.color;
    }
    for w in stops.windows(2) {
        let (a, b) = (w[0], w[1]);
        if t <= b.at {
            let span = b.at - a.at;
            return if span <= 0.0 { b.color } else { mix_px(a.color, b.color, (t - a.at) / span) };
        }
    }
    last.color
}

/// Composites one pixel. `cov` is 0..1. Source-over normally; with `lerp` the source replaces the destination by
/// `cov` (a material's result already contains the backdrop).
fn blend(dst: &mut u32, src: u32, cov: f32, lerp: bool) {
    let c = ((cov.clamp(0.0, 1.0) * 255.0) + 0.5) as u32;
    let c = c.min(255);
    if c == 0 {
        return;
    }
    if lerp {
        let mut out = 0u32;
        for ch in 0..4 {
            let sh = ch * 8;
            let d = (*dst >> sh) & 255;
            let s = (src >> sh) & 255;
            let v = ((d * (255 - c) + s * c + 127) / 255).min(255);
            out |= v << sh;
        }
        *dst = out;
    } else {
        let s = if c == 255 { src } else { scale(src, c) };
        *dst = over(*dst, s);
    }
}

/// Adds the horizontal coverage of the span `[a, b)` to a row of coverage cells starting at column `base`.
fn add_span(cov: &mut [f32], base: f32, a: f32, b: f32, weight: f32) {
    let a = a.max(base);
    let b = b.min(base + cov.len() as f32);
    if !(b > a) {
        return;
    }
    let first = a.floor() as usize;
    let end = b.ceil() as usize;
    for px in first..end {
        let lo = a.max(px as f32);
        let hi = b.min(px as f32 + 1.0);
        if hi > lo {
            cov[px - base as usize] += (hi - lo) * weight;
        }
    }
}

/// The surface pixels of a device rectangle, clipped to the surface: (x, y, w, h).
fn pixel_rect(s: &Surface, r: RectF, o: (f32, f32)) -> Option<(usize, usize, usize, usize)> {
    let x0 = (r.x + o.0).round().max(0.0);
    let y0 = (r.y + o.1).round().max(s.y0 as f32);
    let x1 = (r.x + r.w + o.0).round().min(s.w as f32);
    let y1 = (r.y + r.h + o.1).round().min(s.h as f32);
    if !(x0 < x1 && y0 < y1) {
        return None;
    }
    Some((x0 as usize, y0 as usize, (x1 - x0) as usize, (y1 - y0) as usize))
}

/// Fills the pixels of a rounded rectangle whose colour is given per local point (anti-aliased).
fn shape_fill(s: &mut Surface, r: RectF, radius: f32, o: (f32, f32), color_at: &dyn Fn(f32, f32) -> u32) {
    let rad = radius.max(0.0).min(r.w / 2.0).min(r.h / 2.0);
    let x0 = (r.x + o.0).floor().max(0.0);
    let x1 = (r.x + r.w + o.0).ceil().min(s.w as f32);
    let y0 = (r.y + o.1).floor().max(s.y0 as f32);
    let y1 = (r.y + r.h + o.1).ceil().min(s.h as f32);
    if !(x0 < x1 && y0 < y1) {
        return;
    }
    for py in y0 as usize..y1 as usize {
        for px in x0 as usize..x1 as usize {
            let lx = px as f32 + 0.5 - o.0;
            let ly = py as f32 + 0.5 - o.1;
            let c = rrect_coverage(lx - r.x, ly - r.y, r.w, r.h, rad);
            if c <= 0.0 {
                continue;
            }
            let col = color_at(lx, ly);
            blend(s.at(px, py), col, c, false);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Paths.
// ---------------------------------------------------------------------------------------------------------------------

/// Flattens a path into polylines: (points, closed). Curves become 16 (quadratic) or 24 (cubic) segments.
pub(crate) fn flatten(path: &PathData) -> Vec<(Vec<(f32, f32)>, bool)> {
    let mut out = Vec::new();
    let mut cur: Vec<(f32, f32)> = Vec::new();
    let mut last = (0.0f32, 0.0f32);
    let mut start = (0.0f32, 0.0f32);
    for v in &path.verbs {
        match *v {
            PathVerb::Move(x, y) => {
                if !cur.is_empty() {
                    out.push((std::mem::take(&mut cur), false));
                }
                cur.push((x, y));
                last = (x, y);
                start = (x, y);
            }
            PathVerb::Line(x, y) => {
                if cur.is_empty() {
                    cur.push(last);
                }
                cur.push((x, y));
                last = (x, y);
            }
            PathVerb::Quad(cx, cy, x, y) => {
                if cur.is_empty() {
                    cur.push(last);
                }
                for i in 1..=16 {
                    let t = i as f32 / 16.0;
                    let u = 1.0 - t;
                    cur.push((u * u * last.0 + 2.0 * u * t * cx + t * t * x, u * u * last.1 + 2.0 * u * t * cy + t * t * y));
                }
                last = (x, y);
            }
            PathVerb::Cubic(c1x, c1y, c2x, c2y, x, y) => {
                if cur.is_empty() {
                    cur.push(last);
                }
                for i in 1..=24 {
                    let t = i as f32 / 24.0;
                    let u = 1.0 - t;
                    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    cur.push((a * last.0 + b * c1x + c * c2x + d * x, a * last.1 + b * c1y + c * c2y + d * y));
                }
                last = (x, y);
            }
            PathVerb::Close => {
                if !cur.is_empty() {
                    out.push((std::mem::take(&mut cur), true));
                }
                last = start;
            }
        }
    }
    if !cur.is_empty() {
        out.push((cur, false));
    }
    out
}

/// Fills polylines in surface pixels with 4 sub-scanlines and exact horizontal coverage.
fn fill_polys(s: &mut Surface, polys: &[(Vec<(f32, f32)>, bool)], even_odd: bool, color: u32) {
    let mut edges: Vec<((f32, f32), (f32, f32))> = Vec::new();
    let (mut miny, mut maxy, mut minx, mut maxx) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for (pts, _) in polys {
        if pts.len() < 2 {
            continue;
        }
        for k in 0..pts.len() {
            let p = pts[k];
            let q = pts[(k + 1) % pts.len()];
            edges.push((p, q));
            miny = miny.min(p.1);
            maxy = maxy.max(p.1);
            minx = minx.min(p.0);
            maxx = maxx.max(p.0);
        }
    }
    if edges.is_empty() {
        return;
    }
    let y0 = miny.floor().max(s.y0 as f32);
    let y1 = maxy.ceil().min(s.h as f32);
    let x0 = minx.floor().max(0.0);
    let x1 = maxx.ceil().min(s.w as f32);
    if !(y0 < y1 && x0 < x1) {
        return;
    }
    let xs = x0 as usize;
    let mut cov = vec![0f32; (x1 - x0) as usize];
    let mut cross: Vec<(f32, i32)> = Vec::new();
    for py in y0 as usize..y1 as usize {
        cov.iter_mut().for_each(|c| *c = 0.0);
        for sub in 0..4 {
            let sy = py as f32 + (sub as f32 + 0.5) / 4.0;
            cross.clear();
            for &(p, q) in &edges {
                if p.1 == q.1 {
                    continue;
                }
                let (lo, hi) = if p.1 < q.1 { (p.1, q.1) } else { (q.1, p.1) };
                if sy >= lo && sy < hi {
                    let x = p.0 + (sy - p.1) * (q.0 - p.0) / (q.1 - p.1);
                    let dir = if q.1 > p.1 { 1 } else { -1 };
                    cross.push((x, dir));
                }
            }
            cross.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut wind = 0i32;
            for k in 0..cross.len() {
                wind += cross[k].1;
                let inside = if even_odd { wind % 2 != 0 } else { wind != 0 };
                if inside && k + 1 < cross.len() {
                    add_span(&mut cov, xs as f32, cross[k].0, cross[k + 1].0, 0.25);
                }
            }
        }
        for (i, &c) in cov.iter().enumerate() {
            if c > 0.0 {
                blend(s.at(xs + i, py), color, c.min(1.0), false);
            }
        }
    }
}

fn dist2_segment(px: f32, py: f32, p: (f32, f32), q: (f32, f32)) -> f32 {
    let (dx, dy) = (q.0 - p.0, q.1 - p.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((px - p.0) * dx + (py - p.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (cx, cy) = (p.0 + dx * t - px, p.1 + dy * t - py);
    cx * cx + cy * cy
}

/// Strokes polylines with half-width `hw`: a sample is inside when it lies within `hw` of any segment, which gives
/// round joins and caps. 4x4 samples per pixel.
fn stroke_polys(s: &mut Surface, polys: &[(Vec<(f32, f32)>, bool)], hw: f32, color: u32) {
    let mut segs: Vec<((f32, f32), (f32, f32))> = Vec::new();
    let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (pts, closed) in polys {
        for k in 0..pts.len().saturating_sub(1) {
            segs.push((pts[k], pts[k + 1]));
        }
        if *closed && pts.len() > 2 {
            segs.push((pts[pts.len() - 1], pts[0]));
        }
        for p in pts {
            minx = minx.min(p.0);
            miny = miny.min(p.1);
            maxx = maxx.max(p.0);
            maxy = maxy.max(p.1);
        }
    }
    if segs.is_empty() || !(hw > 0.0) {
        return;
    }
    let x0 = (minx - hw - 1.0).floor().max(0.0);
    let x1 = (maxx + hw + 1.0).ceil().min(s.w as f32);
    let y0 = (miny - hw - 1.0).floor().max(s.y0 as f32);
    let y1 = (maxy + hw + 1.0).ceil().min(s.h as f32);
    if !(x0 < x1 && y0 < y1) {
        return;
    }
    let h2 = hw * hw;
    for py in y0 as usize..y1 as usize {
        for px in x0 as usize..x1 as usize {
            let mut count = 0u32;
            for sj in 0..4 {
                for si in 0..4 {
                    let sx = px as f32 + (si as f32 + 0.5) / 4.0;
                    let sy = py as f32 + (sj as f32 + 0.5) / 4.0;
                    if segs.iter().any(|&(p, q)| dist2_segment(sx, sy, p, q) <= h2) {
                        count += 1;
                    }
                }
            }
            if count > 0 {
                blend(s.at(px, py), color, count as f32 / 16.0, false);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The CPU target: everything goes through `run`, which picks one of three routes.
// ---------------------------------------------------------------------------------------------------------------------

/// How local coordinates reach device pixels.
#[derive(Clone, Copy, Debug)]
enum Map {
    /// device = local + (tx, ty)
    Trans(f32, f32),
    /// device = transform(local); the inverse maps device pixel centres back to local space
    Affine(Transform2D, Transform2D),
}

fn map_of(t: Transform2D) -> Option<Map> {
    if t.is_translation() {
        Some(Map::Trans(t.tx, t.ty))
    } else {
        t.inverse().map(|inv| Map::Affine(t, inv))
    }
}

/// The device pixels a device rectangle covers, clipped to the surface and to the clip box: (x0, y0, x1, y1).
fn region(s: &Surface, st: &CanvasState, dev: RectF) -> Option<(usize, usize, usize, usize)> {
    let mut x0 = dev.x.floor().max(0.0);
    let mut y0 = dev.y.floor().max(s.y0 as f32);
    let mut x1 = (dev.x + dev.w).ceil().min(s.w as f32);
    let mut y1 = (dev.y + dev.h).ceil().min(s.h as f32);
    if let Some(cb) = st.clip_box() {
        x0 = x0.max(cb.x.floor());
        y0 = y0.max(cb.y.floor());
        x1 = x1.min((cb.x + cb.w).ceil());
        y1 = y1.min((cb.y + cb.h).ceil());
    }
    if !(x0 < x1 && y0 < y1) {
        return None;
    }
    Some((x0 as usize, y0 as usize, x1 as usize, y1 as usize))
}

/// Draws one operation. `draw` receives the surface to draw on and an offset `o` such that a local point `p` lands at
/// surface point `p + o`. Plain state (translation, no clip, full opacity) draws straight onto `s`; otherwise the
/// operation goes to a scratch surface and is composited with the clip and opacity. `backdrop` copies the pixels under
/// the scratch first and `lerp` replaces instead of blending (materials).
fn run(s: &mut Surface, st: &CanvasState, local: RectF, map: Map, lerp: bool, backdrop: bool, draw: &dyn Fn(&mut Surface<'_>, (f32, f32))) {
    if let Map::Trans(tx, ty) = map {
        if st.clips.is_empty() && st.alpha >= 1.0 {
            draw(s, (tx, ty));
            return;
        }
    }
    match map {
        Map::Trans(tx, ty) => {
            let dev = RectF::new(local.x + tx, local.y + ty, local.w, local.h);
            let Some((x0, y0, x1, y1)) = region(s, st, dev) else { return };
            let (sw, sh) = (x1 - x0, y1 - y0);
            let mut buf = vec![0u32; sw * sh];
            if backdrop {
                for y in y0..y1 {
                    for x in x0..x1 {
                        buf[(y - y0) * sw + (x - x0)] = *s.at(x, y);
                    }
                }
            }
            {
                let mut sc = Surface { w: sw, h: sh, stride: sw, px: &mut buf[..], y0: 0 };
                draw(&mut sc, (tx - x0 as f32, ty - y0 as f32));
            }
            for y in y0..y1 {
                for x in x0..x1 {
                    let cov = st.clip_coverage(x as f32 + 0.5, y as f32 + 0.5) * st.alpha;
                    if cov > 0.0 {
                        blend(s.at(x, y), buf[(y - y0) * sw + (x - x0)], cov, lerp);
                    }
                }
            }
        }
        Map::Affine(t, inv) => {
            let lx0 = local.x.floor();
            let ly0 = local.y.floor();
            let lw = (local.x + local.w).ceil() - lx0;
            let lh = (local.y + local.h).ceil() - ly0;
            if !(lw >= 1.0 && lh >= 1.0) {
                return;
            }
            let (lw, lh) = (lw as usize, lh as usize);
            let mut buf = vec![0u32; lw * lh];
            {
                let mut sc = Surface { w: lw, h: lh, stride: lw, px: &mut buf[..], y0: 0 };
                draw(&mut sc, (-lx0, -ly0));
            }
            let Some((x0, y0, x1, y1)) = region(s, st, bbox_of(t, local)) else { return };
            for y in y0..y1 {
                for x in x0..x1 {
                    let (lx, ly) = inv.apply(x as f32 + 0.5, y as f32 + 0.5);
                    let ix = (lx - lx0).floor();
                    let iy = (ly - ly0).floor();
                    if ix < 0.0 || iy < 0.0 || ix >= lw as f32 || iy >= lh as f32 {
                        continue;
                    }
                    let src = buf[iy as usize * lw + ix as usize];
                    if src == 0 {
                        continue;
                    }
                    let cov = st.clip_coverage(x as f32 + 0.5, y as f32 + 0.5) * st.alpha;
                    if cov > 0.0 {
                        blend(s.at(x, y), src, cov, lerp);
                    }
                }
            }
        }
    }
}

/// The current state, with the map for its transform. `None` when the transform is singular.
fn current(s: &Surface<'_>) -> Option<(CanvasState, Map)> {
    let st = surface_state(s);
    let map = map_of(st.transform())?;
    Some((st, map))
}

impl Canvas for Surface<'_> {
    fn size(&self) -> (u32, u32) {
        (self.w as u32, self.h as u32)
    }

    fn fill_rect(&mut self, r: RectF, color: u32) {
        let Some((st, map)) = current(self) else { return };
        run(self, &st, r, map, false, false, &|s, o| {
            if let Some((x, y, w, h)) = pixel_rect(s, r, o) {
                s.fill_rect(x, y, w, h, color);
            }
        });
    }

    fn fill_rrect(&mut self, r: RectF, radius: f32, color: u32) {
        let Some((st, map)) = current(self) else { return };
        run(self, &st, r, map, false, false, &|s, o| s.fill_rrect(r.x + o.0, r.y + o.1, r.w, r.h, radius, color));
    }

    fn stroke_rrect(&mut self, r: RectF, radius: f32, width: f32, color: u32) {
        let Some((st, map)) = current(self) else { return };
        run(self, &st, r, map, false, false, &|s, o| s.stroke_rrect(r.x + o.0, r.y + o.1, r.w, r.h, radius, width, color));
    }

    fn linear_gradient(&mut self, r: RectF, radius: f32, start: (f32, f32), end: (f32, f32), stops: &[Stop]) {
        if stops.is_empty() {
            return;
        }
        let Some((st, map)) = current(self) else { return };
        run(self, &st, r, map, false, false, &|s, o| {
            shape_fill(s, r, radius, o, &|lx, ly| ramp(stops, linear_t(start, end, lx, ly)));
        });
    }

    fn shadow(&mut self, r: RectF, radius: f32, blur: f32, offset: (f32, f32), color: u32) {
        if color >> 24 == 0 {
            return;
        }
        let Some((st, map)) = current(self) else { return };
        let sigma = blur.max(0.0) / 2.0;
        let rb = sigma.round() as usize;
        let pad = 3.0 * sigma.round() + 2.0 + offset.0.abs().max(offset.1.abs());
        let (sx, sy) = (r.x + offset.0, r.y + offset.1);
        let rad = radius.max(0.0).min(r.w / 2.0).min(r.h / 2.0);
        let local = RectF::new(sx - pad, sy - pad, r.w + 2.0 * pad, r.h + 2.0 * pad);
        run(self, &st, local, map, false, false, &|s, o| {
            let bx = local.x.floor();
            let by = local.y.floor();
            let bw = (local.x + local.w).ceil() - bx;
            let bh = (local.y + local.h).ceil() - by;
            if !(bw >= 1.0 && bh >= 1.0) {
                return;
            }
            let (bw, bh) = (bw as usize, bh as usize);
            let mut m = Image { w: bw, h: bh, px: vec![0; bw * bh] };
            for j in 0..bh {
                for i in 0..bw {
                    let lx = bx + i as f32 + 0.5;
                    let ly = by + j as f32 + 0.5;
                    let c = rrect_coverage(lx - sx, ly - sy, r.w, r.h, rad);
                    m.px[j * bw + i] = ((c * 255.0).round() as u32) << 24;
                }
            }
            m.blur(rb);
            let ox = (bx + o.0).round() as i64;
            let oy = (by + o.1).round() as i64;
            for j in 0..bh {
                let py = oy + j as i64;
                if py < s.y0 as i64 || py >= s.h as i64 {
                    continue;
                }
                for i in 0..bw {
                    let px = ox + i as i64;
                    if px < 0 || px >= s.w as i64 {
                        continue;
                    }
                    let a = m.px[j * bw + i] >> 24;
                    if a == 0 {
                        continue;
                    }
                    let d = s.at(px as usize, py as usize);
                    *d = over(*d, scale(color, a));
                }
            }
        });
    }

    fn image(&mut self, img: ImageRef<'_>, dst: RectF, alpha: f32) {
        draw_image(self, img, dst, alpha, None);
    }

    fn image_tinted(&mut self, img: ImageRef<'_>, dst: RectF, color: u32) {
        draw_image(self, img, dst, 1.0, Some(color));
    }

    fn text(&mut self, run_: &TextRun, color: u32) {
        let Some(img) = render_text(run_, crate::appearance::contrast(color)) else { return };
        let Some((st, map)) = current(self) else { return };
        let r = RectF::new(run_.x, run_.y, img.w as f32, img.h as f32);
        run(self, &st, r, map, false, false, &|s, o| {
            s.blit(&img, (run_.x + o.0).round() as i32, (run_.y + o.1).round() as i32, 255, 255);
        });
    }

    fn path(&mut self, path: &PathData, paint: PathPaint, color: u32) {
        let polys = flatten(path);
        let mut pts = polys.iter().flat_map(|(p, _)| p.iter());
        let Some(&first) = pts.next() else { return };
        let (mut x0, mut y0, mut x1, mut y1) = (first.0, first.1, first.0, first.1);
        for &(x, y) in pts {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        let hw = match paint {
            PathPaint::Stroke(w) => w.max(0.0) / 2.0,
            _ => 0.0,
        };
        let Some((st, map)) = current(self) else { return };
        let local = RectF::new(x0 - hw - 1.0, y0 - hw - 1.0, x1 - x0 + 2.0 * hw + 2.0, y1 - y0 + 2.0 * hw + 2.0);
        run(self, &st, local, map, false, false, &|s, o| {
            let shifted: Vec<(Vec<(f32, f32)>, bool)> =
                polys.iter().map(|(p, c)| (p.iter().map(|&(x, y)| (x + o.0, y + o.1)).collect(), *c)).collect();
            match paint {
                PathPaint::FillNonZero => fill_polys(s, &shifted, false, color),
                PathPaint::FillEvenOdd => fill_polys(s, &shifted, true, color),
                PathPaint::Stroke(w) => stroke_polys(s, &shifted, w.max(0.0) / 2.0, color),
            }
        });
    }

    fn material(&mut self, r: RectF, radius: f32, style: MaterialStyle) {
        let Some((st, map)) = current(self) else { return };
        let tint = style.tint;
        let blur = style.blur.max(0.0).round() as usize;
        // A material under a rotation or scale covers the device box of its rectangle.
        let (local, map) = match map {
            Map::Trans(..) => (r, map),
            Map::Affine(t, _) => (bbox_of(t, r), Map::Trans(0.0, 0.0)),
        };
        run(self, &st, local, map, true, true, &|s, o| {
            s.glass_rrect(local.x + o.0, local.y + o.1, local.w, local.h, radius, tint, tint, blur);
        });
    }

    fn push_clip(&mut self, r: RectF, radius: f32) {
        edit_surface_state(self, |st| st.push_clip(r, radius));
    }

    fn pop_clip(&mut self) {
        edit_surface_state(self, |st| st.pop_clip());
    }

    fn push_transform(&mut self, t: Transform2D) {
        edit_surface_state(self, |st| st.push_transform(t));
    }

    fn pop_transform(&mut self) {
        edit_surface_state(self, |st| st.pop_transform());
    }

    fn push_opacity(&mut self, alpha: f32) {
        edit_surface_state(self, |st| st.push_opacity(alpha));
    }

    fn pop_opacity(&mut self) {
        edit_surface_state(self, |st| st.pop_opacity());
    }
}

/// Shared by `image` and `image_tinted`: the image is scaled to `dst` and blitted, or blitted in colour.
fn draw_image(s: &mut Surface, img: ImageRef<'_>, dst: RectF, alpha: f32, tint: Option<u32>) {
    let w = dst.w.round().max(0.0) as usize;
    let h = dst.h.round().max(0.0) as usize;
    if w == 0 || h == 0 || img.image.w == 0 || img.image.h == 0 {
        return;
    }
    let Some((st, map)) = current(s) else { return };
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u32;
    let scaled = if img.image.w == w && img.image.h == h { None } else { Some(img.image.resized(w, h)) };
    let src: &Image = scaled.as_ref().unwrap_or(img.image);
    run(s, &st, dst, map, false, false, &|s, o| {
        let x = (dst.x + o.0).round() as i32;
        let y = (dst.y + o.1).round() as i32;
        match tint {
            Some(c) => s.blit_colored(src, x, y, c, 255),
            None => s.blit(src, x, y, 255, a),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // packed as 0xAABBGGRR: the red channel is the low byte
    const WHITE: u32 = 0xffff_ffff;
    const RED: u32 = 0xff00_00ff;
    const BLUE: u32 = 0xffff_0000;

    fn surf(px: &mut Vec<u32>, w: usize, h: usize) -> Surface<'_> {
        Surface { w, h, stride: w, px: &mut px[..], y0: 0 }
    }

    fn fresh() {
        reset_surface_state();
    }

    #[test]
    fn transform_mul_apply_and_inverse() {
        // translate after scale: p -> scale first, then translate
        let t = Transform2D::translate(10.0, 5.0).mul(Transform2D::scale(2.0, 3.0));
        assert_eq!(t.apply(1.0, 1.0), (12.0, 8.0));
        let inv = t.inverse().unwrap();
        let (x, y) = inv.apply(12.0, 8.0);
        assert!((x - 1.0).abs() < 1e-5 && (y - 1.0).abs() < 1e-5);
        assert!(Transform2D::scale(0.0, 1.0).inverse().is_none());
        assert!(Transform2D::translate(3.0, 4.0).is_translation());
        assert!(!Transform2D::scale(2.0, 1.0).is_translation());
    }

    #[test]
    fn bbox_and_intersect() {
        // a 90 degree rotation maps (0,0,4,2) to (-2,0,2,4)
        let rot = Transform2D { a: 0.0, b: 1.0, c: -1.0, d: 0.0, tx: 0.0, ty: 0.0 };
        let b = bbox_of(rot, RectF::new(0.0, 0.0, 4.0, 2.0));
        assert!((b.x + 2.0).abs() < 1e-5 && b.y.abs() < 1e-5 && (b.w - 2.0).abs() < 1e-5 && (b.h - 4.0).abs() < 1e-5);
        let i = intersect(RectF::new(0.0, 0.0, 10.0, 10.0), RectF::new(5.0, 5.0, 10.0, 10.0));
        assert_eq!(i, RectF::new(5.0, 5.0, 5.0, 5.0));
        assert_eq!(intersect(RectF::new(0.0, 0.0, 1.0, 1.0), RectF::new(5.0, 5.0, 1.0, 1.0)).w, 0.0);
    }

    #[test]
    fn state_stacks_restore_transform_clip_and_opacity() {
        let mut st = CanvasState::default();
        st.push_transform(Transform2D::translate(5.0, 0.0));
        st.push_clip(RectF::new(0.0, 0.0, 10.0, 10.0), 0.0);
        assert_eq!(st.clip_box(), Some(RectF::new(5.0, 0.0, 10.0, 10.0)));
        assert_eq!(st.device_bounds(RectF::new(0.0, 0.0, 100.0, 100.0)), RectF::new(5.0, 0.0, 10.0, 10.0));
        st.pop_clip();
        assert_eq!(st.clip_box(), None);
        st.pop_transform();
        assert_eq!(st.transform(), Transform2D::IDENTITY);
        st.push_opacity(0.5);
        st.push_opacity(0.5);
        assert!((st.alpha() - 0.25).abs() < 1e-6);
        st.pop_opacity();
        assert!((st.alpha() - 0.5).abs() < 1e-6);
        st.pop_opacity();
        st.pop_opacity(); // unbalanced: no panic, stays at one
        assert!((st.alpha() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn clip_coverage_is_the_rounded_mask() {
        let mut st = CanvasState::default();
        st.push_clip(RectF::new(0.0, 0.0, 4.0, 4.0), 0.0);
        assert!((st.clip_coverage(2.5, 2.5) - 1.0).abs() < 1e-6);
        assert_eq!(st.clip_coverage(4.5, 2.5), 0.0);
        assert_eq!(st.clip_coverage(-1.5, 2.5), 0.0);
        // a corner pixel of a radius-2 clip is outside
        let mut st = CanvasState::default();
        st.push_clip(RectF::new(0.0, 0.0, 4.0, 4.0), 2.0);
        assert_eq!(st.clip_coverage(0.05, 0.05), 0.0);
        assert!((st.clip_coverage(2.0, 0.5) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn state_key_changes_with_transform_opacity_and_clip() {
        let a = CanvasState::default();
        let mut b = CanvasState::default();
        b.push_opacity(0.5);
        let mut c = CanvasState::default();
        c.push_transform(Transform2D::translate(1.0, 0.0));
        let mut d = CanvasState::default();
        d.push_clip(RectF::new(0.0, 0.0, 1.0, 1.0), 0.0);
        let keys = [a.key(), b.key(), c.key(), d.key()];
        for i in 0..keys.len() {
            for j in i + 1..keys.len() {
                assert_ne!(keys[i], keys[j]);
            }
        }
        assert_eq!(a.key(), CanvasState::default().key());
    }

    #[test]
    fn gradient_ramp_interpolates_premultiplied_channels() {
        let stops = [Stop { at: 0.0, color: 0x0000_0000 }, Stop { at: 1.0, color: WHITE }];
        assert_eq!(ramp(&stops, 0.5), 0x8080_8080);
        assert_eq!(ramp(&stops, -1.0), 0);
        assert_eq!(ramp(&stops, 2.0), WHITE);
        let hard = [Stop { at: 0.0, color: 0 }, Stop { at: 0.5, color: 0 }, Stop { at: 0.5, color: WHITE }];
        assert_eq!(ramp(&hard, 0.6), WHITE);
    }

    #[test]
    fn fill_rect_with_translation_moves_the_rectangle() {
        fresh();
        let mut px = vec![0u32; 10 * 10];
        {
            let mut s = surf(&mut px, 10, 10);
            s.push_transform(Transform2D::translate(1.0, 0.0));
            Canvas::fill_rect(&mut s, RectF::new(2.0, 2.0, 3.0, 3.0), BLUE);
            s.pop_transform();
        }
        assert_eq!(px[2 * 10 + 3], BLUE);
        assert_eq!(px[4 * 10 + 5], BLUE);
        assert_eq!(px[2 * 10 + 2], 0);
        assert_eq!(px[2 * 10 + 6], 0);
    }

    #[test]
    fn clip_masks_the_fill() {
        fresh();
        let mut px = vec![0u32; 10 * 10];
        {
            let mut s = surf(&mut px, 10, 10);
            s.push_clip(RectF::new(0.0, 0.0, 4.0, 4.0), 0.0);
            Canvas::fill_rect(&mut s, RectF::new(0.0, 0.0, 10.0, 10.0), WHITE);
            s.pop_clip();
        }
        assert_eq!(px[3 * 10 + 3], WHITE);
        assert_eq!(px[0], WHITE);
        assert_eq!(px[4], 0);
        assert_eq!(px[4 * 10], 0);
    }

    #[test]
    fn opacity_halves_the_alpha() {
        fresh();
        let mut px = vec![0u32; 4 * 4];
        {
            let mut s = surf(&mut px, 4, 4);
            s.push_opacity(0.5);
            Canvas::fill_rect(&mut s, RectF::new(0.0, 0.0, 4.0, 4.0), WHITE);
            s.pop_opacity();
        }
        let a = px[5] >> 24;
        assert!((127..=129).contains(&a), "alpha {a}");
    }

    #[test]
    fn affine_scale_samples_the_scratch() {
        fresh();
        let mut px = vec![0u32; 8 * 8];
        {
            let mut s = surf(&mut px, 8, 8);
            s.push_transform(Transform2D::scale(2.0, 2.0));
            Canvas::fill_rect(&mut s, RectF::new(0.0, 0.0, 2.0, 2.0), BLUE);
            s.pop_transform();
        }
        assert_eq!(px[0], BLUE);
        assert_eq!(px[3 * 8 + 3], BLUE);
        assert_eq!(px[3], BLUE);
        assert_eq!(px[4], 0);
        assert_eq!(px[4 * 8], 0);
    }

    #[test]
    fn path_fill_covers_the_triangle_only() {
        fresh();
        let mut px = vec![0u32; 8 * 8];
        let path = PathData {
            verbs: vec![PathVerb::Move(0.0, 0.0), PathVerb::Line(8.0, 0.0), PathVerb::Line(0.0, 8.0), PathVerb::Close],
        };
        {
            let mut s = surf(&mut px, 8, 8);
            Canvas::path(&mut s, &path, PathPaint::FillNonZero, WHITE);
        }
        assert_eq!(px[1 * 8 + 1], WHITE);
        assert_eq!(px[7 * 8 + 7], 0);
    }

    #[test]
    fn stroke_draws_a_line_of_its_width() {
        fresh();
        let mut px = vec![0u32; 10 * 10];
        let path = PathData { verbs: vec![PathVerb::Move(1.0, 5.0), PathVerb::Line(9.0, 5.0)] };
        {
            let mut s = surf(&mut px, 10, 10);
            Canvas::path(&mut s, &path, PathPaint::Stroke(2.0), WHITE);
        }
        assert_eq!(px[5 * 10 + 5] >> 24, 255);
        assert_eq!(px[1 * 10 + 5], 0);
    }

    #[test]
    fn shadow_spreads_past_the_edge_and_stays_inside_its_pad() {
        fresh();
        let mut px = vec![0u32; 40 * 40];
        {
            let mut s = surf(&mut px, 40, 40);
            Canvas::shadow(&mut s, RectF::new(15.0, 15.0, 10.0, 10.0), 0.0, 8.0, (0.0, 0.0), 0xff00_0000);
        }
        assert!(px[20 * 40 + 20] >> 24 > 64);
        assert!(px[20 * 40 + 13] >> 24 > 0);
        assert_eq!(px[1 * 40 + 1] >> 24, 0);
    }

    #[test]
    fn text_draws_only_with_a_registered_face() {
        fresh();
        let data: &'static [u8] = include_bytes!("../../assets/SFUI.ttf");
        let face = register_face(data);
        assert_eq!(register_face(data), face);
        assert!(face_data(face).is_some());
        let run_ = TextRun { face, text: "Hi".into(), px: 16.0, vars: vec![], x: 2.0, y: 2.0 };
        let mut px = vec![0u32; 60 * 30];
        {
            let mut s = surf(&mut px, 60, 30);
            Canvas::text(&mut s, &run_, WHITE);
        }
        assert!(px.iter().any(|&p| p >> 24 > 0));
        let missing = TextRun { face: 7, ..run_.clone() };
        let mut blank = vec![0u32; 60 * 30];
        {
            let mut s = surf(&mut blank, 60, 30);
            Canvas::text(&mut s, &missing, WHITE);
        }
        assert!(blank.iter().all(|&p| p == 0));
    }

    #[test]
    fn image_scales_into_the_destination() {
        fresh();
        let img = Image { w: 2, h: 2, px: vec![RED; 4] };
        let mut px = vec![0u32; 8 * 8];
        {
            let mut s = surf(&mut px, 8, 8);
            Canvas::image(&mut s, ImageRef { id: 1, image: &img }, RectF::new(0.0, 0.0, 4.0, 4.0), 1.0);
        }
        assert_eq!(px[3 * 8 + 3], RED);
        assert_eq!(px[4], 0);
    }

    /// Each surface has its own clip stack: a clip pushed on one surface must not cut drawing on another (the scratch
    /// surfaces the impl composites through, nested layers).
    #[test]
    fn clips_do_not_leak_between_surfaces() {
        reset_surface_state();
        let mut a_px = vec![0u32; 8 * 8];
        let mut b_px = vec![0u32; 8 * 8];
        let mut a = Surface { w: 8, h: 8, stride: 8, px: &mut a_px[..], y0: 0 };
        let mut b = Surface { w: 8, h: 8, stride: 8, px: &mut b_px[..], y0: 0 };
        Canvas::push_clip(&mut a, RectF::new(0.0, 0.0, 2.0, 2.0), 0.0);
        Canvas::fill_rect(&mut b, RectF::new(0.0, 0.0, 8.0, 8.0), 0xff00_00ff);
        Canvas::fill_rect(&mut a, RectF::new(0.0, 0.0, 8.0, 8.0), 0xff00_00ff);
        Canvas::pop_clip(&mut a);
        drop((a, b));
        assert_eq!(b_px.iter().filter(|&&p| p != 0).count(), 64, "b is not clipped by a's clip");
        assert_eq!(a_px.iter().filter(|&&p| p != 0).count(), 4, "a is clipped to its 2x2 clip");
        SURFACE_STATE.with(|c| assert!(c.borrow().is_empty(), "balanced pushes and pops leave no state behind"));
    }
}
