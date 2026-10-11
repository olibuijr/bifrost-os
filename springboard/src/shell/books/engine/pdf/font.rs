//! PDF font handling reduced to what text extraction needs: turning shown
//! byte strings into Unicode (ToUnicode CMaps, simple-font encodings with
//! Differences) and the advance widths that reveal word gaps.

use super::doc::Doc;
use super::obj::{Dict, Obj, Parser};
use super::tables::{GLYPHS, MAC_80, WIN_80};
use std::collections::HashMap;

#[derive(Default)]
pub struct Font {
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
}

/// One shown character code: its text, advance in 1/1000 em, and whether it
/// was the single-byte code 32 (word spacing applies to it).
pub struct Glyph {
    pub text: String,
    pub width: f32,
    pub is_space: bool,
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

        if let Obj::Stream(s) = doc.field(d, "ToUnicode") {
            f.parse_cmap(&doc.decode(&s).0);
        }
        if f.two_byte {
            if let Some(desc) = doc.field(d, "DescendantFonts").array().and_then(|a| a.first()).map(|o| doc.get(o)) {
                if let Some(dd) = desc.dict() {
                    if let Some(dw) = doc.field(dd, "DW").num() {
                        f.dw = dw as f32;
                    }
                    if let Obj::Array(w) = doc.field(dd, "W") {
                        f.parse_cid_widths(doc, &w);
                    }
                }
            }
        } else {
            f.load_simple(doc, d);
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
        if let Obj::Dict(e) = &enc {
            if let Obj::Array(diffs) = doc.field(e, "Differences") {
                let mut code = 0u32;
                for o in diffs {
                    match doc.get(&o) {
                        Obj::Int(n) => code = n.max(0) as u32,
                        Obj::Name(n) => {
                            if let (Some(t), true) = (glyph_text(&n), code < 256) {
                                self.diff.insert(code as u8, t);
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
                "beginbfchar" => loop {
                    match p.parse(false) {
                        Some(Obj::Str(src)) => {
                            if let Some(Obj::Str(dst)) = p.parse(false) {
                                self.uni.insert(code_of(&src), utf16be(&dst));
                            }
                        }
                        _ => break,
                    }
                },
                "beginbfrange" => loop {
                    let Some(Obj::Str(lo)) = p.parse(false) else { break };
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
            return 500.0;
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
                    let width = self.cid_widths.get(&code).copied().unwrap_or(self.dw);
                    Glyph { text, width, is_space: false }
                } else {
                    let b = chunk[0];
                    let text = self.uni.get(&code).cloned().or_else(|| self.simple_text(b)).unwrap_or_default();
                    Glyph { text, width: self.simple_width(b), is_space: b == 32 }
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn composite_fonts_decode_two_bytes() {
        let mut f = Font { two_byte: true, code_len: 2, dw: 1000.0, ..Default::default() };
        f.uni.insert(0x0102, "hi".into());
        f.cid_widths.insert(0x0102, 250.0);
        let g = f.glyphs(&[1, 2, 0, 65]);
        assert_eq!((g[0].text.as_str(), g[0].width), ("hi", 250.0));
        assert_eq!((g[1].text.as_str(), g[1].width), ("A", 1000.0));
    }
}
