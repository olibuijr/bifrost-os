//! The detail screen: poster, title, release info, Play, the description
//! and the files in the release with the one that will play marked.

use super::art::Cache;
use super::library::{DETAIL, MARGIN_PT, RADIUS_PT};
use super::play;
use super::scroll::Scroller;
use super::site::{self, Details, Item};
use super::{wrap, within, Target, BLUE, RED};
use crate::gfx::{Image, Surface};
use crate::shell::ui::{Ui, Weight, CELL, SECONDARY, SEPARATOR, WHITE};
use deildu_rust::torrent::{Torrent, TorrentFile};

const NAV_MID_PT: f32 = 46.0;
pub const TOP_PT: f32 = 76.0;
const POSTER_W_PT: f32 = 280.0;
const POSTER_H_PT: f32 = 420.0;
const ROW_PT: f32 = 44.0;
const LINE_PT: f32 = 22.0;
const BUTTON_W_PT: f32 = 140.0;
const BUTTON_H_PT: f32 = 44.0;

pub struct Detail {
    pub item: Item,
    pub details: Option<Details>,
    pub torrent: Option<Torrent>,
    pub file: Option<TorrentFile>,
    pub error: Option<String>,
    pub scroll: Scroller,
    back: Image,
    chevron: Image,
    title: Vec<Image>,
    release: Image,
    info: Image,
    play: Image,
    play_off: Image,
    status: Option<Image>,
    desc_header: Image,
    files_header: Image,
    description: Vec<Image>,
    files: Vec<(Image, Image, bool)>,
    check: Image,
}

impl Detail {
    pub fn new(ui: &Ui, w: i32, item: Item) -> Self {
        let col_w = Self::text_w(ui, w);
        let title = wrap(ui, &site::clean_title(&item.title), 28.0, col_w).into_iter().take(3).map(|l| ui.text(&l, 28.0, Weight::Bold, WHITE)).collect();
        let mut d = Detail {
            details: None,
            torrent: None,
            file: None,
            error: None,
            scroll: Scroller::new(),
            back: ui.text("Library", 17.0, Weight::Regular, BLUE),
            chevron: ui.text("‹", 34.0, Weight::Medium, BLUE),
            title,
            release: ui.text(&ui.fit(&item.title, 13.0, col_w), 13.0, Weight::Regular, SECONDARY),
            info: ui.text(&ui.fit(&item.info, 15.0, col_w), 15.0, Weight::Regular, WHITE),
            play: ui.text("Play", 17.0, Weight::Semibold, super::BLACK),
            play_off: ui.text("Play", 17.0, Weight::Semibold, SECONDARY),
            status: None,
            desc_header: ui.text("DESCRIPTION", 13.0, Weight::Regular, SECONDARY),
            files_header: ui.text("FILES", 13.0, Weight::Regular, SECONDARY),
            description: vec![],
            files: vec![],
            check: ui.text("✓", 17.0, Weight::Semibold, BLUE),
            item,
        };
        d.set_status(ui, "Loading…", SECONDARY);
        d
    }

    fn text_w(ui: &Ui, w: i32) -> f32 {
        w as f32 - ui.pt(MARGIN_PT) * 2.0 - ui.pt(POSTER_W_PT) - ui.pt(24.0)
    }

    fn set_status(&mut self, ui: &Ui, s: &str, color: u32) {
        self.status = Some(ui.text(&ui.fit(s, 15.0, ui.pt(400.0)), 15.0, Weight::Regular, color));
    }

    pub fn set_details(&mut self, ui: &Ui, w: i32, d: Details) {
        let width = w as f32 - 2.0 * ui.pt(MARGIN_PT);
        self.description = wrap(ui, &d.description, 15.0, width).into_iter().take(40).map(|l| ui.text(&l, 15.0, Weight::Regular, WHITE)).collect();
        self.details = Some(d);
    }

    /// Keeps the parsed torrent and marks the file that will play.
    pub fn set_torrent(&mut self, ui: &Ui, w: i32, t: Torrent) {
        self.file = play::pick_file(&t);
        let name_w = w as f32 - 2.0 * ui.pt(MARGIN_PT) - ui.pt(150.0);
        self.files = t
            .files
            .iter()
            .map(|f| {
                let chosen = self.file.as_ref().is_some_and(|c| c.path == f.path);
                (ui.text(&ui.fit(&f.path, 15.0, name_w), 15.0, Weight::Regular, if chosen { WHITE } else { SECONDARY }), ui.text(&size_text(f.length), 15.0, Weight::Regular, SECONDARY), chosen)
            })
            .collect();
        match &self.file {
            Some(f) => {
                let name = f.path.rsplit('/').next().unwrap_or("").to_string();
                self.set_status(ui, &format!("{} · {}", size_text(f.length), name), SECONDARY);
            }
            None => self.set_status(ui, "No compatible file: needs H.264/AAC MP4 or M4V", RED),
        }
        self.torrent = Some(t);
    }

    pub fn set_error(&mut self, ui: &Ui, e: &str) {
        self.error = Some(e.to_string());
        self.set_status(ui, e, RED);
    }

    pub fn can_play(&self) -> bool {
        self.torrent.is_some() && self.file.is_some()
    }

    // --- geometry ---

    fn poster_rect(ui: &Ui) -> (f32, f32, f32, f32) {
        (ui.pt(MARGIN_PT), ui.pt(8.0), ui.pt(POSTER_W_PT), ui.pt(POSTER_H_PT))
    }

    pub fn poster_size(ui: &Ui) -> (usize, usize) {
        (ui.pt(POSTER_W_PT).round() as usize, ui.pt(POSTER_H_PT).round() as usize)
    }

    fn column_x(ui: &Ui) -> f32 {
        ui.pt(MARGIN_PT) + ui.pt(POSTER_W_PT) + ui.pt(24.0)
    }

    /// Content-relative Play button.
    pub fn play_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let y = ui.pt(8.0) + self.title.iter().map(|t| t.h as f32).sum::<f32>() + self.release.h as f32 + self.info.h as f32 + ui.pt(32.0);
        (Self::column_x(ui), y, ui.pt(BUTTON_W_PT), ui.pt(BUTTON_H_PT))
    }

    fn body_top(&self, ui: &Ui) -> f32 {
        let (_, py, _, ph) = Self::poster_rect(ui);
        py + ph + ui.pt(32.0)
    }

    fn files_top(&self, ui: &Ui) -> f32 {
        let desc = if self.description.is_empty() { 0.0 } else { ui.pt(24.0) + self.description.len() as f32 * ui.pt(LINE_PT) + ui.pt(24.0) };
        self.body_top(ui) + desc
    }

    pub fn content_h(&self, ui: &Ui) -> f32 {
        let files = if self.files.is_empty() { 0.0 } else { ui.pt(24.0) + self.files.len() as f32 * ui.pt(ROW_PT) };
        self.files_top(ui) + files + ui.pt(24.0)
    }

    pub fn content_top(ui: &Ui) -> f32 {
        ui.pt(TOP_PT)
    }

    pub fn fit_scroll(&mut self, ui: &Ui, bottom: f32) {
        let max = self.content_h(ui) - (bottom - Self::content_top(ui));
        self.scroll.set_max(max);
    }

    pub fn hit(&self, ui: &Ui, w: i32, bottom: f32, x: f32, y: f32) -> Option<Target> {
        if y < ui.pt(NAV_MID_PT + 22.0) && x < ui.pt(MARGIN_PT + 140.0) {
            return Some(Target::Back);
        }
        if y < Self::content_top(ui) || y >= bottom {
            return None;
        }
        let cy = y - Self::content_top(ui) + self.scroll.offset();
        let _ = w;
        if within(x, cy, self.play_rect(ui)) {
            return Some(Target::Play);
        }
        Some(Target::Content)
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, bottom: f32, art: &mut Cache, pressed: Option<Target>) {
        let top = Self::content_top(ui);
        let m = ui.pt(MARGIN_PT);
        {
            let mut band = s.band(top as usize);
            let off = self.scroll.offset();
            let cy = |y: f32| y - off + top;
            let (px, py, pw, ph) = Self::poster_rect(ui);
            let py = cy(py);
            match art.get(&self.item.id, DETAIL) {
                Some(img) => band.blit(img, px as i32, py as i32, 255, 255),
                None => band.fill_rrect(px, py, pw, ph, ui.pt(RADIUS_PT), CELL),
            }
            let cx = Self::column_x(ui);
            let mut y = cy(ui.pt(8.0));
            for t in &self.title {
                band.blit(t, cx as i32, y as i32, 255, 255);
                y += t.h as f32;
            }
            band.blit(&self.release, cx as i32, y as i32, 255, 255);
            y += self.release.h as f32 + ui.pt(8.0);
            band.blit(&self.info, cx as i32, y as i32, 255, 255);
            let (bx, by, bw, bh) = self.play_rect(ui);
            let by = cy(by);
            let on = self.can_play();
            let fill = if !on { CELL } else if pressed == Some(Target::Play) { SECONDARY } else { WHITE };
            band.fill_rrect(bx, by, bw, bh, ui.pt(10.0), fill);
            let label = if on { &self.play } else { &self.play_off };
            band.blit(label, (bx + (bw - label.w as f32) / 2.0) as i32, (by + (bh - label.h as f32) / 2.0) as i32, 255, 255);
            if let Some(st) = &self.status {
                band.blit(st, cx as i32, (by + bh + ui.pt(12.0)) as i32, 255, 255);
            }
            if !self.description.is_empty() {
                let mut y = cy(self.body_top(ui));
                band.blit(&self.desc_header, (m + ui.pt(4.0)) as i32, y as i32, 255, 255);
                y += ui.pt(24.0);
                for line in &self.description {
                    band.blit(line, m as i32, y as i32, 255, 255);
                    y += ui.pt(LINE_PT);
                }
            }
            if !self.files.is_empty() {
                let mut y = cy(self.files_top(ui));
                band.blit(&self.files_header, (m + ui.pt(4.0)) as i32, y as i32, 255, 255);
                y += ui.pt(24.0);
                let (cw, rh) = (w as f32 - 2.0 * m, ui.pt(ROW_PT));
                band.fill_rrect(m, y, cw, rh * self.files.len() as f32, ui.pt(10.0), CELL);
                for (i, (name, size, chosen)) in self.files.iter().enumerate() {
                    let ry = y + rh * i as f32;
                    let mid = ry + rh / 2.0;
                    let tx = m + ui.pt(16.0);
                    if *chosen {
                        band.blit(&self.check, tx as i32, (mid - self.check.h as f32 / 2.0) as i32, 255, 255);
                    }
                    band.blit(name, (tx + ui.pt(26.0)) as i32, (mid - name.h as f32 / 2.0) as i32, 255, 255);
                    band.blit(size, (m + cw - ui.pt(16.0) - size.w as f32) as i32, (mid - size.h as f32 / 2.0) as i32, 255, 255);
                    if i + 1 < self.files.len() {
                        band.fill_rrect(tx + ui.pt(26.0), ry + rh - 1.0, cw - ui.pt(42.0), 1.0, 0.0, SEPARATOR);
                    }
                }
            }
        }
        s.fill_rect(0, bottom as usize, w as usize, (s.h as f32 - bottom).max(0.0) as usize, super::BLACK);
        s.fill_rect(0, 0, w as usize, top as usize, super::BLACK);
        let nav = ui.pt(NAV_MID_PT);
        let bc = &self.chevron;
        let alpha = if pressed == Some(Target::Back) { 120 } else { 255 };
        s.blit(bc, (m - ui.pt(10.0)) as i32, (nav - bc.h as f32 / 2.0 - ui.pt(2.0)) as i32, 255, alpha);
        s.blit(&self.back, (m - ui.pt(10.0) + bc.w as f32) as i32, (nav - self.back.h as f32 / 2.0) as i32, 255, alpha);
    }
}

/// "1.4 GB" style sizes.
pub fn size_text(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.0} MB", b / 1e6)
    } else {
        format!("{:.0} KB", b / 1e3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::tests::ui;

    fn item() -> Item {
        Item { id: "7".into(), title: "Some.Film.2024.1080p.WEB-DL.x264-GRP".into(), url: "https://www.deildu.net/details.php?id=7".into(), info: "1.4 GB · 12 seeders".into(), torrent_url: String::new() }
    }

    fn torrent(files: &[(&str, u64)]) -> Torrent {
        let mut off = 0;
        let files: Vec<TorrentFile> = files
            .iter()
            .map(|(p, l)| {
                let f = TorrentFile { path: p.to_string(), length: *l, offset: off };
                off += l;
                f
            })
            .collect();
        Torrent { name: "rel".into(), info_hash: [0; 20], piece_length: 1 << 20, hashes: vec![[0; 20]; (off as usize).div_ceil(1 << 20).max(1)], total_length: off.max(1), files, trackers: vec![], web_seeds: vec![] }
    }

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(size_text(1_400_000_000), "1.4 GB");
        assert_eq!(size_text(700_000_000), "700 MB");
        assert_eq!(size_text(12_000), "12 KB");
    }

    #[test]
    fn play_needs_a_compatible_file() {
        let ui = ui();
        let mut d = Detail::new(&ui, 1200, item());
        assert!(!d.can_play());
        d.set_torrent(&ui, 1200, torrent(&[("a.mkv", 10), ("b.x264.mp4", 20)]));
        assert!(d.can_play());
        assert_eq!(d.file.as_ref().unwrap().path, "b.x264.mp4");
        assert_eq!(d.files.iter().map(|f| f.2).collect::<Vec<_>>(), vec![false, true]);
        d.set_torrent(&ui, 1200, torrent(&[("a.mkv", 10)]));
        assert!(!d.can_play());
        d.set_error(&ui, "boom");
        assert_eq!(d.error.as_deref(), Some("boom"));
    }

    #[test]
    fn layout_grows_with_description_and_files_and_hits_play() {
        let ui = ui();
        let mut d = Detail::new(&ui, 1200, item());
        let base = d.content_h(&ui);
        d.set_details(&ui, 1200, Details { description: "word ".repeat(300), ..Details::default() });
        assert!(d.description.len() > 3);
        let with_desc = d.content_h(&ui);
        assert!(with_desc > base);
        d.set_torrent(&ui, 1200, torrent(&[("a.mp4", 10), ("b.mp4", 20)]));
        assert!(d.content_h(&ui) > with_desc);
        let bottom = 1920.0 - ui.pt(28.0);
        let (x, y, w, h) = d.play_rect(&ui);
        assert_eq!(d.hit(&ui, 1200, bottom, x + w / 2.0, y + h / 2.0 + Detail::content_top(&ui)), Some(Target::Play));
        assert_eq!(d.hit(&ui, 1200, bottom, ui.pt(30.0), ui.pt(46.0)), Some(Target::Back));
        assert_eq!(d.hit(&ui, 1200, bottom, 600.0, 1919.0), None);
        let mut buf = vec![0u32; 1200 * 1920];
        let mut art = Cache::new(1 << 20);
        d.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 1200, bottom, &mut art, Some(Target::Play));
        assert!(buf.contains(&WHITE), "something white drawn");
    }
}
