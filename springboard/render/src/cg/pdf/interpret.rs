//! Content-stream interpretation: runs a page's operators and reports each paint operation to a [`PdfSink`]. Paths,
//! colours, text and images are resolved here into device-independent calls; the sink decides how they reach pixels.
//!
//! Not drawn: tiling patterns (their fills keep the last solid colour), soft masks, and the stroke part of text
//! rendering modes (glyphs are filled only). Inline images are decoded like image XObjects.

use super::color::ColorSpace;
use super::doc::Doc;
use super::font::{self, Font};
use super::image;
use super::obj::{is_ws, Dict, Obj, Parser, Stream};
use super::page::{self, Matrix, Page};
use super::shading;
use super::sink::{Blend, FillRule, LineCap, LineJoin, PathSeg, PdfImage, PdfSink, Rgb, Stroke};
use std::collections::HashMap;
use std::rc::Rc;

/// Forms, Type 3 glyph procedures and their nesting stop at this depth.
const MAX_DEPTH: u8 = 6;
/// Operators run per page before the rest of the content is ignored, so a hostile file cannot stall the reader.
const MAX_OPS: usize = 4_000_000;

/// Draws `page` through `sink`. `ctm` maps the page's default space (points, y up) to device pixels, usually
/// [`Page::transform`]. Every `save` the sink receives is matched by a `restore` before this returns.
pub fn draw_page(doc: &Doc, page: &Page, ctm: Matrix, sink: &mut dyn PdfSink) {
    let data = page.contents(doc);
    let mut it = Interp::new(doc, sink, ctm);
    it.run(&data, &page.resources, 0);
    it.finish();
}

/// The graphics and text state. `q` saves it, `Q` restores it.
#[derive(Clone)]
struct State {
    ctm: Matrix,
    fill_cs: Rc<ColorSpace>,
    stroke_cs: Rc<ColorSpace>,
    fill: Rgb,
    stroke: Rgb,
    line: Stroke,
    fill_alpha: f32,
    stroke_alpha: f32,
    blend: Blend,
    font: Option<Rc<Font>>,
    font_size: f64,
    char_space: f64,
    word_space: f64,
    /// horizontal scaling as a fraction (`Tz` / 100)
    scale: f64,
    leading: f64,
    rise: f64,
    /// text rendering mode (`Tr`)
    mode: i64,
}

struct Interp<'a, 'd> {
    doc: &'a Doc,
    sink: &'d mut dyn PdfSink,
    st: State,
    stack: Vec<State>,
    /// the path under construction, in user space
    path: Vec<PathSeg>,
    /// the clip that the next painting operator applies (`W` or `W*`)
    pending_clip: Option<FillRule>,
    /// text matrix and text line matrix
    text: Matrix,
    line: Matrix,
    fonts: HashMap<u32, Rc<Font>>,
    images: HashMap<u32, Rc<PdfImage>>,
    gray: Rc<ColorSpace>,
    rgb: Rc<ColorSpace>,
    cmyk: Rc<ColorSpace>,
    ops: usize,
}

impl<'a, 'd> Interp<'a, 'd> {
    fn new(doc: &'a Doc, sink: &'d mut dyn PdfSink, ctm: Matrix) -> Self {
        let gray = Rc::new(ColorSpace::Gray);
        let st = State {
            ctm,
            fill_cs: gray.clone(),
            stroke_cs: gray.clone(),
            fill: [0.0; 3],
            stroke: [0.0; 3],
            line: Stroke::default(),
            fill_alpha: 1.0,
            stroke_alpha: 1.0,
            blend: Blend::Normal,
            font: None,
            font_size: 0.0,
            char_space: 0.0,
            word_space: 0.0,
            scale: 1.0,
            leading: 0.0,
            rise: 0.0,
            mode: 0,
        };
        Interp {
            doc,
            sink,
            st,
            stack: vec![],
            path: vec![],
            pending_clip: None,
            text: page::IDENTITY,
            line: page::IDENTITY,
            fonts: HashMap::new(),
            images: HashMap::new(),
            gray,
            rgb: Rc::new(ColorSpace::Rgb),
            cmyk: Rc::new(ColorSpace::Cmyk),
            ops: 0,
        }
    }

    /// Closes the states a stream left open.
    fn finish(&mut self) {
        while self.stack.pop().is_some() {
            self.sink.restore();
        }
    }

    /// Runs one content stream with the resources `res`.
    fn run(&mut self, data: &[u8], res: &Dict, depth: u8) {
        let mut p = Parser::new(data, 0);
        let mut args: Vec<Obj> = vec![];
        while let Some(o) = p.parse(false) {
            self.ops += 1;
            if self.ops > MAX_OPS {
                return;
            }
            match o {
                Obj::Op(op) => {
                    if op == "BI" {
                        self.inline_image(&mut p, res);
                    } else {
                        self.operator(&op, &args, res, depth);
                    }
                    args.clear();
                }
                other => {
                    args.push(other);
                    if args.len() > 64 {
                        args.remove(0);
                    }
                }
            }
        }
    }

    fn operator(&mut self, op: &str, args: &[Obj], res: &Dict, depth: u8) {
        let n = nums(args);
        let arg = |i: usize| n.get(i).copied().unwrap_or(0.0);
        match op {
            "q" => {
                self.stack.push(self.st.clone());
                self.sink.save();
            }
            "Q" => {
                if let Some(s) = self.stack.pop() {
                    self.st = s;
                    self.sink.restore();
                }
            }
            "cm" => {
                if n.len() >= 6 {
                    let m = [arg(0), arg(1), arg(2), arg(3), arg(4), arg(5)];
                    self.st.ctm = page::mul(&m, &self.st.ctm);
                }
            }
            "w" => self.st.line.width = arg(0),
            "J" => {
                self.st.line.cap = match arg(0) as i64 {
                    1 => LineCap::Round,
                    2 => LineCap::Square,
                    _ => LineCap::Butt,
                }
            }
            "j" => {
                self.st.line.join = match arg(0) as i64 {
                    1 => LineJoin::Round,
                    2 => LineJoin::Bevel,
                    _ => LineJoin::Miter,
                }
            }
            "M" => self.st.line.miter_limit = arg(0),
            "d" => {
                self.st.line.dash = args.first().and_then(Obj::array).map(nums).unwrap_or_default();
                self.st.line.phase = arg(0);
            }
            "gs" => {
                if let Some(Obj::Name(name)) = args.first() {
                    self.ext_gstate(res, name);
                }
            }

            "m" => {
                if n.len() >= 2 {
                    self.path.push(PathSeg::Move(arg(0), arg(1)));
                }
            }
            "l" => {
                if n.len() >= 2 {
                    self.path.push(PathSeg::Line(arg(0), arg(1)));
                }
            }
            "c" => {
                if n.len() >= 6 {
                    self.path.push(PathSeg::Cubic(arg(0), arg(1), arg(2), arg(3), arg(4), arg(5)));
                }
            }
            "v" => {
                if n.len() >= 4 {
                    let (x, y) = self.current_point();
                    self.path.push(PathSeg::Cubic(x, y, arg(0), arg(1), arg(2), arg(3)));
                }
            }
            "y" => {
                if n.len() >= 4 {
                    self.path.push(PathSeg::Cubic(arg(0), arg(1), arg(2), arg(3), arg(2), arg(3)));
                }
            }
            "h" => self.close_path(),
            "re" => {
                if n.len() >= 4 {
                    let (x, y, w, h) = (arg(0), arg(1), arg(2), arg(3));
                    self.path.extend([PathSeg::Move(x, y), PathSeg::Line(x + w, y), PathSeg::Line(x + w, y + h), PathSeg::Line(x, y + h), PathSeg::Close]);
                }
            }
            "S" => self.paint(None, true),
            "s" => {
                self.close_path();
                self.paint(None, true);
            }
            "f" | "F" => self.paint(Some(FillRule::NonZero), false),
            "f*" => self.paint(Some(FillRule::EvenOdd), false),
            "B" => self.paint(Some(FillRule::NonZero), true),
            "B*" => self.paint(Some(FillRule::EvenOdd), true),
            "b" => {
                self.close_path();
                self.paint(Some(FillRule::NonZero), true);
            }
            "b*" => {
                self.close_path();
                self.paint(Some(FillRule::EvenOdd), true);
            }
            "n" => self.paint(None, false),
            "W" => self.pending_clip = Some(FillRule::NonZero),
            "W*" => self.pending_clip = Some(FillRule::EvenOdd),

            "g" | "G" => {
                let cs = self.gray.clone();
                self.set_color(op == "G", cs, &[arg(0) as f32]);
            }
            "rg" | "RG" => {
                let cs = self.rgb.clone();
                self.set_color(op == "RG", cs, &f32s(&n));
            }
            "k" | "K" => {
                let cs = self.cmyk.clone();
                self.set_color(op == "K", cs, &f32s(&n));
            }
            "cs" | "CS" => {
                if let Some(o) = args.first() {
                    if let Some(cs) = ColorSpace::parse(self.doc, o, Some(res)) {
                        let cs = Rc::new(cs);
                        let initial = cs.initial();
                        self.set_color(op == "CS", cs, &initial);
                    }
                }
            }
            "sc" | "scn" => self.set_components(false, &f32s(&n)),
            "SC" | "SCN" => self.set_components(true, &f32s(&n)),

            "BT" => {
                self.text = page::IDENTITY;
                self.line = page::IDENTITY;
            }
            "Tc" => self.st.char_space = arg(0),
            "Tw" => self.st.word_space = arg(0),
            "Tz" => {
                if let Some(v) = n.first() {
                    self.st.scale = *v / 100.0;
                }
            }
            "TL" => self.st.leading = arg(0),
            "Ts" => self.st.rise = arg(0),
            "Tr" => self.st.mode = arg(0) as i64,
            "Tf" => {
                if let Some(Obj::Name(name)) = args.first() {
                    self.st.font = font::from_resources(self.doc, res, name, &mut self.fonts);
                    self.st.font_size = arg(0);
                }
            }
            "Td" => {
                if n.len() >= 2 {
                    self.move_text(arg(0), arg(1));
                }
            }
            "TD" => {
                if n.len() >= 2 {
                    self.st.leading = -arg(1);
                    self.move_text(arg(0), arg(1));
                }
            }
            "Tm" => {
                if n.len() >= 6 {
                    let m = [arg(0), arg(1), arg(2), arg(3), arg(4), arg(5)];
                    self.text = m;
                    self.line = m;
                }
            }
            "T*" => {
                let l = self.st.leading;
                self.move_text(0.0, -l);
            }
            "Tj" => {
                if let Some(Obj::Str(s)) = args.first() {
                    self.show(s, res, depth);
                }
            }
            "'" => {
                let l = self.st.leading;
                self.move_text(0.0, -l);
                if let Some(Obj::Str(s)) = args.first() {
                    self.show(s, res, depth);
                }
            }
            "\"" => {
                if n.len() >= 2 {
                    self.st.word_space = arg(0);
                    self.st.char_space = arg(1);
                }
                let l = self.st.leading;
                self.move_text(0.0, -l);
                if let Some(Obj::Str(s)) = args.last() {
                    self.show(s, res, depth);
                }
            }
            "TJ" => {
                if let Some(items) = args.first().and_then(Obj::array) {
                    for it in items {
                        match it {
                            Obj::Str(s) => self.show(s, res, depth),
                            other => {
                                if let Some(v) = other.num() {
                                    let tx = -v / 1000.0 * self.st.font_size * self.st.scale;
                                    self.text[4] += tx * self.text[0];
                                    self.text[5] += tx * self.text[1];
                                }
                            }
                        }
                    }
                }
            }

            "Do" => {
                if let Some(Obj::Name(name)) = args.first() {
                    self.xobject(name, res, depth);
                }
            }
            "sh" => {
                if let Some(Obj::Name(name)) = args.first() {
                    self.shading(name, res);
                }
            }
            _ => {}
        }
    }

    /// The end point of the last segment, for `v`.
    fn current_point(&self) -> (f64, f64) {
        let (mut start, mut cur) = ((0.0, 0.0), (0.0, 0.0));
        for s in &self.path {
            match *s {
                PathSeg::Move(x, y) => {
                    start = (x, y);
                    cur = (x, y);
                }
                PathSeg::Line(x, y) => cur = (x, y),
                PathSeg::Cubic(_, _, _, _, x, y) => cur = (x, y),
                PathSeg::Close => cur = start,
            }
        }
        cur
    }

    fn close_path(&mut self) {
        if !self.path.is_empty() {
            self.path.push(PathSeg::Close);
        }
    }

    /// Ends the current path: fills it, strokes it or both, then applies a pending clip. The path is cleared.
    fn paint(&mut self, fill: Option<FillRule>, stroke: bool) {
        let path = std::mem::take(&mut self.path);
        let clip = self.pending_clip.take();
        if path.is_empty() {
            return;
        }
        let ctm = self.st.ctm;
        if let Some(rule) = fill {
            self.sink.fill(&path, &ctm, rule, self.st.fill, self.st.fill_alpha, self.st.blend);
        }
        if stroke {
            self.sink.stroke(&path, &ctm, &self.st.line, self.st.stroke, self.st.stroke_alpha, self.st.blend);
        }
        if let Some(rule) = clip {
            self.sink.clip(&path, &ctm, rule);
        }
    }

    fn set_color(&mut self, stroke: bool, cs: Rc<ColorSpace>, comps: &[f32]) {
        let rgb = cs.to_rgb(comps);
        if stroke {
            self.st.stroke_cs = cs;
            self.st.stroke = rgb;
        } else {
            self.st.fill_cs = cs;
            self.st.fill = rgb;
        }
    }

    /// `sc` and friends: components in the current space. Pattern spaces and empty operands keep the colour.
    fn set_components(&mut self, stroke: bool, comps: &[f32]) {
        let cs = if stroke { self.st.stroke_cs.clone() } else { self.st.fill_cs.clone() };
        if comps.is_empty() || cs.is_pattern() {
            return;
        }
        self.set_color(stroke, cs, comps);
    }

    fn move_text(&mut self, tx: f64, ty: f64) {
        self.line = page::mul(&[1.0, 0.0, 0.0, 1.0, tx, ty], &self.line);
        self.text = self.line;
    }

    /// Shows a string: each glyph is filled (or, for Type 3 fonts, its procedure runs) and the text matrix advances.
    fn show(&mut self, bytes: &[u8], res: &Dict, depth: u8) {
        let Some(font) = self.st.font.clone() else { return };
        let (fs, th, rise) = (self.st.font_size, self.st.scale, self.st.rise);
        let paints = matches!(self.st.mode, 0 | 2 | 4 | 6);
        for g in font.glyphs(bytes) {
            // glyph space (em units) to device: scale by the font size and horizontal scaling, rise, then Tm and CTM
            let trm = page::mul(&page::mul(&[fs * th, 0.0, 0.0, fs, 0.0, rise], &self.text), &self.st.ctm);
            if font.type3.is_some() {
                self.type3_glyph(&font, g.code, &trm, res, depth);
            } else if paints {
                if let Some(outline) = font.outline(&g, None) {
                    let (fill, alpha, blend) = (self.st.fill, self.st.fill_alpha, self.st.blend);
                    self.sink.fill(&outline.path, &trm, FillRule::NonZero, fill, alpha, blend);
                }
            }
            let mut w = g.width as f64 / 1000.0 * fs + self.st.char_space;
            if g.is_space {
                w += self.st.word_space;
            }
            self.text[4] += w * th * self.text[0];
            self.text[5] += w * th * self.text[1];
        }
    }

    /// Runs a Type 3 glyph procedure with the glyph space mapped through the font matrix to `trm`.
    fn type3_glyph(&mut self, font: &Font, code: u32, trm: &Matrix, res: &Dict, depth: u8) {
        if depth >= MAX_DEPTH {
            return;
        }
        let Some(t3) = font.type3.as_ref() else { return };
        let Some(name) = font.proc_name(code) else { return };
        let Some(proc_) = t3.procs.get(name) else { return };
        let data = self.doc.decode(proc_).0;
        let glyph_res = t3.resources.clone().unwrap_or_else(|| res.clone());
        let saved = (self.st.clone(), self.text, self.line);
        let base = self.stack.len();
        self.sink.save();
        self.st.ctm = page::mul(&t3.matrix, trm);
        self.run(&data, &glyph_res, depth + 1);
        while self.stack.len() > base {
            self.stack.pop();
            self.sink.restore();
        }
        self.sink.restore();
        self.st = saved.0;
        self.text = saved.1;
        self.line = saved.2;
    }

    fn xobject(&mut self, name: &str, res: &Dict, depth: u8) {
        let xobjs = self.doc.field(res, "XObject");
        let Some(entry) = xobjs.dict().and_then(|d| d.get(name)).cloned() else { return };
        let num = if let Obj::Ref(n, _) = &entry { Some(*n) } else { None };
        let Obj::Stream(s) = self.doc.get(&entry) else { return };
        match self.doc.field(&s.dict, "Subtype").name() {
            Some("Image") => self.image_xobject(&s, num, res),
            Some("Form") if depth < MAX_DEPTH => self.form(&s, res, depth),
            _ => {}
        }
    }

    fn image_xobject(&mut self, s: &Rc<Stream>, num: Option<u32>, res: &Dict) {
        let cached = num.and_then(|n| self.images.get(&n).cloned());
        let img = match cached {
            Some(img) => img,
            None => {
                let Some(img) = image::decode(self.doc, s, Some(res)) else { return };
                let img = Rc::new(img);
                if let Some(n) = num {
                    self.images.insert(n, img.clone());
                }
                img
            }
        };
        self.draw_image(&img);
    }

    fn draw_image(&mut self, img: &PdfImage) {
        let ctm = self.st.ctm;
        self.sink.image(img, &ctm, self.st.fill, self.st.fill_alpha, self.st.blend, false);
    }

    /// A form XObject: its own matrix and bounding box, then its content with its own resources.
    fn form(&mut self, s: &Rc<Stream>, res: &Dict, depth: u8) {
        let own = self.doc.field(&s.dict, "Resources");
        let form_res: Dict = own.dict().cloned().unwrap_or_else(|| res.clone());
        let saved = self.st.clone();
        let base = self.stack.len();
        self.sink.save();
        let mat = self.doc.field(&s.dict, "Matrix");
        if let Some(m) = matrix_of(&mat) {
            self.st.ctm = page::mul(&m, &self.st.ctm);
        }
        let bbox = self.doc.field(&s.dict, "BBox");
        if let Some([x0, y0, x1, y1]) = rect_of(&bbox) {
            let path = [PathSeg::Move(x0, y0), PathSeg::Line(x1, y0), PathSeg::Line(x1, y1), PathSeg::Line(x0, y1), PathSeg::Close];
            let ctm = self.st.ctm;
            self.sink.clip(&path, &ctm, FillRule::NonZero);
        }
        let data = self.doc.decode(s).0;
        self.run(&data, &form_res, depth + 1);
        while self.stack.len() > base {
            self.stack.pop();
            self.sink.restore();
        }
        self.sink.restore();
        self.st = saved;
    }

    fn shading(&mut self, name: &str, res: &Dict) {
        let table = self.doc.field(res, "Shading");
        let Some(entry) = table.dict().and_then(|d| d.get(name)).cloned() else { return };
        if let Some(sh) = shading::parse(self.doc, &entry, Some(res)) {
            let ctm = self.st.ctm;
            self.sink.shade(&sh, &ctm, self.st.fill_alpha, self.st.blend);
        }
    }

    /// `gs`: the line width, cap, join, miter limit, dash, the two alphas and the blend mode.
    fn ext_gstate(&mut self, res: &Dict, name: &str) {
        let table = self.doc.field(res, "ExtGState");
        let Some(entry) = table.dict().and_then(|d| d.get(name)).cloned() else { return };
        let gs = self.doc.get(&entry);
        let Some(d) = gs.dict() else { return };
        let doc = self.doc;
        let get = |k: &str| doc.field(d, k);
        if let Some(v) = get("LW").num() {
            self.st.line.width = v;
        }
        if let Some(v) = get("LC").int() {
            self.st.line.cap = match v {
                1 => LineCap::Round,
                2 => LineCap::Square,
                _ => LineCap::Butt,
            };
        }
        if let Some(v) = get("LJ").int() {
            self.st.line.join = match v {
                1 => LineJoin::Round,
                2 => LineJoin::Bevel,
                _ => LineJoin::Miter,
            };
        }
        if let Some(v) = get("ML").num() {
            self.st.line.miter_limit = v;
        }
        let dash = get("D");
        if let Some(parts) = dash.array() {
            if let Some(list) = parts.first().and_then(Obj::array) {
                self.st.line.dash = nums(list);
                self.st.line.phase = parts.get(1).and_then(Obj::num).unwrap_or(0.0);
            }
        }
        if let Some(v) = get("CA").num() {
            self.st.stroke_alpha = v.clamp(0.0, 1.0) as f32;
        }
        if let Some(v) = get("ca").num() {
            self.st.fill_alpha = v.clamp(0.0, 1.0) as f32;
        }
        let bm = get("BM");
        let mode = bm.name().or_else(|| bm.array().and_then(|a| a.first()).and_then(Obj::name));
        if let Some(m) = mode {
            self.st.blend = Blend::from_name(m);
        }
    }

    /// An inline image (`BI` ... `ID` data `EI`). Its abbreviated keys and names are expanded, then it decodes like
    /// an image XObject.
    fn inline_image(&mut self, p: &mut Parser, res: &Dict) {
        let mut dict = Dict::new();
        let mut key: Option<String> = None;
        loop {
            let Some(o) = p.parse(false) else { return };
            if o.is_op("ID") {
                break;
            }
            match key.take() {
                None => {
                    if let Obj::Name(n) = o {
                        key = Some(n);
                    }
                }
                Some(k) => {
                    let full = expand_key(&k);
                    let value = expand_value(&full, o);
                    dict.insert(full, value);
                }
            }
        }
        let d = p.d;
        // the data starts after the single white-space byte that follows ID
        let start = (p.pos + 1).min(d.len());
        let mut found = None;
        let mut i = start;
        while i + 2 <= d.len() {
            if &d[i..i + 2] == b"EI" && (i == start || is_ws(d[i - 1])) && d.get(i + 2).map_or(true, |&c| is_ws(c)) {
                found = Some(i);
                break;
            }
            i += 1;
        }
        let stop = found.unwrap_or(d.len());
        let end = if stop > start && is_ws(d[stop - 1]) { stop - 1 } else { stop };
        let stream = Stream { dict, data: d[start..end.max(start)].to_vec() };
        p.pos = (stop + 2).min(d.len());
        let Some(img) = image::decode(self.doc, &stream, Some(res)) else { return };
        self.draw_image(&img);
    }
}

fn nums(args: &[Obj]) -> Vec<f64> {
    args.iter().filter_map(Obj::num).collect()
}

fn f32s(v: &[f64]) -> Vec<f32> {
    v.iter().map(|x| *x as f32).collect()
}

/// Six numbers as a matrix.
fn matrix_of(o: &Obj) -> Option<Matrix> {
    let v = nums(o.array()?);
    if v.len() < 6 {
        return None;
    }
    Some([v[0], v[1], v[2], v[3], v[4], v[5]])
}

/// Four numbers as `[x0, y0, x1, y1]`.
fn rect_of(o: &Obj) -> Option<[f64; 4]> {
    let v = nums(o.array()?);
    if v.len() < 4 {
        return None;
    }
    Some([v[0], v[1], v[2], v[3]])
}

/// The full name of an abbreviated inline-image key.
fn expand_key(k: &str) -> String {
    let full: &str = match k {
        "BPC" => "BitsPerComponent",
        "CS" => "ColorSpace",
        "D" => "Decode",
        "DP" => "DecodeParms",
        "F" => "Filter",
        "H" => "Height",
        "IM" => "ImageMask",
        "I" => "Interpolate",
        "W" => "Width",
        other => other,
    };
    full.to_string()
}

/// Expands the abbreviated colour space and filter names an inline image may use.
fn expand_value(full_key: &str, v: Obj) -> Obj {
    match (full_key, v) {
        ("ColorSpace", Obj::Name(n)) => Obj::Name(colour_space_name(&n)),
        ("Filter", Obj::Name(n)) => Obj::Name(filter_name(&n)),
        ("Filter", Obj::Array(items)) => Obj::Array(
            items
                .into_iter()
                .map(|o| match o {
                    Obj::Name(n) => Obj::Name(filter_name(&n)),
                    other => other,
                })
                .collect(),
        ),
        (_, other) => other,
    }
}

fn colour_space_name(n: &str) -> String {
    let full: &str = match n {
        "G" => "DeviceGray",
        "RGB" => "DeviceRGB",
        "CMYK" => "DeviceCMYK",
        "I" => "Indexed",
        other => other,
    };
    full.to_string()
}

fn filter_name(n: &str) -> String {
    let full: &str = match n {
        "AHx" => "ASCIIHexDecode",
        "A85" => "ASCII85Decode",
        "LZW" => "LZWDecode",
        "Fl" => "FlateDecode",
        "RL" => "RunLengthDecode",
        "CCF" => "CCITTFaxDecode",
        "DCT" => "DCTDecode",
        other => other,
    };
    full.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg::pdf::doc::tests::tiny_pdf;
    use crate::cg::pdf::page::pages;
    use crate::cg::pdf::sink::{Call, Recorder};

    fn record(content: &str) -> Vec<Call> {
        let doc = Doc::open(tiny_pdf(content)).unwrap();
        let list = pages(&doc);
        let mut rec = Recorder::default();
        draw_page(&doc, &list[0], page::IDENTITY, &mut rec);
        rec.calls
    }

    #[test]
    fn a_filled_rectangle_reports_its_colour_and_path() {
        let calls = record("0 0 1 rg 10 10 100 50 re f");
        match calls.as_slice() {
            [Call::Fill(path, m, rule, color, alpha)] => {
                assert_eq!(path.len(), 5);
                assert_eq!(*m, page::IDENTITY);
                assert_eq!(*rule, FillRule::NonZero);
                assert_eq!(*color, [0.0, 0.0, 1.0]);
                assert_eq!(*alpha, 1.0);
            }
            other => panic!("unexpected calls {other:?}"),
        }
    }

    #[test]
    fn fills_keep_their_order_and_colours() {
        let calls = record("0 1 0 rg 0 0 5 5 re f 1 0 0 rg 6 6 5 5 re f*");
        let fills: Vec<(Rgb, FillRule)> = calls
            .iter()
            .filter_map(|c| match c {
                Call::Fill(_, _, rule, color, _) => Some((*color, *rule)),
                _ => None,
            })
            .collect();
        assert_eq!(fills, vec![([0.0, 1.0, 0.0], FillRule::NonZero), ([1.0, 0.0, 0.0], FillRule::EvenOdd)]);
    }

    #[test]
    fn clips_use_the_current_matrix_and_nest_with_save_and_restore() {
        let calls = record("q 2 0 0 2 0 0 cm 0 0 1 1 re W n Q");
        match calls.as_slice() {
            [Call::Save, Call::Clip(_, m, FillRule::NonZero), Call::Restore] => assert_eq!(*m, [2.0, 0.0, 0.0, 2.0, 0.0, 0.0]),
            other => panic!("unexpected calls {other:?}"),
        }
    }

    #[test]
    fn stroke_state_reaches_the_sink() {
        let calls = record("[3 2] 1 d 1 J 2 j 2 w 1 0 0 RG 0 0 m 10 10 l S");
        match calls.as_slice() {
            [Call::Stroke(path, _, s, color, _)] => {
                assert_eq!(path.len(), 2);
                assert_eq!(s.width, 2.0);
                assert_eq!(s.dash, vec![3.0, 2.0]);
                assert_eq!(s.phase, 1.0);
                assert_eq!(s.cap, LineCap::Round);
                assert_eq!(s.join, LineJoin::Round);
                assert_eq!(*color, [1.0, 0.0, 0.0]);
            }
            other => panic!("unexpected calls {other:?}"),
        }
    }

    #[test]
    fn unbalanced_saves_are_closed_at_the_end_of_the_page() {
        assert_eq!(record("q 0 0 1 rg"), vec![Call::Save, Call::Restore]);
    }

    #[test]
    fn text_without_an_embedded_program_draws_no_glyph_paths_and_does_not_panic() {
        let calls = record("BT /F1 12 Tf 72 700 Td (Hello) Tj ET");
        assert!(calls.iter().all(|c| matches!(c, Call::Fill(..))));
    }

    #[test]
    fn inline_key_and_name_abbreviations_expand() {
        assert_eq!(expand_key("W"), "Width");
        assert_eq!(expand_key("BPC"), "BitsPerComponent");
        assert_eq!(expand_value("Filter", Obj::Name("AHx".into())), Obj::Name("ASCIIHexDecode".into()));
        assert_eq!(expand_value("ColorSpace", Obj::Name("RGB".into())), Obj::Name("DeviceRGB".into()));
        assert_eq!(expand_value("Width", Obj::Int(4)), Obj::Int(4));
    }
}
