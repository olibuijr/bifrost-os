//! Texture import and processing kernels for 3D content: what a scene importer does to an image before a
//! renderer samples it. The sRGB transfer, a box-filtered mip chain computed in linear light, normal-map
//! mips (average the vectors, renormalise, and report the length that was lost), the Toksvig-style
//! roughness increase for that lost length so distant bumps still scatter light, occlusion/roughness/
//! metallic channel packing, channel swizzles, and the BC1 (DXT1) block compressor and decompressor.
//! Image decoding is in `imgdec`, sampling in `texture`, shading in `pbr`. Pixels are packed RGBA with
//! red in the low byte. Written from the public descriptions of sRGB (IEC 61966-2-1), box mip filtering,
//! the von Mises-Fisher normal-variance derivation and the S3TC/BC1 block layout.

/// sRGB-encoded byte to linear light in 0..=1.
pub fn srgb_to_linear(v: u8) -> f32 {
    let c = v as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear light to an sRGB-encoded byte (clamped, rounded).
pub fn linear_to_srgb(l: f32) -> u8 {
    let l = if l.is_nan() { 0.0 } else { l.clamp(0.0, 1.0) };
    let c = if l <= 0.003_130_8 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
    (c * 255.0).round().clamp(0.0, 255.0) as u8
}

/// The number of levels of a full mip chain down to 1x1.
pub fn mip_count(w: usize, h: usize) -> usize {
    let mut n = 1;
    let mut m = w.max(h);
    while m > 1 {
        m /= 2;
        n += 1;
    }
    n
}

fn channels(p: u32) -> [u32; 4] {
    [p & 0xff, (p >> 8) & 0xff, (p >> 16) & 0xff, p >> 24]
}

/// One box-filtered level down. Colour channels are averaged in linear light when `srgb` is set (alpha is
/// always linear); the sample window is 2x2 clamped at odd edges.
pub fn mip_level(px: &[u32], w: usize, h: usize, srgb: bool) -> Option<(Vec<u32>, usize, usize)> {
    if w == 0 || h == 0 || px.len() != w * h {
        return None;
    }
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = Vec::with_capacity(nw * nh);
    for y in 0..nh {
        for x in 0..nw {
            let mut acc = [0.0f32; 4];
            for (dx, dy) in [(0usize, 0usize), (1, 0), (0, 1), (1, 1)] {
                let sx = (2 * x + dx).min(w - 1);
                let sy = (2 * y + dy).min(h - 1);
                let c = channels(px[sy * w + sx]);
                for i in 0..3 {
                    acc[i] += if srgb { srgb_to_linear(c[i] as u8) } else { c[i] as f32 / 255.0 };
                }
                acc[3] += c[3] as f32 / 255.0;
            }
            let mut p = 0u32;
            for i in 0..3 {
                let avg = acc[i] / 4.0;
                let byte = if srgb { linear_to_srgb(avg) } else { (avg * 255.0).round().clamp(0.0, 255.0) as u8 };
                p |= (byte as u32) << (8 * i);
            }
            p |= (((acc[3] / 4.0) * 255.0).round().clamp(0.0, 255.0) as u32) << 24;
            out.push(p);
        }
    }
    Some((out, nw, nh))
}

/// The whole chain, level 0 first (a copy of the input), as `(pixels, width, height)`.
pub fn mip_chain(px: &[u32], w: usize, h: usize, srgb: bool) -> Option<Vec<(Vec<u32>, usize, usize)>> {
    if w == 0 || h == 0 || px.len() != w * h {
        return None;
    }
    let mut levels = vec![(px.to_vec(), w, h)];
    while levels.last().map(|l| l.1 > 1 || l.2 > 1).unwrap_or(false) {
        let (p, cw, ch) = levels.last().cloned()?;
        levels.push(mip_level(&p, cw, ch, srgb)?);
    }
    Some(levels)
}

/// Decodes a tangent-space normal from an RGB byte triple (`v * 2 - 1` per channel), unnormalised.
pub fn decode_normal(p: u32) -> [f32; 3] {
    let c = channels(p);
    [c[0] as f32 / 255.0 * 2.0 - 1.0, c[1] as f32 / 255.0 * 2.0 - 1.0, c[2] as f32 / 255.0 * 2.0 - 1.0]
}

/// Encodes a unit normal as opaque RGB bytes.
pub fn encode_normal(n: [f32; 3]) -> u32 {
    let q = |v: f32| -> u32 { ((v * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u32 };
    q(n[0]) | q(n[1]) << 8 | q(n[2]) << 16 | 0xff00_0000
}

/// One normal-map level down: the four decoded normals are averaged, the length of the average is kept as
/// the "lost" coherence, and the vector is renormalised. Returns the level, its size and one average
/// length (0..=1, 1 is perfectly flat) per output pixel.
pub fn normal_map_mip(px: &[u32], w: usize, h: usize) -> Option<(Vec<u32>, usize, usize, Vec<f32>)> {
    if w == 0 || h == 0 || px.len() != w * h {
        return None;
    }
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = Vec::with_capacity(nw * nh);
    let mut lengths = Vec::with_capacity(nw * nh);
    for y in 0..nh {
        for x in 0..nw {
            let mut sum = [0.0f32; 3];
            for (dx, dy) in [(0usize, 0usize), (1, 0), (0, 1), (1, 1)] {
                let sx = (2 * x + dx).min(w - 1);
                let sy = (2 * y + dy).min(h - 1);
                let n = decode_normal(px[sy * w + sx]);
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
                for i in 0..3 {
                    sum[i] += n[i] / len * 0.25;
                }
            }
            let l = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
            lengths.push(l.min(1.0));
            if l > 1e-6 {
                out.push(encode_normal([sum[0] / l, sum[1] / l, sum[2] / l]));
            } else {
                out.push(encode_normal([0.0, 0.0, 1.0]));
            }
        }
    }
    Some((out, nw, nh, lengths))
}

/// Raises an artist roughness (0..=1, GGX alpha = roughness squared) to account for normal variance lost
/// by mip averaging. The average-normal length `len` gives the concentration `kappa = (3 len - len^3) /
/// (1 - len^2)` of a von Mises-Fisher lobe, whose variance `1 / kappa` is added to alpha squared (capped
/// at 1). A length of 1 changes nothing.
pub fn filtered_roughness(roughness: f32, len: f32) -> f32 {
    let r = roughness.clamp(0.0, 1.0);
    let l = len.clamp(0.0, 1.0);
    if l >= 0.9999 {
        return r;
    }
    let kappa = (3.0 * l - l * l * l) / (1.0 - l * l);
    let extra = if kappa > 1e-6 { (1.0 / kappa).min(1.0) } else { 1.0 };
    let alpha2 = (r * r) * (r * r) + extra;
    alpha2.min(1.0).sqrt().sqrt()
}

/// Packs three single-channel planes into one RGBA texture the common way: occlusion in red, roughness in
/// green, metallic in blue, alpha opaque. Values are 0..=1. `None` on unequal lengths.
pub fn pack_orm(occlusion: &[f32], roughness: &[f32], metallic: &[f32]) -> Option<Vec<u32>> {
    if occlusion.len() != roughness.len() || occlusion.len() != metallic.len() {
        return None;
    }
    let q = |v: f32| -> u32 { (v.clamp(0.0, 1.0) * 255.0).round() as u32 };
    Some((0..occlusion.len()).map(|i| q(occlusion[i]) | q(roughness[i]) << 8 | q(metallic[i]) << 16 | 0xff00_0000).collect())
}

/// Reorders channels in place: output channel `i` takes input channel `order[i]` (0 red, 1 green, 2 blue,
/// 3 alpha). `false` when an index is above 3.
pub fn swizzle(px: &mut [u32], order: [usize; 4]) -> bool {
    if order.iter().any(|i| *i > 3) {
        return false;
    }
    for p in px.iter_mut() {
        let c = channels(*p);
        *p = c[order[0]] | c[order[1]] << 8 | c[order[2]] << 16 | c[order[3]] << 24;
    }
    true
}

fn to565(c: [u32; 3]) -> u16 {
    (((c[0] >> 3) << 11) | ((c[1] >> 2) << 5) | (c[2] >> 3)) as u16
}

fn from565(v: u16) -> [i32; 3] {
    let r = ((v >> 11) & 31) as i32;
    let g = ((v >> 5) & 63) as i32;
    let b = (v & 31) as i32;
    [(r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2)]
}

fn bc1_palette(c0: u16, c1: u16) -> [[i32; 3]; 4] {
    let p0 = from565(c0);
    let p1 = from565(c1);
    if c0 > c1 {
        let mix = |a: i32, b: i32, wa: i32, wb: i32| (a * wa + b * wb) / 3;
        [
            p0,
            p1,
            [mix(p0[0], p1[0], 2, 1), mix(p0[1], p1[1], 2, 1), mix(p0[2], p1[2], 2, 1)],
            [mix(p0[0], p1[0], 1, 2), mix(p0[1], p1[1], 1, 2), mix(p0[2], p1[2], 1, 2)],
        ]
    } else {
        [p0, p1, [(p0[0] + p1[0]) / 2, (p0[1] + p1[1]) / 2, (p0[2] + p1[2]) / 2], [0, 0, 0]]
    }
}

/// Compresses one 4x4 block (row-major RGBA pixels, alpha ignored) to the 8-byte BC1 layout: two RGB565
/// endpoints (little endian, the larger first for four-colour mode) and sixteen 2-bit indices. Endpoints
/// are the per-channel bounding box of the block, which is exact for blocks on a line between two colours
/// and a fair fit otherwise.
pub fn bc1_encode_block(block: &[u32; 16]) -> [u8; 8] {
    let mut lo = [255u32; 3];
    let mut hi = [0u32; 3];
    for p in block {
        let c = channels(*p);
        for i in 0..3 {
            lo[i] = lo[i].min(c[i]);
            hi[i] = hi[i].max(c[i]);
        }
    }
    let mut c0 = to565(hi);
    let mut c1 = to565(lo);
    if c0 < c1 {
        std::mem::swap(&mut c0, &mut c1);
    }
    let mut bytes = [0u8; 8];
    bytes[0] = (c0 & 0xff) as u8;
    bytes[1] = (c0 >> 8) as u8;
    bytes[2] = (c1 & 0xff) as u8;
    bytes[3] = (c1 >> 8) as u8;
    if c0 == c1 {
        return bytes; // every index 0
    }
    let palette = bc1_palette(c0, c1);
    let mut indices = 0u32;
    for (i, p) in block.iter().enumerate() {
        let c = channels(*p);
        let mut best = 0usize;
        let mut best_d = i64::MAX;
        for (k, pc) in palette.iter().enumerate() {
            let d: i64 = (0..3).map(|j| ((c[j] as i32 - pc[j]) as i64).pow(2)).sum();
            if d < best_d {
                best_d = d;
                best = k;
            }
        }
        indices |= (best as u32) << (2 * i);
    }
    bytes[4..8].copy_from_slice(&indices.to_le_bytes());
    bytes
}

/// Decompresses a BC1 block to sixteen opaque RGBA pixels. In three-colour mode (`c0 <= c1`) index 3 is
/// transparent black, as the format defines.
pub fn bc1_decode_block(bytes: &[u8; 8]) -> [u32; 16] {
    let c0 = bytes[0] as u16 | (bytes[1] as u16) << 8;
    let c1 = bytes[2] as u16 | (bytes[3] as u16) << 8;
    let palette = bc1_palette(c0, c1);
    let indices = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let mut out = [0u32; 16];
    for (i, o) in out.iter_mut().enumerate() {
        let k = ((indices >> (2 * i)) & 3) as usize;
        if c0 <= c1 && k == 3 {
            *o = 0;
        } else {
            let p = palette[k];
            *o = p[0] as u32 | (p[1] as u32) << 8 | (p[2] as u32) << 16 | 0xff00_0000;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_round_trips_every_byte() {
        for v in 0..=255u8 {
            assert_eq!(linear_to_srgb(srgb_to_linear(v)), v);
        }
        assert!((srgb_to_linear(255) - 1.0).abs() < 1e-6);
        assert_eq!(linear_to_srgb(f32::NAN), 0);
    }

    #[test]
    fn mip_count_counts_levels_to_one_pixel() {
        assert_eq!(mip_count(1, 1), 1);
        assert_eq!(mip_count(4, 4), 3);
        assert_eq!(mip_count(5, 3), 3);
        assert_eq!(mip_count(256, 64), 9);
    }

    #[test]
    fn mip_average_is_taken_in_linear_light_for_srgb() {
        let black = 0xff00_0000u32;
        let white = 0xffff_ffffu32;
        let src = [black, white, black, white];
        let (lin, _, _) = mip_level(&src, 2, 2, false).unwrap();
        assert_eq!(lin[0] & 0xff, 128);
        let (srgb, w, h) = mip_level(&src, 2, 2, true).unwrap();
        assert_eq!((w, h), (1, 1));
        let v = (srgb[0] & 0xff) as i32;
        assert!((v - 188).abs() <= 1, "half linear light is about 188 in sRGB, got {v}");
        assert_eq!(srgb[0] >> 24, 0xff);
    }

    #[test]
    fn mip_chain_lengths_follow_the_size() {
        let px = vec![0xff40_80c0u32; 20];
        let chain = mip_chain(&px, 5, 4, true).unwrap();
        assert_eq!(chain.len(), mip_count(5, 4));
        assert_eq!((chain[1].1, chain[1].2), (2, 2));
        let last = chain.last().unwrap();
        assert_eq!((last.1, last.2), (1, 1));
        assert!(mip_chain(&px, 5, 5, true).is_none());
    }

    #[test]
    fn flat_normals_keep_full_length_and_opposed_tilts_lose_it() {
        let flat = [encode_normal([0.0, 0.0, 1.0]); 4];
        let (out, _, _, len) = normal_map_mip(&flat, 2, 2).unwrap();
        assert!((len[0] - 1.0).abs() < 0.01);
        let n = decode_normal(out[0]);
        assert!(n[2] > 0.99);
        let a = encode_normal([0.5, 0.0, 0.866_025_4]);
        let b = encode_normal([-0.5, 0.0, 0.866_025_4]);
        let tilted = [a, b, a, b];
        let (out, _, _, len) = normal_map_mip(&tilted, 2, 2).unwrap();
        assert!((len[0] - 0.866).abs() < 0.02, "got {}", len[0]);
        let n = decode_normal(out[0]);
        assert!(n[0].abs() < 0.02 && n[2] > 0.99, "renormalised back to straight up");
    }

    #[test]
    fn lost_coherence_raises_roughness() {
        assert_eq!(filtered_roughness(0.3, 1.0), 0.3);
        let rough = filtered_roughness(0.3, 0.9);
        assert!(rough > 0.3 && rough <= 1.0);
        assert!(filtered_roughness(0.3, 0.7) > rough);
        assert_eq!(filtered_roughness(0.3, 0.0), 1.0);
    }

    #[test]
    fn orm_pack_and_swizzle() {
        let px = pack_orm(&[1.0, 0.0], &[0.5, 0.0], &[0.0, 1.0]).unwrap();
        assert_eq!(px[0], 0xff00_80ff);
        assert_eq!(px[1], 0xffff_0000);
        assert!(pack_orm(&[1.0], &[0.5, 0.0], &[0.0]).is_none());
        let mut v = [0xff00_80ffu32];
        assert!(swizzle(&mut v, [2, 1, 0, 3]));
        assert_eq!(v[0], 0xffff_8000);
        assert!(!swizzle(&mut v, [4, 1, 0, 3]));
    }

    #[test]
    fn bc1_two_greys_round_trip_exactly() {
        let black = 0xff00_0000u32;
        let white = 0xffff_ffffu32;
        let mut block = [black; 16];
        for (i, p) in block.iter_mut().enumerate() {
            if i % 2 == 1 {
                *p = white;
            }
        }
        let bytes = bc1_encode_block(&block);
        assert_eq!(bc1_decode_block(&bytes), block);
    }

    #[test]
    fn bc1_solid_block_is_one_endpoint() {
        let c = 0xffff_ffffu32;
        let bytes = bc1_encode_block(&[c; 16]);
        assert_eq!(&bytes[4..8], &[0, 0, 0, 0]);
        let out = bc1_decode_block(&bytes);
        assert!(out.iter().all(|p| *p == c));
    }

    #[test]
    fn bc1_gradient_error_stays_within_one_palette_step() {
        let mut block = [0u32; 16];
        for (i, p) in block.iter_mut().enumerate() {
            let v = (i as u32 * 17).min(255);
            *p = v | v << 8 | v << 16 | 0xff00_0000;
        }
        let out = bc1_decode_block(&bc1_encode_block(&block));
        for (a, b) in block.iter().zip(out.iter()) {
            for shift in [0u32, 8, 16] {
                let da = ((a >> shift) & 0xff) as i32;
                let db = ((b >> shift) & 0xff) as i32;
                assert!((da - db).abs() <= 43, "{da} vs {db}");
            }
        }
    }
}
