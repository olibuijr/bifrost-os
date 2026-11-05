//! PDF documents for apps: open a file to read its pages (count, size, a rasterised picture of a page) and build a new
//! file from drawn pages. The reader is `bifrost_render::cg::pdf` (`Doc`, `page`, `draw`); the writer is its
//! `writer` module. Drawing is limited to what the writer emits: filled and stroked rectangles and lines, filled and
//! stroked paths with curves (`cg::path::Path`, with caps, joins and dashes), raster images, and text in the four
//! standard faces ([`Face`], WinAnsi only). `PdfWriter::finish_filtered` writes through a `QuartzFilter`. Reading gives
//! the text of each page ([`Document::page_text`]) and every occurrence of a string ([`Document::search`], a
//! [`Selection`] per match); annotation editing stays in the PDF app.
//!
//! ```ignore
//! let doc = graphics_pdf::Document::open(bytes)?;
//! let picture = doc.render(0, 800, 1100)?;
//!
//! let mut out = graphics_pdf::PdfWriter::new();
//! let page = out.add_page(612.0, 792.0);
//! page.fill_rect(72.0, 72.0, 200.0, 100.0, [0.0, 0.48, 1.0]);
//! let pdf_bytes = out.finish();
//! ```
//!
//! Coordinates are PDF points with the origin at the bottom left. Colours are straight RGB in 0..=1.

use bifrost_render::cg::pdf::doc::Doc;
use bifrost_render::cg::pdf::draw;
use bifrost_render::cg::pdf::extract::{Extractor, Piece};
use bifrost_render::cg::pdf::outline_tree::{self, OutlineNode};
use bifrost_render::cg::pdf::page::{self, PageBox};
use bifrost_render::cg::path::{Element, Path};
use bifrost_render::cg::pdf::writer::{self, Page as OutPage, PageImage};
use bifrost_render::cg::raster::{FillRule, LineCap, LineJoin, StrokeStyle};
use bifrost_render::gfx::Image;

use crate::graphics_quartz_filter::QuartzFilter;

/// The PDF page context: a drawing context (paths, clipping, the transform, colours, alpha and blend modes, gradients
/// and shadings, images, standard-face text, links, named destinations, an outline and document information) whose
/// pages are written as PDF vector content. Use it to make a PDF from the same drawing calls a bitmap context takes;
/// [`PdfWriter`] stays for the simple page calls of the print pipeline.
///
/// ```ignore
/// let mut cx = graphics_pdf::PdfContext::new(None, graphics_pdf::PdfInfo { title: "Report".into(), ..Default::default() });
/// cx.begin_page(None);
/// cx.set_fill_rgb(0.0, 0.48, 1.0, 1.0);
/// cx.fill_rect(bifrost_render::cg::context::Rect::new(72.0, 72.0, 200.0, 100.0));
/// cx.set_url_for_rect("https://example.org/", bifrost_render::cg::context::Rect::new(72.0, 72.0, 200.0, 100.0));
/// let pdf_bytes = cx.close();
/// ```
pub use bifrost_render::cg::pdf::context::{PageBoxes, PdfContext, PdfInfo, PdfOutlineItem, StandardFace, DEFAULT_MEDIA};

/// One occurrence of a search string on a page (`PDFSelection`): the zero based page, the line of the page's text it
/// sits on, its character range in that line, the text as written in the document, and the line's baseline and font
/// size in page points (the vertical extent a highlight spans).
#[derive(Clone, Debug, PartialEq)]
pub struct Selection {
    pub page: usize,
    pub line: usize,
    /// First character of the match in the line.
    pub start: usize,
    /// One past the last character of the match.
    pub end: usize,
    pub text: String,
    pub baseline: f64,
    pub size: f64,
}

/// The characters of `s`, each lowercased to a single character so indexes line up with the original.
fn fold(s: &str) -> Vec<char> {
    s.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect()
}

/// The path as PDF path construction operators (each followed by a space); quadratic curves become cubic ones.
fn path_ops(path: &Path) -> String {
    let n = |v: f32| num(v as f64);
    let mut s = String::new();
    let mut cur = (0.0f32, 0.0f32);
    let mut start = cur;
    for e in path.elements() {
        match *e {
            Element::Move(p) => {
                s.push_str(&format!("{} {} m ", n(p.0), n(p.1)));
                cur = p;
                start = p;
            }
            Element::Line(p) => {
                s.push_str(&format!("{} {} l ", n(p.0), n(p.1)));
                cur = p;
            }
            Element::Quad(c, p) => {
                let c1 = (cur.0 + 2.0 / 3.0 * (c.0 - cur.0), cur.1 + 2.0 / 3.0 * (c.1 - cur.1));
                let c2 = (p.0 + 2.0 / 3.0 * (c.0 - p.0), p.1 + 2.0 / 3.0 * (c.1 - p.1));
                s.push_str(&format!("{} {} {} {} {} {} c ", n(c1.0), n(c1.1), n(c2.0), n(c2.1), n(p.0), n(p.1)));
                cur = p;
            }
            Element::Cubic(a, b, p) => {
                s.push_str(&format!("{} {} {} {} {} {} c ", n(a.0), n(a.1), n(b.0), n(b.1), n(p.0), n(p.1)));
                cur = p;
            }
            Element::Close => {
                s.push_str("h ");
                cur = start;
            }
        }
    }
    s
}

/// The graphics state operators of a stroke style: width, cap, join, miter limit and dash.
fn style_ops(st: &StrokeStyle) -> String {
    let cap = match st.cap {
        LineCap::Butt => 0,
        LineCap::Round => 1,
        LineCap::Square => 2,
    };
    let join = match st.join {
        LineJoin::Miter => 0,
        LineJoin::Round => 1,
        LineJoin::Bevel => 2,
    };
    let mut s = format!("{} w {cap} J {join} j {} M ", num(st.width.max(0.0) as f64), num(st.miter_limit.max(1.0) as f64));
    let dash_ok = !st.dash.is_empty() && st.dash.iter().all(|d| d.is_finite() && *d >= 0.0) && st.dash.iter().sum::<f32>() > 0.0;
    if dash_ok {
        let lens: Vec<String> = st.dash.iter().map(|d| num(*d as f64)).collect();
        s.push_str(&format!("[{}] {} d ", lens.join(" "), num(st.dash_phase as f64)));
    }
    s
}

/// One outline (bookmark) entry: its title, the zero based index of the page it opens when that page is in the
/// document, its level, and the entries nested under it.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineEntry {
    pub title: String,
    pub page: Option<usize>,
    pub level: u8,
    pub children: Vec<OutlineEntry>,
}

/// The nodes as entries, each destination's page object number mapped to a page index by `page_index`.
fn entries(nodes: &[OutlineNode], page_index: &dyn Fn(u32) -> Option<usize>) -> Vec<OutlineEntry> {
    nodes
        .iter()
        .map(|n| OutlineEntry {
            title: n.item.title.clone(),
            page: page_index(n.item.dest.page),
            level: n.item.level,
            children: entries(&n.children, page_index),
        })
        .collect()
}

/// A PDF file that has been read and checked for its structure.
pub struct Document {
    doc: Doc,
}

impl Document {
    /// Reads the PDF in `data`. Files that need a password are refused.
    pub fn open(data: Vec<u8>) -> Result<Document, String> {
        let doc = Doc::open(data)?;
        if doc.is_encrypted() {
            return Err("the PDF is encrypted".to_string());
        }
        Ok(Document { doc })
    }

    /// The number of pages.
    pub fn page_count(&self) -> usize {
        page::page_count(&self.doc)
    }

    /// The crop box size of page `index` (zero based) in points.
    pub fn page_size(&self, index: usize) -> Option<(f64, f64)> {
        let pages = page::pages(&self.doc);
        pages.get(index).map(|p| p.size(PageBox::Crop))
    }

    /// The outline (bookmarks) as a tree in reading order. An entry's `page` is the index of the page its
    /// destination names, or None when the destination is not a page of this document.
    pub fn outline(&self) -> Vec<OutlineEntry> {
        let pages = page::pages(&self.doc);
        let items = page::outline(&self.doc);
        let page_index = |num: u32| pages.iter().find(|p| p.num == Some(num)).map(|p| p.index);
        entries(&outline_tree::outline_tree(&items), &page_index)
    }

    /// The indexes of the pages whose text contains `needle`, ignoring case and surrounding spaces. A match must lie
    /// within one line of the page's text, so a phrase broken across lines is not found. An empty needle matches nothing.
    pub fn find(&self, needle: &str) -> Vec<usize> {
        let needle = needle.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        page::pages(&self.doc)
            .iter()
            .filter(|page| {
                let mut ex = Extractor::new(&self.doc);
                ex.page(page);
                ex.pieces.iter().any(|p| matches!(p, Piece::Line { text, .. } if text.to_lowercase().contains(&needle)))
            })
            .map(|page| page.index)
            .collect()
    }

    /// Every occurrence of `needle` in the text of the pages (`PDFDocument.findString`), in page and line order,
    /// ignoring case. A match lies within one line and matches do not overlap. An empty needle finds nothing.
    pub fn search(&self, needle: &str) -> Vec<Selection> {
        let want = fold(needle.trim());
        if want.is_empty() {
            return Vec::new();
        }
        let mut found = Vec::new();
        for page in page::pages(&self.doc).iter() {
            let mut ex = Extractor::new(&self.doc);
            ex.page(page);
            let mut line_no = 0;
            for piece in &ex.pieces {
                let Piece::Line { y, size, text } = piece else { continue };
                let chars: Vec<char> = text.chars().collect();
                let folded = fold(text);
                let mut at = 0;
                while at + want.len() <= folded.len() {
                    if folded[at..at + want.len()] == want[..] {
                        found.push(Selection {
                            page: page.index,
                            line: line_no,
                            start: at,
                            end: at + want.len(),
                            text: chars[at..at + want.len()].iter().collect(),
                            baseline: *y as f64,
                            size: *size as f64,
                        });
                        at += want.len();
                    } else {
                        at += 1;
                    }
                }
                line_no += 1;
            }
        }
        found
    }

    /// The text of the selections, one per line, in the order given.
    pub fn selection_text(selections: &[Selection]) -> String {
        selections.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join("\n")
    }

    /// The text of page `index` (zero based): its lines in drawing order, joined with `\n`. None when the page does not
    /// exist. This is the page's string without any selection or layout.
    pub fn page_text(&self, index: usize) -> Option<String> {
        let pages = page::pages(&self.doc);
        let page = pages.get(index)?;
        let mut ex = Extractor::new(&self.doc);
        ex.page(page);
        let lines: Vec<&str> = ex
            .pieces
            .iter()
            .filter_map(|p| match p {
                Piece::Line { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        Some(lines.join("\n"))
    }

    /// Page `index` as a picture scaled to fit `max_w` x `max_h` pixels, keeping its aspect.
    pub fn render(&self, index: usize, max_w: usize, max_h: usize) -> Result<Image, String> {
        let pages = page::pages(&self.doc);
        let page = pages.get(index).ok_or("the PDF has no such page")?;
        draw::render_page_fit(&self.doc, page, max_w, max_h).ok_or_else(|| "the page could not be drawn".to_string())
    }
}

/// The standard faces a page can draw text in. Every page of a written file lists all four, so the index of a face
/// is its resource name: `/F0` for Helvetica, up to `/F3` for Courier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    Helvetica = 0,
    HelveticaBold = 1,
    Times = 2,
    Courier = 3,
}

/// The standard font names, in the order of [`Face`].
const FACES: [&str; 4] = ["Helvetica", "Helvetica-Bold", "Times-Roman", "Courier"];

/// One page being drawn. Operations append to its content stream in call order.
pub struct PdfPage {
    width: f64,
    height: f64,
    content: String,
    images: Vec<PageImage>,
}

impl PdfPage {
    /// Draws `text` in `face` at `size` points with its baseline at (`x`, `y`). The text is written as a hex string of
    /// WinAnsi bytes, so characters outside WinAnsi come out as `?`.
    pub fn draw_text(&mut self, text: &str, x: f64, y: f64, size: f64, face: Face) {
        let hex: String = text.chars().map(|c| format!("{:02X}", writer::winansi(c))).collect();
        self.content.push_str(&format!(
            "BT /F{} {} Tf {} {} Td <{hex}> Tj ET\n",
            face as usize,
            num(size.max(0.0)),
            num(x),
            num(y)
        ));
    }

    /// Fills the rectangle at (`x`, `y`) of size `w` x `h`.
    pub fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64, rgb: [f64; 3]) {
        let [r, g, b] = rgb.map(clamp01);
        self.content.push_str(&format!("{r:.4} {g:.4} {b:.4} rg {} {} {} {} re f\n", num(x), num(y), num(w), num(h)));
    }

    /// Strokes a straight line from (`x0`, `y0`) to (`x1`, `y1`) with `width` points.
    pub fn stroke_line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, width: f64, rgb: [f64; 3]) {
        let [r, g, b] = rgb.map(clamp01);
        self.content.push_str(&format!(
            "{r:.4} {g:.4} {b:.4} RG {} w {} {} m {} {} l S\n",
            num(width.max(0.0)),
            num(x0),
            num(y0),
            num(x1),
            num(y1)
        ));
    }

    /// Fills `path` (PDF points, origin at the bottom left; lines and curves) with `rgb` under `rule`.
    pub fn fill_path(&mut self, path: &Path, rgb: [f64; 3], rule: FillRule) {
        let [r, g, b] = rgb.map(clamp01);
        let op = if rule == FillRule::EvenOdd { "f*" } else { "f" };
        self.content.push_str(&format!("q {r:.4} {g:.4} {b:.4} rg {}{op} Q\n", path_ops(path)));
    }

    /// Strokes `path` with `rgb` and the line width, caps, joins, miter limit and dash of `style`.
    pub fn stroke_path(&mut self, path: &Path, style: &StrokeStyle, rgb: [f64; 3]) {
        let [r, g, b] = rgb.map(clamp01);
        self.content.push_str(&format!("q {r:.4} {g:.4} {b:.4} RG {}{}S Q\n", style_ops(style), path_ops(path)));
    }

    /// Fills `path` with `fill` and then strokes its outline with `stroke` in one paint operation.
    pub fn fill_stroke_path(&mut self, path: &Path, fill: [f64; 3], stroke: [f64; 3], style: &StrokeStyle, rule: FillRule) {
        let [fr, fg, fb] = fill.map(clamp01);
        let [sr, sg, sb] = stroke.map(clamp01);
        let op = if rule == FillRule::EvenOdd { "B*" } else { "B" };
        self.content.push_str(&format!(
            "q {fr:.4} {fg:.4} {fb:.4} rg {sr:.4} {sg:.4} {sb:.4} RG {}{}{op} Q\n",
            style_ops(style),
            path_ops(path)
        ));
    }

    /// Draws a raster image of `w` x `h` pixels (three bytes each, rows top to bottom) into the rectangle at (`x`, `y`)
    /// of size `dw` x `dh`. The image is ignored when its byte count does not match its size.
    pub fn draw_rgb_image(&mut self, rgb: Vec<u8>, w: usize, h: usize, x: f64, y: f64, dw: f64, dh: f64) {
        if w == 0 || h == 0 || rgb.len() != w * h * 3 {
            return;
        }
        let name = self.images.len();
        self.images.push(PageImage { width: w, height: h, rgb });
        self.content.push_str(&format!(
            "q {} 0 0 {} {} {} cm /Im{name} Do Q\n",
            num(dw),
            num(dh),
            num(x),
            num(y)
        ));
    }
}

/// Collects drawn pages and writes them as one PDF file.
#[derive(Default)]
pub struct PdfWriter {
    pages: Vec<PdfPage>,
}

impl PdfWriter {
    pub fn new() -> PdfWriter {
        PdfWriter::default()
    }

    /// Starts a page of `width` x `height` points and returns it for drawing.
    pub fn add_page(&mut self, width: f64, height: f64) -> &mut PdfPage {
        self.pages.push(PdfPage { width, height, content: String::new(), images: Vec::new() });
        self.pages.last_mut().expect("a page was just pushed")
    }

    /// The number of pages added so far.
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Writes the document. A writer with no pages gives a valid file with zero pages.
    pub fn finish(self) -> Vec<u8> {
        let out: Vec<OutPage> = self
            .pages
            .into_iter()
            .map(|p| OutPage {
                width: p.width,
                height: p.height,
                content: p.content.into_bytes(),
                images: p.images,
            })
            .collect();
        writer::write(&out, &FACES)
    }

    /// Writes the document through a [`QuartzFilter`]: fill and stroke colours and raster images are colour-mapped,
    /// and images are scaled down to the filter's size limit. The page content that is not a colour stays as drawn.
    pub fn finish_filtered(self, filter: &QuartzFilter) -> Vec<u8> {
        let out: Vec<OutPage> = self
            .pages
            .into_iter()
            .map(|p| {
                let images = p
                    .images
                    .into_iter()
                    .map(|im| {
                        let (rgb, width, height) = filter.filter_image(&im.rgb, im.width, im.height);
                        PageImage { width, height, rgb }
                    })
                    .collect();
                OutPage { width: p.width, height: p.height, content: filter.filter_content(&p.content).into_bytes(), images }
            })
            .collect();
        writer::write(&out, &FACES)
    }
}

fn clamp01(v: f64) -> f64 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(0.0, 1.0)
    }
}

fn num(v: f64) -> String {
    if v.is_finite() {
        format!("{v:.3}")
    } else {
        "0".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_readable_document_with_the_page_count_and_size() {
        let mut w = PdfWriter::new();
        w.add_page(200.0, 100.0).fill_rect(10.0, 10.0, 50.0, 20.0, [1.0, 0.0, 0.0]);
        w.add_page(300.0, 400.0).stroke_line(0.0, 0.0, 300.0, 400.0, 2.0, [0.0, 0.0, 0.0]);
        let bytes = w.finish();
        assert!(bytes.starts_with(b"%PDF"));
        let doc = Document::open(bytes).expect("the writer output reads back");
        assert_eq!(doc.page_count(), 2);
        assert_eq!(doc.page_size(0), Some((200.0, 100.0)));
        assert_eq!(doc.page_size(1), Some((300.0, 400.0)));
        assert_eq!(doc.page_size(2), None);
    }

    #[test]
    fn renders_a_page_to_a_picture_that_fits_the_box() {
        let mut w = PdfWriter::new();
        w.add_page(200.0, 100.0).fill_rect(0.0, 0.0, 200.0, 100.0, [0.0, 0.0, 1.0]);
        let doc = Document::open(w.finish()).expect("reads back");
        let img = doc.render(0, 100, 100).expect("renders");
        assert_eq!((img.w, img.h), (100, 50));
        assert!(doc.render(1, 100, 100).is_err());
    }

    #[test]
    fn an_image_with_the_wrong_byte_count_is_not_drawn() {
        let mut w = PdfWriter::new();
        w.add_page(100.0, 100.0).draw_rgb_image(vec![0; 5], 2, 2, 0.0, 0.0, 10.0, 10.0);
        assert_eq!(w.pages[0].images.len(), 0);
        w.add_page(100.0, 100.0).draw_rgb_image(vec![255; 12], 2, 2, 0.0, 0.0, 10.0, 10.0);
        assert_eq!(w.pages[1].images.len(), 1);
    }

    #[test]
    fn text_is_drawn_as_hex_in_a_standard_face() {
        let mut w = PdfWriter::new();
        w.add_page(200.0, 100.0).draw_text("Hi\u{20ac}", 10.0, 20.0, 12.0, Face::Times);
        assert!(w.pages[0].content.contains("BT /F2 12.000 Tf 10.000 20.000 Td <486980> Tj ET"));
        let bytes = w.finish();
        assert!(bytes.windows(b"/BaseFont /Times-Roman".len()).any(|x| x == b"/BaseFont /Times-Roman"));
        assert_eq!(Document::open(bytes).expect("reads back").page_count(), 1);
    }

    #[test]
    fn find_names_the_pages_whose_text_has_the_phrase() {
        let mut w = PdfWriter::new();
        w.add_page(300.0, 200.0).draw_text("Hello World", 10.0, 150.0, 12.0, Face::Helvetica);
        w.add_page(300.0, 200.0).draw_text("Second page", 10.0, 150.0, 12.0, Face::Times);
        let doc = Document::open(w.finish()).expect("reads back");
        assert_eq!(doc.find("world"), vec![0]);
        assert_eq!(doc.find("  SECOND page "), vec![1]);
        assert!(doc.find("   ").is_empty());
        assert!(doc.find("missing").is_empty());
    }

    #[test]
    fn page_text_reads_back_the_text_of_one_page() {
        let mut w = PdfWriter::new();
        w.add_page(300.0, 200.0).draw_text("Hello World", 10.0, 150.0, 12.0, Face::Helvetica);
        w.add_page(300.0, 200.0).draw_text("Second page", 10.0, 150.0, 12.0, Face::Times);
        let doc = Document::open(w.finish()).expect("reads back");
        assert!(doc.page_text(0).expect("page 0").contains("Hello World"));
        assert!(doc.page_text(1).expect("page 1").contains("Second page"));
        assert!(!doc.page_text(1).expect("page 1").contains("Hello"));
        assert_eq!(doc.page_text(2), None);
    }

    #[test]
    fn outline_entries_nest_and_name_their_page_indexes() {
        use bifrost_render::cg::pdf::page::{Dest, OutlineItem};
        let item = |title: &str, page: u32, level: u8| OutlineItem {
            title: title.to_string(),
            dest: Dest { page, left: None, top: None, zoom: None },
            level,
        };
        let items = vec![item("Intro", 10, 0), item("Setup", 11, 1), item("Ending", 12, 0)];
        let tree = outline_tree::outline_tree(&items);
        let index = |num: u32| match num {
            10 => Some(0),
            11 => Some(1),
            _ => None,
        };
        let got = entries(&tree, &index);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].title, "Intro");
        assert_eq!(got[0].page, Some(0));
        assert_eq!(got[0].children.len(), 1);
        assert_eq!(got[0].children[0].title, "Setup");
        assert_eq!(got[0].children[0].page, Some(1));
        assert_eq!(got[0].children[0].level, 1);
        assert_eq!(got[1].page, None);
    }

    #[test]
    fn bytes_that_are_not_a_pdf_are_refused() {
        assert!(Document::open(b"plain text".to_vec()).is_err());
    }

    #[test]
    fn an_empty_writer_gives_a_file_with_no_pages() {
        let bytes = PdfWriter::new().finish();
        assert!(bytes.starts_with(b"%PDF"));
        assert_eq!(Document::open(bytes).expect("reads back").page_count(), 0);
    }

    fn pixel(img: &Image, x: usize, y: usize) -> (u32, u32, u32) {
        let p = img.px[y * img.w + x];
        (p & 0xff, (p >> 8) & 0xff, (p >> 16) & 0xff)
    }

    #[test]
    fn a_filled_path_with_lines_draws_where_it_says() {
        let mut path = Path::new();
        path.move_to((10.0, 10.0));
        path.line_to((90.0, 10.0));
        path.line_to((90.0, 90.0));
        path.line_to((10.0, 90.0));
        path.close();
        let mut w = PdfWriter::new();
        w.add_page(100.0, 100.0).fill_path(&path, [1.0, 0.0, 0.0], FillRule::NonZero);
        assert!(w.pages[0].content.contains("10.000 10.000 m 90.000 10.000 l 90.000 90.000 l 10.000 90.000 l h f Q"));
        let img = Document::open(w.finish()).expect("reads back").render(0, 100, 100).expect("renders");
        let (r, g, _) = pixel(&img, 50, 50);
        assert!(r > 200 && g < 60, "the middle is red: {r} {g}");
        let (r, g, b) = pixel(&img, 3, 3);
        assert!(!(r > 200 && g < 60 && b < 60), "outside the path is not red");
    }

    #[test]
    fn curves_and_the_even_odd_rule_reach_the_file() {
        let mut ring = Path::from_ellipse(bifrost_render::cg::path::Rect::new(10.0, 10.0, 80.0, 80.0));
        ring.add_ellipse(bifrost_render::cg::path::Rect::new(30.0, 30.0, 40.0, 40.0));
        let mut w = PdfWriter::new();
        w.add_page(100.0, 100.0).fill_path(&ring, [0.0, 0.0, 1.0], FillRule::EvenOdd);
        let content = w.pages[0].content.clone();
        assert!(content.contains(" c ") && content.contains("f* Q"), "{content}");
        let img = Document::open(w.finish()).expect("reads back").render(0, 100, 100).expect("renders");
        let (_, _, b_ring) = pixel(&img, 20, 50);
        let (r_hole, g_hole, b_hole) = pixel(&img, 50, 50);
        assert!(b_ring > 200, "the ring is blue");
        assert!(!(r_hole < 60 && g_hole < 60 && b_hole > 200), "the hole is not blue");
    }

    #[test]
    fn a_stroked_path_carries_its_style() {
        let mut path = Path::new();
        path.move_to((0.0, 50.0));
        path.line_to((100.0, 50.0));
        let style = StrokeStyle { width: 8.0, cap: LineCap::Round, join: LineJoin::Bevel, dash: vec![10.0, 5.0], dash_phase: 2.0, ..StrokeStyle::default() };
        let mut w = PdfWriter::new();
        w.add_page(100.0, 100.0).stroke_path(&path, &style, [0.0, 0.0, 0.0]);
        let content = w.pages[0].content.clone();
        assert!(content.contains("8.000 w 1 J 2 j 10.000 M [10.000 5.000] 2.000 d "), "{content}");
        assert!(content.ends_with("S Q\n"));
        assert_eq!(Document::open(w.finish()).expect("reads back").page_count(), 1);
    }

    #[test]
    fn fill_and_stroke_in_one_operation() {
        let mut w = PdfWriter::new();
        w.add_page(100.0, 100.0).fill_stroke_path(
            &Path::from_rect(bifrost_render::cg::path::Rect::new(20.0, 20.0, 60.0, 60.0)),
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0],
            &StrokeStyle::default(),
            FillRule::NonZero,
        );
        assert!(w.pages[0].content.contains("rg ") && w.pages[0].content.contains("RG ") && w.pages[0].content.contains("B Q"));
        let img = Document::open(w.finish()).expect("reads back").render(0, 100, 100).expect("renders");
        let (r, g, b) = pixel(&img, 50, 50);
        assert!(g > 200 && r < 60 && b < 60, "green inside: {r} {g} {b}");
    }

    #[test]
    fn a_black_and_white_filter_greys_a_coloured_page() {
        let mut w = PdfWriter::new();
        w.add_page(100.0, 100.0).fill_rect(0.0, 0.0, 100.0, 100.0, [1.0, 0.0, 0.0]);
        let img = Document::open(w.finish_filtered(&QuartzFilter::black_and_white())).expect("reads back").render(0, 100, 100).expect("renders");
        let (r, g, b) = pixel(&img, 50, 50);
        assert!((r as i32 - g as i32).abs() <= 3 && (g as i32 - b as i32).abs() <= 3, "grey: {r} {g} {b}");
        assert!((70..=82).contains(&r), "luma of red is 0.299: {r}");
    }

    #[test]
    fn a_reduce_size_filter_shrinks_images_in_the_file() {
        let filter = QuartzFilter { max_image_side: Some(4), ..QuartzFilter::identity("tiny") };
        let rgb: Vec<u8> = (0..16 * 16 * 3).map(|i| ((i * 37 + 11) % 256) as u8).collect();
        let mut plain = PdfWriter::new();
        plain.add_page(100.0, 100.0).draw_rgb_image(rgb.clone(), 16, 16, 0.0, 0.0, 50.0, 50.0);
        let mut small = PdfWriter::new();
        small.add_page(100.0, 100.0).draw_rgb_image(rgb, 16, 16, 0.0, 0.0, 50.0, 50.0);
        let (a, b) = (plain.finish(), small.finish_filtered(&filter));
        assert!(Document::open(b.clone()).is_ok());
        assert!(b.len() < a.len(), "{} < {}", b.len(), a.len());
    }

    #[test]
    fn search_returns_each_occurrence_with_its_line_and_range() {
        let mut w = PdfWriter::new();
        w.add_page(300.0, 200.0).draw_text("Hello World hello", 10.0, 150.0, 12.0, Face::Helvetica);
        w.add_page(300.0, 200.0).draw_text("Nothing here", 10.0, 150.0, 12.0, Face::Helvetica);
        w.add_page(300.0, 200.0).draw_text("say HELLO", 10.0, 150.0, 12.0, Face::Times);
        let doc = Document::open(w.finish()).expect("reads back");
        let hits = doc.search("hello");
        assert_eq!(hits.len(), 3, "{hits:?}");
        assert_eq!((hits[0].page, hits[0].line, hits[0].start, hits[0].end), (0, 0, 0, 5));
        assert_eq!(hits[0].text, "Hello");
        assert_eq!((hits[1].page, hits[1].start), (0, 12));
        assert_eq!(hits[1].text, "hello");
        assert_eq!((hits[2].page, hits[2].start, hits[2].text.as_str()), (2, 4, "HELLO"));
        assert!(hits.iter().all(|h| h.size > 0.0));
        assert_eq!(Document::selection_text(&hits), "Hello\nhello\nHELLO");
        assert!(doc.search("  ").is_empty() && doc.search("absent").is_empty());
    }
}
