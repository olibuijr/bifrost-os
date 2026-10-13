//! One PDF writer for two jobs.
//!
//! Writing a new document: the print pipeline builds its pages here (text from an app's layout, or raster slices of a
//! captured frame) and the same bytes go to a print queue or to a Save as PDF file. [`write`] takes drawn [`Page`]s,
//! each with a content stream, the standard fourteen faces and raster images. Content streams are written as plain
//! text so a reader and the tests can see the operators; images are RGB and Flate-compressed. Fonts are named by their
//! standard names and text is WinAnsi, see [`text_literal`].
//!
//! Changing an existing document: the Viewer's markup, page order, deletion and rotation go through [`save`]. A change
//! is written by appending the changed and new objects, a cross-reference section and a trailer whose /Prev points at
//! the old cross-reference section, so the bytes already in the file never move. The edits are: the page order for a
//! flat page tree (pages kept, reordered or deleted), the rotation of pages, and annotations with their appearance
//! streams. A save is read back before it is returned: every written object must load again as the same bytes, or the
//! save fails and nothing is written.

use super::doc::Doc;
use super::obj::{fmt_num, is_ws, write_name, Dict, Obj, Stream};
use super::page::{self, PageBox, Rect};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

/// A raster image drawn on a page: `width * height` pixels, three bytes each (red, green, blue), rows top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageImage {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}

/// One drawn page: its size in points, its content stream and the images it draws. The content refers to the
/// document's fonts as `/F<n>` (the index into the font list given to [`write`]) and to this page's images as `/Im<n>`
/// (the index into `images`).
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub width: f64,
    pub height: f64,
    pub content: Vec<u8>,
    pub images: Vec<PageImage>,
}

/// An annotation to add to a page of an existing document.
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

/// The changes to make to an existing file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Edits {
    /// The pages to keep, in their new order, as indices in `page::pages` order. `None` keeps the page tree.
    pub order: Option<Vec<usize>>,
    /// Pages whose /Rotate becomes this many degrees (a multiple of 90), by page index. The degrees are absolute.
    pub rotate: Vec<(usize, i64)>,
    /// Annotations to add.
    pub annotations: Vec<Annotation>,
}

/// The WinAnsi byte for a character, `?` when the standard faces have no glyph for it.
pub fn winansi(ch: char) -> u8 {
    match ch {
        ' '..='~' => ch as u8,
        '\u{a0}'..='\u{ff}' => ch as u32 as u8,
        '\u{20ac}' => 0x80,
        '\u{2026}' => 0x85,
        '\u{2018}' => 0x91,
        '\u{2019}' => 0x92,
        '\u{201c}' => 0x93,
        '\u{201d}' => 0x94,
        '\u{2022}' => 0x95,
        '\u{2013}' => 0x96,
        '\u{2014}' => 0x97,
        _ => b'?',
    }
}

/// Appends `text` as a PDF string literal (`(...)`), escaping parentheses and backslashes.
pub fn text_literal(text: &str, out: &mut Vec<u8>) {
    out.push(b'(');
    for ch in text.chars() {
        match winansi(ch) {
            b @ (b'(' | b')' | b'\\') => out.extend([b'\\', b]),
            b => out.push(b),
        }
    }
    out.push(b')');
}

/// A stream object's bytes: its dictionary entries, then the bytes between `stream` and `endstream`.
fn stream(entries: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {entries} /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend(data);
    out.extend(b"\nendstream");
    out
}

/// The image's stream: raw RGB compressed with zlib (`FlateDecode`).
fn image_object(img: &PageImage) -> Vec<u8> {
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&img.rgb, 6);
    stream(
        &format!(
            "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode",
            img.width, img.height
        ),
        &packed,
    )
}

/// Writes `pages` as one PDF. `fonts` are standard font names (`Helvetica`, `Times-BoldItalic`, `Courier`, ...); every
/// page lists all of them as `/F0`, `/F1`, ... in that order. A page with no images has no XObject resources.
pub fn write(pages: &[Page], fonts: &[&str]) -> Vec<u8> {
    // object numbers: 1 catalog, 2 page tree, then the fonts, then each page, its content and its images
    let first_font = 3;
    let mut next = first_font + fonts.len();
    let mut page_ids = Vec::with_capacity(pages.len());
    let mut layout = Vec::with_capacity(pages.len());
    for page in pages {
        let page_id = next;
        let content_id = next + 1;
        let image_ids: Vec<usize> = (0..page.images.len()).map(|i| content_id + 1 + i).collect();
        next = content_id + 1 + page.images.len();
        page_ids.push(page_id);
        layout.push((page_id, content_id, image_ids));
    }
    let total = next - 1;
    let mut objects: Vec<Vec<u8>> = vec![Vec::new(); total];

    objects[0] = b"<< /Type /Catalog /Pages 2 0 R >>".to_vec();
    let kids: Vec<String> = page_ids.iter().map(|id| format!("{id} 0 R")).collect();
    objects[1] = format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).into_bytes();
    for (i, face) in fonts.iter().enumerate() {
        let mut font = b"<< /Type /Font /Subtype /Type1 /BaseFont ".to_vec();
        write_name(face, &mut font);
        font.extend(b" /Encoding /WinAnsiEncoding >>");
        objects[first_font + i - 1] = font;
    }
    let font_dict: String = (0..fonts.len()).map(|i| format!("/F{i} {} 0 R ", first_font + i)).collect();

    for (page, (page_id, content_id, image_ids)) in pages.iter().zip(&layout) {
        let mut resources = String::new();
        if !fonts.is_empty() {
            resources.push_str(&format!("/Font << {font_dict}>> "));
        }
        if !image_ids.is_empty() {
            let xobjects: String = image_ids.iter().enumerate().map(|(i, id)| format!("/Im{i} {id} 0 R ")).collect();
            resources.push_str(&format!("/XObject << {xobjects}>> "));
        }
        objects[page_id - 1] = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << {resources}>> /Contents {content_id} 0 R >>",
            fmt_num(page.width),
            fmt_num(page.height)
        )
        .into_bytes();
        objects[content_id - 1] = stream("", &page.content);
        for (img, id) in page.images.iter().zip(image_ids) {
            objects[id - 1] = image_object(img);
        }
    }

    let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(total);
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).into_bytes());
        out.extend(body);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", total + 1).into_bytes());
    for off in offsets {
        out.extend(format!("{off:010} 00000 n \n").into_bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", total + 1).into_bytes());
    out
}

/// Maps a point of a page's displayed picture to default user space. The point is given as fractions of the picture
/// (0 to 1 across, y down), where the picture is the page as `page::render_page` draws it, with the page's own
/// rotation. `None` when the page has no usable box.
pub fn user_point(page: &page::Page, u: f64, v: f64) -> Option<(f64, f64)> {
    let m = page.transform(PageBox::Crop, (0.0, 0.0, 1.0, 1.0), false);
    let inv = page::invert(&m)?;
    Some(page::apply(&inv, u, v))
}

/// The default user space rectangle that covers two such points.
pub fn user_rect(page: &page::Page, u0: f64, v0: f64, u1: f64, v1: f64) -> Option<Rect> {
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

    /// A drawn A4-sized page (595 by 842 points) with `content` and `images`.
    fn drawn(content: &str, images: Vec<PageImage>) -> Page {
        Page { width: 595.0, height: 842.0, content: content.as_bytes().to_vec(), images }
    }

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

    /// The byte offset that the xref table records for object `n`.
    fn xref_offset(pdf: &[u8], n: usize) -> usize {
        let text = String::from_utf8_lossy(pdf).into_owned();
        let at = text.rfind("startxref\n").unwrap() + "startxref\n".len();
        let xref: usize = text[at..].lines().next().unwrap().parse().unwrap();
        let entry = text[xref..].lines().nth(2 + n).unwrap();
        entry.split_whitespace().next().unwrap().parse().unwrap()
    }

    #[test]
    fn one_page_per_entry_with_a_valid_xref_and_trailer() {
        let pdf = write(&[drawn("BT ET", vec![]), drawn("BT ET", vec![])], &["Helvetica"]);
        let s = String::from_utf8_lossy(&pdf).into_owned();
        assert!(s.starts_with("%PDF-1.4"));
        assert!(s.contains("/Count 2"));
        assert!(s.trim_end().ends_with("%%EOF"));
        // 2 catalog and tree, 1 font, 2 per page
        assert!(s.contains("/Size 8"));
        assert_eq!(s.matches(" 0 obj").count(), 7);
        for n in 1..=7 {
            assert!(pdf[xref_offset(&pdf, n)..].starts_with(format!("{n} 0 obj").as_bytes()));
        }
    }

    #[test]
    fn fonts_are_named_in_resources_and_the_faces_are_winansi() {
        let pdf = write(&[drawn("BT ET", vec![])], &["Helvetica", "Courier-Bold"]);
        let s = String::from_utf8_lossy(&pdf).into_owned();
        assert!(s.contains("/Font << /F0 3 0 R /F1 4 0 R >>"));
        assert!(s.contains("/BaseFont /Courier-Bold /Encoding /WinAnsiEncoding"));
        assert!(!s.contains("/XObject"), "a page without images has no XObject resource");
    }

    #[test]
    fn images_are_flate_compressed_and_inflate_back_to_their_pixels() {
        let rgb: Vec<u8> = (0..2 * 2 * 3).map(|i| i as u8).collect();
        let img = PageImage { width: 2, height: 2, rgb: rgb.clone() };
        let pdf = write(&[drawn("q 10 0 0 10 0 0 cm /Im0 Do Q", vec![img])], &[]);
        let s = String::from_utf8_lossy(&pdf).into_owned();
        assert!(s.contains("/Im0 5 0 R"), "the image is object 5 after catalog, tree, page and content");
        assert!(s.contains("/Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode"));
        let at = xref_offset(&pdf, 5);
        let body = &pdf[at..];
        let start = body.windows(7).position(|w| w == b"stream\n").unwrap() + 7;
        let end = body.windows(10).position(|w| w == b"\nendstream").unwrap();
        let inflated = miniz_oxide::inflate::decompress_to_vec_zlib(&body[start..end]).unwrap();
        assert_eq!(inflated, rgb);
    }

    #[test]
    fn text_literals_escape_parentheses_and_map_unknown_characters() {
        let mut out = Vec::new();
        text_literal("hello (world) \\ \u{201c}q\u{201d} \u{65e5}", &mut out);
        assert_eq!(out, b"(hello \\(world\\) \\\\ \x93q\x94 ?)".to_vec());
        assert_eq!(winansi('\u{e9}'), 0xe9);
        assert_eq!(winansi('\u{20ac}'), 0x80);
        assert_eq!(winansi('\u{65e5}'), b'?');
    }

    #[test]
    fn page_size_is_written_in_points() {
        let p = Page { width: 612.0, height: 791.5, content: Vec::new(), images: Vec::new() };
        let s = String::from_utf8_lossy(&write(&[p], &[])).into_owned();
        assert!(s.contains("/MediaBox [0 0 612 791.5]"));
    }

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
