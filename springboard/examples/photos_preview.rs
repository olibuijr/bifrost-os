//! Renders the Photos screens on the host from a folder of pictures:
//! `cargo run --example photos_preview -- <pictures dir> <out dir>` writes
//! grid.ppm, grid3.ppm, viewer.ppm, viewer-zoom.ppm, share.ppm,
//! delete.ppm, trash.ppm and shot.ppm (the screenshot thumbnail over the
//! home screen).
use springboard::font::Font;
use springboard::gfx::{Image, Surface};
use springboard::shell::photos::{content_top, Photos};
use springboard::shell::screenshot::{Overlay, FLASH_MS, SHRINK_MS};
use springboard::shell::ui::Ui;
use springboard::shell::Event;
use std::path::PathBuf;

fn save(buf: &[u32], w: usize, h: usize, path: &PathBuf) {
    let mut ppm = format!("P6 {w} {h} 255\n").into_bytes();
    for p in buf {
        ppm.extend([*p as u8, (p >> 8) as u8, (p >> 16) as u8]);
    }
    std::fs::write(path, ppm).unwrap();
}

fn pump_until(p: &mut Photos, ui: &Ui, done: impl Fn(&Photos) -> bool) {
    let start = std::time::Instant::now();
    while !done(p) && start.elapsed().as_secs() < 20 {
        springboard::bare::sys::wait(&p.fds(), 50);
        p.pump(ui);
    }
}

fn tap(p: &mut Photos, ui: &Ui, x: f32, y: f32, t: f64) {
    p.touch(ui, Event::Down(x, y), t);
    p.touch(ui, Event::Up(x, y), t + 30.0);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let pictures = PathBuf::from(args.get(1).expect("pictures dir"));
    let out = PathBuf::from(args.get(2).expect("out dir"));
    let (w, h) = (1200usize, 1920usize);
    let data: &'static [u8] = Box::leak(std::fs::read("assets/SFUI.ttf").unwrap().into_boxed_slice());
    let ui = Ui::new(Font::parse(data).unwrap(), w as f32 / 768.0);
    let dir = out.join("data");
    let _ = std::fs::remove_dir_all(&dir);
    let mut p = Photos::new(&ui, w as i32, h as i32, dir.clone(), vec![pictures], 0).unwrap();
    pump_until(&mut p, &ui, |p| !p.photos().is_empty());
    let mut buf = vec![0u32; w * h];
    let frame = |p: &mut Photos, name: &str, buf: &mut Vec<u32>| {
        p.draw(&mut Surface { w, h, stride: w, px: buf, y0: 0 }, &ui, 0.0);
        save(buf, w, h, &out.join(format!("{name}.ppm")));
    };
    let settled = |p: &mut Photos, ui: &Ui| {
        p.tick(ui, 0.0);
        pump_until(p, ui, |p| !p.describe().contains("photos_pending=") || p.describe().contains("photos_pending=0\n"));
    };
    settled(&mut p, &ui);
    frame(&mut p, "grid", &mut buf);
    tap(&mut p, &ui, w as f32 - ui.pt(16.0 + 22.0), ui.pt(42.0), 100.0);
    settled(&mut p, &ui);
    frame(&mut p, "grid3", &mut buf);
    tap(&mut p, &ui, w as f32 - ui.pt(16.0 + 74.0), ui.pt(42.0), 200.0);
    // the viewer on the first photo
    let (x, y, cw, ch) = (ui.pt(4.0), content_top(&ui) + ui.pt(44.0) + 4.0, 0.0, 0.0);
    let _ = (cw, ch);
    tap(&mut p, &ui, x + 20.0, y + 20.0, 1000.0);
    p.tick(&ui, 1100.0);
    pump_until(&mut p, &ui, |p| p.viewing().is_some());
    std::thread::sleep(std::time::Duration::from_millis(1500));
    p.pump(&ui);
    frame(&mut p, "viewer", &mut buf);
    tap(&mut p, &ui, 400.0, 900.0, 2000.0);
    tap(&mut p, &ui, 400.0, 900.0, 2100.0);
    p.tick(&ui, 2200.0);
    frame(&mut p, "viewer-zoom", &mut buf);
    tap(&mut p, &ui, 400.0, 900.0, 3000.0);
    tap(&mut p, &ui, 400.0, 900.0, 3100.0);
    p.tick(&ui, 3200.0);
    let bar_y = h as f32 - ui.pt(28.0) - ui.pt(25.0);
    tap(&mut p, &ui, w as f32 * 0.12, bar_y, 4000.0);
    frame(&mut p, "share", &mut buf);
    tap(&mut p, &ui, 50.0, 50.0, 4100.0);
    tap(&mut p, &ui, w as f32 * 0.88, bar_y, 5000.0);
    frame(&mut p, "delete", &mut buf);
    tap(&mut p, &ui, 50.0, 50.0, 5100.0);
    tap(&mut p, &ui, ui.pt(40.0), ui.pt(42.0), 6000.0);
    tap(&mut p, &ui, ui.pt(40.0), ui.pt(42.0), 7000.0);
    settled(&mut p, &ui);
    frame(&mut p, "trash", &mut buf);
    // the screenshot thumbnail resting over a frame of the grid
    tap(&mut p, &ui, ui.pt(40.0), ui.pt(42.0), 8000.0);
    settled(&mut p, &ui);
    p.draw(&mut Surface { w, h, stride: w, px: &mut buf, y0: 0 }, &ui, 0.0);
    let shot = Overlay::new(Image { w, h, px: buf.clone() }, PathBuf::from("/x.png"), ui.s, 0.0);
    shot.draw(&mut Surface { w, h, stride: w, px: &mut buf, y0: 0 }, FLASH_MS + SHRINK_MS + 100.0);
    save(&buf, w, h, &out.join("shot.ppm"));
    let _ = std::fs::remove_dir_all(&dir);
}
