//! Map snapshots: static images of a map region. Given a centre, zoom and pixel size, this lists the Web Mercator
//! tiles to fetch, where each tile lands in the output (including partial tiles at the edges), and converts
//! coordinates to snapshot pixels so overlays and pins can be drawn on top. Pure std; the renderer composes the pixels.

use std::f64::consts::PI;

pub const TILE: f64 = 256.0;

/// World pixel coordinates at `zoom` for a latitude and longitude (the tile grid is `256 * 2^zoom` pixels wide).
pub fn world_px(lat: f64, lon: f64, zoom: u32) -> (f64, f64) {
    let scale = TILE * f64::from(1u32 << zoom.min(22));
    let lat = lat.clamp(-85.0511, 85.0511).to_radians();
    let x = (lon + 180.0) / 360.0 * scale;
    let y = (1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / PI) / 2.0 * scale;
    (x, y)
}

/// The inverse of `world_px`.
pub fn lat_lon(x: f64, y: f64, zoom: u32) -> (f64, f64) {
    let scale = TILE * f64::from(1u32 << zoom.min(22));
    let lon = x / scale * 360.0 - 180.0;
    let n = PI - 2.0 * PI * y / scale;
    (n.sinh().atan().to_degrees(), lon)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub tile_x: i64,
    pub tile_y: i64,
    pub zoom: u32,
    /// Where the tile's top-left corner lands in the snapshot, in pixels (may be negative).
    pub dest_x: f64,
    pub dest_y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snapshot {
    pub lat: f64,
    pub lon: f64,
    pub zoom: u32,
    pub width: u32,
    pub height: u32,
}

impl Snapshot {
    /// World pixel of the snapshot's top-left corner.
    fn origin(&self) -> (f64, f64) {
        let (cx, cy) = world_px(self.lat, self.lon, self.zoom);
        (cx - f64::from(self.width) / 2.0, cy - f64::from(self.height) / 2.0)
    }

    /// Tiles needed to cover the snapshot, row by row, with their destination positions. Tile x wraps around the
    /// antimeridian; rows above or below the world are left out.
    pub fn tiles(&self) -> Vec<Placed> {
        let (ox, oy) = self.origin();
        let n = 1i64 << self.zoom.min(22);
        let first_x = (ox / TILE).floor() as i64;
        let last_x = ((ox + f64::from(self.width)) / TILE).floor() as i64;
        let first_y = (oy / TILE).floor() as i64;
        let last_y = ((oy + f64::from(self.height)) / TILE).floor() as i64;
        let mut out = vec![];
        for ty in first_y..=last_y {
            if ty < 0 || ty >= n {
                continue;
            }
            for tx in first_x..=last_x {
                out.push(Placed {
                    tile_x: tx.rem_euclid(n),
                    tile_y: ty,
                    zoom: self.zoom,
                    dest_x: tx as f64 * TILE - ox,
                    dest_y: ty as f64 * TILE - oy,
                });
            }
        }
        out
    }

    /// Snapshot pixel of a coordinate (for pins and overlays); may fall outside the image.
    pub fn point(&self, lat: f64, lon: f64) -> (f64, f64) {
        let (ox, oy) = self.origin();
        let (x, y) = world_px(lat, lon, self.zoom);
        (x - ox, y - oy)
    }

    /// The largest zoom (up to `max`) at which every coordinate fits inside the snapshot with `margin` pixels free.
    pub fn fit_zoom(coords: &[(f64, f64)], width: u32, height: u32, margin: u32, max: u32) -> u32 {
        for z in (0..=max.min(22)).rev() {
            let pts: Vec<(f64, f64)> = coords.iter().map(|c| world_px(c.0, c.1, z)).collect();
            if pts.is_empty() {
                return max;
            }
            let (x0, x1) = pts.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.0), b.max(p.0)));
            let (y0, y1) = pts.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
            if x1 - x0 + 2.0 * f64::from(margin) <= f64::from(width) && y1 - y0 + 2.0 * f64::from(margin) <= f64::from(height) {
                return z;
            }
        }
        0
    }
}

/// The URL path of a tile in the usual `{z}/{x}/{y}` scheme.
pub fn tile_path(p: &Placed) -> String {
    format!("{}/{}/{}", p.zoom, p.tile_x, p.tile_y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_pixels_round_trip() {
        let (x, y) = world_px(64.1466, -21.9426, 12);
        let (lat, lon) = lat_lon(x, y, 12);
        assert!((lat - 64.1466).abs() < 1e-6 && (lon + 21.9426).abs() < 1e-6);
        let (cx, cy) = world_px(0.0, 0.0, 1);
        assert!((cx - 256.0).abs() < 1e-6 && (cy - 256.0).abs() < 1e-6);
    }

    #[test]
    fn centre_tile_sits_under_the_middle() {
        let s = Snapshot { lat: 0.0, lon: 0.0, zoom: 1, width: 256, height: 256 };
        let t = s.tiles();
        // the world is 2x2 tiles; a 256 square centred on (256,256) touches all four
        assert_eq!(t.len(), 4);
        let at = |x: i64, y: i64| t.iter().find(|p| p.tile_x == x && p.tile_y == y).unwrap();
        assert!((at(0, 0).dest_x + 128.0).abs() < 1e-6);
        assert!((at(1, 1).dest_x - 128.0).abs() < 1e-6 && (at(1, 1).dest_y - 128.0).abs() < 1e-6);
        let p = s.point(0.0, 0.0);
        assert!((p.0 - 128.0).abs() < 1e-6 && (p.1 - 128.0).abs() < 1e-6);
    }

    #[test]
    fn wraps_x_and_clips_y() {
        let s = Snapshot { lat: 0.0, lon: 179.9, zoom: 2, width: 600, height: 300 };
        let t = s.tiles();
        assert!(t.iter().any(|p| p.tile_x == 0 || p.tile_x == 3));
        assert!(t.iter().all(|p| (0..4).contains(&p.tile_x) && (0..4).contains(&p.tile_y)));
        let polar = Snapshot { lat: 85.0, lon: 0.0, zoom: 3, width: 512, height: 512 };
        assert!(polar.tiles().iter().all(|p| p.tile_y >= 0));
    }

    #[test]
    fn fit_zoom_shrinks_for_spread_out_pins() {
        let near = [(64.14, -21.94), (64.15, -21.93)];
        let far = [(64.14, -21.94), (66.0, -18.0)];
        let zn = Snapshot::fit_zoom(&near, 512, 512, 20, 18);
        let zf = Snapshot::fit_zoom(&far, 512, 512, 20, 18);
        assert!(zn > zf, "{zn} {zf}");
        assert_eq!(Snapshot::fit_zoom(&[], 100, 100, 0, 9), 9);
        assert_eq!(tile_path(&Placed { tile_x: 3, tile_y: 4, zoom: 5, dest_x: 0.0, dest_y: 0.0 }), "5/3/4");
    }
}
