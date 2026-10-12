//! Surface plots for 3D charts: a function of two variables sampled on a grid, turned into quads in screen points.
//! The height is the z value. A pose turns the surface about the vertical axis (yaw) and then tips it toward the
//! viewer (pitch). The projection is orthographic and fits the surface into the frame. Pure functions over f64;
//! the renderer draws the quads in the order they come back, which is back to front.

/// A grid of heights sampled from a function. Points are stored row by row: `points[row * cols + col]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Surface {
    pub cols: usize,
    pub rows: usize,
    points: Vec<[f64; 3]>,
}

/// Camera pose in radians. Zero is the flat plot facing the viewer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pose {
    pub yaw: f64,
    pub pitch: f64,
}

/// One grid cell in screen points, with its depth and how squarely it faces the viewer.
#[derive(Clone, Debug, PartialEq)]
pub struct Quad {
    /// Corners in grid order: (col, row), (col+1, row), (col+1, row+1), (col, row+1).
    pub corners: [(f64, f64); 4],
    /// Mean depth after the pose. Larger is closer to the viewer.
    pub depth: f64,
    /// 1.0 when the cell faces the viewer, 0.0 when it is edge-on.
    pub facing: f64,
}

/// Samples `f(x, y)` on a `cols` by `rows` grid spanning the x and y ranges, both ends included. Returns nothing
/// when a grid side has fewer than two samples, a range is not finite or is empty, or `f` gives a height that is
/// not finite anywhere on the grid.
pub fn sample<F: Fn(f64, f64) -> f64>(f: F, x: (f64, f64), y: (f64, f64), cols: usize, rows: usize) -> Option<Surface> {
    if cols < 2 || rows < 2 {
        return None;
    }
    if ![x.0, x.1, y.0, y.1].iter().all(|v| v.is_finite()) || x.0 == x.1 || y.0 == y.1 {
        return None;
    }
    let mut points = Vec::with_capacity(cols * rows);
    for row in 0..rows {
        let py = y.0 + (y.1 - y.0) * row as f64 / (rows - 1) as f64;
        for col in 0..cols {
            let px = x.0 + (x.1 - x.0) * col as f64 / (cols - 1) as f64;
            let pz = f(px, py);
            if !pz.is_finite() {
                return None;
            }
            points.push([px, py, pz]);
        }
    }
    Some(Surface { cols, rows, points })
}

/// Turns the pose into a point: yaw about the z axis, then pitch about the x axis that runs across the plot.
fn rotate(p: [f64; 3], pose: Pose) -> [f64; 3] {
    let (sy, cy) = pose.yaw.sin_cos();
    let x1 = p[0] * cy - p[1] * sy;
    let y1 = p[0] * sy + p[1] * cy;
    let (sp, cp) = pose.pitch.sin_cos();
    let y2 = y1 * cp - p[2] * sp;
    let z2 = y1 * sp + p[2] * cp;
    [x1, y2, z2]
}

/// Projects the surface into a `w` by `h` frame. The surface is centred and scaled uniformly so that its wider side
/// fills the frame; screen y grows downward. Returns nothing for a frame that is not positive. The quads come back
/// sorted by depth, farthest first, so drawing them in order paints the near cells over the far ones.
pub fn project(surface: &Surface, pose: Pose, w: f64, h: f64) -> Vec<Quad> {
    if !(w > 0.0 && h > 0.0) {
        return Vec::new();
    }
    let rot: Vec<[f64; 3]> = surface.points.iter().map(|&p| rotate(p, pose)).collect();
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for r in &rot {
        for k in 0..2 {
            lo[k] = lo[k].min(r[k]);
            hi[k] = hi[k].max(r[k]);
        }
    }
    let ext = [hi[0] - lo[0], hi[1] - lo[1]];
    let fit = |span: f64, side: f64| if span > 0.0 { side / span } else { f64::INFINITY };
    let mut scale = fit(ext[0], w).min(fit(ext[1], h));
    if !scale.is_finite() {
        scale = 1.0;
    }
    let mid = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
    let screen = |r: &[f64; 3]| (w / 2.0 + (r[0] - mid[0]) * scale, h / 2.0 - (r[1] - mid[1]) * scale);

    let mut quads = Vec::with_capacity((surface.cols - 1) * (surface.rows - 1));
    for row in 0..surface.rows - 1 {
        for col in 0..surface.cols - 1 {
            let a = row * surface.cols + col;
            let (b, c, d) = (a + 1, a + surface.cols + 1, a + surface.cols);
            let depth = (rot[a][2] + rot[b][2] + rot[c][2] + rot[d][2]) / 4.0;
            // Normal of the cell from its two edges at corner a, taken from the rotated points.
            let u = [rot[b][0] - rot[a][0], rot[b][1] - rot[a][1], rot[b][2] - rot[a][2]];
            let v = [rot[d][0] - rot[a][0], rot[d][1] - rot[a][1], rot[d][2] - rot[a][2]];
            let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            let facing = if len > 0.0 { n[2].abs() / len } else { 0.0 };
            quads.push(Quad {
                corners: [screen(&rot[a]), screen(&rot[b]), screen(&rot[c]), screen(&rot[d])],
                depth,
                facing,
            });
        }
    }
    quads.sort_by(|p, q| p.depth.total_cmp(&q.depth));
    quads
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn sampling_fills_the_grid_end_to_end() {
        let s = sample(|x, y| x + y, (0.0, 2.0), (0.0, 1.0), 3, 2).unwrap();
        assert_eq!(s.cols, 3);
        assert_eq!(s.rows, 2);
        // Row 1 starts at y = 1, column 2 is x = 2, so the last height is 3.
        assert!(close(s.points[5][2], 3.0));
        assert!(close(s.points[0][2], 0.0));
    }

    #[test]
    fn a_grid_smaller_than_two_by_two_or_a_bad_range_samples_nothing() {
        assert!(sample(|_, _| 0.0, (0.0, 1.0), (0.0, 1.0), 1, 4).is_none());
        assert!(sample(|_, _| 0.0, (1.0, 1.0), (0.0, 1.0), 4, 4).is_none());
        assert!(sample(|_, _| 0.0, (0.0, f64::NAN), (0.0, 1.0), 4, 4).is_none());
    }

    #[test]
    fn a_height_that_is_not_finite_rejects_the_whole_surface() {
        let s = sample(|x, _| if x > 0.5 { f64::INFINITY } else { 0.0 }, (0.0, 1.0), (0.0, 1.0), 3, 3);
        assert!(s.is_none());
    }

    #[test]
    fn a_flat_surface_facing_the_viewer_fills_the_frame() {
        let s = sample(|_, _| 0.0, (0.0, 1.0), (0.0, 1.0), 2, 2).unwrap();
        let quads = project(&s, Pose::default(), 100.0, 100.0);
        assert_eq!(quads.len(), 1);
        assert!(close(quads[0].facing, 1.0));
        // The unit square fills the 100 by 100 frame, with y flipped: the (0, 0) corner is at the bottom left.
        let xs: Vec<f64> = quads[0].corners.iter().map(|c| c.0).collect();
        let ys: Vec<f64> = quads[0].corners.iter().map(|c| c.1).collect();
        assert!(close(xs.iter().cloned().fold(f64::INFINITY, f64::min), 0.0));
        assert!(close(xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max), 100.0));
        assert!(close(ys.iter().cloned().fold(f64::INFINITY, f64::min), 0.0));
        assert!(close(ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max), 100.0));
        assert!(close(quads[0].corners[0].1, 100.0));
    }

    #[test]
    fn a_flat_surface_tipped_a_quarter_turn_is_edge_on() {
        let s = sample(|_, _| 0.0, (0.0, 1.0), (0.0, 1.0), 2, 2).unwrap();
        let pose = Pose { yaw: 0.0, pitch: std::f64::consts::FRAC_PI_2 };
        let quads = project(&s, pose, 100.0, 100.0);
        assert!(quads[0].facing < 1e-9);
    }

    #[test]
    fn a_grid_gives_one_quad_per_cell_back_to_front() {
        let s = sample(|_, y| y, (0.0, 1.0), (0.0, 1.0), 3, 4).unwrap();
        let quads = project(&s, Pose { yaw: 0.3, pitch: 0.5 }, 200.0, 120.0);
        assert_eq!(quads.len(), 2 * 3);
        assert!(quads.windows(2).all(|p| p[0].depth <= p[1].depth));
    }

    #[test]
    fn a_frame_that_is_not_positive_draws_nothing() {
        let s = sample(|_, _| 0.0, (0.0, 1.0), (0.0, 1.0), 2, 2).unwrap();
        assert!(project(&s, Pose::default(), 0.0, 100.0).is_empty());
        assert!(project(&s, Pose::default(), 100.0, -1.0).is_empty());
    }
}
