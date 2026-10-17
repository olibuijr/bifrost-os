//! The tile cache: the shared cache lives in `springboard::maptiles::cache`. This file adds the one piece that
//! needs the Maps view: the offline area of the view on screen.
pub use springboard::maptiles::cache::*;

use crate::tiles::MAX_ZOOM;
use crate::view::MapView;

/// The area on screen for a view: its corners, from the zoom level of the view up `levels` more levels.
pub fn of_view(view: &MapView, levels: u8) -> OfflineArea {
    let (w, h) = view.size();
    let (north, west) = view.to_geo(0.0, 0.0);
    let (south, east) = view.to_geo(w, h);
    let base = view.zoom().floor() as u8;
    OfflineArea { south, west, north, east, min_zoom: base, max_zoom: (base + levels).min(MAX_ZOOM as u8) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_area_of_a_view_covers_the_view() {
        let view = MapView::new(64.1466, -21.9426, 12.4, 800.0, 600.0);
        let area = of_view(&view, 1);
        assert_eq!((area.min_zoom, area.max_zoom), (12, 13));
        assert!(area.north > 64.1466 && area.south < 64.1466 && area.west < -21.9426 && area.east > -21.9426);
    }
}
