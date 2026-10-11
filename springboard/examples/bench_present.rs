//! Host micro-benchmark of the present path at the panel's 1200x1920: the ARGB -> XRGB copy-out
//! (`copy_into`) for full, 10 % and tiny damage, and the 90/270 degree turns, each against the
//! scalar code it replaced (kept below), with the output asserted byte-identical.
//!
//! `cargo run --release --example bench_present [threads]`

use springboard::bare::kms::copy_into;
use springboard::gfx::rotate::{rotate_areas, Rotation};
use springboard::gfx::{pool, threads, Area};
use std::time::Instant;

const W: usize = 1200;
const H: usize = 1920;

/// The previous copy-out: rows dealt to threads by `y % n`, scalar swap.
fn old_copy(frame: &[u32], w: usize, out: &mut [u32], pitch: usize, areas: &[Area], swap_rb: bool, n: usize) {
    struct Ptr(*mut u32);
    unsafe impl Sync for Ptr {}
    let map = Ptr(out.as_mut_ptr());
    let worker = |k: usize| {
        let map = &map;
        for a in areas {
            for y in (a.y0..a.y1).filter(|y| y % n == k) {
                let src = &frame[y * w + a.x0..y * w + a.x1];
                let dst = unsafe { std::slice::from_raw_parts_mut(map.0.add(y * pitch + a.x0), src.len()) };
                if swap_rb {
                    for (d, &s) in dst.iter_mut().zip(src) {
                        *d = (s & 0xff00_ff00) | (s & 0xff) << 16 | (s >> 16) & 0xff;
                    }
                } else {
                    dst.copy_from_slice(src);
                }
            }
        }
    };
    if n == 1 {
        return worker(0);
    }
    pool::run(n, &worker);
}

/// The previous quarter turn: 8x8 tiles through a scratch tile, bands on freshly spawned threads.
fn old_rotate(src: &[u32], sw: usize, sh: usize, dst: &mut [u32], rot: Rotation, n: usize) {
    const BLOCK: usize = 8;
    let (dw, _) = rot.size(sw, sh);
    let rows = sw; // a quarter turn has sw destination rows
    let per = rows.div_ceil(n);
    std::thread::scope(|sc| {
        let mut rest = &mut dst[..rows * dw];
        let mut top = 0;
        while top < rows {
            let bottom = (top + per).min(rows);
            let (mine, tail) = rest.split_at_mut((bottom - top) * dw);
            rest = tail;
            let py0 = top;
            sc.spawn(move || {
                let mut tile = [[0u32; BLOCK]; BLOCK];
                let mut py = py0;
                while py < bottom {
                    let m = BLOCK.min(bottom - py);
                    let xa = if rot == Rotation::Ccw { sw - py - m } else { py };
                    let mut yb = 0;
                    while yb < sh {
                        let k = BLOCK.min(sh - yb);
                        for (j, row) in tile.iter_mut().enumerate().take(k) {
                            row[..m].copy_from_slice(&src[(yb + j) * sw + xa..(yb + j) * sw + xa + m]);
                        }
                        for r in 0..m {
                            let base = (py + r - py0) * dw;
                            if rot == Rotation::Ccw {
                                let i = m - 1 - r;
                                for (j, o) in mine[base + yb..base + yb + k].iter_mut().enumerate() {
                                    *o = tile[j][i];
                                }
                            } else {
                                for (j, o) in mine[base + sh - yb - k..base + sh - yb].iter_mut().rev().enumerate() {
                                    *o = tile[j][r];
                                }
                            }
                        }
                        yb += k;
                    }
                    py += m;
                }
            });
            top = bottom;
        }
    });
}

/// Best of a few rounds, in seconds per call.
fn time(mut f: impl FnMut()) -> f64 {
    f();
    let mut best = f64::MAX;
    for _ in 0..5 {
        let iters = 20;
        let t = Instant::now();
        for _ in 0..iters {
            f();
        }
        best = best.min(t.elapsed().as_secs_f64() / iters as f64);
    }
    best
}

fn report(name: &str, bytes: usize, old: f64, new: f64) {
    let gbs = |s: f64| bytes as f64 / s / 1e9;
    println!("{name:<28} old {:>7.3} ms {:>6.2} GB/s | new {:>7.3} ms {:>6.2} GB/s | x{:.2}", old * 1e3, gbs(old), new * 1e3, gbs(new), old / new);
}

fn main() {
    let n: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or_else(threads);
    println!("{W}x{H}, {n} threads");
    let frame: Vec<u32> = (0..W * H).map(|i| (i as u32).wrapping_mul(0x9e37_79b1) | 0xff00_0000).collect();
    let cases = [
        ("full", Area { x0: 0, y0: 0, x1: W, y1: H }),
        ("10% (1200x192)", Area { x0: 0, y0: 600, x1: W, y1: 792 }),
        ("tiny (96x40)", Area { x0: 100, y0: 300, x1: 196, y1: 340 }),
    ];
    for swap in [true, false] {
        for (name, a) in &cases {
            let areas = [*a];
            let (mut want, mut got) = (vec![0u32; W * H], vec![0u32; W * H]);
            let old = time(|| old_copy(&frame, W, &mut want, W, &areas, swap, n));
            let new = time(|| copy_into(&frame, W, &mut got, W, &areas, swap, n));
            assert!(want == got, "copy {name} swap={swap}: output differs from the scalar code");
            report(&format!("copy {name} {}", if swap { "swap" } else { "plain" }), a.pixels() * 4, old, new);
        }
    }
    let all = Area { x0: 0, y0: 0, x1: W, y1: H };
    for (name, rot) in [("rotate 90 (cw)", Rotation::Cw), ("rotate 270 (ccw)", Rotation::Ccw)] {
        let (mut want, mut got) = (vec![0u32; W * H], vec![0u32; W * H]);
        let old = time(|| old_rotate(&frame, W, H, &mut want, rot, n));
        let new = time(|| rotate_areas(&frame, W, H, &mut got, rot, &[all], n));
        assert!(want == got, "{name}: output differs from the scalar code");
        report(name, W * H * 4, old, new);
    }
    println!("all outputs byte-identical to the old scalar code");
}
