//! PDF fonts: turning shown byte strings into Unicode (ToUnicode CMaps,
//! simple-font encodings with Differences) and advance widths for text
//! extraction, and glyph outlines from the embedded TrueType, OpenType,
//! CFF and Type 1 programs (or a fallback face) for drawing.

use super::doc::Doc;
use super::obj::{Dict, Obj, Parser, Stream};
use super::page::Matrix;
use super::sink::{PathSeg, PdfPath};
use super::standard::{self, Builtin, Standard};
use super::tables::{GLYPHS, MAC_80, WIN_80};
use super::type1::Type1;
use crate::cff::{Cff, Sink};
use crate::font::Font as Face;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

/// An embedded font program.
enum Program {
    /// TrueType or OpenType (sfnt container)
    Sfnt(Rc<Vec<u8>>),
    /// a bare CFF table (Type1C, CIDFontType0C)
    Cff(Rc<Vec<u8>>),
    Type1(Rc<Type1>),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FontType {
    #[default]
    Simple,
    Type0,
    Type3,
}

/// A glyph outline in em units (1.0 = the font size), y up.
#[derive(Debug)]
pub struct Outline {
    pub path: PdfPath,
    /// the program's own advance in em, when it has one
    pub advance: Option<f64>,
}

/// The pieces of a Type 3 font: glyphs are content streams.
pub struct Type3 {
    pub matrix: Matrix,
    pub procs: HashMap<String, Rc<Stream>>,
    pub resources: Option<Dict>,
}

#[derive(Default)]
pub struct Font {
    pub kind: FontType,
    pub base_font: String,
    symbolic: bool,
    /// the font has an /Encoding entry
    named_base: bool,
    diff_names: HashMap<u8, String>,
    cid_ranges: Vec<(u32, u32, u32)>,
    identity: bool,
    cid_to_gid: Option<Vec<u16>>,
    program: Option<Program>,
    pub type3: Option<Type3>,
    cache: RefCell<HashMap<u32, Option<Rc<Outline>>>>,
    two_byte: bool,
    /// bytes per code in a composite font (from the CMap's codespace)
    code_len: usize,
    uni: HashMap<u32, String>,
    /// per-code overrides of the base encoding (simple fonts)
    diff: HashMap<u8, String>,
    mac: bool,
    first_char: u32,
    widths: Vec<f32>,
    missing_width: f32,
    has_widths: bool,
    dw: f32,
    cid_widths: HashMap<u32, f32>,
    /// the standard face that draws a font with no embedded program
    standard: Option<Standard>,
    /// the built-in encoding a non-embedded simple font falls back to
    builtin: Builtin,
    /// a "nothing drawn" warning was logged for this font
    warned: Cell<bool>,
}

/// One shown character code: its text, advance in 1/1000 em, and whether it
/// was the single-byte code 32 (word spacing applies to it).
pub struct Glyph {
    pub text: String,
    pub width: f32,
    pub is_space: bool,
    /// the character code as shown
    pub code: u32,
    /// the character identifier (composite fonts; equals the code otherwise)
    pub cid: u32,
}

fn utf16be(b: &[u8]) -> String {
    let units: Vec<u16> = b.chunks(2).map(|c| if c.len() == 2 { (c[0] as u16) << 8 | c[1] as u16 } else { c[0] as u16 }).collect();
    String::from_utf16_lossy(&units)
}

fn code_of(b: &[u8]) -> u32 {
    b.iter().fold(0u32, |a, &x| a << 8 | x as u32)
}

/// A glyph name's text, per the Adobe Glyph List subset in `tables`.
pub fn glyph_text(name: &str) -> Option<String> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(c.to_string());
    }
    if let Some((_, s)) = GLYPHS.iter().find(|(n, _)| *n == name) {
        return Some(s.to_string());
    }
    if let Some(hex) = name.strip_prefix("uni").filter(|h| h.len() >= 4 && h.is_char_boundary(4)) {
        return u32::from_str_radix(&hex[..4], 16).ok().and_then(char::from_u32).map(|c| c.to_string());
    }
    if let Some(hex) = name.strip_prefix('u').filter(|h| (4..=6).contains(&h.len())) {
        return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32).map(|c| c.to_string());
    }
    if let Some((base, _)) = name.split_once('.').filter(|(b, _)| !b.is_empty()) {
        return glyph_text(base);
    }
    None
}

impl Font {
    pub fn load(doc: &Doc, d: &Dict) -> Font {
        let mut f = Font { dw: 1000.0, ..Default::default() };
        let subtype = doc.field(d, "Subtype");
        f.two_byte = subtype.name() == Some("Type0");
        f.code_len = if f.two_byte { 2 } else { 1 };
        f.base_font = doc.field(d, "BaseFont").name().unwrap_or("").to_string();
        f.kind = match subtype.name() {
            Some("Type0") => FontType::Type0,
            Some("Type3") => FontType::Type3,
            _ => FontType::Simple,
        };

        if let Obj::Stream(s) = doc.field(d, "ToUnicode") {
            f.parse_cmap(&doc.decode(&s).0);
        }
        if f.two_byte {
            match doc.field(d, "Encoding") {
                Obj::Stream(s) => {
                    f.parse_cmap(&doc.decode(&s).0);
                    f.identity = f.cid_ranges.is_empty();
                }
                _ => f.identity = true,
            }
            if let Some(desc) = doc.field(d, "DescendantFonts").array().and_then(|a| a.first()).map(|o| doc.get(o)) {
                if let Some(dd) = desc.dict() {
                    if let Some(dw) = doc.field(dd, "DW").num() {
                        f.dw = dw as f32;
                    }
                    if let Obj::Array(w) = doc.field(dd, "W") {
                        f.parse_cid_widths(doc, &w);
                    }
                    if let Obj::Stream(m) = doc.field(dd, "CIDToGIDMap") {
                        let bytes = doc.decode(&m).0;
                        f.cid_to_gid = Some(bytes.chunks(2).map(|c| if c.len() == 2 { (c[0] as u16) << 8 | c[1] as u16 } else { 0 }).collect());
                    }
                    if let Obj::Dict(desc) = doc.field(dd, "FontDescriptor") {
                        f.symbolic = false;
                        f.program = load_program(doc, &desc);
                    }
                    if f.program.is_none() {
                        f.standard = Some(standard_for(doc, &f.base_font, doc.field(dd, "FontDescriptor").dict()));
                    }
                }
            }
            if f.program.is_none() && f.standard.is_none() {
                f.standard = Some(standard_for(doc, &f.base_font, None));
            }
        } else if f.kind == FontType::Type3 {
            f.load_simple(doc, d);
            let mut procs = HashMap::new();
            if let Obj::Dict(cp) = doc.field(d, "CharProcs") {
                for (name, v) in &cp {
                    if let Obj::Stream(s) = doc.get(v) {
                        procs.insert(name.clone(), s);
                    }
                }
            }
            let m = match doc.field(d, "FontMatrix") {
                Obj::Array(a) if a.len() >= 6 => {
                    let v: Vec<f64> = a.iter().map(|o| doc.get(o).num().unwrap_or(0.0)).collect();
                    [v[0], v[1], v[2], v[3], v[4], v[5]]
                }
                _ => [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
            };
            let resources = doc.field(d, "Resources").dict().cloned();
            f.type3 = Some(Type3 { matrix: m, procs, resources });
        } else {
            f.load_simple(doc, d);
            let desc = doc.field(d, "FontDescriptor");
            if let Obj::Dict(desc) = &desc {
                let flags = doc.field(desc, "Flags").int().unwrap_or(0);
                f.symbolic = flags & 4 != 0 && flags & 32 == 0;
                f.program = load_program(doc, desc);
            }
            if f.program.is_none() {
                // not embedded: one of the standard 14 or a font the reader is expected to have
                let s = standard_for(doc, &f.base_font, desc.dict());
                f.standard = Some(s);
                let base_named = match doc.field(d, "Encoding") {
                    Obj::Name(_) => true,
                    Obj::Dict(e) => doc.field(&e, "BaseEncoding").name().is_some(),
                    _ => false,
                };
                let type1 = matches!(subtype.name(), Some("Type1") | Some("MMType1") | None);
                f.builtin = match s.family {
                    standard::Family::Symbol if !base_named => Builtin::Symbol,
                    standard::Family::Dingbats if !base_named => Builtin::Dingbats,
                    _ if type1 && !base_named && !f.symbolic && !s.is_symbol() => Builtin::Standard,
                    _ => Builtin::None,
                };
            }
        }
        f
    }

    fn load_simple(&mut self, doc: &Doc, d: &Dict) {
        let enc = doc.field(d, "Encoding");
        let base_name = match &enc {
            Obj::Name(n) => Some(n.clone()),
            Obj::Dict(e) => doc.field(e, "BaseEncoding").name().map(str::to_string),
            _ => None,
        };
        self.mac = base_name.as_deref() == Some("MacRomanEncoding");
        self.named_base = !matches!(enc, Obj::Null);
        if let Obj::Dict(e) = &enc {
            if let Obj::Array(diffs) = doc.field(e, "Differences") {
                let mut code = 0u32;
                for o in diffs {
                    match doc.get(&o) {
                        Obj::Int(n) => code = n.max(0) as u32,
                        Obj::Name(n) => {
                            if code < 256 {
                                if let Some(t) = glyph_text(&n) {
                                    self.diff.insert(code as u8, t);
                                }
                                self.diff_names.insert(code as u8, n.clone());
                            }
                            code += 1;
                        }
                        _ => {}
                    }
                }
            }
        }
        self.first_char = doc.field(d, "FirstChar").int().unwrap_or(0).max(0) as u32;
        if let Obj::Array(w) = doc.field(d, "Widths") {
            self.widths = w.iter().map(|o| doc.get(o).num().unwrap_or(0.0) as f32).collect();
            self.has_widths = true;
        }
        if let Obj::Dict(desc) = doc.field(d, "FontDescriptor") {
            self.missing_width = doc.field(&desc, "MissingWidth").num().unwrap_or(0.0) as f32;
        }
    }

    fn parse_cid_widths(&mut self, doc: &Doc, w: &[Obj]) {
        let items: Vec<Obj> = w.iter().map(|o| doc.get(o)).collect();
        let mut i = 0;
        while i < items.len() {
            let Some(first) = items[i].int() else { break };
            match items.get(i + 1) {
                Some(Obj::Array(list)) => {
                    for (k, v) in list.iter().enumerate() {
                        self.cid_widths.insert(first as u32 + k as u32, doc.get(v).num().unwrap_or(0.0) as f32);
                    }
                    i += 2;
                }
                Some(o) => {
                    let (Some(last), Some(width)) = (o.int(), items.get(i + 2).and_then(|x| x.num())) else { break };
                    for c in first..=last.min(first + 70_000) {
                        self.cid_widths.insert(c as u32, width as f32);
                    }
                    i += 3;
                }
                None => break,
            }
        }
    }

    fn parse_cmap(&mut self, data: &[u8]) {
        let mut p = Parser::new(data, 0);
        while let Some(o) = p.parse(false) {
            let Obj::Op(op) = &o else { continue };
            match op.as_str() {
                "begincodespacerange" => {
                    if let Some(Obj::Str(lo)) = p.parse(false) {
                        if !lo.is_empty() && lo.len() <= 4 {
                            self.code_len = lo.len();
                        }
                    }
                }
                "begincidrange" => while let Some(Obj::Str(lo)) = p.parse(false) {
                    let Some(Obj::Str(hi)) = p.parse(false) else { break };
                    let Some(cid) = p.parse(false).and_then(|o| o.int()) else { break };
                    self.cid_ranges.push((code_of(&lo), code_of(&hi), cid.max(0) as u32));
                },
                "begincidchar" => while let Some(Obj::Str(c)) = p.parse(false) {
                    let Some(cid) = p.parse(false).and_then(|o| o.int()) else { break };
                    let c = code_of(&c);
                    self.cid_ranges.push((c, c, cid.max(0) as u32));
                },
                "beginbfchar" => while let Some(Obj::Str(src)) = p.parse(false) {
                    if let Some(Obj::Str(dst)) = p.parse(false) {
                        self.uni.insert(code_of(&src), utf16be(&dst));
                    }
                },
                "beginbfrange" => while let Some(Obj::Str(lo)) = p.parse(false) {
                    let Some(Obj::Str(hi)) = p.parse(false) else { break };
                    let (lo, hi) = (code_of(&lo), code_of(&hi));
                    match p.parse(false) {
                        Some(Obj::Str(dst)) => {
                            let base = utf16be(&dst);
                            let mut chars: Vec<char> = base.chars().collect();
                            for c in lo..=hi.min(lo + 65_535) {
                                let Some(last) = chars.last().copied() else { break };
                                self.uni.insert(c, chars.iter().collect());
                                let n = chars.len();
                                chars[n - 1] = char::from_u32(last as u32 + 1).unwrap_or(last);
                            }
                        }
                        Some(Obj::Array(list)) => {
                            for (k, item) in list.iter().enumerate() {
                                if let Obj::Str(dst) = item {
                                    self.uni.insert(lo + k as u32, utf16be(dst));
                                }
                            }
                        }
                        _ => break,
                    }
                },
                _ => {}
            }
        }
    }

    fn simple_text(&self, code: u8) -> Option<String> {
        if let Some(t) = self.diff.get(&code) {
            return Some(t.clone());
        }
        if self.builtin != Builtin::None {
            return self.builtin.text(code).map(|c| c.to_string());
        }
        let c = match code {
            0x80..=0x9f if !self.mac => WIN_80[(code - 0x80) as usize],
            0x80..=0xff if self.mac => MAC_80[(code - 0x80) as usize],
            0x00..=0x1f => return None,
            0x7f => return None,
            c => c as char,
        };
        (c != '\u{fffd}').then(|| c.to_string())
    }

    fn simple_width(&self, code: u8) -> f32 {
        if !self.has_widths {
            let Some(s) = self.standard else { return 500.0 };
            // the standard faces' AFM metrics, else the (metric-compatible) fallback face's own advance
            let text = self.uni.get(&(code as u32)).cloned().or_else(|| self.simple_text(code)).unwrap_or_default();
            return s.width(&text).or_else(|| self.fallback_advance(s, code, &text)).unwrap_or(s.default_width());
        }
        let i = (code as u32).checked_sub(self.first_char).map(|i| i as usize);
        i.and_then(|i| self.widths.get(i)).copied().unwrap_or(self.missing_width)
    }

    /// Splits a shown string into glyphs.
    pub fn glyphs(&self, bytes: &[u8]) -> Vec<Glyph> {
        let n = if self.two_byte { self.code_len.max(1) } else { 1 };
        bytes
            .chunks(n)
            .map(|chunk| {
                let code = code_of(chunk);
                if self.two_byte {
                    let text = self.uni.get(&code).cloned().unwrap_or_else(|| match char::from_u32(code) {
                        Some(c) if code >= 0x20 && !(0xd800..0xe000).contains(&code) => c.to_string(),
                        _ => String::new(),
                    });
                    let cid = self.cid_of(code);
                    let width = self.cid_widths.get(&cid).copied().unwrap_or(self.dw);
                    Glyph { text, width, is_space: false, code, cid }
                } else {
                    let b = chunk[0];
                    let text = self.uni.get(&code).cloned().or_else(|| self.simple_text(b)).unwrap_or_default();
                    Glyph { text, width: self.simple_width(b), is_space: b == 32, code, cid: code }
                }
            })
            .collect()
    }
}

/// The font a page resource names (`/Font` entry `name` of `res`). Font objects are loaded once per object number
/// into `cache`, so a font used on many pages or in many forms is parsed once.
pub fn from_resources(doc: &Doc, res: &Dict, name: &str, cache: &mut HashMap<u32, Rc<Font>>) -> Option<Rc<Font>> {
    let fonts = doc.field(res, "Font");
    let entry = fonts.dict()?.get(name)?.clone();
    if let Obj::Ref(n, _) = entry {
        if let Some(f) = cache.get(&n) {
            return Some(f.clone());
        }
        let f = Rc::new(Font::load(doc, doc.load(n).dict()?));
        cache.insert(n, f.clone());
        return Some(f);
    }
    Some(Rc::new(Font::load(doc, entry.dict()?)))
}

fn load_program(doc: &Doc, desc: &Dict) -> Option<Program> {
    let mut data = None;
    let mut len1 = None;
    for key in ["FontFile2", "FontFile3", "FontFile"] {
        if let Obj::Stream(s) = doc.field(desc, key) {
            len1 = doc.field(&s.dict, "Length1").int().map(|v| v.max(0) as usize);
            data = Some(doc.decode(&s).0);
            break;
        }
    }
    let data = data?;
    // sniff the container; the key is only a hint (producers mislabel programs)
    if data.starts_with(&[0, 1, 0, 0]) || data.starts_with(b"OTTO") || data.starts_with(b"true") {
        Some(Program::Sfnt(Rc::new(data)))
    } else if data.first() == Some(&1) && data.get(1) == Some(&0) {
        Some(Program::Cff(Rc::new(data)))
    } else if data.starts_with(b"%!") || data.first() == Some(&0x80) {
        Type1::parse(&data, len1).map(|t| Program::Type1(Rc::new(t)))
    } else {
        None
    }
}

/// The glyph name for a character (the reverse of [`glyph_text`] for single characters).
pub fn name_for_char(c: char) -> Option<String> {
    if c.is_ascii_alphabetic() {
        return Some(c.to_string());
    }
    let mut buf = [0u8; 4];
    let s: &str = c.encode_utf8(&mut buf);
    GLYPHS.iter().find(|(_, t)| *t == s).map(|(n, _)| n.to_string())
}

/// Path commands from a CFF charstring, scaled to em units.
struct PathSink {
    path: PdfPath,
    k: f64,
    open: bool,
}

impl Sink for PathSink {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.open {
            self.path.push(PathSeg::Close);
        }
        self.path.push(PathSeg::Move(x as f64 * self.k, y as f64 * self.k));
        self.open = true;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.path.push(PathSeg::Line(x as f64 * self.k, y as f64 * self.k));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let k = self.k;
        self.path.push(PathSeg::Cubic(x1 as f64 * k, y1 as f64 * k, x2 as f64 * k, y2 as f64 * k, x as f64 * k, y as f64 * k));
    }
    fn close(&mut self) {
        if self.open {
            self.path.push(PathSeg::Close);
            self.open = false;
        }
    }
}

fn cff_outline(cff: &Cff, gid: u16, k: f64) -> Option<Outline> {
    let mut sink = PathSink { path: vec![], k, open: false };
    let adv = cff.outline(gid, &mut sink)?;
    sink.close();
    Some(Outline { path: sink.path, advance: Some(adv as f64 * k) })
}

/// Glyph `gid` of an sfnt face as an em-unit path. Contours come from the face's flattened outline.
fn face_outline(face: &Face, gid: u16) -> Option<Outline> {
    let k = 1.0 / face.upem.max(1.0) as f64;
    let advance = Some(face.advance(gid) as f64 * k);
    if let Some(cff) = face.cff() {
        return cff_outline(cff, gid, k).map(|mut o| {
            o.advance = advance;
            o
        });
    }
    let mut path: PdfPath = vec![];
    let mut last: Option<(f32, f32)> = None;
    let mut open = false;
    for (a, b) in face.outline_lines(gid) {
        if last != Some(a) {
            if open {
                path.push(PathSeg::Close);
            }
            path.push(PathSeg::Move(a.0 as f64 * k, a.1 as f64 * k));
            open = true;
        }
        path.push(PathSeg::Line(b.0 as f64 * k, b.1 as f64 * k));
        last = Some(b);
    }
    if open {
        path.push(PathSeg::Close);
    }
    Some(Outline { path, advance })
}

impl Font {
    pub fn is_type3(&self) -> bool {
        self.kind == FontType::Type3
    }

    /// True when the PDF itself gives advance widths (otherwise [`Outline::advance`] is the best source).
    pub fn has_widths(&self) -> bool {
        self.has_widths || self.two_byte || self.kind == FontType::Type3
    }

    /// True when an embedded program is available.
    pub fn embedded(&self) -> bool {
        self.program.is_some()
    }

    /// The Type 3 glyph procedure name of a code.
    pub fn proc_name(&self, code: u32) -> Option<&str> {
        self.diff_names.get(&(code as u8)).map(String::as_str)
    }

    fn cid_of(&self, code: u32) -> u32 {
        if self.identity {
            return code;
        }
        self.cid_ranges.iter().find(|(lo, hi, _)| code >= *lo && code <= *hi).map_or(0, |(lo, _, cid)| cid + (code - lo))
    }

    /// The glyph name a simple font gives to a code: its Differences entry, the program's own encoding when
    /// the font names no base encoding, else the name of the character the standard encodings map it to.
    fn glyph_name(&self, code: u8, builtin: Option<&[Option<String>]>) -> Option<String> {
        if let Some(n) = self.diff_names.get(&code) {
            return Some(n.clone());
        }
        if !self.named_base || self.symbolic {
            if let Some(Some(n)) = builtin.and_then(|b| b.get(code as usize)) {
                return Some(n.clone());
            }
        }
        self.simple_text(code).and_then(|t| t.chars().next()).and_then(name_for_char)
    }

    fn sfnt_gid(&self, face: &Face, g: &Glyph) -> u16 {
        if self.two_byte {
            let cid = g.cid;
            if let Some(map) = &self.cid_to_gid {
                return map.get(cid as usize).copied().unwrap_or(0);
            }
            if let Some(cff) = face.cff() {
                return cff.cid_to_gid(cid as u16).unwrap_or(0);
            }
            return cid as u16;
        }
        let code = g.code as u8;
        let ch = self.simple_text(code).and_then(|t| t.chars().next());
        let by_unicode = |ch: Option<char>| ch.map_or(0, |c| face.glyph(c));
        let mut gid = if !self.symbolic || self.named_base {
            let g1 = by_unicode(ch);
            if g1 != 0 {
                g1
            } else {
                face.glyph_symbolic(code)
            }
        } else {
            let g1 = face.glyph_symbolic(code);
            if g1 != 0 {
                g1
            } else {
                by_unicode(ch)
            }
        };
        if gid == 0 {
            if let Some(n) = self.glyph_name(code, None) {
                gid = face.glyph_named(&n).unwrap_or(0);
            }
        }
        if gid == 0 && !face.has_cmap() {
            gid = code as u16;
        }
        gid
    }

    /// The outline of a shown glyph. `fallback` is a face to draw with when the font has no usable embedded
    /// program (its characters come from the glyph's text).
    pub fn outline(&self, g: &Glyph, fallback: Option<&Rc<Vec<u8>>>) -> Option<Rc<Outline>> {
        if let Some(hit) = self.cache.borrow().get(&g.code) {
            return hit.clone();
        }
        let built = self.build(g, fallback).map(Rc::new);
        self.cache.borrow_mut().insert(g.code, built.clone());
        built
    }

    fn build(&self, g: &Glyph, fallback: Option<&Rc<Vec<u8>>>) -> Option<Outline> {
        match &self.program {
            Some(Program::Sfnt(data)) => {
                let face = Face::parse(data)?;
                let gid = self.sfnt_gid(&face, g);
                // gid 0 is .notdef; showing it is what readers do for a missing glyph, but only when it has an outline
                face_outline(&face, gid)
            }
            Some(Program::Cff(data)) => {
                let cff = Cff::parse(data)?;
                let gid = if self.two_byte {
                    cff.cid_to_gid(g.cid as u16)?
                } else {
                    let name = self.glyph_name(g.code as u8, None)?;
                    cff.glyph_named(&name).or_else(|| if cff.is_cid() { None } else { Some(0) })?
                };
                cff_outline(&cff, gid, 0.001)
            }
            Some(Program::Type1(t1)) => {
                let name = self.glyph_name(g.code as u8, Some(&t1.encoding))?;
                let (path, w) = t1.outline(&name)?;
                let m = t1.matrix;
                let path = path
                    .into_iter()
                    .map(|s| {
                        let t = |x: f64, y: f64| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
                        match s {
                            PathSeg::Move(x, y) => {
                                let p = t(x, y);
                                PathSeg::Move(p.0, p.1)
                            }
                            PathSeg::Line(x, y) => {
                                let p = t(x, y);
                                PathSeg::Line(p.0, p.1)
                            }
                            PathSeg::Cubic(a, b, c, d, e, f) => {
                                let (p, q, r) = (t(a, b), t(c, d), t(e, f));
                                PathSeg::Cubic(p.0, p.1, q.0, q.1, r.0, r.1)
                            }
                            PathSeg::Close => PathSeg::Close,
                        }
                    })
                    .collect();
                Some(Outline { path, advance: Some(w * m[0]) })
            }
            None => {
                let ch = g.text.chars().next();
                let s = self.standard.unwrap_or(Standard::SANS);
                let name = if self.two_byte { None } else { self.glyph_name(g.code as u8, None) };
                let code = if self.two_byte { 0 } else { g.code as u8 };
                if let Some(bytes) = standard::face_bytes(s).or_else(|| fallback.cloned()) {
                    if let Some(face) = Face::parse(&bytes) {
                        let gid = standard::glyph_in(&face, s, ch, name.as_deref(), code);
                        if gid != 0 {
                            return face_outline(&face, gid);
                        }
                    }
                }
                // the system cascade: any installed face that has the character
                let ch = ch?;
                let faces = crate::fontfallback::faces();
                let face = faces.iter().find(|f| f.glyph(ch) != 0)?;
                face_outline(face, face.glyph(ch))
            }
        }
    }

    /// The advance (1/1000 em) the fallback face gives a non-embedded simple font's code.
    fn fallback_advance(&self, s: Standard, code: u8, text: &str) -> Option<f32> {
        let bytes = standard::face_bytes(s)?;
        let face = Face::parse(&bytes)?;
        let name = self.glyph_name(code, None);
        let gid = standard::glyph_in(&face, s, text.chars().next(), name.as_deref(), code);
        (gid != 0).then(|| face.advance(gid) * 1000.0 / face.upem.max(1.0))
    }

    /// Logs once per font that its text cannot be drawn (no embedded program and no installed face has the glyph),
    /// so a blank page is never silent.
    pub fn warn_undrawn(&self, text: &str) {
        if !self.warned.replace(true) {
            eprintln!("pdf: font {:?} has no drawable glyph for {:?} (not embedded, no fallback face); its text is not drawn", self.base_font, text);
        }
    }

    /// The standard face a non-embedded font is drawn with.
    pub fn standard(&self) -> Option<Standard> {
        self.standard
    }
}

/// The standard face for a non-embedded font, from its name and descriptor.
fn standard_for(doc: &Doc, base_font: &str, desc: Option<&Dict>) -> Standard {
    let flags = desc.and_then(|d| doc.field(d, "Flags").int()).unwrap_or(0);
    let weight = desc.and_then(|d| doc.field(d, "FontWeight").num());
    let angle = desc.and_then(|d| doc.field(d, "ItalicAngle").num()).unwrap_or(0.0);
    Standard::from_name(base_font, flags, weight, angle)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn glyph_names() {
        assert_eq!(glyph_text("A").as_deref(), Some("A"));
        assert_eq!(glyph_text("eacute").as_deref(), Some("é"));
        assert_eq!(glyph_text("fi").as_deref(), Some("fi"));
        assert_eq!(glyph_text("uni00E9").as_deref(), Some("é"));
        assert_eq!(glyph_text("quoteright").as_deref(), Some("\u{2019}"));
        assert_eq!(glyph_text("a.sc").as_deref(), Some("a"));
        assert_eq!(glyph_text("g123"), None);
        assert_eq!(glyph_text("thorn").as_deref(), Some("þ"));
    }

    #[test]
    fn tounicode_bfchar_and_bfrange() {
        let mut f = Font::default();
        f.parse_cmap(b"1 begincodespacerange <0000> <FFFF> endcodespacerange 2 beginbfchar <0003> <0020> <0024> <00410042> endbfchar 2 beginbfrange <0030> <0032> <0061> <0040> <0041> [<0058> <0059>] endbfrange");
        assert_eq!(f.code_len, 2);
        assert_eq!(f.uni[&3], " ");
        assert_eq!(f.uni[&0x24], "AB");
        assert_eq!((f.uni[&0x30].as_str(), f.uni[&0x31].as_str(), f.uni[&0x32].as_str()), ("a", "b", "c"));
        assert_eq!((f.uni[&0x40].as_str(), f.uni[&0x41].as_str()), ("X", "Y"));
    }

    #[test]
    fn simple_font_encodings_and_widths() {
        let mut f = Font { dw: 1000.0, first_char: 65, widths: vec![700.0, 800.0], has_widths: true, ..Default::default() };
        f.diff.insert(b'B', "fi".into());
        let g = f.glyphs(b"AB C\x93");
        assert_eq!(g.iter().map(|g| g.text.as_str()).collect::<String>(), "Afi C\u{201c}");
        assert_eq!((g[0].width, g[1].width, g[3].width), (700.0, 800.0, 0.0));
        assert!(g[2].is_space);
        let m = Font { mac: true, ..Default::default() };
        assert_eq!(m.glyphs(&[0x8e])[0].text, "é");
    }

    /// A one-page PDF whose `/F1` is `font` (a dictionary body), showing `content`.
    pub(crate) fn pdf_with_font(font: &str, content: &str) -> Vec<u8> {
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offs = vec![];
        let objs: Vec<String> = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources << /Font << /F1 5 0 R >> >> >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R >>".into(),
            format!("<< /Length {} >>\nstream\n{}\nendstream", content.len(), content),
            font.into(),
        ];
        for (i, o) in objs.iter().enumerate() {
            offs.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, o).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
        for o in &offs {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n", objs.len() + 1, xref).as_bytes());
        out
    }

    fn font_of(dict: &str) -> Font {
        let doc = Doc::open(pdf_with_font(dict, "BT ET")).unwrap();
        let d = doc.load(5);
        Font::load(&doc, d.dict().unwrap())
    }

    #[test]
    fn non_embedded_standard_fonts_get_afm_widths_and_a_face() {
        let f = font_of("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>");
        assert!(!f.embedded());
        assert_eq!(f.standard(), Some(Standard::SANS));
        let w: f32 = f.glyphs(b"Hello").iter().map(|g| g.width).sum();
        assert_eq!(w, 722.0 + 556.0 + 222.0 + 222.0 + 556.0, "Helvetica's AFM advances, not a flat 500");
        let c = font_of("<< /Type /Font /Subtype /Type1 /BaseFont /Courier-Bold >>");
        assert!(c.glyphs(b"iW").iter().all(|g| g.width == 600.0));
        // a /Widths array still wins over the AFM table
        let given = font_of("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FirstChar 72 /LastChar 72 /Widths [100] >>");
        assert_eq!(given.glyphs(b"H")[0].width, 100.0);
    }

    #[test]
    fn standard_encoding_applies_when_no_encoding_is_named_and_differences_override_it() {
        let t = font_of("<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman >>");
        let text: String = t.glyphs(b"it'\xae").iter().map(|g| g.text.as_str()).collect();
        assert_eq!(text, "it\u{2019}\u{fb01}", "StandardEncoding: quoteright and the fi ligature");
        let w = font_of("<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding /WinAnsiEncoding >>");
        assert_eq!(w.glyphs(b"'\xe9")[0].text, "'", "WinAnsi keeps quotesingle");
        assert_eq!(w.glyphs(b"\xe9")[0].text, "é");
        let d = font_of("<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding << /Differences [39 /quotesingle 65 /eacute] >> >>");
        let text: String = d.glyphs(b"'AB").iter().map(|g| g.text.as_str()).collect();
        assert_eq!(text, "'éB");
    }

    #[test]
    fn symbol_and_dingbats_use_their_builtin_encodings() {
        let s = font_of("<< /Type /Font /Subtype /Type1 /BaseFont /Symbol >>");
        let text: String = s.glyphs(b"abp").iter().map(|g| g.text.as_str()).collect();
        assert_eq!(text, "αβπ");
        let z = font_of("<< /Type /Font /Subtype /Type1 /BaseFont /ZapfDingbats >>");
        assert_eq!(z.glyphs(b"l")[0].text, "●");
    }

    #[test]
    fn composite_fonts_decode_two_bytes() {
        let mut f = Font { two_byte: true, code_len: 2, dw: 1000.0, identity: true, ..Default::default() };
        f.uni.insert(0x0102, "hi".into());
        f.cid_widths.insert(0x0102, 250.0);
        let g = f.glyphs(&[1, 2, 0, 65]);
        assert_eq!((g[0].text.as_str(), g[0].width), ("hi", 250.0));
        assert_eq!((g[1].text.as_str(), g[1].width), ("A", 1000.0));
    }
}
