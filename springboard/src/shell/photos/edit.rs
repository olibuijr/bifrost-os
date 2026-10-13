//! The edit mode's pixel work: rotate by quarter turns, crop to an aspect
//! preset, filters, blur, brightness and contrast, exposure, saturation,
//! warmth and vignette. The edit itself is stored as a recipe (see `recipe`);
//! the original file is never written. Photos are opaque,
//! so the colour ops treat the packed pixels as straight RGB and leave alpha
//! alone. The blur and the colour effects come from the renderer's own filter
//! models (`blur_kernels`, `effect_chain`), not separate code.

use crate::gfx::Image;
use bifrost_render::{blur_kernels, effect_chain};

/// Crop presets of the edit bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aspect {
    /// the whole picture
    Free,
    Original,
    Square,
    R4x3,
    R3x2,
    R16x9,
}

impl Aspect {
    pub const ALL: [Aspect; 6] = [Aspect::Free, Aspect::Original, Aspect::Square, Aspect::R4x3, Aspect::R3x2, Aspect::R16x9];

    pub fn label(self) -> &'static str {
        match self {
            Aspect::Free => "Free",
            Aspect::Original => "Original",
            Aspect::Square => "Square",
            Aspect::R4x3 => "4:3",
            Aspect::R3x2 => "3:2",
            Aspect::R16x9 => "16:9",
        }
    }

    /// width / height for a picture of `w` x `h`; presets follow the
    /// picture's orientation (a portrait photo crops to 3:4, not 4:3).
    pub fn ratio(self, w: usize, h: usize) -> Option<f32> {
        let landscape = w >= h;
        let r = match self {
            Aspect::Free | Aspect::Original => return None,
            Aspect::Square => 1.0,
            Aspect::R4x3 => 4.0 / 3.0,
            Aspect::R3x2 => 3.0 / 2.0,
            Aspect::R16x9 => 16.0 / 9.0,
        };
        Some(if landscape { r } else { 1.0 / r })
    }
}

/// Colour filters of the edit bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Original,
    Vivid,
    Mono,
    Noir,
    Warm,
    Sepia,
}

impl Filter {
    pub const ALL: [Filter; 6] = [Filter::Original, Filter::Vivid, Filter::Mono, Filter::Noir, Filter::Warm, Filter::Sepia];

    pub fn label(self) -> &'static str {
        match self {
            Filter::Original => "Original",
            Filter::Vivid => "Vivid",
            Filter::Mono => "Mono",
            Filter::Noir => "Noir",
            Filter::Warm => "Warm",
            Filter::Sepia => "Sepia",
        }
    }

    /// One pixel's (r, g, b) through the filter.
    fn pixel(self, r: u8, g: u8, b: u8) -> (u8, u8, u8) {
        let (rf, gf, bf) = (r as f32, g as f32, b as f32);
        let luma = 0.299 * rf + 0.587 * gf + 0.114 * bf;
        let c = |v: f32| v.round().clamp(0.0, 255.0) as u8;
        match self {
            Filter::Original => (r, g, b),
            // push every channel away from the grey of the pixel, then a touch of contrast
            Filter::Vivid => {
                let f = |v: f32| (luma + (v - luma) * 1.45 - 127.5) * 1.08 + 127.5;
                (c(f(rf)), c(f(gf)), c(f(bf)))
            }
            Filter::Mono => {
                let l = c(luma);
                (l, l, l)
            }
            // black and white with a strong S-curve
            Filter::Noir => {
                let l = ((luma - 127.5) * 1.35 + 127.5 - 12.0).round().clamp(0.0, 255.0) as u8;
                (l, l, l)
            }
            Filter::Warm => (c(rf * 1.08 + 6.0), c(gf * 1.02 + 2.0), c(bf * 0.88)),
            // sepia runs through the effect chain in `apply`; this arm is never reached
            Filter::Sepia => (r, g, b),
        }
    }

    pub fn apply(self, img: &mut Image) {
        match self {
            Filter::Original => {}
            Filter::Sepia => tone(img, vec![named("sepia", &[("amount", 1.0)])]),
            _ => {
                for p in &mut img.px {
                    let (r, g, b) = self.pixel(*p as u8, (*p >> 8) as u8, (*p >> 16) as u8);
                    *p = crate::gfx::pack(r, g, b, (*p >> 24) as u8);
                }
            }
        }
    }
}

/// A filter of the core-image catalogue, made by name with the given inputs set.
fn named(name: &str, inputs: &[(&str, f32)]) -> Option<effect_chain::Filter> {
    let mut f = effect_chain::Filter::named(name)?;
    for &(key, value) in inputs {
        f.set(key, value);
    }
    Some(f)
}

/// Runs catalogue filters over the picture, one after another. Missing names are skipped.
fn tone(img: &mut Image, filters: Vec<Option<effect_chain::Filter>>) {
    let mut chain = effect_chain::Chain::new();
    for f in filters.into_iter().flatten() {
        chain.push(f);
    }
    if chain.is_empty() {
        return;
    }
    if let Some(px) = chain.apply(&img.px, img.w, img.h) {
        img.px = px;
    }
}

/// Stops of exposure at the ends of the exposure slider.
pub const EXPOSURE_STOPS: f32 = 2.0;

/// The blur radius in pixels at the top of the blur slider.
pub const BLUR_RADIUS_MAX: f32 = 3.0;

/// The disc radius for a blur slider value in 0..=1.
pub fn blur_radius(amount: f32) -> usize {
    (amount.clamp(0.0, 1.0) * BLUR_RADIUS_MAX).round() as usize
}

/// Averages each pixel with its disc of neighbours `radius` pixels wide. Radius 0 leaves the picture.
pub fn blur(img: &mut Image, radius: usize) {
    if radius == 0 {
        return;
    }
    if let Some(px) = blur_kernels::disc_blur(&img.px, img.w, img.h, radius) {
        img.px = px;
    }
}

/// Everything the edit mode changes, applied in this order: rotate,
/// straighten, crop, filter, blur, brightness and contrast, exposure,
/// saturation, warmth and vignette.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edit {
    /// clockwise quarter turns, 0..=3
    pub turns: u8,
    pub aspect: Aspect,
    /// 1 = the largest crop of that aspect, 2 = half as wide
    pub crop_zoom: f32,
    /// the crop's centre as fractions of the rotated picture
    pub crop_center: (f32, f32),
    /// straighten angle in degrees, `-STRAIGHTEN_MAX..=STRAIGHTEN_MAX`
    pub straighten: f32,
    pub filter: Filter,
    /// -1..=1
    pub brightness: f32,
    /// -1..=1
    pub contrast: f32,
    /// -1..=1, scaled to `EXPOSURE_STOPS` stops either way
    pub exposure: f32,
    /// -1..=1: -1 is grey, 1 doubles the colour
    pub saturation: f32,
    /// -1..=1: cool to warm
    pub warmth: f32,
    /// 0..=1, see `blur_radius`
    pub blur: f32,
    /// 0..=1: darkens the corners
    pub vignette: f32,
}

impl Default for Edit {
    fn default() -> Self {
        Edit {
            turns: 0,
            aspect: Aspect::Free,
            crop_zoom: 1.0,
            crop_center: (0.5, 0.5),
            straighten: 0.0,
            filter: Filter::Original,
            brightness: 0.0,
            contrast: 0.0,
            exposure: 0.0,
            saturation: 0.0,
            warmth: 0.0,
            blur: 0.0,
            vignette: 0.0,
        }
    }
}

impl Edit {
    pub fn is_identity(&self) -> bool {
        self.turns % 4 == 0
            && self.aspect == Aspect::Free
            && self.crop_zoom <= 1.0
            && self.straighten == 0.0
            && self.filter == Filter::Original
            && self.brightness == 0.0
            && self.contrast == 0.0
            && self.exposure == 0.0
            && self.saturation == 0.0
            && self.warmth == 0.0
            && self.blur == 0.0
            && self.vignette == 0.0
    }

    /// Turns a quarter clockwise (or back).
    pub fn rotate(&mut self, clockwise: bool) {
        self.turns = (self.turns + if clockwise { 1 } else { 3 }) % 4;
    }

    /// The size of the picture after the rotation.
    pub fn rotated_size(&self, w: usize, h: usize) -> (usize, usize) {
        let (w, h) = if self.turns % 2 == 1 { (h, w) } else { (w, h) };
        if self.straighten != 0.0 { straightened_size(w, h, self.straighten) } else { (w, h) }
    }

    /// The crop rectangle in the rotated picture.
    pub fn crop_rect(&self, w: usize, h: usize) -> (usize, usize, usize, usize) {
        let (w, h) = self.rotated_size(w, h);
        crop_rect(w, h, self.aspect, self.crop_zoom, self.crop_center)
    }

    /// The edited picture.
    pub fn apply(&self, img: &Image) -> Image {
        let mut out = rotate(img, self.turns);
        if self.straighten != 0.0 {
            out = straighten(&out, self.straighten);
        }
        let (x, y, w, h) = crop_rect(out.w, out.h, self.aspect, self.crop_zoom, self.crop_center);
        if (w, h) != (out.w, out.h) {
            out = out.crop(x, y, w, h);
        }
        self.filter.apply(&mut out);
        blur(&mut out, blur_radius(self.blur));
        adjust(&mut out, self.brightness, self.contrast);
        let mut looks = Vec::new();
        if self.exposure != 0.0 {
            looks.push(named("exposure", &[("ev", self.exposure * EXPOSURE_STOPS)]));
        }
        if self.saturation != 0.0 {
            looks.push(named("saturation", &[("amount", 1.0 + self.saturation)]));
        }
        if self.warmth != 0.0 {
            looks.push(named("temperatureTint", &[("temperature", self.warmth)]));
        }
        if self.vignette > 0.0 {
            looks.push(named("vignette", &[("strength", self.vignette)]));
        }
        tone(&mut out, looks);
        out
    }
}

/// The largest angle of the straighten slider, in degrees.
pub const STRAIGHTEN_MAX: f32 = 30.0;

/// The scale of the largest rectangle with the picture's own aspect that
/// stays inside the picture turned by `deg`.
fn straighten_scale(w: usize, h: usize, deg: f32) -> f32 {
    let t = deg.abs().to_radians();
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    let long = (w / h).max(h / w);
    1.0 / (t.cos() + t.sin() * long)
}

/// The size of the picture after `straighten`.
pub fn straightened_size(w: usize, h: usize, deg: f32) -> (usize, usize) {
    let s = straighten_scale(w, h, deg);
    (((w as f32 * s).round() as usize).clamp(1, w.max(1)), ((h as f32 * s).round() as usize).clamp(1, h.max(1)))
}

/// Turns the picture clockwise by `deg` about its centre and cuts the
/// largest same-aspect rectangle that holds no empty corner.
pub fn straighten(img: &Image, deg: f32) -> Image {
    if img.w == 0 || img.h == 0 {
        return img.clone();
    }
    let (ow, oh) = straightened_size(img.w, img.h, deg);
    let mut out = Image::new(ow, oh);
    let (sin, cos) = deg.to_radians().sin_cos();
    let (cx, cy, ocx, ocy) = (img.w as f32 / 2.0, img.h as f32 / 2.0, ow as f32 / 2.0, oh as f32 / 2.0);
    for y in 0..oh {
        for x in 0..ow {
            let (dx, dy) = (x as f32 + 0.5 - ocx, y as f32 + 0.5 - ocy);
            // inverse of a clockwise turn
            let sx = (cx + dx * cos + dy * sin).floor().clamp(0.0, img.w as f32 - 1.0) as usize;
            let sy = (cy - dx * sin + dy * cos).floor().clamp(0.0, img.h as f32 - 1.0) as usize;
            out.px[y * ow + x] = img.px[sy * img.w + sx];
        }
    }
    out
}

/// Rotates clockwise by `turns` quarter turns.
pub fn rotate(img: &Image, turns: u8) -> Image {
    let (w, h) = (img.w, img.h);
    match turns % 4 {
        0 => img.clone(),
        1 => {
            // (x, y) -> (h - 1 - y, x)
            let mut out = Image::new(h, w);
            for y in 0..h {
                for x in 0..w {
                    out.px[x * h + (h - 1 - y)] = img.px[y * w + x];
                }
            }
            out
        }
        2 => {
            let mut out = img.clone();
            out.px.reverse();
            out
        }
        _ => {
            // (x, y) -> (y, w - 1 - x)
            let mut out = Image::new(h, w);
            for y in 0..h {
                for x in 0..w {
                    out.px[(w - 1 - x) * h + y] = img.px[y * w + x];
                }
            }
            out
        }
    }
}

/// Mirrors left to right.
pub fn mirror(img: &Image) -> Image {
    let mut out = img.clone();
    for row in out.px.chunks_mut(img.w.max(1)) {
        row.reverse();
    }
    out
}

/// Turns a picture upright from its EXIF orientation (1..=8; anything else leaves it).
pub fn orient(img: Image, orientation: u16) -> Image {
    match orientation {
        2 => mirror(&img),
        3 => rotate(&img, 2),
        4 => rotate(&mirror(&img), 2),
        5 => rotate(&mirror(&img), 3),
        6 => rotate(&img, 1),
        7 => rotate(&mirror(&img), 1),
        8 => rotate(&img, 3),
        _ => img,
    }
}

/// The crop (x, y, w, h) of a `w` x `h` picture for `aspect`: the largest
/// rectangle of that ratio, shrunk by `zoom` and centred on `center`
/// (fractions of the picture) as far as it fits.
pub fn crop_rect(w: usize, h: usize, aspect: Aspect, zoom: f32, center: (f32, f32)) -> (usize, usize, usize, usize) {
    if w == 0 || h == 0 {
        return (0, 0, w, h);
    }
    let (bw, bh) = match aspect.ratio(w, h) {
        None => (w as f32, h as f32),
        Some(r) if w as f32 / h as f32 > r => (h as f32 * r, h as f32),
        Some(r) => (w as f32, w as f32 / r),
    };
    let z = zoom.max(1.0);
    let (cw, ch) = (((bw / z).round() as usize).clamp(1, w), ((bh / z).round() as usize).clamp(1, h));
    let cx = (center.0.clamp(0.0, 1.0) * w as f32 - cw as f32 / 2.0).round().clamp(0.0, (w - cw) as f32) as usize;
    let cy = (center.1.clamp(0.0, 1.0) * h as f32 - ch as f32 / 2.0).round().clamp(0.0, (h - ch) as f32) as usize;
    (cx, cy, cw, ch)
}

/// The lookup table for one channel: contrast about mid-grey, then brightness.
fn table(brightness: f32, contrast: f32) -> [u8; 256] {
    let k = 1.0 + contrast.clamp(-1.0, 1.0);
    let shift = brightness.clamp(-1.0, 1.0) * 255.0;
    let mut t = [0u8; 256];
    for (i, v) in t.iter_mut().enumerate() {
        *v = ((i as f32 - 127.5) * k + 127.5 + shift).round().clamp(0.0, 255.0) as u8;
    }
    t
}

/// Applies brightness and contrast (each -1..=1) in place.
pub fn adjust(img: &mut Image, brightness: f32, contrast: f32) {
    if brightness == 0.0 && contrast == 0.0 {
        return;
    }
    let t = table(brightness, contrast);
    for p in &mut img.px {
        let (r, g, b, a) = (*p as u8, (*p >> 8) as u8, (*p >> 16) as u8, (*p >> 24) as u8);
        *p = crate::gfx::pack(t[r as usize], t[g as usize], t[b as usize], a);
    }
}

/// The slider value for finger position `x` on a track from `x0` to `x1`
/// that spans `lo..=hi`.
pub fn slider_value(x: f32, x0: f32, x1: f32, lo: f32, hi: f32) -> f32 {
    if x1 <= x0 {
        return lo;
    }
    lo + ((x - x0) / (x1 - x0)).clamp(0.0, 1.0) * (hi - lo)
}

/// The knob position of `value` (the inverse of `slider_value`).
pub fn slider_pos(value: f32, x0: f32, x1: f32, lo: f32, hi: f32) -> f32 {
    if hi <= lo {
        return x0;
    }
    x0 + ((value - lo) / (hi - lo)).clamp(0.0, 1.0) * (x1 - x0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::pack;

    /// A w x h image whose pixel (x, y) has red = x and green = y.
    fn grid(w: usize, h: usize) -> Image {
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                img.px[y * w + x] = pack(x as u8, y as u8, 7, 255);
            }
        }
        img
    }

    fn at(img: &Image, x: usize, y: usize) -> (u8, u8) {
        let p = img.px[y * img.w + x];
        (p as u8, (p >> 8) as u8)
    }

    #[test]
    fn orientation_turns_a_picture_upright() {
        let img = grid(3, 2);
        assert_eq!(orient(img.clone(), 1), img);
        assert_eq!(at(&orient(img.clone(), 2), 0, 0), (2, 0), "mirrored");
        let six = orient(img.clone(), 6);
        assert_eq!((six.w, six.h), (2, 3));
        assert_eq!(at(&six, 1, 0), (0, 0), "6 turns a quarter clockwise");
        assert_eq!(orient(img.clone(), 3), rotate(&img, 2));
        assert_eq!(at(&orient(img.clone(), 8), 0, 2), (0, 0), "8 turns a quarter back");
        assert_eq!(at(&orient(img.clone(), 4), 0, 0), (0, 1), "4 flips top to bottom");
        let five = orient(img.clone(), 5);
        assert_eq!(at(&five, 0, 0), (0, 0), "5 transposes");
        assert_eq!(at(&five, 1, 2), (2, 1));
        let seven = orient(img.clone(), 7);
        assert_eq!(at(&seven, 1, 2), (0, 0), "7 anti-transposes");
        assert_eq!(orient(img.clone(), 9), img);
    }

    #[test]
    fn rotation_moves_corners_clockwise() {
        let img = grid(3, 2);
        let r = rotate(&img, 1);
        assert_eq!((r.w, r.h), (2, 3));
        assert_eq!(at(&r, 1, 0), (0, 0), "top-left goes to top-right");
        assert_eq!(at(&r, 1, 2), (2, 0), "top-right goes to bottom-right");
        assert_eq!(at(&r, 0, 0), (0, 1), "bottom-left goes to top-left");
        let half = rotate(&img, 2);
        assert_eq!(at(&half, 0, 0), (2, 1));
        assert_eq!(at(&half, 2, 1), (0, 0));
        let ccw = rotate(&img, 3);
        assert_eq!(at(&ccw, 0, 2), (0, 0), "top-left goes to bottom-left");
        assert_eq!(at(&ccw, 0, 0), (2, 0));
        assert_eq!(rotate(&rotate(&img, 1), 3), img);
        assert_eq!(rotate(&img, 4), img);
        let mut e = Edit::default();
        e.rotate(false);
        assert_eq!(e.turns, 3);
        e.rotate(true);
        e.rotate(true);
        assert_eq!(e.turns, 1);
        assert_eq!(e.rotated_size(3, 2), (2, 3));
    }

    #[test]
    fn crop_rects_fit_the_aspect_inside_the_picture() {
        assert_eq!(crop_rect(400, 300, Aspect::Free, 1.0, (0.5, 0.5)), (0, 0, 400, 300));
        assert_eq!(crop_rect(400, 300, Aspect::Original, 1.0, (0.5, 0.5)), (0, 0, 400, 300));
        assert_eq!(crop_rect(400, 300, Aspect::Square, 1.0, (0.5, 0.5)), (50, 0, 300, 300));
        assert_eq!(crop_rect(400, 300, Aspect::R16x9, 1.0, (0.5, 0.5)), (0, 38, 400, 225));
        assert_eq!(crop_rect(400, 300, Aspect::R4x3, 1.0, (0.5, 0.5)), (0, 0, 400, 300));
        // portrait pictures flip the preset
        assert_eq!(crop_rect(300, 400, Aspect::R4x3, 1.0, (0.5, 0.5)), (0, 0, 300, 400));
        assert_eq!(crop_rect(300, 400, Aspect::R16x9, 1.0, (0.5, 0.5)), (38, 0, 225, 400));
        // zoom shrinks it and the centre moves it, clamped inside
        assert_eq!(crop_rect(400, 300, Aspect::Square, 2.0, (0.5, 0.5)), (125, 75, 150, 150));
        assert_eq!(crop_rect(400, 300, Aspect::Square, 2.0, (0.0, 0.0)), (0, 0, 150, 150));
        assert_eq!(crop_rect(400, 300, Aspect::Square, 2.0, (1.0, 1.0)), (250, 150, 150, 150));
        assert_eq!(crop_rect(400, 300, Aspect::Square, 0.5, (0.5, 0.5)), (50, 0, 300, 300), "zoom below 1 is 1");
        assert_eq!(crop_rect(0, 0, Aspect::Square, 1.0, (0.5, 0.5)), (0, 0, 0, 0));
        assert_eq!(Aspect::ALL.len(), 6);
        assert_eq!(Aspect::R3x2.label(), "3:2");
    }

    #[test]
    fn brightness_and_contrast_tables() {
        let t = table(0.0, 0.0);
        assert!((0..256).all(|i| t[i] == i as u8), "identity");
        let up = table(0.2, 0.0);
        assert_eq!((up[0], up[100], up[250]), (51, 151, 255));
        let down = table(-0.2, 0.0);
        assert_eq!((down[0], down[100], down[30]), (0, 49, 0));
        let hi = table(0.0, 0.5);
        assert!(hi[200] > 200 && hi[50] < 50, "contrast spreads from the middle");
        assert!((hi[127] as i32 - 127).abs() <= 1 && (hi[128] as i32 - 128).abs() <= 1);
        let flat = table(0.0, -1.0);
        assert!(flat.iter().all(|&v| (127..=128).contains(&v)), "full negative contrast is grey");
        let mut img = Image { w: 1, h: 1, px: vec![pack(100, 150, 200, 255)] };
        adjust(&mut img, 0.0, 0.0);
        assert_eq!(img.px[0], pack(100, 150, 200, 255));
        adjust(&mut img, 0.2, 0.0);
        assert_eq!(img.px[0], pack(151, 201, 251, 255), "alpha kept");
    }

    #[test]
    fn apply_rotates_then_crops_then_adjusts() {
        let img = grid(8, 4);
        let identity = Edit::default();
        assert!(identity.is_identity());
        assert_eq!(identity.apply(&img), img);
        let e = Edit { turns: 1, aspect: Aspect::Square, ..Edit::default() };
        assert!(!e.is_identity());
        assert_eq!(e.crop_rect(8, 4), (0, 2, 4, 4), "rotated picture is 4 x 8; square crop centred");
        let out = e.apply(&img);
        assert_eq!((out.w, out.h), (4, 4));
        // rotated (x', y') = (3 - y, x); the crop starts at y' = 2, i.e. source x = 2
        assert_eq!(at(&out, 0, 0), (2, 3));
        let bright = Edit { brightness: 0.2, ..Edit::default() }.apply(&img);
        assert_eq!(at(&bright, 0, 0), (51, 51));
    }

    #[test]
    fn straighten_keeps_the_aspect_and_filters_recolour() {
        assert_eq!(straightened_size(400, 300, 0.0), (400, 300));
        let (w, h) = straightened_size(400, 300, 10.0);
        assert!(w < 400 && h < 300 && (w as f32 / h as f32 - 4.0 / 3.0).abs() < 0.02);
        let img = grid(8, 4);
        assert_eq!(straighten(&img, 0.0).w, 8);
        let e = Edit { straighten: 5.0, ..Edit::default() };
        assert!(!e.is_identity());
        let out = e.apply(&img);
        assert_eq!((out.w, out.h), e.rotated_size(8, 4));
        let mut px = Image { w: 1, h: 1, px: vec![pack(200, 100, 50, 255)] };
        Filter::Mono.apply(&mut px);
        let p = px.px[0];
        assert!(p as u8 == (p >> 8) as u8 && (p >> 8) as u8 == (p >> 16) as u8 && (p >> 24) as u8 == 255);
        let mut warm = Image { w: 1, h: 1, px: vec![pack(100, 100, 100, 255)] };
        Filter::Warm.apply(&mut warm);
        assert!(warm.px[0] as u8 > (warm.px[0] >> 16) as u8, "warm pushes red over blue");
        assert_eq!(Filter::ALL.len(), 6);
        assert!(!Edit { filter: Filter::Vivid, ..Edit::default() }.is_identity());
    }

    #[test]
    fn sliders_map_between_pixels_and_values() {
        assert_eq!(slider_value(100.0, 100.0, 300.0, -1.0, 1.0), -1.0);
        assert_eq!(slider_value(200.0, 100.0, 300.0, -1.0, 1.0), 0.0);
        assert_eq!(slider_value(900.0, 100.0, 300.0, -1.0, 1.0), 1.0, "clamped");
        assert_eq!(slider_value(0.0, 100.0, 300.0, 1.0, 4.0), 1.0);
        assert_eq!(slider_value(5.0, 10.0, 10.0, 0.0, 1.0), 0.0, "empty track");
        assert_eq!(slider_pos(0.0, 100.0, 300.0, -1.0, 1.0), 200.0);
        assert_eq!(slider_pos(4.0, 100.0, 300.0, 1.0, 4.0), 300.0);
        assert_eq!(slider_pos(slider_value(160.0, 100.0, 300.0, -1.0, 1.0), 100.0, 300.0, -1.0, 1.0), 160.0);
        assert!(!Edit { crop_zoom: 2.0, ..Edit::default() }.is_identity());
    }


    #[test]
    fn exposure_saturation_and_warmth_use_the_catalogue_maths() {
        // one stop up doubles 64 to 128 (64/255 * 2 * 255 = 128.0)
        let up = Edit { exposure: 0.5, ..Edit::default() }.apply(&Image { w: 1, h: 1, px: vec![pack(64, 64, 64, 255)] });
        assert_eq!(up.px[0], pack(128, 128, 128, 255));
        // saturation -1 is amount 0: red becomes its luma, 0.299 * 255 = 76.2, rounds to 76
        let grey = Edit { saturation: -1.0, ..Edit::default() }.apply(&Image { w: 1, h: 1, px: vec![pack(255, 0, 0, 255)] });
        assert_eq!(grey.px[0], pack(76, 76, 76, 255));
        // warmth 1 is temperature 1: red up by a quarter, blue down by a quarter
        let warm = Edit { warmth: 1.0, ..Edit::default() }.apply(&Image { w: 1, h: 1, px: vec![pack(100, 100, 100, 255)] });
        assert_eq!(warm.px[0], pack(125, 100, 75, 255));
        assert!(!Edit { warmth: 0.1, ..Edit::default() }.is_identity());
        assert!(!Edit { blur: 0.1, ..Edit::default() }.is_identity());
        assert!(!Edit { vignette: 0.1, ..Edit::default() }.is_identity());
        assert!(!Edit { exposure: -0.1, ..Edit::default() }.is_identity());
        assert!(!Edit { saturation: 0.1, ..Edit::default() }.is_identity());
    }

    #[test]
    fn blur_radius_steps_and_the_disc_average() {
        assert_eq!(blur_radius(0.0), 0);
        assert_eq!(blur_radius(0.5), 2, "1.5 rounds up");
        assert_eq!(blur_radius(1.0), 3);
        assert_eq!(blur_radius(3.0), 3, "clamped");
        // radius 1 is the centre and its four neighbours: 255 in five pixels, so the centre is 255 / 5 = 51
        let mut img = Image { w: 3, h: 3, px: vec![pack(0, 0, 0, 255); 9] };
        img.px[4] = pack(255, 255, 255, 255);
        blur(&mut img, 1);
        assert_eq!(img.px[4], pack(51, 51, 51, 255));
        assert_eq!(img.px[4] >> 24, 255, "alpha stays opaque");
        let mut keep = Image { w: 1, h: 1, px: vec![pack(9, 9, 9, 255)] };
        blur(&mut keep, 0);
        assert_eq!(keep.px[0], pack(9, 9, 9, 255));
    }

    #[test]
    fn vignette_darkens_the_corners_not_the_centre() {
        let white = pack(255, 255, 255, 255);
        let out = Edit { vignette: 1.0, ..Edit::default() }.apply(&Image { w: 4, h: 4, px: vec![white; 16] });
        assert_eq!(out.px[0] & 0x00ff_ffff, 0, "the corner at strength 1 is black");
        assert_eq!(out.px[2 * 4 + 2], white, "the centre is untouched");
        assert_eq!(out.px[0] >> 24, 255, "alpha kept");
    }

    #[test]
    fn sepia_turns_white_into_the_warm_tone() {
        // the sepia blue row for white is 0.937, so 0.937 * 255 = 238.9 rounds to 239
        let out = Edit { filter: Filter::Sepia, ..Edit::default() }.apply(&Image { w: 1, h: 1, px: vec![pack(255, 255, 255, 255)] });
        assert_eq!(out.px[0], pack(255, 255, 239, 255));
        assert!(!Edit { filter: Filter::Sepia, ..Edit::default() }.is_identity());
        assert_eq!(Filter::Sepia.label(), "Sepia");
    }
}
