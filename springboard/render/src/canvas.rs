//! `Canvas`: the one drawing vocabulary of BifrOSt (docs/graphics-architecture.md, "Canvas, DisplayList").
//!
//! `Surface` implements it on the CPU with its existing methods, so all current drawing keeps working;
//! `display_list::DisplayList` records it for the GPU executor (`bifrost-2d`). Coordinates are f32 device pixels with
//! the origin at the top left; colours are packed premultiplied RGBA8 (`gfx::pack`).
//!
//! Owner: wave G1 agent `canvas` (the `Surface` impl and its tests). The trait itself is the contract: change it only
//! with the lead.

use crate::gfx::Image;

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

/// A 2D affine transform `[a c tx; b d ty]`.
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
    /// Identifies the face (the address of its font data, stable for the process lifetime).
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
