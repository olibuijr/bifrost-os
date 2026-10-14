//! The canvas view of a board: zoom and pan between screen and board coordinates, zooming about a point (pinch or
//! cmd +/-), fit-to-content, connector geometry between two items, and filtering the board list by title.
//! Pure std on plain `(x, y, w, h)` rectangles.
#![allow(dead_code)] // the Zoom menu, pinch handling, Insert > Connector and the board search field wire this in

pub const ZOOM_MIN: f32 = 0.1;
pub const ZOOM_MAX: f32 = 8.0;
const STEP: f32 = 1.25;

/// A rectangle `(x, y, w, h)`.
pub type Rect = (f32, f32, f32, f32);

/// Zoom factor and the board point shown at the screen origin: `screen = (board - origin) * zoom`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub zoom: f32,
    pub origin: (f32, f32),
}

impl Default for View {
    fn default() -> Self {
        View { zoom: 1.0, origin: (0.0, 0.0) }
    }
}

impl View {
    pub fn to_screen(&self, p: (f32, f32)) -> (f32, f32) {
        ((p.0 - self.origin.0) * self.zoom, (p.1 - self.origin.1) * self.zoom)
    }

    pub fn to_board(&self, p: (f32, f32)) -> (f32, f32) {
        (p.0 / self.zoom + self.origin.0, p.1 / self.zoom + self.origin.1)
    }

    /// Pans by a drag of `d` screen pixels (content follows the pointer).
    pub fn pan(&mut self, d: (f32, f32)) {
        self.origin.0 -= d.0 / self.zoom;
        self.origin.1 -= d.1 / self.zoom;
    }

    /// Zooms to `zoom` keeping the board point under the screen point `anchor` fixed.
    pub fn zoom_about(&mut self, anchor: (f32, f32), zoom: f32) {
        let zoom = zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        let fixed = self.to_board(anchor);
        self.zoom = zoom;
        self.origin = (fixed.0 - anchor.0 / zoom, fixed.1 - anchor.1 / zoom);
    }

    pub fn zoom_in(&mut self, anchor: (f32, f32)) {
        self.zoom_about(anchor, self.zoom * STEP);
    }

    pub fn zoom_out(&mut self, anchor: (f32, f32)) {
        self.zoom_about(anchor, self.zoom / STEP);
    }

    /// Actual size (cmd+0), keeping the anchor fixed.
    pub fn reset(&mut self, anchor: (f32, f32)) {
        self.zoom_about(anchor, 1.0);
    }

    /// Fits `content` into a `w` by `h` screen with `margin` pixels around it.
    pub fn fit(content: Rect, w: f32, h: f32, margin: f32) -> View {
        let (cx, cy, cw, ch) = content;
        if cw <= 0.0 || ch <= 0.0 || w <= 2.0 * margin || h <= 2.0 * margin {
            return View::default();
        }
        let zoom = ((w - 2.0 * margin) / cw).min((h - 2.0 * margin) / ch).clamp(ZOOM_MIN, ZOOM_MAX);
        View { zoom, origin: (cx - (w / zoom - cw) / 2.0, cy - (h / zoom - ch) / 2.0) }
    }

    /// The board rectangle visible on a `w` by `h` screen.
    pub fn visible(&self, w: f32, h: f32) -> Rect {
        (self.origin.0, self.origin.1, w / self.zoom, h / self.zoom)
    }
}

/// The bounding box of rectangles.
pub fn bounds(rects: &[Rect]) -> Option<Rect> {
    let first = rects.first()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first.0, first.1, first.0 + first.2, first.1 + first.3);
    for r in rects {
        x0 = x0.min(r.0);
        y0 = y0.min(r.1);
        x1 = x1.max(r.0 + r.2);
        y1 = y1.max(r.1 + r.3);
    }
    Some((x0, y0, x1 - x0, y1 - y0))
}

pub fn centre(r: Rect) -> (f32, f32) {
    (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0)
}

/// The point where the line from the centre of `r` towards `target` leaves `r`.
pub fn edge_point(r: Rect, target: (f32, f32)) -> (f32, f32) {
    let c = centre(r);
    let (dx, dy) = (target.0 - c.0, target.1 - c.1);
    if dx == 0.0 && dy == 0.0 {
        return c;
    }
    let (hw, hh) = (r.2 / 2.0, r.3 / 2.0);
    let sx = if dx != 0.0 { hw / dx.abs() } else { f32::INFINITY };
    let sy = if dy != 0.0 { hh / dy.abs() } else { f32::INFINITY };
    let s = sx.min(sy);
    (c.0 + dx * s, c.1 + dy * s)
}

/// A straight connector between two items, from the edge of one to the edge of the other.
pub fn connector(a: Rect, b: Rect) -> ((f32, f32), (f32, f32)) {
    (edge_point(a, centre(b)), edge_point(b, centre(a)))
}

/// The distance from `p` to the segment `a`-`b`; a click within a few pixels selects the connector.
pub fn distance_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let len2 = vx * vx + vy * vy;
    let t = if len2 == 0.0 { 0.0 } else { (((p.0 - a.0) * vx + (p.1 - a.1) * vy) / len2).clamp(0.0, 1.0) };
    let (qx, qy) = (a.0 + t * vx, a.1 + t * vy);
    ((p.0 - qx).powi(2) + (p.1 - qy).powi(2)).sqrt()
}

/// The arrowhead's two wing points at `tip` for a line arriving from `from`, with the given length and half angle.
pub fn arrow_wings(from: (f32, f32), tip: (f32, f32), len: f32, half_angle: f32) -> ((f32, f32), (f32, f32)) {
    let back = (from.1 - tip.1).atan2(from.0 - tip.0);
    let wing = |a: f32| (tip.0 + len * (back + a).cos(), tip.1 + len * (back + a).sin());
    (wing(half_angle), wing(-half_angle))
}

/// Indices of the titles containing every word of `query` (case-insensitive). Empty query: all.
pub fn search_titles(titles: &[&str], query: &str) -> Vec<usize> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    titles
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            let t = t.to_lowercase();
            words.iter().all(|w| t.contains(w.as_str()))
        })
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn screen_and_board_are_inverse() {
        let v = View { zoom: 2.0, origin: (10.0, 20.0) };
        let p = (30.0, 50.0);
        assert!(near(v.to_board(v.to_screen(p)), p));
        assert!(near(v.to_screen((10.0, 20.0)), (0.0, 0.0)));
    }

    #[test]
    fn zoom_keeps_the_anchor_fixed() {
        let mut v = View::default();
        v.pan((-100.0, -50.0));
        let anchor = (200.0, 120.0);
        let before = v.to_board(anchor);
        v.zoom_in(anchor);
        assert!((v.zoom - 1.25).abs() < 1e-6);
        assert!(near(v.to_board(anchor), before));
        v.zoom_about(anchor, 100.0);
        assert_eq!(v.zoom, ZOOM_MAX);
        v.reset(anchor);
        assert_eq!(v.zoom, 1.0);
        assert!(near(v.to_board(anchor), before));
    }

    #[test]
    fn panning_moves_content_with_the_pointer() {
        let mut v = View { zoom: 2.0, origin: (0.0, 0.0) };
        let p = v.to_screen((10.0, 10.0));
        v.pan((30.0, -10.0));
        assert!(near(v.to_screen((10.0, 10.0)), (p.0 + 30.0, p.1 - 10.0)));
    }

    #[test]
    fn fit_centres_the_content() {
        let v = View::fit((100.0, 100.0, 200.0, 100.0), 440.0, 300.0, 20.0);
        assert_eq!(v.zoom, 2.0);
        let (sx, sy) = v.to_screen(centre((100.0, 100.0, 200.0, 100.0)));
        assert!(near((sx, sy), (220.0, 150.0)));
        assert_eq!(View::fit((0.0, 0.0, 0.0, 5.0), 100.0, 100.0, 5.0), View::default());
        assert_eq!(bounds(&[(0.0, 0.0, 10.0, 10.0), (20.0, 5.0, 5.0, 30.0)]), Some((0.0, 0.0, 25.0, 35.0)));
        assert_eq!(bounds(&[]), None);
    }

    #[test]
    fn connectors_leave_through_the_facing_edges() {
        let a = (0.0, 0.0, 100.0, 50.0);
        let b = (300.0, 0.0, 100.0, 50.0);
        let (p, q) = connector(a, b);
        assert!(near(p, (100.0, 25.0)));
        assert!(near(q, (300.0, 25.0)));
        // diagonal: leaves through the bottom of a
        let c = (0.0, 200.0, 100.0, 50.0);
        let (p2, _) = connector(a, c);
        assert!(near(p2, (50.0, 50.0)));
        assert!(near(edge_point(a, centre(a)), centre(a)));
    }

    #[test]
    fn segment_distance_and_arrow() {
        assert!((distance_to_segment((5.0, 3.0), (0.0, 0.0), (10.0, 0.0)) - 3.0).abs() < 1e-5);
        assert!((distance_to_segment((-4.0, 3.0), (0.0, 0.0), (10.0, 0.0)) - 5.0).abs() < 1e-5);
        assert!((distance_to_segment((1.0, 1.0), (2.0, 2.0), (2.0, 2.0)) - 2f32.sqrt()).abs() < 1e-5);
        let (w1, w2) = arrow_wings((0.0, 0.0), (10.0, 0.0), 5.0, std::f32::consts::FRAC_PI_6);
        assert!(w1.0 < 10.0 && w2.0 < 10.0);
        assert!((w1.1 + w2.1).abs() < 1e-4);
    }

    #[test]
    fn board_search() {
        let titles = ["Trip planning", "Budget 2026", "Planning the budget"];
        assert_eq!(search_titles(&titles, "plan"), vec![0, 2]);
        assert_eq!(search_titles(&titles, "BUDGET plan"), vec![2]);
        assert_eq!(search_titles(&titles, ""), vec![0, 1, 2]);
    }
}
