//! A web page as a paginated PDF (`createPDF`, and the page layout of a web view's print operation).
//!
//! The engine has no print pipeline (Servo 0.7.0), so a page is printed from its full-page snapshot
//! (`webview_dev::ShotMode::Full`, rows of RGB pixels): the snapshot is scaled to the width of the paper inside the
//! margins and cut into pages, each page carrying one band of the picture, the page title in the top margin and
//! "Page n of m" in the bottom margin. The text of the page is therefore not selectable in the PDF; this is the
//! difference from a layout-based print and is listed in `docs/bifrost-ui.md`.
//!
//! ```ignore
//! let layout = web_print::PrintLayout::a4();
//! let pdf = web_print::pdf_from_rgb(&rgb, width, height, "Page title", &layout)?;
//! ```

use crate::graphics_pdf::{Face, PdfWriter};

/// Most pages one print produces.
pub const MAX_PAGES: usize = 500;
/// Most pixels in the snapshot that is printed.
pub const MAX_PIXELS: usize = 64_000_000;

/// Paper and margins, in PDF points (1/72 inch).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrintLayout {
    pub paper_width: f64,
    pub paper_height: f64,
    /// All four margins.
    pub margin: f64,
    /// Draw the page title and the page number in the margins.
    pub header_footer: bool,
}

impl PrintLayout {
    /// US Letter, half-inch margins.
    pub fn letter() -> PrintLayout {
        PrintLayout { paper_width: 612.0, paper_height: 792.0, margin: 36.0, header_footer: true }
    }

    /// A4, 15 mm margins.
    pub fn a4() -> PrintLayout {
        PrintLayout { paper_width: 595.276, paper_height: 841.89, margin: 42.52, header_footer: true }
    }

    fn content_width(&self) -> f64 {
        self.paper_width - 2.0 * self.margin
    }

    fn content_height(&self) -> f64 {
        self.paper_height - 2.0 * self.margin
    }

    fn usable(&self) -> bool {
        self.margin >= 0.0 && self.content_width() >= 36.0 && self.content_height() >= 36.0 && self.paper_width.is_finite() && self.paper_height.is_finite()
    }
}

/// One printed page: the rows `src_y..src_y + src_h` of the snapshot, drawn `dest_height` points tall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageSlice {
    pub src_y: usize,
    pub src_h: usize,
    pub dest_height: f64,
}

/// Cuts a snapshot of `width` x `height` pixels into pages. The snapshot is scaled to the content width of the
/// paper; every page but the last is full height. Empty for a snapshot with no pixels, an unusable layout, or more
/// than [`MAX_PAGES`] pages.
pub fn plan_pages(width: usize, height: usize, layout: &PrintLayout) -> Vec<PageSlice> {
    if width == 0 || height == 0 || !layout.usable() {
        return Vec::new();
    }
    let scale = layout.content_width() / width as f64; // points per pixel
    let rows_per_page = ((layout.content_height() / scale).floor() as usize).max(1);
    if height.div_ceil(rows_per_page) > MAX_PAGES {
        return Vec::new();
    }
    (0..height)
        .step_by(rows_per_page)
        .map(|y| {
            let h = rows_per_page.min(height - y);
            PageSlice { src_y: y, src_h: h, dest_height: h as f64 * scale }
        })
        .collect()
}

/// Straight RGBA pixels (`w * h * 4` bytes) as RGB over a white page.
pub fn rgba_to_rgb(rgba: &[u8], w: usize, h: usize) -> Result<Vec<u8>, String> {
    if rgba.len() != w.saturating_mul(h).saturating_mul(4) {
        return Err("the pixel count does not match the size".into());
    }
    Ok(rgba
        .chunks_exact(4)
        .flat_map(|p| {
            let a = p[3] as u32;
            let mix = |c: u8| ((c as u32 * a + 255 * (255 - a) + 127) / 255) as u8;
            [mix(p[0]), mix(p[1]), mix(p[2])]
        })
        .collect())
}

/// The PDF of a page snapshot: `rgb` is `width * height` pixels of three bytes, rows top to bottom.
pub fn pdf_from_rgb(rgb: &[u8], width: usize, height: usize, title: &str, layout: &PrintLayout) -> Result<Vec<u8>, String> {
    if width.saturating_mul(height) > MAX_PIXELS {
        return Err(format!("the page is larger than {MAX_PIXELS} pixels"));
    }
    if rgb.len() != width.saturating_mul(height).saturating_mul(3) {
        return Err("the pixel count does not match the size".into());
    }
    let pages = plan_pages(width, height, layout);
    if pages.is_empty() {
        return Err(format!("nothing to print: the page is empty, the margins leave no room, or it needs more than {MAX_PAGES} pages"));
    }
    let mut out = PdfWriter::new();
    let total = pages.len();
    for (n, slice) in pages.iter().enumerate() {
        let page = out.add_page(layout.paper_width, layout.paper_height);
        let band = rgb[slice.src_y * width * 3..(slice.src_y + slice.src_h) * width * 3].to_vec();
        let y = layout.paper_height - layout.margin - slice.dest_height;
        page.draw_rgb_image(band, width, slice.src_h, layout.margin, y, layout.content_width(), slice.dest_height);
        if layout.header_footer && layout.margin >= 24.0 {
            let shown: String = title.chars().take(90).collect();
            page.draw_text(&shown, layout.margin, layout.paper_height - layout.margin / 2.0, 8.0, Face::Helvetica);
            page.draw_text(&format!("Page {} of {total}", n + 1), layout.margin, layout.margin / 2.0 - 3.0, 8.0, Face::Helvetica);
        }
    }
    Ok(out.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics_pdf::Document;

    #[test]
    fn a_tall_page_is_cut_into_full_pages_and_a_short_last_one() {
        let layout = PrintLayout { paper_width: 200.0, paper_height: 300.0, margin: 20.0, header_footer: false };
        // content 160 x 260 points; a 80 px wide snapshot scales 2 points per pixel, so 130 rows fit a page
        let pages = plan_pages(80, 300, &layout);
        assert_eq!(pages.len(), 3);
        assert_eq!(pages[0], PageSlice { src_y: 0, src_h: 130, dest_height: 260.0 });
        assert_eq!(pages[1].src_y, 130);
        assert_eq!(pages[2], PageSlice { src_y: 260, src_h: 40, dest_height: 80.0 });
        assert_eq!(pages.iter().map(|p| p.src_h).sum::<usize>(), 300);
    }

    #[test]
    fn a_wide_snapshot_scales_down_to_the_content_width() {
        let layout = PrintLayout { paper_width: 200.0, paper_height: 300.0, margin: 20.0, header_footer: false };
        let pages = plan_pages(1600, 1000, &layout);
        // 160 / 1600 = 0.1 points per pixel: 2600 rows fit, so one page 100 points tall
        assert_eq!(pages.len(), 1);
        assert!((pages[0].dest_height - 100.0).abs() < 1e-9);
    }

    #[test]
    fn nothing_to_print_gives_no_pages() {
        let l = PrintLayout::letter();
        assert!(plan_pages(0, 10, &l).is_empty() && plan_pages(10, 0, &l).is_empty());
        assert!(plan_pages(10, 10, &PrintLayout { margin: 400.0, ..l }).is_empty(), "margins leave no room");
        assert!(plan_pages(10, 10, &PrintLayout { margin: -1.0, ..l }).is_empty());
        // 1 px wide scales to 540 pt per pixel: 1 row per page, so 501 rows is over the cap
        assert!(plan_pages(1, MAX_PAGES + 1, &l).is_empty());
        assert_eq!(plan_pages(1, MAX_PAGES, &l).len(), MAX_PAGES);
    }

    #[test]
    fn the_pdf_has_one_sized_page_per_slice_with_title_and_numbers() {
        let layout = PrintLayout { paper_width: 200.0, paper_height: 300.0, margin: 30.0, header_footer: true };
        let (w, h) = (40usize, 220usize);
        let rgb: Vec<u8> = (0..w * h * 3).map(|i| (i % 251) as u8).collect();
        let bytes = pdf_from_rgb(&rgb, w, h, "My page", &layout).unwrap();
        assert!(bytes.starts_with(b"%PDF"));
        let doc = Document::open(bytes).expect("the file reads back");
        // content 140 x 240; 3.5 points per pixel, so 68 rows per page: ceil(220 / 68) = 4 pages
        assert_eq!(doc.page_count(), 4);
        assert_eq!(doc.page_size(0), Some((200.0, 300.0)));
        let first = doc.page_text(0).unwrap();
        assert!(first.contains("My page") && first.contains("Page 1 of 4"), "{first}");
        assert!(doc.page_text(3).unwrap().contains("Page 4 of 4"));
    }

    #[test]
    fn a_bad_snapshot_is_refused_not_drawn() {
        let l = PrintLayout::a4();
        assert!(pdf_from_rgb(&[0; 5], 2, 2, "t", &l).is_err());
        assert!(pdf_from_rgb(&[], 0, 0, "t", &l).is_err());
        assert!(pdf_from_rgb(&[], 100_000, 100_000, "t", &l).unwrap_err().contains("larger"));
    }

    #[test]
    fn transparent_pixels_print_as_white() {
        let rgb = rgba_to_rgb(&[10, 20, 30, 255, 10, 20, 30, 0, 0, 0, 0, 128], 3, 1).unwrap();
        assert_eq!(&rgb[0..3], &[10, 20, 30]);
        assert_eq!(&rgb[3..6], &[255, 255, 255]);
        assert!(rgb[6] > 120 && rgb[6] < 135, "half transparent black is mid grey");
        assert!(rgba_to_rgb(&[0; 7], 1, 2).is_err());
    }
}
