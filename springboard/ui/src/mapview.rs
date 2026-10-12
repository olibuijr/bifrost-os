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

/// The presentation a map view shows: a street map, a satellite image with road labels on top, or imagery alone.
/// These are the three official map configurations (standard, hybrid and imagery).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapType {
    Standard,
    Hybrid,
    Imagery,
}

/// Which gestures the user may use on a map view. The four flags mirror the official zoom, scroll, pitch and rotate
/// switches. The official defaults are not stated in the reference, so every flag starts on here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapInteraction {
    pub zoom: bool,
    pub scroll: bool,
    pub pitch: bool,
    pub rotate: bool,
}

impl Default for MapInteraction {
    fn default() -> Self {
        MapInteraction { zoom: true, scroll: true, pitch: true, rotate: true }
    }
}

/// Largest pitch a map view accepts, in degrees. This module's own limit: a top-down view is 0 and a horizon view
/// is 90.
pub const MAX_PITCH_DEG: f64 = 90.0;

/// A map view: the camera it looks through, the presentation, the gestures it accepts and its two optional buttons.
/// It answers gestures with the new state and returns `false` when the gesture is switched off.
#[derive(Clone, Debug, PartialEq)]
pub struct MapView {
    camera: MapCamera,
    pitch_deg: f64,
    pub map_type: MapType,
    pub interaction: MapInteraction,
    pub show_tracking_button: bool,
    pub show_pitch_button: bool,
}

impl MapView {
    /// A view with the standard map, every gesture on and both buttons hidden.
    pub fn new(camera: MapCamera) -> Self {
        MapView {
            camera,
            pitch_deg: 0.0,
            map_type: MapType::Standard,
            interaction: MapInteraction::default(),
            show_tracking_button: false,
            show_pitch_button: false,
        }
    }

    pub fn camera(&self) -> &MapCamera {
        &self.camera
    }

    /// Pitch in degrees, 0..=MAX_PITCH_DEG.
    pub fn pitch_deg(&self) -> f64 {
        self.pitch_deg
    }

    /// The region the view currently shows.
    pub fn region(&self) -> CoordinateRegion {
        CoordinateRegion::from_map_rect(self.camera.visible_rect())
    }

    /// Changes the visible region, as `setRegion(_:animated:)` does, by refitting the camera to it. Returns `false`
    /// when the region cannot be fitted.
    pub fn set_region(&mut self, region: CoordinateRegion) -> bool {
        match MapCamera::fitting(region, self.camera.viewport_w, self.camera.viewport_h) {
            Some(cam) => {
                self.camera.center = cam.center;
                self.camera.zoom = cam.zoom;
                true
            }
            None => false,
        }
    }

    /// Centres the view on a coordinate and keeps the zoom.
    pub fn set_center(&mut self, center: Coordinate) {
        self.camera.center = center;
    }

    /// Sets the visible region so that every coordinate in `points` is inside it, with `padding` as the margin the
    /// fitting leaves around them. Returns `false` when there are no points or the fit fails.
    pub fn show_coordinates(&mut self, points: &[Coordinate], padding: f64) -> bool {
        match CoordinateRegion::fitting(points, padding) {
            Some(region) => self.set_region(region),
            None => false,
        }
    }

    /// Pinch gesture: adds `delta` zoom levels. Off when zoom is disabled.
    pub fn pinch(&mut self, delta: f64) -> bool {
        if !self.interaction.zoom {
            return false;
        }
        self.camera.zoom_by(delta);
        true
    }

    /// Drag gesture: pans by a pixel offset. Off when scrolling is disabled.
    pub fn drag(&mut self, dx: f64, dy: f64) -> bool {
        if !self.interaction.scroll {
            return false;
        }
        self.camera.pan_pixels(dx, dy);
        true
    }

    /// Two-finger twist: turns the map by `delta_deg` clockwise. Off when rotation is disabled.
    pub fn rotate(&mut self, delta_deg: f64) -> bool {
        if !self.interaction.rotate || !delta_deg.is_finite() {
            return false;
        }
        self.camera.set_heading(self.camera.heading_deg + delta_deg);
        true
    }

    /// Tilts the view by `delta_deg`, clamped to 0..=MAX_PITCH_DEG. Off when pitch is disabled.
    pub fn tilt(&mut self, delta_deg: f64) -> bool {
        if !self.interaction.pitch || !delta_deg.is_finite() {
            return false;
        }
        self.pitch_deg = (self.pitch_deg + delta_deg).clamp(0.0, MAX_PITCH_DEG);
        true
    }
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
        // At 64 degrees north the projection stretches the 0.5 degree latitude span about 2.28 times, so the height
        // decides: log2(500 / 256 * 360 / 1.14) is about 9.27.
        assert!(cam.zoom > 9.0 && cam.zoom < 9.5, "zoom was {}", cam.zoom);
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

    fn view_at_zoom_zero() -> MapView {
        MapView::new(MapCamera::new(c(0.0, 0.0), 0.0, 256.0, 256.0).expect("valid camera"))
    }

    #[test]
    fn a_new_map_view_is_standard_with_every_gesture_on() {
        let view = view_at_zoom_zero();
        assert_eq!(view.map_type, MapType::Standard);
        assert_eq!(view.interaction, MapInteraction { zoom: true, scroll: true, pitch: true, rotate: true });
        assert!(!view.show_tracking_button && !view.show_pitch_button);
        assert_eq!(view.pitch_deg(), 0.0);
    }

    #[test]
    fn disabled_gestures_change_nothing() {
        let mut view = view_at_zoom_zero();
        view.interaction = MapInteraction { zoom: false, scroll: false, pitch: false, rotate: false };
        assert!(!view.pinch(3.0));
        assert!(!view.drag(50.0, 0.0));
        assert!(!view.rotate(45.0));
        assert!(!view.tilt(20.0));
        assert_eq!(view.camera().zoom, 0.0);
        assert_eq!(view.camera().center, c(0.0, 0.0));
        assert_eq!(view.camera().heading_deg, 0.0);
        assert_eq!(view.pitch_deg(), 0.0);
    }

    #[test]
    fn a_drag_of_half_the_world_at_zoom_zero_crosses_the_antimeridian() {
        // At zoom 0 one pixel is WORLD_POINTS / 256 points. The centre starts at x = WORLD / 2, and 128 pixels
        // move it by WORLD / 2, to x = 0, which is longitude -180.
        let mut view = view_at_zoom_zero();
        assert!(view.drag(128.0, 0.0));
        assert!((view.camera().center.longitude + 180.0).abs() < 1e-6, "longitude {}", view.camera().center.longitude);
    }

    #[test]
    fn rotation_wraps_into_0_360_and_tilt_is_clamped() {
        let mut view = view_at_zoom_zero();
        assert!(view.rotate(-30.0));
        assert!((view.camera().heading_deg - 330.0).abs() < 1e-9);
        assert!(view.tilt(120.0));
        assert_eq!(view.pitch_deg(), MAX_PITCH_DEG);
        assert!(view.tilt(-200.0));
        assert_eq!(view.pitch_deg(), 0.0);
        assert!(!view.rotate(f64::NAN));
    }

    #[test]
    fn pinch_zooms_by_levels_and_stops_at_the_limits() {
        let mut view = view_at_zoom_zero();
        assert!(view.pinch(2.5));
        assert!((view.camera().zoom - 2.5).abs() < 1e-12);
        assert!(view.pinch(100.0));
        assert_eq!(view.camera().zoom, MAX_ZOOM);
    }

    #[test]
    fn set_region_centres_the_view_and_region_reads_back_the_centre() {
        let mut view = view_at_zoom_zero();
        let region = CoordinateRegion::from_meters(c(64.0, -22.0), 20_000.0, 20_000.0);
        assert!(view.set_region(region));
        let back = view.region();
        assert!((back.center.latitude - 64.0).abs() < 1e-6, "latitude {}", back.center.latitude);
        assert!((back.center.longitude + 22.0).abs() < 1e-6, "longitude {}", back.center.longitude);
        assert!(view.camera().zoom > 0.0, "a 20 km region zooms in from the world view");
    }
}
