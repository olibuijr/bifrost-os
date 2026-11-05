//! An OpenType text shaper: glyph classes and mark attachment classes (`GDEF`), glyph substitution lookups 1 to 8
//! (single, multiple, alternate, ligature, contextual, chaining contextual, extension, reverse chaining), glyph
//! positioning lookups 1 to 9 (single, pair with glyph and class pairs, cursive, mark to base, mark to ligature,
//! mark to mark, contextual, chaining contextual, extension), the legacy `kern` table as a fallback, script and
//! language selection, feature selection with user settings, and feature variations for variable fonts.
//!
//! Input is a run of glyph ids with clusters in logical order; output is glyph ids, clusters, advances and offsets
//! in font units. Not covered: right-to-left reordering (the caller reorders), device and variation-index
//! adjustments inside anchors and value records, and per-range feature settings (a setting covers the whole run).

use crate::kern::Kern;

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(o..o.checked_add(2)?)?.try_into().ok()?))
}
fn i16_at(d: &[u8], o: usize) -> Option<i16> {
    u16_at(d, o).map(|v| v as i16)
}
fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(o..o.checked_add(4)?)?.try_into().ok()?))
}

/// A four byte tag from text, padded with spaces (`"ss01"`, `"kern"`).
pub fn tag(s: &str) -> [u8; 4] {
    let mut t = [b' '; 4];
    for (i, b) in s.bytes().take(4).enumerate() {
        t[i] = b;
    }
    t
}

/// The user's feature settings for a run: a tag and a value (0 off, 1 on, n selects alternate n for alternate
/// substitutions). Features not listed keep their default.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Features {
    items: Vec<([u8; 4], u32)>,
}

impl Features {
    pub fn new() -> Self {
        Features::default()
    }
    pub fn set(&mut self, t: [u8; 4], value: u32) -> &mut Self {
        match self.items.iter_mut().find(|i| i.0 == t) {
            Some(i) => i.1 = value,
            None => self.items.push((t, value)),
        }
        self
    }
    pub fn on(&mut self, t: &str) -> &mut Self {
        self.set(tag(t), 1)
    }
    pub fn off(&mut self, t: &str) -> &mut Self {
        self.set(tag(t), 0)
    }
    pub fn value(&self, t: [u8; 4]) -> Option<u32> {
        self.items.iter().find(|i| i.0 == t).map(|i| i.1)
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn items(&self) -> &[([u8; 4], u32)] {
        &self.items
    }
    /// Parses `"liga=0, ss01, smcp=1"`; a bare tag means on, `-tag` means off.
    pub fn parse(s: &str) -> Features {
        let mut f = Features::new();
        for part in s.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if let Some(t) = part.strip_prefix('-') {
                f.set(tag(t.trim()), 0);
            } else if let Some((t, v)) = part.split_once('=') {
                f.set(tag(t.trim()), v.trim().parse().unwrap_or(1));
            } else {
                f.set(tag(part), 1);
            }
        }
        f
    }
}

/// The script tag for text: the first letter that is not Latin-neutral decides; Latin otherwise.
pub fn detect_script(text: &str) -> [u8; 4] {
    for c in text.chars().filter(|c| c.is_alphabetic()) {
        let u = c as u32;
        let t: &[u8; 4] = match u {
            0..=0x24F | 0x1E00..=0x1EFF => b"latn",
            0x370..=0x3FF => b"grek",
            0x400..=0x52F => b"cyrl",
            0x590..=0x5FF => b"hebr",
            0x600..=0x6FF => b"arab",
            0x900..=0x97F => b"deva",
            0xE00..=0xE7F => b"thai",
            0x3040..=0x30FF => b"kana",
            0x4E00..=0x9FFF => b"hani",
            0xAC00..=0xD7AF => b"hang",
            _ => continue,
        };
        return *t;
    }
    *b"latn"
}

/// One shaped glyph, in font units. As input, only `gid` and `cluster` are read.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct ShapedGlyph {
    pub gid: u16,
    /// Index of the first source character of the glyph.
    pub cluster: u32,
    pub x_advance: i32,
    pub y_advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
}

/// Settings for one shaping call.
#[derive(Clone, Debug)]
pub struct ShapeOptions<'f> {
    pub script: [u8; 4],
    pub language: Option<[u8; 4]>,
    pub features: &'f Features,
    /// Normalised variation coordinates (empty at the defaults), for feature variations.
    pub coords: &'f [f32],
}

// ---- table readers ----

fn coverage_index(d: &[u8], off: usize, gid: u16) -> Option<usize> {
    let n = u16_at(d, off + 2)? as usize;
    let (mut lo, mut hi) = (0, n);
    match u16_at(d, off)? {
        1 => {
            while lo < hi {
                let mid = (lo + hi) / 2;
                let g = u16_at(d, off + 4 + 2 * mid)?;
                if gid < g {
                    hi = mid
                } else if gid > g {
                    lo = mid + 1
                } else {
                    return Some(mid);
                }
            }
            None
        }
        2 => {
            while lo < hi {
                let mid = (lo + hi) / 2;
                let r = off + 4 + 6 * mid;
                let (s, e) = (u16_at(d, r)?, u16_at(d, r + 2)?);
                if gid < s {
                    hi = mid
                } else if gid > e {
                    lo = mid + 1
                } else {
                    return Some(u16_at(d, r + 4)? as usize + (gid - s) as usize);
                }
            }
            None
        }
        _ => None,
    }
}

fn class_def(d: &[u8], off: usize, gid: u16) -> u16 {
    match u16_at(d, off) {
        Some(1) => {
            let start = u16_at(d, off + 2).unwrap_or(0);
            let n = u16_at(d, off + 4).unwrap_or(0);
            if gid >= start && gid - start < n {
                u16_at(d, off + 6 + 2 * (gid - start) as usize).unwrap_or(0)
            } else {
                0
            }
        }
        Some(2) => {
            let n = u16_at(d, off + 2).unwrap_or(0) as usize;
            let (mut lo, mut hi) = (0, n);
            while lo < hi {
                let mid = (lo + hi) / 2;
                let r = off + 4 + 6 * mid;
                let (Some(s), Some(e)) = (u16_at(d, r), u16_at(d, r + 2)) else { return 0 };
                if gid < s {
                    hi = mid
                } else if gid > e {
                    lo = mid + 1
                } else {
                    return u16_at(d, r + 4).unwrap_or(0);
                }
            }
            0
        }
        _ => 0,
    }
}

/// Class of `gid` in the class definition at `base + rel`; 0 for a null offset.
fn class_rel(d: &[u8], base: usize, rel: u16, gid: u16) -> u16 {
    if rel == 0 {
        0
    } else {
        class_def(d, base + rel as usize, gid)
    }
}

struct Gdef<'a> {
    d: &'a [u8],
    glyph_classes: Option<usize>,
    mark_classes: Option<usize>,
    sets: Vec<usize>,
}

impl<'a> Gdef<'a> {
    fn parse(d: &'a [u8]) -> Option<Gdef<'a>> {
        if u16_at(d, 0)? != 1 {
            return None;
        }
        let minor = u16_at(d, 2)?;
        let rel = |o: usize| u16_at(d, o).filter(|&v| v != 0).map(|v| v as usize);
        let mut sets = Vec::new();
        if minor >= 2 {
            if let Some(s) = rel(12) {
                for i in 0..u16_at(d, s + 2).unwrap_or(0) as usize {
                    if let Some(o) = u32_at(d, s + 4 + 4 * i) {
                        sets.push(s + o as usize);
                    }
                }
            }
        }
        Some(Gdef { d, glyph_classes: rel(4), mark_classes: rel(10), sets })
    }
    /// 1 base, 2 ligature, 3 mark, 4 component, 0 unclassified.
    fn class(&self, gid: u16) -> u16 {
        self.glyph_classes.map_or(0, |o| class_def(self.d, o, gid))
    }
    fn mark_attach(&self, gid: u16) -> u16 {
        self.mark_classes.map_or(0, |o| class_def(self.d, o, gid))
    }
    fn in_set(&self, set: u16, gid: u16) -> bool {
        self.sets.get(set as usize).is_some_and(|&o| coverage_index(self.d, o, gid).is_some())
    }
}

struct Lookup {
    flag: u16,
    mfs: Option<u16>,
    /// (subtable type with extensions resolved, absolute offset)
    subs: Vec<(u16, usize)>,
}

struct Table<'a> {
    d: &'a [u8],
    lookups: Vec<Lookup>,
}

impl<'a> Table<'a> {
    fn parse(d: &'a [u8], ext: u16) -> Option<Table<'a>> {
        let ll = u16_at(d, 8)? as usize;
        let n = u16_at(d, ll)? as usize;
        let mut lookups = Vec::with_capacity(n);
        for i in 0..n {
            let lo = ll + u16_at(d, ll + 2 + 2 * i)? as usize;
            let (ty, flag, cnt) = (u16_at(d, lo)?, u16_at(d, lo + 2)?, u16_at(d, lo + 4)? as usize);
            let mut subs = Vec::new();
            for k in 0..cnt {
                let Some(so) = u16_at(d, lo + 6 + 2 * k).map(|v| lo + v as usize) else { continue };
                if ty == ext {
                    if let (Some(1), Some(et), Some(eo)) = (u16_at(d, so), u16_at(d, so + 2), u32_at(d, so + 4)) {
                        subs.push((et, so + eo as usize));
                    }
                } else {
                    subs.push((ty, so));
                }
            }
            let mfs = if flag & 0x10 != 0 { u16_at(d, lo + 6 + 2 * cnt) } else { None };
            lookups.push(Lookup { flag, mfs, subs });
        }
        Some(Table { d, lookups })
    }

    /// Lookups to run for `script`, `language` and the wanted features: (lookup, value, first stage), by index.
    fn select(&self, opts: &ShapeOptions, defaults: &[([u8; 4], bool)]) -> Vec<(u16, u32, bool)> {
        let d = self.d;
        let (Some(sl), Some(fl)) = (u16_at(d, 4), u16_at(d, 6)) else { return vec![] };
        let (sl, fl) = (sl as usize, fl as usize);
        let find_script = |t: [u8; 4]| -> Option<usize> {
            (0..u16_at(d, sl)? as usize).find_map(|i| (d.get(sl + 2 + 6 * i..sl + 6 + 6 * i)? == t).then(|| u16_at(d, sl + 6 + 6 * i).map(|o| sl + o as usize)).flatten())
        };
        let script = [opts.script, *b"DFLT", *b"dflt", *b"latn"].iter().find_map(|&t| find_script(t));
        let Some(script) = script else { return vec![] };
        let mut lang = None;
        if let Some(l) = opts.language {
            for i in 0..u16_at(d, script + 2).unwrap_or(0) as usize {
                let r = script + 4 + 6 * i;
                if d.get(r..r + 4) == Some(&l[..]) {
                    lang = u16_at(d, r + 4).map(|o| script + o as usize);
                }
            }
        }
        let lang = lang.or_else(|| u16_at(d, script).filter(|&o| o != 0).map(|o| script + o as usize));
        let Some(ls) = lang else { return vec![] };
        let mut feats: Vec<u16> = Vec::new();
        if let Some(req) = u16_at(d, ls + 2).filter(|&r| r != 0xFFFF) {
            feats.push(req);
        }
        for i in 0..u16_at(d, ls + 4).unwrap_or(0) as usize {
            if let Some(f) = u16_at(d, ls + 6 + 2 * i) {
                feats.push(f);
            }
        }
        let required = u16_at(d, ls + 2).filter(|&r| r != 0xFFFF);
        let mut out: Vec<(u16, u32, bool)> = Vec::new();
        for f in feats {
            let Some(ftag) = d.get(fl + 2 + 6 * f as usize..fl + 6 + 6 * f as usize) else { continue };
            let ftag: [u8; 4] = ftag.try_into().unwrap();
            let default = defaults.iter().find(|x| x.0 == ftag);
            let value = match opts.features.value(ftag) {
                Some(v) => v,
                None if default.is_some() || required == Some(f) => 1,
                None => 0,
            };
            if value == 0 {
                continue;
            }
            let first = default.is_some_and(|x| x.1) && opts.features.value(ftag).is_none();
            for li in self.feature_lookups(fl, f, opts.coords) {
                match out.iter_mut().find(|x| x.0 == li) {
                    Some(x) => {
                        x.1 = x.1.max(value);
                        x.2 &= first;
                    }
                    None => out.push((li, value, first)),
                }
            }
        }
        out.sort_by_key(|x| x.0);
        out
    }

    fn feature_lookups(&self, fl: usize, f: u16, coords: &[f32]) -> Vec<u16> {
        let d = self.d;
        let read = |ft: usize| -> Vec<u16> { (0..u16_at(d, ft + 2).unwrap_or(0) as usize).filter_map(|i| u16_at(d, ft + 4 + 2 * i)).collect() };
        if let Some(alt) = self.variation_feature(f, coords) {
            return read(alt);
        }
        match u16_at(d, fl + 2 + 6 * f as usize + 4) {
            Some(o) => read(fl + o as usize),
            None => vec![],
        }
    }

    /// The substituted feature table for feature `f` under the first matching feature variation record.
    fn variation_feature(&self, f: u16, coords: &[f32]) -> Option<usize> {
        let d = self.d;
        if u16_at(d, 2)? < 1 {
            return None;
        }
        let fv = u32_at(d, 10)? as usize;
        if fv == 0 {
            return None;
        }
        for r in 0..u32_at(d, fv + 4)? as usize {
            let rec = fv + 8 + 8 * r;
            let (cs, fs) = (u32_at(d, rec)? as usize, u32_at(d, rec + 4)? as usize);
            let ok = cs == 0 || {
                let set = fv + cs;
                (0..u16_at(d, set).unwrap_or(0) as usize).all(|i| {
                    let Some(c) = u32_at(d, set + 2 + 4 * i).map(|o| set + o as usize) else { return false };
                    if u16_at(d, c) != Some(1) {
                        return false;
                    }
                    let (Some(axis), Some(lo), Some(hi)) = (u16_at(d, c + 2), i16_at(d, c + 4), i16_at(d, c + 6)) else { return false };
                    let v = coords.get(axis as usize).copied().unwrap_or(0.0);
                    v >= lo as f32 / 16384.0 && v <= hi as f32 / 16384.0
                })
            };
            if !ok {
                continue;
            }
            if fs == 0 {
                return None;
            }
            let sub = fv + fs;
            for i in 0..u16_at(d, sub + 4)? as usize {
                let rr = sub + 6 + 6 * i;
                if u16_at(d, rr)? == f {
                    return Some(sub + u32_at(d, rr + 2)? as usize);
                }
            }
            return None;
        }
        None
    }
}

/// The layout tables of one font.
pub struct OtLayout<'a> {
    gdef: Option<Gdef<'a>>,
    gsub: Option<Table<'a>>,
    gpos: Option<Table<'a>>,
    kern: Option<Kern>,
}

const GSUB_DEFAULTS: &[([u8; 4], bool)] = &[(*b"rvrn", true), (*b"ccmp", true), (*b"locl", true), (*b"rlig", true), (*b"calt", false), (*b"clig", false), (*b"liga", false), (*b"rclt", false)];
const GPOS_DEFAULTS: &[([u8; 4], bool)] = &[(*b"kern", false), (*b"mark", false), (*b"mkmk", false), (*b"curs", false), (*b"dist", false)];
const MAX_NEST: u32 = 6;

impl<'a> OtLayout<'a> {
    /// From raw table bytes (any may be absent).
    pub fn from_tables(gdef: Option<&'a [u8]>, gsub: Option<&'a [u8]>, gpos: Option<&'a [u8]>, kern: Option<&[u8]>) -> OtLayout<'a> {
        OtLayout {
            gdef: gdef.and_then(Gdef::parse),
            gsub: gsub.and_then(|d| Table::parse(d, 7)),
            gpos: gpos.and_then(|d| Table::parse(d, 9)),
            kern: kern.and_then(Kern::parse).filter(|k| !k.is_empty()),
        }
    }

    /// From a font file's table directory.
    pub fn from_sfnt(data: &'a [u8]) -> OtLayout<'a> {
        Self::from_sfnt_at(data, 0)
    }

    /// From the table directory at `base` (a collection face's entry; table offsets are file-relative).
    pub fn from_sfnt_at(data: &'a [u8], base: usize) -> OtLayout<'a> {
        let n = u16_at(data, base + 4).unwrap_or(0) as usize;
        let table = |t: &[u8; 4]| -> Option<&'a [u8]> {
            (0..n).find_map(|i| {
                let rec = base + 12 + 16 * i;
                if data.get(rec..rec + 4)? != t {
                    return None;
                }
                let off = u32_at(data, rec + 8)? as usize;
                data.get(off..off.checked_add(u32_at(data, rec + 12)? as usize)?)
            })
        };
        OtLayout::from_tables(table(b"GDEF"), table(b"GSUB"), table(b"GPOS"), table(b"kern"))
    }

    /// True when the font has nothing to shape with, so glyph per character with plain advances is exact.
    pub fn is_trivial(&self) -> bool {
        self.gsub.is_none() && self.gpos.is_none() && self.kern.is_none()
    }

    /// Shapes `text`: `cmap` maps characters to glyphs, `hadv` gives a glyph's advance in font units.
    pub fn shape_text(&self, text: &str, cmap: &dyn Fn(char) -> u16, hadv: &dyn Fn(u16) -> i32, opts: &ShapeOptions) -> Vec<ShapedGlyph> {
        let input: Vec<(u16, u32)> = text.chars().enumerate().map(|(i, c)| (cmap(c), i as u32)).collect();
        self.shape_glyphs(&input, hadv, opts)
    }

    /// Shapes glyph ids with their clusters.
    pub fn shape_glyphs(&self, input: &[(u16, u32)], hadv: &dyn Fn(u16) -> i32, opts: &ShapeOptions) -> Vec<ShapedGlyph> {
        let mut buf: Vec<G> = input.iter().map(|&(gid, cluster)| G { gid, cluster, ..G::default() }).collect();
        let mut lig = 0;
        if let Some(t) = &self.gsub {
            let sel = t.select(opts, GSUB_DEFAULTS);
            let mut cx = Ctx::new(self.gdef.as_ref(), t, false, buf, lig);
            for stage in [true, false] {
                for &(li, v, first) in sel.iter().filter(|s| s.2 == stage) {
                    let _ = first;
                    cx.run(li as usize, v);
                }
            }
            buf = cx.buf;
            lig = cx.next_lig;
        }
        for g in buf.iter_mut() {
            g.xa = hadv(g.gid);
        }
        let mut gpos_kern = false;
        if let Some(t) = &self.gpos {
            let sel = t.select(opts, GPOS_DEFAULTS);
            let mut kf = Features::new();
            kf.set(*b"kern", 1);
            let ko = ShapeOptions { script: opts.script, language: opts.language, features: &kf, coords: opts.coords };
            gpos_kern = !t.select(&ko, &[]).is_empty();
            let mut cx = Ctx::new(self.gdef.as_ref(), t, true, buf, lig);
            for &(li, v, _) in &sel {
                cx.run(li as usize, v);
            }
            buf = cx.buf;
        }
        if !gpos_kern && opts.features.value(*b"kern") != Some(0) {
            if let Some(k) = &self.kern {
                for i in 0..buf.len().saturating_sub(1) {
                    buf[i].xa += k.pair(buf[i].gid, buf[i + 1].gid);
                }
            }
        }
        // Marks take no advance. Offsets compensate unless attachment lookups placed them.
        let attached = buf.iter().any(|g| g.attach.is_some());
        if let Some(gd) = &self.gdef {
            for g in buf.iter_mut().filter(|g| gd.class(g.gid) == 3) {
                if !attached {
                    g.xo -= g.xa;
                }
                g.xa = 0;
                g.ya = 0;
            }
        }
        for i in 0..buf.len() {
            let Some((p, cursive)) = buf[i].attach else { continue };
            if p >= i {
                continue;
            }
            let (pxo, pyo) = (buf[p].xo, buf[p].yo);
            buf[i].yo += pyo;
            if !cursive {
                buf[i].xo += pxo;
                for k in p..i {
                    buf[i].xo -= buf[k].xa;
                    buf[i].yo -= buf[k].ya;
                }
            }
        }
        buf.iter().map(|g| ShapedGlyph { gid: g.gid, cluster: g.cluster, x_advance: g.xa, y_advance: g.ya, x_offset: g.xo, y_offset: g.yo }).collect()
    }
}

#[derive(Clone, Default)]
struct G {
    gid: u16,
    cluster: u32,
    xa: i32,
    ya: i32,
    xo: i32,
    yo: i32,
    lig_id: u32,
    lig_comp: u16,
    /// (glyph this one is attached to, cursive)
    attach: Option<(usize, bool)>,
}

#[derive(Clone, Copy, Default)]
struct Val {
    xp: i32,
    yp: i32,
    xa: i32,
    ya: i32,
}

fn vsize(f: u16) -> usize {
    f.count_ones() as usize * 2
}

fn read_val(d: &[u8], o: usize, f: u16) -> Option<Val> {
    let mut v = Val::default();
    let mut p = o;
    for (bit, slot) in [(1, &mut v.xp), (2, &mut v.yp), (4, &mut v.xa), (8, &mut v.ya)] {
        if f & bit != 0 {
            *slot = i16_at(d, p)? as i32;
            p += 2;
        }
    }
    Some(v)
}

fn anchor(d: &[u8], o: usize) -> Option<(i32, i32)> {
    Some((i16_at(d, o + 2)? as i32, i16_at(d, o + 4)? as i32))
}

enum Kind {
    Glyph,
    Class(Option<usize>),
    Cover(usize),
}

fn test(d: &[u8], kind: &Kind, arr: usize, k: usize, gid: u16) -> bool {
    let Some(v) = u16_at(d, arr + 2 * k) else { return false };
    match kind {
        Kind::Glyph => v == gid,
        Kind::Class(cd) => cd.map_or(0, |c| class_def(d, c, gid)) == v,
        Kind::Cover(base) => coverage_index(d, base + v as usize, gid).is_some(),
    }
}

struct Ctx<'x> {
    gdef: Option<&'x Gdef<'x>>,
    d: &'x [u8],
    lookups: &'x [Lookup],
    gpos: bool,
    buf: Vec<G>,
    depth: u32,
    alt: u32,
    flag: u16,
    mfs: Option<u16>,
    next_lig: u32,
}

impl<'x> Ctx<'x> {
    fn new(gdef: Option<&'x Gdef<'x>>, t: &'x Table<'x>, gpos: bool, buf: Vec<G>, next_lig: u32) -> Self {
        Ctx { gdef, d: t.d, lookups: &t.lookups, gpos, buf, depth: 0, alt: 1, flag: 0, mfs: None, next_lig }
    }

    fn class(&self, gid: u16) -> u16 {
        self.gdef.map_or(0, |g| g.class(gid))
    }

    fn skip_gid(&self, gid: u16) -> bool {
        let f = self.flag;
        if f & 0xFF1E == 0 {
            return false;
        }
        let Some(g) = self.gdef else { return false };
        match g.class(gid) {
            1 => f & 2 != 0,
            2 => f & 4 != 0,
            3 => {
                if f & 8 != 0 {
                    return true;
                }
                if f & 0x10 != 0 {
                    return self.mfs.is_some_and(|s| !g.in_set(s, gid));
                }
                let m = f >> 8;
                m != 0 && g.mark_attach(gid) != m
            }
            _ => false,
        }
    }

    fn next_idx(&self, from: usize) -> Option<usize> {
        (from..self.buf.len()).find(|&j| !self.skip_gid(self.buf[j].gid))
    }

    fn prev_idx(&self, before: usize) -> Option<usize> {
        (0..before.min(self.buf.len())).rev().find(|&j| !self.skip_gid(self.buf[j].gid))
    }

    fn fwd(&self, start: usize, n: usize, t: &dyn Fn(usize, u16) -> bool) -> Option<Vec<usize>> {
        let mut out = Vec::with_capacity(n);
        let mut p = start;
        for k in 0..n {
            let j = self.next_idx(p)?;
            if !t(k, self.buf[j].gid) {
                return None;
            }
            out.push(j);
            p = j + 1;
        }
        Some(out)
    }

    fn back(&self, before: usize, n: usize, t: &dyn Fn(usize, u16) -> bool) -> Option<()> {
        let mut p = before;
        for k in 0..n {
            let j = self.prev_idx(p)?;
            if !t(k, self.buf[j].gid) {
                return None;
            }
            p = j;
        }
        Some(())
    }

    fn run(&mut self, li: usize, alt: u32) {
        let lookups = self.lookups;
        let Some(lk) = lookups.get(li) else { return };
        self.alt = alt;
        let (sf, sm) = (self.flag, self.mfs);
        self.flag = lk.flag;
        self.mfs = lk.mfs;
        if !self.gpos && lk.subs.first().is_some_and(|s| s.0 == 8) {
            for i in (0..self.buf.len()).rev() {
                if self.skip_gid(self.buf[i].gid) {
                    continue;
                }
                for &(_, o) in &lk.subs {
                    if self.reverse_sub(o, i) {
                        break;
                    }
                }
            }
        } else {
            let mut i = 0;
            while i < self.buf.len() {
                if self.skip_gid(self.buf[i].gid) {
                    i += 1;
                    continue;
                }
                let before = self.buf.len();
                match self.apply_at(li, i) {
                    Some(n) if n > i || self.buf.len() < before => i = n,
                    _ => i += 1,
                }
            }
        }
        self.flag = sf;
        self.mfs = sm;
    }

    fn apply_at(&mut self, li: usize, pos: usize) -> Option<usize> {
        let lookups = self.lookups;
        let lk = lookups.get(li)?;
        let (sf, sm) = (self.flag, self.mfs);
        self.flag = lk.flag;
        self.mfs = lk.mfs;
        let mut res = None;
        for &(ty, off) in &lk.subs {
            if let Some(n) = self.apply_sub(ty, off, pos) {
                res = Some(n);
                break;
            }
        }
        self.flag = sf;
        self.mfs = sm;
        res
    }

    fn apply_sub(&mut self, ty: u16, o: usize, i: usize) -> Option<usize> {
        if self.gpos {
            match ty {
                1 => self.pos_single(o, i),
                2 => self.pos_pair(o, i),
                3 => self.pos_cursive(o, i),
                4 => self.pos_mark(o, i, false),
                5 => self.pos_mark(o, i, true),
                6 => self.pos_mark_mark(o, i),
                7 => self.seq_ctx(o, i),
                8 => self.chain_ctx(o, i),
                _ => None,
            }
        } else {
            match ty {
                1 => self.sub_single(o, i),
                2 => self.sub_multiple(o, i),
                3 => self.sub_alternate(o, i),
                4 => self.sub_ligature(o, i),
                5 => self.seq_ctx(o, i),
                6 => self.chain_ctx(o, i),
                _ => None,
            }
        }
    }

    // ---- substitution ----

    fn sub_single(&mut self, o: usize, i: usize) -> Option<usize> {
        let d = self.d;
        let gid = self.buf[i].gid;
        let ci = coverage_index(d, o + u16_at(d, o + 2)? as usize, gid)?;
        let new = match u16_at(d, o)? {
            1 => gid.wrapping_add(u16_at(d, o + 4)?),
            2 => {
                if ci >= u16_at(d, o + 4)? as usize {
                    return None;
                }
                u16_at(d, o + 6 + 2 * ci)?
            }
            _ => return None,
        };
        self.buf[i].gid = new;
        Some(i + 1)
    }

    fn sub_multiple(&mut self, o: usize, i: usize) -> Option<usize> {
        let d = self.d;
        if u16_at(d, o)? != 1 {
            return None;
        }
        let ci = coverage_index(d, o + u16_at(d, o + 2)? as usize, self.buf[i].gid)?;
        if ci >= u16_at(d, o + 4)? as usize {
            return None;
        }
        let seq = o + u16_at(d, o + 6 + 2 * ci)? as usize;
        let n = u16_at(d, seq)? as usize;
        let base = self.buf[i].clone();
        let mut repl = Vec::with_capacity(n);
        for k in 0..n {
            repl.push(G { gid: u16_at(d, seq + 2 + 2 * k)?, lig_id: 0, lig_comp: 0, attach: None, ..base.clone() });
        }
        self.buf.splice(i..i + 1, repl);
        Some(i + n)
    }

    fn sub_alternate(&mut self, o: usize, i: usize) -> Option<usize> {
        let d = self.d;
        if u16_at(d, o)? != 1 {
            return None;
        }
        let ci = coverage_index(d, o + u16_at(d, o + 2)? as usize, self.buf[i].gid)?;
        if ci >= u16_at(d, o + 4)? as usize {
            return None;
        }
        let set = o + u16_at(d, o + 6 + 2 * ci)? as usize;
        let n = u16_at(d, set)? as usize;
        let idx = self.alt as usize;
        if idx == 0 || idx > n {
            return None;
        }
        self.buf[i].gid = u16_at(d, set + 2 * idx)?;
        Some(i + 1)
    }

    fn sub_ligature(&mut self, o: usize, i: usize) -> Option<usize> {
        let d = self.d;
        if u16_at(d, o)? != 1 {
            return None;
        }
        let ci = coverage_index(d, o + u16_at(d, o + 2)? as usize, self.buf[i].gid)?;
        if ci >= u16_at(d, o + 4)? as usize {
            return None;
        }
        let set = o + u16_at(d, o + 6 + 2 * ci)? as usize;
        for r in 0..u16_at(d, set)? as usize {
            let lig = set + u16_at(d, set + 2 + 2 * r)? as usize;
            let glyph = u16_at(d, lig)?;
            let cc = u16_at(d, lig + 2)? as usize;
            if cc == 0 {
                continue;
            }
            let comps = lig + 4;
            let Some(rest) = self.fwd(i + 1, cc - 1, &|k, g| u16_at(d, comps + 2 * k) == Some(g)) else { continue };
            let mut all = vec![i];
            all.extend(rest);
            self.ligate(&all, glyph);
            return Some(i + 1);
        }
        None
    }

    fn ligate(&mut self, all: &[usize], glyph: u16) {
        self.next_lig += 1;
        let id = self.next_lig;
        for w in 0..all.len() {
            let from = all[w] + 1;
            let to = if w + 1 < all.len() {
                all[w + 1]
            } else {
                let mut e = from;
                while e < self.buf.len() && self.class(self.buf[e].gid) == 3 {
                    e += 1;
                }
                e
            };
            for m in from..to {
                if self.class(self.buf[m].gid) == 3 {
                    self.buf[m].lig_id = id;
                    self.buf[m].lig_comp = (w + 1) as u16;
                }
            }
        }
        let first = all[0];
        self.buf[first].gid = glyph;
        self.buf[first].lig_id = id;
        self.buf[first].lig_comp = 0;
        for &j in all[1..].iter().rev() {
            self.buf.remove(j);
        }
    }

    fn reverse_sub(&mut self, o: usize, i: usize) -> bool {
        let d = self.d;
        let go = || -> Option<u16> {
            if u16_at(d, o)? != 1 {
                return None;
            }
            let ci = coverage_index(d, o + u16_at(d, o + 2)? as usize, self.buf[i].gid)?;
            let bc = u16_at(d, o + 4)? as usize;
            let bt = o + 6;
            let lc_at = bt + 2 * bc;
            let lc = u16_at(d, lc_at)? as usize;
            let la = lc_at + 2;
            let gc_at = la + 2 * lc;
            if ci >= u16_at(d, gc_at)? as usize {
                return None;
            }
            self.back(i, bc, &|k, g| u16_at(d, bt + 2 * k).is_some_and(|v| coverage_index(d, o + v as usize, g).is_some()))?;
            self.fwd(i + 1, lc, &|k, g| u16_at(d, la + 2 * k).is_some_and(|v| coverage_index(d, o + v as usize, g).is_some()))?;
            u16_at(d, gc_at + 2 + 2 * ci)
        };
        match go() {
            Some(g) => {
                self.buf[i].gid = g;
                true
            }
            None => false,
        }
    }

    // ---- contextual (shared by substitution and positioning) ----

    #[allow(clippy::too_many_arguments)]
    fn apply_rule(&mut self, i: usize, back: (&Kind, usize, usize), inp: (&Kind, usize, usize, bool), look: (&Kind, usize, usize), recs: (usize, usize)) -> Option<usize> {
        let d = self.d;
        let (ik, ia, ic, first_in) = inp;
        let rest_arr = if first_in {
            if !test(d, ik, ia, 0, self.buf[i].gid) {
                return None;
            }
            ia + 2
        } else {
            ia
        };
        let rest = self.fwd(i + 1, ic.saturating_sub(1), &|k, g| test(d, ik, rest_arr, k, g))?;
        let last = rest.last().copied().unwrap_or(i);
        let (bk, ba, bn) = back;
        self.back(i, bn, &|k, g| test(d, bk, ba, k, g))?;
        let (lk, la, ln) = look;
        self.fwd(last + 1, ln, &|k, g| test(d, lk, la, k, g))?;
        let mut pos = vec![i];
        pos.extend(rest);
        for r in 0..recs.1 {
            let ro = recs.0 + 4 * r;
            let (si, li) = (u16_at(d, ro)? as usize, u16_at(d, ro + 2)? as usize);
            if si >= pos.len() || pos[si] >= self.buf.len() {
                continue;
            }
            let before = self.buf.len() as isize;
            if self.depth < MAX_NEST {
                self.depth += 1;
                let p = pos[si];
                self.apply_at(li, p);
                self.depth -= 1;
            }
            let delta = self.buf.len() as isize - before;
            if delta != 0 {
                for q in pos.iter_mut().skip(si + 1) {
                    *q = (*q as isize + delta).max(0) as usize;
                }
            }
        }
        Some((pos[pos.len() - 1] + 1).max(i + 1).min(self.buf.len().max(i + 1)))
    }

    fn seq_ctx(&mut self, o: usize, i: usize) -> Option<usize> {
        let d = self.d;
        let gid = self.buf[i].gid;
        let none = Kind::Glyph;
        match u16_at(d, o)? {
            1 | 2 => {
                let fmt = u16_at(d, o)?;
                let ci = coverage_index(d, o + u16_at(d, o + 2)? as usize, gid)?;
                let (set_idx, count_at, sets, kind) = if fmt == 1 {
                    (ci, o + 4, o + 6, Kind::Glyph)
                } else {
                    let cd = o + u16_at(d, o + 4)? as usize;
                    (class_def(d, cd, gid) as usize, o + 6, o + 8, Kind::Class(Some(cd)))
                };
                if set_idx >= u16_at(d, count_at)? as usize {
                    return None;
                }
                let off = u16_at(d, sets + 2 * set_idx)?;
                if off == 0 {
                    return None;
                }
                let rs = o + off as usize;
                for r in 0..u16_at(d, rs)? as usize {
                    let ru = rs + u16_at(d, rs + 2 + 2 * r)? as usize;
                    let (gc, sc) = (u16_at(d, ru)? as usize, u16_at(d, ru + 2)? as usize);
                    let input = ru + 4;
                    let recs = input + 2 * gc.saturating_sub(1);
                    if let Some(x) = self.apply_rule(i, (&none, 0, 0), (&kind, input, gc, false), (&none, 0, 0), (recs, sc)) {
                        return Some(x);
                    }
                }
                None
            }
            3 => {
                let (gc, sc) = (u16_at(d, o + 2)? as usize, u16_at(d, o + 4)? as usize);
                let covs = o + 6;
                self.apply_rule(i, (&none, 0, 0), (&Kind::Cover(o), covs, gc, true), (&none, 0, 0), (covs + 2 * gc, sc))
            }
            _ => None,
        }
    }

    fn chain_rule(&mut self, i: usize, ru: usize, bk: &Kind, ik: &Kind, lk: &Kind) -> Option<usize> {
        let d = self.d;
        let bc = u16_at(d, ru)? as usize;
        let back = ru + 2;
        let ic = u16_at(d, back + 2 * bc)? as usize;
        let input = back + 2 * bc + 2;
        let lc_at = input + 2 * ic.saturating_sub(1);
        let lc = u16_at(d, lc_at)? as usize;
        let look = lc_at + 2;
        let sc_at = look + 2 * lc;
        let sc = u16_at(d, sc_at)? as usize;
        self.apply_rule(i, (bk, back, bc), (ik, input, ic, false), (lk, look, lc), (sc_at + 2, sc))
    }

    fn chain_ctx(&mut self, o: usize, i: usize) -> Option<usize> {
        let d = self.d;
        let gid = self.buf[i].gid;
        match u16_at(d, o)? {
            1 => {
                let ci = coverage_index(d, o + u16_at(d, o + 2)? as usize, gid)?;
                if ci >= u16_at(d, o + 4)? as usize {
                    return None;
                }
                let off = u16_at(d, o + 6 + 2 * ci)?;
                if off == 0 {
                    return None;
                }
                let rs = o + off as usize;
                for r in 0..u16_at(d, rs)? as usize {
                    let ru = rs + u16_at(d, rs + 2 + 2 * r)? as usize;
                    if let Some(x) = self.chain_rule(i, ru, &Kind::Glyph, &Kind::Glyph, &Kind::Glyph) {
                        return Some(x);
                    }
                }
                None
            }
            2 => {
                coverage_index(d, o + u16_at(d, o + 2)? as usize, gid)?;
                let cd = |at: usize| u16_at(d, at).filter(|&v| v != 0).map(|v| o + v as usize);
                let (bcd, icd, lcd) = (cd(o + 4), cd(o + 6), cd(o + 8));
                let c = icd.map_or(0, |c| class_def(d, c, gid)) as usize;
                if c >= u16_at(d, o + 10)? as usize {
                    return None;
                }
                let off = u16_at(d, o + 12 + 2 * c)?;
                if off == 0 {
                    return None;
                }
                let rs = o + off as usize;
                for r in 0..u16_at(d, rs)? as usize {
                    let ru = rs + u16_at(d, rs + 2 + 2 * r)? as usize;
                    if let Some(x) = self.chain_rule(i, ru, &Kind::Class(bcd), &Kind::Class(icd), &Kind::Class(lcd)) {
                        return Some(x);
                    }
                }
                None
            }
            3 => {
                let bc = u16_at(d, o + 2)? as usize;
                let back = o + 4;
                let ic_at = back + 2 * bc;
                let ic = u16_at(d, ic_at)? as usize;
                let input = ic_at + 2;
                let lc_at = input + 2 * ic;
                let lc = u16_at(d, lc_at)? as usize;
                let look = lc_at + 2;
                let sc_at = look + 2 * lc;
                let sc = u16_at(d, sc_at)? as usize;
                let k = Kind::Cover(o);
                self.apply_rule(i, (&k, back, bc), (&k, input, ic, true), (&k, look, lc), (sc_at + 2, sc))
            }
            _ => None,
        }
    }

    // ---- positioning ----

    fn add_val(&mut self, i: usize, v: Val) {
        let g = &mut self.buf[i];
        g.xo += v.xp;
        g.yo += v.yp;
        g.xa += v.xa;
        g.ya += v.ya;
    }

    fn pos_single(&mut self, o: usize, i: usize) -> Option<usize> {
        let d = self.d;
        let ci = coverage_index(d, o + u16_at(d, o + 2)? as usize, self.buf[i].gid)?;
        let vf = u16_at(d, o + 4)?;
        let v = match u16_at(d, o)? {
            1 => read_val(d, o + 6, vf)?,
            2 => {
                if ci >= u16_at(d, o + 6)? as usize {
                    return None;
                }
                read_val(d, o + 8 + ci * vsize(vf), vf)?
            }
            _ => return None,
        };
        self.add_val(i, v);
        Some(i + 1)
    }

    fn pos_pair(&mut self, o: usize, i: usize) -> Option<usize> {
        let d = self.d;
        let gid = self.buf[i].gid;
        let ci = coverage_index(d, o + u16_at(d, o + 2)? as usize, gid)?;
        let j = self.next_idx(i + 1)?;
        let g2 = self.buf[j].gid;
        let (vf1, vf2) = (u16_at(d, o + 4)?, u16_at(d, o + 6)?);
        let (s1, s2) = (vsize(vf1), vsize(vf2));
        let (v1, v2) = match u16_at(d, o)? {
            1 => {
                if ci >= u16_at(d, o + 8)? as usize {
                    return None;
                }
                let ps = o + u16_at(d, o + 10 + 2 * ci)? as usize;
                let rec = 2 + s1 + s2;
                let (mut lo, mut hi) = (0, u16_at(d, ps)? as usize);
                let mut found = None;
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    let p = ps + 2 + rec * mid;
                    let g = u16_at(d, p)?;
                    if g2 < g {
                        hi = mid
                    } else if g2 > g {
                        lo = mid + 1
                    } else {
                        found = Some(p);
                        break;
                    }
                }
                let p = found?;
                (read_val(d, p + 2, vf1)?, read_val(d, p + 2 + s1, vf2)?)
            }
            2 => {
                let c1 = class_rel(d, o, u16_at(d, o + 8)?, gid) as usize;
                let c2 = class_rel(d, o, u16_at(d, o + 10)?, g2) as usize;
                let (n1, n2) = (u16_at(d, o + 12)? as usize, u16_at(d, o + 14)? as usize);
                if c1 >= n1 || c2 >= n2 {
                    return None;
                }
                let p = o + 16 + (c1 * n2 + c2) * (s1 + s2);
                (read_val(d, p, vf1)?, read_val(d, p + s1, vf2)?)
            }
            _ => return None,
        };
        self.add_val(i, v1);
        self.add_val(j, v2);
        Some(if vf2 != 0 { j + 1 } else { j })
    }

    fn pos_cursive(&mut self, o: usize, i: usize) -> Option<usize> {
        let d = self.d;
        if u16_at(d, o)? != 1 {
            return None;
        }
        let cov = o + u16_at(d, o + 2)? as usize;
        let n = u16_at(d, o + 4)? as usize;
        let ci = coverage_index(d, cov, self.buf[i].gid)?;
        if ci >= n {
            return None;
        }
        let entry_off = u16_at(d, o + 6 + 4 * ci)?;
        if entry_off == 0 {
            return None;
        }
        let entry = anchor(d, o + entry_off as usize)?;
        let j = self.prev_idx(i)?;
        let pi = coverage_index(d, cov, self.buf[j].gid)?;
        if pi >= n {
            return None;
        }
        let exit_off = u16_at(d, o + 8 + 4 * pi)?;
        if exit_off == 0 {
            return None;
        }
        let exit = anchor(d, o + exit_off as usize)?;
        self.buf[j].xa = exit.0 + self.buf[j].xo;
        let dx = entry.0 + self.buf[i].xo;
        self.buf[i].xa -= dx;
        self.buf[i].xo -= dx;
        self.buf[i].yo = exit.1 - entry.1;
        self.buf[i].attach = Some((j, true));
        Some(i + 1)
    }

    fn pos_mark(&mut self, o: usize, i: usize, lig: bool) -> Option<usize> {
        let d = self.d;
        if u16_at(d, o)? != 1 {
            return None;
        }
        let mi = coverage_index(d, o + u16_at(d, o + 2)? as usize, self.buf[i].gid)?;
        let mcc = u16_at(d, o + 6)? as usize;
        let j = (0..i).rev().find(|&j| self.class(self.buf[j].gid) != 3)?;
        let bi = coverage_index(d, o + u16_at(d, o + 4)? as usize, self.buf[j].gid)?;
        let ma = o + u16_at(d, o + 8)? as usize;
        if mi >= u16_at(d, ma)? as usize {
            return None;
        }
        let mclass = u16_at(d, ma + 2 + 4 * mi)? as usize;
        if mclass >= mcc {
            return None;
        }
        let m_anchor = anchor(d, ma + u16_at(d, ma + 4 + 4 * mi)? as usize)?;
        let ba = o + u16_at(d, o + 10)? as usize;
        if bi >= u16_at(d, ba)? as usize {
            return None;
        }
        let (anchor_base, rel) = if lig {
            let la = ba + u16_at(d, ba + 2 + 2 * bi)? as usize;
            let cc = u16_at(d, la)? as usize;
            if cc == 0 {
                return None;
            }
            let (bg, mg) = (&self.buf[j], &self.buf[i]);
            let comp = if mg.lig_id != 0 && mg.lig_id == bg.lig_id && mg.lig_comp > 0 { (mg.lig_comp as usize).min(cc) - 1 } else { cc - 1 };
            (la, u16_at(d, la + 2 + 2 * (comp * mcc + mclass))?)
        } else {
            (ba, u16_at(d, ba + 2 + 2 * (bi * mcc + mclass))?)
        };
        if rel == 0 {
            return None;
        }
        let b_anchor = anchor(d, anchor_base + rel as usize)?;
        self.buf[i].xo = b_anchor.0 - m_anchor.0;
        self.buf[i].yo = b_anchor.1 - m_anchor.1;
        self.buf[i].attach = Some((j, false));
        Some(i + 1)
    }

    fn pos_mark_mark(&mut self, o: usize, i: usize) -> Option<usize> {
        let d = self.d;
        if u16_at(d, o)? != 1 {
            return None;
        }
        let mi = coverage_index(d, o + u16_at(d, o + 2)? as usize, self.buf[i].gid)?;
        let j = self.prev_idx(i)?;
        let bi = coverage_index(d, o + u16_at(d, o + 4)? as usize, self.buf[j].gid)?;
        let (a, b) = (&self.buf[i], &self.buf[j]);
        let same = if a.lig_id == b.lig_id {
            a.lig_id == 0 || a.lig_comp == b.lig_comp
        } else {
            (a.lig_id != 0 && a.lig_comp == 0) || (b.lig_id != 0 && b.lig_comp == 0)
        };
        if !same {
            return None;
        }
        let mcc = u16_at(d, o + 6)? as usize;
        let ma = o + u16_at(d, o + 8)? as usize;
        if mi >= u16_at(d, ma)? as usize {
            return None;
        }
        let mclass = u16_at(d, ma + 2 + 4 * mi)? as usize;
        if mclass >= mcc {
            return None;
        }
        let m_anchor = anchor(d, ma + u16_at(d, ma + 4 + 4 * mi)? as usize)?;
        let m2 = o + u16_at(d, o + 10)? as usize;
        if bi >= u16_at(d, m2)? as usize {
            return None;
        }
        let at = u16_at(d, m2 + 2 + 2 * (bi * mcc + mclass))?;
        if at == 0 {
            return None;
        }
        let b_anchor = anchor(d, m2 + at as usize)?;
        self.buf[i].xo = b_anchor.0 - m_anchor.0;
        self.buf[i].yo = b_anchor.1 - m_anchor.1;
        self.buf[i].attach = Some((j, false));
        Some(i + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16s(v: &[u16]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_be_bytes()).collect()
    }

    /// Joins `parts` after a header of `hdr(offsets)`; offsets are each part's start from the table start.
    fn build(parts: &[Vec<u8>], hdr: impl Fn(&[u16]) -> Vec<u16>) -> Vec<u8> {
        let hl = hdr(&vec![0; parts.len()]).len() * 2;
        let mut offs = Vec::new();
        let mut at = hl;
        for p in parts {
            offs.push(at as u16);
            at += p.len();
        }
        let mut out = u16s(&hdr(&offs));
        for p in parts {
            out.extend(p);
        }
        out
    }

    fn cov(gs: &[u16]) -> Vec<u8> {
        let mut v = u16s(&[1, gs.len() as u16]);
        v.extend(u16s(gs));
        v
    }

    fn lookup(ty: u16, flag: u16, subs: Vec<Vec<u8>>) -> Vec<u8> {
        build(&subs, |o| {
            let mut h = vec![ty, flag, o.len() as u16];
            h.extend(o);
            h
        })
    }

    /// A GSUB/GPOS table: one script (`script`) whose default language lists `feats`.
    fn layout(script: &[u8; 4], feats: &[(&str, Vec<u16>)], lookups: Vec<Vec<u8>>, var: Option<Vec<u8>>) -> Vec<u8> {
        let fidx: Vec<u16> = (0..feats.len() as u16).collect();
        let mut langsys = u16s(&[0, 0xFFFF, fidx.len() as u16]);
        langsys.extend(u16s(&fidx));
        let mut script_tbl = u16s(&[4, 0]);
        script_tbl.extend(&langsys);
        let mut sl = u16s(&[1]);
        sl.extend(script);
        sl.extend(u16s(&[8]));
        sl.extend(&script_tbl);
        // feature list
        let tables: Vec<Vec<u8>> = feats
            .iter()
            .map(|(_, l)| {
                let mut t = u16s(&[0, l.len() as u16]);
                t.extend(u16s(l));
                t
            })
            .collect();
        let mut fl = u16s(&[feats.len() as u16]);
        let mut at = 2 + 6 * feats.len();
        for (k, (t, _)) in feats.iter().enumerate() {
            fl.extend(tag(t));
            fl.extend(u16s(&[at as u16]));
            at += tables[k].len();
        }
        for t in &tables {
            fl.extend(t);
        }
        // lookup list
        let mut ll = u16s(&[lookups.len() as u16]);
        let mut at = 2 + 2 * lookups.len();
        for l in &lookups {
            ll.extend(u16s(&[at as u16]));
            at += l.len();
        }
        for l in &lookups {
            ll.extend(l);
        }
        let hl = if var.is_some() { 14 } else { 10 };
        let (s_off, f_off, l_off) = (hl, hl + sl.len(), hl + sl.len() + fl.len());
        let mut out = if var.is_some() { vec![0, 1, 0, 1] } else { vec![0, 1, 0, 0] };
        out.extend(u16s(&[s_off as u16, f_off as u16, l_off as u16]));
        if var.is_some() {
            out.extend([0u8; 4]);
        }
        out.extend(sl);
        out.extend(fl);
        out.extend(ll);
        if let Some(v) = var {
            out.extend(v);
        }
        out
    }

    fn single(from: &[u16], delta: i16) -> Vec<u8> {
        let mut v = u16s(&[1, 6, delta as u16]);
        v.extend(cov(from));
        v
    }

    fn opts<'f>(f: &'f Features) -> ShapeOptions<'f> {
        ShapeOptions { script: *b"latn", language: None, features: f, coords: &[] }
    }

    fn run(gsub: Option<Vec<u8>>, gpos: Option<Vec<u8>>, gdef: Option<Vec<u8>>, kern: Option<Vec<u8>>, input: &[u16], f: &Features) -> Vec<ShapedGlyph> {
        let l = OtLayout::from_tables(gdef.as_deref(), gsub.as_deref(), gpos.as_deref(), kern.as_deref());
        let inp: Vec<(u16, u32)> = input.iter().enumerate().map(|(i, &g)| (g, i as u32)).collect();
        l.shape_glyphs(&inp, &|_| 500, &opts(f))
    }

    fn gids(v: &[ShapedGlyph]) -> Vec<u16> {
        v.iter().map(|g| g.gid).collect()
    }

    fn lig_sub(out: u16, comps: &[u16]) -> Vec<u8> {
        let lig = {
            let mut v = u16s(&[out, comps.len() as u16 + 1]);
            v.extend(u16s(comps));
            v
        };
        let set = {
            let mut v = u16s(&[1, 4]);
            v.extend(&lig);
            v
        };
        build(&[set, cov(&[10])], |o| vec![1, o[1], 1, o[0]])
    }

    #[test]
    fn single_substitution_follows_the_feature_and_user_off() {
        let gsub = layout(b"latn", &[("smcp", vec![0])], vec![lookup(1, 0, vec![single(&[1, 2], 10)])], None);
        let mut f = Features::new();
        assert_eq!(gids(&run(Some(gsub.clone()), None, None, None, &[1, 5, 2], &f)), vec![1, 5, 2], "smcp is not default");
        f.on("smcp");
        assert_eq!(gids(&run(Some(gsub), None, None, None, &[1, 5, 2], &f)), vec![11, 5, 12]);
    }

    #[test]
    fn ligature_is_default_on_and_clusters_merge() {
        let gsub = layout(b"latn", &[("liga", vec![0])], vec![lookup(4, 0, vec![lig_sub(99, &[11, 12])])], None);
        let out = run(Some(gsub.clone()), None, None, None, &[10, 11, 12, 13], &Features::new());
        assert_eq!(gids(&out), vec![99, 13]);
        assert_eq!((out[0].cluster, out[1].cluster), (0, 3));
        let off = run(Some(gsub), None, None, None, &[10, 11, 12], Features::new().off("liga"));
        assert_eq!(gids(&off), vec![10, 11, 12]);
    }

    #[test]
    fn multiple_and_alternate_substitution() {
        let seq = u16s(&[3, 21, 22, 23]);
        let multi = build(&[seq, cov(&[20])], |o| vec![1, o[1], 1, o[0]]);
        let alt_set = u16s(&[2, 31, 32]);
        let alt = build(&[alt_set, cov(&[30])], |o| vec![1, o[1], 1, o[0]]);
        let gsub = layout(b"latn", &[("ccmp", vec![0]), ("salt", vec![1])], vec![lookup(2, 0, vec![multi]), lookup(3, 0, vec![alt])], None);
        let out = run(Some(gsub.clone()), None, None, None, &[20, 30], &Features::new());
        assert_eq!(gids(&out), vec![21, 22, 23, 30]);
        assert!(out[..3].iter().all(|g| g.cluster == 0));
        let mut f = Features::new();
        f.set(tag("salt"), 2);
        assert_eq!(gids(&run(Some(gsub), None, None, None, &[30], &f)), vec![32]);
    }

    #[test]
    fn chaining_context_runs_nested_lookup() {
        // after glyph 1, replace 2 with 3 (via lookup 0), only when followed by 4
        let k = |gs: &[u16]| cov(gs);
        let ctx = build(&[k(&[1]), k(&[2]), k(&[4])], |o| {
            vec![3, 1, o[0], 1, o[1], 1, o[2], 1, 0, 0]
        });
        let gsub = layout(b"latn", &[("calt", vec![1])], vec![lookup(1, 0, vec![single(&[2], 1)]), lookup(6, 0, vec![ctx])], None);
        assert_eq!(gids(&run(Some(gsub.clone()), None, None, None, &[1, 2, 4], &Features::new())), vec![1, 3, 4]);
        assert_eq!(gids(&run(Some(gsub), None, None, None, &[1, 2, 5], &Features::new())), vec![1, 2, 5]);
    }

    #[test]
    fn extension_lookup_resolves() {
        let inner = single(&[1], 5);
        let mut ext = u16s(&[1, 1]);
        ext.extend(8u32.to_be_bytes());
        ext.extend(&inner);
        let gsub = layout(b"latn", &[("liga", vec![0])], vec![lookup(7, 0, vec![ext])], None);
        assert_eq!(gids(&run(Some(gsub), None, None, None, &[1], &Features::new())), vec![6]);
    }

    #[test]
    fn reverse_chaining_substitutes() {
        // glyph 1 becomes 9 when followed by glyph 2
        let sub = build(&[cov(&[1]), cov(&[2])], |o| vec![1, o[0], 0, 1, o[1], 1, 9]);
        let gsub = layout(b"latn", &[("rclt", vec![0])], vec![lookup(8, 0, vec![sub])], None);
        assert_eq!(gids(&run(Some(gsub.clone()), None, None, None, &[1, 2], &Features::new())), vec![9, 2]);
        assert_eq!(gids(&run(Some(gsub), None, None, None, &[1, 3], &Features::new())), vec![1, 3]);
    }

    fn pair_fmt1(first: u16, second: u16, xa: i16) -> Vec<u8> {
        let ps = u16s(&[1, second, xa as u16]);
        build(&[ps, cov(&[first])], |o| vec![1, o[1], 4, 0, 1, o[0]])
    }

    #[test]
    fn pair_kerning_and_user_off() {
        let gpos = layout(b"latn", &[("kern", vec![0])], vec![lookup(2, 0, vec![pair_fmt1(1, 2, -80)])], None);
        let out = run(None, Some(gpos.clone()), None, None, &[1, 2], &Features::new());
        assert_eq!((out[0].x_advance, out[1].x_advance), (420, 500));
        let off = run(None, Some(gpos), None, None, &[1, 2], Features::new().off("kern"));
        assert_eq!(off[0].x_advance, 500);
    }

    #[test]
    fn class_pair_kerning() {
        let cd1 = u16s(&[1, 1, 1, 1]); // glyph 1 in class 1
        let cd2 = u16s(&[1, 2, 1, 1]); // glyph 2 in class 1
        let records = u16s(&[0, 0, 0, (-30i16) as u16]); // [c1][c2] with 2 classes each, value xAdvance (vf1=4)
        let sub = build(&[records, cd1, cd2, cov(&[1])], |o| vec![2, o[3], 4, 0, o[1], o[2], 2, 2]);
        let gpos = layout(b"latn", &[("kern", vec![0])], vec![lookup(2, 0, vec![sub])], None);
        let out = run(None, Some(gpos), None, None, &[1, 2], &Features::new());
        assert_eq!(out[0].x_advance, 470);
    }

    #[test]
    fn legacy_kern_table_is_the_fallback() {
        let mut kern: Vec<u8> = vec![0, 0, 0, 1, 0, 0];
        kern.extend(u16s(&[6 + 8 + 6, 1, 1]));
        kern.extend([0; 6]);
        kern.extend(u16s(&[1, 2, (-40i16) as u16]));
        let out = run(None, None, None, Some(kern.clone()), &[1, 2], &Features::new());
        assert_eq!(out[0].x_advance, 460);
        let gpos = layout(b"latn", &[("kern", vec![0])], vec![lookup(2, 0, vec![pair_fmt1(1, 2, -80)])], None);
        let both = run(None, Some(gpos), None, Some(kern), &[1, 2], &Features::new());
        assert_eq!(both[0].x_advance, 420, "GPOS kerning replaces the legacy table");
    }

    fn gdef_marks(marks: &[u16]) -> Vec<u8> {
        // glyph class def format 2 with one range per mark glyph: class 3
        let mut cd = u16s(&[2, marks.len() as u16]);
        for &m in marks {
            cd.extend(u16s(&[m, m, 3]));
        }
        build(&[cd], |o| vec![1, 0, o[0], 0, 0, 0])
    }

    #[test]
    fn mark_to_base_attaches_and_zeroes_the_mark_advance() {
        // base glyph 1 anchor (300, 700); mark glyph 50 anchor (100, 0)
        let anchor_fn = |x: i16, y: i16| u16s(&[1, x as u16, y as u16]);
        let mark_array = {
            let mut v = u16s(&[1, 0, 6]);
            v.extend(anchor_fn(100, 0));
            v
        };
        let base_array = {
            let mut v = u16s(&[1, 4]);
            v.extend(anchor_fn(300, 700));
            v
        };
        let sub = build(&[cov(&[50]), cov(&[1]), mark_array, base_array], |o| vec![1, o[0], o[1], 1, o[2], o[3]]);
        let gpos = layout(b"latn", &[("mark", vec![0])], vec![lookup(4, 0, vec![sub])], None);
        let out = run(None, Some(gpos), Some(gdef_marks(&[50])), None, &[1, 50], &Features::new());
        assert_eq!(out[1].x_advance, 0);
        // mark origin sits after the base advance (500), so the offset is 300 - 100 - 500
        assert_eq!((out[1].x_offset, out[1].y_offset), (-300, 700));
    }

    #[test]
    fn cursive_attachment_joins_exit_and_entry() {
        let anchor_fn = |x: i16, y: i16| u16s(&[1, x as u16, y as u16]);
        // glyph 1: entry (0,0) exit (400, 50); glyph 2: entry (30, 10) exit (500, 0)
        let mut parts = vec![cov(&[1, 2])];
        for a in [(0, 0), (400, 50), (30, 10), (500, 0)] {
            parts.push(anchor_fn(a.0, a.1));
        }
        let sub = build(&parts, |o| vec![1, o[0], 2, o[1], o[2], o[3], o[4]]);
        let gpos = layout(b"latn", &[("curs", vec![0])], vec![lookup(3, 0, vec![sub])], None);
        let out = run(None, Some(gpos), None, None, &[1, 2], &Features::new());
        assert_eq!(out[0].x_advance, 400);
        assert_eq!(out[1].x_advance, 470);
        assert_eq!(out[1].x_offset, -30);
        assert_eq!(out[1].y_offset, 40);
    }

    #[test]
    fn feature_variation_swaps_the_feature_lookups() {
        // feature 0 (liga) -> lookup 0 (+1); under axis 0 in [0.5, 1.0] it uses lookup 1 (+2)
        let mut cond = u16s(&[1, 0]);
        cond.extend(8192i16.to_be_bytes());
        cond.extend(16384i16.to_be_bytes());
        let mut set = u16s(&[1]);
        set.extend(6u32.to_be_bytes());
        set.extend(&cond);
        let alt_feature = u16s(&[0, 1, 1]);
        let mut subst = u16s(&[1, 0, 1, 0]);
        subst.extend(12u32.to_be_bytes());
        subst.extend(&alt_feature);
        let mut var = u16s(&[1, 0]);
        var.extend(1u32.to_be_bytes());
        let set_off = 16u32;
        let sub_off = set_off + set.len() as u32;
        var.extend(set_off.to_be_bytes());
        var.extend(sub_off.to_be_bytes());
        var.extend(&set);
        var.extend(&subst);
        let mut gsub = layout(b"latn", &[("liga", vec![0])], vec![lookup(1, 0, vec![single(&[1], 1)]), lookup(1, 0, vec![single(&[1], 2)])], Some(vec![]));
        let var_at = gsub.len() as u32;
        gsub.extend(&var);
        gsub[10..14].copy_from_slice(&var_at.to_be_bytes());
        let l = OtLayout::from_tables(None, Some(&gsub), None, None);
        let f = Features::new();
        let at = |c: &[f32]| {
            let o = ShapeOptions { script: *b"latn", language: None, features: &f, coords: c };
            l.shape_glyphs(&[(1, 0)], &|_| 500, &o)[0].gid
        };
        assert_eq!(at(&[]), 2);
        assert_eq!(at(&[0.75]), 3);
    }

    #[test]
    fn features_parse_and_script_detect() {
        let f = Features::parse("liga=0, ss01, -kern, smcp=2");
        assert_eq!((f.value(tag("liga")), f.value(tag("ss01")), f.value(tag("kern")), f.value(tag("smcp"))), (Some(0), Some(1), Some(0), Some(2)));
        assert_eq!(detect_script("abc"), *b"latn");
        assert_eq!(detect_script("12 \u{43f}"), *b"cyrl");
    }

    #[test]
    fn missing_script_falls_back_to_default_and_empty_font_is_trivial() {
        let gsub = layout(b"DFLT", &[("liga", vec![0])], vec![lookup(1, 0, vec![single(&[1], 1)])], None);
        assert_eq!(gids(&run(Some(gsub), None, None, None, &[1], &Features::new())), vec![2]);
        assert!(OtLayout::from_tables(None, None, None, None).is_trivial());
    }
}
