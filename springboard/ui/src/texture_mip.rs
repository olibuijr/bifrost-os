//! Mip chains for procedural textures. Level 0 is the base image; each following level is half the width and height of
//! the one before (at least one texel), down to a single texel. Levels are built with a box filter: each texel is the
//! average of the 2x2 block it covers, with the edge texel repeated for odd sizes. A sample picks the level whose
//! texel size is closest to the screen footprint, so a surface seen from far away is not aliased.
//! Pure: the chain is plain vectors and no GPU object is made.

use crate::procedural_texture::{sample, Filter, Texel, Texture, Wrap};

/// The levels of a mip chain, base first. Never empty.
pub fn mip_chain(base: &Texture) -> Vec<Texture> {
    let mut levels = vec![base.clone()];
    loop {
        let prev = levels.last().expect("chain starts with the base level");
        if prev.width == 1 && prev.height == 1 {
            break;
        }
        let next = halve(prev);
        levels.push(next);
    }
    levels
}

/// One level down: each new texel averages a 2x2 block of the previous level.
fn halve(src: &Texture) -> Texture {
    let width = (src.width / 2).max(1);
    let height = (src.height / 2).max(1);
    let last_x = src.width - 1;
    let last_y = src.height - 1;
    let mut texels = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let (x0, y0) = (2 * x, 2 * y);
            let xs = [x0, (x0 + 1).min(last_x)];
            let ys = [y0, (y0 + 1).min(last_y)];
            let mut sums = [0u32; 4];
            for yy in ys {
                for xx in xs {
                    let t = src.texel(xx, yy);
                    for c in 0..4 {
                        sums[c] += t[c] as u32;
                    }
                }
            }
            let mut out: Texel = [0; 4];
            for c in 0..4 {
                out[c] = ((sums[c] + 2) / 4) as u8;
            }
            texels.push(out);
        }
    }
    Texture { width, height, texels }
}

/// The level to read for a footprint: `footprint` is the number of base texels one screen pixel covers. Level `n`
/// covers `2^n` texels per side, so the level is the base-2 logarithm of the footprint, rounded down and clamped to
/// the chain. A footprint of one texel or less, or an unusable value, reads the base level.
pub fn level_for_footprint(levels: usize, footprint: f32) -> usize {
    if levels == 0 || !footprint.is_finite() || footprint <= 1.0 {
        return 0;
    }
    let level = footprint.log2().floor() as usize;
    level.min(levels - 1)
}

/// Samples a mip chain at `(u, v)` using the level chosen for `footprint`. `None` for an empty chain.
pub fn sample_mip(levels: &[Texture], u: f32, v: f32, footprint: f32, wrap: Wrap, filter: Filter) -> Option<Texel> {
    if levels.is_empty() {
        return None;
    }
    let index = level_for_footprint(levels.len(), footprint);
    Some(sample(&levels[index], u, v, wrap, filter))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procedural_texture::checkerboard;

    fn grey(width: usize, height: usize, texels: Vec<Texel>) -> Texture {
        Texture { width, height, texels }
    }

    #[test]
    fn chain_halves_each_side_down_to_one_texel() {
        let base = checkerboard(4, 4, 4, [0, 0, 0, 255], [255, 255, 255, 255]).expect("valid size");
        let sizes: Vec<(usize, usize)> = mip_chain(&base).iter().map(|t| (t.width, t.height)).collect();
        assert_eq!(sizes, vec![(4, 4), (2, 2), (1, 1)]);
    }

    #[test]
    fn a_strip_keeps_one_side_at_one_texel() {
        let base = grey(3, 1, vec![[0; 4], [8; 4], [16; 4]]);
        let sizes: Vec<(usize, usize)> = mip_chain(&base).iter().map(|t| (t.width, t.height)).collect();
        assert_eq!(sizes, vec![(3, 1), (1, 1)]);
    }

    #[test]
    fn each_level_averages_its_blocks() {
        let base = grey(2, 2, vec![[0, 0, 0, 255], [4, 4, 4, 255], [8, 8, 8, 255], [12, 12, 12, 255]]);
        let chain = mip_chain(&base);
        assert_eq!(chain[1].texels, vec![[6, 6, 6, 255]]);
    }

    #[test]
    fn a_checkerboard_averages_to_grey_at_the_top_level() {
        let base = checkerboard(2, 2, 2, [0, 0, 0, 255], [255, 255, 255, 255]).expect("valid size");
        let chain = mip_chain(&base);
        assert_eq!(chain[1].texels[0][0], 128);
    }

    #[test]
    fn footprint_picks_a_level_by_powers_of_two() {
        assert_eq!(level_for_footprint(5, 0.5), 0);
        assert_eq!(level_for_footprint(5, 1.0), 0);
        assert_eq!(level_for_footprint(5, 2.0), 1);
        assert_eq!(level_for_footprint(5, 7.9), 2);
        assert_eq!(level_for_footprint(5, 1e9), 4);
        assert_eq!(level_for_footprint(5, f32::NAN), 0);
        assert_eq!(level_for_footprint(0, 8.0), 0);
    }

    #[test]
    fn a_far_sample_reads_the_coarse_level() {
        let base = checkerboard(4, 4, 4, [0, 0, 0, 255], [254, 254, 254, 255]).expect("valid size");
        let chain = mip_chain(&base);
        let near = sample_mip(&chain, 0.1, 0.1, 1.0, Wrap::Clamp, Filter::Nearest).expect("non-empty");
        let far = sample_mip(&chain, 0.1, 0.1, 100.0, Wrap::Clamp, Filter::Nearest).expect("non-empty");
        assert_eq!(near, [0, 0, 0, 255]);
        assert_eq!(far, chain[2].texels[0]);
        assert!(sample_mip(&[], 0.0, 0.0, 1.0, Wrap::Clamp, Filter::Nearest).is_none());
    }
}
