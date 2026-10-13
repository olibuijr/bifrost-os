//! Incremental updates. A change is written by appending the changed and new objects, a cross-reference section and a
//! trailer whose /Prev points at the old cross-reference section, so the bytes already in the file never move. The
//! edits are: the page order for a flat page tree (pages kept, reordered or deleted), the rotation of pages, and
//! annotations with their appearance streams. A save is read back before it is returned: every written object must
//! load again as the same bytes, or the save fails and nothing is written.

use super::doc::Doc;
use super::obj::{is_ws, Dict, Obj, Stream};
use super::page::{self, Page, PageBox, Rect};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

/// An annotation to add to a page.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Annotation {
    /// The page's index in `page::pages` order.
    pub page: usize,
    /// The /Subtype, for example "Highlight", "Square", "Text" or "Ink".
    pub subtype: String,
    /// The rectangle in default user space.
    pub rect: Rect,
    /// Further entries of the annotation dictionary: /C, /QuadPoints, /Contents, /BS, /InkList and so on.
    pub entries: Dict,
    /// The appearance: a content stream in default user space, clipped to `rect`. Empty for no appearance.
    pub appearance: Vec<u8>,
    /// The resources the appearance uses, such as an /ExtGState for opacity.
    pub resources: Dict,
}

/// The changes to make to a file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Edits {
    /// The pages to keep, in their new order, as indices in `page::pages` order. `None` keeps the page tree.
    pub order: Option<Vec<usize>>,
    /// Pages whose /Rotate becomes this many degrees (a multiple of 90), by page index. The degrees are absolute.
    pub rotate: Vec<(usize, i64)>,
    /// Annotations to add.
    pub annotations: Vec<Annotation>,
}

/// Maps a point of a page's displayed picture to default user space. The point is given as fractions of the picture
/// (0 to 1 across, y down), where the picture is the page as `page::render_page` draws it, with the page's own
/// rotation. `None` when the page has no usable box.
pub fn user_point(page: &Page, u: f64, v: f64) -> Option<(f64, f64)> {
    let m = page.transform(PageBox::Crop, (0.0, 0.0, 1.0, 1.0), false);
    let inv = page::invert(&m)?;
    Some(page::apply(&inv, u, v))
}

/// The default user space rectangle that covers two such points.
pub fn user_rect(page: &Page, u0: f64, v0: f64, u1: f64, v1: f64) -> Option<Rect> {
    let (ax, ay) = user_point(page, u0, v0)?;
    let (bx, by) = user_point(page, u1, v1)?;
    Some([ax.min(bx), ay.min(by), ax.max(bx), ay.max(by)])
}

/// The file `base` with `edits` appended. `base` must be the bytes `doc` was opened from. Returns the whole new file.
/// An encrypted file is refused: its new objects would need the file's keys.
pub fn save(base: &[u8], doc: &Doc, edits: &Edits) -> Result<Vec<u8>, String> {
    if doc.is_encrypted() {
        return Err("an encrypted PDF cannot be changed in place".into());
    }
    if edits.order.is_none() && edits.rotate.is_empty() && edits.annotations.is_empty() {
        return Ok(base.to_vec());
    }
    let pages = page::pages(doc);
    let prev = last_startxref(base).ok_or("the file has no cross-reference table")?;
    let size_hint = doc.trailer.get("Size").and_then(|o| o.int()).unwrap_or(0).max(0) as u32;
    let mut next: u32 = doc.object_numbers().last().map_or(1, |n| *n + 1).max(size_hint);
    let num_of = |i: usize| -> Result<u32, String> {
        pages.get(i).and_then(|p| p.num).ok_or_else(|| format!("page {} is not an object of its own", i + 1))
    };

    let mut objects: BTreeMap<u32, Obj> = BTreeMap::new();
    let mut page_dicts: BTreeMap<usize, Dict> = BTreeMap::new();
    let mut expected_pages = pages.len();

    if let Some(order) = &edits.order {
        let mut seen = BTreeSet::new();
        if order.is_empty() {
            return Err("a PDF needs at least one page".into());
        }
        if order.iter().any(|&i| i >= pages.len() || !seen.insert(i)) {
            return Err("the page order names a page twice or names no page".into());
        }
        expected_pages = order.len();
        let identity = order.len() == pages.len() && order.iter().enumerate().all(|(i, &p)| i == p);
        if !identity {
            let cat = doc.catalog().ok_or("the file has no catalog")?;
            let pages_num = match cat.get("Pages") {
                Some(Obj::Ref(n, _)) => *n,
                _ => return Err("the page tree root is not an object of its own".into()),
            };
            let Obj::Dict(mut root) = doc.load(pages_num) else {
                return Err("the page tree root is damaged".into());
            };
            let flat = match doc.field(&root, "Kids") {
                Obj::Array(kids) => kids.iter().all(|k| matches!(doc.get(k), Obj::Dict(d) if !d.contains_key("Kids"))),
                _ => false,
            };
            if !flat {
                return Err("this PDF's page tree is nested, so its pages cannot be reordered or deleted".into());
            }
            let kids = order.iter().map(|&i| num_of(i).map(|n| Obj::Ref(n, 0))).collect::<Result<Vec<Obj>, String>>()?;
            root.insert("Kids".into(), Obj::Array(kids));
            root.insert("Count".into(), Obj::Int(order.len() as i64));
            objects.insert(pages_num, Obj::Dict(root));
        }
    }

    for &(i, degrees) in &edits.rotate {
        let p = pages.get(i).ok_or("a rotated page is missing")?;
        num_of(i)?;
        if degrees.rem_euclid(90) != 0 {
            return Err("a page can only be turned by a multiple of 90 degrees".into());
        }
        let dict = page_dicts.entry(i).or_insert_with(|| p.dict.clone());
        dict.insert("Rotate".into(), Obj::Int(degrees.rem_euclid(360)));
    }

    for ann in &edits.annotations {
        let p = pages.get(ann.page).ok_or("an annotation names a missing page")?;
        let page_num = num_of(ann.page)?;
        let mut annot = ann.entries.clone();
        if !ann.appearance.is_empty() {
            let form_num = next;
            next += 1;
            let mut form = Dict::new();
            form.insert("Type".into(), Obj::Name("XObject".into()));
            form.insert("Subtype".into(), Obj::Name("Form".into()));
            form.insert("BBox".into(), rect_obj(ann.rect));
            form.insert("Resources".into(), Obj::Dict(ann.resources.clone()));
            objects.insert(form_num, Obj::Stream(Rc::new(Stream { dict: form, data: ann.appearance.clone() })));
            let mut appearance = Dict::new();
            appearance.insert("N".into(), Obj::Ref(form_num, 0));
            annot.insert("AP".into(), Obj::Dict(appearance));
        }
        annot.insert("Type".into(), Obj::Name("Annot".into()));
        annot.insert("Subtype".into(), Obj::Name(ann.subtype.clone()));
        annot.insert("Rect".into(), rect_obj(ann.rect));
        annot.insert("F".into(), Obj::Int(4));
        annot.insert("P".into(), Obj::Ref(page_num, 0));
        let annot_num = next;
        next += 1;
        objects.insert(annot_num, Obj::Dict(annot));
        let pd = page_dicts.entry(ann.page).or_insert_with(|| p.dict.clone());
        let mut list = match doc.field(pd, "Annots") {
            Obj::Array(a) => a,
            _ => Vec::new(),
        };
        list.push(Obj::Ref(annot_num, 0));
        pd.insert("Annots".into(), Obj::Array(list));
    }

    for (i, dict) in page_dicts {
        let num = num_of(i)?;
        objects.insert(num, Obj::Dict(dict));
    }
    if objects.is_empty() {
        return Ok(base.to_vec());
    }

    let mut body = Vec::new();
    if !base.ends_with(b"\n") && !base.ends_with(b"\r") {
        body.push(b'\n');
    }
    let mut offsets: Vec<(u32, usize)> = Vec::with_capacity(objects.len());
    for (num, obj) in &objects {
        offsets.push((*num, base.len() + body.len()));
        body.extend_from_slice(format!("{num} 0 obj\n").as_bytes());
        obj.write(&mut body);
        body.extend_from_slice(b"\nendobj\n");
    }
    let xref_at = base.len() + body.len();
    body.extend_from_slice(b"xref\n");
    let mut i = 0;
    while i < offsets.len() {
        let mut j = i;
        while j + 1 < offsets.len() && offsets[j + 1].0 == offsets[j].0 + 1 {
            j += 1;
        }
        body.extend_from_slice(format!("{} {}\n", offsets[i].0, j - i + 1).as_bytes());
        for (_, at) in &offsets[i..=j] {
            body.extend_from_slice(format!("{at:010} 00000 n \n").as_bytes());
        }
        i = j + 1;
    }
    let mut trailer = Dict::new();
    trailer.insert("Size".into(), Obj::Int(next as i64));
    trailer.insert("Prev".into(), Obj::Int(prev as i64));
    for key in ["Root", "Info", "ID"] {
        if let Some(v) = doc.trailer.get(key) {
            trailer.insert(key.into(), v.clone());
        }
    }
    body.extend_from_slice(b"trailer\n");
    Obj::Dict(trailer).write(&mut body);
    body.extend_from_slice(format!("\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());

    let mut out = Vec::with_capacity(base.len() + body.len());
    out.extend_from_slice(base);
    out.extend_from_slice(&body);

    let check = Doc::open(out.clone()).map_err(|e| format!("the saved file does not read back: {e}"))?;
    if page::pages(&check).len() != expected_pages {
        return Err("the saved file has the wrong number of pages".into());
    }
    for (num, obj) in &objects {
        let (mut written, mut read) = (Vec::new(), Vec::new());
        obj.write(&mut written);
        check.load(*num).write(&mut read);
        if written != read {
            return Err(format!("object {num} does not read back as it was written"));
        }
    }
    Ok(out)
}

fn rect_obj(r: Rect) -> Obj {
    Obj::Array(r.iter().map(|v| Obj::Real(*v)).collect())
}

/// The offset in `base` named by its last `startxref` line.
fn last_startxref(base: &[u8]) -> Option<usize> {
    let tail = base.len().saturating_sub(4096);
    let at = base[tail..].windows(9).rposition(|w| w == b"startxref")? + tail + 9;
    let digits: String = base[at..].iter().skip_while(|c| is_ws(**c)).take_while(|c| c.is_ascii_digit()).map(|&c| c as char).collect();
    digits.parse::<usize>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg::pdf::doc::tests::tiny_pdf;

    /// A file with objects 1 to n (`objects[n - 1]`), a classic cross-reference table and a trailer with /Root 1 0 R.
    fn file(objects: &[&str]) -> Vec<u8> {
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offs = vec![];
        for (i, o) in objects.iter().enumerate() {
            offs.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, o).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for o in &offs {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n", objects.len() + 1, xref).as_bytes());
        out
    }

    const TWO: [&str; 4] = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 100] >>",
        "<< /Type /Page /Parent 2 0 R >>",
        "<< /Type /Page /Parent 2 0 R /Rotate 90 >>",
    ];

    #[test]
    fn rotation_and_an_annotation_read_back() {
        let base = tiny_pdf("BT ET");
        let doc = Doc::open(base.clone()).unwrap();
        let annotation = Annotation {
            page: 0,
            subtype: "Square".into(),
            rect: [10.0, 10.0, 100.0, 50.0],
            appearance: b"0 0 1 RG 1 w 10 10 90 40 re S".to_vec(),
            ..Annotation::default()
        };
        let edits = Edits { rotate: vec![(0, 270)], annotations: vec![annotation], ..Edits::default() };
        let out = save(&base, &doc, &edits).unwrap();
        assert!(out.starts_with(&base));
        let check = Doc::open(out).unwrap();
        let pages = page::pages(&check);
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].rotation(), 270);
        let annots = match check.field(&pages[0].dict, "Annots") {
            Obj::Array(a) => a,
            _ => panic!("the page has no annotations"),
        };
        assert_eq!(annots.len(), 1);
        let Obj::Ref(n, _) = &annots[0] else { panic!("the annotation is not an object") };
        let annot = check.load(*n);
        assert_eq!(annot.dict().unwrap()["Subtype"], Obj::Name("Square".into()));
        // whole numbers are written without a point, so they read back as integers: compare the values
        let rect: Vec<f64> = annot.dict().unwrap()["Rect"].array().unwrap().iter().filter_map(Obj::num).collect();
        assert_eq!(rect, vec![10.0, 10.0, 100.0, 50.0]);
    }

    #[test]
    fn deleting_and_reordering_a_flat_page_tree() {
        let base = file(&TWO);
        let doc = Doc::open(base.clone()).unwrap();
        let out = save(&base, &doc, &Edits { order: Some(vec![1]), ..Edits::default() }).unwrap();
        let pages = page::pages(&Doc::open(out).unwrap());
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].num, Some(4));
        assert_eq!(pages[0].rotation(), 90);
        let out = save(&base, &doc, &Edits { order: Some(vec![1, 0]), ..Edits::default() }).unwrap();
        let pages = page::pages(&Doc::open(out).unwrap());
        assert_eq!(pages.iter().map(|p| p.num).collect::<Vec<_>>(), vec![Some(4), Some(3)]);
    }

    #[test]
    fn refuses_an_order_that_names_a_page_twice_or_none() {
        let base = file(&TWO);
        let doc = Doc::open(base.clone()).unwrap();
        assert!(save(&base, &doc, &Edits { order: Some(vec![0, 0]), ..Edits::default() }).is_err());
        assert!(save(&base, &doc, &Edits { order: Some(vec![]), ..Edits::default() }).is_err());
        assert!(save(&base, &doc, &Edits { order: Some(vec![5]), ..Edits::default() }).is_err());
    }

    #[test]
    fn no_edits_return_the_file_unchanged() {
        let base = file(&TWO);
        let doc = Doc::open(base.clone()).unwrap();
        assert_eq!(save(&base, &doc, &Edits::default()).unwrap(), base);
    }

    #[test]
    fn page_points_map_to_user_space() {
        let doc = Doc::open(tiny_pdf("")).unwrap();
        let pages = page::pages(&doc);
        // the top-left quarter of a 612 by 792 page, whose y axis points up in user space
        let r = user_rect(&pages[0], 0.0, 0.0, 0.5, 0.5).unwrap();
        assert!((r[0] - 0.0).abs() < 1e-6 && (r[1] - 396.0).abs() < 1e-6 && (r[2] - 306.0).abs() < 1e-6 && (r[3] - 792.0).abs() < 1e-6, "{r:?}");
    }

    #[test]
    fn the_startxref_offset_is_read_from_the_tail() {
        let base = file(&TWO);
        let at = String::from_utf8_lossy(&base).rfind("startxref").unwrap();
        let value: usize = String::from_utf8_lossy(&base[at + 9..]).trim().split_whitespace().next().unwrap().parse().unwrap();
        assert_eq!(last_startxref(&base), Some(value));
        assert_eq!(last_startxref(b"no table here"), None);
    }
}
