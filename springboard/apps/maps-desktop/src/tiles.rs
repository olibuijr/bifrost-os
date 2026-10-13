//! Web Mercator tile math. World pixels at zoom `z` span `256 * 2^z` on each axis.

use std::f64::consts::PI;

pub const TILE_SIZE: f64 = 256.0;
pub const MIN_ZOOM: f64 = 0.0;
pub const MAX_ZOOM: f64 = 19.0;
/// Latitude limit of the Web Mercator projection.
pub const MAX_LAT: f64 = 85.0511287798;

/// One tile of the pyramid: zoom level and column/row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileId {
    pub z: u8,
    pub x: u32,
    pub y: u32,
}

pub fn clamp_zoom(zoom: f64) -> f64 {
    if zoom.is_nan() {
        return MIN_ZOOM;
    }
    zoom.clamp(MIN_ZOOM, MAX_ZOOM)
}

pub fn clamp_lat(lat: f64) -> f64 {
    lat.clamp(-MAX_LAT, MAX_LAT)
}

/// Wraps a longitude into [-180, 180).
pub fn wrap_lon(lon: f64) -> f64 {
    (lon + 180.0).rem_euclid(360.0) - 180.0
}

/// Width (and height) of the world in pixels at a fractional zoom.
pub fn world_px(zoom: f64) -> f64 {
    TILE_SIZE * 2f64.powf(zoom)
}

/// Projects lat/lon to world pixels at `zoom`. Latitude is clamped to the projection limit.
pub fn lonlat_to_world(lat: f64, lon: f64, zoom: f64) -> (f64, f64) {
    let size = world_px(zoom);
    let s = clamp_lat(lat).to_radians().sin();
    let x = (wrap_lon(lon) + 180.0) / 360.0 * size;
    let y = (0.5 - ((1.0 + s) / (1.0 - s)).ln() / (4.0 * PI)) * size;
    (x, y)
}

/// Inverse of [`lonlat_to_world`]: world pixels at `zoom` to (lat, lon).
pub fn world_to_lonlat(x: f64, y: f64, zoom: f64) -> (f64, f64) {
    let size = world_px(zoom);
    let lon = wrap_lon(x / size * 360.0 - 180.0);
    let n = PI * (1.0 - 2.0 * y / size);
    (n.sinh().atan().to_degrees(), lon)
}

/// The integer tile level for a fractional zoom, and the scale from that level to the zoom.
pub fn tile_level(zoom: f64) -> (u8, f64) {
    let z = clamp_zoom(zoom).floor();
    (z as u8, 2f64.powf(clamp_zoom(zoom) - z))
}

/// The tile at integer level `z` that contains a lat/lon. The row is clamped to the pyramid, the column wraps.
pub fn tile_at(lat: f64, lon: f64, z: u8) -> TileId {
    let (wx, wy) = lonlat_to_world(lat, lon, z as f64);
    let n = 1u32 << z;
    let x = (wx / TILE_SIZE).floor() as i64;
    let y = (wy / TILE_SIZE).floor().clamp(0.0, (n - 1) as f64) as u32;
    TileId { z, x: x.rem_euclid(n as i64) as u32, y }
}

/// Tiles covering a `width` x `height` screen whose centre is at world pixel (`cx`, `cy`) at fractional `zoom`.
/// Each entry is the tile and its top-left corner on screen, in pixels. Tiles are drawn at the scale
/// `2^(zoom - level)`, so a pinch shows the nearest integer level scaled up or down.
pub fn tiles_in_view(cx: f64, cy: f64, width: f64, height: f64, zoom: f64) -> Vec<(TileId, f64, f64)> {
    let (z, _) = tile_level(zoom);
    let scale = 2f64.powf(clamp_zoom(zoom) - z as f64);
    let tile_px = TILE_SIZE * scale;
    // The screen's top-left corner in world pixels at the view zoom.
    let left = cx - width / 2.0;
    let top = cy - height / 2.0;
    let n = 1i64 << z;
    // the right and bottom edges are exclusive: a view exactly one tile wide needs one tile, not a wrapped second
    let (x0, x1) = ((left / tile_px).floor() as i64, ((left + width) / tile_px).ceil() as i64 - 1);
    let (y0, y1) = ((top / tile_px).floor() as i64, ((top + height) / tile_px).ceil() as i64 - 1);
    let mut out = Vec::new();
    for ty in y0.max(0)..=y1.min(n - 1) {
        for tx in x0..=x1 {
            let id = TileId { z, x: tx.rem_euclid(n) as u32, y: ty as u32 };
            out.push((id, tx as f64 * tile_px - left, ty as f64 * tile_px - top));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() < eps
    }

    #[test]
    fn origin_projects_to_the_centre_of_the_world() {
        let (x, y) = lonlat_to_world(0.0, 0.0, 0.0);
        assert!(close(x, 128.0, 1e-9) && close(y, 128.0, 1e-9));
    }

    #[test]
    fn projection_round_trips() {
        for &(lat, lon, z) in &[(64.1466, -21.9426, 12.0), (-33.87, 151.21, 5.5), (51.5, -0.12, 19.0)] {
            let (x, y) = lonlat_to_world(lat, lon, z);
            let (la, lo) = world_to_lonlat(x, y, z);
            assert!(close(la, lat, 1e-6) && close(lo, lon, 1e-6), "{lat},{lon} -> {la},{lo}");
        }
    }

    #[test]
    fn known_tile_for_reykjavik_at_zoom_10() {
        // Standard slippy-map tile for 64.1466 N, 21.9426 W at z=10.
        assert_eq!(tile_at(64.1466, -21.9426, 10), TileId { z: 10, x: 449, y: 272 });
    }

    #[test]
    fn latitude_is_clamped_to_the_projection_limit() {
        let (_, y_pole) = lonlat_to_world(90.0, 0.0, 2.0);
        let (_, y_limit) = lonlat_to_world(MAX_LAT, 0.0, 2.0);
        assert!(close(y_pole, y_limit, 1e-9));
        assert!(close(y_pole, 0.0, 1e-6));
    }

    #[test]
    fn longitude_wraps_and_columns_wrap() {
        assert!(close(wrap_lon(190.0), -170.0, 1e-9));
        assert!(close(wrap_lon(-180.0), -180.0, 1e-9));
        assert_eq!(tile_at(0.0, 180.0, 3).x, 0);
        assert_eq!(tile_at(0.0, -180.0, 3).x, 0);
    }

    #[test]
    fn zoom_is_clamped_and_split_into_level_and_scale() {
        assert_eq!(clamp_zoom(-3.0), 0.0);
        assert_eq!(clamp_zoom(42.0), MAX_ZOOM);
        assert_eq!(clamp_zoom(f64::NAN), 0.0);
        let (z, s) = tile_level(3.5);
        assert_eq!(z, 3);
        assert!(close(s, 2f64.sqrt(), 1e-12));
    }

    #[test]
    fn view_at_zoom_zero_covers_the_single_world_tile() {
        let tiles = tiles_in_view(128.0, 128.0, 256.0, 256.0, 0.0);
        assert_eq!(tiles, vec![(TileId { z: 0, x: 0, y: 0 }, 0.0, 0.0)]);
    }

    #[test]
    fn view_tiles_cover_the_screen_and_wrap_horizontally() {
        // Centre on the antimeridian at zoom 2: the view straddles columns 3 and 0.
        let tiles = tiles_in_view(0.0, 512.0, 800.0, 600.0, 2.0);
        let xs: Vec<u32> = tiles.iter().map(|t| t.0.x).collect();
        assert!(xs.contains(&0) && xs.contains(&3));
        // Every tile stays inside the 4x4 pyramid and the first tile starts at or before the left edge.
        assert!(tiles.iter().all(|t| t.0.y < 4));
        assert!(tiles.iter().any(|t| t.1 <= 0.0 && t.2 <= 0.0));
    }

    #[test]
    fn fractional_zoom_scales_tiles_to_the_view() {
        let tiles = tiles_in_view(256.0, 256.0, 256.0, 256.0, 1.0);
        assert!(tiles.iter().all(|t| t.0.z == 1));
        let scaled = tiles_in_view(362.0, 362.0, 800.0, 800.0, 1.5);
        assert!(scaled.iter().all(|t| t.0.z == 1));
        // At zoom 1.5 a level-1 tile spans 256 * sqrt(2) screen pixels.
        let mut xs: Vec<f64> = scaled.iter().map(|t| t.1).collect();
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        xs.dedup_by(|a, b| close(*a, *b, 1e-9));
        assert!(xs.len() >= 2 && close(xs[1] - xs[0], 256.0 * 2f64.sqrt(), 1e-6));
    }
}
