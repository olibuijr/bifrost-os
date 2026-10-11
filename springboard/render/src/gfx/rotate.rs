//! Quarter turns between the shell's logical frame and the portrait panel.
//! The panel is physically 1200x1920; in landscape the shell lays out and
//! draws a 1920x1200 frame and the presenter turns it into the panel buffer.
//! This module holds the orientations, the pixel-exact 90°/180° rotation of
//! damaged areas (16x16 cache tiles of SIMD 4x4 transposes shared out over threads), the matching
//! damage-rectangle and touch-point transforms, a bilinear resample for
//! letterboxed portrait apps and the frame of the rotation animation.

use super::{scale, threads, Area, Damage, Image, Surface};

/// How the device is held. `Portrait` is the panel's native orientation;
/// `LandscapeLeft` is the device turned counter-clockwise (its top edge on
/// the left), as bifrost names it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Orientation {
    #[default]
    Portrait,
    LandscapeLeft,
    LandscapeRight,
    PortraitUpsideDown,
}

/// A quarter turn applied to an image, as seen on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rotation {
    None,
    /// 90° counter-clockwise: the top row becomes the left column
    Ccw,
    /// 90° clockwise: the top row becomes the right column
    Cw,
    /// 180°
    Half,
}

impl Orientation {
    pub const ALL: [Orientation; 4] = [Orientation::Portrait, Orientation::LandscapeLeft, Orientation::LandscapeRight, Orientation::PortraitUpsideDown];

    /// The turn that takes the logical frame into the panel. A device turned
    /// counter-clockwise has its right edge on top, so the frame's top row goes to the
    /// panel's right column: the content turns clockwise on the panel, and the other way
    /// round for a device turned clockwise.
    ///
    /// ORIENTATION -> PANEL ROTATION TABLE: if the picture is upside down on the device in
    /// landscape, swap `Cw` and `Ccw` in the two landscape lines below (and nowhere else).
    pub fn rotation(self) -> Rotation {
        match self {
            Orientation::Portrait => Rotation::None,
            Orientation::LandscapeLeft => Rotation::Cw,
            Orientation::LandscapeRight => Rotation::Ccw,
            Orientation::PortraitUpsideDown => Rotation::Half,
        }
    }

    pub fn is_landscape(self) -> bool {
        matches!(self, Orientation::LandscapeLeft | Orientation::LandscapeRight)
    }

    /// Logical frame size for a `pw` x `ph` panel.
    pub fn size(self, pw: i32, ph: i32) -> (i32, i32) {
        if self.is_landscape() {
            (ph, pw)
        } else {
            (pw, ph)
        }
    }

    /// A touch at panel pixel (x, y) in logical-frame coordinates.
    pub fn to_logical(self, x: f32, y: f32, pw: i32, ph: i32) -> (f32, f32) {
        match self.rotation() {
            Rotation::None => (x, y),
            Rotation::Ccw => (ph as f32 - 1.0 - y, x),
            Rotation::Cw => (y, pw as f32 - 1.0 - x),
            Rotation::Half => (pw as f32 - 1.0 - x, ph as f32 - 1.0 - y),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Orientation::Portrait => "portrait",
            Orientation::LandscapeLeft => "landscape-left",
            Orientation::LandscapeRight => "landscape-right",
            Orientation::PortraitUpsideDown => "upside-down",
        }
    }
}

impl Rotation {
    /// Clockwise quarter turns, 0..4.
    fn quarters(self) -> u8 {
        match self {
            Rotation::None => 0,
            Rotation::Cw => 1,
            Rotation::Half => 2,
            Rotation::Ccw => 3,
        }
    }

    fn from_quarters(q: u8) -> Rotation {
        match q % 4 {
            0 => Rotation::None,
            1 => Rotation::Cw,
            2 => Rotation::Half,
            _ => Rotation::Ccw,
        }
    }

    pub fn inverse(self) -> Rotation {
        Rotation::from_quarters(4 - self.quarters())
    }

    /// `self` followed by `next`.
    pub fn then(self, next: Rotation) -> Rotation {
        Rotation::from_quarters(self.quarters() + next.quarters())
    }

    /// The signed angle in radians, clockwise positive in screen coordinates.
    pub fn angle(self) -> f32 {
        match self {
            Rotation::None => 0.0,
            Rotation::Cw => std::f32::consts::FRAC_PI_2,
            Rotation::Ccw => -std::f32::consts::FRAC_PI_2,
            Rotation::Half => std::f32::consts::PI,
        }
    }

    /// Size of a `w` x `h` image after the turn.
    pub fn size(self, w: usize, h: usize) -> (usize, usize) {
        match self {
            Rotation::None | Rotation::Half => (w, h),
            Rotation::Cw | Rotation::Ccw => (h, w),
        }
    }

    /// Where a half-open area of a `w` x `h` image lands after the turn.
    pub fn area(self, a: Area, w: usize, h: usize) -> Area {
        match self {
            Rotation::None => a,
            Rotation::Ccw => Area { x0: a.y0, y0: w - a.x1, x1: a.y1, y1: w - a.x0 },
            Rotation::Cw => Area { x0: h - a.y1, y0: a.x0, x1: h - a.y0, y1: a.x1 },
            Rotation::Half => Area { x0: w - a.x1, y0: h - a.y1, x1: w - a.x0, y1: h - a.y0 },
        }
    }
}

/// The damage of a `w` x `h` frame as the panel sees it after the turn.
pub fn rotate_damage(d: &Damage, rot: Rotation, w: usize, h: usize) -> Damage {
    let (rw, rh) = rot.size(w, h);
    let mut out = Damage::new(rw, rh);
    for a in d.areas() {
        let r = rot.area(*a, w, h);
        out.add(r.x0 as i32, r.y0 as i32, (r.x1 - r.x0) as i32, (r.y1 - r.y0) as i32);
    }
    out
}

/// The whole image turned.
pub fn rotated(img: &Image, rot: Rotation) -> Image {
    let (w, h) = rot.size(img.w, img.h);
    let mut out = Image::new(w, h);
    let all = Area { x0: 0, y0: 0, x1: img.w, y1: img.h };
    rotate_areas(&img.px, img.w, img.h, &mut out.px, rot, &[all], 1);
    out
}

/// Edge of the square tile a quarter turn is cut into: 16 pixels are one 64-byte cache line, so
/// every line the tile touches is used whole before it is evicted.
const BLOCK: usize = 16;
const MIN_ROWS_PER_THREAD: usize = 64;

/// The destination of a turn, shared with the pool's threads, which fill disjoint row bands.
#[derive(Clone, Copy)]
struct Out(*mut u32);
// SAFETY: every band is a disjoint range of rows of one live slice.
unsafe impl Send for Out {}
unsafe impl Sync for Out {}

impl Out {
    fn ptr(&self) -> *mut u32 {
        self.0
    }
}

/// Copies `areas` of the `sw` x `sh` frame `src` into `dst`, turned by
/// `rot`; `dst` is the turned frame (`rot.size(sw, sh)`, tightly packed).
/// Each area's destination rows are shared out over up to `n` threads (never more than the
/// cpus the render pool may use).
pub fn rotate_areas(src: &[u32], sw: usize, sh: usize, dst: &mut [u32], rot: Rotation, areas: &[Area], n: usize) {
    rotate_areas_on(src, sw, sh, dst, rot, areas, n.min(super::affinity::pool_cpus()));
}

fn rotate_areas_on(src: &[u32], sw: usize, sh: usize, dst: &mut [u32], rot: Rotation, areas: &[Area], n: usize) {
    let (dw, _) = rot.size(sw, sh);
    for &a in areas {
        if a.x0 >= a.x1 || a.y0 >= a.y1 {
            continue;
        }
        let d = rot.area(a, sw, sh);
        let rows = d.y1 - d.y0;
        let n = n.clamp(1, (rows / MIN_ROWS_PER_THREAD).max(1));
        let region = &mut dst[d.y0 * dw..d.y1 * dw];
        if n == 1 {
            rotate_rows(src, sw, sh, region, d.y0, d.y1, dw, rot, a);
            continue;
        }
        // two bands per thread, dealt out dynamically, each a whole number of tiles tall
        let per = rows.div_ceil(n * 2).next_multiple_of(BLOCK);
        let bands = rows.div_ceil(per);
        let out = Out(region.as_mut_ptr());
        super::pool::run(bands, &|k| {
            let (top, bottom) = (d.y0 + k * per, (d.y0 + (k + 1) * per).min(d.y1));
            // SAFETY: bands are disjoint row ranges inside `region`, which outlives the call.
            let mine = unsafe { std::slice::from_raw_parts_mut(out.ptr().add((top - d.y0) * dw), (bottom - top) * dw) };
            rotate_rows(src, sw, sh, mine, top, bottom, dw, rot, a)
        });
    }
}

/// Transposes four rows of four pixels: `d[i][j] = s[j][i]`.
///
/// # Safety
/// Each `s[j]` must be readable and each `d[i]` writable for four pixels.
#[inline(always)]
unsafe fn transpose4(s: [*const u32; 4], d: [*mut u32; 4]) {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::*;
        let r = |j: usize| _mm_loadu_si128(s[j] as *const __m128i);
        let (a0, a1) = (_mm_unpacklo_epi32(r(0), r(1)), _mm_unpackhi_epi32(r(0), r(1)));
        let (b0, b1) = (_mm_unpacklo_epi32(r(2), r(3)), _mm_unpackhi_epi32(r(2), r(3)));
        _mm_storeu_si128(d[0] as *mut __m128i, _mm_unpacklo_epi64(a0, b0));
        _mm_storeu_si128(d[1] as *mut __m128i, _mm_unpackhi_epi64(a0, b0));
        _mm_storeu_si128(d[2] as *mut __m128i, _mm_unpacklo_epi64(a1, b1));
        _mm_storeu_si128(d[3] as *mut __m128i, _mm_unpackhi_epi64(a1, b1));
    }
    #[cfg(target_arch = "aarch64")]
    {
        use std::arch::aarch64::*;
        let r = |j: usize| vld1q_u32(s[j]);
        let (t0, t1) = (vtrn1q_u32(r(0), r(1)), vtrn2q_u32(r(0), r(1)));
        let (t2, t3) = (vtrn1q_u32(r(2), r(3)), vtrn2q_u32(r(2), r(3)));
        let (t0, t1, t2, t3) = (vreinterpretq_u64_u32(t0), vreinterpretq_u64_u32(t1), vreinterpretq_u64_u32(t2), vreinterpretq_u64_u32(t3));
        vst1q_u32(d[0], vreinterpretq_u32_u64(vtrn1q_u64(t0, t2)));
        vst1q_u32(d[1], vreinterpretq_u32_u64(vtrn1q_u64(t1, t3)));
        vst1q_u32(d[2], vreinterpretq_u32_u64(vtrn2q_u64(t0, t2)));
        vst1q_u32(d[3], vreinterpretq_u32_u64(vtrn2q_u64(t1, t3)));
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    for (i, &di) in d.iter().enumerate() {
        for (j, &sj) in s.iter().enumerate() {
            *di.add(j) = *sj.add(i);
        }
    }
}

/// One tile of a quarter turn: source columns `xa..xa+m` of source rows `yb..yb+k` into the
/// destination rows they land on (`rows` starts at destination row `py0`). Counter-clockwise
/// sends source column `lx` to destination row `sw-1-lx` and row `ly` to column `ly`;
/// clockwise sends column `lx` to row `lx` and row `ly` to column `sh-1-ly`. Whole 4x4
/// blocks go through the vector transpose, the ragged edge pixel by pixel.
#[allow(clippy::too_many_arguments)]
fn turn_tile(src: &[u32], sw: usize, sh: usize, rows: &mut [u32], dw: usize, py0: usize, xa: usize, m: usize, yb: usize, k: usize, ccw: bool) {
    let drow = |lx: usize| (if ccw { sw - 1 - lx } else { lx }) - py0;
    let dcol = |ly: usize| if ccw { ly } else { sh - 1 - ly };
    let (fm, fk) = (m / 4 * 4, k / 4 * 4);
    if fm > 0 && fk > 0 {
        // the extremes of what the block touches, checked once so the vector code may go raw
        assert!((yb + fk - 1) * sw + xa + fm <= src.len());
        let hi = (0..fm).map(|i| drow(xa + i)).max().unwrap_or(0);
        assert!(hi * dw + dw <= rows.len());
        let out = rows.as_mut_ptr();
        let inp = src.as_ptr();
        for j in (0..fk).step_by(4) {
            for i in (0..fm).step_by(4) {
                let (lx, ly) = (xa + i, yb + j);
                // SAFETY: the asserts above bound every row and column used here; the
                // destination rows of one block are distinct.
                unsafe {
                    let order: [usize; 4] = if ccw { [0, 1, 2, 3] } else { [3, 2, 1, 0] };
                    let s = order.map(|q| inp.add((ly + q) * sw + lx));
                    let col = if ccw { ly } else { sh - 4 - ly };
                    let d = [0, 1, 2, 3].map(|q| out.add(drow(lx + q) * dw + col));
                    transpose4(s, d);
                }
            }
        }
    }
    for i in 0..m {
        for j in 0..k {
            if i < fm && j < fk {
                continue;
            }
            rows[drow(xa + i) * dw + dcol(yb + j)] = src[(yb + j) * sw + xa + i];
        }
    }
}

/// Fills destination rows `py0..py1` (`rows` starts at row `py0`, stride
/// `dw`) of the turned frame from the source area `a`.
#[allow(clippy::too_many_arguments)]
fn rotate_rows(src: &[u32], sw: usize, sh: usize, rows: &mut [u32], py0: usize, py1: usize, dw: usize, rot: Rotation, a: Area) {
    match rot {
        Rotation::None => {
            for py in py0..py1 {
                rows[(py - py0) * dw + a.x0..(py - py0) * dw + a.x1].copy_from_slice(&src[py * sw + a.x0..py * sw + a.x1]);
            }
        }
        Rotation::Half => {
            for py in py0..py1 {
                let ly = sh - 1 - py;
                let out = &mut rows[(py - py0) * dw + sw - a.x1..(py - py0) * dw + sw - a.x0];
                for (o, &s) in out.iter_mut().zip(src[ly * sw + a.x0..ly * sw + a.x1].iter().rev()) {
                    *o = s;
                }
            }
        }
        Rotation::Ccw | Rotation::Cw => {
            let ccw = rot == Rotation::Ccw;
            let mut py = py0;
            while py < py1 {
                let m = BLOCK.min(py1 - py);
                // the source columns these destination rows come from
                let xa = if ccw { sw - py - m } else { py };
                let mut yb = a.y0;
                while yb < a.y1 {
                    let k = BLOCK.min(a.y1 - yb);
                    turn_tile(src, sw, sh, rows, dw, py0, xa, m, yb, k, ccw);
                    yb += k;
                }
                py += m;
            }
        }
    }
}

/// Blends two opaque pixels: `f` / 255 of `b`, the rest of `a`.
#[inline]
fn mix(a: u32, b: u32, f: u32) -> u32 {
    scale(a, 255 - f).wrapping_add(scale(b, f))
}

/// Bilinear taps for `n` outputs over `src` inputs: (i0, i1, weight of i1).
fn taps(n: usize, src: usize) -> Vec<(usize, usize, u32)> {
    (0..n)
        .map(|i| {
            let f = ((i as f32 + 0.5) * src as f32 / n as f32 - 0.5).clamp(0.0, (src - 1) as f32);
            let i0 = f as usize;
            (i0, (i0 + 1).min(src - 1), (f.fract() * 255.0).round() as u32)
        })
        .collect()
}

/// Draws `src` bilinearly resampled into the `w` x `h` box at (x, y), the
/// rows shared out over threads. Pixels outside the surface are clipped.
pub fn scale_into(s: &mut Surface, src: &Image, x: i32, y: i32, w: usize, h: usize) {
    if src.w == 0 || src.h == 0 || w == 0 || h == 0 {
        return;
    }
    let cols = taps(w, src.w);
    let rows = taps(h, src.h);
    let top = (y.max(s.y0 as i32)) as usize;
    let bottom = ((y + h as i32).max(0) as usize).min(s.h);
    if top >= bottom {
        return;
    }
    let shrink = |ly: usize, out: &mut [u32]| {
        let row = &src.px[ly * src.w..][..src.w];
        for (o, &(x0, x1, f)) in out.iter_mut().zip(&cols) {
            *o = mix(row[x0], row[x1], f);
        }
    };
    s.draw_rows(&[(top, bottom)], threads(), &|s| {
        let (mut a, mut b) = (vec![0u32; w], vec![0u32; w]);
        let mut have: Option<(usize, usize)> = None;
        for py in s.y0..s.h {
            // y may be negative (box clipped at the top), so index in signed space.
            let (y0, y1, fy) = rows[(py as i32 - y) as usize];
            if have != Some((y0, y1)) {
                shrink(y0, &mut a);
                if y1 == y0 {
                    b.copy_from_slice(&a);
                } else {
                    shrink(y1, &mut b);
                }
                have = Some((y0, y1));
            }
            let x0 = x.max(0) as usize;
            let x1 = ((x + w as i32).max(0) as usize).min(s.w);
            let base = (py - s.y0) * s.stride;
            let out = &mut s.px[base + x0..base + x1];
            let skip = (x0 as i32 - x) as usize;
            for (i, o) in out.iter_mut().enumerate() {
                *o = mix(a[skip + i], b[skip + i], fy);
            }
        }
    });
}

/// Tile of the turn renderer (rows x pixels); see `draw_turn`.
const TILE: usize = 16;
/// Fixed-point fraction bits of the sampling coordinates.
const FIX: i32 = 16;

/// Ease-in-out for the turn: slow start, slow landing.
pub fn ease_in_out(p: f32) -> f32 {
    let p = p.clamp(0.0, 1.0);
    p * p * (3.0 - 2.0 * p)
}

/// Sampling of one image inside the turning rectangle: where the first
/// pixel of a span lands and the step per pixel, in 16.16 fixed point.
#[derive(Clone, Copy)]
struct Sampler {
    w: usize,
    h: usize,
    /// (u, v) of the frame's pixel (0, 0) centre
    origin: (f32, f32),
    /// per frame pixel to the right
    dx: (i32, i32),
    /// per frame row down
    dy: (f32, f32),
}

impl Sampler {
    /// Start of the span at frame pixel (x, y), fixed point.
    #[inline]
    fn at(&self, x: usize, y: usize) -> (i32, i32) {
        let u = self.origin.0 + self.dy.0 * y as f32 + (x as f32 * self.dx.0 as f32) / (1 << FIX) as f32;
        let v = self.origin.1 + self.dy.1 * y as f32 + (x as f32 * self.dx.1 as f32) / (1 << FIX) as f32;
        ((u * (1 << FIX) as f32) as i32, (v * (1 << FIX) as f32) as i32)
    }

    /// The pixel index for fixed-point (u, v), if inside the image.
    #[inline(always)]
    fn index(&self, u: i32, v: i32) -> Option<usize> {
        let (x, y) = ((u >> FIX) as u32 as usize, (v >> FIX) as u32 as usize);
        (x < self.w && y < self.h).then(|| y * self.w + x)
    }
}

/// One frame of the rotation animation at progress `k` (0 = the old frame
/// exactly as the panel showed it, 1 = the new frame): a single rectangle
/// that turns from `from` (how the old content lies in the new frame's
/// coordinates) to upright while its size slides from the old frame's to the
/// new one's, cross-fading between the two. Outside it is black.
pub fn draw_turn(s: &mut Surface, old: &Image, new: &Image, from: Rotation, k: f32) {
    let k = k.clamp(0.0, 1.0);
    let theta = from.angle() * (1.0 - k);
    let (sin, cos) = theta.sin_cos();
    let cw = old.w as f32 + (new.w as f32 - old.w as f32) * k;
    let ch = old.h as f32 + (new.h as f32 - old.h as f32) * k;
    // `new` is the frame being drawn, so its size is the frame's (a band of
    // the surface only carries its clip bottom)
    let (cx, cy) = (new.w as f32 / 2.0, new.h as f32 / 2.0);
    let fade = (k * 255.0).round() as u32;
    // content coordinates of the frame's pixel (0, 0) centre, rotated by
    // -theta, and their change per pixel and per row
    let (dx, dy) = (0.5 - cx, 0.5 - cy);
    let u0 = dx * cos + dy * sin + cw / 2.0;
    let v0 = -dx * sin + dy * cos + ch / 2.0;
    let sampler = |img: &Image| {
        let (sx, sy) = (img.w as f32 / cw, img.h as f32 / ch);
        Sampler {
            w: img.w,
            h: img.h,
            origin: (u0 * sx, v0 * sy),
            dx: ((cos * sx * (1 << FIX) as f32).round() as i32, (-sin * sy * (1 << FIX) as f32).round() as i32),
            dy: (sin * sx, cos * sy),
        }
    };
    let (so, sn) = (sampler(old), sampler(new));
    let (y0, y1) = (s.y0, s.h);
    let width = new.w.min(s.w);
    let black = super::pack(0, 0, 0, 255);
    s.draw_rows(&[(y0, y1)], threads(), &|s| {
        // 16 x 16 tiles: at a steep angle a row alone walks a fresh source
        // cache line every pixel or two, while a tile's rows revisit the
        // lines their neighbours just fetched
        let mut top = s.y0;
        while top < s.h {
            let bottom = (top + TILE).min(s.h);
            let mut x0 = 0;
            while x0 < width {
                let x1 = (x0 + TILE).min(width);
                for py in top..bottom {
                    let base = (py - s.y0) * s.stride;
                    let out = &mut s.px[base + x0..base + x1];
                    let (mut ou, mut ov) = so.at(x0, py);
                    let (mut nu, mut nv) = sn.at(x0, py);
                    macro_rules! span {
                        ($pixel:expr) => {
                            for o in out.iter_mut() {
                                *o = match (so.index(ou, ov), sn.index(nu, nv)) {
                                    (Some(oi), Some(ni)) => $pixel(oi, ni),
                                    _ => black,
                                };
                                ou += so.dx.0;
                                ov += so.dx.1;
                                nu += sn.dx.0;
                                nv += sn.dx.1;
                            }
                        };
                    }
                    match fade {
                        0 => span!(|oi: usize, _ni: usize| old.px[oi]),
                        255 => span!(|_oi: usize, ni: usize| new.px[ni]),
                        f => span!(|oi: usize, ni: usize| mix(old.px[oi], new.px[ni], f)),
                    }
                }
                x0 = x1;
            }
            top = bottom;
        }
    });
}

/// Gravity shorter than this many g (the sensor's own units) means the device lies flat: it calls for no orientation.
const FLAT_G: f32 = 0.35;
/// A held orientation survives gravity this far past the edge of its quarter-circle, so a device held near a
/// boundary does not flip back and forth.
const HYSTERESIS: f32 = 15.0 * std::f32::consts::PI / 180.0;

/// The direction an orientation's top edge points, as the angle of gravity measured from the bottom of the screen.
fn sector_centre(o: Orientation) -> f32 {
    match o {
        Orientation::Portrait => 0.0,
        Orientation::LandscapeLeft => std::f32::consts::FRAC_PI_2,
        Orientation::PortraitUpsideDown => std::f32::consts::PI,
        Orientation::LandscapeRight => -std::f32::consts::FRAC_PI_2,
    }
}

/// The orientation a gravity reading calls for, with hysteresis. `x` runs right and `y` runs down the screen, both in
/// g, and the vector points toward the ground (so a device standing upright reads `(0, 1)`). Returns `None` when the
/// device lies too flat to tell, and keeps `current` while gravity stays within the band around it.
pub fn orientation_from_gravity(x: f32, y: f32, current: Orientation) -> Option<Orientation> {
    // a NaN or infinite reading also returns None
    if !(x.is_finite() && y.is_finite() && (x * x + y * y).sqrt() >= FLAT_G) {
        return None;
    }
    let theta = x.atan2(y);
    let turn = |o: Orientation| {
        let d = (theta - sector_centre(o)).rem_euclid(std::f32::consts::TAU);
        d.min(std::f32::consts::TAU - d)
    };
    if turn(current) <= std::f32::consts::FRAC_PI_4 + HYSTERESIS {
        return Some(current);
    }
    Orientation::ALL.iter().copied().min_by(|a, b| turn(*a).total_cmp(&turn(*b)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::pack;

    fn image(w: usize, h: usize) -> Image {
        Image { w, h, px: (0..w * h as u32 as usize).map(|i| 0xff00_0000 | i as u32).collect() }
    }

    /// The turn one pixel at a time, as the definition.
    fn reference(img: &Image, rot: Rotation) -> Image {
        let (w, h) = rot.size(img.w, img.h);
        let mut out = Image::new(w, h);
        for y in 0..img.h {
            for x in 0..img.w {
                let (ox, oy) = match rot {
                    Rotation::None => (x, y),
                    Rotation::Ccw => (y, img.w - 1 - x),
                    Rotation::Cw => (img.h - 1 - y, x),
                    Rotation::Half => (img.w - 1 - x, img.h - 1 - y),
                };
                out.px[oy * w + ox] = img.px[y * img.w + x];
            }
        }
        out
    }

    #[test]
    fn orientations_map_to_turns_and_sizes() {
        assert_eq!(Orientation::Portrait.rotation(), Rotation::None);
        assert_eq!(Orientation::LandscapeLeft.rotation(), Rotation::Cw);
        assert_eq!(Orientation::LandscapeRight.rotation(), Rotation::Ccw);
        assert_eq!(Orientation::PortraitUpsideDown.rotation(), Rotation::Half);
        assert_eq!(Orientation::LandscapeLeft.size(1200, 1920), (1920, 1200));
        assert_eq!(Orientation::PortraitUpsideDown.size(1200, 1920), (1200, 1920));
        assert!(Orientation::LandscapeRight.is_landscape() && !Orientation::Portrait.is_landscape());
        assert_eq!(Orientation::default(), Orientation::Portrait);
        for o in Orientation::ALL {
            assert!(!o.name().is_empty());
        }
    }

    #[test]
    fn turns_compose_and_invert() {
        for r in [Rotation::None, Rotation::Ccw, Rotation::Cw, Rotation::Half] {
            assert_eq!(r.then(r.inverse()), Rotation::None);
            assert_eq!(r.then(Rotation::None), r);
        }
        assert_eq!(Rotation::Cw.then(Rotation::Cw), Rotation::Half);
        assert_eq!(Rotation::Ccw.inverse(), Rotation::Cw);
        assert_eq!(Rotation::Half.inverse(), Rotation::Half);
        assert_eq!(Rotation::Cw.then(Rotation::Half), Rotation::Ccw);
        assert!(Rotation::Cw.angle() > 0.0 && Rotation::Ccw.angle() < 0.0);
    }

    #[test]
    fn top_left_of_the_logical_frame_lands_where_the_device_top_is() {
        // device turned counter-clockwise: its top edge is the panel's left
        // edge, so the logical top-left corner is the panel's bottom-left
        let img = image(4, 3);
        let ccw = rotated(&img, Rotation::Ccw);
        assert_eq!((ccw.w, ccw.h), (3, 4));
        assert_eq!(ccw.px[3 * 3], img.px[0], "bottom-left of the panel");
        assert_eq!(ccw.px[0], img.px[3], "logical top-right is the panel's top-left");
        let cw = rotated(&img, Rotation::Cw);
        assert_eq!(cw.px[2], img.px[0], "logical top-left is the panel's top-right");
        let half = rotated(&img, Rotation::Half);
        assert_eq!(half.px[11], img.px[0]);
    }

    #[test]
    fn rotation_matches_the_reference_pixel_for_pixel() {
        for (w, h) in [(1, 1), (8, 8), (13, 9), (17, 30), (64, 40), (200, 130)] {
            let img = image(w, h);
            for rot in [Rotation::None, Rotation::Ccw, Rotation::Cw, Rotation::Half] {
                assert_eq!(rotated(&img, rot), reference(&img, rot), "{w}x{h} {rot:?}");
            }
        }
    }

    #[test]
    fn rotating_areas_touches_only_those_areas_on_every_thread_count() {
        let (w, h) = (90usize, 70usize);
        let img = image(w, h);
        let areas = [Area { x0: 3, y0: 5, x1: 40, y1: 66 }, Area { x0: 50, y0: 0, x1: 90, y1: 20 }, Area { x0: 0, y0: 69, x1: 1, y1: 70 }];
        for rot in [Rotation::None, Rotation::Ccw, Rotation::Cw, Rotation::Half] {
            let want = reference(&img, rot);
            for n in 1..=5 {
                let (dw, dh) = rot.size(w, h);
                let mut out = vec![u32::MAX; dw * dh];
                rotate_areas_on(&img.px, w, h, &mut out, rot, &areas, n);
                for py in 0..dh {
                    for px in 0..dw {
                        let inside = areas.iter().map(|&a| rot.area(a, w, h)).any(|a| px >= a.x0 && px < a.x1 && py >= a.y0 && py < a.y1);
                        let got = out[py * dw + px];
                        let expect = if inside { want.px[py * dw + px] } else { u32::MAX };
                        assert_eq!(got, expect, "{rot:?} n={n} at ({px},{py})");
                    }
                }
            }
        }
    }

    #[test]
    fn large_frames_use_every_thread() {
        let (w, h) = (300usize, 400usize);
        let img = image(w, h);
        let all = Area { x0: 0, y0: 0, x1: w, y1: h };
        for rot in [Rotation::Ccw, Rotation::Cw, Rotation::Half] {
            let (dw, dh) = rot.size(w, h);
            let mut out = vec![0u32; dw * dh];
            rotate_areas_on(&img.px, w, h, &mut out, rot, &[all], 4);
            assert_eq!(out, reference(&img, rot).px, "{rot:?}");
        }
    }

    #[test]
    fn damage_turns_with_the_frame() {
        let (w, h) = (1920usize, 1200usize);
        let mut d = Damage::new(w, h);
        d.add(0, 0, 1920, 31);
        d.add(100, 1000, 50, 60);
        for rot in [Rotation::None, Rotation::Ccw, Rotation::Cw, Rotation::Half] {
            let r = rotate_damage(&d, rot, w, h);
            let (dw, dh) = rot.size(w, h);
            assert_eq!(r.pixels(), d.pixels(), "{rot:?}");
            // every damaged logical pixel maps into the rotated damage
            for a in d.areas() {
                for (lx, ly) in [(a.x0, a.y0), (a.x1 - 1, a.y1 - 1), (a.x0, a.y1 - 1), (a.x1 - 1, a.y0)] {
                    let p = Area { x0: lx, y0: ly, x1: lx + 1, y1: ly + 1 };
                    let q = rot.area(p, w, h);
                    assert!(q.x1 <= dw && q.y1 <= dh);
                    assert!(r.areas().iter().any(|b| b.x0 <= q.x0 && b.y0 <= q.y0 && b.x1 >= q.x1 && b.y1 >= q.y1), "{rot:?} {q:?} in {:?}", r.areas());
                }
            }
        }
        let status = rotate_damage(&d, Rotation::Ccw, w, h);
        assert_eq!(status.areas()[0], Area { x0: 0, y0: 0, x1: 31, y1: 1920 }, "the landscape status bar is the panel's left strip");
        let mut all = Damage::new(w, h);
        all.all();
        assert!(rotate_damage(&all, Rotation::Cw, w, h).is_all());
    }

    #[test]
    fn touches_map_back_into_the_logical_frame() {
        let (pw, ph) = (1200, 1920);
        // top-left of the panel
        assert_eq!(Orientation::Portrait.to_logical(0.0, 0.0, pw, ph), (0.0, 0.0));
        // device turned counter-clockwise (top edge left, right edge up): the panel's top-right
        // is the logical top-left and the panel's top-left the logical bottom-left
        assert_eq!(Orientation::LandscapeLeft.to_logical(1199.0, 0.0, pw, ph), (0.0, 0.0));
        assert_eq!(Orientation::LandscapeLeft.to_logical(0.0, 0.0, pw, ph), (0.0, 1199.0));
        // device turned clockwise (top edge right, left edge up): the panel's bottom-left is the
        // logical top-left
        assert_eq!(Orientation::LandscapeRight.to_logical(0.0, 1919.0, pw, ph), (0.0, 0.0));
        assert_eq!(Orientation::LandscapeRight.to_logical(1199.0, 1919.0, pw, ph), (0.0, 1199.0));
        assert_eq!(Orientation::PortraitUpsideDown.to_logical(1199.0, 1919.0, pw, ph), (0.0, 0.0));
        // a pixel drawn at logical (x, y) is touched at the panel point that maps back to it
        let img = image(1920, 1200);
        let (lx, ly) = (300usize, 50usize);
        for o in [Orientation::LandscapeLeft, Orientation::LandscapeRight, Orientation::PortraitUpsideDown] {
            let (w, h) = o.size(pw, ph);
            let img = if o.is_landscape() { img.clone() } else { image(1200, 1920) };
            let panel = rotated(&img, o.rotation());
            let want = img.px[ly * w as usize + lx];
            let (px, py) = (0..panel.w * panel.h).map(|i| (i % panel.w, i / panel.w)).find(|&(x, y)| panel.px[y * panel.w + x] == want).unwrap();
            let (gx, gy) = o.to_logical(px as f32, py as f32, pw, ph);
            assert_eq!((gx as usize, gy as usize), (lx, ly), "{o:?}");
            let _ = h;
        }
    }

    #[test]
    fn scale_into_keeps_flat_colour_and_clips() {
        let src = Image { w: 40, h: 64, px: vec![pack(9, 8, 7, 255); 40 * 64] };
        let mut buf = vec![0u32; 50 * 50];
        scale_into(&mut Surface { w: 50, h: 50, stride: 50, px: &mut buf, y0: 0 }, &src, 10, -5, 25, 40);
        assert_eq!(buf[0], 0);
        assert_eq!(buf[10], pack(9, 8, 7, 255));
        assert_eq!(buf[34 * 50 + 34], pack(9, 8, 7, 255));
        assert_eq!(buf[35 * 50 + 10], 0, "below the box");
        assert_eq!(buf[34 * 50 + 35], 0, "right of the box");
        // a gradient keeps its order and its ends
        let grad = Image { w: 256, h: 2, px: (0..512).map(|i| pack((i % 256) as u8, 0, 0, 255)).collect() };
        let mut row = vec![0u32; 64 * 2];
        scale_into(&mut Surface { w: 64, h: 2, stride: 64, px: &mut row, y0: 0 }, &grad, 0, 0, 64, 2);
        assert!(row[..64].windows(2).all(|p| p[0] & 255 <= p[1] & 255));
        assert!(row[0] & 255 <= 3 && row[63] & 255 >= 252);
        assert!(row.iter().all(|&p| p >> 24 == 255), "stays opaque");
    }

    #[test]
    fn turn_starts_as_the_turned_old_frame_and_ends_as_the_new_one() {
        let old = Image { w: 60, h: 100, px: (0..6000).map(|i| pack((i % 7 * 36) as u8, (i % 5 * 50) as u8, (i / 60) as u8, 255)).collect() };
        let new = Image { w: 100, h: 60, px: (0..6000).map(|i| pack((i % 3 * 90) as u8, (i / 100) as u8, (i % 11 * 23) as u8, 255)).collect() };
        let mut buf = vec![0u32; 100 * 60];
        // portrait -> landscape-left: the device turns counter-clockwise, so the old content
        // lies turned counter-clockwise in the new frame and settles upright
        let from = Orientation::LandscapeLeft.rotation().inverse();
        assert_eq!(from, Rotation::Ccw);
        draw_turn(&mut Surface { w: 100, h: 60, stride: 100, px: &mut buf, y0: 0 }, &old, &new, from, 0.0);
        let want = rotated(&old, from);
        let same = buf.iter().zip(&want.px).filter(|(a, b)| a == b).count();
        assert!(same as f32 > 0.99 * buf.len() as f32, "{same} of {} pixels match the turned old frame", buf.len());
        draw_turn(&mut Surface { w: 100, h: 60, stride: 100, px: &mut buf, y0: 0 }, &old, &new, from, 1.0);
        assert_eq!(buf, new.px, "the last frame is the new frame, pixel for pixel");
        // half way: a square turned 45°, so the corners of the frame are black
        draw_turn(&mut Surface { w: 100, h: 60, stride: 100, px: &mut buf, y0: 0 }, &old, &new, from, 0.5);
        assert_eq!(buf[0], pack(0, 0, 0, 255));
        assert_ne!(buf[30 * 100 + 50], pack(0, 0, 0, 255), "the centre shows content");
        assert_eq!(ease_in_out(0.0), 0.0);
        assert_eq!(ease_in_out(1.0), 1.0);
        assert!(ease_in_out(0.25) < 0.25 && ease_in_out(0.75) > 0.75);
        // banded drawing matches the whole
        let mut whole = vec![0u32; 100 * 60];
        draw_turn(&mut Surface { w: 100, h: 60, stride: 100, px: &mut whole, y0: 0 }, &old, &new, from, 0.3);
        let mut banded = vec![0u32; 100 * 60];
        for (t, b) in [(0, 17), (17, 44), (44, 60)] {
            draw_turn(&mut Surface { w: 100, h: 60, stride: 100, px: &mut banded, y0: 0 }.rows(t, b), &old, &new, from, 0.3);
        }
        assert_eq!(whole, banded);
    }

    #[test]
    fn gravity_names_each_way_the_device_is_held() {
        // upright in portrait: gravity runs down the screen
        assert_eq!(orientation_from_gravity(0.0, 1.0, Orientation::LandscapeLeft), Some(Orientation::Portrait));
        // the device turned counter-clockwise, top edge on the left: its bottom edge is on the right
        assert_eq!(orientation_from_gravity(1.0, 0.0, Orientation::Portrait), Some(Orientation::LandscapeLeft));
        assert_eq!(orientation_from_gravity(-1.0, 0.0, Orientation::Portrait), Some(Orientation::LandscapeRight));
        assert_eq!(orientation_from_gravity(0.02, -0.98, Orientation::Portrait), Some(Orientation::PortraitUpsideDown));
        // the same answer whichever side it was held before, when the reading is far from a boundary
        assert_eq!(orientation_from_gravity(0.0, -1.0, Orientation::LandscapeRight), Some(Orientation::PortraitUpsideDown));
    }

    #[test]
    fn gravity_that_is_too_weak_or_not_finite_calls_for_no_orientation() {
        // lying flat: the reading is mostly along z, so the x and y parts are too small to tell
        assert_eq!(orientation_from_gravity(0.2, 0.2, Orientation::Portrait), None);
        assert_eq!(orientation_from_gravity(f32::NAN, 1.0, Orientation::Portrait), None);
        assert_eq!(orientation_from_gravity(0.0, f32::INFINITY, Orientation::Portrait), None);
    }

    #[test]
    fn hysteresis_keeps_the_held_orientation_near_a_boundary() {
        // 40 degrees off the top: past the midpoint between portrait and landscape-left, but inside the band
        let (x, y) = (40f32.to_radians().sin(), 40f32.to_radians().cos());
        assert_eq!(orientation_from_gravity(x, y, Orientation::Portrait), Some(Orientation::Portrait));
        assert_eq!(orientation_from_gravity(x, y, Orientation::LandscapeLeft), Some(Orientation::LandscapeLeft));
    }

    #[test]
    fn gravity_switches_once_it_passes_the_band() {
        // 70 degrees off the top is past 45 + 15 degrees: the device turns to landscape-left from either side
        let (x, y) = (70f32.to_radians().sin(), 70f32.to_radians().cos());
        assert_eq!(orientation_from_gravity(x, y, Orientation::Portrait), Some(Orientation::LandscapeLeft));
        assert_eq!(orientation_from_gravity(x, y, Orientation::LandscapeRight), Some(Orientation::LandscapeLeft));
    }

    #[test]
    fn gravity_near_the_seam_wraps_to_upside_down() {
        // 173 degrees: right at the seam between upside-down and the landscape sides
        let (x, y) = (7f32.to_radians().sin(), -(7f32.to_radians().cos()));
        assert_eq!(orientation_from_gravity(x, y, Orientation::Portrait), Some(Orientation::PortraitUpsideDown));
        assert_eq!(orientation_from_gravity(-x, y, Orientation::PortraitUpsideDown), Some(Orientation::PortraitUpsideDown));
    }
}
