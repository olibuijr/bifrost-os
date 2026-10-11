//! The Books reader engine: format loaders (PDF, EPUB) that flow a file into
//! one `Book` page model, and the small codecs they need (DEFLATE, ZIP).
//! Nothing here touches the UI.

pub mod epub;
pub mod inflate;
pub mod markup;
pub mod model;
pub mod pdf;
pub mod xhtml;
pub mod zip;

pub use model::{Block, Book, Hit, TocEntry};

use crate::gfx::Image;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Pdf,
    Epub,
}

/// The format a file name suggests.
pub fn format_of(path: &Path) -> Option<Format> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "pdf" => Some(Format::Pdf),
        "epub" => Some(Format::Epub),
        _ => None,
    }
}

/// True for names the Books app opens.
pub fn is_book(path: &Path) -> bool {
    format_of(path).is_some()
}

/// Reads and flows a whole book.
pub fn load(path: &Path) -> Result<Book, String> {
    let format = format_of(path).ok_or("not a PDF or EPUB file")?;
    let data = std::fs::read(path).map_err(|e| format!("cannot read the file: {e}"))?;
    let mut book = match format {
        Format::Pdf => pdf::load(data)?,
        Format::Epub => epub::load(data)?,
    };
    if book.title.is_empty() {
        book.title = file_title(path);
    }
    Ok(book)
}

/// What the library shows for a book without flowing it.
pub struct Meta {
    pub title: String,
    pub author: String,
    pub cover: Option<Image>,
}

pub fn probe(path: &Path) -> Meta {
    let fallback = Meta { title: file_title(path), author: String::new(), cover: None };
    let Some(format) = format_of(path) else { return fallback };
    let Ok(data) = std::fs::read(path) else { return fallback };
    match format {
        Format::Pdf => match pdf::probe(data) {
            Some((t, a)) => Meta { title: if t.is_empty() { fallback.title } else { t }, author: a, cover: None },
            None => fallback,
        },
        Format::Epub => match epub::probe(data) {
            Some(m) => Meta { title: if m.title.is_empty() { fallback.title } else { m.title }, author: m.author, cover: m.cover.map(|c| thumb(c, COVER_MAX_W)) },
            None => fallback,
        },
    }
}

/// Library covers are kept small: dozens of full-size ones would eat too much RAM.
pub const COVER_MAX_W: usize = 420;

/// Scales `img` down to at most `max_w` wide, keeping its aspect.
pub fn thumb(img: Image, max_w: usize) -> Image {
    if img.w <= max_w || img.w == 0 {
        return img;
    }
    let h = (img.h * max_w / img.w).max(1);
    img.resized(max_w, h)
}

/// A readable title from a file name: `the_hobbit-v2.epub` -> `the hobbit v2`.
pub fn file_title(path: &Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Untitled");
    let t: String = stem.chars().map(|c| if c == '_' || c == '-' { ' ' } else { c }).collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() {
        "Untitled".into()
    } else {
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_titles() {
        assert_eq!(format_of(Path::new("/a/B.PDF")), Some(Format::Pdf));
        assert_eq!(format_of(Path::new("x.epub")), Some(Format::Epub));
        assert_eq!(format_of(Path::new("x.txt")), None);
        assert!(!is_book(Path::new("noext")));
        assert_eq!(file_title(Path::new("/b/the_hobbit-v2.epub")), "the hobbit v2");
        assert_eq!(file_title(Path::new("/")), "Untitled");
    }

    #[test]
    fn thumbs_keep_aspect_and_never_grow() {
        let t = thumb(Image::new(840, 1260), 420);
        assert_eq!((t.w, t.h), (420, 630));
        let small = thumb(Image::new(100, 50), 420);
        assert_eq!((small.w, small.h), (100, 50));
    }

    #[test]
    fn load_reports_unreadable_files() {
        assert!(load(Path::new("/nonexistent/book.epub")).is_err());
        assert!(load(Path::new("/nonexistent/book.doc")).is_err());
    }
}
