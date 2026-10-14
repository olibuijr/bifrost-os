//! Depth and portrait effects on CPU pixels: a depth-of-field blur whose radius follows a depth map (so the subject
//! stays sharp and the background softens), a disparity-to-depth conversion, and a masked variable blur (the radius
//! at each pixel comes from a mask). Pixels are `0xAARRGGBB`; the depth map is one byte per pixel, 255 nearest.

/// Radius at each pixel for a focus depth: pixels at the focus depth get 0, the farthest from it get `max_radius`.
pub fn radius_map(depth: &[u8], focus: u8, max_radius: u32) -> Vec<u32> {
    let span = f32::from(focus.max(255 - focus).max(1));
    depth
        .iter()
        .map(|d| {
            let off = (i32::from(*d) - i32::from(focus)).abs() as f32;
            ((off / span) * max_radius as f32).round() as u32
        })
        .collect()
}

/// Disparity (larger is nearer) to depth bytes (255 nearest), normalised to the map's own range.
pub fn disparity_to_depth(disparity: &[f32]) -> Vec<u8> {
    let (lo, hi) = disparity.iter().fold((f32::MAX, f32::MIN), |(l, h), v| (l.min(*v), h.max(*v)));
    if disparity.is_empty() || hi <= lo {
        return vec![0; disparity.len()];
    }
    disparity.iter().map(|v| (((v - lo) / (hi - lo)) * 255.0).round() as u8).collect()
}

fn channels(p: u32) -> [u32; 4] {
    [p >> 24 & 0xff, p >> 16 & 0xff, p >> 8 & 0xff, p & 0xff]
}

/// Blurs each pixel with a box of its own radius (`radii[i]`), using a summed-area table per channel so the cost does
/// not grow with the radius. Pixels keep their own alpha.
pub fn variable_blur(px: &[u32], w: usize, h: usize, radii: &[u32]) -> Vec<u32> {
    if px.len() != w * h || radii.len() != w * h || w == 0 || h == 0 {
        return px.to_vec();
    }
    // summed-area tables with a zero row and column in front: (w+1)*(h+1) per channel
    let stride = w + 1;
    let mut sat = vec![[0u64; 3]; stride * (h + 1)];
    for y in 0..h {
        let mut row = [0u64; 3];
        for x in 0..w {
            let c = channels(px[y * w + x]);
            for k in 0..3 {
                row[k] += u64::from(c[k + 1]);
            }
            let above = sat[y * stride + x + 1];
            for k in 0..3 {
                sat[(y + 1) * stride + x + 1][k] = above[k] + row[k];
            }
        }
    }
    let mut out = Vec::with_capacity(px.len());
    for y in 0..h {
        for x in 0..w {
            let r = radii[y * w + x] as usize;
            if r == 0 {
                out.push(px[y * w + x]);
                continue;
            }
            let (x0, y0) = (x.saturating_sub(r), y.saturating_sub(r));
            let (x1, y1) = ((x + r + 1).min(w), (y + r + 1).min(h));
            let area = ((x1 - x0) * (y1 - y0)) as u64;
            let mut avg = [0u32; 3];
            for k in 0..3 {
                let sum = sat[y1 * stride + x1][k] + sat[y0 * stride + x0][k] - sat[y0 * stride + x1][k] - sat[y1 * stride + x0][k];
                avg[k] = (sum / area) as u32;
            }
            let a = px[y * w + x] >> 24;
            out.push(a << 24 | avg[0] << 16 | avg[1] << 8 | avg[2]);
        }
    }
    out
}

/// Portrait effect: blur by depth around the `focus` depth, leaving pixels near it sharp.
pub fn portrait_blur(px: &[u32], w: usize, h: usize, depth: &[u8], focus: u8, max_radius: u32) -> Vec<u32> {
    variable_blur(px, w, h, &radius_map(depth, focus, max_radius))
}

/// Masked blur: the mask byte (0-255) scales the maximum radius at each pixel.
pub fn masked_blur(px: &[u32], w: usize, h: usize, mask: &[u8], max_radius: u32) -> Vec<u32> {
    let radii: Vec<u32> = mask.iter().map(|m| (u32::from(*m) * max_radius + 127) / 255).collect();
    variable_blur(px, w, h, &radii)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radius_follows_distance_from_focus() {
        let r = radius_map(&[128, 0, 255, 64], 128, 8);
        assert_eq!(r[0], 0);
        assert_eq!(r[1], 8);
        assert!(r[2] <= 8 && r[2] >= 7);
        assert!(r[3] > 0 && r[3] < 8);
        assert_eq!(radius_map(&[10], 10, 5), vec![0]);
    }

    #[test]
    fn disparity_normalises() {
        assert_eq!(disparity_to_depth(&[1.0, 2.0, 3.0]), vec![0, 128, 255]);
        assert_eq!(disparity_to_depth(&[5.0, 5.0]), vec![0, 0]);
        assert!(disparity_to_depth(&[]).is_empty());
    }

    #[test]
    fn zero_radius_leaves_pixels_untouched() {
        let px = vec![0xff10_2030, 0xff40_5060, 0xff70_8090, 0xffa0_b0c0];
        assert_eq!(variable_blur(&px, 2, 2, &[0, 0, 0, 0]), px);
        assert_eq!(variable_blur(&px, 3, 2, &[0; 6]), px); // size mismatch: unchanged
    }

    #[test]
    fn box_blur_averages_the_neighbourhood() {
        // 3x1 row: black, white, black; radius 1 on the middle pixel averages all three
        let px = vec![0xff00_0000, 0xffff_ffff, 0xff00_0000];
        let out = variable_blur(&px, 3, 1, &[0, 1, 0]);
        assert_eq!(out[0], px[0]);
        assert_eq!(out[2], px[2]);
        assert_eq!(out[1], 0xff55_5555);
    }

    #[test]
    fn portrait_keeps_the_subject_sharp() {
        let w = 5;
        let mut px = vec![0xff00_0000u32; w];
        px[2] = 0xffff_ffff;
        let depth = [0, 0, 255, 0, 0];
        let out = portrait_blur(&px, w, 1, &depth, 255, 2);
        assert_eq!(out[2], px[2]); // subject at the focus depth
        assert!(out[1] != px[1]); // background next to it picks up light
        let masked = masked_blur(&px, w, 1, &[0, 0, 0, 0, 255], 1);
        assert_eq!(masked[..4], px[..4]);
    }
}
