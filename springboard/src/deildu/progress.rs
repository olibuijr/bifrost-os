//! The playback screen: streaming progress (peers, pieces, rate) and the
//! player's status, with a Stop button back to the details.

use super::detail::size_text;
use super::play::Progress;
use super::{within, Target, RED};
use crate::gfx::{Image, Surface};
use crate::shell::ui::{centred, Ui, Weight, CELL, SECONDARY, TRACK, WHITE};

const BAR_W_PT: f32 = 560.0;
const BUTTON_W_PT: f32 = 140.0;
const BUTTON_H_PT: f32 = 44.0;

pub struct Screen {
    title: Image,
    file: Image,
    lines: Vec<Image>,
    status: Image,
    stop: Image,
    fraction: f32,
    last: Option<Progress>,
    player: Option<String>,
}

impl Screen {
    pub fn new(ui: &Ui, w: i32, title: &str, file: &str) -> Self {
        let max = w as f32 - ui.pt(80.0);
        Screen {
            title: ui.text(&ui.fit(title, 28.0, max), 28.0, Weight::Bold, WHITE),
            file: ui.text(&ui.fit(file, 15.0, max), 15.0, Weight::Regular, SECONDARY),
            lines: vec![],
            status: ui.text("Finding peers…", 17.0, Weight::Regular, WHITE),
            stop: ui.text("Stop", 17.0, Weight::Semibold, super::BLACK),
            fraction: 0.0,
            last: None,
            player: None,
        }
    }

    /// Lines summarising the engine's progress.
    pub fn summary(p: &Progress) -> Vec<String> {
        vec![
            format!("{} peers · {} known", p.peers, p.known_peers),
            format!("Pieces {} of {}", p.pieces, p.pieces_total),
            format!("{} of {} · {:.1} MB/s", size_text(p.written), size_text(p.length), p.rate / 1e6),
        ]
    }

    /// What the player reported after `open`.
    pub fn set_player(&mut self, ui: &Ui, text: &str) {
        self.player = Some(text.to_string());
        self.status = ui.text(text, 17.0, Weight::Regular, if text.contains("not available") { RED } else { WHITE });
    }

    pub fn update(&mut self, ui: &Ui, p: &Progress) {
        if self.last.as_ref() == Some(p) {
            return;
        }
        self.lines = Self::summary(p).iter().map(|l| ui.text(l, 15.0, Weight::Regular, SECONDARY)).collect();
        self.fraction = p.fraction();
        if self.player.is_none() {
            let (text, color) = match &p.error {
                Some(e) => (e.as_str(), RED),
                None => (p.status.as_str(), WHITE),
            };
            self.status = ui.text(&ui.fit(text, 17.0, ui.pt(600.0)), 17.0, Weight::Regular, color);
        }
        self.last = Some(p.clone());
    }

    pub fn stop_rect(ui: &Ui, w: i32, h: i32) -> (f32, f32, f32, f32) {
        ((w as f32 - ui.pt(BUTTON_W_PT)) / 2.0, h as f32 * 0.5 + ui.pt(160.0), ui.pt(BUTTON_W_PT), ui.pt(BUTTON_H_PT))
    }

    pub fn hit(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<Target> {
        within(x, y, Self::stop_rect(ui, w, h)).then_some(Target::Stop)
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32, pressed: Option<Target>) {
        s.fill_rect(0, 0, w as usize, h as usize, super::BLACK);
        let cx = w / 2;
        let mid = h as f32 * 0.5;
        centred(s, &self.title, cx, (mid - ui.pt(140.0)) as i32);
        centred(s, &self.file, cx, (mid - ui.pt(100.0)) as i32);
        let (bw, bh) = (ui.pt(BAR_W_PT), ui.pt(6.0));
        let bx = (w as f32 - bw) / 2.0;
        s.fill_rrect(bx, mid - ui.pt(40.0), bw, bh, bh / 2.0, TRACK);
        if self.fraction > 0.0 {
            s.fill_rrect(bx, mid - ui.pt(40.0), bw * self.fraction.min(1.0), bh, bh / 2.0, WHITE);
        }
        centred(s, &self.status, cx, (mid - ui.pt(16.0)) as i32);
        let mut y = mid + ui.pt(24.0);
        for l in &self.lines {
            centred(s, l, cx, y as i32);
            y += ui.pt(24.0);
        }
        let (x, y, w2, h2) = Self::stop_rect(ui, w, h);
        s.fill_rrect(x, y, w2, h2, ui.pt(10.0), if pressed == Some(Target::Stop) { SECONDARY } else { WHITE });
        s.blit(&self.stop, (x + (w2 - self.stop.w as f32) / 2.0) as i32, (y + (h2 - self.stop.h as f32) / 2.0) as i32, 255, 255);
        let _ = CELL;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::tests::ui;

    #[test]
    fn summary_lines_and_status() {
        let p = Progress { peers: 3, known_peers: 40, pieces: 12, pieces_total: 900, written: 45_000_000, length: 1_400_000_000, rate: 2_300_000.0, status: "3 peers".into(), ..Progress::default() };
        assert_eq!(Screen::summary(&p), vec!["3 peers · 40 known", "Pieces 12 of 900", "45 MB of 1.4 GB · 2.3 MB/s"]);
        let ui = ui();
        let mut sc = Screen::new(&ui, 1200, "Film", "film.mp4");
        sc.update(&ui, &p);
        assert_eq!(sc.lines.len(), 3);
        assert!((sc.fraction - 45.0 / 1400.0).abs() < 1e-3);
        sc.set_player(&ui, "Decoder not available yet");
        let (x, y, w, h) = Screen::stop_rect(&ui, 1200, 1920);
        assert_eq!(sc.hit(&ui, 1200, 1920, x + w / 2.0, y + h / 2.0), Some(Target::Stop));
        assert_eq!(sc.hit(&ui, 1200, 1920, 10.0, 10.0), None);
        let mut buf = vec![0u32; 1200 * 1920];
        sc.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 1200, 1920, Some(Target::Stop));
    }
}
