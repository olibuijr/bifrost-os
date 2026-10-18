//! Places: the photos that carry an EXIF GPS position, grouped for the map view. The grouping is pure: the photos are
//! projected to Web Mercator pixels at the zoom that fits them all in the map area, then greedily clustered by their
//! distance on screen, so a cluster splits up as the window grows. No map tiles are involved here; the view draws the
//! pins over its own backdrop.

use std::io::Read;
use std::path::Path;

use bifrost_ui::map_snapshot::Snapshot;

/// The most zoom a Places map ever uses (street level).
pub const MAX_ZOOM: u32 = 16;
/// Empty space kept around the outermost pins, in pixels.
const MARGIN_PX: u32 = 32;
/// How much of an image is read to find its GPS position (the APP1 block sits at the start).
const HEAD: u64 = 64 * 1024;

/// The GPS position of the picture at `path` (latitude, longitude), when it has one.
pub fn gps_of(path: &Path) -> Option<(f64, f64)> {
    let mut head = Vec::new();
    std::fs::File::open(path).ok()?.take(HEAD).read_to_end(&mut head).ok()?;
    super::exif::parse_image(&head)?.gps
}

/// Photos that sit within one pin on screen: where the pin is drawn, and which photos it stands for.
#[derive(Clone, Debug, PartialEq)]
pub struct Cluster {
    /// the pin's position in map-area pixels
    pub x: f64,
    pub y: f64,
    /// indices into the points given to `layout`
    pub members: Vec<usize>,
}

/// Places the points (latitude, longitude) on a `width` x `height` map area and clusters them so that no two pins
/// are closer than `cell` pixels. The zoom is the largest that keeps every point inside the area.
pub fn snapshot_for(points: &[(f64, f64)], width: u32, height: u32) -> Option<Snapshot> {
    if points.is_empty() || width == 0 || height == 0 {
        return None;
    }
    let n = points.len() as f64;
    let lat = points.iter().map(|p| p.0).sum::<f64>() / n;
    let lon = points.iter().map(|p| p.1).sum::<f64>() / n;
    let zoom = Snapshot::fit_zoom(points, width, height, MARGIN_PX, MAX_ZOOM);
    Some(Snapshot { lat, lon, zoom, width, height })
}

/// Clusters the points on the map area, placed with the same snapshot the map tiles are drawn with.
pub fn layout(points: &[(f64, f64)], width: u32, height: u32, cell: f64) -> Vec<Cluster> {
    let Some(snap) = snapshot_for(points, width, height) else { return Vec::new() };
    let mut out: Vec<Cluster> = Vec::new();
    for (i, &(plat, plon)) in points.iter().enumerate() {
        let (x, y) = snap.point(plat, plon);
        let near = out.iter().position(|c| ((c.x - x).powi(2) + (c.y - y).powi(2)).sqrt() < cell);
        match near {
            Some(k) => {
                let c = &mut out[k];
                let m = c.members.len() as f64;
                c.x = (c.x * m + x) / (m + 1.0);
                c.y = (c.y * m + y) / (m + 1.0);
                c.members.push(i);
            }
            None => out.push(Cluster { x, y, members: vec![i] }),
        }
    }
    out
}

/// The cluster whose pin is under (`x`, `y`), given a pin radius of `radius` pixels.
pub fn cluster_at(clusters: &[Cluster], x: f64, y: f64, radius: f64) -> Option<usize> {
    clusters.iter().position(|c| ((c.x - x).powi(2) + (c.y - y).powi(2)).sqrt() <= radius)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearby_points_share_a_pin_and_distant_ones_do_not() {
        // Reykjavik twice and Sydney once: the two Reykjavik photos are a few metres apart
        let pts = [(64.1466, -21.9426), (64.1467, -21.9427), (-33.8688, 151.2093)];
        let clusters = layout(&pts, 800, 600, 40.0);
        assert_eq!(clusters.len(), 2);
        let mut sizes: Vec<usize> = clusters.iter().map(|c| c.members.len()).collect();
        sizes.sort();
        assert_eq!(sizes, vec![1, 2]);
        assert!(clusters.iter().all(|c| c.x >= 0.0 && c.x <= 800.0 && c.y >= 0.0 && c.y <= 600.0));
    }

    #[test]
    fn a_bigger_cell_merges_more() {
        let pts = [(64.0, -21.0), (64.2, -21.2), (64.4, -21.4)];
        let fine = layout(&pts, 1000, 800, 5.0);
        let coarse = layout(&pts, 1000, 800, 5000.0);
        assert_eq!(fine.len(), 3);
        assert_eq!(coarse.len(), 1);
        assert_eq!(coarse[0].members.len(), 3);
    }

    #[test]
    fn empty_input_and_empty_area_give_no_pins() {
        assert!(layout(&[], 800, 600, 40.0).is_empty());
        assert!(layout(&[(1.0, 2.0)], 0, 600, 40.0).is_empty());
    }

    #[test]
    fn a_tap_picks_the_pin_under_it() {
        let clusters = vec![Cluster { x: 100.0, y: 100.0, members: vec![0] }, Cluster { x: 300.0, y: 100.0, members: vec![1, 2] }];
        assert_eq!(cluster_at(&clusters, 104.0, 98.0, 18.0), Some(0));
        assert_eq!(cluster_at(&clusters, 300.0, 120.0, 18.0), None);
        assert_eq!(cluster_at(&clusters, 290.0, 100.0, 18.0), Some(1));
    }

    #[test]
    fn reads_the_gps_position_of_a_jpeg_file() {
        use super::super::exif::tests::{ascii, jpeg, rationals, tiff};
        // 64 deg 8' 48" N, 21 deg 56' 0" W
        let gps = vec![ascii(1, "N"), rationals(true, 2, &[(64, 1), (8, 1), (48, 1)]), ascii(3, "W"), rationals(true, 4, &[(21, 1), (56, 1), (0, 1)])];
        let bytes = jpeg(&tiff(true, vec![], vec![], gps), 640, 480);
        let path = std::env::temp_dir().join(format!("places-gps-{}.jpg", std::process::id()));
        std::fs::write(&path, &bytes).unwrap();
        let got = gps_of(&path);
        let _ = std::fs::remove_file(&path);
        let (lat, lon) = got.expect("the fixture has a position");
        assert!((lat - 64.14667).abs() < 1e-4 && (lon + 21.93333).abs() < 1e-4, "{lat} {lon}");
    }

    #[test]
    fn a_missing_file_has_no_position() {
        assert_eq!(gps_of(Path::new("/nonexistent/photo.jpg")), None);
    }
}
