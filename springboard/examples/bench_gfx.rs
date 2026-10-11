//! Pixel primitive benchmark at 1200x1920: rounded rects, blits, fills and
//! blur, timed against the previous scalar implementations (kept below),
//! which also serve as the byte-exact reference on random inputs.
//! Run with `cargo run --release --example bench_gfx`.
use springboard::gfx::{over, pack, rgba, rrect_coverage, scale, shade, Image, Surface};
use std::hint::black_box;
use std::time::Instant;

const W: usize = 1200;
const H: usize = 1920;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
}

/// Random premultiplied pixels with plenty of opaque and empty runs.
fn noise(rng: &mut Rng, n: usize) -> Vec<u32> {
    let mut v = Vec::with_capacity(n);
    while v.len() < n {
        let run = 1 + rng.next() as usize % 40;
        let kind = rng.next() % 4;
        for _ in 0..run.min(n - v.len()) {
            let r = rng.next();
            v.push(match kind {
                0 => 0,
                1 => pack(r as u8, (r >> 8) as u8, (r >> 16) as u8, 255),
                _ => rgba(r as u8, (r >> 8) as u8, (r >> 16) as u8, (r >> 24) as u8),
            });
        }
    }
    v
}

// ---- previous implementations -------------------------------------------

fn old_blit(s: &mut Surface, img: &Image, x: i32, y: i32, tint: u32, alpha: u32) {
    let x0 = x.max(0);
    let y0 = y.max(s.y0 as i32);
    let x1 = (x + img.w as i32).min(s.w as i32);
    let y1 = (y + img.h as i32).min(s.h as i32);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let n = (x1 - x0) as usize;
    for dy in y0..y1 {
        let src = &img.px[(dy - y) as usize * img.w + (x0 - x) as usize..][..n];
        let dst = &mut s.px[(dy as usize - s.y0) * s.stride + x0 as usize..][..n];
        match (tint, alpha) {
            (255, 255) => dst.iter_mut().zip(src).for_each(|(d, &s)| *d = over(*d, s)),
            (255, a) => dst.iter_mut().zip(src).for_each(|(d, &s)| *d = over(*d, scale(s, a))),
            (t, 255) => dst.iter_mut().zip(src).for_each(|(d, &s)| *d = over(*d, shade(s, t))),
            (t, a) => dst.iter_mut().zip(src).for_each(|(d, &s)| *d = over(*d, scale(shade(s, t), a))),
        }
    }
}

/// One pixel at a time, no solid-run shortcut.
fn old_rrect(s: &mut Surface, x: f32, y: f32, w: f32, h: f32, r: f32, color: u32) {
    let r = r.min(w / 2.0).min(h / 2.0);
    let (ix0, iy0) = (x.floor().max(0.0) as usize, (y.floor().max(0.0) as usize).max(s.y0));
    let ix1 = ((x + w).ceil() as usize).min(s.w);
    let iy1 = ((y + h).ceil() as usize).min(s.h);
    for py in iy0..iy1 {
        for px in ix0..ix1 {
            let c = rrect_coverage(px as f32 + 0.5 - x, py as f32 + 0.5 - y, w, h, r);
            if c > 0.0 {
                let d = &mut s.px[(py - s.y0) * s.stride + px];
                *d = over(*d, scale(color, (c * 255.0) as u32));
            }
        }
    }
}

fn old_box_pass(src: &[u32], dst: &mut [u32], w: usize, h: usize, r: usize, horizontal: bool) {
    let span = (2 * r + 1) as u32;
    let avg = |sum: &[u32; 4]| (0..4).fold(0u32, |c, ch| c | ((sum[ch] + span / 2) / span) << (ch * 8));
    let add = |sum: &mut [u32; 4], p: u32| sum.iter_mut().enumerate().for_each(|(ch, s)| *s += (p >> (ch * 8)) & 255);
    let sub = |sum: &mut [u32; 4], p: u32| sum.iter_mut().enumerate().for_each(|(ch, s)| *s -= (p >> (ch * 8)) & 255);
    if horizontal {
        for (src, dst) in src.chunks_exact(w).zip(dst.chunks_exact_mut(w)) {
            let at = |i: isize| src[i.clamp(0, w as isize - 1) as usize];
            let mut sum = [0u32; 4];
            for i in -(r as isize)..=(r as isize) {
                add(&mut sum, at(i));
            }
            for i in 0..w {
                dst[i] = avg(&sum);
                add(&mut sum, at(i as isize + r as isize + 1));
                sub(&mut sum, at(i as isize - r as isize));
            }
        }
        return;
    }
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

fn old_blur(img: &mut Image, radius: usize) {
    let mut tmp = vec![0u32; img.px.len()];
    for _ in 0..3 {
        old_box_pass(&img.px, &mut tmp, img.w, img.h, radius, true);
        old_box_pass(&tmp, &mut img.px, img.w, img.h, radius, false);
    }
}

// ---- harness ------------------------------------------------------------

/// Best of a few runs, in seconds per call.
fn time(iters: usize, mut f: impl FnMut()) -> f64 {
    f();
    (0..3)
        .map(|_| {
            let t = Instant::now();
            for _ in 0..iters {
                f();
            }
            t.elapsed().as_secs_f64() / iters as f64
        })
        .fold(f64::MAX, f64::min)
}

fn report(name: &str, pixels: usize, iters: usize, old: impl FnMut(), new: impl FnMut()) {
    let (o, n) = (time(iters, old), time(iters, new));
    let ns = |t: f64| t * 1e9 / pixels as f64;
    let mbs = |t: f64| pixels as f64 * 4.0 / t / 1e6;
    println!("{name:<28} old {:7.3} ns/px {:9.0} MB/s | new {:7.3} ns/px {:9.0} MB/s | x{:.2}", ns(o), mbs(o), ns(n), mbs(n), o / n);
}

fn surface(px: &mut [u32]) -> Surface<'_> {
    Surface { w: W, h: H, stride: W, px, y0: 0 }
}

fn main() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let bg = noise(&mut rng, W * H);

    // correctness first: new output must equal the old one byte for byte
    for (tint, alpha) in [(255, 255), (150, 255), (255, 90), (150, 90)] {
        let img = Image { w: 301, h: 77, px: noise(&mut rng, 301 * 77) };
        let (mut a, mut b) = (bg.clone(), bg.clone());
        surface(&mut a).blit(&img, -13, 40, tint, alpha);
        old_blit(&mut surface(&mut b), &img, -13, 40, tint, alpha);
        assert!(a == b, "blit {tint} {alpha}");
    }
    for (x, y, w, h, r) in [(10.3, 20.7, 300.0, 200.5, 24.0), (0.0, 0.0, 1200.0, 1920.0, 0.0), (-5.0, 7.0, 90.0, 90.0, 20.0), (40.0, 1800.0, 400.0, 150.0, 40.0)] {
        for color in [pack(255, 255, 255, 255), rgba(20, 200, 90, 120)] {
            let (mut a, mut b) = (bg.clone(), bg.clone());
            surface(&mut a).fill_rrect(x, y, w, h, r, color);
            old_rrect(&mut surface(&mut b), x, y, w, h, r, color);
            assert!(a == b, "rrect {x} {y} {w} {h} {r}");
        }
    }
    for radius in [1, 4, 17, 60] {
        let mut a = Image { w: 97, h: 61, px: noise(&mut rng, 97 * 61) };
        let mut b = a.clone();
        a.blur(radius);
        old_blur(&mut b, radius);
        assert!(a == b, "blur {radius}");
    }
    println!("outputs identical to the previous implementations\n");

    let mut dst = bg.clone();
    let mut dst2 = bg.clone();
    for (label, w, h, r) in [("rrect icon 120", 120.0f32, 120.0f32, 27.0f32), ("rrect dock 1000x150", 1000.0, 150.0, 40.0), ("rrect card 1100x600", 1100.0, 600.0, 28.0), ("rrect full 1200x1920", 1160.0, 1850.0, 0.0)] {
        for (cl, color) in [("opaque", pack(40, 90, 200, 255)), ("alpha", rgba(40, 90, 200, 120))] {
            let iters = if w * h > 1e6 { 20 } else { 400 };
            report(
                &format!("{label} {cl}"),
                (w * h) as usize,
                iters,
                || old_rrect(&mut surface(&mut dst), 20.0, 30.0, black_box(w), h, r, color),
                || surface(&mut dst2).fill_rrect(20.0, 30.0, black_box(w), h, r, color),
            );
        }
    }

    for (label, iw, ih) in [("blit 120x120", 120usize, 120usize), ("blit 1100x600", 1100, 600), ("blit full", W, H)] {
        let img = Image { w: iw, h: ih, px: noise(&mut rng, iw * ih) };
        let opaque = Image { w: iw, h: ih, px: img.px.iter().map(|p| p | 0xff00_0000).collect() };
        let iters = if iw * ih > 1_000_000 { 20 } else { 300 };
        for (name, im, t, a) in [("noisy", &img, 255, 255), ("opaque", &opaque, 255, 255), ("noisy a=140", &img, 255, 140), ("opaque a=140", &opaque, 255, 140), ("noisy tint", &img, 180, 255)] {
            report(&format!("{label} {name}"), iw * ih, iters, || old_blit(&mut surface(&mut dst), im, 0, 0, t, a), || surface(&mut dst2).blit(im, 0, 0, t, a));
        }
    }

    let flat = pack(10, 20, 30, 255);
    report("fill_rect full", W * H, 50, || dst.iter_mut().for_each(|p| *p = black_box(flat)), || surface(&mut dst2).fill_rect(0, 0, W, H, black_box(flat)));

    for radius in [4usize, 16] {
        let base = Image { w: 600, h: 960, px: noise(&mut rng, 600 * 960) };
        let (mut a, mut b) = (base.clone(), base.clone());
        report(&format!("blur r={radius} 600x960"), 600 * 960, 3, || old_blur(&mut a, radius), || b.blur(radius));
    }
}
