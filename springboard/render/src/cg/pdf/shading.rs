//! PDF shadings (types 1 to 7) resolved to the colour ramps, grids and
//! triangle lists a [`PdfSink`](super::sink::PdfSink) paints.

use super::color::ColorSpace;
use super::doc::Doc;
use super::function::Function;
use super::obj::{Dict, Obj, Stream};
use super::page::{Matrix, Rect};
use super::sink::{Rgb, Shading, ShadingKind, Vertex};

/// Control points and corner colours of the previously decoded patch.
type PatchPrev = ([(f64, f64); 16], [Rgb; 4]);

const RAMP: usize = 256;
const GRID: usize = 48;
const PATCH_STEPS: usize = 8;

fn nums(doc: &Doc, d: &Dict, key: &str) -> Vec<f64> {
    match doc.field(d, key) {
        Obj::Array(a) => a.iter().filter_map(|o| doc.get(o).num()).collect(),
        _ => vec![],
    }
}

/// Evaluates the shading function(s) at `t` (or `inputs`) and converts through the colour space.
fn color_at(cs: &ColorSpace, f: Option<&Function>, input: &[f32]) -> Rgb {
    match f {
        Some(f) => cs.to_rgb(&f.eval(input)),
        None => cs.to_rgb(input),
    }
}

struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn read(&mut self, n: usize) -> Option<u32> {
        if n == 0 {
            return Some(0);
        }
        if self.pos + n > self.d.len() * 8 {
            return None;
        }
        let mut v: u64 = 0;
        for _ in 0..n {
            let byte = self.d[self.pos / 8];
            v = v << 1 | ((byte >> (7 - self.pos % 8)) & 1) as u64;
            self.pos += 1;
        }
        Some(v as u32)
    }

    fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }
}

/// Reads a shading object (dictionary or stream). `base` is `Matrix` of a shading pattern when the shading
/// is used as one; the shading itself lives in the space `ctm` maps to the device.
pub fn parse(doc: &Doc, o: &Obj, res: Option<&Dict>) -> Option<Shading> {
    let obj = doc.get(o);
    let d = obj.dict()?;
    let ty = doc.field(d, "ShadingType").int()?;
    let cs = ColorSpace::parse(doc, d.get("ColorSpace")?, res)?;
    let func = d.get("Function").and_then(|f| Function::parse(doc, f));
    let background = match doc.field(d, "Background") {
        Obj::Array(a) => {
            let c: Vec<f32> = a.iter().filter_map(|x| doc.get(x).num()).map(|v| v as f32).collect();
            Some(cs.to_rgb(&c))
        }
        _ => None,
    };
    let bbox: Option<Rect> = {
        let b = nums(doc, d, "BBox");
        (b.len() >= 4).then(|| [b[0].min(b[2]), b[1].min(b[3]), b[0].max(b[2]), b[1].max(b[3])])
    };
    let kind = match ty {
        1 => {
            let dom = {
                let v = nums(doc, d, "Domain");
                if v.len() >= 4 {
                    [v[0], v[1], v[2], v[3]]
                } else {
                    [0.0, 1.0, 0.0, 1.0]
                }
            };
            let m = {
                let v = nums(doc, d, "Matrix");
                if v.len() >= 6 {
                    [v[0], v[1], v[2], v[3], v[4], v[5]]
                } else {
                    [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
                }
            };
            let f = func.as_ref()?;
            let mut colors = Vec::with_capacity(GRID * GRID);
            for j in 0..GRID {
                for i in 0..GRID {
                    let x = dom[0] + (dom[1] - dom[0]) * (i as f64 + 0.5) / GRID as f64;
                    let y = dom[2] + (dom[3] - dom[2]) * (j as f64 + 0.5) / GRID as f64;
                    colors.push(color_at(&cs, Some(f), &[x as f32, y as f32]));
                }
            }
            // unit square -> domain -> shading space
            let unit_to_dom: Matrix = [dom[1] - dom[0], 0.0, 0.0, dom[3] - dom[2], dom[0], dom[2]];
            ShadingKind::Grid { matrix: super::page::mul(&unit_to_dom, &m), w: GRID, h: GRID, colors }
        }
        2 | 3 => {
            let c = nums(doc, d, "Coords");
            let dom = {
                let v = nums(doc, d, "Domain");
                if v.len() >= 2 {
                    (v[0], v[1])
                } else {
                    (0.0, 1.0)
                }
            };
            let ext = match doc.field(d, "Extend") {
                Obj::Array(a) if a.len() >= 2 => (matches!(doc.get(&a[0]), Obj::Bool(true)), matches!(doc.get(&a[1]), Obj::Bool(true))),
                _ => (false, false),
            };
            let f = func.as_ref()?;
            let ramp: Vec<Rgb> = (0..RAMP)
                .map(|i| {
                    let t = dom.0 + (dom.1 - dom.0) * i as f64 / (RAMP - 1) as f64;
                    color_at(&cs, Some(f), &[t as f32])
                })
                .collect();
            if ty == 2 {
                if c.len() < 4 {
                    return None;
                }
                ShadingKind::Axial { x0: c[0], y0: c[1], x1: c[2], y1: c[3], extend: ext, ramp }
            } else {
                if c.len() < 6 {
                    return None;
                }
                ShadingKind::Radial { x0: c[0], y0: c[1], r0: c[2], x1: c[3], y1: c[4], r1: c[5], extend: ext, ramp }
            }
        }
        4..=7 => {
            let Obj::Stream(s) = &obj else { return None };
            ShadingKind::Triangles(mesh(doc, s, ty, &cs, func.as_ref())?)
        }
        _ => return None,
    };
    Some(Shading { kind, background, bbox })
}

struct MeshParams {
    bpcoord: usize,
    bpcomp: usize,
    bpflag: usize,
    decode: Vec<f64>,
    ncomp: usize,
}

impl MeshParams {
    fn point(&self, b: &mut Bits) -> Option<(f64, f64)> {
        let max = if self.bpcoord >= 32 { u32::MAX as f64 } else { ((1u64 << self.bpcoord) - 1) as f64 };
        let x = b.read(self.bpcoord)? as f64 / max;
        let y = b.read(self.bpcoord)? as f64 / max;
        Some((self.decode[0] + x * (self.decode[1] - self.decode[0]), self.decode[2] + y * (self.decode[3] - self.decode[2])))
    }

    fn color(&self, b: &mut Bits, cs: &ColorSpace, f: Option<&Function>) -> Option<Rgb> {
        let max = ((1u64 << self.bpcomp) - 1) as f64;
        let mut comps = Vec::with_capacity(self.ncomp);
        for k in 0..self.ncomp {
            let v = b.read(self.bpcomp)? as f64 / max;
            let (lo, hi) = (self.decode[4 + 2 * k], self.decode[5 + 2 * k]);
            comps.push((lo + v * (hi - lo)) as f32);
        }
        Some(color_at(cs, f, &comps))
    }
}

fn mesh(doc: &Doc, s: &Stream, ty: i64, cs: &ColorSpace, f: Option<&Function>) -> Option<Vec<[Vertex; 3]>> {
    let d = &s.dict;
    let data = doc.decode(s).0;
    let ncomp = if f.is_some() { 1 } else { cs.n() };
    let mut decode = nums(doc, d, "Decode");
    if decode.len() < 4 + 2 * ncomp {
        return None;
    }
    decode.truncate(4 + 2 * ncomp);
    let p = MeshParams {
        bpcoord: doc.field(d, "BitsPerCoordinate").int()?.clamp(1, 32) as usize,
        bpcomp: doc.field(d, "BitsPerComponent").int()?.clamp(1, 16) as usize,
        bpflag: doc.field(d, "BitsPerFlag").int().unwrap_or(8).clamp(0, 8) as usize,
        decode,
        ncomp,
    };
    let mut b = Bits { d: &data, pos: 0 };
    let mut tris: Vec<[Vertex; 3]> = vec![];
    let vertex = |pt: (f64, f64), c: Rgb| Vertex { x: pt.0, y: pt.1, color: c };
    match ty {
        4 => {
            let mut va: Option<Vertex> = None;
            let mut vb: Option<Vertex> = None;
            let mut vc: Option<Vertex> = None;
            while tris.len() < 400_000 {
                let Some(flag) = b.read(p.bpflag) else { break };
                let (Some(pt), Some(col)) = (p.point(&mut b), p.color(&mut b, cs, f)) else { break };
                b.align();
                let v = vertex(pt, col);
                match flag {
                    0 => {
                        // two more vertices follow, their flags ignored
                        let mut rest = vec![];
                        for _ in 0..2 {
                            let Some(_) = b.read(p.bpflag) else { break };
                            let (Some(pt), Some(col)) = (p.point(&mut b), p.color(&mut b, cs, f)) else { break };
                            b.align();
                            rest.push(vertex(pt, col));
                        }
                        if rest.len() < 2 {
                            break;
                        }
                        va = Some(v);
                        vb = Some(rest[0]);
                        vc = Some(rest[1]);
                    }
                    1 => {
                        va = vb;
                        vb = vc;
                        vc = Some(v);
                    }
                    _ => {
                        vb = vc;
                        vc = Some(v);
                    }
                }
                if let (Some(a), Some(bb), Some(c)) = (va, vb, vc) {
                    tris.push([a, bb, c]);
                }
            }
        }
        5 => {
            let per_row = doc.field(d, "VerticesPerRow").int()?.max(2) as usize;
            let mut rows: Vec<Vec<Vertex>> = vec![];
            'outer: loop {
                let mut row = Vec::with_capacity(per_row);
                for _ in 0..per_row {
                    let (Some(pt), Some(col)) = (p.point(&mut b), p.color(&mut b, cs, f)) else { break 'outer };
                    row.push(vertex(pt, col));
                }
                rows.push(row);
                if rows.len() > 4000 {
                    break;
                }
            }
            for w in rows.windows(2) {
                for i in 0..per_row - 1 {
                    tris.push([w[0][i], w[0][i + 1], w[1][i]]);
                    tris.push([w[0][i + 1], w[1][i + 1], w[1][i]]);
                }
            }
        }
        _ => {
            let npts = if ty == 6 { 12 } else { 16 };
            let mut prev: Option<PatchPrev> = None;
            let mut patches = 0;
            while patches < 20_000 {
                let Some(flag) = b.read(p.bpflag) else { break };
                let mut pts = [(0.0, 0.0); 16];
                let mut cols = [[0.0f32; 3]; 4];
                let (skip_pts, skip_cols) = if flag == 0 { (0, 0) } else { (4, 2) };
                if flag != 0 {
                    let Some((pp, pc)) = &prev else { break };
                    let (idx, cidx): ([usize; 4], [usize; 2]) = match flag {
                        1 => ([3, 4, 5, 6], [1, 2]),
                        2 => ([6, 7, 8, 9], [2, 3]),
                        _ => ([9, 10, 11, 0], [3, 0]),
                    };
                    for k in 0..4 {
                        pts[k] = pp[idx[k]];
                    }
                    cols[0] = pc[cidx[0]];
                    cols[1] = pc[cidx[1]];
                }
                let mut ok = true;
                for slot in pts.iter_mut().take(npts).skip(skip_pts) {
                    match p.point(&mut b) {
                        Some(pt) => *slot = pt,
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok {
                    break;
                }
                for slot in cols.iter_mut().skip(skip_cols) {
                    match p.color(&mut b, cs, f) {
                        Some(c) => *slot = c,
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok {
                    break;
                }
                b.align();
                patches += 1;
                tessellate(&pts, &cols, ty == 7, &mut tris);
                prev = Some((pts, cols));
            }
        }
    }
    (!tris.is_empty()).then_some(tris)
}

fn bern(t: f64) -> [f64; 4] {
    let u = 1.0 - t;
    [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t]
}

/// Evaluates a Coons or tensor-product patch on a grid and appends its triangles.
fn tessellate(pts: &[(f64, f64); 16], cols: &[Rgb; 4], tensor: bool, out: &mut Vec<[Vertex; 3]>) {
    let mut g = [[(0.0, 0.0); 4]; 4];
    g[0][0] = pts[0];
    g[0][1] = pts[1];
    g[0][2] = pts[2];
    g[0][3] = pts[3];
    g[1][3] = pts[4];
    g[2][3] = pts[5];
    g[3][3] = pts[6];
    g[3][2] = pts[7];
    g[3][1] = pts[8];
    g[3][0] = pts[9];
    g[2][0] = pts[10];
    g[1][0] = pts[11];
    if tensor {
        g[1][1] = pts[12];
        g[1][2] = pts[13];
        g[2][2] = pts[14];
        g[2][1] = pts[15];
    } else {
        let f = |a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64), e: (f64, f64), f2: (f64, f64), g2: (f64, f64), h: (f64, f64)| -> (f64, f64) {
            (
                (-4.0 * a.0 + 6.0 * (b.0 + c.0) - 2.0 * (d.0 + e.0) + 3.0 * (f2.0 + g2.0) - h.0) / 9.0,
                (-4.0 * a.1 + 6.0 * (b.1 + c.1) - 2.0 * (d.1 + e.1) + 3.0 * (f2.1 + g2.1) - h.1) / 9.0,
            )
        };
        g[1][1] = f(g[0][0], g[0][1], g[1][0], g[0][3], g[3][0], g[3][1], g[1][3], g[3][3]);
        g[1][2] = f(g[0][3], g[0][2], g[1][3], g[0][0], g[3][3], g[3][2], g[1][0], g[3][0]);
        g[2][1] = f(g[3][0], g[3][1], g[2][0], g[3][3], g[0][0], g[0][1], g[2][3], g[0][3]);
        g[2][2] = f(g[3][3], g[3][2], g[2][3], g[3][0], g[0][3], g[0][2], g[2][0], g[0][0]);
    }
    let n = PATCH_STEPS;
    let mut grid: Vec<Vertex> = Vec::with_capacity((n + 1) * (n + 1));
    for iu in 0..=n {
        let u = iu as f64 / n as f64;
        let bu = bern(u);
        for iv in 0..=n {
            let v = iv as f64 / n as f64;
            let bv = bern(v);
            let (mut x, mut y) = (0.0, 0.0);
            for i in 0..4 {
                for j in 0..4 {
                    let w = bu[i] * bv[j];
                    x += w * g[i][j].0;
                    y += w * g[i][j].1;
                }
            }
            // corners: c0 at (u0,v0), c1 at (u0,v1), c2 at (u1,v1), c3 at (u1,v0)
            let mut color = [0.0f32; 3];
            for (k, c) in color.iter_mut().enumerate() {
                let a = cols[0][k] as f64 * (1.0 - u) * (1.0 - v) + cols[1][k] as f64 * (1.0 - u) * v + cols[2][k] as f64 * u * v + cols[3][k] as f64 * u * (1.0 - v);
                *c = a as f32;
            }
            grid.push(Vertex { x, y, color });
        }
    }
    for iu in 0..n {
        for iv in 0..n {
            let a = grid[iu * (n + 1) + iv];
            let b = grid[iu * (n + 1) + iv + 1];
            let c = grid[(iu + 1) * (n + 1) + iv];
            let d = grid[(iu + 1) * (n + 1) + iv + 1];
            out.push([a, b, c]);
            out.push([b, d, c]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg::pdf::doc::tests::tiny_pdf;
    use std::rc::Rc;

    fn doc() -> Doc {
        Doc::open(tiny_pdf("")).unwrap()
    }

    fn arr(v: &[f64]) -> Obj {
        Obj::Array(v.iter().map(|x| Obj::Real(*x)).collect())
    }

    fn dict(entries: Vec<(&str, Obj)>) -> Dict {
        entries.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    fn red_to_blue() -> Obj {
        Obj::Dict(dict(vec![("FunctionType", Obj::Int(2)), ("Domain", arr(&[0.0, 1.0])), ("C0", arr(&[1.0, 0.0, 0.0])), ("C1", arr(&[0.0, 0.0, 1.0])), ("N", Obj::Int(1))]))
    }

    #[test]
    fn axial_shading_builds_a_ramp() {
        let sh = Obj::Dict(dict(vec![
            ("ShadingType", Obj::Int(2)),
            ("ColorSpace", Obj::Name("DeviceRGB".into())),
            ("Coords", arr(&[0.0, 0.0, 100.0, 0.0])),
            ("Function", red_to_blue()),
            ("Extend", Obj::Array(vec![Obj::Bool(true), Obj::Bool(false)])),
        ]));
        let s = parse(&doc(), &sh, None).unwrap();
        match s.kind {
            ShadingKind::Axial { x1, extend, ramp, .. } => {
                assert_eq!(x1, 100.0);
                assert_eq!(extend, (true, false));
                assert_eq!(ramp.len(), RAMP);
                assert_eq!(ramp[0], [1.0, 0.0, 0.0]);
                assert_eq!(ramp[RAMP - 1], [0.0, 0.0, 1.0]);
            }
            _ => panic!("axial"),
        }
    }

    #[test]
    fn radial_needs_six_coords() {
        let bad = Obj::Dict(dict(vec![("ShadingType", Obj::Int(3)), ("ColorSpace", Obj::Name("DeviceRGB".into())), ("Coords", arr(&[0.0, 0.0, 1.0])), ("Function", red_to_blue())]));
        assert!(parse(&doc(), &bad, None).is_none());
    }

    #[test]
    fn free_form_mesh_makes_a_triangle() {
        // 8-bit flag, 8-bit coordinates, 8-bit components, RGB; three vertices with flag 0
        let mut data = vec![];
        for (x, y, c) in [(0u8, 0u8, [255u8, 0, 0]), (255, 0, [0, 255, 0]), (0, 255, [0, 0, 255])] {
            data.push(0);
            data.push(x);
            data.push(y);
            data.extend_from_slice(&c);
        }
        let d = dict(vec![
            ("ShadingType", Obj::Int(4)),
            ("ColorSpace", Obj::Name("DeviceRGB".into())),
            ("BitsPerCoordinate", Obj::Int(8)),
            ("BitsPerComponent", Obj::Int(8)),
            ("BitsPerFlag", Obj::Int(8)),
            ("Decode", arr(&[0.0, 10.0, 0.0, 10.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0])),
        ]);
        let sh = Obj::Stream(Rc::new(Stream { dict: d, data }));
        let s = parse(&doc(), &sh, None).unwrap();
        let ShadingKind::Triangles(t) = s.kind else { panic!("mesh") };
        assert_eq!(t.len(), 1);
        assert_eq!((t[0][1].x, t[0][2].y), (10.0, 10.0));
        assert_eq!(t[0][0].color, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn coons_patch_corners_land_on_the_boundary() {
        let pts: [(f64, f64); 16] = {
            let mut p = [(0.0, 0.0); 16];
            // a unit square boundary walked as the stream orders it
            let b = [(0.0, 0.0), (0.0, 1.0 / 3.0), (0.0, 2.0 / 3.0), (0.0, 1.0), (1.0 / 3.0, 1.0), (2.0 / 3.0, 1.0), (1.0, 1.0), (1.0, 2.0 / 3.0), (1.0, 1.0 / 3.0), (1.0, 0.0), (2.0 / 3.0, 0.0), (1.0 / 3.0, 0.0)];
            p[..12].copy_from_slice(&b);
            p
        };
        let cols = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 1.0]];
        let mut out = vec![];
        tessellate(&pts, &cols, false, &mut out);
        assert_eq!(out.len(), 2 * PATCH_STEPS * PATCH_STEPS);
        let first = out[0][0];
        assert!((first.x - 0.0).abs() < 1e-9 && (first.y - 0.0).abs() < 1e-9);
        assert_eq!(first.color, [1.0, 0.0, 0.0]);
        // the interior of a flat square patch stays inside the square
        for t in &out {
            for v in t {
                assert!(v.x >= -1e-9 && v.x <= 1.0 + 1e-9 && v.y >= -1e-9 && v.y <= 1.0 + 1e-9);
            }
        }
    }
}
