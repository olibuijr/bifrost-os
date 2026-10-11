//! Renders the Deildu screens on the host from canned site responses and
//! sample artwork: `cargo run --example deildu_preview -- <art dir> <out dir>`
//! writes library.ppm, detail.ppm, playing.ppm and login.ppm.
use springboard::deildu::http::{Stream, Transport};
use springboard::deildu::Deildu;
use springboard::font::Font;
use springboard::gfx::Surface;
use springboard::shell::keyboard::Out;
use springboard::shell::ui::Ui;
use springboard::shell::Event;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Serves routed replies (by request-line prefix) or an ordered queue.
struct Canned {
    routes: Vec<(String, Vec<u8>)>,
    queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
}

struct CannedStream {
    routes: Vec<(String, Vec<u8>)>,
    queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
    out: VecDeque<u8>,
}

impl Read for CannedStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = buf.len().min(self.out.len());
        for b in buf.iter_mut().take(n) {
            *b = self.out.pop_front().unwrap();
        }
        Ok(n)
    }
}

impl Write for CannedStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let line = String::from_utf8_lossy(buf).lines().next().unwrap_or("").to_string();
        let reply = self.routes.iter().find(|(p, _)| line.starts_with(p.as_str())).map(|(_, r)| r.clone()).or_else(|| self.queue.lock().unwrap().pop_front());
        match reply {
            Some(r) => self.out.extend(r),
            None => return Err(io::Error::new(io::ErrorKind::BrokenPipe, format!("no reply for {line}"))),
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Transport for Canned {
    fn connect(&self, _host: &str, _port: u16, _tls: bool) -> io::Result<Box<dyn Stream>> {
        Ok(Box::new(CannedStream { routes: self.routes.clone(), queue: self.queue.clone(), out: VecDeque::new() }))
    }
}

fn ok(body: &[u8]) -> Vec<u8> {
    let mut r = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
    r.extend_from_slice(body);
    r
}

fn torrent(name: &str) -> Vec<u8> {
    use deildu_rust::torrent::Value;
    let mut info = std::collections::BTreeMap::new();
    info.insert(b"name".to_vec(), Value::Bytes(name.as_bytes().to_vec()));
    info.insert(b"piece length".to_vec(), Value::Int(1 << 20));
    info.insert(b"pieces".to_vec(), Value::Bytes(vec![0; 20 * 1400]));
    info.insert(b"length".to_vec(), Value::Int(1400 << 20));
    let mut root = std::collections::BTreeMap::new();
    root.insert(b"info".to_vec(), Value::Dict(info));
    let mut out = vec![];
    Value::Dict(root).encode(&mut out);
    out
}

const TITLES: [&str; 9] = [
    "The.Deputy.2024.1080p.WEB-DL.x264-GRP",
    "Northern.Lights.2023.720p.BluRay.x264-ICE",
    "Harbour.Nights.S01E03.1080p.WEB.x264-FJORD",
    "Glacier.2024.2160p.WEB-DL.x264.AAC-PEAK",
    "Reykjavik.After.Dark.2022.1080p.x264-NOX",
    "Volcano.Season.S02E01.720p.WEB.x264-ASH",
    "The.Long.Winter.2021.1080p.BluRay.x264-SNOW",
    "Puffin.Island.2024.1080p.WEB-DL.x264.AAC-BIRD",
    "Midnight.Sun.S03E05.1080p.WEB.x264-DAY",
];

fn browse() -> Vec<u8> {
    let rows: String = TITLES
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let id = i + 1;
            let size = ["1.4 GB", "2.1 GB", "860 MB", "4.5 GB", "1.2 GB", "910 MB", "1.8 GB", "1.5 GB", "720 MB"][i];
            format!(r#"<tr><td>c</td><td><a href="browse.php?cat=6">M</a></td><td><a href="details.php?id={id}" title="{t}">{t}</a><a href="download.php/{id}/f.torrent">dl</a></td><td></td><td></td><td></td><td>{size}</td><td></td><td>{}</td><td>{}</td></tr>"#, 40 - i * 3, i)
        })
        .collect();
    ok(format!(r#"<html><table>{rows}</table><a href="browse.php?cat=6&page=1">next</a></html>"#).as_bytes())
}

fn save(buf: &[u32], w: usize, h: usize, path: &PathBuf) {
    let mut ppm = format!("P6 {w} {h} 255\n").into_bytes();
    for p in buf {
        ppm.extend([*p as u8, (p >> 8) as u8, (p >> 16) as u8]);
    }
    std::fs::write(path, ppm).unwrap();
}

/// Pumps until `done` holds or a second passes.
fn pump_until(d: &mut Deildu, ui: &Ui, done: impl Fn(&Deildu) -> bool) {
    let start = std::time::Instant::now();
    while !done(d) && start.elapsed().as_secs() < 2 {
        springboard::bare::sys::wait(&[d.fd()], 50);
        d.pump(ui);
    }
}

fn tap(d: &mut Deildu, ui: &Ui, x: f32, y: f32) {
    d.touch(ui, Event::Down(x, y), 1000.0);
    d.touch(ui, Event::Up(x, y), 1030.0);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let art = PathBuf::from(args.get(1).expect("art dir"));
    let out = PathBuf::from(args.get(2).expect("out dir"));
    let (w, h) = (1200, 1920);
    let data: &'static [u8] = Box::leak(std::fs::read("assets/SFUI.ttf").unwrap().into_boxed_slice());
    let ui = Ui::new(Font::parse(data).unwrap(), w as f32 / 768.0);
    let mut routes = vec![("GET /browse.php".to_string(), browse())];
    for i in 1..=9 {
        let img = if i == 1 { "hero.jpg".to_string() } else { format!("{i}.jpg") };
        routes.push((format!("GET /download.php/{i}/"), ok(&torrent(&format!("{}.mp4", TITLES[i - 1])))));
        let descr = "A quiet thriller set in a small Icelandic town, where a newly appointed deputy uncovers a chain of disappearances that the locals would rather keep buried. Shot on location through one long winter.";
        routes.push((format!("GET /details.php?id={i} "), ok(format!(r#"<table><tr><td>Lýsing</td><td>{descr}</td></tr></table><img src="https://lookpic.com/{img}">"#).as_bytes())));
        routes.push((format!("GET /{img} "), ok(&std::fs::read(art.join(&img)).unwrap())));
    }
    let dir = out.join("data");
    let _ = std::fs::remove_dir_all(&dir);
    let transport = Arc::new(Canned { routes, queue: Default::default() });
    let mut d = Deildu::new(&ui, w, h, dir.clone(), transport).unwrap();
    pump_until(&mut d, &ui, |d| d.describe().contains("deildu_visible=9"));
    d.tick(&ui, 0.0);
    pump_until(&mut d, &ui, |d| d.describe().contains("deildu_art_pending=0"));
    let mut buf = vec![0u32; w as usize * h as usize];
    let frame = |d: &mut Deildu, name: &str, buf: &mut Vec<u32>| {
        d.draw(&mut Surface { w: w as usize, h: h as usize, stride: w as usize, px: buf, y0: 0 }, &ui, 0.0);
        save(buf, w as usize, h as usize, &out.join(format!("{name}.ppm")));
    };
    frame(&mut d, "library", &mut buf);
    // detail of the featured release
    let (hx, hy, hw, hh) = springboard::deildu::library::Library::hero_rect(&ui, w);
    tap(&mut d, &ui, hx + hw / 2.0, hy + hh / 2.0 + springboard::deildu::library::Library::content_top(&ui));
    pump_until(&mut d, &ui, |d| d.describe().contains("deildu_screen=detail"));
    d.tick(&ui, 0.0);
    pump_until(&mut d, &ui, |d| d.describe().contains("deildu_art_pending=0") && d.play_button(&ui).is_some());
    std::thread::sleep(std::time::Duration::from_millis(200));
    d.pump(&ui);
    frame(&mut d, "detail", &mut buf);
    // the playing screen (no peers on the host, so it waits)
    let (px, py) = d.play_button(&ui).expect("detail shown");
    tap(&mut d, &ui, px, py);
    std::thread::sleep(std::time::Duration::from_millis(700));
    d.pump(&ui);
    frame(&mut d, "playing", &mut buf);
    // the sign-in sheet: a session bounce on first load
    let bounce = Arc::new(Canned { routes: vec![], queue: Arc::new(Mutex::new(VecDeque::from(vec![b"HTTP/1.1 302 Found\r\nLocation: /login.php\r\nContent-Length: 0\r\n\r\n".to_vec(), ok(b"<form><input type=password name=p></form>")]))) });
    let mut l = Deildu::new(&ui, w, h, out.join("data-login"), bounce).unwrap();
    pump_until(&mut l, &ui, |d| d.describe().contains("deildu_screen=login"));
    l.key(&ui, Out::Text("guest".into()));
    l.key(&ui, Out::Enter);
    l.key(&ui, Out::Text("password".into()));
    frame(&mut l, "login", &mut buf);
    let _ = std::fs::remove_dir_all(out.join("data-login"));
    let _ = std::fs::remove_dir_all(dir);
}
