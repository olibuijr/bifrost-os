//! Renders the lock screen and the home screen through the real Shell to
//! target/lock.ppm and target/home.ppm on the host (packed assets required).
use springboard::font::Font;
use springboard::gfx::{Image, Surface, View};
use springboard::shell::{Event, Info, Shell};
use std::fs;

fn leak(path: &str) -> &'static [u8] {
    Box::leak(fs::read(path).unwrap_or_else(|_| panic!("{path}")).into_boxed_slice())
}

fn save(sh: &mut Shell, path: &str, t: f64) {
    let (w, h) = (1200usize, 1920usize);
    let mut buf = vec![0u32; w * h];
    sh.draw(&mut Surface { w, h, stride: w, px: &mut buf, y0: 0 }, t);
    let mut ppm = format!("P6 {w} {h} 255\n").into_bytes();
    for p in buf {
        ppm.extend([p as u8, (p >> 8) as u8, (p >> 16) as u8]);
    }
    fs::write(path, ppm).unwrap();
}

fn main() {
    let font = Font::parse(leak("assets/SFUI.ttf")).unwrap();
    let wallpaper = View::from_raw(leak("assets/wallpaper.raw")).unwrap();
    let icon = |name: &str, size: usize| fs::read(format!("assets/icons/{name}.raw")).ok().and_then(|b| Image::from_raw(&b)).map(|i| i.resized(size, size));
    let info = Info { hour: 23, min: 34, wday: 6, mday: 3, mon: 9, battery: 61, online: true, wifi_bars: 3, brightness: 0.5, ..Info::default() };
    let mut sh = Shell::new(1200, 1920, font, leak("assets/SFUIMono.ttf"), wallpaper, icon, info);
    save(&mut sh, "target/lock.ppm", 0.0);
    sh.event(Event::Down(600.0, 1800.0), 0.0);
    sh.event(Event::Move(600.0, 900.0), 50.0);
    sh.event(Event::Up(600.0, 900.0), 100.0);
    sh.tick(1000.0);
    save(&mut sh, "target/home.ppm", 1000.0);
}
