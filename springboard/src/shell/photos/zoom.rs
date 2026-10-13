//! Pure zoom and pan arithmetic for the viewer: pinch about a focal point,
//! pan bounds, rubber-banding past them and the double-tap targets.

pub const MIN_ZOOM: f32 = 1.0;
pub const MAX_ZOOM: f32 = 5.0;
/// The magnification a double tap goes to.
pub const DOUBLE_TAP_ZOOM: f32 = 2.0;
/// How far a rubber band can stretch, as a fraction of the viewport.
pub const BAND_FRACTION: f32 = 0.25;

/// How far the picture can be panned from centre: half of what overflows.
pub fn max_pan(fit: f32, zoom: f32, view: f32) -> f32 {
    ((fit * zoom - view) / 2.0).max(0.0)
}

/// `pan` limited so the picture keeps covering the viewport.
pub fn clamp_pan(pan: (f32, f32), fit: (f32, f32), zoom: f32, view: (f32, f32)) -> (f32, f32) {
    let (mx, my) = (max_pan(fit.0, zoom, view.0), max_pan(fit.1, zoom, view.1));
    (pan.0.clamp(-mx, mx), pan.1.clamp(-my, my))
}

/// The bifrost-style resistance: `over` px past the edge shows as less, never
/// more than `limit`.
pub use bifrost_ui::rubber_band;
/// A pinch's zoom while the fingers are down: free inside the limits,
/// stiff beyond them.
pub fn soft_zoom(zoom: f32) -> f32 {
    if zoom < MIN_ZOOM {
        MIN_ZOOM - rubber_band(MIN_ZOOM - zoom, 0.3)
    } else if zoom > MAX_ZOOM {
        MAX_ZOOM + rubber_band(zoom - MAX_ZOOM, 1.0)
    } else {
        zoom
    }
}

/// Where a released pinch comes to rest.
pub fn settle_zoom(zoom: f32) -> f32 {
    zoom.clamp(MIN_ZOOM, MAX_ZOOM)
}

/// New (zoom, pan) after scaling by `scale` about screen point `focal`, so
/// the picture point under the fingers stays under them. `pan` is the
/// picture centre's offset from the viewport centre.
pub fn pinch(zoom: f32, pan: (f32, f32), scale: f32, focal: (f32, f32), view: (f32, f32)) -> (f32, f32, f32) {
    let z1 = soft_zoom(zoom * scale);
    let k = if zoom > 0.0 { z1 / zoom } else { 1.0 };
    let (fx, fy) = (focal.0 - view.0 / 2.0, focal.1 - view.1 / 2.0);
    (z1, fx - (fx - pan.0) * k, fy - (fy - pan.1) * k)
}

/// The zoom a double tap goes to: back to fit when zoomed in, else 2x.
pub fn double_tap_target(zoom: f32) -> f32 {
    if zoom > MIN_ZOOM + 0.01 {
        MIN_ZOOM
    } else {
        DOUBLE_TAP_ZOOM
    }
}

/// Pan that keeps `focal` fixed when going from `zoom` (at `pan`) to `target`.
pub fn zoom_to(target: f32, zoom: f32, pan: (f32, f32), focal: (f32, f32), view: (f32, f32), fit: (f32, f32)) -> (f32, f32) {
    if target <= MIN_ZOOM {
        return (0.0, 0.0);
    }
    let (_, px, py) = pinch(zoom, pan, target / zoom.max(0.01), focal, view);
    clamp_pan((px, py), fit, target, view)
}

/// The frame rate the snap-back was tuned at: a quarter of the stretch is given back each 1/60 s frame.
const REFERENCE_FPS: f32 = 60.0;
/// The share of a stretch given back per reference frame.
const SNAP_PER_FRAME: f32 = 0.25;

/// The snap-back of an offset towards zero over `dt_s` seconds: exponential in elapsed time, so it runs at the
/// same speed at any frame rate and a late frame just advances it further. Snaps to zero under half a pixel.
pub fn decay(v: f32, dt_s: f32) -> f32 {
    if v.abs() < 0.5 {
        0.0
    } else {
        v * (1.0 - SNAP_PER_FRAME).powf(REFERENCE_FPS * dt_s.max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: (f32, f32) = (1200.0, 1920.0);

    #[test]
    fn pan_bounds_follow_the_overflow() {
        assert_eq!(max_pan(1200.0, 1.0, 1200.0), 0.0);
        assert_eq!(max_pan(1200.0, 2.0, 1200.0), 600.0);
        assert_eq!(max_pan(900.0, 2.0, 1920.0), 0.0, "a short picture never pans vertically");
        assert_eq!(clamp_pan((900.0, -50.0), (1200.0, 900.0), 2.0, VIEW), (600.0, 0.0));
        assert_eq!(clamp_pan((-900.0, 0.0), (1200.0, 900.0), 2.0, VIEW), (-600.0, 0.0));
    }

    #[test]
    fn rubber_band_resists_and_saturates() {
        assert_eq!(rubber_band(0.0, 300.0), 0.0);
        let a = rubber_band(100.0, 300.0);
        let b = rubber_band(1000.0, 300.0);
        assert!(a > 0.0 && a < 100.0, "a: {a}");
        assert!(b > a && b < 300.0, "never reaches the limit: {b}");
        assert_eq!(rubber_band(-100.0, 300.0), -a, "symmetric");
        assert_eq!(rubber_band(50.0, 0.0), 0.0);
    }

    #[test]
    fn soft_zoom_is_free_inside_limits_and_stiff_outside() {
        assert_eq!(soft_zoom(2.5), 2.5);
        assert!(soft_zoom(0.5) < 1.0 && soft_zoom(0.5) > 0.7);
        assert!(soft_zoom(8.0) > MAX_ZOOM && soft_zoom(8.0) < MAX_ZOOM + 1.0);
        assert_eq!(settle_zoom(0.6), 1.0);
        assert_eq!(settle_zoom(9.0), MAX_ZOOM);
        assert_eq!(settle_zoom(3.0), 3.0);
    }

    #[test]
    fn pinch_keeps_the_point_under_the_fingers() {
        // pinching about the viewport centre does not move the picture
        let (z, px, py) = pinch(1.0, (0.0, 0.0), 2.0, (600.0, 960.0), VIEW);
        assert_eq!((z, px, py), (2.0, 0.0, 0.0));
        // about a point 300 px left of centre: the picture shifts right by 300
        let (z, px, py) = pinch(1.0, (0.0, 0.0), 2.0, (300.0, 960.0), VIEW);
        assert_eq!((z, px, py), (2.0, 300.0, 0.0));
        // the picture point under the focal is the same before and after
        let (zoom, pan, focal) = (1.5, (40.0, -20.0), (800.0, 500.0));
        let before = ((focal.0 - VIEW.0 / 2.0 - pan.0) / zoom, (focal.1 - VIEW.1 / 2.0 - pan.1) / zoom);
        let (z, px, py) = pinch(zoom, pan, 1.8, focal, VIEW);
        let after = ((focal.0 - VIEW.0 / 2.0 - px) / z, (focal.1 - VIEW.1 / 2.0 - py) / z);
        assert!((before.0 - after.0).abs() < 1e-3 && (before.1 - after.1).abs() < 1e-3);
    }

    #[test]
    fn double_tap_toggles_between_fit_and_two_x() {
        assert_eq!(double_tap_target(1.0), DOUBLE_TAP_ZOOM);
        assert_eq!(double_tap_target(2.0), 1.0);
        assert_eq!(double_tap_target(1.005), DOUBLE_TAP_ZOOM, "within the fit tolerance");
        let fit = (1200.0, 900.0);
        assert_eq!(zoom_to(2.0, 1.0, (0.0, 0.0), (300.0, 960.0), VIEW, fit), (300.0, 0.0));
        assert_eq!(zoom_to(1.0, 2.0, (300.0, 0.0), (300.0, 960.0), VIEW, fit), (0.0, 0.0));
        // a tap near the edge is clamped so no gap shows
        assert_eq!(zoom_to(2.0, 1.0, (0.0, 0.0), (0.0, 960.0), VIEW, fit), (600.0, 0.0));
    }

    #[test]
    fn decay_snaps_to_zero() {
        let frame = 1.0 / 60.0;
        assert_eq!(decay(0.4, frame), 0.0);
        assert!((decay(300.0, frame) - 225.0).abs() < 1e-3, "a quarter per reference frame");
        assert!((decay(-300.0, frame) + 225.0).abs() < 1e-3);
        assert_eq!(decay(300.0, 0.0), 300.0, "no time, no movement");
        let mut v = 300.0;
        let mut steps = 0;
        while v != 0.0 {
            v = decay(v, frame);
            steps += 1;
            assert!(steps < 100);
        }
    }

    #[test]
    fn decay_is_independent_of_the_frame_rate() {
        // two half-frames land where one full frame does
        let full = decay(200.0, 1.0 / 60.0);
        let halves = decay(decay(200.0, 1.0 / 120.0), 1.0 / 120.0);
        assert!((full - halves).abs() < 1e-3);
    }
}
