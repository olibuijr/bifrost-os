//! Parity sdk-complete-37: headless tests for the tile cache of a streaming image provider and for
//! BT.2390 tone mapping of HDR luminance. No test decodes a picture or opens a display.

use bifrost_render::hdr_transfer::{pq_encode, tone_map_nits};
use bifrost_render::image_tiles::{Tile, TileCache, TileGrid};

fn tile(col: usize, row: usize) -> Tile {
    Tile { col, row, x: col * 128, y: row * 128, w: 128, h: 128 }
}

#[test]
fn a_cache_with_no_room_is_refused() {
    assert!(TileCache::<u8>::new(0).is_none());
    assert!(TileCache::<u8>::new(1).is_some());
}

#[test]
fn a_missing_tile_is_loaded_once_and_then_held() {
    let mut cache: TileCache<Vec<u8>> = TileCache::new(4).unwrap();
    let mut loads = 0;
    let first = cache.get_or_load(tile(0, 0), |t| {
        loads += 1;
        Some(vec![t.col as u8; 3])
    });
    assert_eq!(first, Some(&vec![0u8, 0, 0]));
    let again = cache.get_or_load(tile(0, 0), |_| {
        loads += 1;
        Some(vec![9])
    });
    assert_eq!(again, Some(&vec![0u8, 0, 0]));
    assert_eq!(loads, 1, "a held tile is not decoded again");
    assert_eq!(cache.len(), 1);
}

#[test]
fn a_provider_that_cannot_decode_leaves_nothing_behind() {
    let mut cache: TileCache<u8> = TileCache::new(2).unwrap();
    assert_eq!(cache.get_or_load(tile(1, 1), |_| None), None);
    assert!(cache.is_empty());
    assert!(!cache.contains(1, 1));
}

#[test]
fn the_least_recently_used_tile_goes_when_the_cache_is_full() {
    let mut cache: TileCache<u8> = TileCache::new(2).unwrap();
    cache.get_or_load(tile(0, 0), |_| Some(10));
    cache.get_or_load(tile(1, 0), |_| Some(11));
    // Touching (0, 0) makes (1, 0) the oldest.
    assert_eq!(cache.get_or_load(tile(0, 0), |_| Some(99)), Some(&10));
    cache.get_or_load(tile(2, 0), |_| Some(12));
    assert_eq!(cache.len(), 2);
    assert!(cache.contains(0, 0) && cache.contains(2, 0));
    assert!(!cache.contains(1, 0), "the oldest tile was evicted");
}

#[test]
fn missing_lists_only_the_tiles_a_provider_still_has_to_load() {
    let mut cache: TileCache<u8> = TileCache::new(8).unwrap();
    cache.get_or_load(tile(1, 0), |_| Some(1));
    let grid = TileGrid::new(384, 128, 128, 128).unwrap();
    let wanted: Vec<Tile> = grid.covering(0, 0, 384, 128);
    let missing: Vec<(usize, usize)> = cache.missing(&wanted).iter().map(|t| (t.col, t.row)).collect();
    assert_eq!(missing, vec![(0, 0), (2, 0)]);
}

#[test]
fn a_streamed_viewport_decodes_only_the_tiles_on_screen() {
    // A 1000 x 600 picture seen through a 300 x 200 window at (700, 0): three tile columns and two rows.
    let grid = TileGrid::new(1000, 600, 256, 256).unwrap();
    let mut cache: TileCache<u8> = TileCache::new(16).unwrap();
    let mut decoded = Vec::new();
    for t in grid.covering(700, 0, 300, 200) {
        cache.get_or_load(t, |t| {
            decoded.push((t.col, t.row));
            Some(0)
        });
    }
    assert_eq!(decoded, vec![(2, 0), (3, 0)], "columns 2 and 3 cover x 700..1000 on the first row");
    assert_eq!(cache.len(), 2);
}

#[test]
fn the_source_peak_maps_to_the_display_peak() {
    let out = tone_map_nits(1000.0, 1000.0, 400.0).unwrap();
    assert!((out - 400.0).abs() < 1e-4, "got {out}");
}

#[test]
fn luminance_below_the_knee_is_unchanged() {
    let out = tone_map_nits(10.0, 1000.0, 400.0).unwrap();
    assert!((out - 10.0).abs() < 1e-6, "got {out}");
}

#[test]
fn tone_mapping_is_monotonic_and_never_exceeds_the_display_peak() {
    let mut last = 0.0;
    for step in 0..=100 {
        let nits = 1000.0 * step as f64 / 100.0;
        let out = tone_map_nits(nits, 1000.0, 400.0).unwrap();
        assert!(out >= last - 1e-9, "{nits} nits went down to {out}");
        assert!(out <= 400.0 + 1e-6, "{nits} nits came out at {out}");
        last = out;
    }
}

#[test]
fn a_display_at_least_as_bright_as_the_source_keeps_the_luminance() {
    assert_eq!(tone_map_nits(800.0, 1000.0, 4000.0), Some(800.0));
    assert_eq!(tone_map_nits(1200.0, 1000.0, 1000.0), Some(1000.0), "input is clamped to the source peak");
    assert_eq!(tone_map_nits(-5.0, 1000.0, 400.0), Some(0.0));
}

#[test]
fn invalid_peaks_are_refused() {
    assert_eq!(tone_map_nits(100.0, 0.0, 400.0), None);
    assert_eq!(tone_map_nits(100.0, 1000.0, -1.0), None);
    assert_eq!(tone_map_nits(100.0, f64::NAN, 400.0), None);
}

#[test]
fn the_roll_off_keeps_the_code_range_of_the_perceptual_quantizer() {
    // The mapped value stays a valid PQ input: encoding it gives a code between 0 and 1.
    let out = tone_map_nits(1000.0, 1000.0, 100.0).unwrap();
    let code = pq_encode(out);
    assert!(code > 0.0 && code <= 1.0, "code {code}");
}
