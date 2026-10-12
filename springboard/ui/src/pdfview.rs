//! PDF document view layout as a pure model: the display mode (one page or two pages per row, stacked continuously or
//! shown one row at a time), the scale factor with its limits and fit-to-width autoscaling, the frame of each page in
//! document space, the scroll position, the current page, the visible pages, and page navigation.
//!
//! Pages are given as their display size in points. Rows are built from pages in order: one page per row in the single
//! page modes, pairs of pages in the two-up modes. Each row is centred horizontally in the content width, which is the
//! wider of the viewport and the widest row. Pages in a row are top-aligned, separated by the page gap. A view draws
//! the returned frames; nothing here draws.

use crate::Rect;

/// How pages are arranged in the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    /// One page at a time. Only the current page is laid out.
    SinglePage,
    /// One page per row, stacked vertically with the page gap between rows.
    SinglePageContinuous,
    /// Two pages side by side. Only the row holding the current page is laid out.
    TwoUp,
    /// Rows of two pages stacked vertically with the page gap between rows.
    TwoUpContinuous,
}

impl DisplayMode {
    /// True for the modes that stack every row in one scrollable column.
    pub fn is_continuous(self) -> bool {
        matches!(self, DisplayMode::SinglePageContinuous | DisplayMode::TwoUpContinuous)
    }

    /// Pages per row.
    pub fn pages_per_row(self) -> usize {
        match self {
            DisplayMode::SinglePage | DisplayMode::SinglePageContinuous => 1,
            DisplayMode::TwoUp | DisplayMode::TwoUpContinuous => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewError {
    /// A page size is zero, negative, or not finite.
    BadSize,
    /// A page index is past the last page.
    NoSuchPage,
    /// A scale limit is zero, negative, not finite, or the minimum is above the maximum.
    BadScale,
}

/// The frame of one page in document space, with its index in the document.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageFrame {
    pub index: usize,
    pub rect: Rect,
}

/// Default scale limits: a page can be shown at a tenth of its size up to sixteen times its size.
pub const DEFAULT_MIN_SCALE: f64 = 0.1;
pub const DEFAULT_MAX_SCALE: f64 = 16.0;
/// Default gap between pages, in points at scale 1.
pub const DEFAULT_PAGE_GAP: f64 = 10.0;

/// The state of one document view.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfView {
    sizes: Vec<(f64, f64)>,
    mode: DisplayMode,
    scale: f64,
    auto_scales: bool,
    min_scale: f64,
    max_scale: f64,
    page_gap: f64,
    view_w: f64,
    view_h: f64,
    scroll_y: f64,
    current: usize,
}

impl PdfView {
    /// An empty view of the given viewport size, in single page mode at scale 1.
    pub fn new(view_w: f64, view_h: f64) -> PdfView {
        PdfView {
            sizes: Vec::new(),
            mode: DisplayMode::SinglePageContinuous,
            scale: 1.0,
            auto_scales: false,
            min_scale: DEFAULT_MIN_SCALE,
            max_scale: DEFAULT_MAX_SCALE,
            page_gap: DEFAULT_PAGE_GAP,
            view_w: view_w.max(0.0),
            view_h: view_h.max(0.0),
            scroll_y: 0.0,
            current: 0,
        }
    }

    /// Replaces the document's pages. Each entry is the page's display size in points. The current page resets to the
    /// first page and the view scrolls to the top.
    pub fn set_pages(&mut self, sizes: &[(f64, f64)]) -> Result<(), ViewError> {
        for &(w, h) in sizes {
            if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
                return Err(ViewError::BadSize);
            }
        }
        self.sizes = sizes.to_vec();
        self.current = 0;
        self.scroll_y = 0.0;
        Ok(())
    }

    pub fn page_count(&self) -> usize {
        self.sizes.len()
    }

    pub fn display_mode(&self) -> DisplayMode {
        self.mode
    }

    /// Changes the display mode. The current page stays the same and the view scrolls to show it.
    pub fn set_display_mode(&mut self, mode: DisplayMode) {
        self.mode = mode;
        self.clamp_scroll();
        if mode.is_continuous() {
            let row_top = self.row_top_of(self.current);
            self.scroll_y = row_top;
            self.clamp_scroll();
        }
    }

    pub fn set_view_size(&mut self, view_w: f64, view_h: f64) {
        self.view_w = view_w.max(0.0);
        self.view_h = view_h.max(0.0);
        self.clamp_scroll();
    }

    pub fn page_gap(&self) -> f64 {
        self.page_gap
    }

    pub fn set_page_gap(&mut self, gap: f64) {
        self.page_gap = if gap.is_finite() { gap.max(0.0) } else { 0.0 };
    }

    /// Sets the scale limits. The current scale is clamped into the new range.
    pub fn set_scale_limits(&mut self, min: f64, max: f64) -> Result<(), ViewError> {
        if !(min.is_finite() && max.is_finite() && min > 0.0 && max >= min) {
            return Err(ViewError::BadScale);
        }
        self.min_scale = min;
        self.max_scale = max;
        self.scale = self.scale.clamp(min, max);
        Ok(())
    }

    /// The scale limits as (min, max).
    pub fn scale_limits(&self) -> (f64, f64) {
        (self.min_scale, self.max_scale)
    }

    /// Sets a fixed scale, clamped into the limits, and turns autoscaling off.
    pub fn set_scale_factor(&mut self, scale: f64) {
        self.auto_scales = false;
        if scale.is_finite() {
            self.scale = scale.clamp(self.min_scale, self.max_scale);
        }
        self.clamp_scroll();
    }

    /// The fixed scale that was set. With autoscaling on, see [`PdfView::effective_scale`].
    pub fn scale_factor(&self) -> f64 {
        self.scale
    }

    pub fn auto_scales(&self) -> bool {
        self.auto_scales
    }

    /// Turns autoscaling on or off. With it on, the widest row fits the viewport width, within the scale limits.
    pub fn set_auto_scales(&mut self, on: bool) {
        self.auto_scales = on;
        self.clamp_scroll();
    }

    /// The scale used for layout: the fixed scale, or the fit-to-width scale when autoscaling is on.
    pub fn effective_scale(&self) -> f64 {
        if !self.auto_scales {
            return self.scale;
        }
        let widest = self.rows().iter().map(|row| self.row_width_unscaled(row)).fold(0.0_f64, f64::max);
        if widest <= 0.0 || self.view_w <= 0.0 {
            return self.scale;
        }
        (self.view_w / widest).clamp(self.min_scale, self.max_scale)
    }

    /// The pages grouped into rows, as document indexes.
    fn rows(&self) -> Vec<Vec<usize>> {
        let per = self.mode.pages_per_row();
        let mut rows = Vec::new();
        let mut i = 0;
        while i < self.sizes.len() {
            let end = (i + per).min(self.sizes.len());
            rows.push((i..end).collect());
            i = end;
        }
        rows
    }

    fn row_width_unscaled(&self, row: &[usize]) -> f64 {
        let widths: f64 = row.iter().map(|&i| self.sizes[i].0).sum();
        widths + self.page_gap * row.len().saturating_sub(1) as f64
    }

    fn row_height_unscaled(&self, row: &[usize]) -> f64 {
        row.iter().map(|&i| self.sizes[i].1).fold(0.0_f64, f64::max)
    }

    /// The index of the row holding `page`.
    fn row_of(&self, page: usize) -> usize {
        page / self.mode.pages_per_row()
    }

    /// The top of the row holding `page` in document space, at the current scale.
    fn row_top_of(&self, page: usize) -> f64 {
        if page >= self.sizes.len() {
            return 0.0;
        }
        let s = self.effective_scale();
        let rows = self.rows();
        let target = self.row_of(page);
        let mut top = 0.0;
        for (r, row) in rows.iter().enumerate() {
            if r == target {
                break;
            }
            top += self.row_height_unscaled(row) * s + self.page_gap * s;
        }
        top
    }

    /// The size of the document in document space at the current scale, as (width, height).
    pub fn content_size(&self) -> (f64, f64) {
        let s = self.effective_scale();
        let rows = self.rows();
        if rows.is_empty() {
            return (self.view_w, self.view_h);
        }
        let widest = rows.iter().map(|r| self.row_width_unscaled(r) * s).fold(0.0_f64, f64::max);
        let width = widest.max(self.view_w);
        let height = if self.mode.is_continuous() {
            let mut h = 0.0;
            for (r, row) in rows.iter().enumerate() {
                h += self.row_height_unscaled(row) * s;
                if r + 1 < rows.len() {
                    h += self.page_gap * s;
                }
            }
            h
        } else {
            // Only the current row is laid out in the non-continuous modes.
            let row = &rows[self.row_of(self.current)];
            self.row_height_unscaled(row) * s
        };
        (width, height)
    }

    /// The largest scroll position that keeps the content covering the viewport.
    pub fn max_scroll_y(&self) -> f64 {
        let (_, h) = self.content_size();
        (h - self.view_h).max(0.0)
    }

    pub fn scroll_y(&self) -> f64 {
        self.scroll_y
    }

    /// Scrolls to `y`, clamped to the content. Only continuous modes scroll; the others always sit at zero.
    pub fn scroll_to(&mut self, y: f64) {
        if !self.mode.is_continuous() || !y.is_finite() {
            return;
        }
        self.scroll_y = y.clamp(0.0, self.max_scroll_y());
    }

    fn clamp_scroll(&mut self) {
        if self.mode.is_continuous() {
            self.scroll_y = self.scroll_y.clamp(0.0, self.max_scroll_y());
        } else {
            self.scroll_y = 0.0;
        }
    }

    /// The frames of the laid-out pages in document space. Continuous modes return every row. The other modes return
    /// the row holding the current page, placed at the top of the content.
    pub fn page_frames(&self) -> Vec<PageFrame> {
        let s = self.effective_scale();
        let rows = self.rows();
        let (content_w, _) = self.content_size();
        let mut frames = Vec::new();
        let mut top = 0.0;
        for (r, row) in rows.iter().enumerate() {
            if !self.mode.is_continuous() && r != self.row_of(self.current) {
                continue;
            }
            let row_w = self.row_width_unscaled(row) * s;
            let mut x = (content_w - row_w) / 2.0;
            for &i in row {
                let (w, h) = self.sizes[i];
                frames.push(PageFrame { index: i, rect: Rect { x, y: top, w: w * s, h: h * s } });
                x += (w + self.page_gap) * s;
            }
            top += (self.row_height_unscaled(row) + self.page_gap) * s;
        }
        frames
    }

    /// The current page. In continuous modes it is the first page of the row that crosses the middle of the viewport.
    pub fn current_page(&self) -> Option<usize> {
        if self.sizes.is_empty() {
            return None;
        }
        if !self.mode.is_continuous() {
            return Some(self.current);
        }
        let mid = self.scroll_y + self.view_h / 2.0;
        let s = self.effective_scale();
        let rows = self.rows();
        let mut top = 0.0;
        for row in rows.iter() {
            let bottom = top + self.row_height_unscaled(row) * s;
            if mid < bottom {
                return row.first().copied();
            }
            top = bottom + self.page_gap * s;
        }
        rows.last().and_then(|r| r.first().copied())
    }

    /// The pages on screen: the laid-out pages whose frames meet the viewport.
    pub fn visible_pages(&self) -> Vec<usize> {
        let frames = self.page_frames();
        if !self.mode.is_continuous() {
            return frames.iter().map(|f| f.index).collect();
        }
        let (top, bottom) = (self.scroll_y, self.scroll_y + self.view_h);
        frames
            .iter()
            .filter(|f| f.rect.y < bottom && f.rect.y + f.rect.h > top)
            .map(|f| f.index)
            .collect()
    }

    /// Makes `page` the current page. In continuous modes the view scrolls so its row starts at the top.
    pub fn go_to_page(&mut self, page: usize) -> Result<(), ViewError> {
        if page >= self.sizes.len() {
            return Err(ViewError::NoSuchPage);
        }
        self.current = page;
        if self.mode.is_continuous() {
            self.scroll_y = self.row_top_of(page);
            self.clamp_scroll();
        }
        Ok(())
    }

    /// Moves to the page after the current one. Returns false at the last page.
    pub fn next_page(&mut self) -> bool {
        match self.current_page() {
            Some(p) if p + 1 < self.sizes.len() => self.go_to_page(p + 1).is_ok(),
            _ => false,
        }
    }

    /// Moves to the page before the current one. Returns false at the first page.
    pub fn previous_page(&mut self) -> bool {
        match self.current_page() {
            Some(p) if p > 0 => self.go_to_page(p - 1).is_ok(),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn three_tall() -> PdfView {
        let mut v = PdfView::new(300.0, 150.0);
        v.set_pages(&[(100.0, 200.0), (100.0, 200.0), (100.0, 200.0)]).unwrap();
        v.set_page_gap(10.0);
        v
    }

    #[test]
    fn continuous_rows_stack_with_gap_and_centre() {
        let v = three_tall();
        let f = v.page_frames();
        assert_eq!(f.len(), 3);
        // Row tops are 0, 210 and 420. Content is 300 wide, so each 100 wide page starts at x 100.
        assert!(approx(f[0].rect.y, 0.0));
        assert!(approx(f[1].rect.y, 210.0));
        assert!(approx(f[2].rect.y, 420.0));
        assert!(approx(f[1].rect.x, 100.0));
        assert_eq!(v.content_size(), (300.0, 620.0));
        assert!(approx(v.max_scroll_y(), 470.0));
    }

    #[test]
    fn two_up_shows_only_the_current_row() {
        let mut v = PdfView::new(500.0, 300.0);
        v.set_pages(&[(100.0, 100.0), (100.0, 100.0), (100.0, 100.0), (100.0, 100.0)]).unwrap();
        v.set_page_gap(10.0);
        v.set_display_mode(DisplayMode::TwoUp);
        v.go_to_page(2).unwrap();
        // The row [2, 3] is 210 wide, so it starts at (500 - 210) / 2 = 145 and the second page is 110 further on.
        let f = v.page_frames();
        assert_eq!(f.iter().map(|p| p.index).collect::<Vec<_>>(), vec![2, 3]);
        assert!(approx(f[0].rect.x, 145.0));
        assert!(approx(f[1].rect.x, 255.0));
        assert_eq!(v.visible_pages(), vec![2, 3]);
        assert_eq!(v.current_page(), Some(2));
    }

    #[test]
    fn scale_is_clamped_and_autoscale_fits_width() {
        let mut v = PdfView::new(400.0, 300.0);
        v.set_pages(&[(200.0, 300.0)]).unwrap();
        v.set_scale_factor(50.0);
        assert!(approx(v.scale_factor(), DEFAULT_MAX_SCALE));
        v.set_scale_factor(0.01);
        assert!(approx(v.scale_factor(), DEFAULT_MIN_SCALE));
        v.set_auto_scales(true);
        // A 200 point page in a 400 point view fits at scale 2.
        assert!(approx(v.effective_scale(), 2.0));
        assert!(v.set_scale_limits(0.0, 4.0).is_err());
        assert!(v.set_scale_limits(5.0, 4.0).is_err());
    }

    #[test]
    fn current_page_follows_scroll_in_continuous_mode() {
        let mut v = three_tall();
        v.scroll_to(230.0);
        // The viewport middle is at 305, which is inside the second row (210 to 410).
        assert_eq!(v.current_page(), Some(1));
        // Rows 1 (210 to 410) is the only one that meets the viewport from 230 to 380.
        assert_eq!(v.visible_pages(), vec![1]);
        v.scroll_to(9999.0);
        assert!(approx(v.scroll_y(), 470.0));
        assert_eq!(v.current_page(), Some(2));
    }

    #[test]
    fn navigation_clamps_and_reports_bad_pages() {
        let mut v = three_tall();
        assert_eq!(v.go_to_page(3), Err(ViewError::NoSuchPage));
        v.go_to_page(2).unwrap();
        // The row top is 420, which is below the 470 limit, so the scroll is not clamped.
        assert!(approx(v.scroll_y(), 420.0));
        assert!(!v.next_page());
        assert!(v.previous_page());
        assert_eq!(v.current_page(), Some(1));
    }

    #[test]
    fn rejects_bad_page_sizes() {
        let mut v = PdfView::new(100.0, 100.0);
        assert_eq!(v.set_pages(&[(0.0, 10.0)]), Err(ViewError::BadSize));
        assert_eq!(v.set_pages(&[(f64::NAN, 10.0)]), Err(ViewError::BadSize));
        assert_eq!(v.page_count(), 0);
        assert_eq!(v.current_page(), None);
    }
}
