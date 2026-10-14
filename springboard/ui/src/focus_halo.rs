//! Focus halo (`UIFocusHaloEffect`): a ring drawn around the view that has keyboard focus, so a keyboard user can see where
//! the focus is. The halo sits outside the view with a small gap, and its corners follow the view's corners. Pure
//! geometry: the caller draws the two rounded rectangles in the focus colour, and the ring is the area between them.
//!
//! The corner radius is capped at `MAX_RADIUS_PT`, because the interface uses flat corners and never a pill shape.

use crate::Rect;

/// Width of the ring itself, in points.
pub const HALO_WIDTH_PT: f32 = 3.0;
/// Gap between the view's edge and the inner edge of the ring, in points.
pub const HALO_GAP_PT: f32 = 2.0;
/// Largest corner radius any part of the halo uses, in points.
pub const MAX_RADIUS_PT: f32 = 6.0;

/// The two edges of the halo ring, with the radius each edge uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Halo {
    /// The inner edge of the ring, just outside the view.
    pub inner: Rect,
    pub inner_radius: f32,
    /// The outer edge of the ring.
    pub outer: Rect,
    pub outer_radius: f32,
}

/// The halo for a focused view of frame `target` whose corners have radius `corner_radius`. `None` for an empty frame.
pub fn halo(target: Rect, corner_radius: f32) -> Option<Halo> {
    if target.w <= 0.0 || target.h <= 0.0 {
        return None;
    }
    let base = corner_radius.max(0.0);
    let inset = |d: f32| Rect { x: target.x - d, y: target.y - d, w: target.w + 2.0 * d, h: target.h + 2.0 * d };
    let inner = inset(HALO_GAP_PT);
    let outer = inset(HALO_GAP_PT + HALO_WIDTH_PT);
    Some(Halo {
        inner,
        inner_radius: (base + HALO_GAP_PT).min(MAX_RADIUS_PT),
        outer,
        outer_radius: (base + HALO_GAP_PT + HALO_WIDTH_PT).min(MAX_RADIUS_PT),
    })
}

/// True when `point` lies inside the ring: inside the outer edge and outside the inner edge.
pub fn in_ring(h: &Halo, x: f32, y: f32) -> bool {
    h.outer.contains(x, y) && !h.inner.contains(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: Rect = Rect { x: 100.0, y: 50.0, w: 80.0, h: 40.0 };

    #[test]
    fn the_ring_surrounds_the_view_at_gap_and_width() {
        let h = halo(VIEW, 4.0).unwrap();
        assert_eq!(h.inner, Rect { x: 98.0, y: 48.0, w: 84.0, h: 44.0 });
        assert_eq!(h.outer, Rect { x: 95.0, y: 45.0, w: 90.0, h: 50.0 });
    }

    #[test]
    fn radii_follow_the_corners_and_stop_at_the_flat_limit() {
        assert_eq!(halo(VIEW, 0.0).unwrap().inner_radius, HALO_GAP_PT);
        assert_eq!(halo(VIEW, 0.0).unwrap().outer_radius, HALO_GAP_PT + HALO_WIDTH_PT);
        let big = halo(VIEW, 40.0).unwrap();
        assert_eq!(big.inner_radius, MAX_RADIUS_PT);
        assert_eq!(big.outer_radius, MAX_RADIUS_PT);
    }

    #[test]
    fn an_empty_frame_has_no_halo() {
        assert_eq!(halo(Rect { x: 0.0, y: 0.0, w: 0.0, h: 10.0 }, 2.0), None);
        assert_eq!(halo(Rect { x: 0.0, y: 0.0, w: 10.0, h: -1.0 }, 2.0), None);
    }

    #[test]
    fn the_ring_excludes_the_view_and_the_gap() {
        let h = halo(VIEW, 4.0).unwrap();
        assert!(!in_ring(&h, 120.0, 60.0));
        assert!(!in_ring(&h, 99.0, 60.0));
        assert!(in_ring(&h, 96.0, 60.0));
        assert!(!in_ring(&h, 94.0, 60.0));
    }
}
