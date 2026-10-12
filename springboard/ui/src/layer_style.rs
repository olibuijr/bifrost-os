//! Layer appearance rules: corner radius, masked corners, shadow footprint and the content mask. These are the
//! per-layer properties the reference SDK exposes next to a view's backing layer, as pure geometry. A painter asks
//! this module for the radius it should draw, the area a shadow can touch, and whether a point is inside the mask.
//! Corner names refer to the box's own edges (min or max on each axis). The y axis grows downward, as in the rest of
//! the UI model.
use crate::Rect;

/// A set of box corners, as a bit set. Mirrors the reference SDK's corner mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CornerMask(u8);

impl CornerMask {
    pub const NONE: CornerMask = CornerMask(0);
    pub const MIN_X_MIN_Y: CornerMask = CornerMask(1);
    pub const MAX_X_MIN_Y: CornerMask = CornerMask(2);
    pub const MIN_X_MAX_Y: CornerMask = CornerMask(4);
    pub const MAX_X_MAX_Y: CornerMask = CornerMask(8);
    pub const ALL: CornerMask = CornerMask(15);

    /// The corners in either mask.
    pub fn union(self, other: CornerMask) -> CornerMask {
        CornerMask(self.0 | other.0)
    }

    /// True when every corner of `other` is also in this mask.
    pub fn contains(self, other: CornerMask) -> bool {
        self.0 & other.0 == other.0
    }
}

/// The corners in the order [`corner_radii`] returns their radii.
const CORNERS: [CornerMask; 4] = [
    CornerMask::MIN_X_MIN_Y,
    CornerMask::MAX_X_MIN_Y,
    CornerMask::MIN_X_MAX_Y,
    CornerMask::MAX_X_MAX_Y,
];

/// The radius drawn for a box `w` by `h`. A radius that is not a positive finite number draws square corners, and
/// the radius is capped at half the shorter side, so the corners never overlap.
pub fn effective_radius(radius: f32, w: f32, h: f32) -> f32 {
    if !(radius > 0.0 && radius.is_finite() && w > 0.0 && h > 0.0) {
        return 0.0;
    }
    radius.min(w.min(h) / 2.0)
}

/// The radius of each corner, in the order min-min, max-min, min-max, max-max. Corners outside `mask` get zero.
pub fn corner_radii(radius: f32, w: f32, h: f32, mask: CornerMask) -> [f32; 4] {
    let r = effective_radius(radius, w, h);
    CORNERS.map(|c| if mask.contains(c) { r } else { 0.0 })
}

/// The shadow of a layer: how dark it is, how far it blurs, and how far it is pushed from the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub opacity: f32,
    /// Blur radius in points.
    pub radius: f32,
    /// Offset in points, x then y.
    pub offset: (f32, f32),
}

impl Default for Shadow {
    /// No shadow: zero opacity, no blur, no offset.
    fn default() -> Self {
        Shadow { opacity: 0.0, radius: 0.0, offset: (0.0, 0.0) }
    }
}

impl Shadow {
    /// The opacity clamped to 0..=1. A non-finite opacity reads as no shadow.
    pub fn effective_opacity(&self) -> f32 {
        if self.opacity.is_finite() {
            self.opacity.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// True when the shadow has any opacity to paint.
    pub fn is_visible(&self) -> bool {
        self.effective_opacity() > 0.0
    }

    /// The area the shadow can paint: the frame moved by the offset and grown by the blur radius on every side.
    /// A negative or non-finite blur counts as zero, and a non-finite offset counts as no offset.
    pub fn footprint(&self, frame: Rect) -> Rect {
        let blur = if self.radius.is_finite() { self.radius.max(0.0) } else { 0.0 };
        let (dx, dy) = if self.offset.0.is_finite() && self.offset.1.is_finite() { self.offset } else { (0.0, 0.0) };
        Rect { x: frame.x + dx - blur, y: frame.y + dy - blur, w: frame.w + 2.0 * blur, h: frame.h + 2.0 * blur }
    }

    /// The area to repaint for a layer drawn with its shadow: the shadow footprint united with the frame.
    pub fn repaint_rect(&self, frame: Rect) -> Rect {
        let f = self.footprint(frame);
        let x0 = f.x.min(frame.x);
        let y0 = f.y.min(frame.y);
        let x1 = (f.x + f.w).max(frame.x + frame.w);
        let y1 = (f.y + f.h).max(frame.y + frame.h);
        Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    }
}

/// Whether a point is inside a layer's content mask: the frame with each masked corner cut by a circle of the
/// effective radius. A point in an unmasked corner, or on an edge, is inside. The frame is half open, as
/// [`Rect::contains`] is.
pub fn mask_contains(frame: Rect, radius: f32, mask: CornerMask, x: f32, y: f32) -> bool {
    if !frame.contains(x, y) {
        return false;
    }
    let r = corner_radii(radius, frame.w, frame.h, mask);
    let (x0, y0) = (frame.x, frame.y);
    let (x1, y1) = (frame.x + frame.w, frame.y + frame.h);
    // The centre of each corner's circle, in the same order as `r`.
    let centres = [
        (x0 + r[0], y0 + r[0]),
        (x1 - r[1], y0 + r[1]),
        (x0 + r[2], y1 - r[2]),
        (x1 - r[3], y1 - r[3]),
    ];
    for i in 0..4 {
        if r[i] <= 0.0 {
            continue;
        }
        let (cx, cy) = centres[i];
        let in_corner = match i {
            0 => x < cx && y < cy,
            1 => x > cx && y < cy,
            2 => x < cx && y > cy,
            _ => x > cx && y > cy,
        };
        if in_corner && (x - cx).powi(2) + (y - cy).powi(2) > r[i] * r[i] {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    fn frame(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn corner_mask_combines_and_tests_corners() {
        let top_left = CornerMask::MIN_X_MIN_Y;
        let bottom_right = CornerMask::MAX_X_MAX_Y;
        assert!(CornerMask::ALL.contains(top_left));
        assert!(!top_left.contains(bottom_right));
        assert!(!top_left.union(bottom_right).contains(CornerMask::ALL));
        assert!(top_left.union(bottom_right).contains(bottom_right));
        assert!(CornerMask::NONE.contains(CornerMask::NONE));
    }

    #[test]
    fn radius_is_capped_at_half_the_shorter_side() {
        assert_eq!(effective_radius(20.0, 10.0, 40.0), 5.0);
        assert_eq!(effective_radius(4.0, 10.0, 40.0), 4.0);
        assert_eq!(effective_radius(-3.0, 10.0, 40.0), 0.0);
        assert_eq!(effective_radius(f32::NAN, 10.0, 40.0), 0.0);
        assert_eq!(effective_radius(6.0, 0.0, 40.0), 0.0);
    }

    #[test]
    fn corner_radii_only_draw_the_masked_corners() {
        let mask = CornerMask::MIN_X_MIN_Y.union(CornerMask::MAX_X_MAX_Y);
        assert_eq!(corner_radii(6.0, 20.0, 20.0, mask), [6.0, 0.0, 0.0, 6.0]);
        assert_eq!(corner_radii(30.0, 20.0, 20.0, CornerMask::ALL), [10.0, 10.0, 10.0, 10.0]);
    }

    #[test]
    fn shadow_footprint_moves_with_the_offset_and_grows_with_the_blur() {
        let s = Shadow { opacity: 0.5, radius: 3.0, offset: (2.0, -1.0) };
        let f = s.footprint(frame(10.0, 10.0, 100.0, 50.0));
        // x: 10 + 2 - 3 = 9, y: 10 - 1 - 3 = 6, width 100 + 6, height 50 + 6.
        assert!(near(f.x, 9.0) && near(f.y, 6.0) && near(f.w, 106.0) && near(f.h, 56.0));
    }

    #[test]
    fn repaint_rect_covers_the_frame_and_the_shadow() {
        // The shadow is pushed 20 to the left and does not blur, so the repaint runs from x -10 to the frame's right.
        let s = Shadow { opacity: 1.0, radius: 0.0, offset: (-20.0, 0.0) };
        let r = s.repaint_rect(frame(10.0, 10.0, 100.0, 50.0));
        assert!(near(r.x, -10.0) && near(r.y, 10.0) && near(r.w, 120.0) && near(r.h, 50.0));
    }

    #[test]
    fn shadow_opacity_is_clamped_and_non_finite_means_none() {
        let with = |o: f32| Shadow { opacity: o, ..Shadow::default() };
        assert_eq!(with(1.7).effective_opacity(), 1.0);
        assert_eq!(with(-0.2).effective_opacity(), 0.0);
        assert_eq!(with(f32::NAN).effective_opacity(), 0.0);
        assert!(with(0.4).is_visible());
        assert!(!Shadow::default().is_visible());
    }

    #[test]
    fn mask_cuts_the_masked_corners_by_the_radius() {
        let f = frame(0.0, 0.0, 10.0, 10.0);
        // Radius 4 with all corners: the corner point (0.5, 0.5) is 4.95 from the circle centre (4, 4), outside.
        assert!(!mask_contains(f, 4.0, CornerMask::ALL, 0.5, 0.5));
        // (2, 2) is 2.83 from (4, 4), inside. The middle of the box is never cut.
        assert!(mask_contains(f, 4.0, CornerMask::ALL, 2.0, 2.0));
        assert!(mask_contains(f, 4.0, CornerMask::ALL, 5.0, 5.0));
        // The same corner point is inside when that corner is not masked.
        assert!(mask_contains(f, 4.0, CornerMask::MAX_X_MIN_Y, 0.5, 0.5));
        // The far corner is cut only when it is in the mask: (9.5, 9.5) is 4.95 from (6, 6) and cut under ALL.
        assert!(!mask_contains(f, 4.0, CornerMask::ALL, 9.5, 9.5));
        assert!(mask_contains(f, 4.0, CornerMask::MIN_X_MIN_Y, 9.5, 9.5));
        assert!(!mask_contains(f, 4.0, CornerMask::MAX_X_MAX_Y, 9.5, 9.5));
    }

    #[test]
    fn mask_is_half_open_and_radius_is_capped() {
        let f = frame(0.0, 0.0, 10.0, 10.0);
        // The right edge is outside the frame.
        assert!(!mask_contains(f, 0.0, CornerMask::NONE, 10.0, 5.0));
        // A radius of 20 is capped to 5, so the corner circle has centre (5, 5): (0.5, 0.5) is out, (2.5, 2.5) is in.
        assert!(!mask_contains(f, 20.0, CornerMask::ALL, 0.5, 0.5));
        assert!(mask_contains(f, 20.0, CornerMask::ALL, 2.5, 2.5));
    }
}
