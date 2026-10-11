//! Document scanning geometry: the page model of a system document scanner. A page is a quadrilateral
//! found in the camera image (its corners in camera points). Scanning turns it into a flat page of known size by
//! a perspective warp, which this module computes: corner ordering, output page size, the 3x3 homography between
//! the page rectangle and the quadrilateral, and a session that holds the scanned pages. Finding the page edges
//! in pixels is the camera's job; this module takes the corners it finds.

use crate::scan::Point;

/// A projective map, row-major `[a, b, c, d, e, f, g, h]` with the ninth entry fixed at 1:
/// `u = (a x + b y + c) / w`, `v = (d x + e y + f) / w`, `w = g x + h y + 1`.
pub type Homography = [f64; 8];

/// Orders four page corners as top-left, top-right, bottom-right, bottom-left. This is exact for the corners of
/// a page seen at any perspective that keeps it convex and not rotated past 45 degrees; the four corners are
/// chosen by extreme sums and differences of x and y.
pub fn order_corners(p: [Point; 4]) -> [Point; 4] {
    let sum = |q: &Point| q.x + q.y;
    let diff = |q: &Point| q.x - q.y;
    let tl = *p.iter().min_by(|a, b| sum(a).total_cmp(&sum(b))).unwrap_or(&p[0]);
    let br = *p.iter().max_by(|a, b| sum(a).total_cmp(&sum(b))).unwrap_or(&p[0]);
    let tr = *p.iter().max_by(|a, b| diff(a).total_cmp(&diff(b))).unwrap_or(&p[0]);
    let bl = *p.iter().min_by(|a, b| diff(a).total_cmp(&diff(b))).unwrap_or(&p[0]);
    [tl, tr, br, bl]
}

fn dist(a: Point, b: Point) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

/// The output size of a page with ordered `corners`, scaled so its longer side is at most `max_side` pixels and
/// its aspect ratio is kept. The size is the longer of each pair of opposite edges. `None` when a side is under
/// one pixel or `max_side` is not positive.
pub fn page_size(corners: [Point; 4], max_side: u32) -> Option<(u32, u32)> {
    if max_side == 0 {
        return None;
    }
    let [tl, tr, br, bl] = corners;
    let w = dist(tl, tr).max(dist(bl, br));
    let h = dist(tl, bl).max(dist(tr, br));
    if w < 1.0 || h < 1.0 {
        return None;
    }
    let scale = (max_side as f32 / w.max(h)).min(1.0);
    let (w, h) = ((w * scale).round() as u32, (h * scale).round() as u32);
    (w >= 1 && h >= 1).then_some((w, h))
}

/// Solves the eight-unknown linear system for the homography that maps each `from[i]` to `to[i]`. `None` when
/// the points are degenerate (for example three of them on a line), so no unique map exists.
pub fn homography(from: [Point; 4], to: [Point; 4]) -> Option<Homography> {
    // Each correspondence gives two rows: [x y 1 0 0 0 -u*x -u*y | u] and [0 0 0 x y 1 -v*x -v*y | v].
    let mut m = [[0.0f64; 9]; 8];
    for i in 0..4 {
        let (x, y) = (from[i].x as f64, from[i].y as f64);
        let (u, v) = (to[i].x as f64, to[i].y as f64);
        m[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
        m[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
    }
    // Gauss-Jordan elimination with partial pivoting.
    for col in 0..8 {
        let pivot = (col..8).max_by(|&a, &b| m[a][col].abs().total_cmp(&m[b][col].abs()))?;
        if m[pivot][col].abs() < 1e-9 {
            return None;
        }
        m.swap(col, pivot);
        let p = m[col][col];
        for v in m[col].iter_mut() {
            *v /= p;
        }
        for row in 0..8 {
            if row != col {
                let f = m[row][col];
                if f != 0.0 {
                    for k in col..9 {
                        m[row][k] -= f * m[col][k];
                    }
                }
            }
        }
    }
    let mut h = [0.0f64; 8];
    for (i, slot) in h.iter_mut().enumerate() {
        *slot = m[i][8];
    }
    Some(h)
}

/// Maps one point through a homography.
pub fn apply(h: &Homography, p: Point) -> Point {
    let (x, y) = (p.x as f64, p.y as f64);
    let w = h[6] * x + h[7] * y + 1.0;
    Point {
        x: ((h[0] * x + h[1] * y + h[2]) / w) as f32,
        y: ((h[3] * x + h[4] * y + h[5]) / w) as f32,
    }
}

/// One scanned page: its corners in camera points, and the size of the flat page it becomes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Page {
    pub corners: [Point; 4],
    pub width: u32,
    pub height: u32,
}

impl Page {
    /// The camera point under output pixel `(u, v)` of the flat page. Pixel `(0, 0)` is the top-left corner.
    /// `None` when the corners are degenerate.
    pub fn source_point(&self, u: f32, v: f32) -> Option<Point> {
        let w = self.width as f32;
        let h = self.height as f32;
        let rect = [
            Point { x: 0.0, y: 0.0 },
            Point { x: w, y: 0.0 },
            Point { x: w, y: h },
            Point { x: 0.0, y: h },
        ];
        // The homography goes from the page rectangle to the camera quadrilateral, so it maps output to camera.
        let map = homography(rect, self.corners)?;
        Some(apply(&map, Point { x: u, y: v }))
    }
}

/// The pages of one document scan, in the order they were captured.
#[derive(Debug, Default)]
pub struct DocumentScan {
    pages: Vec<Page>,
}

impl DocumentScan {
    pub fn new() -> Self {
        Self::default()
    }

    /// Captures a page from its camera corners and returns its index. A page with degenerate corners or a zero
    /// `max_side` is rejected with `None`, and the scan is left unchanged.
    pub fn add_page(&mut self, corners: [Point; 4], max_side: u32) -> Option<usize> {
        let corners = order_corners(corners);
        let (width, height) = page_size(corners, max_side)?;
        self.pages.push(Page { corners, width, height });
        Some(self.pages.len() - 1)
    }

    /// Removes the page at `index`. Returns false when there is no such page.
    pub fn remove_page(&mut self, index: usize) -> bool {
        if index < self.pages.len() {
            self.pages.remove(index);
            true
        } else {
            false
        }
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn pages(&self) -> &[Page] {
        &self.pages
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    fn close(a: Point, b: Point) -> bool {
        (a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3
    }

    #[test]
    fn corners_are_ordered_whatever_order_they_arrive_in() {
        let shuffled = [p(10.0, 90.0), p(90.0, 10.0), p(10.0, 10.0), p(90.0, 90.0)];
        assert_eq!(order_corners(shuffled), [p(10.0, 10.0), p(90.0, 10.0), p(90.0, 90.0), p(10.0, 90.0)]);
    }

    #[test]
    fn a_page_size_keeps_the_aspect_ratio_and_fits_the_longer_side() {
        // An A4-shaped page of 210 x 297 units, scaled so the longer side is 1000 pixels.
        let a4 = [p(0.0, 0.0), p(210.0, 0.0), p(210.0, 297.0), p(0.0, 297.0)];
        assert_eq!(page_size(a4, 1000), Some((707, 1000)));
        // A page smaller than the limit is not enlarged.
        assert_eq!(page_size(a4, 5000), Some((210, 297)));
        assert_eq!(page_size(a4, 0), None);
        assert_eq!(page_size([p(0.0, 0.0); 4], 100), None, "a point has no size");
    }

    #[test]
    fn a_homography_maps_its_source_corners_onto_the_target_corners() {
        let from = [p(0.0, 0.0), p(100.0, 0.0), p(100.0, 100.0), p(0.0, 100.0)];
        // A trapezoid: the top edge is narrower, as a page seen at an angle is.
        let to = [p(20.0, 10.0), p(80.0, 15.0), p(100.0, 90.0), p(0.0, 95.0)];
        let h = homography(from, to).expect("non-degenerate");
        for i in 0..4 {
            assert!(close(apply(&h, from[i]), to[i]), "corner {i} lands on its target");
        }
        // An affine case: scaling the unit square by 2 maps the centre to the centre of the target square.
        let scale = homography([p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)], [p(0.0, 0.0), p(2.0, 0.0), p(2.0, 2.0), p(0.0, 2.0)]).unwrap();
        assert!(close(apply(&scale, p(0.5, 0.5)), p(1.0, 1.0)));
    }

    #[test]
    fn collinear_corners_have_no_homography() {
        // Three source corners on one line leave the eight equations rank-deficient: no unique map.
        let line = [p(0.0, 0.0), p(1.0, 1.0), p(2.0, 2.0), p(3.0, 3.0)];
        let square = [p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)];
        assert!(homography(line, square).is_none());
    }

    #[test]
    fn a_page_samples_its_flat_pixels_from_the_camera_quad() {
        let mut scan = DocumentScan::new();
        // A rotated, shifted page in camera points, supplied in a scrambled order.
        let index = scan.add_page([p(120.0, 40.0), p(20.0, 50.0), p(10.0, 150.0), p(130.0, 140.0)], 400).unwrap();
        let page = scan.pages()[index];
        assert_eq!((page.width, page.height), (page_size(page.corners, 400).unwrap()));
        // The output top-left pixel maps to the top-left camera corner, and the bottom-right pixel to bottom-right.
        assert!(close(page.source_point(0.0, 0.0).unwrap(), p(20.0, 50.0)));
        let br = page.source_point(page.width as f32, page.height as f32).unwrap();
        assert!(close(br, p(130.0, 140.0)));
    }

    #[test]
    fn a_scan_keeps_its_pages_in_capture_order_and_rejects_bad_pages() {
        let mut scan = DocumentScan::new();
        let a = scan.add_page([p(0.0, 0.0), p(50.0, 0.0), p(50.0, 70.0), p(0.0, 70.0)], 100).unwrap();
        let b = scan.add_page([p(0.0, 0.0), p(60.0, 0.0), p(60.0, 60.0), p(0.0, 60.0)], 100).unwrap();
        assert_eq!((a, b, scan.page_count()), (0, 1, 2));
        assert_eq!(scan.add_page([p(5.0, 5.0); 4], 100), None, "degenerate page is refused");
        assert_eq!(scan.page_count(), 2, "a refused page changes nothing");
        assert!(scan.remove_page(0));
        assert_eq!(scan.pages()[0].width, 60, "the second page moved down to index 0");
        assert!(!scan.remove_page(5));
    }
}
