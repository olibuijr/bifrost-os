//! The installer's screens and input handling. Pure state + drawing into a `Surface`, so it is testable
//! without a display; `bin/installer.rs` supplies the KMS output and evdev input.

use super::disks::{self, Disk, Kind, Mode, GIB};
use super::keymaps::{self, Keyboard, Typed};
use super::plan::{self, Config, Progress, Shared};
use crate::bare::input::Event;
use crate::gfx::{pack, rgba, Image, Surface};
use crate::shell::ui::{centred, Ui, Weight, BLACK, CELL, SECONDARY, SEPARATOR, SYSTEM_BLUE, SYSTEM_GREEN, SYSTEM_RED, TRACK, WHITE};
use std::sync::{Arc, Mutex};

/// The keyboard layout each installer language starts with (keyed by locale, layout id from `keymaps_data::LAYOUTS`).
pub const LANGUAGE_LAYOUTS: &[(&str, &str)] = &[
    ("en_US", "us"), ("en_GB", "gb"), ("is_IS", "is"), ("de_DE", "de"), ("fr_FR", "fr"), ("es_ES", "es"), ("it_IT", "it"),
    ("nl_NL", "nl"), ("sv_SE", "se"), ("nb_NO", "no"), ("da_DK", "dk"), ("fi_FI", "fi"), ("pl_PL", "pl"), ("pt_BR", "br"),
    ("cs_CZ", "cz"), ("tr_TR", "tr"),
];

/// The layout id for a locale such as `is_IS.UTF-8`, or None when no layout matches.
pub fn layout_for_locale(locale: &str) -> Option<&'static str> {
    let key = locale.split('.').next().unwrap_or(locale);
    LANGUAGE_LAYOUTS.iter().find(|(l, _)| *l == key).map(|(_, id)| *id)
}

pub const LANGUAGES: &[(&str, &str)] = &[
    ("English (United States)", "en_US.UTF-8"), ("English (United Kingdom)", "en_GB.UTF-8"), ("Íslenska", "is_IS.UTF-8"),
    ("Deutsch", "de_DE.UTF-8"), ("Français", "fr_FR.UTF-8"), ("Español", "es_ES.UTF-8"), ("Italiano", "it_IT.UTF-8"),
    ("Nederlands", "nl_NL.UTF-8"), ("Svenska", "sv_SE.UTF-8"), ("Norsk bokmål", "nb_NO.UTF-8"), ("Dansk", "da_DK.UTF-8"),
    ("Suomi", "fi_FI.UTF-8"), ("Polski", "pl_PL.UTF-8"), ("Português (Brasil)", "pt_BR.UTF-8"), ("Čeština", "cs_CZ.UTF-8"), ("Türkçe", "tr_TR.UTF-8"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Welcome,
    Language,
    Account,
    Disk,
    Confirm,
    Progress,
    Done,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Act {
    Next,
    Back,
    Lang(usize),
    Layout(usize),
    Focus(usize),
    Disk(usize),
    SetMode(Mode),
    Install,
    Erase,
    Restart,
}

type Hit = (f32, f32, f32, f32, Act);

pub struct App {
    pub ui: Ui,
    pub w: usize,
    pub h: usize,
    pub page: Page,
    lang: usize,
    layout: usize,
    column: usize,
    kb: Keyboard,
    test: String,
    fields: [String; 4],
    focus: usize,
    user_edited: bool,
    pub disks: Vec<Disk>,
    disk: Option<usize>,
    mode: Mode,
    pub progress: Shared,
    pub want_install: Option<Config>,
    pub want_restart: bool,
    hits: Vec<Hit>,
    down: Option<Act>,
    pub pointer: (f32, f32),
    /// the display draws the pointer itself (KMS cursor plane)
    pub hw_cursor: bool,
    logo: Option<Image>,
    bg: Vec<u32>,
    msg: (String, f64),
    t: f64,
}

fn inside(h: &Hit, x: f32, y: f32) -> bool {
    x >= h.0 && x < h.0 + h.2 && y >= h.1 && y < h.1 + h.3
}

pub fn size_text(b: u64) -> String {
    if b >= 1000 * 1_000_000_000 { format!("{:.1} TB", b as f64 / 1e12) } else { format!("{:.0} GB", b as f64 / 1e9) }
}

impl App {
    pub fn new(ui: Ui, w: usize, h: usize, disks: Vec<Disk>, logo: Option<Image>, bg: Option<Image>) -> Self {
        let mut a = App {
            ui, w, h, page: Page::Welcome, lang: 0, layout: 0, column: 0, kb: Keyboard::new(0), test: String::new(), fields: Default::default(), focus: 0,
            user_edited: false, disks, disk: None, mode: Mode::Erase, progress: Arc::new(Mutex::new(Progress::default())), want_install: None, want_restart: false,
            hits: vec![], down: None, pointer: (w as f32 / 2.0, h as f32 / 2.0), hw_cursor: false, logo, bg: vec![], msg: (String::new(), 0.0), t: 0.0,
        };
        a.disk = (a.disks.len() == 1).then_some(0);
        a.bg = make_background(bg, w, h);
        a
    }

    /// The still image behind the screens (the fallback when no video plays).
    pub fn background(&self) -> &[u32] {
        &self.bg
    }

    pub fn animating(&self) -> bool {
        matches!(self.page, Page::Progress) || self.t - self.msg.1 < 600.0 || matches!(self.page, Page::Account | Page::Language)
    }

    fn go(&mut self, p: Page) {
        self.page = p;
        self.hits.clear();
    }

    pub fn set_layout(&mut self, l: usize) {
        self.layout = l;
        self.kb.set_layout(l);
    }

    /// Picks a language and moves the keyboard to that language's layout, so a German system gets a German keyboard.
    pub fn set_lang(&mut self, i: usize) {
        self.lang = i;
        let locale = LANGUAGES[i].1;
        if let Some(li) = layout_for_locale(locale).and_then(keymaps::layout_index) {
            self.set_layout(li);
        }
    }

    // ---- input ---------------------------------------------------------------------------------

    pub fn event(&mut self, e: Event) {
        match e {
            Event::Down { x, y, .. } => {
                self.pointer = (x, y);
                self.down = self.hits.iter().rev().find(|h| inside(h, x, y)).map(|h| h.4);
            }
            Event::Move { x, y, .. } => self.pointer = (x, y),
            Event::Up { x, y, .. } => {
                self.pointer = (x, y);
                let hit = self.hits.iter().rev().find(|h| inside(h, x, y)).map(|h| h.4);
                if hit.is_some() && hit == self.down {
                    self.act(hit.unwrap());
                }
                self.down = None;
            }
            Event::Key { code, down } => {
                if let Some(t) = self.kb.feed(code, down) {
                    self.typed(t);
                }
            }
            Event::Switch { .. } | Event::Hover{..} | Event::Context{..} => {}
        }
    }

    fn act(&mut self, a: Act) {
        match a {
            Act::Next => self.next(),
            Act::Back => self.back(),
            Act::Lang(i) => self.set_lang(i),
            Act::Layout(i) => self.set_layout(i),
            Act::Focus(i) => self.focus = i,
            Act::Disk(i) => self.disk = Some(i),
            Act::SetMode(m) => self.mode = m,
            Act::Install => {
                if self.mode == Mode::Erase { self.go(Page::Confirm) } else { self.start() }
            }
            Act::Erase => self.start(),
            Act::Restart => self.want_restart = true,
        }
    }

    fn next(&mut self) {
        match self.page {
            Page::Welcome => self.go(Page::Language),
            Page::Language => self.go(Page::Account),
            Page::Account if self.account_error().is_none() => self.go(Page::Disk),
            Page::Disk if self.disk.is_some() => self.act(Act::Install),
            Page::Confirm => self.start(),
            _ => {}
        }
    }

    fn back(&mut self) {
        match self.page {
            Page::Language => self.go(Page::Welcome),
            Page::Account => self.go(Page::Language),
            Page::Disk => self.go(Page::Account),
            Page::Confirm => self.go(Page::Disk),
            // the password was wiped when the install started: ask for it again
            Page::Failed => self.go(Page::Account),
            _ => {}
        }
    }

    fn account_error(&self) -> Option<&'static str> {
        plan::account_error(&self.fields[0], &self.fields[1], &self.fields[2], &self.fields[3])
    }

    fn start(&mut self) {
        let Some(d) = self.disk.and_then(|i| self.disks.get(i)).cloned() else { return };
        let lay = keymaps::layouts().nth(self.layout).unwrap();
        *self.progress.lock().unwrap() = Progress::default();
        let password = std::mem::take(&mut self.fields[2]);
        self.fields[3].clear();
        self.want_install = Some(Config {
            disk: d, mode: self.mode, locale: LANGUAGES[self.lang].1.to_string(), layout: lay, full_name: self.fields[0].trim().to_string(),
            username: self.fields[1].clone(), password, hostname: "bifrost".into(), timezone: String::new(), mirror: String::new(),
        });
        self.go(Page::Progress);
    }

    fn typed(&mut self, t: Typed) {
        match (self.page, t) {
            (_, Typed::Enter) => self.next(),
            (Page::Welcome, _) => {}
            (Page::Account, Typed::Tab | Typed::Down) => self.focus = (self.focus + 1) % 4,
            (Page::Account, Typed::BackTab | Typed::Up) => self.focus = (self.focus + 3) % 4,
            (Page::Account, Typed::Backspace) => {
                self.fields[self.focus].pop();
                if self.focus == 1 {
                    self.user_edited = true;
                }
                self.autofill();
            }
            (Page::Account, Typed::Char(c)) if !c.is_control() => {
                let f = self.focus;
                if f != 1 || c.is_ascii() {
                    self.fields[f].push(c);
                }
                if f == 1 {
                    self.user_edited = true;
                }
                self.autofill();
            }
            (Page::Disk, k @ (Typed::Up | Typed::Down)) => {
                let n = self.disks.len();
                if n > 0 {
                    let cur = self.disk.unwrap_or(0);
                    self.disk = Some(if matches!(k, Typed::Down) { (cur + 1).min(n - 1) } else { cur.saturating_sub(1) });
                }
            }
            (Page::Language, Typed::Tab) => self.column = 1 - self.column,
            (Page::Language, Typed::Up) => self.step(-1),
            (Page::Language, Typed::Down) => self.step(1),
            (Page::Language, Typed::Backspace) => {
                self.test.pop();
            }
            (Page::Language, Typed::Char(c)) if !c.is_control() => {
                if self.test.chars().count() < 40 {
                    self.test.push(c)
                }
            }
            (Page::Disk, Typed::Left | Typed::Right | Typed::Char(' ')) => {
                if self.disk.and_then(|i| self.disks.get(i)).is_some_and(|d| d.can_use_free()) {
                    self.mode = if self.mode == Mode::Erase { Mode::FreeSpace } else { Mode::Erase };
                }
            }
            (_, Typed::Escape) => self.back(),
            _ => {}
        }
    }

    fn step(&mut self, d: i32) {
        if self.column == 0 {
            let i = (self.lang as i32 + d).clamp(0, LANGUAGES.len() as i32 - 1) as usize;
            self.set_lang(i);
        } else {
            let n = keymaps::layouts().count() as i32;
            self.set_layout((self.layout as i32 + d).clamp(0, n - 1) as usize);
        }
    }

    /// The account name follows the full name until the user edits it.
    fn autofill(&mut self) {
        if self.focus == 0 && !self.user_edited {
            let n: String = self.fields[0].to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect();
            self.fields[1] = n.trim_start_matches(|c: char| !c.is_ascii_lowercase() && c != '_').to_string();
        }
    }

    // ---- drawing -------------------------------------------------------------------------------

    pub fn draw(&mut self, s: &mut Surface, t: f64) {
        s.px[..self.w * self.h].copy_from_slice(&self.bg);
        self.draw_ui(s, t);
    }

    /// The screens only, onto whatever `s` already holds (a transparent overlay when a video plays behind).
    pub fn draw_ui(&mut self, s: &mut Surface, t: f64) {
        self.t = t;
        self.hits.clear();
        let msg = self.progress.lock().unwrap().message.clone();
        if msg != self.msg.0 {
            self.msg = (msg, t);
        }
        // smoked-glass container behind the content, so text stays readable over the video
        let (pw, ph) = ((self.u(700.0)).min(self.w as f32 * 0.96), self.h as f32 * 0.94);
        let (px, py) = ((self.w as f32 - pw) / 2.0, (self.h as f32 - ph) / 2.0);
        s.fill_rrect(px - 1.0, py - 1.0, pw + 2.0, ph + 2.0, self.u(22.0), rgba(255, 255, 255, 28));
        s.fill_rrect(px, py, pw, ph, self.u(21.0), rgba(18, 18, 20, 178));
        match self.page {
            Page::Welcome => self.welcome(s),
            Page::Language => self.language(s),
            Page::Account => self.account(s, t),
            Page::Disk | Page::Confirm => self.disk_page(s),
            Page::Progress | Page::Done | Page::Failed => self.progress_page(s, t),
        }
        if !self.hw_cursor {
            self.cursor(s);
        }
    }

    fn u(&self, v: f32) -> f32 {
        self.ui.pt(v)
    }

    fn text(&self, s: &mut Surface, txt: &str, pt: f32, wt: Weight, color: u32, cx: f32, y: f32) {
        let im = self.ui.text(txt, pt, wt, color);
        centred(s, &im, cx as i32, y as i32);
    }

    fn button(&mut self, s: &mut Surface, label: &str, cx: f32, y: f32, w: f32, fill: u32, enabled: bool, act: Act) {
        let (bw, bh) = (self.u(w), self.u(44.0));
        let x = cx - bw / 2.0;
        let pressed = self.down == Some(act);
        let fill = if !enabled { pack(44, 44, 46, 255) } else if pressed { rgba(0, 0, 0, 0) | dim(fill) } else { fill };
        s.fill_rrect(x, y, bw, bh, self.u(10.0), fill);
        let col = if enabled { WHITE } else { pack(99, 99, 102, 255) };
        let im = self.ui.text(label, 17.0, Weight::Semibold, col);
        s.blit(&im, (cx - im.w as f32 / 2.0) as i32, (y + (bh - im.h as f32) / 2.0) as i32, 255, 255);
        if enabled {
            self.hits.push((x, y, bw, bh, act));
        }
    }

    fn link(&mut self, s: &mut Surface, label: &str, cx: f32, y: f32, act: Act) {
        let im = self.ui.text(label, 17.0, Weight::Semibold, SYSTEM_BLUE);
        let (x, w, h) = (cx - im.w as f32 / 2.0, im.w as f32, im.h as f32);
        s.blit(&im, x as i32, y as i32, 255, 255);
        self.hits.push((x - self.u(12.0), y - self.u(8.0), w + self.u(24.0), h + self.u(16.0), act));
    }

    fn header(&self, s: &mut Surface, title: &str, sub: &str, y: f32) -> f32 {
        let cx = self.w as f32 / 2.0;
        self.text(s, title, 28.0, Weight::Bold, WHITE, cx, y);
        let mut y = y + self.u(40.0);
        for line in self.ui.wrap(sub, 17.0, Weight::Regular, self.u(520.0)) {
            self.text(s, &line, 17.0, Weight::Regular, SECONDARY, cx, y);
            y += self.u(24.0);
        }
        y + self.u(14.0)
    }

    fn welcome(&mut self, s: &mut Surface) {
        let cx = self.w as f32 / 2.0;
        let mut y = self.h as f32 * 0.26;
        if let Some(l) = self.logo.clone() {
            let k = self.u(112.0) / l.h as f32;
            let im = l.resized((l.w as f32 * k) as usize, (l.h as f32 * k) as usize);
            centred(s, &im, cx as i32, y as i32);
            y += im.h as f32 + self.u(26.0);
        }
        self.text(s, "Install BifrOSt", 34.0, Weight::Bold, WHITE, cx, y);
        self.text(s, "Set up a fresh BifrOSt on this computer in a few minutes.", 17.0, Weight::Regular, SECONDARY, cx, y + self.u(52.0));
        self.button(s, "Continue", cx, y + self.u(110.0), 200.0, SYSTEM_BLUE, true, Act::Next);
    }

    fn list_card(&mut self, s: &mut Surface, x: f32, y: f32, w: f32, rows: &[(String, bool)], row_h: f32, act: fn(usize) -> Act, active: bool) -> f32 {
        let (rh, h) = (self.u(row_h), self.u(row_h) * rows.len() as f32);
        s.fill_rrect(x, y, w, h, self.u(10.0), CELL);
        for (i, (label, sel)) in rows.iter().enumerate() {
            let ry = y + rh * i as f32;
            if *sel {
                s.fill_rrect(x + self.u(4.0), ry + self.u(3.0), w - self.u(8.0), rh - self.u(6.0), self.u(8.0), if active { SYSTEM_BLUE } else { pack(72, 72, 74, 255) });
            } else if i > 0 {
                s.fill_rect((x + self.u(16.0)) as usize, ry as usize, (w - self.u(16.0)) as usize, 1, SEPARATOR);
            }
            let im = self.ui.text(&self.ui.fit(label, 17.0, w - self.u(40.0)), 17.0, Weight::Regular, WHITE);
            s.blit(&im, (x + self.u(16.0)) as i32, (ry + (rh - im.h as f32) / 2.0) as i32, 255, 255);
            self.hits.push((x, ry, w, rh, act(i)));
        }
        y + h
    }

    fn language(&mut self, s: &mut Surface) {
        let cx = self.w as f32 / 2.0;
        let top = self.header(s, "Language and keyboard", "Choose your language and the layout of your keyboard. Try it in the box below.", self.h as f32 * 0.07);
        let colw = self.u(300.0).min(self.w as f32 * 0.42);
        let gap = self.u(20.0);
        let (lx, kx) = (cx - colw - gap / 2.0, cx + gap / 2.0);
        let avail = self.h as f32 - top - self.u(170.0);
        let rows_fit = ((avail / self.u(36.0)) as usize).max(4);
        let window = |sel: usize, n: usize| {
            let start = sel.saturating_sub(rows_fit / 2).min(n.saturating_sub(rows_fit));
            start..(start + rows_fit).min(n)
        };
        let lr = window(self.lang, LANGUAGES.len());
        let rows: Vec<(String, bool)> = lr.clone().map(|i| (LANGUAGES[i].0.to_string(), i == self.lang)).collect();
        let base = lr.start;
        self.hits.retain(|_| true);
        let end1 = self.list_card(s, lx, top, colw, &rows, 36.0, |i| Act::Lang(i), self.column == 0);
        // list_card emits indexes relative to the window: shift them
        let n = rows.len();
        for h in self.hits.iter_mut().rev().take(n) {
            if let Act::Lang(i) = h.4 {
                h.4 = Act::Lang(i + base);
            }
        }
        let layouts: Vec<keymaps::Layout> = keymaps::layouts().collect();
        let kr = window(self.layout, layouts.len());
        let krows: Vec<(String, bool)> = kr.clone().map(|i| (layouts[i].name.to_string(), i == self.layout)).collect();
        let kbase = kr.start;
        let end2 = self.list_card(s, kx, top, colw, &krows, 36.0, |i| Act::Layout(i), self.column == 1);
        let n = krows.len();
        for h in self.hits.iter_mut().rev().take(n) {
            if let Act::Layout(i) = h.4 {
                h.4 = Act::Layout(i + kbase);
            }
        }
        let y = end1.max(end2) + self.u(18.0);
        let bw = colw * 2.0 + gap;
        s.fill_rrect(cx - bw / 2.0, y, bw, self.u(44.0), self.u(10.0), CELL);
        let shown = if self.test.is_empty() { "Type here to test your keyboard".to_string() } else { self.test.clone() };
        let im = self.ui.text(&shown, 17.0, Weight::Regular, if self.test.is_empty() { SECONDARY } else { WHITE });
        s.blit(&im, (cx - bw / 2.0 + self.u(16.0)) as i32, (y + (self.u(44.0) - im.h as f32) / 2.0) as i32, 255, 255);
        let by = y + self.u(64.0);
        self.button(s, "Continue", cx, by, 200.0, SYSTEM_BLUE, true, Act::Next);
        self.link(s, "Back", cx, by + self.u(56.0), Act::Back);
    }

    fn account(&mut self, s: &mut Surface, t: f64) {
        let cx = self.w as f32 / 2.0;
        let top = self.header(s, "Create your account", "This account signs you in to BifrOSt. Nothing leaves this computer.", self.h as f32 * 0.12);
        let w = self.u(520.0).min(self.w as f32 * 0.9);
        let (x, rh) = (cx - w / 2.0, self.u(52.0));
        s.fill_rrect(x, top, w, rh * 4.0, self.u(10.0), CELL);
        let labels = ["Full name", "Account name", "Password", "Verify"];
        for i in 0..4 {
            let y = top + rh * i as f32;
            if i > 0 {
                s.fill_rect((x + self.u(16.0)) as usize, y as usize, (w - self.u(16.0)) as usize, 1, SEPARATOR);
            }
            let l = self.ui.text(labels[i], 17.0, Weight::Regular, WHITE);
            s.blit(&l, (x + self.u(16.0)) as i32, (y + (rh - l.h as f32) / 2.0) as i32, 255, 255);
            let val = if i >= 2 { "•".repeat(self.fields[i].chars().count()) } else { self.fields[i].clone() };
            let (txt, col) = if val.is_empty() { (["Ada Lovelace", "ada", "Required", "Re-enter password"][i].to_string(), pack(99, 99, 102, 255)) } else { (val, WHITE) };
            let im = self.ui.text(&self.ui.fit(&txt, 17.0, w - self.u(190.0)), 17.0, Weight::Regular, col);
            let tx = x + self.u(160.0);
            s.blit(&im, tx as i32, (y + (rh - im.h as f32) / 2.0) as i32, 255, 255);
            if i == self.focus && ((t / 500.0) as u64) % 2 == 0 {
                let cw = if self.fields[i].is_empty() { 0.0 } else { im.w as f32 };
                s.fill_rect((tx + cw + 2.0) as usize, (y + self.u(14.0)) as usize, (self.u(2.0)).max(1.0) as usize, self.u(24.0) as usize, SYSTEM_BLUE);
            }
            self.hits.push((x, y, w, rh, Act::Focus(i)));
        }
        let err = self.account_error();
        let shown = if self.fields[2].is_empty() && self.fields[3].is_empty() && self.fields[1].is_empty() { None } else { err };
        if let Some(e) = shown {
            self.text(s, e, 15.0, Weight::Semibold, SYSTEM_RED, cx, top + rh * 4.0 + self.u(16.0));
        }
        let by = top + rh * 4.0 + self.u(56.0);
        self.button(s, "Continue", cx, by, 200.0, SYSTEM_BLUE, err.is_none(), Act::Next);
        self.link(s, "Back", cx, by + self.u(56.0), Act::Back);
    }

    fn disk_page(&mut self, s: &mut Surface) {
        let cx = self.w as f32 / 2.0;
        let top = self.header(s, "Where should BifrOSt go?", "Pick a disk. You can keep what is already on it if there is free space.", self.h as f32 * 0.07);
        let w = self.u(560.0).min(self.w as f32 * 0.92);
        let x = cx - w / 2.0;
        let mut y = top;
        if self.disks.is_empty() {
            self.text(s, "No disks were found.", 17.0, Weight::Regular, SECONDARY, cx, y + self.u(20.0));
            y += self.u(60.0);
        }
        // the buttons are pinned to the bottom; everything above them must fit, so the disk list scrolls as a window
        let foot = (self.h as f32 - self.u(130.0)).max(0.0);
        let below = if self.disk.is_some() { self.u(26.0 + 18.0 + 88.0 + 14.0 + 34.0 + 20.0) } else { self.u(20.0) };
        let fit = (((foot - below - y) / self.u(48.0)) as usize).max(1);
        let all: Vec<(String, bool)> = self.disks.iter().enumerate().map(|(i, d)| (format!("{}  ·  {}  ·  {}", d.name(), size_text(d.size), d.path), Some(i) == self.disk)).collect();
        if !all.is_empty() {
            let n = all.len();
            let start = self.disk.unwrap_or(0).saturating_sub(fit / 2).min(n.saturating_sub(fit));
            let end = (start + fit).min(n);
            let h0 = self.hits.len();
            y = self.list_card(s, x, y, w, &all[start..end], 48.0, |i| Act::Disk(i), true) + self.u(20.0);
            for h in &mut self.hits[h0..] {
                if let Act::Disk(i) = h.4 {
                    h.4 = Act::Disk(i + start);
                }
            }
            if n > end - start {
                let note = format!("{}–{} of {} disks. Use the arrow keys to see the others.", start + 1, end, n);
                self.text(s, &note, 12.0, Weight::Regular, SECONDARY, cx, y - self.u(16.0));
            }
        }
        if let Some(d) = self.disk.and_then(|i| self.disks.get(i)).cloned() {
            let can_free = d.can_use_free();
            if !can_free {
                self.mode = Mode::Erase;
            }
            // the layout bar
            let bar_h = self.u(26.0);
            let seg = disks::preview(&d, self.mode);
            let total: u64 = seg.iter().map(|g| g.size).sum::<u64>().max(1);
            let mut sx = x;
            let min_w = self.u(6.0);
            let widths: Vec<f32> = seg.iter().map(|g| (w * g.size as f32 / total as f32).max(min_w)).collect();
            let scale = w / widths.iter().sum::<f32>();
            for (g, ww) in seg.iter().zip(&widths) {
                let ww = ww * scale;
                let col = match (g.new, g.kind) {
                    (true, Kind::Esp) => pack(142, 142, 147, 255),
                    (true, _) => SYSTEM_BLUE,
                    (_, Kind::Windows) => pack(48, 176, 199, 255),
                    (_, Kind::Linux) => pack(191, 90, 242, 255),
                    (_, Kind::Esp) => pack(99, 99, 102, 255),
                    (_, Kind::Mac) => pack(255, 159, 10, 255),
                    _ => pack(72, 72, 74, 255),
                };
                s.fill_rrect(sx + 1.0, y, (ww - 2.0).max(1.0), bar_h, self.u(5.0), col);
                if ww > self.u(90.0) {
                    let im = self.ui.text(&format!("{} {}", g.label, size_text(g.size)), 12.0, Weight::Medium, WHITE);
                    s.blit(&im, (sx + self.u(8.0)) as i32, (y + (bar_h - im.h as f32) / 2.0) as i32, 255, 255);
                }
                sx += ww;
            }
            y += bar_h + self.u(18.0);
            let modes = [(Mode::Erase, "Erase disk and install".to_string(), true), (Mode::FreeSpace, format!("Install next to what is there ({} free)", size_text(d.largest_free().map_or(0, |f| f.size))), can_free)];
            let rows: Vec<(String, bool)> = modes.iter().map(|m| (m.1.clone(), m.0 == self.mode)).collect();
            let ys = y;
            y = self.list_card(s, x, y, w, &rows, 44.0, |i| if i == 0 { Act::SetMode(Mode::Erase) } else { Act::SetMode(Mode::FreeSpace) }, true) + self.u(14.0);
            if !can_free {
                // the second option is not available: grey it out and drop its hit region
                self.hits.pop();
                s.fill_rrect(x, ys + self.u(44.0), w, self.u(44.0), self.u(10.0), rgba(0, 0, 0, 150));
            }
            let (msg, col) = if self.mode == Mode::Erase { (format!("Everything on {} will be erased.", d.name()), SYSTEM_RED) } else { ("Your existing files and systems stay as they are.".to_string(), SYSTEM_GREEN) };
            self.text(s, &msg, 15.0, Weight::Semibold, col, cx, y);
        }
        let label = if self.mode == Mode::Erase { "Erase and Install" } else { "Install" };
        y = foot;
        let fill = if self.mode == Mode::Erase { SYSTEM_RED } else { SYSTEM_BLUE };
        let ok = self.disk.is_some();
        self.button(s, label, cx, y, 220.0, fill, ok, Act::Install);
        self.link(s, "Back", cx, y + self.u(56.0), Act::Back);
        if self.page == Page::Confirm {
            s.fill_rect(0, 0, self.w, self.h, rgba(0, 0, 0, 170));
            let (cw, ch) = (self.u(380.0), self.u(190.0));
            let (cxl, cy) = (cx - cw / 2.0, self.h as f32 / 2.0 - ch / 2.0);
            s.fill_rrect(cxl, cy, cw, ch, self.u(14.0), pack(44, 44, 46, 255));
            let name = self.disk.and_then(|i| self.disks.get(i)).map_or(String::new(), |d| d.name());
            self.text(s, "Erase this disk?", 20.0, Weight::Semibold, WHITE, cx, cy + self.u(22.0));
            for (i, l) in self.ui.wrap(&format!("Everything on {name} will be deleted and BifrOSt installed. This cannot be undone."), 15.0, Weight::Regular, cw - self.u(48.0)).iter().enumerate() {
                self.text(s, l, 15.0, Weight::Regular, SECONDARY, cx, cy + self.u(58.0 + 20.0 * i as f32));
            }
            self.hits.clear();
            self.button(s, "Erase and Install", cx, cy + ch - self.u(60.0), 330.0, SYSTEM_RED, true, Act::Erase);
            self.link(s, "Cancel", cx, cy + ch - self.u(20.0) - self.u(8.0) + self.u(34.0), Act::Back);
        }
    }

    fn progress_page(&mut self, s: &mut Surface, t: f64) {
        let p = self.progress.lock().unwrap().clone();
        let cx = self.w as f32 / 2.0;
        let mut y = self.h as f32 * 0.2;
        if p.done {
            self.page = Page::Done;
        } else if p.failed.is_some() {
            self.page = Page::Failed;
        }
        if self.page == Page::Failed {
            self.text(s, "The installation could not finish", 24.0, Weight::Bold, WHITE, cx, y);
            let lines: Vec<String> = p.log.iter().rev().take(14).rev().cloned().collect();
            let mut ly = y + self.u(50.0);
            for l in lines.iter().chain(p.failed.iter().flat_map(|f| f.lines().map(|_| &lines[0]).take(0))) {
                let im = self.ui.text(&self.ui.fit(l, 12.0, self.w as f32 * 0.9), 12.0, Weight::Regular, SECONDARY);
                s.blit(&im, (cx - self.u(330.0).min(self.w as f32 * 0.45)) as i32, ly as i32, 255, 255);
                ly += self.u(17.0);
            }
            if let Some(f) = &p.failed {
                let first = f.lines().next().unwrap_or("");
                self.text(s, &self.ui.fit(first, 15.0, self.w as f32 * 0.9), 15.0, Weight::Semibold, SYSTEM_RED, cx, ly + self.u(8.0));
            }
            self.button(s, "Back", cx, ly + self.u(48.0), 200.0, SYSTEM_BLUE, true, Act::Back);
            return;
        }
        if self.page == Page::Done {
            let r = self.u(44.0);
            s.fill_rrect(cx - r, y, r * 2.0, r * 2.0, r, SYSTEM_GREEN);
            self.text(s, "✓", 48.0, Weight::Bold, WHITE, cx, y + self.u(14.0));
            self.text(s, "BifrOSt is installed", 34.0, Weight::Bold, WHITE, cx, y + self.u(110.0));
            self.text(s, "Restart to start using it. Remove the installer medium first if you used one.", 17.0, Weight::Regular, SECONDARY, cx, y + self.u(162.0));
            self.button(s, "Restart", cx, y + self.u(220.0), 200.0, SYSTEM_BLUE, true, Act::Restart);
            return;
        }
        if let Some(l) = self.logo.clone() {
            let k = self.u(88.0) / l.h as f32;
            let im = l.resized((l.w as f32 * k) as usize, (l.h as f32 * k) as usize);
            centred(s, &im, cx as i32, y as i32);
            y += im.h as f32 + self.u(24.0);
        }
        self.text(s, "Installing BifrOSt", 28.0, Weight::Bold, WHITE, cx, y);
        y += self.u(60.0);
        let bw = self.u(420.0).min(self.w as f32 * 0.8);
        s.fill_rrect(cx - bw / 2.0, y, bw, self.u(6.0), self.u(3.0), TRACK);
        let fw = (bw * p.percent as f32 / 100.0).max(self.u(6.0));
        s.fill_rrect(cx - bw / 2.0, y, fw, self.u(6.0), self.u(3.0), WHITE);
        y += self.u(30.0);
        // the status line fades in whenever it changes
        let a = (((t - self.msg.1) / 350.0).clamp(0.0, 1.0) * 255.0) as u32;
        let im = self.ui.text(&p.message, 17.0, Weight::Regular, WHITE);
        s.blit(&im, (cx - im.w as f32 / 2.0) as i32, y as i32, 255, a);
        y += self.u(30.0);
        let eta = p.eta_minutes().map_or(format!("{}%", p.percent), |m| format!("About {m} min remaining  ·  {}%", p.percent));
        self.text(s, &eta, 15.0, Weight::Regular, SECONDARY, cx, y);
        y += self.u(44.0);
        let sx = cx - self.u(140.0);
        for (i, name) in plan::STAGES.iter().enumerate() {
            let (dot, col) = if i < p.stage { (SYSTEM_GREEN, SECONDARY) } else if i == p.stage { (SYSTEM_BLUE, WHITE) } else { (pack(58, 58, 60, 255), pack(99, 99, 102, 255)) };
            let r = self.u(4.0);
            s.fill_rrect(sx, y + self.u(8.0), r * 2.0, r * 2.0, r, dot);
            let im = self.ui.text(name, 15.0, Weight::Regular, col);
            s.blit(&im, (sx + self.u(20.0)) as i32, y as i32, 255, 255);
            y += self.u(26.0);
        }
        self.text(s, "Do not turn off your computer.", 13.0, Weight::Regular, pack(99, 99, 102, 255), cx, self.h as f32 - self.u(40.0));
    }

    fn cursor(&self, s: &mut Surface) {
        // arrow pointer: white body with a dark outline, tip at the hotspot
        let (x, y) = self.pointer;
        let k = self.u(1.0);
        let n = 22;
        for row in 0..n {
            let r = row as f32;
            let half = if row < 16 { r * 0.62 } else { (22.0 - r) * 0.9 };
            let w = (half + 1.0).max(1.0);
            s.fill_rrect(x - 1.0 * k, y + r * k - 1.0 * k, (w + 2.0) * k, k + 2.0 * k, 0.0, rgba(0, 0, 0, 200));
        }
        for row in 0..n {
            let r = row as f32;
            let half = if row < 16 { r * 0.62 } else { (22.0 - r) * 0.9 };
            let w = (half + 1.0).max(1.0);
            s.fill_rrect(x, y + r * k, w * k, k, 0.0, WHITE);
        }
    }
}

fn dim(c: u32) -> u32 {
    let (r, g, b, a) = (c & 255, (c >> 8) & 255, (c >> 16) & 255, c >> 24);
    pack((r * 80 / 100) as u8, (g * 80 / 100) as u8, (b * 80 / 100) as u8, a as u8)
}

/// The still behind the screens: the photo scaled to cover and darkened, or plain black.
fn make_background(img: Option<Image>, w: usize, h: usize) -> Vec<u32> {
    let Some(img) = img else { return vec![BLACK; w * h] };
    let k = (w as f32 / img.w as f32).max(h as f32 / img.h as f32);
    let (sw, sh) = (((img.w as f32 * k).ceil() as usize).max(w), ((img.h as f32 * k).ceil() as usize).max(h));
    let sc = img.resized(sw, sh);
    let (ox, oy) = ((sw - w) / 2, (sh - h) / 2);
    let mut out = vec![0u32; w * h];
    for y in 0..h {
        for x in 0..w {
            let p = sc.px[(y + oy) * sw + x + ox];
            // darken (about 38%) with a vignette toward the corners
            let (dx, dy) = ((x as f32 / w as f32 - 0.5) * 2.0, (y as f32 / h as f32 - 0.5) * 2.0);
            let v = (0.42 - 0.18 * (dx * dx + dy * dy).min(1.6)).max(0.16);
            let (r, g, b) = ((p & 255) as f32 * v, ((p >> 8) & 255) as f32 * v, ((p >> 16) & 255) as f32 * v);
            out[y * w + x] = pack(r as u8, g as u8, b as u8, 255);
        }
    }
    out
}

#[allow(dead_code)]
pub const GB: u64 = GIB;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::Surface;
    use crate::installer::disks::parse_parted;

    fn ui() -> Ui {
        crate::shell::tests::ui()
    }

    fn app() -> App {
        let d = parse_parted("BYT;\n/dev/vda:17179869184B:virtio:512:512:unknown:Virtio Block Device:;\n").unwrap();
        App::new(ui(), 1280, 720, vec![d], None, None)
    }

    fn render(a: &mut App) -> Vec<u32> {
        let mut px = vec![0u32; a.w * a.h];
        let (w, h) = (a.w, a.h);
        let mut s = Surface { w, h, stride: w, px: &mut px, y0: 0 };
        a.draw(&mut s, 1000.0);
        px
    }

    fn click(a: &mut App, x: f32, y: f32) {
        a.event(Event::Down { x, y, t: 0.0 });
        a.event(Event::Up { x, y, t: 1.0 });
    }

    fn type_str(a: &mut App, s: &str) {
        for ch in s.chars() {
            let code = match ch {
                'a'..='z' => [30u16, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45, 21, 44][(ch as u8 - b'a') as usize],
                ' ' => 57,
                '1' => 2,
                _ => continue,
            };
            a.event(Event::Key { code, down: true });
            a.event(Event::Key { code, down: false });
        }
    }

    #[test]
    fn walks_through_the_pages_with_the_keyboard() {
        let mut a = app();
        render(&mut a);
        assert_eq!(a.page, Page::Welcome);
        a.event(Event::Key { code: 28, down: true });
        assert_eq!(a.page, Page::Language);
        a.event(Event::Key { code: 28, down: true });
        assert_eq!(a.page, Page::Account);
        a.event(Event::Key { code: 28, down: true });
        assert_eq!(a.page, Page::Account, "cannot continue without an account");
        type_str(&mut a, "ada lovelace");
        assert_eq!(a.fields[0], "ada lovelace");
        assert_eq!(a.fields[1], "adalovelace", "the account name follows the full name");
        let tab = |a: &mut App| {
            a.event(Event::Key { code: 15, down: true });
            a.event(Event::Key { code: 15, down: false });
        };
        tab(&mut a);
        tab(&mut a);
        type_str(&mut a, "secret1");
        tab(&mut a);
        type_str(&mut a, "secret1");
        assert_eq!((a.fields[2].as_str(), a.fields[3].as_str()), ("secret1", "secret1"));
        a.event(Event::Key { code: 28, down: true });
        assert_eq!(a.page, Page::Disk);
    }

    #[test]
    fn account_flow_by_clicks_and_install_request() {
        let mut a = app();
        render(&mut a);
        click(&mut a, 640.0, 450.0 * 0.0 + 400.0);
        render(&mut a);
        assert_eq!(a.page, Page::Language);
        a.act(Act::Next);
        a.fields = ["Ada Lovelace".into(), "ada".into(), "pw".into(), "pw".into()];
        render(&mut a);
        a.act(Act::Next);
        assert_eq!(a.page, Page::Disk);
        render(&mut a);
        a.act(Act::Install);
        assert_eq!(a.page, Page::Confirm, "erasing asks for confirmation first");
        a.act(Act::Back);
        assert_eq!(a.page, Page::Disk);
        a.act(Act::Install);
        a.act(Act::Erase);
        assert_eq!(a.page, Page::Progress);
        let cfg = a.want_install.take().unwrap();
        assert_eq!((cfg.username.as_str(), cfg.locale.as_str(), cfg.layout.id), ("ada", "en_US.UTF-8", "us"));
        assert_eq!(cfg.mode, Mode::Erase);
    }

    #[test]
    fn every_language_has_a_real_keyboard_layout() {
        for (_, locale) in LANGUAGES {
            let id = layout_for_locale(locale).unwrap_or_else(|| panic!("no layout for {locale}"));
            assert!(keymaps::layout_index(id).is_some(), "{locale} -> {id} is not a layout");
        }
        assert_eq!(layout_for_locale("is_IS.UTF-8"), Some("is"));
    }

    #[test]
    fn choosing_a_language_moves_the_keyboard() {
        let mut a = app();
        a.set_lang(LANGUAGES.iter().position(|l| l.1 == "is_IS.UTF-8").unwrap());
        assert_eq!(keymaps::layouts().nth(a.layout).unwrap().id, "is");
    }

    #[test]
    fn language_page_changes_the_keyboard_layout_live() {
        let mut a = app();
        a.page = Page::Language;
        a.set_layout(keymaps::layout_index("is").unwrap());
        a.event(Event::Key { code: 12, down: true });
        assert_eq!(a.test, "ö");
    }

    #[test]
    fn every_page_draws_something() {
        let mut a = app();
        for p in [Page::Welcome, Page::Language, Page::Account, Page::Disk, Page::Confirm, Page::Progress] {
            a.page = p;
            let px = render(&mut a);
            assert!(px.iter().any(|&c| c == WHITE || c == SYSTEM_BLUE || c == SYSTEM_RED), "{p:?} drew nothing visible");
        }
    }

    #[test]
    fn failed_and_done_pages_follow_the_progress() {
        let mut a = app();
        a.page = Page::Progress;
        a.progress.lock().unwrap().failed = Some("pacstrap failed".into());
        render(&mut a);
        assert_eq!(a.page, Page::Failed);
        let mut b = app();
        b.page = Page::Progress;
        b.progress.lock().unwrap().done = true;
        render(&mut b);
        assert_eq!(b.page, Page::Done);
    }

    #[test]
    fn size_text_units() {
        assert_eq!(size_text(512_110_190_592), "512 GB");
        assert_eq!(size_text(2_000_398_934_016), "2.0 TB");
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use crate::installer::disks::parse_parted;

    #[test]
    #[ignore]
    fn draw_cost() {
        for (w, h) in [(1280usize, 720usize), (1920, 1080), (3840, 2160)] {
            let d = parse_parted("BYT;\n/dev/vda:17179869184B:virtio:512:512:unknown:Virtio Block Device:;\n").unwrap();
            let mut a = App::new(crate::shell::tests::ui(), w, h, vec![d], None, None);
            a.page = Page::Disk;
            let mut px = vec![0u32; w * h];
            let t0 = std::time::Instant::now();
            for i in 0..20 {
                let mut s = Surface { w, h, stride: w, px: &mut px, y0: 0 };
                a.draw(&mut s, i as f64 * 40.0);
            }
            println!("{w}x{h}: {:.1} ms per full draw", t0.elapsed().as_secs_f64() * 50.0);
        }
    }
}

/// The pointer as a 64x64 ARGB sprite for the KMS cursor plane: white arrow, dark outline, tip at (0,0).
pub fn cursor_argb() -> Vec<u32> {
    let mut v = vec![0u32; 64 * 64];
    let inside = |x: i32, y: i32| -> bool {
        // arrow: a wedge 0..=16 rows tall, then a stem
        if (0..17).contains(&y) {
            x >= 0 && (x as f32) <= y as f32 * 0.62 + 0.5
        } else if (17..24).contains(&y) {
            x >= 3 && x <= 6
        } else {
            false
        }
    };
    for y in 0..64i32 {
        for x in 0..64i32 {
            if inside(x, y) {
                v[(y * 64 + x) as usize] = 0xffff_ffff;
            } else if (-1..=1).any(|dy| (-1..=1).any(|dx| inside(x + dx, y + dy))) {
                v[(y * 64 + x) as usize] = 0xe000_0000;
            }
        }
    }
    v
}
