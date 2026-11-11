//! Renders Search on the host: pulled down with the suggestions row
//! (target/quicksearch.ppm), mid-pull (target/quicksearch-pull.ppm) and with
//! "se" typed (target/quicksearch-se.ppm), for comparison with the bifrost
//! captures in ~/Pictures/tab/bifrost-ref/quicksearch. The files Search lists come from a folder of sample files made
//! here (`sample_home`), never from the home folder of whoever runs the preview.
use springboard::font::Font;
use springboard::gfx::{Image, Surface, View};
use springboard::shell::keyboard::Out;
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

/// A home folder of sample files in the temp dir, made afresh: what Search walks in this preview.
fn sample_home() -> std::path::PathBuf {
    let home = std::env::temp_dir().join("bifrost-quicksearch-preview-home");
    let _ = fs::remove_dir_all(&home);
    for (folder, name) in [("Documents", "Seasons report.txt"), ("Documents", "Budget.csv"), ("Pictures", "Seljalandsfoss.jpg"), ("Desktop", "Second draft.md"), ("Downloads", "Manual.pdf")] {
        fs::create_dir_all(home.join(folder)).unwrap();
        fs::write(home.join(folder).join(name), "sample").unwrap();
    }
    home
}

fn main() {
    // before anything reads it and before any thread starts: nothing in this process looks at the real home folder
    let home = sample_home();
    std::env::set_var("HOME", &home);
    let leak = |p: &str| -> &'static [u8] { Box::leak(fs::read(p).unwrap().into_boxed_slice()) };
    let font = Font::parse(leak("assets/SFUI.ttf")).unwrap();
    let wallpaper = View::from_raw(leak("assets/wallpaper.raw")).unwrap();
    let icon = |name: &str, size: usize| fs::read(format!("assets/icons/{name}.raw")).ok().and_then(|b| Image::from_raw(&b)).map(|i| i.resized(size, size));
    let info = Info { hour: 0, min: 24, wday: 0, mday: 4, mon: 9, battery: 47, online: true, wifi_bars: 3, wifi: "WIFI".into(), wifi_on: true, brightness: 0.55, ..Info::default() };
    let mut sh = Shell::new(W as i32, H as i32, font, leak("assets/SFUIMono.ttf"), wallpaper, icon, info);
    sh.set_search_home(home);
    let mut t = 0.0;
    sh.event(Event::Down(600.0, 1800.0), t);
    for i in 1..=10 {
        t += 10.0;
        sh.event(Event::Move(600.0, 1800.0 - 120.0 * i as f32), t);
    }
    sh.event(Event::Up(600.0, 600.0), t);
    sh.tick(t + 1000.0);
    t += 1000.0;
    for app in [AppId::Deildu, AppId::Terminal, AppId::Settings, AppId::Files] {
        sh.note_launch(app);
    }
    // the pull, frozen half way
    sh.event(Event::Down(600.0, 800.0), t);
    sh.event(Event::Move(600.0, 830.0), t + 30.0);
    sh.event(Event::Move(600.0, 870.0), t + 60.0);
    save(&mut sh, t + 60.0, "target/quicksearch-pull.ppm");
    sh.event(Event::Move(600.0, 920.0), t + 90.0);
    sh.event(Event::Up(600.0, 920.0), t + 100.0);
    sh.tick(t + 1000.0);
    t += 1000.0;
    save(&mut sh, t, "target/quicksearch.ppm");
    sh.quicksearch_key(Out::Text("se".into()), t);
    save(&mut sh, t + 10.0, "target/quicksearch-se.ppm");
}
