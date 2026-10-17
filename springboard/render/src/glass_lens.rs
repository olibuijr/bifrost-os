//! Edge lensing and rim highlight for glass layers, as a pure model (std only).
//!
//! A glass panel bends the content behind it most near its edge, in a band the width of
//! the bevel, and it catches light along that edge. `sample_offset` gives, for a point
//! inside a rounded rectangle, the displacement to apply when reading the backdrop: it
//! points toward the edge and grows as the point nears the edge, with a smooth fall-off
//! to zero at the bevel width. `rim_highlight` gives the specular weight of a surface
//! normal against a 2D light direction. No shader runs here; the compositor calls these
//! for the per-pixel work it does on the CPU or uploads to a kernel.

/// The panel the lens covers, in local pixels with the origin at the top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Panel {
    pub width: f32,
    pub height: f32,
    pub radius: f32,
}

/// Signed distance from a point to the rounded rectangle, in pixels. Negative inside.
fn signed_distance(panel: Panel, x: f32, y: f32) -> f32 {
    let radius = panel.radius.clamp(0.0, panel.width.min(panel.height) * 0.5);
    let half_w = panel.width * 0.5 - radius;
    let half_h = panel.height * 0.5 - radius;
    let px = (x - panel.width * 0.5).abs() - half_w;
    let py = (y - panel.height * 0.5).abs() - half_h;
    let outside = (px.max(0.0) * px.max(0.0) + py.max(0.0) * py.max(0.0)).sqrt();
    let inside = px.max(py).min(0.0);
    outside + inside - radius
}

/// The outward unit normal of the panel edge at a point, from central differences of the
/// signed distance. Returns `None` when the gradient is degenerate.
fn outward_normal(panel: Panel, x: f32, y: f32) -> Option<(f32, f32)> {
    let e = 0.5;
    let gx = signed_distance(panel, x + e, y) - signed_distance(panel, x - e, y);
    let gy = signed_distance(panel, x, y + e) - signed_distance(panel, x, y - e);
    let len = (gx * gx + gy * gy).sqrt();
    if len < 1e-6 {
        None
    } else {
        Some((gx / len, gy / len))
    }
}

/// Backdrop sample displacement at a point, in pixels: `(dx, dy)` to add to the sample
/// position. The point must lie inside the panel. `bevel` is the lensing band width in
/// pixels and `strength` the displacement at the very edge. Outside the panel, or at a
/// point deeper than the bevel, the displacement is zero. Returns `None` for a point
/// outside the panel, a panel with no area, or a non-positive bevel.
pub fn sample_offset(panel: Panel, x: f32, y: f32, bevel: f32, strength: f32) -> Option<(f32, f32)> {
    if panel.width <= 0.0 || panel.height <= 0.0 || bevel <= 0.0 {
        return None;
    }
    let d = signed_distance(panel, x, y);
    if d > 0.0 {
        return None;
    }
    let depth = -d;
    if depth >= bevel {
        return Some((0.0, 0.0));
    }
    // t runs 1 at the edge to 0 at the inner end of the bevel; smoothstep gives the fall-off.
    let t = 1.0 - depth / bevel;
    let weight = t * t * (3.0 - 2.0 * t);
    match outward_normal(panel, x, y) {
        // Sample from the direction of the edge: the offset is the inward (negative normal) push.
        Some((nx, ny)) => Some((-nx * strength * weight, -ny * strength * weight)),
        None => Some((0.0, 0.0)),
    }
}

/// Specular weight, 0 to 1, of a surface edge with the given outward normal lit from the
/// given direction. `light` points from the surface toward the light; the highlight is
/// strongest where the normal faces the light and zero where it faces away.
pub fn rim_highlight(normal: (f32, f32), light: (f32, f32)) -> f32 {
    let len = (light.0 * light.0 + light.1 * light.1).sqrt();
    if len <= 0.0 {
        return 0.0;
    }
    let dot = normal.0 * light.0 / len + normal.1 * light.1 / len;
    dot.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel() -> Panel {
        Panel { width: 200.0, height: 100.0, radius: 20.0 }
    }

    #[test]
    fn points_outside_the_panel_have_no_offset() {
        assert_eq!(sample_offset(panel(), -1.0, 50.0, 10.0, 8.0), None);
        assert_eq!(sample_offset(panel(), 100.0, 101.0, 10.0, 8.0), None);
    }

    #[test]
    fn a_degenerate_panel_or_bevel_gives_no_offset() {
        assert_eq!(sample_offset(Panel { width: 0.0, height: 10.0, radius: 0.0 }, 0.0, 0.0, 4.0, 2.0), None);
        assert_eq!(sample_offset(panel(), 100.0, 50.0, 0.0, 8.0), None);
    }

    #[test]
    fn the_middle_of_the_panel_is_not_lensed() {
        assert_eq!(sample_offset(panel(), 100.0, 50.0, 10.0, 8.0), Some((0.0, 0.0)));
    }

    #[test]
    fn the_left_edge_pushes_the_sample_inward_by_nearly_the_full_strength() {
        let (dx, dy) = sample_offset(panel(), 0.5, 50.0, 10.0, 8.0).unwrap();
        assert!(dx > 7.0 && dx <= 8.0, "dx {dx} should be close to the strength 8");
        assert!(dy.abs() < 1e-3, "no vertical push on a vertical edge, got {dy}");
    }

    #[test]
    fn the_offset_falls_off_to_zero_at_the_bevel_width() {
        let (near, _) = sample_offset(panel(), 1.0, 50.0, 10.0, 8.0).unwrap();
        let (mid, _) = sample_offset(panel(), 5.0, 50.0, 10.0, 8.0).unwrap();
        assert!(near > mid && mid > 0.0);
        let (edge_of_bevel, _) = sample_offset(panel(), 10.0, 50.0, 10.0, 8.0).unwrap();
        assert!(edge_of_bevel.abs() < 1e-3);
    }

    #[test]
    fn the_top_left_corner_pushes_diagonally_inward() {
        // (6.5, 6.5) is inside the 20 px corner arc, about 1 px from it along the diagonal.
        let (dx, dy) = sample_offset(panel(), 6.5, 6.5, 10.0, 8.0).unwrap();
        assert!(dx > 0.0 && dy > 0.0, "got ({dx}, {dy})");
        assert!((dx - dy).abs() < 1e-3, "the corner is symmetric about its diagonal");
    }

    #[test]
    fn rim_highlight_is_strongest_facing_the_light_and_zero_facing_away() {
        let light = (-1.0, -1.0);
        let facing = rim_highlight((-1.0, 0.0), light);
        let away = rim_highlight((1.0, 0.0), light);
        assert!(facing > 0.7 && facing <= 1.0);
        assert_eq!(away, 0.0);
        assert_eq!(rim_highlight((0.0, 1.0), (0.0, 0.0)), 0.0, "no light, no highlight");
    }
}
