//! The map picture for printing: the tiles on screen drawn into one RGB image, the route line and the pins laid over
//! them, and the PDF page that carries the picture above the printed text.

use bifrost_app::kit::Image;
use bifrost_app::print::{text_pages, PageImage, PdfPage, PrintInfo, TEXT_SIZE};
use springboard::maptiles::compose;

/// Pixels per point of the printed picture. Two keeps the tile edges sharp on paper.
pub const PRINT_SCALE: f64 = 2.0;

/// Draws loaded tiles into an RGB picture of `width` by `height` points at `PRINT_SCALE` pixels a point. Each tile is
/// `side` points square, its top left corner at `(x, y)` from the top left of the map. The tiles are premultiplied
/// pixels; they are laid over white, which is what a tile that has not loaded leaves behind.
pub fn draw_tiles(tiles: &[(f64, f64, &Image)], side: f64, width: f64, height: f64) -> PageImage {
    // the compositor is shared with the SDK's map snapshots, see `springboard::maptiles::compose`
    let picture = compose::draw_tiles(tiles, side, width, height, PRINT_SCALE);
    PageImage { width: picture.width, height: picture.height, rgb: picture.rgb }
}

/// Draws a line `width` points thick through `points` (in points from the top left of the picture) in `rgb`. The line
/// is stamped as discs along each segment, so a diagonal has no gaps. Parts outside the picture are clipped.
pub fn draw_line(picture: &mut PageImage, points: &[(f64, f64)], width: f64, rgb: [u8; 3]) {
    let radius = width * PRINT_SCALE / 2.0;
    for pair in points.windows(2) {
        let (ax, ay) = (pair[0].0 * PRINT_SCALE, pair[0].1 * PRINT_SCALE);
        let (bx, by) = (pair[1].0 * PRINT_SCALE, pair[1].1 * PRINT_SCALE);
        let steps = (bx - ax).abs().max((by - ay).abs()).ceil().max(1.0) as usize;
        for s in 0..=steps {
            let t = s as f64 / steps as f64;
            stamp(picture, ax + (bx - ax) * t, ay + (by - ay) * t, radius, rgb);
        }
    }
}

/// Draws a filled dot of `radius` points centred at `(x, y)` points from the top left of the picture.
pub fn draw_dot(picture: &mut PageImage, x: f64, y: f64, radius: f64, rgb: [u8; 3]) {
    stamp(picture, x * PRINT_SCALE, y * PRINT_SCALE, radius * PRINT_SCALE, rgb);
}

/// Paints the pixels whose centres lie within `radius` pixels of `(x, y)` pixels.
fn stamp(picture: &mut PageImage, x: f64, y: f64, radius: f64, rgb: [u8; 3]) {
    let r = radius.max(0.5);
    let (w, h) = (picture.width as i64, picture.height as i64);
    for oy in (y - r).floor() as i64..=(y + r).ceil() as i64 {
        for ox in (x - r).floor() as i64..=(x + r).ceil() as i64 {
            if ox < 0 || oy < 0 || ox >= w || oy >= h {
                continue;
            }
            let (dx, dy) = (ox as f64 + 0.5 - x, oy as f64 + 0.5 - y);
            if dx * dx + dy * dy <= r * r {
                let i = (oy as usize * picture.width + ox as usize) * 3;
                picture.rgb[i..i + 3].copy_from_slice(&rgb);
            }
        }
    }
}

/// The first page of the print job: the picture scaled to the printable width (or height, when it is taller), centred
/// at the top of the printable area.
pub fn picture_page(picture: PageImage, info: &PrintInfo) -> PdfPage {
    let (paper_w, paper_h) = info.paper_size();
    let (left, top, width, height) = info.imageable();
    let (pic_w, pic_h) = (picture.width as f32 / PRINT_SCALE as f32, picture.height as f32 / PRINT_SCALE as f32);
    let fit = (width / pic_w.max(1.0)).min(height / pic_h.max(1.0));
    let (draw_w, draw_h) = (pic_w * fit, pic_h * fit);
    let x = left + (width - draw_w) / 2.0;
    let y = paper_h - top - draw_h;
    let content = format!("q {draw_w:.2} 0 0 {draw_h:.2} {x:.2} {y:.2} cm /Im0 Do Q\n");
    PdfPage { width: f64::from(paper_w), height: f64::from(paper_h), content: content.into_bytes(), images: vec![picture] }
}

/// The pages of a map print job: the picture first, then the text laid out below it on pages of its own.
pub fn job_pages(picture: Option<PageImage>, text: &str, info: &PrintInfo) -> Vec<PdfPage> {
    let mut pages = Vec::new();
    if let Some(p) = picture {
        pages.push(picture_page(p, info));
    }
    pages.extend(text_pages(text, TEXT_SIZE, info));
    pages
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::kit::pack;

    fn solid(w: usize, h: usize, px: u32) -> Image {
        Image { w, h, px: vec![px; w * h] }
    }

    fn pixel(p: &PageImage, x: usize, y: usize) -> [u8; 3] {
        let i = (y * p.width + x) * 3;
        [p.rgb[i], p.rgb[i + 1], p.rgb[i + 2]]
    }

    #[test]
    fn a_tile_is_drawn_where_it_is_on_the_map_at_print_scale() {
        let red = solid(4, 4, pack(255, 0, 0, 255));
        let picture = draw_tiles(&[(10.0, 5.0, &red)], 10.0, 40.0, 30.0);
        assert_eq!((picture.width, picture.height), (80, 60));
        // the tile covers points 10 to 20 across and 5 to 15 down, which is pixels 20 to 40 and 10 to 30
        assert_eq!(pixel(&picture, 20, 10), [255, 0, 0]);
        assert_eq!(pixel(&picture, 39, 29), [255, 0, 0]);
        assert_eq!(pixel(&picture, 19, 10), [255, 255, 255], "left of the tile is white");
        assert_eq!(pixel(&picture, 40, 10), [255, 255, 255], "right of the tile is white");
    }

    #[test]
    fn a_transparent_tile_lets_the_white_through_and_a_tile_off_the_edge_is_clipped() {
        let clear = solid(2, 2, 0);
        let picture = draw_tiles(&[(0.0, 0.0, &clear)], 10.0, 10.0, 10.0);
        assert_eq!(pixel(&picture, 0, 0), [255, 255, 255]);
        let half_black = solid(2, 2, pack(0, 0, 0, 128));
        let picture = draw_tiles(&[(-5.0, -5.0, &half_black)], 10.0, 10.0, 10.0);
        // the tile starts 5 points above and left: the visible part is the lower right quarter of its pixels
        assert_eq!(pixel(&picture, 0, 0), [127, 127, 127]);
        assert_eq!(pixel(&picture, 19, 19), [255, 255, 255], "beyond the tile's far edge is white");
    }

    #[test]
    fn a_map_with_no_loaded_tiles_is_blank_and_an_empty_image_is_skipped() {
        let blank = draw_tiles(&[], 10.0, 10.0, 10.0);
        assert!(blank.rgb.iter().all(|&c| c == 255));
        let empty = Image { w: 0, h: 0, px: Vec::new() };
        let picture = draw_tiles(&[(0.0, 0.0, &empty)], 10.0, 10.0, 10.0);
        assert!(picture.rgb.iter().all(|&c| c == 255));
    }

    #[test]
    fn the_picture_is_the_first_page_and_the_text_follows() {
        let info = PrintInfo::default();
        let picture = draw_tiles(&[], 256.0, 300.0, 200.0);
        let pages = job_pages(Some(picture), "Perlan\n64.12950, -21.91860\n", &info);
        assert!(pages.len() >= 2);
        let first = String::from_utf8_lossy(&pages[0].content).into_owned();
        assert!(first.contains("/Im0 Do"), "{first}");
        assert_eq!(pages[0].images.len(), 1);
        assert!(pages[1].images.is_empty());
        let text = String::from_utf8_lossy(&pages[1].content).into_owned();
        assert!(text.contains("Perlan"), "{text}");
        // without a picture there is no image page, only the text
        assert!(job_pages(None, "Perlan", &info)[0].images.is_empty());
    }

    #[test]
    fn a_wide_picture_is_scaled_to_the_printable_width() {
        let info = PrintInfo::default();
        let (_, _, width, _) = info.imageable();
        let picture = draw_tiles(&[], 256.0, 2000.0, 100.0);
        let page = picture_page(picture, &info);
        let content = String::from_utf8_lossy(&page.content).into_owned();
        // the first number of the cm operator is the drawn width, which must fit the printable width
        let drawn: f32 = content.split_whitespace().nth(1).and_then(|w| w.parse().ok()).unwrap_or(f32::MAX);
        assert!(drawn <= width + 0.01, "{content}");
    }
}
