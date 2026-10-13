//! Software rasteriser. Pixels are premultiplied RGBA packed little-endian
//! into a u32 (R in the low byte), which matches RGBA_8888 buffers
//! and bitmaps byte for byte.

pub mod rotate;

#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}

/// A borrowed image, e.g. pixels mapped straight out of the APK.
#[derive(Clone, Copy, Debug)]
pub struct View<'a> {
    pub w: usize,
    pub h: usize,
    pub px: &'a [u32],
}

impl<'a> View<'a> {
    /// Zero-copy view of a `RIMG` asset; None if malformed or misaligned.
    pub fn from_raw(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len() < 12 || &bytes[..4] != b"RIMG" {
            return None;
        }
        let w = u32::from_le_bytes(bytes[4..8].try_into().ok()?) as usize;
        let h = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
        if w == 0 || h == 0 {
            return None;
        }
        let n = w.checked_mul(h)?;
        let data = bytes.get(12..n.checked_mul(4)?.checked_add(12)?)?;
        // SAFETY: u32 has no invalid bit patterns; align_to only yields the aligned middle.
        let (head, px, _) = unsafe { data.align_to::<u32>() };
        (head.is_empty() && px.len() == n).then_some(View { w, h, px })
    }

    /// Copies a sub-rectangle (clipped to the image).
    pub fn crop(&self, x: usize, y: usize, w: usize, h: usize) -> Image {
        let mut out = Image::new(w, h);
        for row in 0..h.min(self.h.saturating_sub(y)) {
            let n = w.min(self.w.saturating_sub(x));
            let src = (y + row) * self.w + x;
            out.px[row * w..row * w + n].copy_from_slice(&self.px[src..src + n]);
        }
        out
    }
}

pub const fn pack(r: u8, g: u8, b: u8, a: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16 | (a as u32) << 24
}

/// Premultiplies a straight-alpha colour.
pub fn rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    let m = |c: u8| div255(c as u32 * a as u32) as u8;
    pack(m(r), m(g), m(b), a)
}

#[inline]
fn div255(x: u32) -> u32 {
    (x + 128 + ((x + 128) >> 8)) >> 8
}

/// Scales every channel (alpha included) by `f / 255`.
#[inline]
pub fn scale(c: u32, f: u32) -> u32 {
    let rb = c & 0x00ff_00ff;
    let ga = (c >> 8) & 0x00ff_00ff;
    let rb = (rb * f + 0x0080_0080 + (((rb * f + 0x0080_0080) >> 8) & 0x00ff_00ff)) >> 8;
    let ga = (ga * f + 0x0080_0080 + (((ga * f + 0x0080_0080) >> 8) & 0x00ff_00ff)) >> 8;
    (rb & 0x00ff_00ff) | (ga & 0x00ff_00ff) << 8
}

/// Scales colour channels by `f / 255`, keeping alpha.
#[inline]
pub fn shade(c: u32, f: u32) -> u32 {
    (scale(c, f) & 0x00ff_ffff) | (c & 0xff00_0000)
}

/// Porter-Duff source-over for premultiplied pixels. Branch-free so the
/// blit loops vectorise; exact for every premultiplied source (a transparent
/// pixel is all zero, an opaque one scales `dst` by 0).
#[inline]
pub fn over(dst: u32, src: u32) -> u32 {
    src.wrapping_add(scale(dst, 255 - (src >> 24)))
}

/// A half-open pixel rectangle inside a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}

impl Area {
    pub fn pixels(&self) -> usize {
        (self.x1 - self.x0) * (self.y1 - self.y0)
    }

    fn contains(&self, o: &Area) -> bool {
        self.x0 <= o.x0 && self.y0 <= o.y0 && self.x1 >= o.x1 && self.y1 >= o.y1
    }

    fn bounds(&self, o: &Area) -> Area {
        Area { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }
}

/// The parts of a frame that changed since the last one, as a short list of
/// rectangles clipped to the frame. Adding a rectangle merges it with any
/// existing one when their bounding box wastes no pixels, so a tile that
/// moved a little costs one rectangle, not two overlapping ones.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Damage {
    w: usize,
    h: usize,
    areas: Vec<Area>,
}

/// Beyond this many rectangles the bookkeeping costs more than it saves.
const MAX_AREAS: usize = 24;

impl Damage {
    pub fn new(w: usize, h: usize) -> Self {
        Damage { w, h, areas: Vec::with_capacity(MAX_AREAS + 1) }
    }

    /// Exact changed row runs; equal frames produce no upload. A missing previous
    /// frame forces initialization. Adjacent runs merge through normal damage rules.
    pub fn between(previous: &[u32], current: &[u32], w: usize, h: usize) -> Self {
        let mut damage = Self::new(w, h);
        assert_eq!(current.len(), w * h);
        if previous.len() != current.len() {
            damage.all();
            return damage;
        }
        for y in 0..h {
            let a = &previous[y*w..(y+1)*w];
            let b = &current[y*w..(y+1)*w];
            if a == b { continue; }
            let left = a.iter().zip(b).position(|(a,b)| a != b).unwrap();
            let right = a.iter().zip(b).rposition(|(a,b)| a != b).unwrap() + 1;
            damage.add(left as i32, y as i32, (right-left) as i32, 1);
        }
        damage
    }

    /// Copies the pixels under `rects` (x, y, w, h; clipped to the frame) so a later draw can be
    /// compared with them by `changed_in`.
    pub fn snapshot_rects(frame: &[u32], stride: usize, w: usize, h: usize, rects: &[(i32, i32, i32, i32)]) -> Vec<Vec<u32>> {
        rects.iter().map(|&r| {
            let (x0, y0, x1, y1) = clip_rect(r, w, h);
            let mut v = Vec::with_capacity((x1 - x0) * (y1 - y0));
            for y in y0..y1 {
                v.extend_from_slice(&frame[y * stride + x0..y * stride + x1]);
            }
            v
        }).collect()
    }

    /// The exact changed row runs inside `rects`, comparing `frame` with the `before` snapshot of
    /// the same rectangles: pixels that came out identical cost no upload and no present.
    pub fn changed_in(rects: &[(i32, i32, i32, i32)], before: &[Vec<u32>], frame: &[u32], stride: usize, w: usize, h: usize) -> Self {
        let mut damage = Self::new(w, h);
        for (&r, old) in rects.iter().zip(before) {
            let (x0, y0, x1, y1) = clip_rect(r, w, h);
            let rw = x1 - x0;
            if old.len() != rw * (y1 - y0) {
                damage.add(x0 as i32, y0 as i32, rw as i32, (y1 - y0) as i32);
                continue;
            }
            for y in y0..y1 {
                let a = &old[(y - y0) * rw..(y - y0 + 1) * rw];
                let b = &frame[y * stride + x0..y * stride + x1];
                if a == b {
                    continue;
                }
                let left = a.iter().zip(b).position(|(a, b)| a != b).unwrap();
                let right = a.iter().zip(b).rposition(|(a, b)| a != b).unwrap() + 1;
                damage.add((x0 + left) as i32, y as i32, (right - left) as i32, 1);
            }
        }
        damage
    }

    pub fn clear(&mut self) {
        self.areas.clear();
    }

    /// Marks the whole frame.
    /// The frame size this damage is tracked for.
    pub fn size(&self) -> (usize, usize) {
        (self.w, self.h)
    }

    pub fn all(&mut self) {
        self.areas.clear();
        self.areas.push(Area { x0: 0, y0: 0, x1: self.w, y1: self.h });
    }

    pub fn is_empty(&self) -> bool {
        self.areas.is_empty()
    }

    pub fn is_all(&self) -> bool {
        self.areas.len() == 1 && self.areas[0] == Area { x0: 0, y0: 0, x1: self.w, y1: self.h }
    }

    pub fn areas(&self) -> &[Area] {
        &self.areas
    }

    /// Pixels inside the rectangles (two that overlap without having merged
    /// count their overlap twice, which is also what copying them costs).
    pub fn pixels(&self) -> usize {
        self.areas.iter().map(Area::pixels).sum()
    }

    /// Adds a rectangle (any units off the frame are clipped away).
    pub fn add(&mut self, x: i32, y: i32, w: i32, h: i32) {
        let a = Area {
            x0: x.clamp(0, self.w as i32) as usize,
            y0: y.clamp(0, self.h as i32) as usize,
            x1: (x + w).clamp(0, self.w as i32) as usize,
            y1: (y + h).clamp(0, self.h as i32) as usize,
        };
        if a.x0 < a.x1 && a.y0 < a.y1 {
            self.merge(a);
        }
    }

    pub fn union(&mut self, other: &Damage) {
        for a in &other.areas {
            self.merge(*a);
        }
    }

    fn merge(&mut self, mut a: Area) {
        let mut i = 0;
        while i < self.areas.len() {
            let e = self.areas[i];
            if e.contains(&a) {
                return;
            }
            let b = e.bounds(&a);
            if a.contains(&e) || b.pixels() <= e.pixels() + a.pixels() {
                // absorb e into a and start over, since the bigger a may now swallow others
                self.areas.swap_remove(i);
                a = b;
                i = 0;
            } else {
                i += 1;
            }
        }
        self.areas.push(a);
        if self.areas.len() > MAX_AREAS {
            let all = self.areas.iter().skip(1).fold(self.areas[0], |b, e| b.bounds(e));
            self.areas.clear();
            self.areas.push(all);
        }
    }

    /// The changed rows as sorted, disjoint `top..bottom` spans.
    pub fn rows(&self, out: &mut Vec<(usize, usize)>) {
        out.clear();
        out.extend(self.areas.iter().map(|a| (a.y0, a.y1)));
        out.sort_unstable();
        let mut n = 0;
        for i in 0..out.len() {
            if n > 0 && out[i].0 <= out[n - 1].1 {
                out[n - 1].1 = out[n - 1].1.max(out[i].1);
            } else {
                out[n] = out[i];
                n += 1;
            }
        }
        out.truncate(n);
    }
}

fn clip_rect((x, y, w, h): (i32, i32, i32, i32), fw: usize, fh: usize) -> (usize, usize, usize, usize) {
    let c = |v: i32, m: usize| v.clamp(0, m as i32) as usize;
    (c(x, fw), c(y, fh), c(x + w, fw), c(y + h, fh))
}

pub mod affinity;
pub mod cursor;
pub mod pool;

/// Worker threads for a frame: `SPRINGBOARD_THREADS` if set, else the core
/// count capped at four (more threads only queue on the same cores).
pub fn threads() -> usize {
    static N: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *N.get_or_init(|| {
        std::env::var("SPRINGBOARD_THREADS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get().min(4)))
            .max(1)
    })
}

/// The frame row at dirty-row index `i` when the dirty rows of `spans` are
/// numbered consecutively; `i` past the end maps to the last span's bottom.
fn span_row(spans: &[(usize, usize)], mut i: usize) -> usize {
    for &(top, bottom) in spans {
        if i < bottom - top {
            return top + i;
        }
        i -= bottom - top;
    }
    spans.last().map_or(0, |s| s.1)
}

/// The parts of `spans` whose dirty-row indices fall in `a..b`.
fn cut_spans(spans: &[(usize, usize)], a: usize, b: usize) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut at = 0;
    spans.iter().filter_map(move |&(top, bottom)| {
        let (lo, hi) = (at, at + bottom - top);
        at = hi;
        let (s, e) = (lo.max(a), hi.min(b));
        (s < e).then_some((top + s - lo, top + e - lo))
    })
}

impl Image {
    pub fn new(w: usize, h: usize) -> Self {
        Image { w, h, px: vec![0; w * h] }
    }

    /// Parses the `RIMG` raw asset format: magic, u32 LE width, u32 LE height,
    /// then width*height premultiplied RGBA pixels.
    pub fn from_raw(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 12 || &bytes[..4] != b"RIMG" {
            return None;
        }
        let w = u32::from_le_bytes(bytes[4..8].try_into().ok()?) as usize;
        let h = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
        if w == 0 || h == 0 {
            return None;
        }
        let data = bytes.get(12..w.checked_mul(h)?.checked_mul(4)?.checked_add(12)?)?;
        let px = data.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        Some(Image { w, h, px })
    }

    /// Multiplies every pixel by a coverage mask of the same size.
    pub fn apply_mask(&mut self, mask: &[u8]) {
        for (p, &m) in self.px.iter_mut().zip(mask) {
            *p = scale(*p, m as u32);
        }
    }

    /// Bilinear resize, used when an asset was packed for another density.
    pub fn resized(&self, w: usize, h: usize) -> Image {
        if (w, h) == (self.w, self.h) {
            return self.clone();
        }
        let mut out = Image::new(w, h);
        if w == 0 || h == 0 || self.w == 0 || self.h == 0 {
            return out;
        }
        let sx = self.w as f32 / w as f32;
        let sy = self.h as f32 / h as f32;
        for y in 0..h {
            let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (self.h - 1) as f32);
            let (y0, ty) = (fy as usize, fy.fract());
            let y1 = (y0 + 1).min(self.h - 1);
            for x in 0..w {
                let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (self.w - 1) as f32);
                let (x0, tx) = (fx as usize, fx.fract());
                let x1 = (x0 + 1).min(self.w - 1);
                let p = |xx: usize, yy: usize| self.px[yy * self.w + xx];
                let mut c = 0u32;
                for ch in 0..4 {
                    let v = |q: u32| ((q >> (ch * 8)) & 255) as f32;
                    let top = v(p(x0, y0)) * (1.0 - tx) + v(p(x1, y0)) * tx;
                    let bot = v(p(x0, y1)) * (1.0 - tx) + v(p(x1, y1)) * tx;
                    c |= ((top * (1.0 - ty) + bot * ty).round() as u32) << (ch * 8);
                }
                out.px[y * w + x] = c;
            }
        }
        out
    }

    pub fn view(&self) -> View<'_> {
        View { w: self.w, h: self.h, px: &self.px }
    }

    /// The bounding box of the non-transparent pixels as (x, y, w, h); None when empty.
    pub fn bounds(&self) -> Option<(usize, usize, usize, usize)> {
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        for (i, &p) in self.px.iter().enumerate() {
            if p >> 24 != 0 {
                let (x, y) = (i % self.w, i / self.w);
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
        (x0 != usize::MAX).then(|| (x0, y0, x1 - x0 + 1, y1 - y0 + 1))
    }

    /// The image cropped to its ink (an empty image stays empty).
    pub fn trimmed(&self) -> Image {
        match self.bounds() {
            Some((x, y, w, h)) => self.crop(x, y, w, h),
            None => Image::new(0, 0),
        }
    }

    pub fn crop(&self, x: usize, y: usize, w: usize, h: usize) -> Image {
        self.view().crop(x, y, w, h)
    }

    /// Material blur: runs the box blur on a 1/2 or 1/4 size copy and scales it back up bilinearly.
    /// For a frosted backdrop this is indistinguishable from `blur` and costs about 1/16 of it at
    /// radius 8 and up; small radii fall back to the exact blur.
    pub fn blur_material(&mut self, radius: usize) {
        let f = match radius {
            0 => return,
            1..=3 => 1,
            4..=7 => 2,
            _ => 4,
        };
        let (w, h) = (self.w, self.h);
        if f == 1 || w < 2 * f || h < 2 * f {
            return self.blur(radius);
        }
        let (sw, sh) = (w.div_ceil(f), h.div_ceil(f));
        let mut small = Image::new(sw, sh);
        for sy in 0..sh {
            for sx in 0..sw {
                let (x0, y0) = (sx * f, sy * f);
                let (x1, y1) = ((x0 + f).min(w), (y0 + f).min(h));
                let mut sum = [0u32; 4];
                for y in y0..y1 {
                    for &p in &self.px[y * w + x0..y * w + x1] {
                        for (ch, c) in sum.iter_mut().enumerate() {
                            *c += (p >> (ch * 8)) & 255;
                        }
                    }
                }
                let n = ((x1 - x0) * (y1 - y0)) as u32;
                small.px[sy * sw + sx] = (0..4).fold(0, |c, ch| c | ((sum[ch] + n / 2) / n) << (ch * 8));
            }
        }
        small.blur((radius / f).max(1));
        let axis = |len: usize, small_len: usize| -> Vec<(usize, usize, u32)> {
            (0..len)
                .map(|i| {
                    let p = ((i as f32 + 0.5) / f as f32 - 0.5).clamp(0.0, (small_len - 1) as f32);
                    let i0 = p as usize;
                    (i0, (i0 + 1).min(small_len - 1), ((p - i0 as f32) * 256.0) as u32)
                })
                .collect()
        };
        let (cols, rws) = (axis(w, sw), axis(h, sh));
        let lerp = |a: u32, b: u32, t: u32| (0..4).fold(0u32, |c, ch| {
            let (x, y) = ((a >> (ch * 8)) & 255, (b >> (ch * 8)) & 255);
            c | (((x * (256 - t) + y * t) >> 8) << (ch * 8))
        });
        for (y, &(r0, r1, fy)) in rws.iter().enumerate() {
            let (top, bot) = (&small.px[r0 * sw..(r0 + 1) * sw], &small.px[r1 * sw..(r1 + 1) * sw]);
            for (d, &(c0, c1, fx)) in self.px[y * w..(y + 1) * w].iter_mut().zip(&cols) {
                *d = lerp(lerp(top[c0], top[c1], fx), lerp(bot[c0], bot[c1], fx), fy);
            }
        }
    }

    /// Three-pass box blur, which approximates a gaussian of sigma ~ radius.
    pub fn blur(&mut self, radius: usize) {
        if radius == 0 {
            return;
        }
        let mut tmp = vec![0u32; self.px.len()];
        for _ in 0..3 {
            box_pass(&self.px, &mut tmp, self.w, self.h, radius, true);
            box_pass(&tmp, &mut self.px, self.w, self.h, radius, false);
        }
    }
}

/// Source-over of one row, eight pixels at a time: a block that is fully
/// opaque is copied (memcpy class), a fully empty one is skipped, and only
/// the blocks in between pay for the blend. Same result as `over` per pixel.
#[inline(always)]
fn over_row(dst: &mut [u32], src: &[u32], f: impl Fn(u32) -> u32) {
    const N: usize = 8;
    let (d, d_rest) = dst.as_chunks_mut::<N>();
    let (s, s_rest) = src.as_chunks::<N>();
    for (d, s) in d.iter_mut().zip(s) {
        let mut t = [0u32; N];
        for (t, &s) in t.iter_mut().zip(s) {
            *t = f(s);
        }
        let (and, or) = t.iter().fold((!0u32, 0u32), |(a, o), &p| (a & p, o | p));
        if and >> 24 == 255 {
            d.copy_from_slice(&t);
        } else if or != 0 {
            d.iter_mut().zip(&t).for_each(|(d, &s)| *d = over(*d, s));
        }
    }
    for (d, &s) in d_rest.iter_mut().zip(s_rest) {
        *d = over(*d, f(s));
    }
}

/// Divides by a fixed span with a multiply instead of a division; exact for
/// the box-filter sums (`n * span < 2^32`), which is checked by the caller.
#[derive(Clone, Copy)]
struct Div {
    span: u32,
    inv: u64,
}

impl Div {
    fn new(span: u32) -> Self {
        Div { span, inv: (1u64 << 32).div_ceil(span as u64) }
    }

    #[inline(always)]
    fn div(self, n: u32) -> u32 {
        if self.span < 4096 && n < (1 << 20) {
            ((n as u64 * self.inv) >> 32) as u32
        } else {
            n / self.span
        }
    }
}

fn box_pass(src: &[u32], dst: &mut [u32], w: usize, h: usize, r: usize, horizontal: bool) {
    let span = (2 * r + 1) as u32;
    let div = Div::new(span);
    let avg = |sum: &[u32; 4]| (0..4).fold(0u32, |c, ch| c | div.div(sum[ch] + span / 2) << (ch * 8));
    let add = |sum: &mut [u32; 4], p: u32| {
        for (ch, s) in sum.iter_mut().enumerate() {
            *s += (p >> (ch * 8)) & 255;
        }
    };
    let sub = |sum: &mut [u32; 4], p: u32| {
        for (ch, s) in sum.iter_mut().enumerate() {
            *s -= (p >> (ch * 8)) & 255;
        }
    };
    if w == 0 || h == 0 {
        return;
    }
    if horizontal {
        for (src, dst) in src.chunks_exact(w).zip(dst.chunks_exact_mut(w)) {
            let at = |i: isize| src[i.clamp(0, w as isize - 1) as usize];
            let mut sum = [0u32; 4];
            for i in -(r as isize)..=(r as isize) {
                add(&mut sum, at(i));
            }
            for (i, d) in dst.iter_mut().enumerate() {
                *d = avg(&sum);
                add(&mut sum, at(i as isize + r as isize + 1));
                sub(&mut sum, at(i as isize - r as isize));
            }
        }
        return;
    }
    // vertical: one running sum per column, walking down the rows so memory stays sequential
    let row = |y: isize| &src[y.clamp(0, h as isize - 1) as usize * w..][..w];
    let mut sums = vec![[0u32; 4]; w];
    for y in -(r as isize)..=(r as isize) {
        for (sum, &p) in sums.iter_mut().zip(row(y)) {
            add(sum, p);
        }
    }
    for y in 0..h {
        for (d, sum) in dst[y * w..(y + 1) * w].iter_mut().zip(&sums) {
            *d = avg(sum);
        }
        let (a, b) = (row(y as isize + r as isize + 1), row(y as isize - r as isize));
        for ((sum, &pa), &pb) in sums.iter_mut().zip(a).zip(b) {
            add(sum, pa);
            sub(sum, pb);
        }
    }
}

/// Coverage of an bifrost-style app icon: a superellipse |x|^5 + |y|^5 = 1,
/// 4x4 supersampled.
pub fn squircle_mask(size: usize) -> Vec<u8> {
    const N: f32 = 5.0;
    const SS: usize = 4;
    let half = size as f32 / 2.0;
    let mut mask = vec![0u8; size * size];
    for y in 0..size {
        for x in 0..size {
            let mut hits = 0;
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = (x as f32 + (sx as f32 + 0.5) / SS as f32 - half) / half;
                    let py = (y as f32 + (sy as f32 + 0.5) / SS as f32 - half) / half;
                    if px.abs().powf(N) + py.abs().powf(N) <= 1.0 {
                        hits += 1;
                    }
                }
            }
            mask[y * size + x] = (hits * 255 / (SS * SS)) as u8;
        }
    }
    mask
}

/// Anti-aliased coverage of a rounded rectangle centred in a w*h box.
pub fn rrect_coverage(px: f32, py: f32, w: f32, h: f32, r: f32) -> f32 {
    let qx = (px - w / 2.0).abs() - (w / 2.0 - r);
    let qy = (py - h / 2.0).abs() - (h / 2.0 - r);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let dist = outside + qx.max(qy).min(0.0) - r;
    (0.5 - dist).clamp(0.0, 1.0)
}

pub fn rrect_mask(w: usize, h: usize, r: f32) -> Vec<u8> {
    let mut mask = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let c = rrect_coverage(x as f32 + 0.5, y as f32 + 0.5, w as f32, h as f32, r);
            mask[y * w + x] = (c * 255.0).round() as u8;
        }
    }
    mask
}

/// A mutable view of a pixel buffer with a row stride (window buffers are padded).
/// Coordinates are absolute; `px` starts at row `y0`, so a band of a larger
/// frame only receives the drawing that falls inside rows `y0..h`.
pub struct Surface<'a> {
    pub w: usize,
    pub h: usize,
    pub stride: usize,
    pub px: &'a mut [u32],
    pub y0: usize,
}

impl Surface<'_> {
    /// The band of rows `top..self.h` as its own surface (clips drawing).
    pub fn band(&mut self, top: usize) -> Surface<'_> {
        let top = top.clamp(self.y0, self.h);
        Surface { w: self.w, h: self.h, stride: self.stride, px: &mut self.px[(top - self.y0) * self.stride..], y0: top }
    }

    /// The rows `top..bottom` (clipped to this surface) as their own surface.
    pub fn rows(&mut self, top: usize, bottom: usize) -> Surface<'_> {
        let top = top.clamp(self.y0, self.h);
        let bottom = bottom.clamp(top, self.h);
        let end = ((bottom - self.y0) * self.stride).min(self.px.len());
        Surface { w: self.w, h: bottom, stride: self.stride, px: &mut self.px[(top - self.y0) * self.stride..end], y0: top }
    }

    /// Draws every row span of `spans` (sorted, disjoint) with `f`, sharing
    /// the rows out over `threads` scoped threads; the caller's thread takes
    /// the last share. Short jobs run inline, since a spawn costs more.
    pub fn draw_rows(&mut self, spans: &[(usize, usize)], threads: usize, f: &(dyn Fn(&mut Surface) + Sync)) {
        const MIN_ROWS_PER_THREAD: usize = 64;
        let total: usize = spans.iter().map(|&(t, b)| b - t).sum();
        let threads = threads.clamp(1, (total / MIN_ROWS_PER_THREAD).max(1));
        let per = total.div_ceil(threads);
        let (w, h, stride) = (self.w, self.h, self.stride);
        let draw = |px: &mut [u32], y0: usize, h: usize, a: usize, b: usize| {
            let mut s = Surface { w, h, stride, px, y0 };
            for (top, bottom) in cut_spans(spans, a, b) {
                f(&mut s.rows(top, bottom));
            }
        };
        if threads == 1 {
            return draw(self.px, self.y0, h, 0, total);
        }
        // cut the pixel rows into one band per share, then let the pool draw them (the caller too)
        type Band<'a> = std::sync::Mutex<Option<(&'a mut [u32], usize, usize, usize, usize)>>;
        let mut bands: Vec<Band<'_>> = Vec::with_capacity(threads);
        let mut rest = &mut *self.px;
        let mut row = self.y0;
        for k in 0..threads {
            let (a, b) = (k * per, ((k + 1) * per).min(total));
            let bottom = if b == total { h } else { span_row(spans, b) };
            let (mine, tail) = rest.split_at_mut(((bottom - row) * stride).min(rest.len()));
            bands.push(std::sync::Mutex::new(Some((mine, row, bottom, a, b))));
            if b == total {
                break;
            }
            rest = tail;
            row = bottom;
        }
        pool::run(bands.len(), &|k| {
            if let Some((mine, top, bottom, a, b)) = bands[k].lock().unwrap().take() {
                draw(mine, top, bottom, a, b);
            }
        });
    }

    /// Mutable pixel at absolute (x, y); the caller keeps (x, y) inside.
    #[inline]
    pub fn at(&mut self, x: usize, y: usize) -> &mut u32 {
        &mut self.px[(y - self.y0) * self.stride + x]
    }

    /// Draws `img` with its top-left at (x, y), source-over, clipped.
    /// `tint` scales colour (255 = unchanged), `alpha` scales opacity.
    pub fn blit(&mut self, img: &Image, x: i32, y: i32, tint: u32, alpha: u32) {
        let x0 = x.max(0);
        let y0 = y.max(self.y0 as i32);
        let x1 = (x + img.w as i32).min(self.w as i32);
        let y1 = (y + img.h as i32).min(self.h as i32);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let n = (x1 - x0) as usize;
        for dy in y0..y1 {
            let src = &img.px[(dy - y) as usize * img.w + (x0 - x) as usize..][..n];
            let dst = &mut self.px[(dy as usize - self.y0) * self.stride + x0 as usize..][..n];
            // one tight loop per case so each vectorises
            match (tint, alpha) {
                (255, 255) => over_row(dst, src, |s| s),
                (255, a) => over_row(dst, src, |s| scale(s, a)),
                (t, 255) => over_row(dst, src, |s| shade(s, t)),
                (t, a) => over_row(dst, src, |s| scale(shade(s, t), a)),
            }
        }
    }

    /// Blits a white-rendered image (text, a template glyph) in `color`: every channel is multiplied by the colour's
    /// channel and the opacity by `alpha`. The colour may be mid-fade between two schemes, so the image is never rebuilt.
    pub fn blit_colored(&mut self, img: &Image, x: i32, y: i32, color: u32, alpha: u32) {
        let x0 = x.max(0);
        let y0 = y.max(self.y0 as i32);
        let x1 = (x + img.w as i32).min(self.w as i32);
        let y1 = (y + img.h as i32).min(self.h as i32);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let n = (x1 - x0) as usize;
        let (cr, cg, cb) = (color & 255, (color >> 8) & 255, (color >> 16) & 255);
        for dy in y0..y1 {
            let src = &img.px[(dy - y) as usize * img.w + (x0 - x) as usize..][..n];
            let dst = &mut self.px[(dy as usize - self.y0) * self.stride + x0 as usize..][..n];
            for (d, &s) in dst.iter_mut().zip(src) {
                let a = s >> 24;
                if a == 0 {
                    continue;
                }
                let m = |c: u32, k: u32| div255(((s >> c) & 255) * k) << c;
                let tinted = m(0, cr) | m(8, cg) | m(16, cb) | (a << 24);
                let tinted = if alpha == 255 { tinted } else { scale(tinted, alpha) };
                *d = over(*d, tinted);
            }
        }
    }

    /// `blit_colored` that keeps each destination pixel's alpha. The colour is blended into the RGB under the glyph
    /// coverage (the image's alpha times the colour's alpha), so a glyph on a translucent material tints what is
    /// there without changing how opaque it is. The colour is premultiplied (`rgba`); its target is un-premultiplied
    /// and then scaled by the destination alpha, so a white target lifts a translucent pixel to the same level as an
    /// opaque one would be lifted by its own amount.
    pub fn blit_colored_keep_alpha(&mut self, img: &Image, x: i32, y: i32, color: u32) {
        let x0 = x.max(0);
        let y0 = y.max(self.y0 as i32);
        let x1 = (x + img.w as i32).min(self.w as i32);
        let y1 = (y + img.h as i32).min(self.h as i32);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let n = (x1 - x0) as usize;
        let ca = color >> 24;
        if ca == 0 {
            return;
        }
        // un-premultiplied target channel, 0..255
        let target = |c: u32| (((color >> c) & 255) * 255 / ca).min(255);
        let (tr, tg, tb) = (target(0), target(8), target(16));
        for dy in y0..y1 {
            let src = &img.px[(dy - y) as usize * img.w + (x0 - x) as usize..][..n];
            let dst = &mut self.px[(dy as usize - self.y0) * self.stride + x0 as usize..][..n];
            for (d, &s) in dst.iter_mut().zip(src) {
                let g = s >> 24;
                if g == 0 {
                    continue;
                }
                // coverage of the colour at this pixel, 0..255
                let k = div255(g * ca);
                let inv = 255 - k;
                let a = *d >> 24;
                let mix = |sh: u32, t: u32| {
                    let dc = (*d >> sh) & 255;
                    let pt = div255(t * a);
                    div255(dc * inv + pt * k) << sh
                };
                *d = mix(0, tr) | mix(8, tg) | mix(16, tb) | (a << 24);
            }
        }
    }

    /// Copies opaque rows straight in; the fast path for the wallpaper.
    pub fn copy(&mut self, img: View, x: i32, y: i32) {
        if x >= self.w as i32 || x.saturating_neg() >= img.w as i32 {
            return;
        }
        for dy in y.max(self.y0 as i32)..(y + img.h as i32).min(self.h as i32) {
            let sx = (-x).max(0) as usize;
            let dx = x.max(0) as usize;
            let n = (img.w - sx).min(self.w - dx);
            let src = &img.px[(dy - y) as usize * img.w + sx..][..n];
            self.px[(dy as usize - self.y0) * self.stride + dx..][..n].copy_from_slice(src);
        }
    }

    /// Fills a pixel-aligned rectangle with an opaque colour, clipped.
    pub fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize, color: u32) {
        let (x1, y1) = ((x + w).min(self.w), (y + h).min(self.h));
        for row in y.max(self.y0).min(y1)..y1 {
            let base = (row - self.y0) * self.stride;
            self.px[base + x.min(x1)..base + x1].fill(color);
        }
    }

    /// Fills an anti-aliased rounded rectangle with a premultiplied colour.
    pub fn fill_rrect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, color: u32) {
        self.rrect(x, y, w, h, r, None, color);
    }

    /// Writes an anti-aliased rounded rectangle: inside it the pixels become `color` exactly, alpha included (no
    /// source-over, so a translucent material replaces what is under it). An edge pixel becomes the
    /// coverage-weighted mix of `color` and the pixel that was there.
    pub fn write_rrect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, color: u32) {
        let r = r.min(w / 2.0).min(h / 2.0);
        let (ix0, iy0) = (x.floor().max(0.0) as usize, (y.floor().max(0.0) as usize).max(self.y0));
        let ix1 = ((x + w).ceil().max(0.0) as usize).min(self.w);
        let iy1 = ((y + h).ceil().max(0.0) as usize).min(self.h);
        if ix0 >= ix1 || iy0 >= iy1 {
            return;
        }
        for py in iy0..iy1 {
            for px in ix0..ix1 {
                let c = rrect_coverage(px as f32 + 0.5 - x, py as f32 + 0.5 - y, w, h, r);
                if c <= 0.0 {
                    continue;
                }
                let d = self.at(px, py);
                *d = if c >= 1.0 {
                    color
                } else {
                    let k = (c * 255.0) as u32;
                    scale(*d, 255 - k).wrapping_add(scale(color, k))
                };
            }
        }
    }

    /// Liquid Glass style panel: blurs what is already on the surface under the rounded rectangle,
    /// tints it, and composites through the anti-aliased mask. `tint` is premultiplied and drawn over
    /// the blurred backdrop. With Reduce Transparency on, the panel is the opaque `solid` colour and
    /// nothing is blurred. `blur` is a box-blur radius in pixels.
    // Rect, radius and the tint/solid/blur triple are independent draw parameters; a struct would be a one-use wrapper.
    #[allow(clippy::too_many_arguments)]
    pub fn glass_rrect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, tint: u32, solid: u32, blur: usize) {
        if crate::appearance::reduce_transparency() {
            self.fill_rrect(x, y, w, h, r, solid);
            return;
        }
        let r = r.min(w / 2.0).min(h / 2.0);
        if crate::glass::gpu() {
            // the GPU presenter draws the blurred backdrop under this shape; only the tint is ours
            crate::glass::push(crate::glass::Rect { x, y, w, h, r, ..Default::default() });
            self.fill_rrect(x, y, w, h, r, crate::glass::tint(tint));
            return;
        }
        let ix0 = x.floor().max(0.0) as usize;
        let iy0 = (y.floor().max(0.0) as usize).max(self.y0);
        let ix1 = ((x + w).ceil().max(0.0) as usize).min(self.w);
        let iy1 = ((y + h).ceil().max(0.0) as usize).min(self.h);
        if ix0 >= ix1 || iy0 >= iy1 {
            return;
        }
        let (bw, bh) = (ix1 - ix0, iy1 - iy0);
        let mut snap = Image { w: bw, h: bh, px: vec![0; bw * bh] };
        for row in 0..bh {
            let src = (iy0 + row - self.y0) * self.stride + ix0;
            snap.px[row * bw..row * bw + bw].copy_from_slice(&self.px[src..src + bw]);
        }
        snap.blur_material(blur);
        for row in 0..bh {
            for col in 0..bw {
                let (px, py) = (ix0 + col, iy0 + row);
                // far from the corners and edges the mask is 1: skip the distance maths
                let inside = (py as f32) >= y + r + 1.0 && (py as f32) < y + h - r - 1.0 && (px as f32) >= x + 1.0 && (px as f32) < x + w - 1.0;
                let cov = if inside {
                    255
                } else {
                    let c = rrect_coverage(px as f32 + 0.5 - x, py as f32 + 0.5 - y, w, h, r);
                    if c <= 0.0 {
                        continue;
                    }
                    (c.min(1.0) * 255.0) as u32
                };
                let glass = over(snap.px[row * bw + col], tint);
                let d = self.at(px, py);
                *d = over(*d, if cov == 255 { glass } else { scale(glass, cov) });
            }
        }
    }

    /// Strokes the inside edge of a rounded rectangle with thickness `t`.
    // Rect, radius, thickness and colour are independent draw parameters, mirroring `fill_rrect`.
    #[allow(clippy::too_many_arguments)]
    pub fn stroke_rrect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, t: f32, color: u32) {
        self.rrect(x, y, w, h, r, Some(t), color);
    }

    // Shared body of `stroke_rrect` and `fill_rrect`; same flat draw parameters.
    #[allow(clippy::too_many_arguments)]
    fn rrect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, stroke: Option<f32>, color: u32) {
        let color = crate::appearance::contrast(color);
        let r = r.min(w / 2.0).min(h / 2.0);
        let (ix0, iy0) = (x.floor().max(0.0) as usize, (y.floor().max(0.0) as usize).max(self.y0));
        let ix1 = ((x + w).ceil() as usize).min(self.w);
        let iy1 = ((y + h).ceil() as usize).min(self.h);
        if ix0 >= ix1 || iy0 >= iy1 {
            return;
        }
        let edge = |s: &mut Self, px: usize, py: usize| {
            let (lx, ly) = (px as f32 + 0.5 - x, py as f32 + 0.5 - y);
            let mut c = rrect_coverage(lx, ly, w, h, r);
            if let Some(t) = stroke {
                let inner = rrect_coverage(lx - t, ly - t, w - 2.0 * t, h - 2.0 * t, (r - t).max(0.0));
                c *= 1.0 - inner;
            }
            if c > 0.0 {
                let d = s.at(px, py);
                *d = over(*d, scale(color, (c * 255.0) as u32));
            }
        };
        if let Some(t) = stroke {
            // A pixel centre a whole pixel inside the inner edge's straight sides and outside its corner squares has
            // inner coverage 1 (and outer 1), so it draws nothing: each such row only walks its two edge bands. The
            // skipped pixels drew nothing before either, so the result is the same bit for bit; a large outline no
            // longer costs a full per-pixel pass over its area (window and card borders were most of a Settings
            // frame on the lab PC, round 75).
            let inset = r.max(t) + 1.0;
            // centres strictly between x + m and x + w - m
            let span = |m: f32| {
                let a = ((x + m - 0.5).floor() as isize + 1).max(ix0 as isize) as usize;
                let b = ((x + w - m - 0.5).ceil() as isize).max(a as isize) as usize;
                (a.min(ix1), b.min(ix1))
            };
            for py in iy0..iy1 {
                let ly = py as f32 + 0.5 - y;
                // below the top band and above the bottom one, the stretch between the corner squares is empty too
                // (both coverages 1 there), so a row near the top or bottom also walks only its two corners
                let (skip0, skip1) = if ly > inset && ly < h - inset {
                    span(t + 1.0)
                } else if ly > t + 1.0 && ly < h - t - 1.0 {
                    span(inset)
                } else {
                    (ix1, ix1)
                };
                for px in ix0..skip0 {
                    edge(self, px, py);
                }
                for px in skip1.max(skip0)..ix1 {
                    edge(self, px, py);
                }
            }
            return;
        }
        // A pixel centre half a pixel inside every straight edge and outside
        // the corner squares has coverage 1 (see `rrect_coverage`), so each
        // row is a few edge pixels, a solid run, and a few edge pixels; only
        // the corner squares need the per-pixel distance. The run keeps a
        // whole pixel of margin so f32 rounding at the half-pixel line still
        // goes through the per-pixel path, bit for bit as before.
        let opaque = color >> 24 == 255;
        for py in iy0..iy1 {
            let ly = py as f32 + 0.5 - y;
            let corner_row = ly < r || ly > h - r;
            let (lo, hi) = if corner_row { ((r + 1.0).max(1.0), (w - r - 1.0).min(w - 1.0)) } else { (1.0, w - 1.0) };
            // pixels whose centre lies in lo..=hi
            let full0 = ((x + lo - 0.5).ceil().max(ix0 as f32) as usize).min(ix1);
            let full1 = ((x + hi - 0.5).floor() as usize + 1).clamp(full0, ix1);
            let solid = ly >= 1.0 && ly <= h - 1.0 && full0 < full1;
            if !solid {
                for px in ix0..ix1 {
                    edge(self, px, py);
                }
                continue;
            }
            for px in ix0..full0 {
                edge(self, px, py);
            }
            let base = (py - self.y0) * self.stride;
            let run = &mut self.px[base + full0..base + full1];
            if opaque {
                run.fill(color);
            } else {
                run.iter_mut().for_each(|d| *d = over(*d, color));
            }
            for px in full1..ix1 {
                edge(self, px, py);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn changed_in_is_empty_for_identical_pixels() {
        let frame = vec![7u32; 16 * 8];
        let rects = [(0, 0, 16, 2), (4, 3, 8, 4)];
        let snap = Damage::snapshot_rects(&frame, 16, 16, 8, &rects);
        assert!(Damage::changed_in(&rects, &snap, &frame, 16, 16, 8).is_empty());
    }

    #[test]
    fn changed_in_reports_only_the_changed_run() {
        let mut frame = vec![0u32; 16 * 8];
        let rects = [(0, 0, 16, 8)];
        let snap = Damage::snapshot_rects(&frame, 16, 16, 8, &rects);
        frame[3 * 16 + 5] = 1;
        frame[3 * 16 + 9] = 1;
        let d = Damage::changed_in(&rects, &snap, &frame, 16, 16, 8);
        assert_eq!(d.pixels(), 5);
        assert_eq!((d.areas()[0].x0, d.areas()[0].x1, d.areas()[0].y0, d.areas()[0].y1), (5, 10, 3, 4));
    }

    #[test]
    fn changed_in_clips_rects_to_the_frame() {
        let mut frame = vec![0u32; 8 * 4];
        let rects = [(-5, -5, 100, 100)];
        let snap = Damage::snapshot_rects(&frame, 8, 8, 4, &rects);
        assert_eq!(snap[0].len(), 32);
        frame[31] = 9;
        assert_eq!(Damage::changed_in(&rects, &snap, &frame, 8, 8, 4).pixels(), 1);
    }

    use super::*;

    #[test]
    fn blit_colored_tints_white_text_to_the_colour() {
        let mut img = Image::new(2, 1);
        img.px.fill(pack(255, 255, 255, 255));
        let mut buf = vec![pack(0, 0, 0, 255); 2];
        let mut s = Surface { w: 2, h: 1, stride: 2, px: &mut buf, y0: 0 };
        s.blit_colored(&img, 0, 0, pack(200, 100, 50, 255), 255);
        assert_eq!(buf[0], pack(200, 100, 50, 255));
        let mut s = Surface { w: 2, h: 1, stride: 2, px: &mut buf, y0: 0 };
        s.blit_colored(&img, 0, 0, pack(255, 255, 255, 255), 255);
        assert_eq!(buf[0], pack(255, 255, 255, 255));
    }

    #[test]
    fn damage_between_tracks_initial_equal_and_small_changes() {
        let a = vec![7; 80*60];
        assert!(Damage::between(&[], &a,80,60).is_all());
        assert!(Damage::between(&a,&a,80,60).is_empty());
        let mut b=a.clone();
        for y in 10..15 { for x in 20..24 {b[y*80+x]=9;} }
        let d=Damage::between(&a,&b,80,60);
        assert_eq!(d.pixels(),20);
        assert_eq!(d.areas(), &[Area{x0:20,y0:10,x1:24,y1:15}]);
        let mut c=b.clone(); c[30*80+50]=11;
        let mut stale=d; stale.union(&Damage::between(&b,&c,80,60));
        assert_eq!(stale.pixels(),21);
    }

    #[test]
    fn fill_rect_is_opaque_and_clipped() {
        let mut buf = vec![0u32; 4 * 3];
        let mut s = Surface { w: 3, h: 3, stride: 4, px: &mut buf, y0: 0 };
        s.fill_rect(1, 1, 5, 5, 7);
        assert_eq!(buf, [0, 0, 0, 0, 0, 7, 7, 0, 0, 7, 7, 0]);
    }

    static GLASS_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn glass_panel_tints_the_backdrop_inside_and_leaves_outside_alone() {
        // Serialised with the Reduce Transparency test: that flag is global.
        let _guard = GLASS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let red = pack(255, 0, 0, 255);
        let mut px = vec![red; 64 * 64];
        let mut s = Surface { w: 64, h: 64, stride: 64, px: &mut px, y0: 0 };
        let tint = rgba(0, 0, 255, 128);
        // solid is green, so if the fallback ran the blue check below would fail instead of passing
        s.glass_rrect(8.0, 8.0, 48.0, 48.0, 6.0, tint, pack(0, 255, 0, 255), 4);
        let centre = s.px[32 * 64 + 32];
        let (r, b, a) = (centre & 0xff, (centre >> 16) & 0xff, centre >> 24);
        assert_eq!(a, 255, "glass over an opaque backdrop stays opaque");
        assert!(b > 60 && r < 200, "blue tint mixes into the red backdrop: r={r} b={b}");
        assert_eq!(s.px[0], red, "a corner outside the panel is untouched");
    }

    /// Baseline for the GPU blur plan: CPU cost of the desktop Search card's glass at 1920x1200.
    /// Run with `cargo test --release -p bifrost-render glass_cost -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn glass_cost_quicksearch_card() {
        let (w, h) = (1920usize, 1200usize);
        let mut px = vec![pack(40, 60, 90, 255); w * h];
        let mut s = Surface { w, h, stride: w, px: &mut px, y0: 0 };
        let tint = rgba(30, 32, 36, 120);
        let solid = pack(38, 41, 45, 222);
        let runs = 20;
        let t0 = std::time::Instant::now();
        for _ in 0..runs {
            s.glass_rrect(620.0, 180.0, 680.0, 440.0, 14.0, tint, solid, 18);
        }
        println!("glass_rrect 680x440 r18 at 1920x1200: {:.2} ms/frame (host)", t0.elapsed().as_secs_f64() * 1000.0 / runs as f64);
        println!("BENCH glass_rrect_680x440 {:.3} ms", t0.elapsed().as_secs_f64() * 1000.0 / runs as f64);
        let mut img = Image::new(680, 440);
        let t1 = std::time::Instant::now();
        for _ in 0..runs { img.blur(18); }
        println!("BENCH blur_exact_680x440_r18 {:.3} ms", t1.elapsed().as_secs_f64() * 1000.0 / runs as f64);
        let t2 = std::time::Instant::now();
        for _ in 0..runs { img.blur_material(18); }
        println!("BENCH blur_material_680x440_r18 {:.3} ms", t2.elapsed().as_secs_f64() * 1000.0 / runs as f64);
        let src = Image::new(64, 64);
        let t3 = std::time::Instant::now();
        for _ in 0..200 { std::hint::black_box(src.resized(52, 52)); }
        println!("BENCH image_resize_64_to_52 {:.4} ms", t3.elapsed().as_secs_f64() * 1000.0 / 200.0);
    }

    #[test]
    fn reduce_transparency_gives_the_opaque_fallback() {
        let _guard = GLASS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut px = vec![pack(255, 0, 0, 255); 32 * 32];
        let solid = pack(20, 20, 30, 255);
        {
            let mut s = Surface { w: 32, h: 32, stride: 32, px: &mut px, y0: 0 };
            crate::appearance::publish(crate::appearance::Appearance { text_scale: 1.0, zoom: 1.0, hold_ms: 500, bold: false, reduce_motion: false, increase_contrast: false, reduce_transparency: true, button_shapes: false });
            s.glass_rrect(4.0, 4.0, 24.0, 24.0, 4.0, rgba(0, 0, 255, 128), solid, 6);
            crate::appearance::publish(crate::appearance::Appearance { text_scale: 1.0, zoom: 1.0, hold_ms: 500, bold: false, reduce_motion: false, increase_contrast: false, reduce_transparency: false, button_shapes: false });
        }
        assert_eq!(px[16 * 32 + 16], solid);
    }

    #[test]
    fn over_respects_alpha() {
        let red = pack(255, 0, 0, 255);
        let half_blue = rgba(0, 0, 255, 128);
        assert_eq!(over(red, pack(0, 0, 0, 0)), red);
        assert_eq!(over(red, pack(0, 255, 0, 255)), pack(0, 255, 0, 255));
        let mixed = over(red, half_blue);
        assert_eq!(mixed & 0xff, 127); // red halved
        assert_eq!((mixed >> 16) & 0xff, 128); // blue added
        assert_eq!(mixed >> 24, 255);
    }

    #[test]
    fn scale_and_shade() {
        let c = pack(200, 100, 50, 255);
        assert_eq!(scale(c, 255), c);
        assert_eq!(scale(c, 0), 0);
        assert_eq!(shade(c, 0), pack(0, 0, 0, 255));
        assert_eq!(scale(c, 128), pack(100, 50, 25, 128));
    }

    #[test]
    fn raw_roundtrip() {
        let mut bytes = b"RIMG".to_vec();
        bytes.extend(2u32.to_le_bytes());
        bytes.extend(1u32.to_le_bytes());
        bytes.extend([1, 2, 3, 4, 5, 6, 7, 8]);
        let img = Image::from_raw(&bytes).unwrap();
        assert_eq!((img.w, img.h), (2, 1));
        assert_eq!(img.px, vec![pack(1, 2, 3, 4), pack(5, 6, 7, 8)]);
        assert!(Image::from_raw(&bytes[..15]).is_none());
        assert!(Image::from_raw(b"NOPE00000000").is_none());
    }

    #[test]
    fn view_maps_aligned_raw_without_copying() {
        let mut words = vec![0u32; 3 + 2];
        let bytes: &mut [u8] = unsafe { std::slice::from_raw_parts_mut(words.as_mut_ptr() as *mut u8, 20) };
        bytes[..4].copy_from_slice(b"RIMG");
        bytes[4..8].copy_from_slice(&2u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
        bytes[12..20].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let v = View::from_raw(bytes).unwrap();
        assert_eq!(v.px, &[pack(1, 2, 3, 4), pack(5, 6, 7, 8)]);
        assert_eq!(v.px.as_ptr() as usize, bytes[12..].as_ptr() as usize);
        assert!(View::from_raw(&bytes[1..]).is_none());
    }

    #[test]
    fn squircle_is_solid_inside_clear_at_corners_and_symmetric() {
        let n = 60;
        let m = squircle_mask(n);
        assert_eq!(m[30 * n + 30], 255);
        assert_eq!(m[0], 0);
        assert_eq!(m[n - 1], 0);
        assert_eq!(m[30 * n], 255, "edge midpoints are covered");
        for y in 0..n {
            for x in 0..n {
                assert_eq!(m[y * n + x], m[y * n + (n - 1 - x)]);
                assert_eq!(m[y * n + x], m[(n - 1 - y) * n + x]);
            }
        }
        // a squircle covers more than a circle but less than the square
        let area: u32 = m.iter().map(|&v| v as u32).sum::<u32>() / 255;
        assert!(area as f32 > 0.80 * (n * n) as f32 && (area as usize) < n * n);
    }

    #[test]
    fn rrect_mask_corners() {
        let m = rrect_mask(40, 20, 8.0);
        assert_eq!(m[0], 0);
        assert_eq!(m[10 * 40 + 20], 255);
        assert_eq!(m[10 * 40], 255, "straight edges are fully covered");
    }

    #[test]
    fn blit_clips_and_blends() {
        let mut buf = vec![pack(0, 0, 0, 255); 4 * 3];
        let mut s = Surface { w: 3, h: 3, stride: 4, px: &mut buf, y0: 0 };
        let img = Image { w: 2, h: 2, px: vec![pack(255, 255, 255, 255); 4] };
        s.blit(&img, 2, 2, 255, 255);
        s.blit(&img, -1, -1, 255, 128);
        assert_eq!(buf[2 * 4 + 2], pack(255, 255, 255, 255));
        assert_eq!(buf[0], pack(128, 128, 128, 255));
        assert_eq!(buf[3], pack(0, 0, 0, 255), "stride padding untouched");
        assert_eq!(buf[1], pack(0, 0, 0, 255));
    }

    #[test]
    fn copy_offsets_rows() {
        let mut buf = vec![0u32; 3 * 2];
        let mut s = Surface { w: 3, h: 2, stride: 3, px: &mut buf, y0: 0 };
        let img = Image { w: 3, h: 2, px: (1..=6).collect() };
        s.copy(img.view(), -1, 0);
        assert_eq!(buf, vec![2, 3, 0, 5, 6, 0]);
    }

    #[test]
    fn copy_fully_off_screen_is_a_no_op() {
        let mut buf = vec![0u32; 3 * 2];
        let mut s = Surface { w: 3, h: 2, stride: 3, px: &mut buf, y0: 0 };
        let img = Image { w: 3, h: 2, px: (1..=6).collect() };
        for x in [3, 10, -3, -10] {
            s.copy(img.view(), x, 0);
        }
        assert_eq!(buf, vec![0; 6]);
    }

    #[test]
    fn copy_clips_right_edge() {
        let mut buf = vec![0u32; 3 * 2];
        let mut s = Surface { w: 3, h: 2, stride: 3, px: &mut buf, y0: 0 };
        let img = Image { w: 3, h: 2, px: (1..=6).collect() };
        s.copy(img.view(), 2, 0);
        assert_eq!(buf, vec![0, 0, 1, 0, 0, 4]);
    }

    #[test]
    fn material_blur_stays_close_to_the_exact_blur() {
        let (w, h) = (96usize, 64usize);
        let mut a = Image::new(w, h);
        for y in 0..h { for x in 0..w { a.px[y * w + x] = pack((x * 2) as u8, (y * 3) as u8, ((x ^ y) * 4) as u8, 255); } }
        let mut b = a.clone();
        a.blur(12);
        b.blur_material(12);
        let worst = a.px.iter().zip(&b.px).map(|(p, q)| (0..3).map(|c| ((p >> (c * 8)) & 255).abs_diff((q >> (c * 8)) & 255)).max().unwrap()).max().unwrap();
        assert!(worst < 24, "worst channel difference {worst}");
        assert!(b.px.iter().all(|p| p >> 24 == 255), "alpha stays opaque");
    }

    #[test]
    fn blur_preserves_flat_colour_and_spreads_a_spike() {
        let mut flat = Image { w: 8, h: 8, px: vec![pack(10, 20, 30, 255); 64] };
        flat.blur(2);
        assert!(flat.px.iter().all(|&p| p == pack(10, 20, 30, 255)));
        let mut spike = Image::new(9, 9);
        spike.px[40] = pack(255, 255, 255, 255);
        spike.blur(1);
        assert!(spike.px[40] >> 24 < 255);
        assert!(spike.px[41] >> 24 > 0);
    }

    #[test]
    fn trimmed_crops_to_ink() {
        let mut img = Image::new(6, 5);
        img.px[1 * 6 + 2] = pack(1, 1, 1, 255);
        img.px[3 * 6 + 4] = pack(1, 1, 1, 9);
        assert_eq!(img.bounds(), Some((2, 1, 3, 3)));
        let t = img.trimmed();
        assert_eq!((t.w, t.h), (3, 3));
        assert_eq!(t.px[0], pack(1, 1, 1, 255));
        assert_eq!(t.px[8], pack(1, 1, 1, 9));
        assert_eq!(Image::new(4, 4).bounds(), None);
        assert_eq!(Image::new(4, 4).trimmed().px.len(), 0);
    }

    #[test]
    fn resize_keeps_flat_colour() {
        let img = Image { w: 4, h: 4, px: vec![pack(9, 8, 7, 255); 16] };
        let r = img.resized(7, 3);
        assert_eq!((r.w, r.h), (7, 3));
        assert!(r.px.iter().all(|&p| p == pack(9, 8, 7, 255)));
    }

    #[test]
    fn fill_rrect_covers_centre() {
        let mut buf = vec![0u32; 20 * 20];
        let mut s = Surface { w: 20, h: 20, stride: 20, px: &mut buf, y0: 0 };
        s.fill_rrect(2.0, 2.0, 16.0, 16.0, 6.0, pack(255, 255, 255, 255));
        assert_eq!(buf[10 * 20 + 10], pack(255, 255, 255, 255));
        assert_eq!(buf[2 * 20 + 2], 0);
        assert_eq!(buf[0], 0);
    }

    #[test]
    fn band_clips_to_its_rows_with_absolute_coordinates() {
        let mut buf = vec![0u32; 4 * 4];
        let mut s = Surface { w: 4, h: 4, stride: 4, px: &mut buf, y0: 0 };
        let mut b = s.band(2);
        b.fill_rect(0, 0, 4, 4, pack(1, 1, 1, 255));
        b.fill_rrect(0.0, 0.0, 4.0, 4.0, 0.0, pack(1, 1, 1, 255));
        b.blit(&Image { w: 1, h: 4, px: vec![pack(9, 9, 9, 255); 4] }, 3, 0, 255, 255);
        assert!(buf[..8].iter().all(|&p| p == 0), "rows above the band untouched");
        assert_eq!(buf[8], pack(1, 1, 1, 255));
        assert_eq!(buf[15], pack(9, 9, 9, 255));
    }

    #[test]
    fn stroke_rrect_matches_the_per_pixel_reference() {
        // the edge-band walk must draw exactly what a full per-pixel pass draws (round 75 speed-up)
        let color = pack(200, 120, 40, 255);
        for &(x, y, w, h, r, t) in &[(2.0f32, 3.0f32, 40.0f32, 30.0f32, 6.0f32, 1.0f32), (1.3, 0.7, 37.4, 28.9, 9.5, 2.5), (0.0, 0.0, 50.0, 50.0, 0.0, 1.0), (4.6, 2.2, 20.0, 41.0, 3.0, 6.0), (0.5, 0.5, 12.0, 12.0, 6.0, 1.5)] {
            let (sw, sh) = (56usize, 52usize);
            let mut fast = vec![0u32; sw * sh];
            Surface { w: sw, h: sh, stride: sw, px: &mut fast, y0: 0 }.stroke_rrect(x, y, w, h, r, t, color);
            let mut slow = vec![0u32; sw * sh];
            let rr = r.min(w / 2.0).min(h / 2.0);
            let c0 = crate::appearance::contrast(color);
            for py in 0..sh {
                for px in 0..sw {
                    let (lx, ly) = (px as f32 + 0.5 - x, py as f32 + 0.5 - y);
                    let c = rrect_coverage(lx, ly, w, h, rr) * (1.0 - rrect_coverage(lx - t, ly - t, w - 2.0 * t, h - 2.0 * t, (rr - t).max(0.0)));
                    if c > 0.0 {
                        slow[py * sw + px] = over(slow[py * sw + px], scale(c0, (c * 255.0) as u32));
                    }
                }
            }
            assert!(fast == slow, "stroke differs for {:?}", (x, y, w, h, r, t));
        }
    }

    #[test]
    fn stroke_rrect_leaves_the_middle_clear() {
        let mut buf = vec![0u32; 20 * 20];
        let mut s = Surface { w: 20, h: 20, stride: 20, px: &mut buf, y0: 0 };
        s.stroke_rrect(2.0, 2.0, 16.0, 16.0, 4.0, 2.0, pack(255, 255, 255, 255));
        assert_eq!(buf[10 * 20 + 10], 0);
        assert_eq!(buf[10 * 20 + 2], pack(255, 255, 255, 255));
    }

    /// The rounded rectangle one pixel at a time, as it was drawn before the
    /// solid-run fast path.
    fn rrect_reference(s: &mut Surface, x: f32, y: f32, w: f32, h: f32, r: f32, color: u32) {
        let r = r.min(w / 2.0).min(h / 2.0);
        let (ix0, iy0) = (x.floor().max(0.0) as usize, (y.floor().max(0.0) as usize).max(s.y0));
        let ix1 = ((x + w).ceil() as usize).min(s.w);
        let iy1 = ((y + h).ceil() as usize).min(s.h);
        for py in iy0..iy1 {
            for px in ix0..ix1 {
                let c = rrect_coverage(px as f32 + 0.5 - x, py as f32 + 0.5 - y, w, h, r);
                if c > 0.0 {
                    let d = s.at(px, py);
                    *d = over(*d, scale(color, (c * 255.0) as u32));
                }
            }
        }
    }

    #[test]
    fn fill_rrect_fast_path_matches_per_pixel_coverage() {
        let (w, h) = (48usize, 40usize);
        let cases = [
            (3.2, 4.7, 30.0, 25.5, 6.0),
            (0.0, 0.0, 48.0, 40.0, 0.0),
            (-5.0, -3.0, 60.0, 50.0, 12.0),
            (10.0, 10.0, 20.0, 20.0, 10.0),
            (7.5, 8.25, 9.0, 0.6, 0.3),
            (4.0, 4.0, 40.0, 30.0, 0.4),
            (1.0, 30.0, 46.0, 12.0, 6.0),
            (0.0, 0.0, 40.0, 30.0, 0.1),
            (2.0, 1.0, 44.0, 38.0, 1.0),
        ];
        for (x, y, cw, ch, r) in cases {
            for color in [pack(255, 255, 255, 255), rgba(20, 200, 90, 120)] {
                let bg = |i: usize| pack((i % 7 * 36) as u8, (i % 5 * 50) as u8, (i % 3 * 90) as u8, 255);
                let mut a: Vec<u32> = (0..w * h).map(bg).collect();
                let mut b = a.clone();
                Surface { w, h, stride: w, px: &mut a, y0: 0 }.fill_rrect(x, y, cw, ch, r, color);
                rrect_reference(&mut Surface { w, h, stride: w, px: &mut b, y0: 0 }, x, y, cw, ch, r, color);
                assert_eq!(a, b, "rrect {x} {y} {cw} {ch} r={r} color={color:08x}");
                // and clipped to a band
                let mut a: Vec<u32> = (0..w * h).map(bg).collect();
                let mut b = a.clone();
                Surface { w, h, stride: w, px: &mut a, y0: 0 }.rows(9, 23).fill_rrect(x, y, cw, ch, r, color);
                rrect_reference(&mut Surface { w, h, stride: w, px: &mut b, y0: 0 }.rows(9, 23), x, y, cw, ch, r, color);
                assert_eq!(a, b, "banded rrect {x} {y} {cw} {ch} r={r}");
            }
        }
    }

    #[test]
    fn blit_matches_the_scalar_reference_for_every_tint_and_alpha() {
        let reference = |d: u32, s: u32| match s >> 24 {
            255 => s,
            0 => d,
            a => s + scale(d, 255 - a),
        };
        let img = Image { w: 13, h: 9, px: (0..13 * 9).map(|i| rgba((i * 7 % 256) as u8, (i * 3 % 256) as u8, (i * 11 % 256) as u8, (i * 5 % 256) as u8)).collect() };
        let bg: Vec<u32> = (0..20 * 20).map(|i| pack((i % 9 * 28) as u8, (i % 4 * 60) as u8, (i % 6 * 40) as u8, 255)).collect();
        for (tint, alpha) in [(255, 255), (150, 255), (255, 90), (150, 90)] {
            let mut a = bg.clone();
            Surface { w: 20, h: 20, stride: 20, px: &mut a, y0: 0 }.blit(&img, 4, -2, tint, alpha);
            let mut b = bg.clone();
            for y in 0..9 {
                for x in 0..13 {
                    let (dx, dy) = (x as i32 + 4, y as i32 - 2);
                    if dy < 0 {
                        continue;
                    }
                    let s = scale(shade(img.px[y * 13 + x], tint), alpha);
                    let d = &mut b[dy as usize * 20 + dx as usize];
                    *d = reference(*d, s);
                }
            }
            assert_eq!(a, b, "tint {tint} alpha {alpha}");
        }
    }

    #[test]
    fn damage_clips_to_the_frame_and_merges_touching_rectangles() {
        let mut d = Damage::new(100, 50);
        assert!(d.is_empty());
        d.add(-10, -10, 20, 20);
        assert_eq!(d.areas(), [Area { x0: 0, y0: 0, x1: 10, y1: 10 }]);
        d.add(90, 45, 50, 50);
        assert_eq!(d.areas().len(), 2, "far apart stays separate");
        assert_eq!(d.areas()[1], Area { x0: 90, y0: 45, x1: 100, y1: 50 });
        d.add(10, 0, 5, 10);
        assert_eq!(d.areas().len(), 2, "an adjacent strip joins its neighbour");
        assert_eq!(d.areas()[1], Area { x0: 0, y0: 0, x1: 15, y1: 10 });
        d.add(2, 2, 3, 3);
        assert_eq!(d.areas().len(), 2, "a contained rectangle adds nothing");
        d.add(200, 0, 10, 10);
        d.add(0, 60, 10, 10);
        d.add(5, 5, 0, 10);
        assert_eq!(d.areas().len(), 2, "off-frame and empty rectangles are dropped");
        assert_eq!(d.pixels(), 150 + 50);
        d.all();
        assert!(d.is_all());
        d.add(1, 1, 1, 1);
        assert!(d.is_all() && d.areas().len() == 1);
        d.clear();
        assert!(d.is_empty() && !d.is_all());
    }

    #[test]
    fn damage_merges_a_moved_tile_into_its_bounding_box() {
        let mut d = Damage::new(1000, 1000);
        d.add(100, 100, 100, 130);
        d.add(80, 100, 100, 130);
        assert_eq!(d.areas(), [Area { x0: 80, y0: 100, x1: 200, y1: 230 }]);
        let mut e = Damage::new(1000, 1000);
        e.add(500, 100, 100, 130);
        d.union(&e);
        assert_eq!(d.areas().len(), 2);
        let mut f = Damage::new(1000, 1000);
        f.all();
        d.union(&f);
        assert!(d.is_all());
    }

    #[test]
    fn damage_collapses_to_one_box_past_the_cap() {
        let mut d = Damage::new(2000, 2000);
        for i in 0..MAX_AREAS + 1 {
            d.add(i as i32 * 60, i as i32 * 60, 10, 10);
        }
        assert_eq!(d.areas().len(), 1);
        assert_eq!(d.areas()[0], Area { x0: 0, y0: 0, x1: MAX_AREAS * 60 + 10, y1: MAX_AREAS * 60 + 10 });
    }

    #[test]
    fn damage_rows_are_sorted_disjoint_spans() {
        let mut d = Damage::new(100, 100);
        d.add(50, 40, 10, 20);
        d.add(0, 10, 10, 10);
        d.add(80, 15, 10, 10);
        d.add(0, 90, 100, 10);
        let mut rows = Vec::new();
        d.rows(&mut rows);
        assert_eq!(rows, [(10, 25), (40, 60), (90, 100)]);
        d.clear();
        d.rows(&mut rows);
        assert!(rows.is_empty());
    }

    #[test]
    fn box_blur_matches_a_naive_clamped_box_average() {
        let (w, h, r) = (13usize, 9usize, 2usize);
        let mut img = Image::new(w, h);
        for (i, p) in img.px.iter_mut().enumerate() {
            let v = (i as u32).wrapping_mul(2654435761);
            *p = v | 0xff00_0000;
        }
        let mut want = img.px.clone();
        for _ in 0..3 {
            let pass = |src: &[u32], horizontal: bool| -> Vec<u32> {
                let mut out = vec![0u32; w * h];
                for y in 0..h {
                    for x in 0..w {
                        let mut sum = [0u32; 4];
                        for k in -(r as isize)..=(r as isize) {
                            let (sx, sy) = if horizontal { ((x as isize + k).clamp(0, w as isize - 1) as usize, y) } else { (x, (y as isize + k).clamp(0, h as isize - 1) as usize) };
                            for ch in 0..4 {
                                sum[ch] += (src[sy * w + sx] >> (ch * 8)) & 255;
                            }
                        }
                        out[y * w + x] = (0..4).fold(0, |c, ch| c | ((sum[ch] + r as u32) / (2 * r as u32 + 1)) << (ch * 8));
                    }
                }
                out
            };
            want = pass(&pass(&want, true), false);
        }
        img.blur(r);
        assert_eq!(img.px, want);
    }

    #[test]
    fn draw_rows_covers_exactly_the_spans_on_every_thread_count() {
        let (w, h) = (7usize, 400usize);
        let spans = [(3, 70), (100, 101), (150, 390)];
        let paint = |s: &mut Surface| {
            for y in s.y0..s.h {
                for x in 0..s.w {
                    *s.at(x, y) = (y * 1000 + x) as u32;
                }
            }
        };
        let mut want = vec![0u32; w * h];
        for &(t, b) in &spans {
            paint(&mut Surface { w, h, stride: w, px: &mut want, y0: 0 }.rows(t, b));
        }
        for threads in 1..=6 {
            let mut got = vec![0u32; w * h];
            Surface { w, h, stride: w, px: &mut got, y0: 0 }.draw_rows(&spans, threads, &paint);
            assert_eq!(got, want, "{threads} threads");
        }
        let mut got = vec![0u32; w * h];
        Surface { w, h, stride: w, px: &mut got, y0: 0 }.draw_rows(&[], 4, &paint);
        assert!(got.iter().all(|&p| p == 0), "no spans, nothing drawn");
    }

    #[test]
    fn rows_clips_both_ends_and_keeps_absolute_coordinates() {
        let mut buf = vec![0u32; 5 * 6];
        let mut s = Surface { w: 5, h: 6, stride: 5, px: &mut buf, y0: 0 };
        let mut r = s.rows(2, 4);
        assert_eq!((r.y0, r.h), (2, 4));
        r.fill_rect(0, 0, 5, 6, 9);
        assert_eq!(r.rows(0, 100).px.len(), 10);
        assert!(buf[..10].iter().all(|&p| p == 0) && buf[20..].iter().all(|&p| p == 0));
        assert!(buf[10..20].iter().all(|&p| p == 9));
    }

    #[test]
    fn thread_count_is_at_least_one() {
        assert!(threads() >= 1);
    }
}
