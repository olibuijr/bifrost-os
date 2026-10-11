//! Host benchmark: 300 frames of a home-page swipe with two pages of icons,
//! first redrawn and copied out in full every frame (the old pipeline), then
//! through damage tracking (only the moved tiles are drawn and copied) on
//! `gfx::threads()` threads. Run with `cargo run --release --example bench`.
use springboard::font::{Font, Style};
use springboard::gfx::{pack, squircle_mask, threads, Damage, Image, Surface};
use springboard::layout::Layout;
use springboard::pager::Pager;
use springboard::scene::{dock_background, Chrome, HomeState, Scene, Tile};
use std::fs;
use std::time::Instant;

const FRAMES: usize = 300;

fn load(path: &str) -> Image {
    Image::from_raw(&fs::read(path).unwrap_or_else(|_| panic!("{path}"))).unwrap()
}

fn text(f: &Font, s: &str, px: f32, embolden: f32, shadow: bool) -> Image {
    f.render(s, &Style { px, embolden, color: pack(255, 255, 255, 255), shadow })
}

fn tile(f: &Font, icon: &str, label: &str, size: usize) -> Tile {
    let mut img = load(&format!("assets/icons/{icon}.raw")).resized(size, size);
    img.apply_mask(&squircle_mask(size));
    Tile { icon: img, label: text(f, label, 18.75, 0.0, true) }
}

/// Drives the pager through a slow half-page drag and the snap that follows.
fn swipe(pager: &mut Pager, i: usize) {
    let t = i as f64 * 16.7;
    match i {
        0 => pager.down(1000.0, 800.0, t, None),
        1..=199 => pager.moved(1000.0 - 3.0 * i as f32, 800.0, t),
        200 => {
            pager.up(400.0, 800.0, t, None);
        }
        _ => {}
    }
    pager.tick(t);
}

fn main() {
    let (w, h) = (1200usize, 1920usize);
    let data = fs::read("assets/SFUI.ttf").unwrap();
    let f = Font::parse(&data).unwrap();
    let l = Layout::new(w as i32, h as i32, 4);
    let wp = load("assets/wallpaper.raw");
    let icons = ["bifrost.clock", "bifrost.camera", "bifrost.photos", "bifrost.appstore", "ws.hbang.Terminal", "is.olibuijr.deildu", "is.olibuijr.ruv", "is.olibuijr.spilarinn"];
    let dock: Vec<Tile> = icons[..4].iter().map(|i| tile(&f, i, "Browser", l.icon as usize)).collect();
    let grid: Vec<Tile> = (0..40).map(|i| tile(&f, icons[i % icons.len()], "App Store", l.icon as usize)).collect();
    let chrome = Chrome { time: Some(text(&f, "21:53", 18.75, 0.6, false)), date: Some(text(&f, "Sat 3 Oct", 18.75, 0.6, false)), percent: Some(text(&f, "50%", 18.75, 0.4, false)), battery: 50, charging: true, online: true, wifi_bars: 2, ..Default::default() };
    let bg = dock_background(wp.view(), &l);
    let mut frame = vec![0u32; w * h];
    let mut scanout = vec![0u32; w * h];
    fn scene<'a>(l: &'a Layout, wp: &'a Image, bg: &'a Option<Image>, dock: &'a [Tile], grid: &'a [Tile], chrome: &'a Chrome, pager: &'a Pager) -> Scene<'a> {
        Scene { layout: l, wallpaper: wp.view(), dock_bg: bg.as_ref(), dock, grid, pager, chrome, today: None, extras: None }
    }

    let mut pager = Pager::new(2, w as f32);
    let (mut draw_ms, mut present_ms) = (0.0, 0.0);
    for i in 0..FRAMES {
        swipe(&mut pager, i);
        let scene = scene(&l, &wp, &bg, &dock, &grid, &chrome, &pager);
        let t0 = Instant::now();
        scene.draw(&mut Surface { w, h, stride: w, px: &mut frame, y0: 0 });
        let t1 = Instant::now();
        scanout.copy_from_slice(&frame);
        std::hint::black_box(&scanout);
        draw_ms += (t1 - t0).as_secs_f64() * 1e3;
        present_ms += t1.elapsed().as_secs_f64() * 1e3;
    }
    println!("full redraw, 1 thread:  draw {:.3} ms, present {:.3} ms per frame", draw_ms / FRAMES as f64, present_ms / FRAMES as f64);

    let mut pager = Pager::new(2, w as f32);
    let (mut draw_ms, mut present_ms, mut pixels) = (0.0, 0.0, 0usize);
    let mut damage = Damage::new(w, h);
    let mut rows = Vec::new();
    let mut prev: Option<HomeState> = None;
    for i in 0..FRAMES {
        swipe(&mut pager, i);
        let scene = scene(&l, &wp, &bg, &dock, &grid, &chrome, &pager);
        let t0 = Instant::now();
        damage.clear();
        scene.damage(prev, &mut damage);
        prev = Some(scene.state());
        damage.rows(&mut rows);
        Surface { w, h, stride: w, px: &mut frame, y0: 0 }.draw_rows(&rows, threads(), &|s| scene.draw(s));
        let t1 = Instant::now();
        for a in damage.areas() {
            for y in a.y0..a.y1 {
                scanout[y * w + a.x0..y * w + a.x1].copy_from_slice(&frame[y * w + a.x0..y * w + a.x1]);
            }
        }
        std::hint::black_box(&scanout);
        draw_ms += (t1 - t0).as_secs_f64() * 1e3;
        present_ms += t1.elapsed().as_secs_f64() * 1e3;
        pixels += damage.pixels();
    }
    println!("damage, {} threads:      draw {:.3} ms, present {:.3} ms per frame, {:.0}% of the frame", threads(), draw_ms / FRAMES as f64, present_ms / FRAMES as f64, 100.0 * pixels as f64 / (FRAMES * w * h) as f64);
}
