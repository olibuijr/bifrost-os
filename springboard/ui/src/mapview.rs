//! Map camera, zoom levels, scale bar and user-tracking state for a map view.
//!
//! A camera holds a centre coordinate, a zoom level and the size of the viewport in pixels. Zoom 0 shows the whole
//! projected world in one 256-pixel tile, and each zoom level doubles the pixels per map point. The model converts
//! between screen pixels and coordinates, pans and zooms, and works out the length of a scale bar. It draws nothing
//! and reads no position: a renderer and the caller own those.
//!
//! The 256-pixel tile convention is the common web-map one. The tracking states (off, follow, follow with heading)
//! follow the official user-tracking states; the order they cycle in and the compass threshold are this module's own
//! rules, not part of the official model.

use crate::geo::{metres_per_point_at_latitude, Coordinate, CoordinateRegion, MapPoint, MapRect, WORLD_POINTS};
use crate::location::normalize_degrees;

/// Pixels in the tile that covers the whole world at zoom 0.
pub const TILE_PX: f64 = 256.0;
/// Smallest zoom level a camera accepts (the whole world).
pub const MIN_ZOOM: f64 = 0.0;
/// Largest zoom level a camera accepts.
pub const MAX_ZOOM: f64 = 20.0;

/// Map points covered by one screen pixel at `zoom`. At zoom 0 that is WORLD_POINTS / TILE_PX, and each zoom level
/// halves it.
fn points_per_pixel_at(zoom: f64) -> f64 {
    WORLD_POINTS / (TILE_PX * 2f64.powf(zoom))
}

/// Wraps a longitude into -180..180, so a pan across the antimeridian stays a valid coordinate.
fn wrap_longitude(longitude: f64) -> f64 {
    (longitude + 540.0) % 360.0 - 180.0
}

/// A view onto the map: the centre, the zoom, the heading the map is turned to, and the viewport size in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapCamera {
    pub center: Coordinate,
    pub zoom: f64,
    /// Degrees clockwise from north, in 0..360. The camera stores it; the renderer applies the turn.
    pub heading_deg: f64,
    pub viewport_w: f64,
    pub viewport_h: f64,
}

impl MapCamera {
    /// `None` when the viewport is not a positive finite size or the zoom is not finite. The zoom is clamped to
    /// `MIN_ZOOM..=MAX_ZOOM`.
    pub fn new(center: Coordinate, zoom: f64, viewport_w: f64, viewport_h: f64) -> Option<Self> {
        let valid = zoom.is_finite()
            && viewport_w.is_finite()
            && viewport_h.is_finite()
            && viewport_w > 0.0
            && viewport_h > 0.0;
        valid.then(|| MapCamera {
            center,
            zoom: zoom.clamp(MIN_ZOOM, MAX_ZOOM),
            heading_deg: 0.0,
            viewport_w,
            viewport_h,
        })
    }

    /// The camera at the largest zoom that still shows all of `region` in a viewport of the given size, centred on
    /// the region. `None` when the viewport is not valid.
    pub fn fitting(region: CoordinateRegion, viewport_w: f64, viewport_h: f64) -> Option<Self> {
        let rect = region.map_rect();
        let needed = (rect.width / viewport_w).max(rect.height / viewport_h);
        let zoom = if needed > 0.0 { MAX_ZOOM - needed.log2() } else { MAX_ZOOM };
        MapCamera::new(region.center, zoom, viewport_w, viewport_h)
    }

    /// Map points covered by one screen pixel at the current zoom.
    pub fn points_per_pixel(&self) -> f64 {
        points_per_pixel_at(self.zoom)
    }

    /// Metres covered by one screen pixel at the centre latitude.
    pub fn metres_per_pixel(&self) -> f64 {
        metres_per_point_at_latitude(self.center.latitude) * self.points_per_pixel()
    }

    /// Screen pixel of a coordinate, measured from the top-left corner of the viewport. Points outside the viewport
    /// give pixels outside 0..viewport.
    pub fn screen_point(&self, c: Coordinate) -> (f64, f64) {
        let ppp = self.points_per_pixel();
        let centre = MapPoint::from_coordinate(self.center);
        let p = MapPoint::from_coordinate(c);
        ((p.x - centre.x) / ppp + self.viewport_w / 2.0, (p.y - centre.y) / ppp + self.viewport_h / 2.0)
    }

    /// Coordinate under a pixel of the viewport. The longitude may fall outside -180..180 for pixels far off the map.
    pub fn coordinate_at(&self, x: f64, y: f64) -> Coordinate {
        let ppp = self.points_per_pixel();
        let centre = MapPoint::from_coordinate(self.center);
        MapPoint::new(centre.x + (x - self.viewport_w / 2.0) * ppp, centre.y + (y - self.viewport_h / 2.0) * ppp)
            .coordinate()
    }

    /// The map rectangle visible in the viewport.
    pub fn visible_rect(&self) -> MapRect {
        let ppp = self.points_per_pixel();
        let centre = MapPoint::from_coordinate(self.center);
        let half_w = self.viewport_w / 2.0 * ppp;
        let half_h = self.viewport_h / 2.0 * ppp;
        MapRect::new(centre.x - half_w, centre.y - half_h, 2.0 * half_w, 2.0 * half_h)
    }

    /// Sets the zoom, clamped to `MIN_ZOOM..=MAX_ZOOM`. A non-finite zoom is ignored.
    pub fn set_zoom(&mut self, zoom: f64) {
        if zoom.is_finite() {
            self.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        }
    }

    /// Adds `delta` to the zoom (positive zooms in), clamped as `set_zoom`.
    pub fn zoom_by(&mut self, delta: f64) {
        let next = self.zoom + delta;
        self.set_zoom(next);
    }

    /// Moves the centre by a pixel offset. Dragging the map right by `dx` moves the centre west, so the point under
    /// the finger stays under it. Longitude wraps across the antimeridian. Non-finite offsets are ignored.
    pub fn pan_pixels(&mut self, dx: f64, dy: f64) {
        if !dx.is_finite() || !dy.is_finite() {
            return;
        }
        let ppp = self.points_per_pixel();
        let centre = MapPoint::from_coordinate(self.center);
        let moved = MapPoint::new(centre.x - dx * ppp, centre.y - dy * ppp).coordinate();
        self.center = Coordinate { latitude: moved.latitude, longitude: wrap_longitude(moved.longitude) };
    }

    /// Sets the heading, normalised to 0..360. A non-finite heading is ignored.
    pub fn set_heading(&mut self, heading_deg: f64) {
        if heading_deg.is_finite() {
            self.heading_deg = normalize_degrees(heading_deg);
        }
    }

    /// Applies the user's position for a tracking state. Off leaves the camera alone; follow centres on the user;
    /// follow with heading also turns the map to `heading_deg` when one is given.
    pub fn follow(&mut self, user: Coordinate, heading_deg: Option<f64>, mode: UserTracking) {
        if !mode.centres_on_user() {
            return;
        }
        self.center = user;
        if let (true, Some(h)) = (mode.rotates_with_heading(), heading_deg) {
            self.set_heading(h);
        }
    }

    /// The longest scale bar that fits in `max_px` pixels, using a length of 1, 2 or 5 times a power of ten metres.
    /// `None` when `max_px` is not a positive finite number or even the smallest length does not fit.
    pub fn scale_bar(&self, max_px: f64) -> Option<ScaleBar> {
        if !max_px.is_finite() || max_px <= 0.0 {
            return None;
        }
        let mpp = self.metres_per_pixel();
        let max_m = max_px * mpp;
        let mut best: Option<f64> = None;
        for exp in -2..=8 {
            for mantissa in [1.0, 2.0, 5.0] {
                let len = mantissa * 10f64.powi(exp);
                if len <= max_m && best.map_or(true, |b| len > b) {
                    best = Some(len);
                }
            }
        }
        best.map(|length_m| ScaleBar { length_m, width_px: length_m / mpp })
    }
}

/// A scale bar: its real length in metres and the width it takes on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScaleBar {
    pub length_m: f64,
    pub width_px: f64,
}

/// The user-tracking state of a map (the states behind the official tracking button).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserTracking {
    Off,
    Follow,
    FollowWithHeading,
}

impl UserTracking {
    /// The state after a press of the tracking button: off, follow, follow with heading, then off again.
    pub fn next(self) -> Self {
        match self {
            UserTracking::Off => UserTracking::Follow,
            UserTracking::Follow => UserTracking::FollowWithHeading,
            UserTracking::FollowWithHeading => UserTracking::Off,
        }
    }

    /// True when the camera centres on the user's position.
    pub fn centres_on_user(self) -> bool {
        self != UserTracking::Off
    }

    /// True when the camera also turns to the user's heading.
    pub fn rotates_with_heading(self) -> bool {
        self == UserTracking::FollowWithHeading
    }
}

/// True when a compass control should show: the map is turned more than half a degree away from north.
/// A non-finite heading shows nothing.
pub fn compass_visible(heading_deg: f64) -> bool {
    if !heading_deg.is_finite() {
        return false;
    }
    let h = normalize_degrees(heading_deg);
    h > 0.5 && h < 359.5
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(latitude: f64, longitude: f64) -> Coordinate {
        Coordinate::new(latitude, longitude).expect("valid test coordinate")
    }

    #[test]
    fn camera_rejects_bad_viewports_and_clamps_zoom() {
        let cam = MapCamera::new(c(0.0, 0.0), 25.0, 256.0, 256.0).expect("valid camera");
        assert_eq!(cam.zoom, MAX_ZOOM);
        assert!(MapCamera::new(c(0.0, 0.0), 3.0, 0.0, 256.0).is_none());
        assert!(MapCamera::new(c(0.0, 0.0), 3.0, 256.0, f64::NAN).is_none());
        assert!(MapCamera::new(c(0.0, 0.0), f64::NAN, 256.0, 256.0).is_none());
    }

    #[test]
    fn zoom_zero_shows_the_whole_world_in_one_tile() {
        let cam = MapCamera::new(c(0.0, 0.0), 0.0, 256.0, 256.0).expect("valid camera");
        let rect = cam.visible_rect();
        assert!((rect.width - WORLD_POINTS).abs() < 1e-3, "width was {}", rect.width);
        assert_eq!(cam.screen_point(c(0.0, 0.0)), (128.0, 128.0));
        assert!((cam.points_per_pixel() - 1_048_576.0).abs() < 1e-6, "2^20 points per pixel at zoom 0");
    }

    #[test]
    fn screen_and_coordinate_conversions_round_trip() {
        let cam = MapCamera::new(c(64.0, -22.0), 10.0, 800.0, 600.0).expect("valid camera");
        let centre = cam.coordinate_at(400.0, 300.0);
        assert!((centre.latitude - 64.0).abs() < 1e-9);
        assert!((centre.longitude + 22.0).abs() < 1e-9);
        let (x, y) = cam.screen_point(c(64.0, -22.0));
        assert!((x - 400.0).abs() < 1e-6 && (y - 300.0).abs() < 1e-6);
        let p = cam.coordinate_at(100.0, 200.0);
        let (bx, by) = cam.screen_point(p);
        assert!((bx - 100.0).abs() < 1e-6 && (by - 200.0).abs() < 1e-6);
        assert!(cam.coordinate_at(500.0, 300.0).longitude > -22.0, "east of centre is larger longitude");
    }

    #[test]
    fn fitting_picks_a_zoom_that_shows_the_region() {
        let flat = CoordinateRegion::new(c(0.0, 0.0), crate::geo::CoordinateSpan::new(0.0, 360.0));
        let cam = MapCamera::fitting(flat, 256.0, 256.0).expect("valid camera");
        assert!(cam.zoom.abs() < 1e-9, "a full-width region fits at zoom 0, got {}", cam.zoom);
        let small = CoordinateRegion::new(c(64.0, -22.0), crate::geo::CoordinateSpan::new(0.5, 0.5));
        let cam = MapCamera::fitting(small, 500.0, 500.0).expect("valid camera");
        assert!(cam.zoom > 10.0 && cam.zoom < 11.0, "zoom was {}", cam.zoom);
        let (shown, wanted) = (cam.visible_rect(), small.map_rect());
        assert!(shown.width >= wanted.width * 0.999 && shown.height >= wanted.height * 0.999);
    }

    #[test]
    fn zoom_steps_are_clamped_and_panning_wraps_the_antimeridian() {
        let mut cam = MapCamera::new(c(0.0, 0.0), 10.0, 256.0, 256.0).expect("valid camera");
        cam.zoom_by(1.0);
        assert_eq!(cam.zoom, 11.0);
        cam.set_zoom(-3.0);
        assert_eq!(cam.zoom, 0.0);
        cam.set_zoom(f64::NAN);
        assert_eq!(cam.zoom, 0.0, "a non-finite zoom is ignored");
        // At zoom 0 one pixel is 2^20 map points, so 256 pixels is exactly one full world width.
        cam.pan_pixels(256.0, 0.0);
        assert!(cam.center.longitude.abs() < 1e-9, "longitude was {}", cam.center.longitude);
        cam.pan_pixels(128.0, 0.0);
        assert!((cam.center.longitude + 180.0).abs() < 1e-9, "longitude was {}", cam.center.longitude);
    }

    #[test]
    fn scale_bar_uses_the_longest_round_length_that_fits() {
        let cam = MapCamera::new(c(0.0, 0.0), 20.0, 256.0, 256.0).expect("valid camera");
        // At zoom 20 on the equator one pixel is one map point, about 0.149 m. In 100 px that is about 14.9 m,
        // so the longest round length is 10 m, which is about 67 px.
        let bar = cam.scale_bar(100.0).expect("a bar fits");
        assert_eq!(bar.length_m, 10.0);
        assert!((bar.width_px - 10.0 / cam.metres_per_pixel()).abs() < 1e-9);
        assert!(bar.width_px > 66.0 && bar.width_px < 68.0, "width was {}", bar.width_px);
        assert!(cam.scale_bar(0.0).is_none());
        assert!(cam.scale_bar(f64::NAN).is_none());
    }

    #[test]
    fn tracking_cycles_and_follow_respects_the_mode() {
        assert_eq!(UserTracking::Off.next(), UserTracking::Follow);
        assert_eq!(UserTracking::Follow.next(), UserTracking::FollowWithHeading);
        assert_eq!(UserTracking::FollowWithHeading.next(), UserTracking::Off);
        assert!(!UserTracking::Off.centres_on_user());
        assert!(UserTracking::Follow.centres_on_user() && !UserTracking::Follow.rotates_with_heading());

        let mut cam = MapCamera::new(c(0.0, 0.0), 12.0, 256.0, 256.0).expect("valid camera");
        cam.follow(c(64.0, -22.0), Some(370.0), UserTracking::Off);
        assert_eq!(cam.center.latitude, 0.0, "off leaves the camera alone");
        cam.follow(c(64.0, -22.0), Some(370.0), UserTracking::Follow);
        assert_eq!(cam.center.latitude, 64.0);
        assert_eq!(cam.heading_deg, 0.0, "follow does not turn the map");
        cam.follow(c(64.0, -22.0), Some(370.0), UserTracking::FollowWithHeading);
        assert!((cam.heading_deg - 10.0).abs() < 1e-9, "370 degrees normalises to 10");
    }

    #[test]
    fn compass_shows_only_when_the_map_is_turned() {
        assert!(!compass_visible(0.0));
        assert!(!compass_visible(360.0), "360 is north");
        assert!(compass_visible(90.0));
        assert!(!compass_visible(359.8), "within half a degree of north");
        assert!(compass_visible(-90.0), "-90 is 270, which is turned");
        assert!(!compass_visible(f64::NAN));
    }
}
