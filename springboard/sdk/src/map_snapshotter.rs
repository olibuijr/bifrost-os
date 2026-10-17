//! `MKMapSnapshotter`: a static image of a map region, composed from map tiles the app already holds.
//!
//! `bifrost_ui::map_snapshot::Snapshot` works out which Web Mercator tiles cover a region and where each lands.
//! [`render`] takes those tiles from a [`TileSource`], decodes them and places them in an image, then draws pins and
//! routes on top. Tiles are never fetched from here: a [`TileSource`] is whatever the app has, and [`TileDirectory`] reads
//! a local tile pack laid out as `ROOT/{z}/{x}/{y}.png` (`.jpg`, `.jpeg` and `.webp` also work). That keeps map viewing
//! free of requests to a third-party tile server unless the app chooses to build such a source itself. Tiles the source
//! does not have are left as a plain background and counted, so the app can tell the person a region is not available
//! offline.

use bifrost_render::cg::imageio::decode;
use bifrost_render::gfx::{pack, Image};
use bifrost_ui::map_snapshot::{Placed, Snapshot};
use std::path::PathBuf;

/// Where the encoded bytes of a tile come from.
pub trait TileSource {
    /// The encoded image (PNG, JPEG or WebP) of the tile, or None when the source does not have it.
    fn tile(&self, placed: &Placed) -> Option<Vec<u8>>;
}

/// A local tile pack in `ROOT/{z}/{x}/{y}.{png,jpg,jpeg,webp}`.
pub struct TileDirectory {
    root: PathBuf,
}

impl TileDirectory {
    pub fn new(root: impl Into<PathBuf>) -> TileDirectory {
        TileDirectory { root: root.into() }
    }
}

impl TileSource for TileDirectory {
    fn tile(&self, placed: &Placed) -> Option<Vec<u8>> {
        let dir = self.root.join(placed.zoom.to_string()).join(placed.tile_x.to_string());
        ["png", "jpg", "jpeg", "webp"]
            .iter()
            .find_map(|ext| std::fs::read(dir.join(format!("{}.{ext}", placed.tile_y))).ok())
    }
}

/// A marker at a coordinate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pin {
    pub latitude: f64,
    pub longitude: f64,
    /// Premultiplied pixel (see `gfx::pack`).
    pub color: u32,
}

/// A line through coordinates, such as a route.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub points: Vec<(f64, f64)>,
    pub color: u32,
    /// Line width in pixels.
    pub width: u32,
}

/// What `render` made.
pub struct Rendered {
    pub image: Image,
    /// Tiles placed in the image.
    pub drawn: usize,
    /// Tiles the source did not have, or that did not decode.
    pub missing: usize,
}

/// The colour under tiles that are not there.
pub const BACKGROUND: u32 = pack(228, 228, 228, 255);

fn over(dst: u32, src: u32) -> u32 {
    let sa = src >> 24;
    if sa == 255 {
        return src;
    }
    if sa == 0 {
        return dst;
    }
    let mut out = 0u32;
    for shift in [0u32, 8, 16, 24] {
        let s = (src >> shift) & 255;
        let d = (dst >> shift) & 255;
        let v = (s + (d * (255 - sa) + 127) / 255).min(255);
        out |= v << shift;
    }
    out
}

fn put(image: &mut Image, x: i64, y: i64, color: u32) {
    if x >= 0 && y >= 0 && (x as usize) < image.w && (y as usize) < image.h {
        let i = y as usize * image.w + x as usize;
        image.px[i] = over(image.px[i], color);
    }
}

fn disc(image: &mut Image, cx: f64, cy: f64, radius: f64, color: u32) {
    let r = radius.ceil() as i64;
    let (ix, iy) = (cx.round() as i64, cy.round() as i64);
    for dy in -r..=r {
        for dx in -r..=r {
            let (px, py) = ((ix + dx) as f64 - cx, (iy + dy) as f64 - cy);
            if px * px + py * py <= radius * radius {
                put(image, ix + dx, iy + dy, color);
            }
        }
    }
}

fn line(image: &mut Image, from: (f64, f64), to: (f64, f64), width: u32, color: u32) {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let steps = dx.abs().max(dy.abs()).ceil().max(1.0) as usize;
    let radius = (f64::from(width.max(1)) / 2.0).max(0.5);
    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        disc(image, from.0 + dx * t, from.1 + dy * t, radius, color);
    }
}

/// Composes the snapshot: tiles from `source`, then `routes`, then `pins` (a pin is a disc with a white ring).
pub fn render(snapshot: &Snapshot, source: &dyn TileSource, pins: &[Pin], routes: &[Route]) -> Rendered {
    let (w, h) = (snapshot.width as usize, snapshot.height as usize);
    let mut image = Image::new(w, h);
    image.px.iter_mut().for_each(|p| *p = BACKGROUND);
    let (mut drawn, mut missing) = (0, 0);
    for placed in snapshot.tiles() {
        let tile = source.tile(&placed).and_then(|bytes| decode(&bytes));
        let Some(tile) = tile else {
            missing += 1;
            continue;
        };
        drawn += 1;
        let (ox, oy) = (placed.dest_x.round() as i64, placed.dest_y.round() as i64);
        for ty in 0..tile.h {
            for tx in 0..tile.w {
                put(&mut image, ox + tx as i64, oy + ty as i64, tile.px[ty * tile.w + tx]);
            }
        }
    }
    for route in routes {
        let pts: Vec<(f64, f64)> = route.points.iter().map(|&(lat, lon)| snapshot.point(lat, lon)).collect();
        for pair in pts.windows(2) {
            line(&mut image, pair[0], pair[1], route.width, route.color);
        }
    }
    for pin in pins {
        let (x, y) = snapshot.point(pin.latitude, pin.longitude);
        disc(&mut image, x, y, 7.0, pack(255, 255, 255, 255));
        disc(&mut image, x, y, 5.0, pin.color);
    }
    Rendered { image, drawn, missing }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_render::cg::imageio::encode::{encode_png, PngOptions};
    use std::collections::HashMap;

    /// A 256 by 256 tile of one colour, as PNG bytes.
    fn solid(r: u8, g: u8, b: u8) -> Vec<u8> {
        let rgba: Vec<u8> = (0..256 * 256).flat_map(|_| [r, g, b, 255]).collect();
        encode_png(256, 256, &rgba, &PngOptions::default()).unwrap()
    }

    struct Tiles(HashMap<String, Vec<u8>>);

    impl TileSource for Tiles {
        fn tile(&self, p: &Placed) -> Option<Vec<u8>> {
            self.0.get(&format!("{}/{}/{}", p.zoom, p.tile_x, p.tile_y)).cloned()
        }
    }

    fn world_at_zoom_one() -> Tiles {
        let mut m = HashMap::new();
        m.insert("1/0/0".into(), solid(255, 0, 0));
        m.insert("1/1/0".into(), solid(0, 255, 0));
        m.insert("1/0/1".into(), solid(0, 0, 255));
        m.insert("1/1/1".into(), solid(255, 255, 0));
        Tiles(m)
    }

    fn centre_view() -> Snapshot {
        Snapshot { lat: 0.0, lon: 0.0, zoom: 1, width: 256, height: 256 }
    }

    fn at(r: &Rendered, x: usize, y: usize) -> u32 {
        r.image.px[y * r.image.w + x]
    }

    #[test]
    fn four_tiles_land_in_their_quadrants() {
        let r = render(&centre_view(), &world_at_zoom_one(), &[], &[]);
        assert_eq!((r.drawn, r.missing), (4, 0));
        assert_eq!(at(&r, 10, 10), pack(255, 0, 0, 255));
        assert_eq!(at(&r, 200, 10), pack(0, 255, 0, 255));
        assert_eq!(at(&r, 10, 200), pack(0, 0, 255, 255));
        assert_eq!(at(&r, 200, 200), pack(255, 255, 0, 255));
    }

    #[test]
    fn missing_tiles_stay_background_and_are_counted() {
        let mut tiles = world_at_zoom_one();
        tiles.0.remove("1/1/1");
        tiles.0.insert("1/0/1".into(), b"not an image".to_vec());
        let r = render(&centre_view(), &tiles, &[], &[]);
        assert_eq!((r.drawn, r.missing), (2, 2));
        assert_eq!(at(&r, 200, 200), BACKGROUND);
        assert_eq!(at(&r, 10, 200), BACKGROUND);
    }

    #[test]
    fn a_pin_is_a_coloured_disc_with_a_white_ring_at_its_coordinate() {
        let pin = Pin { latitude: 0.0, longitude: 0.0, color: pack(10, 20, 30, 255) };
        let r = render(&centre_view(), &world_at_zoom_one(), &[pin], &[]);
        assert_eq!(at(&r, 128, 128), pack(10, 20, 30, 255));
        assert_eq!(at(&r, 128 + 6, 128), pack(255, 255, 255, 255));
        // outside the ring the tile shows
        assert_eq!(at(&r, 128 + 12, 128 + 12), pack(255, 255, 0, 255));
    }

    #[test]
    fn a_pin_off_the_image_draws_nothing_and_does_not_panic() {
        let far = Pin { latitude: 60.0, longitude: 100.0, color: pack(1, 2, 3, 255) };
        let r = render(&centre_view(), &world_at_zoom_one(), &[far], &[]);
        assert_eq!(at(&r, 10, 10), pack(255, 0, 0, 255));
    }

    #[test]
    fn a_route_is_drawn_between_its_points() {
        let route = Route { points: vec![(0.0, 0.0), (0.0, 20.0)], color: pack(0, 0, 0, 255), width: 3 };
        let r = render(&centre_view(), &world_at_zoom_one(), &[], &[route]);
        // along the equator from the centre to the right: the pixel midway is on the line
        let end = centre_view().point(0.0, 20.0);
        let mid = (128 + ((end.0 as usize - 128) / 2), 128);
        assert_eq!(at(&r, mid.0, mid.1), pack(0, 0, 0, 255));
        assert_eq!(at(&r, 20, 20), pack(255, 0, 0, 255));
    }

    #[test]
    fn a_tile_directory_reads_the_pack_layout() {
        let dir = tempfile::tempdir().unwrap();
        let tile = dir.path().join("1").join("0");
        std::fs::create_dir_all(&tile).unwrap();
        std::fs::write(tile.join("0.png"), solid(9, 9, 9)).unwrap();
        let source = TileDirectory::new(dir.path());
        let r = render(&centre_view(), &source, &[], &[]);
        assert_eq!((r.drawn, r.missing), (1, 3));
        assert_eq!(at(&r, 10, 10), pack(9, 9, 9, 255));
    }

    #[test]
    fn over_blends_premultiplied_pixels() {
        assert_eq!(over(pack(100, 100, 100, 255), pack(0, 0, 0, 0)), pack(100, 100, 100, 255));
        assert_eq!(over(pack(100, 100, 100, 255), pack(50, 0, 0, 255)), pack(50, 0, 0, 255));
        let half = over(pack(100, 100, 100, 255), pack(100, 0, 0, 128));
        let r = half & 255;
        assert!((r as i32 - (100 + 50)).abs() <= 2, "{r}");
    }
}
