//! Renders the fullscreen player's HUD over a test picture on the host, in
//! portrait and landscape: `cargo run --example player_preview -- <out dir>`
//! writes player-<state>-<w>x<h>.png.
use springboard::deildu::play::{PlayState, Player};
use springboard::deildu::player_view::PlayerView;
use springboard::font::Font;
use springboard::gfx::{pack, Image, Surface};
use springboard::pngenc;
use springboard::shell::ui::Ui;
use springboard::shell::Event;
use std::path::{Path, PathBuf};

/// A scripted player: a fixed position, length and state.
struct Fake {
    state: PlayState,
    pos: f64,
    frame: Option<Image>,
}

impl Player for Fake {
    fn open(&mut self, _: &Path, _: &str) -> std::io::Result<()> {
        Ok(())
    }
    fn stop(&mut self) {}
    fn status(&self) -> String {
        String::new()
    }
    fn frame(&mut self) -> Option<Image> {
        self.frame.take()
    }
    fn position_ms(&self) -> f64 {
        self.pos
    }
    fn duration_ms(&self) -> Option<f64> {
        Some(5_400_000.0)
    }
    fn state(&self) -> PlayState {
        self.state.clone()
    }
}

/// A 16:9 picture fitted to the surface: a diagonal colour wash with a grid.
fn picture(w: usize, h: usize) -> Image {
    let (pw, ph) = if w * 9 <= h * 16 { (w, w * 9 / 16) } else { (h * 16 / 9, h) };
    let mut img = Image::new(pw, ph);
    for y in 0..ph {
        for x in 0..pw {
            let k = (x + y) as f32 / (pw + ph) as f32;
            let grid = x % 120 == 0 || y % 120 == 0;
            let (r, g, b) = if grid { (230, 230, 230) } else { ((40.0 + 180.0 * k) as u8, (90.0 + 60.0 * (1.0 - k)) as u8, (200.0 - 150.0 * k) as u8) };
            img.px[y * pw + x] = pack(r, g, b, 255);
        }
    }
    img
}

fn main() {
    let out: PathBuf = std::env::args().nth(1).unwrap_or_else(|| "target".into()).into();
    std::fs::create_dir_all(&out).unwrap();
    let data = Box::leak(std::fs::read("assets/SFUI.ttf").unwrap().into_boxed_slice());
    let ui = Ui::new(Font::parse(data).unwrap(), 1200.0 / 768.0);
    for (w, h) in [(1200usize, 1920usize), (1920, 1200)] {
        for (name, state, scrub) in [
            ("playing", PlayState::Playing, false),
            ("paused", PlayState::Paused, false),
            ("scrubbing", PlayState::Paused, true),
            ("buffering", PlayState::Buffering, false),
            ("ended", PlayState::Ended, false),
            ("error", PlayState::Error("Decoder not available yet".into()), false),
        ] {
            let mut v = PlayerView::new(&ui, w as i32, h as i32, "The Deputy", "The.Deputy.2024.1080p.WEB-DL.x264-GRP.mp4");
            let mut p = Fake { state, pos: 1_234_000.0, frame: Some(picture(w, h)) };
            v.set_download(&ui, Some((0.62, false)));
            v.tick(&ui, &mut p, 10.0);
            if scrub {
                let x = (v.layout().track_x0 + v.layout().track_x1) * 0.4;
                v.touch(&ui, &mut p, Event::Down(x, v.layout().track_y), 100.0);
            }
            v.tick(&ui, &mut p, 400.0);
            let mut buf = vec![0u32; w * h];
            v.draw(&mut Surface { w, h, stride: w, px: &mut buf, y0: 0 }, &ui, 500.0);
            std::fs::write(out.join(format!("player-{name}-{w}x{h}.png")), pngenc::encode_rgb(w, h, &buf)).unwrap();
        }
    }
}
