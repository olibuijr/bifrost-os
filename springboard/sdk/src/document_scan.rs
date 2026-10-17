//! Document scanning for apps (`VNDocumentCameraViewController` and its scan): the corners of a page found in a camera
//! frame become a flat page of known size.
//!
//! [`DocumentScan`] keeps the captured pages in order, and [`flatten`] warps a frame's RGBA pixels onto a page by inverse
//! mapping, taking the nearest frame pixel for each page pixel. Finding the page corners in the frame is the camera's
//! job and is not done here. Limits: nearest-pixel sampling (no filtering), and the corners must be in camera points
//! of the frame that [`flatten`] is given.

pub use bifrost_ui::docscan::{apply, homography, order_corners, page_size, DocumentScan, Homography, Page};
pub use bifrost_ui::scan::Point;

/// The flat page for `page` cut from `frame`, which is `frame_w` x `frame_h` RGBA pixels, row-major. Each page pixel
/// takes the frame pixel under its centre. A page pixel whose source lies outside the frame stays transparent black.
/// `None` when the frame's bytes do not match its size, or the page's corners are degenerate.
pub fn flatten(frame: &[u8], frame_w: usize, frame_h: usize, page: &Page) -> Option<Vec<u8>> {
    if frame_w == 0 || frame_h == 0 || frame.len() != frame_w * frame_h * 4 {
        return None;
    }
    let (pw, ph) = (page.width as usize, page.height as usize);
    let (w, h) = (page.width as f32, page.height as f32);
    let rect = [Point { x: 0.0, y: 0.0 }, Point { x: w, y: 0.0 }, Point { x: w, y: h }, Point { x: 0.0, y: h }];
    // The map goes from the page rectangle to the camera quadrilateral, so it maps each page pixel to its source.
    let map = homography(rect, page.corners)?;
    let mut out = vec![0u8; pw * ph * 4];
    for v in 0..ph {
        for u in 0..pw {
            let p = apply(&map, Point { x: u as f32 + 0.5, y: v as f32 + 0.5 });
            let (x, y) = (p.x.floor(), p.y.floor());
            if x < 0.0 || y < 0.0 || x >= frame_w as f32 || y >= frame_h as f32 {
                continue;
            }
            let src = (y as usize * frame_w + x as usize) * 4;
            let dst = (v * pw + u) * 4;
            out[dst..dst + 4].copy_from_slice(&frame[src..src + 4]);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page_over(width: u32, height: u32, corners: [Point; 4]) -> Page {
        Page { corners, width, height }
    }

    fn corner(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    #[test]
    fn a_page_over_the_whole_frame_is_the_frame() {
        let frame: Vec<u8> = (0..4 * 4 * 4).map(|i| i as u8).collect();
        let page = page_over(4, 4, [corner(0.0, 0.0), corner(4.0, 0.0), corner(4.0, 4.0), corner(0.0, 4.0)]);
        assert_eq!(flatten(&frame, 4, 4, &page), Some(frame));
    }

    #[test]
    fn a_page_inside_the_frame_takes_only_its_pixels_and_mismatched_frames_are_refused() {
        // Frame 4x4 with each pixel's red channel set to its column, so the page's columns show through.
        let mut frame = vec![0u8; 4 * 4 * 4];
        for y in 0..4 {
            for x in 0..4 {
                frame[(y * 4 + x) * 4] = x as u8;
                frame[(y * 4 + x) * 4 + 3] = 255;
            }
        }
        let page = page_over(2, 2, [corner(2.0, 0.0), corner(4.0, 0.0), corner(4.0, 2.0), corner(2.0, 2.0)]);
        let out = flatten(&frame, 4, 4, &page).expect("flat page");
        assert_eq!(out.len(), 2 * 2 * 4);
        assert_eq!((out[0], out[4]), (2, 3), "the top row takes frame columns 2 and 3");
        assert!(flatten(&frame[..10], 4, 4, &page).is_none());
    }

    #[test]
    fn degenerate_corners_give_no_page() {
        let frame = vec![0u8; 4 * 4 * 4];
        let line = page_over(2, 2, [corner(0.0, 0.0), corner(1.0, 0.0), corner(2.0, 0.0), corner(3.0, 0.0)]);
        assert_eq!(flatten(&frame, 4, 4, &line), None);
    }
}
