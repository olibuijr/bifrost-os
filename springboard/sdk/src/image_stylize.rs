//! Stylize, sharpen, halftone, tile, transition, convolution and reduction filters for app images: the
//! counterparts of `CIUnsharpMask`, `CISharpenLuminance`, `CIBloom`, `CIGloom`, `CIPixellate`,
//! `CICrystallize`, `CIPointillize`, `CIComicEffect`, `CISpotLight`, the screens and `CICMYKHalftone`,
//! the tile filters, the transition filters, the convolution filters (`CIConvolution3X3` and the 5x5,
//! 7x7, 9 horizontal and 9 vertical forms, with and without alpha) and the colour area reductions
//! (`CIAreaAverage`, `CIAreaMinMax`, `CIAreaHistogram`, `CIRowAverage`, `CIColumnAverage`, `CIKMeans`).
//! Apps decode an image with `ImageSource` and call these on the `Image`; the pixel work lives in the
//! render crate (`stylize_filters`, `halftone_tile_filters`, `tile_shapes`, `transition_filters`,
//! `comic_filter`, `spot_light`, `filter`, `colour_stats`), which the shell can use too.
//! Limits: everything runs on the CPU with no lazy graph; the parameters follow the reference names
//! but several curves are approximations (see the notes in the render modules); convolution takes odd
//! sides 3 to 9, a superset of the reference sizes; nothing here picks a filter by its reference name,
//! use `image_filter::FilterChain` for the named chain.
use bifrost_render::filter::{self, Plane};
use bifrost_render::gfx::Image;

pub use bifrost_render::colorfx::Rgb;
pub use bifrost_render::colour_stats::{Cluster, Region, Rgba};
pub use bifrost_render::filter::Axis;
pub use bifrost_render::halftone_tile_filters::{Screen, ScreenParams};
pub use bifrost_render::spot_light::Point3;

// ---------------------------------------------------------------- sharpen and stylize

/// `CIUnsharpMask`: adds `intensity` times the difference between the image and its blur of `radius`.
pub fn unsharp_mask(img: &Image, radius: f32, intensity: f32) -> Image {
    bifrost_render::stylize_filters::unsharp_mask(img, radius, intensity)
}

/// `CISharpenLuminance`: sharpens only the luminance. Reference defaults: sharpness 0.4, radius 1.69.
pub fn sharpen_luminance(img: &Image, sharpness: f32, radius: f32) -> Image {
    bifrost_render::stylize_filters::sharpen_luminance(img, sharpness, radius)
}

/// `CIBloom`: softens the image and makes the bright parts glow. `intensity` 0 is the identity.
pub fn bloom(img: &Image, radius: f32, intensity: f32) -> Image {
    bifrost_render::stylize_filters::bloom(img, radius, intensity)
}

/// `CIGloom`: dulls the highlights.
pub fn gloom(img: &Image, radius: f32, intensity: f32) -> Image {
    bifrost_render::stylize_filters::gloom(img, radius, intensity)
}

/// `CIPixellate`: replaces each `scale` by `scale` block by its mean colour. A scale of 1 or less is the identity.
pub fn pixellate(img: &Image, center: (f32, f32), scale: f32) -> Image {
    bifrost_render::stylize_filters::pixellate(img, center, scale)
}

/// `CICrystallize`: cells of about `radius` pixels, each filled with one colour.
pub fn crystallize(img: &Image, radius: f32) -> Image {
    bifrost_render::stylize_filters::crystallize(img, radius)
}

/// `CIPointillize`: the image as dots of about `radius` pixels.
pub fn pointillize(img: &Image, radius: f32) -> Image {
    bifrost_render::stylize_filters::pointillize(img, radius)
}

/// `CIComicEffect`: flat colour (`levels` 2 to 8 tones per channel) with black outlines where the edge
/// strength is above `threshold` (0 to 1). The reference filter has no parameters; the defaults are
/// 4 levels and 0.25.
pub fn comic(img: &Image, levels: u8, threshold: f32) -> Image {
    bifrost_render::comic_filter::comic(img, levels, threshold)
}

/// `CISpotLight`: adds the light of a cone-shaped lamp at `light` aimed at `target`, in pixels with the image
/// in the plane `z = 0`. `brightness` 0 returns the image unchanged. The light does not fall off with distance.
pub fn spot_light(img: &Image, light: Point3, target: Point3, brightness: f32, concentration: f32, colour: Rgb) -> Image {
    bifrost_render::spot_light::spot_light(img, light, target, brightness, concentration, colour)
}

// ---------------------------------------------------------------- halftone and tile

/// `CIDotScreen`, `CILineScreen`, `CICircularScreen` or `CIHatchedScreen`, by `kind`.
pub fn halftone(img: &Image, kind: Screen, params: &ScreenParams) -> Image {
    bifrost_render::halftone_tile_filters::screen(img, kind, params)
}

/// `CICMYKHalftone`: one screen per ink at its own angle. `gcr` and `ucr` (0 to 1) are the grey component
/// replacement and the under colour removal.
pub fn cmyk_halftone(img: &Image, params: &ScreenParams, gcr: f32, ucr: f32) -> Image {
    bifrost_render::halftone_tile_filters::cmyk_halftone(img, params, gcr, ucr)
}

/// `CIKaleidoscope`: `count` mirrored wedges about `center`, turned by `angle` radians.
pub fn kaleidoscope(img: &Image, center: (f32, f32), count: u32, angle: f32) -> Image {
    bifrost_render::halftone_tile_filters::kaleidoscope(img, center, count, angle)
}

/// `CIOpTile`: a mirrored, scaled and rotated grid of the image about `center`.
pub fn op_tile(img: &Image, center: (f32, f32), scale: f32, angle: f32, width: f32) -> Image {
    bifrost_render::halftone_tile_filters::op_tile(img, center, scale, angle, width)
}

/// `CIAffineTile`: repeats the image after the affine transform `[a b c d tx ty]`.
pub fn affine_tile(img: &Image, transform: [f32; 6]) -> Image {
    bifrost_render::halftone_tile_filters::affine_tile(img, transform)
}

/// `CITriangleTile`: right triangles of `width` pixels, turned by `angle` radians about `center`.
pub fn triangle_tile(img: &Image, center: (f32, f32), angle: f32, width: f32) -> Image {
    bifrost_render::tile_shapes::triangle_tile(img, center, angle, width)
}

/// `CIParallelogramTile`: parallelograms of `width` pixels with the acute angle `acute` (radians).
pub fn parallelogram_tile(img: &Image, center: (f32, f32), angle: f32, acute: f32, width: f32) -> Image {
    bifrost_render::tile_shapes::parallelogram_tile(img, center, angle, acute, width)
}

/// `CIPerspectiveTile`: the image laid on `quad` (top left, top right, bottom right, bottom left) and repeated.
/// `None` when the quad is degenerate.
pub fn perspective_tile(img: &Image, quad: [(f32, f32); 4]) -> Option<Image> {
    bifrost_render::tile_shapes::perspective_tile(img, quad)
}

// ---------------------------------------------------------------- transitions

/// A transition between two images of the same size. `time` 0 shows the first image and 1 the second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transition {
    /// A cross-dissolve: each channel moves linearly from the first image to the second.
    Dissolve,
    /// A straight edge sweeps across in the direction `angle` (radians, 0 is left to right).
    Swipe { angle: f32 },
    /// Bars of `bar_width` pixels sweep across, alternating direction.
    BarsSwipe { angle: f32, bar_width: f32 },
    /// A bright band of `band` pixels leads the sweep.
    CopyMachine { angle: f32, band: f32 },
    /// A flash at `center` covers the change.
    Flash { center: (f32, f32) },
    /// A shape with `lobes` lobes grows from `center`.
    Mod { center: (f32, f32), lobes: u32 },
    /// A page curls away in the direction `angle`; `curl` is the radius of the curl in pixels.
    PageCurl { angle: f32, curl: f32 },
    /// A ripple spreads from `center`.
    Ripple { center: (f32, f32) },
    /// The image folds like an accordion in `folds` folds.
    AccordionFold { folds: u32 },
    /// The first image breaks up into noise, seeded by `seed`.
    Disintegrate { seed: u32 },
}

/// The frame of `kind` at `time` (0 to 1) between `from` and `to`. `None` when the images differ in size, are
/// empty, or a parameter is not finite.
pub fn transition(from: &Image, to: &Image, time: f32, kind: Transition) -> Option<Image> {
    use bifrost_render::transition_filters as t;
    match kind {
        Transition::Dissolve => dissolve(from, to, time),
        Transition::Swipe { angle } => t::swipe(from, to, time, angle),
        Transition::BarsSwipe { angle, bar_width } => t::bars_swipe(from, to, time, angle, bar_width),
        Transition::CopyMachine { angle, band } => t::copy_machine(from, to, time, angle, band),
        Transition::Flash { center } => t::flash(from, to, time, center),
        Transition::Mod { center, lobes } => t::mod_reveal(from, to, time, center, lobes),
        Transition::PageCurl { angle, curl } => t::page_curl(from, to, time, angle, curl),
        Transition::Ripple { center } => t::ripple(from, to, time, center),
        Transition::AccordionFold { folds } => t::accordion_fold(from, to, time, folds),
        Transition::Disintegrate { seed } => t::disintegrate(from, to, time, seed),
    }
}

fn dissolve(from: &Image, to: &Image, time: f32) -> Option<Image> {
    if from.w != to.w || from.h != to.h || from.px.is_empty() || from.px.len() != to.px.len() || !time.is_finite() {
        return None;
    }
    let t = time.clamp(0.0, 1.0);
    let px = from
        .px
        .iter()
        .zip(&to.px)
        .map(|(&a, &b)| {
            let mix = |shift: u32| {
                let (x, y) = ((a >> shift & 0xff) as f32, (b >> shift & 0xff) as f32);
                ((x + (y - x) * t) + 0.5) as u32
            };
            mix(0) | mix(8) << 8 | mix(16) << 16 | mix(24) << 24
        })
        .collect();
    Some(Image { w: from.w, h: from.h, px })
}

// ---------------------------------------------------------------- convolution

/// The colour planes (and alpha when `alpha`) of `img` through `f`; pixels stay premultiplied, so a colour
/// is clamped to its alpha afterwards. `None` when `f` refuses or the image's pixel count does not match its size.
fn convolve_planes(img: &Image, alpha: bool, f: impl Fn(&Plane) -> Option<Plane>) -> Option<Image> {
    if img.px.len() != img.w.checked_mul(img.h)? || img.px.is_empty() {
        return None;
    }
    let channels = if alpha { 4 } else { 3 };
    let mut out = img.clone();
    let mut planes: Vec<Plane> = Vec::new();
    for c in 0..channels {
        let bytes: Vec<u8> = img.px.iter().map(|&p| (p >> (8 * c) & 0xff) as u8).collect();
        planes.push(f(&Plane::new(img.w, img.h, bytes)?)?);
    }
    for (i, p) in out.px.iter_mut().enumerate() {
        let a = if alpha { planes[3].pixels[i] as u32 } else { *p >> 24 };
        let ch = |c: usize| (planes[c].pixels[i] as u32).min(a);
        *p = ch(0) | ch(1) << 8 | ch(2) << 16 | a << 24;
    }
    Some(out)
}

/// A square convolution (`CIConvolution3X3`, `5X5`, `7X7`; with `alpha` false the `RGB` forms, which leave alpha
/// alone). `weights` has `side * side` entries, row-major and applied without a flip. `bias` is a fraction of the
/// channel range added to each sum, so 0.5 is mid grey. Pixels beyond the edge repeat the nearest edge pixel.
/// `None` unless `side` is odd from 3 to 9 and the weights match.
pub fn convolve(img: &Image, side: usize, weights: &[f32], bias: f32, alpha: bool) -> Option<Image> {
    if !filter::is_convolution_side(side) || !bias.is_finite() {
        return None;
    }
    convolve_planes(img, alpha, |p| filter::convolve(p, side, side, weights, bias * 255.0))
}

/// A one-dimensional convolution along `axis` (`CIConvolution9Horizontal` and `CIConvolution9Vertical`, with
/// `taps` of any odd length from 3 to 9). The other arguments are as for [`convolve`].
pub fn convolve_line(img: &Image, axis: Axis, taps: &[f32], bias: f32, alpha: bool) -> Option<Image> {
    if !bias.is_finite() {
        return None;
    }
    convolve_planes(img, alpha, |p| filter::convolve_axis(p, axis, taps, bias * 255.0))
}

// ---------------------------------------------------------------- reductions

/// `CIAreaAverage` over the whole image.
pub fn average_colour(img: &Image) -> Option<Rgba> {
    bifrost_render::colour_stats::area_average(img, Region::whole(img))
}

/// `CIAreaAverage` over `region`, clipped to the image. `None` when nothing is left.
pub fn area_average(img: &Image, region: Region) -> Option<Rgba> {
    bifrost_render::colour_stats::area_average(img, region)
}

/// `CIAreaMinMax`: the smallest and largest value of each channel in `region`, as `(minimum, maximum)`.
pub fn area_min_max(img: &Image, region: Region) -> Option<(Rgba, Rgba)> {
    bifrost_render::colour_stats::area_min_max(img, region)
}

/// `CIAreaHistogram`: `[red, green, blue, alpha]` counts in `bins` equal slices (1 to 256) of each channel.
pub fn area_histogram(img: &Image, region: Region, bins: usize) -> Option<[Vec<u32>; 4]> {
    bifrost_render::colour_stats::area_histogram(img, region, bins)
}

/// `CIRowAverage`: the mean colour of each row of `region`.
pub fn row_averages(img: &Image, region: Region) -> Option<Vec<Rgba>> {
    bifrost_render::colour_stats::row_averages(img, region)
}

/// `CIColumnAverage`: the mean colour of each column of `region`.
pub fn column_averages(img: &Image, region: Region) -> Option<Vec<Rgba>> {
    bifrost_render::colour_stats::column_averages(img, region)
}

/// `CIKMeans` from the given starting centres (the reference `inputMeans`), `passes` passes.
pub fn kmeans(img: &Image, region: Region, means: &[Rgba], passes: usize) -> Option<Vec<Cluster>> {
    bifrost_render::colour_stats::kmeans(img, region, means, passes)
}

/// The `k` dominant colours of the whole image, largest first, from a repeatable starting spread (no starting
/// centres to supply).
pub fn dominant_colours(img: &Image, k: usize) -> Option<Vec<Cluster>> {
    bifrost_render::colour_stats::dominant_colours(img, Region::whole(img), k, 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLACK: u32 = 0xff00_0000;
    const WHITE: u32 = 0xffff_ffff;

    fn solid(w: usize, h: usize, px: u32) -> Image {
        Image { w, h, px: vec![px; w * h] }
    }

    fn row(reds: &[u8]) -> Image {
        Image { w: reds.len(), h: 1, px: reds.iter().map(|&r| 0xff00_0000 | r as u32).collect() }
    }

    #[test]
    fn identity_settings_leave_the_image_unchanged() {
        let img = solid(4, 4, 0xff40_80c0);
        assert_eq!(unsharp_mask(&img, 2.5, 0.0), img);
        assert_eq!(sharpen_luminance(&img, 0.0, 1.69), img);
        assert_eq!(pixellate(&img, (0.0, 0.0), 1.0), img);
        assert_eq!(spot_light(&img, (0.0, 0.0, 50.0), (2.0, 2.0, 0.0), 0.0, 1.0, Rgb::new(1.0, 1.0, 1.0)), img);
    }

    #[test]
    fn stylize_filters_keep_the_size() {
        let img = solid(9, 7, 0xff40_80c0);
        for out in [crystallize(&img, 3.0), pointillize(&img, 3.0), bloom(&img, 2.0, 0.5), gloom(&img, 2.0, 0.5), comic(&img, 4, 0.25)] {
            assert_eq!((out.w, out.h, out.px.len()), (9, 7, 63));
        }
    }

    #[test]
    fn comic_keeps_the_alpha_channel() {
        let out = comic(&solid(5, 5, 0x8020_2020), 4, 0.25);
        assert!(out.px.iter().all(|&p| p >> 24 == 0x80));
    }

    #[test]
    fn screens_keep_the_size_and_alpha() {
        let img = solid(8, 8, 0x80ff_ffff);
        let p = ScreenParams::default();
        for kind in [Screen::Dot, Screen::Line, Screen::Circular, Screen::Hatched] {
            let out = halftone(&img, kind, &p);
            assert_eq!((out.w, out.h), (8, 8));
            assert!(out.px.iter().all(|&px| px >> 24 == 0x80), "{kind:?}");
        }
        let c = cmyk_halftone(&img, &p, 1.0, 0.5);
        assert_eq!((c.w, c.h), (8, 8));
    }

    #[test]
    fn tiles_keep_the_size_and_refuse_a_degenerate_quad() {
        let img = solid(6, 6, 0xff10_2030);
        assert_eq!(kaleidoscope(&img, (3.0, 3.0), 6, 0.0).px.len(), 36);
        assert_eq!(op_tile(&img, (3.0, 3.0), 2.8, 0.0, 65.0).px.len(), 36);
        assert_eq!(affine_tile(&img, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]).px.len(), 36);
        assert_eq!(triangle_tile(&img, (3.0, 3.0), 0.0, 4.0).px.len(), 36);
        assert_eq!(parallelogram_tile(&img, (3.0, 3.0), 0.0, 1.047, 4.0).px.len(), 36);
        assert!(perspective_tile(&img, [(0.0, 0.0); 4]).is_none());
    }

    #[test]
    fn dissolve_runs_from_the_first_image_to_the_second() {
        let (a, b) = (solid(2, 2, BLACK), solid(2, 2, WHITE));
        assert_eq!(transition(&a, &b, 0.0, Transition::Dissolve).unwrap(), a);
        assert_eq!(transition(&a, &b, 1.0, Transition::Dissolve).unwrap(), b);
        let mid = transition(&a, &b, 0.5, Transition::Dissolve).unwrap().px[0];
        for shift in [0, 8, 16] {
            assert!(((mid >> shift & 0xff) as i32 - 128).abs() <= 1);
        }
    }

    #[test]
    fn transitions_refuse_mismatched_images_and_swipe_hits_both_ends() {
        let (a, b) = (solid(8, 4, BLACK), solid(8, 4, WHITE));
        assert!(transition(&a, &solid(4, 4, WHITE), 0.5, Transition::Dissolve).is_none());
        assert!(transition(&a, &solid(4, 4, WHITE), 0.5, Transition::Swipe { angle: 0.0 }).is_none());
        assert_eq!(transition(&a, &b, 0.0, Transition::Swipe { angle: 0.0 }).unwrap(), a);
        assert_eq!(transition(&a, &b, 1.0, Transition::Swipe { angle: 0.0 }).unwrap(), b);
    }

    #[test]
    fn identity_kernel_changes_nothing_and_a_shift_kernel_moves_pixels() {
        let img = row(&[10, 20, 30, 40]);
        let mut id = [0.0f32; 9];
        id[4] = 1.0;
        assert_eq!(convolve(&img, 3, &id, 0.0, true).unwrap(), img);
        assert_eq!(convolve(&img, 3, &id, 0.0, false).unwrap(), img);
        // The tap right of centre pulls each pixel from its right neighbour; the last one repeats the edge.
        let shift = [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        let out = convolve_line(&img, Axis::Horizontal, &shift, 0.0, true).unwrap();
        let reds: Vec<u32> = out.px.iter().map(|&p| p & 0xff).collect();
        assert_eq!(reds, [20, 30, 40, 40]);
    }

    #[test]
    fn box_kernel_keeps_a_flat_image_and_bias_lifts_it() {
        let img = solid(5, 5, 0xff64_6464);
        let boxk = [1.0f32 / 9.0; 9];
        assert_eq!(convolve(&img, 3, &boxk, 0.0, false).unwrap(), img);
        let lifted = convolve(&img, 3, &[0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0], 0.1, false).unwrap();
        assert_eq!(lifted.px[0] & 0xff, 100 + 26);
    }

    #[test]
    fn convolution_refuses_bad_sizes() {
        let img = solid(4, 4, WHITE);
        assert!(convolve(&img, 4, &[0.0; 16], 0.0, true).is_none());
        assert!(convolve(&img, 3, &[0.0; 8], 0.0, true).is_none());
        assert!(convolve(&img, 11, &[0.0; 121], 0.0, true).is_none());
        assert!(convolve(&img, 3, &[0.0; 9], f32::NAN, true).is_none());
        assert!(convolve_line(&img, Axis::Vertical, &[1.0, 1.0], 0.0, true).is_none());
    }

    #[test]
    fn whole_image_reductions_agree_with_the_pixels() {
        let img = Image { w: 2, h: 1, px: vec![BLACK, WHITE] };
        let avg = average_colour(&img).unwrap();
        assert!((avg.r - 0.5).abs() < 1e-6);
        let (lo, hi) = area_min_max(&img, Region::whole(&img)).unwrap();
        assert_eq!((lo.pack(), hi.pack()), (BLACK, WHITE));
        let hist = area_histogram(&img, Region::whole(&img), 2).unwrap();
        assert_eq!(hist[1], [1, 1]);
        let dom = dominant_colours(&img, 2).unwrap();
        assert_eq!(dom.iter().map(|c| c.count).sum::<usize>(), 2);
    }
}
