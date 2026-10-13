//! Printing: the document is paginated to the page setup's printable area and handed to the SDK's print operation as pages
//! of styled text (the SDK writes the PDF and sends it). The lines come from the same layout the window shows, wrapped to
//! the printable width, so a printed page breaks lines where the window does.

use crate::layout::{self, Layout, Marker, Measure};
use crate::text::{Cell, COLORS, FAMILIES};
use bifrost_app::kit::cg::pdf::writer::text_literal;
use bifrost_app::print::{PaperSize, PdfPage, PrintInfo};
use std::ops::Range;

/// The faces the document prints with: each family in regular, bold, italic and bold italic order, the index being
/// `family * 4 + bold + 2 * italic`. The print operation takes them as its font list.
pub const FACES: [&str; 12] = [
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
pub fn split(layout: &Layout, height: f32) -> Vec<Range<usize>> {
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
            text_literal(&m, &mut out);
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
    text_literal(text, out);
    out.extend(b" Tj ET\n");
}

/// The document as pages for `info`, ready for the print operation with [`FACES`]. `layout` must have been made for the
/// printable width of `info`.
pub fn document(cells: &[Cell], layout: &Layout, info: &PrintInfo, measure: &Measure) -> Vec<PdfPage> {
    let (_, _, _, printable_h) = info.imageable();
    let (w, h) = info.paper_size();
    split(layout, printable_h)
        .into_iter()
        .map(|range| PdfPage {
            width: w as f64,
            height: h as f64,
            content: content(cells, layout, range, info, measure),
            images: Vec::new(),
        })
        .collect()
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
        assert_eq!(split(&l, line * 2.0 + 0.1), vec![0..2, 2..4, 4..5]);
        assert_eq!(split(&l, 1.0), vec![0..1, 1..2, 2..3, 3..4, 4..5], "a line taller than the page gets its own page");
    }

    #[test]
    fn the_document_has_a_page_per_range_with_the_text_escaped() {
        let c = cells("hello (world)\nsecond");
        let info = PrintInfo { margins: [72.0; 4], ..PrintInfo::default() };
        let (_, _, w, h) = info.imageable();
        let l = layout(&c, w, &six);
        let pages = document(&c, &l, &info, &six);
        assert_eq!(pages.len(), 1);
        assert_eq!((pages[0].width, pages[0].height), (595.0, 842.0));
        let text = String::from_utf8_lossy(&pages[0].content).into_owned();
        assert!(text.contains("(hello \\(world\\))"), "parentheses are escaped");
        assert!(text.contains("(second)"));
        // a tall text yields one page per printable height
        let many = cells(&"x\n".repeat((h / l.lines[0].height) as usize * 2));
        let l2 = layout(&many, w, &six);
        assert!(document(&many, &l2, &info, &six).len() >= 2);
    }

    #[test]
    fn every_paper_size_is_offered_once_with_a_name() {
        let names: Vec<&str> = PAPERS.iter().map(|p| paper_name(*p)).collect();
        assert_eq!(names, vec!["A4", "US Letter", "US Legal", "A5", "A3"]);
        assert!(PAPERS.iter().all(|p| PAPERS.iter().filter(|q| *q == p).count() == 1));
    }

    #[test]
    fn the_faces_list_one_name_per_family_style() {
        assert_eq!(FACES.len(), FAMILIES.len() * 4);
        assert_eq!(FACES[5], "Times-Bold");
    }
}
