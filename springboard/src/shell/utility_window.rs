//! Small windows the system menu opens over the desktop: Force Quit Applications (Option+Cmd+Esc), About This Computer,
//! and the Log Out question (Shift+Cmd+Q). Like the preview panel, each is a card the shell draws above the windows and
//! exports as an overlay surface of its own (chrome id 14), so it shows over any app. It takes the pointer inside its
//! card and the keys Esc, Return and the arrows while it is up.
//!
//! The card's content comes from live state: Force Quit lists the apps the Dock shows as running, About reads the facts
//! the Settings About pane shows (`Info`: model, chip, memory, serial) so the two agree. The layout derives from the screen
//! size and `ui.pt()`, colours are the named constants of `render/src/ui.rs`.

use super::desktop_bar::Action;
use super::ui::{
    Ui, Weight, ABOUT_BEZEL, ABOUT_STAND, CARD_BG, FIELD_BG, LABEL, LINK, MAC_BODY_PT, MAC_CALLOUT_PT, MAC_FOOTNOTE_PT, MAC_HEADLINE_PT,
    MAC_TITLE2_PT, SECONDARY, SELECTION, SEPARATOR_C, SYSTEM_RED, TINT, TRAFFIC_INACTIVE_C, WHITE, WINDOW_RADIUS_PT,
};
use super::{apps, AppId, Effect, Shell};
use crate::gfx::{Image, Surface};
use crate::layout::Rect;

/// The chrome surface id the open card is exported as.
pub const CHROME_ID: u32 = super::chrome_ids::UTILITY;

/// Which card is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// System menu > About This Computer
    About,
    /// System menu > Force Quit… and Option+Cmd+Esc
    ForceQuit,
    /// System menu > Log Out and Shift+Cmd+Q: asks first
    LogOut,
    /// an app's About panel (the application menu's About <App>): its icon, name, version and summary (`AppAbout`)
    AboutApp,
}

/// What an app's About panel shows (the standard About panel of the application menu).
#[derive(Clone, Default)]
pub struct AppAbout {
    pub name: String,
    /// the version string; the panel shows "Version <version>", nothing when empty
    pub version: String,
    /// the app's one-line description, wrapped under the version; nothing when empty
    pub summary: String,
    pub icon: Option<Image>,
}

/// The second line of an About panel: "Version 1.2", or nothing for an app with no version.
pub fn about_version_line(version: &str) -> Option<String> {
    let v = version.trim();
    (!v.is_empty()).then(|| format!("Version {v}"))
}

/// What a Force Quit row ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// a built-in or installed app (an installed app's process is killed; a built-in app lives in the shell, so its
    /// windows are closed)
    App(AppId),
    /// a Linux app: its windows, asked to close through the window server (the shell has no process to kill)
    Foreign(Vec<u32>),
}

/// One running app in the Force Quit list.
#[derive(Clone)]
pub struct Entry {
    pub name: String,
    pub target: Target,
    pub icon: Option<Image>,
}

/// The computer's facts About shows, the same strings the Settings About pane shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// the computer's name on the network (the hostname)
    pub name: String,
    /// its model, the large line
    pub model: String,
    /// the processor (`device_name::chip_name`)
    pub chip: String,
    pub memory: String,
    pub serial: String,
    pub version: String,
}

/// A part of the card the pointer can press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Close,
    Primary,
    Secondary,
    Row(usize),
    /// the title bar: a press there drags the card
    Title,
}

/// What a pointer event or key asks the shell to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Nothing,
    Redraw,
    Close,
    ForceQuit(Target),
    LogOut,
    MoreInfo,
}

pub struct UtilityWindow {
    pub kind: Kind,
    pub entries: Vec<Entry>,
    pub selected: Option<usize>,
    pub facts: Facts,
    /// the signed-in account's full name, for the Log Out question
    pub user: String,
    /// the app an About panel (`Kind::AboutApp`) is for
    pub app: Option<AppAbout>,
    /// how far the card was dragged from the centre, in px
    offset: (f32, f32),
    /// the last pointer position of a title-bar drag
    drag: Option<(f32, f32)>,
    /// the part a press began on; it acts on release
    pressed: Option<Part>,
}

/// Where each part of a card is, in px.
struct Layout {
    card: Rect,
    close: Rect,
    title: Rect,
    list: Rect,
    row_h: i32,
    primary: Rect,
    secondary: Rect,
    /// the first line of the message or hint and the rest of the text sits from here
    text_x: i32,
    text_y: i32,
    text_w: f32,
}

fn pad(v: f32) -> i32 {
    v.round() as i32
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x: x.round() as i32, y: y.round() as i32, w: w.round() as i32, h: h.round() as i32 }
}

/// The two-column facts of About, as (label, value), with the empty ones left out.
pub fn about_rows(f: &Facts) -> Vec<(&'static str, String)> {
    [("Name", &f.name), ("Chip", &f.chip), ("Memory", &f.memory), ("Serial number", &f.serial), ("BifrOSt", &f.version)]
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(l, v)| (l, v.clone()))
        .collect()
}

/// The large line of About: the model, else the computer's name.
pub fn about_title(f: &Facts) -> &str {
    if f.model.is_empty() { &f.name } else { &f.model }
}

impl UtilityWindow {
    pub fn new(kind: Kind) -> Self {
        UtilityWindow { kind, entries: Vec::new(), selected: None, facts: Facts::default(), user: String::new(), app: None, offset: (0., 0.), drag: None, pressed: None }
    }

    /// Keeps the selection on a row (or none when the list is empty).
    pub fn set_entries(&mut self, entries: Vec<Entry>) {
        self.entries = entries;
        self.selected = match self.selected {
            Some(i) if i < self.entries.len() => Some(i),
            _ if self.entries.is_empty() => None,
            _ => Some(0),
        };
    }

    /// The selection moved by `delta` rows, held at the ends.
    pub fn step(&mut self, delta: i32) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        let at = self.selected.map_or(if delta > 0 { -1 } else { self.entries.len() as i32 }, |i| i as i32);
        let next = (at + delta).clamp(0, self.entries.len() as i32 - 1) as usize;
        let changed = self.selected != Some(next);
        self.selected = Some(next);
        changed
    }

    /// The text of the Log Out question.
    pub fn log_out_title(&self) -> String {
        if self.user.is_empty() { "Are you sure you want to log out?".to_string() } else { format!("Are you sure you want to log out {}?", self.user) }
    }

    const LOG_OUT_MESSAGE: &'static str = "All open apps will be closed. Unsaved changes in them will be lost.";
    const FORCE_QUIT_MESSAGE: &'static str = "If an app does not respond for a while, select it and click Force Quit.";
    const FORCE_QUIT_HINT: &'static str = "You can open this window by pressing Option-Command-Escape.";

    fn layout(&self, ui: &Ui, sw: i32, sh: i32) -> Layout {
        let k = ui.pt(1.0);
        let (cw, ch) = match self.kind {
            Kind::ForceQuit => (372., 316.),
            Kind::About => {
                let text_w = 164. * k;
                let lines: usize = about_rows(&self.facts).iter().map(|(_, v)| ui.wrap(v, MAC_CALLOUT_PT, Weight::Regular, text_w).len().max(1)).sum();
                (300., 236. + lines as f32 * 17. + about_rows(&self.facts).len() as f32 * 4.)
            }
            Kind::LogOut => {
                let text_w = (320. - 48.) * k;
                let t = ui.wrap(&self.log_out_title(), MAC_HEADLINE_PT, Weight::Bold, text_w).len().max(1);
                let m = ui.wrap(Self::LOG_OUT_MESSAGE, MAC_CALLOUT_PT, Weight::Regular, text_w).len().max(1);
                (320., 24. + t as f32 * 17. + 8. + m as f32 * 15. + 20. + 28. + 20.)
            }
            Kind::AboutApp => {
                let summary = self.app.as_ref().map_or(0, |a| if a.summary.is_empty() { 0 } else { ui.wrap(&a.summary, MAC_FOOTNOTE_PT, Weight::Regular, (280. - 48.) * k).len() });
                (280., 160. + summary as f32 * 14. + 20.)
            }
        };
        let (cwp, chp) = (cw * k, ch * k);
        // centred, a little above the middle, then moved by the drag and held on the screen below the menu bar
        let bar = ui.pt(28.);
        let x = ((sw as f32 - cwp) / 2. + self.offset.0).clamp(0., (sw as f32 - cwp).max(0.));
        let y = ((sh as f32 - chp) / 2. - ui.pt(24.) + self.offset.1).clamp(bar, (sh as f32 - chp).max(bar));
        let card = rect(x, y, cwp, chp);
        let (cx, cy) = (card.x as f32, card.y as f32);
        let light = 12. * k;
        let close = rect(cx + 13. * k, cy + 14. * k, light, light);
        let title = rect(cx, cy, cwp, 40. * k);
        let btn_h = 28. * k;
        let (mut list, mut primary, mut secondary) = (rect(0., 0., 0., 0.), rect(0., 0., 0., 0.), rect(0., 0., 0., 0.));
        let (text_x, text_y, text_w);
        match self.kind {
            Kind::ForceQuit => {
                list = rect(cx + 16. * k, cy + 98. * k, cwp - 32. * k, 150. * k);
                primary = rect(cx + cwp - 16. * k - 96. * k, cy + chp - 16. * k - btn_h, 96. * k, btn_h);
                text_x = card.x + pad(16. * k);
                text_y = card.y + pad(52. * k);
                text_w = cwp - 32. * k;
            }
            Kind::About => {
                let n = about_rows(&self.facts).len();
                let lines: usize = about_rows(&self.facts).iter().map(|(_, v)| ui.wrap(v, MAC_CALLOUT_PT, Weight::Regular, 164. * k).len().max(1)).sum();
                let rows_h = (lines as f32 * 17. + n as f32 * 4.) * k;
                primary = rect(cx + (cwp - 104. * k) / 2., cy + 176. * k + rows_h + 14. * k, 104. * k, 24. * k);
                text_x = card.x + pad(16. * k);
                text_y = card.y + pad(176. * k);
                text_w = 164. * k;
            }
            Kind::AboutApp => {
                text_x = card.x + pad(24. * k);
                text_y = card.y + pad(160. * k);
                text_w = cwp - 48. * k;
            }
            Kind::LogOut => {
                primary = rect(cx + cwp - 24. * k - 104. * k, cy + chp - 20. * k - btn_h, 104. * k, btn_h);
                secondary = rect(primary.x as f32 - 10. * k - 88. * k, primary.y as f32, 88. * k, btn_h);
                text_x = card.x + pad(24. * k);
                text_y = card.y + pad(24. * k);
                text_w = cwp - 48. * k;
            }
        }
        Layout { card, close, title, list, row_h: pad(24. * k), primary, secondary, text_x, text_y, text_w }
    }

    /// The card's rectangle on a `sw` by `sh` screen, in px.
    pub fn card(&self, ui: &Ui, sw: i32, sh: i32) -> Rect {
        self.layout(ui, sw, sh).card
    }

    fn part_at(&self, ui: &Ui, sw: i32, sh: i32, x: f32, y: f32) -> Option<Part> {
        let l = self.layout(ui, sw, sh);
        if !l.card.contains(x, y) {
            return None;
        }
        // the close light is small: its hit area is a little larger than the disc
        if self.kind != Kind::LogOut && inflate(l.close, ui.pt(3.) as i32).contains(x, y) {
            return Some(Part::Close);
        }
        match self.kind {
            Kind::ForceQuit => {
                if l.primary.contains(x, y) {
                    return Some(Part::Primary);
                }
                if l.list.contains(x, y) {
                    let i = ((y - l.list.y as f32) / l.row_h.max(1) as f32) as usize;
                    return (i < self.entries.len()).then_some(Part::Row(i));
                }
            }
            Kind::About => {
                if l.primary.contains(x, y) {
                    return Some(Part::Primary);
                }
            }
            Kind::AboutApp => {}
            Kind::LogOut => {
                if l.primary.contains(x, y) {
                    return Some(Part::Primary);
                }
                if l.secondary.contains(x, y) {
                    return Some(Part::Secondary);
                }
            }
        }
        (self.kind != Kind::LogOut && l.title.contains(x, y)).then_some(Part::Title)
    }

    /// A press at (`x`, `y`): remembers what it began on, selects a row at once, starts a title-bar drag.
    pub fn down(&mut self, ui: &Ui, sw: i32, sh: i32, x: f32, y: f32) -> bool {
        let Some(part) = self.part_at(ui, sw, sh, x, y) else { return false };
        match part {
            Part::Row(i) => self.selected = Some(i),
            Part::Title => self.drag = Some((x, y)),
            _ => {}
        }
        self.pressed = Some(part);
        true
    }

    /// The pointer moved with a title-bar drag on. False when no drag is on.
    pub fn drag_to(&mut self, x: f32, y: f32) -> bool {
        let Some((px, py)) = self.drag else { return false };
        self.offset = (self.offset.0 + x - px, self.offset.1 + y - py);
        self.drag = Some((x, y));
        true
    }

    /// The release: a button or the close light acts when the press began on the same part.
    pub fn up(&mut self, ui: &Ui, sw: i32, sh: i32, x: f32, y: f32) -> (bool, Outcome) {
        let began = self.pressed.take();
        let was_drag = self.drag.take().is_some();
        let at = self.part_at(ui, sw, sh, x, y);
        // a release outside the card still ends a press that began on it, so the desktop gets no Up without its Down
        if at.is_none() && !was_drag && began.is_none() {
            return (false, Outcome::Nothing);
        }
        let outcome = match (began, at) {
            (Some(Part::Close), Some(Part::Close)) => Outcome::Close,
            (Some(Part::Primary), Some(Part::Primary)) => self.activate(),
            (Some(Part::Secondary), Some(Part::Secondary)) => Outcome::Close,
            (Some(Part::Row(_)), _) | (Some(Part::Title), _) => Outcome::Redraw,
            _ => Outcome::Nothing,
        };
        (true, outcome)
    }

    /// The main button (or Return): Force Quit ends the selected app, Log Out logs out, About opens the About pane.
    pub fn activate(&self) -> Outcome {
        match self.kind {
            Kind::ForceQuit => self.selected.and_then(|i| self.entries.get(i)).map_or(Outcome::Nothing, |e| Outcome::ForceQuit(e.target.clone())),
            Kind::LogOut => Outcome::LogOut,
            Kind::About => Outcome::MoreInfo,
            // the About panel has no button: Return does nothing, Esc and the close light close it
            Kind::AboutApp => Outcome::Nothing,
        }
    }

    /// A key while the card is up: Esc closes, Return acts, Up and Down move the Force Quit selection.
    pub fn key(&mut self, code: u16) -> Outcome {
        match code {
            1 => Outcome::Close,
            28 | 96 => self.activate(),
            103 if self.kind == Kind::ForceQuit => if self.step(-1) { Outcome::Redraw } else { Outcome::Nothing },
            108 if self.kind == Kind::ForceQuit => if self.step(1) { Outcome::Redraw } else { Outcome::Nothing },
            _ => Outcome::Nothing,
        }
    }
}

fn inflate(r: Rect, by: i32) -> Rect {
    Rect { x: r.x - by, y: r.y - by, w: r.w + 2 * by, h: r.h + 2 * by }
}

/// The signed-in account's full name, read once.
pub fn user_name() -> &'static str {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| super::account::load().full_name)
}

impl Shell {
    /// Whether a utility card is up (it is its own overlay surface, so the desktop frame is not drawn in parts).
    pub(super) fn utility_up(&self) -> bool {
        self.utility.is_some()
    }

    /// Opens the card, replacing one that is up. Force Quit lists the apps running now; About reads the facts the Settings
    /// About pane shows.
    pub(super) fn utility_open(&mut self, kind: Kind) {
        let mut w = UtilityWindow::new(kind);
        match kind {
            Kind::ForceQuit => w.set_entries(self.force_quit_entries()),
            Kind::About => w.facts = self.about_facts(),
            Kind::LogOut => w.user = user_name().to_string(),
            Kind::AboutApp => {}
        }
        self.utility = Some(w);
        self.utility_changed();
    }

    /// Opens the About panel of an app (the application menu's About <App>), replacing a card that is up.
    pub(super) fn utility_open_about_app(&mut self, about: AppAbout) {
        let mut w = UtilityWindow::new(Kind::AboutApp);
        w.app = Some(about);
        self.utility = Some(w);
        self.utility_changed();
    }

    /// A repaint of the whole frame: the card came, moved or went.
    fn utility_changed(&mut self) {
        self.utility_dirty = true;
        self.dirty = true;
    }

    /// The facts of About: what the Settings About pane shows (`Info`), the version of this build.
    fn about_facts(&self) -> Facts {
        let i = &self.info;
        // the pane's processor line carries the core count after the name; About shows the chip and the cores together
        Facts { name: i.name.clone(), model: i.model.clone(), chip: i.cpu.clone(), memory: i.memory.clone(), serial: i.serial.clone(), version: super::pages::BUILD.version.to_string() }
    }

    /// The apps the Dock shows as running, one row each: built-in and installed apps, then Linux apps with a window.
    pub(super) fn force_quit_entries(&self) -> Vec<Entry> {
        let Some(d) = self.desk.as_ref() else { return Vec::new() };
        let mut v: Vec<Entry> = apps()
            .iter()
            .enumerate()
            .filter(|(i, _)| self.dock_entry_running(*i))
            .map(|(i, a)| Entry { name: a.name.to_string(), target: Target::App(a.id), icon: self.base_icons.get(i).cloned() })
            .collect();
        v.extend(d.linux_apps.iter().filter(|a| !a.windows.is_empty()).map(|a| Entry {
            name: a.app_id.clone(),
            target: Target::Foreign(a.windows.iter().map(|w| w.id).collect()),
            icon: Some(a.icon().clone()),
        }));
        v
    }

    /// Ends one Force Quit row. An installed app's process is killed at once through its host, or, when the window server
    /// runs it, through the app manager (SIGKILL to its whole cgroup); a built-in app (it runs in the shell) has its windows
    /// closed; a Linux app that is not installed has its windows asked to close (its process is not known to the shell).
    fn utility_force_quit(&mut self, target: Target) -> Vec<Effect> {
        match target {
            Target::App(AppId::External(slot)) => {
                // the process first: `desk_close_app` would start a polite quit of a live app and take its host
                let has_host = self.ext.get(slot as usize).and_then(Option::as_ref).is_some();
                if has_host {
                    self.ext_force_quit(slot as usize);
                }
                self.desk_close_app(AppId::External(slot));
                if has_host {
                    return Vec::new();
                }
                // an app with no host of ours runs under the window server: its whole process tree is killed through the
                // app manager (the cgroup of its id), which a hung app cannot ignore, and its windows are also asked to
                // close so the desk drops them at once
                super::apphost::installed()
                    .get(slot as usize)
                    .map(|p| {
                        #[cfg(feature = "desktop-wayland")]
                        crate::bare::app_manager::terminate(&p.manifest.id);
                        self.desk
                            .as_ref()
                            .map(|d| d.native_windows.iter().filter(|w| w.app_id == p.manifest.wayland_app_id).map(|w| Effect::ForeignWindowAction(w.id, false)).collect())
                            .unwrap_or_default()
                    })
                    .unwrap_or_default()
            }
            Target::App(app) => {
                self.desk_close_app(app);
                Vec::new()
            }
            Target::Foreign(ids) => ids.into_iter().map(|id| Effect::ForeignWindowAction(id, false)).collect(),
        }
    }

    /// Log Out: every window and app is closed (installed apps are asked to quit and saved, then killed after the grace period),
    /// the windows are remembered for the next login, and the screen locks to the login window. This is a single-account
    /// computer, so logging out returns to the login window of the same shell; the system's own session stays up.
    pub(super) fn log_out(&mut self, _t: f64) -> Vec<Effect> {
        let mut effects = Vec::new();
        // the windows are written down first, so logging back in can reopen them
        self.session_save_for_exit();
        self.session.restored = false;
        if let Some(d) = self.desk.as_ref() {
            let mut seen = std::collections::HashSet::new();
            let ids: Vec<u32> = d.native_windows.iter().chain(d.linux_apps.iter().flat_map(|a| a.windows.iter())).map(|w| w.id).filter(|id| seen.insert(*id)).collect();
            effects.extend(ids.into_iter().map(|id| Effect::ForeignWindowAction(id, false)));
        }
        let keyboard = super::apphost::installed().iter().position(|a| a.manifest.id == super::apphost::KEYBOARD_APP_ID);
        let slots: Vec<u8> = self.ext.iter().enumerate().filter(|(i, h)| h.is_some() && Some(*i) != keyboard).map(|(i, _)| i as u8).collect();
        for slot in slots {
            self.desk_quit_app(slot);
        }
        let open: Vec<AppId> = self.desk.as_ref().map(|d| d.wins.iter().map(|w| w.app).collect()).unwrap_or_default();
        for app in open {
            self.desk_close_app(app);
        }
        self.utility = None;
        self.utility_changed();
        effects.extend(self.lock_screen());
        effects
    }

    /// Carries out what a pointer event or key asked of the card.
    fn utility_outcome(&mut self, outcome: Outcome, t: f64) -> Vec<Effect> {
        match outcome {
            Outcome::Nothing => Vec::new(),
            Outcome::Redraw => {
                self.utility_changed();
                Vec::new()
            }
            Outcome::Close => {
                self.utility = None;
                self.utility_changed();
                Vec::new()
            }
            Outcome::ForceQuit(target) => {
                let effects = self.utility_force_quit(target);
                // the list follows what runs now
                let entries = self.force_quit_entries();
                if let Some(w) = self.utility.as_mut() {
                    w.set_entries(entries);
                }
                self.utility_changed();
                effects
            }
            Outcome::LogOut => self.log_out(t),
            Outcome::MoreInfo => {
                self.utility = None;
                self.utility_changed();
                self.run_menu_action(Action::Settings("About"), t)
            }
        }
    }

    /// Pointer events for the card: a press or release inside it is used up. A title-bar drag follows the pointer anywhere.
    pub(super) fn utility_pointer(&mut self, ev: super::Event, t: f64) -> Option<Vec<Effect>> {
        let (sw, sh) = (self.w as i32, self.h as i32);
        let w = self.utility.as_mut()?;
        match ev {
            super::Event::Down(x, y) => w.down(&self.ui, sw, sh, x, y).then(Vec::new),
            super::Event::Move(x, y) => w.drag_to(x, y).then(|| {
                self.utility_changed();
                Vec::new()
            }),
            super::Event::Up(x, y) => {
                let (took, outcome) = w.up(&self.ui, sw, sh, x, y);
                if !took {
                    return None;
                }
                Some(self.utility_outcome(outcome, t))
            }
            _ => None,
        }
    }

    /// Keys while the card is up: Esc, Return and (for Force Quit) the arrows. A release of one of them is used up too.
    /// None for any other key, which falls through to the desktop.
    pub(super) fn utility_key(&mut self, code: u16, down: bool, t: f64) -> Option<Vec<Effect>> {
        // never over the login window: Return there submits the password, it does not run the card's action
        if self.is_locked() {
            return None;
        }
        let w = self.utility.as_mut()?;
        let takes = matches!(code, 1 | 28 | 96) || (w.kind == Kind::ForceQuit && matches!(code, 103 | 108));
        if !takes {
            return None;
        }
        if !down {
            return Some(Vec::new());
        }
        let outcome = w.key(code);
        Some(self.utility_outcome(outcome, t))
    }

    /// The card's rectangle on the screen in px, clipped to the screen: its overlay surface and a region the GPU presenter keeps.
    pub(super) fn utility_rect(&self) -> Option<(usize, usize, usize, usize)> {
        let w = self.utility.as_ref()?;
        let c = w.card(&self.ui, self.w as i32, self.h as i32);
        let (x, y) = (c.x.max(0) as usize, c.y.max(0) as usize);
        let cw = (c.w.max(0) as usize).min((self.w as usize).saturating_sub(x));
        let ch = (c.h.max(0) as usize).min((self.h as usize).saturating_sub(y));
        (cw > 0 && ch > 0).then_some((x, y, cw, ch))
    }

    /// The card as an overlay surface: its rectangle and the alpha mask of its rounded corners.
    pub(super) fn utility_overlay(&self) -> Option<((usize, usize, usize, usize), Vec<u8>)> {
        let (x, y, w, h) = self.utility_rect()?;
        Some(((x, y, w, h), crate::gfx::rrect_mask(w, h, self.ui.pt(WINDOW_RADIUS_PT))))
    }

    /// A button: a capsule, the main one in the tint, the other grey; a dimmed one when there is nothing to act on.
    fn draw_utility_button(&self, s: &mut Surface, r: Rect, text: &str, main: bool, enabled: bool) {
        let (fill, ink) = if main && enabled { (LINK, WHITE) } else if enabled { (FIELD_BG.now(), LABEL.now()) } else { (FIELD_BG.now(), SECONDARY) };
        s.fill_rrect(r.x as f32, r.y as f32, r.w as f32, r.h as f32, r.h as f32 / 2., fill);
        self.label(text, MAC_BODY_PT, Weight::Medium, ink, |im| s.blit(im, r.x + (r.w - im.w as i32) / 2, r.y + (r.h - im.h as i32) / 2, 255, 255));
    }

    /// Draws the card over the desktop. Does nothing when no card is up.
    pub(super) fn draw_utility(&self, s: &mut Surface) {
        let Some(w) = self.utility.as_ref() else { return };
        let ui = &self.ui;
        let k = ui.pt(1.0);
        let l = w.layout(ui, s.w as i32, s.h as i32);
        let radius = ui.pt(WINDOW_RADIUS_PT);
        let (cx, cy, cw, ch) = (l.card.x as f32, l.card.y as f32, l.card.w as f32, l.card.h as f32);
        s.fill_rrect(cx, cy, cw, ch, radius, CARD_BG.now());
        s.stroke_rrect(cx, cy, cw, ch, radius, ui.pt(1.0), SEPARATOR_C.now());
        // traffic lights: the close light is lit, the other two are grey, as on a window that has no use for them
        if w.kind != Kind::LogOut {
            let r = l.close.w as f32 / 2.;
            let (lx, ly) = (l.close.x as f32, l.close.y as f32);
            s.fill_rrect(lx, ly, r * 2., r * 2., r, SYSTEM_RED);
            for n in 1..=2 {
                s.fill_rrect(lx + n as f32 * 20. * k, ly, r * 2., r * 2., r, TRAFFIC_INACTIVE_C.now());
            }
        }
        match w.kind {
            Kind::ForceQuit => self.draw_force_quit(s, w, &l),
            Kind::About => self.draw_about(s, w, &l),
            Kind::LogOut => self.draw_log_out(s, w, &l),
            Kind::AboutApp => self.draw_about_app(s, w, &l),
        }
    }

    /// The standard About panel: the app's icon, its name in bold, "Version ..." and its summary, centred.
    fn draw_about_app(&self, s: &mut Surface, w: &UtilityWindow, l: &Layout) {
        let Some(app) = w.app.as_ref() else { return };
        let ui = &self.ui;
        let k = ui.pt(1.0);
        let cx = l.card.x + l.card.w / 2;
        let side = pad(64. * k).max(1) as usize;
        if let Some(im) = &app.icon {
            let icon = im.resized(side, side);
            s.blit(&icon, cx - side as i32 / 2, l.card.y + pad(40. * k), 255, 255);
        }
        let name = ui.wrap(&app.name, MAC_HEADLINE_PT, Weight::Bold, l.text_w);
        let name = &name[..name.len().min(1)];
        self.draw_lines(s, name, MAC_HEADLINE_PT, Weight::Bold, LABEL.now(), 0, l.card.y + pad(116. * k), ui.pt(17.), Some(cx));
        if let Some(version) = about_version_line(&app.version) {
            self.draw_lines(s, &[version], MAC_FOOTNOTE_PT, Weight::Regular, SECONDARY, 0, l.card.y + pad(138. * k), ui.pt(14.), Some(cx));
        }
        if !app.summary.is_empty() {
            let lines = ui.wrap(&app.summary, MAC_FOOTNOTE_PT, Weight::Regular, l.text_w);
            self.draw_lines(s, &lines, MAC_FOOTNOTE_PT, Weight::Regular, SECONDARY, 0, l.text_y, ui.pt(14.), Some(cx));
        }
    }

    fn draw_lines(&self, s: &mut Surface, lines: &[String], pt: f32, weight: Weight, ink: u32, x: i32, y: i32, step: f32, centre: Option<i32>) {
        for (n, line) in lines.iter().enumerate() {
            let ly = y + (n as f32 * step) as i32;
            self.label(line, pt, weight, ink, |im| {
                let lx = centre.map_or(x, |c| c - im.w as i32 / 2);
                s.blit(im, lx, ly, 255, 255)
            });
        }
    }

    fn draw_force_quit(&self, s: &mut Surface, w: &UtilityWindow, l: &Layout) {
        let ui = &self.ui;
        let k = ui.pt(1.0);
        let title = "Force Quit Applications";
        self.label(title, MAC_HEADLINE_PT, Weight::Bold, SECONDARY, |im| s.blit(im, l.card.x + (l.card.w - im.w as i32) / 2, l.card.y + (ui.pt(20.) - im.h as f32 / 2.) as i32, 255, 255));
        let msg = ui.wrap(UtilityWindow::FORCE_QUIT_MESSAGE, MAC_BODY_PT, Weight::Regular, l.text_w);
        self.draw_lines(s, &msg, MAC_BODY_PT, Weight::Regular, LABEL.now(), l.text_x, l.text_y, ui.pt(17.), None);
        // the list: a plate with a row per running app, the selected one in the selection colour
        let li = l.list;
        s.fill_rrect(li.x as f32, li.y as f32, li.w as f32, li.h as f32, ui.pt(6.), super::ui::DETAIL_BG.now());
        s.stroke_rrect(li.x as f32, li.y as f32, li.w as f32, li.h as f32, ui.pt(6.), ui.pt(1.0), SEPARATOR_C.now());
        let visible = (li.h / l.row_h.max(1)).max(1) as usize;
        for (i, e) in w.entries.iter().enumerate().take(visible) {
            let ry = li.y + i as i32 * l.row_h;
            let on = w.selected == Some(i);
            if on {
                s.fill_rrect((li.x + pad(3. * k)) as f32, (ry + pad(1. * k)) as f32, (li.w - pad(6. * k)) as f32, (l.row_h - pad(2. * k)) as f32, ui.pt(5.), SELECTION.now());
            }
            let icon = pad(18. * k).max(1) as usize;
            if let Some(im) = &e.icon {
                let small = im.resized(icon, icon);
                s.blit(&small, li.x + pad(8. * k), ry + (l.row_h - icon as i32) / 2, 255, 255);
            }
            let ink = if on { WHITE } else { LABEL.now() };
            let name = ui.fit(&e.name, MAC_BODY_PT, (li.w - pad(44. * k)) as f32);
            self.label(&name, MAC_BODY_PT, Weight::Regular, ink, |im| s.blit(im, li.x + pad(34. * k), ry + (l.row_h - im.h as i32) / 2, 255, 255));
        }
        if w.entries.is_empty() {
            self.label("No apps are running", MAC_BODY_PT, Weight::Regular, SECONDARY, |im| s.blit(im, li.x + (li.w - im.w as i32) / 2, li.y + pad(12. * k), 255, 255));
        }
        let hint = ui.wrap(UtilityWindow::FORCE_QUIT_HINT, MAC_FOOTNOTE_PT, Weight::Regular, l.card.w as f32 - 32. * k - l.primary.w as f32 - 12. * k);
        self.draw_lines(s, &hint, MAC_FOOTNOTE_PT, Weight::Regular, SECONDARY, l.card.x + pad(16. * k), l.primary.y + (l.primary.h - (hint.len() as f32 * ui.pt(13.)) as i32) / 2, ui.pt(13.), None);
        self.draw_utility_button(s, l.primary, "Force Quit", true, w.selected.is_some());
    }

    fn draw_about(&self, s: &mut Surface, w: &UtilityWindow, l: &Layout) {
        let ui = &self.ui;
        let k = ui.pt(1.0);
        let cx = l.card.x + l.card.w / 2;
        // a computer: a display on its stand
        let top = (l.card.y as f32) + 50. * k;
        let fx = cx as f32;
        s.fill_rrect(fx - 44. * k, top, 88. * k, 58. * k, 5. * k, ABOUT_BEZEL);
        s.fill_rrect(fx - 41. * k, top + 3. * k, 82. * k, 48. * k, 2. * k, TINT);
        s.fill_rrect(fx - 4. * k, top + 58. * k, 8. * k, 8. * k, 0., ABOUT_STAND);
        s.fill_rrect(fx - 18. * k, top + 66. * k, 36. * k, 4. * k, 2. * k, ABOUT_STAND);
        // the model, large and bold, wrapped when the card is narrower than it
        let title = about_title(&w.facts);
        let lines = ui.wrap(title, MAC_TITLE2_PT, Weight::Bold, l.card.w as f32 - 32. * k);
        self.draw_lines(s, &lines, MAC_TITLE2_PT, Weight::Bold, LABEL.now(), 0, l.card.y + pad(128. * k), ui.pt(22.), Some(cx));
        // label and value columns: labels right-aligned at the column edge, values left-aligned and wrapped
        let col = l.card.x + pad(124. * k);
        let mut y = l.text_y as f32;
        for (label, value) in about_rows(&w.facts) {
            let vl = ui.wrap(&value, MAC_CALLOUT_PT, Weight::Regular, l.text_w);
            self.label(label, MAC_CALLOUT_PT, Weight::Medium, LABEL.now(), |im| s.blit(im, col - pad(8. * k) - im.w as i32, y as i32, 255, 255));
            self.draw_lines(s, &vl, MAC_CALLOUT_PT, Weight::Regular, SECONDARY, col, y as i32, ui.pt(17.), None);
            y += vl.len().max(1) as f32 * ui.pt(17.) + ui.pt(4.);
        }
        self.draw_utility_button(s, l.primary, "More Info…", false, true);
    }

    fn draw_log_out(&self, s: &mut Surface, w: &UtilityWindow, l: &Layout) {
        let ui = &self.ui;
        let k = ui.pt(1.0);
        let title = ui.wrap(&w.log_out_title(), MAC_HEADLINE_PT, Weight::Bold, l.text_w);
        self.draw_lines(s, &title, MAC_HEADLINE_PT, Weight::Bold, LABEL.now(), l.text_x, l.text_y, ui.pt(17.), None);
        let msg = ui.wrap(UtilityWindow::LOG_OUT_MESSAGE, MAC_CALLOUT_PT, Weight::Regular, l.text_w);
        self.draw_lines(s, &msg, MAC_CALLOUT_PT, Weight::Regular, SECONDARY, l.text_x, l.text_y + (title.len() as f32 * 17. * k + 8. * k) as i32, ui.pt(15.), None);
        self.draw_utility_button(s, l.secondary, "Cancel", false, true);
        self.draw_utility_button(s, l.primary, "Log Out", true, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, id: u32) -> Entry {
        Entry { name: name.into(), target: Target::Foreign(vec![id]), icon: None }
    }

    fn force_quit(n: usize) -> UtilityWindow {
        let mut w = UtilityWindow::new(Kind::ForceQuit);
        w.set_entries((0..n).map(|i| entry(&format!("App {i}"), i as u32)).collect());
        w
    }

    #[test]
    fn the_arrows_move_the_selection_and_hold_at_the_ends() {
        let mut w = force_quit(3);
        assert_eq!(w.selected, Some(0), "the first app is selected at once");
        assert_eq!(w.key(108), Outcome::Redraw);
        assert_eq!(w.selected, Some(1));
        w.key(108);
        assert_eq!(w.key(108), Outcome::Nothing, "held at the last row");
        assert_eq!(w.selected, Some(2));
        w.key(103);
        w.key(103);
        assert_eq!(w.key(103), Outcome::Nothing, "held at the first row");
        assert_eq!(w.selected, Some(0));
    }

    #[test]
    fn return_force_quits_the_selected_app_and_esc_closes() {
        let mut w = force_quit(2);
        w.key(108);
        assert_eq!(w.key(28), Outcome::ForceQuit(Target::Foreign(vec![1])));
        assert_eq!(w.key(1), Outcome::Close);
        let mut empty = force_quit(0);
        assert_eq!(empty.key(28), Outcome::Nothing, "nothing to quit");
        assert_eq!(empty.selected, None);
    }

    #[test]
    fn a_refreshed_list_keeps_a_valid_selection() {
        let mut w = force_quit(3);
        w.step(2);
        w.set_entries(vec![entry("Only", 9)]);
        assert_eq!(w.selected, Some(0), "the selection falls back into the shorter list");
        w.set_entries(Vec::new());
        assert_eq!(w.selected, None);
    }

    #[test]
    fn about_shows_the_model_large_and_leaves_out_what_the_machine_does_not_say() {
        let f = Facts { name: "midget".into(), model: "ThinkCentre M900".into(), chip: "Intel Core i7-6700T, 4 cores".into(), memory: "15.5 GB".into(), serial: String::new(), version: "0.10.14".into() };
        assert_eq!(about_title(&f), "ThinkCentre M900");
        let rows = about_rows(&f);
        assert_eq!(rows.iter().map(|r| r.0).collect::<Vec<_>>(), vec!["Name", "Chip", "Memory", "BifrOSt"], "no empty serial row");
        let nameless = Facts { name: "midget".into(), ..Facts::default() };
        assert_eq!(about_title(&nameless), "midget");
    }

    #[test]
    fn the_log_out_question_names_the_account() {
        let mut w = UtilityWindow::new(Kind::LogOut);
        assert_eq!(w.log_out_title(), "Are you sure you want to log out?");
        w.user = "Ada Lovelace".into();
        assert_eq!(w.log_out_title(), "Are you sure you want to log out Ada Lovelace?");
        assert_eq!(w.key(28), Outcome::LogOut);
        assert_eq!(w.key(1), Outcome::Close, "Esc cancels");
    }

    #[test]
    fn the_about_panel_shows_the_version_and_has_no_button() {
        assert_eq!(about_version_line(" 1.4.2 ").as_deref(), Some("Version 1.4.2"));
        assert_eq!(about_version_line(""), None, "an app with no version shows no version line");
        let sh = crate::shell::tests::shell();
        let mut w = UtilityWindow::new(Kind::AboutApp);
        w.app = Some(AppAbout { name: "Notes".into(), version: "2.0".into(), summary: "Write notes. ".repeat(12), icon: None });
        let l = w.layout(&sh.ui, 1280, 800);
        assert!(l.card.x >= 0 && l.card.y >= 0 && l.card.x + l.card.w <= 1280 && l.card.y + l.card.h <= 800);
        let short = { let mut v = UtilityWindow::new(Kind::AboutApp); v.app = Some(AppAbout { name: "Notes".into(), ..AppAbout::default() }); v.layout(&sh.ui, 1280, 800).card.h };
        assert!(l.card.h > short, "the summary makes the panel taller");
        assert_eq!(w.activate(), Outcome::Nothing, "Return does nothing");
        assert_eq!(w.key(1), Outcome::Close, "Esc closes");
    }

    #[test]
    fn the_card_is_on_the_screen_and_its_buttons_are_inside_it() {
        let sh = crate::shell::tests::shell();
        let ui = &sh.ui;
        for kind in [Kind::ForceQuit, Kind::About, Kind::LogOut] {
            let mut w = UtilityWindow::new(kind);
            w.facts = Facts { name: "midget".into(), model: "ThinkCentre M900".into(), chip: "Intel Core i7-6700T, 4 cores".into(), memory: "15.5 GB".into(), serial: "S1".into(), version: "0.10.14".into() };
            let l = w.layout(ui, 1280, 800);
            assert!(l.card.x >= 0 && l.card.y >= 0 && l.card.x + l.card.w <= 1280 && l.card.y + l.card.h <= 800, "{kind:?} fits the screen");
            for b in [l.primary, l.secondary].into_iter().filter(|b| b.w > 0) {
                assert!(b.x >= l.card.x && b.x + b.w <= l.card.x + l.card.w && b.y + b.h <= l.card.y + l.card.h, "{kind:?} button inside the card: {b:?}");
            }
        }
    }

    #[test]
    fn a_press_and_release_on_the_force_quit_button_quits_and_a_drag_moves_the_card() {
        let sh = crate::shell::tests::shell();
        let ui = &sh.ui;
        let mut w = force_quit(2);
        let l = w.layout(ui, 1280, 800);
        let (bx, by) = ((l.primary.x + l.primary.w / 2) as f32, (l.primary.y + l.primary.h / 2) as f32);
        assert!(w.down(ui, 1280, 800, bx, by));
        let (took, out) = w.up(ui, 1280, 800, bx, by);
        assert!(took);
        assert_eq!(out, Outcome::ForceQuit(Target::Foreign(vec![0])));
        // a row is selected by a press
        let ry = (l.list.y + l.row_h + l.row_h / 2) as f32;
        assert!(w.down(ui, 1280, 800, (l.list.x + 20) as f32, ry));
        assert_eq!(w.selected, Some(1));
        w.up(ui, 1280, 800, (l.list.x + 20) as f32, ry);
        // the title bar drags the card
        let before = w.card(ui, 1280, 800);
        let (tx, ty) = ((l.title.x + l.title.w / 2) as f32, (l.title.y + 20) as f32);
        assert!(w.down(ui, 1280, 800, tx, ty));
        assert!(w.drag_to(tx + 50., ty + 30.));
        let after = w.card(ui, 1280, 800);
        assert_eq!((after.x - before.x, after.y - before.y), (50, 30));
        // a press off the card is not taken
        assert!(!w.down(ui, 1280, 800, 2., 2.));
    }

    #[test]
    fn the_close_light_closes_on_release() {
        let sh = crate::shell::tests::shell();
        let ui = &sh.ui;
        let mut w = UtilityWindow::new(Kind::About);
        let l = w.layout(ui, 1280, 800);
        let (x, y) = ((l.close.x + l.close.w / 2) as f32, (l.close.y + l.close.h / 2) as f32);
        assert!(w.down(ui, 1280, 800, x, y));
        assert_eq!(w.up(ui, 1280, 800, x, y).1, Outcome::Close);
    }

    #[test]
    fn a_release_off_the_card_ends_the_press_that_began_on_it() {
        let sh = crate::shell::tests::shell();
        let ui = &sh.ui;
        let mut w = UtilityWindow::new(Kind::About);
        let l = w.layout(ui, 1280, 800);
        let (x, y) = ((l.close.x + l.close.w / 2) as f32, (l.close.y + l.close.h / 2) as f32);
        assert!(w.down(ui, 1280, 800, x, y));
        assert_eq!(w.up(ui, 1280, 800, 2., 2.), (true, Outcome::Nothing), "no Up goes to the desktop without its Down");
        assert_eq!(w.up(ui, 1280, 800, 2., 2.), (false, Outcome::Nothing), "a release that began elsewhere is not the card's");
    }

    #[cfg(feature = "desktop-wayland")]
    mod shell {
        use super::*;
        use crate::shell::desktop_bar::{Action, Menu};
        use crate::shell::{hwkeys, Screen};

        fn desktop() -> Shell {
            let mut sh = crate::shell::tests::desktop_shell();
            sh.screen = Screen::Home;
            sh.anim = None;
            sh
        }

        #[test]
        fn the_system_menu_has_force_quit_and_log_out_with_their_keys_and_recent_items_as_a_submenu() {
            let sh = desktop();
            let rows = sh.menu_row_texts(Menu::System);
            let texts: Vec<&str> = rows.iter().map(String::as_str).filter(|t| !t.is_empty()).collect();
            assert_eq!(texts.len(), 10, "{texts:?}");
            assert_eq!(texts[0], "About This Computer");
            assert_eq!(&texts[1..3], ["System Settings…", "App Store…"]);
            assert_eq!(texts[3], "Recent Items");
            assert_eq!(texts[4], "Force Quit…");
            assert_eq!(&texts[5..8], ["Sleep", "Restart…", "Shut Down…"]);
            assert_eq!(texts[8], "Lock Screen");
            assert!(texts[9].starts_with("Log Out ") && texts[9].ends_with('…'), "{texts:?}");
            let keys = sh.menu_row_keys(Menu::System);
            assert!(keys.contains(&"\u{2325}\u{2318}\u{238B}".to_string()), "Force Quit shows option-command-escape: {keys:?}");
            assert!(keys.contains(&"\u{21E7}\u{2318}Q".to_string()), "Log Out shows shift-command-Q: {keys:?}");
            assert!(!rows.iter().any(|t| t == "Clear Menu"), "Clear Menu lives in the submenu");
        }

        #[test]
        fn recent_items_opens_a_submenu_that_lists_launches_and_clears() {
            let mut sh = desktop();
            sh.desk_open(AppId::Terminal, 0.);
            sh.desk_open(AppId::Settings, 0.);
            sh.desktop_menu = Some(Menu::System);
            let _ = sh.run_menu_action(Action::Submenu(Menu::Recent), 0.);
            assert_eq!(sh.desktop_menu, Some(Menu::Recent), "the submenu opens");
            let rows = sh.menu_row_texts(Menu::Recent);
            assert!(rows.iter().any(|t| t == "Settings") && rows.iter().any(|t| t == "Term"), "{rows:?}");
            assert_eq!(rows.last().map(String::as_str), Some("Clear Menu"));
            let (x, _, _, _) = sh.desktop_popup_rect().expect("the submenu has a popup");
            let levels = sh.menu_level_rects();
            assert_eq!(levels.iter().map(|l| l.0).collect::<Vec<_>>(), vec![Menu::System, Menu::Recent], "the system menu stays open beside it");
            assert_eq!(levels[1].1 .0, x);
            let (sx, _, sw, _) = levels[0].1;
            assert!(x > sx && x + sh.ui.pt(4.) as usize >= sx + sw, "the submenu cascades to the right of the system menu: {x} vs {sx}+{sw}");
        }

        #[test]
        fn option_command_escape_and_shift_command_q_open_the_cards() {
            let mut sh = desktop();
            let _ = sh.hw_shortcut(hwkeys::Shortcut::ForceQuit, 1.);
            assert_eq!(sh.utility.as_ref().map(|w| w.kind), Some(Kind::ForceQuit));
            assert!(sh.utility_overlay().is_some(), "the card is an overlay surface");
            // Esc closes it
            assert!(sh.utility_key(1, true, 2.).is_some());
            assert!(sh.utility.is_none());
            let _ = sh.hw_shortcut(hwkeys::Shortcut::LogOut, 3.);
            assert_eq!(sh.utility.as_ref().map(|w| w.kind), Some(Kind::LogOut));
            assert!(!sh.is_locked(), "Log Out asks first");
        }

        #[test]
        fn a_card_does_not_survive_the_lock_and_takes_no_key_at_the_login_window() {
            let mut sh = desktop();
            sh.utility_open(Kind::LogOut);
            let _ = sh.lock_screen();
            assert!(sh.utility.is_none(), "every lock path drops the card");
            assert!(sh.is_locked());
            // a card forced up behind the lock still gets no key: Return there is the password's
            sh.utility = Some(UtilityWindow::new(Kind::LogOut));
            assert!(sh.utility_key(28, true, 2.).is_none(), "Return reaches the login window");
            assert!(sh.utility_key(1, true, 2.).is_none());
            assert!(sh.utility.is_some() && sh.is_locked(), "nothing ran");
            // and the chords do not open one over the login window
            sh.utility = None;
            let _ = sh.hw_shortcut(hwkeys::Shortcut::ForceQuit, 3.);
            let _ = sh.hw_shortcut(hwkeys::Shortcut::LogOut, 3.);
            assert!(sh.utility.is_none());
        }

        #[test]
        fn force_quit_lists_running_apps_and_closes_a_built_in_one() {
            let mut sh = desktop();
            sh.desk_open(AppId::Settings, 0.);
            sh.utility_open(Kind::ForceQuit);
            let w = sh.utility.as_ref().unwrap();
            assert!(w.entries.iter().any(|e| e.target == Target::App(AppId::Settings)), "Settings runs and is listed");
            let fx = sh.utility_force_quit(Target::App(AppId::Settings));
            assert!(fx.is_empty());
            assert!(sh.desk.as_ref().unwrap().index_of(AppId::Settings).is_none(), "its window is gone");
        }

        #[test]
        fn confirming_log_out_closes_every_window_and_returns_to_the_login_window() {
            let mut sh = desktop();
            // the windows are written down for the next login: into a directory of this test, not the user's
            let dir = std::env::temp_dir().join(format!("bifrost-logout-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            sh.data_dir = dir.clone();
            sh.desk_open(AppId::Settings, 0.);
            sh.desk_open(AppId::Terminal, 0.);
            sh.utility_open(Kind::LogOut);
            let _ = sh.utility_outcome(Outcome::LogOut, 5.);
            assert!(sh.utility.is_none());
            assert!(sh.is_locked(), "the screen locks");
            assert!(sh.desk.as_ref().unwrap().wins.is_empty(), "every window is closed");
            assert!(!sh.session.restored, "the next login replays the windows");
            assert!(dir.join(crate::shell::session::FILE).exists(), "the layout was saved before the windows closed");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
