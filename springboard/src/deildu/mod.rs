//! Deildu: the media library app. Networking, decoding and the torrent
//! engine run on worker threads; the UI only reacts to their results, which
//! arrive through a pipe the shell polls (`fd` / `pump`), like the Terminal.

pub mod art;
pub mod auth;
pub mod detail;
pub mod dns;
pub mod http;
pub mod jobs;
pub mod library;
pub mod login;
pub mod play;
pub mod player_view;
pub mod progress;
pub mod scroll;
pub mod site;
pub mod sources;
#[cfg(test)]
mod tests;

use crate::gfx::{pack, rrect_mask, Image, Surface};
use crate::shell::keyboard::{self, Keyboard, Out};
use crate::shell::ui::{home_zone, Ui};
use crate::shell::Event;
use art::Cache;
use detail::Detail;
use http::{Client, Transport};
use jobs::Pool;
use library::{Library, DETAIL, HERO, POSTER, RADIUS_PT};
use login::{Edit, Login};
use play::{AnyDownload, Download, Player, StubPlayer};
use site::{Details, Fetch, Item, Listings, CATEGORIES};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const BLACK: u32 = pack(0, 0, 0, 255);
pub const BLUE: u32 = pack(10, 132, 255, 255);
pub const RED: u32 = pack(255, 69, 58, 255);
pub const FIELD: u32 = pack(44, 44, 46, 255);
pub const SEGMENT_ON: u32 = pack(99, 99, 102, 255);
/// Where the session, caches and streamed files live.
pub fn data_dir() -> std::path::PathBuf {
    crate::shell::apphost::root().join("deildu")
}
const WORKERS: usize = 3;
const COMPAT_FILE: &str = "compat";
/// Item details kept in memory before the map starts over (each one is re-fetched on demand).
const DETAILS_CACHED: usize = 256;

/// Something a touch can land on, across all screens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    /// the picture itself (toggles the overlay)
    Video,
    Tab(usize),
    Search,
    Clear,
    Hero,
    HeroPlay,
    Card(usize),
    Prev,
    Next,
    /// streaming provider switch (Sources tab)
    Provider,
    /// duration filter chip (Sources tab)
    Chip(usize),
    /// scrollable area with nothing to activate
    Content,
    Back,
    Play,
    Stop,
    Cancel,
    Submit,
    /// streaming quality row (Sources detail)
    SQuality(usize),
    /// streaming preview clip (Sources detail)
    SPreview,
    Field(login::Field),
    Keyboard,
}

pub(crate) fn within(x: f32, y: f32, (rx, ry, rw, rh): (f32, f32, f32, f32)) -> bool {
    x >= rx && x < rx + rw && y >= ry && y < ry + rh
}

/// Greedy word wrap to `max_w` px at `pt` points.
pub fn wrap(ui: &Ui, text: &str, pt: f32, max_w: f32) -> Vec<String> {
    let px = ui.pt(pt);
    let mut lines = vec![];
    let mut line = String::new();
    for word in text.split_whitespace() {
        let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if ui.font.measure(&candidate, px) <= max_w || line.is_empty() {
            line = candidate;
        } else {
            lines.push(std::mem::replace(&mut line, word.to_string()));
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Something that happened in the background the shell should announce;
/// collected by `take_events`.
#[derive(Clone, Debug, PartialEq)]
pub enum DeilduEvent {
    /// enough of a download arrived to start playing it
    Ready { title: String },
}

/// A finished background job.
enum Done {
    Listings { gen: u64, result: Fetch<Listings> },
    Compat { id: String, ok: bool },
    Art { id: String, kind: u8, image: Option<Image>, details: Option<Details> },
    Details { id: String, result: Fetch<Details> },
    Torrent { id: String, result: Result<Vec<u8>, String> },
    Login(Result<(), String>),
    SourcePage { gen: u64, key: String, result: Result<(Vec<sources::Video>, Option<String>), String> },
    SourceEnrich { id: String, video: sources::Video },
    SourcePrefetch { key: String, result: Result<(Vec<sources::Video>, Option<String>), String> },
    SourceArt { id: String, kind: u8, image: Option<Image> },
}

struct Playing {
    download: AnyDownload,
    title: String,
    screen: progress::Screen,
    opened: bool,
    /// the picture on screen, once the player delivers one
    frame: Option<Image>,
    /// progress/Stop overlay over the picture; a tap on the video toggles it
    overlay: bool,
    status: String,
    /// the file's name, under the title in the player
    file_name: String,
    /// the fullscreen player, from the moment the decoder opened the file
    view: Option<player_view::PlayerView>,
    /// LAN TV server and its URL while playing (mpv/AirPlay on the same net)
    serve: Option<deildu_rust::serve::Server>,
    serve_url: Option<String>,
}

pub struct Deildu {
    w: i32,
    h: i32,
    dir: PathBuf,
    client: Arc<Client>,
    pool: Pool<Done>,
    art: Cache,
    library: Library,
    detail: Option<Detail>,
    source_detail: Option<sources::Detail>,
    sources: sources::State,
    playing: Option<Playing>,
    login: Option<Login>,
    player: Box<dyn Player>,
    keyboard: Keyboard,
    kb_touch: bool,
    /// the current touch is held by a scroller
    scrolling: bool,
    gen: u64,
    details: HashMap<String, Details>,
    compat: HashMap<String, bool>,
    art_pending: HashSet<(String, u8)>,
    art_failed: HashSet<String>,
    /// source videos still waiting for their enrich pass; one starts as each
    /// finishes so every card on the page gets checked
    enrich_queue: VecDeque<sources::Video>,
    pressed: Option<Target>,
    dirty: bool,
    events: Vec<DeilduEvent>,
    /// the title whose playback was last started
    last_played: Option<Item>,
}

impl Deildu {
    /// Lays the app out for a pane of `w` x `h` px (Split View, Slide Over).
    pub fn set_size(&mut self, w: i32, h: i32) {
        self.w = w;
        self.h = h;
    }

    /// `dir` holds the session and caches; `transport` opens connections.
    pub fn new(ui: &Ui, w: i32, h: i32, dir: PathBuf, transport: Arc<dyn Transport>) -> std::io::Result<Self> {
        let client = Arc::new(Client::new(transport, auth::load_session(&dir)));
        let compat = std::fs::read_to_string(dir.join(COMPAT_FILE))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.split_once('\t').map(|(id, v)| (id.to_string(), v == "1")))
            .collect();
        let mut d = Deildu {
            w,
            h,
            dir,
            client,
            pool: Pool::new(WORKERS)?,
            art: Cache::new(art::BUDGET),
            library: Library::new(ui),
            detail: None,
            source_detail: None,
            sources: sources::State::new(),
            playing: None,
            login: None,
            player: Box::new(StubPlayer::default()),
            keyboard: Keyboard::new(w as f32, ui.s, false),
            kb_touch: false,
            scrolling: false,
            gen: 0,
            details: HashMap::new(),
            compat,
            art_pending: HashSet::new(),
            art_failed: HashSet::new(),
            enrich_queue: VecDeque::new(),
            pressed: None,
            dirty: true,
            events: Vec::new(),
            last_played: None,
        };
        d.library.set_provider_label(ui, sources::PROVIDERS[0].name());
        d.load_page(ui);
        Ok(d)
    }

    /// Swaps in a real decoder.
    pub fn set_player(&mut self, player: Box<dyn Player>) {
        self.player = player;
    }

    /// True while a title is downloading or playing (its download runs for the whole playback).
    pub fn active(&self) -> bool {
        self.playing.is_some()
    }

    pub fn needs_frame(&self) -> bool {
        self.dirty
    }

    /// Events since the last call, oldest first (the shell polls this each tick).
    pub fn take_events(&mut self) -> Vec<DeilduEvent> {
        std::mem::take(&mut self.events)
    }

    /// The fd that is readable when a job finished; call `pump` then.
    pub fn fd(&self) -> std::ffi::c_int {
        self.pool.fd()
    }

    /// Every fd to wait on: finished jobs and, while playing, the player.
    pub fn fds(&self) -> Vec<std::ffi::c_int> {
        let mut v = vec![self.pool.fd()];
        if self.playing.is_some() {
            v.extend(self.player.fd());
        }
        v
    }

    /// key=value lines for remote inspection.
    pub fn describe(&self) -> String {
        let screen = if self.login.is_some() {
            "login"
        } else if self.playing.is_some() {
            "playing"
        } else if self.detail.is_some() || self.source_detail.is_some() {
            "detail"
        } else {
            "library"
        };
        let mut out = format!("deildu_screen={screen}\ndeildu_items={}\ndeildu_visible={}\ndeildu_page={}\ndeildu_art={}\ndeildu_art_pending={}\n", self.library.cards.len(), self.library.visible().len(), self.library.page, self.art.len(), self.art_pending.len());
        if self.library.source_mode() {
            out.push_str(&format!("deildu_provider={}\ndeildu_source_titles={}\n", self.sources.provider.name(), self.sources.videos.len()));
        }
        if let Some(p) = &self.playing {
            if let Some(url) = &p.serve_url {
                out.push_str(&format!("deildu_tv_url={url}\n"));
            }
        }
        out
    }

    // --- jobs ---

    fn load_page(&mut self, ui: &Ui) {
        if self.library.source_mode() {
            self.load_source_page(ui);
            return;
        }
        self.gen += 1;
        let gen = self.gen;
        self.library.loading = true;
        self.library.set_status(ui, Some("Loading…"));
        self.library.cards.clear();
        self.art_pending.clear();
        let (client, cat, page, query) = (self.client.clone(), CATEGORIES[self.library.category].1, self.library.page, self.library.query.clone());
        self.pool.spawn(move || Done::Listings { gen, result: site::fetch_listings(&client, cat, page, &query) });
        self.dirty = true;
    }

    fn check_compat(&mut self, item: &Item) {
        let (client, dir, id, url) = (self.client.clone(), self.dir.clone(), item.id.clone(), item.torrent_url.clone());
        let page = item.url.clone();
        self.pool.spawn(move || {
            let ok = torrent_bytes(&client, &dir, &id, &url, &page).and_then(|b| deildu_rust::torrent::Torrent::parse(&b).ok()).is_some_and(|t| play::pick_file(&t).is_some());
            Done::Compat { id, ok }
        });
    }

    // --- streaming sources ---

    /// Loads the current source catalogue page; a cached page paints instantly
    /// while the fetch revalidates it underneath.
    fn load_source_page(&mut self, ui: &Ui) {
        self.enrich_queue.clear();
        self.gen += 1;
        let gen = self.gen;
        self.library.loading = true;
        let (provider, url) = (self.sources.provider, self.sources.location.clone());
        let key = sources::page_key(provider, &url);
        if let Some((videos, next)) = self.sources.cached(&key) {
            self.sources.videos = videos.clone();
            self.sources.next = next.clone();
            self.set_source_cards(ui, &videos);
            self.library.set_status(ui, Some(&format!("{} · {} titles (cached)…", provider.name(), videos.len())));
        } else {
            self.library.set_status(ui, Some("Loading…"));
            self.library.cards.clear();
        }
        self.art_pending.clear();
        let client = self.client.clone();
        self.pool.spawn(move || Done::SourcePage {
            gen,
            key,
            result: sources::fetch_page(&client, provider, &url).map_err(|e| e),
        });
        self.dirty = true;
    }

    /// Pushes catalogue videos into grid cards, marking playability where the
    /// enrich pass already confirmed it.
    fn set_source_cards(&mut self, ui: &Ui, videos: &[sources::Video]) {
        let query = self.library.query.clone();
        let filter = self.sources.filter;
        let known: HashMap<String, bool> = videos
            .iter()
            .map(|v| (v.item.id.clone(), sources::playable(v) && sources::matches(v, &query, filter)))
            .collect();
        let items: Vec<Item> = videos.iter().map(|v| v.item.clone()).collect();
        self.library.set_source_items(ui, self.w, items, self.sources.next.is_some());
        for (id, ok) in known {
            if ok {
                self.library.set_compat(ui, &id, true);
            }
        }
        self.library.chip = filter;
    }

    /// Re-applies search/filter to the loaded catalogue without refetching.
    fn apply_source_filter(&mut self, ui: &Ui) {
        let query = self.library.query.clone();
        let filter = self.sources.filter;
        for v in self.sources.videos.clone() {
            let ok = sources::playable(&v) && sources::matches(&v, &query, filter);
            let current = self.library.cards.iter().find(|c| c.item.id == v.item.id).and_then(|c| c.compat);
            if current != Some(ok) {
                self.library.set_compat(ui, &v.item.id, ok);
            }
        }
        self.library.chip = filter;
        let n = self.library.visible().len();
        self.library.set_status(ui, if n > 0 || self.library.cards.is_empty() { None } else { Some("No titles match the filters") });
        self.dirty = true;
    }

    fn enrich_source(&mut self, video: &sources::Video) {
        let (client, video) = (self.client.clone(), video.clone());
        self.pool.spawn(move || {
            let enriched = sources::enrich(&client, &video);
            Done::SourceEnrich { id: enriched.item.id.clone(), video: enriched }
        });
    }

    fn prefetch_source(&mut self, key: String, provider: sources::Provider, url: String) {
        let client = self.client.clone();
        self.pool.spawn(move || Done::SourcePrefetch {
            key,
            result: sources::fetch_page(&client, provider, &url).map_err(|e| e),
        });
    }

    fn request_source_art(&mut self, ui: &Ui, id: &str, url: &str, kind: u8) {
        let key = (id.to_string(), kind);
        if self.art.contains(id, kind) || self.art_pending.contains(&key) || self.art_failed.contains(id) {
            return;
        }
        self.art_pending.insert(key);
        let (w, h) = match kind {
            library::SOURCE_DETAIL => sources::detail_art_size(ui, self.w),
            HERO => Library::hero_size(ui, self.w),
            _ => self.library.poster_size(ui, self.w),
        };
        let radius = ui.pt(library::RADIUS_PT);
        let (client, id, url) = (self.client.clone(), id.to_string(), url.to_string());
        self.pool.spawn(move || {
            let image = client
                .get(&url, None, 8 * 1024 * 1024)
                .ok()
                .filter(|r| r.status == 200 && !r.body.is_empty())
                .and_then(|r| art::decode(&r.body))
                .map(|img| {
                    let mut c = art::cover(&img, w, h);
                    c.apply_mask(&rrect_mask(w, h, radius));
                    c
                });
            Done::SourceArt { id, kind, image }
        });
    }

    fn request_art(&mut self, ui: &Ui) {
        // Streaming sources carry thumbnail URLs in the catalogue: fetch them
        // directly instead of the detail-page round trip.
        if self.library.source_mode() && self.playing.is_none() {
            if let Some(d) = &self.source_detail {
                if !d.video.thumbnail.is_empty() {
                    let (id, thumb) = (d.video.item.id.clone(), d.video.thumbnail.clone());
                    self.request_source_art(ui, &id, &thumb, library::SOURCE_DETAIL);
                }
            } else {
                let bottom = self.content_bottom(ui);
                let wanted: Vec<(String, String, u8)> = self
                    .library
                    .on_screen(ui, self.w, bottom)
                    .iter()
                    .filter_map(|(ci, kind)| {
                        let id = self.library.cards.get(*ci)?.item.id.clone();
                        let thumb = self.sources.by_id(&id)?.thumbnail.clone();
                        (!thumb.is_empty()).then(|| (id, thumb, *kind))
                    })
                    .collect();
                for (id, thumb, kind) in wanted {
                    self.request_source_art(ui, &id, &thumb, kind);
                }
            }
            return;
        }
        let bottom = self.content_bottom(ui);
        let mut wanted: Vec<(String, String, u8)> = vec![];
        if let Some(d) = &self.detail {
            wanted.push((d.item.id.clone(), d.item.url.clone(), DETAIL));
        } else if self.playing.is_none() {
            for (ci, kind) in self.library.on_screen(ui, self.w, bottom) {
                let item = &self.library.cards[ci].item;
                wanted.push((item.id.clone(), item.url.clone(), kind));
            }
        }
        for (id, url, kind) in wanted {
            let key = (id.clone(), kind);
            if self.art.contains(&id, kind) || self.art_pending.contains(&key) || self.art_failed.contains(&id) {
                continue;
            }
            self.art_pending.insert(key);
            let (w, h) = match kind {
                HERO => Library::hero_size(ui, self.w),
                DETAIL => Detail::poster_size(ui),
                _ => self.library.poster_size(ui, self.w),
            };
            let radius = ui.pt(if kind == POSTER { 10.0 } else { RADIUS_PT });
            let (client, dir) = (self.client.clone(), self.dir.clone());
            let known = self.details.get(&id).cloned();
            self.pool.spawn(move || {
                let mut details = None;
                let bytes = art::disk_read(&dir, &format!("{id}.img")).or_else(|| {
                    let d = match known {
                        Some(d) => d,
                        None => match site::fetch_details(&client, &url) {
                            Fetch::Ok(d) => d,
                            _ => return None,
                        },
                    };
                    let bytes = site::fetch_artwork(&client, d.artwork.as_deref()?).ok()?;
                    art::disk_write(&dir, &format!("{id}.img"), &bytes);
                    details = Some(d);
                    Some(bytes)
                });
                let image = bytes.and_then(|b| art::decode(&b)).map(|img| {
                    let mut c = art::cover(&img, w, h);
                    c.apply_mask(&rrect_mask(w, h, radius));
                    c
                });
                Done::Art { id, kind, image, details }
            });
        }
    }

    fn fetch_details(&mut self, item: &Item) {
        let (client, id, url) = (self.client.clone(), item.id.clone(), item.url.clone());
        self.pool.spawn(move || Done::Details { id, result: site::fetch_details(&client, &url) });
    }

    fn fetch_torrent(&mut self, item: &Item, url: &str) {
        let (client, dir, id, url, page) = (self.client.clone(), self.dir.clone(), item.id.clone(), url.to_string(), item.url.clone());
        self.pool.spawn(move || Done::Torrent { result: torrent_bytes(&client, &dir, &id, &url, &page).ok_or_else(|| "Could not fetch the torrent".to_string()), id });
    }

    fn submit_login(&mut self, ui: &Ui) {
        let Some(l) = &mut self.login else { return };
        if !l.can_submit() {
            return;
        }
        l.set_error(ui, None);
        l.busy = true;
        let (client, user, pass) = (self.client.clone(), l.user.clone(), l.pass.clone());
        self.pool.spawn(move || Done::Login(auth::login(&client, &user, &pass)));
        self.dirty = true;
    }

    fn save_compat(&self) {
        let text: String = self.compat.iter().map(|(id, ok)| format!("{id}\t{}\n", *ok as u8)).collect();
        let _ = std::fs::create_dir_all(&self.dir);
        let _ = std::fs::write(self.dir.join(COMPAT_FILE), text);
    }

    /// Applies finished jobs; true if the screen changed.
    pub fn pump(&mut self, ui: &Ui) -> bool {
        let mut changed = false;
        for done in self.pool.drain() {
            changed = true;
            match done {
                Done::Listings { gen, result } => {
                    if gen != self.gen {
                        continue;
                    }
                    self.library.loading = false;
                    match result {
                        Fetch::Ok(l) => {
                            let compat = &self.compat;
                            self.library.set_items(ui, self.w, l.items, l.next, &|id| compat.get(id).copied());
                            let unknown: Vec<Item> = self.library.cards.iter().filter(|c| c.compat.is_none()).map(|c| c.item.clone()).collect();
                            for item in &unknown {
                                self.check_compat(item);
                            }
                            self.refresh_status(ui);
                        }
                        Fetch::LoginNeeded => {
                            self.library.set_status(ui, Some("Sign in to Deildu"));
                            if self.login.is_none() {
                                self.login = Some(Login::new(ui));
                            }
                        }
                        Fetch::Err(e) => self.library.set_status(ui, Some(&e)),
                    }
                }
                Done::Compat { id, ok } => {
                    self.compat.insert(id.clone(), ok);
                    self.save_compat();
                    self.library.set_compat(ui, &id, ok);
                    self.refresh_status(ui);
                }
                Done::Art { id, kind, image, details } => {
                    self.art_pending.remove(&(id.clone(), kind));
                    if let Some(d) = details {
                        self.apply_details(ui, &id, d);
                    }
                    match image {
                        Some(img) => self.art.insert(&id, kind, img),
                        None => {
                            self.art_failed.insert(id);
                        }
                    }
                }
                Done::Details { id, result } => match result {
                    Fetch::Ok(d) => self.apply_details(ui, &id, d),
                    Fetch::LoginNeeded => {
                        if self.login.is_none() {
                            self.login = Some(Login::new(ui));
                        }
                    }
                    Fetch::Err(e) => {
                        if let Some(d) = self.detail.as_mut().filter(|d| d.item.id == id) {
                            d.set_error(ui, &e);
                        }
                    }
                },
                Done::Torrent { id, result } => {
                    if let Some(d) = self.detail.as_mut().filter(|d| d.item.id == id) {
                        match result.and_then(|b| deildu_rust::torrent::Torrent::parse(&b)) {
                            Ok(t) => {
                                let w = self.w;
                                d.set_torrent(ui, w, t);
                                let ok = d.can_play();
                                self.compat.insert(id, ok);
                                self.save_compat();
                            }
                            Err(e) => d.set_error(ui, &e),
                        }
                    }
                }
                Done::Login(result) => match result {
                    Ok(()) => {
                        self.login = None;
                        if let Err(e) = auth::save_session(&self.dir, &self.client.jar()) {
                            eprintln!("deildu: session not saved: {e}");
                        }
                        self.load_page(ui);
                    }
                    Err(e) => {
                        if let Some(l) = &mut self.login {
                            l.set_error(ui, Some(&e));
                        }
                    }
                },
                Done::SourcePage { gen, key, result } => {
                    if gen != self.gen || !self.library.source_mode() {
                        continue;
                    }
                    self.library.loading = false;
                    match result {
                        Ok((videos, next)) => {
                            let provider = self.sources.provider;
                            self.sources.videos = videos.clone();
                            self.sources.next = next.clone();
                            self.sources.remember(&key, videos.clone(), next.clone());
                            self.set_source_cards(ui, &videos);
                            let query_empty = self.library.query.trim().is_empty();
                            self.library.set_status(
                                ui,
                                Some(&format!(
                                    "{} · {} titles{}{}",
                                    provider.name(),
                                    videos.len(),
                                    if query_empty { "" } else { " · filtered" },
                                    if next.is_some() { "" } else { " · end" }
                                )),
                            );
                            let mut pending = videos.iter().filter(|v| !sources::playable(v)).cloned();
                            for v in pending.by_ref().take(sources::ENRICH_FIRST) {
                                self.enrich_source(&v);
                            }
                            self.enrich_queue = pending.collect();
                            if let Some(url) = next {
                                let following = sources::page_key(provider, &url);
                                self.prefetch_source(following, provider, url);
                            }
                        }
                        Err(e) => self.library.set_status(ui, Some(&e)),
                    }
                }
                Done::SourceEnrich { id, video } => {
                    if !self.library.source_mode() {
                        continue;
                    }
                    if let Some(next) = self.enrich_queue.pop_front() {
                        self.enrich_source(&next);
                    }
                    if let Some(slot) = self.sources.videos.iter_mut().find(|v| v.item.id == id) {
                        *slot = video.clone();
                    }
                    if let Some(d) = self.source_detail.as_mut().filter(|d| d.video.item.id == id) {
                        d.set_video(ui, self.w, video.clone());
                    }
                    let ok = sources::playable(&video)
                        && sources::matches(&video, &self.library.query, self.sources.filter);
                    let current = self.library.cards.iter().find(|c| c.item.id == id).and_then(|c| c.compat);
                    if current != Some(ok) {
                        self.library.set_compat(ui, &id, ok);
                    }
                }
                Done::SourcePrefetch { key, result } => {
                    if let Ok((videos, next)) = result {
                        self.sources.remember(&key, videos, next);
                    }
                }
                Done::SourceArt { id, kind, image } => {
                    self.art_pending.remove(&(id.clone(), kind));
                    match image {
                        Some(img) => self.art.insert(&id, kind, img),
                        None => {
                            self.art_failed.insert(id);
                        }
                    }
                }
            }
        }
        let mut open = false;
        if let Some(p) = &mut self.playing {
            let progress = p.download.progress();
            p.screen.update(ui, &progress);
            if let Some(v) = &mut p.view {
                v.set_download(ui, Some((progress.fraction(), progress.done)));
            }
            open = progress.ready() && !p.opened;
            changed = true;
        }
        if open {
            self.open_player(ui, true);
        }
        if changed {
            self.dirty = true;
        }
        changed
    }

    /// Hands the downloaded file to the player; on success the fullscreen view takes over the screen.
    fn open_player(&mut self, ui: &Ui, first: bool) {
        let Some(p) = &mut self.playing else { return };
        p.opened = true;
        p.view = None;
        if first {
            self.events.push(DeilduEvent::Ready { title: p.title.clone() });
        }
        let mime = play::mime_for(p.download.label());
        self.player.stop();
        self.player.set_viewport(self.w, self.h);
        let text = match self.player.open(p.download.path(), mime) {
            Ok(()) => {
                let mut v = player_view::PlayerView::new(ui, self.w, self.h, &p.title, &p.file_name);
                let progress = p.download.progress();
                v.set_download(ui, Some((progress.fraction(), progress.done)));
                p.view = Some(v);
                self.player.status()
            }
            Err(e) => e.to_string(),
        };
        p.screen.set_player(ui, &text);
        self.dirty = true;
    }

    /// Opens the file again after a playback error.
    fn retry_play(&mut self, ui: &Ui) {
        self.open_player(ui, false);
    }

    /// The player's picture for the GPU (see `PlayerView::video_layer`). `gpu` says whether the
    /// shell can draw pictures at all (the player then stops converting them), `eligible`
    /// whether it composes the coming frame.
    pub fn video_layer(&mut self, gpu: bool, eligible: bool) -> Option<crate::media::VideoLayer> {
        self.player.set_gpu(gpu);
        let up = self.login.is_none() && self.fullscreen();
        let view = self.playing.as_ref().and_then(|p| p.view.as_ref())?;
        view.video_layer(gpu && eligible && up)
    }

    /// True while the fullscreen player is up (the shell hides its status bar).
    pub fn fullscreen(&self) -> bool {
        self.login.is_none() && self.playing.as_ref().is_some_and(|p| p.view.is_some())
    }

    /// A hardware key (evdev code) for the player: space/enter toggle, arrows skip. False = not used.
    pub fn hw_key(&mut self, code: u16, t: f64) -> bool {
        match self.playing.as_mut().and_then(|p| p.view.as_mut()) {
            Some(v) => {
                self.dirty = true;
                v.key(&mut *self.player, code, t)
            }
            None => false,
        }
    }

    fn apply_details(&mut self, ui: &Ui, id: &str, d: Details) {
        if self.details.len() >= DETAILS_CACHED && !self.details.contains_key(id) {
            self.details.clear();
        }
        self.details.insert(id.to_string(), d.clone());
        if let Some(det) = self.detail.as_mut().filter(|det| det.item.id == id) {
            let w = self.w;
            det.set_details(ui, w, d.clone());
            if det.torrent.is_none() && det.item.torrent_url.is_empty() {
                let item = det.item.clone();
                match d.torrent_url {
                    Some(url) => self.fetch_torrent(&item, &url),
                    None => det.set_error(ui, "No torrent link on the page"),
                }
            }
        }
    }

    fn refresh_status(&mut self, ui: &Ui) {
        let visible = self.library.visible().len();
        let status = if visible > 0 || self.library.checking() > 0 {
            None
        } else if self.library.cards.is_empty() {
            Some("No releases found")
        } else {
            Some("No compatible releases on this page")
        };
        self.library.set_status(ui, status);
    }

    // --- screens ---

    fn kb_top(&self, ui: &Ui) -> f32 {
        self.h as f32 - home_zone(ui) - self.keyboard.h()
    }

    /// Bottom of the scrolling content: above the keyboard or the home zone.
    fn content_bottom(&self, ui: &Ui) -> f32 {
        if self.library.searching && self.detail.is_none() {
            self.kb_top(ui)
        } else {
            self.h as f32 - home_zone(ui)
        }
    }

    /// The title whose playback was last started, for the Today View.
    pub fn last_played(&self) -> Option<&Item> {
        self.last_played.as_ref()
    }

    /// Shows the detail screen of `item` (a title the Today View remembered),
    /// leaving whatever was playing.
    pub fn open_item(&mut self, ui: &Ui, item: Item) {
        self.stop_play();
        self.show_detail(ui, item);
    }

    fn open_detail(&mut self, ui: &Ui, ci: usize) {
        if self.library.source_mode() {
            self.open_source_detail(ui, ci);
            return;
        }
        let item = self.library.cards[ci].item.clone();
        self.show_detail(ui, item);
    }

    fn open_source_detail(&mut self, ui: &Ui, ci: usize) {
        let id = self.library.cards[ci].item.id.clone();
        let Some(video) = self.sources.by_id(&id).cloned() else { return };
        self.detail = None;
        self.source_detail = Some(sources::Detail::new(ui, self.w, video.clone()));
        if !sources::playable(&video) {
            self.enrich_source(&video);
        }
        self.library.searching = false;
        self.dirty = true;
    }

    /// Plays a streaming quality (or the muted preview clip) by downloading
    /// it straight to a file the player opens once ready.
    fn start_source_play(&mut self, ui: &Ui, preview: bool, quality: Option<usize>) {
        let Some(d) = &self.source_detail else { return };
        let url = if preview {
            d.video.preview.clone()
        } else {
            match quality.and_then(|i| d.video.qualities.get(i)) {
                Some(q) => q.url.clone(),
                None => deildu_rust::sources::preferred(&d.video.qualities).map(|q| q.url.clone()).unwrap_or_default(),
            }
        };
        if url.is_empty() {
            if let Some(d) = &mut self.source_detail {
                d.set_error(ui, "No playable stream found. Open the source to sign in.");
            }
            self.dirty = true;
            return;
        }
        let safe: String = d.video.item.id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        let name = if preview { "preview.mp4" } else { "video.mp4" };
        let path = self.dir.join("sources").join(safe).join(name);
        let http: Arc<dyn deildu_rust::stream::Http> = self.client.clone();
        match play::DirectDl::start(http, url, path, self.pool.waker()) {
            Ok(download) => {
                let title = site::clean_title(&d.video.item.title);
                let screen = progress::Screen::new(ui, self.w, &title, name);
                self.last_played = Some(d.video.item.clone());
                let (serve, serve_url) = Self::serve_media(download.media());
                self.playing = Some(Playing { download: AnyDownload::Direct(download), title, screen, opened: false, frame: None, overlay: true, status: String::new(), file_name: name.to_string(), view: None, serve, serve_url });
            }
            Err(e) => {
                if let Some(d) = &mut self.source_detail {
                    d.set_error(ui, &e);
                }
            }
        }
        self.dirty = true;
    }

    /// Serves a download on the LAN for the TV (mpv/AirPlay on the same
    /// network); the local player is unaffected.
    fn serve_media(media: Arc<dyn deildu_rust::media::Media>) -> (Option<deildu_rust::serve::Server>, Option<String>) {
        let Ok(mut server) = deildu_rust::serve::Server::start(media) else {
            return (None, None);
        };
        let Some(ip) = lan_ip() else {
            server.stop();
            return (None, None);
        };
        let url = format!("http://{}:{}/video.mp4", ip, server.port());
        (Some(server), Some(url))
    }

    fn show_detail(&mut self, ui: &Ui, item: Item) {
        let mut d = Detail::new(ui, self.w, item.clone());
        match self.details.get(&item.id).cloned() {
            Some(known) => {
                d.set_details(ui, self.w, known.clone());
                self.detail = Some(d);
                let url = if item.torrent_url.is_empty() { known.torrent_url.clone() } else { Some(item.torrent_url.clone()) };
                match url {
                    Some(u) => self.fetch_torrent(&item, &u),
                    None => self.detail.as_mut().unwrap().set_error(ui, "No torrent link on the page"),
                }
            }
            None => {
                self.detail = Some(d);
                self.fetch_details(&item);
                if !item.torrent_url.is_empty() {
                    let url = item.torrent_url.clone();
                    self.fetch_torrent(&item, &url);
                }
            }
        }
        self.library.searching = false;
        self.dirty = true;
    }

    fn start_play(&mut self, ui: &Ui) {
        let Some(d) = &mut self.detail else { return };
        let (Some(t), Some(f)) = (d.torrent.clone(), d.file.clone()) else { return };
        let path = play::target_path(&self.dir, &d.item.id, &f);
        let http: Arc<dyn deildu_rust::stream::Http> = self.client.clone();
        match Download::start(t, f.clone(), http, path, self.pool.waker()) {
            Ok(download) => {
                let title = site::clean_title(&d.item.title);
                let file_name = f.path.rsplit('/').next().unwrap_or("").to_string();
                let screen = progress::Screen::new(ui, self.w, &title, &file_name);
                self.last_played = Some(d.item.clone());
                let ad = AnyDownload::Torrent(download);
                let (serve, serve_url) = Self::serve_media(ad.media());
                self.playing = Some(Playing { download: ad, title, screen, opened: false, frame: None, overlay: true, status: String::new(), file_name, view: None, serve, serve_url });
            }
            Err(e) => d.set_error(ui, &e),
        }
        self.dirty = true;
    }

    fn stop_play(&mut self) {
        if let Some(mut p) = self.playing.take() {
            if let Some(mut server) = p.serve.take() {
                server.stop();
            }
            self.player.stop();
        }
        self.dirty = true;
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let bottom = self.content_bottom(ui);
        if let Some(l) = &self.login {
            return l.hit(ui, self.w, self.h, self.keyboard.h(), x, y);
        }
        if let Some(p) = &self.playing {
            return match p.screen.hit(ui, self.w, self.h, x, y) {
                Some(t) if p.overlay => Some(t),
                _ if p.frame.is_some() => Some(Target::Video),
                t => t,
            };
        }
        if let Some(d) = &self.detail {
            return d.hit(ui, self.w, bottom, x, y);
        }
        if let Some(d) = &self.source_detail {
            return d.hit(ui, self.w, bottom, x, y);
        }
        if self.library.searching && y >= self.kb_top(ui) {
            return Some(Target::Keyboard);
        }
        self.library.hit(ui, self.w, bottom, x, y)
    }

    fn scroller(&mut self) -> Option<&mut scroll::Scroller> {
        if self.login.is_some() || self.playing.is_some() {
            None
        } else if let Some(d) = &mut self.detail {
            Some(&mut d.scroll)
        } else if let Some(d) = &mut self.source_detail {
            Some(&mut d.scroll)
        } else {
            Some(&mut self.library.scroll)
        }
    }

    /// Settings › Keyboard › Auto-Capitalisation for the search and login fields.
    pub fn set_auto_caps(&mut self, on: bool) {
        self.keyboard.set_auto_caps(on);
    }

    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) {
        self.dirty = true;
        if let Some(v) = self.playing.as_mut().and_then(|p| p.view.as_mut()) {
            match v.touch(ui, &mut *self.player, ev, t) {
                player_view::Outcome::Close => self.stop_play(),
                player_view::Outcome::Retry => self.retry_play(ui),
                player_view::Outcome::None => {}
            }
            return;
        }
        let (x, y) = match ev {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            Event::Power | Event::Wake | Event::VolumeUp | Event::VolumeDown | Event::Screenshot => return,
        };
        let kb_top = if self.login.is_some() { self.login.as_ref().and_then(|l| l.keyboard_top(ui, self.h, self.keyboard.h())).unwrap_or(f32::MAX) } else { self.kb_top(ui) };
        match ev {
            Event::Down(..) => {
                self.pressed = self.hit(ui, x, y);
                if self.pressed == Some(Target::Keyboard) {
                    self.kb_touch = true;
                    self.keyboard.down(x, y - kb_top, t);
                } else if self.pressed.is_some_and(scrolls) {
                    if let Some(s) = self.scroller() {
                        s.down(y, t);
                        self.scrolling = true;
                    }
                }
            }
            Event::Move(..) => {
                if self.kb_touch {
                    self.keyboard.moved(x, y - kb_top);
                } else if self.scrolling {
                    let dragging = self.scroller().map(|s| {
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
                if self.kb_touch {
                    self.kb_touch = false;
                    if let Some(out) = self.keyboard.up(x, y - kb_top, t) {
                        self.key(ui, out);
                    }
                    return;
                }
                let tap = if std::mem::take(&mut self.scrolling) { self.scroller().is_none_or(|s| s.up(y, t)) } else { true };
                if tap {
                    if let Some(p) = pressed.filter(|&p| Some(p) == self.hit(ui, x, y)) {
                        self.activate(ui, p);
                    }
                }
            }
        }
    }

    /// Screen-space centre of the detail screen's Play button, when shown.
    pub fn play_button(&self, ui: &Ui) -> Option<(f32, f32)> {
        let d = self.detail.as_ref().filter(|_| self.playing.is_none() && self.login.is_none())?;
        let (x, y, w, h) = d.play_rect(ui);
        Some((x + w / 2.0, y + h / 2.0 + Detail::content_top(ui) - d.scroll.offset()))
    }

    /// Searches the library for `query` at once (Search's Deildu results).
    pub fn search(&mut self, ui: &Ui, query: &str) {
        self.library.set_query(ui, query);
        self.library.searching = false;
        self.library.page = 0;
        self.load_page(ui);
    }

    /// Whether typing reaches a field: the sign-in sheet or the library search.
    pub fn text_focused(&self) -> bool {
        self.login.is_some() || self.library.searching
    }

    /// A hardware keyboard's key, taken only while the search field or the sign-in sheet has focus.
    pub fn hw_text(&mut self, ui: &Ui, out: Out) -> bool {
        let focused = self.login.is_some() || self.library.searching;
        if focused {
            self.key(ui, out);
        }
        focused
    }

    /// Typed input for whichever field has focus (the search field or the sign-in sheet).
    pub fn key(&mut self, ui: &Ui, out: Out) {
        if let Some(l) = &mut self.login {
            match l.edit(ui, out) {
                Edit::Submit => self.submit_login(ui),
                Edit::Hide | Edit::Changed => {}
            }
            return;
        }
        match out {
            Out::Text(t) => {
                let q = format!("{}{}", self.library.query, t);
                self.library.set_query(ui, &q);
            }
            Out::Backspace => {
                let mut q = self.library.query.clone();
                q.pop();
                self.library.set_query(ui, &q);
            }
            Out::Enter => {
                self.library.searching = false;
                if self.library.source_mode() {
                    // Sources filter locally; no refetch.
                    self.library.page = 0;
                    self.apply_source_filter(ui);
                } else {
                    self.library.page = 0;
                    self.load_page(ui);
                }
            }
            Out::Hide => self.library.searching = false,
            Out::Named(_) => {}
        }
    }

    fn activate(&mut self, ui: &Ui, target: Target) {
        match target {
            Target::Tab(i) => {
                if i != self.library.category {
                    self.library.category = i;
                    self.library.page = 0;
                    self.detail = None;
                    self.source_detail = None;
                    if i == library::SOURCE_TAB {
                        self.sources.location = self.sources.provider.base().to_string();
                        self.sources.history.clear();
                        self.library.set_provider_label(ui, self.sources.provider.name());
                    }
                    self.load_page(ui);
                }
            }
            Target::Provider => {
                let at = sources::PROVIDERS.iter().position(|p| *p == self.sources.provider).unwrap_or(0);
                let next = sources::PROVIDERS[(at + 1) % sources::PROVIDERS.len()];
                self.sources.reset(next);
                self.library.page = 0;
                self.library.chip = 0;
                self.library.set_provider_label(ui, next.name());
                self.load_page(ui);
            }
            Target::Chip(i) => {
                if i < sources::FILTERS.len() {
                    self.sources.filter = i;
                    self.apply_source_filter(ui);
                }
            }
            Target::Search => self.library.searching = true,
            Target::Clear => {
                self.library.set_query(ui, "");
                self.library.page = 0;
                self.load_page(ui);
            }
            Target::Hero | Target::Card(_) => {
                let ci = if target == Target::Hero { self.library.split().0 } else if let Target::Card(c) = target { Some(c) } else { None };
                if let Some(ci) = ci {
                    self.open_detail(ui, ci);
                }
            }
            Target::HeroPlay => {
                if let Some(ci) = self.library.split().0 {
                    self.open_detail(ui, ci);
                }
            }
            Target::Prev => {
                if self.library.source_mode() {
                    if let Some(prev) = self.sources.history.pop() {
                        self.sources.location = prev;
                        self.library.page = self.library.page.saturating_sub(1);
                        self.load_page(ui);
                    }
                } else {
                    self.library.page = self.library.page.saturating_sub(1);
                    self.load_page(ui);
                }
            }
            Target::Next => {
                if self.library.source_mode() {
                    if let Some(next) = self.sources.next.clone() {
                        self.sources.history.push(self.sources.location.clone());
                        self.sources.location = next;
                        self.library.page += 1;
                        self.load_page(ui);
                    }
                } else {
                    self.library.page += 1;
                    self.load_page(ui);
                }
            }
            Target::Back => {
                self.detail = None;
                self.source_detail = None;
            }
            Target::Play => {
                if self.source_detail.is_some() {
                    self.start_source_play(ui, false, None);
                } else {
                    self.start_play(ui);
                }
            }
            Target::SQuality(i) => self.start_source_play(ui, false, Some(i)),
            Target::SPreview => self.start_source_play(ui, true, None),
            Target::Stop => self.stop_play(),
            Target::Video => {
                if let Some(p) = &mut self.playing {
                    p.overlay = !p.overlay;
                }
            }
            Target::Cancel => self.login = None,
            Target::Submit => self.submit_login(ui),
            Target::Field(f) => {
                if let Some(l) = &mut self.login {
                    l.focus = f;
                    l.typing = true;
                }
            }
            Target::Content | Target::Keyboard => {}
        }
        self.dirty = true;
    }

    /// Advances scrolling and asks for artwork that scrolled into view.
    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        if let Some(p) = &mut self.playing {
            if p.opened {
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
                        p.screen.set_player(ui, &status);
                        p.status = status;
                        self.dirty = true;
                    }
                }
            }
        }
        let moving = self.scroller().is_some_and(|s| s.tick(t));
        let bottom = self.content_bottom(ui);
        if let Some(d) = &mut self.detail {
            d.fit_scroll(ui, bottom);
        } else if let Some(d) = &mut self.source_detail {
            d.fit_scroll(ui, bottom);
        } else {
            self.library.fit_scroll(ui, self.w, bottom);
        }
        if !moving {
            self.request_art(ui);
        }
        self.dirty |= moving;
        self.dirty || moving
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        self.dirty = false;
        s.fill_rect(0, 0, self.w as usize, self.h as usize, BLACK);
        let bottom = self.content_bottom(ui);
        let pressed = self.pressed;
        if let Some(p) = &self.playing {
            if let Some(v) = &p.view {
                v.draw(s, ui, t);
            } else {
                if let Some(f) = &p.frame {
                    s.blit(f, (self.w - f.w as i32) / 2, (self.h - f.h as i32) / 2, 255, 255);
                }
                if p.overlay {
                    p.screen.draw(s, ui, self.w, self.h, pressed);
                }
            }
        } else if let Some(d) = &self.detail {
            d.draw(s, ui, self.w, bottom, &mut self.art, pressed);
        } else if let Some(d) = &self.source_detail {
            d.draw(s, ui, self.w, bottom, &mut self.art, pressed);
        } else {
            self.library.draw(s, ui, self.w, bottom, &mut self.art, pressed);
            if self.library.searching {
                let top = self.kb_top(ui);
                self.keyboard.draw(s, ui, top, t);
                s.fill_rect(0, (top + self.keyboard.h()) as usize, self.w as usize, self.h as usize, keyboard::BG);
            }
        }
        if let Some(l) = &self.login {
            l.draw(s, ui, self.w, self.h, self.keyboard.h());
            if let Some(top) = l.keyboard_top(ui, self.h, self.keyboard.h()) {
                self.keyboard.draw(s, ui, top, t);
                s.fill_rect(0, (top + self.keyboard.h()) as usize, self.w as usize, self.h as usize, keyboard::BG);
            }
        }
    }
}

/// Targets that live inside a scrolling area.
fn scrolls(t: Target) -> bool {
    matches!(t, Target::Hero | Target::HeroPlay | Target::Card(_) | Target::Prev | Target::Next | Target::Content | Target::Play | Target::Provider | Target::Chip(_) | Target::SQuality(_) | Target::SPreview)
}

/// This machine's LAN address for the TV URL: the first non-loopback IPv4.
fn lan_ip() -> Option<String> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("192.0.2.1:9").ok()?;
    match sock.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip.to_string()),
        _ => None,
    }
}

/// The torrent for `id`: from the disk cache, else from `url` (or the page's download link).
fn torrent_bytes(client: &Client, dir: &Path, id: &str, url: &str, page: &str) -> Option<Vec<u8>> {
    let name = format!("{id}.torrent");
    if let Some(b) = art::disk_read(dir, &name) {
        return Some(b);
    }
    let url = if url.is_empty() {
        match site::fetch_details(client, page) {
            Fetch::Ok(d) => d.torrent_url?,
            _ => return None,
        }
    } else {
        url.to_string()
    };
    let bytes = site::fetch_torrent(client, &url).ok()?;
    if deildu_rust::torrent::Torrent::parse(&bytes).is_ok() {
        art::disk_write(dir, &name, &bytes);
    }
    Some(bytes)
}
