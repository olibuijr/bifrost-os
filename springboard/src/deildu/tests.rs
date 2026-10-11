//! End-to-end tests of the app on a canned transport.

use super::*;
use crate::shell::tests::ui;
use deildu_rust::torrent::Value;
use http::tests::{ok, Fake};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const LOGIN_HTML: &str = r#"<form action="takelogin.php"><input name="username"><input type="password" name="password"></form>"#;
const LOGIN_BOUNCE: &str = "HTTP/1.1 302 Found\r\nLocation: /login.php\r\nContent-Length: 0\r\n\r\n";

fn browse(ids: &[&str]) -> String {
    let rows: String = ids
        .iter()
        .map(|id| format!(r#"<tr><td>c</td><td><a href="browse.php?cat=6">M</a></td><td><a href="details.php?id={id}" title="Film.{id}.2024.1080p.x264-G">x</a><a href="download.php/{id}/f.torrent">dl</a></td><td></td><td></td><td></td><td>1.4 GB</td><td></td><td>5</td><td>1</td></tr>"#))
        .collect();
    ok(&format!(r#"<html><table>{rows}</table><a href="browse.php?cat=6&page=1">next</a></html>"#))
}

/// A single-file torrent named `file` with one 32-byte piece.
fn torrent(file: &str) -> Vec<u8> {
    let mut info = BTreeMap::new();
    info.insert(b"name".to_vec(), Value::Bytes(file.as_bytes().to_vec()));
    info.insert(b"piece length".to_vec(), Value::Int(32));
    info.insert(b"pieces".to_vec(), Value::Bytes(vec![0; 20]));
    info.insert(b"length".to_vec(), Value::Int(32));
    let mut root = BTreeMap::new();
    root.insert(b"info".to_vec(), Value::Dict(info));
    let mut out = vec![];
    Value::Dict(root).encode(&mut out);
    out
}

/// A 200 reply for a body that is valid UTF-8 (the bencoded fixtures are).
fn raw(body: &[u8]) -> String {
    String::from_utf8(Fake::ok_bytes(body)).unwrap()
}

fn dir(name: &str) -> PathBuf {
    let d = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target")).join(format!("test-deildu-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn app(ui: &Ui, fake: Arc<Fake>, name: &str) -> Deildu {
    Deildu::new(ui, 1200, 1920, dir(name), fake).unwrap()
}

/// Pumps until `done` holds or 5 s pass.
fn pump_until(d: &mut Deildu, ui: &Ui, done: impl Fn(&Deildu) -> bool) -> bool {
    let start = Instant::now();
    while !done(d) && start.elapsed() < Duration::from_secs(5) {
        crate::bare::sys::wait(&[d.fd()], 50);
        d.pump(ui);
    }
    done(d)
}

fn tap(d: &mut Deildu, ui: &Ui, x: f32, y: f32) {
    d.touch(ui, Event::Down(x, y), 1000.0);
    d.touch(ui, Event::Up(x, y), 1030.0);
}

#[test]
fn wraps_words() {
    let ui = ui();
    let lines = wrap(&ui, "one two three four five six", 20.0, ui.pt(80.0));
    assert!(lines.len() >= 3, "{lines:?}");
    assert_eq!(lines.join(" "), "one two three four five six");
    assert_eq!(wrap(&ui, "", 20.0, 100.0), Vec::<String>::new());
    assert_eq!(wrap(&ui, "supercalifragilistic", 20.0, 10.0), vec!["supercalifragilistic"], "a long word is never split");
}

#[test]
fn expired_session_opens_the_login_sheet_and_signing_in_loads_the_library() {
    let ui = ui();
    let fake = Fake::new(vec![LOGIN_BOUNCE, &ok(LOGIN_HTML)]);
    let mut d = app(&ui, fake.clone(), "login");
    assert!(d.library.loading);
    assert!(pump_until(&mut d, &ui, |d| d.login.is_some()));
    assert_eq!(d.library.status.as_deref(), Some("Sign in to Deildu"));
    assert!(d.describe().contains("deildu_screen=login"));
    // type a name, return, a password, return
    let top = d.login.as_ref().unwrap().keyboard_top(&ui, 1920, d.keyboard.h()).unwrap();
    let key = |d: &mut Deildu, label: &str| {
        let (kx, ky) = d.keyboard.centre(label).unwrap();
        tap(d, &ui, kx, top + ky);
    };
    key(&mut d, "m");
    key(&mut d, "e");
    key(&mut d, "return");
    assert_eq!(d.login.as_ref().unwrap().user, "me");
    assert_eq!(d.login.as_ref().unwrap().focus, login::Field::Pass);
    key(&mut d, "p");
    key(&mut d, "w");
    // the sign-in exchange, the check listing, then the reload and its torrents
    {
        let mut r = fake.replies.lock().unwrap();
        r.push_back(ok(LOGIN_HTML).into_bytes());
        r.push_back(b"HTTP/1.1 302 Found\r\nLocation: /my.php\r\nSet-Cookie: pass=1\r\nContent-Length: 0\r\n\r\n".to_vec());
        r.push_back(ok("my").into_bytes());
        r.push_back(browse(&["1"]).into_bytes());
        r.push_back(browse(&["1", "2"]).into_bytes());
    }
    fake.route("GET /download.php/1/", &raw(&torrent("a.x264.mp4")));
    fake.route("GET /download.php/2/", &raw(&torrent("b.mkv")));
    key(&mut d, "return");
    assert!(d.login.as_ref().unwrap().busy);
    assert!(pump_until(&mut d, &ui, |d| d.login.is_none()));
    assert!(pump_until(&mut d, &ui, |d| d.library.cards.len() == 2 && d.library.checking() == 0));
    assert_eq!(d.library.visible(), vec![0], "the mkv release is hidden");
    assert_eq!(d.compat.get("2"), Some(&false));
    assert!(std::fs::read_to_string(d.dir.join(COMPAT_FILE)).unwrap().contains("2\t0"));
    assert!(auth::session_path(&d.dir).exists(), "session saved after sign-in");
    assert!(d.library.status.is_none());
    let _ = std::fs::remove_dir_all(&d.dir);
}

#[test]
fn cancel_closes_the_sheet_and_a_failed_login_shows_the_error() {
    let ui = ui();
    let fake = Fake::new(vec![LOGIN_BOUNCE, &ok(LOGIN_HTML)]);
    let mut d = app(&ui, fake.clone(), "login-fail");
    assert!(pump_until(&mut d, &ui, |d| d.login.is_some()));
    let (cx, cy) = d.login.as_ref().unwrap().button_centre(&ui, 1200, 1920, d.keyboard.h(), false);
    tap(&mut d, &ui, cx, cy);
    assert!(d.login.is_none());
    d.login = Some(Login::new(&ui));
    d.key(&ui, Out::Text("u".into()));
    d.key(&ui, Out::Enter);
    d.key(&ui, Out::Text("p".into()));
    {
        let mut r = fake.replies.lock().unwrap();
        r.push_back(ok(LOGIN_HTML).into_bytes());
        r.push_back(ok("failed").into_bytes());
        r.push_back(LOGIN_BOUNCE.as_bytes().to_vec());
        r.push_back(ok(LOGIN_HTML).into_bytes());
    }
    let (sx, sy) = d.login.as_ref().unwrap().button_centre(&ui, 1200, 1920, d.keyboard.h(), true);
    tap(&mut d, &ui, sx, sy);
    assert!(pump_until(&mut d, &ui, |d| d.login.as_ref().is_some_and(|l| l.error.is_some())));
    assert!(!d.login.as_ref().unwrap().busy);
    let _ = std::fs::remove_dir_all(&d.dir);
}

#[test]
fn library_to_detail_to_play_and_back() {
    let ui = ui();
    let fake = Fake::new(vec![&browse(&["1"])]);
    fake.route("GET /download.php/1/", &raw(&torrent("a.x264.mp4")));
    let mut d = app(&ui, fake.clone(), "flow");
    assert!(pump_until(&mut d, &ui, |d| d.library.visible() == vec![0]));
    assert!(d.describe().contains("deildu_screen=library"));
    // tap the hero: the details page is fetched, the torrent comes from the disk cache
    fake.route("GET /details.php?id=1", &ok(r#"<table><tr><td>Lýsing</td><td>Story.</td></tr></table><img src="https://lookpic.com/a.jpg">"#));
    let (hx, hy, hw, hh) = Library::hero_rect(&ui, 1200);
    tap(&mut d, &ui, hx + hw / 2.0, hy + hh / 2.0 + Library::content_top(&ui));
    assert!(d.detail.is_some());
    assert!(pump_until(&mut d, &ui, |d| d.detail.as_ref().is_some_and(|x| x.can_play() && x.details.is_some())));
    assert_eq!(d.detail.as_ref().unwrap().details.as_ref().unwrap().description, "Story.");
    assert_eq!(d.compat.get("1"), Some(&true));
    // artwork for the detail poster: only the image fetch is needed now
    let png = {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, 4, 6);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&[200u8; 4 * 6 * 3]).unwrap();
        out
    };
    fake.route_bytes("GET /a.jpg", Fake::ok_bytes(&png));
    d.tick(&ui, 2000.0);
    assert!(d.art_pending.contains(&("1".to_string(), DETAIL)));
    assert!(pump_until(&mut d, &ui, |d| d.art.contains("1", DETAIL)));
    let (pw, ph) = Detail::poster_size(&ui);
    let img = d.art.get("1", DETAIL).unwrap();
    assert_eq!((img.w, img.h), (pw, ph));
    assert!(art::disk_read(&d.dir, "1.img").is_some(), "raw download cached");
    // Play starts the download (no peers: it just waits) and shows progress
    let (px, py, pw2, ph2) = d.detail.as_ref().unwrap().play_rect(&ui);
    tap(&mut d, &ui, px + pw2 / 2.0, py + ph2 / 2.0 + Detail::content_top(&ui));
    assert!(d.playing.is_some());
    assert!(d.describe().contains("deildu_screen=playing"));
    let p = d.playing.as_ref().unwrap().download.progress();
    assert_eq!(p.pieces_total, 1);
    assert!(d.playing.as_ref().unwrap().download.path().starts_with(d.dir.join("torrents/1")));
    assert!(art::disk_read(&d.dir, "1.torrent").is_some(), "torrent cached by the compatibility check");
    assert_eq!(fake.requests.lock().unwrap().iter().filter(|r| r.starts_with("GET /download.php/1/")).count(), 1, "fetched once");
    let mut buf = vec![0u32; 1200 * 1920];
    d.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 0.0);
    let (sx, sy, sw, sh) = progress::Screen::stop_rect(&ui, 1200, 1920);
    tap(&mut d, &ui, sx + sw / 2.0, sy + sh / 2.0);
    assert!(d.playing.is_none() && d.detail.is_some());
    tap(&mut d, &ui, ui.pt(30.0), ui.pt(46.0));
    assert!(d.detail.is_none(), "back to the library");
    let _ = std::fs::remove_dir_all(&d.dir);
}

#[test]
fn tabs_search_and_paging_reload_with_a_new_generation() {
    let ui = ui();
    let fake = Fake::new(vec![&browse(&[]), &browse(&[]), &browse(&[]), &browse(&[]), &browse(&[])]);
    let mut d = app(&ui, fake.clone(), "nav");
    assert!(pump_until(&mut d, &ui, |d| !d.library.loading));
    assert_eq!(d.library.status.as_deref(), Some("No releases found"));
    let (x, y, w, h) = Library::seg_rect(&ui, 1);
    tap(&mut d, &ui, x + w / 2.0, y + h / 2.0);
    assert_eq!((d.library.category, d.gen), (1, 2));
    assert!(pump_until(&mut d, &ui, |d| !d.library.loading));
    let sr = Library::search_rect(&ui, 1200);
    tap(&mut d, &ui, sr.0 + 10.0, sr.1 + 10.0);
    assert!(d.library.searching);
    let top = d.kb_top(&ui);
    let (kx, ky) = d.keyboard.centre("q").unwrap();
    tap(&mut d, &ui, kx, top + ky);
    assert_eq!(d.library.query, "q");
    let (rx, ry) = d.keyboard.centre("return").unwrap();
    tap(&mut d, &ui, rx, top + ry);
    assert!(!d.library.searching);
    assert_eq!(d.gen, 3);
    assert!(pump_until(&mut d, &ui, |d| !d.library.loading));
    let reqs = fake.requests.lock().unwrap().clone();
    assert!(reqs.last().unwrap().starts_with("GET /browse.php?cat=8&page=0&search=q "), "{}", reqs.last().unwrap());
    tap(&mut d, &ui, sr.0 + sr.2 - 10.0, sr.1 + 10.0);
    assert_eq!(d.library.query, "");
    assert!(pump_until(&mut d, &ui, |d| !d.library.loading));
    d.library.next = true;
    d.activate(&ui, Target::Next);
    assert_eq!(d.library.page, 1);
    assert!(pump_until(&mut d, &ui, |d| !d.library.loading));
    assert!(fake.requests.lock().unwrap().last().unwrap().contains("page=1"));
    let _ = std::fs::remove_dir_all(&d.dir);
}

#[test]
fn dragging_scrolls_instead_of_tapping_and_ticks_report_motion() {
    let ui = ui();
    let fake = Fake::new(vec![&browse(&["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12"])]);
    let mut d = app(&ui, fake.clone(), "drag");
    assert!(pump_until(&mut d, &ui, |d| d.library.cards.len() == 12));
    for c in &mut d.library.cards {
        c.compat = Some(true);
    }
    d.tick(&ui, 0.0);
    let top = Library::content_top(&ui);
    let (cx, cy, cw, ch) = d.library.cell_rect(&ui, 1200, 0);
    let (x, y0) = (cx + cw / 2.0, cy + ch / 2.0 + top);
    d.touch(&ui, Event::Down(x, y0), 0.0);
    for i in 1..=5 {
        d.touch(&ui, Event::Move(x, y0 - i as f32 * 60.0), i as f64 * 16.0);
    }
    d.touch(&ui, Event::Up(x, y0 - 300.0), 96.0);
    assert!(d.detail.is_none(), "a drag opens nothing");
    assert!(d.library.scroll.offset() > 0.0);
    assert!(d.tick(&ui, 120.0), "fling in progress");
    let mut t = 120.0;
    while d.tick(&ui, t) && t < 5000.0 {
        t += 16.0;
    }
    assert!(d.library.scroll.offset() > 300.0);
    let _ = std::fs::remove_dir_all(&d.dir);
}

#[test]
fn take_events_drains_what_the_background_announced() {
    let ui = ui();
    let mut d = app(&ui, Fake::new(vec![]), "events");
    assert!(d.take_events().is_empty());
    d.events.push(DeilduEvent::Ready { title: "Film".into() });
    assert_eq!(d.take_events(), vec![DeilduEvent::Ready { title: "Film".into() }]);
    assert!(d.take_events().is_empty(), "taken once");
    let _ = std::fs::remove_dir_all(&d.dir);
}

const SOURCE_CARDS: &str = r#"<div class='item hf-video-item'><a href='https://heavyfetish.com/videos/first-clip/' title='First clip'><div class='img'><img class='thumb' data-webp='https://heavyfetish.com/p/1.jpg' alt='First'></div><div class='duration'>8:33</div></a></div><div class='item hf-video-item'><a href='https://heavyfetish.com/videos/second-clip/' title='Second clip'><div class='img'><img class='thumb' data-webp='https://heavyfetish.com/p/2.jpg' alt='Second'></div><div class='duration'>45:00</div></a></div>"#;
const SOURCE_DETAIL: &str = r#"<h1>Clip</h1><a href='https://heavyfetish.com/get/low.mp4'>MP4 480p</a>"#;

#[test]
fn sources_tab_lists_filters_and_confirms_streams() {
    let ui = ui();
    // The canned queue is consumed by the first connection only, so later
    // hosts are served through routes matched on the request line.
    let fake = Fake::new(vec![&browse(&[])]);
    fake.route("GET / HTTP", &ok(SOURCE_CARDS));
    fake.route("GET /videos/", &ok(SOURCE_DETAIL));
    fake.route("GET /p/", &ok("IMG"));
    let mut d = app(&ui, fake.clone(), "sources");
    assert!(pump_until(&mut d, &ui, |d| !d.library.loading));
    // open the Sources tab
    let (x, y) = (Library::seg_rect(&ui, 3).0 + 4.0, Library::seg_rect(&ui, 3).1 + 4.0);
    tap(&mut d, &ui, x, y);
    assert_eq!(d.library.category, library::SOURCE_TAB);
    assert!(pump_until(&mut d, &ui, |d| d.library.cards.len() == 2));
    assert!(pump_until(&mut d, &ui, |d| d.library.visible().len() == 2));
    assert_eq!(d.sources.provider, sources::Provider::HeavyFetish);
    assert!(d.sources.next.is_none(), "fixture has no pagination");
    // duration chip: only the 45:00 clip is over 30 min
    d.sources.filter = 3;
    d.library.chip = 3;
    d.apply_source_filter(&ui);
    assert_eq!(d.library.visible().len(), 1);
    assert_eq!(d.library.cards[d.library.visible()[0]].item.id, "heavyfetish:second-clip");
    d.sources.filter = 0;
    d.library.chip = 0;
    d.apply_source_filter(&ui);
    assert_eq!(d.library.visible().len(), 2);
    let _ = std::fs::remove_dir_all(&d.dir);
}
#[test]
fn sources_check_every_card_on_the_page_not_just_the_first_batch() {
    let ui = ui();
    let n = sources::ENRICH_FIRST + 6;
    let cards: String = (0..n)
        .map(|i| format!("<div class='item hf-video-item'><a href='https://heavyfetish.com/videos/clip-{i}/' title='Clip {i}'><div class='img'><img class='thumb' data-webp='https://heavyfetish.com/p/{i}.jpg'></div><div class='duration'>8:33</div></a></div>"))
        .collect();
    let fake = Fake::new(vec![&browse(&[])]);
    fake.route("GET / HTTP", &ok(&cards));
    fake.route("GET /videos/", &ok(SOURCE_DETAIL));
    fake.route("GET /p/", &ok("IMG"));
    let mut d = app(&ui, fake.clone(), "sources-all");
    assert!(pump_until(&mut d, &ui, |d| !d.library.loading));
    let (x, y) = (Library::seg_rect(&ui, 3).0 + 4.0, Library::seg_rect(&ui, 3).1 + 4.0);
    tap(&mut d, &ui, x, y);
    assert!(pump_until(&mut d, &ui, |d| d.library.cards.len() == n));
    assert!(pump_until(&mut d, &ui, |d| d.library.visible().len() == n), "all {n} cards confirmed, got {}", d.library.visible().len());
    let _ = std::fs::remove_dir_all(&d.dir);
}

#[test]
fn source_hero_art_is_hero_sized() {
    let ui = ui();
    let png = {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, 16, 10);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&[200u8; 16 * 10 * 3]).unwrap();
        out
    };
    let fake = Fake::new(vec![&browse(&[])]);
    fake.route("GET / HTTP", &ok(SOURCE_CARDS));
    fake.route("GET /videos/", &ok(SOURCE_DETAIL));
    fake.route_bytes("GET /p/", Fake::ok_bytes(&png));
    let mut d = app(&ui, fake.clone(), "sources-hero");
    assert!(pump_until(&mut d, &ui, |d| !d.library.loading));
    let (x, y) = (Library::seg_rect(&ui, 3).0 + 4.0, Library::seg_rect(&ui, 3).1 + 4.0);
    tap(&mut d, &ui, x, y);
    assert!(pump_until(&mut d, &ui, |d| d.library.visible().len() == 2));
    let id = d.library.cards[d.library.visible()[0]].item.id.clone();
    d.tick(&ui, 3000.0);
    assert!(pump_until(&mut d, &ui, |d| d.art.contains(&id, HERO)));
    let img = d.art.get(&id, HERO).unwrap();
    assert_eq!((img.w as usize, img.h as usize), Library::hero_size(&ui, 1200));
    let _ = std::fs::remove_dir_all(&d.dir);
}
