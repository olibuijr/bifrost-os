//! Printing: the document is paginated to the page setup's printable area and handed to the SDK's print operation as pages
//! of styled text (the SDK writes the PDF and sends it). The lines come from the same layout the window shows, wrapped to
//! the printable width, so a printed page breaks lines where the window does.

use crate::layout::{self, Layout, Marker, Measure};
use crate::text::{is_break, Cell, COLORS, PAGE_BREAK};
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

/// Splits the lines into pages of at most `height` points. A line taller than a page still gets a page of its own, and
/// a page break ends the page its line is on.
pub fn split(cells: &[Cell], layout: &Layout, height: f32) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut first = 0;
    let mut page_top = 0.0;
    for (i, line) in layout.lines.iter().enumerate() {
        let after_break = i > 0 && layout.lines[i - 1].end > layout.lines[i - 1].start && cells[layout.lines[i - 1].end - 1].ch == PAGE_BREAK;
        if i > first && (after_break || line.top + line.height - page_top > height) {
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
                if s.highlight > 0 {
                    let [r, g, b] = highlight_rgb(s.highlight);
                    out.extend(format!("{r:.3} {g:.3} {b:.3} rg {:.2} {:.2} {:.2} {:.2} re f\n", left + origin + start, y - s.size * 0.25, end - start, s.size * 1.2).bytes());
                }
                show(out, &s, &text, left + origin + start, y);
                if s.strike {
                    let w = (s.size / 24.0).max(0.4);
                    let sy = y + s.size * 0.3;
                    out.extend(format!("{w} w {:.2} {:.2} m {:.2} {:.2} l S\n", left + origin + start, sy, left + origin + end, sy).bytes());
                }
                if s.underline || s.link != 0 {
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
            } else if is_break(cell.ch) {
                // breaks take no width and print nothing
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

/// The fill of a highlight as red, green and blue fractions: the palette colour, lightened as on screen.
fn highlight_rgb(index: u8) -> [f32; 3] {
    let [r, g, b] = COLORS[(index as usize).min(COLORS.len() - 1)].1;
    let lighten = |c: u8| (c as f32 * 2.0 + 255.0) / 3.0 / 255.0;
    [lighten(r), lighten(g), lighten(b)]
}

fn show(out: &mut Vec<u8>, s: &crate::text::Style, text: &str, x: f32, y: f32) {
    // an installed family prints in the base PDF face closest to it
    let face = crate::text::base_family(s.font) * 4 + s.bold as usize + 2 * s.italic as usize;
    // raised and lowered text is smaller and moves off the baseline (`Ts` is the text rise)
    let (size, rise) = match s.script {
        1 => (s.size * crate::view::SCRIPT_SCALE, s.size * 0.35),
        -1 => (s.size * crate::view::SCRIPT_SCALE, -s.size * 0.12),
        _ => (s.size, 0.0),
    };
    // text on a highlight is black, as on screen; a link is blue
    let color = if s.link != 0 {
        "0 0.478 1 rg 0 0.478 1 RG".to_string()
    } else if s.highlight > 0 && s.color == 0 {
        "0 0 0 rg 0 0 0 RG".to_string()
    } else {
        color_words(s.color)
    };
    out.extend(format!("BT /F{face} {size} Tf {rise:.2} Ts {} {:.2} {:.2} Td ", color, x, y).bytes());
    text_literal(text, out);
    out.extend(b" Tj ET\n");
}

/// The document as pages for `info`, ready for the print operation with [`FACES`]. `layout` must have been made for the
/// printable width of `info`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn document(cells: &[Cell], layout: &Layout, info: &PrintInfo, measure: &Measure) -> Vec<PdfPage> {
    document_with(cells, layout, info, measure, None)
}

/// The words of a page's header (the document's title, top left) and footer ("Page 2 of 5", bottom right), in the
/// print margins. Helvetica 9 pt grey, the first face of [`FACES`].
fn header_footer(title: &str, page: usize, pages: usize, info: &PrintInfo) -> Vec<u8> {
    let (left, top, width, height) = info.imageable();
    let (_, page_h) = info.paper_size();
    let mut out = Vec::new();
    let header_y = page_h - (top / 2.0).max(10.0);
    out.extend(format!("BT /F0 9 Tf 0 Ts 0.4 0.4 0.4 rg {:.2} {:.2} Td ", left, header_y).bytes());
    text_literal(title, &mut out);
    out.extend(b" Tj ET\n");
    let bottom = (page_h - top - height).max(0.0);
    let footer_y = (bottom / 2.0 - 4.0).max(6.0);
    let label = format!("Page {page} of {pages}");
    // Helvetica digits and letters average about half an em
    let x = left + width - label.chars().count() as f32 * 4.5;
    out.extend(format!("BT /F0 9 Tf 0 Ts 0.4 0.4 0.4 rg {x:.2} {footer_y:.2} Td ").bytes());
    text_literal(&label, &mut out);
    out.extend(b" Tj ET\n");
    out
}

/// Like [`document`], with the title in each page's header and "Page n of m" in its footer when `title` is given.
pub fn document_with(cells: &[Cell], layout: &Layout, info: &PrintInfo, measure: &Measure, title: Option<&str>) -> Vec<PdfPage> {
    let (_, _, _, printable_h) = info.imageable();
    let (w, h) = info.paper_size();
    let ranges = split(cells, layout, printable_h);
    let pages = ranges.len();
    ranges
        .into_iter()
        .enumerate()
        .map(|(i, range)| {
            let mut content = content(cells, layout, range, info, measure);
            if let Some(title) = title {
                content.extend(header_footer(title, i + 1, pages, info));
            }
            PdfPage { width: w as f64, height: h as f64, content, images: Vec::new() }
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
        assert_eq!(split(&c, &l, line * 2.0 + 0.1), vec![0..2, 2..4, 4..5]);
        assert_eq!(split(&c, &l, 1.0), vec![0..1, 1..2, 2..3, 3..4, 4..5], "a line taller than the page gets its own page");
    }

    #[test]
    fn a_page_break_starts_a_new_page_and_a_line_break_does_not() {
        let c = cells("a\u{c}b\u{2028}c\nd");
        let l = layout(&c, 100.0, &six);
        assert_eq!(l.lines.len(), 4);
        assert_eq!(split(&c, &l, 10_000.0), vec![0..1, 1..4], "only the page break splits the pages");
        let info = PrintInfo { margins: [72.0; 4], ..PrintInfo::default() };
        let (_, _, w, _) = info.imageable();
        let l = layout(&c, w, &six);
        let pages = document(&c, &l, &info, &six);
        assert_eq!(pages.len(), 2);
        let text = String::from_utf8_lossy(&pages[0].content).into_owned();
        assert!(text.contains("(a)") && !text.contains('\u{c}'), "the break itself prints nothing");
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
    fn a_header_and_footer_name_the_document_and_the_page() {
        let c = cells("a\nb\nc");
        let info = PrintInfo { margins: [72.0; 4], ..PrintInfo::default() };
        let (_, _, w, _) = info.imageable();
        let l = layout(&c, w, &six);
        let plain = document(&c, &l, &info, &six);
        let titled = document_with(&c, &l, &info, &six, Some("Notes (draft)"));
        let text = String::from_utf8_lossy(&titled[0].content).into_owned();
        assert!(text.contains("(Notes \\(draft\\))"));
        assert!(text.contains("(Page 1 of 1)"));
        assert!(!String::from_utf8_lossy(&plain[0].content).contains("Page 1 of"));
    }

    #[test]
    fn strikethrough_highlight_and_baseline_shift_reach_the_page() {
        let style = Style { strike: true, highlight: 3, script: 1, ..Style::default() };
        let c = vec![Cell { ch: 'x', style }];
        let info = PrintInfo { margins: [72.0; 4], ..PrintInfo::default() };
        let (_, _, w, _) = info.imageable();
        let l = layout(&c, w, &six);
        let text = String::from_utf8_lossy(&document(&c, &l, &info, &six)[0].content).into_owned();
        assert!(text.contains(" re f"), "the highlight plate");
        assert!(text.contains(" l S"), "the strike rule");
        assert!(text.contains(" Ts "), "the text rise");
    }

    #[test]
    fn every_paper_size_is_offered_once_with_a_name() {
        let names: Vec<&str> = PAPERS.iter().map(|p| paper_name(*p)).collect();
        assert_eq!(names, vec!["A4", "US Letter", "US Legal", "A5", "A3"]);
        assert!(PAPERS.iter().all(|p| PAPERS.iter().filter(|q| *q == p).count() == 1));
    }

    #[test]
    fn the_faces_list_one_name_per_family_style() {
        assert_eq!(FACES.len(), crate::text::FAMILIES.len() * 4);
        assert_eq!(FACES[5], "Times-Bold");
    }
}
