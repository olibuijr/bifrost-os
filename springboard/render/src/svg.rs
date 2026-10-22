//! SVG reader: parses the static SVG subset that icons and images use and rasterises it to ARGB pixels.
//!
//! Supported: the root `width`, `height` and `viewBox`; the shapes `rect` (with `rx` and `ry`), `circle`, `ellipse`,
//! `line`, `polygon` and `polyline`; and `path` with M, L, H, V, C, S, Q, T, A and Z (absolute and relative, with
//! the compact arc flags minified icons use). `transform` (translate, scale, rotate, skewX, skewY, matrix) works on
//! every element and on `g` groups, and `fill`, `stroke` and their opacity, width, cap, join and miter-limit
//! properties are inherited from groups. Each shape is filled with its `fill` colour (`#rgb`, `#rrggbb`, a few
//! names, or `none`) at its `fill-opacity` using `fill-rule` (non-zero or evenodd); `fill` defaults to black. A
//! `stroke` colour draws the outline with the given width (scaled by the transform) with butt, round or square caps
//! and miter, round or bevel joins; dashes are not drawn. Gradients, `use`, clipping, masks, text and an unparsable
//! transform are not drawn: an element that needs them is skipped, and its pixels stay clear. Defs-like containers
//! (`defs`, `clipPath`, `mask`, ...) are not drawn either. The viewBox maps onto the output with independent x and y
//! scale (no aspect-ratio fitting).

use crate::bezier::{BezierPath, Point, WindingRule};

/// Largest raster accepted, in pixels, so an oversized request cannot exhaust memory.
const MAX_PIXELS: u64 = 16 * 1024 * 1024;

/// Control-point distance, as a fraction of the radius, that approximates a quarter circle with one cubic.
const KAPPA: f32 = 0.552_284_8;

/// A rasterised SVG document: ARGB pixels (`0xAARRGGBB`, not premultiplied), row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct SvgImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

/// Intrinsic size in user units: the root `width` and `height`, falling back to the `viewBox` size.
pub fn intrinsic_size(doc: &str) -> Option<(f32, f32)> {
    let els = elements(doc);
    let root = els.iter().find(|e| e.name == "svg")?;
    let vb = root.attr("viewBox").and_then(nums).filter(|v| v.len() == 4).map(|v| (v[2], v[3]));
    let w = root.attr("width").and_then(length).or(vb.map(|v| v.0))?;
    let h = root.attr("height").and_then(length).or(vb.map(|v| v.1))?;
    sized(w, h)
}

/// Rasterises `doc` to `out_w` by `out_h` pixels. Returns `None` when the document has no usable root `svg`
/// element, or when the requested size is zero or too large.
pub fn rasterize(doc: &str, out_w: u32, out_h: u32) -> Option<SvgImage> {
    let total = u64::from(out_w) * u64::from(out_h);
    if out_w == 0 || out_h == 0 || total > MAX_PIXELS {
        return None;
    }
    let els = elements(doc);
    let root = els.iter().find(|e| e.name == "svg")?;
    let (ox, oy, vw, vh) = view_box_of(root)?;
    let (sx, sy) = (out_w as f32 / vw, out_h as f32 / vh);
    let view = Affine { a: sx, b: 0.0, c: 0.0, d: sy, e: -ox * sx, f: -oy * sy };
    let mut img = SvgImage { width: out_w, height: out_h, pixels: vec![0; total as usize] };
    for e in els.iter().filter(|e| e.name != "svg") {
        let Some(user) = e.xf else { continue };
        let xf = view.compose(user);
        let Some(path) = shape_path(e, &xf) else { continue };
        if let Some((rgb, opacity)) = fill_of(e) {
            let rule = if e.attr("fill-rule").map(str::trim) == Some("evenodd") {
                WindingRule::EvenOdd
            } else {
                WindingRule::NonZero
            };
            paint(&mut img, &path.fill_mask(out_w as usize, out_h as usize, rule), rgb, opacity);
        }
        if let Some((rgb, opacity, width)) = stroke_of(e, &xf) {
            let outline = stroke_outline(&path, width, &StrokeStyle::of(e));
            paint(&mut img, &outline.fill_mask(out_w as usize, out_h as usize, WindingRule::NonZero), rgb, opacity);
        }
    }
    Some(img)
}

/// The largest SVG document [`decode_image`] reads, in bytes.
const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;

impl SvgImage {
    /// The pixels as a renderer image (premultiplied RGBA, R in the low byte).
    pub fn to_image(&self) -> crate::gfx::Image {
        let mut img = crate::gfx::Image::new(self.width as usize, self.height as usize);
        for (out, p) in img.px.iter_mut().zip(&self.pixels) {
            *out = crate::gfx::rgba((p >> 16) as u8, (p >> 8) as u8, *p as u8, (p >> 24) as u8);
        }
        img
    }
}

/// Rasterises SVG `bytes` at their intrinsic size (the root `width` and `height`, else the `viewBox`), scaled down
/// to fit `max_side` pixels on the longer side. `None` when the bytes are not UTF-8 text with a sized root `svg`
/// element, are over four megabytes, or the image would be empty. This is what the image decoders fall back to, so a
/// page or a file that holds an SVG shows as a picture.
pub fn decode_image(bytes: &[u8], max_side: usize) -> Option<crate::gfx::Image> {
    if bytes.len() > MAX_DOCUMENT_BYTES || max_side == 0 {
        return None;
    }
    let doc = std::str::from_utf8(bytes).ok()?;
    if !doc.contains("<svg") {
        return None;
    }
    let (w, h) = intrinsic_size(doc)?;
    let k = (max_side as f32 / w.max(h)).min(1.0);
    let (pw, ph) = (((w * k).round() as u32).max(1), ((h * k).round() as u32).max(1));
    Some(rasterize(doc, pw, ph)?.to_image())
}

/// A 2D affine transform: `x' = a x + c y + e`, `y' = b x + d y + f`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Affine {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl Affine {
    const IDENTITY: Affine = Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    fn translate(tx: f32, ty: f32) -> Affine {
        Affine { e: tx, f: ty, ..Affine::IDENTITY }
    }

    fn scale(sx: f32, sy: f32) -> Affine {
        Affine { a: sx, d: sy, ..Affine::IDENTITY }
    }

    fn rotate(deg: f32) -> Affine {
        let (s, c) = deg.to_radians().sin_cos();
        Affine { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 }
    }

    /// `self` applied after `o`: a point goes through `o` first.
    fn compose(self, o: Affine) -> Affine {
        Affine {
            a: self.a * o.a + self.c * o.b,
            b: self.b * o.a + self.d * o.b,
            c: self.a * o.c + self.c * o.d,
            d: self.b * o.c + self.d * o.d,
            e: self.a * o.e + self.c * o.f + self.e,
            f: self.b * o.e + self.d * o.f + self.f,
        }
    }

    fn pt(&self, x: f32, y: f32) -> Point {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }

    /// The linear scale of the transform: the square root of the area factor. Stroke widths follow it.
    fn scale_factor(&self) -> f32 {
        (self.a * self.d - self.b * self.c).abs().sqrt()
    }
}

/// A `transform` attribute: a list such as `translate(2 3) rotate(45)`, applied as nested transforms from left to
/// right. `None` when a function is unknown or has the wrong number of arguments.
fn parse_transform(s: &str) -> Option<Affine> {
    let mut out = Affine::IDENTITY;
    let mut rest = s.trim();
    while !rest.is_empty() {
        let open = rest.find('(')?;
        let close = rest.find(')')?;
        if close < open {
            return None;
        }
        let name = rest[..open].trim();
        let a = nums(&rest[open + 1..close])?;
        let t = match (name, a.len()) {
            ("translate", 1) => Affine::translate(a[0], 0.0),
            ("translate", 2) => Affine::translate(a[0], a[1]),
            ("scale", 1) => Affine::scale(a[0], a[0]),
            ("scale", 2) => Affine::scale(a[0], a[1]),
            ("rotate", 1) => Affine::rotate(a[0]),
            ("rotate", 3) => {
                Affine::translate(a[1], a[2]).compose(Affine::rotate(a[0])).compose(Affine::translate(-a[1], -a[2]))
            }
            ("skewX", 1) => Affine { c: a[0].to_radians().tan(), ..Affine::IDENTITY },
            ("skewY", 1) => Affine { b: a[0].to_radians().tan(), ..Affine::IDENTITY },
            ("matrix", 6) => Affine { a: a[0], b: a[1], c: a[2], d: a[3], e: a[4], f: a[5] },
            _ => return None,
        };
        out = out.compose(t);
        rest = rest[close + 1..].trim_start().trim_start_matches(',').trim_start();
    }
    Some(out)
}

struct Element {
    name: String,
    /// The element's own attributes plus the inherited presentation attributes of its ancestors.
    attrs: Vec<(String, String)>,
    /// The accumulated `transform` of the element and its groups; `None` when one of them cannot be read.
    xf: Option<Affine>,
}

impl Element {
    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

/// Presentation attributes a child takes from its group when it does not set them itself.
const INHERITED: &[&str] = &[
    "fill",
    "fill-opacity",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-opacity",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
];

/// Elements whose subtree is never drawn directly.
const NOT_DRAWN: &[&str] = &[
    "defs",
    "clipPath",
    "mask",
    "symbol",
    "pattern",
    "linearGradient",
    "radialGradient",
    "filter",
    "text",
    "style",
    "title",
    "desc",
    "metadata",
    "script",
];

/// Start tags in document order, minus everything inside a `NOT_DRAWN` subtree. Each element carries the inherited
/// presentation attributes and the transform of its open ancestors. Comments, processing instructions and
/// declarations are dropped.
fn elements(doc: &str) -> Vec<Element> {
    let mut out = Vec::new();
    let mut skip: Option<&str> = None;
    let mut stack: Vec<Element> = Vec::new();
    let mut rest = doc;
    while let Some(lt) = rest.find('<') {
        rest = &rest[lt + 1..];
        if let Some(after) = rest.strip_prefix("!--") {
            match after.find("-->") {
                Some(end) => rest = &after[end + 3..],
                None => break,
            }
            continue;
        }
        let Some(gt) = rest.find('>') else { break };
        let body = &rest[..gt];
        rest = &rest[gt + 1..];
        if body.starts_with('?') || body.starts_with('!') {
            continue;
        }
        if let Some(close) = body.strip_prefix('/') {
            let close = close.trim();
            if skip == Some(close) {
                skip = None;
            } else if skip.is_none() {
                if let Some(at) = stack.iter().rposition(|f| f.name == close) {
                    stack.truncate(at);
                }
            }
            continue;
        }
        let self_closing = body.trim_end().ends_with('/');
        let (name, mut attrs) = split_tag(body);
        if skip.is_some() {
            continue;
        }
        if let Some(hidden) = NOT_DRAWN.iter().find(|n| **n == name) {
            if !self_closing {
                skip = Some(*hidden);
            }
            continue;
        }
        let parent = stack.last();
        for key in INHERITED {
            if attrs.iter().all(|(k, _)| k != key) {
                if let Some(v) = parent.and_then(|p| p.attr(key)) {
                    attrs.push(((*key).to_string(), v.to_string()));
                }
            }
        }
        let parent_xf = parent.map_or(Some(Affine::IDENTITY), |p| p.xf);
        let local = match attrs.iter().find(|(k, _)| k == "transform") {
            Some((_, t)) if name != "svg" => parse_transform(t),
            _ => Some(Affine::IDENTITY),
        };
        let xf = parent_xf.zip(local).map(|(p, l)| p.compose(l));
        if !self_closing {
            stack.push(Element { name: name.to_string(), attrs: attrs.clone(), xf });
        }
        out.push(Element { name: name.to_string(), attrs, xf });
    }
    out
}

/// Splits a tag body (without `<` and `>`) into its name and `key="value"` attributes.
fn split_tag(body: &str) -> (&str, Vec<(String, String)>) {
    let body = body.trim().trim_end_matches('/').trim_end();
    let name_end = body.find(char::is_whitespace).unwrap_or(body.len());
    let name = &body[..name_end];
    let mut attrs = Vec::new();
    let mut s = body[name_end..].trim_start();
    while !s.is_empty() {
        let Some(eq) = s.find('=') else { break };
        let key = s[..eq].trim().to_string();
        let after = s[eq + 1..].trim_start();
        let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else { break };
        let inner = &after[1..];
        let Some(close) = inner.find(q) else { break };
        attrs.push((key, inner[..close].to_string()));
        s = inner[close + 1..].trim_start();
    }
    (name, attrs)
}

/// The viewBox, or the origin plus the width and height when there is none. Sizes must be positive.
fn view_box_of(root: &Element) -> Option<(f32, f32, f32, f32)> {
    if let Some(v) = root.attr("viewBox").and_then(nums).filter(|v| v.len() == 4) {
        if v[2] > 0.0 && v[3] > 0.0 {
            return Some((v[0], v[1], v[2], v[3]));
        }
    }
    let w = root.attr("width").and_then(length)?;
    let h = root.attr("height").and_then(length)?;
    let (w, h) = sized(w, h)?;
    Some((0.0, 0.0, w, h))
}

fn sized(w: f32, h: f32) -> Option<(f32, f32)> {
    (w > 0.0 && h > 0.0).then_some((w, h))
}

fn opacity_attr(e: &Element, key: &str) -> f32 {
    e.attr(key).and_then(|s| s.trim().parse::<f32>().ok()).unwrap_or(1.0).clamp(0.0, 1.0)
}

/// The fill colour and opacity of an element, or `None` when it is not filled.
fn fill_of(e: &Element) -> Option<(u32, f32)> {
    let paint = e.attr("fill").unwrap_or("#000000").trim();
    if paint == "none" {
        return None;
    }
    Some((color(paint)?, opacity_attr(e, "fill-opacity")))
}

/// The stroke colour, opacity and width in output pixels, or `None` when the element is not stroked (the default).
fn stroke_of(e: &Element, xf: &Affine) -> Option<(u32, f32, f32)> {
    let paint = e.attr("stroke")?.trim();
    if paint == "none" {
        return None;
    }
    let width = e.attr("stroke-width").and_then(length).unwrap_or(1.0) * xf.scale_factor();
    (width > 0.0).then(|| Some((color(paint)?, opacity_attr(e, "stroke-opacity"), width))).flatten()
}

/// `#rgb`, `#rrggbb` or a basic colour name, as `0xRRGGBB`.
fn color(s: &str) -> Option<u32> {
    if let Some(hex) = s.strip_prefix('#') {
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        return match hex.len() {
            3 => {
                let v = u32::from_str_radix(hex, 16).ok()?;
                let (r, g, b) = ((v >> 8) & 0xf, (v >> 4) & 0xf, v & 0xf);
                Some(((r * 17) << 16) | ((g * 17) << 8) | (b * 17))
            }
            6 => u32::from_str_radix(hex, 16).ok(),
            _ => None,
        };
    }
    match s.to_ascii_lowercase().as_str() {
        "black" | "currentcolor" => Some(0x000000),
        "white" => Some(0xffffff),
        "red" => Some(0xff0000),
        "green" => Some(0x008000),
        "blue" => Some(0x0000ff),
        _ => None,
    }
}

/// A plain length such as `24`, `24.5` or `24px`. Percentages and other units are not accepted.
fn length(s: &str) -> Option<f32> {
    let t = s.trim();
    let end = t.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+'))).unwrap_or(t.len());
    match &t[end..] {
        "" | "px" => t[..end].parse().ok(),
        _ => None,
    }
}

/// The numbers of a list such as a viewBox or a points attribute. `None` when anything else appears.
fn nums(s: &str) -> Option<Vec<f32>> {
    tokens(s)?
        .into_iter()
        .map(|t| match t {
            Tok::Num(v) => Some(v),
            Tok::Cmd(_) => None,
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tok {
    Cmd(char),
    Num(f32),
}

/// Reads one number starting at `*i` (a sign, digits, a fraction and an exponent) and moves `*i` past it. A sign or
/// a second decimal point ends the number, so `1.5.5-2` reads as 1.5, 0.5, -2.
fn read_number(b: &[u8], i: &mut usize) -> Option<f32> {
    let start = *i;
    if *i < b.len() && (b[*i] == b'-' || b[*i] == b'+') {
        *i += 1;
    }
    let mut digits = false;
    while *i < b.len() && b[*i].is_ascii_digit() {
        *i += 1;
        digits = true;
    }
    if *i < b.len() && b[*i] == b'.' {
        *i += 1;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
            digits = true;
        }
    }
    if !digits {
        return None;
    }
    if *i < b.len() && (b[*i] == b'e' || b[*i] == b'E') {
        let mut j = *i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        if j < b.len() && b[j].is_ascii_digit() {
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            *i = j;
        }
    }
    std::str::from_utf8(&b[start..*i]).ok()?.parse::<f32>().ok()
}

/// Splits a list into command letters and numbers. Separators are whitespace and commas.
fn tokens(d: &str) -> Option<Vec<Tok>> {
    let b = d.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_alphabetic() {
            out.push(Tok::Cmd(c as char));
            i += 1;
        } else if c.is_ascii_whitespace() || c == b',' {
            i += 1;
        } else if c == b'-' || c == b'+' || c == b'.' || c.is_ascii_digit() {
            out.push(Tok::Num(read_number(b, &mut i)?));
        } else {
            return None;
        }
    }
    Some(out)
}

/// Builds a path in user units and puts every point through the transform on the way in.
struct Pen {
    path: BezierPath,
    xf: Affine,
}

impl Pen {
    fn new(xf: &Affine) -> Pen {
        Pen { path: BezierPath::new(), xf: *xf }
    }

    fn mv(&mut self, p: Point) {
        self.path.move_to(self.xf.pt(p.0, p.1));
    }

    fn ln(&mut self, p: Point) {
        self.path.line_to(self.xf.pt(p.0, p.1));
    }

    fn cv(&mut self, c1: Point, c2: Point, p: Point) {
        self.path.curve_to(self.xf.pt(c1.0, c1.1), self.xf.pt(c2.0, c2.1), self.xf.pt(p.0, p.1));
    }

    /// A quadratic raised to the equivalent cubic: each control point moves two thirds of the way.
    fn qv(&mut self, from: Point, qc: Point, p: Point) {
        let c1 = (from.0 + 2.0 / 3.0 * (qc.0 - from.0), from.1 + 2.0 / 3.0 * (qc.1 - from.1));
        let c2 = (p.0 + 2.0 / 3.0 * (qc.0 - p.0), p.1 + 2.0 / 3.0 * (qc.1 - p.1));
        self.cv(c1, c2, p);
    }

    /// A closed rounded rectangle; radii are clamped to half the box, and zero radii give a plain rectangle.
    fn round_rect(&mut self, x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32) {
        let rx = rx.clamp(0.0, w / 2.0);
        let ry = ry.clamp(0.0, h / 2.0);
        let (r, b) = (x + w, y + h);
        if rx == 0.0 || ry == 0.0 {
            self.mv((x, y));
            self.ln((r, y));
            self.ln((r, b));
            self.ln((x, b));
        } else {
            let (kx, ky) = (rx * KAPPA, ry * KAPPA);
            self.mv((x + rx, y));
            self.ln((r - rx, y));
            self.cv((r - rx + kx, y), (r, y + ry - ky), (r, y + ry));
            self.ln((r, b - ry));
            self.cv((r, b - ry + ky), (r - rx + kx, b), (r - rx, b));
            self.ln((x + rx, b));
            self.cv((x + rx - kx, b), (x, b - ry + ky), (x, b - ry));
            self.ln((x, y + ry));
            self.cv((x, y + ry - ky), (x + rx - kx, y), (x + rx, y));
        }
        self.path.close();
    }

    /// The elliptical arc of the path-data `A` command, as one cubic per quarter turn at most.
    #[allow(clippy::too_many_arguments)]
    fn arc(&mut self, from: Point, rx: f32, ry: f32, rot_deg: f32, large: bool, sweep: bool, to: Point) {
        if from == to {
            return;
        }
        let (mut rx, mut ry) = (rx.abs(), ry.abs());
        if rx == 0.0 || ry == 0.0 {
            self.ln(to);
            return;
        }
        let (sin_p, cos_p) = rot_deg.to_radians().sin_cos();
        let (dx, dy) = ((from.0 - to.0) / 2.0, (from.1 - to.1) / 2.0);
        let (x1, y1) = (cos_p * dx + sin_p * dy, -sin_p * dx + cos_p * dy);
        let lambda = x1 * x1 / (rx * rx) + y1 * y1 / (ry * ry);
        if lambda > 1.0 {
            let s = lambda.sqrt();
            rx *= s;
            ry *= s;
        }
        let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
        let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
        let sign = if large == sweep { -1.0 } else { 1.0 };
        let coef = sign * (num / den).max(0.0).sqrt();
        let (cxp, cyp) = (coef * rx * y1 / ry, -coef * ry * x1 / rx);
        let cx = cos_p * cxp - sin_p * cyp + (from.0 + to.0) / 2.0;
        let cy = sin_p * cxp + cos_p * cyp + (from.1 + to.1) / 2.0;
        let angle = |ux: f32, uy: f32, vx: f32, vy: f32| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
        let u = ((x1 - cxp) / rx, (y1 - cyp) / ry);
        let v = ((-x1 - cxp) / rx, (-y1 - cyp) / ry);
        let theta = angle(1.0, 0.0, u.0, u.1);
        let mut delta = angle(u.0, u.1, v.0, v.1);
        let tau = std::f32::consts::TAU;
        if !sweep && delta > 0.0 {
            delta -= tau;
        } else if sweep && delta < 0.0 {
            delta += tau;
        }
        let n = ((delta.abs() / std::f32::consts::FRAC_PI_2) - 1e-4).ceil().max(1.0) as usize;
        let step = delta / n as f32;
        let t = 4.0 / 3.0 * (step / 4.0).tan();
        let at = |a: f32| {
            let (s, c) = a.sin_cos();
            (cx + rx * c * cos_p - ry * s * sin_p, cy + rx * c * sin_p + ry * s * cos_p)
        };
        let slope = |a: f32| {
            let (s, c) = a.sin_cos();
            (-rx * s * cos_p - ry * c * sin_p, -rx * s * sin_p + ry * c * cos_p)
        };
        for k in 0..n {
            let (a1, a2) = (theta + step * k as f32, theta + step * (k + 1) as f32);
            let (p1, d1, p2, d2) = (at(a1), slope(a1), at(a2), slope(a2));
            let end = if k + 1 == n { to } else { p2 };
            self.cv((p1.0 + t * d1.0, p1.1 + t * d1.1), (p2.0 - t * d2.0, p2.1 - t * d2.1), end);
        }
    }
}

/// The path of a drawable element in output pixels, or `None` when it has no geometry we draw.
fn shape_path(e: &Element, xf: &Affine) -> Option<BezierPath> {
    let num = |k: &str| e.attr(k).and_then(length);
    let or0 = |k: &str| num(k).unwrap_or(0.0);
    let mut pen = Pen::new(xf);
    match e.name.as_str() {
        "rect" => {
            let (w, h) = sized(num("width")?, num("height")?)?;
            let (rx, ry) = match (num("rx"), num("ry")) {
                (Some(rx), Some(ry)) => (rx, ry),
                (Some(r), None) | (None, Some(r)) => (r, r),
                (None, None) => (0.0, 0.0),
            };
            pen.round_rect(or0("x"), or0("y"), w, h, rx, ry);
        }
        "circle" => {
            let r = num("r")?;
            let (rx, ry) = sized(r, r)?;
            pen.round_rect(or0("cx") - rx, or0("cy") - ry, 2.0 * rx, 2.0 * ry, rx, ry);
        }
        "ellipse" => {
            let (rx, ry) = sized(num("rx")?, num("ry")?)?;
            pen.round_rect(or0("cx") - rx, or0("cy") - ry, 2.0 * rx, 2.0 * ry, rx, ry);
        }
        "line" => {
            pen.mv((or0("x1"), or0("y1")));
            pen.ln((or0("x2"), or0("y2")));
        }
        "polygon" | "polyline" => {
            let v = nums(e.attr("points")?)?;
            if v.len() < 4 || v.len() % 2 != 0 {
                return None;
            }
            for (i, xy) in v.chunks(2).enumerate() {
                if i == 0 {
                    pen.mv((xy[0], xy[1]));
                } else {
                    pen.ln((xy[0], xy[1]));
                }
            }
            if e.name == "polygon" {
                pen.path.close();
            }
        }
        "path" => return path_data(e.attr("d")?, xf),
        _ => return None,
    }
    Some(pen.path)
}

/// A point given in user units, made absolute when the command is relative.
fn abs(cur: Point, rel: bool, x: f32, y: f32) -> Point {
    if rel {
        (cur.0 + x, cur.1 + y)
    } else {
        (x, y)
    }
}

/// Reads path data one value at a time. Arc flags are single characters, so `a1 1 0 011 1` reads as flags 0 and 1.
struct Scan<'a> {
    b: &'a [u8],
    i: usize,
}

impl Scan<'_> {
    fn skip_sep(&mut self) {
        while self.i < self.b.len() && (self.b[self.i].is_ascii_whitespace() || self.b[self.i] == b',') {
            self.i += 1;
        }
    }

    fn at_end(&mut self) -> bool {
        self.skip_sep();
        self.i >= self.b.len()
    }

    fn command(&mut self) -> Option<char> {
        self.skip_sep();
        let c = *self.b.get(self.i)?;
        c.is_ascii_alphabetic().then(|| {
            self.i += 1;
            c as char
        })
    }

    fn number(&mut self) -> Option<f32> {
        self.skip_sep();
        read_number(self.b, &mut self.i)
    }

    fn pair(&mut self) -> Option<(f32, f32)> {
        Some((self.number()?, self.number()?))
    }

    fn flag(&mut self) -> Option<bool> {
        self.skip_sep();
        let v = match self.b.get(self.i)? {
            b'0' => false,
            b'1' => true,
            _ => return None,
        };
        self.i += 1;
        Some(v)
    }
}

/// Path data with M, L, H, V, C, S, Q, T, A and Z, absolute and relative. `None` when it is malformed.
fn path_data(d: &str, xf: &Affine) -> Option<BezierPath> {
    let mut sc = Scan { b: d.as_bytes(), i: 0 };
    let mut pen = Pen::new(xf);
    let mut cur: Point = (0.0, 0.0);
    let mut start: Point = cur;
    let mut cmd = ' ';
    // The last cubic and quadratic control points, which S and T reflect.
    let (mut prev_c, mut prev_q): (Option<Point>, Option<Point>) = (None, None);
    let mut first = true;
    while !sc.at_end() {
        if let Some(c) = sc.command() {
            cmd = c;
            if first && !matches!(c, 'M' | 'm') {
                return None;
            }
        } else if cmd == ' ' || cmd == 'Z' || cmd == 'z' {
            return None;
        }
        first = false;
        let upper = cmd.to_ascii_uppercase();
        let rel = cmd.is_ascii_lowercase();
        let (mut next_c, mut next_q) = (None, None);
        match upper {
            'Z' => {
                pen.path.close();
                cur = start;
            }
            'M' => {
                let (x, y) = sc.pair()?;
                let q = abs(cur, rel, x, y);
                pen.mv(q);
                cur = q;
                start = q;
                // Further coordinate pairs after a move are line-tos.
                cmd = if rel { 'l' } else { 'L' };
            }
            'L' => {
                let (x, y) = sc.pair()?;
                cur = abs(cur, rel, x, y);
                pen.ln(cur);
            }
            'H' => {
                let x = sc.number()?;
                cur.0 = if rel { cur.0 + x } else { x };
                pen.ln(cur);
            }
            'V' => {
                let y = sc.number()?;
                cur.1 = if rel { cur.1 + y } else { y };
                pen.ln(cur);
            }
            'C' => {
                let (c1, c2, q) = (sc.pair()?, sc.pair()?, sc.pair()?);
                let (c1, c2, q) = (abs(cur, rel, c1.0, c1.1), abs(cur, rel, c2.0, c2.1), abs(cur, rel, q.0, q.1));
                pen.cv(c1, c2, q);
                next_c = Some(c2);
                cur = q;
            }
            'S' => {
                let (c2, q) = (sc.pair()?, sc.pair()?);
                let (c2, q) = (abs(cur, rel, c2.0, c2.1), abs(cur, rel, q.0, q.1));
                let c1 = prev_c.map_or(cur, |p| (2.0 * cur.0 - p.0, 2.0 * cur.1 - p.1));
                pen.cv(c1, c2, q);
                next_c = Some(c2);
                cur = q;
            }
            'Q' => {
                let (qc, q) = (sc.pair()?, sc.pair()?);
                let (qc, q) = (abs(cur, rel, qc.0, qc.1), abs(cur, rel, q.0, q.1));
                pen.qv(cur, qc, q);
                next_q = Some(qc);
                cur = q;
            }
            'T' => {
                let q = sc.pair()?;
                let q = abs(cur, rel, q.0, q.1);
                let qc = prev_q.map_or(cur, |p| (2.0 * cur.0 - p.0, 2.0 * cur.1 - p.1));
                pen.qv(cur, qc, q);
                next_q = Some(qc);
                cur = q;
            }
            'A' => {
                let (rx, ry, rot) = (sc.number()?, sc.number()?, sc.number()?);
                let (large, sweep) = (sc.flag()?, sc.flag()?);
                let q = sc.pair()?;
                let q = abs(cur, rel, q.0, q.1);
                pen.arc(cur, rx, ry, rot, large, sweep, q);
                cur = q;
            }
            _ => return None,
        }
        prev_c = next_c;
        prev_q = next_q;
    }
    Some(pen.path)
}

/// Caps and joins of a stroke, and the miter limit, read from the element.
struct StrokeStyle {
    cap: Cap,
    join: Join,
    miter_limit: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum Cap {
    Butt,
    Round,
    Square,
}

#[derive(Clone, Copy, PartialEq)]
enum Join {
    Miter,
    Round,
    Bevel,
}

impl StrokeStyle {
    fn of(e: &Element) -> StrokeStyle {
        StrokeStyle {
            cap: match e.attr("stroke-linecap").map(str::trim) {
                Some("round") => Cap::Round,
                Some("square") => Cap::Square,
                _ => Cap::Butt,
            },
            join: match e.attr("stroke-linejoin").map(str::trim) {
                Some("round") => Join::Round,
                Some("bevel") => Join::Bevel,
                _ => Join::Miter,
            },
            miter_limit: e
                .attr("stroke-miterlimit")
                .and_then(|s| s.trim().parse::<f32>().ok())
                .filter(|m| *m >= 1.0)
                .unwrap_or(4.0),
        }
    }
}

/// Adds a polygon to the outline, wound the same way as every other piece so that the non-zero rule unites them.
fn add_piece(out: &mut BezierPath, pts: &[Point]) {
    if pts.len() < 3 {
        return;
    }
    let mut area = 0.0;
    for (i, p) in pts.iter().enumerate() {
        let q = pts[(i + 1) % pts.len()];
        area += p.0 * q.1 - q.0 * p.1;
    }
    if area.abs() < 1e-6 {
        return;
    }
    let ordered: Vec<Point> = if area < 0.0 { pts.iter().rev().copied().collect() } else { pts.to_vec() };
    out.move_to(ordered[0]);
    for p in &ordered[1..] {
        out.line_to(*p);
    }
    out.close();
}

fn circle_piece(out: &mut BezierPath, c: Point, r: f32) {
    let n = 24;
    let pts: Vec<Point> = (0..n)
        .map(|k| {
            let (s, co) = (k as f32 / n as f32 * std::f32::consts::TAU).sin_cos();
            (c.0 + r * co, c.1 + r * s)
        })
        .collect();
    add_piece(out, &pts);
}

/// The outline of the stroke of `path` (already in output pixels) at `width` pixels: one quadrilateral per segment,
/// plus cap and join pieces. Overlapping pieces are united by filling the result with the non-zero rule.
fn stroke_outline(path: &BezierPath, width: f32, style: &StrokeStyle) -> BezierPath {
    let hw = width / 2.0;
    let mut out = BezierPath::new();
    for (raw, closed) in path.flatten_with_close(0.25) {
        let mut pts: Vec<Point> = Vec::with_capacity(raw.len());
        for p in raw {
            if pts.last().map_or(true, |q| (q.0 - p.0).hypot(q.1 - p.1) > 1e-4) {
                pts.push(p);
            }
        }
        if closed && pts.len() > 1 && (pts[0].0 - pts[pts.len() - 1].0).hypot(pts[0].1 - pts[pts.len() - 1].1) <= 1e-4 {
            pts.pop();
        }
        if pts.len() == 1 {
            // A zero-length subpath draws a dot only with a round cap and a square only with a square cap.
            let p = pts[0];
            match style.cap {
                Cap::Round => circle_piece(&mut out, p, hw),
                Cap::Square => add_piece(&mut out, &[(p.0 - hw, p.1 - hw), (p.0 + hw, p.1 - hw), (p.0 + hw, p.1 + hw), (p.0 - hw, p.1 + hw)]),
                Cap::Butt => {}
            }
            continue;
        }
        if pts.len() < 2 {
            continue;
        }
        let n = pts.len();
        let seg_count = if closed && n > 2 { n } else { n - 1 };
        let seg = |k: usize| (pts[k], pts[(k + 1) % n]);
        let dir = |k: usize| {
            let (a, b) = seg(k);
            let l = (b.0 - a.0).hypot(b.1 - a.1);
            ((b.0 - a.0) / l, (b.1 - a.1) / l)
        };
        for k in 0..seg_count {
            let (mut a, mut b) = seg(k);
            let d = dir(k);
            let open = !(closed && n > 2);
            if open && style.cap == Cap::Square {
                if k == 0 {
                    a = (a.0 - d.0 * hw, a.1 - d.1 * hw);
                }
                if k == seg_count - 1 {
                    b = (b.0 + d.0 * hw, b.1 + d.1 * hw);
                }
            }
            let nm = (-d.1 * hw, d.0 * hw);
            add_piece(&mut out, &[(a.0 + nm.0, a.1 + nm.1), (b.0 + nm.0, b.1 + nm.1), (b.0 - nm.0, b.1 - nm.1), (a.0 - nm.0, a.1 - nm.1)]);
        }
        if !(closed && n > 2) && style.cap == Cap::Round {
            circle_piece(&mut out, pts[0], hw);
            circle_piece(&mut out, pts[n - 1], hw);
        }
        let joints = if closed && n > 2 { 0..n } else { 1..n - 1 };
        for v in joints {
            join_piece(&mut out, pts[v], dir((v + n - 1) % n), dir(v % (n)), hw, style);
        }
    }
    out
}

/// The piece that fills the outer gap where a segment heading `d0` meets one heading `d1` at `p`.
fn join_piece(out: &mut BezierPath, p: Point, d0: Point, d1: Point, hw: f32, style: &StrokeStyle) {
    let cross = d0.0 * d1.1 - d0.1 * d1.0;
    let dot = d0.0 * d1.0 + d0.1 * d1.1;
    if cross.abs() < 1e-6 && dot > 0.0 {
        return;
    }
    if style.join == Join::Round {
        circle_piece(out, p, hw);
        return;
    }
    // The outer side is opposite the direction of the turn.
    let s = if cross > 0.0 { -1.0 } else { 1.0 };
    let n0 = (-d0.1 * s, d0.0 * s);
    let n1 = (-d1.1 * s, d1.0 * s);
    let a = (p.0 + n0.0 * hw, p.1 + n0.1 * hw);
    let b = (p.0 + n1.0 * hw, p.1 + n1.1 * hw);
    // Miter length over stroke width is 1 / sin(half the angle between the segments).
    let half_sin = ((1.0 + dot) / 2.0).max(0.0).sqrt();
    if style.join == Join::Miter && half_sin > 1e-3 && 1.0 / half_sin <= style.miter_limit {
        let k = hw / (1.0 + n0.0 * n1.0 + n0.1 * n1.1);
        let apex = (p.0 + (n0.0 + n1.0) * k, p.1 + (n0.1 + n1.1) * k);
        add_piece(out, &[p, a, apex, b]);
    } else {
        add_piece(out, &[p, a, b]);
    }
}

/// Source-over composite of one solid colour through a 0 or 255 coverage mask.
fn paint(img: &mut SvgImage, mask: &[u8], rgb: u32, opacity: f32) {
    let shape_alpha = (opacity * 255.0).round() as u32;
    for (px, cover) in img.pixels.iter_mut().zip(mask) {
        if *cover == 0 || shape_alpha == 0 {
            continue;
        }
        let src_a = shape_alpha * u32::from(*cover) / 255;
        let old = *px;
        let dst_a = old >> 24;
        let dst_w = dst_a * (255 - src_a) / 255;
        let out_a = src_a + dst_w;
        if out_a == 0 {
            continue;
        }
        let ch = |shift: u32| {
            let s = (rgb >> shift) & 0xff;
            let d = (old >> shift) & 0xff;
            (s * src_a + d * dst_w) / out_a
        };
        *px = (out_a << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(img: &SvgImage, x: u32, y: u32) -> u32 {
        img.pixels[(y * img.width + x) as usize]
    }

    #[test]
    fn rect_fills_its_pixels_and_nothing_else() {
        let doc = r##"<svg viewBox="0 0 4 4"><rect x="1" y="1" width="2" height="2" fill="#ff0000"/></svg>"##;
        let img = rasterize(doc, 4, 4).unwrap();
        assert_eq!(at(&img, 1, 1), 0xffff0000);
        assert_eq!(at(&img, 2, 2), 0xffff0000);
        assert_eq!(at(&img, 0, 0), 0);
        assert_eq!(at(&img, 3, 3), 0);
    }

    #[test]
    fn fill_none_is_not_drawn() {
        let doc = r#"<svg viewBox="0 0 4 4"><rect width="4" height="4" fill="none"/></svg>"#;
        let img = rasterize(doc, 4, 4).unwrap();
        assert!(img.pixels.iter().all(|p| *p == 0));
    }

    #[test]
    fn circle_defaults_to_black_and_leaves_corners_clear() {
        let doc = r#"<svg viewBox="0 0 10 10"><circle cx="5" cy="5" r="5"/></svg>"#;
        let img = rasterize(doc, 10, 10).unwrap();
        assert_eq!(at(&img, 5, 5), 0xff000000);
        assert_eq!(at(&img, 0, 0), 0);
    }

    #[test]
    fn relative_path_commands_draw_a_square() {
        let doc = r#"<svg viewBox="0 0 4 4"><path d="M1 1 h2 v2 h-2 z"/></svg>"#;
        let img = rasterize(doc, 4, 4).unwrap();
        assert_eq!(at(&img, 1, 1), 0xff000000);
        assert_eq!(at(&img, 2, 2), 0xff000000);
        assert_eq!(at(&img, 3, 3), 0);
    }

    #[test]
    fn viewbox_scales_to_the_output_size() {
        let doc = r#"<svg viewBox="0 0 2 2"><rect width="1" height="1"/></svg>"#;
        let img = rasterize(doc, 4, 4).unwrap();
        assert_eq!(at(&img, 1, 1), 0xff000000);
        assert_eq!(at(&img, 2, 2), 0);
    }

    #[test]
    fn fill_opacity_sets_alpha() {
        let doc = r#"<svg viewBox="0 0 2 2"><rect width="2" height="2" fill-opacity="0.5"/></svg>"#;
        let img = rasterize(doc, 2, 2).unwrap();
        assert_eq!(at(&img, 0, 0) >> 24, 128);
    }

    #[test]
    fn defs_are_not_drawn() {
        let doc = r#"<svg viewBox="0 0 2 2"><defs><rect width="2" height="2"/></defs></svg>"#;
        let img = rasterize(doc, 2, 2).unwrap();
        assert!(img.pixels.iter().all(|p| *p == 0));
    }

    #[test]
    fn an_arc_draws_the_half_disc_on_the_sweep_side() {
        // From (0,2) to (4,2) with the sweep flag set the arc runs over the top, so the disc is the upper half.
        let doc = r#"<svg viewBox="0 0 4 4"><path d="M0 2 A2 2 0 0 1 4 2 Z"/></svg>"#;
        let img = rasterize(doc, 4, 4).unwrap();
        assert_eq!(at(&img, 2, 0), 0xff000000);
        assert_eq!(at(&img, 2, 1), 0xff000000);
        assert_eq!(at(&img, 2, 3), 0);
        assert_eq!(at(&img, 0, 0), 0);
        let flipped = rasterize(r#"<svg viewBox="0 0 4 4"><path d="M0 2 A2 2 0 0 0 4 2 Z"/></svg>"#, 4, 4).unwrap();
        assert_eq!(at(&flipped, 2, 3), 0xff000000);
        assert_eq!(at(&flipped, 2, 0), 0);
    }

    #[test]
    fn a_too_small_arc_radius_is_scaled_up_and_the_arc_ends_at_the_target() {
        let mut pen = Pen::new(&Affine::IDENTITY);
        pen.mv((0.0, 0.0));
        pen.arc((0.0, 0.0), 0.5, 0.5, 0.0, false, true, (4.0, 0.0));
        let (x, y, w, h) = pen.path.bounds().unwrap();
        assert!((x - 0.0).abs() < 0.1 && (w - 4.0).abs() < 0.1);
        // the radius grew to 2, so the half circle bulges 2 units
        assert!((h - 2.0).abs() < 0.1 && (y + 2.0).abs() < 0.1, "bounds {x} {y} {w} {h}");
        assert_eq!(pen.path.current_point(), Some((4.0, 0.0)));
    }

    #[test]
    fn compact_arc_flags_parse() {
        // Minified icons write the two flags with no separator: "a2 2 0 01 2 2" is flags 0 and 1, then 2 2.
        let compact = path_data("M2 0 a2 2 0 01 2 2", &Affine::IDENTITY).unwrap();
        let spaced = path_data("M2 0 a2 2 0 0 1 2 2", &Affine::IDENTITY).unwrap();
        assert_eq!(compact, spaced);
        assert!(path_data("M2 0 a2 2 0 21 2 2", &Affine::IDENTITY).is_none());
    }

    #[test]
    fn smooth_curve_reflects_the_previous_control_point() {
        let smooth = rasterize(r#"<svg viewBox="0 0 4 4"><path d="M0 4 C0 3 1 1 2 1 S4 3 4 4 Z"/></svg>"#, 8, 8).unwrap();
        let spelled = rasterize(r#"<svg viewBox="0 0 4 4"><path d="M0 4 C0 3 1 1 2 1 C3 1 4 3 4 4 Z"/></svg>"#, 8, 8).unwrap();
        assert_eq!(smooth, spelled);
        assert!(smooth.pixels.iter().any(|p| *p != 0));
    }

    #[test]
    fn smooth_quadratic_reflects_the_previous_control_point() {
        let smooth = rasterize(r#"<svg viewBox="0 0 8 8"><path d="M0 4 Q2 0 4 4 T8 4 Z"/></svg>"#, 8, 8).unwrap();
        let spelled = rasterize(r#"<svg viewBox="0 0 8 8"><path d="M0 4 Q2 0 4 4 Q6 8 8 4 Z"/></svg>"#, 8, 8).unwrap();
        assert_eq!(smooth, spelled);
    }

    #[test]
    fn a_path_must_start_with_a_move() {
        assert!(path_data("L1 1", &Affine::IDENTITY).is_none());
        assert!(path_data("M0 0 L1", &Affine::IDENTITY).is_none());
    }

    #[test]
    fn group_translate_moves_its_children() {
        let doc = r#"<svg viewBox="0 0 4 4"><g transform="translate(2 0)"><rect width="2" height="2"/></g></svg>"#;
        let img = rasterize(doc, 4, 4).unwrap();
        assert_eq!(at(&img, 2, 0), 0xff000000);
        assert_eq!(at(&img, 3, 1), 0xff000000);
        assert_eq!(at(&img, 0, 0), 0);
    }

    #[test]
    fn nested_transforms_compose_outer_first() {
        let doc = r#"<svg viewBox="0 0 8 8"><g transform="translate(2 0)"><rect width="1" height="1" transform="scale(2)"/></g></svg>"#;
        let img = rasterize(doc, 8, 8).unwrap();
        assert_eq!(at(&img, 2, 0), 0xff000000);
        assert_eq!(at(&img, 3, 1), 0xff000000);
        assert_eq!(at(&img, 4, 0), 0);
        assert_eq!(at(&img, 1, 0), 0);
    }

    #[test]
    fn rotate_about_a_point_turns_a_bar_upright() {
        // (x, y) -> (4 - y, x): the bar across the top becomes a bar down the right edge.
        let doc = r#"<svg viewBox="0 0 4 4"><rect width="4" height="1" transform="rotate(90 2 2)"/></svg>"#;
        let img = rasterize(doc, 4, 4).unwrap();
        assert_eq!(at(&img, 3, 0), 0xff000000);
        assert_eq!(at(&img, 3, 3), 0xff000000);
        assert_eq!(at(&img, 0, 0), 0);
        assert_eq!(at(&img, 2, 1), 0);
    }

    #[test]
    fn transform_lists_parse_and_bad_ones_are_refused() {
        let t = parse_transform("translate(1,2) scale(3)").unwrap();
        assert_eq!(t.pt(1.0, 1.0), (4.0, 5.0));
        assert_eq!(parse_transform("matrix(1 0 0 1 5 6)").unwrap().pt(0.0, 0.0), (5.0, 6.0));
        assert_eq!(parse_transform("").unwrap(), Affine::IDENTITY);
        assert!(parse_transform("wobble(1)").is_none());
        assert!(parse_transform("translate(1 2 3)").is_none());
        let doc = r#"<svg viewBox="0 0 2 2"><rect width="2" height="2" transform="wobble(1)"/></svg>"#;
        assert!(rasterize(doc, 2, 2).unwrap().pixels.iter().all(|p| *p == 0));
    }

    #[test]
    fn groups_pass_their_paint_down_and_children_override_it() {
        let doc = r##"<svg viewBox="0 0 4 2"><g fill="#ff0000"><rect width="2" height="2"/><rect x="2" width="2" height="2" fill="#0000ff"/></g></svg>"##;
        let img = rasterize(doc, 4, 2).unwrap();
        assert_eq!(at(&img, 0, 0), 0xffff0000);
        assert_eq!(at(&img, 3, 0), 0xff0000ff);
    }

    #[test]
    fn a_closed_group_stops_passing_its_paint_down() {
        let doc = r##"<svg viewBox="0 0 4 2"><g fill="#ff0000"><rect width="2" height="2"/></g><rect x="2" width="2" height="2"/></svg>"##;
        let img = rasterize(doc, 4, 2).unwrap();
        assert_eq!(at(&img, 0, 0), 0xffff0000);
        assert_eq!(at(&img, 3, 0), 0xff000000);
    }

    #[test]
    fn evenodd_leaves_the_inner_square_open() {
        let d = "M0 0h4v4h-4z M1 1h2v2h-2z";
        let nonzero = rasterize(&format!(r#"<svg viewBox="0 0 4 4"><path d="{d}"/></svg>"#), 4, 4).unwrap();
        let evenodd = rasterize(&format!(r#"<svg viewBox="0 0 4 4"><path fill-rule="evenodd" d="{d}"/></svg>"#), 4, 4).unwrap();
        assert_eq!(at(&nonzero, 2, 2), 0xff000000);
        assert_eq!(at(&evenodd, 2, 2), 0);
        assert_eq!(at(&evenodd, 0, 0), 0xff000000);
    }

    #[test]
    fn rounded_rect_clears_its_corners() {
        let doc = r#"<svg viewBox="0 0 4 4"><rect width="4" height="4" rx="2"/></svg>"#;
        let img = rasterize(doc, 4, 4).unwrap();
        assert_eq!(at(&img, 0, 0), 0);
        assert_eq!(at(&img, 3, 3), 0);
        assert_eq!(at(&img, 2, 2), 0xff000000);
    }

    #[test]
    fn stroke_draws_a_ring_around_an_unfilled_rect() {
        let doc = r##"<svg viewBox="0 0 8 8"><rect x="2" y="2" width="4" height="4" fill="none" stroke="#0000ff" stroke-width="2"/></svg>"##;
        let img = rasterize(doc, 8, 8).unwrap();
        assert_eq!(at(&img, 1, 1), 0xff0000ff, "mitered corner");
        assert_eq!(at(&img, 4, 1), 0xff0000ff);
        assert_eq!(at(&img, 6, 4), 0xff0000ff);
        assert_eq!(at(&img, 4, 4), 0, "the hole");
        assert_eq!(at(&img, 0, 0), 0);
    }

    #[test]
    fn stroke_is_off_by_default_and_follows_stroke_opacity() {
        let drawn = r#"<svg viewBox="0 0 4 4"><line x1="0" y1="2" x2="4" y2="2" stroke="black" stroke-width="2"/></svg>"#;
        assert_eq!(at(&rasterize(drawn, 4, 4).unwrap(), 2, 1), 0xff000000);
        let none = rasterize(r#"<svg viewBox="0 0 4 4"><line x1="0" y1="2" x2="4" y2="2"/></svg>"#, 4, 4).unwrap();
        assert!(none.pixels.iter().all(|p| *p == 0));
        let faint = r#"<svg viewBox="0 0 4 4"><line x1="0" y1="2" x2="4" y2="2" stroke="black" stroke-width="2" stroke-opacity="0.5"/></svg>"#;
        assert_eq!(at(&rasterize(faint, 4, 4).unwrap(), 2, 1) >> 24, 128);
    }

    #[test]
    fn stroke_width_follows_the_transform_scale() {
        // Width 1 under scale(2) is two pixels: the line at y 4 covers rows 3 and 4 and not row 2.
        let doc = r#"<svg viewBox="0 0 8 8"><g stroke="black" stroke-width="1" transform="scale(2)"><line x1="0" y1="2" x2="4" y2="2"/></g></svg>"#;
        let img = rasterize(doc, 8, 8).unwrap();
        assert_eq!(at(&img, 2, 3), 0xff000000);
        assert_eq!(at(&img, 2, 4), 0xff000000);
        assert_eq!(at(&img, 2, 2), 0);
        assert_eq!(at(&img, 2, 5), 0);
    }

    #[test]
    fn linecaps_extend_or_not() {
        let line = |cap: &str| {
            let doc = format!(r##"<svg viewBox="0 0 6 4"><line x1="1" y1="2" x2="5" y2="2" stroke="#000" stroke-width="2" stroke-linecap="{cap}"/></svg>"##);
            rasterize(&doc, 6, 4).unwrap()
        };
        assert_eq!(at(&line("butt"), 0, 1), 0);
        assert_eq!(at(&line("square"), 0, 1), 0xff000000);
        assert_eq!(at(&line("square"), 5, 2), 0xff000000);
        let round = line("round");
        assert_eq!(at(&round, 0, 1), 0xff000000, "the round cap covers the pixel next to the end");
    }

    #[test]
    fn miter_joins_reach_the_corner_and_bevels_cut_it() {
        let poly = |join: &str| {
            let doc = format!(
                r##"<svg viewBox="0 0 8 8"><polyline points="1,7 1,1 7,1" fill="none" stroke="#000" stroke-width="2" stroke-linejoin="{join}"/></svg>"##
            );
            rasterize(&doc, 16, 16).unwrap()
        };
        assert_eq!(at(&poly("miter"), 0, 0), 0xff000000);
        assert_eq!(at(&poly("bevel"), 0, 0), 0);
        assert_eq!(at(&poly("round"), 0, 0), 0, "a round join stays inside the corner square");
        assert_eq!(at(&poly("round"), 2, 2), 0xff000000);
    }

    #[test]
    fn a_sharp_angle_over_the_miter_limit_falls_back_to_a_bevel() {
        let mut out = BezierPath::new();
        let style = StrokeStyle { cap: Cap::Butt, join: Join::Miter, miter_limit: 4.0 };
        // a hairpin turn: the miter would be many widths long
        join_piece(&mut out, (0.0, 0.0), (1.0, 0.0), (-0.98, 0.199), 1.0, &style);
        let (_, _, w, h) = out.bounds().unwrap();
        assert!(w < 3.0 && h < 3.0, "bevel stays near the point: {w} x {h}");
        let mut ok = BezierPath::new();
        join_piece(&mut ok, (0.0, 0.0), (1.0, 0.0), (0.0, 1.0), 1.0, &style);
        assert!(ok.bounds().is_some());
    }

    #[test]
    fn a_zero_length_subpath_with_round_caps_is_a_dot() {
        let doc = r##"<svg viewBox="0 0 4 4"><path d="M2 2 L2 2" stroke="#000" stroke-width="2" stroke-linecap="round"/></svg>"##;
        let img = rasterize(doc, 4, 4).unwrap();
        assert_eq!(at(&img, 2, 2), 0xff000000);
        let butt = rasterize(r##"<svg viewBox="0 0 4 4"><path d="M2 2 L2 2" stroke="#000" stroke-width="2"/></svg>"##, 4, 4).unwrap();
        assert!(butt.pixels.iter().all(|p| *p == 0));
    }

    #[test]
    fn an_svg_file_decodes_to_a_premultiplied_image_at_its_intrinsic_size() {
        let doc = br##"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"><rect width="2" height="2" fill="#ff0000" fill-opacity="0.5"/></svg>"##;
        let img = decode_image(doc, 100).unwrap();
        assert_eq!((img.w, img.h), (4, 2));
        assert_eq!(img.px[0], crate::gfx::rgba(255, 0, 0, 128));
        assert_eq!(img.px[3], 0, "outside the rect stays clear");
    }

    #[test]
    fn a_large_svg_is_scaled_to_the_side_limit_and_garbage_is_refused() {
        let doc = br#"<svg viewBox="0 0 400 200"><rect width="400" height="200"/></svg>"#;
        let img = decode_image(doc, 40).unwrap();
        assert_eq!((img.w, img.h), (40, 20));
        assert_eq!(img.px[0], crate::gfx::rgba(0, 0, 0, 255));
        assert!(decode_image(doc, 0).is_none());
        assert!(decode_image(b"not an image", 40).is_none());
        assert!(decode_image(&[0xff, 0xfe, 0xfd], 40).is_none(), "not UTF-8");
        assert!(decode_image(b"<svg></svg>", 40).is_none(), "no size to draw at");
    }

    #[test]
    fn tokens_split_compact_numbers() {
        assert_eq!(
            tokens("M1.5.5-2").unwrap(),
            vec![Tok::Cmd('M'), Tok::Num(1.5), Tok::Num(0.5), Tok::Num(-2.0)]
        );
    }

    #[test]
    fn colors_parse_hex_and_names() {
        assert_eq!(color("#f00"), Some(0xff0000));
        assert_eq!(color("#00ff7f"), Some(0x00ff7f));
        assert_eq!(color("red"), Some(0xff0000));
        assert_eq!(color("url(#grad)"), None);
    }

    #[test]
    fn intrinsic_size_reads_width_or_viewbox() {
        assert_eq!(intrinsic_size(r#"<svg width="24px" height="12"></svg>"#), Some((24.0, 12.0)));
        assert_eq!(intrinsic_size(r#"<svg viewBox="0 0 24 12"></svg>"#), Some((24.0, 12.0)));
        assert_eq!(intrinsic_size("<svg"), None);
    }

    #[test]
    fn malformed_or_oversized_input_is_refused() {
        assert!(rasterize("<svg", 4, 4).is_none());
        assert!(rasterize(r#"<svg viewBox="0 0 2 2"></svg>"#, 0, 4).is_none());
        assert!(rasterize(r#"<svg viewBox="0 0 2 2"></svg>"#, 1 << 13, 1 << 13).is_none());
    }
}
