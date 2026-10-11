//! Renders the Today View (home page -1) to target/today-*.ppm on the host:
//! fully in, halfway through the swipe and in edit mode. Convert with
//! `magick target/today-*.ppm`.
use springboard::font::Font;
use springboard::gfx::{squircle_mask, Image, Surface};
use springboard::layout::Layout;
use springboard::pager::Pager;
use springboard::scene::{dock_background, Chrome, Scene, Tile};
use springboard::shell::clock::tz;
use springboard::shell::prefs::Prefs;
use springboard::shell::today::{Sources, Today};
use springboard::shell::ui::{Ui, Weight, WHITE};
use springboard::shell::Info;
use std::fs;

const W: i32 = 1200;
const H: i32 = 1920;

fn load(path: &str) -> Image {
    Image::from_raw(&fs::read(path).unwrap_or_else(|_| panic!("{path}"))).unwrap()
}

fn save(buf: &[u32], name: &str) {
    let mut ppm = format!("P6 {W} {H} 255\n").into_bytes();
    for p in buf {
        ppm.extend([*p as u8, (p >> 8) as u8, (p >> 16) as u8]);
    }
    fs::write(format!("target/{name}.ppm"), ppm).unwrap();
}

fn tile(ui: &Ui, l: &Layout, icon: &str, label: &str) -> Tile {
    let size = l.icon as usize;
    let mut img = load(&format!("assets/icons/{icon}.raw")).resized(size, size);
    img.apply_mask(&squircle_mask(size));
    Tile { icon: img, label: ui.styled(&ui.fit(label, 12.0, l.label_w as f32), 12.0, Weight::Regular, WHITE, true) }
}

fn main() {
    let data: &'static [u8] = Box::leak(fs::read("assets/SFUI.ttf").unwrap().into_boxed_slice());
    let mono: &'static [u8] = Box::leak(fs::read("assets/SFUIMono.ttf").unwrap().into_boxed_slice());
    let l = Layout::new(W, H, 3);
    let ui = Ui::new(Font::parse(data).unwrap(), l.s);
    let wp = load("assets/wallpaper.raw");
    let dock: Vec<Tile> = [("bifrost.settings", "Settings"), ("bifrost.clock", "Clock"), ("is.olibuijr.term", "Term")].iter().map(|(i, n)| tile(&ui, &l, i, n)).collect();
    let grid: Vec<Tile> = [("is.olibuijr.deildu", "Deildu")].iter().map(|(i, n)| tile(&ui, &l, i, n)).collect();
    let chrome = Chrome {
        time: Some(ui.text("00:24", 12.0, Weight::Semibold, WHITE)),
        date: Some(ui.text("Sun 4 Oct", 12.0, Weight::Semibold, WHITE)),
        percent: Some(ui.text("47%", 12.0, Weight::Medium, WHITE)),
        battery: 47,
        charging: false,
        online: true,
        wifi_bars: 3,
        ..Default::default()
    };
    let bg = dock_background(wp.view(), &l);
    let info = Info { hour: 0, min: 24, wday: 0, mday: 4, mon: 9, battery: 47, online: true, wifi_on: true, wifi: "Olibuijr".into(), ip: "192.168.1.40".into(), wifi_bars: 3, ..Info::default() };
    let unix = tz::days_from_civil(2026, 10, 4) * 86_400 + 24 * 60;

    let mut today = Today::new(&ui, W, H, mono);
    today.set_wallpaper(wp.view());
    let mut prefs = Prefs::memory();
    prefs.set("deildu.last", "1|https://www.deildu.net/details.php?id=1|The.Film.2024.1080p.WEB-DL").unwrap();
    today.load_prefs(prefs);
    let lines = Some(vec!["$ uname -a".to_string(), "Linux localhost 4.19.191 aarch64".to_string(), "$ ".to_string()]);
    today.refresh(&ui, &Sources { info: &info, term: lines, unix }, 0.0);

    let mut pager = Pager::new(1, W as f32);
    pager.set_leading(true);
    let mut buf = vec![0u32; (W * H) as usize];
    let draw = |pager: &Pager, today: &Today, buf: &mut Vec<u32>| {
        let scene = Scene { layout: &l, wallpaper: wp.view(), dock_bg: bg.as_ref(), dock: &dock, grid: &grid, pager, chrome: &chrome, today: Some(today), extras: None };
        scene.draw(&mut Surface { w: W as usize, h: H as usize, stride: W as usize, px: buf, y0: 0 });
    };

    // halfway through the swipe
    pager.down(100.0, 900.0, 0.0, None);
    pager.moved(100.0 + W as f32 / 2.0, 900.0, 100.0);
    draw(&pager, &today, &mut buf);
    save(&buf, "today-half");

    // fully in
    pager.moved(100.0 + W as f32, 900.0, 200.0);
    pager.up(100.0 + W as f32, 900.0, 250.0, None);
    pager.tick(5000.0);
    draw(&pager, &today, &mut buf);
    save(&buf, "today");

    // edit mode, with the battery switched off and the clock dragged down a row
    let edit = today.edit_rect();
    let (ex, ey) = (edit.x as f32 + edit.w as f32 / 2.0, edit.y as f32 + edit.h as f32 / 2.0);
    today.down(ex, ey, 1000.0);
    today.up(ex, ey, 1030.0);
    let placed = today.placed();
    let battery = placed.iter().find(|p| p.kind == springboard::shell::widgets::Kind::Battery).unwrap().rect;
    let (bx, by) = (battery.x as f32 + 9.0, battery.y as f32 + 9.0);
    today.down(bx, by, 1100.0);
    today.up(bx, by, 1130.0);
    draw(&pager, &today, &mut buf);
    save(&buf, "today-edit");

}
