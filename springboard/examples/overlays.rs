//! Renders Control Center, the app switcher and a mid go-home swipe to
//! target/cc.ppm, target/switcher.ppm and target/gesture.ppm on the host,
//! for comparison with the bifrost captures in ~/Pictures/tab/bifrost-ref.
use springboard::font::Font;
use springboard::gfx::{Image, Surface, View};
use springboard::layout::Layout;
use springboard::shell::{AppId, Event, Info, Shell};
use std::fs;

const W: usize = 1200;
const H: usize = 1920;

fn save(sh: &mut Shell, t: f64, path: &str) {
    let mut buf = vec![0u32; W * H];
    let start = std::time::Instant::now();
    sh.draw(&mut Surface { w: W, h: H, stride: W, px: &mut buf, y0: 0 }, t);
    eprintln!("{path}: {:?}", start.elapsed());
    let mut ppm = format!("P6 {W} {H} 255\n").into_bytes();
    for p in buf {
        ppm.extend([p as u8, (p >> 8) as u8, (p >> 16) as u8]);
    }
    fs::write(path, ppm).unwrap();
}

/// A finger moving from y0 to y1 in ten steps over `ms`, resting `pause` ms before lifting.
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
    let font = Font::parse(leak("assets/SFUI.ttf")).unwrap();
    let wallpaper = View::from_raw(leak("assets/wallpaper.raw")).unwrap();
    let icon = |name: &str, size: usize| fs::read(format!("assets/icons/{name}.raw")).ok().and_then(|b| Image::from_raw(&b)).map(|i| i.resized(size, size));
    let info = Info { hour: 23, min: 32, wday: 6, mday: 3, mon: 9, battery: 61, online: true, wifi_bars: 3, wifi: "WIFI".into(), wifi_on: true, brightness: 0.55, ..Info::default() };
    let mut sh = Shell::new(W as i32, H as i32, font, leak("assets/SFUIMono.ttf"), wallpaper, icon, info);
    // unlock
    let mut t = swipe(&mut sh, 600.0, 1800.0, 600.0, 0.0, 100.0, 0.0);
    sh.tick(t + 1000.0);
    // Control Center over the home screen
    t = swipe(&mut sh, 1100.0, 5.0, 700.0, t + 1000.0, 150.0, 0.0);
    sh.tick(t + 1000.0);
    save(&mut sh, t + 1000.0, "target/cc.ppm");
    tap(&mut sh, 200.0, 1200.0, t + 1100.0);
    sh.tick(t + 2000.0);
    t += 2000.0;
    // open Settings and Clock so the switcher has cards
    // all three apps sit in the dock, in APPS order
    let layout = Layout::new(W as i32, H as i32, 3);
    let dock = |app: AppId| layout.dock_icons[springboard::shell::APPS.iter().position(|a| a.id == app).unwrap()];
    for app in [AppId::Settings, AppId::Files] {
        let r = dock(app);
        tap(&mut sh, r.x as f32 + 20.0, r.y as f32 + 20.0, t);
        sh.tick(t + 1000.0);
        t = swipe(&mut sh, 600.0, 1915.0, 1500.0, t + 1000.0, 80.0, 0.0);
        sh.tick(t + 1000.0);
        t += 1000.0;
    }
    // open Clock, drag it half way up for the follow-the-finger frame, then rest for the switcher
    let r = dock(AppId::Terminal);
    tap(&mut sh, r.x as f32 + 20.0, r.y as f32 + 20.0, t);
    sh.tick(t + 1000.0);
    t += 1000.0;
    sh.event(Event::Down(600.0, 1915.0), t);
    sh.event(Event::Move(640.0, 1500.0), t + 100.0);
    save(&mut sh, t + 100.0, "target/gesture.ppm");
    sh.event(Event::Move(650.0, 1200.0), t + 200.0);
    sh.event(Event::Up(650.0, 1200.0), t + 500.0);
    sh.tick(t + 1500.0);
    save(&mut sh, t + 1500.0, "target/switcher.ppm");
}
