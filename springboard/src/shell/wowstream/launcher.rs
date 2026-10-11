//! The game launcher: a full-screen page of game cards drawn with the shell's
//! parts. The state (games, which one runs, confirm questions, "Starting...") is the client
//! core's `launcher::Nav`; this file only lays out, draws and hit-tests it.

use super::render::{text, text_c};
use crate::gfx::{pack, rgba, rrect_mask, Image, Surface};
use crate::imgdec;
use crate::scene::STATUS_PT;
use crate::shell::cache::TextCache;
use crate::shell::icons::{self, Icon, BLUE, GREEN, RED};
use crate::shell::ui::{Ui, Weight, BLACK, CELL, SECONDARY, WHITE};
use std::collections::HashMap;
use wowstream_client::launcher::{Confirm, Nav};

const EMBLEM: &[u8] = include_bytes!("emblem.png");
const ORANGE: u32 = pack(255, 159, 10, 255);

type Rect = (f32, f32, f32, f32);

/// What a touch landed on.
#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    None,
    Settings,
    Play(String),
    Close,
    Yes,
    No,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Link {
    Online,
    Connecting,
    Offline,
}

/// The connection chip under the title.
pub struct Chip {
    pub link: Link,
    pub text: String,
}

struct CardPlan {
    id: String,
    name: String,
    card: Rect,
    art: Rect,
    play: Rect,
    close: Option<Rect>,
}

struct Plan {
    gear: Rect,
    chip: Rect,
    cards: Vec<CardPlan>,
    /// the alert, its No and its Yes button
    sheet: Option<(Rect, Rect, Rect)>,
}

pub struct Launcher {
    cache: TextCache,
    emblem: Option<Image>,
    art: HashMap<(String, usize, usize), Image>,
    scale: u32,
}

fn inside((x, y, w, h): Rect, px: f32, py: f32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

fn initials(name: &str) -> String {
    let s: String = name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect();
    if s.is_empty() { "?".into() } else { s.to_uppercase() }
}

/// A stable gradient for a game without artwork.
fn gradient_for(id: &str) -> ([f32; 3], [f32; 3]) {
    let h = id.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let c = |shift: u32| 70.0 + ((h >> shift) & 0x7f) as f32;
    let top = [c(0), c(8), c(16)];
    (top, [top[0] * 0.3, top[1] * 0.3, top[2] * 0.3])
}

impl Launcher {
    pub fn new() -> Self {
        Launcher { cache: TextCache::default(), emblem: imgdec::decode(EMBLEM), art: HashMap::new(), scale: 0 }
    }

    fn plan(&self, ui: &Ui, w: f32, h: f32, nav: &Nav) -> Plan {
        let col = (w - ui.pt(40.0)).min(ui.pt(740.0));
        let x0 = (w - col) / 2.0;
        let gear = (x0 + col - ui.pt(44.0), ui.pt(STATUS_PT + 14.0), ui.pt(44.0), ui.pt(44.0));
        let chip = (x0, ui.pt(STATUS_PT + 66.0), 0.0, ui.pt(30.0));
        let gap = ui.pt(16.0);
        let per_row = if col >= ui.pt(600.0) { 2 } else { 1 };
        let cw = (col - gap * (per_row as f32 - 1.0)) / per_row as f32;
        let pad = ui.pt(10.0);
        let art_h = (cw - 2.0 * pad) * 0.56;
        let ch = pad + art_h + ui.pt(12.0 + 26.0 + 10.0 + 40.0 + 14.0);
        let top = ui.pt(STATUS_PT + 116.0);
        let mut cards = vec![];
        for (n, g) in nav.games.iter().enumerate() {
            let (row, col_n) = (n / per_row, n % per_row);
            let (cx, cy) = (x0 + col_n as f32 * (cw + gap), top + row as f32 * (ch + gap));
            let by = cy + ch - ui.pt(14.0 + 40.0);
            let running = nav.running == g.id;
            let bw = cw - 2.0 * pad;
            let (play, close) = if running {
                let half = (bw - ui.pt(10.0)) / 2.0;
                ((cx + pad, by, half, ui.pt(40.0)), Some((cx + pad + half + ui.pt(10.0), by, half, ui.pt(40.0))))
            } else {
                ((cx + pad, by, bw, ui.pt(40.0)), None)
            };
            cards.push(CardPlan { id: g.id.clone(), name: g.name.clone(), card: (cx, cy, cw, ch), art: (cx + pad, cy + pad, cw - 2.0 * pad, art_h), play, close });
        }
        let sheet = nav.confirm.as_ref().map(|_| {
            let (sw, sh) = (ui.pt(300.0), ui.pt(170.0));
            let (sx, sy) = ((w - sw) / 2.0, (h - sh) / 2.0);
            let bh = ui.pt(46.0);
            ((sx, sy, sw, sh), (sx, sy + sh - bh, sw / 2.0, bh), (sx + sw / 2.0, sy + sh - bh, sw / 2.0, bh))
        });
        Plan { gear, chip, cards, sheet }
    }

    pub fn hit(&self, ui: &Ui, w: f32, h: f32, nav: &Nav, x: f32, y: f32) -> Hit {
        let plan = self.plan(ui, w, h, nav);
        if let Some((_, no, yes)) = plan.sheet {
            return if inside(yes, x, y) {
                Hit::Yes
            } else if inside(no, x, y) {
                Hit::No
            } else {
                Hit::None
            };
        }
        if inside(plan.gear, x, y) {
            return Hit::Settings;
        }
        if nav.pending_text().is_some() {
            return Hit::None;
        }
        for c in &plan.cards {
            if inside(c.play, x, y) {
                return Hit::Play(c.id.clone());
            }
            if c.close.is_some_and(|r| inside(r, x, y)) {
                return Hit::Close;
            }
        }
        Hit::None
    }

    fn art_for(&mut self, id: &str, name: &str, ui: &Ui, w: usize, h: usize) -> &Image {
        let emblem = &self.emblem;
        self.art.entry((id.to_string(), w, h)).or_insert_with(|| {
            let (top, bottom) = if id == "wow" { ([24.0, 44.0, 84.0], [6.0, 10.0, 24.0]) } else { gradient_for(id) };
            let mut img = Image::new(w, h);
            for y in 0..h {
                let k = y as f32 / h.max(1) as f32;
                let c = |i: usize| (top[i] + (bottom[i] - top[i]) * k) as u8;
                img.px[y * w..(y + 1) * w].fill(pack(c(0), c(1), c(2), 255));
            }
            let mut surface = Surface { w, h, stride: w, px: &mut img.px, y0: 0 };
            match emblem.as_ref().filter(|_| id == "wow") {
                Some(e) => {
                    let side = (h as f32 * 0.86) as usize;
                    surface.blit(&e.resized(side, side), ((w - side) / 2) as i32, ((h - side) / 2) as i32, 255, 255);
                }
                None => {
                    let label = ui.chrome(&initials(name), (h as f32 * 0.42) / ui.s, Weight::Bold, rgba(255, 255, 255, 200));
                    surface.blit(&label, ((w - label.w) / 2) as i32, ((h - label.h) / 2) as i32, 255, 255);
                }
            }
            img.apply_mask(&rrect_mask(w, h, ui.pt(12.0)));
            img
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, nav: &Nav, chip: &Chip, pressed: &Hit, t: f64) {
        let _ = t;
        if self.scale != ui.s.to_bits() {
            self.cache.clear();
            self.art.clear();
            self.scale = ui.s.to_bits();
        }
        let (w, h) = (s.w as f32, s.h as f32);
        s.fill_rect(0, 0, s.w, s.h, BLACK);
        let plan = self.plan(ui, w, h, nav);
        let x0 = plan.chip.0;
        text(s, &mut self.cache, ui, "WoW", 34.0, Weight::Bold, WHITE, x0, ui.pt(STATUS_PT + 36.0));
        // settings button
        let g = plan.gear;
        let tile = ui.pt(icons::TILE_PT);
        icons::tile(s, ui, Icon::General, g.0 + (g.2 - tile) / 2.0, g.1 + (g.3 - tile) / 2.0);
        if *pressed == Hit::Settings {
            s.fill_rrect(g.0, g.1, g.2, g.3, ui.pt(10.0), rgba(255, 255, 255, 40));
        }
        // connection chip: dot, host, path, round trip
        let chip_w = ui.measure(&chip.text, 14.0, Weight::Regular) + ui.pt(36.0);
        let (cx, cy, ch) = (plan.chip.0, plan.chip.1, plan.chip.3);
        s.fill_rrect(cx, cy, chip_w, ch, ch / 2.0, CELL);
        let dot = match chip.link {
            Link::Online => GREEN,
            Link::Connecting => ORANGE,
            Link::Offline => RED,
        };
        s.fill_rrect(cx + ui.pt(12.0), cy + ch / 2.0 - ui.pt(4.0), ui.pt(8.0), ui.pt(8.0), ui.pt(4.0), dot);
        text(s, &mut self.cache, ui, &chip.text, 14.0, Weight::Regular, WHITE, cx + ui.pt(28.0), cy + ch / 2.0);
        let mut note_y = cy + ch + ui.pt(12.0);
        if let Some(note) = nav.pending_text() {
            text(s, &mut self.cache, ui, &note, 15.0, Weight::Medium, ORANGE, x0, note_y + ui.pt(8.0));
            note_y += ui.pt(26.0);
        } else if let Some(err) = &nav.error {
            text(s, &mut self.cache, ui, err, 15.0, Weight::Medium, RED, x0, note_y + ui.pt(8.0));
        }
        let _ = note_y;
        if plan.cards.is_empty() {
            let msg = if chip.link == Link::Online && nav.loaded() { "No games on this server." } else { "Waiting for the server..." };
            text_c(s, &mut self.cache, ui, msg, 17.0, Weight::Regular, SECONDARY, w / 2.0, h / 2.0);
        }
        for c in &plan.cards {
            self.draw_card(s, ui, nav, c, pressed);
        }
        if let (Some((rect, no, yes)), Some(confirm)) = (plan.sheet, &nav.confirm) {
            self.draw_sheet(s, ui, nav, confirm, rect, no, yes, pressed);
        }
    }

    fn draw_card(&mut self, s: &mut Surface, ui: &Ui, nav: &Nav, c: &CardPlan, pressed: &Hit) {
        let (x, y, w, h) = c.card;
        s.fill_rrect(x, y, w, h, ui.pt(18.0), CELL);
        let (ax, ay, aw, ah) = c.art;
        let art = self.art_for(&c.id, &c.name, ui, aw as usize, ah as usize);
        s.blit(art, ax.round() as i32, ay.round() as i32, 255, 255);
        let running = nav.running == c.id;
        let name_y = ay + ah + ui.pt(12.0 + 13.0);
        let fit = ui.fit(&c.name, 17.0, w - ui.pt(28.0) - if running { ui.pt(70.0) } else { 0.0 });
        let used = text(s, &mut self.cache, ui, &fit, 17.0, Weight::Semibold, WHITE, ax + ui.pt(4.0), name_y);
        if running {
            text(s, &mut self.cache, ui, "Running", 13.0, Weight::Medium, GREEN, ax + ui.pt(4.0) + used + ui.pt(10.0), name_y);
        }
        let busy = nav.pending_text().is_some();
        let label = if running { "Resume" } else { "Play" };
        let fill = if busy { rgba(120, 120, 124, 90) } else if running { GREEN } else { BLUE };
        let down = *pressed == Hit::Play(c.id.clone());
        self.button(s, ui, c.play, label, if down { rgba(255, 255, 255, 40) } else { 0 }, fill, WHITE);
        if let Some(r) = c.close {
            let down = *pressed == Hit::Close;
            self.button(s, ui, r, "Close game", if down { rgba(255, 255, 255, 30) } else { 0 }, rgba(255, 69, 58, 46), RED);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn button(&mut self, s: &mut Surface, ui: &Ui, (x, y, w, h): Rect, label: &str, overlay: u32, fill: u32, color: u32) {
        s.fill_rrect(x, y, w, h, h / 2.0, fill);
        if overlay != 0 {
            s.fill_rrect(x, y, w, h, h / 2.0, overlay);
        }
        text_c(s, &mut self.cache, ui, label, 16.0, Weight::Semibold, color, x + w / 2.0, y + h / 2.0);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_sheet(&mut self, s: &mut Surface, ui: &Ui, nav: &Nav, confirm: &Confirm, rect: Rect, no: Rect, yes: Rect, pressed: &Hit) {
        s.fill_rect(0, 0, s.w, s.h, rgba(0, 0, 0, 140));
        let (x, y, w, h) = rect;
        s.fill_rrect(x, y, w, h, ui.pt(14.0), pack(44, 44, 46, 255));
        let (title, message, action) = match confirm {
            Confirm::Close => (format!("Close {}?", nav.name(&nav.running)), "Progress that is not saved in the game is lost.".to_string(), "Close game"),
            Confirm::Switch { id } => (format!("Switch to {}?", nav.name(id)), format!("This closes {} first.", nav.name(&nav.running)), "Switch"),
        };
        let fit = |ui: &Ui, t: &str, pt: f32| ui.fit(t, pt, w - ui.pt(32.0));
        text_c(s, &mut self.cache, ui, &fit(ui, &title, 17.0), 17.0, Weight::Semibold, WHITE, x + w / 2.0, y + ui.pt(34.0));
        text_c(s, &mut self.cache, ui, &fit(ui, &message, 13.0), 13.0, Weight::Regular, WHITE, x + w / 2.0, y + ui.pt(64.0));
        s.fill_rect(x as usize, no.1 as usize, w as usize, 1, pack(84, 84, 88, 255));
        s.fill_rect((x + w / 2.0) as usize, no.1 as usize, 1, no.3 as usize, pack(84, 84, 88, 255));
        for (r, label, color, hit) in [(no, "Cancel", BLUE, Hit::No), (yes, action, RED, Hit::Yes)] {
            if *pressed == hit {
                s.fill_rect(r.0 as usize, r.1 as usize + 1, r.2 as usize, r.3 as usize, rgba(255, 255, 255, 30));
            }
            text_c(s, &mut self.cache, ui, label, 17.0, Weight::Semibold, color, r.0 + r.2 / 2.0, r.1 + r.3 / 2.0);
        }
    }
}
