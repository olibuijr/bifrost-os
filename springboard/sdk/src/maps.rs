//! Maps for apps (the MapKit surface): coordinates and regions, the map camera and view state, map controls, user
//! location tracking, snapshot tile placement and the place model. The models live in `bifrost_ui`; this module makes
//! them reachable from the SDK, so an app builds a map without reaching into the UI kit.
//!
//! ```ignore
//! use bifrost_app::maps::{Coordinate, MapCamera, MapView};
//! let centre = Coordinate::new(64.1466, -21.9426).unwrap();
//! let view = MapView::new(MapCamera::new(centre, 12.0, 800.0, 600.0).unwrap());
//! ```
//!
//! The tiles, the tile loader and the geocoder are the Maps app's, shared through `springboard::maptiles`, so the SDK
//! draws the same map the Maps app does. [`Snapshotter`] renders a picture of a map; [`fix_from_location`] and
//! [`feed_user_location`] take the shell's location service into the user-location model; [`search_places`] is
//! forward geocoding with the placemark parts (country, region, address details).
//!
//! Drawing an embedded map view into an app window is not here yet: the window still belongs to the app's own
//! surface, and the Maps app's window is the reference for it.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::core_location::Location as CoreLocation;

pub use bifrost_ui::geo::{Coordinate, CoordinateRegion, CoordinateSpan, MapPoint, MapRect, MapSize};
pub use bifrost_ui::location::LocationFix;
pub use bifrost_ui::map_controls::MapControls;
pub use bifrost_ui::map_item::MapItem;
pub use bifrost_ui::map_snapshot::{lat_lon, tile_path, world_px, Placed, Snapshot};
pub use bifrost_ui::map_user_location::UserLocation;
pub use bifrost_ui::mapview::{compass_visible, MapCamera, MapInteraction, MapType, MapView, ScaleBar, UserTracking};
pub use springboard::maptiles::cache::default_root;
pub use springboard::maptiles::compose::{draw_tiles, Picture};
pub use springboard::maptiles::geocode::{parse_coordinates, Details, Geocoder, Place, DEFAULT_BASE};
pub use springboard::maptiles::loader::TileLoader;
pub use springboard::maptiles::net::{Live, Net, Offline};
pub use springboard::maptiles::source::{user_agent, TileSource};

/// Renders map pictures from tiles. The tiles come from the shared disk cache at `root`, then from `net` at the tile
/// usage policy's pace, exactly as the Maps app loads them.
pub struct Snapshotter {
    loader: TileLoader,
}

impl Snapshotter {
    pub fn new(root: &Path, source: TileSource, net: Arc<dyn Net>) -> Self {
        Snapshotter { loader: TileLoader::new(root, source, net) }
    }

    /// The map centred on `centre` at `zoom`, `width` by `height` points, at `scale` pixels a point. Waits for the tiles
    /// for at most `timeout`; a tile that does not arrive is left white, so the picture is always the view's size.
    pub fn render(&mut self, centre: Coordinate, zoom: f64, width: f64, height: f64, scale: f64, timeout: Duration) -> Picture {
        springboard::maptiles::compose::snapshot(&mut self.loader, centre.latitude, centre.longitude, zoom, width, height, scale, timeout)
    }
}

/// The shell's location report as the user-location model takes it. The report has no course or speed, so those stay
/// unknown, as [`LocationFix::new`] leaves them; a report without altitude keeps altitude zero.
pub fn fix_from_location(location: &CoreLocation) -> LocationFix {
    let c = location.coordinate;
    let mut fix = LocationFix::new(Coordinate { latitude: c.latitude, longitude: c.longitude }, location.timestamp);
    fix.horizontal_accuracy_m = location.horizontal_accuracy;
    if let Some(altitude) = location.altitude {
        fix.altitude_m = altitude;
    }
    fix
}

/// Feeds a location from the shell's location service into the user-location model. Returns whether the model kept it:
/// not when the app may not use location services, or the fix has no valid accuracy.
pub fn feed_user_location(user: &mut UserLocation, location: &CoreLocation) -> bool {
    user.record_fix(fix_from_location(location))
}

/// Forward geocoding: the places a text query names, each with its position and the placemark parts the geocoder
/// returns (see [`Details`]). Reads through `net`, so the caller decides the network.
pub fn search_places(geocoder: &Geocoder, query: &str, net: &dyn Net) -> Result<Vec<Place>, String> {
    let url = geocoder.search_url(query)?;
    let body = net.get(&url, geocoder.user_agent(), 256 * 1024).map_err(|e| e.to_string())?;
    geocoder.parse(&String::from_utf8_lossy(&body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_location::{Coordinate as CoreCoordinate, Location as CoreLocation};
    use bifrost_ui::location_auth::AuthorizationStatus;

    fn report(altitude: Option<f64>) -> CoreLocation {
        CoreLocation {
            coordinate: CoreCoordinate { latitude: 64.1466, longitude: -21.9426 },
            altitude,
            horizontal_accuracy: 12.0,
            timestamp: 100.0,
            source: "test".into(),
            city: String::new(),
            country: String::new(),
        }
    }

    #[test]
    fn a_location_report_becomes_a_fix_with_its_accuracy() {
        let fix = fix_from_location(&report(Some(31.0)));
        assert_eq!((fix.coordinate.latitude, fix.coordinate.longitude), (64.1466, -21.9426));
        assert_eq!((fix.horizontal_accuracy_m, fix.altitude_m, fix.timestamp), (12.0, 31.0, 100.0));
        assert!(fix.course_deg < 0.0 && fix.speed_mps < 0.0, "course and speed are unknown");
        assert_eq!(fix_from_location(&report(None)).altitude_m, 0.0);
    }

    #[test]
    fn the_user_model_keeps_a_fix_only_when_the_app_may_use_location() {
        let mut user = UserLocation::new(AuthorizationStatus::NotDetermined);
        assert!(!feed_user_location(&mut user, &report(None)), "no access, no fix");
        user.set_authorization(AuthorizationStatus::AuthorizedAlways);
        assert!(feed_user_location(&mut user, &report(None)));
        assert!(user.fix().is_some());
    }

    #[test]
    fn a_snapshot_is_the_view_size_even_offline() {
        let root = tempfile::tempdir().expect("a temp dir");
        let source = TileSource::new(springboard::maptiles::source::DEFAULT_TEMPLATE, &user_agent("test")).expect("a source");
        let mut snapshotter = Snapshotter::new(root.path(), source, Arc::new(Offline));
        let centre = Coordinate::new(64.1466, -21.9426).expect("a valid coordinate");
        let picture = snapshotter.render(centre, 3.0, 64.0, 48.0, 1.0, Duration::from_secs(5));
        assert_eq!((picture.width, picture.height), (64, 48));
        assert!(picture.rgb.iter().all(|&b| b == 255), "no tiles arrive offline, so the map is white");
    }

    #[test]
    fn a_view_starts_from_a_camera_on_a_coordinate() {
        let centre = Coordinate::new(64.1466, -21.9426).expect("a valid coordinate");
        let camera = MapCamera::new(centre, 12.0, 800.0, 600.0).expect("a camera for a non-empty viewport");
        let view = MapView::new(camera);
        let region = view.region();
        assert!((region.center.latitude - 64.1466).abs() < 1e-3);
        assert!((region.center.longitude + 21.9426).abs() < 1e-3);
    }

    #[test]
    fn coordinates_outside_the_globe_are_refused() {
        assert!(Coordinate::new(91.0, 0.0).is_none());
        assert!(Coordinate::new(0.0, 181.0).is_none());
    }
}
