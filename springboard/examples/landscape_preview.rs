//! Renders the shell in landscape — lock screen, home, Control Center,
//! switcher, Terminal with the wide keyboard, Settings, the letterboxed
//! Clock and a frame of the turn animation — to target/landscape-*.ppm,
//! and times the rotation pipeline on the host:
//! `cargo run --release --example landscape_preview`.
use springboard::gfx::rotate::{rotate_areas, rotated, scale_into, Orientation, Rotation};
use springboard::gfx::{threads, Area, Image, Surface, View};
use springboard::shell::rotate::TURN_MS;
use springboard::shell::{AppId, Event, Info, Shell};
use std::fs;
use std::time::Instant;

fn save(sh: &mut Shell, t: f64, name: &str) -> Image {
    let (w, h) = sh.size();
    let (w, h) = (w as usize, h as usize);
    let mut img = Image::new(w, h);
    let start = Instant::now();
    sh.draw(&mut Surface { w, h, stride: w, px: &mut img.px, y0: 0 }, t);
    eprintln!("{name}: draw {:?}", start.elapsed());
    write_ppm(&img, &format!("target/landscape-{name}.ppm"));
    img
}

fn write_ppm(img: &Image, path: &str) {
    let mut ppm = format!("P6 {} {} 255\n", img.w, img.h).into_bytes();
    for &p in &img.px {
        ppm.extend([p as u8, (p >> 8) as u8, (p >> 16) as u8]);
    }
    fs::write(path, ppm).unwrap();
}

fn swipe(sh: &mut Shell, x: f32, y0: f32, y1: f32, t: f64, ms: f64, pause: f64) -> f64 {
    sh.event(Event::Down(x, y0), t);
    for i in 1..=10 {
        sh.event(Event::Move(x, y0 + (y1 - y0) * i as f32 / 10.0), t + ms * i as f64 / 10.0);
    }
    sh.event(Event::Up(x, y1), t + ms + pause);
    t + ms + pause
}

fn tap(sh: &mut Shell, x: f32, y: f32, t: f64) {
    sh.event(Event::Down(x, y), t);
    sh.event(Event::Up(x, y), t + 30.0);
}

fn main() {
    let leak = |p: &str| -> &'static [u8] { Box::leak(fs::read(p).unwrap().into_boxed_slice()) };
    let font = springboard::font::Font::parse(leak("assets/SFUI.ttf")).unwrap();
    let wallpaper = View::from_raw(leak("assets/wallpaper.raw")).unwrap();
    let icon = |name: &str, size: usize| fs::read(format!("assets/icons/{name}.raw")).ok().and_then(|b| Image::from_raw(&b)).map(|i| i.resized(size, size));
    let info = Info { hour: 23, min: 32, wday: 6, mday: 3, mon: 9, battery: 61, online: true, wifi_bars: 3, wifi: "WIFI".into(), wifi_on: true, brightness: 0.55, ..Info::default() };
    let mut sh = Shell::new(1200, 1920, font, leak("assets/SFUIMono.ttf"), wallpaper, icon, info);
    let mut t = 1000.0;

    // lock screen, turned while locked
    let start = Instant::now();
    sh.set_orientation(Orientation::LandscapeLeft, t);
    eprintln!("set_orientation (snapshot, relayout, snapshot): {:?}", start.elapsed());
    save(&mut sh, t + TURN_MS * 0.5, "turn-mid");
    sh.tick(t + TURN_MS);
    t += TURN_MS;
    save(&mut sh, t, "lock");
    // unlock
    t = swipe(&mut sh, 960.0, 1100.0, 300.0, t, 100.0, 0.0);
    sh.tick(t + 1000.0);
    t += 1000.0;
    save(&mut sh, t, "home");
    // Control Center
    t = swipe(&mut sh, 1800.0, 5.0, 600.0, t, 150.0, 0.0);
    sh.tick(t + 1000.0);
    t += 1000.0;
    save(&mut sh, t, "cc");
    tap(&mut sh, 300.0, 900.0, t);
    sh.tick(t + 1000.0);
    t += 1000.0;
    // Settings
    let dock = |sh: &Shell, app: AppId| {
        let i = springboard::shell::APPS.iter().position(|a| a.id == app).unwrap();
        let (dock_tiles, _) = sh.tiles();
        assert!(i < dock_tiles.len());
        i
    };
    let settings_i = dock(&sh, AppId::Settings);
    let r = springboard::layout::Layout::new(1920, 1200, 3).dock_icons[settings_i];
    tap(&mut sh, r.x as f32 + 20.0, r.y as f32 + 20.0, t);
    sh.tick(t + 1000.0);
    t += 1000.0;
    save(&mut sh, t, "settings");
    t = swipe(&mut sh, 960.0, 1195.0, 700.0, t, 80.0, 0.0);
    sh.tick(t + 1000.0);
    t += 1000.0;
    // Clock, letterboxed
    let r = springboard::layout::Layout::new(1920, 1200, 3).dock_icons[dock(&sh, AppId::Terminal)];
    tap(&mut sh, r.x as f32 + 20.0, r.y as f32 + 20.0, t);
    sh.tick(t + 1000.0);
    t += 1000.0;
    save(&mut sh, t, "clock");
    t = swipe(&mut sh, 960.0, 1195.0, 700.0, t, 80.0, 0.0);
    sh.tick(t + 1000.0);
    t += 1000.0;
    // Terminal with the wide keyboard
    sh.set_terminal_command(&["/bin/sh"]);
    let r = springboard::layout::Layout::new(1920, 1200, 3).dock_icons[dock(&sh, AppId::Terminal)];
    tap(&mut sh, r.x as f32 + 20.0, r.y as f32 + 20.0, t);
    sh.tick(t + 1000.0);
    t += 1000.0;
    save(&mut sh, t, "terminal");
    // switcher: lift and pause
    sh.event(Event::Down(960.0, 1195.0), t);
    for i in 1..=10 {
        sh.event(Event::Move(960.0, 1195.0 - 50.0 * i as f32), t + 30.0 * i as f64);
    }
    sh.event(Event::Up(960.0, 695.0), t + 600.0);
    sh.tick(t + 1600.0);
    t += 1600.0;
    save(&mut sh, t, "switcher");

    // ---- costs
    let full = Area { x0: 0, y0: 0, x1: 1920, y1: 1200 };
    let frame = Image { w: 1920, h: 1200, px: (0..1920 * 1200).map(|i| i as u32 | 0xff00_0000).collect() };
    let mut panel = vec![0u32; 1200 * 1920];
    for rot in [Rotation::Ccw, Rotation::Cw, Rotation::Half] {
        for n in [1, threads()] {
            let runs = 20;
            let start = Instant::now();
            for _ in 0..runs {
                rotate_areas(&frame.px, 1920, 1200, &mut panel, rot, &[full], n);
            }
            eprintln!("rotate {rot:?} full frame, {n} thread(s): {:.2} ms", start.elapsed().as_secs_f64() * 1e3 / runs as f64);
        }
    }
    let status = Area { x0: 0, y0: 0, x1: 1920, y1: 31 };
    let start = Instant::now();
    for _ in 0..200 {
        rotate_areas(&frame.px, 1920, 1200, &mut panel, Rotation::Ccw, &[status], threads());
    }
    eprintln!("rotate Ccw status strip (1920x31): {:.3} ms", start.elapsed().as_secs_f64() * 1e3 / 200.0);
    assert_eq!(panel, rotated(&frame, Rotation::Ccw).px.iter().enumerate().map(|(i, &p)| if i % 1200 < 31 { p } else { panel[i] }).collect::<Vec<_>>());
    let portrait = Image { w: 1200, h: 1920, px: vec![0xff80_4020; 1200 * 1920] };
    let mut out = vec![0u32; 1920 * 1200];
    let start = Instant::now();
    for _ in 0..20 {
        scale_into(&mut Surface { w: 1920, h: 1200, stride: 1920, px: &mut out, y0: 0 }, &portrait, 585, 0, 750, 1200);
    }
    eprintln!("letterbox scale 1200x1920 -> 750x1200, {} threads: {:.2} ms", threads(), start.elapsed().as_secs_f64() * 1e3 / 20.0);
    let old = Image { w: 1200, h: 1920, px: vec![0xff20_4080; 1200 * 1920] };
    let start = Instant::now();
    for i in 0..20 {
        springboard::gfx::rotate::draw_turn(&mut Surface { w: 1920, h: 1200, stride: 1920, px: &mut out, y0: 0 }, &old, &frame, Rotation::Cw, i as f32 / 20.0);
    }
    eprintln!("turn animation frame, {} threads: {:.2} ms", threads(), start.elapsed().as_secs_f64() * 1e3 / 20.0);
    for k in [0.0f32, 0.5, 1.0] {
        let start = Instant::now();
        for _ in 0..20 {
            springboard::gfx::rotate::draw_turn(&mut Surface { w: 1920, h: 1200, stride: 1920, px: &mut out, y0: 0 }, &old, &frame, Rotation::Cw, k);
        }
        eprintln!("turn frame at k={k}: {:.2} ms", start.elapsed().as_secs_f64() * 1e3 / 20.0);
    }
    eprintln!("threads: {}", threads());
}
