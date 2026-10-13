//! The map view: centre, fractional zoom and the screen size, with pan, anchored zoom (pinch) and pins.
//! Everything derives from the screen size, so the view works at any window size or scale.

use crate::tiles::{self, TileId};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapView {
    lat: f64,
    lon: f64,
    zoom: f64,
    width: f64,
    height: f64,
}

/// A pinned place, shown as a marker on the map.
#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    pub label: String,
    pub lat: f64,
    pub lon: f64,
}

impl MapView {
    /// A view centred on lat/lon at `zoom` (clamped) for a `width` x `height` point area.
    pub fn new(lat: f64, lon: f64, zoom: f64, width: f64, height: f64) -> Self {
        Self {
            lat: tiles::clamp_lat(lat),
            lon: tiles::wrap_lon(lon),
            zoom: tiles::clamp_zoom(zoom),
            width: width.max(1.0),
            height: height.max(1.0),
        }
    }

    pub fn centre(&self) -> (f64, f64) {
        (self.lat, self.lon)
    }

    pub fn zoom(&self) -> f64 {
        self.zoom
    }

    pub fn size(&self) -> (f64, f64) {
        (self.width, self.height)
    }

    /// The area changed size (window resize or scale change). The centre and zoom stay the same.
    pub fn resize(&mut self, width: f64, height: f64) {
        self.width = width.max(1.0);
        self.height = height.max(1.0);
    }

    pub fn set_zoom(&mut self, zoom: f64) {
        self.zoom = tiles::clamp_zoom(zoom);
    }

    pub fn centre_to(&mut self, lat: f64, lon: f64) {
        self.lat = tiles::clamp_lat(lat);
        self.lon = tiles::wrap_lon(lon);
    }

    /// A drag of `dx`, `dy` screen points: the map moves with the finger, so the centre moves the other way.
    pub fn pan(&mut self, dx: f64, dy: f64) {
        let (x, y) = tiles::lonlat_to_world(self.lat, self.lon, self.zoom);
        let (lat, lon) = tiles::world_to_lonlat(x - dx, y - dy, self.zoom);
        self.lat = lat;
        self.lon = lon;
    }

    /// Multiplies the zoom by `factor` (a pinch gesture) and keeps the point under (`ax`, `ay`) in place.
    pub fn zoom_at(&mut self, factor: f64, ax: f64, ay: f64) {
        if !(factor.is_finite() && factor > 0.0) {
            return;
        }
        let (lat, lon) = self.to_geo(ax, ay);
        let new_zoom = tiles::clamp_zoom(self.zoom + factor.log2());
        let (px, py) = tiles::lonlat_to_world(lat, lon, new_zoom);
        // Put the centre so that the anchored point projects back onto (ax, ay).
        let cx = px + self.width / 2.0 - ax;
        let cy = py + self.height / 2.0 - ay;
        self.zoom = new_zoom;
        let (clat, clon) = tiles::world_to_lonlat(cx, cy, new_zoom);
        self.lat = tiles::clamp_lat(clat);
        self.lon = tiles::wrap_lon(clon);
    }

    /// Shows all of `points` (lat, lon) with `pad` points of margin: the highest zoom at which they fit, centred on
    /// their middle. One point (or none that differ) centres at street level.
    pub fn fit(&mut self, points: &[(f64, f64)], pad: f64) {
        let Some(&(lat0, lon0)) = points.first() else { return };
        if points.iter().all(|&(la, lo)| (la - lat0).abs() < 1e-9 && (lo - lon0).abs() < 1e-9) {
            self.centre_to(lat0, lon0);
            self.set_zoom(15.0);
            return;
        }
        let (avail_w, avail_h) = ((self.width - 2.0 * pad).max(1.0), (self.height - 2.0 * pad).max(1.0));
        // The extents are measured at whole zoom levels from the top down; world pixels double with each level.
        let mut z = tiles::MAX_ZOOM.floor();
        loop {
            let pts: Vec<(f64, f64)> = points.iter().map(|&(la, lo)| tiles::lonlat_to_world(la, lo, z)).collect();
            let (min_x, max_x) = pts.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.0), a.1.max(p.0)));
            let (min_y, max_y) = pts.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.1), a.1.max(p.1)));
            if (max_x - min_x <= avail_w && max_y - min_y <= avail_h) || z <= tiles::MIN_ZOOM {
                let (lat, lon) = tiles::world_to_lonlat((min_x + max_x) / 2.0, (min_y + max_y) / 2.0, z);
                self.centre_to(lat, lon);
                self.set_zoom(z);
                return;
            }
            z -= 1.0;
        }
    }

    /// Screen point (in points, origin top-left) of a lat/lon.
    pub fn to_screen(&self, lat: f64, lon: f64) -> (f64, f64) {
        let (cx, cy) = self.centre_world();
        let (px, py) = tiles::lonlat_to_world(lat, lon, self.zoom);
        (px - cx + self.width / 2.0, py - cy + self.height / 2.0)
    }

    /// The lat/lon under a screen point.
    pub fn to_geo(&self, sx: f64, sy: f64) -> (f64, f64) {
        let (cx, cy) = self.centre_world();
        tiles::world_to_lonlat(cx + sx - self.width / 2.0, cy + sy - self.height / 2.0, self.zoom)
    }

    /// The tiles to draw, with their top-left corners on screen.
    pub fn tiles(&self) -> Vec<(TileId, f64, f64)> {
        let (cx, cy) = self.centre_world();
        tiles::tiles_in_view(cx, cy, self.width, self.height, self.zoom)
    }

    fn centre_world(&self) -> (f64, f64) {
        tiles::lonlat_to_world(self.lat, self.lon, self.zoom)
    }
}

/// The pinned places, in the order they were added.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pins {
    items: Vec<Pin>,
}

impl Pins {
    /// Adds a pin. Coordinates outside the projection are refused.
    pub fn add(&mut self, label: &str, lat: f64, lon: f64) -> Result<(), String> {
        if !(lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0) {
            return Err("a pin needs a latitude within 90 and a longitude within 180".into());
        }
        self.items.push(Pin { label: label.trim().to_string(), lat, lon });
        Ok(())
    }

    pub fn remove(&mut self, index: usize) -> Option<Pin> {
        (index < self.items.len()).then(|| self.items.remove(index))
    }

    pub fn items(&self) -> &[Pin] {
        &self.items
    }

    /// The pins that fall inside the view, with their screen points.
    pub fn visible(&self, view: &MapView) -> Vec<(usize, f64, f64)> {
        let (w, h) = view.size();
        self.items
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let (x, y) = view.to_screen(p.lat, p.lon);
                (i, x, y)
            })
            .filter(|&(_, x, y)| x >= 0.0 && y >= 0.0 && x <= w && y <= h)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn the_centre_of_the_view_is_the_centre_coordinate() {
        let v = MapView::new(64.1466, -21.9426, 12.0, 800.0, 600.0);
        let (x, y) = v.to_screen(64.1466, -21.9426);
        assert!(close(x, 400.0) && close(y, 300.0));
        let (lat, lon) = v.to_geo(400.0, 300.0);
        assert!(close(lat, 64.1466) && close(lon, -21.9426));
    }

    #[test]
    fn a_drag_moves_the_map_with_the_finger() {
        let mut v = MapView::new(0.0, 0.0, 4.0, 400.0, 400.0);
        v.pan(50.0, 0.0);
        // Dragging right reveals the west: the centre longitude goes down.
        assert!(v.centre().1 < 0.0);
        let mut moved = v;
        moved.pan(0.0, -30.0);
        // Dragging up reveals the south: the centre latitude goes down.
        assert!(moved.centre().0 < 0.0);
    }

    #[test]
    fn pinch_keeps_the_anchor_under_the_fingers() {
        let mut v = MapView::new(64.0, -21.0, 10.0, 800.0, 600.0);
        let anchor = (200.0, 150.0);
        let before = v.to_geo(anchor.0, anchor.1);
        v.zoom_at(2.0, anchor.0, anchor.1);
        assert!(close(v.zoom(), 11.0));
        let after = v.to_screen(before.0, before.1);
        assert!((after.0 - anchor.0).abs() < 1e-4 && (after.1 - anchor.1).abs() < 1e-4);
    }

    #[test]
    fn zoom_is_clamped_and_bad_factors_are_ignored() {
        let mut v = MapView::new(0.0, 0.0, 18.5, 300.0, 300.0);
        v.zoom_at(1e6, 150.0, 150.0);
        assert!(close(v.zoom(), tiles::MAX_ZOOM));
        v.zoom_at(f64::NAN, 0.0, 0.0);
        v.zoom_at(0.0, 0.0, 0.0);
        assert!(close(v.zoom(), tiles::MAX_ZOOM));
    }

    #[test]
    fn resizing_keeps_the_centre_and_the_tiles_follow_the_size() {
        let mut v = MapView::new(0.0, 0.0, 1.0, 256.0, 256.0);
        let small = v.tiles().len();
        v.resize(1024.0, 768.0);
        assert_eq!(v.centre(), (0.0, 0.0));
        assert!(v.tiles().len() > small);
    }

    #[test]
    fn fitting_shows_every_point_with_the_margin_and_a_single_point_gets_street_level() {
        let mut v = MapView::new(0.0, 0.0, 2.0, 800.0, 600.0);
        let points = [(64.1466, -21.9426), (65.6835, -18.1262), (63.4, -20.3)];
        v.fit(&points, 40.0);
        for &(la, lo) in &points {
            let (x, y) = v.to_screen(la, lo);
            assert!(x >= 40.0 - 1e-6 && x <= 760.0 + 1e-6 && y >= 40.0 - 1e-6 && y <= 560.0 + 1e-6, "{x} {y}");
        }
        // one level more would not fit
        let mut closer = v;
        closer.set_zoom(v.zoom() + 1.0);
        assert!(points.iter().any(|&(la, lo)| {
            let (x, y) = closer.to_screen(la, lo);
            x < 40.0 || x > 760.0 || y < 40.0 || y > 560.0
        }));
        let mut one = MapView::new(0.0, 0.0, 2.0, 800.0, 600.0);
        one.fit(&[(64.1466, -21.9426)], 40.0);
        assert!(close(one.zoom(), 15.0) && close(one.centre().0, 64.1466));
        one.fit(&[], 40.0);
        assert!(close(one.zoom(), 15.0));
    }

    #[test]
    fn only_pins_on_screen_are_visible() {
        let v = MapView::new(64.1466, -21.9426, 12.0, 800.0, 600.0);
        let mut pins = Pins::default();
        pins.add("Here", 64.1466, -21.9426).unwrap();
        pins.add("Far away", -33.87, 151.21).unwrap();
        assert!(pins.add("Bad", 120.0, 0.0).is_err());
        let shown = pins.visible(&v);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].0, 0);
        assert_eq!(pins.remove(1).unwrap().label, "Far away");
        assert!(pins.remove(9).is_none());
    }
}
