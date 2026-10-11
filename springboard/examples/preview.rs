//! Renders a full home screen (dock, two pages of icons, status bar) to
//! target/preview.ppm on the host, using the shell's own text styles.
use springboard::font::Font;
use springboard::gfx::{squircle_mask, Image, Surface};
use springboard::layout::Layout;
use springboard::pager::Pager;
use springboard::scene::{dock_background, Chrome, Scene, Tile};
use springboard::shell::ui::{Ui, Weight, WHITE};
use std::fs;

fn load(path: &str) -> Image {
    Image::from_raw(&fs::read(path).unwrap_or_else(|_| panic!("{path}"))).unwrap()
}

fn tile(ui: &Ui, l: &Layout, icon: &str, label: &str) -> Tile {
    let size = l.icon as usize;
    let mut img = load(&format!("assets/icons/{icon}.raw")).resized(size, size);
    img.apply_mask(&squircle_mask(size));
    Tile { icon: img, label: ui.styled(&ui.fit(label, 12.0, l.label_w as f32), 12.0, Weight::Regular, WHITE, true) }
}

fn main() {
    let (w, h) = (1200, 1920);
    let dock_ids = [("bifrost.browser", "Browser"), ("bifrost.messages", "Messages"), ("bifrost.settings", "Settings"), ("bifrost.files", "Files")];
    let grid_ids = [
        ("bifrost.files", "Files"),
        ("bifrost.clock", "Clock"),
        ("bifrost.messages", "Hello World"),
        ("bifrost.settings", "Settings"),
        ("bifrost.camera", "Camera"),
        ("bifrost.photos", "Photos"),
        ("bifrost.appstore", "App Store"),
        ("ws.hbang.Terminal", "Terminal"),
        ("is.olibuijr.deildu", "Deildu"),
        ("is.olibuijr.ruv", "RÚV"),
        ("is.olibuijr.spilarinn", "Spilarinn"),
    ];
    let data: &'static [u8] = Box::leak(fs::read("assets/SFUI.ttf").unwrap().into_boxed_slice());
    let l = Layout::new(w, h, dock_ids.len());
    let ui = Ui::new(Font::parse(data).unwrap(), l.s);
    let wp = load("assets/wallpaper.raw");
    let dock: Vec<Tile> = dock_ids.iter().map(|(i, n)| tile(&ui, &l, i, n)).collect();
    let grid: Vec<Tile> = grid_ids.iter().map(|(i, n)| tile(&ui, &l, i, n)).collect();
    let pager = Pager::new(2, w as f32);
    let chrome = Chrome {
        time: Some(ui.text("23:34", 12.0, Weight::Semibold, WHITE)),
        date: Some(ui.text("Sat 3 Oct", 12.0, Weight::Semibold, WHITE)),
        percent: Some(ui.text("61%", 12.0, Weight::Medium, WHITE)),
        battery: 61,
        charging: false,
        online: true,
        wifi_bars: 3,
        ..Default::default()
    };
    let bg = dock_background(wp.view(), &l);
    let scene = Scene { layout: &l, wallpaper: wp.view(), dock_bg: bg.as_ref(), dock: &dock, grid: &grid, pager: &pager, chrome: &chrome, today: None, extras: None };
    let mut buf = vec![0u32; (w * h) as usize];
    let t = std::time::Instant::now();
    scene.draw(&mut Surface { w: w as usize, h: h as usize, stride: w as usize, px: &mut buf, y0: 0 });
    eprintln!("frame {:?}", t.elapsed());
    let mut ppm = format!("P6 {w} {h} 255\n").into_bytes();
    for p in buf {
        ppm.extend([p as u8, (p >> 8) as u8, (p >> 16) as u8]);
    }
    fs::write("target/preview.ppm", ppm).unwrap();
}
