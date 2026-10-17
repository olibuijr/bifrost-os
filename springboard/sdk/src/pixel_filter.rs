//! Pixel filters for app images on the luma channel: threshold modes, histogram, histogram equalization,
//! rectangular erosion and dilation, the median, and the Sobel and Laplacian edge filters. These are the
//! app-facing entry points for the filters in `bifrost_render::filter`, which the shell does not call. An app
//! passes an `Image` (premultiplied RGBA, see `bifrost_render::gfx`); the filter works on its luma plane and
//! returns a grey image. Colour is dropped, so use the colour filters in `image_filter` to keep colour.
//! See docs/bifrost-ui.md for the decisions and the gaps (no GPU dispatch and no lazy image graph yet).
use bifrost_render::filter as f;
use bifrost_render::gfx::Image;

pub use bifrost_render::filter::{Plane, Threshold};

/// Luma of a pixel with the Rec. 601 weights in integer form, from its colour bytes. Alpha is ignored.
fn luma_of(px: u32) -> u8 {
    let r = (px & 0xff) as u32;
    let g = ((px >> 8) & 0xff) as u32;
    let b = ((px >> 16) & 0xff) as u32;
    ((r * 299 + g * 587 + b * 114 + 500) / 1000) as u8
}

/// The luma plane of `img`, one byte per pixel.
pub fn luma_plane(img: &Image) -> Plane {
    Plane { width: img.w, height: img.h, pixels: img.px.iter().map(|&px| luma_of(px)).collect() }
}

/// An opaque grey image where each pixel's colour is the plane value.
pub fn grey_image(plane: &Plane) -> Image {
    let px = plane
        .pixels
        .iter()
        .map(|&v| {
            let v = v as u32;
            0xff00_0000 | (v << 16) | (v << 8) | v
        })
        .collect();
    Image { w: plane.width, h: plane.height, px }
}

/// Applies a threshold mode to the luma of `img`. See `bifrost_render::filter::Threshold`.
pub fn threshold(img: &Image, mode: Threshold, t: u8, max: u8) -> Image {
    grey_image(&f::threshold(&luma_plane(img), mode, t, max))
}

/// The 256-bin histogram of the luma of `img`. Each bin holds the count of pixels with that value.
pub fn luma_histogram(img: &Image) -> Vec<u32> {
    let info = f::HistogramInfo::new(256, 0, 255).expect("256 bins over 0..=255 is a valid range");
    f::histogram(&luma_plane(img), info)
}

/// Histogram equalization of the luma of `img`, as a grey image.
pub fn equalize(img: &Image) -> Image {
    grey_image(&f::equalize(&luma_plane(img)))
}

/// Erosion over a `kw` by `kh` window of the luma. `None` when a side is zero or even.
pub fn erode(img: &Image, kw: usize, kh: usize) -> Option<Image> {
    f::erode(&luma_plane(img), kw, kh).map(|p| grey_image(&p))
}

/// Dilation over a `kw` by `kh` window of the luma. `None` when a side is zero or even.
pub fn dilate(img: &Image, kw: usize, kh: usize) -> Option<Image> {
    f::dilate(&luma_plane(img), kw, kh).map(|p| grey_image(&p))
}

/// The median of the luma over a square of side `diameter`. `None` when the diameter is not an odd side.
pub fn median(img: &Image, diameter: usize) -> Option<Image> {
    f::median(&luma_plane(img), diameter).map(|p| grey_image(&p))
}

/// The Sobel gradient magnitude of the luma of `img`.
pub fn sobel(img: &Image) -> Image {
    grey_image(&f::sobel(&luma_plane(img)))
}

/// The four-neighbour Laplacian of the luma of `img`, with `bias` added before clamping.
pub fn laplacian(img: &Image, bias: f32) -> Image {
    grey_image(&f::laplacian(&luma_plane(img), bias))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, px: u32) -> Image {
        Image { w, h, px: vec![px; w * h] }
    }

    #[test]
    fn luma_uses_the_rec601_weights() {
        assert_eq!(luma_of(0xff00_0000), 0);
        assert_eq!(luma_of(0xffff_ffff), 255);
        // Pure red, green and blue give 76, 150 and 29 (299, 587 and 114 out of 1000).
        assert_eq!(luma_of(0xff00_00ff), 76);
        assert_eq!(luma_of(0xff00_ff00), 150);
        assert_eq!(luma_of(0xffff_0000), 29);
    }

    #[test]
    fn a_plane_round_trips_through_a_grey_image() {
        let plane = Plane::new(2, 1, vec![10, 200]).unwrap();
        let img = grey_image(&plane);
        assert_eq!(img.px, vec![0xff0a_0a0a, 0xffc8_c8c8]);
        assert_eq!(luma_plane(&img), plane);
    }

    #[test]
    fn binary_threshold_gives_max_above_the_threshold() {
        let img = Image { w: 2, h: 1, px: vec![0xff00_0000, 0xffff_ffff] };
        let out = threshold(&img, Threshold::Binary, 100, 255);
        assert_eq!(out.px, vec![0xff00_0000, 0xffff_ffff]);
    }

    #[test]
    fn the_histogram_counts_every_pixel() {
        let img = solid(3, 2, 0xff80_8080);
        let bins = luma_histogram(&img);
        assert_eq!(bins.len(), 256);
        assert_eq!(bins[128], 6);
        assert_eq!(bins.iter().sum::<u32>(), 6);
    }

    #[test]
    fn a_flat_image_keeps_its_value_under_erosion_and_dilation() {
        let img = solid(4, 4, 0xff40_4040);
        assert_eq!(erode(&img, 3, 3).unwrap().px, img.px);
        assert_eq!(dilate(&img, 3, 3).unwrap().px, img.px);
    }

    #[test]
    fn an_even_window_is_refused() {
        let img = solid(4, 4, 0xff00_0000);
        assert!(erode(&img, 2, 3).is_none());
        assert!(dilate(&img, 3, 4).is_none());
    }

    #[test]
    fn a_flat_image_has_no_edges() {
        let img = solid(4, 4, 0xff55_5555);
        let edges = sobel(&img);
        assert!(edges.px.iter().all(|&px| px & 0xff == 0));
    }
}
