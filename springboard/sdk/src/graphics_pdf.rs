//! PDF documents for apps: open a file to read its pages (count, size, a rasterised picture of a page) and build a new
//! file from drawn pages. The reader is `bifrost_render::cg::pdf` (`Doc`, `page`, `draw`); the writer is its
//! `writer` module. Drawing is limited to what the writer emits: filled and stroked rectangles and lines, raster
//! images, and text in the four standard faces ([`Face`], WinAnsi only). Text extraction and annotation editing stay
//! in the PDF app.
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
use bifrost_render::cg::pdf::outline_tree::{self, OutlineNode};
use bifrost_render::cg::pdf::page::{self, PageBox};
use bifrost_render::cg::pdf::writer::{self, Page as OutPage, PageImage};
use bifrost_render::gfx::Image;

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
}
