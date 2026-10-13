//! CFF outlines for OpenType fonts with a `CFF ` table (OTTO): INDEX and
//! DICT parsing, the charset (glyph names) and a Type 2 charstring
//! interpreter that emits move/line/cubic path commands. CID-keyed fonts
//! render without local subroutines; hints are parsed and discarded.

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

pub struct Cff<'a> {
    data: &'a [u8],
    char_strings: Index,
    global_subrs: Index,
    local_subrs: Option<Index>,
    strings: Index,
    /// charset table offset (0 = ISOAdobe: glyph i has SID i)
    charset: usize,
    nominal_width: f32,
    default_width: f32,
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
        let (mut local_subrs, mut nominal_width, mut default_width) = (None, 0.0, 0.0);
        if let Some(&[size, off]) = find(18) {
            let (size, off) = (size as usize, off as usize);
            let private = dict(data.get(off..off + size)?);
            for (op, v) in &private {
                match (op, v.first()) {
                    (19, Some(&s)) => local_subrs = Index::parse(data, off + s as usize),
                    (20, Some(&w)) => default_width = w,
                    (21, Some(&w)) => nominal_width = w,
                    _ => {}
                }
            }
        }
        Some(Cff { data, char_strings, global_subrs, local_subrs, strings, charset, nominal_width, default_width, is_cid: find(1230).is_some() })
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

    /// Runs the glyph's charstring into `sink`; returns the advance width.
    pub fn outline(&self, gid: u16, sink: &mut impl Sink) -> Option<f32> {
        let cs = self.char_strings.get(self.data, gid as usize)?;
        let mut t2 = Type2 { cff: self, sink, stack: Vec::new(), x: 0.0, y: 0.0, stems: 0, width: None, open: false, trans: Vec::new() };
        t2.run(cs, 0);
        if t2.open {
            t2.sink.close();
        }
        Some(t2.width.unwrap_or(self.default_width))
    }
}

struct Type2<'s, 'a, S: Sink> {
    cff: &'s Cff<'a>,
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
            self.width = Some(self.cff.default_width);
            if self.stack.len() > needed && (self.stack.len() - needed) % 2 == 1 || (needed == 0 && self.stack.len() == 1) {
                self.width = Some(self.cff.nominal_width + self.stack.remove(0));
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
                    let (index, cff) = if b0 == 10 { (self.cff.local_subrs, self.cff) } else { (Some(self.cff.global_subrs), self.cff) };
                    let Some(index) = index else { return false };
                    let i = n as i32 + bias(index.count);
                    let Some(sub) = usize::try_from(i).ok().and_then(|i| index.get(cff.data, i)) else {
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
    fn parses_the_bifrost_symbols_stub() {
        let Ok(otf) = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUISymbols-Regular.otf")) else {
            return;
        };
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
        assert_eq!(f.num_glyphs(), 8);
        let names: Vec<_> = (0..8).filter_map(|g| f.glyph_name(g)).collect();
        assert_eq!(names.len(), 8, "{names:?}");
        for g in 1..8 {
            let mut rec = Rec::default();
            f.outline(g, &mut rec).unwrap();
            assert!(rec.0.iter().any(|c| matches!(c, Cmd::C(..))), "glyph {g} has curves");
            assert_eq!(rec.0.last(), Some(&Cmd::Z));
        }
    }
}
