//! SVG reader: parses the static SVG subset that icons and images use and rasterises it to ARGB pixels.
//!
//! Supported: the root `width`, `height` and `viewBox`; the shapes `rect`, `circle`, `ellipse`, `polygon` and
//! `polyline`; and `path` with M, L, H, V, C, Q and Z (absolute and relative). Each shape is filled with its `fill`
//! colour (`#rgb`, `#rrggbb`, a few names, or `none`) at its `fill-opacity`; `fill` defaults to black. Gradients,
//! strokes, transforms, `use`, clipping, text and arc or smooth curve commands are not drawn yet: an element that
//! uses them is skipped, and its pixels stay clear. Defs-like containers (`defs`, `clipPath`, `mask`, ...) are not
//! drawn either. The viewBox maps onto the output with independent x and y scale (no aspect-ratio fitting).

use crate::bezier::{BezierPath, Point, WindingRule};

/// Largest raster accepted, in pixels, so an oversized request cannot exhaust memory.
const MAX_PIXELS: u64 = 16 * 1024 * 1024;

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
    let map = Map { ox, oy, sx: out_w as f32 / vw, sy: out_h as f32 / vh };
    let mut img = SvgImage { width: out_w, height: out_h, pixels: vec![0; total as usize] };
    for e in els.iter().filter(|e| e.name != "svg") {
        let Some((rgb, opacity)) = fill_of(e) else { continue };
        let Some(path) = shape_path(e, &map) else { continue };
        let mask = path.fill_mask(out_w as usize, out_h as usize, WindingRule::NonZero);
        paint(&mut img, &mask, rgb, opacity);
    }
    Some(img)
}

/// Maps user units to output pixels: `(x - ox) * sx`, `(y - oy) * sy`.
#[derive(Clone, Copy)]
struct Map {
    ox: f32,
    oy: f32,
    sx: f32,
    sy: f32,
}

impl Map {
    fn pt(&self, x: f32, y: f32) -> Point {
        ((x - self.ox) * self.sx, (y - self.oy) * self.sy)
    }

    fn len(&self, w: f32, h: f32) -> (f32, f32) {
        (w * self.sx, h * self.sy)
    }
}

struct Element {
    name: String,
    attrs: Vec<(String, String)>,
}

impl Element {
    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

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

/// Start tags in document order, minus everything inside a `NOT_DRAWN` subtree. Comments, processing instructions and
/// declarations are dropped.
fn elements(doc: &str) -> Vec<Element> {
    let mut out = Vec::new();
    let mut skip: Option<&str> = None;
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
            if skip == Some(close.trim()) {
                skip = None;
            }
            continue;
        }
        let self_closing = body.trim_end().ends_with('/');
        let (name, attrs) = split_tag(body);
        if skip.is_some() {
            continue;
        }
        if let Some(hidden) = NOT_DRAWN.iter().find(|n| **n == name) {
            if !self_closing {
                skip = Some(*hidden);
            }
            continue;
        }
        out.push(Element { name: name.to_string(), attrs });
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

/// The fill colour and opacity of an element, or `None` when it is not filled.
fn fill_of(e: &Element) -> Option<(u32, f32)> {
    let paint = e.attr("fill").unwrap_or("#000000").trim();
    if paint == "none" {
        return None;
    }
    let rgb = color(paint)?;
    let opacity = e.attr("fill-opacity").and_then(|s| s.trim().parse::<f32>().ok()).unwrap_or(1.0);
    Some((rgb, opacity.clamp(0.0, 1.0)))
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

/// Splits path data into command letters and numbers. A sign or a second decimal point starts a new number, so
/// `1.5.5-2` reads as 1.5, 0.5, -2. Separators are whitespace and commas.
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
            let start = i;
            if c == b'-' || c == b'+' {
                i += 1;
            }
            let mut digits = false;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
                digits = true;
            }
            if i < b.len() && b[i] == b'.' {
                i += 1;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                    digits = true;
                }
            }
            if !digits {
                return None;
            }
            if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
                let mut j = i + 1;
                if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
                    j += 1;
                }
                if j < b.len() && b[j].is_ascii_digit() {
                    while j < b.len() && b[j].is_ascii_digit() {
                        j += 1;
                    }
                    i = j;
                }
            }
            out.push(Tok::Num(d[start..i].parse::<f32>().ok()?));
        } else {
            return None;
        }
    }
    Some(out)
}

/// The path of a drawable element in output pixels, or `None` when it has no geometry we draw.
fn shape_path(e: &Element, m: &Map) -> Option<BezierPath> {
    let num = |k: &str| e.attr(k).and_then(length);
    let or0 = |k: &str| num(k).unwrap_or(0.0);
    match e.name.as_str() {
        "rect" => {
            let (w, h) = sized(num("width")?, num("height")?)?;
            let (x, y) = m.pt(or0("x"), or0("y"));
            let (w, h) = m.len(w, h);
            Some(BezierPath::rect(x, y, w, h))
        }
        "circle" => {
            let r = num("r")?;
            ellipse(m, or0("cx"), or0("cy"), r, r)
        }
        "ellipse" => ellipse(m, or0("cx"), or0("cy"), num("rx")?, num("ry")?),
        "polygon" | "polyline" => {
            let v = nums(e.attr("points")?)?;
            if v.len() < 4 || v.len() % 2 != 0 {
                return None;
            }
            let mut p = BezierPath::new();
            for (i, xy) in v.chunks(2).enumerate() {
                let pt = m.pt(xy[0], xy[1]);
                if i == 0 {
                    p.move_to(pt);
                } else {
                    p.line_to(pt);
                }
            }
            if e.name == "polygon" {
                p.close();
            }
            Some(p)
        }
        "path" => path_data(e.attr("d")?, m),
        _ => None,
    }
}

fn ellipse(m: &Map, cx: f32, cy: f32, rx: f32, ry: f32) -> Option<BezierPath> {
    let (rx, ry) = sized(rx, ry)?;
    let (x, y) = m.pt(cx - rx, cy - ry);
    let (w, h) = m.len(2.0 * rx, 2.0 * ry);
    Some(BezierPath::oval(x, y, w, h))
}

/// A point given in user units, made absolute when the command is relative.
fn abs(cur: Point, rel: bool, x: f32, y: f32) -> Point {
    if rel {
        (cur.0 + x, cur.1 + y)
    } else {
        (x, y)
    }
}

/// Path data with M, L, H, V, C, Q and Z. Arc and smooth-curve commands (A, S, T) make the path unsupported.
fn path_data(d: &str, m: &Map) -> Option<BezierPath> {
    let toks = tokens(d)?;
    let mut p = BezierPath::new();
    let mut cur: Point = (0.0, 0.0);
    let mut start: Point = cur;
    let mut cmd = ' ';
    let mut i = 0;
    while i < toks.len() {
        if let Tok::Cmd(c) = toks[i] {
            cmd = c;
            i += 1;
        } else if cmd == ' ' || cmd == 'Z' || cmd == 'z' {
            return None;
        }
        let upper = cmd.to_ascii_uppercase();
        let rel = cmd.is_ascii_lowercase();
        if upper == 'Z' {
            p.close();
            cur = start;
            continue;
        }
        let need = match upper {
            'M' | 'L' => 2,
            'H' | 'V' => 1,
            'Q' => 4,
            'C' => 6,
            _ => return None,
        };
        if i + need > toks.len() {
            return None;
        }
        let mut a = [0.0f32; 6];
        for (k, slot) in a.iter_mut().enumerate().take(need) {
            match toks[i + k] {
                Tok::Num(v) => *slot = v,
                Tok::Cmd(_) => return None,
            }
        }
        i += need;
        match upper {
            'M' => {
                let q = abs(cur, rel, a[0], a[1]);
                p.move_to(m.pt(q.0, q.1));
                cur = q;
                start = q;
                // Further coordinate pairs after a move are line-tos.
                cmd = if rel { 'l' } else { 'L' };
            }
            'L' => {
                let q = abs(cur, rel, a[0], a[1]);
                p.line_to(m.pt(q.0, q.1));
                cur = q;
            }
            'H' => {
                let q = (if rel { cur.0 + a[0] } else { a[0] }, cur.1);
                p.line_to(m.pt(q.0, q.1));
                cur = q;
            }
            'V' => {
                let q = (cur.0, if rel { cur.1 + a[0] } else { a[0] });
                p.line_to(m.pt(q.0, q.1));
                cur = q;
            }
            'C' => {
                let c1 = abs(cur, rel, a[0], a[1]);
                let c2 = abs(cur, rel, a[2], a[3]);
                let q = abs(cur, rel, a[4], a[5]);
                p.curve_to(m.pt(c1.0, c1.1), m.pt(c2.0, c2.1), m.pt(q.0, q.1));
                cur = q;
            }
            'Q' => {
                // A quadratic is raised to the equivalent cubic: each control point moves two thirds of the way.
                let qc = abs(cur, rel, a[0], a[1]);
                let q = abs(cur, rel, a[2], a[3]);
                let c1 = (cur.0 + 2.0 / 3.0 * (qc.0 - cur.0), cur.1 + 2.0 / 3.0 * (qc.1 - cur.1));
                let c2 = (q.0 + 2.0 / 3.0 * (qc.0 - q.0), q.1 + 2.0 / 3.0 * (qc.1 - q.1));
                p.curve_to(m.pt(c1.0, c1.1), m.pt(c2.0, c2.1), m.pt(q.0, q.1));
                cur = q;
            }
            _ => return None,
        }
    }
    Some(p)
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
    fn arc_paths_are_skipped_without_panicking() {
        let doc = r#"<svg viewBox="0 0 4 4"><path d="M0 0 A1 1 0 0 1 2 2"/></svg>"#;
        let img = rasterize(doc, 4, 4).unwrap();
        assert!(img.pixels.iter().all(|p| *p == 0));
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
