//! Text rendering: cmap lookup, TrueType (glyf) and CFF (Type 2) outlines,
//! curve flattening and an exact-area coverage rasteriser. TrueType
//! variable fonts render any instance (`wght`, `opsz`, ...) through `gvar`.

use crate::cff::{Cff, Sink};
use crate::gfx::{pack, scale, Image};
use crate::gvar::{self, Axis, Gvar};
use crate::shape_ot::{self, Features, OtLayout, ShapeOptions};

/// One glyph placed by the shaper: advance and offsets in font units (offset y is up).
struct PlacedGlyph {
    gid: u16,
    advance: f32,
    dx: f32,
    dy: f32,
}

/// A variation setting: axis tag and value in user units, e.g. (b"wght", 590).
pub type Var = ([u8; 4], f32);

pub struct Font<'a> {
    data: &'a [u8],
    /// CFF outlines (OpenType `OTTO`); otherwise glyf/loca are used
    cff: Option<Cff<'a>>,
    axes: Vec<Axis>,
    avar: Option<&'a [u8]>,
    gvar: Option<Gvar<'a>>,
    glyf: usize,
    loca: usize,
    hmtx: usize,
    loca_long: bool,
    num_hmetrics: u16,
    num_glyphs: u16,
    cmap: usize,
    cmap_format: u16,
    pub upem: f32,
    pub ascent: f32,
    pub descent: f32,
    /// Font units, read once in `parse`; zero when the table is absent.
    line_gap: f32,
    x_height: f32,
    cap_height: f32,
    underline_position: f32,
    underline_thickness: f32,
    cache: GlyphCache,
    ot: std::sync::OnceLock<crate::shape_ot::OtLayout<'a>>,
}

/// Vertical metrics of a font at one size, in px (`NativeFontMetrics`). Descent is positive, as the platform
/// reports it; the other values keep their sign from the font (the underline position is usually negative).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub leading: f32,
    pub cap_height: f32,
    pub x_height: f32,
    pub underline_position: f32,
    pub underline_thickness: f32,
}

impl LineMetrics {
    /// Line height with no extra spacing: ascent, descent and leading.
    pub fn line_height(&self) -> f32 {
        self.ascent + self.descent + self.leading
    }
}

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(o..o + 2)?.try_into().ok()?))
}
fn i16_at(d: &[u8], o: usize) -> Option<i16> {
    u16_at(d, o).map(|v| v as i16)
}
fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Pt {
    x: f32,
    y: f32,
}

fn lerp(a: Pt, b: Pt, t: f32) -> Pt {
    Pt { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t }
}

/// Affine transform applied to glyph-space points (composite glyphs nest them).
#[derive(Clone, Copy)]
struct Affine([f32; 6]);

impl Affine {
    fn apply(&self, x: f32, y: f32) -> Pt {
        let m = self.0;
        Pt { x: m[0] * x + m[2] * y + m[4], y: m[1] * x + m[3] * y + m[5] }
    }
    fn then(&self, outer: &Affine) -> Affine {
        let (a, b) = (self.0, outer.0);
        Affine([
            b[0] * a[0] + b[2] * a[1],
            b[1] * a[0] + b[3] * a[1],
            b[0] * a[2] + b[2] * a[3],
            b[1] * a[2] + b[3] * a[3],
            b[0] * a[4] + b[2] * a[5] + b[4],
            b[1] * a[4] + b[3] * a[5] + b[5],
        ])
    }
}

impl<'a> Font<'a> {
    pub fn parse(data: &'a [u8]) -> Option<Self> {
        let tables = u16_at(data, 4)? as usize;
        let table = |tag: &[u8; 4]| -> Option<&'a [u8]> {
            (0..tables).find_map(|i| {
                let rec = 12 + 16 * i;
                if data.get(rec..rec + 4)? != tag {
                    return None;
                }
                let off = u32_at(data, rec + 8)? as usize;
                let len = u32_at(data, rec + 12)? as usize;
                data.get(off..off + len)
            })
        };
        let find = |tag: &[u8; 4]| -> Option<usize> {
            (0..tables).find_map(|i| {
                let rec = 12 + 16 * i;
                (data.get(rec..rec + 4)? == tag).then(|| u32_at(data, rec + 8).map(|o| o as usize)).flatten()
            })
        };
        let head = find(b"head")?;
        let hhea = find(b"hhea")?;
        let maxp = find(b"maxp")?;
        // prefer a full-Unicode (format 12) subtable, else BMP format 4; a symbol table (3, 0) is the last resort
        // and a font without any cmap still parses (PDF subsets drop it), with every character lookup missing
        let mut cmap = None;
        let mut symbol = None;
        if let Some(cmap_table) = find(b"cmap") {
            for i in 0..u16_at(data, cmap_table + 2).unwrap_or(0) as usize {
                let rec = cmap_table + 4 + 8 * i;
                let (Some(pid), Some(eid)) = (u16_at(data, rec), u16_at(data, rec + 2)) else { break };
                let Some(off) = u32_at(data, rec + 4).map(|o| cmap_table + o as usize) else { break };
                let Some(format) = u16_at(data, off) else { continue };
                let unicode = pid == 0 || (pid == 3 && (eid == 1 || eid == 10));
                if unicode && (format == 12 || (format == 4 && cmap.is_none())) {
                    cmap = Some((off, format));
                }
                if pid == 3 && eid == 0 && format == 4 && symbol.is_none() {
                    symbol = Some((off, format));
                }
            }
        }
        let (cmap, cmap_format) = cmap.or(symbol).unwrap_or((0, 0));
        let cff = find(b"CFF ").and_then(|o| Cff::parse(data.get(o..)?));
        let (glyf, loca) = match cff {
            Some(_) => (0, 0),
            None => (find(b"glyf")?, find(b"loca")?),
        };
        let axes = table(b"fvar").and_then(gvar::axes).unwrap_or_default();
        // OS/2 carries the x-height and cap height from version 2 on; post carries the underline (both optional).
        let os2 = find(b"OS/2").filter(|&o| u16_at(data, o).is_some_and(|v| v >= 2));
        let os2_field = |off: usize| os2.and_then(|o| i16_at(data, o + off)).unwrap_or(0) as f32;
        let post = find(b"post");
        let post_field = |off: usize| post.and_then(|o| i16_at(data, o + off)).unwrap_or(0) as f32;
        Some(Font {
            line_gap: i16_at(data, hhea + 8).unwrap_or(0) as f32,
            x_height: os2_field(86),
            cap_height: os2_field(88),
            underline_position: post_field(8),
            underline_thickness: post_field(10),
            data,
            cff,
            gvar: table(b"gvar").filter(|_| !axes.is_empty()).and_then(Gvar::parse),
            avar: table(b"avar"),
            axes,
            glyf,
            loca,
            hmtx: find(b"hmtx")?,
            loca_long: i16_at(data, head + 50)? != 0,
            num_hmetrics: u16_at(data, hhea + 34)?,
            num_glyphs: u16_at(data, maxp + 4)?,
            cmap,
            cmap_format,
            upem: u16_at(data, head + 18)? as f32,
            ascent: i16_at(data, hhea + 4)? as f32,
            descent: i16_at(data, hhea + 6)? as f32,
            cache: GlyphCache::new(0, CACHE_BYTES),
            ot: std::sync::OnceLock::new(),
        })
    }

    pub fn glyph(&self, ch: char) -> u16 {
        self.lookup(ch as u32).unwrap_or(0)
    }

    /// The glyph with this PostScript name (CFF fonts only).
    pub fn glyph_named(&self, name: &str) -> Option<u16> {
        self.cff.as_ref()?.glyph_named(name)
    }

    /// The glyph for a raw character code in the font's own cmap (0 when missing).
    pub fn glyph_code(&self, code: u32) -> u16 {
        self.lookup(code).unwrap_or(0)
    }

    /// A symbolic font's glyph for a single-byte code: symbol cmaps place codes at 0xF000 or 0xF100.
    pub fn glyph_symbolic(&self, code: u8) -> u16 {
        for base in [0xF000u32, 0, 0xF100] {
            let g = self.glyph_code(base + code as u32);
            if g != 0 {
                return g;
            }
        }
        0
    }

    pub fn has_cmap(&self) -> bool {
        self.cmap_format != 0
    }

    pub fn num_glyphs(&self) -> u16 {
        self.num_glyphs
    }

    /// The glyph outline as flattened line segments in font units, y up.
    pub fn outline_lines(&self, gid: u16) -> Vec<((f32, f32), (f32, f32))> {
        let mut lines = Vec::new();
        self.outline(gid, Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]), &mut lines, 0, &[]);
        lines.into_iter().map(|(a, b)| ((a.x, a.y), (b.x, b.y))).collect()
    }

    /// The CFF program behind an OpenType font, if any.
    pub fn cff(&self) -> Option<&Cff<'a>> {
        self.cff.as_ref()
    }

    fn lookup(&self, c: u32) -> Option<u16> {
        if self.cmap_format == 0 {
            return None;
        }
        let d = self.data;
        let t = self.cmap;
        if self.cmap_format == 12 {
            let groups = u32_at(d, t + 12)? as usize;
            let (mut lo, mut hi) = (0, groups);
            while lo < hi {
                let mid = (lo + hi) / 2;
                let g = t + 16 + 12 * mid;
                let (start, end) = (u32_at(d, g)?, u32_at(d, g + 4)?);
                if c < start {
                    hi = mid;
                } else if c > end {
                    lo = mid + 1;
                } else {
                    return Some((u32_at(d, g + 8)? + c - start) as u16);
                }
            }
            return None;
        }
        if c > 0xffff {
            return None;
        }
        let segs = u16_at(d, t + 6)? as usize / 2;
        let ends = t + 14;
        let starts = ends + 2 * segs + 2;
        let deltas = starts + 2 * segs;
        let ranges = deltas + 2 * segs;
        let i = (0..segs).find(|&i| u16_at(d, ends + 2 * i).is_some_and(|e| e as u32 >= c))?;
        let start = u16_at(d, starts + 2 * i)? as u32;
        if c < start {
            return None;
        }
        let delta = u16_at(d, deltas + 2 * i)?;
        let range = u16_at(d, ranges + 2 * i)? as usize;
        if range == 0 {
            return Some((c as u16).wrapping_add(delta));
        }
        let g = u16_at(d, ranges + 2 * i + range + 2 * (c - start) as usize)?;
        (g != 0).then(|| g.wrapping_add(delta))
    }

    /// Advance width of `gid` in font units at a variation instance.
    pub fn advance_var(&self, gid: u16, vars: &[Var]) -> f32 {
        self.advance_at(gid, &self.normalized(vars))
    }

    pub fn advance(&self, gid: u16) -> f32 {
        let i = gid.min(self.num_hmetrics.saturating_sub(1)) as usize;
        u16_at(self.data, self.hmtx + 4 * i).unwrap_or(0) as f32
    }

    /// Vertical metrics at `px` (`NativeFontMetrics`): ascent and descent from hhea, leading from its line gap,
    /// x-height and cap height from OS/2, underline from post. A missing table gives zero for its values.
    pub fn line_metrics(&self, px: f32) -> LineMetrics {
        let k = px / self.upem;
        LineMetrics {
            ascent: self.ascent * k,
            descent: -self.descent * k,
            leading: self.line_gap * k,
            cap_height: self.cap_height * k,
            x_height: self.x_height * k,
            underline_position: self.underline_position * k,
            underline_thickness: self.underline_thickness * k,
        }
    }

    /// The outline of glyph `gid` at size `px` as straight segments `[x0, y0, x1, y1]`, curves flattened. The origin
    /// is the glyph origin on the baseline, x grows right and y grows down (so ink above the baseline is negative).
    /// `vars` selects a variable-font instance. Empty for a glyph without outline.
    pub fn glyph_segments(&self, gid: u16, px: f32, vars: &[Var]) -> Vec<[f32; 4]> {
        let k = px / self.upem;
        let mut lines: Vec<(Pt, Pt)> = Vec::new();
        let norm = self.normalized(vars);
        self.outline(gid, Affine([k, 0.0, 0.0, -k, 0.0, 0.0]), &mut lines, 0, &norm);
        lines.iter().map(|(a, b)| [a.x, a.y, b.x, b.y]).collect()
    }

    /// The variation axes (empty for static fonts).
    pub fn axes(&self) -> &[Axis] {
        &self.axes
    }

    /// Normalised axis coordinates for `vars`; empty when the font is static
    /// or every axis sits at its default.
    fn normalized(&self, vars: &[Var]) -> Vec<f32> {
        if self.gvar.is_none() {
            return Vec::new();
        }
        let n = gvar::normalize(&self.axes, self.avar, vars);
        if n.iter().all(|&v| v == 0.0) {
            Vec::new()
        } else {
            n
        }
    }

    /// Number of outline points (simple) or components (composite); 0 when empty.
    fn point_count(&self, gid: u16) -> usize {
        let Some((start, _)) = self.glyph_range(gid) else {
            return 0;
        };
        let d = self.data;
        let contours = i16_at(d, start).unwrap_or(0);
        if contours >= 0 {
            return if contours == 0 { 0 } else { u16_at(d, start + 10 + 2 * (contours as usize - 1)).map_or(0, |e| e as usize + 1) };
        }
        let mut p = start + 10;
        let mut n = 0;
        loop {
            let Some(flags) = u16_at(d, p) else { return n };
            n += 1;
            p += 4
                + if flags & 1 != 0 { 4 } else { 2 }
                + if flags & 8 != 0 {
                    2
                } else if flags & 0x40 != 0 {
                    4
                } else if flags & 0x80 != 0 {
                    8
                } else {
                    0
                };
            if flags & 0x20 == 0 {
                return n;
            }
        }
    }

    /// Advance width at the normalised coordinates, from the phantom points.
    fn advance_at(&self, gid: u16, norm: &[f32]) -> f32 {
        let base = self.advance(gid);
        let Some(gv) = self.gvar.as_ref().filter(|_| !norm.is_empty()) else {
            return base;
        };
        let n = self.point_count(gid);
        let (mut xs, mut ys) = (vec![0.0; n + 4], vec![0.0; n + 4]);
        match gv.apply(gid, norm, &mut xs, &mut ys, &[]) {
            Some(()) => base + xs[n + 1] - xs[n],
            None => base,
        }
    }

    fn glyph_range(&self, gid: u16) -> Option<(usize, usize)> {
        if gid >= self.num_glyphs {
            return None;
        }
        let (a, b) = if self.loca_long {
            (u32_at(self.data, self.loca + 4 * gid as usize)? as usize, u32_at(self.data, self.loca + 4 * gid as usize + 4)? as usize)
        } else {
            (u16_at(self.data, self.loca + 2 * gid as usize)? as usize * 2, u16_at(self.data, self.loca + 2 * gid as usize + 2)? as usize * 2)
        };
        (b > a).then_some((self.glyf + a, self.glyf + b))
    }

    /// Appends the glyph's outline as line segments (after `m`), at the
    /// normalised variation coordinates `norm` (empty = default instance).
    fn outline(&self, gid: u16, m: Affine, lines: &mut Vec<(Pt, Pt)>, depth: u32, norm: &[f32]) -> Option<()> {
        if let Some(cff) = &self.cff {
            let mut sink = Flatten { m, lines, cur: Pt { x: 0.0, y: 0.0 }, start: Pt { x: 0.0, y: 0.0 } };
            return cff.outline(gid, &mut sink).map(|_| ());
        }
        let (start, _) = self.glyph_range(gid)?;
        let d = self.data;
        let contours = i16_at(d, start)?;
        if contours < 0 {
            return if depth < 8 { self.composite(gid, start + 10, m, lines, depth, norm) } else { None };
        }
        let n = contours as usize;
        let ends: Vec<usize> = (0..n).map(|i| u16_at(d, start + 10 + 2 * i).map(|e| e as usize)).collect::<Option<_>>()?;
        let points = ends.last().map_or(0, |e| e + 1);
        let ins = u16_at(d, start + 10 + 2 * n)? as usize;
        let mut p = start + 12 + 2 * n + ins;
        let mut flags = Vec::with_capacity(points);
        while flags.len() < points {
            let f = *d.get(p)?;
            p += 1;
            flags.push(f);
            if f & 8 != 0 {
                let r = *d.get(p)?;
                p += 1;
                flags.extend(std::iter::repeat_n(f, r as usize));
            }
        }
        flags.truncate(points);
        let mut coord = |short: u8, same: u8| -> Option<Vec<f32>> {
            let mut v = 0i32;
            let mut out = Vec::with_capacity(points);
            for &f in &flags {
                if f & short != 0 {
                    let b = *d.get(p)? as i32;
                    p += 1;
                    v += if f & same != 0 { b } else { -b };
                } else if f & same == 0 {
                    v += i16_at(d, p)? as i32;
                    p += 2;
                }
                out.push(v as f32);
            }
            Some(out)
        };
        let mut xs = coord(2, 16)?;
        let mut ys = coord(4, 32)?;
        if let Some(gv) = self.gvar.as_ref().filter(|_| !norm.is_empty()) {
            xs.extend([0.0; 4]);
            ys.extend([0.0; 4]);
            gv.apply(gid, norm, &mut xs, &mut ys, &ends);
            xs.truncate(points);
            ys.truncate(points);
        }
        let mut first = 0;
        for &end in &ends {
            let pts: Vec<(Pt, bool)> = (first..=end).map(|i| (m.apply(xs[i], ys[i]), flags[i] & 1 != 0)).collect();
            first = end + 1;
            contour(&pts, lines);
        }
        Some(())
    }

    fn composite(&self, parent: u16, mut p: usize, m: Affine, lines: &mut Vec<(Pt, Pt)>, depth: u32, norm: &[f32]) -> Option<()> {
        let d = self.data;
        // component offsets are the composite's variable "points"
        let mut offsets = (Vec::new(), Vec::new());
        if let Some(gv) = self.gvar.as_ref().filter(|_| !norm.is_empty()) {
            let n = self.point_count(parent);
            offsets = (vec![0.0; n + 4], vec![0.0; n + 4]);
            gv.apply(parent, norm, &mut offsets.0, &mut offsets.1, &[]);
        }
        let mut index = 0;
        loop {
            let flags = u16_at(d, p)?;
            let gid = u16_at(d, p + 2)?;
            p += 4;
            let (mut dx, mut dy) = if flags & 1 != 0 {
                let v = (i16_at(d, p)?, i16_at(d, p + 2)?);
                p += 4;
                (v.0 as f32, v.1 as f32)
            } else {
                let v = (*d.get(p)? as i8, *d.get(p + 1)? as i8);
                p += 2;
                (v.0 as f32, v.1 as f32)
            };
            if let (Some(&ox), Some(&oy)) = (offsets.0.get(index), offsets.1.get(index)) {
                dx += ox;
                dy += oy;
            }
            index += 1;
            let f2 = |o: usize| i16_at(d, o).map(|v| v as f32 / 16384.0);
            let (mut a, mut b, mut c, mut e) = (1.0, 0.0, 0.0, 1.0);
            if flags & 8 != 0 {
                a = f2(p)?;
                e = a;
                p += 2;
            } else if flags & 0x40 != 0 {
                a = f2(p)?;
                e = f2(p + 2)?;
                p += 4;
            } else if flags & 0x80 != 0 {
                (a, b, c, e) = (f2(p)?, f2(p + 2)?, f2(p + 4)?, f2(p + 6)?);
                p += 8;
            }
            let local = Affine([a, b, c, e, dx, dy]);
            self.outline(gid, local.then(&m), lines, depth + 1, norm);
            if flags & 0x20 == 0 {
                return Some(());
            }
        }
    }
}

/// Turns CFF path commands into transformed line segments.
struct Flatten<'l> {
    m: Affine,
    lines: &'l mut Vec<(Pt, Pt)>,
    cur: Pt,
    start: Pt,
}

impl Sink for Flatten<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.cur = self.m.apply(x, y);
        self.start = self.cur;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.m.apply(x, y);
        self.lines.push((self.cur, p));
        self.cur = p;
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let p = self.m.apply(x, y);
        cubic(self.cur, self.m.apply(x1, y1), self.m.apply(x2, y2), p, self.lines);
        self.cur = p;
    }
    fn close(&mut self) {
        if self.cur != self.start {
            self.lines.push((self.cur, self.start));
            self.cur = self.start;
        }
    }
}

/// Turns one TrueType contour (on/off-curve points) into line segments.
fn contour(pts: &[(Pt, bool)], lines: &mut Vec<(Pt, Pt)>) {
    if pts.len() < 2 {
        return;
    }
    let n = pts.len();
    // start on an on-curve point (or the implied midpoint of two off points)
    let (start, first) = match pts.iter().position(|p| p.1) {
        Some(i) => (pts[i].0, i),
        None => (lerp(pts[0].0, pts[1].0, 0.5), 0),
    };
    let mut cur = start;
    let mut ctrl: Option<Pt> = None;
    for k in 1..=n {
        let (p, on) = pts[(first + k) % n];
        match (on, ctrl) {
            (true, None) => {
                lines.push((cur, p));
                cur = p;
            }
            (true, Some(c)) => {
                quad(cur, c, p, lines);
                cur = p;
                ctrl = None;
            }
            (false, None) => ctrl = Some(p),
            (false, Some(c)) => {
                let mid = lerp(c, p, 0.5);
                quad(cur, c, mid, lines);
                cur = mid;
                ctrl = Some(p);
            }
        }
    }
    match ctrl {
        Some(c) => quad(cur, c, start, lines),
        None if cur != start => lines.push((cur, start)),
        None => {}
    }
}

/// Segment count for a curve whose second difference has squared length `devsq`.
fn subdivisions(devsq: f32) -> usize {
    1 + (3.0 * devsq).sqrt().sqrt().floor() as usize
}

fn quad(p0: Pt, p1: Pt, p2: Pt, lines: &mut Vec<(Pt, Pt)>) {
    let dev = Pt { x: p0.x - 2.0 * p1.x + p2.x, y: p0.y - 2.0 * p1.y + p2.y };
    let n = subdivisions(dev.x * dev.x + dev.y * dev.y);
    let mut prev = p0;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let p = lerp(lerp(p0, p1, t), lerp(p1, p2, t), t);
        lines.push((prev, p));
        prev = p;
    }
}

fn cubic(p0: Pt, p1: Pt, p2: Pt, p3: Pt, lines: &mut Vec<(Pt, Pt)>) {
    let d1 = Pt { x: p0.x - 2.0 * p1.x + p2.x, y: p0.y - 2.0 * p1.y + p2.y };
    let d2 = Pt { x: p1.x - 2.0 * p2.x + p3.x, y: p1.y - 2.0 * p2.y + p3.y };
    let n = subdivisions((d1.x * d1.x + d1.y * d1.y).max(d2.x * d2.x + d2.y * d2.y) * 2.25);
    let mut prev = p0;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let (a, b, c) = (lerp(p0, p1, t), lerp(p1, p2, t), lerp(p2, p3, t));
        let p = lerp(lerp(a, b, t), lerp(b, c, t), t);
        lines.push((prev, p));
        prev = p;
    }
}

/// Exact-area coverage accumulation (the font-rs technique).
struct Raster {
    w: usize,
    h: usize,
    a: Vec<f32>,
}

impl Raster {
    fn new(w: usize, h: usize) -> Self {
        Raster { w, h, a: vec![0.0; w * h + 4] }
    }

    fn line(&mut self, p0: Pt, p1: Pt) {
        if (p0.y - p1.y).abs() <= f32::EPSILON {
            return;
        }
        let (dir, p0, p1) = if p0.y < p1.y { (1.0, p0, p1) } else { (-1.0, p1, p0) };
        let dxdy = (p1.x - p0.x) / (p1.y - p0.y);
        let mut x = p0.x;
        if p0.y < 0.0 {
            x -= p0.y * dxdy;
        }
        for y in (p0.y.max(0.0) as usize)..self.h.min(p1.y.ceil() as usize) {
            let row = y * self.w;
            let dy = ((y + 1) as f32).min(p1.y) - (y as f32).max(p0.y);
            let xnext = x + dxdy * dy;
            let d = dy * dir;
            let (x0, x1) = if x < xnext { (x, xnext) } else { (xnext, x) };
            let x0f = x0.floor();
            let x0i = (x0f as isize).max(0) as usize;
            let x1i = x1.ceil() as usize;
            if x1i <= x0i + 1 {
                let xm = 0.5 * (x + xnext) - x0f;
                self.a[row + x0i] += d - d * xm;
                self.a[row + x0i + 1] += d * xm;
            } else {
                let s = (x1 - x0).recip();
                let x0frac = x0 - x0f;
                let a0 = 0.5 * s * (1.0 - x0frac) * (1.0 - x0frac);
                let x1frac = x1 - x1.ceil() + 1.0;
                let am = 0.5 * s * x1frac * x1frac;
                self.a[row + x0i] += d * a0;
                if x1i == x0i + 2 {
                    self.a[row + x0i + 1] += d * (1.0 - a0 - am);
                } else {
                    let a1 = s * (1.5 - x0frac);
                    self.a[row + x0i + 1] += d * (a1 - a0);
                    for xi in x0i + 2..x1i - 1 {
                        self.a[row + xi] += d * s;
                    }
                    let a2 = a1 + (x1i - x0i - 3) as f32 * s;
                    self.a[row + x1i - 1] += d * (1.0 - a2 - am);
                }
                self.a[row + x1i] += d * am;
            }
            x = xnext;
        }
    }

    fn coverage(&self) -> Vec<u8> {
        let mut acc = 0.0f32;
        self.a[..self.w * self.h]
            .iter()
            .map(|v| {
                acc += v;
                (acc.abs().min(1.0) * 255.0 + 0.5) as u8
            })
            .collect()
    }
}

const SUBPIXEL: f32 = 16.0;
const CACHE_BYTES: usize = 4 << 20;

struct Mask {
    ox: i32,
    oy: i32,
    w: usize,
    h: usize,
    a: Vec<u8>,
}

#[derive(Hash, PartialEq, Eq, Clone)]
struct MaskKey {
    gid: u16,
    /// pixel size in 1/64 px
    size: u32,
    qx: u8,
    qy: u8,
    /// normalized variation coordinates
    norm: Vec<i16>,
}

/// Glyph coverage cache: LRU, at most `CACHE_BYTES` of coverage, halved under memory pressure.
type GlyphCache = crate::purgeable::Cache<MaskKey, std::sync::Arc<Mask>>;

pub struct Style {
    pub px: f32,
    /// extra horizontal stroke in px, a stand-in for heavier weights
    pub embolden: f32,
    pub color: u32,
    pub shadow: bool,
}

/// Advance and size of an inline color emoji at text size `px` (about 1.2x cap height).
pub fn emoji_advance(px: f32) -> f32 {
    (px * 1.17).round()
}

impl Font<'_> {
    /// Identity of the loaded font file (address and length of its bytes): rendered text cached under one face
    /// is never served for another.
    pub(crate) fn ident(&self) -> (usize, usize) {
        (self.data.as_ptr() as usize, self.data.len())
    }

    pub fn measure(&self, text: &str, px: f32) -> f32 {
        self.measure_var(text, px, &[])
    }

    /// Width in px at a variation instance, shaped with the default features (kerning, ligatures).
    pub fn measure_var(&self, text: &str, px: f32, vars: &[Var]) -> f32 {
        self.measure_feat(text, px, vars, &Features::new())
    }

    /// Width in px with user feature settings.
    pub fn measure_feat(&self, text: &str, px: f32, vars: &[Var], features: &Features) -> f32 {
        let segs = self.emoji_segments(text);
        if segs.iter().any(|s| s.1) {
            return segs.iter().map(|&(t, e)| if e { emoji_advance(px) } else { self.measure_feat(t, px, vars, features) }).sum();
        }
        if let Some((faces, runs)) = crate::fontfallback::runs(self, text) {
            return runs.iter().map(|&(f, t)| f.map_or_else(|| self.measure_plain(t, px, vars), |i| faces[i].measure_plain(t, px, vars))).sum();
        }
        self.measure_shaped(text, px, vars, features)
    }

    /// Width with no emoji or fallback handling: every character takes this face's own glyph.
    pub(crate) fn measure_plain(&self, text: &str, px: f32, vars: &[Var]) -> f32 {
        self.measure_shaped(text, px, vars, &Features::new())
    }

    fn measure_shaped(&self, text: &str, px: f32, vars: &[Var], features: &Features) -> f32 {
        let norm = self.normalized(vars);
        let k = px / self.upem;
        self.shaped(text, &norm, features).iter().map(|g| g.advance * k).sum()
    }

    /// The glyph ids with advances (font units) and offsets for `text`: shaped when the font has layout tables,
    /// one glyph per character otherwise.
    fn shaped(&self, text: &str, norm: &[f32], features: &Features) -> Vec<PlacedGlyph> {
        let layout = self.ot.get_or_init(|| OtLayout::from_sfnt(self.data));
        if layout.is_trivial() {
            return text.chars().map(|c| { let gid = self.glyph(c); PlacedGlyph { gid, advance: self.advance_at(gid, norm), dx: 0.0, dy: 0.0 } }).collect();
        }
        let opts = ShapeOptions { script: shape_ot::detect_script(text), language: None, features, coords: norm };
        let out = layout.shape_text(text, &|c| self.glyph(c), &|g| self.advance_at(g, norm).round() as i32, &opts);
        out.iter().map(|g| PlacedGlyph { gid: g.gid, advance: g.x_advance as f32, dx: g.x_offset as f32, dy: g.y_offset as f32 }).collect()
    }

    /// Runs of `text`, emoji clusters (drawn from the color emoji font) flagged true.
    fn emoji_segments<'t>(&self, text: &'t str) -> Vec<(&'t str, bool)> {
        crate::emoji::segments(text, &|c| self.glyph(c) != 0)
    }

    /// Text with inline color emoji: text runs through the outline path, each
    /// emoji drawn ~1.2x cap height, bottom a little below the baseline.
    fn render_mixed(&self, segs: &[(&str, bool)], st: &Style, vars: &[Var]) -> Image {
        let k = st.px / self.upem;
        let side = emoji_advance(st.px);
        let pad = 2usize;
        let baseline = pad as f32 + self.ascent * k;
        let top = (baseline - side * 0.86).floor();
        let shift = if top < 0.0 { (-top) as usize } else { 0 };
        let base_h = ((self.ascent - self.descent) * k).ceil() as usize + 2 * pad;
        let mut parts: Vec<(f32, Image, Option<f32>)> = vec![];
        let mut x = 0.0;
        for &(t, e) in segs {
            if e {
                if let Some(img) = crate::emoji::render(t, side.round()) {
                    parts.push((x, img, None));
                    x += side;
                    continue;
                }
            }
            let adv = self.measure_var(t, st.px, vars);
            parts.push((x, self.render_var(t, st, vars), Some(adv)));
            x += adv;
        }
        let (w, h) = (x.ceil() as usize + 2 * pad, base_h + shift);
        let mut out = Image::new(w, h);
        for (x, img, adv) in &parts {
            let (ox, oy) = match adv {
                None => (*x as i32 + pad as i32, (baseline + side * 0.14 - img.h as f32).round() as i32 + shift as i32),
                Some(adv) => {
                    // the run's own padding (outline, shadow) sits around its advance
                    let p = ((img.w as f32 - adv) / 2.0).round() as i32;
                    let ip = ((img.h as f32 - (self.ascent - self.descent) * k) / 2.0).round() as i32;
                    (*x as i32 + pad as i32 - p, shift as i32 + pad as i32 - ip)
                }
            };
            for iy in 0..img.h as i32 {
                for ix in 0..img.w as i32 {
                    let (dx, dy) = (ox + ix, oy + iy);
                    if dx >= 0 && dy >= 0 && (dx as usize) < w && (dy as usize) < h {
                        let d = &mut out.px[dy as usize * w + dx as usize];
                        *d = crate::gfx::over(*d, img.px[iy as usize * img.w + ix as usize]);
                    }
                }
            }
        }
        out
    }

    /// Truncates with an ellipsis so the text fits `max_w` px.
    pub fn fit(&self, text: &str, px: f32, max_w: f32) -> String {
        self.fit_var(text, px, max_w, &[])
    }

    pub fn fit_var(&self, text: &str, px: f32, max_w: f32, vars: &[Var]) -> String {
        if self.measure_var(text, px, vars) <= max_w {
            return text.to_string();
        }
        let mut chars: Vec<char> = text.chars().collect();
        while !chars.is_empty() && self.measure_var(&format!("{}…", String::from_iter(&chars).trim_end()), px, vars) > max_w {
            chars.pop();
        }
        format!("{}…", String::from_iter(&chars).trim_end())
    }

    pub fn render(&self, text: &str, st: &Style) -> Image {
        self.render_var(text, st, &[])
    }

    /// Renders at a variation instance, e.g. `&[(*b"wght", 590.0), (*b"opsz", 17.0)]`, with default features.
    pub fn render_var(&self, text: &str, st: &Style, vars: &[Var]) -> Image {
        self.render_feat(text, st, vars, &Features::new())
    }

    /// Renders with user feature settings.
    pub fn render_feat(&self, text: &str, st: &Style, vars: &[Var], features: &Features) -> Image {
        let segs = self.emoji_segments(text);
        if segs.iter().any(|s| s.1) {
            return self.render_mixed(&segs, st, vars);
        }
        if let Some((faces, runs)) = crate::fontfallback::runs(self, text) {
            return self.render_fallback(&faces, &runs, st, vars);
        }
        self.render_shaped(text, st, vars, features)
    }

    /// Renders with this face's own glyphs only (no emoji or fallback handling), default features.
    pub(crate) fn render_plain(&self, text: &str, st: &Style, vars: &[Var]) -> Image {
        self.render_shaped(text, st, vars, &Features::new())
    }

    fn render_shaped(&self, text: &str, st: &Style, vars: &[Var], features: &Features) -> Image {
        let norm = self.normalized(vars);
        let placed = self.shaped(text, &norm, features);
        self.render_placed(&placed, st, &norm, true)
    }

    /// Text whose runs come from different faces, laid on one baseline.
    fn render_fallback(&self, faces: &[Font<'static>], runs: &[(Option<usize>, &str)], st: &Style, vars: &[Var]) -> Image {
        let k = st.px / self.upem;
        let pad = 2 + st.embolden.ceil() as usize + if st.shadow { (st.px / 8.0).ceil() as usize } else { 0 };
        let base_h = ((self.ascent - self.descent) * k).ceil() as usize + 2 * pad;
        let mut parts: Vec<(i32, i32, Image)> = Vec::new();
        let mut x = 0.0f32;
        for &(f, t) in runs {
            let (img, adv, dy) = match f {
                None => (self.render_plain(t, st, vars), self.measure_plain(t, st.px, vars), 0.0),
                Some(i) => {
                    let face = &faces[i];
                    let fk = st.px / face.upem;
                    (face.render_plain(t, st, vars), face.measure_plain(t, st.px, vars), self.ascent * k - face.ascent * fk)
                }
            };
            parts.push((x.round() as i32, dy.round() as i32, img));
            x += adv;
        }
        let top = parts.iter().map(|p| p.1).min().unwrap_or(0).min(0);
        let bottom = parts.iter().map(|p| p.1 + p.2.h as i32).max().unwrap_or(0).max(base_h as i32);
        let w = x.ceil() as usize + 2 * pad;
        let h = (bottom - top).max(1) as usize;
        let mut out = Image::new(w, h);
        for (px0, dy, img) in &parts {
            let oy = dy - top;
            for iy in 0..img.h as i32 {
                for ix in 0..img.w as i32 {
                    let (dx, ty) = (px0 + ix, oy + iy);
                    if dx >= 0 && ty >= 0 && (dx as usize) < w && (ty as usize) < h {
                        let d = &mut out.px[ty as usize * w + dx as usize];
                        *d = crate::gfx::over(*d, img.px[iy as usize * img.w + ix as usize]);
                    }
                }
            }
        }
        out
    }

    /// Renders one glyph by id (symbols looked up by name rather than character).
    pub fn render_glyph(&self, gid: u16, st: &Style) -> Image {
        self.render_glyphs(&[gid], st, &[])
    }

    fn render_glyphs(&self, gids: &[u16], st: &Style, norm: &[f32]) -> Image {
        self.render_with(gids, st, norm, true)
    }

    /// `cached`: compose glyph masks (subpixel positions quantized to 1/16 px);
    /// otherwise rasterize the whole string exactly (reference path).
    fn render_with(&self, gids: &[u16], st: &Style, norm: &[f32], cached: bool) -> Image {
        let placed: Vec<PlacedGlyph> = gids.iter().map(|&gid| PlacedGlyph { gid, advance: self.advance_at(gid, norm), dx: 0.0, dy: 0.0 }).collect();
        self.render_placed(&placed, st, norm, cached)
    }

    fn render_placed(&self, placed: &[PlacedGlyph], st: &Style, norm: &[f32], cached: bool) -> Image {
        let k = st.px / self.upem;
        let pad = 2 + st.embolden.ceil() as usize + if st.shadow { (st.px / 8.0).ceil() as usize } else { 0 };
        let gids: Vec<u16> = placed.iter().map(|g| g.gid).collect();
        let gids = &gids[..];
        let advances: Vec<f32> = placed.iter().map(|g| g.advance * k).collect();
        let offs: Vec<(f32, f32)> = placed.iter().map(|g| (g.dx * k, g.dy * k)).collect();
        let w = advances.iter().sum::<f32>().ceil() as usize + 2 * pad;
        let h = ((self.ascent - self.descent) * k).ceil() as usize + 2 * pad;
        let baseline = pad as f32 + self.ascent * k;
        let mut cov = if cached {
            self.compose(gids, &advances, &offs, st.px, pad as f32, baseline, w, h, norm)
        } else {
            self.raster_string(gids, &advances, &offs, k, pad as f32, baseline, w, h, norm)
        };
        let bold = st.embolden.round() as usize;
        if bold > 0 {
            for row in cov.chunks_mut(w) {
                for x in (0..w).rev() {
                    row[x] = (x.saturating_sub(bold)..=x).map(|i| row[i]).max().unwrap_or(0);
                }
            }
        }
        let mut img = Image::new(w, h);
        if st.shadow {
            let mut sh = Image { w, h, px: cov.iter().map(|&c| pack(0, 0, 0, c)).collect() };
            sh.blur((st.px / 14.0).max(1.0) as usize);
            let dy = (st.px / 20.0).ceil() as usize;
            for y in dy..h {
                for x in 0..w {
                    img.px[y * w + x] = scale(sh.px[(y - dy) * w + x], 110);
                }
            }
        }
        for (p, &c) in img.px.iter_mut().zip(&cov) {
            *p = crate::gfx::over(*p, scale(st.color, c as u32));
        }
        img
    }

    fn raster_string(&self, gids: &[u16], advances: &[f32], offs: &[(f32, f32)], k: f32, pad: f32, baseline: f32, w: usize, h: usize, norm: &[f32]) -> Vec<u8> {
        let mut lines = Vec::new();
        let mut pen = pad;
        for (n, (&gid, adv)) in gids.iter().zip(advances).enumerate() {
            let (ox, oy) = offs.get(n).copied().unwrap_or((0.0, 0.0));
            let m = Affine([k, 0.0, 0.0, -k, pen + ox, baseline - oy]);
            self.outline(gid, m, &mut lines, 0, norm);
            pen += adv;
        }
        let mut r = Raster::new(w, h);
        for (a, b) in lines {
            r.line(a, b);
        }
        r.coverage()
    }

    /// Rasterizes one glyph with its origin at (`fx`, `fy`) inside its own tight box.
    fn glyph_mask(&self, gid: u16, k: f32, fx: f32, fy: f32, norm: &[f32]) -> Mask {
        let mut lines = Vec::new();
        self.outline(gid, Affine([k, 0.0, 0.0, -k, fx, fy]), &mut lines, 0, norm);
        if lines.is_empty() {
            return Mask { ox: 0, oy: 0, w: 0, h: 0, a: Vec::new() };
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (a, b) in &lines {
            for p in [a, b] {
                x0 = x0.min(p.x);
                x1 = x1.max(p.x);
                y0 = y0.min(p.y);
                y1 = y1.max(p.y);
            }
        }
        let (ox, oy) = (x0.floor() as i32, y0.floor() as i32);
        let w = (x1.ceil() as i32 - ox + 1).max(1) as usize;
        let h = (y1.ceil() as i32 - oy + 1).max(1) as usize;
        let mut r = Raster::new(w, h);
        for (a, b) in lines {
            r.line(Pt { x: a.x - ox as f32, y: a.y - oy as f32 }, Pt { x: b.x - ox as f32, y: b.y - oy as f32 });
        }
        Mask { ox, oy, w, h, a: r.coverage() }
    }

    fn cached_mask(&self, gid: u16, px: f32, qx: u8, qy: u8, norm: &[f32]) -> std::sync::Arc<Mask> {
        let size = (px * 64.0).round() as u32;
        let key = MaskKey { gid, size, qx, qy, norm: norm.iter().map(|v| (v * 16384.0).round() as i16).collect() };
        if let Some(m) = self.cache.get(&key) {
            return m;
        }
        let k = size as f32 / 64.0 / self.upem;
        let m = std::sync::Arc::new(self.glyph_mask(gid, k, qx as f32 / SUBPIXEL, qy as f32 / SUBPIXEL, norm));
        self.cache.set(key, m.clone(), m.a.len() + 64);
        m
    }

    #[allow(clippy::too_many_arguments)]
    fn compose(&self, gids: &[u16], advances: &[f32], offs: &[(f32, f32)], px: f32, pad: f32, baseline: f32, w: usize, h: usize, norm: &[f32]) -> Vec<u8> {
        let split = |v: f32| {
            let mut i = v.floor();
            let mut q = ((v - i) * SUBPIXEL).round();
            if q >= SUBPIXEL {
                i += 1.0;
                q = 0.0;
            }
            (i as i32, q as u8)
        };
        let mut cov = vec![0u8; w * h];
        let mut pen = pad;
        for (n, (&gid, adv)) in gids.iter().zip(advances).enumerate() {
            let (ox, oy) = offs.get(n).copied().unwrap_or((0.0, 0.0));
            let (bx, qx) = split(pen + ox);
            let (by, qy) = split(baseline - oy);
            pen += adv;
            let m = self.cached_mask(gid, px, qx, qy, norm);
            for row in 0..m.h {
                let y = by + m.oy + row as i32;
                if y < 0 || y >= h as i32 {
                    continue;
                }
                for col in 0..m.w {
                    let x = bx + m.ox + col as i32;
                    if x < 0 || x >= w as i32 {
                        continue;
                    }
                    let d = &mut cov[y as usize * w + x as usize];
                    *d = d.saturating_add(m.a[row * m.w + col]);
                }
            }
        }
        cov
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sf() -> Vec<u8> {
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).expect("assets/SFUI.ttf (run tools/pack.py)")
    }

    fn ink(img: &Image) -> u64 {
        img.px.iter().map(|p| (p >> 24) as u64).sum()
    }

    #[test]
    fn parses_metrics_and_maps_characters() {
        let data = sf();
        let f = Font::parse(&data).unwrap();
        assert!(f.upem >= 1000.0);
        assert!(f.ascent > 0.0 && f.descent < 0.0);
        let a = f.glyph('A');
        assert_ne!(a, 0);
        assert_ne!(f.glyph('Ú'), 0);
        assert_ne!(f.glyph('…'), 0);
        assert!(f.advance(a) > 0.0);
        assert!(f.advance(f.glyph('W')) > f.advance(f.glyph('i')));
        assert_eq!(f.glyph_named("A"), None, "TrueType fonts have no CFF charset");
    }

    #[test]
    fn renders_ink_inside_bounds() {
        let data = sf();
        let f = Font::parse(&data).unwrap();
        let img = f.render("Hg", &Style { px: 40.0, embolden: 0.0, color: pack(255, 255, 255, 255), shadow: false });
        assert!(ink(&img) > 255 * 40, "drew something");
        // margins stay clear
        assert!(img.px[..img.w].iter().all(|&p| p == 0));
        assert!(img.px[img.px.len() - img.w..].iter().all(|&p| p == 0));
        // the H stem is solid
        assert!(img.px.iter().any(|&p| p >> 24 == 255));
    }

    #[test]
    fn closed_glyph_has_a_hole() {
        let data = sf();
        let f = Font::parse(&data).unwrap();
        let img = f.render("O", &Style { px: 100.0, embolden: 0.0, color: pack(255, 255, 255, 255), shadow: false });
        let mid = img.px[(img.h / 2) * img.w + img.w / 2];
        assert_eq!(mid >> 24, 0, "counter of O is empty");
    }

    #[test]
    fn embolden_adds_ink_and_fit_truncates() {
        let data = sf();
        let f = Font::parse(&data).unwrap();
        let ink = |e: f32| ink(&f.render("Settings", &Style { px: 30.0, embolden: e, color: pack(255, 255, 255, 255), shadow: false }));
        assert!(ink(1.0) > ink(0.0));
        let long = "mcp-signage-android";
        let fitted = f.fit(long, 20.0, 100.0);
        assert!(fitted.ends_with('…'));
        assert!(f.measure(&fitted, 20.0) <= 100.0);
        assert_eq!(f.fit("Clock", 20.0, 100.0), "Clock");
    }

    #[test]
    fn cmap_misses_return_notdef() {
        let data = sf();
        let f = Font::parse(&data).unwrap();
        assert_eq!(f.glyph('\u{10FFFD}'), 0);
    }

    #[test]
    fn cff_font_renders_through_the_same_path() {
        // a CFF symbol font is optional (the packed defaults are TrueType); skip without one
        let Ok(data) = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUISymbols-Regular.otf")) else {
            return;
        };
        let f = Font::parse(&data).unwrap();
        assert!(f.cff.is_some());
        let gid = f.glyph('\u{E8FE}');
        assert_ne!(gid, 0);
        assert!(f.advance(gid) > 0.0);
        let st = Style { px: 40.0, embolden: 0.0, color: pack(255, 255, 255, 255), shadow: false };
        let img = f.render_glyph(gid, &st);
        assert!(ink(&img) > 255 * 40, "the glyph has ink");
        assert!(img.px.iter().any(|&p| p >> 24 == 255), "solid interior");
        assert_eq!(ink(&f.render_glyph(200, &st)), 0, "a glyph past the end is empty");
        // every charset name resolves back to its glyph
        for g in 0..8u16 {
            let name = f.cff.as_ref().unwrap().glyph_name(g as usize).unwrap();
            assert_eq!(f.glyph_named(&name), Some(g), "{name}");
        }
    }

    #[test]
    fn variable_instances_change_weight_and_width() {
        let data = sf();
        let f = Font::parse(&data).unwrap();
        let tags: Vec<&[u8; 4]> = f.axes().iter().map(|a| &a.tag).collect();
        assert!(tags.contains(&b"wght") && tags.contains(&b"opsz"), "{tags:?}");
        let st = Style { px: 60.0, embolden: 0.0, color: pack(255, 255, 255, 255), shadow: false };
        let regular = f.render_var("Hamburg", &st, &[(*b"wght", 400.0)]);
        assert_eq!(regular.px, f.render("Hamburg", &st).px, "the default instance is the plain render");
        let bold = f.render_var("Hamburg", &st, &[(*b"wght", 700.0)]);
        assert!(ink(&bold) as f32 > ink(&regular) as f32 * 1.15, "bold has clearly more ink");
        assert!(f.measure_var("Hamburg", 60.0, &[(*b"wght", 700.0)]) > f.measure("Hamburg", 60.0), "and wider advances");
        // the text cut (smallest optical size) is wider and looser than the display cut (largest)
        let opsz = f.axes().iter().find(|a| &a.tag == b"opsz").unwrap();
        let text = f.measure_var("61%", 18.75, &[(*b"opsz", opsz.min)]);
        let display = f.measure_var("61%", 18.75, &[(*b"opsz", opsz.max)]);
        assert!(text > display * 1.02, "{text} vs {display}");
        // composites (accents) vary too and stay attached to their base
        let e = f.render_var("é", &st, &[(*b"wght", 700.0)]);
        assert!(ink(&e) > ink(&f.render("é", &st)));
        let fitted = f.fit_var("Enter the password for the network", 18.75, 120.0, &[(*b"wght", 590.0)]);
        assert!(fitted.ends_with('…') && f.measure_var(&fitted, 18.75, &[(*b"wght", 590.0)]) <= 120.0);
    }

    #[test]
    fn cubic_flattening_reaches_the_end_point() {
        let mut lines = Vec::new();
        let p = |x, y| Pt { x, y };
        cubic(p(0.0, 0.0), p(0.0, 50.0), p(100.0, 50.0), p(100.0, 0.0), &mut lines);
        assert!(lines.len() > 4, "a wide arc gets subdivided");
        assert_eq!(lines.first().unwrap().0, p(0.0, 0.0));
        assert_eq!(lines.last().unwrap().1, p(100.0, 0.0));
        let mut straight = Vec::new();
        cubic(p(0.0, 0.0), p(1.0, 1.0), p(2.0, 2.0), p(3.0, 3.0), &mut straight);
        assert_eq!(straight.len(), 1);
    }

    fn maxdiff(a: &Image, b: &Image) -> u32 {
        assert_eq!((a.w, a.h), (b.w, b.h));
        a.px.iter().zip(&b.px).map(|(&p, &q)| (p >> 24).abs_diff(q >> 24)).max().unwrap_or(0)
    }

    #[test]
    fn cached_render_matches_exact_raster() {
        let data = sf();
        let f = Font::parse(&data).unwrap();
        let emoji = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUISymbols-Regular.otf")).ok();
        let sym = emoji.as_deref().and_then(Font::parse);
        for text in ["Þórður æði ö", "Hamburg 61%", "Stillingar", "iiiilll", "😀 ok"] {
            for px in [11.0f32, 17.5, 22.0, 41.3] {
                for w in [400.0, 590.0, 700.0] {
                    let vars = [(*b"wght", w), (*b"opsz", px)];
                    let st = Style { px, embolden: 0.0, color: pack(255, 255, 255, 255), shadow: false };
                    let norm = f.normalized(&vars);
                    let gids: Vec<u16> = text.chars().map(|c| f.glyph(c)).collect();
                    let exact = f.render_with(&gids, &st, &norm, false);
                    let cached = f.render_with(&gids, &st, &norm, true);
                    assert!(maxdiff(&exact, &cached) <= 24, "{text} {px} {w}: {}", maxdiff(&exact, &cached));
                    assert_eq!(cached, f.render_with(&gids, &st, &norm, true), "cache hits are deterministic");
                }
            }
        }
        if let Some(sym) = sym {
            let st = Style { px: 40.0, embolden: 0.0, color: pack(255, 255, 255, 255), shadow: false };
            let g = sym.glyph('\u{E8FE}');
            assert!(maxdiff(&sym.render_with(&[g], &st, &[], false), &sym.render_with(&[g], &st, &[], true)) <= 24);
        }
    }

    #[test]
    fn glyph_cache_is_bounded() {
        let data = sf();
        let f = Font::parse(&data).unwrap();
        for i in 0..400 {
            let st = Style { px: 60.0 + i as f32 * 0.37, embolden: 0.0, color: pack(255, 255, 255, 255), shadow: false };
            f.render("WgÞ@", &st);
        }
        assert!(f.cache.total_cost() <= CACHE_BYTES);
    }

    #[test]
    fn line_metrics_scale_with_size_and_stay_sane() {
        let data = sf();
        let f = Font::parse(&data).unwrap();
        let m = f.line_metrics(20.0);
        assert!(m.ascent > 0.0 && m.descent > 0.0, "{m:?}");
        assert!(m.leading >= 0.0 && m.underline_thickness >= 0.0, "{m:?}");
        assert!(m.cap_height >= m.x_height && m.x_height >= 0.0, "{m:?}");
        assert!((m.line_height() - (m.ascent + m.descent + m.leading)).abs() < 1e-4);
        let double = f.line_metrics(40.0);
        assert!((double.ascent - 2.0 * m.ascent).abs() < 1e-3, "linear in size");
        assert!((double.underline_position - 2.0 * m.underline_position).abs() < 1e-3);
        assert!((m.ascent - f.ascent * 20.0 / f.upem).abs() < 1e-3);
    }
}
