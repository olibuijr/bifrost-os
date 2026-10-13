//! Scaling helpers that turn decoded pictures into thumbnails and screen-fitted images. Decoding itself lives in
//! `cg::imageio` (`ImageSource` and `decode`).

use crate::gfx::Image;

/// Pictures wider or taller than this are refused rather than decoded.
pub use crate::cg::imageio::MAX_SIDE;

/// Shrinks by an integer factor with a box filter (keeps fine detail from aliasing).
pub fn box_down(img: &Image, f: usize) -> Image {
    if f <= 1 {
        return img.clone();
    }
    let (w, h) = (img.w / f, img.h / f);
    let mut out = Image::new(w, h);
    let n = (f * f) as u32;
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0u32; 4];
            for sy in 0..f {
                for sx in 0..f {
                    let p = img.px[(y * f + sy) * img.w + x * f + sx];
                    for (c, s) in sum.iter_mut().enumerate() {
                        *s += (p >> (c * 8)) & 255;
                    }
                }
            }
            out.px[y * w + x] = (0..4).fold(0, |acc, c| acc | ((sum[c] + n / 2) / n) << (c * 8));
        }
    }
    out
}

/// Scales to fill `w` x `h` keeping the aspect, cropping the overflow centred.
pub fn cover(img: &Image, w: usize, h: usize) -> Image {
    if img.w == 0 || img.h == 0 || w == 0 || h == 0 {
        return Image::new(w, h);
    }
    let scale = (w as f32 / img.w as f32).max(h as f32 / img.h as f32);
    let (sw, sh) = (((img.w as f32 * scale).round() as usize).max(w), ((img.h as f32 * scale).round() as usize).max(h));
    let factor = ((img.w / sw).min(img.h / sh)).max(1);
    let scaled = box_down(img, factor).resized(sw, sh);
    scaled.crop((sw - w) / 2, (sh - h) / 2, w, h)
}

/// The size `img` takes when scaled to fit inside `w` x `h` (never enlarged).
pub fn fit_size(img: &Image, w: usize, h: usize) -> (usize, usize) {
    fit_dims(img.w, img.h, w, h)
}

/// The fitted size of an `iw` x `ih` picture inside `w` x `h` (never enlarged), from the
/// dimensions alone, so a header-only size needs no pixels. `(0, 0)` for an empty picture.
pub fn fit_dims(iw: usize, ih: usize, w: usize, h: usize) -> (usize, usize) {
    if iw == 0 || ih == 0 {
        return (0, 0);
    }
    let k = (w as f32 / iw as f32).min(h as f32 / ih as f32).min(1.0);
    (((iw as f32 * k).round() as usize).max(1), ((ih as f32 * k).round() as usize).max(1))
}

/// Scales down to fit inside `w` x `h` keeping the aspect; smaller images
/// are returned as they are.
pub fn contain(img: &Image, w: usize, h: usize) -> Image {
    let (fw, fh) = fit_size(img, w, h);
    if (fw, fh) == (img.w, img.h) || fw == 0 {
        return img.clone();
    }
    let factor = ((img.w / fw).min(img.h / fh)).max(1);
    box_down(img, factor).resized(fw, fh)
}

/// Scales `img` down to fit inside `w` x `h`, keeping the aspect; smaller
/// images are returned as they are (never enlarged).
pub fn fit(img: &Image, w: usize, h: usize) -> Image {
    if img.w == 0 || img.h == 0 || w == 0 || h == 0 || (img.w <= w && img.h <= h) {
        return img.clone();
    }
    let k = (w as f32 / img.w as f32).min(h as f32 / img.h as f32);
    img.resized(((img.w as f32 * k).round() as usize).max(1), ((img.h as f32 * k).round() as usize).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::pack;

    fn solid(w: usize, h: usize, c: u32) -> Image {
        Image { w, h, px: vec![c; w * h] }
    }

    #[test]
    fn cover_fills_and_centres() {
        let mut img = solid(100, 50, pack(0, 0, 255, 255));
        for y in 0..50 {
            for x in 40..60 {
                img.px[y * 100 + x] = pack(255, 0, 0, 255);
            }
        }
        let c = cover(&img, 20, 20);
        assert_eq!((c.w, c.h), (20, 20));
        assert_eq!(c.px[10 * 20 + 10] & 0xff, 255, "the red band stays centred");
        assert_eq!(c.px[10 * 20 + 1] >> 16 & 0xff, 255, "blue at the edges");
        let big = cover(&solid(10, 10, 7), 40, 20);
        assert_eq!((big.w, big.h), (40, 20));
        assert!(big.px.iter().all(|&p| p == 7));
        assert_eq!(cover(&Image::new(0, 0), 4, 4).px.len(), 16);
    }

    #[test]
    fn contain_shrinks_to_fit_and_never_enlarges() {
        let img = solid(4000, 3000, 5);
        let c = contain(&img, 1200, 1920);
        assert_eq!((c.w, c.h), (1200, 900));
        assert!(c.px.iter().all(|&p| p == 5));
        let tall = contain(&solid(300, 3000, 5), 1200, 1920);
        assert_eq!((tall.w, tall.h), (192, 1920));
        let small = contain(&solid(10, 10, 5), 1200, 1920);
        assert_eq!((small.w, small.h), (10, 10));
        assert_eq!(fit_size(&Image::new(0, 0), 10, 10), (0, 0));
    }

    #[test]
    fn fit_only_shrinks() {
        let big = solid(400, 200, 1);
        let f = fit(&big, 100, 100);
        assert_eq!((f.w, f.h), (100, 50));
        let small = solid(10, 20, 1);
        assert_eq!(fit(&small, 100, 100), small);
        assert_eq!(fit(&Image::new(0, 0), 10, 10).px.len(), 0);
    }
}
