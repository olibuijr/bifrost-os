//! Transition filters between two images of the same size, with the parameters of the reference
//! filters `CISwipeTransition`, `CIBarsSwipeTransition` and `CICopyMachineTransition`. `time` runs
//! from 0 (the result is `from`) to 1 (the result is `to`); values outside are clamped. The sweep
//! direction is `angle` in radians: 0 sweeps left to right, and the sweep moves in the direction
//! of (cos angle, sin angle) in image space, which has y pointing down.
//! Pixels are packed like `gfx::Image` (red in the low byte). A transition only selects between the
//! two pixels, except the copy machine, which brightens the edge of the revealed region.
//! Differences from the reference: the bars swipe alternates the direction of each bar instead of
//! using the reference bar timing, and the copy machine's light band is linear in distance.
use crate::colorfx::{self, Rgb};
use crate::gfx::Image;

/// The sweep frame of a `w` by `h` image turned by `angle`: the direction (dx, dy), the centre, and
/// the length of the sweep and of the line across it that cover the image.
struct Sweep {
    dx: f32,
    dy: f32,
    cx: f32,
    cy: f32,
    along_len: f32,
    across_len: f32,
}

impl Sweep {
    fn new(w: usize, h: usize, angle: f32) -> Sweep {
        let (s, c) = angle.sin_cos();
        let (dx, dy) = (c, s);
        Sweep {
            dx,
            dy,
            cx: w as f32 * 0.5,
            cy: h as f32 * 0.5,
            along_len: w as f32 * dx.abs() + h as f32 * dy.abs(),
            across_len: w as f32 * dy.abs() + h as f32 * dx.abs(),
        }
    }

    /// Distance of pixel (x, y) along the sweep, from 0 at the leading edge to `along_len` at the far one.
    fn along(&self, x: usize, y: usize) -> f32 {
        (x as f32 + 0.5 - self.cx) * self.dx + (y as f32 + 0.5 - self.cy) * self.dy + self.along_len * 0.5
    }

    /// Distance of pixel (x, y) across the sweep, from 0 to `across_len`.
    fn across(&self, x: usize, y: usize) -> f32 {
        (x as f32 + 0.5 - self.cx) * -self.dy + (y as f32 + 0.5 - self.cy) * self.dx + self.across_len * 0.5
    }
}

fn time_of(time: f32) -> f32 {
    if time.is_finite() { time.clamp(0.0, 1.0) } else { 0.0 }
}

/// The pixel count shared by the two images, or `None` when their sizes differ or are empty.
fn same_size(from: &Image, to: &Image) -> Option<usize> {
    let n = from.w.checked_mul(from.h)?;
    (n > 0 && from.w == to.w && from.h == to.h && from.px.len() == n && to.px.len() == n).then_some(n)
}

/// Builds the output by asking `pick` for each pixel, given its index and the two source pixels.
fn select(from: &Image, to: &Image, n: usize, mut pick: impl FnMut(usize, u32, u32) -> u32) -> Image {
    let mut out = Image::new(from.w, from.h);
    for i in 0..n {
        out.px[i] = pick(i, from.px[i], to.px[i]);
    }
    out
}

/// `CISwipeTransition`: a straight edge sweeps across the image, showing `to` behind it and `from`
/// ahead of it. `None` when the images differ in size or `angle` is not finite.
pub fn swipe(from: &Image, to: &Image, time: f32, angle: f32) -> Option<Image> {
    let n = same_size(from, to)?;
    if !angle.is_finite() {
        return None;
    }
    let sw = Sweep::new(from.w, from.h, angle);
    let front = time_of(time) * sw.along_len;
    Some(select(from, to, n, |i, a, b| {
        let (x, y) = (i % from.w, i / from.w);
        if sw.along(x, y) < front { b } else { a }
    }))
}

/// `CIBarsSwipeTransition`: the image is cut into bars of `bar_width` pixels across the sweep, and each
/// bar is swept in turn: even bars from the leading edge, odd bars from the trailing edge. `None` when
/// the sizes differ, `angle` is not finite, or `bar_width` is not a positive finite number.
pub fn bars_swipe(from: &Image, to: &Image, time: f32, angle: f32, bar_width: f32) -> Option<Image> {
    let n = same_size(from, to)?;
    if !angle.is_finite() || !bar_width.is_finite() || bar_width <= 0.0 {
        return None;
    }
    let sw = Sweep::new(from.w, from.h, angle);
    let front = time_of(time) * sw.along_len;
    Some(select(from, to, n, |i, a, b| {
        let (x, y) = (i % from.w, i / from.w);
        let bar = (sw.across(x, y) / bar_width).floor() as i64;
        let reveal = if bar.rem_euclid(2) == 0 {
            sw.along(x, y) < front
        } else {
            sw.along(x, y) > sw.along_len - front
        };
        if reveal { b } else { a }
    }))
}

/// `CICopyMachineTransition`: like the swipe, but the edge of the revealed `to` region carries a light
/// band `band` pixels wide that brightens towards white at the sweep front. The band fades out as
/// `time` reaches 1, so the last frame is exactly `to`. `None` when the sizes differ, `angle` is not
/// finite, or `band` is negative or not finite.
pub fn copy_machine(from: &Image, to: &Image, time: f32, angle: f32, band: f32) -> Option<Image> {
    let n = same_size(from, to)?;
    if !angle.is_finite() || !band.is_finite() || band < 0.0 {
        return None;
    }
    let t = time_of(time);
    let sw = Sweep::new(from.w, from.h, angle);
    let front = t * sw.along_len;
    let white = Rgb::new(1.0, 1.0, 1.0);
    Some(select(from, to, n, |i, a, b| {
        let (x, y) = (i % from.w, i / from.w);
        let s = sw.along(x, y);
        if s >= front {
            return a;
        }
        let lead = front - s;
        if band <= 0.0 || lead >= band {
            return b;
        }
        let glow = (1.0 - lead / band) * (1.0 - t);
        colorfx::map_pixel(b, |c| c.mix(white, glow))
    }))
}

/// `CIFlashTransition`: a flash of light centred on `center` (pixels). In the first half of the
/// transition the picture brightens towards white at the centre and the glow fades with distance
/// (to nothing at the far corner), peaking at time 0.5; in the second half the picture switches to
/// `to`, still flashed, and the glow dies away. `None` when the sizes differ or `center` is not finite.
pub fn flash(from: &Image, to: &Image, time: f32, center: (f32, f32)) -> Option<Image> {
    let n = same_size(from, to)?;
    if !center.0.is_finite() || !center.1.is_finite() {
        return None;
    }
    let t = time_of(time);
    let reach = ((from.w * from.w + from.h * from.h) as f32).sqrt();
    let strength = 1.0 - (2.0 * t - 1.0).abs();
    let white = Rgb::new(1.0, 1.0, 1.0);
    Some(select(from, to, n, |i, a, b| {
        let (x, y) = (i % from.w, i / from.w);
        let d = ((x as f32 + 0.5 - center.0).powi(2) + (y as f32 + 0.5 - center.1).powi(2)).sqrt();
        let glow = strength * (1.0 - d / reach).max(0.0);
        let px = if t < 0.5 { a } else { b };
        if glow <= 0.0 { px } else { colorfx::map_pixel(px, |c| c.mix(white, glow)) }
    }))
}

/// `CIModTransition`: a radial reveal of `to` from `center` (pixels) whose front is modulated by
/// `lobes` bumps around the centre, so it spreads as a flower rather than a circle. A pixel shows `to`
/// once its distance over the far-corner distance is below `time * (1.25 + 0.25 * cos(lobes * theta))`.
/// Time 0 shows `from` everywhere and time 1 shows `to` everywhere. `None` when the sizes differ or
/// `center` is not finite. A `lobes` of 0 is treated as 1.
pub fn mod_reveal(from: &Image, to: &Image, time: f32, center: (f32, f32), lobes: u32) -> Option<Image> {
    let n = same_size(from, to)?;
    if !center.0.is_finite() || !center.1.is_finite() {
        return None;
    }
    let t = time_of(time);
    let reach = ((from.w * from.w + from.h * from.h) as f32).sqrt();
    let k = lobes.max(1) as f32;
    Some(select(from, to, n, |i, a, b| {
        let (x, y) = (i % from.w, i / from.w);
        let (dx, dy) = (x as f32 + 0.5 - center.0, y as f32 + 0.5 - center.1);
        let d = (dx * dx + dy * dy).sqrt();
        let theta = dy.atan2(dx);
        let front = t * (1.25 + 0.25 * (k * theta).cos());
        if d / reach < front { b } else { a }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: usize, h: usize, px: u32) -> Image {
        let mut img = Image::new(w, h);
        img.px.iter_mut().for_each(|p| *p = px);
        img
    }

    const BLACK: u32 = 0xff00_0000;
    const GREY: u32 = 0xff80_8080;

    #[test]
    fn swipe_ends_are_exactly_from_and_to() {
        let (a, b) = (flat(4, 3, BLACK), flat(4, 3, GREY));
        assert_eq!(swipe(&a, &b, 0.0, 0.0).unwrap(), a);
        assert_eq!(swipe(&a, &b, 1.0, 0.0).unwrap(), b);
        assert_eq!(swipe(&a, &b, 1.0, 1.0).unwrap(), b, "a diagonal sweep also ends on to");
    }

    #[test]
    fn swipe_at_half_time_splits_a_row_at_the_middle() {
        let (a, b) = (flat(4, 1, BLACK), flat(4, 1, GREY));
        let out = swipe(&a, &b, 0.5, 0.0).unwrap();
        assert_eq!(out.px, vec![GREY, GREY, BLACK, BLACK]);
    }

    #[test]
    fn swipe_rejects_size_mismatch_and_non_finite_angles() {
        let a = flat(4, 4, BLACK);
        assert!(swipe(&a, &flat(4, 3, GREY), 0.5, 0.0).is_none());
        assert!(swipe(&a, &a, 0.5, f32::NAN).is_none());
    }

    #[test]
    fn time_outside_the_range_is_clamped() {
        let (a, b) = (flat(4, 1, BLACK), flat(4, 1, GREY));
        assert_eq!(swipe(&a, &b, 7.0, 0.0).unwrap(), b);
        assert_eq!(swipe(&a, &b, -1.0, 0.0).unwrap(), a);
        assert_eq!(swipe(&a, &b, f32::NAN, 0.0).unwrap(), a);
    }

    #[test]
    fn bars_alternate_the_direction_of_each_row() {
        let (a, b) = (flat(4, 2, BLACK), flat(4, 2, GREY));
        let out = bars_swipe(&a, &b, 0.5, 0.0, 1.0).unwrap();
        // Row 0 is an even bar: revealed from the left. Row 1 is odd: revealed from the right.
        assert_eq!(&out.px[0..4], &[GREY, GREY, BLACK, BLACK]);
        assert_eq!(&out.px[4..8], &[BLACK, BLACK, GREY, GREY]);
    }

    #[test]
    fn bars_swipe_ends_and_rejects_bad_widths() {
        let (a, b) = (flat(4, 4, BLACK), flat(4, 4, GREY));
        assert_eq!(bars_swipe(&a, &b, 0.0, 0.0, 2.0).unwrap(), a);
        assert_eq!(bars_swipe(&a, &b, 1.0, 0.0, 2.0).unwrap(), b);
        assert!(bars_swipe(&a, &b, 0.5, 0.0, 0.0).is_none());
        assert!(bars_swipe(&a, &b, 0.5, 0.0, f32::INFINITY).is_none());
    }

    #[test]
    fn copy_machine_without_a_band_is_a_plain_swipe() {
        let (a, b) = (flat(8, 1, BLACK), flat(8, 1, GREY));
        assert_eq!(copy_machine(&a, &b, 0.5, 0.0, 0.0), swipe(&a, &b, 0.5, 0.0));
    }

    #[test]
    fn copy_machine_brightens_the_front_and_ends_on_to() {
        let (a, b) = (flat(8, 1, BLACK), flat(8, 1, GREY));
        let mid = copy_machine(&a, &b, 0.5, 0.0, 2.0).unwrap();
        // The front is at x = 4. Pixel 3 is one pixel behind it, inside the band, so it is brighter
        // than `to`; pixel 0 is far behind the band and keeps `to`'s grey.
        assert!(mid.px[3] & 0xff > 0x80, "front pixel should be brighter than to");
        assert_eq!(mid.px[0], GREY);
        assert_eq!(mid.px[7], BLACK);
        assert_eq!(copy_machine(&a, &b, 1.0, 0.0, 2.0).unwrap(), b);
        assert_eq!(copy_machine(&a, &b, 0.0, 0.0, 2.0).unwrap(), a);
    }

    #[test]
    fn flash_starts_and_ends_on_the_pictures_and_peaks_at_the_centre() {
        let (a, b) = (flat(8, 8, BLACK), flat(8, 8, GREY));
        assert_eq!(flash(&a, &b, 0.0, (4.0, 4.0)).unwrap(), a);
        assert_eq!(flash(&a, &b, 1.0, (4.0, 4.0)).unwrap(), b);
        let mid = flash(&a, &b, 0.5, (4.0, 4.0)).unwrap();
        assert!(mid.px[4 * 8 + 4] & 0xff > 0xe0, "the centre is nearly white at the peak");
        assert!(mid.px[4 * 8 + 4] & 0xff > mid.px[0] & 0xff, "the glow fades with distance");
    }

    #[test]
    fn mod_reveals_in_flower_lobes_and_ends_on_to() {
        let (a, b) = (flat(9, 9, BLACK), flat(9, 9, GREY));
        assert_eq!(mod_reveal(&a, &b, 0.0, (4.0, 4.0), 1).unwrap(), a);
        assert_eq!(mod_reveal(&a, &b, 1.0, (4.0, 4.0), 1).unwrap(), b);
        let mid = mod_reveal(&a, &b, 0.25, (4.0, 4.0), 1).unwrap();
        // One lobe: the front reaches further along +x than along -x, so (8, 4) is revealed and (0, 4) is not.
        assert_eq!(mid.px[4 * 9 + 8], GREY);
        assert_eq!(mid.px[4 * 9], BLACK);
    }

    #[test]
    fn flash_rejects_a_non_finite_centre() {
        let a = flat(4, 4, BLACK);
        assert!(flash(&a, &a, 0.5, (f32::NAN, 0.0)).is_none());
    }

    #[test]
    fn copy_machine_rejects_a_negative_band() {
        let a = flat(4, 1, BLACK);
        assert!(copy_machine(&a, &a, 0.5, 0.0, -1.0).is_none());
    }
}
