//! Markup as PDF annotations. Each mark becomes an annotation on its page, with an appearance stream so any reader
//! draws it, and the page order, deletions and turns become page-tree and rotation edits. The mapping reads the pages
//! on the PDF worker (it needs each page's box and rotation); the file work is `cg::pdf::writer`.

use crate::model::{self, Mark, MarkKind};
use crate::pdfjob::{self, Job, Pdf, Reply};
use springboard::cg::pdf::obj::{fmt_num, Dict, Obj};
use springboard::cg::pdf::page::Page as PdfPage;
use springboard::cg::pdf::writer::{self, Annotation, Edits};
use std::path::{Path, PathBuf};

/// A note's marker in default user space, in points.
const NOTE_PT: f64 = 14.0;
/// The size of a text box's type, in points.
const TEXT_PT: f64 = 12.0;
/// Highlight opacity, as the multiply blend of a highlighter.
const HIGHLIGHT_ALPHA: f64 = 0.45;

/// The edits that save `marks`, the page `order` (display order, as source page indices) and `turns` (quarter turns
/// of each source page). Marks on pages that are not in the order are left out. A non-empty `author` is written as
/// the author of every annotation.
pub fn edits(pages: &[PdfPage], order: &[usize], turns: &[u8], marks: &[Mark], author: &str) -> Result<Edits, String> {
    let identity: Vec<usize> = (0..pages.len()).collect();
    let mut out = Edits::default();
    if order != identity.as_slice() {
        out.order = Some(order.to_vec());
    }
    for (src, &t) in turns.iter().enumerate().take(pages.len()) {
        if t % 4 != 0 {
            let degrees = (pages[src].rotation() + 90 * (t % 4) as i32).rem_euclid(360) as i64;
            out.rotate.push((src, degrees));
        }
    }
    for m in marks {
        if m.page >= pages.len() || !order.contains(&m.page) {
            continue;
        }
        if let Some(mut ann) = annotation(&pages[m.page], m) {
            if !author.is_empty() {
                ann.entries.insert("T".into(), text_string(author));
            }
            out.annotations.push(ann);
        }
    }
    Ok(out)
}

/// Writes the edited PDF to `target`: the file at `source` with the edits appended. Runs on the worker, which holds
/// the parsed file, so the edits are made there. Returns `target`.
pub fn write_edited(pdf: &Pdf, source: &Path, target: &Path, order: &[usize], turns: &[u8], marks: &[Mark], author: &str) -> Result<PathBuf, String> {
    let base = std::fs::read(source).map_err(|e| e.to_string())?;
    let plan = edits(&pdf.pages, order, turns, marks, author)?;
    let bytes = writer::save(&base, &pdf.doc, &plan)?;
    std::fs::write(target, bytes).map_err(|e| e.to_string())?;
    Ok(target.to_path_buf())
}

/// A job that writes the edited PDF to `target` (see `write_edited`) and replies with the file written.
pub fn save_job(source: PathBuf, target: PathBuf, order: Vec<usize>, turns: Vec<u8>, marks: Vec<Mark>, author: String) -> Job {
    pdfjob::run(move |pdf: &Pdf| Reply::Written(write_edited(pdf, &source, &target, &order, &turns, &marks, &author)))
}

/// Writes the edited PDF beside `source` and moves it over `source`. The caller has already kept a backup of the
/// original. A failed write leaves `source` as it was.
pub fn replace_file(pdf: &Pdf, source: &Path, order: &[usize], turns: &[u8], marks: &[Mark], author: &str) -> Result<PathBuf, String> {
    let temp = source.with_extension("pdf.saving");
    write_edited(pdf, source, &temp, order, turns, marks, author)?;
    if let Err(e) = std::fs::rename(&temp, source) {
        let _ = std::fs::remove_file(&temp);
        return Err(e.to_string());
    }
    Ok(source.to_path_buf())
}

/// A job that saves the edited PDF over `source` (see `replace_file`) and replies with the file written.
pub fn save_in_place_job(source: PathBuf, order: Vec<usize>, turns: Vec<u8>, marks: Vec<Mark>, author: String) -> Job {
    pdfjob::run(move |pdf: &Pdf| Reply::Written(replace_file(pdf, &source, &order, &turns, &marks, &author)))
}

/// The annotation for one mark, or None when the mark cannot be placed on its page.
fn annotation(page: &PdfPage, m: &Mark) -> Option<Annotation> {
    let color = rgb(m.color);
    let rgb_op = format!("{} {} {}", n(color[0]), n(color[1]), n(color[2]));
    let r = m.rect;
    let (u0, v0, u1, v1) = (r.x as f64, r.y as f64, (r.x + r.w) as f64, (r.y + r.h) as f64);
    let mut a = Annotation { page: m.page, ..Annotation::default() };
    a.entries.insert("C".into(), real_array(&color));
    match m.kind {
        MarkKind::Highlight => {
            let rect = writer::user_rect(page, u0, v0, u1, v1)?;
            let [x0, y0, x1, y1] = rect;
            a.subtype = "Highlight".into();
            a.rect = rect;
            a.entries.insert("QuadPoints".into(), real_array(&[x0, y1, x1, y1, x0, y0, x1, y0]));
            a.appearance = format!("/GS0 gs {rgb_op} rg {} {} {} {} re f", n(x0), n(y0), n(x1 - x0), n(y1 - y0)).into_bytes();
            let mut state = Dict::new();
            state.insert("CA".into(), Obj::Real(HIGHLIGHT_ALPHA));
            state.insert("ca".into(), Obj::Real(HIGHLIGHT_ALPHA));
            state.insert("BM".into(), Obj::Name("Multiply".into()));
            let mut states = Dict::new();
            states.insert("GS0".into(), Obj::Dict(state));
            a.resources.insert("ExtGState".into(), Obj::Dict(states));
        }
        MarkKind::Rect | MarkKind::Rounded => {
            // a rounded rectangle is drawn square: the appearance has no corner curves
            let rect = writer::user_rect(page, u0, v0, u1, v1)?;
            let [x0, y0, x1, y1] = rect;
            a.subtype = "Square".into();
            a.rect = rect;
            let mut border = Dict::new();
            border.insert("W".into(), Obj::Int(2));
            border.insert("S".into(), Obj::Name("S".into()));
            a.entries.insert("BS".into(), Obj::Dict(border));
            a.appearance = format!(
                "{rgb_op} RG 2 w {} {} {} {} re S",
                n(x0 + 1.0),
                n(y0 + 1.0),
                n((x1 - x0 - 2.0).max(0.0)),
                n((y1 - y0 - 2.0).max(0.0))
            )
            .into_bytes();
        }
        MarkKind::Note => {
            // the viewer's marker hangs down and to the right of its anchor; in user space that is down and to the right
            let (x, y) = writer::user_point(page, u0, v0)?;
            let rect = [x, y - NOTE_PT, x + NOTE_PT, y];
            a.subtype = "Text".into();
            a.rect = rect;
            a.entries.insert("Name".into(), Obj::Name("Note".into()));
            a.entries.insert("Open".into(), Obj::Bool(false));
            a.entries.insert("Contents".into(), text_string(&m.text));
            a.appearance = format!("{rgb_op} rg {} {} {} {} re f", n(rect[0]), n(rect[1]), n(NOTE_PT), n(NOTE_PT)).into_bytes();
        }
        MarkKind::Redact => {
            // the box is drawn solid black; the text under it stays in the file, which the Viewer says when it saves
            let rect = writer::user_rect(page, u0, v0, u1, v1)?;
            let [x0, y0, x1, y1] = rect;
            a.subtype = "Square".into();
            a.rect = rect;
            a.entries.insert("IC".into(), real_array(&[0.0, 0.0, 0.0]));
            a.entries.insert("C".into(), real_array(&[0.0, 0.0, 0.0]));
            a.appearance = format!("0 g {} {} {} {} re f", n(x0), n(y0), n(x1 - x0), n(y1 - y0)).into_bytes();
        }
        MarkKind::Underline | MarkKind::Strike => {
            let rect = writer::user_rect(page, u0, v0, u1, v1)?;
            let [x0, y0, x1, y1] = rect;
            a.subtype = if m.kind == MarkKind::Underline { "Underline" } else { "StrikeOut" }.into();
            a.rect = rect;
            a.entries.insert("QuadPoints".into(), real_array(&[x0, y1, x1, y1, x0, y0, x1, y0]));
            let y = if m.kind == MarkKind::Underline { y0 + 1.0 } else { (y0 + y1) / 2.0 };
            a.appearance = format!("{rgb_op} RG 1.5 w {} {} m {} {} l S", n(x0), n(y), n(x1), n(y)).into_bytes();
        }
        MarkKind::Text => {
            let rect = writer::user_rect(page, u0, v0, u1, v1)?;
            let [x0, _, x1, y1] = rect;
            a.subtype = "FreeText".into();
            a.rect = rect;
            a.entries.insert("Contents".into(), text_string(&m.text));
            a.entries.insert("DA".into(), Obj::Str(format!("/Helv {TEXT_PT} Tf {rgb_op} rg").into_bytes()));
            let mut content = format!("BT /Helv {TEXT_PT} Tf {rgb_op} rg {} {} Td {} TL ", n(x0 + 2.0), n(y1 - TEXT_PT - 2.0), n(TEXT_PT * 1.2)).into_bytes();
            for (i, line) in m.text.lines().enumerate() {
                if i > 0 {
                    content.extend(b" T* ");
                }
                writer::text_literal(line, &mut content);
                content.extend(b" Tj");
            }
            content.extend(b" ET");
            a.appearance = content;
            let mut face = Dict::new();
            face.insert("Type".into(), Obj::Name("Font".into()));
            face.insert("Subtype".into(), Obj::Name("Type1".into()));
            face.insert("BaseFont".into(), Obj::Name("Helvetica".into()));
            face.insert("Encoding".into(), Obj::Name("WinAnsiEncoding".into()));
            let mut fonts = Dict::new();
            fonts.insert("Helv".into(), Obj::Dict(face));
            a.resources.insert("Font".into(), Obj::Dict(fonts));
            let _ = x1;
        }
        MarkKind::Oval | MarkKind::Line | MarkKind::Arrow | MarkKind::Star | MarkKind::Bubble => {
            // these are drawn from the same outlines the window draws, mapped into user space
            let mut content = format!("{rgb_op} RG 2 w 1 J 1 j");
            let mut all: Vec<(f64, f64)> = Vec::new();
            let mut first: Vec<(f64, f64)> = Vec::new();
            for (i, line) in model::outlines(m).iter().enumerate() {
                let pts = line
                    .iter()
                    .map(|&(u, v)| writer::user_point(page, u as f64, v as f64))
                    .collect::<Option<Vec<(f64, f64)>>>()?;
                for (k, (x, y)) in pts.iter().enumerate() {
                    content.push_str(&format!(" {} {} {}", n(*x), n(*y), if k == 0 { "m" } else { "l" }));
                }
                content.push_str(" S");
                if i == 0 {
                    first = pts.clone();
                }
                all.extend(pts);
            }
            if all.len() < 2 {
                return None;
            }
            let min_x = all.iter().map(|p| p.0).fold(f64::INFINITY, f64::min) - 2.0;
            let min_y = all.iter().map(|p| p.1).fold(f64::INFINITY, f64::min) - 2.0;
            let max_x = all.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max) + 2.0;
            let max_y = all.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max) + 2.0;
            a.rect = [min_x, min_y, max_x, max_y];
            a.appearance = content.into_bytes();
            let mut border = Dict::new();
            border.insert("W".into(), Obj::Int(2));
            a.entries.insert("BS".into(), Obj::Dict(border));
            match m.kind {
                MarkKind::Oval => a.subtype = "Circle".into(),
                MarkKind::Line | MarkKind::Arrow => {
                    a.subtype = "Line".into();
                    let (start, end) = (first.first()?, first.last()?);
                    a.entries.insert("L".into(), real_array(&[start.0, start.1, end.0, end.1]));
                    let tip = if m.kind == MarkKind::Arrow { "OpenArrow" } else { "None" };
                    a.entries.insert("LE".into(), Obj::Array(vec![Obj::Name("None".into()), Obj::Name(tip.into())]));
                }
                _ => {
                    a.subtype = "Polygon".into();
                    let flat: Vec<f64> = first.iter().flat_map(|p| [p.0, p.1]).collect();
                    a.entries.insert("Vertices".into(), real_array(&flat));
                }
            }
        }
        MarkKind::Ink => {
            let pts = m
                .ink
                .iter()
                .map(|&(u, v)| writer::user_point(page, u as f64, v as f64))
                .collect::<Option<Vec<(f64, f64)>>>()?;
            if pts.len() < 2 {
                return None;
            }
            let min_x = pts.iter().map(|p| p.0).fold(f64::INFINITY, f64::min) - 2.0;
            let min_y = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min) - 2.0;
            let max_x = pts.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max) + 2.0;
            let max_y = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max) + 2.0;
            a.subtype = "Ink".into();
            a.rect = [min_x, min_y, max_x, max_y];
            let flat: Vec<f64> = pts.iter().flat_map(|p| [p.0, p.1]).collect();
            a.entries.insert("InkList".into(), Obj::Array(vec![real_array(&flat)]));
            let mut border = Dict::new();
            border.insert("W".into(), Obj::Int(2));
            a.entries.insert("BS".into(), Obj::Dict(border));
            let mut content = format!("{rgb_op} RG 2 w 1 J 1 j");
            for (k, (x, y)) in pts.iter().enumerate() {
                content.push_str(&format!(" {} {} {}", n(*x), n(*y), if k == 0 { "m" } else { "l" }));
            }
            content.push_str(" S");
            a.appearance = content.into_bytes();
        }
    }
    Some(a)
}

/// A colour's red, green and blue as fractions, from a packed colour (red in the low byte).
fn rgb(c: u32) -> [f64; 3] {
    [(c & 0xff) as f64 / 255.0, ((c >> 8) & 0xff) as f64 / 255.0, ((c >> 16) & 0xff) as f64 / 255.0]
}

fn n(v: f64) -> String {
    fmt_num(v)
}

fn real_array(values: &[f64]) -> Obj {
    Obj::Array(values.iter().map(|v| Obj::Real(*v)).collect())
}

/// A text string as PDF writes it: UTF-16BE with a byte order mark, so any character reads back.
fn text_string(text: &str) -> Obj {
    let mut bytes = vec![0xfe, 0xff];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    Obj::Str(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Norm, HIGHLIGHT, MARK_BLUE};
    use springboard::cg::pdf::doc::Doc;
    use springboard::cg::pdf::page;

    /// A one-page PDF (letter size, 612 by 792) with a classic cross-reference table.
    fn one_page() -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        ];
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

    fn mark(kind: MarkKind, page: usize, rect: Norm) -> Mark {
        Mark { kind, page, rect, color: if kind == MarkKind::Highlight { HIGHLIGHT } else { MARK_BLUE }, text: String::new(), ink: Vec::new() }
    }

    #[test]
    fn a_highlight_becomes_a_highlight_annotation_with_quad_points() {
        let doc = Doc::open(one_page()).unwrap();
        let pages = page::pages(&doc);
        let m = mark(MarkKind::Highlight, 0, Norm { x: 0.0, y: 0.0, w: 0.5, h: 0.5 });
        let e = edits(&pages, &[0], &[0], std::slice::from_ref(&m), "").unwrap();
        assert_eq!(e.order, None);
        assert!(e.rotate.is_empty());
        assert_eq!(e.annotations.len(), 1);
        let a = &e.annotations[0];
        assert_eq!(a.subtype, "Highlight");
        // the page-to-user mapping is an inverted matrix, so the corners carry float noise (395.99999999999994)
        for (got, want) in a.rect.iter().zip([0.0, 396.0, 306.0, 792.0]) {
            assert!((got - want).abs() < 1e-9, "{:?}", a.rect);
        }
        assert_eq!(a.entries["QuadPoints"].array().unwrap().len(), 8);
        assert!(a.appearance.starts_with(b"/GS0 gs"));
    }

    #[test]
    fn turns_and_deletions_become_rotation_and_page_order() {
        let doc = Doc::open(one_page()).unwrap();
        let pages = page::pages(&doc);
        // one page turned a quarter clockwise, the page left in place
        let e = edits(&pages, &[0], &[1], &[], "").unwrap();
        assert_eq!(e.rotate, vec![(0, 90)]);
        assert_eq!(e.order, None);
        // a deleted page drops the marks on it
        let m = mark(MarkKind::Rect, 0, Norm { x: 0.1, y: 0.1, w: 0.2, h: 0.2 });
        let e = edits(&pages, &[], &[0], std::slice::from_ref(&m), "").unwrap();
        assert_eq!(e.order, Some(Vec::new()));
        assert!(e.annotations.is_empty());
    }

    #[test]
    fn a_stroke_needs_two_points_and_gets_an_ink_list() {
        let doc = Doc::open(one_page()).unwrap();
        let pages = page::pages(&doc);
        let mut m = mark(MarkKind::Ink, 0, Norm { x: 0.0, y: 0.0, w: 1.0, h: 1.0 });
        m.ink = vec![(0.0, 0.0)];
        assert!(edits(&pages, &[0], &[0], std::slice::from_ref(&m), "").unwrap().annotations.is_empty());
        m.ink = vec![(0.0, 0.0), (1.0, 1.0)];
        let e = edits(&pages, &[0], &[0], std::slice::from_ref(&m), "").unwrap();
        assert_eq!(e.annotations[0].subtype, "Ink");
        assert_eq!(e.annotations[0].entries["InkList"].array().unwrap()[0].array().unwrap().len(), 4);
    }

    fn rect_of_page() -> Norm {
        Norm { x: 0.1, y: 0.2, w: 0.4, h: 0.1 }
    }

    #[test]
    fn each_new_kind_gets_its_annotation_type() {
        let doc = Doc::open(one_page()).unwrap();
        let pages = page::pages(&doc);
        let cases = [
            (MarkKind::Oval, "Circle"),
            (MarkKind::Star, "Polygon"),
            (MarkKind::Bubble, "Polygon"),
            (MarkKind::Underline, "Underline"),
            (MarkKind::Strike, "StrikeOut"),
            (MarkKind::Redact, "Square"),
            (MarkKind::Text, "FreeText"),
        ];
        for (kind, subtype) in cases {
            let mut m = mark(kind, 0, rect_of_page());
            m.text = "Hello".into();
            let e = edits(&pages, &[0], &[0], std::slice::from_ref(&m), "").unwrap();
            assert_eq!(e.annotations.len(), 1, "{kind:?}");
            assert_eq!(e.annotations[0].subtype, subtype, "{kind:?}");
            assert!(!e.annotations[0].appearance.is_empty(), "{kind:?}");
        }
    }

    #[test]
    fn a_polygon_lists_its_vertices_and_a_line_its_ends() {
        let doc = Doc::open(one_page()).unwrap();
        let pages = page::pages(&doc);
        let star = mark(MarkKind::Star, 0, rect_of_page());
        let e = edits(&pages, &[0], &[0], std::slice::from_ref(&star), "").unwrap();
        assert_eq!(e.annotations[0].entries["Vertices"].array().unwrap().len(), 22);
        let mut arrow = mark(MarkKind::Arrow, 0, rect_of_page());
        arrow.ink = vec![(0.1, 0.2), (0.5, 0.3)];
        let e = edits(&pages, &[0], &[0], std::slice::from_ref(&arrow), "").unwrap();
        let a = &e.annotations[0];
        assert_eq!(a.subtype, "Line");
        assert_eq!(a.entries["L"].array().unwrap().len(), 4);
        assert_eq!(a.entries["LE"].array().unwrap().len(), 2);
    }

    #[test]
    fn a_text_box_carries_its_text_and_font() {
        let doc = Doc::open(one_page()).unwrap();
        let pages = page::pages(&doc);
        let mut m = mark(MarkKind::Text, 0, rect_of_page());
        m.text = "one\ntwo".into();
        let e = edits(&pages, &[0], &[0], std::slice::from_ref(&m), "").unwrap();
        let a = &e.annotations[0];
        assert!(a.entries.contains_key("Contents") && a.entries.contains_key("DA"));
        assert!(a.resources.contains_key("Font"));
        let text = String::from_utf8_lossy(&a.appearance).to_string();
        assert!(text.contains("(one) Tj") && text.contains("T* (two) Tj"), "{text}");
    }

    #[test]
    fn the_author_is_written_on_every_annotation_when_set() {
        let doc = Doc::open(one_page()).unwrap();
        let pages = page::pages(&doc);
        let m = mark(MarkKind::Rect, 0, rect_of_page());
        let with = edits(&pages, &[0], &[0], std::slice::from_ref(&m), "Sigga").unwrap();
        assert!(with.annotations[0].entries.contains_key("T"));
        let without = edits(&pages, &[0], &[0], std::slice::from_ref(&m), "").unwrap();
        assert!(!without.annotations[0].entries.contains_key("T"));
    }

    #[test]
    fn saving_in_place_replaces_the_file_and_leaves_no_temporary() {
        let dir = std::env::temp_dir().join(format!("viewer-inplace-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("doc.pdf");
        std::fs::write(&file, one_page()).unwrap();
        let doc = Doc::open(one_page()).unwrap();
        let pages = page::pages(&doc);
        let pdf = Pdf { doc, pages };
        let m = mark(MarkKind::Rect, 0, rect_of_page());
        let done = replace_file(&pdf, &file, &[0], &[0], std::slice::from_ref(&m), "").unwrap();
        assert_eq!(done, file);
        assert!(std::fs::read(&file).unwrap().len() > one_page().len());
        assert!(!file.with_extension("pdf.saving").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn text_is_written_as_utf16_with_a_byte_order_mark() {
        assert_eq!(text_string("ó"), Obj::Str(vec![0xfe, 0xff, 0x00, 0xf3]));
    }
}
