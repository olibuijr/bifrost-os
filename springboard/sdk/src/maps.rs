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
//! Drawing the map (tiles, pins and the user marker) and the tile source are not in this module yet; the models
//! say what to draw and where, and the shell's weather map is the first place that draws them.

pub use bifrost_ui::geo::{Coordinate, CoordinateRegion, CoordinateSpan, MapPoint, MapRect, MapSize};
pub use bifrost_ui::map_controls::MapControls;
pub use bifrost_ui::map_item::MapItem;
pub use bifrost_ui::map_snapshot::{lat_lon, tile_path, world_px, Placed, Snapshot};
pub use bifrost_ui::map_user_location::UserLocation;
pub use bifrost_ui::mapview::{compass_visible, MapCamera, MapInteraction, MapType, MapView, ScaleBar, UserTracking};

#[cfg(test)]
mod tests {
    use super::*;

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
