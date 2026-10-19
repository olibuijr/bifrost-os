//! The page layout of a fixed-layout PDF view (`PDFView`, `PDFThumbnailView`): which pages are shown at what size and
//! where, for single-page, continuous and two-up display, zoom and fit-to-width scaling, the current page, scrolling
//! to a page, and mapping a point in the view to a point on a page. It holds no pixels: an app asks for the slots
//! ([`PdfViewState::layout`]), renders each visible page with `graphics_pdf::Document::render` and draws it in its slot.
//!
//! ```ignore
//! let doc = graphics_pdf::Document::open(bytes)?;
//! let mut view = PdfViewState::for_document(&doc);
//! view.set_viewport(800.0, 600.0);
//! for slot in view.visible_slots(scroll_y) { /* draw page slot.page at slot.x, slot.y, slot.w, slot.h */ }
//! ```
//!
//! View coordinates have the origin at the top left of the scrolled content, y growing down. Page coordinates are PDF
//! points with the origin at the bottom left of the page.

use crate::graphics_pdf::Document;

/// `PDFDisplayMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    SinglePage,
    SinglePageContinuous,
    TwoUp,
    TwoUpContinuous,
}

impl DisplayMode {
    pub fn is_continuous(self) -> bool {
        matches!(self, DisplayMode::SinglePageContinuous | DisplayMode::TwoUpContinuous)
    }

    pub fn is_two_up(self) -> bool {
        matches!(self, DisplayMode::TwoUp | DisplayMode::TwoUpContinuous)
    }
}

/// Where one page is drawn, in view coordinates at the current scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageSlot {
    pub page: usize,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// A point on a page, in page points (origin at the bottom left).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PagePoint {
    pub page: usize,
    pub x: f64,
    pub y: f64,
}

const MIN_SCALE: f64 = 0.1;
const MAX_SCALE: f64 = 8.0;
const ZOOM_STEP: f64 = 1.25;

/// The state of a PDF page view.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfViewState {
    sizes: Vec<(f64, f64)>,
    mode: DisplayMode,
    scale: f64,
    auto_scales: bool,
    book: bool,
    gap: f64,
    margin: f64,
    viewport: (f64, f64),
    current: usize,
}

impl PdfViewState {
    /// A view of pages of the given sizes (points), in continuous single-page mode at scale 1 with a 8 pt gap between
    /// pages and a 12 pt margin.
    pub fn new(sizes: Vec<(f64, f64)>) -> PdfViewState {
        let sizes = sizes.into_iter().map(|(w, h)| (w.max(1.0), h.max(1.0))).collect();
        PdfViewState {
            sizes,
            mode: DisplayMode::SinglePageContinuous,
            scale: 1.0,
            auto_scales: false,
            book: false,
            gap: 8.0,
            margin: 12.0,
            viewport: (800.0, 600.0),
            current: 0,
        }
    }

    /// A view of every page of `doc`.
    pub fn for_document(doc: &Document) -> PdfViewState {
        PdfViewState::new((0..doc.page_count()).filter_map(|i| doc.page_size(i)).collect())
    }

    pub fn page_count(&self) -> usize {
        self.sizes.len()
    }

    pub fn display_mode(&self) -> DisplayMode {
        self.mode
    }

    pub fn set_display_mode(&mut self, mode: DisplayMode) {
        self.mode = mode;
        self.refresh();
    }

    /// Two-up layouts show the first page alone, as a book does (`displaysAsBook`).
    pub fn set_displays_as_book(&mut self, book: bool) {
        self.book = book;
        self.refresh();
    }

    pub fn scale_factor(&self) -> f64 {
        self.scale
    }

    /// Sets the scale (clamped to 0.1 through 8) and turns off automatic scaling.
    pub fn set_scale_factor(&mut self, scale: f64) {
        self.auto_scales = false;
        self.scale = clamp_scale(scale);
    }

    pub fn auto_scales(&self) -> bool {
        self.auto_scales
    }

    /// With automatic scaling the scale follows the viewport: the widest row of pages fits its width (and, outside
    /// continuous modes, the tallest row its height).
    pub fn set_auto_scales(&mut self, on: bool) {
        self.auto_scales = on;
        self.refresh();
    }

    pub fn zoom_in(&mut self) {
        self.set_scale_factor(self.scale * ZOOM_STEP);
    }

    pub fn zoom_out(&mut self) {
        self.set_scale_factor(self.scale / ZOOM_STEP);
    }

    pub fn can_zoom_in(&self) -> bool {
        self.scale < MAX_SCALE - 1e-9
    }

    pub fn can_zoom_out(&self) -> bool {
        self.scale > MIN_SCALE + 1e-9
    }

    /// The scale at which the widest row fits the viewport (and the tallest too outside continuous modes).
    pub fn scale_to_fit(&self) -> f64 {
        let rows = self.shown_rows();
        if rows.is_empty() {
            return 1.0;
        }
        let widest = rows.iter().map(|r| self.natural_row_size(r).0).fold(0.0, f64::max);
        let tallest = rows.iter().map(|r| self.natural_row_size(r).1).fold(0.0, f64::max);
        let by_width = (self.viewport.0 - 2.0 * self.margin).max(1.0) / widest.max(1.0);
        let fit = if self.mode.is_continuous() {
            by_width
        } else {
            by_width.min((self.viewport.1 - 2.0 * self.margin).max(1.0) / tallest.max(1.0))
        };
        clamp_scale(fit)
    }

    pub fn viewport(&self) -> (f64, f64) {
        self.viewport
    }

    pub fn set_viewport(&mut self, w: f64, h: f64) {
        self.viewport = (w.max(1.0), h.max(1.0));
        self.refresh();
    }

    /// The zero based current page.
    pub fn current_page(&self) -> usize {
        self.current
    }

    /// Makes `index` the current page (ignored past the last page). Non-continuous modes show the row it is in.
    pub fn go_to_page(&mut self, index: usize) -> bool {
        if index >= self.sizes.len() {
            return false;
        }
        self.current = index;
        self.refresh();
        true
    }

    pub fn can_go_to_next_page(&self) -> bool {
        self.rows().iter().position(|r| r.contains(&self.current)).is_some_and(|i| i + 1 < self.rows().len())
    }

    pub fn can_go_to_previous_page(&self) -> bool {
        self.rows().iter().position(|r| r.contains(&self.current)).is_some_and(|i| i > 0)
    }

    /// Moves to the first page of the next row (a page in single-page modes, a spread in two-up modes).
    pub fn go_to_next_page(&mut self) -> bool {
        let rows = self.rows();
        match rows.iter().position(|r| r.contains(&self.current)) {
            Some(i) if i + 1 < rows.len() => self.go_to_page(rows[i + 1][0]),
            _ => false,
        }
    }

    pub fn go_to_previous_page(&mut self) -> bool {
        let rows = self.rows();
        match rows.iter().position(|r| r.contains(&self.current)) {
            Some(i) if i > 0 => self.go_to_page(rows[i - 1][0]),
            _ => false,
        }
    }

    // ---- layout ---------------------------------------------------------------------------------------

    /// Pages grouped into display rows: one page each, or pairs in two-up modes (the first page alone for a book).
    fn rows(&self) -> Vec<Vec<usize>> {
        let n = self.sizes.len();
        if !self.mode.is_two_up() {
            return (0..n).map(|i| vec![i]).collect();
        }
        let mut rows = Vec::new();
        let mut i = 0;
        if self.book && n > 0 {
            rows.push(vec![0]);
            i = 1;
        }
        while i < n {
            let end = (i + 2).min(n);
            rows.push((i..end).collect());
            i = end;
        }
        rows
    }

    /// The rows that are laid out: all of them in continuous modes, else the one holding the current page.
    fn shown_rows(&self) -> Vec<Vec<usize>> {
        let rows = self.rows();
        if self.mode.is_continuous() {
            return rows;
        }
        rows.into_iter().filter(|r| r.contains(&self.current)).collect()
    }

    /// The width and height of a row at scale 1.
    fn natural_row_size(&self, row: &[usize]) -> (f64, f64) {
        let w: f64 = row.iter().map(|&i| self.sizes[i].0).sum::<f64>() + self.gap * row.len().saturating_sub(1) as f64;
        let h = row.iter().map(|&i| self.sizes[i].1).fold(0.0, f64::max);
        (w, h)
    }

    fn refresh(&mut self) {
        if self.auto_scales {
            self.scale = self.scale_to_fit();
        }
    }

    /// The width of the scrolled content: the viewport, or wider when the pages are.
    pub fn content_width(&self) -> f64 {
        let widest = self.shown_rows().iter().map(|r| self.natural_row_size(r).0).fold(0.0, f64::max);
        self.viewport.0.max(widest * self.scale + 2.0 * self.margin)
    }

    /// The height of the scrolled content.
    pub fn content_height(&self) -> f64 {
        let rows = self.shown_rows();
        if rows.is_empty() {
            return 0.0;
        }
        let heights: f64 = rows.iter().map(|r| self.natural_row_size(r).1 * self.scale).sum();
        heights + self.gap * (rows.len() - 1) as f64 + 2.0 * self.margin
    }

    /// Every laid-out page with its slot, in reading order. Rows are centred in the content width and the pages of a
    /// row are centred on each other vertically.
    pub fn layout(&self) -> Vec<PageSlot> {
        let content_w = self.content_width();
        let mut y = self.margin;
        let mut out = Vec::new();
        for row in self.shown_rows() {
            let (nw, nh) = self.natural_row_size(&row);
            let (row_w, row_h) = (nw * self.scale, nh * self.scale);
            let mut x = (content_w - row_w) / 2.0;
            for &i in &row {
                let (w, h) = (self.sizes[i].0 * self.scale, self.sizes[i].1 * self.scale);
                out.push(PageSlot { page: i, x, y: y + (row_h - h) / 2.0, w, h });
                x += w + self.gap;
            }
            y += row_h + self.gap;
        }
        out
    }

    /// The slots that intersect the viewport when the content is scrolled `scroll_y` down.
    pub fn visible_slots(&self, scroll_y: f64) -> Vec<PageSlot> {
        let (top, bottom) = (scroll_y, scroll_y + self.viewport.1);
        self.layout().into_iter().filter(|s| s.y + s.h > top && s.y < bottom).collect()
    }

    /// The page at the vertical middle of the viewport (the nearest one when the middle falls in a gap), the page a
    /// reader would call current while scrolling. `None` for a document with no pages.
    pub fn page_at_scroll(&self, scroll_y: f64) -> Option<usize> {
        let mid = scroll_y + self.viewport.1 / 2.0;
        self.layout()
            .iter()
            .min_by(|a, b| {
                let da = (mid - (a.y + a.h / 2.0)).abs() - a.h / 2.0;
                let db = (mid - (b.y + b.h / 2.0)).abs() - b.h / 2.0;
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|s| s.page)
    }

    /// The scroll offset that brings page `index` to the top of the viewport (clamped to the scrollable range); `None`
    /// when the page is not laid out (it does not exist, or another spread is shown).
    pub fn scroll_offset_for_page(&self, index: usize) -> Option<f64> {
        let slot = self.layout().into_iter().find(|s| s.page == index)?;
        let max = (self.content_height() - self.viewport.1).max(0.0);
        Some((slot.y - self.margin).clamp(0.0, max))
    }

    /// The page point under the view point (`x`, `y`) when scrolled `scroll_y`; `None` between pages.
    pub fn point_to_page(&self, x: f64, y: f64, scroll_y: f64) -> Option<PagePoint> {
        let vy = y + scroll_y;
        let s = self.layout().into_iter().find(|s| x >= s.x && x < s.x + s.w && vy >= s.y && vy < s.y + s.h)?;
        Some(PagePoint { page: s.page, x: (x - s.x) / self.scale, y: (s.h - (vy - s.y)) / self.scale })
    }

    /// The view point (scrolled content coordinates) of a page point; `None` when the page is not laid out.
    pub fn page_to_point(&self, p: PagePoint) -> Option<(f64, f64)> {
        let s = self.layout().into_iter().find(|s| s.page == p.page)?;
        Some((s.x + p.x * self.scale, s.y + s.h - p.y * self.scale))
    }
}

fn clamp_scale(s: f64) -> f64 {
    if s.is_nan() {
        1.0
    } else {
        s.clamp(MIN_SCALE, MAX_SCALE)
    }
}

/// The size of a thumbnail of a page of `page` points that fits a square of `max_side` pixels, keeping the aspect.
pub fn thumbnail_size(page: (f64, f64), max_side: f64) -> (f64, f64) {
    let long = page.0.max(page.1).max(1.0);
    let k = max_side.max(1.0) / long;
    ((page.0 * k).max(1.0), (page.1 * k).max(1.0))
}

/// The cells of a thumbnail view (`PDFThumbnailView`): `count` cells of `cell` size in rows as wide as the viewport
/// allows, `spacing` apart and from the edges. Each cell is (x, y, w, h).
pub fn thumbnail_grid(count: usize, viewport_w: f64, cell: (f64, f64), spacing: f64) -> Vec<(f64, f64, f64, f64)> {
    let columns = (((viewport_w - spacing) / (cell.0 + spacing)).floor() as usize).max(1);
    (0..count)
        .map(|i| {
            let (col, row) = (i % columns, i / columns);
            (spacing + col as f64 * (cell.0 + spacing), spacing + row as f64 * (cell.1 + spacing), cell.0, cell.1)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics_pdf::PdfWriter;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    fn letter(n: usize) -> PdfViewState {
        PdfViewState::new(vec![(612.0, 792.0); n])
    }

    #[test]
    fn continuous_pages_stack_with_a_gap_and_centre_in_the_viewport() {
        let mut v = letter(3);
        v.set_viewport(1000.0, 700.0);
        let l = v.layout();
        assert_eq!(l.len(), 3);
        assert!(near(l[0].y, 12.0) && near(l[1].y, 12.0 + 792.0 + 8.0) && near(l[2].y, 12.0 + 2.0 * 800.0));
        assert!(near(l[0].x, (1000.0 - 612.0) / 2.0), "centred: {}", l[0].x);
        assert!(near(v.content_height(), 3.0 * 792.0 + 2.0 * 8.0 + 24.0));
        assert!(near(v.content_width(), 1000.0));
    }

    #[test]
    fn zoom_scales_the_slots_and_widens_the_content() {
        let mut v = letter(2);
        v.set_viewport(500.0, 500.0);
        v.zoom_in();
        assert!(near(v.scale_factor(), 1.25));
        let l = v.layout();
        assert!(near(l[0].w, 765.0) && near(l[0].h, 990.0));
        assert!(near(v.content_width(), 765.0 + 24.0), "wider than the viewport: {}", v.content_width());
        assert!(near(l[0].x, 12.0));
        for _ in 0..40 {
            v.zoom_in();
        }
        assert!(near(v.scale_factor(), 8.0) && !v.can_zoom_in());
        for _ in 0..80 {
            v.zoom_out();
        }
        assert!(near(v.scale_factor(), 0.1) && !v.can_zoom_out());
    }

    #[test]
    fn auto_scaling_fits_the_width_and_follows_the_viewport() {
        let mut v = letter(5);
        v.set_auto_scales(true);
        v.set_viewport(636.0, 400.0);
        assert!(near(v.scale_factor(), (636.0 - 24.0) / 612.0), "{}", v.scale_factor());
        v.set_viewport(1224.0 + 24.0, 400.0);
        assert!(near(v.scale_factor(), 2.0));
        v.set_scale_factor(1.0);
        assert!(!v.auto_scales());
        v.set_viewport(300.0, 300.0);
        assert!(near(v.scale_factor(), 1.0), "a manual scale stays put");
    }

    #[test]
    fn single_page_mode_shows_only_the_current_page_and_fits_both_ways() {
        let mut v = letter(4);
        v.set_display_mode(DisplayMode::SinglePage);
        v.set_viewport(1000.0, 800.0);
        v.set_auto_scales(true);
        assert!(near(v.scale_factor(), (800.0 - 24.0) / 792.0), "height is the tighter fit: {}", v.scale_factor());
        assert_eq!(v.layout().len(), 1);
        assert!(v.go_to_page(2));
        assert_eq!(v.layout()[0].page, 2);
        assert!(!v.go_to_page(9));
        assert!(v.go_to_next_page() && v.current_page() == 3);
        assert!(!v.go_to_next_page() && !v.can_go_to_next_page());
        assert!(v.go_to_previous_page() && v.current_page() == 2);
    }

    #[test]
    fn two_up_pairs_pages_and_a_book_starts_with_a_lone_page() {
        let mut v = letter(5);
        v.set_display_mode(DisplayMode::TwoUpContinuous);
        v.set_viewport(1400.0, 800.0);
        let l = v.layout();
        assert_eq!(l.iter().map(|s| s.page).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
        assert!(near(l[0].y, l[1].y) && l[2].y > l[0].y);
        assert!(near(l[1].x - (l[0].x + l[0].w), 8.0), "the gap between the pair");
        v.set_displays_as_book(true);
        let b = v.layout();
        assert_eq!(b.len(), 5);
        assert!(b[1].y > b[0].y, "page 0 is on its own row");
        assert!(near(b[1].y, b[2].y));
        assert!(v.go_to_page(3));
    }

    #[test]
    fn pages_of_different_sizes_centre_within_their_row() {
        let mut v = PdfViewState::new(vec![(200.0, 100.0), (200.0, 300.0)]);
        v.set_display_mode(DisplayMode::TwoUp);
        v.set_viewport(600.0, 600.0);
        let l = v.layout();
        assert!(near(l[0].y, 12.0 + 100.0), "the short page sits in the middle of the tall one");
        assert!(near(l[1].y, 12.0));
    }

    #[test]
    fn visible_slots_and_the_page_at_the_middle_follow_the_scroll() {
        let mut v = letter(10);
        v.set_viewport(700.0, 900.0);
        let top = v.visible_slots(0.0);
        assert_eq!(top.iter().map(|s| s.page).collect::<Vec<_>>(), vec![0, 1]);
        assert_eq!(v.page_at_scroll(0.0), Some(0));
        let y = v.scroll_offset_for_page(4).unwrap();
        assert!(near(y, 4.0 * 800.0));
        assert_eq!(v.page_at_scroll(y), Some(4));
        let far = v.scroll_offset_for_page(9).unwrap();
        assert!(near(far, v.content_height() - 900.0), "clamped to the end: {far}");
        assert_eq!(v.scroll_offset_for_page(10), None);
        assert_eq!(PdfViewState::new(Vec::new()).page_at_scroll(0.0), None);
    }

    #[test]
    fn points_map_between_the_view_and_the_page() {
        let mut v = letter(2);
        v.set_viewport(612.0 + 24.0, 600.0);
        // top-left of page 0 is view (12, 12), which is page point (0, 792)
        let p = v.point_to_page(12.0, 12.0, 0.0).unwrap();
        assert_eq!(p.page, 0);
        assert!(near(p.x, 0.0) && near(p.y, 792.0));
        // a point 100 pt in from the left and 200 pt up from the bottom of page 1, scrolled 100 down
        let target = PagePoint { page: 1, x: 100.0, y: 200.0 };
        let (vx, vy) = v.page_to_point(target).unwrap();
        let back = v.point_to_page(vx, vy - 100.0, 100.0).unwrap();
        assert_eq!(back.page, 1);
        assert!(near(back.x, 100.0) && near(back.y, 200.0), "{back:?}");
        assert!(v.point_to_page(12.0, 12.0 + 792.0 + 4.0, 0.0).is_none(), "the gap between pages is no page");
        assert!(v.point_to_page(1.0, 20.0, 0.0).is_none(), "the margin is no page");
    }

    #[test]
    fn scale_applies_to_point_mapping() {
        let mut v = letter(1);
        v.set_viewport(1300.0, 900.0);
        v.set_scale_factor(2.0);
        let s = v.layout()[0];
        let p = v.point_to_page(s.x + 200.0, s.y + 100.0, 0.0).unwrap();
        assert!(near(p.x, 100.0) && near(p.y, 792.0 - 50.0));
    }

    #[test]
    fn a_view_is_built_from_a_written_document() {
        let mut w = PdfWriter::new();
        w.add_page(300.0, 400.0);
        w.add_page(500.0, 200.0);
        let doc = Document::open(w.finish()).expect("reads back");
        let v = PdfViewState::for_document(&doc);
        assert_eq!(v.page_count(), 2);
        let l = v.layout();
        assert!(near(l[0].w, 300.0) && near(l[1].w, 500.0) && near(l[1].h, 200.0));
    }

    #[test]
    fn thumbnails_keep_the_aspect_and_wrap_to_the_viewport() {
        let (w, h) = thumbnail_size((612.0, 792.0), 100.0);
        assert!(near(h, 100.0) && near(w, 100.0 * 612.0 / 792.0));
        let grid = thumbnail_grid(5, 330.0, (100.0, 120.0), 10.0);
        // (330 - 10) / 110 = 2.9: two columns
        assert_eq!(grid[0], (10.0, 10.0, 100.0, 120.0));
        assert_eq!(grid[1], (120.0, 10.0, 100.0, 120.0));
        assert_eq!(grid[2], (10.0, 140.0, 100.0, 120.0));
        assert_eq!(grid.len(), 5);
        assert_eq!(thumbnail_grid(2, 10.0, (100.0, 100.0), 10.0).len(), 2, "at least one column");
    }
}
