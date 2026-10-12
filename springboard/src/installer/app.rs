//! The installer's screens and input handling. Pure state + drawing into a `Surface`, so it is testable
//! without a display; `bin/installer.rs` supplies the KMS output and evdev input.

use super::disks::{self, Disk, Kind, Mode, GIB};
use super::keymaps::{self, Keyboard, Typed};
use super::plan::{self, Config, Progress, Shared};
use crate::shell::wallpaper_page::Wallpaper;
use bifrost_render::appearance::{self as ap, Accent};
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
    /// Appearance, accent colour and wallpaper on one page.
    Look,
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
    Look(ap::Mode),
    Tint(Accent),
    Wall(usize),
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
    look: ap::Mode,
    accent: Accent,
    /// the section of the Look page the arrow keys act on: 0 appearance, 1 accent, 2 wallpaper
    look_row: usize,
    /// (id, name, thumbnail) of every wallpaper tile; the first is the packaged default (empty id)
    walls: Vec<(String, String, Option<Image>)>,
    wall: usize,
    wall_cols: usize,
    /// the thumbnails scaled to the tile size they were last drawn at
    wall_scaled: ((usize, usize), Vec<Option<Image>>),
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
    published: String,
}

const FIELD_IDS: [&str; 4] = ["name", "username", "password", "verify"];

fn act_id(a: Act) -> String {
    match a {
        Act::Next => "next".into(),
        Act::Back => "back".into(),
        Act::Lang(i) => format!("lang{i}"),
        Act::Layout(i) => format!("layout{i}"),
        Act::Focus(i) => format!("field{i}"),
        Act::Look(_) => "look".into(),
        Act::Tint(_) => "tint".into(),
        Act::Wall(i) => format!("wall{i}"),
        Act::Disk(i) => format!("disk{i}"),
        Act::SetMode(_) => "mode".into(),
        Act::Install => "install".into(),
        Act::Erase => "erase".into(),
        Act::Restart => "restart".into(),
    }
}

type Rect = (f32, f32, f32, f32);

/// The state file: one line per field, rectangles in panel pixels.
pub fn format_state(size: (usize, usize), screen: &str, stage: &str, percent: u32, buttons: &[(String, Rect)], fields: &[(String, Rect, bool)]) -> String {
    let mut o = format!("size={} {}\nscreen={screen}\nstage={stage}\npercent={percent}\n", size.0, size.1);
    for (id, r) in buttons {
        o += &format!("button={id} {} {} {} {}\n", r.0.round() as i32, r.1.round() as i32, r.2.round() as i32, r.3.round() as i32);
    }
    for (id, r, f) in fields {
        o += &format!("field={id} {} {} {} {} {}\n", r.0.round() as i32, r.1.round() as i32, r.2.round() as i32, r.3.round() as i32, *f as u8);
    }
    o
}

fn inside(h: &Hit, x: f32, y: f32) -> bool {
    x >= h.0 && x < h.0 + h.2 && y >= h.1 && y < h.1 + h.3
}

/// The system wallpapers packed in `assets` (`wallpapers/index.ini` + `<id>.thumb.raw`); empty when there are none.
pub fn load_wallpapers(assets: &str) -> Vec<Wallpaper> {
    let index = crate::shell::prefs::Prefs::load(format!("{assets}/wallpapers/index.ini"));
    index
        .entries()
        .filter_map(|(id, name)| {
            let thumb = Image::from_raw(&std::fs::read(format!("{assets}/wallpapers/{id}.thumb.raw")).ok()?)?;
            Some(Wallpaper { id: id.to_string(), name: name.to_string(), thumb })
        })
        .collect()
}

pub fn size_text(b: u64) -> String {
    if b >= 1000 * 1_000_000_000 { format!("{:.1} TB", b as f64 / 1e12) } else { format!("{:.0} GB", b as f64 / 1e9) }
}

impl App {
    pub fn new(ui: Ui, w: usize, h: usize, disks: Vec<Disk>, logo: Option<Image>, bg: Option<Image>) -> Self {
        let mut a = App {
            ui, w, h, page: Page::Welcome, lang: 0, layout: 0, column: 0, kb: Keyboard::new(0), test: String::new(), fields: Default::default(), focus: 0,
            user_edited: false, disks, disk: None, mode: Mode::Erase,
            look: ap::Mode::Dark, accent: Accent::Blue, look_row: 0, walls: vec![(String::new(), "Default".into(), None)], wall: 0, wall_cols: 4,
            wall_scaled: ((0, 0), vec![]), progress: Arc::new(Mutex::new(Progress::default())), want_install: None, want_restart: false,
            hits: vec![], down: None, pointer: (w as f32 / 2.0, h as f32 / 2.0), hw_cursor: false, logo, bg: vec![], msg: (String::new(), 0.0), t: 0.0, published: String::new(),
        };
        a.disk = (a.disks.len() == 1).then_some(0);
        a.bg = make_background(bg, w, h);
        a
    }

    /// Offers the system wallpapers on the Look page (after the packaged default, which stays first).
    pub fn set_wallpapers(&mut self, list: Vec<Wallpaper>) {
        self.walls.truncate(1);
        self.walls.extend(list.into_iter().filter(|w| plan::valid_wallpaper_id(&w.id)).map(|w| (w.id, w.name, Some(w.thumb))));
        self.wall = 0;
        self.wall_scaled = ((0, 0), vec![]);
    }

    /// Whether the Look page is drawn light: the pick, or the hour of day for Auto (UTC: no timezone is chosen yet).
    fn look_light(&self) -> bool {
        match self.look {
            ap::Mode::Light => true,
            ap::Mode::Dark => false,
            ap::Mode::Auto => {
                let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                ap::auto_is_light((secs / 3600 % 24) as u32)
            }
        }
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
            Act::Look(m) => {
                self.look = m;
                self.look_row = 0;
            }
            Act::Tint(a) => {
                self.accent = a;
                self.look_row = 1;
            }
            Act::Wall(i) => {
                self.wall = i.min(self.walls.len() - 1);
                self.look_row = 2;
            }
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
            Page::Account if self.account_error().is_none() => self.go(Page::Look),
            Page::Look => self.go(Page::Disk),
            Page::Disk if self.disk.is_some() => self.act(Act::Install),
            Page::Confirm => self.start(),
            _ => {}
        }
    }

    fn back(&mut self) {
        match self.page {
            Page::Language => self.go(Page::Welcome),
            Page::Account => self.go(Page::Language),
            Page::Look => self.go(Page::Account),
            Page::Disk => self.go(Page::Look),
            Page::Confirm => self.go(Page::Disk),
            // the password was wiped when the install started: ask for it again
            Page::Failed => self.go(Page::Account),
            _ => {}
        }
    }

    fn account_error(&self) -> Option<&'static str> {
        plan::account_error(&self.fields[0], &self.fields[1], &self.fields[2], &self.fields[3])
    }

    /// Straight to the progress page with a ready config (the unattended lab install): no questions asked.
    pub fn begin(&mut self, cfg: Config) {
        *self.progress.lock().unwrap() = Progress::default();
        self.want_install = Some(cfg);
        self.go(Page::Progress);
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
            appearance: self.look.key().to_string(), accent: self.accent.key().to_string(),
            wallpaper: self.walls.get(self.wall).map(|w| w.0.clone()).unwrap_or_default(),
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
            (Page::Look, Typed::Tab) => self.look_row = (self.look_row + 1) % 3,
            (Page::Look, Typed::BackTab) => self.look_row = (self.look_row + 2) % 3,
            (Page::Look, k @ (Typed::Left | Typed::Right)) => self.look_step(if matches!(k, Typed::Right) { 1 } else { -1 }),
            (Page::Look, k @ (Typed::Up | Typed::Down)) => {
                let d: i32 = if matches!(k, Typed::Down) { 1 } else { -1 };
                if self.look_row == 2 && (0..self.walls.len() as i32).contains(&(self.wall as i32 + d * self.wall_cols as i32)) {
                    self.look_step(d * self.wall_cols as i32);
                } else {
                    self.look_row = (self.look_row as i32 + d).clamp(0, 2) as usize;
                }
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

    /// Moves the pick of the focused Look section by `d` places (clamped).
    fn look_step(&mut self, d: i32) {
        let modes = [ap::Mode::Light, ap::Mode::Dark, ap::Mode::Auto];
        match self.look_row {
            0 => {
                let i = modes.iter().position(|m| *m == self.look).unwrap_or(1) as i32;
                self.look = modes[(i + d).clamp(0, 2) as usize];
            }
            1 => {
                let i = Accent::ALL.iter().position(|a| *a == self.accent).unwrap_or(1) as i32;
                self.accent = Accent::ALL[(i + d).clamp(0, Accent::ALL.len() as i32 - 1) as usize];
            }
            _ => self.wall = (self.wall as i32 + d).clamp(0, self.walls.len() as i32 - 1) as usize,
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
        // the Look page follows the scheme being picked; the other pages keep the dark glass
        let (edge, fill) = if self.page == Page::Look && self.look_light() { (rgba(0, 0, 0, 36), rgba(244, 244, 247, 236)) } else { (rgba(255, 255, 255, 28), rgba(18, 18, 20, 178)) };
        s.fill_rrect(px - 1.0, py - 1.0, pw + 2.0, ph + 2.0, self.u(22.0), edge);
        s.fill_rrect(px, py, pw, ph, self.u(21.0), fill);
        match self.page {
            Page::Look => self.look_page(s),
            Page::Welcome => self.welcome(s),
            Page::Language => self.language(s),
            Page::Account => self.account(s, t),
            Page::Disk | Page::Confirm => self.disk_page(s),
            Page::Progress | Page::Done | Page::Failed => self.progress_page(s, t),
        }
        if !self.hw_cursor {
            self.cursor(s);
        }
        self.publish_state();
    }

    fn page_name(&self) -> &'static str {
        match self.page {
            Page::Welcome => "welcome",
            Page::Language => "language",
            Page::Account => "account",
            Page::Look => "look",
            Page::Disk => "disk",
            Page::Confirm => "confirm",
            Page::Progress => "progress",
            Page::Done => "done",
            Page::Failed => "failed",
        }
    }

    /// Publish the visible screen for the QA driver (`midget.py qa-install`), only when it changed.
    fn publish_state(&mut self) {
        let (stage, percent) = {
            let p = self.progress.lock().unwrap();
            let name = plan::STAGES.get(p.stage).map(|n| n.to_string()).unwrap_or_default();
            (format!("{} {}", p.stage, if p.message.is_empty() { name } else { p.message.clone() }), p.percent as u32)
        };
        let mut buttons = Vec::new();
        let mut fields = Vec::new();
        for h in &self.hits {
            let r = (h.0, h.1, h.2, h.3);
            match h.4 {
                Act::Focus(i) => fields.push((FIELD_IDS.get(i).copied().unwrap_or("field").to_string(), r, i == self.focus)),
                a => buttons.push((act_id(a), r)),
            }
        }
        let text = format_state((self.w, self.h), self.page_name(), &stage, percent, &buttons, &fields);
        if text == self.published {
            return;
        }
        self.published = text.clone();
        let dir = std::path::Path::new("/run/bifrost-installer");
        if std::fs::create_dir_all(dir).is_ok() {
            let tmp = dir.join("state.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, dir.join("state"));
            }
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
        self.link_c(s, label, cx, y, act, SYSTEM_BLUE);
    }

    fn link_c(&mut self, s: &mut Surface, label: &str, cx: f32, y: f32, act: Act, color: u32) {
        let im = self.ui.text(label, 17.0, Weight::Semibold, color);
        let (x, w, h) = (cx - im.w as f32 / 2.0, im.w as f32, im.h as f32);
        s.blit(&im, x as i32, y as i32, 255, 255);
        self.hits.push((x - self.u(12.0), y - self.u(8.0), w + self.u(24.0), h + self.u(16.0), act));
    }

    fn header(&self, s: &mut Surface, title: &str, sub: &str, y: f32) -> f32 {
        self.header_c(s, title, sub, y, WHITE, SECONDARY)
    }

    fn header_c(&self, s: &mut Surface, title: &str, sub: &str, y: f32, fg: u32, sub_fg: u32) -> f32 {
        let cx = self.w as f32 / 2.0;
        self.text(s, title, 28.0, Weight::Bold, fg, cx, y);
        let mut y = y + self.u(40.0);
        for line in self.ui.wrap(sub, 17.0, Weight::Regular, self.u(520.0)) {
            self.text(s, &line, 17.0, Weight::Regular, sub_fg, cx, y);
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

    /// A miniature desktop in the given scheme (Auto: light on the left, dark on the right), the window tinted with the accent.
    fn look_preview(&self, s: &mut Surface, (x, y, w, h): (f32, f32, f32, f32), m: ap::Mode, accent: Accent) {
        let (lbg, lbar, lwin) = (pack(226, 226, 231, 255), pack(238, 238, 242, 255), pack(255, 255, 255, 255));
        let (dbg, dbar, dwin) = (pack(44, 44, 48, 255), pack(30, 30, 32, 255), pack(66, 66, 70, 255));
        let (r, rw) = (self.u(8.0), self.u(4.0));
        let light_first = m != ap::Mode::Dark;
        let (bg, bar, win, acc) = if light_first { (lbg, lbar, lwin, accent.colours().1) } else { (dbg, dbar, dwin, accent.colours().0) };
        s.fill_rrect(x, y, w, h, r, if m == ap::Mode::Auto { dbg } else { bg });
        if m == ap::Mode::Auto {
            s.fill_rrect(x, y, w / 2.0 + r, h, r, lbg);
            s.fill_rect((x + w / 2.0) as usize, y as usize, r as usize + 1, h as usize, dbg);
        }
        let (wx, wy, ww, wh) = (x + w * 0.16, y + h * 0.30, w * 0.68, h * 0.56);
        s.fill_rrect(wx, wy, ww, wh, rw, win);
        s.fill_rrect(wx, wy, ww, wh * 0.2, rw, bar);
        if m == ap::Mode::Auto {
            let (hx, hw) = ((wx + ww / 2.0) as usize, (ww / 2.0) as usize);
            s.fill_rect(hx, wy as usize, hw, wh as usize, dwin);
            s.fill_rect(hx, wy as usize, hw, (wh * 0.2) as usize, dbar);
        }
        s.fill_rrect(wx + ww * 0.08, wy + wh * 0.5, ww * 0.34, wh * 0.14, self.u(2.0), acc);
    }

    /// Light or dark, the accent colour and the wallpaper, together on one page. The page itself is drawn in the scheme
    /// and accent being picked, so each choice shows at once.
    fn look_page(&mut self, s: &mut Surface) {
        let light = self.look_light();
        let (fg, sub) = if light { (pack(28, 28, 30, 255), pack(99, 99, 102, 255)) } else { (WHITE, SECONDARY) };
        let acc = if light { self.accent.colours().1 } else { self.accent.colours().0 };
        let neutral = if light { pack(208, 208, 214, 255) } else { pack(58, 58, 60, 255) };
        let cx = self.w as f32 / 2.0;
        let mut y = self.header_c(s, "Choose your look", "Pick light or dark, an accent colour and a wallpaper.", self.h as f32 * 0.05, fg, sub);
        let section = |this: &Self, s: &mut Surface, label: &str, row: usize, y: f32| {
            this.text(s, label, 13.0, Weight::Semibold, if this.look_row == row { acc } else { sub }, cx, y);
        };
        // appearance
        section(self, s, "Appearance", 0, y);
        y += self.u(24.0);
        let gap = self.u(16.0);
        let tw = self.u(120.0).min((self.w as f32 * 0.88 - gap * 2.0) / 3.0);
        let th = tw * 0.62;
        let x0 = cx - (tw * 3.0 + gap * 2.0) / 2.0;
        for (i, (m, label)) in [(ap::Mode::Light, "Light"), (ap::Mode::Dark, "Dark"), (ap::Mode::Auto, "Auto")].into_iter().enumerate() {
            let x = x0 + (tw + gap) * i as f32;
            self.look_preview(s, (x, y, tw, th), m, self.accent);
            if self.look == m {
                let p = self.u(3.0);
                s.stroke_rrect(x - p, y - p, tw + 2.0 * p, th + 2.0 * p, self.u(8.0) + p, self.u(2.5), acc);
            }
            self.text(s, label, 13.0, if self.look == m { Weight::Semibold } else { Weight::Regular }, fg, x + tw / 2.0, y + th + self.u(8.0));
            self.hits.push((x, y, tw, th + self.u(28.0), Act::Look(m)));
        }
        y += th + self.u(44.0);
        // accent colour
        section(self, s, "Accent colour", 1, y);
        y += self.u(26.0);
        let n = Accent::ALL.len() as f32;
        let dgap = self.u(14.0);
        let d = self.u(26.0).min((self.w as f32 * 0.88 - dgap * (n - 1.0)) / n);
        let dx0 = cx - (d * n + dgap * (n - 1.0)) / 2.0;
        for (i, a) in Accent::ALL.into_iter().enumerate() {
            let x = dx0 + (d + dgap) * i as f32;
            let col = if light { a.colours().1 } else { a.colours().0 };
            if a == Accent::Multicolour {
                // the automatic choice: stacked rings of several accents
                for (k, c) in [Accent::Orange, Accent::Pink, Accent::Purple, Accent::Blue].into_iter().enumerate() {
                    let inset = d * 0.17 * k as f32;
                    s.fill_rrect(x + inset, y + inset, d - 2.0 * inset, d - 2.0 * inset, (d - 2.0 * inset) / 2.0, if light { c.colours().1 } else { c.colours().0 });
                }
            } else {
                s.fill_rrect(x, y, d, d, d / 2.0, col);
            }
            if self.accent == a {
                let p = self.u(4.0);
                s.stroke_rrect(x - p, y - p, d + 2.0 * p, d + 2.0 * p, (d + 2.0 * p) / 2.0, self.u(2.0), col);
            }
            self.hits.push((x - dgap / 2.0, y - self.u(4.0), d + dgap, d + self.u(8.0), Act::Tint(a)));
        }
        y += d + self.u(14.0);
        let name = if self.accent == Accent::Multicolour { "Multicolour".to_string() } else { self.accent.label().to_string() };
        self.text(s, &name, 12.0, Weight::Regular, sub, cx, y);
        y += self.u(32.0);
        // wallpaper
        section(self, s, "Wallpaper", 2, y);
        y += self.u(26.0);
        let foot = (self.h as f32 - self.u(130.0)).max(0.0);
        let count = self.walls.len();
        // a paged grid: as many columns as the panel takes at a readable tile size, as many rows as fit above the
        // buttons (room kept for the name and the page dots), and the page that holds the selection
        let wgap = self.u(12.0);
        // inside the page's panel (560 pt wide at most), not the screen
        let max_w = (self.w as f32 * 0.88).min(self.u(560.0));
        let cols = (((max_w + wgap) / (self.u(110.0) + wgap)).floor() as usize).clamp(1, 6).min(count.max(1));
        self.wall_cols = cols;
        let tw = (max_w - wgap * (cols as f32 - 1.0)) / cols as f32;
        let th = tw * 9.0 / 16.0;
        let avail = foot - y - self.u(64.0);
        let rows = (((avail + wgap) / (th + wgap)).floor() as usize).max(1).min(count.div_ceil(cols).max(1));
        let per_page = cols * rows;
        let pages = count.div_ceil(per_page).max(1);
        let page = self.wall.min(count.saturating_sub(1)) / per_page;
        let gx0 = cx - (tw * cols as f32 + wgap * (cols as f32 - 1.0)) / 2.0;
        let size = (tw as usize, th as usize);
        if self.wall_scaled.0 != size {
            let scaled = self.walls.iter().map(|w| w.2.as_ref().map(|im| im.resized(size.0.max(1), size.1.max(1)))).collect();
            self.wall_scaled = (size, scaled);
        }
        for i in page * per_page..((page + 1) * per_page).min(count) {
            let k = i - page * per_page;
            let (x, ty) = (gx0 + (tw + wgap) * (k % cols) as f32, y + (th + wgap) * (k / cols) as f32);
            match self.wall_scaled.1.get(i).and_then(|o| o.as_ref()) {
                Some(im) => s.blit(im, x as i32, ty as i32, 255, 255),
                None => {
                    s.fill_rrect(x, ty, tw, th, self.u(4.0), neutral);
                    let label = self.ui.fit(&self.walls[i].1, 11.0, tw - self.u(8.0));
                    self.text(s, &label, 11.0, Weight::Regular, fg, x + tw / 2.0, ty + th / 2.0 - self.u(7.0));
                }
            }
            if i == self.wall {
                let p = self.u(3.0);
                s.stroke_rrect(x - p, ty - p, tw + 2.0 * p, th + 2.0 * p, self.u(6.0), self.u(2.5), acc);
            }
            self.hits.push((x, ty, tw, th, Act::Wall(i)));
        }
        y += (th + wgap) * rows as f32;
        let picked = self.walls[self.wall.min(count - 1)].1.clone();
        self.text(s, &picked, 13.0, Weight::Regular, sub, cx, y);
        if pages > 1 {
            // page dots; a click jumps to that page's first wallpaper
            let (dd, dg) = (self.u(7.0), self.u(10.0));
            let dy = y + self.u(26.0);
            let px0 = cx - (dd * pages as f32 + dg * (pages as f32 - 1.0)) / 2.0;
            for p in 0..pages {
                let x = px0 + (dd + dg) * p as f32;
                s.fill_rrect(x, dy, dd, dd, dd / 2.0, if p == page { fg } else { neutral });
                self.hits.push((x - dg / 2.0, dy - self.u(8.0), dd + dg, dd + self.u(16.0), Act::Wall(p * per_page)));
            }
        }
        self.button(s, "Continue", cx, foot, 200.0, acc, true, Act::Next);
        self.link_c(s, "Back", cx, foot + self.u(56.0), Act::Back, acc);
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
        assert_eq!(a.page, Page::Look, "the look comes after the account");
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
        assert_eq!(a.page, Page::Look);
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
        assert_eq!((cfg.appearance.as_str(), cfg.accent.as_str(), cfg.wallpaper.as_str()), ("dark", "blue", ""), "untouched choices keep today's look");
    }

    /// The centre of the hit region of `act` on the page as last drawn.
    fn centre_of(a: &App, act: Act) -> (f32, f32) {
        let h = a.hits.iter().find(|h| h.4 == act).unwrap_or_else(|| panic!("no hit region for {act:?}"));
        (h.0 + h.2 / 2.0, h.1 + h.3 / 2.0)
    }

    fn look_app() -> App {
        let mut a = app();
        a.set_wallpapers(
            ["gullfoss_canyon", "kirkjufell_dusk"]
                .iter()
                .map(|id| Wallpaper { id: id.to_string(), name: id.replace('_', " "), thumb: Image { w: 320, h: 180, px: vec![pack(30, 90, 140, 255); 320 * 180] } })
                .collect(),
        );
        a.fields = ["Ada Lovelace".into(), "ada".into(), "pw".into(), "pw".into()];
        a.page = Page::Look;
        render(&mut a);
        a
    }

    #[test]
    fn the_look_page_sits_between_account_and_disk() {
        let mut a = look_app();
        a.page = Page::Account;
        a.act(Act::Next);
        assert_eq!(a.page, Page::Look);
        a.event(Event::Key { code: 28, down: true });
        assert_eq!(a.page, Page::Disk);
        a.act(Act::Back);
        assert_eq!(a.page, Page::Look);
        a.act(Act::Back);
        assert_eq!(a.page, Page::Account);
    }

    #[test]
    fn clicking_light_dark_and_auto_changes_the_appearance() {
        let mut a = look_app();
        assert_eq!(a.look, ap::Mode::Dark);
        for m in [ap::Mode::Light, ap::Mode::Auto, ap::Mode::Dark] {
            let (x, y) = centre_of(&a, Act::Look(m));
            click(&mut a, x, y);
            assert_eq!(a.look, m);
            render(&mut a);
        }
    }

    #[test]
    fn the_page_follows_the_picked_scheme() {
        let mut a = look_app();
        let dark = render(&mut a);
        let (x, y) = centre_of(&a, Act::Look(ap::Mode::Light));
        click(&mut a, x, y);
        let light = render(&mut a);
        let probe = a.w * (a.h / 2) + a.w / 2 - a.u(300.0) as usize;
        assert_ne!(dark[probe], light[probe], "the card behind the page changes with the scheme");
    }

    #[test]
    fn clicking_an_accent_dot_changes_the_accent_and_tints_the_button() {
        let mut a = look_app();
        let (x, y) = centre_of(&a, Act::Tint(Accent::Green));
        click(&mut a, x, y);
        assert_eq!(a.accent, Accent::Green);
        let px = render(&mut a);
        assert!(px.contains(&Accent::Green.colours().0), "the primary button takes the accent");
        let (x, y) = centre_of(&a, Act::Tint(Accent::Multicolour));
        click(&mut a, x, y);
        assert_eq!(a.accent, Accent::Multicolour);
    }

    #[test]
    fn clicking_a_wallpaper_tile_changes_the_wallpaper() {
        let mut a = look_app();
        assert_eq!(a.walls.len(), 3, "the default plus two system wallpapers");
        let (x, y) = centre_of(&a, Act::Wall(2));
        click(&mut a, x, y);
        assert_eq!(a.wall, 2);
        render(&mut a);
    }

    #[test]
    fn the_arrow_keys_and_tab_move_the_look_picks() {
        let mut a = look_app();
        let key = |a: &mut App, code: u16| {
            a.event(Event::Key { code, down: true });
            a.event(Event::Key { code, down: false });
        };
        key(&mut a, 106); // right: Dark -> Auto
        assert_eq!(a.look, ap::Mode::Auto);
        key(&mut a, 15); // tab: accent row
        key(&mut a, 106);
        assert_eq!(a.accent, Accent::Purple, "Blue is followed by Purple");
        key(&mut a, 15); // wallpaper row
        key(&mut a, 106);
        assert_eq!(a.wall, 1);
    }

    #[test]
    fn the_look_choice_reaches_the_install_config() {
        let mut a = look_app();
        for act in [Act::Look(ap::Mode::Light), Act::Tint(Accent::Orange), Act::Wall(1)] {
            a.act(act);
        }
        a.disk = Some(0);
        a.start();
        let cfg = a.want_install.take().unwrap();
        assert_eq!((cfg.appearance.as_str(), cfg.accent.as_str(), cfg.wallpaper.as_str()), ("light", "orange", "gullfoss_canyon"));
        assert_eq!(a.page, Page::Progress);
    }

    #[test]
    fn missing_wallpapers_do_not_break_the_page() {
        let mut a = app();
        assert!(load_wallpapers("/nonexistent/assets").is_empty());
        a.set_wallpapers(vec![]);
        a.page = Page::Look;
        render(&mut a);
        assert_eq!(a.walls.len(), 1);
    }

    #[test]
    fn an_unattended_config_goes_straight_to_the_progress_page() {
        let mut a = app();
        a.progress.lock().unwrap().failed = Some("an earlier attempt".into());
        let cfg = Config {
            disk: a.disks[0].clone(), mode: Mode::Erase, locale: "en_US.UTF-8".into(), layout: keymaps::layouts().next().unwrap(),
            full_name: "Tester".into(), username: "tester".into(), password: "secret-pw".into(), hostname: "bifrost".into(),
            timezone: String::new(), mirror: "http://10.99.0.1:8078/mirror/".into(),
            appearance: plan::DEFAULT_APPEARANCE.into(), accent: plan::DEFAULT_ACCENT.into(), wallpaper: String::new(),
        };
        a.begin(cfg);
        assert_eq!(a.page, Page::Progress, "the install shows its progress on screen, never a black display");
        assert!(a.progress.lock().unwrap().failed.is_none(), "a fresh attempt starts from clean progress");
        let cfg = a.want_install.take().unwrap();
        assert_eq!((cfg.username.as_str(), cfg.mirror.as_str()), ("tester", "http://10.99.0.1:8078/mirror/"));
        assert!(render(&mut a).iter().any(|&c| c == WHITE), "the progress page draws");
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
        for p in [Page::Welcome, Page::Language, Page::Account, Page::Look, Page::Disk, Page::Confirm, Page::Progress] {
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
    fn state_text_lists_buttons_and_fields() {
        let t = format_state((1280, 800), "account", "0 Waiting", 5, &[("next".into(), (10.4, 20.0, 200.0, 44.0))], &[("name".into(), (1.0, 2.0, 3.0, 4.0), true)]);
        assert_eq!(t, "size=1280 800\nscreen=account\nstage=0 Waiting\npercent=5\nbutton=next 10 20 200 44\nfield=name 1 2 3 4 1\n");
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

    /// Developer preview: `PREVIEW_DIR=DIR cargo test preview_look -- --ignored` writes look-dark.png and
    /// look-light.png (the Look page with the real wallpapers and branding from assets/ and packaging/).
    #[test]
    #[ignore]
    fn preview_look() {
        let Some(dir) = std::env::var_os("PREVIEW_DIR") else { return };
        let root = env!("CARGO_MANIFEST_DIR");
        let branding = format!("{root}/../packaging/archiso/airootfs/usr/share/bifrost/installer");
        let image = |n: &str| std::fs::read(format!("{branding}/{n}")).ok().and_then(|b| crate::imgdec::decode(&b));
        for (name, mode) in [("dark", ap::Mode::Dark), ("light", ap::Mode::Light)] {
            let d = parse_parted("BYT;\n/dev/vda:17179869184B:virtio:512:512:unknown:Virtio Block Device:;\n").unwrap();
            let (w, h) = (1920usize, 1080usize);
            let mut a = App::new(crate::shell::tests::ui(), w, h, vec![d], image("logo.png"), image("bg.jpg"));
            a.set_wallpapers(load_wallpapers(&format!("{root}/assets")));
            a.page = Page::Look;
            a.look = mode;
            a.wall = 2.min(a.walls.len() - 1);
            let mut img = crate::gfx::Image::new(w, h);
            for i in 0..30 {
                let mut s = Surface { w, h, stride: w, px: &mut img.px, y0: 0 };
                a.draw(&mut s, 1000.0 + i as f64 * 40.0);
            }
            std::fs::write(std::path::Path::new(&dir).join(format!("look-{name}.png")), crate::shell::screenshot::encode(&img)).unwrap();
        }
    }

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
