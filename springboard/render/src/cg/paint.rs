//! Paint: what fills a shape. A [`Paint`] is a solid colour, a linear or radial gradient, a shading driven by a
//! [`Function`], or a tiled pattern, and it produces a premultiplied pixel (red in the low byte, as the software
//! rasteriser stores them) for any point, for a row of pixels, or under a coverage mask.
//!
//! Paint coordinates are user space: `shade(x, y)` takes a point in the space the paint was defined in, and the
//! fill functions map device pixels back through the inverse of the current transform. Gradients and shadings
//! interpolate in their own colour space and convert to sRGB once, into a 1024-entry table.

use crate::blend::{composite, Mode as BlendMode};
use crate::cg::color::{Color, ColorConversionInfo, ColorSpace, RenderingIntent};
use crate::cg::path::Affine;
use crate::cg::raster::Coverage;
use crate::gfx::{over, rgba, scale, Surface};
use std::sync::Arc;

const LUT: usize = 1024;

// ---------------------------------------------------------------------------------------------------------------
// colour conversion shared by ramps and shadings

#[derive(Clone, Debug, PartialEq)]
struct ToSrgb {
    /// `None` when the source space already is sRGB.
    conv: Option<ColorConversionInfo>,
}

impl ToSrgb {
    fn new(space: &ColorSpace) -> Option<ToSrgb> {
        if *space == ColorSpace::Srgb {
            return Some(ToSrgb { conv: None });
        }
        Some(ToSrgb { conv: Some(ColorConversionInfo::new(space, &ColorSpace::Srgb, RenderingIntent::Default)?) })
    }

    /// Straight 8-bit sRGB for `comps` in the source space and an alpha.
    fn rgba8(&self, comps: &[f32], alpha: f32) -> [u8; 4] {
        let rgb = match &self.conv {
            None => [comps[0], comps[1], comps[2]],
            Some(c) => convert3(c, comps),
        };
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        [q(rgb[0]), q(rgb[1]), q(rgb[2]), q(alpha)]
    }
}

fn convert3(c: &ColorConversionInfo, comps: &[f32]) -> [f32; 3] {
    match c.apply(comps) {
        Some(v) if v.len() >= 3 => [v[0], v[1], v[2]],
        _ => [0.0, 0.0, 0.0],
    }
}

fn premul(px: [u8; 4]) -> u32 {
    rgba(px[0], px[1], px[2], px[3])
}

// ---------------------------------------------------------------------------------------------------------------
// colour ramps

/// Colours at locations along 0..=1, interpolated component by component in a colour space (alpha straight).
#[derive(Clone, Debug, PartialEq)]
pub struct ColorRamp {
    space: ColorSpace,
    locs: Vec<f32>,
    /// Per stop: the space's components followed by alpha.
    comps: Vec<Vec<f32>>,
    out: ToSrgb,
    lut: Vec<[u8; 4]>,
}

impl ColorRamp {
    /// A ramp in `space` from `(location, colour)` stops; colours convert to `space` first. Locations clamp to 0..=1
    /// and sort (stops at the same location keep their order, which makes a hard edge). `None` for no stops or a
    /// space that cannot hold colours.
    pub fn new(space: &ColorSpace, stops: &[(f32, Color)]) -> Option<ColorRamp> {
        if stops.is_empty() || space.num_components() == 0 || matches!(space, ColorSpace::Pattern(_)) {
            return None;
        }
        let mut v: Vec<(f32, Vec<f32>)> = Vec::with_capacity(stops.len());
        for (at, c) in stops {
            let conv = c.converted(space, RenderingIntent::Default)?;
            let mut comps = conv.components().to_vec();
            comps.push(conv.alpha());
            v.push((norm_loc(*at), comps));
        }
        ColorRamp::finish(space.clone(), v)
    }

    /// A ramp in sRGB from straight 8-bit stops.
    pub fn from_rgba8(stops: &[(f32, [u8; 4])]) -> Option<ColorRamp> {
        if stops.is_empty() {
            return None;
        }
        let v = stops
            .iter()
            .map(|(at, c)| (norm_loc(*at), vec![c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, c[3] as f32 / 255.0]))
            .collect();
        ColorRamp::finish(ColorSpace::Srgb, v)
    }

    /// A ramp with the colours spread evenly from 0 to 1.
    pub fn evenly(space: &ColorSpace, colors: &[Color]) -> Option<ColorRamp> {
        let n = colors.len();
        let stops: Vec<(f32, Color)> =
            colors.iter().enumerate().map(|(i, c)| (if n > 1 { i as f32 / (n - 1) as f32 } else { 0.0 }, c.clone())).collect();
        ColorRamp::new(space, &stops)
    }

    fn finish(space: ColorSpace, mut v: Vec<(f32, Vec<f32>)>) -> Option<ColorRamp> {
        v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let out = ToSrgb::new(&space)?;
        let (locs, comps): (Vec<f32>, Vec<Vec<f32>>) = v.into_iter().unzip();
        let mut r = ColorRamp { space, locs, comps, out, lut: Vec::new() };
        r.lut = (0..LUT).map(|i| r.exact(i as f32 / (LUT - 1) as f32)).collect();
        Some(r)
    }

    pub fn space(&self) -> &ColorSpace {
        &self.space
    }

    /// The stop locations, sorted.
    pub fn locations(&self) -> &[f32] {
        &self.locs
    }

    /// The straight sRGB colour at `t` (clamped to 0..=1), from the table.
    pub fn color_at(&self, t: f32) -> [u8; 4] {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        self.lut[(t * (LUT - 1) as f32 + 0.5) as usize]
    }

    /// The straight sRGB colour at `t`, computed without the table. Between two stops the components interpolate
    /// linearly; outside the first and last stops the end colours hold.
    pub fn exact(&self, t: f32) -> [u8; 4] {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        let n = self.locs.len();
        let comps: Vec<f32> = if t <= self.locs[0] {
            self.comps[0].clone()
        } else if t >= self.locs[n - 1] {
            self.comps[n - 1].clone()
        } else {
            let mut found = self.comps[n - 1].clone();
            for i in 0..n - 1 {
                let (a, b) = (self.locs[i], self.locs[i + 1]);
                if t <= b {
                    let f = if b > a { (t - a) / (b - a) } else { 1.0 };
                    found = self.comps[i].iter().zip(&self.comps[i + 1]).map(|(x, y)| x + (y - x) * f).collect();
                    break;
                }
            }
            found
        };
        let k = comps.len() - 1;
        self.out.rgba8(&comps[..k], comps[k])
    }
}

fn norm_loc(at: f32) -> f32 {
    if at.is_finite() {
        at.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

// ---------------------------------------------------------------------------------------------------------------
// geometry

/// The ramp position of `(x, y)` on the axis from `start` to `end`, or `None` where the gradient does not paint.
fn axial_t(start: (f32, f32), end: (f32, f32), x: f32, y: f32, ext_start: bool, ext_end: bool) -> Option<f32> {
    let (dx, dy) = (end.0 - start.0, end.1 - start.1);
    let len2 = dx * dx + dy * dy;
    if len2 <= 0.0 {
        return None;
    }
    let t = ((x - start.0) * dx + (y - start.1) * dy) / len2;
    if (t < 0.0 && !ext_start) || (t > 1.0 && !ext_end) {
        return None;
    }
    Some(t)
}

/// The ramp position of `(x, y)` for the circles `(c0, r0)` to `(c1, r1)`: the largest `t` whose circle (centre and
/// radius interpolated by `t`, radius not negative) passes through the point. `None` where nothing paints.
#[allow(clippy::too_many_arguments)]
fn radial_t(c0: (f32, f32), r0: f32, c1: (f32, f32), r1: f32, x: f32, y: f32, ext_start: bool, ext_end: bool) -> Option<f32> {
    let (dcx, dcy, dr) = (c1.0 - c0.0, c1.1 - c0.1, r1 - r0);
    let (px, py) = (x - c0.0, y - c0.1);
    let a = dcx * dcx + dcy * dcy - dr * dr;
    let b = px * dcx + py * dcy + r0 * dr;
    let c = px * px + py * py - r0 * r0;
    let ok = |t: f32| t.is_finite() && r0 + t * dr >= 0.0 && (t >= 0.0 || ext_start) && (t <= 1.0 || ext_end);
    if a.abs() < 1e-9 {
        if b.abs() < 1e-12 {
            return None;
        }
        let t = c / (2.0 * b);
        return if ok(t) { Some(t) } else { None };
    }
    let disc = b * b - a * c;
    if disc < 0.0 {
        return None;
    }
    let sq = disc.sqrt();
    let (t1, t2) = ((b + sq) / a, (b - sq) / a);
    let (hi, lo) = if t1 >= t2 { (t1, t2) } else { (t2, t1) };
    if ok(hi) {
        Some(hi)
    } else if ok(lo) {
        Some(lo)
    } else {
        None
    }
}

/// The position of `(x, y)` as the clockwise angle from `start_angle` (radians, from the positive x axis towards
/// positive y) around `center`, as a fraction of a turn in 0..1.
pub fn conic_t(center: (f32, f32), start_angle: f32, x: f32, y: f32) -> f32 {
    let angle = (y - center.1).atan2(x - center.0) - start_angle;
    angle.rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU
}

// ---------------------------------------------------------------------------------------------------------------
// gradients

/// A gradient along the line from `start` to `end`.
#[derive(Clone, Debug, PartialEq)]
pub struct LinearGradient {
    pub start: (f32, f32),
    pub end: (f32, f32),
    pub ramp: ColorRamp,
    /// Whether the start colour continues before `start`, and the end colour after `end`. Otherwise those areas are
    /// left unpainted.
    pub extend_start: bool,
    pub extend_end: bool,
}

impl LinearGradient {
    /// A gradient that paints only between its end points.
    pub fn new(start: (f32, f32), end: (f32, f32), ramp: ColorRamp) -> Self {
        LinearGradient { start, end, ramp, extend_start: false, extend_end: false }
    }

    pub fn extended(mut self, start: bool, end: bool) -> Self {
        self.extend_start = start;
        self.extend_end = end;
        self
    }

    /// The straight sRGB colour at the point, or `None` where the gradient does not paint.
    pub fn shade_rgba(&self, x: f32, y: f32) -> Option<[u8; 4]> {
        let t = axial_t(self.start, self.end, x, y, self.extend_start, self.extend_end)?;
        Some(self.ramp.color_at(t))
    }
}

/// A gradient between two circles. The ramp runs from the start circle to the end circle; with a zero start radius
/// it is the usual centre-out radial gradient.
#[derive(Clone, Debug, PartialEq)]
pub struct RadialGradient {
    pub start_center: (f32, f32),
    pub start_radius: f32,
    pub end_center: (f32, f32),
    pub end_radius: f32,
    pub ramp: ColorRamp,
    pub extend_start: bool,
    pub extend_end: bool,
}

impl RadialGradient {
    pub fn new(start_center: (f32, f32), start_radius: f32, end_center: (f32, f32), end_radius: f32, ramp: ColorRamp) -> Self {
        RadialGradient { start_center, start_radius, end_center, end_radius, ramp, extend_start: false, extend_end: false }
    }

    pub fn extended(mut self, start: bool, end: bool) -> Self {
        self.extend_start = start;
        self.extend_end = end;
        self
    }

    pub fn shade_rgba(&self, x: f32, y: f32) -> Option<[u8; 4]> {
        let t = radial_t(
            self.start_center,
            self.start_radius,
            self.end_center,
            self.end_radius,
            x,
            y,
            self.extend_start,
            self.extend_end,
        )?;
        Some(self.ramp.color_at(t))
    }
}

/// A gradient that sweeps around a centre. Not part of [`Paint`]; layers that sweep colour call it directly.
#[derive(Clone, Debug, PartialEq)]
pub struct ConicGradient {
    pub center: (f32, f32),
    pub start_angle: f32,
    pub ramp: ColorRamp,
}

impl ConicGradient {
    pub fn shade_rgba(&self, x: f32, y: f32) -> [u8; 4] {
        self.ramp.color_at(conic_t(self.center, self.start_angle, x, y))
    }
}

// ---------------------------------------------------------------------------------------------------------------
// functions

/// A one-input function of colour components, as a shading evaluates it.
#[derive(Clone, Debug, PartialEq)]
pub enum Function {
    /// `outputs` values per sample, evenly spaced across `domain`, linearly interpolated.
    Sampled { domain: (f32, f32), outputs: usize, samples: Vec<f32> },
    /// `c0 + x^n * (c1 - c0)` per output.
    Exponential { domain: (f32, f32), c0: Vec<f32>, c1: Vec<f32>, n: f32 },
    /// Several functions over consecutive subdomains, each fed its input remapped by `encode`.
    Stitching { domain: (f32, f32), functions: Vec<Function>, bounds: Vec<f32>, encode: Vec<(f32, f32)> },
}

fn valid_domain(d: (f32, f32)) -> bool {
    d.0.is_finite() && d.1.is_finite() && d.1 > d.0
}

impl Function {
    /// A sampled function. `None` for an invalid domain, no outputs, or fewer than two samples per output.
    pub fn sampled(domain: (f32, f32), outputs: usize, samples: Vec<f32>) -> Option<Function> {
        if !valid_domain(domain) || outputs == 0 || samples.len() < 2 * outputs || !samples.len().is_multiple_of(outputs) {
            return None;
        }
        Some(Function::Sampled { domain, outputs, samples })
    }

    /// An exponential function. `None` for an invalid domain, empty or unequal end values, or a bad exponent.
    pub fn exponential(domain: (f32, f32), c0: Vec<f32>, c1: Vec<f32>, n: f32) -> Option<Function> {
        if !valid_domain(domain) || c0.is_empty() || c0.len() != c1.len() || !n.is_finite() || n < 0.0 {
            return None;
        }
        Some(Function::Exponential { domain, c0, c1, n })
    }

    /// A stitching function: `bounds` has one fewer entry than `functions`, increasing and inside `domain`;
    /// `encode` has one pair per function; all functions have the same number of outputs.
    pub fn stitching(domain: (f32, f32), functions: Vec<Function>, bounds: Vec<f32>, encode: Vec<(f32, f32)>) -> Option<Function> {
        if !valid_domain(domain) || functions.is_empty() || bounds.len() + 1 != functions.len() || encode.len() != functions.len() {
            return None;
        }
        let mut prev = domain.0;
        for &b in &bounds {
            if !b.is_finite() || b < prev || b > domain.1 {
                return None;
            }
            prev = b;
        }
        let k = functions[0].outputs();
        if functions.iter().any(|f| f.outputs() != k) {
            return None;
        }
        Some(Function::Stitching { domain, functions, bounds, encode })
    }

    pub fn domain(&self) -> (f32, f32) {
        match self {
            Function::Sampled { domain, .. } | Function::Exponential { domain, .. } | Function::Stitching { domain, .. } => *domain,
        }
    }

    /// The number of values `eval` returns.
    pub fn outputs(&self) -> usize {
        match self {
            Function::Sampled { outputs, .. } => *outputs,
            Function::Exponential { c0, .. } => c0.len(),
            Function::Stitching { functions, .. } => functions[0].outputs(),
        }
    }

    /// The function at `x`. The input clamps to the domain; NaN is the start of the domain.
    pub fn eval(&self, x: f32) -> Vec<f32> {
        let (d0, d1) = self.domain();
        let x = if x.is_nan() { d0 } else { x.clamp(d0, d1) };
        match self {
            Function::Sampled { outputs, samples, .. } => {
                let n = samples.len() / outputs;
                let p = (x - d0) / (d1 - d0) * (n - 1) as f32;
                let i = (p.floor() as usize).min(n - 2);
                let f = p - i as f32;
                (0..*outputs).map(|o| samples[i * outputs + o] * (1.0 - f) + samples[(i + 1) * outputs + o] * f).collect()
            }
            Function::Exponential { c0, c1, n, .. } => {
                let xn = if *n == 1.0 { x } else { x.powf(*n) };
                let xn = if xn.is_finite() { xn } else { 0.0 };
                c0.iter().zip(c1).map(|(a, b)| a + xn * (b - a)).collect()
            }
            Function::Stitching { functions, bounds, encode, .. } => {
                let i = bounds.iter().position(|&b| x < b).unwrap_or(functions.len() - 1);
                let lo = if i == 0 { d0 } else { bounds[i - 1] };
                let hi = if i == bounds.len() { d1 } else { bounds[i] };
                let (e0, e1) = encode[i];
                let xe = if hi > lo { e0 + (x - lo) / (hi - lo) * (e1 - e0) } else { e0 };
                functions[i].eval(xe)
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// shadings

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShadingKind {
    Axial { start: (f32, f32), end: (f32, f32) },
    Radial { start_center: (f32, f32), start_radius: f32, end_center: (f32, f32), end_radius: f32 },
}

/// A gradient whose colour at each position comes from a [`Function`] of the position along the axis or between the
/// circles, mapped onto the function's domain.
#[derive(Clone, Debug, PartialEq)]
pub struct Shading {
    kind: ShadingKind,
    function: Function,
    space: ColorSpace,
    pub extend_start: bool,
    pub extend_end: bool,
    lut: Vec<[u8; 4]>,
}

impl Shading {
    /// A shading in `space`. The function must return the space's components, optionally followed by an alpha.
    pub fn new(kind: ShadingKind, space: &ColorSpace, function: Function, extend_start: bool, extend_end: bool) -> Option<Shading> {
        let n = space.num_components();
        let k = function.outputs();
        if n == 0 || (k != n && k != n + 1) {
            return None;
        }
        let out = ToSrgb::new(space)?;
        let (d0, d1) = function.domain();
        let lut = (0..LUT)
            .map(|i| {
                let v = function.eval(d0 + (d1 - d0) * i as f32 / (LUT - 1) as f32);
                out.rgba8(&v[..n], if k > n { v[n] } else { 1.0 })
            })
            .collect();
        Some(Shading { kind, function, space: space.clone(), extend_start, extend_end, lut })
    }

    pub fn axial(space: &ColorSpace, start: (f32, f32), end: (f32, f32), function: Function, extend_start: bool, extend_end: bool) -> Option<Shading> {
        Shading::new(ShadingKind::Axial { start, end }, space, function, extend_start, extend_end)
    }

    #[allow(clippy::too_many_arguments)] // two circles plus extend flags mirror the PDF radial shading dictionary
    pub fn radial(
        space: &ColorSpace,
        start_center: (f32, f32),
        start_radius: f32,
        end_center: (f32, f32),
        end_radius: f32,
        function: Function,
        extend_start: bool,
        extend_end: bool,
    ) -> Option<Shading> {
        let kind = ShadingKind::Radial { start_center, start_radius, end_center, end_radius };
        Shading::new(kind, space, function, extend_start, extend_end)
    }

    pub fn kind(&self) -> ShadingKind {
        self.kind
    }

    pub fn function(&self) -> &Function {
        &self.function
    }

    pub fn space(&self) -> &ColorSpace {
        &self.space
    }

    pub fn shade_rgba(&self, x: f32, y: f32) -> Option<[u8; 4]> {
        let t = match self.kind {
            ShadingKind::Axial { start, end } => axial_t(start, end, x, y, self.extend_start, self.extend_end)?,
            ShadingKind::Radial { start_center, start_radius, end_center, end_radius } => {
                radial_t(start_center, start_radius, end_center, end_radius, x, y, self.extend_start, self.extend_end)?
            }
        };
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        Some(self.lut[(t * (LUT - 1) as f32 + 0.5) as usize])
    }
}

// ---------------------------------------------------------------------------------------------------------------
// patterns

/// How a pattern's step is fitted to whole device pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tiling {
    /// Use the step as given; tiles land on fractional pixels.
    #[default]
    NoDistortion,
    /// Round the step to whole pixels by changing the spacing only.
    ConstantSpacing,
    /// Round the step to whole pixels, stretching the tile by the same factor so the spacing stays proportional.
    ConstantSpacingMinimalDistortion,
}

/// Whether the cell carries its own colours or only a shape painted in one colour chosen at fill time.
#[derive(Clone, Debug, PartialEq)]
pub enum PatternPaint {
    Colored,
    /// The cell's alpha is the shape; the colour comes from here.
    Uncolored(Color),
}

/// A tiled pattern: a cell of premultiplied pixels repeated every `step` in pattern space. `matrix` (the 2x3
/// `[a, b, c, d, tx, ty]` layout, `(x, y) -> (a*x + c*y + tx, b*x + d*y + ty)`) maps pattern space to user space.
/// Between cells, where the step exceeds the cell, the pattern is transparent.
#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    w: usize,
    h: usize,
    px: Arc<Vec<u32>>,
    bounds: (f32, f32),
    step: (f32, f32),
    matrix: [f32; 6],
    inv: [f32; 6],
    tiling: Tiling,
}

/// The inverse of a `[a, b, c, d, tx, ty]` matrix, or `None` when it is singular.
pub fn invert_matrix(m: &[f32; 6]) -> Option<[f32; 6]> {
    let det = m[0] * m[3] - m[1] * m[2];
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let (ia, ib, ic, id) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    Some([ia, ib, ic, id, -(ia * m[4] + ic * m[5]), -(ib * m[4] + id * m[5])])
}

impl Pattern {
    /// A pattern from a ready cell of `w` by `h` premultiplied pixels covering `bounds` in pattern space. `None` for
    /// an empty cell, a pixel count that does not match, non-positive bounds or step, or a singular matrix.
    pub fn from_tile(
        w: usize,
        h: usize,
        px: Vec<u32>,
        bounds: (f32, f32),
        step: (f32, f32),
        matrix: [f32; 6],
        tiling: Tiling,
    ) -> Option<Pattern> {
        let pos = |v: f32| v.is_finite() && v > 0.0;
        if w == 0 || h == 0 || px.len() != w * h || !pos(bounds.0) || !pos(bounds.1) || !pos(step.0) || !pos(step.1) {
            return None;
        }
        let inv = invert_matrix(&matrix)?;
        Some(Pattern { w, h, px: Arc::new(px), bounds, step, matrix, inv, tiling })
    }

    /// A pattern whose cell is drawn by `draw`. The callback gets a transparent surface of
    /// `ceil(bounds * resolution)` pixels and the number of surface pixels per pattern unit, and paints the cell
    /// in surface coordinates. For an uncoloured pattern only the callback's alpha is kept, painted in the given colour.
    pub fn with_cell(
        bounds: (f32, f32),
        step: (f32, f32),
        matrix: [f32; 6],
        resolution: f32,
        paint: PatternPaint,
        tiling: Tiling,
        draw: impl FnOnce(&mut Surface, f32),
    ) -> Option<Pattern> {
        if !bounds.0.is_finite() || !bounds.1.is_finite() || !resolution.is_finite() || resolution <= 0.0 {
            return None;
        }
        let w = ((bounds.0 * resolution).ceil() as usize).clamp(1, 4096);
        let h = ((bounds.1 * resolution).ceil() as usize).clamp(1, 4096);
        let mut px = vec![0u32; w * h];
        {
            let mut surface = Surface { w, h, stride: w, px: &mut px, y0: 0 };
            draw(&mut surface, resolution);
        }
        if let PatternPaint::Uncolored(c) = &paint {
            let tint = c.to_premul();
            for p in px.iter_mut() {
                *p = scale(tint, *p >> 24);
            }
        }
        Pattern::from_tile(w, h, px, bounds, step, matrix, tiling)
    }

    pub fn tiling(&self) -> Tiling {
        self.tiling
    }

    pub fn step(&self) -> (f32, f32) {
        self.step
    }

    pub fn bounds(&self) -> (f32, f32) {
        self.bounds
    }

    /// The premultiplied pixel at the user-space point `(x, y)`; 0 between cells.
    pub fn shade(&self, x: f32, y: f32) -> u32 {
        let px = self.inv[0] * x + self.inv[2] * y + self.inv[4];
        let py = self.inv[1] * x + self.inv[3] * y + self.inv[5];
        let lx = px.rem_euclid(self.step.0);
        let ly = py.rem_euclid(self.step.1);
        if lx >= self.bounds.0 || ly >= self.bounds.1 {
            return 0;
        }
        let tx = ((lx / self.bounds.0 * self.w as f32) as usize).min(self.w - 1);
        let ty = ((ly / self.bounds.1 * self.h as f32) as usize).min(self.h - 1);
        self.px[ty * self.w + tx]
    }

    /// This pattern with its step fitted to whole device pixels under `ctm` (a `[a, b, c, d, tx, ty]` matrix from
    /// user space to device space), according to its [`Tiling`].
    pub fn fitted(&self, ctm: &[f32; 6]) -> Pattern {
        if self.tiling == Tiling::NoDistortion {
            return self.clone();
        }
        let m = &self.matrix;
        let axis = |px: f32, py: f32| {
            // a pattern-space direction, through the pattern matrix then the transform
            let (ux, uy) = (m[0] * px + m[2] * py, m[1] * px + m[3] * py);
            let (dx, dy) = (ctm[0] * ux + ctm[2] * uy, ctm[1] * ux + ctm[3] * uy);
            (dx * dx + dy * dy).sqrt()
        };
        let (sx, sy) = (axis(1.0, 0.0), axis(0.0, 1.0));
        let mut out = self.clone();
        let fit = |step: f32, s: f32| -> f32 {
            if s <= 0.0 || !s.is_finite() {
                return 1.0;
            }
            let dev = step * s;
            (dev.round().max(1.0)) / dev
        };
        let (kx, ky) = (fit(self.step.0, sx), fit(self.step.1, sy));
        out.step = (self.step.0 * kx, self.step.1 * ky);
        if self.tiling == Tiling::ConstantSpacingMinimalDistortion {
            out.bounds = (self.bounds.0 * kx, self.bounds.1 * ky);
        }
        out
    }
}

// ---------------------------------------------------------------------------------------------------------------
// paint

/// Whatever can fill a shape.
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    Solid(Color),
    Linear(LinearGradient),
    Radial(RadialGradient),
    Shading(Shading),
    Pattern(Pattern),
}

impl Paint {
    /// The premultiplied pixel at the user-space point `(x, y)`; 0 where the paint does not reach. A solid colour
    /// converts every call; use [`Paint::shade_row`] for bulk work.
    pub fn shade(&self, x: f32, y: f32) -> u32 {
        match self {
            Paint::Solid(c) => c.to_premul(),
            Paint::Linear(g) => g.shade_rgba(x, y).map_or(0, premul),
            Paint::Radial(g) => g.shade_rgba(x, y).map_or(0, premul),
            Paint::Shading(s) => s.shade_rgba(x, y).map_or(0, premul),
            Paint::Pattern(p) => p.shade(x, y),
        }
    }

    /// Fills `out` with the paint for the device pixels `x0..x0 + out.len()` of row `y`, sampling pixel centres.
    /// `inv` maps device space to user space (`[a, b, c, d, tx, ty]`).
    pub fn shade_row(&self, inv: &[f32; 6], y: i32, x0: i32, out: &mut [u32]) {
        if let Paint::Solid(c) = self {
            out.fill(c.to_premul());
            return;
        }
        let fy = y as f32 + 0.5;
        for (i, o) in out.iter_mut().enumerate() {
            let fx = (x0 + i as i32) as f32 + 0.5;
            *o = self.shade(inv[0] * fx + inv[2] * fy + inv[4], inv[1] * fx + inv[3] * fy + inv[5]);
        }
    }

    /// Paints the paint through the 8-bit mask `cov` onto `dst`, under the transform `ctm` (user space to device
    /// space), the blend mode and an overall alpha (0..=1).
    pub fn fill_coverage(&self, dst: &mut Surface, cov: &Coverage, ctm: &Affine, blend: BlendMode, alpha: f32) {
        let m = affine_parts(ctm);
        let Some(inv) = invert_matrix(&m) else { return };
        let (cx, cy, cw, ch, data) = coverage_parts(cov);
        if cw == 0 || ch == 0 || data.len() < cw * ch {
            return;
        }
        let fitted;
        let paint: &Paint = match self {
            Paint::Pattern(p) if p.tiling != Tiling::NoDistortion => {
                fitted = Paint::Pattern(p.fitted(&m));
                &fitted
            }
            _ => self,
        };
        let a255 = (alpha.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
        if a255 == 0 {
            return;
        }
        let x0 = cx.max(0);
        let x1 = (cx + cw as i32).min(dst.w as i32);
        if x1 <= x0 {
            return;
        }
        let mut row = vec![0u32; (x1 - x0) as usize];
        for j in 0..ch {
            let y = cy + j as i32;
            if y < 0 || y as usize >= dst.h || (y as usize) < dst.y0 {
                continue;
            }
            let cov_row = &data[j * cw + (x0 - cx) as usize..j * cw + (x1 - cx) as usize];
            if cov_row.iter().all(|&c| c == 0) {
                continue;
            }
            paint.shade_row(&inv, y, x0, &mut row);
            blend_row(dst, y as usize, x0 as usize, &row, Some(cov_row), a255, blend);
        }
    }

    /// Paints the paint over the device rectangle `x0..x1` by `y0..y1` with no mask, user space equal to device
    /// space.
    #[allow(clippy::too_many_arguments)] // destination, device rectangle, blend and alpha; a struct would add noise
    pub fn fill_rect(&self, dst: &mut Surface, x0: i32, y0: i32, x1: i32, y1: i32, blend: BlendMode, alpha: f32) {
        let a255 = (alpha.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
        let (x0, x1) = (x0.max(0), x1.min(dst.w as i32));
        if a255 == 0 || x1 <= x0 {
            return;
        }
        let ident = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        let mut row = vec![0u32; (x1 - x0) as usize];
        for y in y0.max(dst.y0 as i32)..y1.min(dst.h as i32) {
            self.shade_row(&ident, y, x0, &mut row);
            blend_row(dst, y as usize, x0 as usize, &row, None, a255, blend);
        }
    }
}

fn blend_row(dst: &mut Surface, y: usize, x0: usize, src: &[u32], cov: Option<&[u8]>, a255: u32, blend: BlendMode) {
    let base = (y - dst.y0) * dst.stride + x0;
    let Some(out) = dst.px.get_mut(base..base + src.len()) else { return };
    for (i, d) in out.iter_mut().enumerate() {
        let s = src[i];
        if s == 0 {
            continue;
        }
        let f = match cov {
            Some(c) => (c[i] as u32 * a255 + 127) / 255,
            None => a255,
        };
        if f == 0 {
            continue;
        }
        let s = if f == 255 { s } else { scale(s, f) };
        *d = if blend == BlendMode::Normal { over(*d, s) } else { composite(s, *d, blend) };
    }
}

// The two places where this module reads the raster and path types. Their layouts are the contract's: an affine is
// the 2x3 `a b c d tx ty` matrix and a coverage mask is an origin, a size and 8-bit alpha rows.

fn affine_parts(m: &Affine) -> [f32; 6] {
    [m.a, m.b, m.c, m.d, m.tx, m.ty]
}

fn coverage_parts(c: &Coverage) -> (i32, i32, usize, usize, &[u8]) {
    (c.x, c.y, c.w, c.h, &c.a)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENT: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

    fn bw() -> ColorRamp {
        ColorRamp::from_rgba8(&[(0.0, [0, 0, 0, 255]), (1.0, [255, 255, 255, 255])]).unwrap()
    }

    #[test]
    fn ramp_midpoint_and_ends() {
        let r = bw();
        assert_eq!(r.exact(0.5), [128, 128, 128, 255]);
        assert_eq!(r.exact(0.0), [0, 0, 0, 255]);
        assert_eq!(r.exact(1.0), [255, 255, 255, 255]);
        assert_eq!(r.exact(-3.0), [0, 0, 0, 255]);
        assert_eq!(r.exact(f32::NAN), [0, 0, 0, 255]);
        let t = r.color_at(0.5);
        assert!((t[0] as i32 - 128).abs() <= 1);
    }

    #[test]
    fn ramp_sorts_clamps_and_holds_the_ends() {
        let r = ColorRamp::from_rgba8(&[(1.5, [255, 255, 255, 255]), (0.2, [0, 0, 0, 255])]).unwrap();
        assert_eq!(r.locations(), &[0.2, 1.0]);
        assert_eq!(r.exact(0.1), [0, 0, 0, 255]);
        assert!(ColorRamp::from_rgba8(&[]).is_none());
    }

    #[test]
    fn ramp_three_stops_and_alpha() {
        let r = ColorRamp::from_rgba8(&[(0.0, [255, 0, 0, 255]), (0.5, [0, 255, 0, 255]), (1.0, [0, 0, 255, 255])]).unwrap();
        assert_eq!(r.exact(0.25), [128, 128, 0, 255]);
        assert_eq!(r.exact(0.5), [0, 255, 0, 255]);
        let a = ColorRamp::from_rgba8(&[(0.0, [0, 0, 0, 0]), (1.0, [0, 0, 0, 255])]).unwrap();
        assert_eq!(a.exact(0.5)[3], 128);
    }

    #[test]
    fn hard_stop_keeps_both_colours() {
        let r = ColorRamp::from_rgba8(&[(0.4, [255, 0, 0, 255]), (0.4, [0, 0, 255, 255]), (1.0, [0, 0, 255, 255])]).unwrap();
        assert_eq!(r.exact(0.35), [255, 0, 0, 255]);
        assert_eq!(r.exact(0.45), [0, 0, 255, 255]);
    }

    #[test]
    fn ramp_interpolates_in_its_own_space() {
        // Halfway between black and white in a linear space is brighter than halfway in sRGB.
        let stops = [(0.0, Color::new(ColorSpace::LinearSrgb, &[0.0, 0.0, 0.0], 1.0).unwrap()), (1.0, Color::new(ColorSpace::LinearSrgb, &[1.0, 1.0, 1.0], 1.0).unwrap())];
        let r = ColorRamp::new(&ColorSpace::LinearSrgb, &stops).unwrap();
        let mid = r.exact(0.5)[0];
        assert!((mid as i32 - 188).abs() <= 2, "{mid}");
        // Stops given in another space are converted first.
        let g = Color::generic_gray(1.0, 1.0);
        let r = ColorRamp::new(&ColorSpace::Srgb, &[(0.0, g.clone()), (1.0, g)]).unwrap();
        assert_eq!(r.exact(0.5), [255, 255, 255, 255]);
    }

    #[test]
    fn linear_projection_and_extend() {
        let g = LinearGradient::new((0.0, 0.0), (10.0, 0.0), bw());
        let mid = g.shade_rgba(5.0, 99.0).unwrap();
        assert!((mid[0] as i32 - 128).abs() <= 1, "off-axis distance does not matter");
        assert!(g.shade_rgba(-1.0, 0.0).is_none());
        assert!(g.shade_rgba(11.0, 0.0).is_none());
        let e = g.clone().extended(true, false);
        assert_eq!(e.shade_rgba(-5.0, 0.0), Some([0, 0, 0, 255]));
        assert!(e.shade_rgba(11.0, 0.0).is_none());
        let both = g.extended(true, true);
        assert_eq!(both.shade_rgba(50.0, 0.0), Some([255, 255, 255, 255]));
        let degenerate = LinearGradient::new((1.0, 1.0), (1.0, 1.0), bw()).extended(true, true);
        assert!(degenerate.shade_rgba(1.0, 1.0).is_none());
    }

    #[test]
    fn radial_concentric() {
        let g = RadialGradient::new((0.0, 0.0), 0.0, (0.0, 0.0), 10.0, bw());
        assert_eq!(g.shade_rgba(0.0, 0.0), Some([0, 0, 0, 255]));
        let mid = g.shade_rgba(3.0, 4.0).unwrap();
        assert!((mid[0] as i32 - 128).abs() <= 1);
        assert!(g.shade_rgba(30.0, 0.0).is_none(), "outside the end circle without extend");
        assert_eq!(g.extended(false, true).shade_rgba(30.0, 0.0), Some([255, 255, 255, 255]));
    }

    #[test]
    fn radial_with_start_radius_and_offset_circles() {
        // An annulus: radius 5 to 10, same centre. Inside the start circle paints only with extend.
        let g = RadialGradient::new((0.0, 0.0), 5.0, (0.0, 0.0), 10.0, bw());
        assert!(g.shade_rgba(1.0, 0.0).is_none());
        let inner = g.clone().extended(true, false).shade_rgba(1.0, 0.0).unwrap();
        assert_eq!(inner, [0, 0, 0, 255]);
        let mid = g.shade_rgba(7.5, 0.0).unwrap();
        assert!((mid[0] as i32 - 128).abs() <= 1);
        // A cone: a point at the start circle's centre side and one past the end are covered or not as expected.
        let cone = RadialGradient::new((0.0, 0.0), 1.0, (10.0, 0.0), 3.0, bw());
        assert!(cone.shade_rgba(10.0, 0.0).is_some());
        assert!(cone.shade_rgba(0.0, 50.0).is_none());
    }

    #[test]
    fn conic_position() {
        let t = conic_t((0.0, 0.0), 0.0, 0.0, 1.0);
        assert!((t - 0.25).abs() < 1e-6);
        let t = conic_t((0.0, 0.0), std::f32::consts::FRAC_PI_2, 1.0, 0.0);
        assert!((t - 0.75).abs() < 1e-6);
        let g = ConicGradient { center: (0.0, 0.0), start_angle: 0.0, ramp: bw() };
        assert_eq!(g.shade_rgba(1.0, 0.0), [0, 0, 0, 255]);
    }

    #[test]
    fn exponential_function() {
        let f = Function::exponential((0.0, 1.0), vec![0.0, 0.0], vec![1.0, 0.5], 1.0).unwrap();
        assert_eq!(f.outputs(), 2);
        assert_eq!(f.eval(0.5), vec![0.5, 0.25]);
        assert_eq!(f.eval(5.0), vec![1.0, 0.5]);
        let sq = Function::exponential((0.0, 1.0), vec![0.0], vec![1.0], 2.0).unwrap();
        assert!((sq.eval(0.5)[0] - 0.25).abs() < 1e-6);
        assert!(Function::exponential((0.0, 1.0), vec![0.0], vec![1.0, 2.0], 1.0).is_none());
        assert!(Function::exponential((1.0, 1.0), vec![0.0], vec![1.0], 1.0).is_none());
    }

    #[test]
    fn sampled_function() {
        let f = Function::sampled((0.0, 2.0), 3, vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]).unwrap();
        assert_eq!(f.eval(1.0), vec![0.0, 1.0, 0.0]);
        let q = f.eval(0.25);
        assert!((q[0] - 0.75).abs() < 1e-6 && (q[1] - 0.25).abs() < 1e-6);
        assert_eq!(f.eval(f32::NAN), vec![1.0, 0.0, 0.0]);
        assert!(Function::sampled((0.0, 1.0), 2, vec![1.0, 2.0, 3.0]).is_none());
        assert!(Function::sampled((0.0, 1.0), 1, vec![1.0]).is_none());
    }

    #[test]
    fn stitching_function() {
        let a = Function::exponential((0.0, 1.0), vec![0.0], vec![1.0], 1.0).unwrap();
        let b = Function::exponential((0.0, 1.0), vec![1.0], vec![0.0], 1.0).unwrap();
        // Up over 0..0.5, then back down over 0.5..1.
        let f = Function::stitching((0.0, 1.0), vec![a, b], vec![0.5], vec![(0.0, 1.0), (0.0, 1.0)]).unwrap();
        assert!((f.eval(0.25)[0] - 0.5).abs() < 1e-6);
        assert!((f.eval(0.5)[0] - 1.0).abs() < 1e-6);
        assert!((f.eval(0.75)[0] - 0.5).abs() < 1e-6);
        assert!((f.eval(1.0)[0]).abs() < 1e-6);
        // Reversed encode runs the second function backwards.
        let c = Function::exponential((0.0, 1.0), vec![0.0], vec![1.0], 1.0).unwrap();
        let g = Function::stitching((0.0, 1.0), vec![c.clone(), c], vec![0.5], vec![(0.0, 1.0), (1.0, 0.0)]).unwrap();
        assert!((g.eval(0.75)[0] - 0.5).abs() < 1e-6);
        assert!(Function::stitching((0.0, 1.0), vec![], vec![], vec![]).is_none());
    }

    #[test]
    fn axial_shading_through_a_function() {
        let f = Function::exponential((0.0, 1.0), vec![1.0, 0.0, 0.0], vec![0.0, 0.0, 1.0], 1.0).unwrap();
        let s = Shading::axial(&ColorSpace::Srgb, (0.0, 0.0), (10.0, 0.0), f.clone(), false, false).unwrap();
        assert_eq!(s.shade_rgba(0.0, 0.0), Some([255, 0, 0, 255]));
        assert_eq!(s.shade_rgba(10.0, 0.0), Some([0, 0, 255, 255]));
        assert!(s.shade_rgba(-1.0, 0.0).is_none());
        let mid = s.shade_rgba(5.0, 0.0).unwrap();
        assert!((mid[0] as i32 - 128).abs() <= 1 && (mid[2] as i32 - 128).abs() <= 1);
        // Wrong output count for the space.
        let two = Function::exponential((0.0, 1.0), vec![0.0, 0.0], vec![1.0, 1.0], 1.0).unwrap();
        assert!(Shading::axial(&ColorSpace::Srgb, (0.0, 0.0), (1.0, 0.0), two, true, true).is_none());
    }

    #[test]
    fn radial_shading_with_alpha_output() {
        let f = Function::exponential((0.0, 1.0), vec![0.0, 0.0, 0.0, 1.0], vec![0.0, 0.0, 0.0, 0.0], 1.0).unwrap();
        let s = Shading::radial(&ColorSpace::Srgb, (0.0, 0.0), 0.0, (0.0, 0.0), 10.0, f, false, false).unwrap();
        assert_eq!(s.shade_rgba(0.0, 0.0), Some([0, 0, 0, 255]));
        assert_eq!(s.shade_rgba(10.0, 0.0).unwrap()[3], 0);
    }

    fn cell_2x2() -> Pattern {
        // Cell pixels 1..4 (row-major), placed every 4 units, bounds 2x2, one pixel per unit.
        Pattern::from_tile(2, 2, vec![1, 2, 3, 4], (2.0, 2.0), (4.0, 4.0), IDENT, Tiling::NoDistortion).unwrap()
    }

    #[test]
    fn pattern_cells_repeat_with_gaps() {
        let p = cell_2x2();
        assert_eq!(p.shade(0.5, 0.5), 1);
        assert_eq!(p.shade(1.5, 0.5), 2);
        assert_eq!(p.shade(0.5, 1.5), 3);
        assert_eq!(p.shade(1.5, 1.5), 4);
        assert_eq!(p.shade(2.5, 0.5), 0, "gap between cells");
        assert_eq!(p.shade(4.5, 0.5), 1, "next cell");
        assert_eq!(p.shade(-3.5, 0.5), 1, "negative coordinates wrap");
        assert_eq!(p.shade(-0.5, 0.5), 0);
    }

    #[test]
    fn pattern_matrix_moves_and_scales_the_cells() {
        let m = [2.0, 0.0, 0.0, 2.0, 1.0, 0.0];
        let p = Pattern::from_tile(2, 2, vec![1, 2, 3, 4], (2.0, 2.0), (4.0, 4.0), m, Tiling::NoDistortion).unwrap();
        // Pattern point (0.5, 0.5) is user (2, 1).
        assert_eq!(p.shade(2.0, 1.0), 1);
        assert_eq!(p.shade(0.5, 1.0), 0, "left of the first cell");
    }

    #[test]
    fn invalid_patterns_are_rejected() {
        assert!(Pattern::from_tile(2, 2, vec![1, 2, 3], (2.0, 2.0), (4.0, 4.0), IDENT, Tiling::NoDistortion).is_none());
        assert!(Pattern::from_tile(2, 2, vec![0; 4], (2.0, 2.0), (0.0, 4.0), IDENT, Tiling::NoDistortion).is_none());
        assert!(Pattern::from_tile(2, 2, vec![0; 4], (2.0, 2.0), (4.0, 4.0), [0.0; 6], Tiling::NoDistortion).is_none());
    }

    #[test]
    fn colored_cell_callback() {
        let red = rgba(255, 0, 0, 255);
        let p = Pattern::with_cell((4.0, 4.0), (4.0, 4.0), IDENT, 2.0, PatternPaint::Colored, Tiling::NoDistortion, |s, k| {
            assert_eq!(k, 2.0);
            assert_eq!((s.w, s.h), (8, 8));
            for i in 0..4 {
                s.px[i] = red;
            }
        })
        .unwrap();
        assert_eq!(p.shade(0.1, 0.1), red);
        assert_eq!(p.shade(0.1, 3.5), 0);
    }

    #[test]
    fn uncolored_cell_takes_the_fill_colour() {
        let blue = Color::from_rgba8(0, 0, 255, 255);
        let p = Pattern::with_cell((2.0, 2.0), (2.0, 2.0), IDENT, 1.0, PatternPaint::Uncolored(blue), Tiling::NoDistortion, |s, _| {
            s.px[0] = 0xFF00_0000; // opaque black: only the alpha matters
            s.px[1] = 0x8000_0000; // half
        })
        .unwrap();
        assert_eq!(p.shade(0.5, 0.5), rgba(0, 0, 255, 255));
        let half = p.shade(1.5, 0.5);
        assert_eq!(half >> 24, 128);
        assert_eq!(p.shade(0.5, 1.5), 0);
    }

    #[test]
    fn tiling_fits_the_step_to_whole_pixels() {
        let p = Pattern::from_tile(2, 2, vec![1, 2, 3, 4], (2.0, 2.0), (3.3, 3.3), IDENT, Tiling::ConstantSpacing).unwrap();
        let f = p.fitted(&IDENT);
        assert!((f.step().0 - 3.0).abs() < 1e-5 && (f.step().1 - 3.0).abs() < 1e-5);
        assert_eq!(f.bounds(), (2.0, 2.0));
        let q = Pattern::from_tile(2, 2, vec![1, 2, 3, 4], (2.0, 2.0), (3.3, 3.3), IDENT, Tiling::ConstantSpacingMinimalDistortion).unwrap();
        let g = q.fitted(&IDENT);
        assert!((g.step().0 - 3.0).abs() < 1e-5);
        assert!(g.bounds().0 < 2.0);
        // Scaled up by 2 the device step 6.6 rounds to 7.
        let h = p.fitted(&[2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
        assert!((h.step().0 - 3.5).abs() < 1e-5);
        let n = Pattern::from_tile(2, 2, vec![0; 4], (2.0, 2.0), (3.3, 3.3), IDENT, Tiling::NoDistortion).unwrap();
        assert_eq!(n.fitted(&IDENT).step(), (3.3, 3.3));
    }

    #[test]
    fn matrix_inverse() {
        let m = [2.0, 0.0, 0.0, 4.0, 10.0, 20.0];
        let i = invert_matrix(&m).unwrap();
        // (3, 5) -> (16, 40) -> back
        let (x, y) = (m[0] * 3.0 + m[2] * 5.0 + m[4], m[1] * 3.0 + m[3] * 5.0 + m[5]);
        let (bx, by) = (i[0] * x + i[2] * y + i[4], i[1] * x + i[3] * y + i[5]);
        assert!((bx - 3.0).abs() < 1e-5 && (by - 5.0).abs() < 1e-5);
        assert!(invert_matrix(&[1.0, 2.0, 2.0, 4.0, 0.0, 0.0]).is_none());
    }

    fn surface(buf: &mut Vec<u32>, w: usize, h: usize) -> Surface<'_> {
        Surface { w, h, stride: w, px: buf, y0: 0 }
    }

    #[test]
    fn solid_paint_fills_a_rect() {
        let mut buf = vec![0u32; 16];
        let mut s = surface(&mut buf, 4, 4);
        Paint::Solid(Color::from_rgba8(255, 0, 0, 255)).fill_rect(&mut s, 1, 1, 3, 3, BlendMode::Normal, 1.0);
        let red = rgba(255, 0, 0, 255);
        assert_eq!(buf[0], 0);
        assert_eq!(buf[5], red);
        assert_eq!(buf[10], red);
        assert_eq!(buf[15], 0);
    }

    #[test]
    fn gradient_rect_runs_left_to_right_and_alpha_scales() {
        let mut buf = vec![0u32; 10];
        let mut s = surface(&mut buf, 10, 1);
        let g = LinearGradient::new((0.0, 0.0), (10.0, 0.0), bw()).extended(true, true);
        Paint::Linear(g).fill_rect(&mut s, 0, 0, 10, 1, BlendMode::Normal, 0.5);
        assert!(buf.windows(2).all(|w| (w[0] & 0xff) <= (w[1] & 0xff)));
        assert_eq!(buf[0] >> 24, 128);
        assert!((buf[9] & 0xff) > 120);
    }

    #[test]
    fn rect_fill_respects_blend_mode_and_clips() {
        let white = rgba(255, 255, 255, 255);
        let mut buf = vec![white; 4];
        let mut s = surface(&mut buf, 2, 2);
        // Multiply by mid grey darkens; the rectangle past the surface edge is clipped.
        Paint::Solid(Color::from_rgba8(128, 128, 128, 255)).fill_rect(&mut s, -5, -5, 1, 9, BlendMode::Multiply, 1.0);
        assert!((buf[0] & 0xff) < 140 && (buf[0] & 0xff) > 120);
        assert_eq!(buf[1], white);
    }

    #[test]
    fn shade_row_maps_through_the_inverse_transform() {
        // User space is device space scaled by 2: the inverse halves the coordinates.
        let g = LinearGradient::new((0.0, 0.0), (10.0, 0.0), bw());
        let mut row = vec![0u32; 20];
        Paint::Linear(g).shade_row(&[0.5, 0.0, 0.0, 0.5, 0.0, 0.0], 0, 0, &mut row);
        assert!((row[10] & 0xff) > 120 && (row[10] & 0xff) < 136, "device 10.5 is user 5.25");
        assert_ne!(row[19], 0);
    }
}
