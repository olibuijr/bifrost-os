//! The Music app, bifrost 15 dark: a Library with Artists / Albums / Songs,
//! a full-screen Now Playing sheet, a mini player bar, and the playback
//! engine (`media::audioplayer`) that keeps playing after the app closes.
//! The library is scanned and artwork decoded on worker threads; results
//! arrive through a pipe the shell polls (`fds` / `pump`).
//!
//! The Radio tab lists live stations (`radio`): the list and logos come
//! from the network on a second worker pool and are cached on disk; a tap
//! plays the stream as a live track (no length, no seeking) and the
//! station list becomes the queue, so Previous and Next step through it.

pub mod browse;
pub mod chrome;
pub mod engine;
pub mod jobs;
pub mod library;
pub mod nowplay;
pub mod playlists;
pub mod system;
pub mod upnext;

use bifrost_app::layout::{top_inset, UNIFIED_BAR_PT};
use bifrost_app::kit::ui::{CARD_BG, DETAIL_BG, FIELD_BG, LABEL, LABEL_2, LABEL_3, ROW_PRESSED, SEPARATOR_C};
use bifrost_app::kit::ui::{SPACE_HUGE, SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XS, SPACE_XXL, SPACE_XXS, SPACE_XXXL};
use bifrost_app::kit::ui::{MAC_BODY_PT, MAC_CAPTION1_PT, MAC_TITLE1_PT};
use bifrost_app::kit::ui::{MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT, MAC_TITLE3_PT};
use bifrost_app::kit::Event;
use browse::SortKey;
use bifrost_app::audio::Client;
use bifrost_app::proto::sys::now_ms;
use engine::{AudioPlayer, LiveStatus, Phase, Repeat, Snapshot, Track};
use library::Library;
use playlists::Playlists;
use springboard::deildu::art::{self, Cache};
use bifrost_app::controls::prefs_layout::m;
use bifrost_app::dispatch::Qos;
use jobs::Jobs;
use springboard::deildu::scroll::Scroller;
use springboard::gfx::{pack, rrect_mask, scale, Image, Surface};
use springboard::layout::Rect;
use springboard::shell::cache::TextCache;
use springboard::shell::music::icons;
use springboard::shell::music::nowplaying::{self, clock, draw_slider, Layout, Mini, R};
use springboard::shell::music::radio::{self, Fetch, Radio, Station, Store};
use springboard::shell::switcher::draw_scaled;
use springboard::shell::ui::{home_zone, Ui, Weight, BLACK, WHITE};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use upnext::UpNext;
use bifrost_app::controls::media_changes::{MediaLibrary, Stamp};
use bifrost_app::controls::media_sources::LoadOptions;

/// Music's accent is the system tint like every other app (links and bar buttons are `LINK`).
pub use bifrost_app::kit::ui::LINK as PINK;

/// Nothing is left to ask of the host: volume goes straight to the service.
#[derive(Debug, PartialEq)]
pub enum Effect {}

/// The Now Playing sheet's fallback background: a fixed dark menu tone, since the sheet keeps white text in both appearances.
const SHEET: u32 = bifrost_app::kit::ui::DESKTOP_MENU_SOLID;
/// Text room kept clear of the song duration on a row, the mini player's text, and the song menu's width.
const SONG_TRAIL_PT: f32 = 110.0;
const MINI_TEXT_PT: f32 = 240.0;
const MENU_MAX_W_PT: f32 = 360.0;

/// White and black at `a` of 255, premultiplied as `rgba` makes them. The Now Playing sheet draws white over
/// artwork, so its dims and scrims are these.
fn white_at(a: u8) -> u32 {
    scale(WHITE, a as u32)
}

fn black_at(a: u8) -> u32 {
    scale(BLACK, a as u32)
}
const MARGIN_PT: f32 = 20.0;
const SONG_ROW_PT: f32 = 56.0;
const ARTIST_ROW_PT: f32 = 64.0;
const ALBUM_ROW_PT: f32 = 44.0;
const STATION_ROW_PT: f32 = 64.0;
/// the right edge of a station row that toggles its favourite
const FAV_ZONE_PT: f32 = 64.0;
const THUMB_PT: f32 = 48.0;
const GRID_COLS: usize = 3;
const GRID_GAP_PT: f32 = 16.0;
const GRID_LABEL_PT: f32 = 44.0;
const ALBUM_HEADER_PT: f32 = 236.0;
const ALBUM_COVER_PT: f32 = 180.0;
const BUTTON_H_PT: f32 = 44.0;
const SHEET_MS: f64 = 350.0;
/// text images kept before the cache is emptied
const TEXT_CACHE_MAX: usize = 800;
/// artwork kinds: list thumbnail, grid cell, the Now Playing picture
pub const THUMB: u8 = 0;
pub const CELL_ART: u8 = 1;
pub const BIG: u8 = 2;

const PLAYLIST_HEADER_PT: f32 = 68.0;
const MENU_ROW_PT: f32 = 48.0;
const MENU_HEAD_PT: f32 = 44.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Artists,
    Albums,
    Songs,
    Radio,
    Playlists,
}

/// A page of the Library navigation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Root,
    Artist(usize),
    Album(usize),
    /// the play queue: the current song, then Up Next
    Queue,
    Playlist(usize),
}

/// Something a touch can land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Seg(usize),
    Back,
    /// the i-th item of the page's list
    Row(usize),
    /// the heart of the i-th station row
    Fav(usize),
    Play,
    Shuffle,
    /// the Songs header: cycle the sort order, open or close the search field
    Sort,
    Search,
    /// the "..." of the i-th song row
    More(usize),
    /// move the i-th queue row up one place, or take it out
    QUp(usize),
    QDel(usize),
    /// the playlist page's nav bar: rename, delete
    Rename,
    DeletePlaylist,
    /// a row of the open song menu, or outside it
    MenuItem(usize),
    MenuDismiss,
    /// scrollable area with nothing to activate
    Content,
    Mini(nowplaying::Target),
    Np(nowplaying::Target),
}

/// What the song menu offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuItem {
    PlayNext,
    PlayLast,
    AddTo(usize),
    NewPlaylist,
    /// (playlist, position in it)
    RemoveFromPlaylist(usize, usize),
}

/// The action sheet opened by a song row's "...".
struct Menu {
    song: usize,
    items: Vec<MenuItem>,
}

enum Done {
    /// A full scan: the library and the change stamps of its files.
    Scanned(Library, Vec<Stamp>),
    /// A check of the files on disk against the last scan (`MediaLibrary::observe`).
    Stamps(Vec<Stamp>),
    Art { album: usize, kind: u8, image: Option<Image> },
    Stations(Result<Vec<Station>, String>),
    Logo { id: u64, kind: u8, image: Option<Image> },
}

/// What a player surface says about a track: (id, live, title, subtitle).
type Texts = (u64, bool, String, String);

/// Text rendered once per song shown by the player surfaces.
struct Labels {
    key: Texts,
    title: Image,
    artist: Image,
    mini: Image,
    mini_artist: Image,
}

pub struct Music {
    w: i32,
    h: i32,
    pool: Jobs<Done>,
    /// network jobs (station list, logos), apart from `pool` so a slow
    /// connection never holds up the library
    net: Jobs<Done>,
    fetcher: Arc<dyn Fetch>,
    radio: Radio,
    /// logos being fetched, and logos that could not be had (until the next refresh)
    logo_pending: HashSet<(u64, u8)>,
    logo_failed: HashSet<(u64, u8)>,
    player: AudioPlayer,
    library: Library,
    /// album index of every song, by song id
    album_of: Vec<usize>,
    scanning: bool,
    tab: Tab,
    pages: Vec<Page>,
    scroll: Scroller,
    /// a finger is on the Songs A-Z index
    indexing: bool,
    art: Cache,
    art_pending: HashSet<(usize, u8)>,
    /// covers that would not decode: not asked for again
    art_failed: HashSet<(usize, u8)>,
    cache: TextCache,
    labels: Option<Labels>,
    snap: Snapshot,
    /// the app's Now Playing session, kept in step with `snap`
    now_playing: Option<bifrost_app::now_playing::MediaSession>,
    /// the Now Playing sheet: shown, and its slide (from, to, start)
    np: bool,
    np_anim: Option<(f32, f32, f64)>,
    /// artwork scale on the sheet, eased towards playing/paused
    art_scale: f32,
    /// the second last drawn, to repaint the clocks once a second
    drawn_sec: i64,
    pressed: Option<Target>,
    scrolling: bool,
    /// a finger on a scrubber: the fraction under it, and which surface
    scrub: Option<(f32, Surface_)>,
    vol_drag: bool,
    volume: f32,
    seed: u64,
    /// Songs tab: the order and filter shown (ids into the library)
    view: Vec<usize>,
    sort: SortKey,
    search: String,
    searching: bool,
    /// what is queued, in play order; empty while a station plays
    queue: UpNext,
    path_id: HashMap<PathBuf, usize>,
    /// where the queue is saved (`enable_library_store`)
    store: Option<PathBuf>,
    /// the saved queue, waiting for the scan to finish
    restore: Option<(UpNext, f64)>,
    playlists: Playlists,
    /// a playlist name being typed
    renaming: Option<String>,
    menu: Option<Menu>,
    /// y where a finger went down on the Now Playing sheet (swipe down to dismiss)
    np_swipe: Option<f32>,
    dirty: bool,
    /// a song to play once the scan has found it
    pending_open: Option<PathBuf>,
    /// the music folders the library is scanned from, and the load options that choose their folders
    roots: Vec<PathBuf>,
    load: LoadOptions,
    /// change observation of the library: the last scan's stamps, reported only while notifications are on
    watch: MediaLibrary,
    /// the clock time of the next check of the files on disk
    next_check: f64,
}

/// Which player surface a scrub belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Surface_ {
    Sheet,
}

fn within(x: f32, y: f32, r: R) -> bool {
    r.contains(x, y)
}

/// How often the music folders are checked for added, removed or changed files.
const LIBRARY_CHECK_SECS: f64 = 5.0;

/// Scans the music folders: the library, and the stamps of its files for change observation.
fn scan_now(roots: &[PathBuf], load: &LoadOptions) -> Done {
    let files = library_files(roots, load);
    let stamps = library::stamps_of(&files);
    Done::Scanned(Library::build(library::scan_all(&files)), stamps)
}

/// The stamps of the files the music folders hold now.
fn stamp_now(roots: &[PathBuf], load: &LoadOptions) -> Done {
    Done::Stamps(library::stamps_of(&library_files(roots, load)))
}

/// The files the load options keep under `roots`.
fn library_files(roots: &[PathBuf], load: &LoadOptions) -> Vec<PathBuf> {
    let (walk, skip) = library::plan(roots, load);
    library::find_files_except(&walk, &skip)
}

/// Unix seconds now, 0 when the clock is before the epoch.
fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Music {
    /// Lays the app out for a pane of `w` x `h` px (Split View, Slide Over).
    pub fn set_size(&mut self, w: i32, h: i32) {
        self.w = w;
        self.h = h;
    }

    /// Starts scanning `roots`; playback goes through the audio service `client`.
    pub fn new(w: i32, h: i32, roots: Vec<PathBuf>, load: LoadOptions, client: Client) -> std::io::Result<Self> {
        let player = AudioPlayer::new(client);
        let volume = player.snapshot().volume;
        let pool = Jobs::new(Qos::Utility);
        let net = Jobs::new(Qos::Utility);
        let (scan_roots, scan_load) = (roots.clone(), load.clone());
        pool.spawn(move || scan_now(&scan_roots, &scan_load));
        let mut watch = MediaLibrary::new();
        watch.begin_generating();
        let snap = player.snapshot();
        Ok(Music {
            now_playing: nowplay::session(),
            w,
            h,
            pool,
            net,
            fetcher: Arc::new(radio::Http::default()),
            radio: Radio::default(),
            logo_pending: HashSet::new(),
            logo_failed: HashSet::new(),
            player,
            library: Library::default(),
            album_of: Vec::new(),
            scanning: true,
            tab: Tab::Songs,
            pages: vec![Page::Root],
            scroll: Scroller::new(),
            indexing: false,
            art: Cache::new(art::BUDGET / 2),
            art_pending: HashSet::new(),
            art_failed: HashSet::new(),
            cache: TextCache::default(),
            labels: None,
            snap,
            np: false,
            np_anim: None,
            art_scale: nowplaying::PAUSED_SCALE,
            drawn_sec: -1,
            pressed: None,
            scrolling: false,
            scrub: None,
            vol_drag: false,
            volume,
            seed: 0x5EED,
            view: Vec::new(),
            sort: SortKey::Title,
            search: String::new(),
            searching: false,
            queue: UpNext::default(),
            path_id: HashMap::new(),
            store: None,
            restore: None,
            playlists: Playlists::default(),
            renaming: None,
            menu: None,
            np_swipe: None,
            dirty: true,
            pending_open: None,
            roots,
            load,
            watch,
            next_check: 0.0,
        })
    }

    /// Turns the Radio tab on: stations, logos and favourites are kept in
    /// `dir`; `fetcher` downloads them. The saved list shows at once; the
    /// network is asked when the tab is opened.
    pub fn enable_radio(&mut self, dir: PathBuf, fetcher: Arc<dyn Fetch>) {
        self.radio = Radio::open(Store::new(dir));
        self.fetcher = fetcher;
        self.dirty = true;
    }

    /// Keeps the play queue and the playlists in `dir`: playlists as `.m3u`
    /// files in `dir/Playlists`, the queue in `dir/queue.txt`. The saved
    /// queue comes back paused once the library is scanned.
    pub fn enable_library_store(&mut self, dir: PathBuf) {
        self.playlists = Playlists::open(dir.join("Playlists"));
        self.restore = std::fs::read_to_string(dir.join("queue.txt")).ok().and_then(|t| UpNext::from_text(&t));
        self.store = Some(dir);
        self.dirty = true;
    }

    /// Whether a text field (Songs search, playlist name) wants the keyboard.
    pub fn wants_keyboard(&self) -> bool {
        self.searching || self.renaming.is_some()
    }

    /// Typed text for the field that has the keyboard; a newline accepts it.
    pub fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            if c == '\n' {
                self.commit_rename();
            } else if c.is_control() {
                continue;
            } else if let Some(name) = &mut self.renaming {
                name.push(c);
            } else if self.searching {
                self.search.push(c);
                self.refilter();
            }
        }
        self.dirty = true;
    }

    pub fn backspace(&mut self) {
        if let Some(name) = &mut self.renaming {
            name.pop();
        } else if self.searching && self.search.pop().is_some() {
            self.refilter();
        }
        self.dirty = true;
    }

    fn commit_rename(&mut self) {
        let (Some(name), Page::Playlist(p)) = (self.renaming.take(), self.page()) else { return };
        if self.playlists.rename(p, &name) {
            let clean = playlists::clean_name(&name).unwrap_or_default().to_lowercase();
            if let Some(i) = self.playlists.lists().iter().position(|l| l.name.to_lowercase() == clean) {
                *self.pages.last_mut().unwrap() = Page::Playlist(i);
            }
        }
    }

    fn rebuild_view(&mut self) {
        self.view = browse::view(&self.library.songs, &self.search, self.sort);
    }

    fn refilter(&mut self) {
        self.rebuild_view();
        self.scroll.jump_top();
    }

    /// Whether the current track is a live stream (no length, no seeking).
    pub fn is_live(&self) -> bool {
        self.snap.track.as_ref().is_some_and(Track::is_live)
    }

    /// Whether a song is loaded (the mini player, lock card and CC tile show it).
    pub fn has_track(&self) -> bool {
        self.snap.track.is_some()
    }

    pub fn page(&self) -> Page {
        *self.pages.last().unwrap_or(&Page::Root)
    }

    /// When the next repaint without input is due: every 250 ms while a
    /// clock or scrubber is moving or the service may have changed state, 50 ms
    /// while a job or an animation is pending (the app only waits on the shell socket).
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        let busy = self.scanning || self.scroll.animating() || self.np_anim.is_some() || !self.art_pending.is_empty() || !self.logo_pending.is_empty();
        let wake = t + if busy { 50.0 } else { 250.0 };
        Some(self.player.next_wake(t).map_or(wake, |w| w.min(wake)))
    }

    // --- jobs ---

    /// Applies finished jobs and the engine's latest state; true if the
    /// screen changed.
    pub fn pump(&mut self, ui: &Ui) -> bool {
        let finished: Vec<Done> = self.pool.drain().into_iter().chain(self.net.drain()).collect();
        let mut changed = !finished.is_empty();
        for done in finished {
            self.apply(done);
        }
        self.player.drain(now_ms());
        changed |= self.refresh(ui);
        self.dirty |= changed;
        changed
    }

    fn apply(&mut self, done: Done) {
        match done {
            Done::Scanned(lib, stamps) => {
                self.album_of = (0..lib.songs.len()).map(|s| lib.album_of(s).unwrap_or(0)).collect();
                self.path_id = lib.songs.iter().map(|s| (s.path.clone(), s.id)).collect();
                self.library = lib;
                self.scanning = false;
                let _ = self.watch.observe(&stamps, unix_now());
                self.rebuild_view();
                self.restore_queue();
                if let Some(path) = self.pending_open.take() {
                    self.play_path(&path);
                }
            }
            Done::Stamps(stamps) => {
                // a file was added, removed or changed since the last scan: scan again
                if !self.scanning && self.watch.observe(&stamps, unix_now()).is_some() {
                    self.start_scan();
                    self.dirty = true;
                }
            }
            Done::Art { album, kind, image } => {
                self.art_pending.remove(&(album, kind));
                match image {
                    Some(img) => self.art.insert(&album.to_string(), kind, img),
                    None => {
                        self.art_failed.insert((album, kind));
                    }
                }
            }
            Done::Stations(result) => self.radio.fetched(result),
            Done::Logo { id, kind, image } => {
                self.logo_pending.remove(&(id, kind));
                match image {
                    Some(img) => self.art.insert(&logo_key(id), kind, img),
                    None => {
                        self.logo_failed.insert((id, kind));
                    }
                }
            }
        }
    }

    /// Pulls the engine's snapshot; true when what is shown changed.
    fn refresh(&mut self, ui: &Ui) -> bool {
        let mut snap = self.player.snapshot();
        snap.shuffle = self.queue.shuffled();
        if snap.track.as_ref().is_some_and(Track::is_live) {
            self.queue = UpNext::default();
            snap.shuffle = false;
        } else if let Some(i) = snap.index.filter(|&i| i != self.queue.current() && snap.queue_len == self.queue.items().len()) {
            self.queue.set_current(i);
            self.save_queue();
        }
        let texts = snap.track.as_ref().map(|t| self.texts(t, snap.live.as_ref()));
        if texts.as_ref() != self.labels.as_ref().map(|l| &l.key) {
            self.labels = texts.map(|t| self.make_labels(ui, t));
        }
        let sec = (snap.position_ms / 1000.0) as i64;
        let changed = snap.playing != self.snap.playing || snap.index != self.snap.index || snap.shuffle != self.snap.shuffle || snap.repeat != self.snap.repeat || sec != self.drawn_sec || snap.error != self.snap.error || snap.live != self.snap.live;
        self.snap = snap;
        self.sync_now_playing();
        changed
    }

    /// The title and second line a player surface shows for `track`. A
    /// station shows its name and, under it, how the stream is doing or
    /// what is on air (the stream's own title, else the API's, else the
    /// frequency).
    fn texts(&self, track: &Track, live: Option<&LiveStatus>) -> Texts {
        if !track.is_live() {
            let (title, artist) = self.library.songs.get(track.id as usize).map(|s| (s.title.clone(), s.artist.clone())).unwrap_or_else(|| ("Unknown".into(), String::new()));
            return (track.id, false, title, artist);
        }
        let station = self.radio.station(track.id);
        let on_air = || live.and_then(|l| l.title.clone()).or_else(|| station.and_then(|s| s.now.clone())).or_else(|| station.map(Station::frequency_text).filter(|f| !f.is_empty()));
        let sub = match live.map(|l| l.phase) {
            Some(Phase::Reconnecting(_)) => "Reconnecting…".to_string(),
            Some(Phase::Buffering) => "Buffering…".to_string(),
            _ => on_air().unwrap_or_else(|| "Live".into()),
        };
        (track.id, true, station.map_or("Radio".to_string(), |s| s.name.clone()), sub)
    }

    fn make_labels(&mut self, ui: &Ui, key: Texts) -> Labels {
        let (title, artist) = (key.2.clone(), key.3.clone());
        let np_w = self.w as f32 - 2.0 * ui.pt(nowplaying::SIDE_PT) - ui.pt(44.0);
        let dim = white_at(150);
        Labels {
            key,
            title: ui.text(&ui.fit(&title, MAC_TITLE3_PT, np_w), MAC_TITLE3_PT, Weight::Semibold, WHITE),
            artist: ui.text(&ui.fit(&artist, MAC_TITLE3_PT, np_w), MAC_TITLE3_PT, Weight::Regular, dim),
            mini: ui.text(&ui.fit(&title, MAC_SUBHEAD_PT, self.w as f32 - ui.pt(MINI_TEXT_PT)), MAC_SUBHEAD_PT, Weight::Semibold, WHITE),
            mini_artist: ui.text(&ui.fit(&artist, MAC_FOOTNOTE_PT, self.w as f32 - ui.pt(MINI_TEXT_PT)), MAC_FOOTNOTE_PT, Weight::Regular, WHITE),
        }
    }

    /// The artwork cache key of what `track` shows: the album of a song,
    /// the logo of a station.
    fn art_key(&self, track: &Track) -> Option<String> {
        if track.is_live() {
            return Some(logo_key(track.id));
        }
        self.album_of.get(track.id as usize).map(|a| a.to_string())
    }

    /// Starts fetching the station list unless it is fresh; retries logos
    /// that failed before.
    fn radio_refresh(&mut self) {
        let Some(store) = self.radio.store().cloned().filter(|_| self.radio.stale()) else { return };
        self.radio.fetching = true;
        self.logo_failed.clear();
        let net = self.fetcher.clone();
        self.net.spawn(move || Done::Stations(store.refresh(&*net).map_err(|e| e.to_string())));
    }

    /// Asks for the logos the visible surfaces need and do not have.
    fn request_logos(&mut self, ui: &Ui) {
        let mut wanted: Vec<(u64, u8)> = Vec::new();
        if let Some(t) = self.snap.track.as_ref().filter(|t| t.is_live()) {
            wanted.push((t.id, THUMB));
            if self.np || self.np_anim.is_some() {
                wanted.push((t.id, BIG));
            }
        }
        if !self.np && self.tab == Tab::Radio && self.page() == Page::Root {
            let (top, bottom) = (self.content_top(ui), self.content_bottom(ui));
            let (off, row) = (self.scroll.offset(), ui.pt(STATION_ROW_PT));
            let first = (off / row).max(0.0) as usize;
            let last = ((off + bottom - top) / row).max(0.0) as usize + 1;
            let order = self.radio.order();
            wanted.extend(order.iter().skip(first).take(last - first).map(|&i| (self.radio.stations()[i].id, THUMB)));
        }
        for (id, kind) in wanted {
            let key = (id, kind);
            if self.art.contains(&logo_key(id), kind) || self.logo_pending.contains(&key) || self.logo_failed.contains(&key) {
                continue;
            }
            let (Some(station), Some(store)) = (self.radio.station(id).cloned(), self.radio.store().cloned()) else { continue };
            self.logo_pending.insert(key);
            let (size, radius) = match kind {
                THUMB => (ui.pt(THUMB_PT) as usize, ui.pt(m::ICON_RADIUS)),
                _ => (Layout::new(ui, self.w, self.h).art.w as usize, 0.0),
            };
            let net = self.fetcher.clone();
            self.net.spawn(move || {
                let image = store.logo(&station, &*net).and_then(|bytes| art::decode(&bytes)).map(|img| {
                    let mut c = art::cover(&img, size, size);
                    if radius > 0.0 {
                        c.apply_mask(&rrect_mask(size, size, radius));
                    }
                    c
                });
                Done::Logo { id, kind, image }
            });
        }
    }

    /// Asks for artwork the visible surfaces need and do not have.
    fn request_art(&mut self, ui: &Ui) {
        let mut wanted: Vec<(usize, u8)> = Vec::new();
        if let Some(id) = self.snap.track.as_ref().filter(|t| !t.is_live()).map(|t| t.id as usize) {
            if let Some(&a) = self.album_of.get(id) {
                wanted.push((a, THUMB));
                if self.np || self.np_anim.is_some() {
                    wanted.push((a, BIG));
                }
            }
        }
        if !self.np {
            let (top, bottom) = (self.content_top(ui), self.content_bottom(ui));
            let off = self.scroll.offset();
            match (self.page(), self.tab) {
                (Page::Root, Tab::Songs) => {
                    let row = ui.pt(SONG_ROW_PT);
                    let first = (off / row).max(0.0) as usize;
                    let last = ((off + bottom - top) / row).max(0.0) as usize + 1;
                    for i in first..last.min(self.view.len()) {
                        wanted.push((self.album_of[self.view[i]], THUMB));
                    }
                }
                (Page::Root, Tab::Albums) => {
                    for (i, (_, y, _, h)) in self.grid_cells(ui, self.library.albums.len()).into_iter().enumerate() {
                        if y - off + h >= 0.0 && y - off <= bottom - top {
                            wanted.push((i, CELL_ART));
                        }
                    }
                }
                (Page::Artist(a), _) => {
                    for &album in &self.library.artists[a].albums {
                        wanted.push((album, CELL_ART));
                    }
                }
                (Page::Album(a), _) => wanted.push((a, CELL_ART)),
                (Page::Playlist(p), _) => {
                    let row = ui.pt(SONG_ROW_PT);
                    let first = ((off - ui.pt(PLAYLIST_HEADER_PT)) / row).max(0.0) as usize;
                    let last = ((off + bottom - top) / row).max(0.0) as usize + 1;
                    wanted.extend(self.playlist_entries(p).into_iter().skip(first).take(last.saturating_sub(first)).map(|e| (self.album_of[e.1], THUMB)));
                }
                (Page::Queue, _) => {
                    let row = ui.pt(SONG_ROW_PT);
                    let first = (off / row).max(0.0) as usize;
                    let last = ((off + bottom - top) / row).max(0.0) as usize + 1;
                    let from = self.queue.current();
                    let ids: Vec<usize> = self.queue.items().iter().skip(from + first).take(last.saturating_sub(first)).filter_map(|p| self.path_id.get(p).copied()).collect();
                    wanted.extend(ids.into_iter().map(|id| (self.album_of[id], THUMB)));
                }
                (Page::Root, Tab::Artists | Tab::Radio | Tab::Playlists) => {}
            }
        }
        for (album, kind) in wanted {
            let key = (album, kind);
            if self.art.contains(&album.to_string(), kind) || self.art_pending.contains(&key) {
                continue;
            }
            if self.art_failed.contains(&key) {
                continue;
            }
            let Some(bytes) = self.library.covers.get(album).cloned().flatten() else { continue };
            self.art_pending.insert(key);
            let (size, radius) = match kind {
                THUMB => (ui.pt(THUMB_PT) as usize, ui.pt(m::ICON_RADIUS)),
                CELL_ART => (self.grid_cell_w(ui) as usize, ui.pt(m::ICON_RADIUS)),
                _ => (Layout::new(ui, self.w, self.h).art.w as usize, 0.0),
            };
            self.pool.spawn(move || {
                let image = art::decode(&bytes).map(|img| {
                    let mut c = art::cover(&img, size, size);
                    if radius > 0.0 {
                        c.apply_mask(&rrect_mask(size, size, radius));
                    }
                    c
                });
                Done::Art { album, kind, image }
            });
        }
    }

    // --- geometry ---

    fn content_top(&self, ui: &Ui) -> f32 {
        match self.page() {
            _ => ui.pt(UNIFIED_BAR_PT),
        }
    }

    /// Bottom of the scrolling content: the mini player or the home zone.
    fn content_bottom(&self, ui: &Ui) -> f32 {
        if self.has_track() {
            Mini::new(ui, self.w, self.h).bar.y
        } else {
            self.h as f32 - home_zone(ui)
        }
    }

    fn grid_cell_w(&self, ui: &Ui) -> f32 {
        ((self.w as f32 - 2.0 * ui.pt(MARGIN_PT) - (GRID_COLS as f32 - 1.0) * ui.pt(GRID_GAP_PT)) / GRID_COLS as f32).floor()
    }

    /// Album grid cells (x, y, w, h) relative to the content top, unscrolled.
    fn grid_cells(&self, ui: &Ui, n: usize) -> Vec<(f32, f32, f32, f32)> {
        let cw = self.grid_cell_w(ui);
        let pitch = cw + ui.pt(GRID_LABEL_PT);
        (0..n).map(|i| (ui.pt(MARGIN_PT) + (i % GRID_COLS) as f32 * (cw + ui.pt(GRID_GAP_PT)), ui.pt(SPACE_S) + (i / GRID_COLS) as f32 * pitch, cw, cw + ui.pt(GRID_LABEL_PT))).collect()
    }

    /// The Play and Shuffle buttons on screen now (album and playlist pages), as (label, frame in pane pixels).
    /// The window publishes them as controls.
    pub fn action_buttons(&self, ui: &Ui) -> Vec<(&'static str, R)> {
        let buttons = match self.page() {
            Page::Album(_) => self.album_buttons(ui),
            Page::Playlist(_) => self.playlist_buttons(ui),
            _ => return vec![],
        };
        // the header scrolls with the list: its top is the content top less the scroll offset
        let (top, bottom) = (self.content_top(ui), self.content_bottom(ui));
        let y0 = top - self.scroll.offset();
        buttons
            .into_iter()
            .zip(["Play", "Shuffle"])
            .map(|(b, label)| (label, R { x: b.x, y: y0 + b.y, w: b.w, h: b.h }))
            .filter(|(_, r)| r.y + r.h > top && r.y < bottom)
            .collect()
    }

    /// Play or Shuffle on the album or playlist page, as if its button were pressed.
    pub fn press_action(&mut self, shuffle: bool, t: f64) {
        self.activate(if shuffle { Target::Shuffle } else { Target::Play }, t);
    }

    /// The album page's Play and Shuffle buttons, unscrolled.
    fn album_buttons(&self, ui: &Ui) -> [R; 2] {
        let x = ui.pt(MARGIN_PT + ALBUM_COVER_PT + 20.0);
        let bw = (self.w as f32 - x - ui.pt(MARGIN_PT) - ui.pt(SPACE_M)) / 2.0;
        let y = ui.pt(16.0 + ALBUM_COVER_PT - BUTTON_H_PT);
        [R { x, y, w: bw, h: ui.pt(BUTTON_H_PT) }, R { x: x + bw + ui.pt(SPACE_M), y, w: bw, h: ui.pt(BUTTON_H_PT) }]
    }

    /// The playlist page's Play and Shuffle buttons, unscrolled.
    fn playlist_buttons(&self, ui: &Ui) -> [R; 2] {
        let m = ui.pt(MARGIN_PT);
        let bw = (self.w as f32 - 2.0 * m - ui.pt(SPACE_M)) / 2.0;
        let y = ui.pt(SPACE_M);
        [R { x: m, y, w: bw, h: ui.pt(BUTTON_H_PT) }, R { x: m + bw + ui.pt(SPACE_M), y, w: bw, h: ui.pt(BUTTON_H_PT) }]
    }

    /// Height of the current page's content.
    fn content_h(&self, ui: &Ui) -> f32 {
        let n = |len: usize| len as f32;
        match (self.page(), self.tab) {
            (Page::Root, Tab::Songs) => n(self.view.len()) * ui.pt(SONG_ROW_PT) + ui.pt(SPACE_L),
            (Page::Root, Tab::Playlists) => n(self.playlists.lists().len() + 1) * ui.pt(ARTIST_ROW_PT) + ui.pt(SPACE_L),
            (Page::Playlist(p), _) => ui.pt(PLAYLIST_HEADER_PT) + n(self.playlist_entries(p).len()) * ui.pt(SONG_ROW_PT) + ui.pt(SPACE_L),
            (Page::Queue, _) => n(self.queue.items().len().saturating_sub(self.queue.current())) * ui.pt(SONG_ROW_PT) + ui.pt(SPACE_L),
            (Page::Root, Tab::Artists) => n(self.library.artists.len()) * ui.pt(ARTIST_ROW_PT) + ui.pt(SPACE_L),
            (Page::Root, Tab::Radio) => n(self.radio.stations().len()) * ui.pt(STATION_ROW_PT) + ui.pt(SPACE_L),
            (Page::Root, Tab::Albums) => self.grid_cells(ui, self.library.albums.len()).last().map_or(0.0, |c| c.1 + c.3 + ui.pt(SPACE_L)),
            (Page::Artist(a), _) => self.grid_cells(ui, self.library.artists[a].albums.len()).last().map_or(0.0, |c| c.1 + c.3 + ui.pt(SPACE_L)),
            (Page::Album(a), _) => ui.pt(ALBUM_HEADER_PT) + n(self.library.albums[a].songs.len()) * ui.pt(ALBUM_ROW_PT) + ui.pt(SPACE_XXL),
        }
    }

    fn fit_scroll(&mut self, ui: &Ui) {
        let visible = self.content_bottom(ui) - self.content_top(ui);
        let max = self.content_h(ui) - visible;
        self.scroll.set_max(max);
    }

    /// The list item under content-relative (x, y).
    fn row_at(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        if y < 0.0 {
            return None;
        }
        let in_grid = |cells: Vec<(f32, f32, f32, f32)>| cells.iter().position(|&(cx, cy, cw, ch)| x >= cx && x < cx + cw && y >= cy && y < cy + ch).map(Target::Row);
        match (self.page(), self.tab) {
            (Page::Root, Tab::Songs) => Some((y / ui.pt(SONG_ROW_PT)) as usize).filter(|&i| i < self.view.len()).map(|i| if x >= self.w as f32 - ui.pt(FAV_ZONE_PT) { Target::More(i) } else { Target::Row(i) }),
            (Page::Root, Tab::Playlists) => Some((y / ui.pt(ARTIST_ROW_PT)) as usize).filter(|&i| i <= self.playlists.lists().len()).map(Target::Row),
            (Page::Playlist(p), _) => {
                let [play, shuffle] = self.playlist_buttons(ui);
                if within(x, y, play) {
                    return Some(Target::Play);
                }
                if within(x, y, shuffle) {
                    return Some(Target::Shuffle);
                }
                let header = ui.pt(PLAYLIST_HEADER_PT);
                (y >= header).then(|| ((y - header) / ui.pt(SONG_ROW_PT)) as usize).filter(|&i| i < self.playlist_entries(p).len()).map(|i| if x >= self.w as f32 - ui.pt(FAV_ZONE_PT) { Target::More(i) } else { Target::Row(i) })
            }
            (Page::Queue, _) => {
                let i = (y / ui.pt(SONG_ROW_PT)) as usize;
                if i >= self.queue.items().len().saturating_sub(self.queue.current()) {
                    return None;
                }
                let zone = ui.pt(FAV_ZONE_PT);
                Some(if i >= 1 && x >= self.w as f32 - zone {
                    Target::QDel(i)
                } else if i >= 2 && x >= self.w as f32 - 2.0 * zone {
                    Target::QUp(i)
                } else {
                    Target::Row(i)
                })
            }
            (Page::Root, Tab::Artists) => Some((y / ui.pt(ARTIST_ROW_PT)) as usize).filter(|&i| i < self.library.artists.len()).map(Target::Row),
            (Page::Root, Tab::Radio) => Some((y / ui.pt(STATION_ROW_PT)) as usize).filter(|&i| i < self.radio.stations().len()).map(|i| if x >= self.w as f32 - ui.pt(FAV_ZONE_PT) { Target::Fav(i) } else { Target::Row(i) }),
            (Page::Root, Tab::Albums) => in_grid(self.grid_cells(ui, self.library.albums.len())),
            (Page::Artist(a), _) => in_grid(self.grid_cells(ui, self.library.artists[a].albums.len())),
            (Page::Album(a), _) => {
                let [play, shuffle] = self.album_buttons(ui);
                if within(x, y, play) {
                    return Some(Target::Play);
                }
                if within(x, y, shuffle) {
                    return Some(Target::Shuffle);
                }
                let header = ui.pt(ALBUM_HEADER_PT);
                (y >= header).then(|| ((y - header) / ui.pt(ALBUM_ROW_PT)) as usize).filter(|&i| i < self.library.albums[a].songs.len()).map(|i| if x >= self.w as f32 - ui.pt(FAV_ZONE_PT) { Target::More(i) } else { Target::Row(i) })
            }
        }
    }

    /// Whether the Songs A-Z index is shown.
    fn index_shown(&self) -> bool {
        self.page() == Page::Root && self.tab == Tab::Songs && !self.searching && !self.view.is_empty() && browse::indexed(self.sort)
    }

    /// The vertical extent of the A-Z index: (top, slot height).
    fn index_geometry(&self, ui: &Ui) -> (f32, f32) {
        let top = self.content_top(ui) + ui.pt(SPACE_S);
        let avail = self.content_bottom(ui) - top - ui.pt(SPACE_S);
        (top, (avail / browse::INDEX.len() as f32).min(ui.pt(SPACE_L)))
    }

    /// The index slot under (x, y), when the index is shown there.
    fn index_slot(&self, ui: &Ui, x: f32, y: f32) -> Option<usize> {
        if !self.index_shown() || x < self.w as f32 - ui.pt(SPACE_XXL) || y >= self.content_bottom(ui) {
            return None;
        }
        let (top, step) = self.index_geometry(ui);
        (y >= top - step).then(|| (((y - top) / step).max(0.0) as usize).min(browse::INDEX.len() - 1))
    }

    /// Scrolls the Songs list to the section of `slot`.
    fn jump_to_slot(&mut self, ui: &Ui, slot: usize) {
        let sec = browse::sections(&self.library.songs, &self.view, self.sort);
        if let Some(pos) = browse::target_for(&sec, slot) {
            self.fit_scroll(ui);
            self.scroll.jump_to(pos as f32 * ui.pt(SONG_ROW_PT));
        }
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        if let Some(menu) = &self.menu {
            let card = self.menu_rect(ui, menu.items.len());
            let first = card.y + ui.pt(MENU_HEAD_PT);
            return Some(if !within(x, y, card) {
                Target::MenuDismiss
            } else if y >= first {
                Target::MenuItem(((y - first) / ui.pt(MENU_ROW_PT)) as usize)
            } else {
                Target::Content
            });
        }
        if self.np {
            return Layout::new(ui, self.w, self.h).hit(x, y).map(Target::Np);
        }
        if self.has_track() {
            if let Some(t) = Mini::new(ui, self.w, self.h).hit(x, y) {
                return Some(Target::Mini(t));
            }
        }
        let top = self.content_top(ui);
        if y < top {
            return None;
        }
        if y >= self.content_bottom(ui) {
            return None;
        }
        Some(self.row_at(ui, x, y - top + self.scroll.offset()).unwrap_or(Target::Content))
    }

    // --- input ---

    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) -> Vec<Effect> {
        self.dirty = true;
        let (x, y) = match ev {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            Event::Power | Event::Wake | Event::VolumeUp | Event::VolumeDown | Event::Screenshot => return vec![],
        };
        let layout = Layout::new(ui, self.w, self.h);
        if self.menu.is_none() && !self.np {
            match ev {
                Event::Down(..) => {
                    if let Some(slot) = self.index_slot(ui, x, y) {
                        self.indexing = true;
                        self.pressed = None;
                        self.jump_to_slot(ui, slot);
                        return vec![];
                    }
                }
                Event::Move(..) if self.indexing => {
                    let slot = self.index_slot(ui, self.w as f32, y).unwrap_or(0);
                    self.jump_to_slot(ui, slot);
                    return vec![];
                }
                Event::Move(..) => {}
                _ if self.indexing => {
                    self.indexing = false;
                    return vec![];
                }
                _ => {}
            }
        }
        match ev {
            Event::Down(..) => {
                self.pressed = self.hit(ui, x, y);
                self.np_swipe = if self.np { Some(y) } else { None };
                match self.pressed {
                    Some(Target::Np(nowplaying::Target::Scrubber)) if !self.is_live() => self.scrub = Some((Layout::fraction(layout.scrub, x), Surface_::Sheet)),
                    Some(Target::Np(nowplaying::Target::Volume)) => {
                        self.vol_drag = true;
                        return self.set_volume_from(Layout::fraction(layout.volume, x));
                    }
                    Some(p) if scrolls(p) => {
                        self.scroll.down(y, t);
                        self.scrolling = true;
                    }
                    _ => {}
                }
            }
            Event::Move(..) => {
                if let Some((_, Surface_::Sheet)) = self.scrub {
                    self.scrub = Some((Layout::fraction(layout.scrub, x), Surface_::Sheet));
                } else if self.vol_drag {
                    return self.set_volume_from(Layout::fraction(layout.volume, x));
                } else if self.scrolling {
                    self.scroll.moved(y, t);
                    if self.scroll.dragging() {
                        self.pressed = None;
                    }
                }
            }
            Event::Up(..) | Event::Power | Event::Wake | Event::VolumeUp | Event::VolumeDown | Event::Screenshot => {
                let pressed = self.pressed.take();
                if let Some((f, Surface_::Sheet)) = self.scrub.take() {
                    self.seek_fraction(f);
                    return vec![];
                }
                if std::mem::take(&mut self.vol_drag) {
                    return vec![];
                }
                if let Some(y0) = self.np_swipe.take().filter(|_| self.np) {
                    if y - y0 > ui.pt(80.0) {
                        return self.activate(Target::Np(nowplaying::Target::Dismiss), t);
                    }
                }
                let tap = if std::mem::take(&mut self.scrolling) { self.scroll.up(y, t) } else { true };
                if tap {
                    if let Some(p) = pressed.filter(|&p| Some(p) == self.hit(ui, x, y)) {
                        return self.activate(p, t);
                    }
                }
            }
        }
        vec![]
    }

    fn set_volume_from(&mut self, f: f32) -> Vec<Effect> {
        self.volume = f;
        self.player.set_volume(f);
        vec![]
    }

    fn seek_fraction(&mut self, f: f32) {
        if let Some(d) = self.snap.duration_ms {
            self.player.seek_ms(f as f64 * d);
            self.snap.position_ms = f as f64 * d;
        }
    }

    fn open_sheet(&mut self, open: bool, t: f64) {
        if self.np == open {
            return;
        }
        let from = self.sheet_progress(t);
        self.np = open;
        self.np_anim = Some((from, if open { 1.0 } else { 0.0 }, t));
        self.dirty = true;
    }

    /// 0 = sheet away, 1 = sheet shown.
    fn sheet_progress(&self, t: f64) -> f32 {
        match self.np_anim {
            Some((from, to, start)) => from + (to - from) * springboard::shell::gestures::ease_out_quartic(springboard::shell::gestures::progress(start, t, SHEET_MS)),
            None => self.np as u8 as f32,
        }
    }

    /// Plays the library song at `path` (Files hands songs over); false when it is not in the library.
    pub fn play_path(&mut self, path: &std::path::Path) -> bool {
        if self.scanning {
            self.pending_open = Some(path.to_path_buf());
            return true;
        }
        match self.library.songs.iter().position(|s| s.path == path) {
            Some(id) => {
                self.play_songs(vec![id], 0, false);
                true
            }
            None => false,
        }
    }

    /// Replaces the queue with `ids` and plays `ids[start]`; with `shuffle` the
    /// songs after it are put in random order.
    fn play_songs(&mut self, ids: Vec<usize>, start: usize, shuffle: bool) {
        // MIDI files are listed but have no sound yet: they are left out of the queue, and the start follows the song
        let start_id = ids.get(start).copied();
        let ids: Vec<usize> = ids.into_iter().filter(|&id| !self.library.songs[id].is_midi()).collect();
        if ids.is_empty() {
            return;
        }
        let start = start_id.and_then(|s| ids.iter().position(|&id| id == s)).unwrap_or(0);
        let paths = ids.iter().map(|&id| self.library.songs[id].path.clone()).collect();
        let mut queue = UpNext::new(paths, start);
        if shuffle {
            self.next_seed();
            queue.set_shuffle(true, self.seed);
        }
        self.queue = queue;
        self.push_queue(0.0, true);
    }

    fn next_seed(&mut self) {
        self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    }

    /// The engine's snapshot, with the shuffle state this app keeps.
    fn take_snapshot(&mut self) {
        let mut snap = self.player.snapshot();
        snap.shuffle = self.queue.shuffled();
        self.snap = snap;
        self.sync_now_playing();
    }

    /// Keeps the app's Now Playing session in step with the engine's snapshot: the playing song or station as content.
    fn sync_now_playing(&mut self) {
        let content = self.snap.track.as_ref().and_then(|t| {
            let (_, live, title, artist) = self.texts(t, self.snap.live.as_ref());
            let album = if live { String::new() } else { self.library.songs.get(t.id as usize).map(|s| s.album.clone()).unwrap_or_default() };
            nowplay::content(t, &title, &artist, &album, self.snap.duration_ms)
        });
        let snap = &self.snap;
        if let Some(session) = self.now_playing.as_mut() {
            nowplay::sync(session, snap, content);
        }
    }

    /// Runs a system playback command (by its name: play, pause, toggle, next, previous) on the transport. False when
    /// the command is unknown or the session does not allow it now.
    pub fn remote_command(&mut self, name: &str) -> bool {
        let Some(cmd) = nowplay::command_named(name) else { return false };
        let playing = self.snap.playing;
        let Some(target) = self.now_playing.as_ref().and_then(|s| nowplay::route(s, cmd, playing)) else { return false };
        self.control(target);
        true
    }

    /// The app's Now Playing session, as published to Control Center (`app.rs`).
    pub fn session(&self) -> Option<&bifrost_app::now_playing::MediaSession> {
        self.now_playing.as_ref()
    }

    /// Starts a scan of the music folders in the background; the library is replaced when it finishes.
    fn start_scan(&mut self) {
        self.scanning = true;
        let (roots, load) = (self.roots.clone(), self.load.clone());
        self.pool.spawn(move || scan_now(&roots, &load));
    }

    /// Drops the library and scans the music folders again. Change observation starts again from the new scan.
    pub fn reload_library(&mut self) {
        self.watch.unload();
        self.start_scan();
        self.dirty = true;
    }

    /// Whether the library is still being scanned (the Dock tile shows progress meanwhile).
    pub fn is_scanning(&self) -> bool {
        self.scanning
    }

    /// A seek from Control Center, in milliseconds. False when the session does not allow it now: no song, a station,
    /// or no length. The position is clamped to the song's length.
    pub fn remote_seek(&mut self, position_ms: f64) -> bool {
        let allowed = self.now_playing.as_ref().is_some_and(|s| s.dispatch(bifrost_app::now_playing::RemoteCommand::SeekTo).is_ok());
        if !allowed || self.is_live() {
            return false;
        }
        let Some(d) = self.snap.duration_ms else { return false };
        let ms = position_ms.clamp(0., d);
        self.player.seek_ms(ms);
        self.snap.position_ms = ms;
        self.sync_now_playing();
        true
    }

    /// Hands the queue to the engine in play order (shuffle is done here, so
    /// the engine's own shuffle stays off), keeping the position and the
    /// paused state, and saves it.
    fn push_queue(&mut self, position_ms: f64, playing: bool) {
        let tracks: Vec<Track> = self.queue.items().iter().filter_map(|p| self.path_id.get(p).map(|&id| Track::file(id as u64, p.clone()))).collect();
        self.player.play_queue(tracks, self.queue.current(), false, self.seed);
        if position_ms > 0.0 {
            self.player.seek_ms(position_ms);
        }
        if !playing {
            self.player.pause();
        }
        self.take_snapshot();
        self.save_queue();
        self.dirty = true;
    }

    fn save_queue(&self) {
        let Some(dir) = self.store.as_ref().filter(|_| !self.queue.is_empty()) else { return };
        let _ = std::fs::create_dir_all(dir);
        let tmp = dir.join(".queue.tmp");
        if std::fs::write(&tmp, self.queue.to_text(self.snap.position_ms)).is_ok() {
            let _ = std::fs::rename(&tmp, dir.join("queue.txt"));
        }
    }

    /// Puts the saved queue back, paused where it was, unless something plays already.
    fn restore_queue(&mut self) {
        let Some((mut queue, position)) = self.restore.take() else { return };
        queue.retain(|p| self.path_id.contains_key(p));
        if queue.is_empty() {
            return;
        }
        if self.snap.track.is_some() {
            // the service kept playing while the app was closed: show its queue, do not restart it
            if let Some(i) = self.snap.index.filter(|_| self.snap.queue_len == queue.items().len()) {
                queue.set_current(i);
                self.queue = queue;
                self.dirty = true;
            }
            return;
        }
        self.queue = queue;
        self.push_queue(position, false);
    }

    /// Adds a song after the current one (`next`) or at the end of the queue;
    /// with nothing queued it just plays.
    fn queue_add(&mut self, id: usize, next: bool) {
        if self.queue.is_empty() {
            self.play_songs(vec![id], 0, false);
            return;
        }
        let path = self.library.songs[id].path.clone();
        if next {
            self.queue.play_next(path);
        } else {
            self.queue.play_last(path);
        }
        self.push_queue(self.snap.position_ms, self.snap.playing);
    }

    fn queue_remove(&mut self, upcoming: usize) {
        if self.queue.remove_upcoming(upcoming) {
            self.push_queue(self.snap.position_ms, self.snap.playing);
        }
    }

    fn queue_move_up(&mut self, upcoming: usize) {
        if upcoming > 0 && self.queue.move_upcoming(upcoming, upcoming - 1) {
            self.push_queue(self.snap.position_ms, self.snap.playing);
        }
    }

    /// Tunes in the `i`-th station of the list as shown; the whole list is
    /// the queue, so Previous and Next step through the stations. The
    /// station already tuned is left alone, or tuned in again when paused
    /// (a paused stream is stale; resuming means going live).
    fn play_station(&mut self, i: usize) {
        let stations = self.radio.stations();
        let order = self.radio.order();
        let Some(&chosen) = order.get(i) else { return };
        let id = stations[chosen].id;
        if self.snap.track.as_ref().is_some_and(|t| t.is_live() && t.id == id) {
            self.resume_live();
            return;
        }
        let tracks: Vec<Track> = order.into_iter().map(|s| Track::live(stations[s].id, stations[s].stream_url.clone())).collect();
        self.player.play_queue(tracks, i, false, self.seed);
        self.take_snapshot();
    }

    /// Plays a paused live track again from the live edge by reopening the stream.
    fn resume_live(&mut self) {
        if !self.snap.playing {
            if let Some(i) = self.snap.index {
                self.player.jump_to(i);
            }
        }
        self.take_snapshot();
    }

    /// A transport control from any surface (sheet, mini player, lock
    /// screen, Control Center).
    pub fn control(&mut self, target: nowplaying::Target) {
        match target {
            nowplaying::Target::Play if self.is_live() && !self.snap.playing => self.resume_live(),
            nowplaying::Target::Play => {
                self.player.toggle();
                self.take_snapshot();
                self.save_queue();
            }
            nowplaying::Target::Prev => self.player.prev(),
            nowplaying::Target::Next => self.player.next(),
            nowplaying::Target::Shuffle if !self.is_live() && !self.queue.is_empty() => {
                let on = !self.queue.shuffled();
                self.next_seed();
                self.queue.set_shuffle(on, self.seed);
                self.push_queue(self.snap.position_ms, self.snap.playing);
            }
            nowplaying::Target::Repeat if !self.is_live() => self.player.set_repeat(self.snap.repeat.cycle()),
            _ => {}
        }
        self.take_snapshot();
        self.dirty = true;
    }

    /// The song id and, on a playlist page, (playlist, position) behind the
    /// `i`-th song row of the current page.
    fn song_at(&self, i: usize) -> Option<(usize, Option<(usize, usize)>)> {
        match (self.page(), self.tab) {
            (Page::Root, Tab::Songs) => self.view.get(i).map(|&id| (id, None)),
            (Page::Album(a), _) => self.library.albums[a].songs.get(i).map(|&id| (id, None)),
            (Page::Playlist(p), _) => self.playlist_entries(p).get(i).map(|&(at, id)| (id, Some((p, at)))),
            _ => None,
        }
    }

    /// (position in the playlist, song id) of the songs of playlist `p` the library has.
    fn playlist_entries(&self, p: usize) -> Vec<(usize, usize)> {
        let Some(list) = self.playlists.lists().get(p) else { return Vec::new() };
        list.songs.iter().enumerate().filter_map(|(at, path)| self.path_id.get(path).map(|&id| (at, id))).collect()
    }

    fn open_menu(&mut self, song: usize, from: Option<(usize, usize)>) {
        let mut items = vec![MenuItem::PlayNext, MenuItem::PlayLast];
        items.extend((0..self.playlists.lists().len()).map(MenuItem::AddTo));
        items.push(MenuItem::NewPlaylist);
        if let Some((p, at)) = from {
            items.push(MenuItem::RemoveFromPlaylist(p, at));
        }
        self.menu = Some(Menu { song, items });
    }

    fn run_menu(&mut self, item: MenuItem, song: usize) {
        let path = self.library.songs[song].path.clone();
        match item {
            MenuItem::PlayNext => self.queue_add(song, true),
            MenuItem::PlayLast => self.queue_add(song, false),
            MenuItem::AddTo(p) => {
                self.playlists.add(p, &path);
            }
            MenuItem::NewPlaylist => {
                let name = self.playlists.fresh_name();
                if let Some(p) = self.playlists.create(&name) {
                    self.playlists.add(p, &path);
                }
            }
            MenuItem::RemoveFromPlaylist(p, at) => {
                self.playlists.remove(p, at);
            }
        }
    }

    /// The song menu card in px.
    fn menu_rect(&self, ui: &Ui, rows: usize) -> R {
        let w = (self.w as f32 - 2.0 * ui.pt(MARGIN_PT)).min(ui.pt(MENU_MAX_W_PT));
        let h = ui.pt(MENU_HEAD_PT) + rows as f32 * ui.pt(MENU_ROW_PT);
        R { x: (self.w as f32 - w) / 2.0, y: ((self.h as f32 - h) / 2.0).max(ui.pt(top_inset())), w, h }
    }

    fn activate(&mut self, target: Target, t: f64) -> Vec<Effect> {
        match target {
            Target::Seg(i) => {
                let tab = [Tab::Artists, Tab::Albums, Tab::Songs, Tab::Radio, Tab::Playlists][i.min(4)];
                if tab != self.tab || self.pages.len() > 1 {
                    self.tab = tab;
                    self.pages.truncate(1);
                    self.renaming = None;
                    self.scroll.jump_top();
                }
                if tab != Tab::Songs && self.searching {
                    self.searching = false;
                    self.search.clear();
                    self.refilter();
                }
                if tab == Tab::Radio {
                    self.radio_refresh();
                }
            }
            Target::Back => {
                if self.pages.len() > 1 {
                    self.pages.pop();
                    self.scroll.jump_top();
                }
            }
            Target::Row(i) => match (self.page(), self.tab) {
                (Page::Root, Tab::Songs) => self.play_songs(self.view.clone(), i, false),
                (Page::Root, Tab::Playlists) => {
                    if i == 0 {
                        let name = self.playlists.fresh_name();
                        if let Some(p) = self.playlists.create(&name) {
                            self.push(Page::Playlist(p));
                        }
                    } else if i <= self.playlists.lists().len() {
                        self.push(Page::Playlist(i - 1));
                    }
                }
                (Page::Playlist(p), _) => {
                    let ids: Vec<usize> = self.playlist_entries(p).into_iter().map(|e| e.1).collect();
                    if i < ids.len() {
                        self.play_songs(ids, i, false);
                    }
                }
                (Page::Queue, _) => {
                    if i > 0 && i <= self.queue.upcoming().len() {
                        self.player.jump_to(self.queue.current() + i);
                        self.take_snapshot();
                    }
                }
                (Page::Root, Tab::Artists) => self.push(Page::Artist(i)),
                (Page::Root, Tab::Radio) => self.play_station(i),
                (Page::Root, Tab::Albums) => self.push(Page::Album(i)),
                (Page::Artist(a), _) => {
                    let album = self.library.artists[a].albums[i];
                    self.push(Page::Album(album));
                }
                (Page::Album(a), _) => self.play_songs(self.library.albums[a].songs.clone(), i, false),
            },
            Target::Fav(i) => {
                if let Some(&station) = self.radio.order().get(i) {
                    let id = self.radio.stations()[station].id;
                    self.radio.toggle_favourite(id);
                }
            }
            Target::Play | Target::Shuffle => match self.page() {
                Page::Album(a) => self.play_songs(self.library.albums[a].songs.clone(), 0, target == Target::Shuffle),
                Page::Playlist(p) => {
                    let ids: Vec<usize> = self.playlist_entries(p).into_iter().map(|e| e.1).collect();
                    if !ids.is_empty() {
                        self.play_songs(ids, 0, target == Target::Shuffle);
                    }
                }
                _ => {}
            },
            Target::Sort => {
                self.sort = self.sort.cycle();
                self.refilter();
            }
            Target::Search => {
                self.searching = !self.searching;
                if !self.searching {
                    self.search.clear();
                }
                self.refilter();
            }
            Target::More(i) => {
                if let Some((id, from)) = self.song_at(i) {
                    self.open_menu(id, from);
                }
            }
            Target::QUp(i) => self.queue_move_up(i.saturating_sub(1)),
            Target::QDel(i) => self.queue_remove(i.saturating_sub(1)),
            Target::Rename => {
                if let Page::Playlist(p) = self.page() {
                    self.renaming = self.playlists.lists().get(p).map(|l| l.name.clone());
                }
            }
            Target::DeletePlaylist => {
                if let Page::Playlist(p) = self.page() {
                    self.playlists.delete(p);
                    self.renaming = None;
                    self.pages.pop();
                    self.scroll.jump_top();
                }
            }
            Target::MenuItem(k) => {
                if let Some(menu) = self.menu.take() {
                    if let Some(&item) = menu.items.get(k) {
                        self.run_menu(item, menu.song);
                    }
                }
            }
            Target::MenuDismiss => self.menu = None,
            Target::Mini(nowplaying::Target::Open) | Target::Mini(nowplaying::Target::Artwork) => self.open_sheet(true, t),
            Target::Mini(nowplaying::Target::Queue) => {}
            Target::Mini(c) => self.control(c),
            Target::Np(nowplaying::Target::Dismiss) => self.open_sheet(false, t),
            Target::Np(nowplaying::Target::Queue) => {
                self.pages.push(Page::Queue);
                self.scroll.jump_top();
                self.open_sheet(false, t);
            }
            Target::Np(nowplaying::Target::Artwork | nowplaying::Target::Open | nowplaying::Target::Scrubber | nowplaying::Target::Volume) => {}
            Target::Np(c) => self.control(c),
            Target::Content => {}
        }
        self.dirty = true;
        vec![]
    }

    fn push(&mut self, page: Page) {
        self.pages.push(page);
        self.scroll.jump_top();
    }

    /// Advances scrolling, the sheet and artwork animations; true if a repaint is due.
    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        let mut moving = self.scroll.tick(t);
        self.fit_scroll(ui);
        if !self.scanning && t >= self.next_check {
            self.next_check = t + LIBRARY_CHECK_SECS;
            let (roots, load) = (self.roots.clone(), self.load.clone());
            self.pool.spawn(move || stamp_now(&roots, &load));
        }
        if let Some((_, to, start)) = self.np_anim {
            moving = true;
            if springboard::shell::gestures::progress(start, t, SHEET_MS) >= 1.0 {
                self.np_anim = None;
                self.np = to >= 1.0;
            }
        }
        if self.refresh(ui) {
            self.dirty = true;
        }
        let target = if self.snap.playing { 1.0 } else { nowplaying::PAUSED_SCALE };
        if (self.art_scale - target).abs() > 0.002 {
            self.art_scale += (target - self.art_scale) * 0.2;
            moving |= self.np || self.np_anim.is_some();
        } else {
            self.art_scale = target;
        }
        if !moving {
            self.request_art(ui);
            self.request_logos(ui);
            if self.tab == Tab::Radio && self.page() == Page::Root && !self.np {
                self.radio_refresh();
            }
        }
        self.dirty |= moving;
        self.dirty
    }

    // --- drawing ---

    fn label(&mut self, s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, x: f32, mid: f32, align: f32) -> f32 {
        if self.cache.len() > TEXT_CACHE_MAX {
            self.cache.clear();
        }
        let img = self.cache.get(ui, text, pt, weight, color);
        let (w, h) = (img.w as f32, img.h as f32);
        s.blit(img, (x - w * align) as i32, (mid - h / 2.0) as i32, 255, 255);
        w
    }

    fn art_for(&mut self, album: usize, kind: u8) -> Option<&Image> {
        self.art.get(&album.to_string(), kind)
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        self.dirty = false;
        self.drawn_sec = (self.snap.position_ms / 1000.0) as i64;
        s.fill_rect(0, 0, self.w as usize, self.h as usize, DETAIL_BG.now());
        let p = self.sheet_progress(t);
        if p < 1.0 {
            self.draw_library(s, ui);
            if self.has_track() {
                self.draw_mini(s, ui);
            }
        }
        if p > 0.0 {
            let dy = ((1.0 - p) * self.h as f32).round();
            self.draw_sheet(s, ui, dy);
        }
    }

    fn draw_library(&mut self, s: &mut Surface, ui: &Ui) {
        let (top, bottom) = (self.content_top(ui), self.content_bottom(ui));
        let off = self.scroll.offset();
        let pressed = self.pressed;
        let page = self.page();
        {
            let mut band = s.rows(top as usize, bottom as usize);
            let cy = |y: f32| y - off + top;
            match (page, self.tab) {
                (Page::Root, Tab::Songs) => {
                    let ids = self.view.clone();
                    self.draw_song_rows(&mut band, ui, &ids, cy(0.0), bottom, pressed, true);
                    if self.index_shown() {
                        let sec = browse::sections(&self.library.songs, &ids, self.sort);
                        let (y0, step) = self.index_geometry(ui);
                        for (i, c) in browse::INDEX.iter().enumerate() {
                            let color = if sec[i].is_some() { PINK } else { LABEL_3.now() };
                            self.label(&mut band, ui, &c.to_string(), MAC_SUBHEAD_PT, Weight::Semibold, color, self.w as f32 - ui.pt(SPACE_M), y0 + (i as f32 + 0.5) * step, 0.5);
                        }
                    }
                }
                (Page::Root, Tab::Playlists) => self.draw_playlists(&mut band, ui, cy(0.0), bottom, pressed),
                (Page::Playlist(p), _) => self.draw_playlist(&mut band, ui, p, cy(0.0), bottom, pressed),
                (Page::Queue, _) => self.draw_queue(&mut band, ui, cy(0.0), bottom, pressed),
                (Page::Root, Tab::Artists) => self.draw_artists(&mut band, ui, cy(0.0), bottom, pressed),
                (Page::Root, Tab::Radio) => self.draw_stations(&mut band, ui, cy(0.0), bottom, pressed),
                (Page::Root, Tab::Albums) => {
                    let albums: Vec<usize> = (0..self.library.albums.len()).collect();
                    self.draw_grid(&mut band, ui, &albums, cy(0.0), bottom, pressed);
                }
                (Page::Artist(a), _) => {
                    let albums = self.library.artists[a].albums.clone();
                    self.draw_grid(&mut band, ui, &albums, cy(0.0), bottom, pressed);
                }
                (Page::Album(a), _) => self.draw_album(&mut band, ui, a, cy(0.0), bottom, pressed),
            }
            // empty-state text sits on the middle of the content area, whatever its height
            let mid = (top + bottom) / 2.0;
            let radio = page == Page::Root && self.tab == Tab::Radio;
            if radio && self.radio.stations().is_empty() {
                let (text, hint) = match (self.radio.fetching, self.radio.error.is_some()) {
                    (true, _) => ("Loading stations…", ""),
                    (false, true) => ("No Stations", "Connect to Wi-Fi to load the radio stations"),
                    (false, false) => ("No Stations", ""),
                };
                self.label(&mut band, ui, text, MAC_TITLE1_PT, Weight::Semibold, LABEL_2.now(), self.w as f32 / 2.0, mid - ui.pt(SPACE_L), 0.5);
                self.label(&mut band, ui, hint, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), self.w as f32 / 2.0, mid + ui.pt(SPACE_L), 0.5);
            } else if page == Page::Root && self.tab == Tab::Songs && !self.library.is_empty() && self.view.is_empty() {
                self.label(&mut band, ui, "No Results", MAC_TITLE1_PT, Weight::Semibold, LABEL_2.now(), self.w as f32 / 2.0, mid - ui.pt(SPACE_L), 0.5);
            } else if page == Page::Queue && self.queue.items().is_empty() {
                self.label(&mut band, ui, "Nothing Queued", MAC_TITLE1_PT, Weight::Semibold, LABEL_2.now(), self.w as f32 / 2.0, mid - ui.pt(SPACE_L), 0.5);
            } else if page == Page::Root && self.tab == Tab::Playlists || matches!(page, Page::Playlist(_)) {
            } else if !radio && self.library.is_empty() {
                let text = if self.scanning { "Looking for music…" } else { "No Music" };
                self.label(&mut band, ui, text, MAC_TITLE1_PT, Weight::Semibold, LABEL_2.now(), self.w as f32 / 2.0, mid - ui.pt(SPACE_L), 0.5);
                if !self.scanning {
                    self.label(&mut band, ui, "Copy MP3 or M4A files to Music", MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), self.w as f32 / 2.0, mid + ui.pt(SPACE_L), 0.5);
                }
            }
        }
        s.fill_rect(0, 0, self.w as usize, top as usize, DETAIL_BG.now());
        s.fill_rect(0, bottom as usize, self.w as usize, (self.h as f32 - bottom).max(0.0) as usize, DETAIL_BG.now());
        if self.menu.is_some() {
            self.draw_menu(s, ui, pressed);
        }
    }

    /// Song rows: thumbnail (or track number), title, artist · album.
    #[allow(clippy::too_many_arguments)]
    fn draw_song_rows(&mut self, s: &mut Surface, ui: &Ui, ids: &[usize], y0: f32, bottom: f32, pressed: Option<Target>, thumbs: bool) {
        let row = ui.pt(if thumbs { SONG_ROW_PT } else { ALBUM_ROW_PT });
        let (m, thumb) = (ui.pt(MARGIN_PT), ui.pt(THUMB_PT));
        let text_x = if thumbs { m + thumb + ui.pt(SPACE_M) } else { m + ui.pt(36.0) };
        let playing = self.snap.track.as_ref().map(|t| t.id as usize);
        for (i, &id) in ids.iter().enumerate() {
            let y = y0 + i as f32 * row;
            if y + row < s.y0 as f32 || y > bottom {
                continue;
            }
            if pressed == Some(Target::Row(i)) {
                s.fill_rect(0, y.max(0.0) as usize, self.w as usize, row as usize, ROW_PRESSED.now());
            }
            let song = &self.library.songs[id];
            let (title, artist, track, dur) = (song.title.clone(), song.artist.clone(), song.track, song.duration_ms);
            let mid = y + row / 2.0;
            let is_current = playing == Some(id);
            if thumbs {
                let album_i = self.album_of[id];
                let ty = (y + (row - thumb) / 2.0) as i32;
                match self.art_for(album_i, THUMB) {
                    Some(img) => s.blit(img, m as i32, ty, 255, 255),
                    None => s.blit(&icons::placeholder(thumb as usize, ui.pt(SPACE_XS)), m as i32, ty, 255, 255),
                }
                let color = if is_current { PINK } else { LABEL.now() };
                let title = ui.fit(&title, MAC_BODY_PT, self.w as f32 - text_x - m - ui.pt(SONG_TRAIL_PT));
                self.label(s, ui, &title, MAC_BODY_PT, Weight::Regular, color, text_x, mid - ui.pt(SPACE_M), 0.0);
                if let Some(d) = dur {
                    self.label(s, ui, &clock(d), MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), self.w as f32 - ui.pt(60.0), mid, 1.0);
                }
                let sub = ui.fit(&artist, MAC_FOOTNOTE_PT, self.w as f32 - text_x - m - ui.pt(SONG_TRAIL_PT));
                self.label(s, ui, &sub, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), text_x, mid + ui.pt(SPACE_M), 0.0);
            } else {
                let num = track.map_or(String::new(), |n| n.to_string());
                if is_current {
                    icons::note(s, (m + ui.pt(SPACE_M), mid), ui.pt(SPACE_L), PINK);
                } else {
                    self.label(s, ui, &num, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), m + ui.pt(SPACE_L), mid, 0.5);
                }
                let color = if is_current { PINK } else { LABEL.now() };
                let title = ui.fit(&title, MAC_BODY_PT, self.w as f32 - text_x - m - ui.pt(60.0));
                self.label(s, ui, &title, MAC_BODY_PT, Weight::Regular, color, text_x, mid, 0.0);
                if let Some(d) = dur {
                    self.label(s, ui, &clock(d), MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), self.w as f32 - ui.pt(FAV_ZONE_PT), mid, 1.0);
                }
            }
            let dots = if pressed == Some(Target::More(i)) { LABEL_3.now() } else { LABEL_2.now() };
            Self::ellipsis(s, (self.w as f32 - m - ui.pt(SPACE_M), mid), ui.pt(SPACE_XXS), ui.pt(SPACE_S), dots);
            s.fill_rrect(text_x, y + row - 1.0, self.w as f32 - text_x, 1.0, 0.0, SEPARATOR_C.now());
        }
    }

    /// Three dots in a row centred on `c`.
    fn ellipsis(s: &mut Surface, c: (f32, f32), r: f32, gap: f32, color: u32) {
        for k in [-1.0f32, 0.0, 1.0] {
            let p = (c.0 + k * (gap + r), c.1);
            springboard::shell::glyphs::paint(s, (p.0 - r, p.1 - r, 2.0 * r, 2.0 * r), color, springboard::shell::glyphs::disc(p, r));
        }
    }

    /// Station rows: logo, name, "FM 98.9 · on air", and the favourite heart.
    fn draw_stations(&mut self, s: &mut Surface, ui: &Ui, y0: f32, bottom: f32, pressed: Option<Target>) {
        let row = ui.pt(STATION_ROW_PT);
        let (m, thumb) = (ui.pt(MARGIN_PT), ui.pt(THUMB_PT));
        let text_x = m + thumb + ui.pt(SPACE_M);
        let tuned = self.snap.track.as_ref().filter(|t| t.is_live()).map(|t| t.id);
        let live = self.snap.live.clone();
        for (i, index) in self.radio.order().into_iter().enumerate() {
            let y = y0 + i as f32 * row;
            if y + row < s.y0 as f32 || y > bottom {
                continue;
            }
            if pressed == Some(Target::Row(i)) {
                s.fill_rect(0, y.max(0.0) as usize, self.w as usize, row as usize, ROW_PRESSED.now());
            }
            let station = self.radio.stations()[index].clone();
            let mid = y + row / 2.0;
            let ty = (y + (row - thumb) / 2.0) as i32;
            match self.art.get(&logo_key(station.id), THUMB) {
                Some(img) => s.blit(img, m as i32, ty, 255, 255),
                None => s.blit(&icons::placeholder(thumb as usize, ui.pt(SPACE_XS)), m as i32, ty, 255, 255),
            }
            let is_tuned = tuned == Some(station.id);
            let room = self.w as f32 - text_x - ui.pt(FAV_ZONE_PT);
            let name = ui.fit(&station.name, MAC_BODY_PT, room);
            self.label(s, ui, &name, MAC_BODY_PT, Weight::Regular, if is_tuned { PINK } else { LABEL.now() }, text_x, mid - ui.pt(SPACE_M), 0.0);
            let on_air = if is_tuned { live.as_ref().and_then(|l| l.title.clone()) } else { None }.or(station.now.clone());
            let sub = [Some(station.frequency_text()).filter(|f| !f.is_empty()), on_air].into_iter().flatten().collect::<Vec<_>>().join(" · ");
            let sub = ui.fit(&sub, MAC_FOOTNOTE_PT, room);
            self.label(s, ui, &sub, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), text_x, mid + ui.pt(SPACE_M), 0.0);
            let fav = self.radio.is_favourite(station.id);
            let dim = if pressed == Some(Target::Fav(i)) { LABEL_3.now() } else if fav { PINK } else { LABEL_3.now() };
            icons::heart(s, (self.w as f32 - m - ui.pt(SPACE_M), mid), ui.pt(SPACE_XXL), dim);
            s.fill_rrect(text_x, y + row - 1.0, self.w as f32 - text_x, 1.0, 0.0, SEPARATOR_C.now());
        }
    }

    /// The Playlists tab: "New Playlist", then each playlist with its song count.
    fn draw_playlists(&mut self, s: &mut Surface, ui: &Ui, y0: f32, bottom: f32, pressed: Option<Target>) {
        let row = ui.pt(ARTIST_ROW_PT);
        let m = ui.pt(MARGIN_PT);
        let rows: Vec<(String, String)> = std::iter::once(("New Playlist".to_string(), String::new()))
            .chain(self.playlists.lists().iter().map(|l| (l.name.clone(), format!("{} {}", l.songs.len(), if l.songs.len() == 1 { "song" } else { "songs" }))))
            .collect();
        for (i, (name, sub)) in rows.into_iter().enumerate() {
            let y = y0 + i as f32 * row;
            if y + row < s.y0 as f32 || y > bottom {
                continue;
            }
            if pressed == Some(Target::Row(i)) {
                s.fill_rect(0, y.max(0.0) as usize, self.w as usize, row as usize, ROW_PRESSED.now());
            }
            let mid = y + row / 2.0;
            let text_x = m + ui.pt(SPACE_S);
            let name = ui.fit(&name, MAC_BODY_PT, self.w as f32 - text_x - ui.pt(60.0));
            if i == 0 {
                self.label(s, ui, &name, MAC_BODY_PT, Weight::Regular, PINK, text_x, mid, 0.0);
            } else {
                self.label(s, ui, &name, MAC_BODY_PT, Weight::Regular, LABEL.now(), text_x, mid - ui.pt(SPACE_M), 0.0);
                self.label(s, ui, &sub, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), text_x, mid + ui.pt(SPACE_M), 0.0);
                self.label(s, ui, "›", MAC_TITLE1_PT, Weight::Semibold, LABEL_3.now(), self.w as f32 - m, mid, 1.0);
            }
            s.fill_rrect(text_x, y + row - 1.0, self.w as f32 - text_x, 1.0, 0.0, SEPARATOR_C.now());
        }
    }

    /// A playlist page: Play and Shuffle, then its songs.
    fn draw_playlist(&mut self, s: &mut Surface, ui: &Ui, p: usize, y0: f32, bottom: f32, pressed: Option<Target>) {
        for (btn, (label, target)) in self.playlist_buttons(ui).into_iter().zip([("Play", Target::Play), ("Shuffle", Target::Shuffle)]) {
            let bg = if pressed == Some(target) { ROW_PRESSED.now() } else { CARD_BG.now() };
            let by = y0 + btn.y;
            s.fill_rrect(btn.x, by, btn.w, btn.h, ui.pt(BUTTON_H_PT / 2.0), bg);
            let (cx, cy) = (btn.x + btn.w / 2.0, by + btn.h / 2.0);
            self.label(s, ui, label, MAC_BODY_PT, Weight::Semibold, PINK, cx, cy, 0.5);
        }
        let ids: Vec<usize> = self.playlist_entries(p).into_iter().map(|e| e.1).collect();
        self.draw_song_rows(s, ui, &ids, y0 + ui.pt(PLAYLIST_HEADER_PT), bottom, pressed, true);
        if ids.is_empty() {
            self.label(s, ui, "No Songs", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), self.w as f32 / 2.0, y0 + ui.pt(PLAYLIST_HEADER_PT + 60.0), 0.5);
            self.label(s, ui, "Use ... on a song to add it", MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), self.w as f32 / 2.0, y0 + ui.pt(PLAYLIST_HEADER_PT + 88.0), 0.5);
        }
    }

    /// The queue page: the song playing (red), then Up Next, each with move-up and remove.
    fn draw_queue(&mut self, s: &mut Surface, ui: &Ui, y0: f32, bottom: f32, pressed: Option<Target>) {
        let row = ui.pt(SONG_ROW_PT);
        let (m, thumb, zone) = (ui.pt(MARGIN_PT), ui.pt(THUMB_PT), ui.pt(FAV_ZONE_PT));
        let text_x = m + thumb + ui.pt(SPACE_M);
        let current = self.queue.current();
        let items: Vec<PathBuf> = self.queue.items().iter().skip(current).cloned().collect();
        for (i, path) in items.into_iter().enumerate() {
            let y = y0 + i as f32 * row;
            if y + row < s.y0 as f32 || y > bottom {
                continue;
            }
            if pressed == Some(Target::Row(i)) {
                s.fill_rect(0, y.max(0.0) as usize, self.w as usize, row as usize, ROW_PRESSED.now());
            }
            let Some(&id) = self.path_id.get(&path) else { continue };
            let (title, sub) = (self.library.songs[id].title.clone(), format!("{} · {}", self.library.songs[id].artist, self.library.songs[id].album));
            let mid = y + row / 2.0;
            let ty = (y + (row - thumb) / 2.0) as i32;
            let album_i = self.album_of[id];
            match self.art_for(album_i, THUMB) {
                Some(img) => s.blit(img, m as i32, ty, 255, 255),
                None => s.blit(&icons::placeholder(thumb as usize, ui.pt(SPACE_XS)), m as i32, ty, 255, 255),
            }
            let room = self.w as f32 - text_x - if i == 0 { m } else { 2.0 * zone };
            let title = ui.fit(&title, MAC_BODY_PT, room);
            self.label(s, ui, &title, MAC_BODY_PT, Weight::Regular, if i == 0 { PINK } else { LABEL.now() }, text_x, mid - ui.pt(SPACE_M), 0.0);
            let sub = ui.fit(&sub, MAC_FOOTNOTE_PT, room);
            self.label(s, ui, &sub, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), text_x, mid + ui.pt(SPACE_M), 0.0);
            let (seg, either) = (springboard::shell::glyphs::segment, springboard::shell::glyphs::either);
            let (r, t) = (ui.pt(SPACE_S), ui.pt(SPACE_XXS));
            if i >= 1 {
                let c = (self.w as f32 - zone / 2.0, mid);
                let color = if pressed == Some(Target::QDel(i)) { LABEL_3.now() } else { LABEL_2.now() };
                springboard::shell::glyphs::paint(s, (c.0 - r - t, c.1 - r - t, 2.0 * r + 2.0 * t, 2.0 * r + 2.0 * t), color, either(seg((c.0 - r, c.1 - r), (c.0 + r, c.1 + r), t), seg((c.0 - r, c.1 + r), (c.0 + r, c.1 - r), t)));
            }
            if i >= 2 {
                let c = (self.w as f32 - zone * 1.5, mid);
                let color = if pressed == Some(Target::QUp(i)) { LABEL_3.now() } else { LABEL_2.now() };
                springboard::shell::glyphs::paint(s, (c.0 - r - t, c.1 - r - t, 2.0 * r + 2.0 * t, 2.0 * r + 2.0 * t), color, either(seg((c.0 - r, c.1 + r * 0.4), (c.0, c.1 - r * 0.6), t), seg((c.0, c.1 - r * 0.6), (c.0 + r, c.1 + r * 0.4), t)));
            }
            s.fill_rrect(text_x, y + row - 1.0, self.w as f32 - text_x, 1.0, 0.0, SEPARATOR_C.now());
        }
    }

    /// The song menu: the song's title, then Play Next / Play Last / Add to
    /// each playlist / New Playlist (/ Remove from this one).
    fn draw_menu(&mut self, s: &mut Surface, ui: &Ui, pressed: Option<Target>) {
        let Some((song, items)) = self.menu.as_ref().map(|m| (m.song, m.items.clone())) else { return };
        s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, black_at(150));
        let card = self.menu_rect(ui, items.len());
        s.fill_rrect(card.x, card.y, card.w, card.h, ui.pt(m::CARD_RADIUS), CARD_BG.now());
        let title = ui.fit(&self.library.songs[song].title, MAC_SUBHEAD_PT, card.w - ui.pt(SPACE_XXXL));
        self.label(s, ui, &title, MAC_SUBHEAD_PT, Weight::Semibold, LABEL_2.now(), card.x + card.w / 2.0, card.y + ui.pt(MENU_HEAD_PT) / 2.0, 0.5);
        for (k, item) in items.into_iter().enumerate() {
            let y = card.y + ui.pt(MENU_HEAD_PT) + k as f32 * ui.pt(MENU_ROW_PT);
            if pressed == Some(Target::MenuItem(k)) {
                s.fill_rect(card.x as usize, y as usize, card.w as usize, ui.pt(MENU_ROW_PT) as usize, ROW_PRESSED.now());
            }
            s.fill_rrect(card.x, y, card.w, 1.0, 0.0, SEPARATOR_C.now());
            let (text, color) = match item {
                MenuItem::PlayNext => ("Play Next".to_string(), LABEL.now()),
                MenuItem::PlayLast => ("Play Last".to_string(), LABEL.now()),
                MenuItem::AddTo(p) => (format!("Add to {}", self.playlists.lists().get(p).map_or("", |l| l.name.as_str())), LABEL.now()),
                MenuItem::NewPlaylist => ("Add to New Playlist".to_string(), LABEL.now()),
                MenuItem::RemoveFromPlaylist(..) => ("Remove from Playlist".to_string(), PINK),
            };
            let text = ui.fit(&text, MAC_BODY_PT, card.w - ui.pt(SPACE_HUGE));
            self.label(s, ui, &text, MAC_BODY_PT, Weight::Regular, color, card.x + ui.pt(SPACE_XL), y + ui.pt(MENU_ROW_PT) / 2.0, 0.0);
        }
    }

    fn draw_artists(&mut self, s: &mut Surface, ui: &Ui, y0: f32, bottom: f32, pressed: Option<Target>) {
        let row = ui.pt(ARTIST_ROW_PT);
        let (m, size) = (ui.pt(MARGIN_PT), ui.pt(THUMB_PT));
        for i in 0..self.library.artists.len() {
            let y = y0 + i as f32 * row;
            if y + row < s.y0 as f32 || y > bottom {
                continue;
            }
            if pressed == Some(Target::Row(i)) {
                s.fill_rect(0, y.max(0.0) as usize, self.w as usize, row as usize, ROW_PRESSED.now());
            }
            let (name, albums, count) = {
                let a = &self.library.artists[i];
                (a.name.clone(), a.albums.len(), a.song_count)
            };
            let mid = y + row / 2.0;
            let c = (m + size / 2.0, mid);
            springboard::shell::glyphs::paint(s, (c.0 - size / 2.0, c.1 - size / 2.0, size, size), FIELD_BG.now(), springboard::shell::glyphs::disc(c, size / 2.0));
            icons::note(s, c, size * 0.5, LABEL_2.now());
            let text_x = m + size + ui.pt(SPACE_L);
            let name = ui.fit(&name, MAC_BODY_PT, self.w as f32 - text_x - ui.pt(60.0));
            self.label(s, ui, &name, MAC_BODY_PT, Weight::Regular, LABEL.now(), text_x, mid - ui.pt(SPACE_M), 0.0);
            let sub = format!("{albums} {} · {count} {}", if albums == 1 { "album" } else { "albums" }, if count == 1 { "song" } else { "songs" });
            self.label(s, ui, &sub, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), text_x, mid + ui.pt(SPACE_M), 0.0);
            self.label(s, ui, "›", MAC_TITLE1_PT, Weight::Semibold, LABEL_3.now(), self.w as f32 - m, mid, 1.0);
            s.fill_rrect(text_x, y + row - 1.0, self.w as f32 - text_x, 1.0, 0.0, SEPARATOR_C.now());
        }
    }

    /// An album grid: covers with name and artist under each.
    fn draw_grid(&mut self, s: &mut Surface, ui: &Ui, albums: &[usize], y0: f32, bottom: f32, pressed: Option<Target>) {
        let cells = self.grid_cells(ui, albums.len());
        for (i, (&album, (x, y, w, h))) in albums.iter().zip(cells).enumerate() {
            let y = y0 + y;
            if y + h < s.y0 as f32 || y > bottom {
                continue;
            }
            let tint = if pressed == Some(Target::Row(i)) { 150 } else { 255 };
            match self.art_for(album, CELL_ART) {
                Some(img) => s.blit(img, x as i32, y as i32, tint, 255),
                None => s.blit(&icons::placeholder(w as usize, ui.pt(SPACE_S)), x as i32, y as i32, tint, 255),
            }
            let (name, artist) = {
                let a = &self.library.albums[album];
                (ui.fit(&a.name, MAC_SUBHEAD_PT, w), ui.fit(&a.artist, MAC_FOOTNOTE_PT, w))
            };
            self.label(s, ui, &name, MAC_SUBHEAD_PT, Weight::Regular, LABEL.now(), x, y + w + ui.pt(SPACE_M), 0.0);
            self.label(s, ui, &artist, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x, y + w + ui.pt(30.0), 0.0);
        }
    }

    /// The album page: cover, titles, Play / Shuffle, then its songs.
    #[allow(clippy::too_many_arguments)]
    fn draw_album(&mut self, s: &mut Surface, ui: &Ui, a: usize, y0: f32, bottom: f32, pressed: Option<Target>) {
        let (m, cover) = (ui.pt(MARGIN_PT), ui.pt(ALBUM_COVER_PT));
        let (x, y) = (m, y0 + ui.pt(SPACE_L));
        let r = Rect { x: x as i32, y: y as i32, w: cover as i32, h: cover as i32 };
        match self.art_for(a, CELL_ART).cloned() {
            Some(img) => draw_scaled(s, &img, r, ui.pt(SPACE_S), 255),
            None => s.blit(&icons::placeholder(cover as usize, ui.pt(SPACE_S)), x as i32, y as i32, 255, 255),
        }
        let (name, artist, year, songs) = {
            let al = &self.library.albums[a];
            (al.name.clone(), al.artist.clone(), al.year, al.songs.clone())
        };
        let tx = x + cover + ui.pt(SPACE_XL);
        let tw = self.w as f32 - tx - m;
        let name = ui.fit(&name, MAC_TITLE1_PT, tw);
        self.label(s, ui, &name, MAC_TITLE1_PT, Weight::Bold, LABEL.now(), tx, y + ui.pt(SPACE_L), 0.0);
        let artist = ui.fit(&artist, MAC_BODY_PT, tw);
        self.label(s, ui, &artist, MAC_BODY_PT, Weight::Regular, PINK, tx, y + ui.pt(44.0), 0.0);
        let total: f64 = songs.iter().filter_map(|&id| self.library.songs[id].duration_ms).sum();
        let mut info = format!("{} {}", songs.len(), if songs.len() == 1 { "song" } else { "songs" });
        if let Some(y) = year {
            info = format!("{y} · {info}");
        }
        if total > 0.0 {
            info = format!("{info}, {} min", (total / 60_000.0).round().max(1.0));
        }
        self.label(s, ui, &info, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), tx, y + ui.pt(68.0), 0.0);
        for (btn, (label, target)) in self.album_buttons(ui).into_iter().zip([("Play", Target::Play), ("Shuffle", Target::Shuffle)]) {
            let bg = if pressed == Some(target) { ROW_PRESSED.now() } else { CARD_BG.now() };
            let by = y0 + btn.y;
            s.fill_rrect(btn.x, by, btn.w, btn.h, ui.pt(BUTTON_H_PT / 2.0), bg);
            let (cx, cy) = (btn.x + btn.w / 2.0, by + btn.h / 2.0);
            let lw = ui.measure(label, MAC_BODY_PT, Weight::Semibold);
            let gx = cx - lw / 2.0 - ui.pt(SPACE_S);
            if target == Target::Play {
                icons::play(s, (gx, cy), ui.pt(SPACE_L), PINK);
            } else {
                icons::shuffle(s, (gx, cy), ui.pt(SPACE_XL), ui.pt(SPACE_XXS), PINK);
            }
            self.label(s, ui, label, MAC_BODY_PT, Weight::Semibold, PINK, cx + ui.pt(SPACE_M), cy, 0.5);
        }
        self.draw_song_rows(s, ui, &songs, y0 + ui.pt(ALBUM_HEADER_PT), bottom, pressed, false);
    }

    /// The mini player bar above the home indicator.
    fn draw_mini(&mut self, s: &mut Surface, ui: &Ui) {
        let mini = Mini::new(ui, self.w, self.h);
        s.fill_rect(0, mini.bar.y as usize, self.w as usize, (self.h as f32 - mini.bar.y) as usize, ROW_PRESSED.now());
        s.fill_rrect(0.0, mini.bar.y, self.w as f32, 1.0, 0.0, SEPARATOR_C.now());
        let key = self.snap.track.as_ref().and_then(|t| self.art_key(t));
        let art = key.and_then(|k| self.art.get(&k, THUMB).cloned());
        match art {
            Some(img) => s.blit(&img, mini.art.x as i32, mini.art.y as i32, 255, 255),
            None => s.blit(&icons::placeholder(mini.art.w as usize, ui.pt(SPACE_XS)), mini.art.x as i32, mini.art.y as i32, 255, 255),
        }
        if let Some(l) = &self.labels {
            // title over artist, centred as a pair like the bifrost mini player
            let gap = ui.pt(SPACE_XXS);
            let pair = l.mini.h as f32 + gap + l.mini_artist.h as f32;
            let ty = mini.bar.y + (mini.bar.h - pair) / 2.0;
            s.blit_colored(&l.mini, mini.text_x(ui) as i32, ty as i32, LABEL.now(), 255);
            s.blit_colored(&l.mini_artist, mini.text_x(ui) as i32, (ty + l.mini.h as f32 + gap) as i32, LABEL_2.now(), 255);
        }
        let dim = |t: nowplaying::Target| if self.pressed == Some(Target::Mini(t)) { LABEL_3.now() } else { LABEL.now() };
        icons::play_pause(s, mini.play, ui.pt(SPACE_XXL), self.snap.playing, dim(nowplaying::Target::Play));
        icons::skip(s, mini.next, ui.pt(SPACE_XL), true, dim(nowplaying::Target::Next));
    }

    /// The full-screen Now Playing sheet, offset down by `dy` px while sliding.
    fn draw_sheet(&mut self, s: &mut Surface, ui: &Ui, dy: f32) {
        let l = Layout::new(ui, self.w, self.h);
        let key = self.snap.track.as_ref().and_then(|t| self.art_key(t));
        let big = key.and_then(|k| self.art.get(&k, BIG).cloned());
        let bg = big.as_ref().map_or(SHEET, |img| tint_of(img));
        s.fill_rrect(0.0, dy, self.w as f32, self.h as f32, ui.pt(m::CARD_RADIUS), bg);
        s.fill_rrect(l.handle.x, l.handle.y + dy, l.handle.w, l.handle.h, l.handle.h / 2.0, white_at(90));
        let art = l.art_at(self.art_scale);
        let r = Rect { x: art.x as i32, y: (art.y + dy) as i32, w: art.w as i32, h: art.h as i32 };
        // a soft shadow under the picture
        for i in 1..=4 {
            let k = i as f32 * ui.pt(SPACE_XS);
            s.fill_rrect(art.x - k, art.y + dy - k + ui.pt(SPACE_S), art.w + 2.0 * k, art.h + 2.0 * k, ui.pt(ART_RADIUS) + k, black_at(22));
        }
        match big {
            Some(img) => draw_scaled(s, &img, r, ui.pt(ART_RADIUS), 255),
            None => draw_scaled(s, &icons::placeholder(64, 0.0), r, ui.pt(ART_RADIUS), 255),
        }
        // artwork-tinted depth: the lower half fades toward black, like the blurred bifrost backdrop
        for i in 0..8 {
            let y = self.h as f32 * (0.4 + 0.075 * i as f32);
            s.fill_rrect(0.0, y + dy, self.w as f32, self.h as f32 - y, 0.0, black_at(26));
        }
        let side = ui.pt(nowplaying::SIDE_PT);
        if let Some(lab) = &self.labels {
            s.blit(&lab.title, side as i32, (l.title_y + dy - lab.title.h as f32 / 2.0) as i32, 255, 255);
            s.blit(&lab.artist, side as i32, (l.artist_y + dy - lab.artist.h as f32 / 2.0) as i32, 255, 255);
        }
        let dim = white_at(150);
        let live = self.is_live();
        if live {
            // a live stream has no timeline: just say so
            self.label(s, ui, "LIVE", MAC_CAPTION1_PT, Weight::Semibold, dim, self.w as f32 / 2.0, l.times_y + dy - ui.pt(SPACE_S), 0.5);
        } else {
            // scrubber
            let (pos, dur) = (self.snap.position_ms, self.snap.duration_ms.unwrap_or(0.0));
            let frac = match self.scrub {
                Some((f, Surface_::Sheet)) => f,
                _ if dur > 0.0 => (pos / dur) as f32,
                _ => 0.0,
            };
            draw_slider(s, ui, R { y: l.scrub.y + dy, ..l.scrub }, frac, self.scrub.is_some_and(|(_, w)| w == Surface_::Sheet));
            let shown = if self.scrub.is_some() { frac as f64 * dur } else { pos };
            self.label(s, ui, &clock(shown), MAC_CAPTION1_PT, Weight::Medium, dim, l.scrub.x, l.times_y + dy, 0.0);
            let remaining = if dur > 0.0 { format!("-{}", clock(dur - shown)) } else { "-:--".into() };
            self.label(s, ui, &remaining, MAC_CAPTION1_PT, Weight::Medium, dim, l.scrub.x + l.scrub.w, l.times_y + dy, 1.0);
        }
        // transport
        let pressed = |t: nowplaying::Target, me: &Self| if me.pressed == Some(Target::Np(t)) { white_at(140) } else { WHITE };
        icons::skip(s, (l.prev.0, l.prev.1 + dy), ui.pt(nowplaying::SKIP_PT), false, pressed(nowplaying::Target::Prev, self));
        icons::play_pause(s, (l.play.0, l.play.1 + dy), ui.pt(nowplaying::PLAY_PT), self.snap.playing, pressed(nowplaying::Target::Play, self));
        icons::skip(s, (l.next.0, l.next.1 + dy), ui.pt(nowplaying::SKIP_PT), true, pressed(nowplaying::Target::Next, self));
        // volume
        let vol = R { y: l.volume.y + dy, ..l.volume };
        draw_slider(s, ui, vol, self.volume, false);
        icons::speaker(s, (vol.x - ui.pt(SPACE_XL), vol.y + vol.h / 2.0), ui.pt(SPACE_L), 0, dim);
        icons::speaker(s, (vol.x + vol.w + ui.pt(SPACE_L), vol.y + vol.h / 2.0), ui.pt(SPACE_L), 2, dim);
        // route glyph (AirPlay-like): triangle under two arcs, beside Up Next
        {
            let c = (self.w as f32 - side - ui.pt(SPACE_L), l.queue.1 + dy);
            let k = ui.pt(SPACE_S);
            icons::fill_poly(s, &[(c.0 - k * 0.7, c.1 + k), (c.0 + k * 0.7, c.1 + k), (c.0, c.1 + k * 0.1)], dim);
            s.fill_rrect(c.0 - k, c.1 - k, 2.0 * k, ui.pt(1.8), 0.0, dim);
            s.fill_rrect(c.0 - k, c.1 - k, ui.pt(1.8), k * 1.3, 0.0, dim);
            s.fill_rrect(c.0 + k - ui.pt(1.8), c.1 - k, ui.pt(1.8), k * 1.3, 0.0, dim);
        }
        // Up Next
        if !live {
            let c = (l.queue.0, l.queue.1 + dy);
            let color = if self.pressed == Some(Target::Np(nowplaying::Target::Queue)) { white_at(140) } else { WHITE };
            for k in 0..3 {
                s.fill_rrect(c.0 - ui.pt(SPACE_M), c.1 - ui.pt(SPACE_S) + k as f32 * ui.pt(SPACE_S), ui.pt(SPACE_XXL), ui.pt(SPACE_XXS), 0.0, color);
            }
        }
        // shuffle and repeat (a stream has neither)
        for (c, on, t) in [(l.shuffle, self.snap.shuffle, nowplaying::Target::Shuffle), (l.repeat, self.snap.repeat != Repeat::Off, nowplaying::Target::Repeat)].into_iter().filter(|_| !live) {
            let c = (c.0, c.1 + dy);
            if on {
                s.fill_rrect(c.0 - ui.pt(SPACE_XXL), c.1 - ui.pt(SPACE_L), ui.pt(44.0), ui.pt(SPACE_XXXL), ui.pt(SPACE_S), white_at(70));
            }
            let color = if on { WHITE } else { pressed(t, self) };
            let color = if self.pressed == Some(Target::Np(t)) { white_at(140) } else { color };
            if t == nowplaying::Target::Shuffle {
                icons::shuffle(s, c, ui.pt(SPACE_XXL), ui.pt(SPACE_XXS), color);
            } else {
                icons::repeat(s, c, ui.pt(SPACE_XXL), ui.pt(SPACE_XXS), self.snap.repeat == Repeat::One, color);
            }
        }
    }
}

const ART_RADIUS: f32 = nowplaying::ART_RADIUS_PT;

/// A dark background colour drawn from the artwork's average, as bifrost tints
/// the sheet with the picture.
pub fn tint_of(img: &Image) -> u32 {
    if img.px.is_empty() {
        return SHEET;
    }
    let mut sum = [0u64; 3];
    for &p in img.px.iter().step_by(7) {
        for (c, s) in sum.iter_mut().enumerate() {
            *s += ((p >> (c * 8)) & 255) as u64;
        }
    }
    let n = img.px.len().div_ceil(7) as u64;
    let ch = |c: usize| ((sum[c] / n) as f32 * 0.32 + 14.0) as u8;
    pack(ch(0), ch(1), ch(2), 255)
}

/// Targets that live inside the scrolling area.
fn scrolls(t: Target) -> bool {
    matches!(t, Target::Row(_) | Target::Fav(_) | Target::More(_) | Target::QUp(_) | Target::QDel(_) | Target::Content | Target::Play | Target::Shuffle)
}

/// The artwork cache key of a station's logo.
fn logo_key(id: u64) -> String {
    format!("r{id}")
}
