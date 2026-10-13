//! Text extraction: walks a page's operators and records the text lines
//! (with baseline and size) and images in drawing order, for reflowed reading.

use super::doc::Doc;
use super::font::{self, Font};
use super::image;
use super::obj::{Dict, Obj, Parser};
use super::page::Page;
use crate::gfx::Image;
use std::collections::HashMap;
use std::rc::Rc;

pub enum Piece {
    Line { y: f32, size: f32, text: String },
    Image(Image),
    /// the next pieces belong to a new page
    Page,
}

const MAX_IMAGES: usize = 400;
const MAX_FORM_DEPTH: u8 = 4;

#[derive(Clone)]
struct Gs {
    font: Option<Rc<Font>>,
    size: f32,
    tc: f32,
    tw: f32,
    th: f32,
}

struct Ts {
    gs: Gs,
    stack: Vec<Gs>,
    tm: [f32; 6],
    tlm: [f32; 6],
    leading: f32,
    /// the line being built: baseline y, font size, text
    cur: Option<(f32, f32, String)>,
    /// text-space x where the previous string ended
    end_e: f32,
}

const IDENT: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

pub struct Extractor<'a> {
    doc: &'a Doc,
    fonts: HashMap<u32, Rc<Font>>,
    images: HashMap<u32, Option<Image>>,
    uses: HashMap<u32, u32>,
    emitted: usize,
    pub pieces: Vec<Piece>,
}

fn nums(args: &[Obj]) -> Vec<f32> {
    args.iter().filter_map(|o| o.num()).map(|v| v as f32).collect()
}

impl<'a> Extractor<'a> {
    pub fn new(doc: &'a Doc) -> Self {
        Extractor { doc, fonts: HashMap::new(), images: HashMap::new(), uses: HashMap::new(), emitted: 0, pieces: vec![] }
    }

    /// Interprets one page.
    pub fn page(&mut self, page: &Page) {
        self.pieces.push(Piece::Page);
        let data = page.contents(self.doc);
        self.run(&data, &page.resources, 0);
    }

    fn font(&mut self, res: &Dict, name: &str) -> Option<Rc<Font>> {
        font::from_resources(self.doc, res, name, &mut self.fonts)
    }

    fn flush(&mut self, st: &mut Ts) {
        if let Some((y, size, text)) = st.cur.take() {
            if !text.trim().is_empty() {
                self.pieces.push(Piece::Line { y, size, text });
            }
        }
    }

    fn run(&mut self, data: &[u8], res: &Dict, depth: u8) {
        let mut st = Ts { gs: Gs { font: None, size: 12.0, tc: 0.0, tw: 0.0, th: 1.0 }, stack: vec![], tm: IDENT, tlm: IDENT, leading: 0.0, cur: None, end_e: 0.0 };
        let mut p = Parser::new(data, 0);
        let mut args: Vec<Obj> = vec![];
        while let Some(o) = p.parse(false) {
            let Obj::Op(op) = o else {
                args.push(o);
                if args.len() > 64 {
                    args.remove(0);
                }
                continue;
            };
            self.operator(&op, &args, &mut st, res, depth, &mut p);
            args.clear();
        }
        self.flush(&mut st);
    }

    fn operator(&mut self, op: &str, args: &[Obj], st: &mut Ts, res: &Dict, depth: u8, p: &mut Parser) {
        let n = nums(args);
        match op {
            "q" => st.stack.push(st.gs.clone()),
            "Q" => {
                if let Some(g) = st.stack.pop() {
                    st.gs = g;
                }
            }
            "BT" => {
                st.tm = IDENT;
                st.tlm = IDENT;
            }
            "Tf" => {
                if let (Some(Obj::Name(name)), Some(size)) = (args.first(), n.last()) {
                    st.gs.font = self.font(res, name);
                    st.gs.size = *size;
                }
            }
            "Tc" => st.gs.tc = n.first().copied().unwrap_or(0.0),
            "Tw" => st.gs.tw = n.first().copied().unwrap_or(0.0),
            "Tz" => st.gs.th = n.first().copied().unwrap_or(100.0) / 100.0,
            "TL" => st.leading = n.first().copied().unwrap_or(0.0),
            "Td" => {
                if n.len() >= 2 {
                    td(st, n[0], n[1]);
                }
            }
            "TD" => {
                if n.len() >= 2 {
                    st.leading = -n[1];
                    td(st, n[0], n[1]);
                }
            }
            "Tm" => {
                if n.len() >= 6 {
                    st.tm = [n[0], n[1], n[2], n[3], n[4], n[5]];
                    st.tlm = st.tm;
                }
            }
            "T*" => {
                let l = st.leading;
                td(st, 0.0, -l);
            }
            "Tj" => {
                if let Some(Obj::Str(s)) = args.first() {
                    self.show(st, s);
                }
            }
            "'" => {
                let l = st.leading;
                td(st, 0.0, -l);
                if let Some(Obj::Str(s)) = args.first() {
                    self.show(st, s);
                }
            }
            "\"" => {
                if n.len() >= 2 {
                    st.gs.tw = n[0];
                    st.gs.tc = n[1];
                }
                let l = st.leading;
                td(st, 0.0, -l);
                if let Some(Obj::Str(s)) = args.last() {
                    self.show(st, s);
                }
            }
            "TJ" => {
                if let Some(Obj::Array(items)) = args.first() {
                    for it in items {
                        match it {
                            Obj::Str(s) => self.show(st, s),
                            other => {
                                if let Some(v) = other.num() {
                                    let tx = -(v as f32) / 1000.0 * st.gs.size * st.gs.th;
                                    st.tm[4] += tx * st.tm[0];
                                    st.tm[5] += tx * st.tm[1];
                                }
                            }
                        }
                    }
                }
            }
            "Do" => {
                if let Some(Obj::Name(name)) = args.first() {
                    self.xobject(st, res, name, depth);
                }
            }
            "BI" => {
                // inline image: skip its dictionary and data
                while let Some(o) = p.parse(false) {
                    if o.is_op("ID") {
                        break;
                    }
                }
                let d = p.d;
                let mut i = p.pos;
                while i + 2 <= d.len() {
                    let ws_before = i == 0 || super::obj::is_ws(d[i - 1]);
                    let ws_after = d.get(i + 2).map_or(true, |&c| super::obj::is_ws(c));
                    if ws_before && &d[i..(i + 2).min(d.len())] == b"EI" && ws_after {
                        break;
                    }
                    i += 1;
                }
                p.pos = (i + 2).min(d.len());
            }
            _ => {}
        }
    }

    fn xobject(&mut self, st: &mut Ts, res: &Dict, name: &str, depth: u8) {
        let xobjs = self.doc.field(res, "XObject");
        let Some(entry) = xobjs.dict().and_then(|d| d.get(name)).cloned() else { return };
        let num = if let Obj::Ref(n, _) = entry { Some(n) } else { None };
        let Obj::Stream(s) = self.doc.get(&entry) else { return };
        match self.doc.field(&s.dict, "Subtype").name() {
            Some("Image") => {
                if self.emitted >= MAX_IMAGES {
                    return;
                }
                let key = num.unwrap_or(u32::MAX);
                let uses = self.uses.entry(key).or_insert(0);
                *uses += 1;
                // a logo repeated on every page is chrome, not content
                if *uses > 2 && key != u32::MAX {
                    return;
                }
                let doc = self.doc;
                let img = self.images.entry(key).or_insert_with(|| image::load(doc, &s)).clone();
                if let Some(img) = img {
                    self.flush(st);
                    self.emitted += 1;
                    self.pieces.push(Piece::Image(img));
                }
            }
            Some("Form") if depth < MAX_FORM_DEPTH => {
                self.flush(st);
                let own = self.doc.field(&s.dict, "Resources");
                let data = self.doc.decode(&s).0;
                match own.dict() {
                    Some(r) => self.run(&data, r, depth + 1),
                    None => self.run(&data, res, depth + 1),
                }
            }
            _ => {}
        }
    }

    fn show(&mut self, st: &mut Ts, bytes: &[u8]) {
        let glyphs = match &st.gs.font {
            Some(f) => f.glyphs(bytes),
            None => Font::default().glyphs(bytes),
        };
        let scale = st.tm[2].hypot(st.tm[3]);
        let eff = (st.gs.size * if scale > 0.0 { scale } else { 1.0 }).abs().max(1.0);
        let y = st.tm[5];
        let same = matches!(&st.cur, Some((cy, csize, _)) if (cy - y).abs() <= 0.45 * eff.max(*csize));
        if same {
            let gap = st.tm[4] - st.end_e;
            let first_char = glyphs.iter().find_map(|g| g.text.chars().next());
            if let Some((_, _, text)) = &mut st.cur {
                let spaced = text.ends_with(' ') || first_char == Some(' ');
                if !spaced && (gap > 0.17 * eff || gap < -0.5 * eff) {
                    text.push(' ');
                }
            }
        } else {
            self.flush(st);
            st.cur = Some((y, eff, String::new()));
        }
        for g in &glyphs {
            if let Some((_, _, text)) = &mut st.cur {
                text.push_str(&g.text);
            }
            let w = g.width / 1000.0 * st.gs.size + st.gs.tc + if g.is_space { st.gs.tw } else { 0.0 };
            st.tm[4] += w * st.gs.th * st.tm[0];
            st.tm[5] += w * st.gs.th * st.tm[1];
        }
        st.end_e = st.tm[4];
    }
}

fn td(st: &mut Ts, tx: f32, ty: f32) {
    st.tlm[4] += tx * st.tlm[0] + ty * st.tlm[2];
    st.tlm[5] += tx * st.tlm[1] + ty * st.tlm[3];
    st.tm = st.tlm;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg::pdf::doc::tests::tiny_pdf;
    use crate::cg::pdf::page::pages;

    fn lines(content: &str) -> Vec<(f32, String)> {
        let doc = Doc::open(tiny_pdf(content)).unwrap();
        let list = pages(&doc);
        let mut x = Extractor::new(&doc);
        x.page(&list[0]);
        x.pieces.iter().filter_map(|p| if let Piece::Line { y, text, .. } = p { Some((*y, text.clone())) } else { None }).collect()
    }

    #[test]
    fn lines_words_and_kerning() {
        let l = lines("BT /F1 12 Tf 72 700 Td (Hello) Tj ( world) Tj 0 -14 Td [(Sec)-20(ond) -300 (line)] TJ ET");
        assert_eq!(l, vec![(700.0, "Hello world".to_string()), (686.0, "Second line".to_string())]);
    }

    #[test]
    fn gaps_between_positioned_words_become_spaces() {
        let l = lines("BT /F1 10 Tf 72 700 Td (one) Tj 40 0 Td (two) Tj ET");
        assert_eq!(l[0].1, "one two");
        let l = lines("BT /F1 10 Tf 72 700 Td (ab) Tj 10 0 Td (cd) Tj ET");
        assert_eq!(l[0].1, "abcd", "abutting glyph runs stay one word");
    }

    #[test]
    fn tm_leading_and_quote_operators() {
        let l = lines("BT /F1 12 Tf 1 0 0 1 50 500 Tm 14 TL (A) Tj T* (B) Tj (C) ' ET");
        assert_eq!(l.iter().map(|x| x.1.as_str()).collect::<Vec<_>>(), ["A", "B", "C"]);
        assert_eq!(l[1].0, 486.0);
    }

    #[test]
    fn inline_images_are_skipped() {
        let l = lines("BT /F1 12 Tf 72 700 Td (a) Tj ET q BI /W 1 /H 1 ID \u{1}\u{2} EI ET Q BT /F1 12 Tf 72 600 Td (b) Tj ET");
        assert_eq!(l.len(), 2);
    }
}
