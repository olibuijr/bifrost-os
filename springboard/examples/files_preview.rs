//! Renders the Files app's screens to target/files-*.ppm on the host:
//! locations, a listing (list and grid), a push half way, the context
//! menu, the delete and rename alerts, the text, image and info previews
//! and the Move to… picker. The listing comes from a tree built under
//! target/files-preview-tree.
use springboard::files::menu::Action;
use springboard::files::pages::{self, LocationsPage, CONTENT_TOP_PT, NAV_MID_PT, ROW_PT};
use springboard::files::{fs, Files, LONG_PRESS_MS};
use springboard::font::Font;
use springboard::gfx::Surface;
use springboard::shell::keyboard::Out;
use springboard::shell::ui::Ui;
use springboard::shell::Event;
use std::fs as stdfs;
use std::path::PathBuf;

const W: usize = 1200;
const H: usize = 1920;

fn save(f: &mut Files, ui: &Ui, name: &str, t: f64) {
    let mut buf = vec![0u32; W * H];
    f.draw(&mut Surface { w: W, h: H, stride: W, px: &mut buf, y0: 0 }, ui, t);
    let mut ppm = format!("P6 {W} {H} 255\n").into_bytes();
    for p in buf {
        ppm.extend([p as u8, (p >> 8) as u8, (p >> 16) as u8]);
    }
    stdfs::write(format!("target/files-{name}.ppm"), ppm).unwrap();
}

fn tap(f: &mut Files, ui: &Ui, x: f32, y: f32, t: f64) {
    f.touch(ui, Event::Down(x, y), t);
    f.touch(ui, Event::Up(x, y), t + 30.0);
}

fn long_press(f: &mut Files, ui: &Ui, x: f32, y: f32, t: f64) {
    f.touch(ui, Event::Down(x, y), t);
    f.tick(ui, t + LONG_PRESS_MS + 10.0);
    f.touch(ui, Event::Up(x, y), t + LONG_PRESS_MS + 20.0);
}

fn pump(f: &mut Files, ui: &Ui) {
    let start = std::time::Instant::now();
    while start.elapsed().as_millis() < 1500 {
        if springboard::bare::sys::wait(&f.fds(), 50).contains(&true) {
            f.pump(ui);
        }
    }
}

fn settle(f: &mut Files, ui: &Ui, t: f64) {
    f.tick(ui, t + 1000.0);
}

fn row(ui: &Ui, k: usize) -> (f32, f32) {
    (600.0, ui.pt(CONTENT_TOP_PT) + (k as f32 + 0.5) * ui.pt(ROW_PT))
}

/// A PNG with a colour gradient, for the image preview.
fn gradient_png(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().unwrap();
    let data: Vec<u8> = (0..h).flat_map(|y| (0..w).flat_map(move |x| [(x * 255 / w) as u8, (y * 255 / h) as u8, 160])).collect();
    wr.write_image_data(&data).unwrap();
    drop(wr);
    out
}

fn tree() -> PathBuf {
    let dir = PathBuf::from("target/files-preview-tree");
    let _ = stdfs::remove_dir_all(&dir);
    let home = dir.join("home");
    for d in ["Documents", "Movies", "Screenshots", "Backups"] {
        stdfs::create_dir_all(home.join(d)).unwrap();
    }
    stdfs::write(home.join("Documents/notes.md"), "# Notes\n\nShopping list for the week:\n\n- milk\n- bread\n- coffee\n").unwrap();
    stdfs::write(home.join("Documents/report.txt"), (1..=80).map(|i| format!("Line {i}: the quick brown fox jumps over the lazy dog\n")).collect::<String>()).unwrap();
    stdfs::write(home.join("README.md"), "# SpringBoard\n\nThe pure-Rust BifrOSt desktop shell for PC.\n\n## Build\n\n    cargo build --release\n").unwrap();
    stdfs::write(home.join("boot.log"), "[0.000] springboard: 1200x1920 @ 60Hz\n[0.120] wifi: scanning\n[1.400] wifi: connected\n").unwrap();
    stdfs::write(home.join("sunset.png"), gradient_png(640, 480)).unwrap();
    stdfs::write(home.join("Movies/clip.mp4"), vec![0u8; 2_300_000]).unwrap();
    stdfs::write(home.join("Backups/settings.ini"), "wallpaper=lake\n").unwrap();
    stdfs::write(home.join("firmware.bin"), vec![0u8, 1, 2, 3, 4, 5, 6, 7]).unwrap();
    stdfs::write(home.join("archive.tar.gz"), vec![0x1f, 0x8b, 8, 0]).unwrap();
    stdfs::write(home.join("song.m4a"), vec![0u8; 4_100_000]).unwrap();
    home
}

fn main() {
    let data: &'static [u8] = Box::leak(stdfs::read("assets/SFUI.ttf").unwrap().into_boxed_slice());
    let mono: &'static [u8] = Box::leak(stdfs::read("assets/SFUIMono.ttf").unwrap().into_boxed_slice());
    let ui = Ui::new(Font::parse(data).unwrap(), W as f32 / 768.0);
    let home = tree();
    let mut locations = fs::locations();
    locations[0].path = home.clone();
    locations[4].path = home.parent().unwrap().to_path_buf();
    let mut f = Files::with_locations(&ui, W as i32, H as i32, mono, locations).unwrap();
    f.set_wall(1_791_073_320, 0);
    save(&mut f, &ui, "locations", 0.0);

    tap(&mut f, &ui, 600.0, LocationsPage::row_top(&ui, 0) + 10.0, 1000.0);
    pump(&mut f, &ui);
    f.tick(&ui, 1150.0);
    save(&mut f, &ui, "push-mid", 1150.0);
    settle(&mut f, &ui, 1000.0);
    save(&mut f, &ui, "browse-list", 2000.0);

    tap(&mut f, &ui, pages::nav_right_x(&ui, W as i32, 0) + 10.0, ui.pt(NAV_MID_PT), 3000.0);
    save(&mut f, &ui, "browse-grid", 3000.0);
    tap(&mut f, &ui, pages::nav_right_x(&ui, W as i32, 0) + 10.0, ui.pt(NAV_MID_PT), 3100.0);

    // the context menu on README.md (entries: archive.tar.gz, Backups, boot.log, Documents, firmware.bin, Movies, README.md, ...)
    let (x, y) = row(&ui, 6);
    long_press(&mut f, &ui, x, y, 4000.0);
    save(&mut f, &ui, "menu", 4600.0);
    let bottom = H as f32 - ui.pt(28.0);
    let (mx, my) = f.menu_row(&ui, Action::Delete, bottom).unwrap();
    tap(&mut f, &ui, mx, my, 5000.0);
    save(&mut f, &ui, "delete-alert", 5000.0);
    let (bx, by) = f.alert_button_centre(&ui, 0).unwrap();
    tap(&mut f, &ui, bx, by, 5200.0);

    long_press(&mut f, &ui, x, y, 6000.0);
    let (mx, my) = f.menu_row(&ui, Action::Rename, bottom).unwrap();
    tap(&mut f, &ui, mx, my, 6600.0);
    settle(&mut f, &ui, 6600.0);
    save(&mut f, &ui, "rename", 7700.0);
    let (bx, by) = f.alert_button_centre(&ui, 0).unwrap();
    tap(&mut f, &ui, bx, by, 7800.0);
    settle(&mut f, &ui, 7800.0);

    long_press(&mut f, &ui, x, y, 8000.0);
    let (mx, my) = f.menu_row(&ui, Action::Share, bottom).unwrap();
    tap(&mut f, &ui, mx, my, 8600.0);
    save(&mut f, &ui, "share", 8600.0);
    tap(&mut f, &ui, 1100.0, 1850.0, 8700.0);

    // search
    let (sx, sy, _, sh) = pages::search_rect(&ui, W as i32, false);
    tap(&mut f, &ui, sx + 10.0, sy + sh / 2.0, 9000.0);
    f.key(&ui, Out::Text("re".into()), 9100.0);
    settle(&mut f, &ui, 9000.0);
    save(&mut f, &ui, "search", 10_000.0);
    let (sx, sy, sw, sh) = pages::search_rect(&ui, W as i32, true);
    tap(&mut f, &ui, sx + sw + 20.0, sy + sh / 2.0, 10_100.0);
    settle(&mut f, &ui, 10_100.0);

    // previews: README.md (text), sunset.png (image), firmware.bin (info)
    tap(&mut f, &ui, x, y, 11_000.0);
    pump(&mut f, &ui);
    settle(&mut f, &ui, 11_000.0);
    save(&mut f, &ui, "text", 12_000.0);
    tap(&mut f, &ui, 40.0, ui.pt(NAV_MID_PT), 12_100.0);
    settle(&mut f, &ui, 12_100.0);
    let (x, y) = row(&ui, 9);
    tap(&mut f, &ui, x, y, 13_000.0);
    pump(&mut f, &ui);
    settle(&mut f, &ui, 13_000.0);
    save(&mut f, &ui, "image", 14_000.0);
    tap(&mut f, &ui, 40.0, ui.pt(NAV_MID_PT), 14_100.0);
    settle(&mut f, &ui, 14_100.0);
    let (x, y) = row(&ui, 4);
    tap(&mut f, &ui, x, y, 15_000.0);
    pump(&mut f, &ui);
    settle(&mut f, &ui, 15_000.0);
    save(&mut f, &ui, "info", 16_000.0);
    tap(&mut f, &ui, 40.0, ui.pt(NAV_MID_PT), 16_100.0);
    settle(&mut f, &ui, 16_100.0);

    // Move to… picker, inside Documents
    long_press(&mut f, &ui, x, y, 17_000.0);
    let (mx, my) = f.menu_row(&ui, Action::MoveTo, bottom).unwrap();
    tap(&mut f, &ui, mx, my, 17_600.0);
    tap(&mut f, &ui, 600.0, LocationsPage::row_top(&ui, 0) + 10.0, 18_000.0);
    pump(&mut f, &ui);
    settle(&mut f, &ui, 18_000.0);
    let (dx, dy) = row(&ui, 3);
    tap(&mut f, &ui, dx, dy, 19_000.0);
    pump(&mut f, &ui);
    settle(&mut f, &ui, 19_000.0);
    save(&mut f, &ui, "picker", 20_000.0);

    // leave the picker (Back, Back, Cancel), then open Documents for its breadcrumb
    for i in 0..3 {
        tap(&mut f, &ui, 40.0, ui.pt(NAV_MID_PT), 20_100.0 + i as f64 * 100.0);
        settle(&mut f, &ui, 20_100.0 + i as f64 * 100.0);
    }
    tap(&mut f, &ui, dx, dy, 21_000.0);
    pump(&mut f, &ui);
    settle(&mut f, &ui, 21_000.0);
    save(&mut f, &ui, "subfolder", 22_000.0);
    println!("wrote target/files-*.ppm");
}
