//! The edit mode's pixel work: rotate by quarter turns, crop to an aspect
//! preset, brightness and contrast, and saving the result as a copy. Photos
//! are opaque, so the colour ops treat the packed pixels as straight RGB
//! and leave alpha alone.

use crate::gfx::Image;
use std::path::{Path, PathBuf};

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
}

impl Filter {
    pub const ALL: [Filter; 5] = [Filter::Original, Filter::Vivid, Filter::Mono, Filter::Noir, Filter::Warm];

    pub fn label(self) -> &'static str {
        match self {
            Filter::Original => "Original",
            Filter::Vivid => "Vivid",
            Filter::Mono => "Mono",
            Filter::Noir => "Noir",
            Filter::Warm => "Warm",
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
        }
    }

    pub fn apply(self, img: &mut Image) {
        if self == Filter::Original {
            return;
        }
        for p in &mut img.px {
            let (r, g, b) = self.pixel(*p as u8, (*p >> 8) as u8, (*p >> 16) as u8);
            *p = crate::gfx::pack(r, g, b, (*p >> 24) as u8);
        }
    }
}

/// Everything the edit mode changes, applied in this order: rotate,
/// straighten, crop, filter, brightness and contrast.
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
}

impl Default for Edit {
    fn default() -> Self {
        Edit { turns: 0, aspect: Aspect::Free, crop_zoom: 1.0, crop_center: (0.5, 0.5), straighten: 0.0, filter: Filter::Original, brightness: 0.0, contrast: 0.0 }
    }
}

impl Edit {
    pub fn is_identity(&self) -> bool {
        self.turns % 4 == 0 && self.aspect == Aspect::Free && self.crop_zoom <= 1.0 && self.straighten == 0.0 && self.filter == Filter::Original && self.brightness == 0.0 && self.contrast == 0.0
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
        adjust(&mut out, self.brightness, self.contrast);
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

/// Where a copy of `original` goes: "IMG_1 edited.png" next to it, numbered
/// ("IMG_1 edited 2.png") while the name is taken.
pub fn copy_path(original: &Path, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let stem = original.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Photo".into());
    let dir = original.parent().unwrap_or(Path::new("."));
    let mut path = dir.join(format!("{stem} edited.png"));
    let mut n = 2;
    while taken(&path) {
        path = dir.join(format!("{stem} edited {n}.png"));
        n += 1;
    }
    path
}

/// Writes `img` as a PNG copy beside `original`; returns the new path.
pub fn save_copy(original: &Path, img: &Image) -> std::io::Result<PathBuf> {
    let path = copy_path(original, |p| p.exists());
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, crate::pngenc::encode_rgb(img.w, img.h, &img.px))?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
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
        assert_eq!(Filter::ALL.len(), 5);
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
    fn copies_never_overwrite_the_original_or_each_other() {
        let o = Path::new("/p/IMG_1.jpg");
        assert_eq!(copy_path(o, |_| false), Path::new("/p/IMG_1 edited.png"));
        assert_eq!(copy_path(o, |p| p.ends_with("IMG_1 edited.png")), Path::new("/p/IMG_1 edited 2.png"));
        let dir = super::super::library::tests::scratch("edit-copy");
        let orig = dir.join("a.jpg");
        std::fs::write(&orig, b"x").unwrap();
        let img = grid(4, 2);
        let first = save_copy(&orig, &img).unwrap();
        let second = save_copy(&orig, &img).unwrap();
        assert_eq!(first, dir.join("a edited.png"));
        assert_eq!(second, dir.join("a edited 2.png"));
        assert_eq!(std::fs::read(&orig).unwrap(), b"x");
        let back = crate::imgdec::decode(&std::fs::read(&first).unwrap()).unwrap();
        assert_eq!((back.w, back.h), (4, 2));
        assert_eq!(back.px[5], img.px[5]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
