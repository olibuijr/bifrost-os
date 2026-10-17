//! Drawing into a PDF and paginating a view for printing.
//!
//! [`DrawOps`] is the small set of drawing calls shared by every drawing context that writes a page: fills, strokes,
//! paths, text and images. [`PdfContext`] implements it by writing a PDF content stream, one page between
//! [`PdfContext::begin_pdf_page`] and [`PdfContext::end_pdf_page`] (the shape of a PDF graphics context).
//!
//! A [`PrintableView`] describes what to print. Views use y-down coordinates with the origin at the top left. When a
//! view does not name its own pages ([`PrintableView::knows_page_range`] returns `None`) the bounds are cut into tiles
//! the size of the printable area divided by the scaling factor, running down each column and then across. When it
//! does, [`PrintableView::rect_for_page`] gives the rect each page shows. [`paginate`] returns those rects and
//! [`render_pages`] draws them into PDF pages.
//!
//! Text uses the standard Helvetica face, font `/F0`, so a document written from these pages takes `["Helvetica"]` as
//! its font list.

use crate::print::PrintInfo;
use bifrost_render::cg::pdf::writer::{text_literal, Page, PageImage};

/// A rect as x, y, width, height.
pub type Rect = (f32, f32, f32, f32);

/// The drawing calls a page context accepts. Colours are 0.0 to 1.0.
pub trait DrawOps {
    fn set_fill_rgb(&mut self, r: f32, g: f32, b: f32);
    fn set_stroke_rgb(&mut self, r: f32, g: f32, b: f32);
    fn set_line_width(&mut self, width: f32);
    fn fill_rect(&mut self, rect: Rect);
    fn stroke_rect(&mut self, rect: Rect);
    fn move_to(&mut self, x: f32, y: f32);
    fn line_to(&mut self, x: f32, y: f32);
    fn curve_to(&mut self, c1: (f32, f32), c2: (f32, f32), to: (f32, f32));
    fn close_path(&mut self);
    fn fill_path(&mut self);
    fn stroke_path(&mut self);
    /// Draws `text` with its baseline starting at (x, y).
    fn draw_text(&mut self, x: f32, y: f32, size: f32, text: &str);
    /// Draws a picture into `rect`.
    fn draw_image(&mut self, image: PageImage, rect: Rect);
}

fn num(v: f32) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Writes pages as PDF content streams.
#[derive(Debug, Default)]
pub struct PdfContext {
    done: Vec<Page>,
    current: Option<Page>,
    /// True after [`PdfContext::begin_view_page`]: the page space is y-down, so text and images flip back.
    flipped: bool,
}

impl PdfContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a page of `width` x `height` points in PDF space (origin at the lower left, y up). A page still open is
    /// ended first.
    pub fn begin_pdf_page(&mut self, width: f64, height: f64) {
        self.end_pdf_page();
        self.flipped = false;
        self.current = Some(Page { width, height, content: Vec::new(), images: Vec::new() });
    }

    /// Starts a page that shows `source` of a y-down view. The part of the sheet inside `imageable` (x, y from the top
    /// left, width, height, in points) holds the view, scaled by `scale` and clipped to the source rect.
    pub fn begin_view_page(&mut self, paper: (f32, f32), imageable: Rect, source: Rect, scale: f32) {
        self.begin_pdf_page(paper.0 as f64, paper.1 as f64);
        let (left, top, ..) = imageable;
        let (rx, ry, rw, rh) = source;
        let clip_h = rh * scale;
        // clip to the tile, then map view space (y down) onto the sheet
        self.raw(&format!("q {} {} {} {} re W n\n", num(left), num(paper.1 - top - clip_h), num(rw * scale), num(clip_h)));
        self.raw(&format!(
            "{} 0 0 {} {} {} cm\n",
            num(scale),
            num(-scale),
            num(left - rx * scale),
            num(paper.1 - top + ry * scale)
        ));
        self.flipped = true;
    }

    /// Ends the open page. Does nothing without one.
    pub fn end_pdf_page(&mut self) {
        if let Some(mut page) = self.current.take() {
            if self.flipped {
                page.content.extend(b"Q\n");
            }
            self.done.push(page);
        }
        self.flipped = false;
    }

    /// Ends the open page and returns every page written.
    pub fn finish(mut self) -> Vec<Page> {
        self.end_pdf_page();
        self.done
    }

    fn raw(&mut self, ops: &str) {
        if let Some(page) = self.current.as_mut() {
            page.content.extend(ops.as_bytes());
        }
    }

    fn rect_op(&mut self, rect: Rect, paint: &str) {
        let (x, y, w, h) = rect;
        self.raw(&format!("{} {} {} {} re {paint}\n", num(x), num(y), num(w), num(h)));
    }
}

impl DrawOps for PdfContext {
    fn set_fill_rgb(&mut self, r: f32, g: f32, b: f32) {
        self.raw(&format!("{} {} {} rg\n", num(r), num(g), num(b)));
    }
    fn set_stroke_rgb(&mut self, r: f32, g: f32, b: f32) {
        self.raw(&format!("{} {} {} RG\n", num(r), num(g), num(b)));
    }
    fn set_line_width(&mut self, width: f32) {
        self.raw(&format!("{} w\n", num(width)));
    }
    fn fill_rect(&mut self, rect: Rect) {
        self.rect_op(rect, "f");
    }
    fn stroke_rect(&mut self, rect: Rect) {
        self.rect_op(rect, "S");
    }
    fn move_to(&mut self, x: f32, y: f32) {
        self.raw(&format!("{} {} m\n", num(x), num(y)));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.raw(&format!("{} {} l\n", num(x), num(y)));
    }
    fn curve_to(&mut self, c1: (f32, f32), c2: (f32, f32), to: (f32, f32)) {
        self.raw(&format!("{} {} {} {} {} {} c\n", num(c1.0), num(c1.1), num(c2.0), num(c2.1), num(to.0), num(to.1)));
    }
    fn close_path(&mut self) {
        self.raw("h\n");
    }
    fn fill_path(&mut self) {
        self.raw("f\n");
    }
    fn stroke_path(&mut self) {
        self.raw("S\n");
    }
    fn draw_text(&mut self, x: f32, y: f32, size: f32, text: &str) {
        let d = if self.flipped { -1 } else { 1 };
        let mut ops = format!("BT /F0 {} Tf 1 0 0 {d} {} {} Tm ", num(size), num(x), num(y)).into_bytes();
        text_literal(text, &mut ops);
        ops.extend(b" Tj ET\n");
        if let Some(page) = self.current.as_mut() {
            page.content.extend(ops);
        }
    }
    fn draw_image(&mut self, image: PageImage, rect: Rect) {
        let flipped = self.flipped;
        let Some(page) = self.current.as_mut() else { return };
        let index = page.images.len();
        page.images.push(image);
        let (x, y, w, h) = rect;
        // a y-down page needs the picture's own y axis flipped back
        let (sy, oy) = if flipped { (-h, y + h) } else { (h, y) };
        let ops = format!("q {} 0 0 {} {} {} cm /Im{index} Do Q\n", num(w), num(sy), num(x), num(oy));
        page.content.extend(ops.as_bytes());
    }
}

/// Something that can be printed.
pub trait PrintableView {
    /// The view's extent, y down.
    fn bounds(&self) -> Rect;
    /// `knowsPageRange`: the first and last page numbers (1-based) when the view paginates itself.
    fn knows_page_range(&self) -> Option<(u32, u32)> {
        None
    }
    /// `rectForPage`: the part of the view that page `page` shows. Only asked when [`knows_page_range`] is `Some`.
    ///
    /// [`knows_page_range`]: PrintableView::knows_page_range
    fn rect_for_page(&self, _page: u32) -> Rect {
        self.bounds()
    }
    /// Draws the part of the view inside `rect`.
    fn draw(&self, rect: Rect, out: &mut dyn DrawOps);
}

/// The size of the view area one page shows, in view units: the printable area divided by the scaling factor.
fn page_extent(info: &PrintInfo) -> (f32, f32) {
    let (_, _, w, h) = info.imageable();
    let scale = info.scaling();
    (w / scale, h / scale)
}

/// The rect each page shows, in page order.
pub fn paginate(view: &dyn PrintableView, info: &PrintInfo) -> Vec<Rect> {
    if let Some((first, last)) = view.knows_page_range() {
        return (first..=last).map(|n| view.rect_for_page(n)).collect();
    }
    let (bx, by, bw, bh) = view.bounds();
    let (pw, ph) = page_extent(info);
    if pw <= 0.0 || ph <= 0.0 || bw <= 0.0 || bh <= 0.0 {
        return Vec::new();
    }
    let columns = (bw / pw).ceil() as u32;
    let rows = (bh / ph).ceil() as u32;
    let mut rects = Vec::with_capacity((columns * rows) as usize);
    for column in 0..columns {
        for row in 0..rows {
            let (x, y) = (bx + column as f32 * pw, by + row as f32 * ph);
            rects.push((x, y, pw.min(bx + bw - x), ph.min(by + bh - y)));
        }
    }
    rects
}

/// Draws the pages of `view` for `info` and returns the PDF pages. When `info` has a page range, only those 1-based pages
/// are drawn, in order, so the PDF has one page per number in the range (and none past the last page of the view).
pub fn render_pages(view: &dyn PrintableView, info: &PrintInfo) -> Vec<Page> {
    let paper = info.paper_size();
    let mut ctx = PdfContext::new();
    let keep = |n: usize| info.pages.map_or(true, |(first, last)| (first as usize..=last as usize).contains(&n));
    for (index, rect) in paginate(view, info).into_iter().enumerate() {
        if !keep(index + 1) {
            continue;
        }
        ctx.begin_view_page(paper, info.imageable(), rect, info.scaling());
        view.draw(rect, &mut ctx);
        ctx.end_pdf_page();
    }
    ctx.finish()
}

/// Draws the part of `view` inside `rect` onto one page of the same size, at scale 1.
pub fn render_region(view: &dyn PrintableView, rect: Rect) -> Page {
    let (_, _, w, h) = rect;
    let mut ctx = PdfContext::new();
    ctx.begin_view_page((w, h), (0.0, 0.0, w, h), rect, 1.0);
    view.draw(rect, &mut ctx);
    ctx.finish().remove(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::print::{Orientation, PaperSize};
    use bifrost_render::cg::pdf::{doc::Doc, page, writer};

    /// A tall view that draws a label and a rect.
    struct Sheet {
        bounds: Rect,
        pages: Option<(u32, u32)>,
    }

    impl PrintableView for Sheet {
        fn bounds(&self) -> Rect {
            self.bounds
        }
        fn knows_page_range(&self) -> Option<(u32, u32)> {
            self.pages
        }
        fn rect_for_page(&self, page: u32) -> Rect {
            (0.0, (page - 1) as f32 * 100.0, 200.0, 100.0)
        }
        fn draw(&self, rect: Rect, out: &mut dyn DrawOps) {
            out.set_fill_rgb(1.0, 0.0, 0.0);
            out.fill_rect((rect.0 + 10.0, rect.1 + 10.0, 50.0, 20.0));
            out.draw_text(rect.0 + 10.0, rect.1 + 60.0, 12.0, "page");
        }
    }

    fn a4_no_margins() -> PrintInfo {
        PrintInfo { paper: PaperSize::A4, margins: [0.0; 4], ..PrintInfo::default() }
    }

    fn text(page: &Page) -> String {
        String::from_utf8_lossy(&page.content).into_owned()
    }

    #[test]
    fn a_context_writes_pages_with_fills_paths_and_text() {
        let mut ctx = PdfContext::new();
        ctx.begin_pdf_page(200.0, 100.0);
        ctx.set_fill_rgb(1.0, 0.5, 0.0);
        ctx.fill_rect((10.0, 20.0, 30.0, 40.0));
        ctx.set_stroke_rgb(0.0, 0.0, 1.0);
        ctx.set_line_width(2.0);
        ctx.move_to(0.0, 0.0);
        ctx.line_to(50.0, 25.5);
        ctx.curve_to((1.0, 2.0), (3.0, 4.0), (5.0, 6.0));
        ctx.close_path();
        ctx.stroke_path();
        ctx.draw_text(5.0, 6.0, 10.0, "Hi (x)");
        ctx.end_pdf_page();
        ctx.begin_pdf_page(200.0, 100.0);
        let pages = ctx.finish();
        assert_eq!(pages.len(), 2, "finish ends the open page");
        assert_eq!((pages[0].width, pages[0].height), (200.0, 100.0));
        let c = text(&pages[0]);
        assert!(c.contains("1 0.5 0 rg\n10 20 30 40 re f\n"));
        assert!(c.contains("0 0 1 RG\n2 w\n0 0 m\n50 25.5 l\n1 2 3 4 5 6 c\nh\nS\n"));
        assert!(c.contains("BT /F0 10 Tf 1 0 0 1 5 6 Tm (Hi \\(x\\)) Tj ET"));
    }

    #[test]
    fn images_are_recorded_per_page_and_referenced_by_index() {
        let mut ctx = PdfContext::new();
        ctx.begin_pdf_page(100.0, 100.0);
        ctx.draw_image(PageImage { width: 1, height: 1, rgb: vec![9, 8, 7] }, (10.0, 20.0, 30.0, 40.0));
        ctx.draw_image(PageImage { width: 1, height: 1, rgb: vec![1, 2, 3] }, (0.0, 0.0, 5.0, 5.0));
        let pages = ctx.finish();
        assert_eq!(pages[0].images.len(), 2);
        assert!(text(&pages[0]).contains("q 30 0 0 40 10 20 cm /Im0 Do Q"));
        assert!(text(&pages[0]).contains("/Im1 Do"));
    }

    #[test]
    fn drawing_without_a_page_is_ignored() {
        let mut ctx = PdfContext::new();
        ctx.fill_rect((0.0, 0.0, 1.0, 1.0));
        ctx.draw_image(PageImage { width: 1, height: 1, rgb: vec![0; 3] }, (0.0, 0.0, 1.0, 1.0));
        assert!(ctx.finish().is_empty());
    }

    #[test]
    fn a_view_without_a_page_range_is_cut_into_tiles_down_then_across() {
        let info = a4_no_margins();
        // A4 is 595 x 842 points: a 1000 x 1000 view needs 2 columns and 2 rows
        let view = Sheet { bounds: (0.0, 0.0, 1000.0, 1000.0), pages: None };
        let rects = paginate(&view, &info);
        assert_eq!(rects.len(), 4);
        assert_eq!(rects[0], (0.0, 0.0, 595.0, 842.0));
        assert_eq!(rects[1], (0.0, 842.0, 595.0, 158.0), "the next tile is below, shorter at the edge");
        assert_eq!(rects[2], (595.0, 0.0, 405.0, 842.0), "then the next column");
    }

    #[test]
    fn the_scaling_factor_changes_how_much_fits_on_a_page() {
        let view = Sheet { bounds: (0.0, 0.0, 595.0, 842.0), pages: None };
        let mut info = a4_no_margins();
        assert_eq!(paginate(&view, &info).len(), 1);
        info.scaling_factor = 2.0;
        assert_eq!(paginate(&view, &info).len(), 4, "doubling the size needs a 2 x 2 grid");
        info.scaling_factor = 0.5;
        assert_eq!(paginate(&view, &info).len(), 1);
    }

    #[test]
    fn a_view_that_knows_its_pages_decides_the_rects() {
        let view = Sheet { bounds: (0.0, 0.0, 200.0, 300.0), pages: Some((1, 3)) };
        let rects = paginate(&view, &a4_no_margins());
        assert_eq!(rects, vec![(0.0, 0.0, 200.0, 100.0), (0.0, 100.0, 200.0, 100.0), (0.0, 200.0, 200.0, 100.0)]);
    }

    #[test]
    fn an_empty_view_has_no_pages() {
        let view = Sheet { bounds: (0.0, 0.0, 0.0, 0.0), pages: None };
        assert!(paginate(&view, &PrintInfo::default()).is_empty());
    }

    #[test]
    fn view_pages_map_the_view_onto_the_printable_area_and_flip_text_upright() {
        let info = PrintInfo { paper: PaperSize::A4, orientation: Orientation::Portrait, ..PrintInfo::default() };
        let view = Sheet { bounds: (0.0, 0.0, 200.0, 100.0), pages: None };
        let pages = render_pages(&view, &info);
        assert_eq!(pages.len(), 1);
        let c = text(&pages[0]);
        // clip to the 200 x 100 tile at the top left of the printable area (72 from the top: 842 - 72 - 100 = 670)
        assert!(c.starts_with("q 72 670 200 100 re W n\n1 0 0 -1 72 770 cm\n"));
        assert!(c.contains("Tm (page) Tj"), "text is drawn");
        assert!(c.contains("BT /F0 12 Tf 1 0 0 -1 10 60 Tm"), "a y-down page flips text back upright");
        assert!(c.ends_with("Q\n"));
    }

    #[test]
    fn rendered_pages_make_a_pdf_the_parser_reads() {
        let view = Sheet { bounds: (0.0, 0.0, 1000.0, 1000.0), pages: None };
        let pages = render_pages(&view, &a4_no_margins());
        let bytes = writer::write(&pages, &["Helvetica"]);
        let doc = Doc::open(bytes).expect("the written PDF parses");
        assert_eq!(page::page_count(&doc), 4);
    }

    #[test]
    fn a_region_is_one_page_of_its_own_size() {
        let view = Sheet { bounds: (0.0, 0.0, 400.0, 400.0), pages: None };
        let page = render_region(&view, (50.0, 60.0, 120.0, 80.0));
        assert_eq!((page.width, page.height), (120.0, 80.0));
        assert!(text(&page).starts_with("q 0 0 120 80 re W n\n1 0 0 -1 -50 140 cm\n"));
    }

    #[test]
    fn a_page_range_draws_only_those_pages_of_the_view() {
        let view = Sheet { bounds: (0.0, 0.0, 200.0, 350.0), pages: None };
        let info = PrintInfo { margins: [0.0; 4], ..PrintInfo::default() };
        assert_eq!(render_pages(&view, &info).len(), 1, "a short view is one page");
        let tall = Sheet { bounds: (0.0, 0.0, 200.0, 2000.0), pages: None };
        assert_eq!(render_pages(&tall, &info).len(), 3);
        let cut = PrintInfo { pages: Some((2, 3)), ..info.clone() };
        let pages = render_pages(&tall, &cut);
        assert_eq!(pages.len(), 2);
        assert_eq!((pages[0].width, pages[0].height), (595.0, 842.0));
        assert!(render_pages(&tall, &PrintInfo { pages: Some((9, 10)), ..info }).is_empty());
    }
}
