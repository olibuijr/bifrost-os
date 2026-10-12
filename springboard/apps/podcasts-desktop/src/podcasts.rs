//! Podcasts: subscribe to RSS feeds by URL, list episodes and play the MP3
//! enclosures. An episode is downloaded to the app folder, then played by the
//! audio service (`bifrost_app::audio`), so playback continues when the app
//! leaves the foreground or is closed. Nothing plays until the user taps an episode.

use bifrost_app::kit::ui::{SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XS, SPACE_XXL, SPACE_XXS, SPACE_XXXL};
use bifrost_app::kit::ui::{MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT, MAC_TITLE3_PT};
use bifrost_app::kit::ui::{CARD_BG, DESTRUCTIVE, DETAIL_BG, FIELD_BG, LABEL, LABEL_2, LINK, ROW_PRESSED, SELECTION, SEPARATOR_C, TRACK_C, WHITE};
use crate::feed::{self, Show, Store};
use crate::transport;
use bifrost_app::audio::{self, Track};
use bifrost_app::dispatch::{DispatchQueue, Qos};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::prefs_layout::m;
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets;
use bifrost_app::kit::{Event, Surface, TextCache, Ui, Weight};
use springboard::deildu::scroll::Scroller;
use springboard::shell::keyboard::Out;
use springboard::shell::music::radio::Fetch;
use springboard::shell::textfield::TextField;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const ROW: f32 = 60.0;
/// Width of the source-list sidebar, in points.
const SIDEBAR_W: f32 = 220.0;
/// The first row sits under the unified toolbar band and a line for errors (points).
const LIST_TOP: f32 = UNIFIED_BAR_PT + 28.0;
/// Height of the floating mini player capsule (points).
const PLAYER_H: f32 = 56.0;
/// Sidebar rows start under the traffic lights.
const SIDEBAR_SCROLL: f32 = -(UNIFIED_BAR_PT + 4.0);
const SIDE_SHOWS: u32 = 1;
const SIDE_SHOW: u32 = 100;
const BAR_TITLE: u32 = 1;
const BAR_ADD: u32 = 2;
const BAR_REMOVE: u32 = 3;
const BAR_CANCEL: u32 = 4;
const BAR_DONE: u32 = 5;
const BAR_CLOSE: u32 = 6;
const POLL_MS: f64 = 250.0;
const FEED_LIMIT: usize = 8 << 20;
const EPISODE_LIMIT: usize = 160 << 20;
/// An episode arrives in ranges of this size, straight into its `.part` file.
const CHUNK: u64 = 2 << 20;
/// sleep timer choices in minutes; 0 is off
const SLEEPS: [u32; 5] = [0, 5, 15, 30, 60];
const SAVE_EVERY_MS: f64 = 5_000.0;
/// how long a started episode may take to show up as playing in the service
const START_TIMEOUT_MS: f64 = 5_000.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Row(usize),
    Add,
    /// a source-list row
    Side(u32),
    Remove,
    Stop,
    SkipBack,
    SkipFwd,
    Cancel,
    Done,
    List,
    Sheet,
    CloseSheet,
    PlayPause,
    Scrub,
    Speed,
    Sleep,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    Shows,
    Episodes(usize),
}

enum Msg {
    /// the show's title and episodes, parsed on the pool
    Feed(String, Result<(String, Vec<feed::Episode>), String>),
    /// bytes of the episode on disk so far and, when the server said, its size
    Progress(String, u64, Option<u64>),
    Episode(String, Result<PathBuf, String>),
}

/// The results a worker produced, waiting for the app loop to apply them.
type Inbox = Arc<Mutex<VecDeque<Msg>>>;

/// Queues `msg` for the app loop: the push runs on the main queue, so the loop wakes for it and `tick` applies it.
fn deliver(inbox: &Inbox, msg: Msg) {
    let inbox = inbox.clone();
    DispatchQueue::main().async_(move || {
        if let Ok(mut queue) = inbox.lock() {
            queue.push_back(msg);
        }
    });
}

pub struct Podcasts {
    w: i32,
    h: i32,
    store: Store,
    net: Arc<dyn Fetch>,
    inbox: Inbox,
    shows: Vec<Show>,
    view: View,
    confirm_remove: bool,
    field: Option<TextField>,
    field_touch: bool,
    scroll: Scroller,
    scrolling: bool,
    pressed: Option<Target>,
    /// feeds being fetched and the episode being downloaded (by URL)
    loading: Vec<String>,
    downloading: Option<String>,
    /// how far the download got: bytes, and the size when known
    progress: Option<(u64, Option<u64>)>,
    audio: audio::Client,
    /// the episode (by URL) this app started or adopted, and the file the service plays for it
    playing: Option<String>,
    playing_path: Option<String>,
    /// the service reported that file playing; false while the command is in flight
    started: bool,
    started_at: f64,
    /// the file just stopped: not adopted again until the service reports it paused
    released: Option<String>,
    now: f64,
    position_ms: f64,
    duration_ms: f64,
    paused: bool,
    sheet: bool,
    scrubbing: Option<f64>,
    /// the playback rate the speed control shows, one of `transport::RATES` (kept per session; the engine plays at 1x)
    rate: f64,
    sleep: usize,
    sleep_at: Option<f64>,
    resume: Vec<(String, u64)>,
    last_save: f64,
    error: String,
    dirty: bool,
    cache: TextCache,
    cache_style: (u32, u32, bool),
}

impl Podcasts {
    pub fn new(ui: &Ui, w: i32, h: i32, dir: PathBuf, net: Arc<dyn Fetch>, shell_dir: &Path) -> Self {
        let store = Store::new(dir);
        let shows = store.load();
        let resume = store.positions();
        let inbox: Inbox = Arc::new(Mutex::new(VecDeque::new()));
        Self {
            w,
            h,
            store,
            net,
            inbox,
            shows,
            view: View::Shows,
            confirm_remove: false,
            field: None,
            field_touch: false,
            scroll: Scroller::new(),
            scrolling: false,
            pressed: None,
            loading: Vec::new(),
            downloading: None,
            progress: None,
            audio: audio::Client::for_root(shell_dir),
            playing: None,
            playing_path: None,
            started: false,
            started_at: 0.0,
            released: None,
            now: 0.0,
            position_ms: 0.0,
            duration_ms: 0.0,
            paused: false,
            sheet: false,
            scrubbing: None,
            rate: 1.0,
            sleep: 0,
            sleep_at: None,
            resume,
            last_save: 0.0,
            error: String::new(),
            dirty: true,
            cache: TextCache::default(),
            cache_style: (ui.s.to_bits(), ui.text_scale.to_bits(), ui.bold),
        }
    }

    /// The app left the foreground: the episode keeps playing in the service; remember where it is.
    pub fn suspend(&mut self) {
        if let Some(url) = self.playing.clone() {
            self.set_resume(&url, Some(self.position_ms as u64).filter(|&ms| ms >= 5_000));
        }
        self.audio.pump();
    }

    // network -----------------------------------------------------------

    fn fetch_feed(&mut self, url: &str) {
        if self.loading.iter().any(|u| u == url) {
            return;
        }
        self.loading.push(url.to_string());
        let (net, inbox, url, store) = (self.net.clone(), self.inbox.clone(), url.to_string(), self.store.clone());
        // fetched, kept and parsed on the pool: the UI thread only applies the result
        DispatchQueue::global(Qos::Utility).async_(move || {
            let r = net.get(&url, FEED_LIMIT).map_err(|e| e.to_string()).map(|bytes| {
                let xml = String::from_utf8_lossy(&bytes).into_owned();
                store.cache(&url, &xml);
                feed::parse(&xml)
            });
            deliver(&inbox, Msg::Feed(url, r));
        });
    }

    fn download(&mut self, url: &str) {
        let path = self.store.episode_file(url);
        if path.is_file() {
            self.play_file(url, path);
            return;
        }
        if self.downloading.is_some() {
            return;
        }
        self.downloading = Some(url.to_string());
        self.progress = None;
        let (net, inbox, url) = (self.net.clone(), self.inbox.clone(), url.to_string());
        DispatchQueue::global(Qos::Utility).async_(move || {
            let ticks = inbox.clone();
            let name = url.clone();
            let r = download_episode(&*net, &url, &path, |at, total| {
                deliver(&ticks, Msg::Progress(name.clone(), at, total));
            })
            .map(|_| path);
            deliver(&inbox, Msg::Episode(url, r));
        });
    }

    fn next_msg(&self) -> Option<Msg> {
        self.inbox.lock().ok()?.pop_front()
    }

    fn pump(&mut self) {
        while let Some(msg) = self.next_msg() {
            self.dirty = true;
            match msg {
                Msg::Feed(url, r) => {
                    self.loading.retain(|u| *u != url);
                    match r {
                        Ok((title, episodes)) => {
                            if let Some(show) = self.shows.iter_mut().find(|s| s.url == url) {
                                if !title.is_empty() {
                                    show.title = title;
                                }
                                show.episodes = episodes;
                                self.store.save(&self.shows);
                            }
                        }
                        Err(e) => self.error = format!("Cannot load feed: {e}"),
                    }
                }
                Msg::Progress(url, at, total) => {
                    if self.downloading.as_deref() == Some(url.as_str()) {
                        self.progress = Some((at, total));
                    }
                }
                Msg::Episode(url, r) => {
                    if self.downloading.as_deref() == Some(url.as_str()) {
                        self.downloading = None;
                        self.progress = None;
                    }
                    match r {
                        Ok(path) => self.play_file(&url, path),
                        Err(e) => self.error = format!("Cannot download: {e}"),
                    }
                }
            }
        }
    }

    // playback ----------------------------------------------------------

    fn play_file(&mut self, url: &str, path: PathBuf) {
        self.stop_playback();
        let path = path.to_string_lossy().into_owned();
        if !self.audio.play_queue(vec![Track { id: 0, path: path.clone(), url: None }], 0, false, 0) {
            self.error = "Cannot play: the audio service is not running".into();
            return;
        }
        self.playing = Some(url.to_string());
        self.playing_path = Some(path);
        self.started = false;
        self.started_at = self.now;
        self.released = None;
        self.position_ms = 0.0;
        self.duration_ms = 0.0;
        self.paused = false;
        if let Some(&(_, ms)) = self.resume.iter().find(|(u, _)| u == url) {
            self.audio.seek_ms(ms as f64);
            self.position_ms = ms as f64;
        }
    }

    fn stop_playback(&mut self) {
        self.end_playback(false, true);
    }

    /// Forgets the episode; the position is kept for resuming unless it finished. `stop` also
    /// silences the service (not when another app has taken the speaker).
    fn end_playback(&mut self, finished: bool, stop: bool) {
        if stop && self.playing.is_some() {
            self.audio.stop();
            self.released = self.playing_path.clone();
        }
        self.playing_path = None;
        self.started = false;
        if let Some(url) = self.playing.take() {
            let ms = if finished || self.position_ms < 5_000.0 { None } else { Some(self.position_ms as u64) };
            self.set_resume(&url, ms);
        }
        self.position_ms = 0.0;
        self.sheet = false;
        self.scrubbing = None;
        self.sleep_at = None;
        self.paused = false;
    }

    /// Follows the service: the episode's position and state, another app taking over, and an
    /// episode that is already playing when the app starts.
    fn sync_audio(&mut self, t: f64) {
        self.dirty |= self.audio.pump();
        self.dirty |= self.audio.tick(t);
        let snap = self.audio.snapshot().clone();
        let theirs = snap.track.as_ref().map(|tr| tr.path.as_str());
        let Some(mine) = self.playing_path.clone() else {
            self.adopt(&snap);
            return;
        };
        if theirs == Some(mine.as_str()) {
            if let Some(e) = &snap.error {
                self.error = format!("Cannot play: {e}");
                self.stop_playback();
            } else if snap.playing {
                self.started = true;
                self.paused = false;
                self.position_ms = snap.position_ms;
                self.duration_ms = snap.duration_ms.unwrap_or(0.0);
            } else if self.started {
                let at_end = self.duration_ms > 0.0 && self.position_ms + 2_500.0 >= self.duration_ms;
                if at_end {
                    self.end_playback(true, false);
                } else {
                    self.paused = true;
                }
            }
        } else if self.started {
            self.end_playback(false, false);
        }
        if self.playing.is_some() && !self.started && t - self.started_at >= START_TIMEOUT_MS {
            self.error = "Cannot play: the audio service did not respond".into();
            self.end_playback(false, false);
        }
    }

    /// Picks up an episode of ours that the service is playing (the app was closed meanwhile).
    fn adopt(&mut self, snap: &audio::Snapshot) {
        let Some(path) = snap.track.as_ref().map(|tr| tr.path.clone()) else {
            self.released = None;
            return;
        };
        if self.released.as_deref() == Some(path.as_str()) {
            if !snap.playing {
                self.released = None;
            }
            return;
        }
        self.released = None;
        // only files of ours are looked up (hashing every episode URL per tick is wasted on others)
        if !snap.playing || !Path::new(&path).starts_with(self.store.dir.join("episodes")) {
            return;
        }
        let known = self.shows.iter().flat_map(|s| s.episodes.iter().map(|e| e.url.as_str())).chain(self.resume.iter().map(|(u, _)| u.as_str()));
        let found = known.into_iter().find(|u| self.store.episode_file(u).to_string_lossy() == path).map(str::to_string);
        if let Some(url) = found {
            self.playing = Some(url);
            self.playing_path = Some(path);
            self.started = true;
            self.paused = false;
            self.position_ms = snap.position_ms;
            self.duration_ms = snap.duration_ms.unwrap_or(0.0);
            self.dirty = true;
        }
    }

    fn set_resume(&mut self, url: &str, ms: Option<u64>) {
        self.resume.retain(|(u, _)| u != url);
        if let Some(ms) = ms {
            self.resume.push((url.to_string(), ms));
        }
        self.store.save_positions(&self.resume);
    }

    fn seek_to(&mut self, ms: f64) {
        if self.playing.is_some() {
            let to = if self.duration_ms > 0.0 { ms.clamp(0.0, self.duration_ms) } else { ms.max(0.0) };
            self.audio.seek_ms(to);
            self.position_ms = to;
        }
    }

    fn sleep_label(&self, t: f64) -> String {
        match self.sleep_at {
            Some(at) => {
                let s = ((at - t) / 1000.0).max(0.0) as u64;
                format!("Sleep {}:{:02}", s / 60, s % 60)
            }
            None => "Sleep Off".to_string(),
        }
    }

    /// Moves back or forward by `interval_s` seconds, clamped by the transport model (`transport::skip_target_ms`).
    fn skip(&mut self, backward: bool, interval_s: f64) {
        let to = transport::skip_target_ms(backward, interval_s, self.position_ms, self.duration_ms);
        self.seek_to(to);
    }

    // geometry ----------------------------------------------------------
    // Pixels unless a name says points; the kit widgets (sidebar, toolbar) take points.

    /// The sidebar column width in pixels.
    fn side(&self, ui: &Ui) -> f32 {
        ui.pt(SIDEBAR_W)
    }

    fn content_w(&self, ui: &Ui) -> f32 {
        (self.w as f32 - self.side(ui)).max(0.0)
    }

    fn list_top(&self, ui: &Ui) -> f32 {
        ui.pt(LIST_TOP)
    }

    // Now Playing sheet geometry (in the content column)
    fn art_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        // square artwork: at most 40 % of the window height, never wider than the column
        let side = (self.h as f32 * 0.4).min(self.content_w(ui) - 2.0 * self.margin(ui)).max(0.0);
        (self.side(ui) + (self.content_w(ui) - side) / 2.0, self.list_top(ui), side, side)
    }

    fn scrub_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let a = self.art_rect(ui);
        let m = self.margin(ui);
        (self.side(ui) + m + ui.pt(SPACE_S), a.1 + a.3 + ui.pt(70.0), self.content_w(ui) - 2.0 * m - ui.pt(SPACE_L), ui.pt(44.0))
    }

    /// 0 skip back, 1 play/pause, 2 skip forward, 3 speed, 4 sleep
    fn sheet_button(&self, ui: &Ui, n: usize) -> (f32, f32, f32, f32) {
        let sc = self.scrub_rect(ui);
        let m = self.margin(ui);
        let x0 = self.side(ui) + m;
        let cw = self.content_w(ui) - 2.0 * m;
        let gap = ui.pt(SPACE_M);
        let h = ui.pt(widgets::CAPSULE_PT);
        if n < 3 {
            let w = (cw - 2.0 * gap) / 3.0;
            (x0 + (w + gap) * n as f32, sc.1 + sc.3 + ui.pt(SPACE_XXL), w, h)
        } else {
            let w2 = (cw - gap) / 2.0;
            (x0 + (w2 + gap) * (n - 3) as f32, sc.1 + sc.3 + ui.pt(SPACE_XXL) + h + gap, w2, h)
        }
    }

    // subscriptions -----------------------------------------------------

    fn open_show(&mut self, i: usize) {
        self.view = View::Episodes(i);
        self.confirm_remove = false;
        self.scroll = Scroller::new();
        self.error.clear();
        let url = self.shows[i].url.clone();
        if self.shows[i].episodes.is_empty() {
            if let Some((_, eps)) = self.store.cached(&url) {
                self.shows[i].episodes = eps;
            }
        }
        self.fetch_feed(&url);
    }

    fn subscribe(&mut self, input: &str) {
        let Some(url) = feed::normalize(input) else {
            self.error = "Enter a feed URL".into();
            return;
        };
        let i = match self.shows.iter().position(|s| s.url == url) {
            Some(i) => i,
            None => {
                let title = url.trim_start_matches("https://").trim_start_matches("http://").to_string();
                self.shows.push(Show { url, title, episodes: Vec::new() });
                self.store.save(&self.shows);
                self.shows.len() - 1
            }
        };
        self.open_show(i);
    }

    fn remove(&mut self, i: usize) {
        let playing_here = self.playing.as_deref().is_some_and(|p| self.shows[i].episodes.iter().any(|e| e.url == p));
        if playing_here {
            self.stop_playback();
        }
        let show = self.shows.remove(i);
        self.store.forget(&show.url);
        // the show's downloads go with it (the episode list may be unloaded: the cached feed has it)
        let cached = if show.episodes.is_empty() { self.store.cached(&show.url).map(|c| c.1).unwrap_or_default() } else { Vec::new() };
        let urls: Vec<&str> = show.episodes.iter().chain(&cached).map(|e| e.url.as_str()).collect();
        self.store.forget_episodes(urls.iter().copied());
        self.resume.retain(|(u, _)| !urls.contains(&u.as_str()));
        self.store.save_positions(&self.resume);
        self.store.save(&self.shows);
        self.view = View::Shows;
        self.scroll = Scroller::new();
    }

    // layout ------------------------------------------------------------

    /// Top of the floating mini player (the bottom of the list when nothing plays).
    fn controls_top(&self, ui: &Ui) -> f32 {
        self.h as f32 - if self.playing.is_some() { ui.pt(PLAYER_H + 2.0 * SPACE_M) } else { 0.0 }
    }

    fn margin(&self, ui: &Ui) -> f32 {
        ui.pt(SPACE_XL)
    }

    fn rows(&self) -> usize {
        match self.view {
            View::Shows => self.shows.len(),
            View::Episodes(i) => self.shows.get(i).map_or(0, |s| s.episodes.len()),
        }
    }

    fn row_rect(&self, ui: &Ui, i: usize) -> (f32, f32, f32, f32) {
        let m = self.margin(ui);
        (self.side(ui) + m, self.list_top(ui) + ui.pt(ROW) * i as f32 - self.scroll.offset(), self.content_w(ui) - 2.0 * m, ui.pt(ROW))
    }

    /// The floating mini player capsule (Golden Gate Podcasts and Music).
    fn player_capsule(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let m = self.margin(ui);
        (self.side(ui) + m, self.h as f32 - ui.pt(PLAYER_H + SPACE_M), self.content_w(ui) - 2.0 * m, ui.pt(PLAYER_H))
    }

    /// 0 skip back, 1 stop, 2 skip forward, at the trailing end of the mini player.
    fn player_rect(&self, ui: &Ui, n: usize) -> (f32, f32, f32, f32) {
        let c = self.player_capsule(ui);
        let (w, h, gap) = (ui.pt(64.0), ui.pt(widgets::CAPSULE_PT - 8.0), ui.pt(SPACE_XS));
        let right = c.0 + c.2 - ui.pt(SPACE_M);
        (right - (3 - n) as f32 * (w + gap) + gap, c.1 + (c.3 - h) / 2.0, w, h)
    }

    fn field_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let m = self.margin(ui);
        (self.side(ui) + m + ui.pt(SPACE_L), self.list_top(ui) + ui.pt(SPACE_XL), self.content_w(ui) - 2.0 * m - ui.pt(2.0 * SPACE_L), ui.pt(36.0))
    }

    fn scrub_ms(&self, ui: &Ui, x: f32) -> f64 {
        let r = self.scrub_rect(ui);
        ((x - r.0) / r.2).clamp(0.0, 1.0) as f64 * self.duration_ms
    }

    fn inside((x, y, w, h): (f32, f32, f32, f32), px: f32, py: f32) -> bool {
        px >= x && px < x + w && py >= y && py < y + h
    }

    fn title(&self) -> String {
        if self.field.is_some() {
            "New Podcast".into()
        } else if self.sheet && self.playing.is_some() {
            "Now Playing".into()
        } else {
            match self.view {
                View::Shows => "Podcasts".into(),
                View::Episodes(i) => self.shows.get(i).map_or(String::new(), |s| s.title.clone()),
            }
        }
    }

    /// The unified toolbar (`NativeToolbar`): the title, then the actions for the current screen.
    fn bar(&self, ui: &Ui) -> Toolbar {
        let room = (self.w as f32 / ui.s - SIDEBAR_W - 220.0).max(40.0);
        let title_w = (ui.measure(&self.title(), MAC_TITLE3_PT, Weight::Bold) / ui.s + 4.0).min(room);
        let mut items = vec![ToolbarItem::custom(BAR_TITLE, "Title", title_w), ToolbarItem::flexible_space()];
        if self.field.is_some() {
            items.push(ToolbarItem::button(BAR_CANCEL, "Cancel"));
            items.push(ToolbarItem::button(BAR_DONE, "Subscribe"));
        } else if self.sheet && self.playing.is_some() {
            items.push(ToolbarItem::button(BAR_CLOSE, "Done"));
        } else {
            if let View::Episodes(_) = self.view {
                let label = if self.confirm_remove { "Confirm Remove" } else { "Remove Show" };
                items.push(ToolbarItem::icon(BAR_REMOVE, label, "trash").selected(self.confirm_remove));
            }
            items.push(ToolbarItem::icon(BAR_ADD, "Add Podcast", "plus"));
        }
        Toolbar::new(items)
    }

    /// The toolbar rectangle in points.
    fn bar_bounds(&self, ui: &Ui) -> Rect {
        let x = SIDEBAR_W + widgets::TOOLBAR_GAP_PT + SPACE_XS;
        let h = widgets::CAPSULE_PT;
        Rect { x, y: (UNIFIED_BAR_PT - h) / 2.0, w: (self.w as f32 / ui.s - x - SPACE_L).max(0.0), h }
    }

    /// Lays the toolbar out as `widgets::toolbar` does, for hit-testing and controls.
    fn bar_layout(&self, ui: &Ui) -> ToolbarLayout {
        let width = |it: &ToolbarItem| {
            if it.symbol.is_some() { widgets::CAPSULE_PT } else { (ui.measure(&it.label, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0) + 2.0 * SPACE_S * 1.5).max(widgets::CAPSULE_PT) }
        };
        self.bar(ui).layout_with(self.bar_bounds(ui), width, widgets::CAPSULE_PT, widgets::TOOLBAR_GAP_PT)
    }

    /// The source list: the library, then one row per subscription.
    fn sidebar(&self) -> SourceList {
        let mut items = vec![SidebarItem::header("Library"), SidebarItem::row(SIDE_SHOWS, "Shows").with_symbol("list.bullet").with_count(self.shows.len() as u32)];
        if !self.shows.is_empty() {
            items.push(SidebarItem::header("Subscriptions"));
            for (i, sh) in self.shows.iter().enumerate() {
                items.push(SidebarItem::row(SIDE_SHOW + i as u32, &sh.title).with_symbol("globe"));
            }
        }
        let mut list = SourceList::new(items, 32.0, 30.0);
        list.set_selected(Some(match self.view {
            View::Shows => SIDE_SHOWS,
            View::Episodes(i) => SIDE_SHOW + i as u32,
        }));
        list
    }

    fn sidebar_bounds(&self, ui: &Ui) -> Rect {
        Rect { x: 0.0, y: 0.0, w: SIDEBAR_W, h: self.h as f32 / ui.s }
    }

    /// The toolbar buttons as semantic controls, so the shell does not treat them as the window's drag band.
    pub fn controls(&self, ui: &Ui) -> Vec<Node> {
        let bar = self.bar(ui);
        let placed = self.bar_layout(ui);
        placed
            .visible
            .iter()
            .filter(|(id, _)| *id != BAR_TITLE)
            .filter_map(|(id, r)| bar.items.iter().find(|i| i.id == *id).map(|it| Node { id: *id as u64, role: Role::Button, label: it.label.clone(), value: String::new(), bounds: *r, enabled: it.enabled }))
            .collect()
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let (px, py) = (x / ui.s, y / ui.s);
        if x < self.side(ui) {
            return self.sidebar().hit(self.sidebar_bounds(ui), SIDEBAR_SCROLL, px, py).map(Target::Side);
        }
        if py < UNIFIED_BAR_PT {
            return match self.bar_layout(ui).hit(px, py)? {
                BAR_ADD => Some(Target::Add),
                BAR_REMOVE => Some(Target::Remove),
                BAR_CANCEL => Some(Target::Cancel),
                BAR_DONE => Some(Target::Done),
                BAR_CLOSE => Some(Target::CloseSheet),
                _ => None,
            };
        }
        if self.field.is_some() {
            return None;
        }
        if self.sheet && self.playing.is_some() {
            let sc = self.scrub_rect(ui);
            if Self::inside((sc.0 - ui.pt(SPACE_M), sc.1, sc.2 + ui.pt(SPACE_XL), sc.3), x, y) {
                return Some(Target::Scrub);
            }
            let targets = [Target::SkipBack, Target::PlayPause, Target::SkipFwd, Target::Speed, Target::Sleep];
            return targets.into_iter().enumerate().find(|&(n, _)| Self::inside(self.sheet_button(ui, n), x, y)).map(|(_, t)| t);
        }
        if self.playing.is_some() && Self::inside(self.player_capsule(ui), x, y) {
            for (n, target) in [Target::SkipBack, Target::Stop, Target::SkipFwd].into_iter().enumerate() {
                if Self::inside(self.player_rect(ui, n), x, y) {
                    return Some(target);
                }
            }
            return Some(Target::Sheet);
        }
        if y >= self.list_top(ui) && y < self.controls_top(ui) {
            for i in 0..self.rows() {
                if Self::inside(self.row_rect(ui, i), x, y) {
                    return Some(Target::Row(i));
                }
            }
            return Some(Target::List);
        }
        None
    }

    fn activate(&mut self, ui: &Ui, target: Target, t: f64) {
        if target != Target::Remove {
            self.confirm_remove = false;
        }
        match target {
            Target::Add => {
                self.error.clear();
                self.sheet = false;
                self.field = Some(TextField::new(ui, "https://example.com/feed.xml", MAC_BODY_PT, false, 512));
            }
            Target::Cancel => {
                self.field = None;
            }
            Target::Done => self.finish_add(),
            Target::Side(id) => {
                self.field = None;
                self.sheet = false;
                if id == SIDE_SHOWS {
                    self.view = View::Shows;
                    self.scroll = Scroller::new();
                } else if let Some(i) = id.checked_sub(SIDE_SHOW).map(|i| i as usize).filter(|&i| i < self.shows.len()) {
                    self.open_show(i);
                }
            }
            Target::Remove => {
                if let View::Episodes(i) = self.view {
                    if self.confirm_remove {
                        self.remove(i);
                    } else {
                        self.confirm_remove = true;
                    }
                }
            }
            Target::Row(i) => match self.view {
                View::Shows => self.open_show(i),
                View::Episodes(s) => {
                    if let Some(url) = self.shows[s].episodes.get(i).map(|e| e.url.clone()) {
                        self.error.clear();
                        self.download(&url);
                    }
                }
            },
            Target::Stop => self.stop_playback(),
            Target::SkipBack => self.skip(true, if self.sheet { transport::SHEET_BACK_S } else { transport::PLAYER_SKIP_S }),
            Target::Sheet => self.sheet = true,
            Target::CloseSheet => self.sheet = false,
            Target::Scrub => {}
            Target::PlayPause => {
                if self.playing.is_some() {
                    self.audio.toggle();
                    self.paused = !self.paused;
                }
            }
            Target::Speed => self.rate = transport::next_rate(self.rate),
            Target::Sleep => {
                self.sleep = (self.sleep + 1) % SLEEPS.len();
                self.sleep_at = (SLEEPS[self.sleep] > 0).then(|| t + SLEEPS[self.sleep] as f64 * 60_000.0);
            }
            Target::SkipFwd => self.skip(false, transport::PLAYER_SKIP_S),
            Target::List => {}
        }
        self.dirty = true;
    }

    fn finish_add(&mut self) {
        let text = self.field.as_ref().map(|f| f.text().to_string()).unwrap_or_default();
        self.field = None;
        self.subscribe(&text);
    }

    pub fn key(&mut self, ui: &Ui, out: Out, t: f64) {
        if self.field.is_some() {
            if matches!(out, Out::Hide) {
                self.activate(ui, Target::Cancel, t);
            } else if matches!(out, Out::Enter) {
                self.finish_add();
            } else if let Some(f) = &mut self.field {
                f.edit(ui, &out, t);
            }
        } else {
            // transport keys, the in-app stand-in for the system's remote commands: space plays or pauses, arrows skip
            match &out {
                Out::Text(s) if s == " " => self.activate(ui, Target::PlayPause, t),
                Out::Named("Left") => self.skip(true, transport::PLAYER_SKIP_S),
                Out::Named("Right") => self.skip(false, transport::PLAYER_SKIP_S),
                _ => {}
            }
        }
        self.dirty = true;
    }

    pub fn touch(&mut self, ui: &Ui, event: Event, t: f64) {
        let (x, y) = match event {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            _ => return,
        };
        match event {
            Event::Down(..) => {
                self.dirty = true;
                if self.field.is_some() {
                    let (frame, w) = (self.field_rect(ui), self.w as f32);
                    if let Some(f) = &mut self.field {
                        if f.touch(ui, event, frame, w, t) {
                            self.field_touch = true;
                            return;
                        }
                    }
                }
                self.pressed = self.hit(ui, x, y);
                if self.pressed == Some(Target::Scrub) {
                    self.scrubbing = Some(self.scrub_ms(ui, x));
                    return;
                }
                if matches!(self.pressed, Some(Target::Row(_) | Target::List)) {
                    self.scrolling = true;
                    self.scroll.down(y, t);
                }
            }
            Event::Move(..) => {
                if self.field_touch {
                    let (frame, w) = (self.field_rect(ui), self.w as f32);
                    if let Some(f) = &mut self.field {
                        f.touch(ui, event, frame, w, t);
                    }
                    self.dirty = true;
                } else if self.scrubbing.is_some() {
                    self.scrubbing = Some(self.scrub_ms(ui, x));
                    self.dirty = true;
                } else if self.scrolling {
                    self.scroll.moved(y, t);
                    if self.scroll.dragging() {
                        self.pressed = None;
                    }
                    self.dirty = true;
                } else if self.pressed.is_some() && self.hit(ui, x, y) != self.pressed {
                    self.pressed = None;
                    self.dirty = true;
                }
            }
            Event::Up(..) => {
                self.dirty = true;
                if std::mem::take(&mut self.field_touch) {
                    let (frame, w) = (self.field_rect(ui), self.w as f32);
                    if let Some(f) = &mut self.field {
                        f.touch(ui, event, frame, w, t);
                    }
                    return;
                }
                if self.scrubbing.take().is_some() {
                    self.pressed = None;
                    let ms = self.scrub_ms(ui, x);
                    self.seek_to(ms);
                    return;
                }
                let tap = if std::mem::take(&mut self.scrolling) { self.scroll.up(y, t) } else { true };
                let pressed = self.pressed.take();
                if tap {
                    if let Some(target) = pressed.filter(|&p| self.hit(ui, x, y) == Some(p)) {
                        self.activate(ui, target, t);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        self.now = t;
        self.pump();
        self.scroll.set_max((self.list_top(ui) + ui.pt(ROW * self.rows() as f32) - self.controls_top(ui)).max(0.0));
        self.dirty |= self.scroll.tick(t);
        if let Some(f) = &mut self.field {
            self.dirty |= f.tick(ui, t);
            if f.focused {
                self.dirty = true;
            }
        }
        self.sync_audio(t);
        if self.playing.is_some() {
            if self.started && !self.paused && t - self.last_save >= SAVE_EVERY_MS {
                self.last_save = t;
                if let Some(url) = self.playing.clone() {
                    self.set_resume(&url, Some(self.position_ms as u64));
                }
            }
            if self.sleep_at.is_some_and(|at| t >= at) {
                self.sleep = 0;
                self.stop_playback();
            }
            self.dirty = true;
        }
        self.dirty
    }

    pub fn next_wake(&self, t: f64) -> Option<f64> {
        // a playback clock only: downloads and feeds arrive through `deliver` (a main-queue wake), not by polling
        let mut due = self.playing.is_some().then_some(t + POLL_MS);
        // the service socket is only read between frames: once a second while idle, to notice an episode playing
        let listening = if self.audio.connected() { Some(t + 1000.0) } else { self.audio.next_wake(t) };
        due = [due, listening].into_iter().flatten().reduce(f64::min);
        for wake in [self.field.as_ref().and_then(|f| f.next_wake(t)), self.scroll.animating().then_some(t)] {
            if let Some(w) = wake {
                due = Some(due.map_or(w, |d| d.min(w)));
            }
        }
        due
    }

    pub fn set_size(&mut self, w: i32, h: i32) {
        if (self.w, self.h) != (w, h) {
            self.w = w;
            self.h = h;
            self.dirty = true;
        }
    }

    // drawing -----------------------------------------------------------

    fn text(&mut self, s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, x: f32, y: f32) {
        let image = self.cache.get(ui, text, pt, weight, color);
        s.blit(image, x as i32, y as i32, 255, 255);
    }

    fn centred(&mut self, s: &mut Surface, ui: &Ui, text: &str, pt: f32, color: u32, (x, y, w, h): (f32, f32, f32, f32)) {
        let image = self.cache.get(ui, text, pt, Weight::Regular, color);
        s.blit(image, (x + (w - image.w as f32) / 2.0) as i32, (y + (h - image.h as f32) / 2.0) as i32, 255, 255);
    }

    /// A push button: a capsule plate with tinted text (`NativeButton`, rounded bezel).
    fn button(&mut self, s: &mut Surface, ui: &Ui, target: Target, label: &str, color: u32, r: (f32, f32, f32, f32)) {
        let fill = if self.pressed == Some(target) { ROW_PRESSED.now() } else { FIELD_BG.now() };
        s.fill_rrect(r.0, r.1, r.2, r.3, r.3 / 2.0, fill);
        self.centred(s, ui, label, MAC_BODY_PT, color, r);
    }

    fn draw_sheet(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        let url = self.playing.clone().unwrap_or_default();
        let found = self.shows.iter().find_map(|sh| sh.episodes.iter().find(|e| e.url == url).map(|e| (sh.title.clone(), e.title.clone())));
        let (show, title) = found.unwrap_or_default();
        let (x0, cw) = (self.side(ui), self.content_w(ui));
        let m = self.margin(ui);
        // artwork: the Podcasts mark on a rounded plate
        let a = self.art_rect(ui);
        let side = a.2 as usize;
        let art = crate::icon::icon(side);
        s.fill_rrect(a.0, a.1, a.2, a.3, ui.pt(SPACE_L), CARD_BG.now());
        s.blit(&art, a.0 as i32, a.1 as i32, 255, 255);
        let title = ui.fit(&title, MAC_TITLE3_PT, cw - 2.0 * m);
        self.centred(s, ui, &title, MAC_TITLE3_PT, LABEL.now(), (x0, a.1 + a.3 + ui.pt(SPACE_S), cw, ui.pt(30.0)));
        let show = ui.fit(&show, MAC_SUBHEAD_PT, cw - 2.0 * m);
        self.centred(s, ui, &show, MAC_SUBHEAD_PT, LABEL_2.now(), (x0, a.1 + a.3 + ui.pt(36.0), cw, ui.pt(SPACE_XXL)));
        // scrubber
        let sc = self.scrub_rect(ui);
        let pos = self.scrubbing.unwrap_or(self.position_ms);
        let frac = if self.duration_ms > 0.0 { (pos / self.duration_ms).clamp(0.0, 1.0) as f32 } else { 0.0 };
        let ty = sc.1 + ui.pt(SPACE_M);
        s.fill_rrect(sc.0, ty, sc.2, ui.pt(SPACE_XS), ui.pt(SPACE_XXS), TRACK_C.now());
        s.fill_rrect(sc.0, ty, (sc.2 * frac).max(ui.pt(SPACE_XS)), ui.pt(SPACE_XS), ui.pt(SPACE_XXS), LINK);
        let knob = if self.scrubbing.is_some() { 20.0 } else { 12.0 };
        s.fill_rrect(sc.0 + sc.2 * frac - ui.pt(knob) / 2.0, ty + ui.pt(SPACE_XXS) - ui.pt(knob) / 2.0, ui.pt(knob), ui.pt(knob), ui.pt(knob) / 2.0, LINK);
        let clock = springboard::shell::music::nowplaying::clock;
        let left = clock(pos);
        let right = if self.duration_ms > 0.0 { format!("-{}", clock((self.duration_ms - pos).max(0.0))) } else { String::new() };
        self.text(s, ui, &left, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), sc.0, ty + ui.pt(SPACE_L));
        let rw = ui.measure(&right, MAC_FOOTNOTE_PT, Weight::Regular);
        self.text(s, ui, &right, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), sc.0 + sc.2 - rw, ty + ui.pt(SPACE_L));
        self.button(s, ui, Target::SkipBack, "Back 15", LINK, self.sheet_button(ui, 0));
        self.button(s, ui, Target::PlayPause, if self.paused { "Play" } else { "Pause" }, LINK, self.sheet_button(ui, 1));
        self.button(s, ui, Target::SkipFwd, "Fwd 30", LINK, self.sheet_button(ui, 2));
        self.button(s, ui, Target::Speed, &transport::speed_label(self.rate), LINK, self.sheet_button(ui, 3));
        let label = self.sleep_label(t);
        self.button(s, ui, Target::Sleep, &label, LINK, self.sheet_button(ui, 4));
    }

    fn draw_rows(&mut self, s: &mut Surface, ui: &Ui) {
        let x0 = self.side(ui);
        let cw = self.content_w(ui);
        let bottom = self.controls_top(ui);
        let empty = match self.view {
            View::Shows => Some("No Podcasts. Click Add and paste a feed URL."),
            View::Episodes(_) => Some("No Episodes"),
        }
        .filter(|_| self.rows() == 0);
        if let Some(msg) = empty {
            let msg = if self.loading.is_empty() { msg } else { "Loading..." };
            self.centred(s, ui, msg, MAC_TITLE3_PT, LABEL_2.now(), (x0, self.list_top(ui), cw, (bottom - self.list_top(ui)).max(0.0)));
        }
        for i in 0..self.rows() {
            let r = self.row_rect(ui, i);
            if r.1 + r.3 < self.list_top(ui) || r.1 >= bottom {
                continue;
            }
            let (title, sub, active) = match self.view {
                View::Shows => {
                    let sh = &self.shows[i];
                    (sh.title.clone(), format!("{} episodes", sh.episodes.len()), false)
                }
                View::Episodes(si) => {
                    let e = &self.shows[si].episodes[i];
                    let mut sub = e.date.clone();
                    if !e.duration.is_empty() {
                        sub = format!("{sub}  {}", e.duration);
                    }
                    if self.downloading.as_deref() == Some(e.url.as_str()) {
                        sub = match self.progress {
                            Some((at, Some(total))) if total > 0 => format!("{sub}  Downloading {}%", at * 100 / total),
                            Some((at, _)) => format!("{sub}  Downloading {} MB", at >> 20),
                            None => format!("{sub}  Downloading..."),
                        };
                    } else if self.store.downloaded(&e.url) {
                        sub = format!("{sub}  Downloaded");
                    }
                    (e.title.clone(), sub, self.playing.as_deref() == Some(e.url.as_str()))
                }
            };
            // the playing episode sits on the accent selection, as in Golden Gate Podcasts
            let ink = if active { WHITE } else { LABEL.now() };
            if active {
                s.fill_rrect(r.0, r.1.max(0.0), r.2, r.3, ui.pt(m::SELECTION_RADIUS), SELECTION.now());
            } else if self.pressed == Some(Target::Row(i)) {
                s.fill_rrect(r.0, r.1.max(0.0), r.2, r.3, ui.pt(m::SELECTION_RADIUS), ROW_PRESSED.now());
            } else if i + 1 < self.rows() {
                // hairline inset to the text start, none under the last row
                s.fill_rect((r.0 + ui.pt(SPACE_L)) as usize, (r.1 + r.3 - 1.0).max(0.0) as usize, (r.2 - ui.pt(SPACE_L)) as usize, 1, SEPARATOR_C.now());
            }
            let title = ui.fit(&title, MAC_BODY_PT, r.2 - ui.pt(SPACE_XXXL));
            self.text(s, ui, &title, MAC_BODY_PT, Weight::Semibold, ink, r.0 + ui.pt(SPACE_L), r.1 + ui.pt(SPACE_M));
            let sub = ui.fit(&sub, MAC_SUBHEAD_PT, r.2 - ui.pt(SPACE_XXXL));
            let sub_ink = if active { WHITE } else { LABEL_2.now() };
            self.text(s, ui, &sub, MAC_SUBHEAD_PT, Weight::Regular, sub_ink, r.0 + ui.pt(SPACE_L), r.1 + ui.pt(38.0));
        }
    }

    fn draw_player(&mut self, s: &mut Surface, ui: &Ui) {
        let Some(url) = self.playing.clone() else { return };
        let c = self.player_capsule(ui);
        widgets::capsule(s, ui, Rect { x: c.0 / ui.s, y: c.1 / ui.s, w: c.2 / ui.s, h: c.3 / ui.s });
        let secs = (self.position_ms / 1000.0) as u64;
        let now = format!("{}:{:02}", secs / 60, secs % 60);
        let title = self.shows.iter().flat_map(|s| s.episodes.iter()).find(|e| e.url == url).map_or(String::new(), |e| e.title.clone());
        let room = self.player_rect(ui, 0).0 - c.0 - ui.pt(2.0 * SPACE_L);
        let title = ui.fit(&title, MAC_BODY_PT, room);
        let x = c.0 + ui.pt(SPACE_L) + ui.pt(SPACE_XS);
        self.text(s, ui, &title, MAC_BODY_PT, Weight::Semibold, LABEL.now(), x, c.1 + ui.pt(SPACE_S + SPACE_XXS));
        self.text(s, ui, &now, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), x, c.1 + ui.pt(SPACE_S + SPACE_XXS + 18.0));
        self.button(s, ui, Target::SkipBack, "-30s", LINK, self.player_rect(ui, 0));
        self.button(s, ui, Target::Stop, "Stop", DESTRUCTIVE, self.player_rect(ui, 1));
        self.button(s, ui, Target::SkipFwd, "+30s", LINK, self.player_rect(ui, 2));
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        self.dirty = false;
        let style = (ui.s.to_bits(), ui.text_scale.to_bits(), ui.bold);
        if style != self.cache_style {
            self.cache.clear();
            self.cache_style = style;
        }
        if self.cache.len() > 256 {
            self.cache.clear();
        }
        s.fill_rect(0, 0, self.w as usize, self.h as usize, DETAIL_BG.now());
        let x0 = self.side(ui);
        let cw = self.content_w(ui);
        let m = self.margin(ui);
        if self.field.is_some() {
            let f = self.field_rect(ui);
            self.text(s, ui, "Feed URL", MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), f.0, f.1 - ui.pt(SPACE_L + SPACE_XS));
            let pad = ui.pt(SPACE_S);
            // search fields are capsules (CLAUDE.md, Radii)
            s.fill_rrect(f.0 - pad, f.1, f.2 + 2.0 * pad, f.3, f.3 / 2.0, FIELD_BG.now());
            if let Some(field) = &self.field {
                field.draw(s, ui, f, t);
            }
        } else if self.sheet && self.playing.is_some() {
            self.draw_sheet(s, ui, t);
        } else {
            self.draw_rows(s, ui);
            self.draw_player(s, ui);
        }
        if !self.error.is_empty() {
            let msg = ui.fit(&self.error, MAC_FOOTNOTE_PT, cw - 2.0 * m);
            self.centred(s, ui, &msg, MAC_FOOTNOTE_PT, DESTRUCTIVE, (x0, ui.pt(UNIFIED_BAR_PT), cw, ui.pt(LIST_TOP - UNIFIED_BAR_PT)));
        }
        // the band and sidebar repaint over any row that scrolled under them
        let pressed_side = match self.pressed {
            Some(Target::Side(id)) => Some(id),
            _ => None,
        };
        widgets::source_list(s, ui, self.sidebar_bounds(ui), &self.sidebar(), SIDEBAR_SCROLL, pressed_side);
        s.fill_rect((x0 - 1.0).max(0.0) as usize, 0, 1, self.h as usize, SEPARATOR_C.now());
        let w_pt = self.w as f32 / ui.s;
        s.fill_rect(x0 as usize, 0, cw as usize, ui.pt(UNIFIED_BAR_PT) as usize, DETAIL_BG.now());
        widgets::toolbar_band(s, ui, Rect { x: SIDEBAR_W, y: 0.0, w: w_pt - SIDEBAR_W, h: UNIFIED_BAR_PT });
        let pressed_bar = match self.pressed {
            Some(Target::Add) => Some(BAR_ADD),
            Some(Target::Remove) => Some(BAR_REMOVE),
            Some(Target::Cancel) => Some(BAR_CANCEL),
            Some(Target::Done) => Some(BAR_DONE),
            Some(Target::CloseSheet) => Some(BAR_CLOSE),
            _ => None,
        };
        let placed = widgets::toolbar(s, ui, self.bar_bounds(ui), &self.bar(ui), pressed_bar);
        if let Some(r) = placed.rect_of(BAR_TITLE) {
            let title = ui.fit(&self.title(), MAC_TITLE3_PT, ui.pt(r.w));
            let im = self.cache.get(ui, &title, MAC_TITLE3_PT, Weight::Bold, LABEL.now());
            s.blit(im, ui.pt(r.x) as i32, (ui.pt(r.y + r.h / 2.0) as i32) - im.h as i32 / 2, 255, 255);
        }
    }
}

/// Streams `url` into `file` in ranges, so an episode never has to fit in memory;
/// `progress` hears the bytes written so far and the size when the server gave it.
/// A fetcher without ranges downloads the whole body at once.
fn stream_to(net: &dyn Fetch, url: &str, file: &mut std::fs::File, mut progress: impl FnMut(u64, Option<u64>)) -> io::Result<()> {
    let mut at = 0u64;
    loop {
        let chunk = match net.get_range(url, at, CHUNK) {
            Err(e) if at == 0 && e.kind() == io::ErrorKind::Unsupported => {
                let bytes = net.get(url, EPISODE_LIMIT)?;
                file.write_all(&bytes)?;
                progress(bytes.len() as u64, Some(bytes.len() as u64));
                return Ok(());
            }
            r => r?,
        };
        if chunk.bytes.is_empty() {
            return Err(io::Error::other("empty range answer"));
        }
        at += chunk.bytes.len() as u64;
        if at > EPISODE_LIMIT as u64 {
            return Err(io::Error::other("episode too large"));
        }
        file.write_all(&chunk.bytes)?;
        progress(at, chunk.total);
        if chunk.total.map_or((chunk.bytes.len() as u64) < CHUNK, |total| at >= total) {
            return Ok(());
        }
    }
}

/// Downloads an episode to `path` through a `.part` file that is renamed when complete
/// and removed when the download fails.
fn download_episode(net: &dyn Fetch, url: &str, path: &Path, progress: impl FnMut(u64, Option<u64>)) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let part = path.with_extension("part");
    let result = std::fs::File::create(&part).and_then(|mut f| stream_to(net, url, &mut f, progress).and_then(|_| f.sync_all())).and_then(|_| std::fs::rename(&part, path));
    if let Err(e) = result {
        let _ = std::fs::remove_file(&part);
        return Err(e.to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use springboard::shell::music::radio::Chunk;

    /// A server that honours ranges over `data`.
    struct Ranged(Vec<u8>);

    impl Fetch for Ranged {
        fn get(&self, _: &str, _: usize) -> io::Result<Vec<u8>> {
            Err(io::Error::other("the whole file must not be fetched"))
        }
        fn get_range(&self, _: &str, start: u64, len: u64) -> io::Result<Chunk> {
            let end = (start + len).min(self.0.len() as u64) as usize;
            Ok(Chunk { bytes: self.0[start as usize..end].to_vec(), total: Some(self.0.len() as u64) })
        }
    }

    /// A fetcher with no range support.
    struct Whole(Vec<u8>);

    impl Fetch for Whole {
        fn get(&self, _: &str, _: usize) -> io::Result<Vec<u8>> {
            Ok(self.0.clone())
        }
    }

    struct Broken;

    impl Fetch for Broken {
        fn get(&self, _: &str, _: usize) -> io::Result<Vec<u8>> {
            Err(io::Error::other("offline"))
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn an_episode_streams_in_ranges_with_progress() {
        let dir = scratch("stream");
        let data: Vec<u8> = (0..(CHUNK * 2 + 100)).map(|i| i as u8).collect();
        let path = dir.join("episodes/a.mp3");
        let mut seen = Vec::new();
        download_episode(&Ranged(data.clone()), "https://x.test/a.mp3", &path, |at, total| seen.push((at, total))).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), data);
        assert!(!path.with_extension("part").exists());
        let size = Some(data.len() as u64);
        assert_eq!(seen, [(CHUNK, size), (CHUNK * 2, size), (CHUNK * 2 + 100, size)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_fetcher_without_ranges_falls_back_to_the_whole_body() {
        let dir = scratch("whole");
        let path = dir.join("b.mp3");
        download_episode(&Whole(b"mp3 bytes".to_vec()), "u", &path, |_, _| {}).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"mp3 bytes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_download_leaves_no_part_file_and_no_episode() {
        let dir = scratch("fail");
        let path = dir.join("c.mp3");
        let err = download_episode(&Broken, "u", &path, |_, _| {}).unwrap_err();
        assert!(err.contains("offline"), "{err}");
        assert!(!path.exists() && !path.with_extension("part").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
