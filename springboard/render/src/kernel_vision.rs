//! Computer-vision pixel kernels: the classical image-analysis steps under text recognition, subject
//! detection and photo analysis. Sobel gradients, Canny edges (non-maximum suppression and hysteresis),
//! the Harris corner response, Otsu and Sauvola thresholds, normalised cross-correlation template
//! matching, text-line bands from a row ink profile, and a centre-surround saliency map with the box
//! around the salient subject. All work on single-channel `f32` planes in row-major order (0..=1 or
//! 0..=255, the kernels are scale-free unless a parameter says otherwise); colour callers convert with
//! `plane_arith` first. Written from the textbook descriptions of each method; no trained model is involved.

/// Gradient magnitude and direction (radians, `atan2(gy, gx)`) by the 3x3 Sobel operator with the border
/// replicated. `None` when the plane size does not match.
pub fn sobel(src: &[f32], w: usize, h: usize) -> Option<(Vec<f32>, Vec<f32>)> {
    if w == 0 || h == 0 || src.len() != w * h {
        return None;
    }
    let at = |x: isize, y: isize| -> f32 {
        let xx = x.clamp(0, w as isize - 1) as usize;
        let yy = y.clamp(0, h as isize - 1) as usize;
        src[yy * w + xx]
    };
    let mut mag = vec![0.0f32; w * h];
    let mut dir = vec![0.0f32; w * h];
    for y in 0..h as isize {
        for x in 0..w as isize {
            let gx = -at(x - 1, y - 1) + at(x + 1, y - 1) - 2.0 * at(x - 1, y) + 2.0 * at(x + 1, y) - at(x - 1, y + 1)
                + at(x + 1, y + 1);
            let gy = -at(x - 1, y - 1) - 2.0 * at(x, y - 1) - at(x + 1, y - 1)
                + at(x - 1, y + 1)
                + 2.0 * at(x, y + 1)
                + at(x + 1, y + 1);
            let i = y as usize * w + x as usize;
            mag[i] = (gx * gx + gy * gy).sqrt();
            dir[i] = gy.atan2(gx);
        }
    }
    Some((mag, dir))
}

/// Canny edges: Sobel, thin to the local maximum along the gradient direction, then keep weak edges
/// (`>= low`) only when they connect to a strong one (`>= high`). `low` must not exceed `high`.
pub fn canny(src: &[f32], w: usize, h: usize, low: f32, high: f32) -> Option<Vec<bool>> {
    if low > high {
        return None;
    }
    let (mag, dir) = sobel(src, w, h)?;
    // Non-maximum suppression: compare with the two neighbours along the quantised gradient direction.
    let mut thin = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let mut a = dir[i].to_degrees();
            if a < 0.0 {
                a += 180.0;
            }
            let (dx, dy): (isize, isize) = if a < 22.5 || a >= 157.5 {
                (1, 0)
            } else if a < 67.5 {
                (1, 1)
            } else if a < 112.5 {
                (0, 1)
            } else {
                (-1, 1)
            };
            let n = |sx: isize, sy: isize| -> f32 {
                let nx = x as isize + sx;
                let ny = y as isize + sy;
                if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                    0.0
                } else {
                    mag[ny as usize * w + nx as usize]
                }
            };
            if mag[i] >= n(dx, dy) && mag[i] >= n(-dx, -dy) {
                thin[i] = mag[i];
            }
        }
    }
    // Hysteresis: flood from every strong pixel through weak ones (8-connected).
    let mut edge = vec![false; w * h];
    let mut stack: Vec<usize> = Vec::new();
    for i in 0..w * h {
        if thin[i] >= high && thin[i] > 0.0 {
            edge[i] = true;
            stack.push(i);
        }
    }
    while let Some(i) = stack.pop() {
        let x = (i % w) as isize;
        let y = (i / w) as isize;
        for dy in -1isize..=1 {
            for dx in -1isize..=1 {
                let nx = x + dx;
                let ny = y + dy;
                if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                if !edge[j] && thin[j] >= low && thin[j] > 0.0 {
                    edge[j] = true;
                    stack.push(j);
                }
            }
        }
    }
    Some(edge)
}

/// The summed-area table of a plane in `f64` with a zero first row and column, `(w + 1) * (h + 1)` long.
pub fn summed_area(src: &[f32], w: usize, h: usize) -> Vec<f64> {
    let stride = w + 1;
    let mut t = vec![0.0f64; stride * (h + 1)];
    for y in 0..h {
        let mut row = 0.0f64;
        for x in 0..w {
            row += src[y * w + x] as f64;
            t[(y + 1) * stride + x + 1] = t[y * stride + x + 1] + row;
        }
    }
    t
}

/// The sum over the half-open window `x0..x1`, `y0..y1`, clamped to the plane.
pub fn window_sum(table: &[f64], w: usize, h: usize, x0: isize, y0: isize, x1: isize, y1: isize) -> f64 {
    let stride = w + 1;
    let cx0 = x0.clamp(0, w as isize) as usize;
    let cx1 = x1.clamp(0, w as isize) as usize;
    let cy0 = y0.clamp(0, h as isize) as usize;
    let cy1 = y1.clamp(0, h as isize) as usize;
    if cx1 <= cx0 || cy1 <= cy0 {
        return 0.0;
    }
    table[cy1 * stride + cx1] - table[cy0 * stride + cx1] - table[cy1 * stride + cx0] + table[cy0 * stride + cx0]
}

/// The Harris corner response `det(M) - k * trace(M)^2` of the structure tensor summed over a
/// `(2 * radius + 1)` window. Large positive at corners, negative along edges, near zero when flat.
pub fn harris_response(src: &[f32], w: usize, h: usize, k: f32, radius: usize) -> Option<Vec<f32>> {
    if w == 0 || h == 0 || src.len() != w * h {
        return None;
    }
    let at = |x: isize, y: isize| -> f32 {
        src[y.clamp(0, h as isize - 1) as usize * w + x.clamp(0, w as isize - 1) as usize]
    };
    let mut ixx = vec![0.0f32; w * h];
    let mut iyy = vec![0.0f32; w * h];
    let mut ixy = vec![0.0f32; w * h];
    for y in 0..h as isize {
        for x in 0..w as isize {
            let gx = (at(x + 1, y) - at(x - 1, y)) * 0.5;
            let gy = (at(x, y + 1) - at(x, y - 1)) * 0.5;
            let i = y as usize * w + x as usize;
            ixx[i] = gx * gx;
            iyy[i] = gy * gy;
            ixy[i] = gx * gy;
        }
    }
    let txx = summed_area(&ixx, w, h);
    let tyy = summed_area(&iyy, w, h);
    let txy = summed_area(&ixy, w, h);
    let r = radius as isize;
    let mut out = vec![0.0f32; w * h];
    for y in 0..h as isize {
        for x in 0..w as isize {
            let a = window_sum(&txx, w, h, x - r, y - r, x + r + 1, y + r + 1);
            let b = window_sum(&tyy, w, h, x - r, y - r, x + r + 1, y + r + 1);
            let c = window_sum(&txy, w, h, x - r, y - r, x + r + 1, y + r + 1);
            let det = a * b - c * c;
            let tr = a + b;
            out[y as usize * w + x as usize] = (det - k as f64 * tr * tr) as f32;
        }
    }
    Some(out)
}

/// Otsu's threshold of 8-bit values: the level that maximises the between-class variance. A value `>`
/// the result is foreground. Returns 0 for an empty slice.
pub fn otsu_threshold(values: &[u8]) -> u8 {
    let mut hist = [0u64; 256];
    for v in values {
        hist[*v as usize] += 1;
    }
    let total: u64 = values.len() as u64;
    if total == 0 {
        return 0;
    }
    let sum_all: f64 = (0..256).map(|i| i as f64 * hist[i] as f64).sum();
    let mut w_b = 0u64;
    let mut sum_b = 0.0f64;
    let mut best = 0u8;
    let mut best_var = -1.0f64;
    for t in 0..256usize {
        w_b += hist[t];
        if w_b == 0 {
            continue;
        }
        let w_f = total - w_b;
        if w_f == 0 {
            break;
        }
        sum_b += t as f64 * hist[t] as f64;
        let m_b = sum_b / w_b as f64;
        let m_f = (sum_all - sum_b) / w_f as f64;
        let between = w_b as f64 * w_f as f64 * (m_b - m_f) * (m_b - m_f);
        if between > best_var {
            best_var = between;
            best = t as u8;
        }
    }
    best
}

/// Sauvola local binarisation for text: a pixel is ink (`true`) when it is darker than
/// `mean * (1 + k * (std / r - 1))` over its window. `r` is the dynamic range of the data (128 for 0..=255,
/// 0.5 for 0..=1), `k` about 0.2 to 0.5.
pub fn sauvola(src: &[f32], w: usize, h: usize, radius: usize, k: f32, r: f32) -> Option<Vec<bool>> {
    if w == 0 || h == 0 || src.len() != w * h || r <= 0.0 {
        return None;
    }
    let squares: Vec<f32> = src.iter().map(|v| v * v).collect();
    let t1 = summed_area(src, w, h);
    let t2 = summed_area(&squares, w, h);
    let rad = radius as isize;
    let mut out = vec![false; w * h];
    for y in 0..h as isize {
        for x in 0..w as isize {
            let x0 = (x - rad).max(0);
            let x1 = (x + rad + 1).min(w as isize);
            let y0 = (y - rad).max(0);
            let y1 = (y + rad + 1).min(h as isize);
            let n = ((x1 - x0) * (y1 - y0)) as f64;
            let mean = window_sum(&t1, w, h, x0, y0, x1, y1) / n;
            let var = (window_sum(&t2, w, h, x0, y0, x1, y1) / n - mean * mean).max(0.0);
            let std = var.sqrt();
            let thr = mean * (1.0 + k as f64 * (std / r as f64 - 1.0));
            out[y as usize * w + x as usize] = (src[y as usize * w + x as usize] as f64) < thr;
        }
    }
    Some(out)
}

/// Normalised cross-correlation template matching. Returns the top-left `(x, y)` of the best window and
/// its score in -1..=1 (1 is a perfect match up to brightness and contrast). Flat windows score 0.
pub fn ncc_match(img: &[f32], w: usize, h: usize, tpl: &[f32], tw: usize, th: usize) -> Option<(usize, usize, f32)> {
    if w == 0 || h == 0 || tw == 0 || th == 0 || img.len() != w * h || tpl.len() != tw * th || tw > w || th > h {
        return None;
    }
    let n = (tw * th) as f64;
    let t_mean = tpl.iter().map(|v| *v as f64).sum::<f64>() / n;
    let t_var: f64 = tpl.iter().map(|v| (*v as f64 - t_mean).powi(2)).sum();
    let mut best = (0usize, 0usize, f32::NEG_INFINITY);
    for y in 0..=(h - th) {
        for x in 0..=(w - tw) {
            let mut sum = 0.0f64;
            for ty in 0..th {
                for tx in 0..tw {
                    sum += img[(y + ty) * w + x + tx] as f64;
                }
            }
            let mean = sum / n;
            let mut cov = 0.0f64;
            let mut var = 0.0f64;
            for ty in 0..th {
                for tx in 0..tw {
                    let a = img[(y + ty) * w + x + tx] as f64 - mean;
                    let b = tpl[ty * tw + tx] as f64 - t_mean;
                    cov += a * b;
                    var += a * a;
                }
            }
            let denom = (var * t_var).sqrt();
            let score = if denom > 1e-12 { (cov / denom) as f32 } else { 0.0 };
            if score > best.2 {
                best = (x, y, score);
            }
        }
    }
    Some(best)
}

/// Text-line bands from a binarised page: rows whose ink count is at least `min_ink` are grouped into
/// runs; each run is returned as `(first_row, last_row)` inclusive.
pub fn text_line_bands(ink: &[bool], w: usize, h: usize, min_ink: usize) -> Vec<(usize, usize)> {
    let mut bands = Vec::new();
    if w == 0 || ink.len() != w * h {
        return bands;
    }
    let mut start: Option<usize> = None;
    for y in 0..h {
        let count = ink[y * w..(y + 1) * w].iter().filter(|b| **b).count();
        if count >= min_ink.max(1) {
            if start.is_none() {
                start = Some(y);
            }
        } else if let Some(s) = start.take() {
            bands.push((s, y - 1));
        }
    }
    if let Some(s) = start {
        bands.push((s, h - 1));
    }
    bands
}

/// A centre-surround saliency map: how much each pixel differs from the mean of its wider surround,
/// measured against a small centre window (`center` and `surround` are radii, `surround > center`).
pub fn saliency(src: &[f32], w: usize, h: usize, center: usize, surround: usize) -> Option<Vec<f32>> {
    if w == 0 || h == 0 || src.len() != w * h || surround <= center {
        return None;
    }
    let t = summed_area(src, w, h);
    let mean = |x: isize, y: isize, r: isize| -> f64 {
        let x0 = (x - r).max(0);
        let x1 = (x + r + 1).min(w as isize);
        let y0 = (y - r).max(0);
        let y1 = (y + r + 1).min(h as isize);
        window_sum(&t, w, h, x0, y0, x1, y1) / (((x1 - x0) * (y1 - y0)) as f64)
    };
    let mut out = vec![0.0f32; w * h];
    for y in 0..h as isize {
        for x in 0..w as isize {
            out[y as usize * w + x as usize] = (mean(x, y, center as isize) - mean(x, y, surround as isize)).abs() as f32;
        }
    }
    Some(out)
}

/// The bounding box `(x0, y0, x1, y1)` (inclusive) of every pixel at or above `threshold`, or `None`.
pub fn salient_box(map: &[f32], w: usize, h: usize, threshold: f32) -> Option<(usize, usize, usize, usize)> {
    if w == 0 || map.len() != w * h {
        return None;
    }
    let mut bbox: Option<(usize, usize, usize, usize)> = None;
    for y in 0..h {
        for x in 0..w {
            if map[y * w + x] >= threshold {
                bbox = Some(match bbox {
                    None => (x, y, x, y),
                    Some((a, b, c, d)) => (a.min(x), b.min(y), c.max(x), d.max(y)),
                });
            }
        }
    }
    bbox
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 16x16 plane, dark with a bright square at 4..12.
    fn square() -> Vec<f32> {
        let mut p = vec![0.0f32; 256];
        for y in 4..12 {
            for x in 4..12 {
                p[y * 16 + x] = 1.0;
            }
        }
        p
    }

    #[test]
    fn sobel_is_zero_on_flat_and_horizontal_on_a_vertical_edge() {
        let (m, _) = sobel(&[0.5; 16], 4, 4).unwrap();
        assert!(m.iter().all(|v| *v == 0.0));
        let step = [0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0];
        let (m, d) = sobel(&step, 4, 4).unwrap();
        assert!(m[1 * 4 + 1] > 3.9);
        assert!(d[1 * 4 + 1].abs() < 1e-6, "gradient points along +x");
        assert!(sobel(&step, 5, 4).is_none());
    }

    #[test]
    fn canny_finds_the_outline_of_a_square_and_not_its_inside() {
        let e = canny(&square(), 16, 16, 0.5, 2.0).unwrap();
        assert!(e.iter().any(|b| *b));
        assert!(!e[0], "flat corner is not an edge");
        assert!(!e[8 * 16 + 8], "flat interior is not an edge");
        // Some edge pixel sits at the square boundary on row 8.
        assert!(e[8 * 16 + 3] || e[8 * 16 + 4]);
        assert!(canny(&square(), 16, 16, 3.0, 2.0).is_none());
    }

    #[test]
    fn harris_peaks_at_corners_not_edges() {
        let r = harris_response(&square(), 16, 16, 0.04, 1).unwrap();
        let corner = r[4 * 16 + 4];
        let edge = r[8 * 16 + 4];
        let flat = r[0];
        assert!(corner > 0.0);
        assert!(corner > edge);
        assert!(edge <= 1e-6);
        assert!(flat.abs() < 1e-9);
    }

    #[test]
    fn otsu_splits_two_clusters() {
        let mut v = vec![20u8; 50];
        v.extend(vec![200u8; 50]);
        let t = otsu_threshold(&v);
        assert!(t >= 20 && t < 200);
        assert_eq!(otsu_threshold(&[]), 0);
    }

    #[test]
    fn sauvola_marks_dark_text_on_an_uneven_page() {
        // Light page with a gradient, one dark stroke.
        let (w, h) = (16usize, 8usize);
        let mut p = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                p[y * w + x] = 0.6 + 0.02 * x as f32;
            }
        }
        for x in 3..12 {
            p[4 * w + x] = 0.1;
        }
        let ink = sauvola(&p, w, h, 4, 0.3, 0.5).unwrap();
        assert!(ink[4 * w + 7]);
        assert!(!ink[0]);
        assert!(!ink[w * 7 + 15]);
    }

    #[test]
    fn ncc_finds_the_template_and_ignores_brightness() {
        let mut img = vec![0.1f32; 10 * 10];
        let tpl = [0.0, 1.0, 1.0, 0.0, 0.5, 1.0, 1.0, 0.0, 0.2];
        for ty in 0..3 {
            for tx in 0..3 {
                img[(5 + ty) * 10 + 2 + tx] = tpl[ty * 3 + tx] * 0.5 + 0.2;
            }
        }
        let (x, y, s) = ncc_match(&img, 10, 10, &tpl, 3, 3).unwrap();
        assert_eq!((x, y), (2, 5));
        assert!(s > 0.999);
        assert!(ncc_match(&img, 10, 10, &tpl, 11, 3).is_none());
    }

    #[test]
    fn text_bands_group_consecutive_inked_rows() {
        let (w, h) = (6usize, 9usize);
        let mut ink = vec![false; w * h];
        for y in [1usize, 2, 6] {
            for x in 0..4 {
                ink[y * w + x] = true;
            }
        }
        assert_eq!(text_line_bands(&ink, w, h, 2), vec![(1, 2), (6, 6)]);
        assert!(text_line_bands(&ink, w, 3, 2).is_empty());
    }

    #[test]
    fn saliency_marks_the_odd_one_out() {
        let mut p = vec![0.2f32; 20 * 20];
        for y in 9..12 {
            for x in 9..12 {
                p[y * 20 + x] = 1.0;
            }
        }
        let s = saliency(&p, 20, 20, 1, 6).unwrap();
        let (x0, y0, x1, y1) = salient_box(&s, 20, 20, 0.3).unwrap();
        assert!(x0 >= 8 && x1 <= 12 && y0 >= 8 && y1 <= 12);
        assert!(saliency(&p, 20, 20, 6, 6).is_none());
        assert!(salient_box(&s, 20, 20, 10.0).is_none());
    }

    #[test]
    fn window_sum_clamps_to_the_plane() {
        let t = summed_area(&[1.0; 9], 3, 3);
        assert_eq!(window_sum(&t, 3, 3, -5, -5, 10, 10), 9.0);
        assert_eq!(window_sum(&t, 3, 3, 1, 1, 2, 2), 1.0);
        assert_eq!(window_sum(&t, 3, 3, 2, 2, 2, 3), 0.0);
    }
}
