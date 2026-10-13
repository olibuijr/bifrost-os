//! Image blur filters with the parameters of the reference Core Image filters: gaussian
//! (`inputRadius` is the standard deviation), box, motion (radius and angle), zoom (centre and
//! amount), disc, bokeh (radius, ring amount, ring size, softness) and masked variable blur.
//! Pixels are premultiplied RGBA packed in a `u32`, red in the low byte, like `gfx::Image`.
//! The work is done in f32 on an interleaved buffer: the separable passes are row-wide
//! multiply-add loops (`out += w * row`) that the compiler vectorises, and each pass uses one
//! padded row or a handful of source rows, never a buffer per tap.
//!
//! Edge modes: `Edge::Clamp` repeats the border pixel (`clampedToExtent`), `Edge::Transparent`
//! treats everything outside the image as clear, so the border fades the way a filter does on an
//! unclamped input. The output has the size of the input (the reference grows the extent; crop
//! or pad first to get that). Radius 0 returns the input unchanged.
//! Differences from the reference: the gaussian kernel is truncated at 3 sigma and renormalised;
//! the zoom blur's amount is the shift in pixels at the farthest corner; the masked blur reads the
//! mask's alpha and interpolates between five gaussian levels.
use crate::gfx::Image;

/// What a sample outside the image returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// The nearest border pixel (`clampedToExtent`).
    Clamp,
    /// Transparent black.
    Transparent,
}

const MAX_RADIUS: f32 = 512.0;

fn clean(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, MAX_RADIUS)
    } else {
        0.0
    }
}

fn unpack(img: &Image) -> Vec<f32> {
    img.px.iter().flat_map(|&p| [p & 255, p >> 8 & 255, p >> 16 & 255, p >> 24].map(|c| c as f32)).collect()
}

fn pack(w: usize, h: usize, f: &[f32]) -> Image {
    let mut out = Image::new(w, h);
    for (d, c) in out.px.iter_mut().zip(f.chunks_exact(4)) {
        let q = |v: f32| (v + 0.5).clamp(0.0, 255.0) as u32;
        let a = q(c[3]);
        *d = q(c[0]).min(a) | q(c[1]).min(a) << 8 | q(c[2]).min(a) << 16 | a << 24;
    }
    out
}

/// Normalised gaussian weights for `sigma`, `2 * r + 1` taps with `r = ceil(3 * sigma)`.
pub fn gaussian_weights(sigma: f32) -> Vec<f32> {
    let r = (3.0 * sigma).ceil().max(1.0) as i32;
    let mut w: Vec<f32> = (-r..=r).map(|i| (-((i * i) as f32) / (2.0 * sigma * sigma)).exp()).collect();
    let s: f32 = w.iter().sum();
    w.iter_mut().for_each(|v| *v /= s);
    w
}

/// Horizontal pass: each row is copied into a padded scratch row, then every tap adds a scaled,
/// shifted copy of it to the output row.
fn pass_h(src: &[f32], dst: &mut [f32], w: usize, wt: &[f32], edge: Edge) {
    let r = wt.len() / 2;
    let mut pad = vec![0.0f32; (w + 2 * r) * 4];
    for (srow, drow) in src.chunks_exact(w * 4).zip(dst.chunks_exact_mut(w * 4)) {
        pad[r * 4..(r + w) * 4].copy_from_slice(srow);
        let (l, rt) = match edge {
            Edge::Clamp => ([srow[0], srow[1], srow[2], srow[3]], [srow[(w - 1) * 4], srow[(w - 1) * 4 + 1], srow[(w - 1) * 4 + 2], srow[(w - 1) * 4 + 3]]),
            Edge::Transparent => ([0.0; 4], [0.0; 4]),
        };
        for i in 0..r {
            pad[i * 4..i * 4 + 4].copy_from_slice(&l);
            pad[(r + w + i) * 4..(r + w + i) * 4 + 4].copy_from_slice(&rt);
        }
        drow.fill(0.0);
        for (k, &c) in wt.iter().enumerate() {
            for (d, s) in drow.iter_mut().zip(&pad[k * 4..k * 4 + w * 4]) {
                *d += c * s;
            }
        }
    }
}

/// Vertical pass: an output row is the weighted sum of whole source rows.
fn pass_v(src: &[f32], dst: &mut [f32], w: usize, h: usize, wt: &[f32], edge: Edge) {
    let r = (wt.len() / 2) as isize;
    let rowlen = w * 4;
    for (y, drow) in dst.chunks_exact_mut(rowlen).enumerate() {
        drow.fill(0.0);
        for (k, &c) in wt.iter().enumerate() {
            let sy = y as isize + k as isize - r;
            let sy = match edge {
                Edge::Clamp => sy.clamp(0, h as isize - 1),
                Edge::Transparent if sy < 0 || sy >= h as isize => continue,
                Edge::Transparent => sy,
            } as usize;
            for (d, s) in drow.iter_mut().zip(&src[sy * rowlen..(sy + 1) * rowlen]) {
                *d += c * s;
            }
        }
    }
}

fn separable(f: &mut [f32], w: usize, h: usize, wx: &[f32], wy: &[f32], edge: Edge) {
    let mut tmp = vec![0.0f32; f.len()];
    pass_h(f, &mut tmp, w, wx, edge);
    pass_v(&tmp, f, w, h, wy, edge);
}

fn gaussian_f(f: &mut [f32], w: usize, h: usize, sigma: f32, edge: Edge) {
    if sigma > 0.0 && w > 0 && h > 0 {
        let k = gaussian_weights(sigma);
        separable(f, w, h, &k, &k, edge);
    }
}

/// `CIGaussianBlur`: separable gaussian with standard deviation `radius` pixels.
pub fn gaussian_blur(img: &Image, radius: f32, edge: Edge) -> Image {
    let s = clean(radius);
    if s <= 0.0 || img.px.is_empty() {
        return img.clone();
    }
    let mut f = unpack(img);
    gaussian_f(&mut f, img.w, img.h, s, edge);
    pack(img.w, img.h, &f)
}

/// `CIBoxBlur`: a square box `2 * round(radius) + 1` pixels wide.
pub fn box_blur(img: &Image, radius: f32, edge: Edge) -> Image {
    let r = clean(radius).round() as usize;
    if r == 0 || img.px.is_empty() {
        return img.clone();
    }
    let k = vec![1.0 / (2 * r + 1) as f32; 2 * r + 1];
    let mut f = unpack(img);
    separable(&mut f, img.w, img.h, &k, &k, edge);
    pack(img.w, img.h, &f)
}

/// Bilinear sample of the interleaved buffer at (x, y), accumulated into `acc` with weight `wt`.
#[inline]
#[allow(clippy::too_many_arguments)]
fn tap(f: &[f32], w: usize, h: usize, x: f32, y: f32, wt: f32, edge: Edge, acc: &mut [f32; 4]) {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (x0, y0) = (x0 as i64, y0 as i64);
    for (dx, dy, c) in [(0, 0, (1.0 - fx) * (1.0 - fy)), (1, 0, fx * (1.0 - fy)), (0, 1, (1.0 - fx) * fy), (1, 1, fx * fy)] {
        if c == 0.0 {
            continue;
        }
        let (mut px, mut py) = (x0 + dx, y0 + dy);
        if px < 0 || py < 0 || px >= w as i64 || py >= h as i64 {
            if edge == Edge::Transparent {
                continue;
            }
            px = px.clamp(0, w as i64 - 1);
            py = py.clamp(0, h as i64 - 1);
        }
        let i = (py as usize * w + px as usize) * 4;
        for (a, s) in acc.iter_mut().zip(&f[i..i + 4]) {
            *a += wt * c * s;
        }
    }
}

/// Applies the same list of `(dx, dy, weight)` taps to every pixel.
fn gather(f: &[f32], w: usize, h: usize, taps: &[(f32, f32, f32)], edge: Edge) -> Vec<f32> {
    let mut out = vec![0.0f32; f.len()];
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0.0f32; 4];
            for &(dx, dy, c) in taps {
                tap(f, w, h, x as f32 + dx, y as f32 + dy, c, edge, &mut acc);
            }
            out[(y * w + x) * 4..][..4].copy_from_slice(&acc);
        }
    }
    out
}

fn normalised(mut taps: Vec<(f32, f32, f32)>) -> Vec<(f32, f32, f32)> {
    let s: f32 = taps.iter().map(|t| t.2).sum();
    taps.iter_mut().for_each(|t| t.2 /= s);
    taps
}

/// `CIMotionBlur`: `2 * radius + 1` samples along a line through each pixel. `angle_deg` is
/// counter-clockwise from the +x axis as the reference measures it (screen y points down, so
/// 90 degrees blurs vertically).
pub fn motion_blur(img: &Image, radius: f32, angle_deg: f32, edge: Edge) -> Image {
    let r = clean(radius);
    if r <= 0.0 || img.px.is_empty() {
        return img.clone();
    }
    let (s, c) = angle_deg.to_radians().sin_cos();
    let n = r.round().max(1.0) as i32;
    let step = r / n as f32;
    let taps = normalised((-n..=n).map(|i| (c * i as f32 * step, -s * i as f32 * step, 1.0)).collect());
    pack(img.w, img.h, &gather(&unpack(img), img.w, img.h, &taps, edge))
}

/// `CIZoomBlur`: samples along the line from each pixel towards `center` (pixel coordinates).
/// `amount` is the shift in pixels at the farthest corner; it grows linearly with the distance.
pub fn zoom_blur(img: &Image, center: (f32, f32), amount: f32, edge: Edge) -> Image {
    let a = clean(amount);
    if a <= 0.0 || img.px.is_empty() {
        return img.clone();
    }
    let (w, h) = (img.w, img.h);
    let far = [(0.0, 0.0), (w as f32, 0.0), (0.0, h as f32), (w as f32, h as f32)]
        .iter()
        .map(|&(x, y)| ((x - center.0).powi(2) + (y - center.1).powi(2)).sqrt())
        .fold(1.0f32, f32::max);
    let k = a / far;
    let n = a.ceil().max(1.0) as i32;
    let f = unpack(img);
    let mut out = vec![0.0f32; f.len()];
    let wt = 1.0 / (n + 1) as f32;
    for y in 0..h {
        for x in 0..w {
            let (vx, vy) = (center.0 - x as f32, center.1 - y as f32);
            let mut acc = [0.0f32; 4];
            for i in 0..=n {
                let t = k * i as f32 / n as f32;
                tap(&f, w, h, x as f32 + vx * t, y as f32 + vy * t, wt, edge, &mut acc);
            }
            out[(y * w + x) * 4..][..4].copy_from_slice(&acc);
        }
    }
    pack(w, h, &out)
}

/// Taps of a disc of `radius`; the weight at distance `d / radius` comes from `profile`.
fn disc_taps(radius: f32, profile: impl Fn(f32) -> f32) -> Vec<(f32, f32, f32)> {
    let n = radius.ceil() as i32;
    let mut taps = Vec::new();
    for dy in -n..=n {
        for dx in -n..=n {
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            if d <= radius {
                taps.push((dx as f32, dy as f32, profile(d / radius)));
            }
        }
    }
    normalised(taps)
}

/// `CIDiscBlur`: the mean over a filled disc of `radius` pixels.
pub fn disc_blur(img: &Image, radius: f32, edge: Edge) -> Image {
    let r = clean(radius);
    if r < 1.0 || img.px.is_empty() {
        return img.clone();
    }
    pack(img.w, img.h, &gather(&unpack(img), img.w, img.h, &disc_taps(r, |_| 1.0), edge))
}

/// `CIBokehBlur`: a disc whose outer `ring_size` fraction is weighted `1 + ring_amount`, then a
/// gaussian of `softness` pixels (reference defaults: radius 20, ring amount 0, ring size 0.1,
/// softness 1).
pub fn bokeh_blur(img: &Image, radius: f32, ring_amount: f32, ring_size: f32, softness: f32, edge: Edge) -> Image {
    let r = clean(radius);
    if r < 1.0 || img.px.is_empty() {
        return img.clone();
    }
    let (amt, size) = (clean(ring_amount), ring_size.clamp(0.0, 1.0));
    let inner = 1.0 - size;
    let taps = disc_taps(r, |u| if u >= inner { 1.0 + amt } else { 1.0 });
    let mut out = gather(&unpack(img), img.w, img.h, &taps, edge);
    gaussian_f(&mut out, img.w, img.h, clean(softness), edge);
    pack(img.w, img.h, &out)
}

/// `CIMaskedVariableBlur`: a gaussian whose standard deviation at each pixel is `radius` times
/// the mask's alpha (0..1). The mask must have the image's size; otherwise the image is returned
/// unchanged. Five gaussian levels are interpolated.
pub fn masked_variable_blur(img: &Image, mask: &Image, radius: f32, edge: Edge) -> Image {
    let r = clean(radius);
    if r <= 0.0 || img.px.is_empty() || (mask.w, mask.h) != (img.w, img.h) {
        return img.clone();
    }
    const LEVELS: usize = 5;
    let base = unpack(img);
    let mut stack = vec![base.clone()];
    for i in 1..LEVELS {
        let mut f = base.clone();
        gaussian_f(&mut f, img.w, img.h, r * i as f32 / (LEVELS - 1) as f32, edge);
        stack.push(f);
    }
    let mut out = vec![0.0f32; base.len()];
    for (i, m) in mask.px.iter().enumerate() {
        let t = (m >> 24) as f32 / 255.0 * (LEVELS - 1) as f32;
        let l0 = (t as usize).min(LEVELS - 2);
        let fr = t - l0 as f32;
        for c in 0..4 {
            out[i * 4 + c] = stack[l0][i * 4 + c] * (1.0 - fr) + stack[l0 + 1][i * 4 + c] * fr;
        }
    }
    pack(img.w, img.h, &out)
}

/// declarative UI `.blur(radius:opaque:)`: a gaussian of `radius` points (times `scale` for pixels).
/// With `opaque` the edge is clamped and the result is composited over black so it is fully
/// opaque, as the reference does for a view that must not show what is behind it; otherwise the
/// edge is transparent and the border fades.
pub fn view_blur(img: &Image, radius: f32, scale: f32, opaque: bool) -> Image {
    let edge = if opaque { Edge::Clamp } else { Edge::Transparent };
    let mut out = gaussian_blur(img, clean(radius) * scale.max(0.0), edge);
    if opaque {
        for p in &mut out.px {
            // premultiplied colour over black is the colour itself; only alpha becomes full
            *p |= 0xff00_0000;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn impulse(n: usize) -> Image {
        let mut i = Image::new(n, n);
        i.px[(n / 2) * n + n / 2] = 0xffff_ffff;
        i
    }

    fn solid(w: usize, h: usize, p: u32) -> Image {
        let mut i = Image::new(w, h);
        i.px.fill(p);
        i
    }

    fn alpha_row(img: &Image) -> Vec<u32> {
        let y = img.h / 2;
        (0..img.w).map(|x| img.px[y * img.w + x] >> 24).collect()
    }

    fn alpha_sum(img: &Image) -> u32 {
        img.px.iter().map(|p| p >> 24).sum()
    }

    #[test]
    fn gaussian_weights_sum_to_one_and_have_the_sigma_width() {
        let w = gaussian_weights(4.0);
        assert_eq!(w.len(), 25);
        assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        let var: f32 = w.iter().enumerate().map(|(i, c)| c * ((i as i32 - 12) as f32).powi(2)).sum();
        assert!((var.sqrt() - 4.0).abs() < 0.15, "sd {}", var.sqrt());
    }

    #[test]
    fn gaussian_impulse_widens_with_radius_and_is_symmetric() {
        let src = impulse(65);
        let narrow = alpha_row(&gaussian_blur(&src, 2.0, Edge::Transparent));
        let wide = alpha_row(&gaussian_blur(&src, 5.0, Edge::Transparent));
        let span = |r: &[u32]| r.iter().filter(|&&a| a >= 1).count();
        assert!(span(&wide) > span(&narrow));
        assert!(wide[32] < narrow[32], "the peak drops as the radius grows");
        assert_eq!(narrow[32 - 3], narrow[32 + 3]);
        assert_eq!(narrow[0], 0);
    }

    #[test]
    fn gaussian_peak_matches_the_2d_formula() {
        // 255 / (2 pi sigma^2) at sigma 3 is 4.5
        let a = alpha_row(&gaussian_blur(&impulse(65), 3.0, Edge::Transparent));
        assert!((4..=5).contains(&a[32]), "peak {}", a[32]);
    }

    #[test]
    fn energy_is_preserved_away_from_the_edges() {
        for img in [
            gaussian_blur(&impulse(65), 3.0, Edge::Transparent),
            box_blur(&impulse(65), 4.0, Edge::Transparent),
            motion_blur(&impulse(65), 8.0, 30.0, Edge::Transparent),
            disc_blur(&impulse(65), 6.0, Edge::Transparent),
            bokeh_blur(&impulse(65), 8.0, 1.0, 0.2, 1.0, Edge::Transparent),
        ] {
            let s = alpha_sum(&img) as i32;
            assert!((s - 255).abs() <= 60, "sum {s}");
        }
    }

    #[test]
    fn clamp_keeps_a_solid_image_solid_and_transparent_fades_the_border() {
        let src = solid(32, 32, 0xff80_4020);
        assert_eq!(gaussian_blur(&src, 6.0, Edge::Clamp), src);
        assert_eq!(box_blur(&src, 3.0, Edge::Clamp), src);
        let t = gaussian_blur(&src, 6.0, Edge::Transparent);
        assert!(t.px[0] >> 24 < 140, "corner alpha {}", t.px[0] >> 24);
        assert!(t.px[16 * 32 + 16] >> 24 >= 250);
    }

    #[test]
    fn box_blur_spreads_an_impulse_over_two_r_plus_one() {
        let a = alpha_row(&box_blur(&impulse(33), 2.0, Edge::Transparent));
        let lit: Vec<usize> = (0..33).filter(|&x| a[x] > 0).collect();
        assert_eq!((lit[0], *lit.last().unwrap()), (14, 18));
        assert_eq!(a[16], 10); // 255 / 25
    }

    #[test]
    fn motion_blur_follows_the_angle() {
        let src = impulse(41);
        let h = motion_blur(&src, 6.0, 0.0, Edge::Transparent);
        let v = motion_blur(&src, 6.0, 90.0, Edge::Transparent);
        let at = |i: &Image, x: usize, y: usize| i.px[y * 41 + x] >> 24;
        assert!(at(&h, 24, 20) > 0 && at(&h, 20, 24) == 0);
        assert!(at(&v, 20, 24) > 0 && at(&v, 24, 20) == 0);
        assert_eq!(at(&h, 28, 20), 0, "beyond the radius");
    }

    #[test]
    fn zoom_blur_leaves_the_centre_and_streaks_radially() {
        let mut src = Image::new(41, 41);
        src.px[20 * 41 + 30] = 0xffff_ffff;
        let out = zoom_blur(&src, (20.0, 20.0), 8.0, Edge::Transparent);
        assert_eq!(zoom_blur(&solid(9, 9, 0xff10_2030), (4.0, 4.0), 5.0, Edge::Clamp).px[4 * 9 + 4], 0xff10_2030);
        let a = |x: usize, y: usize| out.px[y * 41 + x] >> 24;
        assert!(a(33, 20) > 0, "streak away from the centre");
        assert_eq!(a(27, 20), 0, "nothing on the centre side");
        assert_eq!(a(30, 30), 0, "no spread across the radius");
    }

    #[test]
    fn disc_is_flat_and_bokeh_ring_is_brighter_at_the_rim() {
        let src = impulse(65);
        let d = alpha_row(&disc_blur(&src, 8.0, Edge::Transparent));
        assert_eq!(d[32], d[32 - 5]);
        assert_eq!(d[32 - 9], 0);
        let b = alpha_row(&bokeh_blur(&src, 8.0, 3.0, 0.2, 0.0, Edge::Transparent));
        assert!(b[32 - 7] > b[32 - 3], "rim {} centre {}", b[32 - 7], b[32 - 3]);
    }

    #[test]
    fn masked_variable_blur_blurs_only_where_the_mask_is_set() {
        let src = impulse(33);
        let mut mask = Image::new(33, 33);
        mask.px[33 * 16..].fill(0xff00_0000);
        let out = masked_variable_blur(&src, &mask, 3.0, Edge::Transparent);
        assert!(out.px[16 * 33 + 16] >> 24 < 255);
        assert_eq!(masked_variable_blur(&src, &Image::new(33, 33), 3.0, Edge::Transparent), src);
        assert_eq!(masked_variable_blur(&src, &Image::new(2, 2), 3.0, Edge::Clamp), src);
    }

    #[test]
    fn view_blur_scales_by_the_display_scale_and_opaque_makes_alpha_full() {
        let src = impulse(65);
        let a = alpha_row(&view_blur(&src, 2.0, 2.0, false));
        let b = alpha_row(&gaussian_blur(&src, 4.0, Edge::Transparent));
        assert_eq!(a, b);
        let o = view_blur(&impulse(8), 3.0, 1.0, true);
        assert!(o.px.iter().all(|p| p >> 24 == 255));
        assert_eq!(view_blur(&src, 0.0, 1.0, false), src);
    }

    #[test]
    fn degenerate_inputs_are_returned_unchanged() {
        let e = Image::new(0, 0);
        assert_eq!(gaussian_blur(&e, 5.0, Edge::Clamp), e);
        let s = impulse(5);
        assert_eq!(gaussian_blur(&s, f32::NAN, Edge::Clamp), s);
        assert_eq!(motion_blur(&s, -1.0, 0.0, Edge::Clamp), s);
        assert_eq!(disc_blur(&s, 0.5, Edge::Clamp), s);
    }

    /// `cargo test --release -p bifrost-render glass_cost_blur_filters -- --ignored --nocapture`
    /// (the name keeps it inside the `glass_cost` suite of `tools/bench.py`).
    #[test]
    #[ignore]
    fn glass_cost_blur_filters() {
        let img = solid(680, 440, 0xff60_5040);
        let bench = |name: &str, f: &dyn Fn() -> Image| {
            let t = std::time::Instant::now();
            for _ in 0..10 {
                std::hint::black_box(f());
            }
            println!("BENCH {name} {:.3} ms", t.elapsed().as_secs_f64() * 100.0);
        };
        bench("blur_filter_gaussian_680x440_r18", &|| gaussian_blur(&img, 18.0, Edge::Clamp));
        bench("blur_filter_box_680x440_r18", &|| box_blur(&img, 18.0, Edge::Clamp));
        bench("blur_filter_motion_680x440_r18", &|| motion_blur(&img, 18.0, 30.0, Edge::Clamp));
        bench("blur_filter_disc_680x440_r8", &|| disc_blur(&img, 8.0, Edge::Clamp));
    }
}
