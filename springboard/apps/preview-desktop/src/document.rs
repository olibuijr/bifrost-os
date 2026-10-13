//! Opening documents. A picture (PNG or JPEG) is one page. A PDF is drawn by the PDF engine in `cg::pdf`: each page
//! is a picture that is drawn when it is shown, so a long document opens at once. Markup lives in a sidecar file
//! under the app's data directory, keyed by the document's path.

use crate::model::{self, Adjust, Mark};
use bifrost_app::image_filter::{Filter, FilterChain};
use bifrost_app::image_source::ImageSource;
use bifrost_app::kit::Image;
use springboard::cg::pdf::doc::Doc;
use springboard::cg::pdf::draw;
use springboard::cg::pdf::page::{self, Page as PdfPage};
use springboard::shell::books::engine;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Thumbnails are at most this many pixels wide.
pub const THUMB_W: usize = 132;
/// A PDF page is drawn to fit this box (its aspect kept).
const PDF_PAGE_W: usize = 1000;
const PDF_PAGE_H: usize = 1400;
/// Drawn PDF pages kept in memory; the cache is emptied when it would grow past this.
const PDF_KEPT: usize = 6;

/// What one page shows.
pub enum Body {
    Picture(Image),
    /// A PDF page, drawn when it is shown. The size is the picture size before rotation.
    Pdf { w: usize, h: usize },
}

/// One page of a document, with its thumbnail.
pub struct Page {
    pub body: Body,
    pub thumb: Option<Image>,
}

/// A page's picture: a picture the document holds, or a PDF page drawn for this frame.
pub enum PageImage<'a> {
    Borrowed(&'a Image),
    Drawn(Rc<Image>),
}

impl Deref for PageImage<'_> {
    type Target = Image;

    fn deref(&self) -> &Image {
        match self {
            PageImage::Borrowed(img) => *img,
            PageImage::Drawn(img) => img.as_ref(),
        }
    }
}

/// An open document: its pages and the markup saved for it.
pub struct Document {
    /// Shown as the window title once the SDK can set one (apps have no window title API yet).
    #[allow(dead_code)]
    pub title: String,
    pub path: PathBuf,
    /// For the PDF-only commands (outline, page thumbnails) still to come.
    #[allow(dead_code)]
    pub is_pdf: bool,
    pub pages: Vec<Page>,
    pub marks: Vec<Mark>,
    /// The PDF being drawn page by page, for a PDF document.
    pdf: Option<PdfSource>,
}

impl Document {
    /// The number of pages.
    pub fn count(&self) -> usize {
        self.pages.len()
    }

    /// The size of page `page` in picture pixels, before rotation.
    pub fn page_size(&self, page: usize) -> Option<(usize, usize)> {
        match &self.pages.get(page)?.body {
            Body::Picture(img) => Some((img.w, img.h)),
            Body::Pdf { w, h } => Some((*w, *h)),
        }
    }

    /// The picture of page `page`. A PDF page is drawn now, or taken from the drawn pages kept for the frames before.
    pub fn picture(&self, page: usize) -> Option<PageImage<'_>> {
        match &self.pages.get(page)?.body {
            Body::Picture(img) => Some(PageImage::Borrowed(img)),
            Body::Pdf { .. } => self.pdf.as_ref()?.draw(page).map(PageImage::Drawn),
        }
    }
}

/// A PDF open for drawing: its parsed file, its pages in order and the pictures drawn recently.
struct PdfSource {
    doc: Doc,
    pages: Vec<PdfPage>,
    drawn: RefCell<HashMap<usize, Rc<Image>>>,
}

impl PdfSource {
    fn draw(&self, page: usize) -> Option<Rc<Image>> {
        if let Some(img) = self.drawn.borrow().get(&page).cloned() {
            return Some(img);
        }
        let img = Rc::new(draw::render_page_fit(&self.doc, self.pages.get(page)?, PDF_PAGE_W, PDF_PAGE_H)?);
        let mut drawn = self.drawn.borrow_mut();
        if drawn.len() >= PDF_KEPT {
            drawn.clear();
        }
        drawn.insert(page, img.clone());
        Some(img)
    }
}

/// Opens `path`. A PDF is drawn by the PDF engine (EPUB and other formats are refused here); anything else must be
/// a PNG or a JPEG. `data_dir` is the app's private storage directory.
pub fn open(path: &Path, data_dir: &Path) -> Result<Document, String> {
    let is_pdf = matches!(engine::format_of(path), Some(engine::Format::Pdf));
    let (pages, pdf) = if is_pdf { pdf_pages(path)? } else { (picture_pages(path)?, None) };
    if pages.is_empty() {
        return Err("The document has no pages.".into());
    }
    let marks = load_marks(data_dir, path);
    let title = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Document").to_string();
    Ok(Document { title, path: path.to_path_buf(), is_pdf, pages, marks, pdf })
}

fn picture_pages(path: &Path) -> Result<Vec<Page>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let source = ImageSource::new(&bytes).map_err(|e| format!("{e:?}"))?;
    let img = source.decode().map_err(|e| format!("{e:?}"))?;
    let thumb = engine::thumb(img.clone(), THUMB_W);
    Ok(vec![Page { body: Body::Picture(img), thumb: Some(thumb) }])
}

/// The pages of a PDF. Their thumbnails are drawn now; the pictures are drawn when a page is shown.
fn pdf_pages(path: &Path) -> Result<(Vec<Page>, Option<PdfSource>), String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let doc = Doc::open(data)?;
    let list = page::pages(&doc);
    let mut pages = Vec::with_capacity(list.len());
    for p in &list {
        let (w, h) = draw::fit_size(p, PDF_PAGE_W, PDF_PAGE_H).ok_or("A page has no size.")?;
        let thumb = draw::render_page_fit(&doc, p, THUMB_W, 2 * THUMB_W);
        pages.push(Page { body: Body::Pdf { w, h }, thumb });
    }
    Ok((pages, Some(PdfSource { doc, pages: list, drawn: RefCell::new(HashMap::new()) })))
}

/// The sidecar file for `path` under `data_dir`.
pub fn sidecar_path(data_dir: &Path, path: &Path) -> PathBuf {
    data_dir.join("markup").join(model::sidecar_name(&path.to_string_lossy()))
}

/// The markup saved for `path`. A missing or unreadable file gives no marks.
pub fn load_marks(data_dir: &Path, path: &Path) -> Vec<Mark> {
    std::fs::read_to_string(sidecar_path(data_dir, path)).map(|t| model::parse_sidecar(&t)).unwrap_or_default()
}

/// Saves the markup for `path`.
pub fn save_marks(data_dir: &Path, path: &Path, marks: &[Mark]) -> std::io::Result<()> {
    let file = sidecar_path(data_dir, path);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, model::to_sidecar(marks))
}

/// `img` with the colour adjustments applied. Returns a copy unchanged when nothing is set.
pub fn adjusted(img: &Image, adjust: &Adjust) -> Image {
    let mut chain = FilterChain::new();
    for (name, value) in adjust.settings() {
        if let Some(mut filter) = Filter::named(name) {
            if let Some(key) = filter.input_keys().first().copied() {
                filter.set(key, value);
            }
            chain.push(filter);
        }
    }
    if chain.is_empty() {
        return img.clone();
    }
    chain.apply(img).unwrap_or_else(|| img.clone())
}

/// Writes the picture of page `page`, rotated, adjusted and with its markup burned in, as a PNG
/// next to the document (`<name>-markup.png`). Returns the file written.
pub fn export_png(doc: &Document, page: usize, quarters: u8, adjust: &Adjust) -> Result<PathBuf, String> {
    let img = doc.picture(page).ok_or("This page has no picture to export.")?;
    let mut out = model::rotate(&adjusted(&img, adjust), quarters);
    model::burn(&mut out, &doc.marks, page, quarters);
    let bytes = springboard::pngenc::encode_rgb(out.w, out.h, &out.px);
    let stem = doc.path.file_stem().and_then(|s| s.to_str()).unwrap_or("export");
    let target = doc.path.with_file_name(format!("{stem}-markup.png"));
    std::fs::write(&target, bytes).map_err(|e| e.to_string())?;
    Ok(target)
}
