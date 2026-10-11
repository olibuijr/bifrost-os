//! Golden-image tests for the SDK kit controls in Light and Dark. See `snapshots/README.md`.
use bifrost_app::controls::{values, Rect};
use bifrost_app::kit::{appearance, gfx, Font, Surface, Ui};
use bifrost_app::view::*;
use std::path::PathBuf;

const SCALES: [f32; 2] = [1.0, 1.5625];
const CHANNEL_TOL: i32 = 2;
const MAX_DIFF_FRAC: f64 = 0.001;

fn ui(scale: f32) -> Ui {
    let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
    Ui::new(Font::parse(data).unwrap(), scale)
}

fn controls() -> Vec<(&'static str, (f32, f32), Box<dyn View>)> {
    let section = Section::new(vec![Box::new(Switch::new("Wi-Fi", true, "wifi")), Box::new(Checkbox::new("Ask to join", false, "ask"))])
        .header("NETWORK")
        .footer("Known networks are joined automatically.");
    vec![
        ("button", (160.0, 44.0), Box::new(Button::new("Continue", "go"))),
        ("checkbox", (200.0, 36.0), Box::new(Checkbox::new("Enable", true, "c"))),
        ("switch", (200.0, 44.0), Box::new(Switch::new("Wi-Fi", true, "s"))),
        ("slider", (240.0, 44.0), Box::new(Slider::new("Volume", values::Slider::new(0.0, 10.0, 4.0, None), "v"))),
        ("stepper", (200.0, 44.0), Box::new(Stepper::new("Count", values::Stepper::new(2.0, 0.0, 5.0, 1.0), "n"))),
        ("popup", (200.0, 44.0), Box::new(PopUpButton::new(&["One", "Two"], 1, "p"))),
        ("segmented", (240.0, 44.0), Box::new(Segmented::new(&["Day", "Week", "Month"], Some(1), "seg"))),
        ("textfield", (240.0, 44.0), Box::new(TextField::new("Placeholder", "t"))),
        ("progress", (240.0, 24.0), Box::new(ProgressIndicator::bar(0.4))),
        ("section", (340.0, 170.0), Box::new(Form::new(vec![section]))),
    ]
}

fn render(view: Box<dyn View>, pt: (f32, f32), scale: f32, light: bool) -> (usize, usize, Vec<u32>) {
    appearance::jump_to(light);
    let ui = ui(scale);
    let (w, h) = ((pt.0 * scale).round() as usize, (pt.1 * scale).round() as usize);
    let mut win: Window<NoApp> = Window::new(Stack::vstack(0.0, vec![view]), 1);
    win.layout(&ui, Rect { x: 0.0, y: 0.0, w: pt.0, h: pt.1 });
    let mut px = vec![if light { gfx::pack(236, 236, 236, 255) } else { gfx::pack(40, 40, 43, 255) }; w * h];
    let mut s = Surface { w, h, stride: w, px: &mut px, y0: 0 };
    win.draw(&mut s, &ui);
    (w, h, px)
}

// ---- minimal PNG (RGB, 8 bit, stored deflate): no dependencies ----

fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend(data);
    out.extend(&body);
    out.extend(crc32(&body).to_be_bytes());
}

fn encode_png(w: usize, h: usize, px: &[u32]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((w * 3 + 1) * h);
    for row in px.chunks(w) {
        raw.push(0);
        for p in row {
            raw.extend(&p.to_le_bytes()[..3]); // pack(): r, g, b, a in memory order
        }
    }
    let mut z = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65535).peekable();
    while let Some(b) = blocks.next() {
        z.push(blocks.peek().is_none() as u8);
        z.extend((b.len() as u16).to_le_bytes());
        z.extend((!(b.len() as u16)).to_le_bytes());
        z.extend(b);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend(((b << 16) | a).to_be_bytes());
    let mut ihdr = Vec::new();
    ihdr.extend((w as u32).to_be_bytes());
    ihdr.extend((h as u32).to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Reads back only what `encode_png` writes.
fn decode_png(d: &[u8]) -> Option<(usize, usize, Vec<u32>)> {
    let be = |i: usize| u32::from_be_bytes(d[i..i + 4].try_into().unwrap()) as usize;
    if d.get(..8)? != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let (mut i, mut w, mut h, mut z) = (8, 0, 0, Vec::new());
    while i + 8 <= d.len() {
        let n = be(i);
        match &d[i + 4..i + 8] {
            b"IHDR" => (w, h) = (be(i + 8), be(i + 12)),
            b"IDAT" => z.extend(d.get(i + 8..i + 8 + n)?),
            _ => {}
        }
        i += 12 + n;
    }
    let (mut raw, mut j) = (Vec::new(), 2);
    loop {
        let len = u16::from_le_bytes(z.get(j + 1..j + 3)?.try_into().ok()?) as usize;
        raw.extend(z.get(j + 5..j + 5 + len)?);
        if z[j] & 1 == 1 {
            break;
        }
        j += 5 + len;
    }
    let mut px = Vec::with_capacity(w * h);
    for row in raw.chunks(w * 3 + 1) {
        for c in row.get(1..)?.chunks(3) {
            px.push(gfx::pack(c[0], c[1], c[2], 255));
        }
    }
    (px.len() == w * h).then_some((w, h, px))
}

fn differs(a: u32, b: u32) -> bool {
    // goldens are stored opaque; compare RGB only
    (0..3).any(|k| (((a >> (8 * k)) & 255) as i32 - ((b >> (8 * k)) & 255) as i32).abs() > CHANNEL_TOL)
}

#[test]
fn kit_controls_match_their_golden_images() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots");
    let bless = std::env::var("BIFROST_BLESS").is_ok_and(|v| v == "1");
    let mut failures = Vec::new();
    for light in [true, false] {
        for scale in SCALES {
            for (name, pt, view) in controls() {
                let id = format!("{name}-{}-{scale}x", if light { "light" } else { "dark" });
                let (w, h, px) = render(view, pt, scale, light);
                let golden = dir.join(format!("{id}.png"));
                let actual = dir.join(format!("{id}.actual.png"));
                let _ = std::fs::remove_file(&actual);
                let _ = std::fs::remove_file(dir.join(format!("{id}.diff.png")));
                let stored = if bless { None } else { std::fs::read(&golden).ok().and_then(|d| decode_png(&d)) };
                let Some((gw, gh, gpx)) = stored else {
                    std::fs::write(&golden, encode_png(w, h, &px)).unwrap();
                    println!("blessed {id}");
                    continue;
                };
                if (gw, gh) != (w, h) {
                    std::fs::write(&actual, encode_png(w, h, &px)).unwrap();
                    failures.push(format!("{id}: size {w}x{h}, golden {gw}x{gh}"));
                    continue;
                }
                let bad: Vec<bool> = px.iter().zip(&gpx).map(|(a, b)| differs(*a, *b)).collect();
                let n = bad.iter().filter(|b| **b).count();
                if n as f64 > MAX_DIFF_FRAC * (w * h) as f64 {
                    std::fs::write(&actual, encode_png(w, h, &px)).unwrap();
                    let diff: Vec<u32> = bad
                        .iter()
                        .zip(&gpx)
                        .map(|(b, g)| if *b { gfx::pack(255, 0, 0, 255) } else { gfx::pack((*g & 255) as u8 / 3 + 170, ((*g >> 8) & 255) as u8 / 3 + 170, ((*g >> 16) & 255) as u8 / 3 + 170, 255) })
                        .collect();
                    std::fs::write(dir.join(format!("{id}.diff.png")), encode_png(w, h, &diff)).unwrap();
                    failures.push(format!("{id}: {n} of {} pixels differ", w * h));
                }
            }
        }
    }
    appearance::jump_to(false);
    assert!(failures.is_empty(), "snapshot mismatches (BIFROST_BLESS=1 to accept):\n{}", failures.join("\n"));
}
