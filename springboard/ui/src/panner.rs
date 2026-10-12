//! Two-dimensional panner pad: a square control that maps a point to a source position (left to right, back to front)
//! and to stereo gains. The pad is the model behind a panner view; it has no drawing of its own. Positions are kept in
//! the unit square, with x running from -1 (left edge) to +1 (right edge) and y from -1 (back edge) to +1 (front edge).
//! Stereo gains use the equal-power law, so the sum of the squared gains stays at 1 across the pad.
//! Pure arithmetic: window-local points are mapped through whatever rectangle the caller gives, so the pad scales with
//! its window.

use crate::Rect;

/// A source position in the unit square. Both coordinates are always finite and inside [-1, 1].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PannerPosition {
    pub x: f32,
    pub y: f32,
}

impl PannerPosition {
    /// The centre of the pad.
    pub const CENTRE: Self = Self { x: 0.0, y: 0.0 };

    /// A position clamped into the unit square. A NaN coordinate becomes 0.
    pub fn new(x: f32, y: f32) -> Self {
        Self { x: clamp_unit(x), y: clamp_unit(y) }
    }

    /// The position under a window-local point, for a pad drawn in `pad`. Points outside the pad are clamped to its
    /// edges, so a drag that leaves the pad keeps the nearest edge. Returns `None` when the pad has no area.
    pub fn from_point(pad: Rect, px: f32, py: f32) -> Option<Self> {
        if !(pad.w > 0.0) || !(pad.h > 0.0) {
            return None;
        }
        let x = 2.0 * (px - pad.x) / pad.w - 1.0;
        let y = 1.0 - 2.0 * (py - pad.y) / pad.h;
        Some(Self::new(x, y))
    }

    /// The point inside `pad` where this position is drawn. The inverse of [`PannerPosition::from_point`].
    pub fn to_point(self, pad: Rect) -> (f32, f32) {
        let px = pad.x + (self.x + 1.0) * 0.5 * pad.w;
        let py = pad.y + (1.0 - self.y) * 0.5 * pad.h;
        (px, py)
    }

    /// Equal-power stereo gains `(left, right)` for this position. The left-to-right position sets the angle from
    /// 0 (hard left) to a quarter turn (hard right); the front-to-back position does not change the stereo gains.
    pub fn stereo_gains(self) -> (f32, f32) {
        let angle = (self.x + 1.0) * 0.5 * std::f32::consts::FRAC_PI_2;
        (angle.cos(), angle.sin())
    }
}

fn clamp_unit(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(-1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad() -> Rect {
        Rect { x: 10.0, y: 20.0, w: 200.0, h: 100.0 }
    }

    #[test]
    fn new_clamps_and_maps_nan_to_zero() {
        assert_eq!(PannerPosition::new(2.0, -3.0), PannerPosition { x: 1.0, y: -1.0 });
        assert_eq!(PannerPosition::new(f32::NAN, 0.5), PannerPosition { x: 0.0, y: 0.5 });
    }

    #[test]
    fn centre_of_pad_is_the_origin() {
        let p = PannerPosition::from_point(pad(), 110.0, 70.0).unwrap();
        assert_eq!(p, PannerPosition::CENTRE);
    }

    #[test]
    fn top_left_is_back_left_and_bottom_right_is_front_right() {
        // Top edge is the front (y = +1) and the left edge is x = -1 in this model.
        let top_left = PannerPosition::from_point(pad(), 10.0, 20.0).unwrap();
        assert_eq!(top_left, PannerPosition { x: -1.0, y: 1.0 });
        let bottom_right = PannerPosition::from_point(pad(), 210.0, 120.0).unwrap();
        assert_eq!(bottom_right, PannerPosition { x: 1.0, y: -1.0 });
    }

    #[test]
    fn points_outside_the_pad_clamp_to_its_edge() {
        let p = PannerPosition::from_point(pad(), 300.0, 70.0).unwrap();
        assert_eq!(p, PannerPosition { x: 1.0, y: 0.0 });
    }

    #[test]
    fn to_point_inverts_from_point() {
        let p = PannerPosition { x: 1.0, y: -1.0 };
        assert_eq!(p.to_point(pad()), (210.0, 120.0));
        let back = PannerPosition::from_point(pad(), 210.0, 120.0).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn a_pad_with_no_area_has_no_position() {
        let flat = Rect { x: 0.0, y: 0.0, w: 0.0, h: 50.0 };
        assert!(PannerPosition::from_point(flat, 1.0, 1.0).is_none());
    }

    #[test]
    fn centre_gains_are_equal_at_minus_three_decibels() {
        let (l, r) = PannerPosition::CENTRE.stereo_gains();
        let half = std::f32::consts::FRAC_1_SQRT_2;
        assert!((l - half).abs() < 1e-6);
        assert!((r - half).abs() < 1e-6);
    }

    #[test]
    fn hard_left_and_hard_right_send_all_signal_to_one_side() {
        let (l, r) = PannerPosition::new(-1.0, 0.0).stereo_gains();
        assert!((l - 1.0).abs() < 1e-6 && r.abs() < 1e-6);
        let (l, r) = PannerPosition::new(1.0, 0.0).stereo_gains();
        assert!(l.abs() < 1e-6 && (r - 1.0).abs() < 1e-6);
    }

    #[test]
    fn stereo_power_is_constant_across_the_pad() {
        for x in [-1.0_f32, -0.3, 0.0, 0.3, 1.0] {
            let (l, r) = PannerPosition::new(x, 0.7).stereo_gains();
            assert!((l * l + r * r - 1.0).abs() < 1e-6, "power at x={x}");
        }
    }
}
