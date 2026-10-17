//! PDF thumbnails: the first page, drawn by the shared PDF renderer (`bifrost_render::cg::pdf::draw::render_file_page`),
//! fitted into the square the request asks for. A file the renderer cannot read gets its type icon.

use super::images::pixels_of;
use bifrost_ui::thumb_service::{SourceFile, ThumbArt, ThumbProvider};

/// Largest PDF read for a thumbnail.
pub const PDF_MAX: u64 = 64 << 20;

/// The first page of a PDF held in `bytes`, fitted into a square of `px` pixels.
pub fn first_page(bytes: Vec<u8>, px: u32) -> Option<ThumbArt> {
    let side = px.max(1) as usize;
    let page = bifrost_render::cg::pdf::draw::render_file_page(bytes, 0, side, side).ok()?;
    pixels_of(&page).map(ThumbArt::Pixels)
}

/// PDF documents: the picture of their first page.
pub struct PdfProvider;

impl ThumbProvider for PdfProvider {
    fn name(&self) -> &'static str {
        "pdf"
    }
    fn types(&self) -> &'static [&'static str] {
        &["com.adobe.pdf"]
    }
    fn render(&self, file: &SourceFile, px: u32) -> Option<ThumbArt> {
        if file.stamp.size > PDF_MAX {
            return None;
        }
        let bytes = std::fs::read(&file.path).ok()?;
        first_page(bytes, px)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-page PDF, 200 by 100 points, filled blue. The xref is left out on purpose: the reader scans for objects
    /// when the table is missing.
    fn one_blue_page() -> Vec<u8> {
        let body = "%PDF-1.4\n\
            1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
            2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
            3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R >> endobj\n\
            4 0 obj << /Length 26 >> stream\n\
            0 0 1 rg 0 0 200 100 re f\n\
            endstream endobj\n\
            trailer << /Root 1 0 R >>\n%%EOF\n";
        body.as_bytes().to_vec()
    }

    #[test]
    fn the_first_page_is_drawn_inside_the_requested_square() {
        assert!(matches!(first_page(one_blue_page(), 64), Some(ThumbArt::Pixels(_))));
    }

    #[test]
    fn bytes_that_are_not_a_pdf_fall_back_to_the_icon() {
        assert!(first_page(b"not a pdf at all".to_vec(), 64).is_none());
    }

    #[test]
    fn the_provider_is_for_pdf_documents() {
        assert_eq!(PdfProvider.types(), &["com.adobe.pdf"]);
        assert_eq!(PdfProvider.name(), "pdf");
    }
}
