//! Document level reading: the page tree with inherited attributes, page
//! boxes and rotation, document information, destinations and the outline.

use super::doc::Doc;
use super::obj::{Dict, Obj};
use std::collections::HashSet;

/// Pages read before giving up, so a huge file cannot stall the reader forever.
pub const MAX_PAGES: usize = 20_000;

/// `[x0, y0, x1, y1]`, normalised so that `x0 <= x1` and `y0 <= y1`.
pub type Rect = [f64; 4];

/// `[a b c d e f]`: x' = a x + c y + e, y' = b x + d y + f.
pub type Matrix = [f64; 6];

pub const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `a` applied first, then `b` (the PDF `cm` order: new = a x b).
pub fn mul(a: &Matrix, b: &Matrix) -> Matrix {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

pub fn apply(m: &Matrix, x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

pub fn invert(m: &Matrix) -> Option<Matrix> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det.abs() < 1e-12 || !det.is_finite() {
        return None;
    }
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    Some([a, b, c, d, -(m[4] * a + m[5] * c), -(m[4] * b + m[5] * d)])
}

fn rect_of(doc: &Doc, o: &Obj) -> Option<Rect> {
    let a = match doc.get(o) {
        Obj::Array(a) => a,
        _ => return None,
    };
    let v: Vec<f64> = a.iter().filter_map(|x| doc.get(x).num()).collect();
    if v.len() < 4 {
        return None;
    }
    let r = [v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])];
    (r[2] - r[0] > 0.0 && r[3] - r[1] > 0.0).then_some(r)
}

/// One leaf of the page tree with everything it inherits resolved.
#[derive(Clone, Debug)]
pub struct Page {
    /// zero based position in reading order
    pub index: usize,
    /// the page object's number, when it is an indirect object
    pub num: Option<u32>,
    pub dict: Dict,
    pub resources: Dict,
    pub media: Rect,
    crop: Option<Rect>,
    bleed: Option<Rect>,
    trim: Option<Rect>,
    art: Option<Rect>,
    rotate: i64,
    pub user_unit: f64,
}

/// The five page boxes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageBox {
    Media,
    Crop,
    Bleed,
    Trim,
    Art,
}

impl Page {
    pub fn media_box(&self) -> Rect {
        self.media
    }

    /// The crop box, clipped to the media box; the media box when absent.
    pub fn crop_box(&self) -> Rect {
        match self.crop {
            Some(c) => {
                let r = [c[0].max(self.media[0]), c[1].max(self.media[1]), c[2].min(self.media[2]), c[3].min(self.media[3])];
                if r[2] > r[0] && r[3] > r[1] {
                    r
                } else {
                    self.media
                }
            }
            None => self.media,
        }
    }

    pub fn bleed_box(&self) -> Rect {
        self.bleed.unwrap_or_else(|| self.crop_box())
    }

    pub fn trim_box(&self) -> Rect {
        self.trim.unwrap_or_else(|| self.crop_box())
    }

    pub fn art_box(&self) -> Rect {
        self.art.unwrap_or_else(|| self.crop_box())
    }

    pub fn rect(&self, which: PageBox) -> Rect {
        match which {
            PageBox::Media => self.media_box(),
            PageBox::Crop => self.crop_box(),
            PageBox::Bleed => self.bleed_box(),
            PageBox::Trim => self.trim_box(),
            PageBox::Art => self.art_box(),
        }
    }

    /// Clockwise rotation to apply when showing the page: 0, 90, 180 or 270.
    pub fn rotation(&self) -> i32 {
        (((self.rotate % 360) + 360) % 360 / 90 * 90) as i32
    }

    /// The size in points of `which` after rotation and user unit.
    pub fn size(&self, which: PageBox) -> (f64, f64) {
        let r = self.rect(which);
        let (w, h) = ((r[2] - r[0]) * self.user_unit, (r[3] - r[1]) * self.user_unit);
        if self.rotation() % 180 == 0 {
            (w, h)
        } else {
            (h, w)
        }
    }

    /// Decoded content streams, joined with newlines.
    pub fn contents(&self, doc: &Doc) -> Vec<u8> {
        let mut out = vec![];
        let list: Vec<Obj> = match doc.field(&self.dict, "Contents") {
            Obj::Array(a) => a,
            other => vec![other],
        };
        for o in list {
            if let Obj::Stream(s) = doc.get(&o) {
                out.extend_from_slice(&doc.decode(&s).0);
                out.push(b'\n');
            }
        }
        out
    }

    /// The matrix that maps the page's default space (points, y up) so that `which` fills `dest`
    /// (`x, y, w, h` with y down). With `keep_aspect` the page is scaled to fit and centred.
    pub fn transform(&self, which: PageBox, dest: (f64, f64, f64, f64), keep_aspect: bool) -> Matrix {
        page_matrix(self.rect(which), self.rotation(), dest, keep_aspect)
    }
}

/// The matrix that shows the box `b`, rotated clockwise by `rotation` degrees, inside `dest` (x, y, w, h; y down).
pub fn page_matrix(b: Rect, rotation: i32, dest: (f64, f64, f64, f64), keep_aspect: bool) -> Matrix {
    let (bw, bh) = (b[2] - b[0], b[3] - b[1]);
    let rotated = rotation.rem_euclid(180) != 0;
    let (w, h) = if rotated { (bh, bw) } else { (bw, bh) };
    let (mut sx, mut sy) = (dest.2 / w, dest.3 / h);
    let (mut dx, mut dy) = (dest.0, dest.1);
    if keep_aspect {
        let s = sx.min(sy);
        dx += (dest.2 - w * s) / 2.0;
        dy += (dest.3 - h * s) / 2.0;
        sx = s;
        sy = s;
    }
    // work in "unrotated, y-down, scaled" terms by composing small matrices
    let flip_scale: Matrix = [1.0, 0.0, 0.0, -1.0, -b[0], b[3]];
    // quarter turns clockwise about the origin of the y-down space, then back into the positive quadrant
    let turn: Matrix = match rotation.rem_euclid(360) {
        90 => [0.0, 1.0, -1.0, 0.0, bh, 0.0],
        180 => [-1.0, 0.0, 0.0, -1.0, bw, bh],
        270 => [0.0, -1.0, 1.0, 0.0, 0.0, bw],
        _ => IDENTITY,
    };
    let place: Matrix = [sx, 0.0, 0.0, sy, dx, dy];
    mul(&mul(&flip_scale, &turn), &place)
}

struct Inherit {
    resources: Dict,
    media: Option<Rect>,
    crop: Option<Rect>,
    rotate: Option<i64>,
}

fn walk(doc: &Doc, node: &Obj, inh: &Inherit, out: &mut Vec<Page>, seen: &mut HashSet<u32>, depth: u8) {
    if depth > 32 || out.len() >= MAX_PAGES {
        return;
    }
    let num = if let Obj::Ref(n, _) = node { Some(*n) } else { None };
    if let Some(n) = num {
        if !seen.insert(n) {
            return;
        }
    }
    let Obj::Dict(d) = doc.get(node) else { return };
    let resources = match doc.field(&d, "Resources") {
        Obj::Dict(r) => r,
        _ => inh.resources.clone(),
    };
    let media = d.get("MediaBox").and_then(|o| rect_of(doc, o)).or(inh.media);
    let crop = d.get("CropBox").and_then(|o| rect_of(doc, o)).or(inh.crop);
    let rotate = doc.field(&d, "Rotate").int().or(inh.rotate);
    match doc.field(&d, "Kids") {
        Obj::Array(kids) => {
            let next = Inherit { resources, media, crop, rotate };
            for k in &kids {
                walk(doc, k, &next, out, seen, depth + 1);
            }
        }
        _ => {
            let get = |k: &str| d.get(k).and_then(|o| rect_of(doc, o));
            let page = Page {
                index: out.len(),
                num,
                resources,
                media: media.unwrap_or([0.0, 0.0, 612.0, 792.0]),
                crop,
                bleed: get("BleedBox"),
                trim: get("TrimBox"),
                art: get("ArtBox"),
                rotate: rotate.unwrap_or(0),
                user_unit: doc.field(&d, "UserUnit").num().filter(|u| *u > 0.0).unwrap_or(1.0),
                dict: d,
            };
            out.push(page);
        }
    }
}

/// Every page in order.
pub fn pages(doc: &Doc) -> Vec<Page> {
    let mut out = vec![];
    if let Some(cat) = doc.catalog() {
        if let Some(root) = cat.get("Pages") {
            let inh = Inherit { resources: Dict::new(), media: None, crop: None, rotate: None };
            walk(doc, root, &inh, &mut out, &mut HashSet::new(), 0);
        }
    }
    out
}

/// The number of pages (the tree is walked, not trusted to `/Count`).
pub fn page_count(doc: &Doc) -> usize {
    pages(doc).len()
}

/// A PDF text string (UTF-16BE or UTF-8 with BOM, else PDFDocEncoding ~ Latin-1).
pub fn text_string(b: &[u8]) -> String {
    if b.starts_with(&[0xfe, 0xff]) {
        let units: Vec<u16> = b[2..].chunks(2).filter(|c| c.len() == 2).map(|c| (c[0] as u16) << 8 | c[1] as u16).collect();
        String::from_utf16_lossy(&units)
    } else if b.starts_with(&[0xef, 0xbb, 0xbf]) {
        String::from_utf8_lossy(&b[3..]).into_owned()
    } else {
        b.iter().map(|&c| c as char).collect()
    }
}

/// The document information dictionary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocInfo {
    pub title: String,
    pub author: String,
    pub subject: String,
    pub keywords: String,
    pub creator: String,
    pub producer: String,
    /// raw `D:YYYYMMDDHHmmSS...` strings
    pub creation_date: String,
    pub mod_date: String,
}

pub fn info(doc: &Doc) -> DocInfo {
    let Some(Obj::Dict(d)) = doc.trailer.get("Info").map(|o| doc.get(o)) else { return DocInfo::default() };
    let get = |k: &str| match doc.field(&d, k) {
        Obj::Str(s) => text_string(&s).trim().to_string(),
        _ => String::new(),
    };
    DocInfo {
        title: get("Title"),
        author: get("Author"),
        subject: get("Subject"),
        keywords: get("Keywords"),
        creator: get("Creator"),
        producer: get("Producer"),
        creation_date: get("CreationDate"),
        mod_date: get("ModDate"),
    }
}

/// Where a destination lands.
#[derive(Clone, Debug, PartialEq)]
pub struct Dest {
    /// the page object number
    pub page: u32,
    pub left: Option<f64>,
    pub top: Option<f64>,
    pub zoom: Option<f64>,
}

/// Resolves a destination (array, dictionary with `/D`, or a name looked up in the catalog).
pub fn resolve_dest(doc: &Doc, dest: &Obj, depth: u8) -> Option<Dest> {
    let cat = doc.catalog()?;
    match doc.get(dest) {
        Obj::Array(a) => {
            let Obj::Ref(n, _) = a.first()? else { return None };
            let num = |i: usize| a.get(i).and_then(|o| doc.get(o).num());
            let kind = a.get(1).map(|o| doc.get(o)).and_then(|o| o.name().map(str::to_string));
            let (left, top, zoom) = match kind.as_deref() {
                Some("XYZ") => (num(2), num(3), num(4)),
                Some("FitH") | Some("FitBH") => (None, num(2), None),
                Some("FitV") | Some("FitBV") => (num(2), None, None),
                _ => (None, None, None),
            };
            Some(Dest { page: *n, left, top, zoom })
        }
        Obj::Dict(d) => resolve_dest(doc, d.get("D")?, depth + 1),
        Obj::Name(n) if depth < 3 => named_dest(doc, &cat, n.as_bytes()).and_then(|o| resolve_dest(doc, &o, depth + 1)),
        Obj::Str(s) if depth < 3 => named_dest(doc, &cat, &s).and_then(|o| resolve_dest(doc, &o, depth + 1)),
        _ => None,
    }
}

fn named_dest(doc: &Doc, cat: &Dict, name: &[u8]) -> Option<Obj> {
    if let Obj::Dict(d) = doc.field(cat, "Dests") {
        if let Some(o) = d.get(&String::from_utf8_lossy(name).into_owned()) {
            return Some(o.clone());
        }
    }
    let names = doc.field(cat, "Names");
    let tree = doc.field(names.dict()?, "Dests");
    name_tree(doc, &tree, name, 0)
}

fn name_tree(doc: &Doc, node: &Obj, name: &[u8], depth: u8) -> Option<Obj> {
    if depth > 8 {
        return None;
    }
    let d = node.dict()?;
    if let Obj::Array(pairs) = doc.field(d, "Names") {
        for kv in pairs.chunks(2) {
            if let (Obj::Str(k), Some(v)) = (doc.get(&kv[0]), kv.get(1)) {
                if k == name {
                    return Some(v.clone());
                }
            }
        }
    }
    if let Obj::Array(kids) = doc.field(d, "Kids") {
        for k in &kids {
            if let Some(o) = name_tree(doc, &doc.get(k), name, depth + 1) {
                return Some(o);
            }
        }
    }
    None
}

/// One outline (bookmark) entry.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineItem {
    pub title: String,
    pub dest: Dest,
    pub level: u8,
}

/// The outline tree flattened in reading order, entries without a usable destination skipped.
pub fn outline(doc: &Doc) -> Vec<OutlineItem> {
    let mut out = vec![];
    let Some(cat) = doc.catalog() else { return out };
    let Obj::Dict(root) = doc.field(&cat, "Outlines") else { return out };
    fn level(doc: &Doc, first: &Obj, lvl: u8, out: &mut Vec<OutlineItem>, seen: &mut HashSet<u32>) {
        let mut cur = first.clone();
        while out.len() < 5000 {
            if let Obj::Ref(n, _) = &cur {
                if !seen.insert(*n) {
                    return;
                }
            }
            let Obj::Dict(item) = doc.get(&cur) else { return };
            let title = match doc.field(&item, "Title") {
                Obj::Str(s) => text_string(&s).trim().to_string(),
                _ => String::new(),
            };
            let dest = match item.get("Dest") {
                Some(d) => Some(d.clone()),
                None => doc.field(&item, "A").dict().and_then(|a| a.get("D").cloned()),
            };
            if let (false, Some(dest)) = (title.is_empty(), dest.and_then(|d| resolve_dest(doc, &d, 0))) {
                out.push(OutlineItem { title, dest, level: lvl });
            }
            if let Some(child) = item.get("First") {
                if lvl < 8 {
                    level(doc, child, lvl + 1, out, seen);
                }
            }
            match item.get("Next") {
                Some(n) => cur = n.clone(),
                None => return,
            }
        }
    }
    if let Some(first) = root.get("First") {
        level(doc, first, 0, &mut out, &mut HashSet::new());
    }
    out
}

/// A link annotation on a page.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub rect: Rect,
    pub uri: Option<String>,
    pub dest: Option<Dest>,
}

/// The link annotations of a page.
pub fn links(doc: &Doc, page: &Page) -> Vec<Link> {
    let mut out = vec![];
    let Obj::Array(annots) = doc.field(&page.dict, "Annots") else { return out };
    for a in &annots {
        let Obj::Dict(d) = doc.get(a) else { continue };
        if doc.field(&d, "Subtype").name() != Some("Link") {
            continue;
        }
        let Some(rect) = d.get("Rect").and_then(|o| rect_of(doc, o)) else { continue };
        let action = doc.field(&d, "A");
        let uri = action.dict().and_then(|a| match doc.field(a, "URI") {
            Obj::Str(s) => Some(text_string(&s)),
            _ => None,
        });
        let dest = match d.get("Dest") {
            Some(x) => resolve_dest(doc, x, 0),
            None => action.dict().and_then(|a| a.get("D")).and_then(|x| resolve_dest(doc, x, 0)),
        };
        out.push(Link { rect, uri, dest });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg::pdf::doc::tests::tiny_pdf;

    #[test]
    fn walks_pages_and_reads_boxes() {
        let doc = Doc::open(tiny_pdf("")).unwrap();
        let p = pages(&doc);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].media_box(), [0.0, 0.0, 612.0, 792.0]);
        assert_eq!(p[0].crop_box(), p[0].media_box());
        assert_eq!(p[0].trim_box(), p[0].crop_box());
        assert_eq!(p[0].rotation(), 0);
        assert_eq!(p[0].size(PageBox::Media), (612.0, 792.0));
        assert!(p[0].resources.contains_key("Font"));
    }

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
    }

    #[test]
    fn page_matrix_places_the_corners() {
        let b = [10.0, 20.0, 110.0, 220.0];
        let m = page_matrix(b, 0, (0.0, 0.0, 100.0, 200.0), false);
        assert!(close(apply(&m, 10.0, 220.0), (0.0, 0.0)), "top left");
        assert!(close(apply(&m, 110.0, 20.0), (100.0, 200.0)), "bottom right");
        // 90 degrees clockwise: the top left of the page goes to the top right
        let m = page_matrix(b, 90, (0.0, 0.0, 200.0, 100.0), false);
        assert!(close(apply(&m, 10.0, 220.0), (200.0, 0.0)));
        assert!(close(apply(&m, 10.0, 20.0), (0.0, 0.0)), "bottom left goes to the top left");
        let m = page_matrix(b, 180, (0.0, 0.0, 100.0, 200.0), false);
        assert!(close(apply(&m, 10.0, 220.0), (100.0, 200.0)));
        let m = page_matrix(b, 270, (0.0, 0.0, 200.0, 100.0), false);
        assert!(close(apply(&m, 10.0, 220.0), (0.0, 100.0)));
        assert!(close(apply(&m, 110.0, 220.0), (0.0, 0.0)));
    }

    #[test]
    fn aspect_fit_centres_the_page() {
        let m = page_matrix([0.0, 0.0, 100.0, 100.0], 0, (0.0, 0.0, 300.0, 100.0), true);
        assert!(close(apply(&m, 0.0, 100.0), (100.0, 0.0)));
        assert!(close(apply(&m, 100.0, 0.0), (200.0, 100.0)));
    }

    #[test]
    fn matrix_helpers() {
        let m = [2.0, 0.0, 0.0, 3.0, 5.0, 7.0];
        let inv = invert(&m).unwrap();
        let p = apply(&m, 1.0, 1.0);
        assert!(close(apply(&inv, p.0, p.1), (1.0, 1.0)));
        assert!(invert(&[0.0; 6]).is_none());
    }

    #[test]
    fn text_strings() {
        assert_eq!(text_string(&[0xfe, 0xff, 0, 0x48, 0, 0x69]), "Hi");
        assert_eq!(text_string(b"Plain"), "Plain");
    }
}
