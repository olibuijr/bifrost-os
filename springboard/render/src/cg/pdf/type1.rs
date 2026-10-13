//! Embedded Type 1 font programs: the eexec envelope, the charstring
//! encryption, and an interpreter that turns charstrings into outlines.

use super::sink::{PathSeg, PdfPath};
use std::collections::HashMap;

pub struct Type1 {
    /// glyph name to decrypted charstring
    chars: HashMap<String, Vec<u8>>,
    subrs: Vec<Vec<u8>>,
    /// the font's own encoding: code to glyph name
    pub encoding: Vec<Option<String>>,
    pub matrix: [f64; 6],
}

fn decrypt(data: &[u8], mut r: u16, skip: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for &c in data {
        out.push(c ^ (r >> 8) as u8);
        r = (c as u16).wrapping_add(r).wrapping_mul(52845).wrapping_add(22719);
    }
    out.drain(..skip.min(out.len()));
    out
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= hay.len() || needle.is_empty() {
        return None;
    }
    hay[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

/// Removes PFB segment headers (0x80 type length) when the data is a PFB file.
fn unwrap_pfb(data: &[u8]) -> Vec<u8> {
    if data.first() != Some(&0x80) {
        return data.to_vec();
    }
    let mut out = vec![];
    let mut p = 0;
    while p + 6 <= data.len() && data[p] == 0x80 {
        let kind = data[p + 1];
        if kind == 3 {
            break;
        }
        let len = u32::from_le_bytes([data[p + 2], data[p + 3], data[p + 4], data[p + 5]]) as usize;
        let end = (p + 6 + len).min(data.len());
        out.extend_from_slice(&data[p + 6..end]);
        p = end;
    }
    out
}

/// A tokenizer over the private dictionary text, with the binary charstrings skipped by length.
struct Scan<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Scan<'a> {
    fn token(&mut self) -> Option<&'a [u8]> {
        while self.p < self.d.len() && (self.d[self.p] as char).is_ascii_whitespace() {
            self.p += 1;
        }
        let start = self.p;
        if start >= self.d.len() {
            return None;
        }
        // single-character tokens
        if matches!(self.d[start], b'[' | b']' | b'{' | b'}') {
            self.p += 1;
            return Some(&self.d[start..start + 1]);
        }
        self.p += 1;
        while self.p < self.d.len() && !(self.d[self.p] as char).is_ascii_whitespace() && !matches!(self.d[self.p], b'[' | b']' | b'{' | b'}' | b'/') {
            self.p += 1;
        }
        Some(&self.d[start..self.p])
    }
}

impl Type1 {
    /// Parses a FontFile stream. `len1` is /Length1 (the clear-text part) when the PDF gives it.
    pub fn parse(raw: &[u8], len1: Option<usize>) -> Option<Type1> {
        let data = unwrap_pfb(raw);
        let split = match find(&data, b"eexec", 0) {
            Some(p) => p + 5,
            None => len1?,
        };
        let clear = &data[..split.min(data.len())];
        let mut enc_part = &data[split.min(data.len())..];
        while enc_part.first().is_some_and(|c| matches!(c, b'\r' | b'\n' | b' ' | b'\t')) {
            enc_part = &enc_part[1..];
        }
        // hex form when the first four bytes are all hex digits
        let is_hex = enc_part.len() >= 4 && enc_part[..4].iter().all(|c| c.is_ascii_hexdigit());
        let cipher: Vec<u8> = if is_hex {
            let mut out = vec![];
            let mut hi: Option<u8> = None;
            for &c in enc_part {
                let Some(v) = (c as char).to_digit(16) else { continue };
                match hi.take() {
                    Some(h) => out.push(h << 4 | v as u8),
                    None => hi = Some(v as u8),
                }
            }
            out
        } else {
            enc_part.to_vec()
        };
        let private = decrypt(&cipher, 55665, 4);

        let mut matrix = [0.001, 0.0, 0.0, 0.001, 0.0, 0.0];
        if let Some(p) = find(clear, b"/FontMatrix", 0) {
            let mut sc = Scan { d: clear, p: p + 11 };
            if sc.token() == Some(&b"["[..]) {
                let mut v = vec![];
                while let Some(t) = sc.token() {
                    if t == b"]" {
                        break;
                    }
                    if let Ok(x) = String::from_utf8_lossy(t).parse::<f64>() {
                        v.push(x);
                    }
                }
                if v.len() == 6 && v[0] != 0.0 {
                    matrix = [v[0], v[1], v[2], v[3], v[4], v[5]];
                }
            }
        }
        let mut encoding: Vec<Option<String>> = vec![None; 256];
        if let Some(p) = find(clear, b"/Encoding", 0) {
            let mut sc = Scan { d: clear, p: p + 9 };
            let first = sc.token();
            if first != Some(&b"StandardEncoding"[..]) {
                // `dup <code> /<name> put` entries until `def`
                let mut guard = 0;
                while let Some(t) = sc.token() {
                    guard += 1;
                    if guard > 5000 || t == b"def" {
                        break;
                    }
                    if t == b"dup" {
                        let code = sc.token().and_then(|c| String::from_utf8_lossy(c).parse::<usize>().ok());
                        let name = sc.token();
                        if let (Some(c), Some(n)) = (code, name) {
                            if c < 256 && n.first() == Some(&b'/') {
                                encoding[c] = Some(String::from_utf8_lossy(&n[1..]).into_owned());
                            }
                        }
                    }
                }
            }
        }

        let len_iv = find(&private, b"/lenIV", 0)
            .and_then(|p| {
                let mut sc = Scan { d: &private, p: p + 6 };
                sc.token().and_then(|t| String::from_utf8_lossy(t).parse::<usize>().ok())
            })
            .unwrap_or(4);

        let mut subrs: Vec<Vec<u8>> = vec![];
        if let Some(p) = find(&private, b"/Subrs", 0) {
            let mut sc = Scan { d: &private, p: p + 6 };
            let count = sc.token().and_then(|t| String::from_utf8_lossy(t).parse::<usize>().ok()).unwrap_or(0);
            subrs = vec![vec![]; count.min(10_000)];
            while let Some(t) = sc.token() {
                if t == b"dup" {
                    let idx = sc.token().and_then(|t| String::from_utf8_lossy(t).parse::<usize>().ok());
                    let len = sc.token().and_then(|t| String::from_utf8_lossy(t).parse::<usize>().ok());
                    let _rd = sc.token();
                    if let (Some(i), Some(l)) = (idx, len) {
                        // exactly one space follows RD
                        let start = sc.p + 1;
                        let Some(bytes) = private.get(start..start + l) else { break };
                        if i < subrs.len() {
                            subrs[i] = decrypt(bytes, 4330, len_iv);
                        }
                        sc.p = start + l;
                    }
                } else if t == b"/CharStrings" {
                    break;
                }
            }
        }

        let mut chars = HashMap::new();
        let cs = find(&private, b"/CharStrings", 0)?;
        let mut sc = Scan { d: &private, p: cs + 12 };
        // skip `<count> dict dup begin`
        while let Some(t) = sc.token() {
            if t == b"begin" {
                break;
            }
        }
        while let Some(t) = sc.token() {
            if t == b"end" {
                break;
            }
            // a name token keeps its slash: `/A 123 RD <bytes> ND`
            if t.len() > 1 && t[0] == b'/' {
                let name = String::from_utf8_lossy(&t[1..]).into_owned();
                let len = sc.token().and_then(|t| String::from_utf8_lossy(t).parse::<usize>().ok());
                let _rd = sc.token();
                if let Some(l) = len {
                    let start = sc.p + 1;
                    let Some(bytes) = private.get(start..start + l) else { break };
                    chars.insert(name, decrypt(bytes, 4330, len_iv));
                    sc.p = start + l;
                }
            }
        }
        if chars.is_empty() {
            return None;
        }
        Some(Type1 { chars, subrs, encoding, matrix })
    }

    pub fn has_glyph(&self, name: &str) -> bool {
        self.chars.contains_key(name)
    }

    /// The outline of a named glyph in glyph space (multiply by the font matrix for em units) and its advance.
    pub fn outline(&self, name: &str) -> Option<(PdfPath, f64)> {
        let mut st = Interp { font: self, path: vec![], x: 0.0, y: 0.0, sbx: 0.0, width: 0.0, stack: vec![], ps: vec![], flex: None, flex_from: (0.0, 0.0), open: false, done: false, depth: 0 };
        st.run(self.chars.get(name)?);
        if st.open {
            st.path.push(PathSeg::Close);
        }
        Some((st.path, st.width))
    }
}

struct Interp<'a> {
    font: &'a Type1,
    path: PdfPath,
    x: f64,
    y: f64,
    sbx: f64,
    width: f64,
    stack: Vec<f64>,
    /// the PostScript stack of callothersubr results
    ps: Vec<f64>,
    /// flex points while a flex is being collected
    flex: Option<Vec<(f64, f64)>>,
    flex_from: (f64, f64),
    open: bool,
    done: bool,
    depth: u8,
}

impl Interp<'_> {
    fn move_to(&mut self, dx: f64, dy: f64) {
        self.x += dx;
        self.y += dy;
        if let Some(f) = &mut self.flex {
            f.push((self.x, self.y));
            return;
        }
        if self.open {
            self.path.push(PathSeg::Close);
        }
        self.path.push(PathSeg::Move(self.x, self.y));
        self.open = true;
    }

    fn line_to(&mut self, dx: f64, dy: f64) {
        self.x += dx;
        self.y += dy;
        if !self.open {
            self.path.push(PathSeg::Move(self.x - dx, self.y - dy));
            self.open = true;
        }
        self.path.push(PathSeg::Line(self.x, self.y));
    }

    fn curve_to(&mut self, d: [f64; 6]) {
        if !self.open {
            self.path.push(PathSeg::Move(self.x, self.y));
            self.open = true;
        }
        let (x1, y1) = (self.x + d[0], self.y + d[1]);
        let (x2, y2) = (x1 + d[2], y1 + d[3]);
        let (x3, y3) = (x2 + d[4], y2 + d[5]);
        self.path.push(PathSeg::Cubic(x1, y1, x2, y2, x3, y3));
        self.x = x3;
        self.y = y3;
    }

    fn run(&mut self, code: &[u8]) {
        if self.depth > 10 {
            return;
        }
        self.depth += 1;
        let mut p = 0;
        while p < code.len() && !self.done {
            let b = code[p];
            p += 1;
            match b {
                32..=246 => self.stack.push(b as f64 - 139.0),
                247..=250 => {
                    let w = code.get(p).copied().unwrap_or(0) as f64;
                    p += 1;
                    self.stack.push((b as f64 - 247.0) * 256.0 + w + 108.0);
                }
                251..=254 => {
                    let w = code.get(p).copied().unwrap_or(0) as f64;
                    p += 1;
                    self.stack.push(-(b as f64 - 251.0) * 256.0 - w - 108.0);
                }
                255 => {
                    let v = code.get(p..p + 4).map(|s| i32::from_be_bytes([s[0], s[1], s[2], s[3]])).unwrap_or(0);
                    p += 4;
                    self.stack.push(v as f64);
                }
                1 | 3 => self.stack.clear(),
                4 => {
                    let dy = self.arg(0);
                    self.stack.clear();
                    self.move_to(0.0, dy);
                }
                5 => {
                    let (dx, dy) = (self.arg(0), self.arg(1));
                    self.stack.clear();
                    self.line_to(dx, dy);
                }
                6 => {
                    let dx = self.arg(0);
                    self.stack.clear();
                    self.line_to(dx, 0.0);
                }
                7 => {
                    let dy = self.arg(0);
                    self.stack.clear();
                    self.line_to(0.0, dy);
                }
                8 => {
                    let d = [self.arg(0), self.arg(1), self.arg(2), self.arg(3), self.arg(4), self.arg(5)];
                    self.stack.clear();
                    self.curve_to(d);
                }
                9 => {
                    if self.open {
                        self.path.push(PathSeg::Close);
                        self.open = false;
                    }
                    self.stack.clear();
                }
                10 => {
                    let idx = self.stack.pop().unwrap_or(-1.0);
                    if idx >= 0.0 {
                        if let Some(sub) = self.font.subrs.get(idx as usize) {
                            let sub = sub.clone();
                            self.run(&sub);
                        }
                    }
                }
                11 => break,
                13 => {
                    self.sbx = self.arg(0);
                    self.width = self.arg(1);
                    self.x = self.sbx;
                    self.y = 0.0;
                    self.stack.clear();
                }
                14 => {
                    self.done = true;
                }
                21 => {
                    let (dx, dy) = (self.arg(0), self.arg(1));
                    self.stack.clear();
                    self.move_to(dx, dy);
                }
                22 => {
                    let dx = self.arg(0);
                    self.stack.clear();
                    self.move_to(dx, 0.0);
                }
                30 => {
                    let d = [0.0, self.arg(0), self.arg(1), self.arg(2), self.arg(3), 0.0];
                    self.stack.clear();
                    self.curve_to(d);
                }
                31 => {
                    let d = [self.arg(0), 0.0, self.arg(1), self.arg(2), 0.0, self.arg(3)];
                    self.stack.clear();
                    self.curve_to(d);
                }
                12 => {
                    let e = code.get(p).copied().unwrap_or(0);
                    p += 1;
                    match e {
                        0..=2 => self.stack.clear(),
                        6 => {
                            // seac: accent composition from two standard-encoding glyphs
                            let (asb, adx, ady, bchar, achar) = (self.arg(0), self.arg(1), self.arg(2), self.arg(3), self.arg(4));
                            self.stack.clear();
                            let base = std_name(bchar as u8);
                            let accent = std_name(achar as u8);
                            let sbx = self.sbx;
                            if let Some(b) = base.and_then(|n| self.font.outline(n)) {
                                self.path.extend(b.0);
                            }
                            if let Some((acc, _)) = accent.and_then(|n| self.font.outline(n)) {
                                let (dx, dy) = (sbx - asb + adx, ady);
                                self.path.extend(acc.into_iter().map(|s| match s {
                                    PathSeg::Move(x, y) => PathSeg::Move(x + dx, y + dy),
                                    PathSeg::Line(x, y) => PathSeg::Line(x + dx, y + dy),
                                    PathSeg::Cubic(a, b, c, d, e, f) => PathSeg::Cubic(a + dx, b + dy, c + dx, d + dy, e + dx, f + dy),
                                    PathSeg::Close => PathSeg::Close,
                                }));
                            }
                            self.open = false;
                            self.done = true;
                        }
                        7 => {
                            self.sbx = self.arg(0);
                            self.width = self.arg(2);
                            self.x = self.sbx;
                            self.y = self.arg(1);
                            self.stack.clear();
                        }
                        12 => {
                            let b = self.stack.pop().unwrap_or(1.0);
                            let a = self.stack.pop().unwrap_or(0.0);
                            self.stack.push(if b != 0.0 { a / b } else { 0.0 });
                        }
                        16 => {
                            let which = self.stack.pop().unwrap_or(-1.0) as i32;
                            let n = self.stack.pop().unwrap_or(0.0).max(0.0) as usize;
                            let mut args = vec![];
                            for _ in 0..n {
                                args.push(self.stack.pop().unwrap_or(0.0));
                            }
                            match which {
                                1 => {
                                    self.flex = Some(vec![]);
                                    self.flex_from = (self.x, self.y);
                                }
                                2 => {}
                                0 => {
                                    // args (popped order): y, x, flex height
                                    if let Some(pts) = self.flex.take() {
                                        if pts.len() >= 7 {
                                            if !self.open {
                                                self.path.push(PathSeg::Move(self.flex_from.0, self.flex_from.1));
                                                self.open = true;
                                            }
                                            self.path.push(PathSeg::Cubic(pts[1].0, pts[1].1, pts[2].0, pts[2].1, pts[3].0, pts[3].1));
                                            self.path.push(PathSeg::Cubic(pts[4].0, pts[4].1, pts[5].0, pts[5].1, pts[6].0, pts[6].1));
                                            self.x = pts[6].0;
                                            self.y = pts[6].1;
                                        }
                                    }
                                    // the end point is returned for the following `pop pop setcurrentpoint`
                                    self.ps = vec![args.first().copied().unwrap_or(self.y), args.get(1).copied().unwrap_or(self.x)];
                                }
                                3 => self.ps = vec![3.0],
                                _ => {
                                    self.ps = args;
                                }
                            }
                        }
                        17 => {
                            let v = self.ps.pop().unwrap_or(0.0);
                            self.stack.push(v);
                        }
                        33 => {
                            self.x = self.arg(0);
                            self.y = self.arg(1);
                            self.stack.clear();
                        }
                        _ => self.stack.clear(),
                    }
                }
                _ => self.stack.clear(),
            }
        }
        self.depth -= 1;
    }

    fn arg(&self, i: usize) -> f64 {
        self.stack.get(i).copied().unwrap_or(0.0)
    }
}

/// The glyph name of a standard-encoding code, for composed accents. Covers letters and the accents
/// seac uses; other codes yield None.
fn std_name(code: u8) -> Option<&'static str> {
    const LETTERS: &str = "A B C D E F G H I J K L M N O P Q R S T U V W X Y Z";
    if code.is_ascii_uppercase() {
        return LETTERS.split(' ').nth((code - b'A') as usize);
    }
    const LOWER: &str = "a b c d e f g h i j k l m n o p q r s t u v w x y z";
    if code.is_ascii_lowercase() {
        return LOWER.split(' ').nth((code - b'a') as usize);
    }
    match code {
        0xc1 => Some("grave"),
        0xc2 => Some("acute"),
        0xc3 => Some("circumflex"),
        0xc4 => Some("tilde"),
        0xc5 => Some("macron"),
        0xc6 => Some("breve"),
        0xc7 => Some("dotaccent"),
        0xc8 => Some("dieresis"),
        0xca => Some("ring"),
        0xcb => Some("cedilla"),
        0xcd => Some("hungarumlaut"),
        0xce => Some("ogonek"),
        0xcf => Some("caron"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encrypt(data: &[u8], mut r: u16, prefix: &[u8]) -> Vec<u8> {
        let mut out = vec![];
        for &p in prefix.iter().chain(data) {
            let c = p ^ (r >> 8) as u8;
            r = (c as u16).wrapping_add(r).wrapping_mul(52845).wrapping_add(22719);
            out.push(c);
        }
        out
    }

    /// Charstring for a 100x100 square: hsbw 0 500; 0 0 rmoveto; 100 hlineto; 100 vlineto; -100 hlineto; closepath; endchar.
    fn square() -> Vec<u8> {
        let n = |v: i32| -> Vec<u8> {
            if (-107..=107).contains(&v) {
                vec![(v + 139) as u8]
            } else if (108..=1131).contains(&v) {
                let w = v - 108;
                vec![(w / 256 + 247) as u8, (w % 256) as u8]
            } else {
                let w = -v - 108;
                vec![(w / 256 + 251) as u8, (w % 256) as u8]
            }
        };
        let mut c = vec![];
        c.extend(n(0));
        c.extend(n(500));
        c.push(13);
        c.extend(n(0));
        c.extend(n(0));
        c.push(21);
        c.extend(n(100));
        c.push(6);
        c.extend(n(100));
        c.push(7);
        c.extend(n(-100));
        c.push(6);
        c.push(9);
        c.push(14);
        c
    }

    #[test]
    fn parses_a_binary_font_and_interprets_a_glyph() {
        let cs = encrypt(&square(), 4330, &[0, 0, 0, 0]);
        let mut private = b"dup /Private 8 dict dup begin /lenIV 4 def /Subrs 0 array\n2 index /CharStrings 1 dict dup begin\n/square ".to_vec();
        private.extend(format!("{} RD ", cs.len()).bytes());
        private.extend(&cs);
        private.extend(b" ND\nend end readonly put\n");
        let enc = encrypt(&private, 55665, &[1, 2, 3, 4]);
        let mut font = b"%!PS-AdobeFont-1.0: Test\n/FontMatrix [0.001 0 0 0.001 0 0] readonly def\n/Encoding 256 array\n0 1 255 {1 index exch /.notdef put} for\ndup 65 /square put\nreadonly def\ncurrentdict end\ncurrentfile eexec\n".to_vec();
        font.extend(&enc);
        let t = Type1::parse(&font, None).unwrap();
        assert!(t.has_glyph("square"));
        assert_eq!(t.encoding[65].as_deref(), Some("square"));
        let (path, w) = t.outline("square").unwrap();
        assert_eq!(w, 500.0);
        assert_eq!(path[0], PathSeg::Move(0.0, 0.0));
        assert_eq!(path[1], PathSeg::Line(100.0, 0.0));
        assert_eq!(path[2], PathSeg::Line(100.0, 100.0));
        assert_eq!(path[3], PathSeg::Line(0.0, 100.0));
        assert_eq!(*path.last().unwrap(), PathSeg::Close);
    }

    #[test]
    fn eexec_decryption_inverts_encryption() {
        let enc = encrypt(b"hello", 55665, &[0, 0, 0, 0]);
        assert_eq!(decrypt(&enc, 55665, 4), b"hello");
    }
}
