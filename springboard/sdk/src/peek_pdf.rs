//! Multi-page PDF previews: the layout an app gets when it previews a PDF with page thumbnails beside the content.
//!
//! [`PdfPreview`] wraps the page model in `bifrost_ui::pdfview` (the same model Books uses) and adds the three preview
//! styles of the platform's PDF preview context: the standard style, and the style with a column of page thumbnails on
//! the left or on the right of the content. The thumbnail column is laid out here and the content area is what is left
//! of the viewport, so a view draws [`PdfPreview::sidebar_frames`] and [`PdfPreview::content_view`] and nothing here
//! draws. Pure model with host unit tests.
//!
//! ```ignore
//! let mut pdf = PdfPreview::new(PdfStyle::ThumbnailsLeft, 900.0, 640.0);
//! pdf.set_pages(&[(612.0, 792.0), (612.0, 792.0)])?;
//! pdf.go_to_page(1)?;
//! ```

use bifrost_ui::pdfview::{DisplayMode, PageFrame, PageRect, PdfView, ViewError};

/// Width of the thumbnail column, in points.
pub const THUMBNAIL_COLUMN_WIDTH: f64 = 120.0;
/// Gap between thumbnails in the column, in points.
pub const THUMBNAIL_GAP: f64 = 8.0;
/// Largest page count a preview accepts. A longer document is refused, not truncated.
pub const MAX_PAGES: usize = 10_000;

/// The appearance of a PDF preview.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PdfStyle {
    /// The content alone, in the system's standard look.
    #[default]
    Standard,
    /// A column of page thumbnails on the left of the content.
    ThumbnailsLeft,
    /// A column of page thumbnails on the right of the content.
    ThumbnailsRight,
}

/// Which edge of the viewport the thumbnail column sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl PdfStyle {
    /// The side of the thumbnail column, or None for the standard style.
    pub fn thumbnail_side(self) -> Option<Side> {
        match self {
            PdfStyle::Standard => None,
            PdfStyle::ThumbnailsLeft => Some(Side::Left),
            PdfStyle::ThumbnailsRight => Some(Side::Right),
        }
    }

    /// Width the thumbnail column takes from the viewport (zero for the standard style).
    pub fn column_width(self) -> f64 {
        if self.thumbnail_side().is_some() {
            THUMBNAIL_COLUMN_WIDTH
        } else {
            0.0
        }
    }
}

/// A PDF preview: a document, its style and the page the reader is on.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfPreview {
    style: PdfStyle,
    view: PdfView,
    /// Each page's size in points, in document order.
    sizes: Vec<(f64, f64)>,
    view_w: f64,
    view_h: f64,
}

/// Why a PDF preview refused a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PdfError {
    /// A page size is zero, negative or not finite.
    BadPageSize,
    /// The document has more pages than [`MAX_PAGES`].
    TooManyPages,
    /// The page is past the last page.
    NoSuchPage,
}

impl From<ViewError> for PdfError {
    fn from(e: ViewError) -> Self {
        match e {
            ViewError::NoSuchPage => PdfError::NoSuchPage,
            ViewError::BadSize | ViewError::BadScale => PdfError::BadPageSize,
        }
    }
}

impl PdfPreview {
    /// An empty preview of a viewport `view_w` by `view_h` points, in continuous single-page mode.
    pub fn new(style: PdfStyle, view_w: f64, view_h: f64) -> Self {
        let view_w = view_w.max(0.0);
        let view_h = view_h.max(0.0);
        let mut view = PdfView::new(Self::content_width_for(style, view_w), view_h);
        view.set_display_mode(DisplayMode::SinglePageContinuous);
        PdfPreview { style, view, sizes: Vec::new(), view_w, view_h }
    }

    pub fn style(&self) -> PdfStyle {
        self.style
    }

    /// Changes the style. The current page stays the same.
    pub fn set_style(&mut self, style: PdfStyle) {
        self.style = style;
        self.view.set_view_size(Self::content_width_for(style, self.view_w), self.view_h);
    }

    /// Resizes the viewport.
    pub fn set_view_size(&mut self, view_w: f64, view_h: f64) {
        self.view_w = view_w.max(0.0);
        self.view_h = view_h.max(0.0);
        self.view.set_view_size(Self::content_width_for(self.style, self.view_w), self.view_h);
    }

    /// Replaces the pages. Each entry is a page's size in points. The preview returns to the first page.
    pub fn set_pages(&mut self, sizes: &[(f64, f64)]) -> Result<(), PdfError> {
        if sizes.len() > MAX_PAGES {
            return Err(PdfError::TooManyPages);
        }
        self.view.set_pages(sizes)?;
        self.sizes = sizes.to_vec();
        Ok(())
    }

    pub fn page_count(&self) -> usize {
        self.view.page_count()
    }

    /// The page the reader is on, or None for an empty document.
    pub fn current_page(&self) -> Option<usize> {
        self.view.current_page()
    }

    pub fn go_to_page(&mut self, page: usize) -> Result<(), PdfError> {
        self.view.go_to_page(page)?;
        Ok(())
    }

    /// Moves to the next page. Returns false at the last page.
    pub fn next_page(&mut self) -> bool {
        self.view.next_page()
    }

    /// Moves to the previous page. Returns false at the first page.
    pub fn previous_page(&mut self) -> bool {
        self.view.previous_page()
    }

    /// The area the content takes: the viewport less the thumbnail column, starting at the column's edge for the left
    /// side, at x zero otherwise.
    pub fn content_rect(&self) -> PageRect {
        let column = self.style.column_width();
        let x = if self.style.thumbnail_side() == Some(Side::Left) { column } else { 0.0 };
        PageRect { x, y: 0.0, w: Self::content_width_for(self.style, self.view_w), h: self.view_h }
    }

    /// The content view: the page model laid out in the content area.
    pub fn content_view(&self) -> &PdfView {
        &self.view
    }

    /// Frames of the thumbnails in the column, top to bottom, in viewport coordinates. Each thumbnail keeps its page's
    /// shape and fits the column width. Empty for the standard style.
    pub fn sidebar_frames(&self) -> Vec<PageFrame> {
        let Some(side) = self.style.thumbnail_side() else {
            return Vec::new();
        };
        let inner = THUMBNAIL_COLUMN_WIDTH - 2.0 * THUMBNAIL_GAP;
        let x = match side {
            Side::Left => THUMBNAIL_GAP,
            Side::Right => self.view_w - THUMBNAIL_COLUMN_WIDTH + THUMBNAIL_GAP,
        };
        let mut y = THUMBNAIL_GAP;
        let mut frames = Vec::with_capacity(self.sizes.len());
        for (index, &(w, h)) in self.sizes.iter().enumerate() {
            let scale = inner / w;
            let rect = PageRect { x, y, w: inner, h: h * scale };
            y += rect.h + THUMBNAIL_GAP;
            frames.push(PageFrame { index, rect });
        }
        frames
    }

    /// The thumbnail of the current page, if any, so a view can outline it.
    pub fn current_thumbnail(&self) -> Option<PageRect> {
        let current = self.current_page()?;
        self.sidebar_frames().into_iter().find(|f| f.index == current).map(|f| f.rect)
    }

    fn content_width_for(style: PdfStyle, view_w: f64) -> f64 {
        (view_w - style.column_width()).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letter() -> Vec<(f64, f64)> {
        vec![(612.0, 792.0), (612.0, 792.0), (792.0, 612.0)]
    }

    #[test]
    fn the_standard_style_has_no_thumbnail_column() {
        let mut p = PdfPreview::new(PdfStyle::Standard, 800.0, 600.0);
        p.set_pages(&letter()).unwrap();
        assert_eq!(p.style().thumbnail_side(), None);
        assert_eq!(p.style().column_width(), 0.0);
        assert!(p.sidebar_frames().is_empty());
        assert_eq!(p.content_rect(), PageRect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 });
    }

    #[test]
    fn the_thumbnail_column_takes_its_width_from_the_content() {
        let left = PdfPreview::new(PdfStyle::ThumbnailsLeft, 800.0, 600.0);
        assert_eq!(left.content_rect(), PageRect { x: THUMBNAIL_COLUMN_WIDTH, y: 0.0, w: 800.0 - THUMBNAIL_COLUMN_WIDTH, h: 600.0 });
        let right = PdfPreview::new(PdfStyle::ThumbnailsRight, 800.0, 600.0);
        assert_eq!(right.content_rect(), PageRect { x: 0.0, y: 0.0, w: 800.0 - THUMBNAIL_COLUMN_WIDTH, h: 600.0 });
        assert_eq!(right.content_view().scale_factor(), 1.0);
    }

    #[test]
    fn a_narrow_viewport_keeps_a_zero_width_content_area() {
        let p = PdfPreview::new(PdfStyle::ThumbnailsLeft, 50.0, 300.0);
        assert_eq!(p.content_rect().w, 0.0);
    }

    #[test]
    fn thumbnails_keep_each_page_shape_and_stack_top_down() {
        let mut p = PdfPreview::new(PdfStyle::ThumbnailsLeft, 800.0, 600.0);
        p.set_pages(&letter()).unwrap();
        let frames = p.sidebar_frames();
        assert_eq!(frames.len(), 3);
        let inner = THUMBNAIL_COLUMN_WIDTH - 2.0 * THUMBNAIL_GAP;
        let first = frames[0].rect;
        assert!((first.w - inner).abs() < 1e-9);
        assert!((first.h - inner * 792.0 / 612.0).abs() < 1e-9);
        assert_eq!(first.x, THUMBNAIL_GAP);
        assert_eq!(first.y, THUMBNAIL_GAP);
        assert!(frames[1].rect.y > first.y + first.h);
        let landscape = frames[2].rect;
        assert!((landscape.h - inner * 612.0 / 792.0).abs() < 1e-9);
    }

    #[test]
    fn right_side_thumbnails_sit_at_the_right_edge() {
        let mut p = PdfPreview::new(PdfStyle::ThumbnailsRight, 800.0, 600.0);
        p.set_pages(&letter()).unwrap();
        let f = p.sidebar_frames();
        let edge = f[0].rect.x + f[0].rect.w;
        assert!((edge - (800.0 - THUMBNAIL_GAP)).abs() < 1e-9);
    }

    #[test]
    fn navigation_follows_the_page_model_and_is_clamped_at_the_ends() {
        let mut p = PdfPreview::new(PdfStyle::ThumbnailsLeft, 800.0, 600.0);
        assert_eq!(p.current_page(), None);
        p.set_pages(&letter()).unwrap();
        assert_eq!(p.current_page(), Some(0));
        assert!(p.next_page());
        assert_eq!(p.current_page(), Some(1));
        assert_eq!(p.go_to_page(7), Err(PdfError::NoSuchPage));
        assert_eq!(p.current_page(), Some(1));
        assert!(p.previous_page());
        assert_eq!(p.current_page(), Some(0));
        assert!(!p.previous_page());
        p.go_to_page(2).unwrap();
        assert!(!p.next_page());
        assert_eq!(p.current_page(), Some(2));
    }

    #[test]
    fn the_current_thumbnail_is_the_one_for_the_current_page() {
        let mut p = PdfPreview::new(PdfStyle::ThumbnailsLeft, 800.0, 600.0);
        p.set_pages(&letter()).unwrap();
        p.go_to_page(1).unwrap();
        assert_eq!(p.current_thumbnail(), Some(p.sidebar_frames()[1].rect));
        let standard = PdfPreview::new(PdfStyle::Standard, 800.0, 600.0);
        assert_eq!(standard.current_thumbnail(), None);
    }

    #[test]
    fn changing_the_style_moves_the_content_area_and_keeps_the_page() {
        let mut p = PdfPreview::new(PdfStyle::Standard, 800.0, 600.0);
        p.set_pages(&letter()).unwrap();
        p.go_to_page(1).unwrap();
        p.set_style(PdfStyle::ThumbnailsLeft);
        assert_eq!(p.content_rect().w, 800.0 - THUMBNAIL_COLUMN_WIDTH);
        assert_eq!(p.current_page(), Some(1));
        p.set_style(PdfStyle::Standard);
        assert_eq!(p.content_rect().w, 800.0);
    }

    #[test]
    fn resizing_updates_the_column_and_the_content() {
        let mut p = PdfPreview::new(PdfStyle::ThumbnailsRight, 800.0, 600.0);
        p.set_pages(&letter()).unwrap();
        p.set_view_size(1000.0, 700.0);
        assert_eq!(p.content_rect().w, 1000.0 - THUMBNAIL_COLUMN_WIDTH);
        assert_eq!(p.content_rect().h, 700.0);
        let f = p.sidebar_frames();
        assert!((f[0].rect.x + f[0].rect.w - (1000.0 - THUMBNAIL_GAP)).abs() < 1e-9);
    }

    #[test]
    fn bad_pages_are_refused_and_leave_the_document_alone() {
        let mut p = PdfPreview::new(PdfStyle::Standard, 800.0, 600.0);
        p.set_pages(&letter()).unwrap();
        assert_eq!(p.set_pages(&[(0.0, 10.0)]), Err(PdfError::BadPageSize));
        assert_eq!(p.set_pages(&[(f64::NAN, 10.0)]), Err(PdfError::BadPageSize));
        assert_eq!(p.page_count(), 3);
        let many = vec![(10.0, 10.0); MAX_PAGES + 1];
        assert_eq!(p.set_pages(&many), Err(PdfError::TooManyPages));
        assert_eq!(p.page_count(), 3);
    }

    #[test]
    fn an_empty_document_has_no_thumbnails() {
        let p = PdfPreview::new(PdfStyle::ThumbnailsLeft, 800.0, 600.0);
        assert!(p.sidebar_frames().is_empty());
        assert_eq!(p.current_thumbnail(), None);
    }
}
