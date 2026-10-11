//! `cargo run -p bifrost-daw --example icon -- icon.png`: renders the DAW app icon (180 px).
//! A dark squircle with a piano-roll motif: four coloured note bars.
fn main() {
    let n = 180usize;
    let mut px = vec![0u32; n * n];
    let bars = [(24usize, 40usize, 22usize, [10u8, 132, 255]), (60, 96, 22, [48, 209, 88]), (96, 62, 22, [255, 159, 10]), (124, 118, 22, [255, 69, 58])];
    for y in 0..n {
        for x in 0..n {
            // dark vertical gradient in the corners, rounded corners as transparency-free background
            let t = y as f32 / n as f32;
            let base = [(22.0 + 10.0 * t) as u8, (22.0 + 8.0 * t) as u8, (30.0 + 14.0 * t) as u8];
            let mut c = base;
            for (bx, by, len, col) in bars {
                let x0 = bx;
                let x1 = bx + len;
                let y0 = 60 + by / 2;
                if x >= x0 && x < x1 && y >= y0 && y < y0 + 12 {
                    c = col;
                }
            }
            px[y * n + x] = c[0] as u32 | (c[1] as u32) << 8 | (c[2] as u32) << 16 | 255 << 24;
        }
    }
    let png = springboard::pngenc::encode_rgb(n, n, &px);
    std::fs::write(std::env::args().nth(1).expect("output path"), png).unwrap();
}
