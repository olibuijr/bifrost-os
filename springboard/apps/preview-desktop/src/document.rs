//! Opening documents. A picture (PNG or JPEG) is one page. A PDF is read by the shell's PDF engine,
//! which gives reflowed text and embedded pictures rather than rasterised pages, so the text is
//! cut into pages here and each picture becomes a page of its own. Markup lives in a sidecar file
//! under the app's data directory, keyed by the document's path.

use crate::model::{self, Adjust, Mark, TEXT_PAGE_CHARS};
use bifrost_app::image_filter::{Filter, FilterChain};
use bifrost_app::image_source::ImageSource;
use bifrost_app::kit::Image;
use springboard::shell::books::engine::{self, Block};
use std::path::{Path, PathBuf};

/// Thumbnails are at most this many pixels wide.
pub const THUMB_W: usize = 132;

/// What one page shows.
pub enum Body {
    Picture(Image),
    Text(String),
}

/// One page of a document, with its thumbnail when it has a picture.
pub struct Page {
    pub body: Body,
    pub thumb: Option<Image>,
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
}

impl Document {
    /// The number of pages.
    pub fn count(&self) -> usize {
        self.pages.len()
    }
}

/// Opens `path`. A PDF is read through the PDF engine (EPUB and other formats are refused here);
/// anything else must be a PNG or a JPEG. `data_dir` is the app's private storage directory.
pub fn open(path: &Path, data_dir: &Path) -> Result<Document, String> {
    let is_pdf = matches!(engine::format_of(path), Some(engine::Format::Pdf));
    let pages = if is_pdf { pdf_pages(path)? } else { picture_pages(path)? };
    if pages.is_empty() {
        return Err("The document has no pages.".into());
    }
    let marks = load_marks(data_dir, path);
    let title = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Document").to_string();
    Ok(Document { title, path: path.to_path_buf(), is_pdf, pages, marks })
}

fn picture_pages(path: &Path) -> Result<Vec<Page>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let source = ImageSource::new(&bytes).map_err(|e| format!("{e:?}"))?;
    let img = source.decode().map_err(|e| format!("{e:?}"))?;
    let thumb = engine::thumb(img.clone(), THUMB_W);
    Ok(vec![Page { body: Body::Picture(img), thumb: Some(thumb) }])
}

fn pdf_pages(path: &Path) -> Result<Vec<Page>, String> {
    let book = engine::load(path)?;
    let mut pages = Vec::new();
    let mut text: Vec<String> = Vec::new();
    for block in &book.blocks {
        match block {
            Block::Image(img) => {
                flush_text(&mut text, &mut pages);
                pages.push(Page { body: Body::Picture(img.clone()), thumb: Some(engine::thumb(img.clone(), THUMB_W)) });
            }
            other => {
                if let Some(t) = other.text() {
                    text.push(t.to_string());
                }
            }
        }
    }
    flush_text(&mut text, &mut pages);
    Ok(pages)
}

fn flush_text(text: &mut Vec<String>, pages: &mut Vec<Page>) {
    let cut = {
        let refs: Vec<&str> = text.iter().map(|s| s.as_str()).collect();
        model::chunk_text(&refs, TEXT_PAGE_CHARS)
    };
    text.clear();
    for body in cut {
        pages.push(Page { body: Body::Text(body), thumb: None });
    }
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
    let Some(Page { body: Body::Picture(img), .. }) = doc.pages.get(page) else {
        return Err("Only picture pages export to PNG.".into());
    };
    let mut out = model::rotate(&adjusted(img, adjust), quarters);
    model::burn(&mut out, &doc.marks, page, quarters);
    let bytes = springboard::pngenc::encode_rgb(out.w, out.h, &out.px);
    let stem = doc.path.file_stem().and_then(|s| s.to_str()).unwrap_or("export");
    let target = doc.path.with_file_name(format!("{stem}-markup.png"));
    std::fs::write(&target, bytes).map_err(|e| e.to_string())?;
    Ok(target)
}
