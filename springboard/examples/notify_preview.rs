//! Renders the notification screens through the real Shell on the host:
//! target/notify-lock.ppm (stacks under the clock), target/notify-banner.ppm
//! (a banner over the home screen with badges), target/notify-center.ppm
//! (Notification Center) and target/notify-center-empty.ppm, for comparison
//! with the bifrost captures in ~/Pictures/tab/bifrost-ref/notify.
use springboard::font::Font;
use springboard::gfx::{Image, Surface, View};
use springboard::shell::notify::Notification;
use springboard::shell::{AppId, Event, Info, Shell};
use std::fs;

const W: usize = 1200;
const H: usize = 1920;

fn save(sh: &mut Shell, t: f64, path: &str) {
    let mut buf = vec![0u32; W * H];
    sh.draw(&mut Surface { w: W, h: H, stride: W, px: &mut buf, y0: 0 }, t);
    let mut ppm = format!("P6 {W} {H} 255\n").into_bytes();
    for p in buf {
        ppm.extend([p as u8, (p >> 8) as u8, (p >> 16) as u8]);
    }
    fs::write(path, ppm).unwrap();
}

fn swipe(sh: &mut Shell, x: f32, y0: f32, y1: f32, t: f64) -> f64 {
    sh.event(Event::Down(x, y0), t);
    for i in 1..=10 {
        sh.event(Event::Move(x, y0 + (y1 - y0) * i as f32 / 10.0), t + 15.0 * i as f64);
    }
    sh.event(Event::Up(x, y1), t + 150.0);
    t + 150.0
}

fn main() {
    let leak = |p: &str| -> &'static [u8] { Box::leak(fs::read(p).unwrap().into_boxed_slice()) };
    let font = Font::parse(leak("assets/SFUI.ttf")).unwrap();
    let wallpaper = View::from_raw(leak("assets/wallpaper.raw")).unwrap();
    let icon = |name: &str, size: usize| fs::read(format!("assets/icons/{name}.raw")).ok().and_then(|b| Image::from_raw(&b)).map(|i| i.resized(size, size));
    let info = Info { hour: 0, min: 22, wday: 0, mday: 4, mon: 9, battery: 48, online: true, wifi_bars: 3, wifi: "WIFI".into(), wifi_on: true, brightness: 0.5, ..Info::default() };
    let mut sh = Shell::new(W as i32, H as i32, font, leak("assets/SFUIMono.ttf"), wallpaper, icon, info);
    let now = 1_696_372_920; // 2023-10-04 00:22 UTC
    sh.set_wall(now, 0, 0.0);
    sh.tick(0.0);
    // empty Notification Center first
    let mut t = swipe(&mut sh, 600.0, 1800.0, 600.0, 100.0);
    sh.tick(t + 1000.0);
    t = swipe(&mut sh, 300.0, 5.0, 700.0, t + 1000.0);
    sh.tick(t + 1000.0);
    save(&mut sh, t + 1000.0, "target/notify-center-empty.ppm");
    sh.event(Event::Down(600.0, 1700.0), t + 1100.0);
    sh.event(Event::Up(600.0, 1700.0), t + 1130.0);
    sh.tick(t + 2000.0);
    t += 2000.0;
    // notifications arrive at home: the last one shows as a banner
    sh.notify(Notification::new(AppId::Files, "Timer", "Your timer is done.", now - 25 * 60));
    sh.notify(Notification::new(AppId::Settings, "Low Battery", "20% battery remaining.", now - 3 * 3600));
    sh.notify(Notification::new(AppId::Deildu, "Ready to play", "Dune: Part Two (2024)", now - 120));
    sh.notify(Notification::new(AppId::Files, "Wake up", "07:30", now - 10));
    sh.tick(t + 1000.0);
    save(&mut sh, t + 1000.0, "target/notify-banner.ppm");
    sh.tick(t + 7000.0);
    t += 7000.0;
    // Notification Center with the groups expanded
    t = swipe(&mut sh, 300.0, 5.0, 700.0, t);
    sh.tick(t + 1000.0);
    save(&mut sh, t + 1000.0, "target/notify-center.ppm");
    // the lock screen with the stacks under the clock
    sh.event(Event::Power, t + 2000.0);
    sh.event(Event::Power, t + 2100.0);
    sh.tick(t + 2200.0);
    save(&mut sh, t + 2200.0, "target/notify-lock.ppm");
}
