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
pub mod folders;
pub mod info;
pub mod jobs;
pub mod library;
pub mod lyrics;
pub mod menus;
pub mod nowplay;
pub mod playlists;
pub mod smart;
pub mod stars;
pub mod stats;
pub mod system;
pub mod tempo;
pub mod equalizer;
pub mod upnext;
pub mod miniplayer;
pub mod playback;
pub mod retry;
pub mod visualizer;

use bifrost_app::layout::{top_inset, UNIFIED_BAR_PT};
use bifrost_app::kit::ui::{CARD_BG, CONTENT_BG, FIELD_BG, LABEL, LABEL_2, LABEL_3, ROW_PRESSED, SEPARATOR_C};
use bifrost_app::kit::ui::{SPACE_HUGE, SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XS, SPACE_XXL, SPACE_XXS, SPACE_XXXL};
use bifrost_app::kit::ui::{MAC_BODY_PT, MAC_CAPTION1_PT, MAC_TITLE1_PT};
use bifrost_app::kit::ui::{MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT, MAC_TITLE3_PT};
use bifrost_app::kit::Event;
use browse::{Scope, SortKey};
use bifrost_app::audio::Client;
use bifrost_app::proto::sys::now_ms;
use engine::{AudioPlayer, LiveStatus, Phase, Repeat, Snapshot, Track};
use folders::{Folders, Row as TabRow};
use info::{Field, Info};
use library::Library;
use lyrics::Lyrics;
use menus::{Command, MenuState, Place};
use playlists::Playlists;
use smart::{Kind, SmartPlaylists};
use stats::{PlayEvent, PlayTracker, Stats};
use springboard::deildu::art::{self, Cache};
use bifrost_app::controls::prefs_layout::m;
use bifrost_app::dispatch::Qos;
use bifrost_app::kit::motion;
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
/// Where the tempo column of a song row ends, from the window's trailing edge (left of the duration).
const BPM_RIGHT_PT: f32 = 96.0;
/// Tempo analyses run at once, at most; each reads the start of one song.
const TEMPO_JOBS: usize = 2;
/// How much Volume Up and Volume Down in the Controls menu move the volume (a fraction of full scale).
const VOLUME_STEP: f32 = 0.1;
/// A song row's favourite heart sits this far from the trailing edge; its rating stars have this radius and pitch.
const FAVOURITE_X_PT: f32 = 50.0;
const STAR_R_PT: f32 = 5.0;
const STAR_PITCH_PT: f32 = 11.0;
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
/// text images kept before the cache is emptied
const TEXT_CACHE_MAX: usize = 800;

/// The Now Playing sheet's slide from its open or close time (ms on the app clock): `PanelReveal`, which snaps
/// under Reduce Motion.
fn sheet_slide(start_ms: f64) -> motion::Anim {
    motion::Anim::new(0.0, 1.0, start_ms / 1000.0, motion::Kind::PanelReveal)
}
/// artwork kinds: list thumbnail, grid cell, the Now Playing picture
pub const THUMB: u8 = 0;
pub const CELL_ART: u8 = 1;
pub const BIG: u8 = 2;
/// the kind of a Control Center cover decoded from the song's embedded picture (not a cache kind)
const EMBEDDED: u8 = 3;

const PLAYLIST_HEADER_PT: f32 = 68.0;
const MENU_ROW_PT: f32 = 48.0;
const MENU_HEAD_PT: f32 = 44.0;
/// The route button of the mini player bar sits this far left of Play; its touch reach is MINI_ROUTE_HIT_PT.
const MINI_ROUTE_GAP_PT: f32 = 56.0;
const MINI_ROUTE_HIT_PT: f32 = 24.0;
/// The mini player bar's volume slider: its length, its gap to the route button, and the narrowest bar that has one.
const MINI_VOLUME_PT: f32 = 72.0;
const MINI_VOLUME_GAP_PT: f32 = 24.0;
const MINI_VOLUME_MIN_BAR_PT: f32 = 480.0;
/// The room the elapsed time (or LIVE and the time on air) needs left of the volume slider, and the room the title
/// keeps before the time is shown at all.
const MINI_STATUS_MIN_PT: f32 = 96.0;
const MINI_TITLE_MIN_PT: f32 = 120.0;

/// The mini player bar's volume track, left of the route button; None when the bar is too narrow to hold one.
fn mini_volume_track(ui: &Ui, mini: &Mini) -> Option<R> {
    if mini.bar.w < ui.pt(MINI_VOLUME_MIN_BAR_PT) {
        return None;
    }
    let route_x = mini.play.0 - ui.pt(MINI_ROUTE_GAP_PT);
    let w = ui.pt(MINI_VOLUME_PT);
    let h = ui.pt(SPACE_XXS);
    Some(R { x: route_x - ui.pt(MINI_VOLUME_GAP_PT) - w, y: mini.play.1 - h / 2.0, w, h })
}

/// How wide the mini player bar's title and second line may be: up to the time and the volume slider when the bar has
/// them, else up to the buttons.
fn mini_text_w(ui: &Ui, w: i32, h: i32) -> f32 {
    let mini = Mini::new(ui, w, h);
    match (mini_volume_track(ui, &mini), mini_status_right(ui, &mini)) {
        (Some(_), Some(right)) => right - ui.pt(MINI_STATUS_MIN_PT) - ui.pt(SPACE_L) - mini.text_x(ui),
        (Some(track), None) => track.x - ui.pt(SPACE_L) - mini.text_x(ui),
        _ => w as f32 - ui.pt(MINI_TEXT_PT),
    }
}

/// Where the mini player bar's time text ends (it is right-aligned there); None when the bar has no room for it.
fn mini_status_right(ui: &Ui, mini: &Mini) -> Option<f32> {
    let track = mini_volume_track(ui, mini)?;
    let right = track.x - ui.pt(SPACE_L);
    (right - mini.text_x(ui) >= ui.pt(MINI_STATUS_MIN_PT + MINI_TITLE_MIN_PT) + ui.pt(SPACE_L)).then_some(right)
}

/// The title and the hint of an empty library, per Library section: what is missing, and how to add music (File >
/// Import, or files copied to the Music folder). Nothing is hinted while the folder is still being read.
fn library_empty_text(tab: Tab, scope: Scope, scanning: bool) -> (&'static str, &'static str) {
    if scanning {
        return ("Looking for music\u{2026}", "");
    }
    const ADD: &str = "Choose File > Import\u{2026} (\u{2318}O) or copy MP3 or M4A files to your Music folder";
    let title = match (tab, scope) {
        (Tab::Artists, _) => "No Artists",
        (Tab::Albums, _) => "No Albums",
        (Tab::Songs, Scope::RecentlyAdded) => "Nothing Added Yet",
        (Tab::Songs, Scope::RecentlyPlayed) => "Nothing Played Yet",
        (Tab::Songs, Scope::Favourites) => "No Favourites",
        _ => "No Songs",
    };
    (title, ADD)
}

/// What the mini player bar says about the time: a song's elapsed and total time, a station's LIVE and how long it
/// has been listened to.
fn mini_status(live: bool, position_ms: f64, duration_ms: Option<f64>) -> String {
    if live {
        return format!("LIVE \u{00b7} {}", clock(position_ms));
    }
    match duration_ms.filter(|d| *d > 0.0) {
        Some(d) => format!("{} / {}", clock(position_ms.min(d)), clock(d)),
        None => clock(position_ms),
    }
}
/// The Get Info page's rows: Title, Artist, Album, Restore Original Tags, File, Length.
const INFO_ROWS: usize = 6;
/// The width of the label column on the Get Info page.
const INFO_LABEL_PT: f32 = 110.0;

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
    /// the songs that played lately, most recent first
    History,
    /// a smart playlist's songs
    Smart(usize),
    /// a smart playlist's rules
    SmartEdit(usize),
    /// Get Info for the song at `Music::info_path`
    Info,
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
    /// the grab handle of the i-th queue row: drag it to another place
    QDrag(usize),
    /// the Clear Up Next row at the end of the queue: takes every upcoming song out
    QClear,
    /// the playlist page's nav bar: rename, delete
    Rename,
    DeletePlaylist,
    /// open the rules editor of the smart playlist shown
    EditRules,
    /// the rules editor's rows: the match mode, the field, operator and value of the i-th rule, its remove button, and
    /// Add Rule
    MatchToggle,
    RuleField(usize),
    RuleOp(usize),
    RuleValue(usize),
    RuleDel(usize),
    RuleAdd,
    /// the lyrics switch and the output route on the Now Playing sheet
    Lyrics,
    Route,
    /// the route button of the mini player bar
    MiniRoute,
    /// the volume slider of the mini player bar
    MiniVolume,
    /// Get Info: the i-th field (Title, Artist, Album), and Restore Original Tags
    InfoField(usize),
    InfoReset,
    /// the Move to Folder button of a playlist page
    MoveToFolder,
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
    /// step the rating: none, 1 to 5 stars, none
    Rate,
    ToggleFavourite,
    AddTo(usize),
    NewPlaylist,
    /// (playlist, position in it)
    RemoveFromPlaylist(usize, usize),
    /// the k-th output device of `Music::outputs`
    Output(usize),
    /// Get Info for the song the menu is open on
    GetInfo,
    /// put the playlist on screen in a folder (an index into the folders), or in none
    MoveToFolder(Option<usize>),
}

/// The action sheet opened by a song row's "..." (or by the route button of the Now Playing sheet).
struct Menu {
    song: usize,
    items: Vec<MenuItem>,
    /// the title over the rows; a song's title when None
    heading: Option<String>,
}

/// A queue row being dragged: which row, and where the finger is (px below the top of the list).
#[derive(Clone, Copy)]
struct QueueDrag {
    from: usize,
    y: f32,
}

enum Done {
    /// A full scan: the scanned files and the change stamps of their files.
    Scanned(Vec<library::Scanned>, Vec<Stamp>),
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

/// Where the app's repaints came from since the last report: finished jobs, the audio service's state, motion
/// (scrolling, easing). With `BIFROST_FRAME_STATS=1` one line a second names them while the app keeps repainting, so
/// an app that draws without cause shows who asked.
#[derive(Default)]
pub struct RepaintWhy {
    jobs: u32,
    player: u32,
    motion: u32,
    asked: u32,
    since: f64,
}

impl RepaintWhy {
    /// Counts a turn that asked for a repaint (`asked`) and, once a second, prints the sources when the stats are on.
    fn report(&mut self, t: f64, asked: bool) {
        self.asked += asked as u32;
        if t - self.since < 1000.0 {
            return;
        }
        if self.asked > 4 && std::env::var_os("BIFROST_FRAME_STATS").is_some_and(|v| v == "1") {
            eprintln!("music: {} turns asked for a repaint in the last second (jobs {}, audio service {}, motion {})", self.asked, self.jobs, self.player, self.motion);
        }
        *self = RepaintWhy { since: t, ..RepaintWhy::default() };
    }
}

pub struct Music {
    w: i32,
    h: i32,
    pool: Jobs<Done>,
    /// tempo jobs: the beats per minute of a song, by its path (`music/tempo.rs`)
    tempo: Jobs<(PathBuf, f64)>,
    bpm: HashMap<PathBuf, f64>,
    bpm_pending: HashSet<PathBuf>,
    /// network jobs (station list, logos), apart from `pool` so a slow
    /// connection never holds up the library
    net: Jobs<Done>,
    fetcher: Arc<dyn Fetch>,
    radio: Radio,
    /// logos being fetched, and logos that could not be had (until the next refresh)
    logo_pending: HashSet<(u64, u8)>,
    logo_failed: HashSet<(u64, u8)>,
    player: AudioPlayer,
    /// why the last playback command could not be sent (the audio service is down), until it connects
    notice: Option<String>,
    /// the station tune-in to send again while the audio service cannot be reached (`retry`)
    retry: retry::Retry,
    /// where the app's repaints came from in the last second (shown with `BIFROST_FRAME_STATS=1`)
    why: RepaintWhy,
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
    /// the current song's cover as sent to Control Center, with the cache key and kind it came from
    np_art: Option<(String, u8, bifrost_app::now_playing::ArtworkImage)>,
    /// the cache key whose embedded picture would not decode: not tried again, the cached thumbnail is used
    np_embedded_failed: Option<String>,
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
    /// the volume track a finger is dragging: the Now Playing sheet's or the mini player bar's
    vol_drag: Option<R>,
    volume: f32,
    seed: u64,
    /// Songs tab: the order and filter shown (ids into the library)
    view: Vec<usize>,
    sort: SortKey,
    /// the Songs list runs the sort backwards (View > Sort Songs By > Descending)
    descending: bool,
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
    /// the playlist folders, and the folders hidden in the Playlists tab (by name)
    folders: Folders,
    collapsed: Vec<String>,
    /// the smart playlists, the songs of the one shown (`sync_smart`), and what that list was worked out for: the
    /// playlist and `smart_rev`, which moves whenever rules, ratings, plays or the library change
    smart: SmartPlaylists,
    smart_view: Vec<usize>,
    smart_key: Option<(usize, u64)>,
    smart_rev: u64,
    /// a rule value being typed: the rule and the text so far
    rule_edit: Option<(usize, String)>,
    /// ratings, favourites, play counts and the play history, and the logic that counts plays
    stats: Stats,
    /// the playback settings (crossfade), kept in `playback.txt` of the library store
    playback: playback::Playback,
    tracker: PlayTracker,
    /// which list the Songs tab shows (all songs, recently added, recently played, favourites)
    scope: Scope,
    /// the lyrics beside the playing song, which song they were read for, and whether the Now Playing sheet shows them
    lyrics: Option<Lyrics>,
    lyrics_for: Option<PathBuf>,
    lyrics_on: bool,
    /// the Up Next row being dragged
    qdrag: Option<QueueDrag>,
    /// the edits of Get Info, the path of the song its page shows, and the field being typed
    info: Info,
    info_path: Option<PathBuf>,
    info_edit: Option<(Field, String)>,
    /// output devices as (id, name), the default one, and the hand-offs to the window: it fetches the list when
    /// `output_request` is set and applies `output_choice`
    outputs: Vec<(String, String)>,
    output_now: Option<String>,
    output_request: bool,
    output_choice: Option<String>,
    /// the clock time of the last tick or touch, for actions that start from a menu
    last_t: f64,
    /// a playlist name being typed
    renaming: Option<String>,
    /// Cmd+A chose the whole name: the next typed text or Backspace replaces or clears it
    rename_selected: bool,
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
    Done::Scanned(library::scan_all(&files), stamps)
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
            tempo: Jobs::new(Qos::Utility),
            bpm: HashMap::new(),
            bpm_pending: HashSet::new(),
            fetcher: Arc::new(radio::Http::default()),
            radio: Radio::default(),
            logo_pending: HashSet::new(),
            logo_failed: HashSet::new(),
            player,
            notice: None,
            retry: retry::Retry::default(),
            why: RepaintWhy::default(),
            library: Library::default(),
            album_of: Vec::new(),
            scanning: true,
            tab: Tab::Songs,
            pages: vec![Page::Root],
            scroll: Scroller::new(),
            indexing: false,
            art: Cache::new(art::BUDGET / 2),
            np_art: None,
            np_embedded_failed: None,
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
            vol_drag: None,
            volume,
            seed: 0x5EED,
            view: Vec::new(),
            sort: SortKey::Title,
            descending: false,
            search: String::new(),
            searching: false,
            queue: UpNext::default(),
            path_id: HashMap::new(),
            store: None,
            restore: None,
            playlists: Playlists::default(),
            folders: Folders::default(),
            collapsed: Vec::new(),
            smart: SmartPlaylists::default(),
            smart_view: Vec::new(),
            smart_key: None,
            smart_rev: 0,
            rule_edit: None,
            stats: Stats::default(),
            playback: playback::Playback::default(),
            tracker: PlayTracker::default(),
            scope: Scope::All,
            lyrics: None,
            lyrics_for: None,
            lyrics_on: false,
            qdrag: None,
            info: Info::default(),
            info_path: None,
            info_edit: None,
            outputs: Vec::new(),
            output_now: None,
            output_request: false,
            output_choice: None,
            last_t: 0.0,
            renaming: None,
            rename_selected: false,
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

    /// Keeps the play queue, the playlists and the listening data in `dir`: playlists as `.m3u`
    /// files in `dir/Playlists`, smart playlists as `.smart` files in `dir/Smart`, ratings, favourites, plays and
    /// the history in `dir/stats.txt`, the queue in `dir/queue.txt`. The saved
    /// queue comes back paused once the library is scanned.
    pub fn enable_library_store(&mut self, dir: PathBuf) {
        self.playlists = Playlists::open(dir.join("Playlists"));
        self.smart = SmartPlaylists::open(dir.join("Smart"));
        self.stats = Stats::open(dir.join("stats.txt"));
        self.folders = Folders::open(dir.join("Playlists").join("folders.txt"));
        self.info = Info::open(dir.join("info.txt"));
        self.playback = std::fs::read_to_string(dir.join("playback.txt")).map(|t| playback::Playback::parse(&t)).unwrap_or_default();
        // the audio service keeps the last crossfade it was given until it restarts, so the saved choice is sent again
        self.player.set_crossfade_ms(self.playback.crossfade_secs * 1000);
        self.smart_rev += 1;
        self.restore = std::fs::read_to_string(dir.join("queue.txt")).ok().and_then(|t| UpNext::from_text(&t));
        self.store = Some(dir);
        self.dirty = true;
    }

    /// Whether a text field (Songs search, playlist name, a rule's value) wants the keyboard.
    pub fn wants_keyboard(&self) -> bool {
        self.searching || self.renaming.is_some() || self.rule_edit.is_some() || self.info_edit.is_some()
    }

    /// Typed text for the field that has the keyboard; a newline accepts it.
    pub fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            if c == '\n' {
                self.commit_rename();
                self.commit_rule_edit();
                self.commit_info_edit();
            } else if c.is_control() {
                continue;
            } else if let Some(name) = &mut self.renaming {
                if std::mem::take(&mut self.rename_selected) {
                    name.clear();
                }
                name.push(c);
            } else if let Some((_, value)) = &mut self.rule_edit {
                value.push(c);
            } else if let Some((_, value)) = &mut self.info_edit {
                value.push(c);
            } else if self.searching {
                self.search.push(c);
                self.refilter();
            }
        }
        self.dirty = true;
    }

    /// Cmd+A while a playlist name is being typed: the whole name is selected. False when no name is being typed.
    pub fn select_all_name(&mut self) -> bool {
        if self.renaming.is_none() {
            return false;
        }
        self.rename_selected = true;
        self.dirty = true;
        true
    }

    pub fn backspace(&mut self) {
        if let Some(name) = &mut self.renaming {
            if std::mem::take(&mut self.rename_selected) {
                name.clear();
            } else {
                name.pop();
            }
        } else if let Some((_, value)) = &mut self.rule_edit {
            value.pop();
        } else if let Some((_, value)) = &mut self.info_edit {
            value.pop();
        } else if self.searching && self.search.pop().is_some() {
            self.refilter();
        }
        self.dirty = true;
    }

    fn commit_rename(&mut self) {
        let Some(name) = self.renaming.take() else { return };
        self.rename_selected = false;
        let clean = playlists::clean_name(&name).unwrap_or_default().to_lowercase();
        match self.page() {
            Page::Playlist(p) => {
                let old = self.playlists.lists().get(p).map(|l| l.name.clone());
                if self.playlists.rename(p, &name) {
                    if let Some(i) = self.playlists.lists().iter().position(|l| l.name.to_lowercase() == clean) {
                        *self.pages.last_mut().unwrap() = Page::Playlist(i);
                        if let Some(old) = old {
                            let new = self.playlists.lists()[i].name.clone();
                            self.folders.rename_playlist(&old, &new);
                        }
                    }
                }
            }
            Page::Smart(p) => {
                if self.smart.rename(p, &name) {
                    if let Some(i) = self.smart.lists().iter().position(|l| l.name.to_lowercase() == clean) {
                        *self.pages.last_mut().unwrap() = Page::Smart(i);
                    }
                    self.smart_rev += 1;
                }
            }
            _ => {}
        }
    }

    /// Applies the rule value being typed to its rule.
    fn commit_rule_edit(&mut self) {
        let Some((rule, text)) = self.rule_edit.take() else { return };
        if let Page::SmartEdit(p) = self.page() {
            if self.smart.set_value(p, rule, &text) {
                self.smart_rev += 1;
            }
        }
    }

    /// Get Info: commits the field being typed. A changed value is saved and the library scanned again, so it shows.
    fn commit_info_edit(&mut self) {
        let Some((field, text)) = self.info_edit.take() else { return };
        let Some(path) = self.info_path.clone() else { return };
        let shown = self.info_song().map(|id| self.info_shown(&self.library.songs[id], field));
        if shown.as_deref() != Some(text.trim()) && self.info.set(&path, field, &text) {
            self.start_scan();
        }
        self.dirty = true;
    }

    /// Opens Get Info for the song at library id `id`, closing the Now Playing sheet.
    fn open_info(&mut self, id: usize) {
        self.commit_info_edit();
        let Some(path) = self.library.songs.get(id).map(|song| song.path.clone()) else { return };
        self.info_path = Some(path);
        self.info_edit = None;
        if self.page() != Page::Info {
            let t = self.last_t;
            self.open_sheet(false, t);
            self.push(Page::Info);
        }
        self.dirty = true;
    }

    /// The library id of the song Get Info shows, while it is in the library.
    fn info_song(&self) -> Option<usize> {
        self.info_path.as_ref().and_then(|p| self.path_id.get(p).copied())
    }

    /// The title, artist or album Get Info shows for a song: its edit, else the file's tag.
    fn info_shown(&self, song: &library::Song, field: Field) -> String {
        self.info.value(&song.path, field).map(str::to_string).unwrap_or_else(|| match field {
            Field::Title => song.title.clone(),
            Field::Artist => song.artist.clone(),
            Field::Album => song.album.clone(),
        })
    }

    /// Makes a playlist folder named "Folder", "Folder 2", ...; the Playlists tab lists it.
    fn new_folder(&mut self) {
        let name = self.folders.fresh_name();
        let _ = self.folders.create(&name);
        self.dirty = true;
    }

    /// Shows or hides the playlists of folder `f` in the Playlists tab.
    fn toggle_folder(&mut self, f: usize) {
        let Some(name) = self.folders.names().get(f).cloned() else { return };
        match self.collapsed.iter().position(|c| *c == name) {
            Some(at) => {
                self.collapsed.remove(at);
            }
            None => self.collapsed.push(name),
        }
        self.dirty = true;
    }

    /// The Move to Folder menu of the playlist on screen: No Folder, then each folder.
    fn open_folder_menu(&mut self) {
        if !matches!(self.page(), Page::Playlist(_)) {
            return;
        }
        let mut items = vec![MenuItem::MoveToFolder(None)];
        items.extend((0..self.folders.names().len()).map(|f| MenuItem::MoveToFolder(Some(f))));
        self.menu = Some(Menu { song: 0, items, heading: Some("Move to Folder".to_string()) });
        self.dirty = true;
    }

    /// Puts the playlist on screen in `folder` (an index into the folders), or in none for None.
    fn move_shown_playlist(&mut self, folder: Option<usize>) {
        let Page::Playlist(p) = self.page() else { return };
        let Some(name) = self.playlists.lists().get(p).map(|l| l.name.clone()) else { return };
        if self.folders.put(&name, folder) {
            self.dirty = true;
        }
    }

    /// The folder the playlist on screen is in, as an index into the folders.
    fn shown_folder(&self) -> Option<usize> {
        match self.page() {
            Page::Playlist(p) => self.playlists.lists().get(p).and_then(|l| self.folders.folder_index(&l.name)),
            _ => None,
        }
    }

    /// The centre of the mini player's route button: just left of Play.
    fn mini_route(ui: &Ui, mini: &Mini) -> (f32, f32) {
        (mini.play.0 - ui.pt(MINI_ROUTE_GAP_PT), mini.play.1)
    }

    fn rebuild_view(&mut self) {
        self.view = browse::view_in_order(&self.library.songs, &self.search, self.sort, self.descending, self.scope, &self.stats);
    }

    /// Points the Songs list at `scope`. Entering a recent list opens it in its own order; leaving one returns to Title.
    fn set_scope(&mut self, scope: Scope) {
        let was = self.scope;
        self.scope = scope;
        if scope != Scope::All || was != Scope::All {
            self.sort = scope.sort();
        }
        self.refilter();
    }

    /// Ratings, favourites or plays changed: the lists that depend on them are worked out again.
    fn stats_changed(&mut self) {
        self.smart_rev += 1;
        if self.scope != Scope::All || matches!(self.sort, SortKey::Rating | SortKey::Plays | SortKey::LastPlayed) {
            self.rebuild_view();
        }
        self.dirty = true;
    }

    /// Works out the songs of the smart playlist on screen when it, its rules or the data it reads changed.
    fn sync_smart(&mut self) {
        let want = match self.page() {
            Page::Smart(p) | Page::SmartEdit(p) => Some((p, self.smart_rev)),
            _ => None,
        };
        if want == self.smart_key {
            return;
        }
        self.smart_key = want;
        self.smart_view = match want {
            Some((p, _)) => self.smart.lists().get(p).map_or_else(Vec::new, |l| l.select(&self.library.songs, &self.stats, unix_now())),
            None => Vec::new(),
        };
    }

    /// The ids of the songs that played lately, most recent first, for those still in the library.
    fn history_ids(&self) -> Vec<usize> {
        self.stats.history().iter().filter_map(|p| self.path_id.get(p).copied()).collect()
    }

    /// The library song that is loaded in the player, by its file (the player's ids are only as fresh as the queue).
    fn current_song(&self) -> Option<usize> {
        self.snap.track.as_ref().filter(|t| !t.is_live()).and_then(|t| self.path_id.get(&t.path).copied())
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
        // a command to the audio service that is waiting for its retry
        let wake = self.retry.next_wake().map_or(wake, |at| wake.min(at.max(t)));
        Some(self.player.next_wake(t).map_or(wake, |w| w.min(wake)))
    }

    // --- jobs ---

    /// Applies finished jobs and the engine's latest state; true if the
    /// screen changed.
    pub fn pump(&mut self, ui: &Ui) -> bool {
        let finished: Vec<Done> = self.pool.drain().into_iter().chain(self.net.drain()).collect();
        let mut changed = !finished.is_empty();
        self.why.jobs += finished.len() as u32;
        for done in finished {
            self.apply(done);
        }
        for (path, bpm) in self.tempo.drain() {
            self.bpm_pending.remove(&path);
            self.bpm.insert(path, bpm);
            changed = true;
            self.why.jobs += 1;
        }
        self.start_tempo();
        self.player.drain(now_ms());
        let player = self.refresh(ui);
        self.why.player += player as u32;
        changed |= player;
        self.sync_smart();
        self.dirty |= changed;
        changed
    }

    /// While the Songs list is on screen, starts tempo analyses for its songs not yet known, in list order, at most
    /// `TEMPO_JOBS` at a time.
    fn start_tempo(&mut self) {
        if self.scanning || self.np || self.tab != Tab::Songs || self.page() != Page::Root {
            return;
        }
        for i in 0..self.view.len() {
            if self.bpm_pending.len() >= TEMPO_JOBS {
                break;
            }
            let path = {
                let known = &self.library.songs[self.view[i]].path;
                if self.bpm.contains_key(known) || self.bpm_pending.contains(known) {
                    continue;
                }
                known.clone()
            };
            self.bpm_pending.insert(path.clone());
            self.tempo.spawn(move || {
                let bpm = tempo::bpm_of(&path);
                (path, bpm)
            });
        }
    }

    fn apply(&mut self, done: Done) {
        match done {
            Done::Scanned(mut files, stamps) => {
                // the edits of Get Info are laid over the tags before the library groups the songs
                for file in &mut files {
                    self.info.apply(file);
                }
                let lib = Library::build(files);
                self.album_of = (0..lib.songs.len()).map(|s| lib.album_of(s).unwrap_or(0)).collect();
                self.path_id = lib.songs.iter().map(|s| (s.path.clone(), s.id)).collect();
                self.library = lib;
                self.smart_rev += 1;
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
        if self.notice.is_some() && self.player.connected() {
            self.notice = None;
        }
        // the station plays: nothing is left to retry
        if snap.playing {
            self.retry.cancel();
        }
        // every playback error reaches the log with its cause, once per change
        if snap.error != self.snap.error {
            if let Some(e) = &snap.error {
                eprintln!("music: playback error: {e}");
            }
        }
        let live_error = |s: &Snapshot| s.live.as_ref().and_then(|l| l.error.clone());
        if live_error(&snap) != live_error(&self.snap) {
            if let Some(e) = live_error(&snap) {
                eprintln!("music: stream error: {e}");
            }
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
            _ if live.is_some_and(|l| l.error.is_some()) => live.and_then(|l| l.error.clone()).unwrap_or_default(),
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
            mini: ui.text(&ui.fit(&title, MAC_SUBHEAD_PT, mini_text_w(ui, self.w, self.h)), MAC_SUBHEAD_PT, Weight::Semibold, WHITE),
            mini_artist: ui.text(&ui.fit(&artist, MAC_FOOTNOTE_PT, mini_text_w(ui, self.w, self.h)), MAC_FOOTNOTE_PT, Weight::Regular, WHITE),
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
                (Page::History, _) => {
                    let row = ui.pt(SONG_ROW_PT);
                    let first = (off / row).max(0.0) as usize;
                    let last = ((off + bottom - top) / row).max(0.0) as usize + 1;
                    let ids: Vec<usize> = self.history_ids().into_iter().skip(first).take(last.saturating_sub(first)).collect();
                    wanted.extend(ids.into_iter().map(|id| (self.album_of[id], THUMB)));
                }
                (Page::Smart(_), _) => {
                    let row = ui.pt(SONG_ROW_PT);
                    let first = ((off - ui.pt(PLAYLIST_HEADER_PT)) / row).max(0.0) as usize;
                    let last = ((off + bottom - top) / row).max(0.0) as usize + 1;
                    wanted.extend(self.smart_view.iter().skip(first).take(last.saturating_sub(first)).map(|&id| (self.album_of[id], THUMB)));
                }
                (Page::SmartEdit(_), _) => {}
                (Page::Info, _) => {}
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
            Page::Playlist(_) | Page::Smart(_) => self.playlist_buttons(ui),
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
            (Page::Root, Tab::Playlists) => n(self.playlist_rows()) * ui.pt(ARTIST_ROW_PT) + ui.pt(SPACE_L),
            (Page::Playlist(p), _) => ui.pt(PLAYLIST_HEADER_PT) + n(self.playlist_entries(p).len()) * ui.pt(SONG_ROW_PT) + ui.pt(SPACE_L),
            (Page::Smart(_), _) => ui.pt(PLAYLIST_HEADER_PT) + n(self.smart_view.len()) * ui.pt(SONG_ROW_PT) + ui.pt(SPACE_L),
            // the match row, one row per rule, Add Rule, and the count line
            (Page::SmartEdit(p), _) => n(self.smart.lists().get(p).map_or(0, |l| l.rules.len()) + 3) * ui.pt(SONG_ROW_PT) + ui.pt(SPACE_L),
            (Page::History, _) => n(self.history_ids().len()) * ui.pt(SONG_ROW_PT) + ui.pt(SPACE_L),
            (Page::Queue, _) => {
                // the rows, then the Clear Up Next row when something is upcoming
                let rows = n(self.queue.items().len().saturating_sub(self.queue.current()));
                let clear = if self.queue.upcoming().is_empty() { 0.0 } else { 1.0 };
                (rows + clear) * ui.pt(SONG_ROW_PT) + ui.pt(SPACE_L)
            }
            (Page::Info, _) => n(INFO_ROWS) * ui.pt(SONG_ROW_PT) + ui.pt(SPACE_L),
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
            (Page::Root, Tab::Playlists) => Some((y / ui.pt(ARTIST_ROW_PT)) as usize).filter(|&i| i < self.playlist_rows()).map(Target::Row),
            (Page::Smart(_), _) => {
                let [play, shuffle] = self.playlist_buttons(ui);
                if within(x, y, play) {
                    return Some(Target::Play);
                }
                if within(x, y, shuffle) {
                    return Some(Target::Shuffle);
                }
                let header = ui.pt(PLAYLIST_HEADER_PT);
                (y >= header).then(|| ((y - header) / ui.pt(SONG_ROW_PT)) as usize).filter(|&i| i < self.smart_view.len()).map(|i| if x >= self.w as f32 - ui.pt(FAV_ZONE_PT) { Target::More(i) } else { Target::Row(i) })
            }
            (Page::History, _) => Some((y / ui.pt(SONG_ROW_PT)) as usize).filter(|&i| i < self.history_ids().len()).map(|i| if x >= self.w as f32 - ui.pt(FAV_ZONE_PT) { Target::More(i) } else { Target::Row(i) }),
            (Page::SmartEdit(p), _) => {
                let rules = self.smart.lists().get(p).map_or(0, |l| l.rules.len());
                let i = (y / ui.pt(SONG_ROW_PT)) as usize;
                if i == 0 {
                    return Some(Target::MatchToggle);
                }
                if i == rules + 1 {
                    return Some(Target::RuleAdd);
                }
                if i > rules + 1 {
                    return None;
                }
                let cols = self.rule_columns(ui);
                Some(if x < cols[1] {
                    Target::RuleField(i - 1)
                } else if x < cols[2] {
                    Target::RuleOp(i - 1)
                } else if x < cols[3] {
                    Target::RuleValue(i - 1)
                } else {
                    Target::RuleDel(i - 1)
                })
            }
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
                let rows = self.queue.items().len().saturating_sub(self.queue.current());
                if i == rows && !self.queue.upcoming().is_empty() {
                    return Some(Target::QClear);
                }
                if i >= rows {
                    return None;
                }
                let zone = ui.pt(FAV_ZONE_PT);
                Some(if i >= 1 && x >= self.w as f32 - zone {
                    Target::QDel(i)
                } else if i >= 2 && x >= self.w as f32 - 2.0 * zone {
                    Target::QUp(i)
                } else if i >= 1 && x >= self.w as f32 - 3.0 * zone {
                    Target::QDrag(i)
                } else {
                    Target::Row(i)
                })
            }
            (Page::Info, _) => {
                let i = (y / ui.pt(SONG_ROW_PT)) as usize;
                match i {
                    0..=2 => Some(Target::InfoField(i)),
                    3 => Some(Target::InfoReset),
                    _ => None,
                }
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

    /// Where the lyrics switch and the output route button sit on the Now Playing sheet, as centres in px: the row of
    /// the Up Next button, at the left and right sides.
    fn sheet_buttons(&self, ui: &Ui) -> ((f32, f32), (f32, f32)) {
        let l = Layout::new(ui, self.w, self.h);
        let side = ui.pt(nowplaying::SIDE_PT);
        ((side + ui.pt(SPACE_XXL), l.queue.1), (self.w as f32 - side - ui.pt(SPACE_L), l.queue.1))
    }

    /// The lyrics switch (when the song has lyrics) or the route button under (x, y) on the Now Playing sheet.
    fn sheet_extra(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let (lyrics, route) = self.sheet_buttons(ui);
        let near = |c: (f32, f32), reach: f32| (x - c.0).abs() <= reach && (y - c.1).abs() <= ui.pt(28.0);
        if self.lyrics.is_some() && !self.is_live() && near(lyrics, ui.pt(36.0)) {
            return Some(Target::Lyrics);
        }
        near(route, ui.pt(28.0)).then_some(Target::Route)
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
            return self.sheet_extra(ui, x, y).or_else(|| Layout::new(ui, self.w, self.h).hit(x, y).map(Target::Np));
        }
        if self.has_track() {
            let mini = Mini::new(ui, self.w, self.h);
            let (rx, ry) = Self::mini_route(ui, &mini);
            if (x - rx).abs() <= ui.pt(MINI_ROUTE_HIT_PT) && (y - ry).abs() <= ui.pt(MINI_ROUTE_HIT_PT) {
                return Some(Target::MiniRoute);
            }
            if mini_volume_track(ui, &mini).is_some_and(|v| v.inset(ui.pt(SPACE_S)).contains(x, y)) {
                return Some(Target::MiniVolume);
            }
            if let Some(t) = mini.hit(x, y) {
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
        self.last_t = t;
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
                        self.vol_drag = Some(layout.volume);
                        return self.set_volume_from(Layout::fraction(layout.volume, x));
                    }
                    Some(Target::MiniVolume) => {
                        let Some(track) = mini_volume_track(ui, &Mini::new(ui, self.w, self.h)) else { return vec![] };
                        self.vol_drag = Some(track);
                        return self.set_volume_from(Layout::fraction(track, x));
                    }
                    Some(Target::QDrag(row)) => self.qdrag = Some(QueueDrag { from: row, y: y - self.content_top(ui) + self.scroll.offset() }),
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
                } else if let Some(track) = self.vol_drag {
                    return self.set_volume_from(Layout::fraction(track, x));
                } else if self.qdrag.is_some() {
                    let rel = y - self.content_top(ui) + self.scroll.offset();
                    if let Some(drag) = self.qdrag.as_mut() {
                        drag.y = rel;
                    }
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
                if self.vol_drag.take().is_some() {
                    return vec![];
                }
                if let Some(drag) = self.qdrag.take() {
                    // dropped: row `from` of the page is upcoming song `from - 1`
                    let rows = self.queue.items().len().saturating_sub(self.queue.current());
                    let to = upnext::drop_row(y - self.content_top(ui) + self.scroll.offset(), ui.pt(SONG_ROW_PT), rows);
                    self.queue_move(drag.from.saturating_sub(1), to.saturating_sub(1));
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

    /// 0 = sheet away, 1 = sheet shown. The slide runs on the shared `PanelReveal` motion (instant under Reduce Motion).
    fn sheet_progress(&self, t: f64) -> f32 {
        match self.np_anim {
            Some((from, to, start)) => from + (to - from) * sheet_slide(start).value(t / 1000.0),
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
        // MIDI files queue like any song: the audio decoder renders them with the synth (media::audiofile::Midi).
        if ids.is_empty() {
            return;
        }
        let start = start.min(ids.len() - 1);
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
        self.track_plays();
        self.sync_lyrics();
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

    /// Brings the cover for Control Center up to date with the current song: the encoded picture embedded in the
    /// album's tags when it has one (decoded straight to wire size), else the Now Playing picture when the sheet has
    /// loaded it, else the list thumbnail (always requested for the playing song). The conversion to wire pixels runs
    /// once per cover, not per tick. A song with no cover (or one not loaded yet) leaves none, so the tile keeps its glyph.
    pub fn refresh_now_playing_artwork(&mut self) {
        use bifrost_app::now_playing::ArtworkImage;
        let key = self.snap.track.as_ref().and_then(|t| self.art_key(t));
        let Some(key) = key else {
            self.np_art = None;
            return;
        };
        if self.np_art.as_ref().is_some_and(|(k, d, _)| *k == key && *d == EMBEDDED) {
            return;
        }
        let live = self.snap.track.as_ref().is_some_and(|t| t.is_live());
        let encoded = if live || self.np_embedded_failed.as_deref() == Some(key.as_str()) {
            None
        } else {
            key.parse::<usize>().ok().and_then(|album| self.library.cover_of(album))
        };
        if encoded.is_some() {
            match nowplay::embedded_cover(encoded) {
                Some(art) => {
                    self.np_art = Some((key, EMBEDDED, art));
                    return;
                }
                None => self.np_embedded_failed = Some(key.clone()),
            }
        }
        let kind = if self.art.peek(&key, BIG).is_some() { BIG } else { THUMB };
        if self.np_art.as_ref().is_some_and(|(k, d, _)| *k == key && *d == kind) {
            return;
        }
        let converted = self.art.peek(&key, kind).and_then(|img| ArtworkImage::from_premultiplied(img.w, img.h, &img.px).ok());
        self.np_art = converted.map(|art| (key, kind, art));
    }

    /// The cover `refresh_now_playing_artwork` last prepared, for the published session (`app.rs`).
    pub fn now_playing_artwork(&self) -> Option<&bifrost_app::now_playing::ArtworkImage> {
        self.np_art.as_ref().map(|(_, _, art)| art)
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

    /// The Up Next list Control Center shows: the songs after the current one.
    pub fn up_next_entries(&self) -> Vec<bifrost_app::now_playing::QueueEntry> {
        nowplay::up_next_entries(self.queue.upcoming(), |path| {
            self.path_id.get(path).and_then(|&id| self.library.songs.get(id)).map(|song| (song.title.clone(), song.artist.clone()))
        })
    }

    /// Plays the Up Next song Control Center picked, by its place among the upcoming songs. False when there is no such
    /// song in the library.
    pub fn remote_queue_pick(&mut self, index: usize) -> bool {
        let absolute = self.queue.current() + 1 + index;
        let playable = self.queue.items().get(absolute).is_some_and(|path| self.path_id.contains_key(path));
        if !playable {
            return false;
        }
        self.queue.set_current(absolute);
        self.push_queue(0.0, true);
        true
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
        self.retry.cancel();
        let tracks: Vec<Track> = self.queue.items().iter().filter_map(|p| self.path_id.get(p).map(|&id| Track::file(id as u64, p.clone()))).collect();
        let sent = self.player.play_queue(tracks, self.queue.current(), false, self.seed);
        self.report("play the queue", sent);
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

    /// Clear Up Next: every upcoming song leaves the queue; the current song keeps playing where it was.
    fn queue_clear(&mut self) {
        if self.queue.clear_upcoming() > 0 {
            self.push_queue(self.snap.position_ms, self.snap.playing);
        }
    }

    fn queue_move_up(&mut self, upcoming: usize) {
        if upcoming > 0 {
            self.queue_move(upcoming, upcoming - 1);
        }
    }

    /// Moves the `from`-th upcoming song to the `to`-th place (a drag in the Up Next list).
    fn queue_move(&mut self, from: usize, to: usize) {
        if from != to && self.queue.move_upcoming(from, to) {
            self.push_queue(self.snap.position_ms, self.snap.playing);
        }
    }

    /// Tunes in the `i`-th station of the list as shown; the whole list is
    /// the queue, so Previous and Next step through the stations. The
    /// station already tuned is left alone, or tuned in again when paused
    /// (a paused stream is stale; resuming means going live).
    fn play_station(&mut self, i: usize) {
        // a choice ends the retries of the one before
        self.retry.cancel();
        let stations = self.radio.stations();
        let order = self.radio.order();
        let Some(&chosen) = order.get(i) else { return };
        let id = stations[chosen].id;
        if self.snap.track.as_ref().is_some_and(|t| t.is_live() && t.id == id) {
            self.resume_live();
            return;
        }
        self.tune_in(i, id);
    }

    /// Sends the station list as the queue, starting at the `i`-th station (`id`). A command the audio service could not
    /// take is kept for `retry`.
    fn tune_in(&mut self, i: usize, id: u64) {
        let stations = self.radio.stations();
        let tracks: Vec<Track> = self.radio.order().into_iter().map(|s| Track::live(stations[s].id, stations[s].stream_url.clone())).collect();
        let sent = self.player.play_queue(tracks, i, false, self.seed);
        if sent.is_err() {
            self.retry.arm(retry::Wanted::Station(id), self.last_t);
        } else {
            self.retry.cancel();
        }
        self.report("tune in the station", sent);
        self.take_snapshot();
    }

    /// The retry of a tune-in whose wait is over. A station that left the list is forgotten; a service that is still
    /// away leaves the wish on its (longer) schedule.
    fn retry_due(&mut self, t: f64) {
        let Some(retry::Wanted::Station(id)) = self.retry.take_due(t) else { return };
        let position = self.radio.order().iter().position(|&s| self.radio.stations()[s].id == id);
        match position {
            Some(i) => self.tune_in(i, id),
            None => self.retry.cancel(),
        }
    }

    /// Records the outcome of a command to the audio service: a failure goes to stderr with its cause and stays on
    /// screen (`playback_problem`) until the service is reachable; a success clears it.
    fn report(&mut self, what: &str, sent: Result<(), String>) {
        match sent {
            Ok(()) => self.notice = None,
            Err(cause) => {
                eprintln!("music: cannot {what}: {cause}");
                // the screen changes only when the message does: the same failure again (a retry) is not a repaint
                if self.notice.as_deref() != Some(cause.as_str()) {
                    self.notice = Some(cause);
                    self.dirty = true;
                }
            }
        }
    }

    /// What is wrong with playback, in a few words for the screen: the command that could not be sent, the service's
    /// error for the queue, or the stream's own.
    pub fn playback_problem(&self) -> Option<String> {
        self.notice.clone().or_else(|| self.snap.live.as_ref().and_then(|l| l.error.clone())).or_else(|| self.snap.error.clone())
    }

    /// Plays a paused live track again from the live edge by reopening the stream.
    fn resume_live(&mut self) {
        if !self.snap.playing {
            if let Some(i) = self.snap.index {
                let sent = self.player.jump_to(i);
                if let (Err(_), Some(id)) = (&sent, self.snap.track.as_ref().map(|t| t.id)) {
                    self.retry.arm(retry::Wanted::Station(id), self.last_t);
                }
                self.report("resume the stream", sent);
            }
        }
        self.take_snapshot();
    }

    /// A transport control from any surface (sheet, mini player, lock
    /// screen, Control Center).
    pub fn control(&mut self, target: nowplaying::Target) {
        // pausing, skipping or any other transport choice is the person's: nothing is retried behind it
        self.retry.cancel();
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
            (Page::Smart(_), _) => self.smart_view.get(i).copied().filter(|&id| id < self.library.songs.len()).map(|id| (id, None)),
            (Page::History, _) => self.history_ids().get(i).map(|&id| (id, None)),
            _ => None,
        }
    }

    /// (position in the playlist, song id) of the songs of playlist `p` the library has.
    fn playlist_entries(&self, p: usize) -> Vec<(usize, usize)> {
        let Some(list) = self.playlists.lists().get(p) else { return Vec::new() };
        list.songs.iter().enumerate().filter_map(|(at, path)| self.path_id.get(path).map(|&id| (at, id))).collect()
    }

    /// The rows of the Playlists tab (`folders::rows`).
    fn tab_rows(&self) -> Vec<TabRow> {
        folders::rows(&self.folders, self.playlists.lists(), self.smart.lists().len(), &self.collapsed)
    }

    /// How many rows the Playlists tab has.
    fn playlist_rows(&self) -> usize {
        self.tab_rows().len()
    }

    /// The x edges of a rule row's cells in px: the field, the operator, the value, and the remove button after them.
    fn rule_columns(&self, ui: &Ui) -> [f32; 4] {
        let m = ui.pt(MARGIN_PT);
        let total = self.w as f32 - 2.0 * m - ui.pt(FAV_ZONE_PT);
        [m, m + total * 0.28, m + total * 0.62, m + total]
    }

    /// Feeds the transport's position to the play counter: a song that starts is put in the history, and one heard for
    /// long enough counts as played. Only while it plays, so a queue restored paused at launch is not a play.
    fn track_plays(&mut self) {
        if !self.snap.playing {
            return;
        }
        let path = self.snap.track.as_ref().filter(|t| !t.is_live()).map(|t| t.path.clone());
        let events = self.tracker.observe(path.as_deref(), self.snap.position_ms, self.snap.duration_ms);
        if events.is_empty() {
            return;
        }
        let now = unix_now();
        for event in events {
            match event {
                PlayEvent::Started(p) => self.stats.record_start(&p, now),
                PlayEvent::Played(p) => self.stats.record_play(&p, now),
            }
        }
        self.stats_changed();
    }

    /// Reads the lyrics that go with the song now loaded, once per song.
    fn sync_lyrics(&mut self) {
        let path = self.snap.track.as_ref().filter(|t| !t.is_live()).map(|t| t.path.clone());
        if path == self.lyrics_for {
            return;
        }
        self.lyrics = path.as_deref().and_then(Lyrics::load);
        self.lyrics_for = path;
        if self.lyrics.is_none() {
            self.lyrics_on = false;
        }
        self.dirty = true;
    }

    /// Where the content pane points, for the View menu.
    fn place(&self) -> Place {
        match self.page() {
            Page::Queue => Place::UpNext,
            Page::History => Place::History,
            _ => match (self.tab, self.scope) {
                (Tab::Artists, _) => Place::Artists,
                (Tab::Albums, _) => Place::Albums,
                (Tab::Songs, Scope::All) => Place::Songs,
                (Tab::Songs, Scope::RecentlyAdded) => Place::RecentlyAdded,
                (Tab::Songs, Scope::RecentlyPlayed) => Place::RecentlyPlayed,
                (Tab::Songs, Scope::Favourites) => Place::Favourites,
                (Tab::Radio, _) => Place::Radio,
                (Tab::Playlists, _) => Place::Playlists,
            },
        }
    }

    /// What the app menus need to know now.
    pub fn menu_state(&self) -> MenuState {
        let path = self.current_song().map(|id| &self.library.songs[id].path);
        MenuState {
            has_track: self.has_track(),
            playing: self.snap.playing,
            live: self.is_live(),
            has_song: path.is_some(),
            shuffle: self.snap.shuffle,
            repeat: self.snap.repeat != Repeat::Off,
            rating: path.map_or(0, |p| self.stats.rating(p)),
            favourite: path.is_some_and(|p| self.stats.is_favourite(p)),
            playlists: self.playlists.lists().iter().map(|l| l.name.clone()).collect(),
            place: self.place(),
            sort: self.sort,
            descending: self.descending,
            lyrics_shown: self.lyrics_on,
            has_lyrics: self.lyrics.is_some(),
            crossfade_secs: self.playback.crossfade_secs,
            // the window knows the devices (`MusicApp::main_menu` fills them in)
            outputs: Vec::new(),
            output: None,
        }
    }

    /// What the MiniPlayer window shows of the current song.
    pub fn mini_info(&self) -> miniplayer::Info {
        let (title, artist) = match &self.snap.track {
            Some(t) => {
                let texts = self.texts(t, self.snap.live.as_ref());
                (texts.2, texts.3)
            }
            None => ("Not Playing".to_string(), String::new()),
        };
        let progress = match self.snap.duration_ms {
            Some(d) if d > 0.0 => (self.snap.position_ms / d).clamp(0.0, 1.0) as f32,
            _ => 0.0,
        };
        miniplayer::Info { title, artist, playing: self.snap.playing, progress, volume: self.volume, live: self.is_live() }
    }

    /// The artwork of the current song for the MiniPlayer, when the cache has it.
    pub fn mini_art(&mut self) -> Option<Image> {
        let key = self.snap.track.as_ref().and_then(|t| self.art_key(t))?;
        self.art.get(&key, BIG).cloned()
    }

    /// Seeks the current song to `f` of its length (0 to 1). A live station has no length to seek in.
    pub fn seek_to_fraction(&mut self, f: f32) {
        if self.is_live() {
            return;
        }
        let Some(d) = self.snap.duration_ms.filter(|d| *d > 0.0) else { return };
        self.player.seek_ms(d * f.clamp(0.0, 1.0) as f64);
        self.take_snapshot();
    }

    /// Sets the volume to `f` (0 to 1), as the MiniPlayer's volume bar does.
    pub fn set_volume_fraction(&mut self, f: f32) {
        let _ = self.set_volume_from(f.clamp(0.0, 1.0));
    }

    /// Runs a menu command; true when the screen needs a frame. File > Import is the window's (it owns the open panel)
    /// and is not run here.
    pub fn run_command(&mut self, cmd: Command) -> bool {
        let song = self.current_song();
        match cmd {
            // the window runs these: the open panel, its windows and the shell's device service are its
            Command::Import | Command::MiniPlayer | Command::KeepMiniOnTop | Command::Output(_) => return false,
            Command::PlayPause => self.control(nowplaying::Target::Play),
            Command::Next => self.control(nowplaying::Target::Next),
            Command::Previous => self.control(nowplaying::Target::Prev),
            Command::Shuffle => self.control(nowplaying::Target::Shuffle),
            Command::Repeat => self.control(nowplaying::Target::Repeat),
            Command::VolumeUp => self.nudge_volume(VOLUME_STEP),
            Command::VolumeDown => self.nudge_volume(-VOLUME_STEP),
            Command::PlayNext => song.into_iter().for_each(|id| self.queue_add(id, true)),
            Command::PlayLast => song.into_iter().for_each(|id| self.queue_add(id, false)),
            Command::NewPlaylist => song.into_iter().for_each(|id| self.run_menu(MenuItem::NewPlaylist, id)),
            Command::AddTo(p) if p < self.playlists.lists().len() => song.into_iter().for_each(|id| self.run_menu(MenuItem::AddTo(p), id)),
            Command::AddTo(_) => return false,
            Command::Favourite => song.into_iter().for_each(|id| self.run_menu(MenuItem::ToggleFavourite, id)),
            Command::Rate(n) => {
                if let Some(id) = song {
                    let path = self.library.songs[id].path.clone();
                    self.stats.set_rating(&path, n);
                    self.stats_changed();
                }
            }
            Command::GetInfo => song.into_iter().for_each(|id| self.open_info(id)),
            Command::NewFolder => {
                self.show_tab(Tab::Playlists, Scope::All);
                self.new_folder();
            }
            Command::Show(place) => self.show_place(place),
            Command::SortBy(key) => {
                if self.tab != Tab::Songs || self.page() != Page::Root {
                    self.show_place(Place::Songs);
                }
                self.sort = key;
                self.refilter();
            }
            Command::SortDescending => {
                if self.tab != Tab::Songs || self.page() != Page::Root {
                    self.show_place(Place::Songs);
                }
                self.descending = !self.descending;
                self.refilter();
            }
            Command::Crossfade(secs) => {
                self.playback = self.playback.with_crossfade(secs);
                self.player.set_crossfade_ms(self.playback.crossfade_secs * 1000);
                if let Some(dir) = &self.store {
                    // the store folder may not exist yet when nothing else has been saved to it
                    let _ = std::fs::create_dir_all(dir);
                    let _ = std::fs::write(dir.join("playback.txt"), self.playback.to_text());
                }
            }
            Command::Lyrics => {
                if self.lyrics.is_some() && !self.is_live() {
                    self.lyrics_on = !self.lyrics_on;
                    if self.lyrics_on {
                        self.open_sheet(true, self.last_t);
                    }
                }
            }
        }
        self.dirty = true;
        true
    }

    /// Moves the player's volume by `step` (a fraction of full scale), staying between silence and full.
    fn nudge_volume(&mut self, step: f32) {
        let _ = self.set_volume_from((self.volume + step).clamp(0.0, 1.0));
    }

    /// Points the window at `place`, closing the Now Playing sheet if it is up.
    fn show_place(&mut self, place: Place) {
        let t = self.last_t;
        self.open_sheet(false, t);
        match place {
            Place::Artists => self.show_tab(Tab::Artists, Scope::All),
            Place::Albums => self.show_tab(Tab::Albums, Scope::All),
            Place::Songs => self.show_tab(Tab::Songs, Scope::All),
            Place::Radio => self.show_tab(Tab::Radio, Scope::All),
            Place::Playlists => self.show_tab(Tab::Playlists, Scope::All),
            Place::RecentlyAdded => self.show_tab(Tab::Songs, Scope::RecentlyAdded),
            Place::RecentlyPlayed => self.show_tab(Tab::Songs, Scope::RecentlyPlayed),
            Place::Favourites => self.show_tab(Tab::Songs, Scope::Favourites),
            Place::UpNext => {
                if !self.queue.is_empty() && self.page() != Page::Queue {
                    self.push(Page::Queue);
                }
            }
            Place::History => {
                if self.page() != Page::History {
                    self.push(Page::History);
                }
            }
        }
    }

    /// Shows `tab` at its top level; the Songs tab shows the list of `scope`.
    fn show_tab(&mut self, tab: Tab, scope: Scope) {
        let index = [Tab::Artists, Tab::Albums, Tab::Songs, Tab::Radio, Tab::Playlists].iter().position(|t| *t == tab).unwrap_or(2);
        self.activate(Target::Seg(index), self.last_t);
        if tab == Tab::Songs && scope != self.scope {
            self.set_scope(scope);
        }
    }

    /// Copies one chosen song into the first music folder (File > Import). True when it was copied.
    pub fn import_song(&mut self, name: &str, source: &mut impl std::io::Read) -> bool {
        let Some(root) = self.roots.first() else { return false };
        matches!(library::import_into(root, name, source), Ok(Some(_)))
    }

    /// A batch of imports finished: scan again so the songs show up, unless a scan is already running (the check that
    /// follows the music folders finds the rest).
    pub fn imports_done(&mut self, copied: usize) {
        if copied > 0 && !self.scanning {
            self.start_scan();
            self.dirty = true;
        }
    }

    /// Whether the window should fetch the output devices (the route button was pressed). Clears the request.
    pub fn take_output_request(&mut self) -> bool {
        std::mem::take(&mut self.output_request)
    }

    /// Lists the output devices as (id, name) in a menu, `current` being the default one.
    pub fn show_outputs(&mut self, devices: Vec<(String, String)>, current: Option<String>) {
        self.outputs = devices;
        self.output_now = current;
        if self.outputs.is_empty() {
            return;
        }
        let items = (0..self.outputs.len()).map(MenuItem::Output).collect();
        self.menu = Some(Menu { song: 0, items, heading: Some("Output".to_string()) });
        self.dirty = true;
    }

    /// The output device the user picked, to make the default; None when none was picked since the last call.
    pub fn take_output_choice(&mut self) -> Option<String> {
        self.output_choice.take()
    }

    fn open_menu(&mut self, song: usize, from: Option<(usize, usize)>) {
        let mut items = vec![MenuItem::PlayNext, MenuItem::PlayLast, MenuItem::Rate, MenuItem::ToggleFavourite, MenuItem::GetInfo];
        items.extend((0..self.playlists.lists().len()).map(MenuItem::AddTo));
        items.push(MenuItem::NewPlaylist);
        if let Some((p, at)) = from {
            items.push(MenuItem::RemoveFromPlaylist(p, at));
        }
        self.menu = Some(Menu { song, items, heading: None });
    }

    fn run_menu(&mut self, item: MenuItem, song: usize) {
        if let MenuItem::Output(k) = item {
            self.output_choice = self.outputs.get(k).map(|(id, _)| id.clone());
            return;
        }
        if let MenuItem::MoveToFolder(folder) = item {
            self.move_shown_playlist(folder);
            return;
        }
        let path = self.library.songs[song].path.clone();
        match item {
            MenuItem::Output(_) | MenuItem::MoveToFolder(_) => {}
            MenuItem::GetInfo => self.open_info(song),
            MenuItem::PlayNext => self.queue_add(song, true),
            MenuItem::PlayLast => self.queue_add(song, false),
            MenuItem::Rate => {
                self.stats.cycle_rating(&path);
                self.stats_changed();
            }
            MenuItem::ToggleFavourite => {
                self.stats.toggle_favourite(&path);
                self.stats_changed();
            }
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
        // a rule value or a Get Info field being typed is kept when anything else is tapped
        self.commit_rule_edit();
        self.commit_info_edit();
        match target {
            Target::Seg(i) => {
                let tab = [Tab::Artists, Tab::Albums, Tab::Songs, Tab::Radio, Tab::Playlists][i.min(4)];
                if tab != self.tab || self.pages.len() > 1 {
                    self.tab = tab;
                    self.pages.truncate(1);
                    self.renaming = None;
                    self.scroll.jump_top();
                }
                if self.scope != Scope::All {
                    self.set_scope(Scope::All);
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
                (Page::Root, Tab::Playlists) => match self.tab_rows().get(i).copied() {
                    Some(TabRow::NewPlaylist) => {
                        let name = self.playlists.fresh_name();
                        if let Some(p) = self.playlists.create(&name) {
                            self.push(Page::Playlist(p));
                        }
                    }
                    Some(TabRow::NewSmart) => {
                        // a new smart playlist opens on its rules
                        let name = self.smart.fresh_name();
                        if let Some(p) = self.smart.create(&name) {
                            self.smart_rev += 1;
                            self.push(Page::SmartEdit(p));
                        }
                    }
                    Some(TabRow::NewFolder) => self.new_folder(),
                    Some(TabRow::Folder(f)) => self.toggle_folder(f),
                    Some(TabRow::Playlist { index, .. }) => self.push(Page::Playlist(index)),
                    Some(TabRow::Smart(p)) => self.push(Page::Smart(p)),
                    None => {}
                },
                (Page::Playlist(p), _) => {
                    let ids: Vec<usize> = self.playlist_entries(p).into_iter().map(|e| e.1).collect();
                    if i < ids.len() {
                        self.play_songs(ids, i, false);
                    }
                }
                (Page::Smart(_), _) => {
                    if i < self.smart_view.len() {
                        self.play_songs(self.smart_view.clone(), i, false);
                    }
                }
                (Page::History, _) => {
                    let ids = self.history_ids();
                    if i < ids.len() {
                        self.play_songs(ids, i, false);
                    }
                }
                (Page::SmartEdit(_), _) => {}
                (Page::Info, _) => {}
                (Page::Queue, _) => {
                    if i > 0 && i <= self.queue.upcoming().len() {
                        self.retry.cancel();
                        let sent = self.player.jump_to(self.queue.current() + i);
                        self.report("jump to the song", sent);
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
                Page::Smart(_) => {
                    if !self.smart_view.is_empty() {
                        self.play_songs(self.smart_view.clone(), 0, target == Target::Shuffle);
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
            Target::QClear => self.queue_clear(),
            Target::QDrag(_) => {}
            Target::Rename => {
                self.rename_selected = false;
                match self.page() {
                    Page::Playlist(p) => self.renaming = self.playlists.lists().get(p).map(|l| l.name.clone()),
                    Page::Smart(p) => self.renaming = self.smart.lists().get(p).map(|l| l.name.clone()),
                    _ => {}
                }
            }
            Target::DeletePlaylist => match self.page() {
                Page::Playlist(p) => {
                    if let Some(name) = self.playlists.lists().get(p).map(|l| l.name.clone()) {
                        self.folders.forget_playlist(&name);
                    }
                    self.playlists.delete(p);
                    self.renaming = None;
                    self.pages.pop();
                    self.scroll.jump_top();
                }
                Page::Smart(p) => {
                    self.smart.delete(p);
                    self.smart_rev += 1;
                    self.renaming = None;
                    self.pages.pop();
                    self.scroll.jump_top();
                }
                _ => {}
            },
            Target::MatchToggle | Target::RuleField(_) | Target::RuleOp(_) | Target::RuleValue(_) | Target::RuleDel(_) | Target::RuleAdd => self.edit_rules(target),
            Target::EditRules => {
                if let Page::Smart(p) = self.page() {
                    self.push(Page::SmartEdit(p));
                }
            }
            Target::Lyrics => {
                if self.lyrics.is_some() {
                    self.lyrics_on = !self.lyrics_on;
                }
            }
            Target::Route | Target::MiniRoute => self.output_request = true,
            Target::MenuItem(k) => {
                if let Some(menu) = self.menu.take() {
                    if let Some(&item) = menu.items.get(k) {
                        self.run_menu(item, menu.song);
                    }
                }
            }
            Target::InfoField(k) => {
                if let (Some(field), Some(id)) = (Field::ALL.get(k).copied(), self.info_song()) {
                    let value = self.info_shown(&self.library.songs[id], field);
                    self.info_edit = Some((field, value));
                }
            }
            Target::InfoReset => {
                self.info_edit = None;
                if let Some(path) = self.info_path.clone() {
                    if self.info.reset(&path) {
                        self.start_scan();
                    }
                }
            }
            Target::MoveToFolder => self.open_folder_menu(),
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
            Target::Content | Target::MiniVolume => {}
        }
        self.dirty = true;
        vec![]
    }

    fn push(&mut self, page: Page) {
        self.pages.push(page);
        self.scroll.jump_top();
    }

    /// A tap on a row of the smart playlist's rules editor.
    fn edit_rules(&mut self, target: Target) {
        let Page::SmartEdit(p) = self.page() else { return };
        let changed = match target {
            Target::MatchToggle => self.smart.toggle_match(p),
            Target::RuleField(r) => self.smart.cycle_field(p, r),
            Target::RuleOp(r) => self.smart.cycle_op(p, r),
            Target::RuleDel(r) => self.smart.remove_rule(p, r),
            Target::RuleAdd => self.smart.add_rule(p),
            Target::RuleValue(r) => {
                let rule = self.smart.lists().get(p).and_then(|l| l.rules.get(r)).map(|rule| (rule.field.kind(), rule.value.clone()));
                match rule {
                    // a yes or no value flips; any other is typed
                    Some((Kind::Flag, _)) => self.smart.toggle_flag(p, r),
                    Some((_, value)) => {
                        self.rule_edit = Some((r, value));
                        false
                    }
                    None => false,
                }
            }
            _ => false,
        };
        if changed {
            self.smart_rev += 1;
        }
    }

    /// Advances scrolling, the sheet and artwork animations; true if a repaint is due.
    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        self.last_t = t;
        self.sync_smart();
        let mut moving = self.scroll.tick(t);
        self.fit_scroll(ui);
        if !self.scanning && t >= self.next_check {
            self.next_check = t + LIBRARY_CHECK_SECS;
            let (roots, load) = (self.roots.clone(), self.load.clone());
            self.pool.spawn(move || stamp_now(&roots, &load));
        }
        if let Some((_, to, start)) = self.np_anim {
            moving = true;
            if sheet_slide(start).done(t / 1000.0) {
                self.np_anim = None;
                self.np = to >= 1.0;
            }
        }
        if self.refresh(ui) {
            self.dirty = true;
            self.why.player += 1;
        }
        // a tune-in the audio service could not take is sent again on its schedule
        self.retry_due(t);
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
        self.why.motion += moving as u32;
        self.dirty |= moving;
        self.why.report(t, self.dirty);
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
        self.sync_smart();
        self.drawn_sec = (self.snap.position_ms / 1000.0) as i64;
        s.fill_rect(0, 0, self.w as usize, self.h as usize, CONTENT_BG.now());
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
            // a menu opened from the sheet (the output devices) sits over it; the library draws its own
            if p >= 1.0 && self.menu.is_some() {
                self.draw_menu(s, ui, self.pressed);
            }
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
                (Page::Smart(_), _) => self.draw_smart(&mut band, ui, cy(0.0), bottom, pressed),
                (Page::SmartEdit(p), _) => self.draw_rules(&mut band, ui, p, cy(0.0), bottom, pressed),
                (Page::History, _) => {
                    let ids = self.history_ids();
                    self.draw_song_rows(&mut band, ui, &ids, cy(0.0), bottom, pressed, true);
                }
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
                (Page::Info, _) => self.draw_info(&mut band, ui, cy(0.0), bottom, pressed),
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
                let (text, hint) = match (self.scope, self.search.is_empty()) {
                    (Scope::Favourites, true) => ("No Favourites", "Choose Add to Favourites from a song's ... menu"),
                    (Scope::RecentlyPlayed, true) => ("Nothing Played Yet", "Songs you play show up here"),
                    _ => ("No Results", ""),
                };
                self.label(&mut band, ui, text, MAC_TITLE1_PT, Weight::Semibold, LABEL_2.now(), self.w as f32 / 2.0, mid - ui.pt(SPACE_L), 0.5);
                self.label(&mut band, ui, hint, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), self.w as f32 / 2.0, mid + ui.pt(SPACE_L), 0.5);
            } else if page == Page::Queue && self.queue.items().is_empty() {
                self.label(&mut band, ui, "Nothing Queued", MAC_TITLE1_PT, Weight::Semibold, LABEL_2.now(), self.w as f32 / 2.0, mid - ui.pt(SPACE_L), 0.5);
            } else if page == Page::History && self.history_ids().is_empty() {
                self.label(&mut band, ui, "No History", MAC_TITLE1_PT, Weight::Semibold, LABEL_2.now(), self.w as f32 / 2.0, mid - ui.pt(SPACE_L), 0.5);
                self.label(&mut band, ui, "Songs you play show up here", MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), self.w as f32 / 2.0, mid + ui.pt(SPACE_L), 0.5);
            } else if page == Page::Root && self.tab == Tab::Playlists || matches!(page, Page::Playlist(_) | Page::Smart(_) | Page::SmartEdit(_) | Page::Info) {
            } else if !radio && self.library.is_empty() {
                let (text, hint) = library_empty_text(self.tab, self.scope, self.scanning);
                self.label(&mut band, ui, text, MAC_TITLE1_PT, Weight::Semibold, LABEL_2.now(), self.w as f32 / 2.0, mid - ui.pt(SPACE_L), 0.5);
                let hint = ui.fit(hint, MAC_SUBHEAD_PT, self.w as f32 - 2.0 * ui.pt(SPACE_L));
                self.label(&mut band, ui, &hint, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), self.w as f32 / 2.0, mid + ui.pt(SPACE_L), 0.5);
            }
        }
        s.fill_rect(0, 0, self.w as usize, top as usize, CONTENT_BG.now());
        s.fill_rect(0, bottom as usize, self.w as usize, (self.h as f32 - bottom).max(0.0) as usize, CONTENT_BG.now());
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
            let path = song.path.clone();
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
                if let Some(bpm) = self.bpm.get(&path).and_then(|b| tempo::label(*b)) {
                    self.label(s, ui, &bpm, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), self.w as f32 - ui.pt(BPM_RIGHT_PT), mid, 1.0);
                }
                let kept = self.stats.get(&path);
                if kept.favourite {
                    icons::heart(s, (self.w as f32 - ui.pt(FAVOURITE_X_PT), mid), ui.pt(SPACE_M), PINK);
                }
                // a rating shows as that many small stars under the length
                let star_r = ui.pt(STAR_R_PT);
                for cx in stars::star_centres(self.w as f32 - ui.pt(60.0), kept.rating, star_r, ui.pt(STAR_PITCH_PT)) {
                    icons::fill_poly(s, &stars::star_points((cx, mid + ui.pt(SPACE_M)), star_r), LABEL_2.now());
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
            let on_air = if is_tuned { live.as_ref().and_then(|l| l.error.clone().or(l.title.clone())) } else { None }.or(station.now.clone());
            let sub = [Some(station.frequency_text()).filter(|f| !f.is_empty()), on_air].into_iter().flatten().collect::<Vec<_>>().join(" · ");
            let sub = ui.fit(&sub, MAC_FOOTNOTE_PT, room);
            self.label(s, ui, &sub, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), text_x, mid + ui.pt(SPACE_M), 0.0);
            let fav = self.radio.is_favourite(station.id);
            let dim = if pressed == Some(Target::Fav(i)) { LABEL_3.now() } else if fav { PINK } else { LABEL_3.now() };
            icons::heart(s, (self.w as f32 - m - ui.pt(SPACE_M), mid), ui.pt(SPACE_XXL), dim);
            s.fill_rrect(text_x, y + row - 1.0, self.w as f32 - text_x, 1.0, 0.0, SEPARATOR_C.now());
        }
        // a short message when playback does not work, over the foot of the list
        if let Some(problem) = self.playback_problem() {
            let line = ui.pt(MAC_FOOTNOTE_PT + 2.0 * SPACE_M);
            s.fill_rect(0, (bottom - line).max(0.0) as usize, self.w as usize, line as usize, CONTENT_BG.now());
            let text = ui.fit(&format!("Cannot play: {problem}"), MAC_FOOTNOTE_PT, self.w as f32 - 2.0 * m);
            self.label(s, ui, &text, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), self.w as f32 / 2.0, bottom - line / 2.0, 0.5);
        }
    }

    /// The Playlists tab (`folders::rows`): New Playlist, New Smart Playlist and New Playlist Folder; each folder with
    /// its playlists under it (Show or Hide on the folder row); the playlists in no folder; the smart playlists.
    fn draw_playlists(&mut self, s: &mut Surface, ui: &Ui, y0: f32, bottom: f32, pressed: Option<Target>) {
        let row = ui.pt(ARTIST_ROW_PT);
        let (m, w) = (ui.pt(MARGIN_PT), self.w as f32);
        let rows = self.tab_rows();
        for (i, entry) in rows.into_iter().enumerate() {
            let y = y0 + i as f32 * row;
            if y + row < s.y0 as f32 || y > bottom {
                continue;
            }
            if pressed == Some(Target::Row(i)) {
                s.fill_rect(0, y.max(0.0) as usize, self.w as usize, row as usize, ROW_PRESSED.now());
            }
            let mid = y + row / 2.0;
            let indent = if matches!(entry, TabRow::Playlist { in_folder: true, .. }) { ui.pt(SPACE_XL) } else { 0.0 };
            let text_x = m + ui.pt(SPACE_S) + indent;
            let room = w - text_x - ui.pt(60.0);
            match entry {
                TabRow::NewPlaylist | TabRow::NewSmart | TabRow::NewFolder => {
                    let text = match entry {
                        TabRow::NewPlaylist => "New Playlist",
                        TabRow::NewSmart => "New Smart Playlist",
                        _ => "New Playlist Folder",
                    };
                    self.label(s, ui, text, MAC_BODY_PT, Weight::Regular, PINK, text_x, mid, 0.0);
                }
                TabRow::Folder(f) => {
                    let name = self.folders.names().get(f).cloned().unwrap_or_default();
                    let count = self.playlists.lists().iter().filter(|l| self.folders.folder_index(&l.name) == Some(f)).count();
                    let hidden = self.collapsed.contains(&name);
                    let name = ui.fit(&name, MAC_BODY_PT, room);
                    self.label(s, ui, &name, MAC_BODY_PT, Weight::Regular, LABEL.now(), text_x, mid - ui.pt(SPACE_M), 0.0);
                    let sub = format!("{count} {}", if count == 1 { "playlist" } else { "playlists" });
                    self.label(s, ui, &sub, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), text_x, mid + ui.pt(SPACE_M), 0.0);
                    self.label(s, ui, if hidden { "Show" } else { "Hide" }, MAC_SUBHEAD_PT, Weight::Regular, PINK, w - m, mid, 1.0);
                }
                TabRow::Playlist { index, .. } => {
                    let Some((name, count)) = self.playlists.lists().get(index).map(|l| (l.name.clone(), l.songs.len())) else { continue };
                    let name = ui.fit(&name, MAC_BODY_PT, room);
                    self.label(s, ui, &name, MAC_BODY_PT, Weight::Regular, LABEL.now(), text_x, mid - ui.pt(SPACE_M), 0.0);
                    let sub = format!("{count} {}", if count == 1 { "song" } else { "songs" });
                    self.label(s, ui, &sub, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), text_x, mid + ui.pt(SPACE_M), 0.0);
                    self.label(s, ui, "›", MAC_TITLE1_PT, Weight::Semibold, LABEL_3.now(), w - m, mid, 1.0);
                }
                TabRow::Smart(p) => {
                    let Some((name, rules)) = self.smart.lists().get(p).map(|l| (l.name.clone(), l.rules.len())) else { continue };
                    let name = ui.fit(&name, MAC_BODY_PT, room);
                    self.label(s, ui, &name, MAC_BODY_PT, Weight::Regular, LABEL.now(), text_x, mid - ui.pt(SPACE_M), 0.0);
                    let sub = format!("Smart Playlist · {rules} {}", if rules == 1 { "rule" } else { "rules" });
                    self.label(s, ui, &sub, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), text_x, mid + ui.pt(SPACE_M), 0.0);
                    self.label(s, ui, "›", MAC_TITLE1_PT, Weight::Semibold, LABEL_3.now(), w - m, mid, 1.0);
                }
            }
            s.fill_rrect(text_x, y + row - 1.0, w - text_x, 1.0, 0.0, SEPARATOR_C.now());
        }
    }

    /// The Play and Shuffle buttons at the top of a playlist page.
    fn draw_play_buttons(&mut self, s: &mut Surface, ui: &Ui, y0: f32, pressed: Option<Target>) {
        for (btn, (label, target)) in self.playlist_buttons(ui).into_iter().zip([("Play", Target::Play), ("Shuffle", Target::Shuffle)]) {
            let bg = if pressed == Some(target) { ROW_PRESSED.now() } else { CARD_BG.now() };
            let by = y0 + btn.y;
            s.fill_rrect(btn.x, by, btn.w, btn.h, ui.pt(BUTTON_H_PT / 2.0), bg);
            let (cx, cy) = (btn.x + btn.w / 2.0, by + btn.h / 2.0);
            self.label(s, ui, label, MAC_BODY_PT, Weight::Semibold, PINK, cx, cy, 0.5);
        }
    }

    /// A playlist page: Play and Shuffle, then its songs.
    fn draw_playlist(&mut self, s: &mut Surface, ui: &Ui, p: usize, y0: f32, bottom: f32, pressed: Option<Target>) {
        self.draw_play_buttons(s, ui, y0, pressed);
        let ids: Vec<usize> = self.playlist_entries(p).into_iter().map(|e| e.1).collect();
        self.draw_song_rows(s, ui, &ids, y0 + ui.pt(PLAYLIST_HEADER_PT), bottom, pressed, true);
        if ids.is_empty() {
            self.label(s, ui, "No Songs", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), self.w as f32 / 2.0, y0 + ui.pt(PLAYLIST_HEADER_PT + 60.0), 0.5);
            self.label(s, ui, "Use ... on a song to add it", MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), self.w as f32 / 2.0, y0 + ui.pt(PLAYLIST_HEADER_PT + 88.0), 0.5);
        }
    }

    /// A smart playlist page: Play and Shuffle, then the songs its rules pick.
    fn draw_smart(&mut self, s: &mut Surface, ui: &Ui, y0: f32, bottom: f32, pressed: Option<Target>) {
        self.draw_play_buttons(s, ui, y0, pressed);
        let ids = self.smart_view.clone();
        self.draw_song_rows(s, ui, &ids, y0 + ui.pt(PLAYLIST_HEADER_PT), bottom, pressed, true);
        if ids.is_empty() {
            self.label(s, ui, "No Matching Songs", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), self.w as f32 / 2.0, y0 + ui.pt(PLAYLIST_HEADER_PT + 60.0), 0.5);
            self.label(s, ui, "Edit Rules changes what it holds", MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), self.w as f32 / 2.0, y0 + ui.pt(PLAYLIST_HEADER_PT + 88.0), 0.5);
        }
    }

    /// The rules editor of smart playlist `p`: a Match row, one row per rule (field, operator, value, remove), Add Rule,
    /// and how many songs match now.
    fn draw_rules(&mut self, s: &mut Surface, ui: &Ui, p: usize, y0: f32, bottom: f32, pressed: Option<Target>) {
        let Some(list) = self.smart.lists().get(p).cloned() else { return };
        let row = ui.pt(SONG_ROW_PT);
        let (m, w) = (ui.pt(MARGIN_PT), self.w as f32);
        let cols = self.rule_columns(ui);
        let highlight = |s: &mut Surface, i: usize, target: Target| {
            if pressed == Some(target) {
                s.fill_rect(0, (y0 + i as f32 * row).max(0.0) as usize, w as usize, row as usize, ROW_PRESSED.now());
            }
        };
        let line = |s: &mut Surface, i: usize| s.fill_rrect(m, y0 + (i + 1) as f32 * row - 1.0, w - m, 1.0, 0.0, SEPARATOR_C.now());
        let clip = s.y0 as f32;
        let skip = |i: usize| y0 + (i + 1) as f32 * row < clip || y0 + i as f32 * row > bottom;
        // the match mode
        if !skip(0) {
            highlight(s, 0, Target::MatchToggle);
            let mid = y0 + row / 2.0;
            self.label(s, ui, "Match", MAC_BODY_PT, Weight::Regular, LABEL.now(), m, mid, 0.0);
            let mode = if list.all { "All Rules" } else { "Any Rule" };
            self.label(s, ui, mode, MAC_BODY_PT, Weight::Regular, PINK, w - m, mid, 1.0);
            line(s, 0);
        }
        // the rules
        for (r, rule) in list.rules.iter().enumerate() {
            let i = r + 1;
            if skip(i) {
                continue;
            }
            let mid = y0 + i as f32 * row + row / 2.0;
            if matches!(pressed, Some(Target::RuleField(k) | Target::RuleOp(k) | Target::RuleValue(k) | Target::RuleDel(k)) if k == r) {
                s.fill_rect(0, (y0 + i as f32 * row).max(0.0) as usize, w as usize, row as usize, ROW_PRESSED.now());
            }
            let field = ui.fit(rule.field.label(), MAC_BODY_PT, cols[1] - cols[0] - ui.pt(SPACE_S));
            self.label(s, ui, &field, MAC_BODY_PT, Weight::Regular, LABEL.now(), cols[0], mid, 0.0);
            let op = ui.fit(rule.op.label(), MAC_SUBHEAD_PT, cols[2] - cols[1] - ui.pt(SPACE_S));
            self.label(s, ui, &op, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), cols[1], mid, 0.0);
            // the value sits in a field; the one being typed shows a caret
            let editing = self.rule_edit.as_ref().filter(|(k, _)| *k == r).map(|(_, text)| format!("{text}|"));
            let value = editing.clone().unwrap_or_else(|| rule.value_text());
            let pad = ui.pt(SPACE_S);
            s.fill_rrect(cols[2], mid - ui.pt(SPACE_M), cols[3] - cols[2] - ui.pt(SPACE_S), 2.0 * ui.pt(SPACE_M), ui.pt(SPACE_XS), FIELD_BG.now());
            let value = ui.fit(&value, MAC_BODY_PT, cols[3] - cols[2] - ui.pt(SPACE_S) - 2.0 * pad);
            self.label(s, ui, &value, MAC_BODY_PT, Weight::Regular, if editing.is_some() { LABEL.now() } else { PINK }, cols[2] + pad, mid, 0.0);
            if list.rules.len() > 1 {
                let c = (cols[3] + ui.pt(FAV_ZONE_PT) / 2.0 - ui.pt(SPACE_S), mid);
                let (seg, either) = (springboard::shell::glyphs::segment, springboard::shell::glyphs::either);
                let (rad, t) = (ui.pt(SPACE_S), ui.pt(SPACE_XXS));
                let color = if pressed == Some(Target::RuleDel(r)) { LABEL_3.now() } else { LABEL_2.now() };
                springboard::shell::glyphs::paint(s, (c.0 - rad - t, c.1 - rad - t, 2.0 * rad + 2.0 * t, 2.0 * rad + 2.0 * t), color, either(seg((c.0 - rad, c.1 - rad), (c.0 + rad, c.1 + rad), t), seg((c.0 - rad, c.1 + rad), (c.0 + rad, c.1 - rad), t)));
            }
            line(s, i);
        }
        // Add Rule, then the live count
        let n = list.rules.len();
        if !skip(n + 1) {
            highlight(s, n + 1, Target::RuleAdd);
            self.label(s, ui, "Add Rule", MAC_BODY_PT, Weight::Regular, PINK, w / 2.0, y0 + (n + 1) as f32 * row + row / 2.0, 0.5);
            line(s, n + 1);
        }
        if !skip(n + 2) {
            let count = self.smart_view.len();
            let text = format!("{count} {} match", if count == 1 { "song" } else { "songs" });
            self.label(s, ui, &text, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), w / 2.0, y0 + (n + 2) as f32 * row + row / 2.0, 0.5);
        }
    }

    /// The queue page: the song playing (red), then Up Next, each with move-up and remove.
    fn draw_queue(&mut self, s: &mut Surface, ui: &Ui, y0: f32, bottom: f32, pressed: Option<Target>) {
        let row = ui.pt(SONG_ROW_PT);
        let (m, thumb, zone) = (ui.pt(MARGIN_PT), ui.pt(THUMB_PT), ui.pt(FAV_ZONE_PT));
        let text_x = m + thumb + ui.pt(SPACE_M);
        let current = self.queue.current();
        let rows = self.queue.items().len().saturating_sub(current);
        let items: Vec<PathBuf> = self.queue.items().iter().skip(current).cloned().collect();
        // while a row is dragged the list is drawn as it would be if dropped now
        let order = match self.qdrag {
            Some(d) => upnext::drag_order(rows, d.from, upnext::drop_row(d.y, row, rows)),
            None => (0..rows).collect(),
        };
        let dragged = self.qdrag.map(|d| d.from);
        for (i, source) in order.into_iter().enumerate() {
            let path = items[source].clone();
            let y = y0 + i as f32 * row;
            if y + row < s.y0 as f32 || y > bottom {
                continue;
            }
            if pressed == Some(Target::Row(i)) || dragged == Some(source) {
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
            let room = self.w as f32 - text_x - if i == 0 { m } else { 3.0 * zone };
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
            if i >= 1 {
                // the grab handle: three bars to drag the row to another place
                let c = (self.w as f32 - zone * 2.5, mid);
                let color = if dragged == Some(source) { LABEL.now() } else { LABEL_2.now() };
                for k in [-1.0f32, 0.0, 1.0] {
                    s.fill_rrect(c.0 - r, c.1 + k * ui.pt(SPACE_XS) - t / 2.0, 2.0 * r, t, 0.0, color);
                }
            }
            s.fill_rrect(text_x, y + row - 1.0, self.w as f32 - text_x, 1.0, 0.0, SEPARATOR_C.now());
        }
        // Clear Up Next: one text row after the last song, shown only when something is upcoming
        if !self.queue.upcoming().is_empty() {
            let y = y0 + rows as f32 * row;
            if y + row >= s.y0 as f32 && y <= bottom {
                if pressed == Some(Target::QClear) {
                    s.fill_rect(0, y.max(0.0) as usize, self.w as usize, row as usize, ROW_PRESSED.now());
                }
                let mid = y + row / 2.0;
                self.label(s, ui, "Clear Up Next", MAC_BODY_PT, Weight::Regular, PINK, self.w as f32 / 2.0, mid, 0.5);
            }
        }
    }

    /// The song menu: the song's title, then Play Next / Play Last / Add to
    /// each playlist / New Playlist (/ Remove from this one).
    fn draw_menu(&mut self, s: &mut Surface, ui: &Ui, pressed: Option<Target>) {
        let Some((song, items, heading)) = self.menu.as_ref().map(|m| (m.song, m.items.clone(), m.heading.clone())) else { return };
        s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, black_at(150));
        let card = self.menu_rect(ui, items.len());
        s.fill_rrect(card.x, card.y, card.w, card.h, ui.pt(m::CARD_RADIUS), CARD_BG.now());
        let heading = heading.unwrap_or_else(|| self.library.songs[song].title.clone());
        let title = ui.fit(&heading, MAC_SUBHEAD_PT, card.w - ui.pt(SPACE_XXXL));
        self.label(s, ui, &title, MAC_SUBHEAD_PT, Weight::Semibold, LABEL_2.now(), card.x + card.w / 2.0, card.y + ui.pt(MENU_HEAD_PT) / 2.0, 0.5);
        let here = self.shown_folder();
        for (k, item) in items.into_iter().enumerate() {
            let y = card.y + ui.pt(MENU_HEAD_PT) + k as f32 * ui.pt(MENU_ROW_PT);
            if pressed == Some(Target::MenuItem(k)) {
                s.fill_rect(card.x as usize, y as usize, card.w as usize, ui.pt(MENU_ROW_PT) as usize, ROW_PRESSED.now());
            }
            s.fill_rrect(card.x, y, card.w, 1.0, 0.0, SEPARATOR_C.now());
            let (text, color) = match item {
                MenuItem::PlayNext => ("Play Next".to_string(), LABEL.now()),
                MenuItem::PlayLast => ("Play Last".to_string(), LABEL.now()),
                MenuItem::Rate => (format!("Rating: {}", stats::rating_text(self.stats.rating(&self.library.songs[song].path))), LABEL.now()),
                MenuItem::ToggleFavourite => {
                    let favourite = self.stats.is_favourite(&self.library.songs[song].path);
                    (String::from(if favourite { "Remove from Favourites" } else { "Add to Favourites" }), LABEL.now())
                }
                MenuItem::Output(k) => match self.outputs.get(k) {
                    Some((id, name)) => (name.clone(), if self.output_now.as_ref() == Some(id) { PINK } else { LABEL.now() }),
                    None => (String::new(), LABEL.now()),
                },
                MenuItem::AddTo(p) => (format!("Add to {}", self.playlists.lists().get(p).map_or("", |l| l.name.as_str())), LABEL.now()),
                MenuItem::NewPlaylist => ("Add to New Playlist".to_string(), LABEL.now()),
                MenuItem::RemoveFromPlaylist(..) => ("Remove from Playlist".to_string(), PINK),
                MenuItem::GetInfo => ("Get Info".to_string(), LABEL.now()),
                MenuItem::MoveToFolder(f) => {
                    let name = f.and_then(|i| self.folders.names().get(i).cloned()).unwrap_or_else(|| "No Folder".to_string());
                    (name, if f == here { PINK } else { LABEL.now() })
                }
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

    /// The Get Info page of the song at `info_path`: Title, Artist and Album (tap one to type a new value), Restore
    /// Original Tags, then the file's name and its length.
    fn draw_info(&mut self, s: &mut Surface, ui: &Ui, y0: f32, bottom: f32, pressed: Option<Target>) {
        let Some(id) = self.info_song() else { return };
        let song = self.library.songs[id].clone();
        let row = ui.pt(SONG_ROW_PT);
        let (m, w) = (ui.pt(MARGIN_PT), self.w as f32);
        let value_x = m + ui.pt(INFO_LABEL_PT);
        for i in 0..INFO_ROWS {
            let y = y0 + i as f32 * row;
            if y + row < s.y0 as f32 || y > bottom {
                continue;
            }
            let mid = y + row / 2.0;
            let target = match i {
                0..=2 => Some(Target::InfoField(i)),
                3 => Some(Target::InfoReset),
                _ => None,
            };
            if target.is_some() && pressed == target {
                s.fill_rect(0, y.max(0.0) as usize, w as usize, row as usize, ROW_PRESSED.now());
            }
            match i {
                0..=2 => {
                    let field = Field::ALL[i];
                    let typed = self.info_edit.as_ref().filter(|(f, _)| *f == field).map(|(_, text)| format!("{text}|"));
                    let value = typed.unwrap_or_else(|| self.info_shown(&song, field));
                    let value = ui.fit(&value, MAC_BODY_PT, w - value_x - m);
                    self.label(s, ui, field.label(), MAC_BODY_PT, Weight::Regular, LABEL.now(), m, mid, 0.0);
                    self.label(s, ui, &value, MAC_BODY_PT, Weight::Regular, LABEL.now(), value_x, mid, 0.0);
                }
                3 => {
                    self.label(s, ui, "Restore Original Tags", MAC_BODY_PT, Weight::Regular, PINK, w / 2.0, mid, 0.5);
                }
                4 => {
                    let file = song.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    let file = ui.fit(&file, MAC_BODY_PT, w - value_x - m);
                    self.label(s, ui, "File", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), m, mid, 0.0);
                    self.label(s, ui, &file, MAC_BODY_PT, Weight::Regular, LABEL.now(), value_x, mid, 0.0);
                }
                _ => {
                    let length = song.duration_ms.map_or_else(|| "-:--".to_string(), clock);
                    self.label(s, ui, "Length", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), m, mid, 0.0);
                    self.label(s, ui, &length, MAC_BODY_PT, Weight::Regular, LABEL.now(), value_x, mid, 0.0);
                }
            }
            s.fill_rrect(m, y + row - 1.0, w - m, 1.0, 0.0, SEPARATOR_C.now());
        }
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
        // how far in (a station: LIVE and how long it has been on) and the volume, left of the route button
        if let Some(track) = mini_volume_track(ui, &mini) {
            // drawn in the label colours (the sheet's white slider would vanish on the light bar)
            let fx = track.x + track.w * self.volume.clamp(0.0, 1.0);
            s.fill_rrect(track.x, track.y, track.w, track.h, track.h / 2.0, SEPARATOR_C.now());
            s.fill_rrect(track.x, track.y, fx - track.x, track.h, track.h / 2.0, LABEL_2.now());
            let k = ui.pt(if self.vol_drag == Some(track) { SPACE_M } else { SPACE_S });
            s.fill_rrect(fx - k / 2.0, track.y + track.h / 2.0 - k / 2.0, k, k, k / 2.0, LABEL.now());
            if let Some(right) = mini_status_right(ui, &mini) {
                let status = mini_status(self.is_live(), self.snap.position_ms, self.snap.duration_ms);
                let im = ui.text(&ui.fit(&status, MAC_FOOTNOTE_PT, ui.pt(MINI_STATUS_MIN_PT)), MAC_FOOTNOTE_PT, Weight::Regular, WHITE);
                let x = right - im.w as f32;
                s.blit_colored(&im, x as i32, (mini.bar.y + (mini.bar.h - im.h as f32) / 2.0) as i32, LABEL_2.now(), 255);
            }
        }
        let dim = |t: nowplaying::Target| if self.pressed == Some(Target::Mini(t)) { LABEL_3.now() } else { LABEL.now() };
        icons::play_pause(s, mini.play, ui.pt(SPACE_XXL), self.snap.playing, dim(nowplaying::Target::Play));
        icons::skip(s, mini.next, ui.pt(SPACE_XL), true, dim(nowplaying::Target::Next));
        let color = if self.pressed == Some(Target::MiniRoute) { LABEL_3.now() } else { LABEL.now() };
        draw_route(s, ui, Self::mini_route(ui, &mini), color);
    }

    /// The lyrics over the artwork of the Now Playing sheet: the line being sung in the middle, with the lines around it.
    /// Plain lyrics (no times) are listed from the top.
    fn draw_lyrics(&mut self, s: &mut Surface, ui: &Ui, area: R, dy: f32) {
        let Some(lyrics) = &self.lyrics else { return };
        let current = lyrics.current(self.snap.position_ms);
        let line_h = ui.pt(LYRIC_LINE_PT);
        let visible = ((area.h / line_h) as usize).max(1);
        let first = current.map_or(0, |c| c.saturating_sub(visible / 2));
        let shown: Vec<(usize, String)> = lyrics.lines().iter().enumerate().skip(first).take(visible).map(|(k, line)| (k, line.text.clone())).collect();
        s.fill_rrect(area.x, area.y + dy, area.w, area.h, ui.pt(ART_RADIUS), black_at(170));
        let width = area.w - 2.0 * ui.pt(SPACE_L);
        for (k, text) in shown {
            let mid = area.y + dy + (k - first) as f32 * line_h + line_h / 2.0;
            let (pt, weight, color) = if current == Some(k) { (MAC_TITLE3_PT, Weight::Semibold, WHITE) } else { (MAC_BODY_PT, Weight::Regular, white_at(120)) };
            let text = ui.fit(&text, pt, width);
            self.label(s, ui, &text, pt, weight, color, area.x + area.w / 2.0, mid, 0.5);
        }
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
        if self.lyrics_on && !self.is_live() {
            self.draw_lyrics(s, ui, l.art, dy);
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
        // the output route, beside Up Next
        draw_route(s, ui, (self.w as f32 - side - ui.pt(SPACE_L), l.queue.1 + dy), dim);
        // the lyrics switch, when the song has lyrics
        if self.lyrics.is_some() && !live {
            let (left, _) = self.sheet_buttons(ui);
            let c = (left.0, left.1 + dy);
            if self.lyrics_on {
                s.fill_rrect(c.0 - ui.pt(SPACE_XXXL), c.1 - ui.pt(SPACE_L), ui.pt(64.0), ui.pt(SPACE_XXXL), ui.pt(SPACE_S), white_at(70));
            }
            let color = if self.pressed == Some(Target::Lyrics) { white_at(140) } else if self.lyrics_on { WHITE } else { dim };
            self.label(s, ui, "Lyrics", MAC_CAPTION1_PT, Weight::Semibold, color, c.0, c.1, 0.5);
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
/// The height of one line of lyrics on the Now Playing sheet.
const LYRIC_LINE_PT: f32 = 32.0;

/// The output route glyph: a triangle under a bar, centred on `c`.
fn draw_route(s: &mut Surface, ui: &Ui, c: (f32, f32), color: u32) {
    let k = ui.pt(SPACE_S);
    icons::fill_poly(s, &[(c.0 - k * 0.7, c.1 + k), (c.0 + k * 0.7, c.1 + k), (c.0, c.1 + k * 0.1)], color);
    s.fill_rrect(c.0 - k, c.1 - k, 2.0 * k, ui.pt(1.8), 0.0, color);
    s.fill_rrect(c.0 - k, c.1 - k, ui.pt(1.8), k * 1.3, 0.0, color);
    s.fill_rrect(c.0 + k - ui.pt(1.8), c.1 - k, ui.pt(1.8), k * 1.3, 0.0, color);
}

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
    matches!(
        t,
        Target::Row(_) | Target::Fav(_) | Target::More(_) | Target::QUp(_) | Target::QDel(_) | Target::QClear | Target::Content | Target::Play | Target::Shuffle | Target::InfoField(_) | Target::InfoReset
            | Target::MatchToggle | Target::RuleField(_) | Target::RuleOp(_) | Target::RuleValue(_) | Target::RuleDel(_) | Target::RuleAdd
    )
}

/// The artwork cache key of a station's logo.
fn logo_key(id: u64) -> String {
    format!("r{id}")
}

#[cfg(test)]
mod tests {
    use super::library::Scanned;
    use super::*;
    use springboard::media::audiofile::Tags;
    use std::path::Path;

    fn scanned(title: &str, artist: &str, added: u64) -> Scanned {
        Scanned {
            path: PathBuf::from(format!("/m/{title}.mp3")),
            tags: Tags { title: Some(title.into()), artist: Some(artist.into()), album: Some("Album".into()), album_artist: None, track: None, year: None, cover: None, lyrics: None, chapters: Vec::new(), replaygain_db: None },
            duration_ms: Some(200_000.0),
            has_cover: false,
            added,
        }
    }

    fn folder(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-music-app").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// A Music over three songs (Alpha and Charlie by Abba, Bravo by Blur; ids 0 to 2 in title order, Alpha the newest
    /// file). The service is not connected, so commands to it are dropped. Everything it keeps is under `dir`.
    fn music(dir: &Path) -> Music {
        let mut m = Music::new(800, 600, vec![dir.join("Music")], LoadOptions::all(), Client::for_root(dir)).unwrap();
        m.enable_library_store(dir.join("data"));
        let files = vec![scanned("Alpha", "Abba", 300), scanned("Bravo", "Blur", 200), scanned("Charlie", "Abba", 100)];
        m.apply(Done::Scanned(files, Vec::new()));
        m
    }

    fn song_path(m: &Music, id: usize) -> PathBuf {
        m.library.songs[id].path.clone()
    }

    fn test_ui() -> Ui {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
        let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
        Ui::new(bifrost_app::kit::Font::parse(data).unwrap(), 1.0)
    }

    #[test]
    fn an_empty_library_names_the_section_and_says_how_to_add_music() {
        assert_eq!(library_empty_text(Tab::Albums, Scope::All, false).0, "No Albums");
        assert_eq!(library_empty_text(Tab::Artists, Scope::All, false).0, "No Artists");
        assert_eq!(library_empty_text(Tab::Songs, Scope::All, false).0, "No Songs");
        assert_eq!(library_empty_text(Tab::Songs, Scope::RecentlyAdded, false).0, "Nothing Added Yet");
        for (tab, scope) in [(Tab::Artists, Scope::All), (Tab::Songs, Scope::Favourites), (Tab::Songs, Scope::RecentlyPlayed)] {
            let hint = library_empty_text(tab, scope, false).1;
            assert!(hint.contains("Import") && hint.contains("Music folder"), "{hint}");
        }
        assert_eq!(library_empty_text(Tab::Songs, Scope::All, true).1, "", "no hint while the folder is read");
    }

    #[test]
    fn the_sidebar_puts_radio_above_the_library() {
        let dir = folder("sidebar-order");
        let m = music(&dir);
        let rows: Vec<String> = m
            .sidebar()
            .items
            .iter()
            .map(|i| match i {
                bifrost_app::controls::sidebar::SidebarItem::Row { label, .. } => label.clone(),
                bifrost_app::controls::sidebar::SidebarItem::Header(label) | bifrost_app::controls::sidebar::SidebarItem::Section { label, .. } => format!("[{label}]"),
            })
            .collect();
        assert_eq!(rows[0], "Radio");
        assert_eq!(rows[1], "[Library]");
        assert_eq!(rows.iter().filter(|r| r.as_str() == "Radio").count(), 1, "one Radio row");
        assert!(rows.iter().position(|r| r == "Artists") < rows.iter().position(|r| r == "[Playlists]"));
    }

    #[test]
    fn the_mini_bar_says_live_and_the_time_on_air_for_a_station_and_elapsed_of_total_for_a_song() {
        assert_eq!(mini_status(true, 83_000.0, None), "LIVE \u{00b7} 1:23");
        assert_eq!(mini_status(false, 83_000.0, Some(225_000.0)), "1:23 / 3:45");
        assert_eq!(mini_status(false, 300_000.0, Some(225_000.0)), "3:45 / 3:45", "never past the end");
        assert_eq!(mini_status(false, 5_000.0, None), "0:05", "a song of unknown length shows the elapsed time");
    }

    #[test]
    fn the_mini_bar_has_a_volume_slider_left_of_the_route_button_when_it_is_wide_enough() {
        let ui = test_ui();
        let wide = Mini::new(&ui, 900, 600);
        let track = mini_volume_track(&ui, &wide).expect("a wide bar has a volume slider");
        let route_x = wide.play.0 - ui.pt(MINI_ROUTE_GAP_PT);
        assert!(track.x + track.w < route_x && track.x > wide.text_x(&ui), "{track:?}");
        assert!(wide.bar.contains(track.x, track.y), "the slider sits in the bar");
        let right = mini_status_right(&ui, &wide).expect("a wide bar shows the time");
        assert!(right <= track.x && mini_text_w(&ui, 900, 600) + wide.text_x(&ui) + ui.pt(MINI_STATUS_MIN_PT) <= right, "the title stops before the time and the slider");
        let middle = Mini::new(&ui, 500, 600);
        if mini_volume_track(&ui, &middle).is_some() && mini_status_right(&ui, &middle).is_none() {
            assert!(mini_text_w(&ui, 500, 600) >= ui.pt(MINI_TITLE_MIN_PT), "without the time the title keeps its room");
        }
        let narrow = Mini::new(&ui, 400, 600);
        assert_eq!(mini_volume_track(&ui, &narrow), None, "a narrow bar keeps only its buttons");
    }

    #[test]
    fn dragging_the_mini_bar_volume_sets_the_volume() {
        let dir = folder("mini-volume");
        let mut m = music(&dir);
        let ui = test_ui();
        let mini = Mini::new(&ui, m.w, m.h);
        let Some(track) = mini_volume_track(&ui, &mini) else { return };
        m.vol_drag = Some(track);
        m.touch(&ui, Event::Move(track.x + track.w * 0.25, track.y), 0.0);
        assert!((m.volume - 0.25).abs() < 0.02, "{}", m.volume);
        m.touch(&ui, Event::Up(track.x + track.w * 0.25, track.y), 0.0);
        assert_eq!(m.vol_drag, None);
    }

    #[test]
    fn a_command_the_service_cannot_take_leaves_its_cause_on_screen() {
        let dir = folder("service-down");
        let mut m = music(&dir);
        assert!(m.playback_problem().is_none());
        m.report("tune in the station", Err("the audio service is not reachable (no socket)".into()));
        assert_eq!(m.playback_problem().as_deref(), Some("the audio service is not reachable (no socket)"));
        m.report("tune in the station", Ok(()));
        assert!(m.playback_problem().is_none(), "a command that was sent clears it");
    }

    #[test]
    fn the_same_failure_again_is_not_a_repaint() {
        let dir = folder("same-failure");
        let mut m = music(&dir);
        let cause = || Err("the audio service is not reachable (no socket)".to_string());
        m.report("tune in the station", cause());
        assert!(m.dirty, "the first failure is shown");
        m.dirty = false;
        m.report("tune in the station", cause());
        assert!(!m.dirty, "the same message again changes nothing on screen");
        m.report("tune in the station", Err("the audio service is not reachable (refused)".into()));
        assert!(m.dirty, "another cause is shown");
    }

    #[test]
    fn a_failed_tune_in_waits_for_its_retry_and_the_persons_next_choice_ends_it() {
        let dir = folder("retry-ends");
        let mut m = music(&dir);
        // armed a second ago: due at 100 ms on the app clock, before the usual 250 ms wake
        m.retry.arm(retry::Wanted::Station(99), -900.0);
        assert_eq!(m.next_wake(10.0), Some(100.0), "the app wakes for the retry");
        m.retry_due(99.0);
        assert_eq!(m.retry.pending(), Some(retry::Wanted::Station(99)), "not yet due");
        // due, but the station is no longer in the list: forgotten, not sent
        m.retry_due(100.0);
        assert_eq!(m.retry.pending(), None);
        // pausing, skipping or any other transport choice ends a pending retry
        m.retry.arm(retry::Wanted::Station(99), 0.0);
        m.control(nowplaying::Target::Next);
        assert_eq!(m.retry.pending(), None, "a transport choice ends the retries");
        // choosing a song does too
        m.retry.arm(retry::Wanted::Station(99), 0.0);
        m.push_queue(0.0, true);
        assert_eq!(m.retry.pending(), None);
    }

    #[test]
    fn tuning_in_without_the_service_reports_the_unreachable_socket() {
        let dir = folder("tune-without-service");
        let mut m = music(&dir);
        let tracks = vec![Track::live(4, "http://example.test/live".into())];
        let sent = m.player.play_queue(tracks, 0, false, 1);
        let cause = sent.expect_err("nothing listens on the service socket");
        assert!(cause.contains("audio service"), "{cause}");
    }

    #[test]
    fn the_recent_and_favourite_lists_follow_the_listening_data() {
        let dir = folder("scopes");
        let mut m = music(&dir);
        m.set_scope(Scope::Favourites);
        assert!(m.view.is_empty());
        let charlie = song_path(&m, 2);
        m.stats.toggle_favourite(&charlie);
        m.stats_changed();
        assert_eq!(m.view, [2], "a new favourite shows at once");
        m.set_scope(Scope::RecentlyAdded);
        assert_eq!(m.view, [0, 1, 2], "newest file first");
        m.set_scope(Scope::RecentlyPlayed);
        assert!(m.view.is_empty());
        let alpha = song_path(&m, 0);
        m.stats.record_start(&alpha, 50);
        m.stats_changed();
        assert_eq!(m.view, [0]);
        // the Songs row of the source list leaves the scope and its order
        m.activate(Target::Seg(2), 0.0);
        assert_eq!((m.scope, m.sort, m.view.len()), (Scope::All, SortKey::Title, 3));
        assert_eq!(m.title(), "Songs");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn menu_commands_point_the_window_and_act_on_nothing_without_a_song() {
        let dir = folder("menus");
        let mut m = music(&dir);
        let state = m.menu_state();
        assert!(!state.has_track && !state.has_song);
        assert_eq!(state.place, Place::Songs);
        assert!(m.run_command(Command::Rate(3)));
        assert!(m.stats.is_empty(), "no song is playing, so no rating is set");
        m.run_command(Command::Show(Place::Favourites));
        assert_eq!((m.scope, m.menu_state().place), (Scope::Favourites, Place::Favourites));
        m.run_command(Command::Show(Place::Playlists));
        assert_eq!((m.tab, m.scope), (Tab::Playlists, Scope::All));
        assert_eq!(m.menu_state().place, Place::Playlists);
        m.run_command(Command::SortBy(SortKey::Rating));
        assert_eq!((m.tab, m.sort), (Tab::Songs, SortKey::Rating), "sorting shows the songs");
        m.run_command(Command::Show(Place::History));
        assert_eq!((m.page(), m.menu_state().place), (Page::History, Place::History));
        assert!(!m.run_command(Command::Import), "the window owns the open panel");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_crossfade_choice_is_kept_in_the_menu_state_and_the_library_store() {
        let dir = folder("crossfade");
        let mut m = music(&dir);
        assert_eq!(m.menu_state().crossfade_secs, 0, "gapless until chosen");
        assert!(m.run_command(Command::Crossfade(6)));
        assert_eq!(m.menu_state().crossfade_secs, 6);
        assert!(m.run_command(Command::Crossfade(99)));
        assert_eq!(m.menu_state().crossfade_secs, 12, "cut to the longest crossfade");
        m.run_command(Command::Crossfade(4));
        let saved = std::fs::read_to_string(dir.join("data").join("playback.txt")).expect("the choice is written");
        assert!(saved.contains("crossfade 4"), "{saved}");
        // a new session over the same store starts with the saved choice
        let again = music(&dir);
        assert_eq!(again.menu_state().crossfade_secs, 4);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_smart_playlist_opens_its_rules_and_lists_the_songs_they_pick() {
        let dir = folder("smart");
        let mut m = music(&dir);
        m.activate(Target::Seg(4), 0.0);
        assert_eq!(m.playlist_rows(), 3, "New Playlist, New Smart Playlist and New Playlist Folder");
        m.activate(Target::Row(1), 0.0);
        assert_eq!(m.page(), Page::SmartEdit(0));
        assert_eq!(m.playlist_rows(), 4);
        m.sync_smart();
        assert_eq!(m.smart_view, [0, 1, 2], "a rule without a value keeps every song");
        // Artist contains "abba"
        m.activate(Target::RuleValue(0), 0.0);
        assert!(m.wants_keyboard());
        m.type_text("abba");
        m.type_text("\n");
        assert!(!m.wants_keyboard());
        m.sync_smart();
        assert_eq!(m.smart_view, [0, 2]);
        // a second rule on the title, matching any
        m.activate(Target::RuleAdd, 0.0);
        m.activate(Target::MatchToggle, 0.0);
        m.activate(Target::RuleValue(1), 0.0);
        m.type_text("bravo");
        m.activate(Target::Content, 0.0);
        m.sync_smart();
        assert_eq!(m.smart_view, [0, 1, 2], "tapping elsewhere keeps the typed value");
        // back to the list: the playlist is there, opens, renames and deletes
        m.activate(Target::Back, 0.0);
        m.activate(Target::Row(3), 0.0);
        assert_eq!(m.page(), Page::Smart(0));
        assert_eq!(m.title(), "Smart Playlist");
        m.activate(Target::Rename, 0.0);
        assert_eq!(m.renaming.as_deref(), Some("Smart Playlist"));
        m.renaming = Some("Mix".into());
        m.type_text("\n");
        assert_eq!(m.smart.lists()[0].name, "Mix");
        assert_eq!(m.page(), Page::Smart(0));
        m.activate(Target::DeletePlaylist, 0.0);
        assert!(m.smart.lists().is_empty());
        assert_eq!(m.page(), Page::Root);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cmd_a_selects_the_name_being_renamed_so_typing_replaces_it() {
        let dir = folder("rename-all");
        let mut m = music(&dir);
        m.activate(Target::Seg(4), 0.0);
        m.activate(Target::Row(1), 0.0);
        m.activate(Target::Back, 0.0);
        m.activate(Target::Row(3), 0.0);
        assert_eq!(m.page(), Page::Smart(0));
        assert!(!m.select_all_name(), "no name is being typed yet");
        m.activate(Target::Rename, 0.0);
        assert!(m.select_all_name());
        m.type_text("Mix");
        assert_eq!(m.renaming.as_deref(), Some("Mix"), "the typed text replaces the selected name");
        m.type_text("\n");
        assert_eq!(m.smart.lists()[0].name, "Mix");
        // Backspace on a selected name clears it all
        m.activate(Target::Rename, 0.0);
        assert!(m.select_all_name());
        m.backspace();
        assert_eq!(m.renaming.as_deref(), Some(""));
        // a new rename starts unselected: typing appends
        m.cancel(0.0);
        m.activate(Target::Rename, 0.0);
        m.type_text("!");
        assert_eq!(m.renaming.as_deref(), Some("Mix!"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_smart_playlist_follows_ratings_as_they_change() {
        let dir = folder("smart-live");
        let mut m = music(&dir);
        m.activate(Target::Seg(4), 0.0);
        m.activate(Target::Row(1), 0.0);
        // the starting rule is on the artist; Rating is three fields on (Album, Year, Rating)
        for _ in 0..3 {
            m.activate(Target::RuleField(0), 0.0);
        }
        assert_eq!(m.smart.lists()[0].rules[0].field, smart::Field::Rating);
        m.activate(Target::RuleOp(0), 0.0);
        m.activate(Target::RuleOp(0), 0.0);
        assert_eq!(m.smart.lists()[0].rules[0].op, smart::Op::Greater);
        m.activate(Target::RuleValue(0), 0.0);
        m.rule_edit = Some((0, "3".to_string()));
        m.type_text("\n");
        m.sync_smart();
        assert!(m.smart_view.is_empty(), "nothing is rated above 3 yet");
        let bravo = song_path(&m, 1);
        m.stats.set_rating(&bravo, 5);
        m.stats_changed();
        m.sync_smart();
        assert_eq!(m.smart_view, [1]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_song_menu_steps_the_rating_and_flips_the_favourite() {
        let dir = folder("song-menu");
        let mut m = music(&dir);
        let bravo = song_path(&m, 1);
        m.open_menu(1, None);
        let items = m.menu.as_ref().map(|menu| menu.items.clone()).unwrap();
        assert_eq!(items[..4].to_vec(), vec![MenuItem::PlayNext, MenuItem::PlayLast, MenuItem::Rate, MenuItem::ToggleFavourite]);
        m.run_menu(MenuItem::Rate, 1);
        m.run_menu(MenuItem::Rate, 1);
        assert_eq!(m.stats.rating(&bravo), 2);
        m.run_menu(MenuItem::ToggleFavourite, 1);
        assert!(m.stats.is_favourite(&bravo));
        // the data is kept: a second Music over the same folder reads it back
        let again = music(&dir);
        assert_eq!((again.stats.rating(&bravo), again.stats.is_favourite(&bravo)), (2, true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_song_that_plays_is_logged_in_the_history_and_counted_after_thirty_seconds() {
        let dir = folder("plays");
        let mut m = music(&dir);
        let alpha = song_path(&m, 0);
        m.snap.playing = true;
        m.snap.track = Some(Track::file(0, alpha.clone()));
        m.snap.duration_ms = Some(200_000.0);
        m.snap.position_ms = 1_000.0;
        m.track_plays();
        assert_eq!(m.history_ids(), [0]);
        assert_eq!(m.stats.plays(&alpha), 0);
        m.snap.position_ms = 31_000.0;
        m.track_plays();
        assert_eq!(m.stats.plays(&alpha), 1);
        m.snap.playing = false;
        m.snap.position_ms = 120_000.0;
        m.track_plays();
        assert_eq!(m.stats.plays(&alpha), 1, "a paused song counts for nothing");
        assert_eq!(m.current_song(), Some(0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dragging_an_up_next_row_moves_that_song() {
        let dir = folder("drag");
        let mut m = music(&dir);
        let paths: Vec<PathBuf> = (0..3).map(|id| song_path(&m, id)).collect();
        m.queue = UpNext::new(paths.clone(), 0);
        m.queue_move(1, 0);
        assert_eq!(m.queue.items().to_vec(), vec![paths[0].clone(), paths[2].clone(), paths[1].clone()]);
        m.queue_move(0, 0);
        assert_eq!(m.queue.items().to_vec(), vec![paths[0].clone(), paths[2].clone(), paths[1].clone()], "dropped where it was");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_route_button_asks_for_the_devices_and_the_chosen_one_is_handed_over() {
        let dir = folder("outputs");
        let mut m = music(&dir);
        assert!(!m.take_output_request());
        m.activate(Target::Route, 0.0);
        assert!(m.take_output_request());
        assert!(!m.take_output_request(), "asked for once");
        m.show_outputs(vec![("speaker".into(), "Speakers".into()), ("headphones".into(), "Headphones".into())], Some("speaker".into()));
        assert!(m.overlay());
        m.activate(Target::MenuItem(1), 0.0);
        assert_eq!(m.take_output_choice().as_deref(), Some("headphones"));
        assert_eq!(m.take_output_choice(), None);
        assert!(!m.overlay());
        m.show_outputs(Vec::new(), None);
        assert!(!m.overlay(), "no devices, no menu");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_copies_songs_into_the_first_music_folder_and_scans_again() {
        let dir = folder("import");
        let mut m = music(&dir);
        assert!(m.import_song("riff.mid", &mut &b"MThd"[..]));
        assert!(dir.join("Music/Imported/riff.mid").exists());
        assert!(!m.import_song("notes.txt", &mut &b"x"[..]));
        assert!(!m.is_scanning());
        m.imports_done(0);
        assert!(!m.is_scanning(), "nothing copied, nothing to scan");
        m.imports_done(1);
        assert!(m.is_scanning());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn get_info_edits_a_song_and_the_scan_lays_the_edit_over_its_tag() {
        let dir = folder("get-info");
        let mut m = music(&dir);
        let bravo = song_path(&m, 1);
        m.open_info(1);
        assert_eq!(m.page(), Page::Info);
        assert_eq!(m.title(), "Get Info");
        m.activate(Target::InfoField(0), 0.0);
        assert!(m.wants_keyboard());
        assert_eq!(m.info_edit.as_ref().map(|(f, t)| (*f, t.as_str())), Some((Field::Title, "Bravo")));
        m.info_edit = Some((Field::Title, "Bravo Live".into()));
        m.type_text("\n");
        assert!(!m.wants_keyboard());
        assert_eq!(m.info.value(&bravo, Field::Title), Some("Bravo Live"));
        // the scan that follows reads the tags again and lays the edit over them
        m.apply(Done::Scanned(vec![scanned("Bravo", "Blur", 200)], Vec::new()));
        assert!(m.library.songs.iter().any(|s| s.path == bravo && s.title == "Bravo Live"));
        m.activate(Target::InfoReset, 0.0);
        assert_eq!(m.info.value(&bravo, Field::Title), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_playlist_moves_into_a_folder_from_its_page_and_the_tab_nests_it() {
        let dir = folder("folders");
        let mut m = music(&dir);
        m.activate(Target::Seg(4), 0.0);
        m.run_command(Command::NewFolder);
        assert_eq!(m.tab, Tab::Playlists);
        assert_eq!(m.folders.names(), ["Folder"]);
        m.activate(Target::Row(0), 0.0);
        assert_eq!(m.page(), Page::Playlist(0));
        m.open_folder_menu();
        m.activate(Target::MenuItem(1), 0.0);
        assert_eq!(m.folders.folder_of("Playlist"), Some("Folder"));
        m.activate(Target::Back, 0.0);
        let rows = m.tab_rows();
        assert_eq!(rows[3], TabRow::Folder(0));
        assert_eq!(rows[4], TabRow::Playlist { index: 0, in_folder: true });
        m.activate(Target::Row(3), 0.0);
        assert_eq!(m.tab_rows().len(), 4, "the folder is hidden and its playlist with it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lyrics_are_read_beside_the_playing_song_and_dropped_with_it() {
        let dir = folder("lyrics");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.lrc"), "[00:01.00]one\n[00:05.00]two\n").unwrap();
        let mut m = music(&dir);
        m.snap.track = Some(Track::file(0, dir.join("x.mp3")));
        m.sync_lyrics();
        assert_eq!(m.lyrics.as_ref().map(|l| l.lines().len()), Some(2));
        assert!(m.menu_state().has_lyrics);
        m.run_command(Command::Lyrics);
        assert!(m.lyrics_on && m.menu_state().lyrics_shown);
        m.snap.track = Some(Track::file(0, dir.join("y.mp3")));
        m.sync_lyrics();
        assert!(m.lyrics.is_none() && !m.lyrics_on, "a song without lyrics shows none");
        m.snap.track = Some(Track::live(7, "http://stream".into()));
        m.sync_lyrics();
        assert!(m.lyrics.is_none(), "a station has none");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
