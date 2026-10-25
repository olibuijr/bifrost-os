//! CFF outlines for OpenType fonts with a `CFF ` table (OTTO): INDEX and
//! DICT parsing, the charset (glyph names) and a Type 2 charstring
//! interpreter that emits move/line/cubic path commands. CID-keyed fonts
//! pick each glyph's font DICT through FDSelect (formats 0 and 3), so its
//! local subroutines and widths come from that DICT's Private DICT; hints
//! are parsed and discarded.

/// Receives the path of one glyph in font units.
pub trait Sink {
    fn move_to(&mut self, x: f32, y: f32);
    fn line_to(&mut self, x: f32, y: f32);
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32);
    fn close(&mut self);
}

/// A CFF INDEX: `count` items whose byte ranges come from an offset array.
#[derive(Clone, Copy, Debug)]
struct Index {
    count: usize,
    off_size: usize,
    /// start of the offset array
    offsets: usize,
    /// byte position that offset 1 refers to
    base: usize,
    /// first byte after the whole INDEX
    end: usize,
}

impl Index {
    fn parse(d: &[u8], at: usize) -> Option<Index> {
        let count = u16::from_be_bytes(d.get(at..at + 2)?.try_into().ok()?) as usize;
        if count == 0 {
            return Some(Index { count, off_size: 0, offsets: at + 2, base: at + 2, end: at + 2 });
        }
        let off_size = *d.get(at + 2)? as usize;
        if !(1..=4).contains(&off_size) {
            return None;
        }
        let offsets = at + 3;
        let base = offsets + (count + 1) * off_size - 1;
        let last = read_offset(d, offsets + count * off_size, off_size)?;
        Some(Index { count, off_size, offsets, base, end: base + last })
    }

    fn get<'a>(&self, d: &'a [u8], i: usize) -> Option<&'a [u8]> {
        if i >= self.count {
            return None;
        }
        let a = read_offset(d, self.offsets + i * self.off_size, self.off_size)?;
        let b = read_offset(d, self.offsets + (i + 1) * self.off_size, self.off_size)?;
        d.get(self.base + a..self.base + b)
    }
}

fn read_offset(d: &[u8], at: usize, size: usize) -> Option<usize> {
    d.get(at..at + size)?.iter().try_fold(0usize, |acc, &b| Some(acc << 8 | b as usize))
}

/// Subroutine index bias (Type 2 charstrings, section 4.4).
fn bias(count: usize) -> i32 {
    if count < 1240 {
        107
    } else if count < 33900 {
        1131
    } else {
        32768
    }
}

/// Parses a DICT into (operator, operands); two-byte operators are 1200 + the second byte.
fn dict(d: &[u8]) -> Vec<(u16, Vec<f32>)> {
    let mut out = Vec::new();
    let mut ops = Vec::new();
    let mut p = 0;
    while p < d.len() {
        let b0 = d[p] as usize;
        p += 1;
        match b0 {
            0..=21 => {
                let op = if b0 == 12 {
                    let b1 = d.get(p).copied().unwrap_or(0);
                    p += 1;
                    1200 + b1 as u16
                } else {
                    b0 as u16
                };
                out.push((op, std::mem::take(&mut ops)));
            }
            28 => {
                ops.push(i16::from_be_bytes([d.get(p).copied().unwrap_or(0), d.get(p + 1).copied().unwrap_or(0)]) as f32);
                p += 2;
            }
            29 => {
                let v = d.get(p..p + 4).map(|s| i32::from_be_bytes([s[0], s[1], s[2], s[3]])).unwrap_or(0);
                ops.push(v as f32);
                p += 4;
            }
            30 => {
                let (v, n) = real(&d[p..]);
                ops.push(v);
                p += n;
            }
            32..=246 => ops.push(b0 as f32 - 139.0),
            247..=250 => {
                ops.push(((b0 as i32 - 247) * 256 + d.get(p).copied().unwrap_or(0) as i32 + 108) as f32);
                p += 1;
            }
            251..=254 => {
                ops.push((-(b0 as i32 - 251) * 256 - d.get(p).copied().unwrap_or(0) as i32 - 108) as f32);
                p += 1;
            }
            _ => {}
        }
    }
    out
}

/// A nibble-encoded real number; returns the value and the bytes consumed.
fn real(d: &[u8]) -> (f32, usize) {
    let mut s = String::new();
    let mut n = 0;
    'outer: for &b in d {
        n += 1;
        for nib in [b >> 4, b & 15] {
            match nib {
                0..=9 => s.push((b'0' + nib) as char),
                0xa => s.push('.'),
                0xb => s.push('E'),
                0xc => s.push_str("E-"),
                0xe => s.push('-'),
                0xf => break 'outer,
                _ => {}
            }
        }
    }
    (s.parse().unwrap_or(0.0), n)
}

/// What a charstring takes from its Private DICT: local subroutines and the width defaults.
#[derive(Clone, Copy, Debug, Default)]
struct FontDict {
    local_subrs: Option<Index>,
    nominal_width: f32,
    default_width: f32,
}

impl FontDict {
    /// Reads the Private DICT named by a `Private` (op 18) entry: `[size, offset]`, the offset from the table start.
    /// A missing or malformed entry gives no local subrs and zero widths.
    fn from_private(data: &[u8], entry: Option<&[f32]>) -> FontDict {
        let mut fd = FontDict::default();
        let Some(&[size, off]) = entry else { return fd };
        let (size, off) = (size as usize, off as usize);
        let Some(private) = data.get(off..off.saturating_add(size)) else { return fd };
        for (op, v) in dict(private) {
            match (op, v.first()) {
                // Subrs is relative to the Private DICT
                (19, Some(&s)) => fd.local_subrs = Index::parse(data, off + s as usize),
                (20, Some(&w)) => fd.default_width = w,
                (21, Some(&w)) => fd.nominal_width = w,
                _ => {}
            }
        }
        fd
    }
}

/// The font DICT index of glyph `gid` from the FDSelect table at `at` (formats 0 and 3).
fn fd_index(d: &[u8], at: usize, gid: usize) -> Option<usize> {
    let u16_at = |o: usize| d.get(o..o + 2).map(|s| u16::from_be_bytes([s[0], s[1]]) as usize);
    match *d.get(at)? {
        0 => d.get(at + 1 + gid).map(|&b| b as usize),
        3 => {
            // nRanges, then (first glyph u16, fd u8) per range, then a sentinel glyph id
            let n = u16_at(at + 1)?;
            let rec = |i: usize| at + 3 + 3 * i;
            if n == 0 || gid < u16_at(rec(0))? || gid >= u16_at(rec(n))? {
                return None;
            }
            // the last range whose first glyph is at most gid: first(lo) <= gid < first(hi)
            let (mut lo, mut hi) = (0, n);
            while hi - lo > 1 {
                let mid = (lo + hi) / 2;
                if u16_at(rec(mid))? <= gid {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            d.get(rec(lo) + 2).map(|&b| b as usize)
        }
        _ => None,
    }
}

pub struct Cff<'a> {
    data: &'a [u8],
    char_strings: Index,
    global_subrs: Index,
    strings: Index,
    /// charset table offset (0 = ISOAdobe: glyph i has SID i)
    charset: usize,
    /// the font DICTs: one (the top DICT's Private) for a name-keyed font, the FDArray for a CID-keyed one
    fds: Vec<FontDict>,
    /// FDSelect table offset (CID-keyed fonts); None means every glyph uses `fds[0]`
    fd_select: Option<usize>,
    /// CID-keyed (the charset maps glyphs to CIDs)
    is_cid: bool,
}

impl<'a> Cff<'a> {
    /// Parses the table (`data` starts at the CFF header).
    pub fn parse(data: &'a [u8]) -> Option<Self> {
        let hdr_size = *data.get(2)? as usize;
        let names = Index::parse(data, hdr_size)?;
        let top_dicts = Index::parse(data, names.end)?;
        let strings = Index::parse(data, top_dicts.end)?;
        let global_subrs = Index::parse(data, strings.end)?;
        let top = dict(top_dicts.get(data, 0)?);
        let find = |op: u16| top.iter().find(|(o, _)| *o == op).map(|(_, v)| v.as_slice());
        let char_strings = Index::parse(data, *find(17)?.first()? as usize)?;
        let charset = find(15).and_then(|v| v.first()).map_or(0, |&v| v as usize);
        let is_cid = find(1230).is_some();
        // a CID-keyed font keeps its Private DICTs (and so its local subrs and widths) per font DICT in the
        // FDArray (12 36), and FDSelect (12 37) names each glyph's font DICT
        let fd_array = is_cid.then(|| find(1236)).flatten().and_then(|v| v.first()).and_then(|&o| Index::parse(data, o as usize));
        let (fds, fd_select) = match fd_array {
            Some(fd_array) => {
                let fds = (0..fd_array.count)
                    .map(|i| {
                        let fd = fd_array.get(data, i).map(dict).unwrap_or_default();
                        FontDict::from_private(data, fd.iter().find(|(o, _)| *o == 18).map(|(_, v)| v.as_slice()))
                    })
                    .collect();
                (fds, find(1237).and_then(|v| v.first()).map(|&o| o as usize))
            }
            None => (vec![FontDict::from_private(data, find(18))], None),
        };
        Some(Cff { data, char_strings, global_subrs, strings, charset, fds, fd_select, is_cid })
    }

    /// The font DICT that glyph `gid` runs with; a glyph FDSelect does not cover gets no local subrs.
    fn font_dict(&self, gid: usize) -> FontDict {
        let i = match self.fd_select {
            Some(at) => fd_index(self.data, at, gid),
            None => Some(0),
        };
        i.and_then(|i| self.fds.get(i)).copied().unwrap_or_default()
    }

    pub fn is_cid(&self) -> bool {
        self.is_cid
    }

    /// The glyph for a CID in a CID-keyed font (the glyph id itself otherwise).
    pub fn cid_to_gid(&self, cid: u16) -> Option<u16> {
        if !self.is_cid {
            return ((cid as usize) < self.num_glyphs()).then_some(cid);
        }
        (0..self.num_glyphs()).find(|&g| self.sid(g) == Some(cid)).map(|g| g as u16)
    }

    pub fn num_glyphs(&self) -> usize {
        self.char_strings.count
    }

    /// The glyph's SID (string id) from the charset.
    fn sid(&self, gid: usize) -> Option<u16> {
        if gid == 0 {
            return Some(0);
        }
        let d = self.data;
        if self.charset == 0 {
            return Some(gid as u16);
        }
        let u16_at = |o: usize| d.get(o..o + 2).map(|s| u16::from_be_bytes([s[0], s[1]]));
        match *d.get(self.charset)? {
            0 => u16_at(self.charset + 1 + 2 * (gid - 1)),
            fmt @ (1 | 2) => {
                let mut p = self.charset + 1;
                let mut g = 1;
                loop {
                    let first = u16_at(p)?;
                    let left = if fmt == 1 { *d.get(p + 2)? as usize } else { u16_at(p + 2)? as usize };
                    if gid < g + left + 1 {
                        return Some(first + (gid - g) as u16);
                    }
                    g += left + 1;
                    p += if fmt == 1 { 3 } else { 4 };
                }
            }
            _ => None,
        }
    }

    /// The glyph's PostScript name.
    pub fn glyph_name(&self, gid: usize) -> Option<String> {
        let sid = self.sid(gid)? as usize;
        if sid < STANDARD_STRINGS_COUNT {
            return STANDARD_STRINGS.split(' ').nth(sid).map(String::from);
        }
        self.strings.get(self.data, sid - STANDARD_STRINGS_COUNT).map(|s| String::from_utf8_lossy(s).into_owned())
    }

    /// The glyph id with this PostScript name.
    pub fn glyph_named(&self, name: &str) -> Option<u16> {
        (0..self.num_glyphs()).find(|&g| self.glyph_name(g).as_deref() == Some(name)).map(|g| g as u16)
    }

    /// The glyph's box in font units as (x_min, y_min, x_max, y_max); a glyph with no outline gives zeros.
    /// None when the glyph does not exist.
    pub fn glyph_bounds(&self, gid: u16) -> Option<[f32; 4]> {
        let mut b = Bounds { min: (f32::MAX, f32::MAX), max: (f32::MIN, f32::MIN), cur: (0.0, 0.0), any: false };
        self.outline(gid, &mut b)?;
        Some(if b.any { [b.min.0, b.min.1, b.max.0, b.max.1] } else { [0.0; 4] })
    }

    /// Runs the glyph's charstring into `sink`; returns the advance width.
    pub fn outline(&self, gid: u16, sink: &mut impl Sink) -> Option<f32> {
        let cs = self.char_strings.get(self.data, gid as usize)?;
        let fd = self.font_dict(gid as usize);
        let mut t2 = Type2 { cff: self, fd, sink, stack: Vec::new(), x: 0.0, y: 0.0, stems: 0, width: None, open: false, trans: Vec::new() };
        t2.run(cs, 0);
        if t2.open {
            t2.sink.close();
        }
        Some(t2.width.unwrap_or(fd.default_width))
    }
}

/// Collects the extent of an outline; curves are sampled, so the box is exact to a small fraction of a unit.
struct Bounds {
    min: (f32, f32),
    max: (f32, f32),
    cur: (f32, f32),
    any: bool,
}

impl Bounds {
    fn add(&mut self, x: f32, y: f32) {
        self.min = (self.min.0.min(x), self.min.1.min(y));
        self.max = (self.max.0.max(x), self.max.1.max(y));
        self.any = true;
    }
}

impl Sink for Bounds {
    fn move_to(&mut self, x: f32, y: f32) {
        self.cur = (x, y);
        self.add(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.cur = (x, y);
        self.add(x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x0, y0) = self.cur;
        for i in 1..=16 {
            let t = i as f32 / 16.0;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.add(a * x0 + b * x1 + c * x2 + d * x, a * y0 + b * y1 + c * y2 + d * y);
        }
        self.cur = (x, y);
    }
    fn close(&mut self) {}
}

struct Type2<'s, 'a, S: Sink> {
    cff: &'s Cff<'a>,
    /// the glyph's font DICT: local subrs and widths
    fd: FontDict,
    sink: &'s mut S,
    stack: Vec<f32>,
    x: f32,
    y: f32,
    stems: usize,
    width: Option<f32>,
    open: bool,
    /// transient array (put/get)
    trans: Vec<f32>,
}

impl<S: Sink> Type2<'_, '_, S> {
    /// Takes the width argument if the first stack-clearing operator has one
    /// more argument than it needs (`even` = the operator's own argument count parity).
    fn take_width(&mut self, needed: usize) {
        if self.width.is_none() {
            self.width = Some(self.fd.default_width);
            if self.stack.len() > needed && (self.stack.len() - needed) % 2 == 1 || (needed == 0 && self.stack.len() == 1) {
                self.width = Some(self.fd.nominal_width + self.stack.remove(0));
            }
        }
    }

    fn move_to(&mut self, dx: f32, dy: f32) {
        if self.open {
            self.sink.close();
        }
        self.x += dx;
        self.y += dy;
        self.sink.move_to(self.x, self.y);
        self.open = true;
    }

    fn line_to(&mut self, dx: f32, dy: f32) {
        self.x += dx;
        self.y += dy;
        self.sink.line_to(self.x, self.y);
    }

    fn curve_to(&mut self, dx1: f32, dy1: f32, dx2: f32, dy2: f32, dx: f32, dy: f32) {
        let (x1, y1) = (self.x + dx1, self.y + dy1);
        let (x2, y2) = (x1 + dx2, y1 + dy2);
        self.x = x2 + dx;
        self.y = y2 + dy;
        self.sink.curve_to(x1, y1, x2, y2, self.x, self.y);
    }

    /// Interprets a charstring; returns false once endchar is reached.
    fn run(&mut self, cs: &[u8], depth: u32) -> bool {
        if depth > 10 {
            return false;
        }
        let mut p = 0;
        while p < cs.len() {
            let b0 = cs[p] as usize;
            p += 1;
            match b0 {
                32..=246 => self.stack.push(b0 as f32 - 139.0),
                247..=250 => {
                    self.stack.push(((b0 as i32 - 247) * 256 + cs.get(p).copied().unwrap_or(0) as i32 + 108) as f32);
                    p += 1;
                }
                251..=254 => {
                    self.stack.push((-(b0 as i32 - 251) * 256 - cs.get(p).copied().unwrap_or(0) as i32 - 108) as f32);
                    p += 1;
                }
                28 => {
                    self.stack.push(i16::from_be_bytes([cs.get(p).copied().unwrap_or(0), cs.get(p + 1).copied().unwrap_or(0)]) as f32);
                    p += 2;
                }
                255 => {
                    let v = cs.get(p..p + 4).map(|s| i32::from_be_bytes([s[0], s[1], s[2], s[3]])).unwrap_or(0);
                    self.stack.push(v as f32 / 65536.0);
                    p += 4;
                }
                1 | 3 | 18 | 23 => {
                    // hstem vstem hstemhm vstemhm
                    self.take_width(self.stack.len() & !1);
                    self.stems += self.stack.len() / 2;
                    self.stack.clear();
                }
                19 | 20 => {
                    // hintmask cntrmask, with implicit vstems
                    self.take_width(self.stack.len() & !1);
                    self.stems += self.stack.len() / 2;
                    self.stack.clear();
                    p += self.stems.div_ceil(8);
                }
                21 => {
                    self.take_width(2);
                    let (dx, dy) = (self.arg(0), self.arg(1));
                    self.move_to(dx, dy);
                    self.stack.clear();
                }
                22 | 4 => {
                    self.take_width(1);
                    let d = self.arg(0);
                    if b0 == 22 {
                        self.move_to(d, 0.0);
                    } else {
                        self.move_to(0.0, d);
                    }
                    self.stack.clear();
                }
                5 => {
                    for i in (0..self.stack.len() / 2).map(|i| 2 * i) {
                        self.line_to(self.stack[i], self.stack[i + 1]);
                    }
                    self.stack.clear();
                }
                6 | 7 => {
                    // hlineto / vlineto alternate directions
                    let mut horizontal = b0 == 6;
                    for i in 0..self.stack.len() {
                        let d = self.stack[i];
                        if horizontal {
                            self.line_to(d, 0.0);
                        } else {
                            self.line_to(0.0, d);
                        }
                        horizontal = !horizontal;
                    }
                    self.stack.clear();
                }
                8 => {
                    self.curves(0, self.stack.len() / 6);
                    self.stack.clear();
                }
                24 => {
                    // rcurveline: curves then one line
                    let n = (self.stack.len().saturating_sub(2)) / 6;
                    self.curves(0, n);
                    let (dx, dy) = (self.arg(6 * n), self.arg(6 * n + 1));
                    self.line_to(dx, dy);
                    self.stack.clear();
                }
                25 => {
                    // rlinecurve: lines then one curve
                    let n = (self.stack.len().saturating_sub(6)) / 2;
                    for i in 0..n {
                        self.line_to(self.stack[2 * i], self.stack[2 * i + 1]);
                    }
                    self.curves(2 * n, 1);
                    self.stack.clear();
                }
                26 | 27 => {
                    // vvcurveto / hhcurveto: an odd leading argument is the other delta
                    let mut i = 0;
                    let mut d1 = 0.0;
                    if self.stack.len() % 4 == 1 {
                        d1 = self.stack[0];
                        i = 1;
                    }
                    while i + 4 <= self.stack.len() {
                        let (a, b, c, d) = (self.stack[i], self.stack[i + 1], self.stack[i + 2], self.stack[i + 3]);
                        if b0 == 26 {
                            self.curve_to(d1, a, b, c, 0.0, d);
                        } else {
                            self.curve_to(a, d1, b, c, d, 0.0);
                        }
                        d1 = 0.0;
                        i += 4;
                    }
                    self.stack.clear();
                }
                30 | 31 => {
                    // vhcurveto / hvcurveto: alternate, last curve may take a 5th argument
                    let mut horizontal = b0 == 31;
                    let mut i = 0;
                    while i + 4 <= self.stack.len() {
                        let last = i + 8 > self.stack.len();
                        let dlast = if last && i + 5 == self.stack.len() { self.stack[i + 4] } else { 0.0 };
                        let (a, b, c, d) = (self.stack[i], self.stack[i + 1], self.stack[i + 2], self.stack[i + 3]);
                        if horizontal {
                            self.curve_to(a, 0.0, b, c, dlast, d);
                        } else {
                            self.curve_to(0.0, a, b, c, d, dlast);
                        }
                        horizontal = !horizontal;
                        i += 4;
                    }
                    self.stack.clear();
                }
                10 | 29 => {
                    let Some(n) = self.stack.pop() else {
                        return false;
                    };
                    // the bias follows the count of the INDEX being called (the glyph's own local subrs, or the global ones)
                    let index = if b0 == 10 { self.fd.local_subrs } else { Some(self.cff.global_subrs) };
                    let Some(index) = index else { return false };
                    let i = n as i32 + bias(index.count);
                    let data = self.cff.data;
                    let Some(sub) = usize::try_from(i).ok().and_then(|i| index.get(data, i)) else {
                        return false;
                    };
                    if !self.run(sub, depth + 1) {
                        return false;
                    }
                }
                11 => return true,
                14 => {
                    self.take_width(if self.stack.len() >= 4 { 4 } else { 0 });
                    if self.open {
                        self.sink.close();
                        self.open = false;
                    }
                    return false;
                }
                12 => {
                    let b1 = cs.get(p).copied().unwrap_or(0);
                    p += 1;
                    self.escape(b1);
                }
                _ => self.stack.clear(),
            }
        }
        true
    }

    fn arg(&self, i: usize) -> f32 {
        self.stack.get(i).copied().unwrap_or(0.0)
    }

    /// `n` rrcurveto curves from stack position `from`.
    fn curves(&mut self, from: usize, n: usize) {
        for k in 0..n {
            let i = from + 6 * k;
            let a: [f32; 6] = std::array::from_fn(|j| self.arg(i + j));
            self.curve_to(a[0], a[1], a[2], a[3], a[4], a[5]);
        }
    }

    fn escape(&mut self, op: u8) {
        let s = self.stack.clone();
        let a = |i: usize| s.get(i).copied().unwrap_or(0.0);
        match op {
            35 => {
                // flex: two curves, fd ignored
                self.curve_to(a(0), a(1), a(2), a(3), a(4), a(5));
                self.curve_to(a(6), a(7), a(8), a(9), a(10), a(11));
            }
            34 => {
                // hflex
                let y0 = self.y;
                self.curve_to(a(0), 0.0, a(1), a(2), a(3), 0.0);
                self.curve_to(a(4), 0.0, a(5), y0 - self.y, a(6), 0.0);
            }
            36 => {
                // hflex1
                let y0 = self.y;
                self.curve_to(a(0), a(1), a(2), a(3), a(4), 0.0);
                let dy2 = y0 - (self.y + a(7));
                self.curve_to(a(5), 0.0, a(6), a(7), a(8), dy2);
            }
            37 => {
                // flex1: the last point returns to the start in the smaller dimension
                let (x0, y0) = (self.x, self.y);
                let dx: f32 = (0..5).map(|i| a(2 * i)).sum();
                let dy: f32 = (0..5).map(|i| a(2 * i + 1)).sum();
                self.curve_to(a(0), a(1), a(2), a(3), a(4), a(5));
                let (x1, y1) = (self.x + a(6), self.y + a(7));
                let (x2, y2) = (x1 + a(8), y1 + a(9));
                let (ex, ey) = if dx.abs() > dy.abs() { (x2 + a(10), y0) } else { (x0, y2 + a(10)) };
                self.sink.curve_to(x1, y1, x2, y2, ex, ey);
                self.x = ex;
                self.y = ey;
            }
            20 => {
                // put
                let i = a(1) as usize;
                if self.trans.len() <= i {
                    self.trans.resize(i + 1, 0.0);
                }
                self.trans[i] = a(0);
                self.stack.clear();
                return;
            }
            21 => {
                // get
                let v = self.trans.get(a(0) as usize).copied().unwrap_or(0.0);
                self.stack.pop();
                self.stack.push(v);
                return;
            }
            _ => {}
        }
        self.stack.clear();
    }
}

const STANDARD_STRINGS_COUNT: usize = 391;
/// The 391 CFF standard strings, space separated, in SID order.
const STANDARD_STRINGS: &str = ".notdef space exclam quotedbl numbersign dollar percent ampersand quoteright parenleft parenright asterisk plus comma hyphen period slash zero one two three four five six seven eight nine colon semicolon less equal greater question at A B C D E F G H I J K L M N O P Q R S T U V W X Y Z bracketleft backslash bracketright asciicircum underscore quoteleft a b c d e f g h i j k l m n o p q r s t u v w x y z braceleft bar braceright asciitilde exclamdown cent sterling fraction yen florin section currency quotesingle quotedblleft guillemotleft guilsinglleft guilsinglright fi fl endash dagger daggerdbl periodcentered paragraph bullet quotesinglbase quotedblbase quotedblright guillemotright ellipsis perthousand questiondown grave acute circumflex tilde macron breve dotaccent dieresis ring cedilla hungarumlaut ogonek caron emdash AE ordfeminine Lslash Oslash OE ordmasculine ae dotlessi lslash oslash oe germandbls onesuperior logicalnot mu trademark Eth onehalf plusminus Thorn onequarter divide brokenbar degree thorn threequarters twosuperior registered minus eth multiply threesuperior copyright Aacute Acircumflex Adieresis Agrave Aring Atilde Ccedilla Eacute Ecircumflex Edieresis Egrave Iacute Icircumflex Idieresis Igrave Ntilde Oacute Ocircumflex Odieresis Ograve Otilde Scaron Uacute Ucircumflex Udieresis Ugrave Yacute Ydieresis Zcaron aacute acircumflex adieresis agrave aring atilde ccedilla eacute ecircumflex edieresis egrave iacute icircumflex idieresis igrave ntilde oacute ocircumflex odieresis ograve otilde scaron uacute ucircumflex udieresis ugrave yacute ydieresis zcaron exclamsmall Hungarumlautsmall dollaroldstyle dollarsuperior ampersandsmall Acutesmall parenleftsuperior parenrightsuperior twodotenleader onedotenleader zerooldstyle oneoldstyle twooldstyle threeoldstyle fouroldstyle fiveoldstyle sixoldstyle sevenoldstyle eightoldstyle nineoldstyle commasuperior threequartersemdash periodsuperior questionsmall asuperior bsuperior centsuperior dsuperior esuperior isuperior lsuperior msuperior nsuperior osuperior rsuperior ssuperior tsuperior ff ffi ffl parenleftinferior parenrightinferior Circumflexsmall hyphensuperior Gravesmall Asmall Bsmall Csmall Dsmall Esmall Fsmall Gsmall Hsmall Ismall Jsmall Ksmall Lsmall Msmall Nsmall Osmall Psmall Qsmall Rsmall Ssmall Tsmall Usmall Vsmall Wsmall Xsmall Ysmall Zsmall colonmonetary onefitted rupiah Tildesmall exclamdownsmall centoldstyle Lslashsmall Scaronsmall Zcaronsmall Dieresissmall Brevesmall Caronsmall Dotaccentsmall Macronsmall figuredash hypheninferior Ogoneksmall Ringsmall Cedillasmall questiondownsmall oneeighth threeeighths fiveeighths seveneighths onethird twothirds zerosuperior foursuperior fivesuperior sixsuperior sevensuperior eightsuperior ninesuperior zeroinferior oneinferior twoinferior threeinferior fourinferior fiveinferior sixinferior seveninferior eightinferior nineinferior centinferior dollarinferior periodinferior commainferior Agravesmall Aacutesmall Acircumflexsmall Atildesmall Adieresissmall Aringsmall AEsmall Ccedillasmall Egravesmall Eacutesmall Ecircumflexsmall Edieresissmall Igravesmall Iacutesmall Icircumflexsmall Idieresissmall Ethsmall Ntildesmall Ogravesmall Oacutesmall Ocircumflexsmall Otildesmall Odieresissmall OEsmall Oslashsmall Ugravesmall Uacutesmall Ucircumflexsmall Udieresissmall Yacutesmall Thornsmall Ydieresissmall 001.000 001.001 001.002 001.003 Black Bold Book Light Medium Regular Roman Semibold";

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    enum Cmd {
        M(f32, f32),
        L(f32, f32),
        C(f32, f32, f32, f32, f32, f32),
        Z,
    }

    #[derive(Default)]
    struct Rec(Vec<Cmd>);

    impl Sink for Rec {
        fn move_to(&mut self, x: f32, y: f32) {
            self.0.push(Cmd::M(x, y));
        }
        fn line_to(&mut self, x: f32, y: f32) {
            self.0.push(Cmd::L(x, y));
        }
        fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
            self.0.push(Cmd::C(x1, y1, x2, y2, x, y));
        }
        fn close(&mut self) {
            self.0.push(Cmd::Z);
        }
    }

    /// Encodes a Type 2 operand.
    fn num(v: i32) -> Vec<u8> {
        match v {
            -107..=107 => vec![(v + 139) as u8],
            108..=1131 => {
                let v = v - 108;
                vec![(v / 256 + 247) as u8, (v % 256) as u8]
            }
            -1131..=-108 => {
                let v = -v - 108;
                vec![(v / 256 + 251) as u8, (v % 256) as u8]
            }
            _ => {
                let mut o = vec![28];
                o.extend((v as i16).to_be_bytes());
                o
            }
        }
    }

    fn cs(items: &[&[u8]]) -> Vec<u8> {
        items.concat()
    }

    /// Builds a one-byte-offset INDEX from items.
    fn index(items: &[&[u8]]) -> Vec<u8> {
        let mut out = (items.len() as u16).to_be_bytes().to_vec();
        if items.is_empty() {
            return out;
        }
        out.push(1);
        let mut off = 1u8;
        out.push(off);
        for it in items {
            off += it.len() as u8;
            out.push(off);
        }
        for it in items {
            out.extend_from_slice(it);
        }
        out
    }

    /// A minimal CFF with one top DICT, the given charstrings, global subrs,
    /// and a private DICT with local subrs and nominal width 500.
    fn font(charstrings: &[&[u8]], gsubrs: &[&[u8]], lsubrs: &[&[u8]]) -> Vec<u8> {
        let header = [1u8, 0, 4, 1];
        let names = index(&[b"T"]);
        let strings = index(&[b"wifi", b"bolt"]);
        let gs = index(gsubrs);
        let charstrings_idx = index(charstrings);
        // private dict: nominalWidthX 500 (op 21), defaultWidthX 250 (op 20), Subrs (op 19)
        let mut private = cs(&[&num(250), &[20], &num(500), &[21]]);
        let subrs_off = private.len() + num(0).len() + 1;
        private.extend(num(subrs_off as i32));
        private.push(19);
        let ls = index(lsubrs);
        // charset format 0: glyph 1 = SID 391 (wifi), glyph 2 = SID 392 (bolt)
        let charset = [0u8, 1, 135, 1, 136];
        // top dict with fixed-width (29 = int32) offsets
        let top_len = 4 * 5 + 3;
        let top_idx_len = 2 + 1 + 2 + top_len;
        let base = header.len() + names.len() + top_idx_len + strings.len() + gs.len();
        let cs_off = base;
        let charset_off = cs_off + charstrings_idx.len();
        let priv_off = charset_off + charset.len();
        let int32 = |v: usize| {
            let mut o = vec![29];
            o.extend((v as i32).to_be_bytes());
            o
        };
        let top = cs(&[&int32(cs_off), &[17], &int32(charset_off), &[15], &int32(private.len()), &int32(priv_off), &[18]]);
        assert_eq!(top.len(), top_len);
        let top_idx = index(&[&top]);
        assert_eq!(top_idx.len(), top_idx_len);
        let mut out = header.to_vec();
        out.extend(names);
        out.extend(top_idx);
        out.extend(strings);
        out.extend(gs);
        out.extend(charstrings_idx);
        out.extend(charset);
        out.extend(private);
        out.extend(ls);
        out
    }

    fn int32(v: usize) -> Vec<u8> {
        let mut o = vec![29];
        o.extend((v as i32).to_be_bytes());
        o
    }

    /// A minimal CID-keyed CFF: a ROS top DICT, the charstrings (glyph g has CID g), the given FDSelect table and
    /// an FDArray with one font DICT per `(defaultWidthX, nominalWidthX, local subrs)`.
    fn cid_font(charstrings: &[&[u8]], fd_select: &[u8], fds: &[(i32, i32, &[&[u8]])]) -> Vec<u8> {
        let header = [1u8, 0, 4, 1];
        let names = index(&[b"T"]);
        let strings = index(&[b"Adobe", b"Identity"]);
        let gs = index(&[]);
        let charstrings_idx = index(charstrings);
        let charset: Vec<u8> = std::iter::once(0u8).chain((1..charstrings.len() as u16).flat_map(u16::to_be_bytes)).collect();
        // each font DICT is `int32 size, int32 offset, Private` (11 bytes)
        let fd_array_len = 3 + (fds.len() + 1) + 11 * fds.len();
        let top = |strings_at: usize, cset: usize, fdsel: usize, fdarr: usize| {
            cs(&[&int32(391), &int32(392), &int32(0), &[12, 30], &int32(strings_at), &[17], &int32(cset), &[15], &int32(fdsel), &[12, 37], &int32(fdarr), &[12, 36]])
        };
        let top_idx_len = index(&[&top(0, 0, 0, 0)]).len();
        let cs_off = header.len() + names.len() + top_idx_len + strings.len() + gs.len();
        let charset_off = cs_off + charstrings_idx.len();
        let fdsel_off = charset_off + charset.len();
        let fdarr_off = fdsel_off + fd_select.len();
        let mut at = fdarr_off + fd_array_len;
        let mut fd_dicts = Vec::new();
        let mut privates = Vec::new();
        for &(dw, nw, subrs) in fds {
            let mut private = cs(&[&num(dw), &[20], &num(nw), &[21]]);
            let subrs_off = private.len() + 6;
            private.extend(int32(subrs_off));
            private.push(19);
            fd_dicts.push(cs(&[&int32(private.len()), &int32(at), &[18]]));
            private.extend(index(subrs));
            at += private.len();
            privates.extend(private);
        }
        let fd_array = index(&fd_dicts.iter().map(Vec::as_slice).collect::<Vec<_>>());
        assert_eq!(fd_array.len(), fd_array_len);
        let top_idx = index(&[&top(cs_off, charset_off, fdsel_off, fdarr_off)]);
        assert_eq!(top_idx.len(), top_idx_len);
        let mut out = header.to_vec();
        for part in [names, top_idx, strings, gs, charstrings_idx, charset, fd_select.to_vec(), fd_array, privates] {
            out.extend(part);
        }
        out
    }

    /// Two font DICTs whose local subr 0 draws differently (FD 0: a line of 50 across, FD 1: a line of 70 up) and
    /// whose widths differ; glyphs 1 and 4 use FD 0, glyphs 2 and 3 use FD 1, and glyphs 1 and 3 carry a width of 10.
    fn check_cid_font(fd_select: &[u8]) {
        let with_width = cs(&[&num(10), &num(0), &num(0), &[21], &num(-107), &[10, 14]]);
        let no_width = cs(&[&num(0), &num(0), &[21], &num(-107), &[10, 14]]);
        let across = cs(&[&num(50), &[6, 11]]);
        let up = cs(&[&num(70), &[7, 11]]);
        let data = cid_font(&[&[14], &with_width, &no_width, &with_width, &no_width], fd_select, &[(300, 600, &[&across]), (1000, 900, &[&up])]);
        let f = Cff::parse(&data).unwrap();
        assert!(f.is_cid());
        assert_eq!(f.cid_to_gid(3), Some(3));
        let glyph = |g: u16| {
            let mut rec = Rec::default();
            let w = f.outline(g, &mut rec).unwrap();
            (rec.0, w)
        };
        let fd0 = vec![Cmd::M(0.0, 0.0), Cmd::L(50.0, 0.0), Cmd::Z];
        let fd1 = vec![Cmd::M(0.0, 0.0), Cmd::L(0.0, 70.0), Cmd::Z];
        assert_eq!(glyph(1), (fd0.clone(), 610.0), "FD 0 subrs, FD 0 nominal width 600 + 10");
        assert_eq!(glyph(2), (fd1.clone(), 1000.0), "FD 1 subrs, FD 1 default width");
        assert_eq!(glyph(3), (fd1, 910.0), "FD 1 nominal width 900 + 10");
        assert_eq!(glyph(4), (fd0, 300.0), "FD 0 default width");
    }

    #[test]
    fn cid_keyed_glyphs_use_their_font_dicts_subrs_and_widths() {
        // FDSelect format 0: one FD byte per glyph
        check_cid_font(&[0, 0, 0, 1, 1, 0]);
        // FDSelect format 3: ranges [0, 2) -> 0, [2, 4) -> 1, [4, 5) -> 0, sentinel 5
        check_cid_font(&[3, 0, 3, 0, 0, 0, 0, 2, 1, 0, 4, 0, 0, 5]);
    }

    #[test]
    fn fd_select_outside_its_ranges_gives_no_font_dict() {
        let fdsel = [3, 0, 2, 0, 1, 0, 0, 3, 1, 0, 5];
        assert_eq!(fd_index(&fdsel, 0, 0), None, "before the first range");
        assert_eq!(fd_index(&fdsel, 0, 1), Some(0));
        assert_eq!(fd_index(&fdsel, 0, 2), Some(0));
        assert_eq!(fd_index(&fdsel, 0, 3), Some(1));
        assert_eq!(fd_index(&fdsel, 0, 4), Some(1));
        assert_eq!(fd_index(&fdsel, 0, 5), None, "at the sentinel");
        assert_eq!(fd_index(&[0, 2, 7], 0, 1), Some(7));
        assert_eq!(fd_index(&[0, 2, 7], 0, 2), None);
        assert_eq!(fd_index(&[4, 0], 0, 0), None, "format 4 is CFF2 only");
    }

    fn run(charstring: &[u8]) -> (Vec<Cmd>, f32) {
        let data = font(&[&[14], charstring], &[], &[]);
        let f = Cff::parse(&data).unwrap();
        let mut rec = Rec::default();
        let w = f.outline(1, &mut rec).unwrap();
        (rec.0, w)
    }

    #[test]
    fn operand_encodings_round_trip() {
        for v in [0, 1, -1, 107, -107, 108, 1131, -108, -1131, 2000, -2000] {
            let (cmds, _) = run(&cs(&[&num(v), &num(0), &[21, 14]]));
            assert_eq!(cmds, vec![Cmd::M(v as f32, 0.0), Cmd::Z], "{v}");
        }
        // 16.16 fixed
        let (cmds, _) = run(&cs(&[&[255, 0, 1, 128, 0], &num(0), &[21, 14]]));
        assert_eq!(cmds[0], Cmd::M(1.5, 0.0));
    }

    #[test]
    fn lines_make_a_closed_square() {
        let (cmds, w) = run(&cs(&[&num(10), &num(20), &[21], &num(100), &[6], &num(100), &[7], &num(-100), &[6, 14]]));
        assert_eq!(cmds, vec![Cmd::M(10.0, 20.0), Cmd::L(110.0, 20.0), Cmd::L(110.0, 120.0), Cmd::L(10.0, 120.0), Cmd::Z]);
        assert_eq!(w, 250.0, "default width");
    }

    #[test]
    fn hlineto_alternates_and_rlineto_takes_pairs() {
        let (cmds, _) = run(&cs(&[&num(0), &num(0), &[21], &num(5), &num(6), &num(7), &[6], &num(1), &num(2), &num(3), &num(4), &[5, 14]]));
        assert_eq!(cmds[1..4], [Cmd::L(5.0, 0.0), Cmd::L(5.0, 6.0), Cmd::L(12.0, 6.0)]);
        assert_eq!(cmds[4..6], [Cmd::L(13.0, 8.0), Cmd::L(16.0, 12.0)]);
    }

    #[test]
    fn width_comes_from_the_odd_leading_argument() {
        let (_, w) = run(&cs(&[&num(-100), &num(10), &num(20), &[21, 14]]));
        assert_eq!(w, 400.0, "nominal 500 - 100");
        let (_, w) = run(&cs(&[&num(30), &num(10), &[22, 14]]));
        assert_eq!(w, 530.0);
        let (_, w) = run(&cs(&[&num(7), &[14]]));
        assert_eq!(w, 507.0, "width on a bare endchar");
        let (_, w) = run(&cs(&[&num(7), &num(1), &num(2), &num(3), &num(4), &[1], &num(0), &num(0), &[21, 14]]));
        assert_eq!(w, 507.0, "width before stems");
    }

    #[test]
    fn curves() {
        let (cmds, _) = run(&cs(&[&num(0), &num(0), &[21], &num(1), &num(2), &num(3), &num(4), &num(5), &num(6), &[8, 14]]));
        assert_eq!(cmds[1], Cmd::C(1.0, 2.0, 4.0, 6.0, 9.0, 12.0));
        // hhcurveto with a leading dy1
        let (cmds, _) = run(&cs(&[&num(0), &num(0), &[21], &num(9), &num(10), &num(1), &num(2), &num(10), &[27, 14]]));
        assert_eq!(cmds[1], Cmd::C(10.0, 9.0, 11.0, 11.0, 21.0, 11.0));
        // vvcurveto
        let (cmds, _) = run(&cs(&[&num(0), &num(0), &[21], &num(10), &num(1), &num(2), &num(10), &[26, 14]]));
        assert_eq!(cmds[1], Cmd::C(0.0, 10.0, 1.0, 12.0, 1.0, 22.0));
        // hvcurveto: horizontal start, vertical end, then alternate; 5th arg on the last
        let (cmds, _) = run(&cs(&[&num(0), &num(0), &[21], &num(10), &num(1), &num(2), &num(10), &num(10), &num(1), &num(2), &num(10), &num(3), &[31, 14]]));
        assert_eq!(cmds[1], Cmd::C(10.0, 0.0, 11.0, 2.0, 11.0, 12.0));
        assert_eq!(cmds[2], Cmd::C(11.0, 22.0, 12.0, 24.0, 22.0, 27.0));
        // vhcurveto
        let (cmds, _) = run(&cs(&[&num(0), &num(0), &[21], &num(10), &num(1), &num(2), &num(10), &[30, 14]]));
        assert_eq!(cmds[1], Cmd::C(0.0, 10.0, 1.0, 12.0, 11.0, 12.0));
        // rcurveline and rlinecurve
        let (cmds, _) = run(&cs(&[&num(0), &num(0), &[21], &num(1), &num(1), &num(1), &num(1), &num(1), &num(1), &num(5), &num(5), &[24, 14]]));
        assert_eq!(cmds[2], Cmd::L(8.0, 8.0));
        let (cmds, _) = run(&cs(&[&num(0), &num(0), &[21], &num(5), &num(5), &num(1), &num(1), &num(1), &num(1), &num(1), &num(1), &[25, 14]]));
        assert_eq!(cmds[1], Cmd::L(5.0, 5.0));
        assert_eq!(cmds[2], Cmd::C(6.0, 6.0, 7.0, 7.0, 8.0, 8.0));
    }

    #[test]
    fn flex_variants_end_on_the_start_line() {
        let (cmds, _) = run(&cs(&[&num(0), &num(0), &[21], &num(1), &num(2), &num(3), &num(4), &num(5), &num(6), &num(7), &[12, 34, 14]]));
        assert_eq!(cmds[1], Cmd::C(1.0, 0.0, 3.0, 3.0, 7.0, 3.0));
        assert_eq!(cmds[2], Cmd::C(12.0, 3.0, 18.0, 0.0, 25.0, 0.0));
        let args: Vec<u8> = [10, 1, 10, 1, 10, 1, 10, 1, 10, 1, 5].into_iter().flat_map(num).collect();
        let (cmds, _) = run(&cs(&[&num(0), &num(0), &[21], &args, &[12, 37, 14]]));
        assert_eq!(cmds[2], Cmd::C(40.0, 4.0, 50.0, 5.0, 55.0, 0.0), "flex1 returns to the start y");
    }

    #[test]
    fn hints_are_skipped_including_hintmask_bytes() {
        // 2 hstems + 2 vstems via hintmask = 4 stems: one mask byte follows
        let s = cs(&[&num(0), &num(10), &num(20), &num(10), &[1], &num(0), &num(10), &num(20), &num(10), &[19, 0xF0], &num(5), &num(5), &[21], &num(1), &[6, 14]]);
        let (cmds, _) = run(&s);
        assert_eq!(cmds, vec![Cmd::M(5.0, 5.0), Cmd::L(6.0, 5.0), Cmd::Z]);
    }

    #[test]
    fn subroutines_apply_the_bias() {
        // local subr 0 (called as -107): a line of 50; global subr 0: a line of 7 then return
        let lsub = cs(&[&num(50), &[6, 11]]);
        let gsub = cs(&[&num(7), &[7, 11]]);
        let main = cs(&[&num(0), &num(0), &[21], &num(-107), &[10], &num(-107), &[29, 14]]);
        let data = font(&[&[14], &main], &[&gsub], &[&lsub]);
        let f = Cff::parse(&data).unwrap();
        let mut rec = Rec::default();
        f.outline(1, &mut rec);
        assert_eq!(rec.0, vec![Cmd::M(0.0, 0.0), Cmd::L(50.0, 0.0), Cmd::L(50.0, 7.0), Cmd::Z]);
        assert_eq!(bias(0), 107);
        assert_eq!(bias(1240), 1131);
        assert_eq!(bias(33900), 32768);
    }

    #[test]
    fn charset_names_resolve_custom_and_standard_strings() {
        let data = font(&[&[14], &[14], &[14]], &[], &[]);
        let f = Cff::parse(&data).unwrap();
        assert_eq!(f.num_glyphs(), 3);
        assert_eq!(f.glyph_name(0).as_deref(), Some(".notdef"));
        assert_eq!(f.glyph_name(1).as_deref(), Some("wifi"));
        assert_eq!(f.glyph_named("bolt"), Some(2));
        assert_eq!(f.glyph_named("nope"), None);
        assert_eq!(STANDARD_STRINGS.split(' ').count(), STANDARD_STRINGS_COUNT);
        assert_eq!(STANDARD_STRINGS.split(' ').nth(34), Some("A"));
    }

    #[test]
    fn dict_numbers() {
        let d = dict(&[0x1c, 0xff, 0x9c, 17, 29, 0, 0, 0x10, 0, 18, 30, 0xe1, 0x2a, 0x5f, 20]);
        assert_eq!(d[0], (17, vec![-100.0]));
        assert_eq!(d[1], (18, vec![4096.0]));
        assert_eq!(d[2], (20, vec![-12.5]));
    }

    #[test]
    fn parses_the_cff_table_of_the_test_font() {
        // BifrOSt's own CFF-flavoured test font: .notdef, a box (lines), a disc and a ring (cubic curves)
        let otf = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test-cff.otf")).expect("tests/fixtures/test-cff.otf");
        let n = u16::from_be_bytes([otf[4], otf[5]]) as usize;
        let cff = (0..n).find_map(|i| {
            let r = 12 + 16 * i;
            (&otf[r..r + 4] == b"CFF ").then(|| {
                let off = u32::from_be_bytes(otf[r + 8..r + 12].try_into().unwrap()) as usize;
                let len = u32::from_be_bytes(otf[r + 12..r + 16].try_into().unwrap()) as usize;
                &otf[off..off + len]
            })
        });
        let f = Cff::parse(cff.expect("CFF table")).unwrap();
        assert_eq!(f.num_glyphs(), 4);
        let names: Vec<_> = (0..4).filter_map(|g| f.glyph_name(g)).collect();
        assert_eq!(names, [".notdef", "box", "disc", "ring"]);
        let mut rec = Rec::default();
        assert_eq!(f.outline(1, &mut rec), Some(800.0));
        assert!(rec.0.iter().any(|c| matches!(c, Cmd::L(..))) && !rec.0.iter().any(|c| matches!(c, Cmd::C(..))), "the box is lines");
        for g in 2..4 {
            let mut rec = Rec::default();
            f.outline(g, &mut rec).unwrap();
            assert!(rec.0.iter().any(|c| matches!(c, Cmd::C(..))), "glyph {g} has curves");
            assert_eq!(rec.0.last(), Some(&Cmd::Z));
        }
    }

    #[test]
    fn glyph_bounds_cover_lines_and_curves() {
        let main = cs(&[&num(10), &num(20), &[21], &num(50), &[6], &num(30), &[7], &[14]]);
        let data = font(&[&[14], &main], &[], &[]);
        let f = Cff::parse(&data).unwrap();
        assert_eq!(f.glyph_bounds(1), Some([10.0, 20.0, 60.0, 50.0]));
        assert_eq!(f.glyph_bounds(0), Some([0.0; 4]), "an empty glyph has no box");
        assert_eq!(f.glyph_bounds(9), None);
        // a curve that bulges past its end points is measured by sampling
        let mut b = Bounds { min: (f32::MAX, f32::MAX), max: (f32::MIN, f32::MIN), cur: (0.0, 0.0), any: false };
        b.move_to(0.0, 0.0);
        b.curve_to(0.0, 100.0, 100.0, 100.0, 100.0, 0.0);
        assert!(b.max.1 > 70.0 && b.max.1 <= 75.0 + 1e-3, "{:?}", b.max);
    }
}
