//! The drawing interface the page renderer talks to. A drawing context
//! implements [`PdfSink`] to receive resolved paint operations; the types
//! here are plain data so the sink needs no knowledge of the file format.

pub use super::page::{Matrix, Rect};
use std::rc::Rc;

/// RGB in 0..=1, already converted from the colour space the file used.
pub type Rgb = [f32; 3];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathSeg {
    Move(f64, f64),
    Line(f64, f64),
    Cubic(f64, f64, f64, f64, f64, f64),
    Close,
}

pub type PdfPath = Vec<PathSeg>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineJoin {
    Miter,
    Round,
    Bevel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub width: f64,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f64,
    pub dash: Vec<f64>,
    pub phase: f64,
}

impl Default for Stroke {
    fn default() -> Self {
        Stroke { width: 1.0, cap: LineCap::Butt, join: LineJoin::Miter, miter_limit: 10.0, dash: vec![], phase: 0.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Blend {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl Blend {
    pub fn from_name(n: &str) -> Blend {
        match n {
            "Multiply" => Blend::Multiply,
            "Screen" => Blend::Screen,
            "Overlay" => Blend::Overlay,
            "Darken" => Blend::Darken,
            "Lighten" => Blend::Lighten,
            "ColorDodge" => Blend::ColorDodge,
            "ColorBurn" => Blend::ColorBurn,
            "HardLight" => Blend::HardLight,
            "SoftLight" => Blend::SoftLight,
            "Difference" => Blend::Difference,
            "Exclusion" => Blend::Exclusion,
            "Hue" => Blend::Hue,
            "Saturation" => Blend::Saturation,
            "Color" => Blend::Color,
            "Luminosity" => Blend::Luminosity,
            _ => Blend::Normal,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Blend::Normal => "Normal",
            Blend::Multiply => "Multiply",
            Blend::Screen => "Screen",
            Blend::Overlay => "Overlay",
            Blend::Darken => "Darken",
            Blend::Lighten => "Lighten",
            Blend::ColorDodge => "ColorDodge",
            Blend::ColorBurn => "ColorBurn",
            Blend::HardLight => "HardLight",
            Blend::SoftLight => "SoftLight",
            Blend::Difference => "Difference",
            Blend::Exclusion => "Exclusion",
            Blend::Hue => "Hue",
            Blend::Saturation => "Saturation",
            Blend::Color => "Color",
            Blend::Luminosity => "Luminosity",
        }
    }
}

/// A decoded image: straight (not premultiplied) RGBA, 4 bytes a pixel, row 0 on top.
/// For a stencil mask only the alpha channel matters: it is the coverage the current fill colour is
/// painted with.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfImage {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
    pub stencil: bool,
}

impl PdfImage {
    pub fn new(w: usize, h: usize) -> PdfImage {
        PdfImage { w, h, rgba: vec![0; w * h * 4], stencil: false }
    }

    /// True when every pixel is opaque.
    pub fn is_opaque(&self) -> bool {
        self.rgba.chunks_exact(4).all(|p| p[3] == 255)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub x: f64,
    pub y: f64,
    pub color: Rgb,
}

/// A smooth colour fill. Coordinates are in shading space; the `ctm` handed to [`PdfSink::shade`] maps it to the device.
#[derive(Clone, Debug)]
pub enum ShadingKind {
    /// Colour along the line from (x0, y0) to (x1, y1); `ramp` holds colours at even steps of t from 0 to 1.
    Axial { x0: f64, y0: f64, x1: f64, y1: f64, extend: (bool, bool), ramp: Vec<Rgb> },
    /// Colour between two circles.
    Radial { x0: f64, y0: f64, r0: f64, x1: f64, y1: f64, r1: f64, extend: (bool, bool), ramp: Vec<Rgb> },
    /// A colour grid over the unit square mapped by `matrix` (a function-based shading, sampled).
    Grid { matrix: Matrix, w: usize, h: usize, colors: Vec<Rgb> },
    /// Gouraud triangles (mesh shadings and tessellated patches).
    Triangles(Vec<[Vertex; 3]>),
}

#[derive(Clone, Debug)]
pub struct Shading {
    pub kind: ShadingKind,
    /// painted outside the shading's own area when present
    pub background: Option<Rgb>,
    pub bbox: Option<Rect>,
}

/// Receives the resolved drawing of a page. Paths are in the space `ctm` maps to the device; the
/// clip is part of the saved state.
pub trait PdfSink {
    fn save(&mut self);
    fn restore(&mut self);
    /// Intersects the clip with `path`.
    fn clip(&mut self, path: &[PathSeg], ctm: &Matrix, rule: FillRule);
    fn fill(&mut self, path: &[PathSeg], ctm: &Matrix, rule: FillRule, color: Rgb, alpha: f32, blend: Blend);
    fn stroke(&mut self, path: &[PathSeg], ctm: &Matrix, stroke: &Stroke, color: Rgb, alpha: f32, blend: Blend);
    /// Draws `img` on the unit square `ctm` maps to the device (image row 0 at y = 1). A stencil image is
    /// painted with `color`.
    fn image(&mut self, img: &PdfImage, ctm: &Matrix, color: Rgb, alpha: f32, blend: Blend, interpolate: bool);
    /// Paints the shading over the current clip.
    fn shade(&mut self, shading: &Shading, ctm: &Matrix, alpha: f32, blend: Blend);
    /// Starts a transparency group composited with `alpha` and `blend` when it ends. The default draws straight through.
    fn begin_group(&mut self, _alpha: f32, _blend: Blend) {}
    fn end_group(&mut self) {}
}

/// One recorded call, for tests and debugging.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    Save,
    Restore,
    Clip(PdfPath, Matrix, FillRule),
    Fill(PdfPath, Matrix, FillRule, Rgb, f32),
    Stroke(PdfPath, Matrix, Stroke, Rgb, f32),
    Image(usize, usize, Matrix, bool),
    Shade(Matrix),
    BeginGroup(f32),
    EndGroup,
}

/// A sink that only records what it is asked to draw.
#[derive(Default)]
pub struct Recorder {
    pub calls: Vec<Call>,
}

impl PdfSink for Recorder {
    fn save(&mut self) {
        self.calls.push(Call::Save);
    }
    fn restore(&mut self) {
        self.calls.push(Call::Restore);
    }
    fn clip(&mut self, path: &[PathSeg], ctm: &Matrix, rule: FillRule) {
        self.calls.push(Call::Clip(path.to_vec(), *ctm, rule));
    }
    fn fill(&mut self, path: &[PathSeg], ctm: &Matrix, rule: FillRule, color: Rgb, alpha: f32, _blend: Blend) {
        self.calls.push(Call::Fill(path.to_vec(), *ctm, rule, color, alpha));
    }
    fn stroke(&mut self, path: &[PathSeg], ctm: &Matrix, stroke: &Stroke, color: Rgb, alpha: f32, _blend: Blend) {
        self.calls.push(Call::Stroke(path.to_vec(), *ctm, stroke.clone(), color, alpha));
    }
    fn image(&mut self, img: &PdfImage, ctm: &Matrix, _color: Rgb, _alpha: f32, _blend: Blend, _interpolate: bool) {
        self.calls.push(Call::Image(img.w, img.h, *ctm, img.stencil));
    }
    fn shade(&mut self, _shading: &Shading, ctm: &Matrix, _alpha: f32, _blend: Blend) {
        self.calls.push(Call::Shade(*ctm));
    }
    fn begin_group(&mut self, alpha: f32, _blend: Blend) {
        self.calls.push(Call::BeginGroup(alpha));
    }
    fn end_group(&mut self) {
        self.calls.push(Call::EndGroup);
    }
}

/// A shared, cheaply clonable image (the renderer caches decoded XObjects).
pub type SharedImage = Rc<PdfImage>;

/// The control points of an ellipse inscribed in the rectangle, as four cubics.
pub fn ellipse_path(x: f64, y: f64, w: f64, h: f64) -> PdfPath {
    const K: f64 = 0.552_284_749_8;
    let (cx, cy, rx, ry) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
    vec![
        PathSeg::Move(cx + rx, cy),
        PathSeg::Cubic(cx + rx, cy + K * ry, cx + K * rx, cy + ry, cx, cy + ry),
        PathSeg::Cubic(cx - K * rx, cy + ry, cx - rx, cy + K * ry, cx - rx, cy),
        PathSeg::Cubic(cx - rx, cy - K * ry, cx - K * rx, cy - ry, cx, cy - ry),
        PathSeg::Cubic(cx + K * rx, cy - ry, cx + rx, cy - K * ry, cx + rx, cy),
        PathSeg::Close,
    ]
}

pub fn rect_path(x: f64, y: f64, w: f64, h: f64) -> PdfPath {
    vec![PathSeg::Move(x, y), PathSeg::Line(x + w, y), PathSeg::Line(x + w, y + h), PathSeg::Line(x, y + h), PathSeg::Close]
}

/// Flattens a path to polylines (one `Vec` of points per subpath, with whether it was closed).
pub fn flatten(path: &[PathSeg], m: &Matrix, tol: f64) -> Vec<(Vec<(f64, f64)>, bool)> {
    use super::page::apply;
    let mut out: Vec<(Vec<(f64, f64)>, bool)> = vec![];
    let mut cur: Vec<(f64, f64)> = vec![];
    let mut last = (0.0, 0.0);
    let mut start = (0.0, 0.0);
    for seg in path {
        match *seg {
            PathSeg::Move(x, y) => {
                if !cur.is_empty() {
                    out.push((std::mem::take(&mut cur), false));
                }
                last = apply(m, x, y);
                start = last;
                cur.push(last);
            }
            PathSeg::Line(x, y) => {
                if cur.is_empty() {
                    cur.push(last);
                }
                last = apply(m, x, y);
                cur.push(last);
            }
            PathSeg::Cubic(x1, y1, x2, y2, x3, y3) => {
                if cur.is_empty() {
                    cur.push(last);
                }
                let (p1, p2, p3) = (apply(m, x1, y1), apply(m, x2, y2), apply(m, x3, y3));
                // segment count from the control polygon length
                let len = ((p1.0 - last.0).hypot(p1.1 - last.1) + (p2.0 - p1.0).hypot(p2.1 - p1.1) + (p3.0 - p2.0).hypot(p3.1 - p2.1)).max(0.0);
                let n = ((len / tol.max(0.05)).sqrt() * 1.5).ceil().clamp(2.0, 64.0) as usize;
                for i in 1..=n {
                    let t = i as f64 / n as f64;
                    let u = 1.0 - t;
                    let x = u * u * u * last.0 + 3.0 * u * u * t * p1.0 + 3.0 * u * t * t * p2.0 + t * t * t * p3.0;
                    let y = u * u * u * last.1 + 3.0 * u * u * t * p1.1 + 3.0 * u * t * t * p2.1 + t * t * t * p3.1;
                    cur.push((x, y));
                }
                last = p3;
            }
            PathSeg::Close => {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flatten_closes_and_transforms() {
        let p = rect_path(0.0, 0.0, 10.0, 5.0);
        let f = flatten(&p, &[2.0, 0.0, 0.0, 2.0, 1.0, 1.0], 0.25);
        assert_eq!(f.len(), 1);
        assert!(f[0].1);
        assert_eq!(f[0].0, vec![(1.0, 1.0), (21.0, 1.0), (21.0, 11.0), (1.0, 11.0)]);
    }

    #[test]
    fn ellipse_flattens_to_a_round_loop() {
        let f = flatten(&ellipse_path(0.0, 0.0, 10.0, 10.0), &[1.0, 0.0, 0.0, 1.0, 0.0, 0.0], 0.1);
        let pts = &f[0].0;
        assert!(pts.len() > 8);
        for (x, y) in pts {
            let d = ((x - 5.0).powi(2) + (y - 5.0).powi(2)).sqrt();
            assert!((d - 5.0).abs() < 0.1, "{d}");
        }
    }

    #[test]
    fn blend_names_round_trip() {
        for b in [Blend::Multiply, Blend::Screen, Blend::Luminosity, Blend::Normal] {
            assert_eq!(Blend::from_name(b.name()), b);
        }
    }
}
