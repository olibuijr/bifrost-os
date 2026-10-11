//! Renders a home page swipe frame by frame, once repainting whole damaged rows (what the shell
//! did before) and once cutting at the damaged rectangles, and prints ms/frame and the pixel
//! bytes each frame writes. Run with `cargo run --release --example bench_frames [frames]`.
use springboard::font::Font;
use springboard::gfx::{squircle_mask, threads, Damage, Image, Surface};
use springboard::layout::Layout;
use springboard::pager::Pager;
use springboard::scene::{dock_background, Chrome, Scene, Tile};
use springboard::shell::ui::{Ui, Weight, WHITE};
use std::fs;
use std::time::Instant;

fn load(path: &str) -> Image {
    Image::from_raw(&fs::read(path).unwrap_or_else(|_| panic!("{path}"))).unwrap()
}

fn tile(ui: &Ui, l: &Layout, icon: &str, label: &str) -> Tile {
    let size = l.icon as usize;
    let mut img = load(&format!("assets/icons/{icon}.raw")).resized(size, size);
    img.apply_mask(&squircle_mask(size));
    Tile { icon: img, label: ui.styled(&ui.fit(label, 12.0, l.label_w as f32), 12.0, Weight::Regular, WHITE, true) }
}

/// One swipe of `n` frames: down, drag left most of a page, release and settle.
fn drag(pager: &mut Pager, frame: usize, n: usize, w: f32) {
    let t = frame as f64 * 16.0;
    let last = n * 3 / 4;
    match frame {
        0 => pager.down(w - 100.0, 800.0, t, None),
        f if f < last => pager.moved(w - 100.0 - w * 0.6 * f as f32 / last as f32, 800.0, t),
        f if f == last => {
            pager.up(w * 0.4, 800.0, t, None);
        }
        _ => {}
    }
    pager.tick(t);
}

fn main() {
    let n: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(120);
    let (w, h) = (1200usize, 1920usize);
    let ids = [
        ("bifrost.files", "Files"),
        ("bifrost.clock", "Clock"),
        ("bifrost.messages", "Hello World"),
        ("bifrost.settings", "Settings"),
        ("bifrost.camera", "Camera"),
        ("bifrost.photos", "Photos"),
        ("bifrost.appstore", "App Store"),
        ("ws.hbang.Terminal", "Terminal"),
    ];
    let data: &'static [u8] = Box::leak(fs::read("assets/SFUI.ttf").unwrap().into_boxed_slice());
    let l = Layout::new(w as i32, h as i32, 4);
    let ui = Ui::new(Font::parse(data).unwrap(), l.s);
    let wp = load("assets/wallpaper.raw");
    let dock: Vec<Tile> = ids[..4].iter().map(|(i, t)| tile(&ui, &l, i, t)).collect();
    let grid: Vec<Tile> = (0..40).map(|k| tile(&ui, &l, ids[k % ids.len()].0, ids[k % ids.len()].1)).collect();
    let chrome = Chrome::default();
    let bg = dock_background(wp.view(), &l);
    for clipped in [false, true] {
        let mut pager = Pager::new(2, w as f32);
        let mut buf = vec![0u32; w * h];
        let (mut damage, mut rows, mut prev) = (Damage::new(w, h), Vec::new(), None);
        let (mut spent, mut bytes, mut drawn) = (0.0, 0usize, 0usize);
        for f in 0..n {
            drag(&mut pager, f, n, w as f32);
            let scene = Scene { layout: &l, wallpaper: wp.view(), dock_bg: bg.as_ref(), dock: &dock, grid: &grid, pager: &pager, chrome: &chrome, today: None, extras: None };
            damage.clear();
            scene.damage(prev, &mut damage);
            prev = Some(scene.state());
            damage.rows(&mut rows);
            let t = Instant::now();
            let mut s = Surface { w, h, stride: w, px: &mut buf, y0: 0 };
            if clipped && scene.can_clip() && !damage.is_all() {
                let areas = damage.areas();
                s.draw_rows(&rows, threads(), &|s| scene.draw_clipped(s, areas));
                bytes += 4 * damage.pixels();
            } else {
                s.draw_rows(&rows, threads(), &|s| scene.draw(s));
                bytes += 4 * w * rows.iter().map(|&(a, b)| b - a).sum::<usize>();
            }
            spent += t.elapsed().as_secs_f64() * 1000.0;
            drawn += !damage.is_empty() as usize;
        }
        let d = drawn.max(1) as f64;
        println!("{:8}: {:.2} ms/frame, {:.2} MB written/frame over {drawn} drawn frames", if clipped { "clipped" } else { "rows" }, spent / d, bytes as f64 / d / 1e6);
    }
}
