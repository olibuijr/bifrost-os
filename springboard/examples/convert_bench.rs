//! Host benchmark of the video thread's pixel path: 1280x720 NV12 to a
//! 1200x675 RGBA frame (the 16:9 fit on the 1200-wide panel).
//!
//! `cargo run --release --example convert_bench [threads]`

use springboard::media::convert::{fit, nv12_to_rgba, Nv12, BT709};

fn main() {
    let threads: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(3);
    let (w, h) = (1280usize, 720usize);
    let y: Vec<u8> = (0..w * h).map(|i| (i % 251) as u8).collect();
    let uv: Vec<u8> = (0..w * h / 2).map(|i| (i % 253) as u8).collect();
    let (ow, oh) = fit(w as u32, h as u32, 1200, 1920);
    let n = 120;
    let t0 = std::time::Instant::now();
    let mut sink = 0u64;
    for _ in 0..n {
        let img = nv12_to_rgba(Nv12 { y: &y, uv: &uv, stride: w, w, h }, ow, oh, BT709, threads);
        sink = sink.wrapping_add(img.px[img.px.len() / 2] as u64);
    }
    let dt = t0.elapsed().as_secs_f64();
    println!("{w}x{h} -> {ow}x{oh}, {threads} threads: {:.2} ms/frame ({:.0} fps) [{sink}]", dt / n as f64 * 1000.0, n as f64 / dt);
}
