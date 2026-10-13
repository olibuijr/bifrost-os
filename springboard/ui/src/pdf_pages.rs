//! Page navigation for a multi-page PDF preview, as a pure model (the page context a preview gets for a PDF document, and
//! the page size it is drawn at). The preview view asks this module which page is current, how to step between pages,
//! and what scale fits a page into the view. Drawing the pages is left to the renderer.

#[derive(Clone, Debug, PartialEq)]
pub enum PdfPreviewError {
    /// A document has at least one page.
    NoPages,
    /// Page size must be positive and finite.
    BadPageSize,
    /// The view size must be positive and finite.
    BadViewSize,
    /// The requested page is outside the document.
    PageOutOfRange(usize),
}

/// The page context: page count, the page size in points, and the current page (zero based).
#[derive(Clone, Debug, PartialEq)]
pub struct PdfPreview {
    page_count: usize,
    page_width: f64,
    page_height: f64,
    current: usize,
}

impl PdfPreview {
    /// Creates the context with the first page current.
    pub fn new(page_count: usize, page_width: f64, page_height: f64) -> Result<Self, PdfPreviewError> {
        if page_count == 0 {
            return Err(PdfPreviewError::NoPages);
        }
        if !valid_size(page_width) || !valid_size(page_height) {
            return Err(PdfPreviewError::BadPageSize);
        }
        Ok(Self { page_count, page_width, page_height, current: 0 })
    }

    pub fn page_count(&self) -> usize {
        self.page_count
    }

    pub fn current_page(&self) -> usize {
        self.current
    }

    /// The label a reader shows for the current page, one based, such as "3 of 12".
    pub fn page_label(&self) -> String {
        format!("{} of {}", self.current + 1, self.page_count)
    }

    pub fn goto(&mut self, page: usize) -> Result<(), PdfPreviewError> {
        if page >= self.page_count {
            return Err(PdfPreviewError::PageOutOfRange(page));
        }
        self.current = page;
        Ok(())
    }

    /// Moves to the next page. Returns false and stays put on the last page.
    pub fn next_page(&mut self) -> bool {
        if self.current + 1 < self.page_count {
            self.current += 1;
            true
        } else {
            false
        }
    }

    /// Moves to the previous page. Returns false and stays put on the first page.
    pub fn previous_page(&mut self) -> bool {
        if self.current > 0 {
            self.current -= 1;
            true
        } else {
            false
        }
    }

    /// The scale that fits one page inside a view of the given size, keeping the aspect ratio. When `allow_enlarge` is
    /// false the scale is capped at 1.0, so small pages are not enlarged.
    pub fn fit_scale(&self, view_width: f64, view_height: f64, allow_enlarge: bool) -> Result<f64, PdfPreviewError> {
        if !valid_size(view_width) || !valid_size(view_height) {
            return Err(PdfPreviewError::BadViewSize);
        }
        let scale = (view_width / self.page_width).min(view_height / self.page_height);
        Ok(if allow_enlarge { scale } else { scale.min(1.0) })
    }
}

fn valid_size(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_stop_at_both_ends() {
        let mut p = PdfPreview::new(3, 612.0, 792.0).unwrap();
        assert!(!p.previous_page());
        assert!(p.next_page());
        assert!(p.next_page());
        assert!(!p.next_page());
        assert_eq!(p.current_page(), 2);
        assert_eq!(p.page_label(), "3 of 3");
    }

    #[test]
    fn goto_checks_range() {
        let mut p = PdfPreview::new(5, 612.0, 792.0).unwrap();
        p.goto(4).unwrap();
        assert_eq!(p.current_page(), 4);
        assert_eq!(p.goto(5), Err(PdfPreviewError::PageOutOfRange(5)));
        assert_eq!(p.current_page(), 4);
    }

    #[test]
    fn empty_and_bad_sizes_are_refused() {
        assert_eq!(PdfPreview::new(0, 612.0, 792.0), Err(PdfPreviewError::NoPages));
        assert_eq!(PdfPreview::new(1, 0.0, 792.0), Err(PdfPreviewError::BadPageSize));
        assert_eq!(PdfPreview::new(1, 612.0, f64::NAN), Err(PdfPreviewError::BadPageSize));
    }

    #[test]
    fn fit_scale_keeps_aspect_and_respects_enlarge_flag() {
        let p = PdfPreview::new(1, 612.0, 792.0).unwrap();
        let fit = p.fit_scale(306.0, 1000.0, false).unwrap();
        assert!((fit - 0.5).abs() < 1e-9);
        let big = p.fit_scale(1224.0, 1584.0, true).unwrap();
        assert!((big - 2.0).abs() < 1e-9);
        assert!((p.fit_scale(1224.0, 1584.0, false).unwrap() - 1.0).abs() < 1e-9);
        assert_eq!(p.fit_scale(0.0, 100.0, true), Err(PdfPreviewError::BadViewSize));
    }
}
