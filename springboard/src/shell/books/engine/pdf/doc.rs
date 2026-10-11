//! The PDF file structure: cross-reference tables and streams, object
//! streams, lazy object loading, stream filters, and a scan-based rebuild
//! for files whose xref is damaged.

use super::obj::{is_ws, Dict, Obj, Parser, Stream};
use crate::shell::books::engine::inflate;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entry {
    Offset(usize),
    /// (object stream number, index inside it)
    InStream(u32, usize),
}

pub struct Doc {
    data: Vec<u8>,
    xref: HashMap<u32, Entry>,
    pub trailer: Dict,
    cache: RefCell<HashMap<u32, Obj>>,
    objstms: RefCell<HashMap<u32, Rc<Vec<(u32, Obj)>>>>,
    loading: RefCell<HashSet<u32>>,
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() || from > hay.len() - needle.len() {
        return None;
    }
    hay[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).rposition(|w| w == needle)
}

impl Doc {
    pub fn open(data: Vec<u8>) -> Result<Doc, String> {
        if find(&data[..data.len().min(1024)], b"%PDF-", 0).is_none() {
            return Err("not a PDF file".into());
        }
        let mut doc = Doc { data, xref: HashMap::new(), trailer: Dict::new(), cache: Default::default(), objstms: Default::default(), loading: Default::default() };
        let ok = doc.read_xref_chain().is_some() && doc.catalog().is_some();
        if !ok {
            doc.rebuild();
        }
        if doc.trailer.contains_key("Encrypt") {
            return Err("this PDF is encrypted".into());
        }
        if doc.catalog().is_none() {
            return Err("damaged PDF: no document catalog".into());
        }
        Ok(doc)
    }

    pub fn catalog(&self) -> Option<Dict> {
        match self.get(self.trailer.get("Root")?) {
            Obj::Dict(d) => Some(d),
            _ => None,
        }
    }

    // --- cross references ---

    fn read_xref_chain(&mut self) -> Option<()> {
        let tail_start = self.data.len().saturating_sub(2048);
        let sx = rfind(&self.data[tail_start..], b"startxref")? + tail_start;
        let mut p = Parser::new(&self.data, sx + 9);
        let mut next = Some(p.parse(false)?.int()? as usize);
        let mut seen = HashSet::new();
        let mut first = true;
        while let Some(off) = next.take() {
            if !seen.insert(off) || off >= self.data.len() {
                break;
            }
            let (entries, trailer) = self.read_xref_at(off)?;
            for (n, e) in entries {
                self.xref.entry(n).or_insert(e);
            }
            // a hybrid file's XRefStm entries rank after the table's own
            if let Some(Obj::Int(sm)) = trailer.get("XRefStm") {
                if let Some((more, _)) = self.read_xref_at(*sm as usize) {
                    for (n, e) in more {
                        self.xref.entry(n).or_insert(e);
                    }
                }
            }
            next = match trailer.get("Prev") {
                Some(o) => o.int().map(|v| v as usize),
                None => None,
            };
            if first {
                self.trailer = trailer;
                first = false;
            } else {
                for (k, v) in trailer {
                    self.trailer.entry(k).or_insert(v);
                }
            }
        }
        (!first).then_some(())
    }

    fn read_xref_at(&self, off: usize) -> Option<(Vec<(u32, Entry)>, Dict)> {
        let mut p = Parser::new(&self.data, off);
        p.skip_ws();
        if self.data[p.pos..].starts_with(b"xref") {
            p.pos += 4;
            let mut out = vec![];
            loop {
                p.skip_ws();
                match p.parse(false)? {
                    Obj::Op(o) if o == "trailer" => break,
                    Obj::Int(start) => {
                        let count = p.parse(false)?.int()?;
                        for i in 0..count {
                            let o = p.parse(false)?.int()?;
                            let _gen = p.parse(false)?.int()?;
                            let kind = p.parse(false)?;
                            if kind.is_op("n") && o > 0 {
                                out.push(((start + i) as u32, Entry::Offset(o as usize)));
                            }
                        }
                    }
                    _ => return None,
                }
            }
            let Obj::Dict(trailer) = p.parse(true)? else { return None };
            Some((out, trailer))
        } else {
            // an xref stream: "N G obj << ... >> stream"
            let s = match self.parse_indirect_at(off)?.1 {
                Obj::Stream(s) => s,
                _ => return None,
            };
            if s.dict.get("Type").and_then(Obj::name) != Some("XRef") {
                return None;
            }
            let w: Vec<usize> = s.dict.get("W")?.array()?.iter().map(|o| o.int().unwrap_or(0) as usize).collect();
            if w.len() < 3 || w.iter().any(|&x| x > 8) {
                return None;
            }
            let size = s.dict.get("Size")?.int()?;
            let index: Vec<i64> = match s.dict.get("Index") {
                Some(Obj::Array(a)) => a.iter().filter_map(Obj::int).collect(),
                _ => vec![0, size],
            };
            let data = self.decode(&s).0;
            let row = w[0] + w[1] + w[2];
            if row == 0 {
                return None;
            }
            let field = |b: &[u8]| b.iter().fold(0u64, |a, &x| a << 8 | x as u64);
            let mut out = vec![];
            let mut at = 0;
            for pair in index.chunks(2) {
                if pair.len() < 2 {
                    break;
                }
                for i in 0..pair.get(1).copied().unwrap_or(0) {
                    let Some(r) = data.get(at..at + row) else { break };
                    at += row;
                    let kind = if w[0] == 0 { 1 } else { field(&r[..w[0]]) };
                    let a = field(&r[w[0]..w[0] + w[1]]);
                    let b = field(&r[w[0] + w[1]..]);
                    let num = (pair[0] + i) as u32;
                    match kind {
                        1 => out.push((num, Entry::Offset(a as usize))),
                        2 => out.push((num, Entry::InStream(a as u32, b as usize))),
                        _ => {}
                    }
                }
            }
            Some((out, s.dict.clone()))
        }
    }

    /// Scans the whole file for `N G obj` and trailers when the xref cannot be trusted.
    fn rebuild(&mut self) {
        self.xref.clear();
        self.cache.borrow_mut().clear();
        self.objstms.borrow_mut().clear();
        self.trailer = Dict::new();
        let d = &self.data;
        let mut i = 0;
        let mut found: Vec<(u32, usize)> = vec![];
        let mut trailers: Vec<usize> = vec![];
        while let Some(p) = find(d, b"obj", i) {
            i = p + 3;
            if p >= 3 && &d[p - 3..p] == b"end" {
                continue;
            }
            // walk back over `N G `
            let mut q = p;
            let mut nums = vec![];
            for _ in 0..2 {
                while q > 0 && is_ws(d[q - 1]) {
                    q -= 1;
                }
                let end = q;
                while q > 0 && d[q - 1].is_ascii_digit() {
                    q -= 1;
                }
                if q == end {
                    break;
                }
                nums.push((q, std::str::from_utf8(&d[q..end]).unwrap_or("0").parse::<u64>().unwrap_or(0)));
            }
            if nums.len() == 2 {
                let (start, num) = nums[1];
                found.push((num as u32, start));
            }
        }
        let mut j = 0;
        while let Some(p) = find(d, b"trailer", j) {
            trailers.push(p + 7);
            j = p + 7;
        }
        for (n, off) in found {
            self.xref.insert(n, Entry::Offset(off));
        }
        for off in trailers {
            if let Some(Obj::Dict(t)) = Parser::new(&self.data, off).parse(true) {
                for (k, v) in t {
                    self.trailer.insert(k, v);
                }
            }
        }
        // objects stored in object streams, and xref-stream dictionaries that carry the trailer
        let nums: Vec<u32> = self.xref.keys().copied().collect();
        for n in nums {
            let Obj::Stream(s) = self.get(&Obj::Ref(n, 0)) else { continue };
            match s.dict.get("Type").and_then(Obj::name) {
                Some("ObjStm") => {
                    for (idx, (num, _)) in self.object_stream(n).iter().enumerate() {
                        self.xref.entry(*num).or_insert(Entry::InStream(n, idx));
                    }
                }
                Some("XRef") => {
                    for (k, v) in &s.dict {
                        if matches!(k.as_str(), "Root" | "Info" | "Encrypt" | "ID") {
                            self.trailer.entry(k.clone()).or_insert_with(|| v.clone());
                        }
                    }
                }
                _ => {}
            }
        }
        if !self.trailer.contains_key("Root") {
            let nums: Vec<u32> = self.xref.keys().copied().collect();
            for n in nums {
                if let Obj::Dict(d) = self.get(&Obj::Ref(n, 0)) {
                    if d.get("Type").and_then(Obj::name) == Some("Catalog") {
                        self.trailer.insert("Root".into(), Obj::Ref(n, 0));
                        break;
                    }
                }
            }
        }
    }

    // --- objects ---

    /// Parses `N G obj ... endobj` at `off`, returning the number and object.
    fn parse_indirect_at(&self, off: usize) -> Option<(u32, Obj)> {
        let mut p = Parser::new(&self.data, off);
        let num = p.parse(false)?.int()? as u32;
        p.parse(false)?.int()?;
        if !p.parse(false)?.is_op("obj") {
            return None;
        }
        let o = p.parse(true)?;
        if let Obj::Dict(dict) = &o {
            p.skip_ws();
            if self.data[p.pos.min(self.data.len())..].starts_with(b"stream") {
                let mut s = p.pos + 6;
                if self.data.get(s) == Some(&b'\r') {
                    s += 1;
                }
                if self.data.get(s) == Some(&b'\n') {
                    s += 1;
                }
                let declared = match dict.get("Length") {
                    Some(l) => self.get(l).int().filter(|&n| n >= 0).map(|n| n as usize),
                    None => None,
                };
                let end = declared
                    .filter(|&n| s + n <= self.data.len() && self.after_is_endstream(s + n))
                    .map(|n| s + n)
                    .or_else(|| find(&self.data, b"endstream", s).map(|e| trim_eol(&self.data, s, e)))
                    .unwrap_or(self.data.len());
                let data = self.data[s..end.max(s)].to_vec();
                return Some((num, Obj::Stream(Rc::new(Stream { dict: dict.clone(), data }))));
            }
        }
        Some((num, o))
    }

    fn after_is_endstream(&self, at: usize) -> bool {
        let mut p = at;
        while self.data.get(p).is_some_and(|&c| is_ws(c)) {
            p += 1;
        }
        self.data[p.min(self.data.len())..].starts_with(b"endstream")
    }

    /// The parsed objects of object stream `n`, as `(number, object)` pairs.
    fn object_stream(&self, n: u32) -> Rc<Vec<(u32, Obj)>> {
        if let Some(v) = self.objstms.borrow().get(&n) {
            return v.clone();
        }
        let mut out = vec![];
        if let Obj::Stream(s) = self.get(&Obj::Ref(n, 0)) {
            let count = s.dict.get("N").map(|o| self.get(o)).and_then(|o| o.int()).unwrap_or(0).max(0) as usize;
            let first = s.dict.get("First").map(|o| self.get(o)).and_then(|o| o.int()).unwrap_or(0).max(0) as usize;
            let data = self.decode(&s).0;
            let mut head = Parser::new(&data, 0);
            let mut pairs = vec![];
            for _ in 0..count {
                let (Some(num), Some(off)) = (head.parse(false).and_then(|o| o.int()), head.parse(false).and_then(|o| o.int())) else { break };
                pairs.push((num as u32, off as usize));
            }
            for (num, off) in pairs {
                let mut p = Parser::new(&data, first + off);
                out.push((num, p.parse(true).unwrap_or(Obj::Null)));
            }
        }
        let rc = Rc::new(out);
        self.objstms.borrow_mut().insert(n, rc.clone());
        rc
    }

    /// Loads indirect object `num` (Null when missing).
    pub fn load(&self, num: u32) -> Obj {
        if let Some(o) = self.cache.borrow().get(&num) {
            return o.clone();
        }
        if !self.loading.borrow_mut().insert(num) {
            return Obj::Null;
        }
        let o = match self.xref.get(&num).copied() {
            Some(Entry::Offset(off)) => self.parse_indirect_at(off).filter(|(n, _)| *n == num).map(|(_, o)| o).unwrap_or(Obj::Null),
            Some(Entry::InStream(stm, idx)) => {
                let list = self.object_stream(stm);
                match list.get(idx) {
                    Some((n, o)) if *n == num => o.clone(),
                    _ => list.iter().find(|(n, _)| *n == num).map(|(_, o)| o.clone()).unwrap_or(Obj::Null),
                }
            }
            None => Obj::Null,
        };
        self.loading.borrow_mut().remove(&num);
        self.cache.borrow_mut().insert(num, o.clone());
        o
    }

    /// Follows references until a direct object (Null on a broken chain).
    pub fn get(&self, o: &Obj) -> Obj {
        let mut cur = o.clone();
        for _ in 0..16 {
            match cur {
                Obj::Ref(n, _) => cur = self.load(n),
                other => return other,
            }
        }
        Obj::Null
    }

    /// `dict[key]`, resolved.
    pub fn field(&self, d: &Dict, key: &str) -> Obj {
        d.get(key).map(|o| self.get(o)).unwrap_or(Obj::Null)
    }

    // --- streams ---

    /// Decodes a stream's filters. The second value names an image filter
    /// (DCTDecode, JPXDecode, CCITTFaxDecode, JBIG2Decode) that was left
    /// undone, in which case the bytes are still in that format.
    pub fn decode(&self, s: &Stream) -> (Vec<u8>, Option<String>) {
        let filters: Vec<String> = match self.field(&s.dict, "Filter") {
            Obj::Name(n) => vec![n],
            Obj::Array(a) => a.iter().filter_map(|o| self.get(o).name().map(str::to_string)).collect(),
            _ => vec![],
        };
        let parms: Vec<Dict> = match s.dict.get("DecodeParms").or_else(|| s.dict.get("DP")).map(|o| self.get(o)) {
            Some(Obj::Dict(d)) => vec![d],
            Some(Obj::Array(a)) => a.iter().map(|o| self.get(o).dict().cloned().unwrap_or_default()).collect(),
            _ => vec![],
        };
        let mut data = s.data.clone();
        for (i, f) in filters.iter().enumerate() {
            let parm = parms.get(i).cloned().unwrap_or_default();
            data = match f.as_str() {
                "FlateDecode" | "Fl" => predictor(inflate::zlib_lenient(&data), &parm, self),
                "LZWDecode" | "LZW" => predictor(lzw(&data, self.field(&parm, "EarlyChange").int().unwrap_or(1) != 0), &parm, self),
                "ASCIIHexDecode" | "AHx" => ascii_hex(&data),
                "ASCII85Decode" | "A85" => ascii85(&data),
                "RunLengthDecode" | "RL" => run_length(&data),
                "Crypt" => data,
                "DCTDecode" | "DCT" => return (data, Some("DCTDecode".into())),
                other => return (data, Some(other.to_string())),
            };
        }
        (data, None)
    }
}

fn trim_eol(d: &[u8], start: usize, mut end: usize) -> usize {
    if end > start && d[end - 1] == b'\n' {
        end -= 1;
    }
    if end > start && d[end - 1] == b'\r' {
        end -= 1;
    }
    end
}

fn predictor(data: Vec<u8>, parm: &Dict, doc: &Doc) -> Vec<u8> {
    let pred = doc.field(parm, "Predictor").int().unwrap_or(1);
    if pred < 2 {
        return data;
    }
    let colors = doc.field(parm, "Colors").int().unwrap_or(1).max(1) as usize;
    let bpc = doc.field(parm, "BitsPerComponent").int().unwrap_or(8).max(1) as usize;
    let columns = doc.field(parm, "Columns").int().unwrap_or(1).max(1) as usize;
    let bpp = (colors * bpc).div_ceil(8).max(1);
    let row = (colors * bpc * columns).div_ceil(8);
    if pred == 2 {
        // TIFF predictor, byte samples only
        if bpc != 8 {
            return data;
        }
        let mut out = data;
        for r in out.chunks_mut(row) {
            for i in colors..r.len() {
                r[i] = r[i].wrapping_add(r[i - colors]);
            }
        }
        return out;
    }
    // PNG predictors: each row starts with its filter type
    let mut out = Vec::with_capacity(data.len());
    let mut prev = vec![0u8; row];
    for chunk in data.chunks(row + 1) {
        let kind = chunk[0];
        let mut cur: Vec<u8> = chunk[1..].to_vec();
        cur.resize(row, 0);
        for i in 0..row {
            let a = if i >= bpp { cur[i - bpp] as i32 } else { 0 };
            let b = prev[i] as i32;
            let c = if i >= bpp { prev[i - bpp] as i32 } else { 0 };
            let add = match kind {
                1 => a,
                2 => b,
                3 => (a + b) / 2,
                4 => {
                    let p = a + b - c;
                    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                _ => 0,
            };
            cur[i] = cur[i].wrapping_add(add as u8);
        }
        out.extend_from_slice(&cur);
        prev = cur;
    }
    out
}

fn ascii_hex(d: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    let mut hi = None;
    for &c in d {
        if c == b'>' {
            break;
        }
        let Some(v) = (c as char).to_digit(16) else { continue };
        match hi.take() {
            Some(h) => out.push((h << 4 | v) as u8),
            None => hi = Some(v),
        }
    }
    if let Some(h) = hi {
        out.push((h << 4) as u8);
    }
    out
}

fn ascii85(d: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    let mut group = [0u32; 5];
    let mut n = 0;
    let mut i = 0;
    while i < d.len() {
        let c = d[i];
        i += 1;
        match c {
            b'~' => break,
            b'z' if n == 0 => out.extend_from_slice(&[0; 4]),
            b'!'..=b'u' => {
                group[n] = (c - b'!') as u32;
                n += 1;
                if n == 5 {
                    let v = group.iter().fold(0u32, |a, &g| a.wrapping_mul(85).wrapping_add(g));
                    out.extend_from_slice(&v.to_be_bytes());
                    n = 0;
                }
            }
            _ => {}
        }
    }
    if n > 1 {
        for g in group.iter_mut().skip(n) {
            *g = 84;
        }
        let v = group.iter().fold(0u32, |a, &g| a.wrapping_mul(85).wrapping_add(g));
        out.extend_from_slice(&v.to_be_bytes()[..n - 1]);
    }
    out
}

fn run_length(d: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    let mut i = 0;
    while i < d.len() {
        let n = d[i] as usize;
        i += 1;
        if n == 128 {
            break;
        }
        if n < 128 {
            let end = (i + n + 1).min(d.len());
            out.extend_from_slice(&d[i..end]);
            i = end;
        } else if let Some(&b) = d.get(i) {
            out.extend(std::iter::repeat(b).take(257 - n));
            i += 1;
        }
    }
    out
}

fn lzw(d: &[u8], early: bool) -> Vec<u8> {
    let mut out: Vec<u8> = vec![];
    let mut table: Vec<Vec<u8>> = vec![];
    let reset = |t: &mut Vec<Vec<u8>>| {
        t.clear();
        t.extend((0..=255u8).map(|b| vec![b]));
        t.push(vec![]); // 256 clear
        t.push(vec![]); // 257 end
    };
    reset(&mut table);
    let (mut bits, mut acc, mut width) = (0u32, 0u32, 9u32);
    let mut prev: Option<Vec<u8>> = None;
    for &byte in d {
        acc = acc << 8 | byte as u32;
        bits += 8;
        while bits >= width {
            let code = (acc >> (bits - width)) & ((1 << width) - 1);
            bits -= width;
            if code == 256 {
                reset(&mut table);
                width = 9;
                prev = None;
                continue;
            }
            if code == 257 {
                return out;
            }
            let entry = if (code as usize) < table.len() {
                table[code as usize].clone()
            } else if let Some(p) = &prev {
                let mut e = p.clone();
                e.push(p[0]);
                e
            } else {
                return out;
            };
            out.extend_from_slice(&entry);
            if let Some(p) = prev.take() {
                let mut e = p;
                e.push(entry[0]);
                table.push(e);
            }
            prev = Some(entry);
            let size = table.len() + usize::from(early);
            width = match size {
                0..=511 => 9,
                512..=1023 => 10,
                1024..=2047 => 11,
                _ => 12,
            };
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A tiny two-page-capable PDF with a classic xref table; returns the bytes.
    pub fn tiny_pdf(content: &str) -> Vec<u8> {
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offs = vec![];
        let objs: Vec<String> = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources << /Font << /F1 5 0 R >> >> >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>".into(),
            format!("<< /Length {} >>\nstream\n{}\nendstream", content.len(), content),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into(),
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

    #[test]
    fn reads_a_classic_xref_table() {
        let doc = Doc::open(tiny_pdf("BT ET")).unwrap();
        assert_eq!(doc.catalog().unwrap()["Type"], Obj::Name("Catalog".into()));
        let Obj::Stream(s) = doc.load(4) else { panic!("object 4 is a stream") };
        assert_eq!(s.data, b"BT ET");
        assert_eq!(doc.get(&Obj::Ref(3, 0)).dict().unwrap()["Contents"], Obj::Ref(4, 0));
    }

    #[test]
    fn rebuilds_when_the_xref_is_garbage() {
        let mut pdf = tiny_pdf("BT ET");
        let at = rfind(&pdf, b"startxref").unwrap();
        pdf.truncate(at);
        pdf.extend_from_slice(b"startxref\n999999\n%%EOF\n");
        let doc = Doc::open(pdf).unwrap();
        assert!(doc.catalog().is_some());
        assert!(matches!(doc.load(4), Obj::Stream(_)));
    }

    #[test]
    fn rejects_non_pdf_and_encrypted() {
        assert!(Doc::open(b"hello".to_vec()).is_err());
        let mut pdf = tiny_pdf("");
        let s = String::from_utf8(pdf.clone()).unwrap().replace("/Root 1 0 R", "/Root 1 0 R /Encrypt 9 0 R");
        pdf = s.into_bytes();
        let e = Doc::open(pdf).err().unwrap();
        assert!(e.contains("encrypted"), "{e}");
    }

    #[test]
    fn reads_an_xref_stream_with_an_object_stream() {
        // objects 1 (catalog) and 2 (pages) live in object stream 4; object 3 is a plain page-less dict
        let o1 = "<< /Type /Catalog /Pages 2 0 R >>";
        let o2 = "<< /Type /Pages /Kids [] /Count 0 >>";
        let header = format!("1 0 2 {} ", o1.len() + 1);
        let body = format!("{header}{o1} {o2}");
        let mut out = b"%PDF-1.5\n".to_vec();
        let objstm_off = out.len();
        out.extend_from_slice(format!("4 0 obj\n<< /Type /ObjStm /N 2 /First {} /Length {} >>\nstream\n{}\nendstream\nendobj\n", header.len(), body.len(), body).as_bytes());
        let x_off = out.len();
        // rows: type(1) a(2) b(1); objects 0..=4
        let rows: Vec<u8> = vec![0, 0, 0, 255, 2, 0, 4, 0, 2, 0, 4, 1, 0, 0, 0, 0, 1, (objstm_off >> 8) as u8, objstm_off as u8, 0, 1, (x_off >> 8) as u8, x_off as u8, 0];
        out.extend_from_slice(format!("5 0 obj\n<< /Type /XRef /Size 6 /W [1 2 1] /Root 1 0 R /Length {} >>\nstream\n", rows.len()).as_bytes());
        out.extend_from_slice(&rows);
        out.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{x_off}\n%%EOF\n").as_bytes());
        let doc = Doc::open(out).unwrap();
        assert_eq!(doc.catalog().unwrap()["Pages"], Obj::Ref(2, 0));
        assert_eq!(doc.get(&Obj::Ref(2, 0)).dict().unwrap()["Count"], Obj::Int(0));
    }

    #[test]
    fn filters() {
        assert_eq!(ascii_hex(b"48 65 6C6c 6F>"), b"Hello");
        assert_eq!(ascii85(b"87cURDZ~>"), b"Hello");
        assert_eq!(run_length(&[2, b'a', b'b', b'c', 254, b'z', 128]), b"abczzz");
        // LZW: the PDF spec's example
        let enc = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        assert_eq!(lzw(&enc, true), [45, 45, 45, 45, 45, 65, 45, 45, 45, 66]);
    }

    #[test]
    fn png_up_predictor() {
        let doc = Doc::open(tiny_pdf("")).unwrap();
        let mut parm = Dict::new();
        parm.insert("Predictor".into(), Obj::Int(12));
        parm.insert("Columns".into(), Obj::Int(3));
        let out = predictor(vec![2, 1, 2, 3, 2, 1, 1, 1], &parm, &doc);
        assert_eq!(out, [1, 2, 3, 2, 3, 4]);
    }
}
