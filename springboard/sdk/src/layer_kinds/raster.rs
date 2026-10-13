//! Pixel helpers for the layer content kinds: coverage compositing, blits and bilinear sampling on premultiplied
//! images. Scan conversion itself is `bifrost_render::cg::raster`.
use bifrost_render::gfx::{self, Image};
pub use bifrost_render::cg::raster::{fill_polygons, stroke_polygons, Coverage, FillRule, Pt, Run};

/// Composites `color` (straight RGBA) over `img` where `cov` is set.
pub fn paint(img: &mut Image, cov: &Coverage, color: [u8; 4]) {
    let c = gfx::rgba(color[0], color[1], color[2], color[3]);
    for (p, &a) in img.px.iter_mut().zip(&cov.a) {
        if a != 0 {
            *p = gfx::over(*p, gfx::scale(c, a as u32));
        }
    }
}

/// Source-over of `src` onto `dst` with the top-left of `src` at `(x, y)`; clipped to `dst`.
pub fn blit_over(dst: &mut Image, src: &Image, x: i32, y: i32) {
    for sy in 0..src.h {
        let dy = y + sy as i32;
        if dy < 0 || dy >= dst.h as i32 {
            continue;
        }
        for sx in 0..src.w {
            let dx = x + sx as i32;
            if dx < 0 || dx >= dst.w as i32 {
                continue;
            }
            let s = src.px[sy * src.w + sx];
            if s != 0 {
                let d = &mut dst.px[dy as usize * dst.w + dx as usize];
                *d = gfx::over(*d, s);
            }
        }
    }
}

/// Packed premultiplied pixel to floats 0..1 (r, g, b, a), still premultiplied.
pub fn unpack(p: u32) -> [f32; 4] {
    let ch = |s: u32| ((p >> s) & 255) as f32 / 255.0;
    [ch(0), ch(8), ch(16), ch(24)]
}

/// Premultiplied floats 0..1 (r, g, b, a) to a packed pixel.
pub fn pack_premul(c: [f32; 4]) -> u32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    q(c[0]) | q(c[1]) << 8 | q(c[2]) << 16 | q(c[3]) << 24
}

/// Bilinear sample of a premultiplied image at pixel-space `(x, y)` (pixel centres at half integers). Outside the
/// image is transparent.
pub fn sample(img: &Image, x: f32, y: f32) -> [f32; 4] {
    let (fx, fy) = (x - 0.5, y - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let get = |ix: i32, iy: i32| -> [f32; 4] {
        if ix < 0 || iy < 0 || ix >= img.w as i32 || iy >= img.h as i32 {
            return [0.0; 4];
        }
        unpack(img.px[iy as usize * img.w + ix as usize])
    };
    let (ix, iy) = (x0 as i32, y0 as i32);
    let (a, b, c, d) = (get(ix, iy), get(ix + 1, iy), get(ix, iy + 1), get(ix + 1, iy + 1));
    let mut out = [0.0; 4];
    for i in 0..4 {
        let top = a[i] + (b[i] - a[i]) * tx;
        let bot = c[i] + (d[i] - c[i]) * tx;
        out[i] = top + (bot - top) * ty;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paint_composites_premultiplied_colour_by_coverage() {
        let mut img = Image::new(2, 1);
        let cov = Coverage { x: 0, y: 0, w: 2, h: 1, a: vec![255, 0] };
        paint(&mut img, &cov, [255, 0, 0, 255]);
        assert_eq!(img.px[0], gfx::rgba(255, 0, 0, 255));
        assert_eq!(img.px[1], 0);
    }

    #[test]
    fn bilinear_sampling_blends_neighbours_and_is_transparent_outside() {
        let img = Image { w: 2, h: 1, px: vec![gfx::rgba(255, 255, 255, 255), 0] };
        let mid = sample(&img, 1.0, 0.5);
        assert!((mid[3] - 0.5).abs() < 0.01);
        assert_eq!(sample(&img, 5.0, 0.5)[3], 0.0);
    }
}
