//! Edge-preserving guided filter on single-channel `f32` planes (He, Sun and Tang's guided filter). The output
//! follows the input `src` where the guidance image `guide` has an edge and smooths where it is flat. For every
//! window of radius `r` the filter fits `q = a * guide + b` with `a = cov(guide, src) / (var(guide) + epsilon)`
//! and `b = mean(src) - a * mean(guide)`, then averages `a` and `b` over all windows that contain a pixel. Window
//! means use a summed-area table and clamp at the image border, dividing by the number of pixels inside. The
//! regression and reconstruction are done in one call, on the CPU. Plain std code, no GPU, no app-facing entry
//! point yet.

/// Mean of `src` over a window of radius `r` around each pixel, clamped at the border. Row-major.
fn box_mean(src: &[f64], w: usize, h: usize, r: usize) -> Vec<f64> {
    let stride = w + 1;
    // `sat[y * stride + x]` holds the sum of all pixels above and left of the corner (x, y).
    let mut sat = vec![0.0f64; stride * (h + 1)];
    for y in 0..h {
        let mut row = 0.0f64;
        for x in 0..w {
            row += src[y * w + x];
            sat[(y + 1) * stride + x + 1] = sat[y * stride + x + 1] + row;
        }
    }
    let mut out = vec![0.0f64; w * h];
    for y in 0..h {
        let y0 = y.saturating_sub(r);
        let y1 = (y + r + 1).min(h);
        for x in 0..w {
            let x0 = x.saturating_sub(r);
            let x1 = (x + r + 1).min(w);
            let sum = sat[y1 * stride + x1] - sat[y0 * stride + x1] - sat[y1 * stride + x0] + sat[y0 * stride + x0];
            let count = ((y1 - y0) * (x1 - x0)) as f64;
            out[y * w + x] = sum / count;
        }
    }
    out
}

/// Guided filter of `src` by `guide`, both row-major `width * height` planes. `diameter` is the odd window
/// width (`2 r + 1`) and `epsilon` the regularisation that stops flat guidance from amplifying noise. Returns
/// `None` when the plane sizes differ or the diameter is zero or even.
pub fn guided_filter(guide: &[f32], src: &[f32], width: usize, height: usize, diameter: usize, epsilon: f32) -> Option<Vec<f32>> {
    let n = width.checked_mul(height)?;
    if guide.len() != n || src.len() != n || diameter == 0 || diameter.is_multiple_of(2) {
        return None;
    }
    let r = diameter / 2;
    let i: Vec<f64> = guide.iter().map(|&v| v as f64).collect();
    let p: Vec<f64> = src.iter().map(|&v| v as f64).collect();
    let i_sq: Vec<f64> = i.iter().map(|v| v * v).collect();
    let i_p: Vec<f64> = i.iter().zip(&p).map(|(a, b)| a * b).collect();

    let mean_i = box_mean(&i, width, height, r);
    let mean_p = box_mean(&p, width, height, r);
    let corr_i = box_mean(&i_sq, width, height, r);
    let corr_ip = box_mean(&i_p, width, height, r);

    let eps = epsilon as f64;
    let mut a = vec![0.0f64; n];
    let mut b = vec![0.0f64; n];
    for k in 0..n {
        let var_i = corr_i[k] - mean_i[k] * mean_i[k];
        let cov_ip = corr_ip[k] - mean_i[k] * mean_p[k];
        a[k] = cov_ip / (var_i + eps);
        b[k] = mean_p[k] - a[k] * mean_i[k];
    }
    let mean_a = box_mean(&a, width, height, r);
    let mean_b = box_mean(&b, width, height, r);
    Some((0..n).map(|k| (mean_a[k] * i[k] + mean_b[k]) as f32).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn a_zero_radius_window_returns_the_input_unchanged() {
        // With radius 0 the window is the pixel itself, so var and cov are 0 and q = src.
        let guide = [0.3f32, 0.9, 0.1];
        let src = [5.0f32, -2.0, 7.5];
        let q = guided_filter(&guide, &src, 3, 1, 1, 0.01).unwrap();
        assert!(q.iter().zip(src.iter()).all(|(a, b)| close(*a, *b)));
    }

    #[test]
    fn a_flat_guide_averages_the_input_over_each_window() {
        // Guide is flat, so a = 0 and q = box mean of src. Src = [0, 3, 6], radius 1:
        // b = [1.5, 3, 4.5] (window means), and their clamped window means are [2.25, 3, 3.75].
        let q = guided_filter(&[1.0, 1.0, 1.0], &[0.0, 3.0, 6.0], 3, 1, 3, 0.1).unwrap();
        assert!(close(q[0], 2.25));
        assert!(close(q[1], 3.0));
        assert!(close(q[2], 3.75));
    }

    #[test]
    fn a_constant_input_stays_constant() {
        let guide = [0.0f32, 1.0, 2.0, 3.0];
        let q = guided_filter(&guide, &[4.0; 4], 2, 2, 3, 0.5).unwrap();
        assert!(q.iter().all(|v| close(*v, 4.0)));
    }

    #[test]
    fn an_even_or_zero_diameter_is_refused() {
        let g = [0.0f32; 4];
        assert!(guided_filter(&g, &g, 2, 2, 2, 0.1).is_none());
        assert!(guided_filter(&g, &g, 2, 2, 0, 0.1).is_none());
    }

    #[test]
    fn mismatched_plane_sizes_are_refused() {
        assert!(guided_filter(&[0.0; 3], &[0.0; 4], 2, 2, 3, 0.1).is_none());
        assert!(guided_filter(&[0.0; 4], &[0.0; 4], 3, 2, 3, 0.1).is_none());
    }

    #[test]
    fn box_mean_clamps_at_the_border() {
        // Row [2, 4, 6] with radius 1: window means are (2+4)/2, (2+4+6)/3, (4+6)/2 = 3, 4, 5.
        let m = box_mean(&[2.0, 4.0, 6.0], 3, 1, 1);
        assert!((m[0] - 3.0).abs() < 1e-12);
        assert!((m[1] - 4.0).abs() < 1e-12);
        assert!((m[2] - 5.0).abs() < 1e-12);
    }
}
