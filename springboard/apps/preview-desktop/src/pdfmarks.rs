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
/// Highlight opacity, as the multiply blend of a highlighter.
const HIGHLIGHT_ALPHA: f64 = 0.45;
/// The size of a text box's type, in points.
const TEXT_PT: f64 = 12.0;

/// The edits that save `marks`, the page `order` (display order, as source page indices) and `turns` (quarter turns
/// of each source page). Marks on pages that are not in the order are left out.
pub fn edits(pages: &[PdfPage], order: &[usize], turns: &[u8], marks: &[Mark]) -> Result<Edits, String> {
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
        let quarters = turns.get(m.page).copied().unwrap_or(0);
        if let Some(ann) = annotation(&pages[m.page], m, quarters) {
            out.annotations.push(ann);
        }
    }
    Ok(out)
}

/// Writes `author` as the author (`/T`) of every annotation in `plan`. A blank author leaves them as they are.
pub fn stamp_author(plan: &mut Edits, author: &str) {
    if author.trim().is_empty() {
        return;
    }
    for a in &mut plan.annotations {
        a.entries.insert("T".into(), text_string(author));
    }
}

/// Writes the edited PDF to `target`: the file at `source` with the edits appended. Runs on the worker, which holds
/// the parsed file, so the edits are made there. Returns `target`.
pub fn write_edited(pdf: &Pdf, source: &Path, target: &Path, order: &[usize], turns: &[u8], marks: &[Mark], author: &str) -> Result<PathBuf, String> {
    let base = std::fs::read(source).map_err(|e| e.to_string())?;
    let mut plan = edits(&pdf.pages, order, turns, marks)?;
    stamp_author(&mut plan, author);
    let bytes = writer::save(&base, &pdf.doc, &plan)?;
    std::fs::write(target, bytes).map_err(|e| e.to_string())?;
    Ok(target.to_path_buf())
}

/// A job that writes the edited PDF to `target` (see `write_edited`) and replies with the file written.
pub fn save_job(source: PathBuf, target: PathBuf, order: Vec<usize>, turns: Vec<u8>, marks: Vec<Mark>, author: String) -> Job {
    pdfjob::run(move |pdf: &Pdf| Reply::Written(write_edited(pdf, &source, &target, &order, &turns, &marks, &author)))
}

/// The annotation for one mark, or None when the mark cannot be placed on its page. `quarters` is how far the page is
/// turned when it is saved, which decides which edge an underline follows.
fn annotation(page: &PdfPage, m: &Mark, quarters: u8) -> Option<Annotation> {
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
        MarkKind::Oval => {
            let rect = writer::user_rect(page, u0, v0, u1, v1)?;
            let [x0, y0, x1, y1] = rect;
            a.subtype = "Circle".into();
            a.rect = rect;
            a.entries.insert("BS".into(), Obj::Dict(width_border(2)));
            a.appearance = format!("{rgb_op} RG 2 w {} S", ellipse_ops(x0 + 1.0, y0 + 1.0, x1 - 1.0, y1 - 1.0)).into_bytes();
        }
        MarkKind::Line | MarkKind::Arrow => {
            let (p, q) = match m.ink.as_slice() {
                [p, q, ..] => (*p, *q),
                _ => return None,
            };
            let from = writer::user_point(page, p.0 as f64, p.1 as f64)?;
            let to = writer::user_point(page, q.0 as f64, q.1 as f64)?;
            let mut pts = vec![from, to];
            let mut content = format!("{rgb_op} RG 2 w 1 J 1 j {} {} m {} {} l S", n(from.0), n(from.1), n(to.0), n(to.1));
            if m.kind == MarkKind::Arrow {
                if let Some([h1, h2]) = model::arrow_head((from.0 as f32, from.1 as f32), (to.0 as f32, to.1 as f32), 1.0) {
                    let (h1, h2) = ((h1.0 as f64, h1.1 as f64), (h2.0 as f64, h2.1 as f64));
                    content.push_str(&format!(" {} {} m {} {} l {} {} l S", n(h1.0), n(h1.1), n(to.0), n(to.1), n(h2.0), n(h2.1)));
                    pts.extend([h1, h2]);
                }
                a.entries.insert("LE".into(), Obj::Array(vec![Obj::Name("None".into()), Obj::Name("OpenArrow".into())]));
            }
            a.subtype = "Line".into();
            a.rect = bounds(&pts, 2.0);
            a.entries.insert("L".into(), real_array(&[from.0, from.1, to.0, to.1]));
            a.entries.insert("BS".into(), Obj::Dict(width_border(2)));
            a.appearance = content.into_bytes();
        }
        MarkKind::Star | MarkKind::Polygon | MarkKind::Bubble => {
            let pts = model::outline(m.kind, r)
                .iter()
                .map(|&(u, v)| writer::user_point(page, u as f64, v as f64))
                .collect::<Option<Vec<(f64, f64)>>>()?;
            a.subtype = "Polygon".into();
            a.rect = bounds(&pts, 1.0);
            let flat: Vec<f64> = pts.iter().flat_map(|p| [p.0, p.1]).collect();
            a.entries.insert("Vertices".into(), real_array(&flat));
            a.entries.insert("BS".into(), Obj::Dict(width_border(2)));
            let mut content = format!("{rgb_op} RG 2 w 1 j");
            for (k, (x, y)) in pts.iter().enumerate() {
                content.push_str(&format!(" {} {} {}", n(*x), n(*y), if k == 0 { "m" } else { "l" }));
            }
            content.push_str(" h S");
            a.appearance = content.into_bytes();
        }
        MarkKind::Text => {
            // a text box with no text is not kept; the window drops it when the typing ends
            if m.text.is_empty() {
                return None;
            }
            let rect = writer::user_rect(page, u0, v0, u1, v1)?;
            let [x0, _, _, y1] = rect;
            a.subtype = "FreeText".into();
            a.rect = rect;
            a.entries.insert("Contents".into(), text_string(&m.text));
            a.entries.insert("DA".into(), Obj::Str(format!("/F1 {} Tf {rgb_op} rg", n(TEXT_PT)).into_bytes()));
            let mut content: Vec<u8> = format!("BT /F1 {} Tf {rgb_op} rg {} {} Td ", n(TEXT_PT), n(x0 + 2.0), n(y1 - TEXT_PT - 2.0)).into_bytes();
            writer::text_literal(&m.text, &mut content);
            content.extend_from_slice(b" Tj ET");
            a.appearance = content;
            // the standard Helvetica face, in the WinAnsi encoding the text is written in
            let mut font = Dict::new();
            font.insert("Type".into(), Obj::Name("Font".into()));
            font.insert("Subtype".into(), Obj::Name("Type1".into()));
            font.insert("BaseFont".into(), Obj::Name("Helvetica".into()));
            font.insert("Encoding".into(), Obj::Name("WinAnsiEncoding".into()));
            let mut fonts = Dict::new();
            fonts.insert("F1".into(), Obj::Dict(font));
            a.resources.insert("Font".into(), Obj::Dict(fonts));
        }
        // a redaction is burned into the picture it covers; a PDF keeps what is under it, so none is written
        MarkKind::Redact => return None,
        MarkKind::Underline | MarkKind::Strike => {
            let rect = writer::user_rect(page, u0, v0, u1, v1)?;
            let [x0, y0, x1, y1] = rect;
            let (p, q) = model::text_line_ends(m.kind, r, quarters);
            let from = writer::user_point(page, p.0 as f64, p.1 as f64)?;
            let to = writer::user_point(page, q.0 as f64, q.1 as f64)?;
            let subtype = if m.kind == MarkKind::Underline { "Underline" } else { "StrikeOut" };
            a.subtype = subtype.into();
            // the 2 point line spills a point past the rectangle it follows
            a.rect = [x0 - 1.0, y0 - 1.0, x1 + 1.0, y1 + 1.0];
            a.entries.insert("QuadPoints".into(), real_array(&[x0, y1, x1, y1, x0, y0, x1, y0]));
            a.appearance = format!("{rgb_op} RG 2 w {} {} m {} {} l S", n(from.0), n(from.1), n(to.0), n(to.1)).into_bytes();
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

/// A solid border `width` points wide.
fn width_border(width: i64) -> Dict {
    let mut border = Dict::new();
    border.insert("W".into(), Obj::Int(width));
    border
}

/// The smallest rectangle around some points, grown by `pad` on every side.
fn bounds(points: &[(f64, f64)], pad: f64) -> [f64; 4] {
    let min_x = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let min_y = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let max_x = points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let max_y = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    [min_x - pad, min_y - pad, max_x + pad, max_y + pad]
}

/// The path operators of the ellipse inside a rectangle: four curves, starting at the right-most point.
fn ellipse_ops(x0: f64, y0: f64, x1: f64, y1: f64) -> String {
    // the control point offset that makes a quarter curve follow a circle
    const K: f64 = 0.552_284_749_8;
    let (cx, cy, rx, ry) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, ((x1 - x0) / 2.0).max(0.0), ((y1 - y0) / 2.0).max(0.0));
    let (kx, ky) = (K * rx, K * ry);
    let curve = |a: (f64, f64), b: (f64, f64), c: (f64, f64)| format!(" {} {} {} {} {} {} c", n(a.0), n(a.1), n(b.0), n(b.1), n(c.0), n(c.1));
    let mut out = format!("{} {} m", n(cx + rx), n(cy));
    out.push_str(&curve((cx + rx, cy + ky), (cx + kx, cy + ry), (cx, cy + ry)));
    out.push_str(&curve((cx - kx, cy + ry), (cx - rx, cy + ky), (cx - rx, cy)));
    out.push_str(&curve((cx - rx, cy - ky), (cx - kx, cy - ry), (cx, cy - ry)));
    out.push_str(&curve((cx + kx, cy - ry), (cx + rx, cy - ky), (cx + rx, cy)));
    out
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
        let e = edits(&pages, &[0], &[0], std::slice::from_ref(&m)).unwrap();
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
        let e = edits(&pages, &[0], &[1], &[]).unwrap();
        assert_eq!(e.rotate, vec![(0, 90)]);
        assert_eq!(e.order, None);
        // a deleted page drops the marks on it
        let m = mark(MarkKind::Rect, 0, Norm { x: 0.1, y: 0.1, w: 0.2, h: 0.2 });
        let e = edits(&pages, &[], &[0], std::slice::from_ref(&m)).unwrap();
        assert_eq!(e.order, Some(Vec::new()));
        assert!(e.annotations.is_empty());
    }

    #[test]
    fn a_stroke_needs_two_points_and_gets_an_ink_list() {
        let doc = Doc::open(one_page()).unwrap();
        let pages = page::pages(&doc);
        let mut m = mark(MarkKind::Ink, 0, Norm { x: 0.0, y: 0.0, w: 1.0, h: 1.0 });
        m.ink = vec![(0.0, 0.0)];
        assert!(edits(&pages, &[0], &[0], std::slice::from_ref(&m)).unwrap().annotations.is_empty());
        m.ink = vec![(0.0, 0.0), (1.0, 1.0)];
        let e = edits(&pages, &[0], &[0], std::slice::from_ref(&m)).unwrap();
        assert_eq!(e.annotations[0].subtype, "Ink");
        assert_eq!(e.annotations[0].entries["InkList"].array().unwrap()[0].array().unwrap().len(), 4);
    }

    fn line(kind: MarkKind, ends: [(f32, f32); 2]) -> Mark {
        model::drag_mark(kind, 0, ends[0], ends[1], MARK_BLUE)
    }

    #[test]
    fn an_oval_becomes_a_circle_annotation_with_four_curves() {
        let doc = Doc::open(one_page()).unwrap();
        let pages = page::pages(&doc);
        let m = mark(MarkKind::Oval, 0, Norm { x: 0.0, y: 0.0, w: 0.5, h: 0.5 });
        let e = edits(&pages, &[0], &[0], std::slice::from_ref(&m)).unwrap();
        let a = &e.annotations[0];
        assert_eq!(a.subtype, "Circle");
        let text = String::from_utf8(a.appearance.clone()).unwrap();
        assert_eq!(text.matches(" c").count(), 4, "{text}");
        assert!(text.ends_with(" S"));
    }

    #[test]
    fn lines_and_arrows_become_line_annotations_and_only_the_arrow_has_a_head() {
        let doc = Doc::open(one_page()).unwrap();
        let pages = page::pages(&doc);
        let l = line(MarkKind::Line, [(0.1, 0.1), (0.9, 0.5)]);
        let e = edits(&pages, &[0], &[0], std::slice::from_ref(&l)).unwrap();
        let a = &e.annotations[0];
        assert_eq!(a.subtype, "Line");
        assert_eq!(a.entries["L"].array().unwrap().len(), 4);
        assert!(!a.entries.contains_key("LE"));
        let ar = line(MarkKind::Arrow, [(0.1, 0.1), (0.9, 0.5)]);
        let e = edits(&pages, &[0], &[0], std::slice::from_ref(&ar)).unwrap();
        let head = &e.annotations[0];
        assert_eq!(head.entries["LE"].array().unwrap().len(), 2);
        assert!(head.appearance.len() > a.appearance.len());
        // the head lies inside the annotation's rectangle
        assert!(head.rect[2] > head.rect[0] && head.rect[3] > head.rect[1]);
        // a line with one end is not placed
        let mut broken = l.clone();
        broken.ink.truncate(1);
        assert!(edits(&pages, &[0], &[0], &[broken]).unwrap().annotations.is_empty());
    }

    #[test]
    fn underline_and_strike_follow_the_edge_the_page_shows_as_bottom_and_middle() {
        let doc = Doc::open(one_page()).unwrap();
        let pages = page::pages(&doc);
        let r = Norm { x: 0.25, y: 0.25, w: 0.5, h: 0.1 };
        let under = model::drag_mark(MarkKind::Underline, 0, (r.x, r.y), (r.x + r.w, r.y + r.h), MARK_BLUE);
        let e = edits(&pages, &[0], &[0], std::slice::from_ref(&under)).unwrap();
        let a = &e.annotations[0];
        assert_eq!(a.subtype, "Underline");
        assert_eq!(a.entries["QuadPoints"].array().unwrap().len(), 8);
        // unturned, the line is the rectangle's bottom edge: a letter page is 792 high and the rectangle spans 0.25 to 0.35
        let text = String::from_utf8(a.appearance.clone()).unwrap();
        let y: f64 = text.split_whitespace().nth(7).unwrap().parse().unwrap();
        assert!((y - 792.0 * (1.0 - 0.35)).abs() < 1e-3, "{text}");
        let strike = model::drag_mark(MarkKind::Strike, 0, (r.x, r.y), (r.x + r.w, r.y + r.h), model::MARK_RED);
        let e = edits(&pages, &[0], &[0], std::slice::from_ref(&strike)).unwrap();
        assert_eq!(e.annotations[0].subtype, "StrikeOut");
        // turned a quarter, the underline runs down the rectangle's right edge instead
        let e = edits(&pages, &[0], &[1], std::slice::from_ref(&under)).unwrap();
        let turned = String::from_utf8(e.annotations[0].appearance.clone()).unwrap();
        let tokens: Vec<f64> = turned.split_whitespace().filter_map(|t| t.parse().ok()).collect();
        // the numbers are r g b, the width 2, then x1 y1 x2 y2, so the two x values match for a vertical line
        assert!((tokens[4] - tokens[6]).abs() < 1e-6, "{turned}");
    }

    #[test]
    fn a_text_box_becomes_a_free_text_annotation_and_needs_text() {
        let doc = Doc::open(one_page()).unwrap();
        let pages = page::pages(&doc);
        let mut m = mark(MarkKind::Text, 0, Norm { x: 0.1, y: 0.1, w: 0.4, h: 0.1 });
        assert!(edits(&pages, &[0], &[0], std::slice::from_ref(&m)).unwrap().annotations.is_empty());
        m.text = "Hello (draft)".into();
        let e = edits(&pages, &[0], &[0], std::slice::from_ref(&m)).unwrap();
        let a = &e.annotations[0];
        assert_eq!(a.subtype, "FreeText");
        let text = String::from_utf8_lossy(&a.appearance).into_owned();
        assert!(text.contains("(Hello \\(draft\\)) Tj"), "{text}");
        assert!(a.resources.contains_key("Font"));
    }

    #[test]
    fn stars_and_bubbles_become_closed_polygon_annotations() {
        let doc = Doc::open(one_page()).unwrap();
        let pages = page::pages(&doc);
        let star = mark(MarkKind::Star, 0, Norm { x: 0.1, y: 0.1, w: 0.5, h: 0.5 });
        let e = edits(&pages, &[0], &[0], std::slice::from_ref(&star)).unwrap();
        let a = &e.annotations[0];
        assert_eq!(a.subtype, "Polygon");
        assert_eq!(a.entries["Vertices"].array().unwrap().len(), 20, "ten points");
        assert!(String::from_utf8(a.appearance.clone()).unwrap().ends_with(" h S"));
        let bubble = mark(MarkKind::Bubble, 0, Norm { x: 0.1, y: 0.1, w: 0.5, h: 0.2 });
        assert_eq!(edits(&pages, &[0], &[0], std::slice::from_ref(&bubble)).unwrap().annotations[0].subtype, "Polygon");
    }

    #[test]
    fn a_redaction_is_not_written_to_a_pdf_because_the_text_under_it_would_stay() {
        let doc = Doc::open(one_page()).unwrap();
        let pages = page::pages(&doc);
        let m = mark(MarkKind::Redact, 0, Norm { x: 0.1, y: 0.1, w: 0.5, h: 0.5 });
        assert!(edits(&pages, &[0], &[0], std::slice::from_ref(&m)).unwrap().annotations.is_empty());
    }

    #[test]
    fn the_author_is_stamped_on_every_annotation_unless_it_is_blank() {
        let doc = Doc::open(one_page()).unwrap();
        let pages = page::pages(&doc);
        let m = mark(MarkKind::Highlight, 0, Norm { x: 0.0, y: 0.0, w: 0.5, h: 0.5 });
        let mut e = edits(&pages, &[0], &[0], std::slice::from_ref(&m)).unwrap();
        stamp_author(&mut e, "   ");
        assert!(!e.annotations[0].entries.contains_key("T"));
        stamp_author(&mut e, "Ana");
        assert_eq!(e.annotations[0].entries["T"], text_string("Ana"));
    }

    #[test]
    fn text_is_written_as_utf16_with_a_byte_order_mark() {
        assert_eq!(text_string("ó"), Obj::Str(vec![0xfe, 0xff, 0x00, 0xf3]));
    }
}
