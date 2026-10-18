//! Map pictures: the tiles that cover a view, loaded and composed into one RGB image. The Maps app prints with it, and
//! the SDK renders map snapshots with it, so both draw the same tiles the same way.

use super::loader::TileLoader;
use super::tiles::{lonlat_to_world, tile_level, tiles_in_view, TileId, TILE_SIZE};
use crate::gfx::Image;
use std::time::{Duration, Instant};

/// An RGB picture: `width * height * 3` bytes, rows from the top left.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}

/// Draws loaded tiles into an RGB picture of `width` by `height` points at `scale` pixels a point. Each tile is `side`
/// points square, its top left corner at `(x, y)` from the top left of the map. The tiles are premultiplied pixels;
/// they are laid over white, which is what a tile that has not loaded leaves behind.
pub fn draw_tiles(tiles: &[(f64, f64, &Image)], side: f64, width: f64, height: f64, scale: f64) -> Picture {
    let k = scale;
    let w = (width * k).ceil().max(1.0) as usize;
    let h = (height * k).ceil().max(1.0) as usize;
    let mut rgb = vec![255u8; w * h * 3];
    let side_px = (side * k).round() as i64;
    if side_px <= 0 {
        return Picture { width: w, height: h, rgb };
    }
    for &(x, y, img) in tiles {
        if img.w == 0 || img.h == 0 || img.px.len() < img.w * img.h {
            continue;
        }
        let (x0, y0) = ((x * k).round() as i64, (y * k).round() as i64);
        for dy in 0..side_px {
            let oy = y0 + dy;
            if oy < 0 || oy >= h as i64 {
                continue;
            }
            let sy = (dy as usize * img.h / side_px as usize).min(img.h - 1);
            for dx in 0..side_px {
                let ox = x0 + dx;
                if ox < 0 || ox >= w as i64 {
                    continue;
                }
                let sx = (dx as usize * img.w / side_px as usize).min(img.w - 1);
                let p = img.px[sy * img.w + sx];
                let (r, g, b, a) = (p & 0xff, (p >> 8) & 0xff, (p >> 16) & 0xff, p >> 24);
                // premultiplied colour over white: the white shows through by the part that is transparent
                let clear = 255 - a.min(255);
                let i = (oy as usize * w + ox as usize) * 3;
                rgb[i] = (r + clear).min(255) as u8;
                rgb[i + 1] = (g + clear).min(255) as u8;
                rgb[i + 2] = (b + clear).min(255) as u8;
            }
        }
    }
    Picture { width: w, height: h, rgb }
}

/// Renders the map centred on (`lat`, `lon`) at `zoom` into a `width` by `height` point picture at `scale` pixels a
/// point. Asks the loader for the tiles of the view and waits for them, for at most `timeout`. A tile that never
/// arrives (offline, or refused) is left white, so the picture is always the view's size.
#[allow(clippy::too_many_arguments)]
pub fn snapshot(loader: &mut TileLoader, lat: f64, lon: f64, zoom: f64, width: f64, height: f64, scale: f64, timeout: Duration) -> Picture {
    let (cx, cy) = lonlat_to_world(lat, lon, zoom);
    let placed = tiles_in_view(cx, cy, width, height, zoom);
    let ids: Vec<TileId> = placed.iter().map(|(t, _, _)| *t).collect();
    let side = TILE_SIZE * tile_level(zoom).1;
    let started = Instant::now();
    let mut asked = false;
    loop {
        loader.want(&ids);
        loader.pump();
        let all = ids.iter().all(|t| loader.image(*t).is_some());
        // a loader with nothing left in flight has done all it can: missing tiles stay white
        if all || (asked && !loader.busy()) || started.elapsed() >= timeout {
            break;
        }
        asked = true;
        std::thread::sleep(Duration::from_millis(20));
    }
    let loaded: Vec<(f64, f64, &Image)> = placed.iter().filter_map(|(t, x, y)| loader.image(*t).map(|img| (*x, *y, img))).collect();
    draw_tiles(&loaded, side, width, height, scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, rgba: u32) -> Image {
        Image { w, h, px: vec![rgba; w * h] }
    }

    #[test]
    fn a_tile_is_drawn_where_it_is_on_the_map_at_scale() {
        let red = solid(4, 4, 0xff00_00ff);
        // a 10 point tile at (10, 5) on a 40 x 30 map, at 2 pixels a point
        let picture = draw_tiles(&[(10.0, 5.0, &red)], 10.0, 40.0, 30.0, 2.0);
        assert_eq!((picture.width, picture.height), (80, 60));
        let px = |x: usize, y: usize| {
            let i = (y * picture.width + x) * 3;
            [picture.rgb[i], picture.rgb[i + 1], picture.rgb[i + 2]]
        };
        assert_eq!(px(19, 10), [255, 255, 255], "left of the tile is white");
        assert_eq!(px(20, 10), [255, 0, 0], "the tile starts at point 10 across, pixel 20");
        assert_eq!(px(40, 10), [255, 255, 255], "right of the tile is white");
    }

    #[test]
    fn an_empty_map_is_all_white_at_the_view_size() {
        let picture = draw_tiles(&[], 256.0, 10.0, 5.0, 1.0);
        assert_eq!((picture.width, picture.height), (10, 5));
        assert!(picture.rgb.iter().all(|&b| b == 255));
    }
}
