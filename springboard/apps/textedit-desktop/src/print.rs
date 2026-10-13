//! Printing: the document is paginated to the page setup's printable area and written as a PDF (std only, the standard
//! text faces, WinAnsi text), which the print kit then sends to a printer or saves. The lines come from the same layout
//! the window shows, wrapped to the printable width, so a printed page breaks lines where the window does.

use crate::layout::{self, Layout, Marker, Measure};
use crate::text::{Cell, COLORS, FAMILIES};
use bifrost_app::print::{PaperSize, PrintInfo};
use std::ops::Range;

/// The faces: each family in regular, bold, italic and bold italic order, the index being `family * 4 + bold + 2 * italic`.
const FACES: [&str; 12] = [
    "Helvetica", "Helvetica-Bold", "Helvetica-Oblique", "Helvetica-BoldOblique",
    "Times-Roman", "Times-Bold", "Times-Italic", "Times-BoldItalic",
    "Courier", "Courier-Bold", "Courier-Oblique", "Courier-BoldOblique",
];

/// How far past a line's top its baseline sits, as a multiple of the line's largest size.
const BASELINE: f32 = 1.0;

/// The paper sizes Page Setup offers, in menu order.
pub const PAPERS: [PaperSize; 5] = [PaperSize::A4, PaperSize::Letter, PaperSize::Legal, PaperSize::A5, PaperSize::A3];

/// The name of a paper size in the Page Setup menu.
pub fn paper_name(paper: PaperSize) -> &'static str {
    match paper {
        PaperSize::A4 => "A4",
        PaperSize::A3 => "A3",
        PaperSize::A5 => "A5",
        PaperSize::Letter => "US Letter",
        PaperSize::Legal => "US Legal",
    }
}

/// Splits the lines into pages of at most `height` points. A line taller than a page still gets a page of its own.
pub fn pages(layout: &Layout, height: f32) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut first = 0;
    let mut page_top = 0.0;
    for (i, line) in layout.lines.iter().enumerate() {
        if i > first && line.top + line.height - page_top > height {
            out.push(first..i);
            first = i;
            page_top = line.top;
        }
    }
    out.push(first..layout.lines.len());
    out
}

/// The WinAnsi byte for a character, `?` when the face has none.
fn ansi(ch: char) -> u8 {
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

/// A PDF string literal for `text`.
fn literal(text: &str, out: &mut Vec<u8>) {
    out.push(b'(');
    for ch in text.chars() {
        match ansi(ch) {
            b @ (b'(' | b')' | b'\\') => out.extend([b'\\', b]),
            b => out.push(b),
        }
    }
    out.push(b')');
}

fn color_words(index: u8) -> String {
    let [r, g, b] = COLORS[(index as usize).min(COLORS.len() - 1)].1;
    if index == 0 {
        "0 0 0 rg 0 0 0 RG".to_string()
    } else {
        let f = |c: u8| format!("{:.3}", c as f32 / 255.0);
        format!("{} {} {} rg {} {} {} RG", f(r), f(g), f(b), f(r), f(g), f(b))
    }
}

/// One page's content stream: the text of `lines`, positioned from the page's printable origin.
fn content(cells: &[Cell], layout: &Layout, lines: Range<usize>, info: &PrintInfo, measure: &Measure) -> Vec<u8> {
    let (left, top, _, _) = info.imageable();
    let (_, page_h) = info.paper_size();
    let page_top = layout.lines.get(lines.start).map_or(0.0, |l| l.top);
    let mut out = Vec::new();
    for line in &layout.lines[lines] {
        let size = cells[line.start..line.end].iter().map(|c| c.style.size).fold(0.0, f32::max);
        let size = if size > 0.0 { size } else { crate::text::DEFAULT_SIZE };
        let y = page_h - top - (line.top - page_top) - size * BASELINE;
        // the marker hangs in the indent
        let marker = match line.marker {
            Marker::None => None,
            Marker::Bullet => Some("\u{2022}".to_string()),
            Marker::Number(n) => Some(format!("{n}.")),
        };
        if let Some(m) = marker {
            out.extend(format!("BT /F0 {size} Tf 0 0 0 rg {:.2} {:.2} Td ", left + line.indent - 16.0, y).bytes());
            literal(&m, &mut out);
            out.extend(b" Tj ET\n");
        }
        let origin = layout.x_offset(line);
        let mut rel = 0.0;
        // the run being collected: its text, where it starts, and its style
        let mut run: Option<(String, f32, crate::text::Style)> = None;
        let flush = |out: &mut Vec<u8>, run: &mut Option<(String, f32, crate::text::Style)>, end: f32| {
            if let Some((text, start, s)) = run.take() {
                show(out, &s, &text, left + origin + start, y);
                if s.underline {
                    let w = (s.size / 24.0).max(0.4);
                    let uy = y - s.size * 0.12;
                    out.extend(format!("{w} w {:.2} {:.2} m {:.2} {:.2} l S\n", left + origin + start, uy, left + origin + end, uy).bytes());
                }
            }
        };
        for cell in &cells[line.start..line.end] {
            let step = layout::advance(rel, cell, layout.tab, measure);
            if cell.ch == '\t' {
                flush(&mut out, &mut run, rel);
            } else {
                if run.as_ref().is_some_and(|(_, _, s)| !s.same_run(&cell.style)) {
                    flush(&mut out, &mut run, rel);
                }
                run.get_or_insert_with(|| (String::new(), rel, cell.style)).0.push(cell.ch);
            }
            rel += step;
        }
        flush(&mut out, &mut run, rel);
    }
    out
}

fn show(out: &mut Vec<u8>, s: &crate::text::Style, text: &str, x: f32, y: f32) {
    let face = (s.font as usize).min(FAMILIES.len() - 1) * 4 + s.bold as usize + 2 * s.italic as usize;
    out.extend(format!("BT /F{face} {} Tf {} {:.2} {:.2} Td ", s.size, color_words(s.color), x, y).bytes());
    literal(text, out);
    out.extend(b" Tj ET\n");
}

/// The document as a PDF, paginated for `info`. `layout` must have been made for the printable width of `info`.
pub fn pdf(cells: &[Cell], layout: &Layout, info: &PrintInfo, measure: &Measure) -> Vec<u8> {
    let (_, _, _, printable_h) = info.imageable();
    let pages = pages(layout, printable_h);
    let (w, h) = info.paper_size();
    // objects: 1 catalog, 2 page tree, 3..=14 faces, then a page and its content for each page
    let mut objects: Vec<Vec<u8>> = vec![Vec::new(); 14];
    objects[0] = b"<< /Type /Catalog /Pages 2 0 R >>".to_vec();
    let kids: Vec<String> = (0..pages.len()).map(|p| format!("{} 0 R", 15 + 2 * p)).collect();
    objects[1] = format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).into_bytes();
    for (i, face) in FACES.iter().enumerate() {
        objects[2 + i] = format!("<< /Type /Font /Subtype /Type1 /BaseFont /{face} /Encoding /WinAnsiEncoding >>").into_bytes();
    }
    let fonts: String = (0..FACES.len()).map(|i| format!("/F{i} {} 0 R ", 3 + i)).collect();
    for (p, range) in pages.iter().enumerate() {
        objects.push(
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w:.2} {h:.2}] /Resources << /Font << {fonts}>> >> /Contents {} 0 R >>", 16 + 2 * p)
                .into_bytes(),
        );
        let stream = content(cells, layout, range.clone(), info, measure);
        let mut obj = format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes();
        obj.extend(stream);
        obj.extend(b"endstream");
        objects.push(obj);
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).bytes());
        out.extend(obj);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for o in offsets {
        out.extend(format!("{o:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::layout;
    use crate::text::Style;

    fn cells(s: &str) -> Vec<Cell> {
        s.chars().map(|ch| Cell { ch, style: Style::default() }).collect()
    }

    fn six(_: char, _: &Style) -> f32 {
        6.0
    }

    #[test]
    fn lines_are_split_into_pages_by_height() {
        let c = cells("a\nb\nc\nd\ne");
        let l = layout(&c, 100.0, &six);
        let line = l.lines[0].height;
        assert_eq!(pages(&l, line * 2.0 + 0.1), vec![0..2, 2..4, 4..5]);
        assert_eq!(pages(&l, 1.0), vec![0..1, 1..2, 2..3, 3..4, 4..5], "a line taller than the page gets its own page");
    }

    #[test]
    fn the_pdf_has_a_page_per_range_and_the_text() {
        let c = cells("hello (world)\nsecond");
        let info = PrintInfo { margins: [72.0; 4], ..PrintInfo::default() };
        let (_, _, w, h) = info.imageable();
        let l = layout(&c, w, &six);
        let bytes = pdf(&c, &l, &info, &six);
        let text = String::from_utf8_lossy(&bytes).into_owned();
        assert!(text.starts_with("%PDF-1.4"));
        assert!(text.trim_end().ends_with("%%EOF"));
        assert!(text.contains("/Count 1"));
        assert!(text.contains("(hello \\(world\\))"), "parentheses are escaped");
        assert!(text.contains("(second)"));
        // a tall page count follows the height
        let many = cells(&"x\n".repeat((h / l.lines[0].height) as usize * 2));
        let l2 = layout(&many, w, &six);
        let doc = String::from_utf8_lossy(&pdf(&many, &l2, &info, &six)).into_owned();
        assert!(doc.contains("/Count 3") || doc.contains("/Count 2"));
    }

    #[test]
    fn xref_offsets_point_at_their_objects() {
        let c = cells("x");
        let info = PrintInfo::default();
        let l = layout(&c, 100.0, &six);
        let bytes = pdf(&c, &l, &info, &six);
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let at = text.rfind("startxref\n").unwrap() + "startxref\n".len();
        let xref: usize = text[at..].lines().next().unwrap().parse().unwrap();
        assert!(text[xref..].starts_with("xref"));
        let first_obj = text[xref..].lines().nth(3).unwrap();
        let offset: usize = first_obj.split_whitespace().next().unwrap().parse().unwrap();
        assert!(text[offset..].starts_with("1 0 obj"));
    }

    #[test]
    fn every_paper_size_is_offered_once_with_a_name() {
        let names: Vec<&str> = PAPERS.iter().map(|p| paper_name(*p)).collect();
        assert_eq!(names, vec!["A4", "US Letter", "US Legal", "A5", "A3"]);
        assert!(PAPERS.iter().all(|p| PAPERS.iter().filter(|q| *q == p).count() == 1));
    }

    #[test]
    fn non_latin_characters_print_as_question_marks_and_quotes_map() {
        assert_eq!(ansi('\u{201c}'), 0x93);
        assert_eq!(ansi('é'), 0xe9);
        assert_eq!(ansi('日'), b'?');
    }
}
