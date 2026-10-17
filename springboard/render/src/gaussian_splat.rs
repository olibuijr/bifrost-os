//! 3D Gaussian splat rasteriser, CPU reference (std only), after the public description in Kerbl, Kopanas, Leimkuhler
//! and Drettakis, "3D Gaussian Splatting for Real-Time Radiance Field Rendering", and the EWA splatting projection of
//! Zwicker et al. Each Gaussian has a centre, a per-axis scale, a rotation quaternion, a colour and an opacity. Its 3D
//! covariance `R S S^T R^T` is projected to a 2D covariance with the Jacobian of the perspective map, a 0.3 pixel
//! low-pass term is added, and the inverse (the conic) gives the falloff `exp(-1/2 d^T conic d)` around the screen
//! position. Splats are composited front to back by view depth with alpha capped at 0.99, skipping contributions below
//! 1/255 and stopping a pixel once its transmittance is below 1e-4.
//!
//! The camera looks down +z in its own space (`view` maps world to camera), with pixel focal lengths `fx`, `fy` and
//! principal point `(cx, cy)`. Higher spherical-harmonic bands (view-dependent colour) are not evaluated: colour is the
//! degree-0 term, converted with `sh0_to_rgb`.

/// The degree-0 spherical-harmonic basis constant `1 / (2 sqrt(pi))`.
pub const SH_C0: f32 = 0.282_094_8;

/// One Gaussian.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gaussian {
    pub position: [f32; 3],
    /// Standard deviations along the Gaussian's own axes, in world units.
    pub scale: [f32; 3],
    /// Rotation quaternion `(w, x, y, z)`; normalised on use (a zero quaternion is the identity).
    pub rotation: [f32; 4],
    pub color: [f32; 3],
    /// Opacity in 0..=1.
    pub opacity: f32,
}

/// A pinhole camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// World to camera, rows of `[R | t]`.
    pub view: [[f32; 4]; 3],
    pub fx: f32,
    pub fy: f32,
    pub cx: f32,
    pub cy: f32,
    pub width: usize,
    pub height: usize,
    /// Splats nearer than this view depth are dropped.
    pub near: f32,
}

/// A Gaussian projected to the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Splat {
    pub mean: [f32; 2],
    pub depth: f32,
    /// The inverse 2D covariance as `[xx, xy, yy]`.
    pub conic: [f32; 3],
    /// Pixel radius that holds 3 standard deviations.
    pub radius: f32,
    pub color: [f32; 3],
    pub opacity: f32,
}

/// Colour from the degree-0 SH coefficients: `0.5 + C0 * c`, clamped at 0 below.
pub fn sh0_to_rgb(c: [f32; 3]) -> [f32; 3] {
    [(0.5 + SH_C0 * c[0]).max(0.0), (0.5 + SH_C0 * c[1]).max(0.0), (0.5 + SH_C0 * c[2]).max(0.0)]
}

/// The rotation matrix of a quaternion `(w, x, y, z)`.
pub fn quat_to_matrix(q: [f32; 4]) -> [[f32; 3]; 3] {
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if n < 1e-12 {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    let (w, x, y, z) = (q[0] / n, q[1] / n, q[2] / n, q[3] / n);
    [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - w * z), 2.0 * (x * z + w * y)],
        [2.0 * (x * y + w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - w * x)],
        [2.0 * (x * z - w * y), 2.0 * (y * z + w * x), 1.0 - 2.0 * (x * x + y * y)],
    ]
}

/// The 3D covariance `R S S^T R^T` (symmetric 3x3).
pub fn covariance3d(scale: [f32; 3], rotation: [f32; 4]) -> [[f32; 3]; 3] {
    let r = quat_to_matrix(rotation);
    let mut m = [[0.0f32; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            m[i][j] = r[i][j] * scale[j];
        }
    }
    let mut c = [[0.0f32; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            c[i][j] = (0..3).map(|k| m[i][k] * m[j][k]).sum();
        }
    }
    c
}

/// Projects a Gaussian. `None` when it is nearer than the camera's `near`, its 2D covariance is degenerate, or its
/// footprint lies entirely outside the image.
pub fn project(g: &Gaussian, cam: &Camera) -> Option<Splat> {
    let p = g.position;
    let v = &cam.view;
    let t = [
        v[0][0] * p[0] + v[0][1] * p[1] + v[0][2] * p[2] + v[0][3],
        v[1][0] * p[0] + v[1][1] * p[1] + v[1][2] * p[2] + v[1][3],
        v[2][0] * p[0] + v[2][1] * p[1] + v[2][2] * p[2] + v[2][3],
    ];
    if !(t[2] > cam.near.max(1e-6)) {
        return None;
    }
    let mean = [cam.fx * t[0] / t[2] + cam.cx, cam.fy * t[1] / t[2] + cam.cy];
    let sigma = covariance3d(g.scale, g.rotation);
    // Jacobian of the perspective map, then J * W where W is the view rotation.
    let j = [[cam.fx / t[2], 0.0, -cam.fx * t[0] / (t[2] * t[2])], [0.0, cam.fy / t[2], -cam.fy * t[1] / (t[2] * t[2])]];
    let mut tm = [[0.0f32; 3]; 2];
    for r in 0..2 {
        for c in 0..3 {
            tm[r][c] = (0..3).map(|k| j[r][k] * v[k][c]).sum();
        }
    }
    let mut a = [[0.0f32; 3]; 2];
    for r in 0..2 {
        for c in 0..3 {
            a[r][c] = (0..3).map(|k| tm[r][k] * sigma[k][c]).sum();
        }
    }
    let mut cov = [[0.0f32; 2]; 2];
    for r in 0..2 {
        for c in 0..2 {
            cov[r][c] = (0..3).map(|k| a[r][k] * tm[c][k]).sum();
        }
    }
    cov[0][0] += 0.3;
    cov[1][1] += 0.3;
    let det = cov[0][0] * cov[1][1] - cov[0][1] * cov[1][0];
    if !(det > 1e-12) {
        return None;
    }
    let conic = [cov[1][1] / det, -cov[0][1] / det, cov[0][0] / det];
    let mid = 0.5 * (cov[0][0] + cov[1][1]);
    let lambda = mid + (mid * mid - det).max(0.1).sqrt();
    let radius = (3.0 * lambda.sqrt()).ceil();
    if mean[0] + radius < 0.0 || mean[0] - radius > cam.width as f32 || mean[1] + radius < 0.0 || mean[1] - radius > cam.height as f32 {
        return None;
    }
    Some(Splat { mean, depth: t[2], conic, radius, color: g.color, opacity: g.opacity.clamp(0.0, 1.0) })
}

/// Renders the Gaussians over `background`. Row-major linear RGB, `width * height` pixels. `None` for an empty image.
pub fn render(gaussians: &[Gaussian], cam: &Camera, background: [f32; 3]) -> Option<Vec<[f32; 3]>> {
    let (w, h) = (cam.width, cam.height);
    if w == 0 || h == 0 {
        return None;
    }
    let mut splats: Vec<Splat> = gaussians.iter().filter_map(|g| project(g, cam)).collect();
    splats.sort_by(|a, b| a.depth.partial_cmp(&b.depth).unwrap_or(std::cmp::Ordering::Equal));
    let mut color = vec![[0.0f32; 3]; w * h];
    let mut trans = vec![1.0f32; w * h];
    for s in &splats {
        let x0 = ((s.mean[0] - s.radius).floor().max(0.0)) as usize;
        let y0 = ((s.mean[1] - s.radius).floor().max(0.0)) as usize;
        let x1 = (((s.mean[0] + s.radius).ceil()).min(w as f32 - 1.0)).max(0.0) as usize;
        let y1 = (((s.mean[1] + s.radius).ceil()).min(h as f32 - 1.0)).max(0.0) as usize;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let i = y * w + x;
                if trans[i] < 1e-4 {
                    continue;
                }
                let dx = x as f32 + 0.5 - s.mean[0];
                let dy = y as f32 + 0.5 - s.mean[1];
                let power = -0.5 * (s.conic[0] * dx * dx + 2.0 * s.conic[1] * dx * dy + s.conic[2] * dy * dy);
                if power > 0.0 {
                    continue;
                }
                let alpha = (s.opacity * power.exp()).min(0.99);
                if alpha < 1.0 / 255.0 {
                    continue;
                }
                for c in 0..3 {
                    color[i][c] += trans[i] * alpha * s.color[c];
                }
                trans[i] *= 1.0 - alpha;
            }
        }
    }
    for i in 0..w * h {
        for c in 0..3 {
            color[i][c] += trans[i] * background[c];
        }
    }
    Some(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> Camera {
        Camera {
            view: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
            fx: 32.0,
            fy: 32.0,
            cx: 16.0,
            cy: 16.0,
            width: 32,
            height: 32,
            near: 0.1,
        }
    }

    fn blob(z: f32, color: [f32; 3], opacity: f32) -> Gaussian {
        Gaussian { position: [0.0, 0.0, z], scale: [0.25, 0.25, 0.25], rotation: [1.0, 0.0, 0.0, 0.0], color, opacity }
    }

    #[test]
    fn quaternions_make_rotations() {
        let id = quat_to_matrix([1.0, 0.0, 0.0, 0.0]);
        assert_eq!(id, [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
        assert_eq!(quat_to_matrix([0.0; 4]), id);
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let rz = quat_to_matrix([h, 0.0, 0.0, h]);
        // 90 degrees about z sends x to y.
        assert!(rz[0][0].abs() < 1e-6 && (rz[1][0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn covariance_of_an_isotropic_gaussian_is_a_scaled_identity() {
        let c = covariance3d([0.5, 0.5, 0.5], [0.3, 0.1, -0.2, 0.9]);
        for i in 0..3 {
            for j in 0..3 {
                let want = if i == j { 0.25 } else { 0.0 };
                assert!((c[i][j] - want).abs() < 1e-6);
            }
        }
        // An elongated axis rotated 90 degrees about z swaps x and y variance.
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let e = covariance3d([2.0, 1.0, 1.0], [h, 0.0, 0.0, h]);
        assert!((e[0][0] - 1.0).abs() < 1e-5 && (e[1][1] - 4.0).abs() < 1e-5);
    }

    #[test]
    fn projection_puts_the_blob_on_the_principal_point() {
        let s = project(&blob(4.0, [1.0, 0.0, 0.0], 1.0), &camera()).unwrap();
        assert!((s.mean[0] - 16.0).abs() < 1e-5 && (s.mean[1] - 16.0).abs() < 1e-5);
        assert_eq!(s.depth, 4.0);
        // Pixel sigma is fx / z * scale = 2, so the covariance is 4 + 0.3.
        assert!((s.conic[0] - 1.0 / 4.3).abs() < 1e-5 && (s.conic[2] - 1.0 / 4.3).abs() < 1e-5 && s.conic[1].abs() < 1e-6);
        assert!(project(&blob(-1.0, [1.0; 3], 1.0), &camera()).is_none());
        assert!(project(&blob(0.05, [1.0; 3], 1.0), &camera()).is_none());
        let mut off = blob(4.0, [1.0; 3], 1.0);
        off.position[0] = 100.0;
        assert!(project(&off, &camera()).is_none());
    }

    #[test]
    fn an_opaque_blob_fills_its_centre_and_leaves_the_corners() {
        let img = render(&[blob(4.0, [1.0, 0.0, 0.0], 1.0)], &camera(), [0.0, 0.0, 0.0]).unwrap();
        let centre = img[16 * 32 + 16];
        assert!(centre[0] > 0.9 && centre[1] == 0.0 && centre[2] == 0.0, "{centre:?}");
        assert_eq!(img[0], [0.0, 0.0, 0.0]);
        // The falloff is symmetric.
        let left = img[16 * 32 + 12][0];
        let right = img[16 * 32 + 19][0];
        assert!((left - right).abs() < 0.05);
        assert!(left < centre[0]);
    }

    #[test]
    fn nearer_splats_cover_farther_ones_whatever_the_input_order() {
        let far = blob(6.0, [1.0, 0.0, 0.0], 1.0);
        let near = blob(3.0, [0.0, 0.0, 1.0], 1.0);
        let a = render(&[far, near], &camera(), [0.0; 3]).unwrap();
        let b = render(&[near, far], &camera(), [0.0; 3]).unwrap();
        assert_eq!(a, b);
        let c = a[16 * 32 + 16];
        assert!(c[2] > 0.9 && c[0] < 0.05, "{c:?}");
    }

    #[test]
    fn opacity_and_background_blend() {
        let img = render(&[blob(4.0, [1.0, 1.0, 1.0], 0.5)], &camera(), [0.0, 0.0, 1.0]).unwrap();
        let c = img[16 * 32 + 16];
        assert!(c[0] > 0.3 && c[0] < 0.5, "{c:?}");
        assert!(c[2] > 0.9, "{c:?}");
        let empty = render(&[blob(-2.0, [1.0; 3], 1.0)], &camera(), [0.2, 0.3, 0.4]).unwrap();
        assert!(empty.iter().all(|p| *p == [0.2, 0.3, 0.4]));
        let mut zero = camera();
        zero.width = 0;
        assert!(render(&[], &zero, [0.0; 3]).is_none());
    }

    #[test]
    fn sh0_maps_zero_to_mid_grey() {
        assert_eq!(sh0_to_rgb([0.0; 3]), [0.5; 3]);
        assert_eq!(sh0_to_rgb([-100.0, 0.0, 1.0])[0], 0.0);
        assert!((sh0_to_rgb([0.0, 0.0, 1.0])[2] - 0.782_094_8).abs() < 1e-5);
    }
}
