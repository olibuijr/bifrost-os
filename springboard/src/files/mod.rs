//! Files: the bifrost 15 file browser. A navigation stack of pages (the
//! locations list, folder listings, previews) with the push transition,
//! a long-press context menu, alerts for rename/delete/new folder, a
//! move/copy picker and a video hand-off to the shell's media player.
//! Every directory read and file decode runs on a worker pool whose
//! results the shell pumps through `fds`, so listing /system never stalls
//! a frame.

pub mod alert;
pub mod archive;
pub mod batch;
mod draw;
pub mod find;
pub mod handoff;
pub mod fs;
pub mod info;
pub mod icons;
pub mod menu;
pub mod nav;
pub mod pages;
pub mod pathbar;
pub mod preview;
pub mod text;
pub mod tags;
pub mod trash;
pub mod viewprefs;
pub mod zip;

use crate::deildu::jobs::Pool;
use crate::deildu::play::{Player, StubPlayer};
use crate::font::Font;
use crate::gfx::Image;
use crate::shell::keyboard::{Keyboard, Out};
use crate::shell::ui::{home_zone, Ui, Weight, WHITE};
use crate::shell::Event;
use alert::{Alert, Purpose};
use fs::{Entry, Kind, Location, Sort};
use menu::{Action, Menu, Subject};
use nav::Nav;
use pages::{Body, DirPage, LocationsPage, Page, PreviewPage, Target, FOOTER_PT};
use std::path::{Path, PathBuf};
use text::TextView;

const WORKERS: usize = 2;
/// How long a finger rests before the context menu opens.
pub const LONG_PRESS_MS: f64 = 500.0;
/// Movement that turns a press into a drag.
const SLOP: f32 = 12.0;
/// Bytes sniffed to classify a file of unknown kind.
const SNIFF: usize = 4096;

/// Something the shell must do for the app.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// open the Terminal at this directory
    OpenTerminal(PathBuf),
    /// share this file (the share sheet: Copy, Save to Files, Send Nearby)
    Share(PathBuf),
    /// play this video with the shell's media player
    Play(PathBuf),
    /// show this picture in the Photos viewer (Files previews it when Photos does not know it)
    OpenPhoto(Entry),
    /// play this song in Music (Files previews it when Music does not know it)
    PlayAudio(Entry),
    /// open this PDF or EPUB in Books
    OpenBook(PathBuf),
    /// mark up this picture in the shell's Markup editor (Preview)
    Markup(PathBuf),
}

/// What a preview job found.
enum Loaded {
    Text(String, bool),
    Image(Image),
    Info(Kind),
    Error(String),
}

/// A finished background job.
enum Done {
    Listing { gen: u64, path: PathBuf, result: Result<Vec<Entry>, String>, available: Option<u64> },
    Preview { path: PathBuf, result: Loaded },
    Op { what: &'static str, result: Result<(), String> },
}

/// The Move to… / Copy destination picker: its own stack over the app.
struct Picker {
    srcs: Vec<PathBuf>,
    copy: bool,
    nav: Nav<Page>,
    cancel: Image,
    confirm: Image,
}

/// Video playback over everything, through the shell's player.
struct Playing {
    title: Image,
    frame: Option<Image>,
    status: String,
    status_img: Image,
    overlay: bool,
    stop: Image,
    /// the fullscreen player once the decoder opened the file
    view: Option<crate::deildu::player_view::PlayerView>,
    path: PathBuf,
}

pub struct Files {
    w: i32,
    h: i32,
    mono: Font<'static>,
    /// Where Delete puts things (the first location's Recently Deleted).
    bin: PathBuf,
    pool: Pool<Done>,
    nav: Nav<Page>,
    picker: Option<Picker>,
    menu: Option<Menu>,
    alert: Option<Alert>,
    playing: Option<Playing>,
    player: Box<dyn Player>,
    keyboard: Keyboard,
    kb_touch: bool,
    /// the path Copy put aside for Paste
    clipboard: Option<PathBuf>,
    /// select mode: tapping marks items for the Copy/Move/Delete bar
    selecting: bool,
    selected: Vec<PathBuf>,
    /// hand images to Photos and audio to Music instead of previewing them
    handoff: bool,
    grid: bool,
    sort: Sort,
    /// colour tags of items, persisted in the sidecar of the first location
    tags: tags::TagIndex,
    tags_file: PathBuf,
    /// wall clock (Unix seconds) and zone offset, for the dates shown
    now: i64,
    offset: i32,
    pressed: Option<Target>,
    /// where and when the current touch began, until it moves
    press: Option<(f32, f32, f64)>,
    /// the current touch is held by a scroller
    scrolling: bool,
    /// the current touch opened the menu: its Up does nothing
    swallow: bool,
    /// where a touch on a Preview page began, to tell a swipe between files from a scroll
    ql_start: Option<(f32, f32)>,
    menu_pressed: Option<Action>,
    alert_pressed: Option<alert::Hit>,
    gen: u64,
    dirty: bool,
    /// Rendered labels (Back, sort, menu and footer text), bounded like the shell's text caches
    text_cache: std::cell::RefCell<crate::shell::cache::TextCache>,
}

impl Files {
    /// `mono` is SFUIMono.ttf for text previews. Creates the app's own
    /// folder (`fs::home()`) when it is missing.
    pub fn new(ui: &Ui, w: i32, h: i32, mono: &'static [u8]) -> std::io::Result<Self> {
        Self::with_locations(ui, w, h, mono, fs::locations())
    }

    /// Like `new` with the sidebar roots given (tests point them at temp dirs).
    pub fn with_locations(ui: &Ui, w: i32, h: i32, mono: &'static [u8], locations: Vec<Location>) -> std::io::Result<Self> {
        if let Some(home) = locations.first() {
            let _ = std::fs::create_dir_all(&home.path);
        }
        let mono = Font::parse(mono).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad mono font"))?;
        let tags_file = tags::sidecar_path(&locations.first().map_or_else(fs::home, |l| l.path.clone()));
        let bin = locations.first().map(|l| trash::dir(&l.path)).unwrap_or_else(fs::trash_dir);
        Ok(Files {
            w,
            h,
            mono,
            bin,
            pool: Pool::new(WORKERS)?,
            nav: Nav::new(Page::Locations(LocationsPage::new(ui, locations))),
            picker: None,
            menu: None,
            alert: None,
            playing: None,
            player: Box::new(StubPlayer::default()),
            keyboard: Keyboard::new_hidden(w as f32, ui.s),
            kb_touch: false,
            clipboard: None,
            selecting: false,
            selected: vec![],
            handoff: false,
            grid: false,
            sort: Sort::Name,
            tags: tags::TagIndex::load(&tags_file),
            tags_file,
            now: 0,
            offset: 0,
            pressed: None,
            press: None,
            scrolling: false,
            swallow: false,
            ql_start: None,
            menu_pressed: None,
            alert_pressed: None,
            gen: 0,
            dirty: true,
            text_cache: Default::default(),
        })
    }

    /// Lays the app out for a surface of `w` x `h` px (portrait column, or the full landscape
    /// screen while the video player is up) and tells the player its viewport.
    pub fn set_size(&mut self, w: i32, h: i32) {
        if (w, h) != (self.w, self.h) {
            self.w = w;
            self.h = h;
            self.keyboard.resize(w as f32);
            if self.playing.is_some() {
                self.player.set_viewport(w, h);
            }
            self.dirty = true;
        }
    }

    /// Swaps in the shell's media player.
    pub fn set_player(&mut self, player: Box<dyn Player>) {
        self.player = player;
    }

    /// Turns on the Photos and Music hand-offs (the shell does; they need the other apps).
    pub fn set_handoff(&mut self, on: bool) {
        self.handoff = on;
    }

    /// Opens the preview page for `entry` (the fallback when a hand-off finds nothing).
    pub fn preview_entry(&mut self, ui: &Ui, entry: &Entry, t: f64) {
        self.open_preview(ui, entry, t);
        self.dirty = true;
    }

    /// Search's Show in Folder: the location list with `folder` pushed on top of it, under the
    /// location that holds it. False (nothing changes) when no location holds the folder.
    pub fn show_folder(&mut self, ui: &Ui, folder: &Path, t: f64) -> bool {
        let root = match self.nav.pages().first() {
            Some(Page::Locations(l)) => l.locations.iter().filter(|l| folder.starts_with(&l.path)).max_by_key(|l| l.path.as_os_str().len()).cloned(),
            _ => None,
        };
        let Some(root) = root else { return false };
        self.active_nav_mut().pop_to(1, t);
        self.push_dir(ui, &root, folder, t);
        self.dirty = true;
        true
    }

    /// Anchors the dates shown to the wall clock.
    pub fn set_wall(&mut self, unix: i64, offset: i32) {
        self.now = unix;
        self.offset = offset;
    }

    /// Fds to wait on: finished jobs and, while playing, the player.
    pub fn fds(&self) -> Vec<std::ffi::c_int> {
        let mut v = vec![self.pool.fd()];
        if self.playing.is_some() {
            v.extend(self.player.fd());
        }
        v
    }

    /// True while something moves on its own (a push, a fling, the keyboard).
    pub fn animating(&self) -> bool {
        self.nav.animating() || self.picker.as_ref().is_some_and(|p| p.nav.animating()) || self.keyboard.animating() || self.top_scroll_animating()
    }

    fn top_scroll_animating(&self) -> bool {
        let nav = self.picker.as_ref().map_or(&self.nav, |p| &p.nav);
        match nav.top() {
            Page::Dir(d) => d.scroll.animating(),
            Page::Preview(p) => p.scroll.animating(),
            Page::Locations(_) => false,
        }
    }

    /// When `tick` is next needed without input: the long-press deadline
    /// or a caret blink.
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        let mut due: Option<f64> = self.press.filter(|_| self.menu.is_none() && self.alert.is_none()).map(|(_, _, t0)| t0 + crate::shell::accessibility::long_press_ms(LONG_PRESS_MS));
        let blink = match (&self.alert, self.nav.top()) {
            (Some(a), _) => a.field.as_ref().and_then(|f| f.next_blink(t)),
            (None, Page::Dir(d)) if d.searching => d.query.next_blink(t),
            _ => None,
        };
        if let Some(b) = blink {
            due = Some(due.map_or(b, |d| d.min(b)));
        }
        due
    }

    /// key=value lines for remote inspection.
    pub fn describe(&self) -> String {
        let page = match self.nav.top() {
            Page::Locations(_) => "locations".to_string(),
            Page::Dir(d) => format!("dir:{}", d.path.display()),
            Page::Preview(p) => format!("preview:{}", p.path.display()),
        };
        let overlay = if self.playing.is_some() {
            "playing"
        } else if self.alert.is_some() {
            "alert"
        } else if self.menu.is_some() {
            "menu"
        } else if self.picker.is_some() {
            "picker"
        } else {
            "none"
        };
        format!("files_page={page}\nfiles_depth={}\nfiles_overlay={overlay}\nfiles_grid={}\nfiles_sort={:?}\nfiles_selecting={}\nfiles_selected={}\n", self.nav.depth(), self.grid, self.sort, self.selecting, self.selected.len())
    }

    // --- geometry ---

    fn kb_top(&self, ui: &Ui) -> f32 {
        self.h as f32 - home_zone(ui) - self.keyboard.h()
    }

    /// Bottom of the page content: above the keyboard, the footer or the home zone.
    fn bottom(&self, ui: &Ui) -> f32 {
        if self.keyboard.shown() && self.alert.is_none() {
            return self.kb_top(ui);
        }
        let nav = self.picker.as_ref().map_or(&self.nav, |p| &p.nav);
        let footer = if matches!(nav.top(), Page::Dir(_)) { ui.pt(FOOTER_PT) } else { 0.0 };
        self.h as f32 - home_zone(ui) - footer
    }

    fn active_nav(&self) -> &Nav<Page> {
        self.picker.as_ref().map_or(&self.nav, |p| &p.nav)
    }

    fn active_nav_mut(&mut self) -> &mut Nav<Page> {
        match &mut self.picker {
            Some(p) => &mut p.nav,
            None => &mut self.nav,
        }
    }

    /// The nav bar's left button and right buttons for the page on top.
    fn nav_buttons(&self) -> (Option<Target>, Vec<Target>) {
        let nav = self.active_nav();
        let picking = self.picker.is_some();
        let left = match (picking, nav.depth()) {
            (true, 1) => Some(Target::Cancel),
            (_, 1) => None,
            _ => Some(Target::Back),
        };
        let right = match (picking, nav.top()) {
            (true, Page::Dir(_)) => vec![Target::Confirm],
            (false, Page::Dir(_)) if self.selecting => vec![Target::Select],
            (false, Page::Dir(d)) if d.path == Path::new(fs::RECENTS) => vec![Target::ViewToggle, Target::Select],
            (false, Page::Dir(_)) => vec![Target::ViewToggle, Target::NewFolder, Target::Select],
            (false, Page::Preview(p)) if matches!(p.body, Body::Image(_)) => vec![Target::Share, Target::Markup],
            (false, Page::Preview(p)) if !matches!(p.body, Body::Loading) => vec![Target::Share],
            _ => vec![],
        };
        (left, right)
    }

    /// What a touch on the page (no overlay) hits.
    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        if self.keyboard.shown() && y >= self.kb_top(ui) {
            return Some(Target::Keyboard);
        }
        let (left, right) = self.nav_buttons();
        if let Some(t) = pages::nav_hit(ui, self.w, left, &right, x, y) {
            return Some(t);
        }
        let bottom = self.bottom(ui);
        if self.selecting && self.picker.is_none() {
            if let Some(t) = pages::bulk_hit(ui, self.w, bottom, x, y) {
                return Some(t);
            }
        }
        match self.active_nav().top() {
            Page::Locations(l) => l.hit(ui, self.w, x, y),
            Page::Dir(d) => d.hit(ui, self.w, self.grid, bottom, x, y),
            Page::Preview(_) => (y >= ui.pt(pages::STATUS_PT + pages::NAV_PT) && y < bottom).then_some(Target::Content),
        }
    }

    // --- jobs ---

    /// Reloads the folder on top of the main stack.
    fn refresh_top(&mut self) {
        self.gen += 1;
        if let Page::Dir(d) = self.nav.top_mut() {
            spawn_listing(&self.pool, self.gen, d);
        }
    }

    fn push_dir(&mut self, ui: &Ui, root: &Location, path: &Path, t: f64) {
        let mut page = DirPage::new(ui, root, path);
        self.gen += 1;
        spawn_listing(&self.pool, self.gen, &mut page);
        self.hide_keyboard(t);
        self.active_nav_mut().push(Page::Dir(page), t);
    }

    fn open_preview(&mut self, ui: &Ui, entry: &Entry, t: f64) {
        let page = self.spawn_preview(ui, entry);
        self.hide_keyboard(t);
        self.nav.push(Page::Preview(page), t);
    }

    /// Starts loading `entry`'s preview on a worker and returns its page.
    fn spawn_preview(&mut self, ui: &Ui, entry: &Entry) -> PreviewPage {
        let page = PreviewPage::new(ui, entry);
        let (path, kind) = (entry.path.clone(), entry.kind);
        let (w, h) = (self.w as usize, (self.bottom(ui) - ui.pt(pages::STATUS_PT + pages::NAV_PT)) as usize);
        self.pool.spawn(move || {
            let kind = if kind == Kind::Other { fs::head(&path, SNIFF).map(|h| fs::sniff(&h)).unwrap_or(Kind::Other) } else { kind };
            let result = if preview::is_pdf(&path.to_string_lossy()) {
                // the first page, drawn by the PDF engine and fitted to the pane
                match std::fs::read(&path) {
                    Ok(bytes) => match bifrost_render::cg::pdf::draw::render_file_page(bytes, 0, w, h) {
                        Ok(img) => Loaded::Image(img),
                        Err(e) => Loaded::Error(e),
                    },
                    Err(e) => Loaded::Error(e.to_string()),
                }
            } else {
                match kind {
                    Kind::Text => match fs::read_text(&path) {
                        Ok((text, truncated)) => Loaded::Text(text, truncated),
                        Err(e) => Loaded::Error(e.to_string()),
                    },
                    Kind::Image => match std::fs::read(&path) {
                        Ok(bytes) => match crate::cg::imageio::decode(&bytes) {
                            Some(img) => Loaded::Image(crate::imgdec::fit(&img, w, h)),
                            None => Loaded::Error("The image could not be decoded.".into()),
                        },
                        Err(e) => Loaded::Error(e.to_string()),
                    },
                    k => Loaded::Info(k),
                }
            };
            Done::Preview { path, result }
        });
        page
    }

    /// Preview: swaps the open preview for the previous (`-1`) or next
    /// (`1`) file of its folder; false at either end or outside a folder.
    fn step_preview(&mut self, ui: &Ui, dir: i32) -> bool {
        let Page::Preview(cur) = self.nav.top() else { return false };
        let Some(Page::Dir(d)) = self.nav.under() else { return false };
        let files: Vec<&Entry> = d.shown.iter().map(|&i| &d.entries[i]).filter(|e| !e.is_dir && e.kind != Kind::Video).collect();
        let Some(at) = files.iter().position(|e| e.path == cur.path) else { return false };
        let Some(next) = at.checked_add_signed(dir as isize).and_then(|i| files.get(i)).map(|e| (*e).clone()) else { return false };
        let page = self.spawn_preview(ui, &next);
        *self.nav.top_mut() = Page::Preview(page);
        true
    }

    fn spawn_op(&mut self, what: &'static str, op: impl FnOnce() -> std::io::Result<()> + Send + 'static) {
        self.pool.spawn(move || Done::Op { what, result: op().map_err(|e| e.to_string()) });
    }

    /// Applies finished jobs; true if the screen changed.
    pub fn pump(&mut self, ui: &Ui) -> bool {
        let mut changed = false;
        for done in self.pool.drain() {
            changed = true;
            match done {
                Done::Listing { gen, path, result, available } => {
                    let sort = self.sort;
                    let pages = self.nav.pages_mut().iter_mut().chain(self.picker.iter_mut().flat_map(|p| p.nav.pages_mut().iter_mut()));
                    for page in pages {
                        if let Page::Dir(d) = page {
                            if d.gen == gen && d.path == path {
                                match &result {
                                    Ok(entries) => d.set_entries(ui, entries.clone(), sort, available),
                                    Err(e) => d.set_error(ui, e),
                                }
                            }
                        }
                    }
                }
                Done::Preview { path, result } => {
                    let (w, now, offset) = (self.w, self.now, self.offset);
                    for page in self.nav.pages_mut() {
                        if let Page::Preview(p) = page {
                            if p.path != path {
                                continue;
                            }
                            p.body = match &result {
                                Loaded::Text(text, truncated) => Body::Text(TextView::new(text, *truncated)),
                                Loaded::Image(img) => Body::Image(img.clone()),
                                Loaded::Info(kind) => {
                                    p.kind = *kind;
                                    Body::Info(PreviewPage::info_rows(ui, w, &p.entry, *kind, now, offset))
                                }
                                Loaded::Error(e) => Body::Error(ui.text(&ui.fit(e, 15.0, ui.pt(600.0)), 15.0, Weight::Regular, crate::shell::ui::SECONDARY)),
                            };
                            p.scroll.jump_top();
                        }
                    }
                }
                Done::Op { what, result } => match result {
                    Ok(()) => self.refresh_top(),
                    Err(e) => self.alert = Some(Alert::error(ui, &format!("Couldn’t {what}"), &e)),
                },
            }
        }
        if changed {
            self.dirty = true;
        }
        changed
    }

    // --- playback ---

    /// Plays `path` with the player (the shell calls this for videos).
    pub fn play(&mut self, ui: &Ui, path: &Path) {
        self.stop();
        let title = ui.text(&ui.fit(&fs::title_of(path), 17.0, ui.pt(600.0)), 17.0, Weight::Semibold, WHITE);
        self.player.set_viewport(self.w, self.h);
        let opened = self.player.open(path, crate::deildu::play::mime_for(&path.to_string_lossy()));
        let status = match &opened {
            Ok(()) => self.player.status(),
            Err(e) => e.to_string(),
        };
        let view = opened.is_ok().then(|| crate::deildu::player_view::PlayerView::new(ui, self.w, self.h, &fs::title_of(path), ""));
        self.playing = Some(Playing { view, path: path.to_path_buf(), title, frame: None, status_img: ui.text(&ui.fit(&status, 15.0, ui.pt(600.0)), 15.0, Weight::Regular, crate::shell::ui::SECONDARY), status, overlay: true, stop: ui.text("Stop", 17.0, Weight::Semibold, crate::shell::ui::BLACK) });
        self.dirty = true;
    }

    fn stop(&mut self) {
        if self.playing.take().is_some() {
            self.player.stop();
            self.dirty = true;
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playing.is_some()
    }

    /// The player's picture for the GPU (see `PlayerView::video_layer`). `gpu` says whether the
    /// shell can draw pictures at all (the player then stops converting them), `eligible`
    /// whether it composes the coming frame.
    pub fn video_layer(&mut self, gpu: bool, eligible: bool) -> Option<crate::media::VideoLayer> {
        self.player.set_gpu(gpu);
        let up = self.fullscreen();
        let view = self.playing.as_ref().and_then(|p| p.view.as_ref())?;
        view.video_layer(gpu && eligible && up)
    }

    /// True while the fullscreen player is up (the shell hides its status bar).
    pub fn fullscreen(&self) -> bool {
        self.playing.as_ref().is_some_and(|p| p.view.is_some())
    }

    /// A hardware key (evdev code) for the player: space/enter toggle, arrows skip. False = not used.
    /// Esc: up one folder (a menu or alert swallows it); false at the root.
    pub fn hw_back(&mut self, t: f64) -> bool {
        if self.menu.is_some() || self.alert.is_some() {
            return true;
        }
        if self.active_nav_mut().pages().len() < 2 {
            return false;
        }
        self.leave_select();
        self.hide_keyboard(t);
        self.active_nav_mut().pop(t);
        self.dirty = true;
        true
    }

    pub fn hw_key(&mut self, code: u16, t: f64) -> bool {
        match self.playing.as_mut().and_then(|p| p.view.as_mut()) {
            Some(v) => {
                self.dirty = true;
                v.key(&mut *self.player, code, t)
            }
            None => false,
        }
    }

    /// True while the folder on top is still being listed.
    pub fn loading(&self) -> bool {
        matches!(self.nav.top(), Page::Dir(d) if d.loading)
    }

    /// Centre of the open menu's row for `action` (tests and previews drive
    /// the app through this); `bottom` is the lowest usable y.
    pub fn menu_row(&self, ui: &Ui, action: Action, bottom: f32) -> Option<(f32, f32)> {
        self.menu.as_ref()?.rows(ui, self.w, bottom).into_iter().find(|(a, _)| *a == action).map(|(_, (x, y, w, h))| (x + w / 2.0, y + h / 2.0))
    }

    /// Centre of the open alert's button `i`.
    pub fn alert_button_centre(&self, ui: &Ui, i: usize) -> Option<(f32, f32)> {
        let bottom = if self.keyboard.shown() { self.kb_top(ui) } else { self.h as f32 - home_zone(ui) };
        let (x, y, w, h) = self.alert.as_ref()?.button_rect(ui, self.w, bottom, i);
        Some((x + w / 2.0, y + h / 2.0))
    }

    fn stop_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        ((self.w as f32 - ui.pt(140.0)) / 2.0, self.h as f32 - ui.pt(160.0), ui.pt(140.0), ui.pt(44.0))
    }

    // --- input ---

    fn show_keyboard(&mut self, t: f64) {
        self.keyboard.set_shown(true, t);
    }

    fn hide_keyboard(&mut self, t: f64) {
        self.keyboard.set_shown(false, t);
        if let Page::Dir(d) = self.nav.top_mut() {
            d.query.focused = false;
        }
    }

    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) -> Vec<Request> {
        self.dirty = true;
        let (x, y) = match ev {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            Event::Power | Event::Wake | Event::VolumeUp | Event::VolumeDown | Event::Screenshot => return vec![],
        };
        if let Some(v) = self.playing.as_mut().and_then(|p| p.view.as_mut()) {
            match v.touch(ui, &mut *self.player, ev, t) {
                crate::deildu::player_view::Outcome::Close => self.stop(),
                crate::deildu::player_view::Outcome::Retry => {
                    if let Some(path) = self.playing.as_ref().map(|p| p.path.clone()) {
                        self.play(ui, &path);
                    }
                }
                crate::deildu::player_view::Outcome::None => {}
            }
            return vec![];
        }
        if self.playing.is_some() {
            if let Event::Up(..) = ev {
                let stop = self.stop_rect(ui);
                let (overlay, has_frame) = self.playing.as_ref().map_or((false, false), |p| (p.overlay, p.frame.is_some()));
                if overlay && crate::deildu::within(x, y, stop) {
                    self.stop();
                } else if has_frame {
                    if let Some(p) = &mut self.playing {
                        p.overlay = !p.overlay;
                    }
                }
            }
            return vec![];
        }
        // the touch that opened the menu ends without touching the menu
        if self.swallow {
            if let Event::Up(..) = ev {
                self.swallow = false;
                self.pressed = None;
                self.press = None;
                self.scrolling = false;
            }
            return vec![];
        }
        if self.alert.is_some() {
            self.alert_touch(ui, ev, x, y, t);
            return vec![];
        }
        if self.menu.is_some() {
            return self.menu_touch(ui, ev, x, y, t);
        }
        let kb_top = self.kb_top(ui);
        match ev {
            Event::Down(..) => {
                self.swallow = false;
                self.pressed = self.hit(ui, x, y);
                self.press = Some((x, y, t));
                self.ql_start = (self.pressed == Some(Target::Content) && matches!(self.nav.top(), Page::Preview(_))).then_some((x, y));
                if self.pressed == Some(Target::Keyboard) {
                    self.kb_touch = true;
                    self.keyboard.down(x, y - kb_top, t);
                } else if self.pressed.is_some_and(scrolls) {
                    if let Some(s) = self.active_nav_mut().top_mut().scroller() {
                        s.down(y, t);
                        self.scrolling = true;
                    }
                }
            }
            Event::Move(..) => {
                if let Some((x0, y0, _)) = self.press {
                    if (x - x0).abs() > SLOP || (y - y0).abs() > SLOP {
                        self.press = None;
                    }
                }
                if self.kb_touch {
                    self.keyboard.moved(x, y - kb_top);
                } else if self.scrolling {
                    let dragging = self.active_nav_mut().top_mut().scroller().map(|s| {
                        s.moved(y, t);
                        s.dragging()
                    });
                    if dragging == Some(true) {
                        self.pressed = None;
                    }
                }
            }
            Event::Up(..) | Event::Power | Event::Wake | Event::VolumeUp | Event::VolumeDown | Event::Screenshot => {
                let pressed = self.pressed.take();
                self.press = None;
                if self.kb_touch {
                    self.kb_touch = false;
                    if let Some(out) = self.keyboard.up(x, y - kb_top, t) {
                        self.key(ui, out, t);
                    }
                    return vec![];
                }
                if let Some((x0, y0)) = self.ql_start.take() {
                    let (dx, dy) = (x - x0, y - y0);
                    if dx.abs() > ui.pt(80.0) && dx.abs() > 2.0 * dy.abs() {
                        if self.scrolling {
                            self.scrolling = false;
                            if let Some(s) = self.active_nav_mut().top_mut().scroller() {
                                s.up(y, t);
                            }
                        }
                        self.step_preview(ui, if dx < 0.0 { 1 } else { -1 });
                        return vec![];
                    }
                }
                let tap = if std::mem::take(&mut self.scrolling) { self.active_nav_mut().top_mut().scroller().is_none_or(|s| s.up(y, t)) } else { true };
                if tap {
                    if let Some(p) = pressed.filter(|&p| Some(p) == self.hit(ui, x, y)) {
                        return self.activate(ui, p, t);
                    }
                }
            }
        }
        vec![]
    }

    fn alert_touch(&mut self, ui: &Ui, ev: Event, x: f32, y: f32, t: f64) {
        let kb_top = self.kb_top(ui);
        let bottom = if self.keyboard.shown() { kb_top } else { self.h as f32 - home_zone(ui) };
        let on_keyboard = self.keyboard.shown() && y >= kb_top;
        match ev {
            Event::Down(..) => {
                if on_keyboard {
                    self.kb_touch = true;
                    self.keyboard.down(x, y - kb_top, t);
                } else {
                    self.alert_pressed = self.alert.as_ref().and_then(|a| a.hit(ui, self.w, bottom, x, y));
                }
            }
            Event::Move(..) if self.kb_touch => self.keyboard.moved(x, y - kb_top),
            Event::Move(..) => {}
            _ => {
                if self.kb_touch {
                    self.kb_touch = false;
                    if let Some(out) = self.keyboard.up(x, y - kb_top, t) {
                        self.key(ui, out, t);
                    }
                    return;
                }
                let pressed = self.alert_pressed.take();
                let now = self.alert.as_ref().and_then(|a| a.hit(ui, self.w, bottom, x, y));
                if let Some(hit) = pressed.filter(|p| Some(*p) == now) {
                    match hit {
                        alert::Hit::Button(i) => self.alert_button(i, t),
                        alert::Hit::Field => {
                            let fx = self.alert.as_ref().and_then(|a| a.field_rect(ui, self.w, bottom)).map(|r| r.0);
                            if let (Some(fx), Some(f)) = (fx, self.alert.as_mut().and_then(|a| a.field.as_mut())) {
                                f.tap(ui, x - fx - ui.pt(8.0), t);
                            }
                            self.show_keyboard(t);
                        }
                    }
                }
            }
        }
    }

    fn alert_button(&mut self, i: usize, t: f64) {
        let Some(a) = &self.alert else { return };
        let (confirm, can, name, purpose) = (a.confirm_index(), a.can_confirm(), a.name(), a.purpose.clone());
        if i == confirm {
            if !can {
                return;
            }
            match purpose {
                // into Recently Deleted (kept 30 days); inside it, for good
                Purpose::Delete(p) => {
                    let bin = self.bin.clone();
                    self.spawn_op("delete", move || trash::delete_item(&p, &bin));
                }
                Purpose::DeleteMany(ps) => {
                    self.leave_select();
                    let bin = self.bin.clone();
                    self.spawn_op("delete", move || each(&ps, |p| trash::delete_item(p, &bin)));
                }
                Purpose::Rename(p) => self.spawn_op("rename", move || fs::rename(&p, &name).map(|_| ())),
                Purpose::NewFolder(dir) => self.spawn_op("create the folder", move || fs::new_folder(&dir, &name).map(|_| ())),
                Purpose::Error => {}
            }
        }
        self.alert = None;
        self.hide_keyboard(t);
    }

    fn menu_touch(&mut self, ui: &Ui, ev: Event, x: f32, y: f32, t: f64) -> Vec<Request> {
        let bottom = self.h as f32 - home_zone(ui);
        let hit = |m: &Menu| m.hit(ui, self.w, bottom, x, y);
        match ev {
            Event::Down(..) => self.menu_pressed = self.menu.as_ref().and_then(hit),
            Event::Move(..) => {}
            _ => {
                let pressed = self.menu_pressed.take();
                let now = self.menu.as_ref().and_then(hit);
                match (pressed, now) {
                    (Some(a), Some(b)) if a == b => return self.act(ui, a, t),
                    (None, None) => self.menu = None,
                    _ => {}
                }
            }
        }
        vec![]
    }

    /// Typed input: the search field, or an alert's name field.
    pub fn key(&mut self, ui: &Ui, out: Out, t: f64) {
        if out == Out::Hide {
            self.hide_keyboard(t);
            return;
        }
        if let Some(a) = &mut self.alert {
            if a.edit(ui, &out, t) {
                let i = a.confirm_index();
                self.alert_button(i, t);
            }
            return;
        }
        if let Page::Dir(d) = self.nav.top_mut() {
            if d.searching {
                match out {
                    Out::Enter => self.hide_keyboard(t),
                    o => {
                        d.query.edit(ui, &o, t);
                        d.refilter(ui);
                    }
                }
            }
        }
    }

    fn activate(&mut self, ui: &Ui, target: Target, t: f64) -> Vec<Request> {
        match target {
            Target::Back => {
                self.leave_select();
                self.hide_keyboard(t);
                self.active_nav_mut().pop(t);
            }
            Target::Cancel => self.picker = None,
            Target::Confirm => {
                if let Some(p) = self.picker.take() {
                    if let Page::Dir(d) = p.nav.top() {
                        let (srcs, dst) = (p.srcs.clone(), d.path.clone());
                        self.leave_select();
                        if p.copy {
                            self.spawn_op("copy", move || each(&srcs, |s| fs::copy_to(s, &dst).map(|_| ())));
                        } else if srcs.first().is_some_and(|s| s.starts_with(fs::trash_dir())) {
                            self.spawn_op("put back", move || each(&srcs, |s| trash::restore(s, &dst).map(|_| ())));
                        } else {
                            self.spawn_op("move", move || each(&srcs, |s| fs::move_to(s, &dst).map(|_| ())));
                        }
                    }
                }
            }
            Target::ViewToggle => self.grid = !self.grid,
            Target::NewFolder => {
                if let Some(Page::Dir(d)) = Some(self.nav.top()).filter(|p| !matches!(p, Page::Dir(d) if d.path == Path::new(fs::RECENTS))) {
                    self.alert = Some(Alert::new_folder(ui, d.path.clone()));
                    self.show_keyboard(t);
                }
            }
            Target::Select => {
                if self.selecting {
                    self.leave_select();
                } else {
                    self.selecting = true;
                }
            }
            Target::BulkCopy | Target::BulkMove | Target::BulkDelete => {
                if self.selected.is_empty() {
                    return vec![];
                }
                let paths = self.selected.clone();
                match target {
                    Target::BulkCopy => self.open_picker(ui, paths, true),
                    Target::BulkMove => self.open_picker(ui, paths, false),
                    _ => self.alert = Some(Alert::delete_many(ui, paths)),
                }
            }
            Target::Search => {
                if let Page::Dir(d) = self.active_nav_mut().top_mut() {
                    d.searching = true;
                    d.query.focused = true;
                }
                self.show_keyboard(t);
            }
            Target::SearchClear => {
                if let Page::Dir(d) = self.active_nav_mut().top_mut() {
                    d.query.set_text(ui, "");
                    d.refilter(ui);
                }
            }
            Target::SearchCancel => {
                if let Page::Dir(d) = self.active_nav_mut().top_mut() {
                    d.searching = false;
                    d.query.set_text(ui, "");
                    d.refilter(ui);
                }
                self.hide_keyboard(t);
            }
            Target::Sort(s) => {
                self.sort = s;
                if let Page::Dir(d) = self.active_nav_mut().top_mut() {
                    d.resort(ui, s);
                }
            }
            Target::Crumb(i) => {
                self.hide_keyboard(t);
                self.active_nav_mut().pop_to(i + 2, t);
            }
            Target::Location(i) => {
                let Page::Locations(l) = self.active_nav().top() else { return vec![] };
                let root = l.locations[i].clone();
                self.push_dir(ui, &root, &root.path.clone(), t);
            }
            Target::Entry(i) => {
                let Page::Dir(d) = self.active_nav().top() else { return vec![] };
                let (root, entry) = (d.root.clone(), d.entries[i].clone());
                if self.selecting && self.picker.is_none() {
                    match self.selected.iter().position(|p| *p == entry.path) {
                        Some(k) => {
                            self.selected.remove(k);
                        }
                        None => self.selected.push(entry.path),
                    }
                } else if entry.is_dir {
                    self.push_dir(ui, &root, &entry.path, t);
                } else if self.picker.is_none() {
                    if entry.kind == Kind::Video {
                        return vec![Request::Play(entry.path)];
                    }
                    if self.handoff && entry.kind == Kind::Image {
                        return vec![Request::OpenPhoto(entry)];
                    }
                    if self.handoff && crate::shell::books::engine::is_book(&entry.path) {
                        return vec![Request::OpenBook(entry.path)];
                    }
                    if self.handoff && entry.kind == Kind::Audio {
                        return vec![Request::PlayAudio(entry)];
                    }
                    self.open_preview(ui, &entry, t);
                }
            }
            Target::Content | Target::Keyboard => {}
            Target::Share | Target::Markup => {
                if let Page::Preview(p) = self.nav.top() {
                    let path = p.path.clone();
                    return vec![if target == Target::Share { Request::Share(path) } else { Request::Markup(path) }];
                }
            }
        }
        vec![]
    }

    /// Opens the context menu for the pressed item at the finger.
    fn long_press(&mut self, x: f32, y: f32) {
        let Page::Dir(d) = self.nav.top() else { return };
        if self.selecting {
            return;
        }
        let in_trash = d.path == fs::trash_dir();
        let subject = match self.pressed {
            Some(Target::Entry(i)) => Subject::Entry(i),
            Some(Target::Content) => Subject::Background,
            _ => return,
        };
        let mut menu = Menu::new(subject, self.clipboard.is_some(), (x, y));
        if d.path == Path::new(fs::RECENTS) && subject == Subject::Background {
            menu.items.retain(|a| *a != Action::NewFolder);
        }
        if let Subject::Entry(i) = subject {
            if d.entries.get(i).is_some_and(|e| !e.is_dir && archive::is_archive(&e.name)) {
                menu.items.insert(0, Action::Extract);
            }
        }
        if in_trash {
            menu.items = match subject {
                Subject::Entry(_) => menu::trash_actions(),
                Subject::Background => vec![],
            };
        }
        if menu.items.is_empty() {
            return;
        }
        self.menu = Some(menu);
        self.pressed = None;
        self.press = None;
        self.scrolling = false;
        self.swallow = true;
        self.dirty = true;
    }

    /// Performs a menu action.
    fn act(&mut self, ui: &Ui, action: Action, t: f64) -> Vec<Request> {
        let Some(m) = &self.menu else { return vec![] };
        let Page::Dir(d) = self.nav.top() else { return vec![] };
        let dir = d.path.clone();
        let entry = match m.subject {
            Subject::Entry(i) => d.entries.get(i).cloned(),
            Subject::Background => None,
        };
        let mut requests = vec![];
        match (action, entry) {
            (Action::Share, e) => {
                self.menu.as_mut().unwrap().show_share(e.is_some_and(|e| !e.is_dir));
                return vec![];
            }
            (Action::SendNearby, Some(e)) => {
                self.menu = None;
                requests.push(Request::Share(e.path));
            }
            (Action::Rename, Some(e)) => {
                self.alert = Some(Alert::rename(ui, e.path.clone(), &e.name));
                self.show_keyboard(t);
            }
            (Action::Delete, Some(e)) => self.alert = Some(Alert::delete(ui, e.path.clone(), &e.name, e.is_dir)),
            (Action::MoveTo | Action::Restore, Some(e)) => self.open_picker(ui, vec![e.path.clone()], false),
            (Action::Compress, Some(e)) => {
                let into = e.path.parent().map(Path::to_path_buf).unwrap_or(dir);
                let src = vec![e.path.clone()];
                self.spawn_op("compress", move || archive::create(&src, &into, archive::Format::Zip, &mut |_, _| true).map(|_| ()));
            }
            (Action::Extract, Some(e)) => {
                let into = e.path.parent().map(Path::to_path_buf).unwrap_or(dir);
                let src = e.path.clone();
                self.spawn_op("extract", move || archive::extract(&src, &into, &mut |_, _| true).map(|_| ()));
            }
            (Action::Tag, Some(e)) => {
                let cur = self.tags.tags(&e.path).first().copied();
                self.tags.clear(&e.path);
                let next = match cur {
                    None => Some(tags::Tag::ALL[0]),
                    Some(c) => tags::Tag::ALL.get(c as usize + 1).copied(),
                };
                if let Some(n) = next {
                    self.tags.toggle(&e.path, n);
                }
                let _ = self.tags.save(&self.tags_file);
                self.refresh_top();
            }
            (Action::Copy, Some(e)) => self.clipboard = Some(e.path.clone()),
            (Action::GetInfo, Some(e)) => {
                let mut page = PreviewPage::new(ui, &e);
                page.body = Body::Info(PreviewPage::info_rows(ui, self.w, &e, e.kind, self.now, self.offset));
                self.nav.push(Page::Preview(page), t);
            }
            (Action::OpenInTerminal, e) => {
                let at = match e {
                    Some(e) if e.is_dir => e.path,
                    Some(e) => e.path.parent().map(Path::to_path_buf).unwrap_or(dir),
                    None => dir,
                };
                requests.push(Request::OpenTerminal(at));
            }
            (Action::NewFolder, _) => {
                self.alert = Some(Alert::new_folder(ui, dir));
                self.show_keyboard(t);
            }
            (Action::Paste, _) => {
                if let Some(src) = self.clipboard.clone() {
                    self.spawn_op("paste", move || fs::copy_to(&src, &dir).map(|_| ()));
                }
            }
            _ => {}
        }
        self.menu = None;
        requests
    }

    /// Leaves select mode and forgets the marks.
    fn leave_select(&mut self) {
        self.selecting = false;
        self.selected.clear();
    }

    fn open_picker(&mut self, ui: &Ui, srcs: Vec<PathBuf>, copy: bool) {
        let locations = match self.nav.pages().first() {
            Some(Page::Locations(l)) => l.locations.clone(),
            _ => fs::locations(),
        };
        let restoring = srcs.first().is_some_and(|s| s.starts_with(fs::trash_dir()));
        let locations: Vec<Location> = locations.into_iter().filter(|l| l.path != Path::new(fs::RECENTS) && l.path != fs::trash_dir() && !l.path.starts_with(fs::TAG_ROOT)).collect();
        self.picker = Some(Picker {
            srcs,
            copy,
            nav: Nav::new(Page::Locations(LocationsPage::new(ui, locations))),
            cancel: ui.text("Cancel", 17.0, Weight::Regular, icons::BLUE),
            confirm: ui.text(if copy { "Copy" } else if restoring { "Put Back" } else { "Move" }, 17.0, Weight::Semibold, icons::BLUE),
        });
    }

    /// Advances animations, the long press and playback; true when a frame is needed.
    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        if let Some((x, y, t0)) = self.press {
            if t - t0 >= crate::shell::accessibility::long_press_ms(LONG_PRESS_MS) && self.menu.is_none() && self.alert.is_none() && self.picker.is_none() {
                self.long_press(x, y);
            }
        }
        let mut moving = self.nav.tick(t);
        if let Some(p) = &mut self.picker {
            moving |= p.nav.tick(t);
        }
        for out in self.keyboard.tick(t) {
            self.key(ui, out, t);
            self.dirty = true;
        }
        moving |= self.keyboard.animating();
        let bottom = self.bottom(ui);
        let grid = self.grid;
        match self.active_nav_mut().top_mut() {
            Page::Dir(d) => {
                moving |= d.scroll.tick(t);
                d.fit_scroll(ui, grid, bottom);
            }
            Page::Preview(p) => {
                moving |= p.scroll.tick(t);
                p.fit_scroll(ui, bottom);
            }
            Page::Locations(_) => {}
        }
        if let Some(p) = &mut self.playing {
            if let Some(v) = p.view.as_mut() {
                v.set_size(self.w, self.h);
                self.dirty |= v.tick(ui, &mut *self.player, t);
            } else {
                if let Some(f) = self.player.frame() {
                    if p.frame.is_none() {
                        p.overlay = false;
                    }
                    p.frame = Some(f);
                    self.dirty = true;
                }
                let status = self.player.status();
                if status != p.status {
                    p.status_img = ui.text(&ui.fit(&status, 15.0, ui.pt(600.0)), 15.0, Weight::Regular, crate::shell::ui::SECONDARY);
                    p.status = status;
                    self.dirty = true;
                }
            }
        }
        let blinking = self.alert.as_ref().is_some_and(|a| a.field.is_some()) || matches!(self.nav.top(), Page::Dir(d) if d.searching);
        self.dirty |= moving || blinking;
        self.dirty
    }

    /// The Back label reading `text`, rendered once per string.
    fn back_image(&self, ui: &Ui, text: &str) -> Image {
        let fitted = ui.fit(text, 17.0, ui.pt(200.0));
        self.text_cache.borrow_mut().get(ui, &fitted, 17.0, Weight::Regular, icons::BLUE).clone()
    }

    /// Runs `f` on the rendering of `text` in this style, taken from the text cache (rendered once per string and style).
    fn with_text<R>(&self, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, f: impl FnOnce(&Image) -> R) -> R {
        f(self.text_cache.borrow_mut().get(ui, text, pt, weight, color))
    }
}

/// Runs `op` on every path, carrying on after a failure and reporting the first.
fn each(paths: &[PathBuf], op: impl Fn(&Path) -> std::io::Result<()>) -> std::io::Result<()> {
    let mut first = None;
    for p in paths {
        if let Err(e) = op(p) {
            first.get_or_insert(e);
        }
    }
    first.map_or(Ok(()), Err)
}

/// Targets that live inside a scrolling area.
fn scrolls(t: Target) -> bool {
    matches!(t, Target::Entry(_) | Target::Content)
}

/// Lists `page`'s directory on a worker; the result carries `gen` so a
/// listing that was superseded is dropped.
fn spawn_listing(pool: &Pool<Done>, gen: u64, page: &mut DirPage) {
    page.gen = gen;
    page.loading = true;
    let path = page.path.clone();
    pool.spawn(move || {
        let result = fs::list(&path).map_err(|e| e.to_string());
        Done::Listing { gen, available: fs::available(&path), path, result }
    });
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod bench;
