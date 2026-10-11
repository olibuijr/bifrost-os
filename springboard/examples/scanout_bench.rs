//! Host benchmark of the CPU copy-out that zero-copy scanout removes (OLI-695):
//! full-frame and typical-damage copies at the Tab A8 panel size (1920x1200),
//! with and without the red/blue swap, on 1..N threads.
//! Run with `cargo run --release --example scanout_bench`.
use springboard::bare::kms::copy_into;
use springboard::gfx::Area;
use std::time::Instant;

const W: usize = 1920;
const H: usize = 1200;
const RUNS: usize = 200;

fn time(frame: &[u32], out: &mut [u32], areas: &[Area], swap: bool, n: usize) -> f64 {
    copy_into(frame, W, out, W, areas, swap, n);
    let t = Instant::now();
    for _ in 0..RUNS {
        copy_into(frame, W, out, W, areas, swap, n);
    }
    t.elapsed().as_secs_f64() * 1000.0 / RUNS as f64
}

fn main() {
    let frame: Vec<u32> = (0..W * H).map(|i| i as u32 * 2654435761).collect();
    let mut out = vec![0u32; W * H];
    let cases = [
        ("full frame", vec![Area { x0: 0, y0: 0, x1: W, y1: H }]),
        ("half frame (swipe)", vec![Area { x0: 0, y0: 0, x1: W, y1: H / 2 }]),
        ("dock + status bar", vec![Area { x0: 0, y0: 0, x1: W, y1: 40 }, Area { x0: 300, y0: H - 220, x1: W - 300, y1: H }]),
        ("one tile", vec![Area { x0: 200, y0: 300, x1: 380, y1: 480 }]),
    ];
    let max = springboard::gfx::threads().max(1);
    println!("copy-out cost, ms per frame (budget 16.7 ms at 60 Hz); threads available: {max}");
    for (name, areas) in &cases {
        for swap in [true, false] {
            let row: Vec<String> = [1usize, 2, 4, max].iter().map(|&n| format!("{n}t {:.2}", time(&frame, &mut out, areas, swap, n))).collect();
            println!("{name:20} {}  {}", if swap { "swap" } else { "copy" }, row.join("  "));
        }
    }
}
