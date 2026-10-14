//! Layered cursor art: every [`Kind`] is drawn from vector paths on a 24 point grid into two alpha masks, the
//! fill and the outline around it. The masks never overlap, so the pointer outline and fill colours
//! (Accessibility > Display > Pointer) are applied separately by [`CursorArt::compose`].

use crate::cursor::Kind;

/// Side of the art in points; the bitmap is `SIZE * scale` pixels square.
pub const SIZE: u32 = 24;
const SUPERSAMPLE: u32 = 4;
/// Outline thickness in points.
const OUTLINE: f32 = 1.0;

type P = (f32, f32);

/// Drawing steps in order: `Add` covers the polygon, `Sub` punches it out of what is covered so far.
enum Op {
    Add(Vec<P>),
    Sub(Vec<P>),
}

fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<P> {
    vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
}

fn circle(cx: f32, cy: f32, r: f32) -> Vec<P> {
    (0..32)
        .map(|i| {
            let a = i as f32 / 32.0 * std::f32::consts::TAU;
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

/// A line of thickness `t` from `a` to `b`.
fn line(a: P, b: P, t: f32) -> Vec<P> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let n = (dx * dx + dy * dy).sqrt().max(f32::EPSILON);
    let (px, py) = (-dy / n * t / 2.0, dx / n * t / 2.0);
    vec![(a.0 + px, a.1 + py), (b.0 + px, b.1 + py), (b.0 - px, b.1 - py), (a.0 - px, a.1 - py)]
}

/// An arrow head with its point at `tip`, pointing along the unit vector `dir`.
fn head(tip: P, dir: P, len: f32, half_w: f32) -> Vec<P> {
    let base = (tip.0 - dir.0 * len, tip.1 - dir.1 * len);
    let perp = (-dir.1, dir.0);
    vec![tip, (base.0 + perp.0 * half_w, base.1 + perp.1 * half_w), (base.0 - perp.0 * half_w, base.1 - perp.1 * half_w)]
}

fn unit(deg: f32) -> P {
    let a = deg.to_radians();
    (a.cos(), a.sin())
}

fn at(c: P, dir: P, d: f32) -> P {
    (c.0 + dir.0 * d, c.1 + dir.1 * d)
}

/// A shaft with an arrow head at both ends, along `deg` degrees (0 = right, 90 = down), through the centre.
fn double_arrow(deg: f32, reach: f32) -> Vec<Op> {
    let (c, d) = ((12.0, 12.0), unit(deg));
    let back = (-d.0, -d.1);
    vec![
        Op::Add(line(at(c, back, reach - 3.0), at(c, d, reach - 3.0), 2.4)),
        Op::Add(head(at(c, d, reach), d, 5.0, 4.0)),
        Op::Add(head(at(c, back, reach), back, 5.0, 4.0)),
    ]
}

/// A shaft ending in a head at the tip and a stop bar across it, along `deg`.
fn stop_arrow(deg: f32) -> Vec<Op> {
    let (c, d) = ((12.0, 12.0), unit(deg));
    let perp = (-d.1, d.0);
    let bar_c = at(c, d, 9.5);
    vec![
        Op::Add(line(at(c, d, -6.0), at(c, d, 5.0), 2.4)),
        Op::Add(head(at(c, d, 8.0), d, 5.0, 4.0)),
        Op::Add(line(at(bar_c, perp, -5.0), at(bar_c, perp, 5.0), 2.0)),
    ]
}

fn arrow_pointer() -> Vec<P> {
    vec![(4.0, 3.0), (4.0, 19.0), (8.2, 15.2), (11.0, 21.0), (13.6, 19.8), (10.9, 14.2), (16.5, 14.2)]
}

fn ring(cx: f32, cy: f32, r: f32, w: f32) -> Vec<Op> {
    vec![Op::Add(circle(cx, cy, r)), Op::Sub(circle(cx, cy, r - w))]
}

fn hand(fingers: &[f32], palm_top: f32) -> Vec<Op> {
    let mut ops = vec![Op::Add(rect(6.0, palm_top, 18.0, 20.0)), Op::Add(rect(8.0, 19.0, 16.0, 22.0))];
    for (i, &top) in fingers.iter().enumerate() {
        let x = 6.0 + i as f32 * 3.0;
        ops.push(Op::Add(rect(x, top, x + 2.6, palm_top + 1.0)));
    }
    ops
}

fn magnifier(sign: Vec<Vec<P>>) -> Vec<Op> {
    let mut ops = ring(10.0, 10.0, 7.0, 2.0);
    ops.push(Op::Add(line((15.0, 15.0), (21.0, 21.0), 3.0)));
    ops.extend(sign.into_iter().map(Op::Add));
    ops
}

/// The shape of `kind` and its hotspot, in points.
fn design(kind: Kind) -> (Vec<Op>, P) {
    let pointer = |extra: Vec<Op>| {
        let mut ops = vec![Op::Add(arrow_pointer())];
        ops.extend(extra);
        (ops, (4.0, 3.0))
    };
    let centred = |ops: Vec<Op>| (ops, (12.0, 12.0));
    match kind {
        Kind::Arrow => pointer(vec![]),
        Kind::IBeam => centred(vec![Op::Add(rect(11.0, 4.0, 13.0, 20.0)), Op::Add(rect(8.0, 3.0, 16.0, 5.0)), Op::Add(rect(8.0, 19.0, 16.0, 21.0))]),
        Kind::VerticalIBeam => centred(vec![Op::Add(rect(4.0, 11.0, 20.0, 13.0)), Op::Add(rect(3.0, 8.0, 5.0, 16.0)), Op::Add(rect(19.0, 8.0, 21.0, 16.0))]),
        Kind::Crosshair => centred(vec![Op::Add(rect(11.0, 3.0, 13.0, 21.0)), Op::Add(rect(3.0, 11.0, 21.0, 13.0))]),
        Kind::OpenHand => (hand(&[5.0, 3.0, 4.0, 6.0], 9.0), (12.0, 12.0)),
        Kind::ClosedHand => (hand(&[9.0, 8.0, 8.5, 10.0], 10.0), (12.0, 13.0)),
        Kind::PointingHand => (hand(&[3.0, 11.0, 11.0, 12.0], 11.0), (8.0, 4.0)),
        Kind::ContextualMenu => pointer(vec![Op::Add(rect(15.0, 15.0, 22.0, 21.0))]),
        Kind::OperationNotAllowed => {
            let mut ops = ring(12.0, 12.0, 9.0, 2.5);
            ops.push(Op::Add(line((6.0, 6.0), (18.0, 18.0), 2.5)));
            centred(ops)
        }
        Kind::DragCopy => pointer(vec![Op::Add(rect(18.0, 13.0, 20.0, 21.0)), Op::Add(rect(15.0, 16.0, 23.0, 18.0))]),
        Kind::DragLink => pointer(vec![Op::Add(head((22.0, 18.0), (1.0, 0.0), 5.0, 3.5)), Op::Add(rect(14.0, 17.0, 18.0, 19.0))]),
        Kind::DisappearingItem => pointer(vec![Op::Add(circle(18.0, 18.0, 4.0)), Op::Sub(circle(18.0, 18.0, 1.5))]),
        Kind::ZoomIn => (magnifier(vec![rect(6.0, 9.0, 14.0, 11.0), rect(9.0, 6.0, 11.0, 14.0)]), (10.0, 10.0)),
        Kind::ZoomOut => (magnifier(vec![rect(6.0, 9.0, 14.0, 11.0)]), (10.0, 10.0)),
        Kind::ResizeLeftRight => centred(double_arrow(0.0, 9.5)),
        Kind::ResizeUpDown => centred(double_arrow(90.0, 9.5)),
        Kind::ResizeNorthEastSouthWest => centred(double_arrow(-45.0, 9.5)),
        Kind::ResizeNorthWestSouthEast => centred(double_arrow(45.0, 9.5)),
        Kind::ResizeLeft => centred(stop_arrow(180.0)),
        Kind::ResizeRight => centred(stop_arrow(0.0)),
        Kind::ResizeUp => centred(stop_arrow(-90.0)),
        Kind::ResizeDown => centred(stop_arrow(90.0)),
        Kind::ColumnResize => {
            let mut ops = double_arrow(0.0, 9.5);
            ops.push(Op::Add(rect(11.0, 3.0, 13.0, 21.0)));
            centred(ops)
        }
        Kind::RowResize => {
            let mut ops = double_arrow(90.0, 9.5);
            ops.push(Op::Add(rect(3.0, 11.0, 21.0, 13.0)));
            centred(ops)
        }
        Kind::Move => {
            let mut ops = double_arrow(0.0, 10.0);
            ops.extend(double_arrow(90.0, 10.0));
            centred(ops)
        }
        Kind::Rotate => {
            let mut ops = ring(12.0, 12.0, 8.5, 2.4);
            ops.push(Op::Sub(vec![(12.0, 12.0), (24.0, 2.0), (24.0, 12.0)]));
            ops.push(Op::Add(head((20.5, 11.0), (0.0, 1.0), 5.0, 4.0)));
            centred(ops)
        }
        Kind::ScreenshotWindow => centred(vec![
            Op::Add(rect(3.0, 5.0, 21.0, 19.0)),
            Op::Sub(rect(5.0, 7.0, 19.0, 17.0)),
            Op::Add(rect(11.0, 9.0, 13.0, 15.0)),
            Op::Add(rect(9.0, 11.0, 15.0, 13.0)),
        ]),
    }
}

fn inside(poly: &[P], x: f32, y: f32) -> bool {
    let mut odd = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a.1 > y) != (b.1 > y) && x < (b.0 - a.0) * (y - a.1) / (b.1 - a.1) + a.0 {
            odd = !odd;
        }
        j = i;
    }
    odd
}

fn covered(ops: &[Op], x: f32, y: f32) -> bool {
    let mut on = false;
    for op in ops {
        match op {
            Op::Add(p) => on |= inside(p, x, y),
            Op::Sub(p) => on &= !inside(p, x, y),
        }
    }
    on
}

/// Cursor art: two alpha masks (`size * size` bytes each, row-major, 255 = covered) and the hotspot in pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorArt {
    pub size: u32,
    pub scale: u32,
    pub hotspot: (u32, u32),
    pub fill: Vec<u8>,
    pub outline: Vec<u8>,
}

/// An RGBA colour, straight alpha.
pub type Rgba = [u8; 4];

/// The pointer colours of Accessibility > Display > Pointer: white outline, black fill by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointerColors {
    pub outline: Rgba,
    pub fill: Rgba,
}

impl Default for PointerColors {
    fn default() -> Self {
        PointerColors { outline: [255, 255, 255, 255], fill: [0, 0, 0, 255] }
    }
}

/// The art of `kind` at `scale` (1 or 2; anything else is clamped into that range).
pub fn art(kind: Kind, scale: u32) -> CursorArt {
    let scale = scale.clamp(1, 2);
    let size = SIZE * scale;
    let (ops, (hx, hy)) = design(kind);
    let n = size as usize;
    let mut fill = vec![0u8; n * n];
    let sub = SUPERSAMPLE;
    for py in 0..n {
        for px in 0..n {
            let mut hits = 0;
            for sy in 0..sub {
                for sx in 0..sub {
                    let x = (px as f32 + (sx as f32 + 0.5) / sub as f32) / scale as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / sub as f32) / scale as f32;
                    if covered(&ops, x, y) {
                        hits += 1;
                    }
                }
            }
            fill[py * n + px] = (hits * 255 / (sub * sub)) as u8;
        }
    }
    let outline = outline_of(&fill, n, OUTLINE * scale as f32);
    let hotspot = (((hx * scale as f32) as u32).min(size - 1), ((hy * scale as f32) as u32).min(size - 1));
    CursorArt { size, scale, hotspot, fill, outline }
}

/// The fill grown by `radius` pixels, with the fill itself taken out so the layers do not overlap.
fn outline_of(fill: &[u8], n: usize, radius: f32) -> Vec<u8> {
    let r = radius.ceil() as i32;
    let mut out = vec![0u8; n * n];
    for y in 0..n as i32 {
        for x in 0..n as i32 {
            let mut grown = 0u8;
            for dy in -r..=r {
                for dx in -r..=r {
                    if ((dx * dx + dy * dy) as f32).sqrt() > radius {
                        continue;
                    }
                    let (nx, ny) = (x + dx, y + dy);
                    if nx >= 0 && ny >= 0 && (nx as usize) < n && (ny as usize) < n {
                        grown = grown.max(fill[ny as usize * n + nx as usize]);
                    }
                }
            }
            let own = fill[y as usize * n + x as usize] as u32;
            out[y as usize * n + x as usize] = (grown as u32 * (255 - own) / 255) as u8;
        }
    }
    out
}

impl CursorArt {
    /// The bitmap (RGBA, straight alpha): the fill colour over the outline colour, each scaled by its mask.
    pub fn compose(&self, outline: Rgba, fill: Rgba) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.fill.len() * 4);
        for (&o, &f) in self.outline.iter().zip(&self.fill) {
            let oa = o as f32 / 255.0 * outline[3] as f32 / 255.0;
            let fa = f as f32 / 255.0 * fill[3] as f32 / 255.0;
            let a = fa + oa * (1.0 - fa);
            for c in 0..3 {
                let v = if a > 0.0 { (fill[c] as f32 * fa + outline[c] as f32 * oa * (1.0 - fa)) / a } else { 0.0 };
                out.push(v.round() as u8);
            }
            out.push((a * 255.0).round() as u8);
        }
        out
    }

    pub fn compose_colors(&self, colors: PointerColors) -> Vec<u8> {
        self.compose(colors.outline, colors.fill)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_separate_non_empty_layers_and_a_hotspot_inside() {
        for &k in Kind::ALL {
            for scale in [1, 2] {
                let a = art(k, scale);
                let n = SIZE * scale;
                assert_eq!((a.size, a.fill.len(), a.outline.len()), (n, (n * n) as usize, (n * n) as usize), "{k:?}");
                assert!(a.fill.iter().any(|&v| v > 0), "{k:?} fill is empty");
                assert!(a.outline.iter().any(|&v| v > 0), "{k:?} outline is empty");
                assert!(a.hotspot.0 < n && a.hotspot.1 < n, "{k:?} hotspot {:?}", a.hotspot);
            }
        }
    }

    #[test]
    fn the_layers_do_not_overlap_where_the_fill_is_solid() {
        for &k in Kind::ALL {
            let a = art(k, 2);
            for (o, f) in a.outline.iter().zip(&a.fill) {
                assert!(*f < 255 || *o == 0, "{k:?} outline under solid fill");
            }
        }
    }

    #[test]
    fn two_x_art_is_twice_the_size_with_a_doubled_hotspot() {
        let (a, b) = (art(Kind::Arrow, 1), art(Kind::Arrow, 2));
        assert_eq!(b.size, a.size * 2);
        assert_eq!(b.hotspot, (a.hotspot.0 * 2, a.hotspot.1 * 2));
        assert_eq!(art(Kind::Arrow, 9), b, "scale clamps to 2");
    }

    #[test]
    fn recolouring_the_fill_leaves_outline_pixels_alone() {
        let a = art(Kind::ResizeLeftRight, 2);
        let red = a.compose([255, 255, 255, 255], [255, 0, 0, 255]);
        let blue = a.compose([255, 255, 255, 255], [0, 0, 255, 255]);
        let mut outline_pixels = 0;
        for i in 0..a.fill.len() {
            if a.fill[i] == 0 && a.outline[i] > 0 {
                outline_pixels += 1;
                assert_eq!(red[i * 4..i * 4 + 4], blue[i * 4..i * 4 + 4]);
            }
        }
        assert!(outline_pixels > 0);
        let solid = a.fill.iter().position(|&v| v == 255).unwrap();
        assert_eq!(red[solid * 4..solid * 4 + 4], [255, 0, 0, 255]);
        assert_eq!(blue[solid * 4..solid * 4 + 4], [0, 0, 255, 255]);
    }

    #[test]
    fn recolouring_the_outline_leaves_fill_pixels_alone() {
        let a = art(Kind::Move, 1);
        let p = a.compose([255, 255, 255, 255], [0, 0, 0, 255]);
        let q = a.compose([0, 255, 0, 255], [0, 0, 0, 255]);
        let solid = a.fill.iter().position(|&v| v == 255).unwrap();
        assert_eq!(p[solid * 4..solid * 4 + 4], q[solid * 4..solid * 4 + 4]);
        let edge = (0..a.fill.len()).find(|&i| a.fill[i] == 0 && a.outline[i] == 255).unwrap();
        assert_eq!(q[edge * 4..edge * 4 + 4], [0, 255, 0, 255]);
    }

    #[test]
    fn the_default_pointer_is_a_black_fill_with_a_white_outline() {
        let a = art(Kind::Arrow, 1);
        let px = a.compose_colors(PointerColors::default());
        let solid = a.fill.iter().position(|&v| v == 255).unwrap();
        assert_eq!(px[solid * 4..solid * 4 + 4], [0, 0, 0, 255]);
    }

    #[test]
    fn shapes_differ_from_each_other() {
        let mut seen = std::collections::HashSet::new();
        for &k in Kind::ALL {
            assert!(seen.insert(art(k, 1).fill), "{k:?} repeats another shape");
        }
    }

    #[test]
    fn the_not_allowed_ring_has_a_hole_off_the_slash() {
        let a = art(Kind::OperationNotAllowed, 1);
        let n = a.size as usize;
        assert_eq!(a.fill[(n / 2 - 3) * n + n / 2 + 3], 0);
    }
}
