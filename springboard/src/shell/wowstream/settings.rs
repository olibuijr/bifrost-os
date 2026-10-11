//! Login and stream settings as an bifrost grouped list built from the shell's own parts: inset
//! cell groups, `TextField`s for host, user and (secure) password, a segmented control for the
//! stream profile, switches, and the system keyboard.

use super::render::{text, text_c, text_r};
use crate::gfx::{rgba, Surface};
use crate::scene::STATUS_PT;
use crate::shell::cache::TextCache;
use crate::shell::controls::{draw_switch, switch_progress, SWITCH_H_PT, SWITCH_MS, SWITCH_W_PT};
use crate::shell::icons::{BLUE, RED};
use crate::shell::keyboard::{self, Keyboard, Out};
use crate::shell::list::fill_row;
use crate::shell::textfield::TextField;
use crate::shell::ui::{home_zone, Ui, Weight, BLACK, CELL, SECONDARY, SEPARATOR, TRACK, WHITE};
use crate::shell::Event;
use wowstream_client::config::Settings;
use wowstream_client::profile::Profile;

const ROW_PT: f32 = 50.0;
const SEGMENTS_PT: f32 = 84.0;
const NAV_PT: f32 = 44.0;
const LABEL_PT: f32 = 118.0;
const PROFILE_NAMES: [&str; 4] = ["Data saver", "Balanced", "High", "Max"];
const HOST: usize = 0;
const USER: usize = 1;
const PASS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Field(usize),
    Segments,
    Switch(usize),
}

/// header, footer, rows
const GROUPS: [(&str, &str, &[Kind]); 3] = [
    ("SERVER", "Addresses are tried in order: the LAN one first, then the internet one.", &[Kind::Field(HOST)]),
    ("ACCOUNT", "", &[Kind::Field(USER), Kind::Field(PASS)]),
    ("STREAM", "Fill screen asks for the display's own shape instead of 16:9.", &[Kind::Segments, Kind::Switch(0), Kind::Switch(1)]),
];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    None,
    Cancel,
    Done,
    Field(usize),
    Segment(usize),
    Switch(usize),
    Keyboard,
}

/// What the page asks of its owner.
pub enum Outcome {
    Save(Settings),
    Cancel,
}

type Rect = (f32, f32, f32, f32);

struct Row {
    kind: Kind,
    rect: Rect,
    first: bool,
    last: bool,
}

struct Layout {
    rows: Vec<Row>,
    /// (text, x, centre y)
    headers: Vec<(&'static str, f32, f32)>,
    footers: Vec<(&'static str, f32, f32)>,
    w: f32,
}

pub struct SettingsPage {
    w: i32,
    h: i32,
    /// the landscape size the profiles are shaped for
    display: (f32, f32),
    base: Settings,
    fields: Vec<TextField>,
    profile: Profile,
    toggles: [bool; 2],
    flipped: [f64; 2],
    keyboard: Keyboard,
    focus: Option<usize>,
    target: Target,
    scroll: f32,
    cache: TextCache,
    scale: u32,
    /// shown under the title, e.g. "Login failed"
    pub status: String,
}

impl SettingsPage {
    pub fn new(ui: &Ui, w: i32, h: i32, display: (f32, f32), settings: &Settings, status: String) -> Self {
        let make = |placeholder: &str, secure: bool, text: &str| {
            let mut f = TextField::new(ui, placeholder, 17.0, secure, 128);
            f.set_text(ui, text);
            f.focused = false;
            f
        };
        SettingsPage {
            w,
            h,
            display,
            base: settings.clone(),
            fields: vec![make("lan.example.com, example.com", false, &settings.host), make("Required", false, &settings.user), make("Required", true, &settings.password)],
            profile: settings.profile,
            toggles: [settings.fill, settings.show_hud],
            flipped: [-1e9; 2],
            keyboard: Keyboard::new_hidden(w as f32, ui.s),
            focus: None,
            target: Target::None,
            scroll: 0.0,
            cache: TextCache::default(),
            scale: ui.s.to_bits(),
            status,
        }
    }

    /// The settings as edited.
    pub fn settings(&self) -> Settings {
        Settings {
            host: self.fields[HOST].text().trim().to_string(),
            user: self.fields[USER].text().trim().to_string(),
            password: self.fields[PASS].text().to_string(),
            show_hud: self.toggles[1],
            profile: self.profile,
            fill: self.toggles[0],
            force_rdv: self.base.force_rdv,
        }
    }

    fn kb_top(&self, ui: &Ui) -> f32 {
        self.h as f32 - home_zone(ui) - self.keyboard.h()
    }

    fn layout(&self, ui: &Ui) -> Layout {
        let col = (self.w as f32 - ui.pt(40.0)).min(ui.pt(600.0));
        let x = (self.w as f32 - col) / 2.0;
        let mut y = ui.pt(STATUS_PT + NAV_PT + 12.0) - self.scroll;
        if !self.status.is_empty() {
            y += ui.pt(30.0);
        }
        let (mut rows, mut headers, mut footers) = (vec![], vec![], vec![]);
        for (header, footer, kinds) in GROUPS {
            headers.push((header, x + ui.pt(16.0), y + ui.pt(16.0)));
            y += ui.pt(32.0);
            for (i, &kind) in kinds.iter().enumerate() {
                let h = ui.pt(if kind == Kind::Segments { SEGMENTS_PT } else { ROW_PT });
                rows.push(Row { kind, rect: (x, y, col, h), first: i == 0, last: i + 1 == kinds.len() });
                y += h;
            }
            if footer.is_empty() {
                y += ui.pt(8.0);
            } else {
                footers.push((footer, x + ui.pt(16.0), y + ui.pt(18.0)));
                y += ui.pt(40.0);
            }
        }
        Layout { rows, headers, footers, w: col }
    }

    fn field_frame(&self, ui: &Ui, r: Rect) -> Rect {
        let label = ui.pt(LABEL_PT);
        (r.0 + label, r.1, r.2 - label - ui.pt(16.0), r.3)
    }

    fn cancel_rect(&self, ui: &Ui) -> Rect {
        (ui.pt(8.0), ui.pt(STATUS_PT), ui.pt(90.0), ui.pt(NAV_PT))
    }

    fn done_rect(&self, ui: &Ui) -> Rect {
        (self.w as f32 - ui.pt(98.0), ui.pt(STATUS_PT), ui.pt(90.0), ui.pt(NAV_PT))
    }

    fn segment_track(&self, ui: &Ui, r: Rect) -> Rect {
        (r.0 + ui.pt(12.0), r.1 + ui.pt(12.0), r.2 - ui.pt(24.0), ui.pt(36.0))
    }

    fn hit(&self, ui: &Ui, lay: &Layout, x: f32, y: f32) -> Target {
        let inside = |r: Rect| x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3;
        if inside(self.cancel_rect(ui)) {
            return Target::Cancel;
        }
        if inside(self.done_rect(ui)) {
            return Target::Done;
        }
        for row in &lay.rows {
            if !inside(row.rect) {
                continue;
            }
            return match row.kind {
                Kind::Field(i) => Target::Field(i),
                Kind::Switch(i) => Target::Switch(i),
                Kind::Segments => {
                    let t = self.segment_track(ui, row.rect);
                    if inside(t) {
                        Target::Segment((((x - t.0) / (t.2 / 4.0)) as usize).min(3))
                    } else {
                        Target::None
                    }
                }
            };
        }
        Target::None
    }

    fn focus_field(&mut self, ui: &Ui, i: usize, t: f64) {
        self.focus = Some(i);
        for (n, f) in self.fields.iter_mut().enumerate() {
            f.focused = n == i;
        }
        self.keyboard.set_shown(true, t);
        let lay = self.layout(ui);
        if let Some(row) = lay.rows.iter().find(|r| r.kind == Kind::Field(i)) {
            let over = row.rect.1 + row.rect.3 + ui.pt(16.0) - self.kb_top(ui);
            self.scroll = (self.scroll + over).max(0.0);
        }
    }

    fn blur(&mut self, t: f64) {
        self.focus = None;
        self.fields.iter_mut().for_each(|f| f.focused = false);
        self.keyboard.set_shown(false, t);
        self.scroll = 0.0;
    }

    /// A key from the system keyboard (or a hardware one).
    pub fn key(&mut self, ui: &Ui, out: Out, t: f64) {
        match out {
            Out::Hide => self.blur(t),
            Out::Enter => match self.focus {
                Some(i) if i + 1 < self.fields.len() => self.focus_field(ui, i + 1, t),
                _ => self.blur(t),
            },
            other => {
                if let Some(i) = self.focus {
                    self.fields[i].edit(ui, &other, t);
                }
            }
        }
    }

    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) -> Option<Outcome> {
        let lay = self.layout(ui);
        let kb_top = self.kb_top(ui);
        let w = self.w as f32;
        match ev {
            Event::Down(x, y) => {
                if self.keyboard.shown() && y >= kb_top {
                    self.target = Target::Keyboard;
                    self.keyboard.down(x, y - kb_top, t);
                    return None;
                }
                self.target = self.hit(ui, &lay, x, y);
                match self.target {
                    Target::Field(i) => {
                        self.focus_field(ui, i, t);
                        let frame = self.field_frame(ui, self.layout(ui).rows.iter().find(|r| r.kind == Kind::Field(i))?.rect);
                        self.fields[i].touch(ui, ev, frame, w, t);
                    }
                    Target::None if self.keyboard.shown() => self.blur(t),
                    _ => {}
                }
            }
            Event::Move(x, y) => match self.target {
                Target::Keyboard => self.keyboard.moved(x, y - kb_top),
                Target::Field(i) => {
                    let frame = self.field_frame(ui, lay.rows.iter().find(|r| r.kind == Kind::Field(i))?.rect);
                    self.fields[i].touch(ui, ev, frame, w, t);
                }
                _ => {}
            },
            Event::Up(x, y) => {
                let target = std::mem::replace(&mut self.target, Target::None);
                match target {
                    Target::Keyboard => {
                        if let Some(out) = self.keyboard.up(x, y - kb_top, t) {
                            self.key(ui, out, t);
                        }
                    }
                    Target::Field(i) => {
                        let frame = self.field_frame(ui, lay.rows.iter().find(|r| r.kind == Kind::Field(i))?.rect);
                        self.fields[i].touch(ui, ev, frame, w, t);
                    }
                    Target::None => {}
                    _ if self.hit(ui, &lay, x, y) != target => {}
                    Target::Cancel => return Some(Outcome::Cancel),
                    Target::Done => {
                        if self.settings().is_complete() {
                            return Some(Outcome::Save(self.settings()));
                        }
                        self.status = "Enter the server, user and password.".into();
                    }
                    Target::Segment(k) => self.profile = Profile::ALL[k],
                    Target::Switch(i) => {
                        self.toggles[i] = !self.toggles[i];
                        self.flipped[i] = t;
                    }
                }
            }
            _ => {}
        }
        None
    }

    /// Ticks the fields and the keyboard; true when the page needs a frame.
    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        let mut changed = self.keyboard.animating() || self.keyboard.shown();
        for out in self.keyboard.tick(t) {
            self.key(ui, out, t);
        }
        for f in &mut self.fields {
            changed |= f.tick(ui, t);
        }
        changed || self.flipped.iter().any(|&f| t - f < SWITCH_MS)
    }

    pub fn next_wake(&self, t: f64) -> Option<f64> {
        if self.keyboard.animating() || self.flipped.iter().any(|&f| t - f < SWITCH_MS) {
            return Some(t);
        }
        self.fields.iter().filter_map(|f| f.next_wake(t)).reduce(f64::min)
    }

    fn shape_caption(&self) -> String {
        let s = self.profile.shape(self.toggles[0], self.display);
        format!("{}x{} at {} fps, up to {:.0} Mbit/s", s.width, s.height, s.fps, s.kbps as f32 / 1000.0)
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        if self.scale != ui.s.to_bits() {
            self.cache.clear();
            self.scale = ui.s.to_bits();
        }
        let (w, h) = (self.w as f32, self.h as f32);
        s.fill_rect(0, 0, self.w as usize, self.h as usize, BLACK);
        let lay = self.layout(ui);
        let nav_y = ui.pt(STATUS_PT + NAV_PT / 2.0);
        text_c(s, &mut self.cache, ui, "WoW", 17.0, Weight::Semibold, WHITE, w / 2.0, nav_y);
        text(s, &mut self.cache, ui, "Cancel", 17.0, Weight::Regular, BLUE, ui.pt(16.0), nav_y);
        let done = if self.settings().is_complete() { BLUE } else { SECONDARY };
        text_r(s, &mut self.cache, ui, "Done", 17.0, Weight::Semibold, done, w - ui.pt(16.0), nav_y);
        if !self.status.is_empty() {
            let y = ui.pt(STATUS_PT + NAV_PT + 22.0);
            text_c(s, &mut self.cache, ui, &self.status.clone(), 14.0, Weight::Regular, RED, w / 2.0, y);
        }
        for &(label, x, y) in &lay.headers {
            text(s, &mut self.cache, ui, label, 13.0, Weight::Regular, SECONDARY, x, y);
        }
        for &(label, x, y) in &lay.footers {
            let fit = ui.fit(label, 13.0, lay.w - ui.pt(32.0));
            text(s, &mut self.cache, ui, &fit, 13.0, Weight::Regular, SECONDARY, x, y);
        }
        for row in &lay.rows {
            self.draw_row(s, ui, &lay, row, t);
        }
        if self.keyboard.shown() || self.keyboard.animating() {
            let top = self.kb_top(ui);
            self.keyboard.draw(s, ui, top, t);
            let below = (top + self.keyboard.h() + self.keyboard.offset(t)).max(0.0) as usize;
            s.fill_rect(0, below, self.w as usize, self.h as usize, keyboard::BG);
        }
        let _ = h;
    }

    fn draw_row(&mut self, s: &mut Surface, ui: &Ui, lay: &Layout, row: &Row, t: f64) {
        let (x, y, w, h) = row.rect;
        fill_row(s, ui, row.rect, row.first, row.last, CELL);
        if !row.first {
            s.fill_rect((x + ui.pt(16.0)) as usize, y as usize, (w - ui.pt(16.0)) as usize, 1, SEPARATOR);
        }
        let mid = y + h / 2.0;
        match row.kind {
            Kind::Field(i) => {
                let label = ["Hosts", "User", "Password"][i];
                text(s, &mut self.cache, ui, label, 17.0, Weight::Regular, WHITE, x + ui.pt(16.0), mid);
                let frame = self.field_frame(ui, row.rect);
                self.fields[i].draw(s, ui, frame, t);
            }
            Kind::Switch(i) => {
                let label = ["Fill screen", "Show stats overlay"][i];
                text(s, &mut self.cache, ui, label, 17.0, Weight::Regular, WHITE, x + ui.pt(16.0), mid);
                let k = switch_progress(self.toggles[i], self.flipped[i], t);
                draw_switch(s, ui, x + w - ui.pt(16.0 + SWITCH_W_PT), mid - ui.pt(SWITCH_H_PT) / 2.0, k);
            }
            Kind::Segments => {
                let (tx, ty, tw, th) = self.segment_track(ui, row.rect);
                s.fill_rrect(tx, ty, tw, th, ui.pt(9.0), TRACK);
                let seg = tw / 4.0;
                let selected = Profile::ALL.iter().position(|&p| p == self.profile).unwrap_or(1);
                let pad = ui.pt(2.0);
                s.fill_rrect(tx + seg * selected as f32 + pad, ty + pad, seg - 2.0 * pad, th - 2.0 * pad, ui.pt(7.0), rgba(120, 120, 124, 255));
                for (n, name) in PROFILE_NAMES.iter().enumerate() {
                    let weight = if n == selected { Weight::Semibold } else { Weight::Regular };
                    text_c(s, &mut self.cache, ui, name, 14.0, weight, WHITE, tx + seg * (n as f32 + 0.5), ty + th / 2.0);
                }
                let caption = self.shape_caption();
                text_c(s, &mut self.cache, ui, &caption, 13.0, Weight::Regular, SECONDARY, x + w / 2.0, ty + th + ui.pt(18.0));
            }
        }
        let _ = lay;
    }
}
