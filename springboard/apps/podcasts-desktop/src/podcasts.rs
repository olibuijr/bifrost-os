//! Podcasts: subscribe to RSS feeds by URL, list episodes and play the MP3
//! enclosures. An episode is downloaded to the app folder, then played by the
//! audio service (`bifrost_app::audio`), so playback continues when the app
//! leaves the foreground or is closed. Nothing plays until the user taps an episode.

use bifrost_app::kit::ui::{SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XS, SPACE_XXL, SPACE_XXS, SPACE_XXXL};
use bifrost_app::kit::ui::{MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT, MAC_TITLE3_PT};
use bifrost_app::kit::ui::{CARD_BG, DESTRUCTIVE, CONTENT_BG, FIELD_BG, LABEL, LABEL_2, LINK, ROW_PRESSED, SELECTION, SEPARATOR_C, TRACK_C, WHITE};
use crate::chapters;
use crate::feed::{self, Show, Store};
use crate::info;
use crate::library::{self, Library};
use crate::opml;
use crate::prefs::Prefs;
use crate::refresh;
use crate::search;
use crate::transcript::{self, Cue};
use crate::media::{self, Bar, Shown};
use crate::transport;
use crate::wrap;
use bifrost_app::{AppMenu, AppMenuItem, Ctx};
use bifrost_app::dock_tile;
use bifrost_app::now_playing::{self, Command};
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
const SIDE_NEW: u32 = 2;
const SIDE_UP_NEXT: u32 = 3;
const SIDE_DOWNLOADED: u32 = 4;
const SIDE_SEARCH: u32 = 5;
const SIDE_SETTINGS: u32 = 6;
const SIDE_CHARTS: u32 = 7;
const SIDE_SHOW: u32 = 100;
const BAR_REFRESH: u32 = 7;
const BAR_SHOW_SETTINGS: u32 = 8;
const BAR_BACK: u32 = 9;
/// New Episodes lists at most this many.
const NEW_LIMIT: usize = 100;
/// The search field's band above the results, in points.
const SEARCH_BAR_PT: f32 = 48.0;
/// One row of a settings list and of the chapter list, in points.
const OPT_ROW_PT: f32 = 48.0;
const CHAPTER_ROW_PT: f32 = 30.0;
/// Transcripts kept in memory.
const TRANSCRIPTS_KEPT: usize = 4;
const TRANSCRIPT_LIMIT: usize = 4 << 20;
const SEARCH_LIMIT: usize = 1 << 20;
/// The largest chapters file read, and how many episodes are remembered as already asked for theirs.
const CHAPTERS_LIMIT: usize = 1 << 20;
const CHAPTERS_ASKED: usize = 256;
/// Two clicks on a row this close together play the episode, in milliseconds.
const DOUBLE_CLICK_MS: f64 = 450.0;
/// One line of notes or transcript text, and the top of the episode page's buttons, in points.
const LINE_PT: f32 = 20.0;
const DETAIL_BUTTONS_PT: f32 = 64.0;
/// Time interval for checking the playback service clock while connected; this is media polling, not window geometry.
const AUDIO_WAKE_INTERVAL_S: f64 = 1.0;
const BAR_TITLE: u32 = 1;
const BAR_ADD: u32 = 2;
const BAR_REMOVE: u32 = 3;
const BAR_CANCEL: u32 = 4;
const BAR_DONE: u32 = 5;
const BAR_CLOSE: u32 = 6;
/// Ids of the app's own menu items (`main_menu`); the shell sends them back to `menu_action`.
const M_ADD: u32 = 1;
const M_IMPORT: u32 = 2;
const M_EXPORT: u32 = 3;
const M_REFRESH: u32 = 10;
const M_VIEW_NEW: u32 = 11;
const M_VIEW_UP_NEXT: u32 = 12;
const M_VIEW_DOWNLOADED: u32 = 13;
const M_VIEW_SHOWS: u32 = 14;
const M_VIEW_SEARCH: u32 = 15;
const M_SETTINGS: u32 = 16;
const M_SHOW_SETTINGS: u32 = 17;
const M_PLAY_PAUSE: u32 = 20;
const M_STOP: u32 = 21;
const M_SKIP_BACK: u32 = 22;
const M_SKIP_FWD: u32 = 23;
const M_NEXT: u32 = 24;
const M_SLOWER: u32 = 25;
const M_FASTER: u32 = 26;
/// the speed menu's items, one per rate of `transport::RATES`
const M_RATE: u32 = 30;
/// the sleep timer menu's items, one per entry of `SLEEPS`
const M_SLEEP: u32 = 40;
const M_EP_PLAY: u32 = 50;
const M_EP_DOWNLOAD: u32 = 51;
const M_EP_REMOVE_DOWNLOAD: u32 = 52;
const M_EP_PLAY_NEXT: u32 = 53;
const M_EP_PLAY_LAST: u32 = 54;
const M_EP_UNQUEUE: u32 = 55;
const M_EP_PLAYED: u32 = 56;
const M_EP_DETAILS: u32 = 57;
const M_EP_UP: u32 = 58;
const M_EP_DOWN: u32 = 59;
const M_CLEAR_QUEUE: u32 = 60;
const M_SHOW_PLAYED: u32 = 61;
/// the New Episodes filters: M_NEW_FILTER + index into `library::NewFilter::ALL`
const M_NEW_FILTER: u32 = 62;
/// View > Top Charts
const M_VIEW_CHARTS: u32 = 90;
const POLL_MS: f64 = 250.0;
const FEED_LIMIT: usize = 8 << 20;
const EPISODE_LIMIT: usize = 160 << 20;
/// An episode arrives in ranges of this size, straight into its `.part` file.
const CHUNK: u64 = 2 << 20;
/// sleep timer choices in minutes; 0 is off
const SLEEPS: [u32; 5] = [0, 5, 15, 30, 60];
/// height of one row of the speed menu, in points (a menu row at body size)
const SPEED_ROW_PT: f32 = 26.0;
const SAVE_EVERY_MS: f64 = 5_000.0;
/// how long a started episode may take to show up as playing in the service
const START_TIMEOUT_MS: f64 = 5_000.0;
/// how long a press on an Up Next row must stay still before the row can be dragged to a new place
const HOLD_MS: f64 = 500.0;

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
    /// opens or closes the speed menu
    Speed,
    /// one rate of the speed menu, an index into `transport::RATES`
    Rate(usize),
    Sleep,
    Refresh,
    ShowSettings,
    Back,
    /// a row of a settings list (app settings or one show's)
    Opt(usize),
    /// a button of the episode detail: 0 play, 1 download, 2 up next, 3 played
    Detail(usize),
    /// the Chapters and Transcript buttons of the Now Playing sheet
    Chapters,
    Transcript,
    /// one row of the sheet's chapter list
    Chapter(usize),
    /// the sheet's transcript area (it scrolls)
    Panel,
    /// one line of the sheet's transcript, by the cue it belongs to; tapping it seeks to the cue
    Line(usize),
    /// the Follow button of a search result
    Follow(usize),
    /// the search field above the results
    SearchField,
}

/// What the file panels are for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileJob {
    Import,
    Export,
}

/// A command on the selected episode (the Episode menu, its context menu, the detail buttons).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EpAction {
    Play,
    Download,
    RemoveDownload,
    PlayNext,
    PlayLast,
    Unqueue,
    TogglePlayed,
    Details,
    MoveUp,
    MoveDown,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    Shows,
    Episodes(usize),
    /// unplayed episodes of every show, newest first
    New,
    UpNext,
    Downloaded,
    Search,
    /// the most followed shows of the open directory, to browse and follow
    TopCharts,
    Settings,
    ShowSettings(usize),
    /// an episode's notes and chapters: show index, episode index
    Detail(usize, usize),
}

/// What a row of the list stands for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Row {
    Show(usize),
    /// show index, episode index
    Ep(usize, usize),
    /// an entry of the search results
    Hit(usize),
}

/// What the Now Playing sheet shows in place of the artwork.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SheetPanel {
    None,
    Chapters,
    Transcript,
}

enum Msg {
    /// the show's title and episodes, parsed on the pool
    Feed(String, Result<(String, Vec<feed::Episode>), String>),
    /// bytes of the episode on disk so far and, when the server said, its size
    Progress(String, u64, Option<u64>),
    Episode(String, Result<PathBuf, String>),
    /// the answer of a show search
    Search(Result<Vec<search::Hit>, String>),
    /// the Top Charts list
    Charts(Result<Vec<search::Hit>, String>),
    /// the transcript cues of an episode (by URL)
    Transcript(String, Result<Vec<Cue>, String>),
    /// the chapters of an episode (by URL), read from the JSON file its feed links to
    Chapters(String, Result<Vec<feed::Chapter>, String>),
}

/// The wrapped lines of the open detail or transcript: what they are for, the width (bits), and each line with the
/// transcript cue it belongs to.
type Wrapped = (String, u32, Vec<(String, Option<usize>)>);

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
    /// the counts of the feed batch in flight, applied from the refresh event sequence
    refresh: refresh::Status,
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
    /// the playback rate, one of `transport::RATES`; sent to the audio service (`set_rate`) and kept for the session
    rate: f64,
    /// which episodes the New Episodes list keeps (a smart filter from the View menu)
    new_filter: library::NewFilter,
    /// the audio service's connection count when the rate was last sent, so a restarted service gets it again
    rate_sent_at: u32,
    /// the speed menu is open above the Speed button
    speed_menu: bool,
    sleep: usize,
    sleep_at: Option<f64>,
    resume: Vec<(String, u64)>,
    last_save: f64,
    error: String,
    dirty: bool,
    cache: TextCache,
    cache_style: (u32, u32, bool),
    /// the Now Playing session last sent, and the Dock bar last sent (`sync_services` sends only what changed)
    published: Option<Shown>,
    dock: Bar,
    /// service calls waiting for their reply (replies are only consumed, never retried)
    calls: Vec<u32>,
    /// an episode finished downloading: the Dock tile bounces once, if the app is not in front
    bounce_ready: bool,
    /// the app settings and the options of each show
    prefs: Prefs,
    /// played flags and the Up Next queue
    library: Library,
    /// the rows of the current view, rebuilt whenever what they list changes
    rows: Vec<Row>,
    /// the selected row of `rows`
    selected: Option<usize>,
    /// the view a panel (settings, detail) returns to
    back: View,
    /// episodes waiting to download after the current one
    dl_queue: Vec<String>,
    /// the episode that plays as soon as its download is done
    play_after: Option<String>,
    /// when feeds were last refreshed; None before the first tick
    last_refresh: Option<f64>,
    /// the show search: its field, the last answer and whether a request is out
    search_field: Option<TextField>,
    results: Vec<search::Hit>,
    searching: bool,
    /// the Top Charts as last fetched, and whether a request is out
    charts: Vec<search::Hit>,
    charts_loading: bool,
    /// a line of news in grey (imports, exports), cleared with the next view change
    notice: String,
    sheet_panel: SheetPanel,
    transcripts: Vec<(String, Vec<Cue>)>,
    transcript_loading: Option<String>,
    /// chapters read from linked JSON files (episode URL, chapters), and the episodes whose file was already asked for
    json_chapters: Vec<(String, Vec<feed::Chapter>)>,
    chapters_asked: Vec<String>,
    /// wrapped lines of the open detail or transcript, and the width they were wrapped for (bits), cached
    wrapped: Option<Wrapped>,
    /// pixels from the list top to the end of the scrolling content of a panel
    body_h: f32,
    /// the open request for the subscriptions file to read, and the save in progress (request, bytes)
    import: Option<u32>,
    export: Option<(u32, Vec<u8>)>,
    /// every show's cached episodes were read from disk (New Episodes, Up Next and Downloaded need them all)
    cached_all: bool,
    /// the sheet's panel scrolls apart from the list, and its content height in pixels
    panel_scroll: Scroller,
    panel_h: f32,
    /// a file panel the app loop has to open (it needs the context)
    want_file: Option<FileJob>,
    /// the episode whose page is open (its place in the lists changes when a feed updates)
    detail_url: String,
    /// the selected row, by what it stands for (its place changes when the rows are rebuilt)
    selected_key: Option<String>,
    /// the row last clicked and when, to tell a second click
    last_click: (usize, f64),
    /// the transcript cue the panel last scrolled to
    followed: Option<usize>,
    /// an Up Next row pressed and still held: its episode, when the press began, and where it was pressed
    hold: Option<(String, f64, f32)>,
    /// the Up Next episode being dragged to a new place
    reordering: Option<String>,
}

impl Podcasts {
    pub fn new(ui: &Ui, w: i32, h: i32, dir: PathBuf, net: Arc<dyn Fetch>, shell_dir: &Path) -> Self {
        let store = Store::new(dir);
        let shows = store.load();
        let resume = store.positions();
        let prefs = Prefs::load(&store.dir);
        let library = Library::load(&store.dir);
        let inbox: Inbox = Arc::new(Mutex::new(VecDeque::new()));
        Self {
            w,
            h,
            store,
            net,
            inbox,
            refresh: refresh::Status::new(),
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
            audio: audio::Client::for_app(shell_dir),
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
            rate_sent_at: 0,
            new_filter: library::NewFilter::default(),
            speed_menu: false,
            sleep: 0,
            sleep_at: None,
            resume,
            last_save: 0.0,
            error: String::new(),
            dirty: true,
            cache: TextCache::default(),
            cache_style: (ui.s.to_bits(), ui.text_scale.to_bits(), ui.bold),
            published: None,
            dock: Bar::Off,
            calls: Vec::new(),
            bounce_ready: false,
            prefs,
            library,
            rows: Vec::new(),
            selected: None,
            back: View::Shows,
            dl_queue: Vec::new(),
            play_after: None,
            last_refresh: None,
            search_field: None,
            results: Vec::new(),
            searching: false,
            charts: Vec::new(),
            charts_loading: false,
            notice: String::new(),
            sheet_panel: SheetPanel::None,
            transcripts: Vec::new(),
            transcript_loading: None,
            json_chapters: Vec::new(),
            chapters_asked: Vec::new(),
            wrapped: None,
            body_h: 0.0,
            import: None,
            export: None,
            cached_all: false,
            panel_scroll: Scroller::new(),
            panel_h: 0.0,
            want_file: None,
            detail_url: String::new(),
            selected_key: None,
            last_click: (usize::MAX, 0.0),
            followed: None,
            hold: None,
            reordering: None,
        }
        .with_rows()
    }

    fn with_rows(mut self) -> Self {
        self.rebuild();
        self
    }

    /// The app left the foreground: the episode keeps playing in the service; remember where it is.
    pub fn suspend(&mut self) {
        if let Some(url) = self.playing.clone() {
            self.set_resume(&url, Some(self.position_ms as u64).filter(|&ms| ms >= 5_000));
        }
        self.audio.pump();
    }

    // system services ---------------------------------------------------

    /// Keeps the Control Center tile and the Dock bar in step with the app. Each service is called only when what it
    /// shows changes (never per frame); a finished download asks the Dock tile to bounce once.
    pub fn sync_services(&mut self, ctx: &Ctx) {
        let found = self.playing.as_deref().and_then(|url| {
            self.shows.iter().find_map(|s| s.episodes.iter().find(|e| e.url == url).map(|e| (url, e.title.as_str(), s.title.as_str())))
        });
        let want = media::shown(found, self.started, self.paused, self.duration_ms);
        if want != self.published {
            match &want {
                Some(shown) => {
                    if let Some(id) = media::publish(ctx, shown, self.position_ms) {
                        self.note_call(id);
                    }
                }
                None => {
                    if self.published.is_some() {
                        let id = now_playing::clear(ctx);
                        self.note_call(id);
                    }
                }
            }
            self.published = want;
        }
        let bar = media::dock_bar(self.downloading.is_some(), self.progress);
        if bar != self.dock {
            let id = media::send_bar(ctx, bar);
            self.note_call(id);
            self.dock = bar;
        }
        if std::mem::take(&mut self.bounce_ready) {
            let id = dock_tile::bounce(ctx, false);
            self.note_call(id);
        }
    }

    /// A transport command from the Control Center tile. Only an episode the audio service has confirmed is acted on;
    /// the tile is published again by `sync_services` when the playback state changes.
    pub fn remote(&mut self, command: Command) {
        if self.playing.is_none() || !self.started {
            return;
        }
        match command {
            Command::Play => {
                if self.paused {
                    self.audio.play();
                    self.paused = false;
                }
            }
            Command::Pause => {
                if !self.paused {
                    self.audio.pause();
                    self.paused = true;
                }
            }
            Command::Toggle => {
                self.audio.toggle();
                self.paused = !self.paused;
            }
            Command::Stop => self.stop_playback(),
            Command::SkipForward => self.skip(false, self.prefs.settings.forward_s as f64),
            Command::SkipBackward => self.skip(true, self.prefs.settings.back_s as f64),
            Command::Seek(ms) => self.seek_to(ms),
            Command::Next => self.next_episode(),
            Command::Previous => {}
        }
        self.dirty = true;
    }

    /// Consumes the reply to one of this app's service calls. False for any other request.
    pub fn service_reply(&mut self, request: u32) -> bool {
        match self.calls.iter().position(|&id| id == request) {
            Some(i) => {
                self.calls.remove(i);
                true
            }
            None => false,
        }
    }

    fn note_call(&mut self, id: u32) {
        self.calls.push(id);
        if self.calls.len() > 32 {
            self.calls.remove(0);
        }
    }

    // network -----------------------------------------------------------

    fn fetch_feed(&mut self, url: &str) {
        if self.loading.iter().any(|u| u == url) {
            return;
        }
        self.loading.push(url.to_string());
        self.refresh.started();
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

    /// Fetches every followed show's feed again (View > Refresh, and the refresh period).
    fn refresh_all(&mut self) {
        self.last_refresh = Some(self.now);
        let urls: Vec<String> = self.shows.iter().map(|s| s.url.clone()).collect();
        for url in urls {
            self.fetch_feed(&url);
        }
    }

    /// Downloads `url` (queued behind a download in progress); with `play` the episode plays as soon as it is on
    /// disk, and jumps the queue.
    fn download(&mut self, url: &str, play: bool) {
        let path = self.store.episode_file(url);
        if path.is_file() {
            if play {
                self.play_file(url, path);
            }
            return;
        }
        if play {
            self.play_after = Some(url.to_string());
        }
        if self.downloading.as_deref() == Some(url) {
            return;
        }
        if self.downloading.is_some() {
            self.dl_queue.retain(|u| u != url);
            if play {
                self.dl_queue.insert(0, url.to_string());
            } else {
                self.dl_queue.push(url.to_string());
            }
            return;
        }
        self.start_download(url);
    }

    /// Starts the next queued download when none is running.
    fn next_download(&mut self) {
        while self.downloading.is_none() && !self.dl_queue.is_empty() {
            let url = self.dl_queue.remove(0);
            if !self.store.downloaded(&url) {
                self.start_download(&url);
            }
        }
    }

    fn start_download(&mut self, url: &str) {
        let path = self.store.episode_file(url);
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

    /// Deletes an episode's download (stopping it first when it plays).
    fn remove_download(&mut self, url: &str) {
        if self.playing.as_deref() == Some(url) {
            self.stop_playback();
        }
        self.dl_queue.retain(|u| u != url);
        self.store.forget_episodes([url]);
        self.rebuild();
    }

    /// After a feed arrived, or a show's options changed: the downloads beyond the number to keep are deleted, and a
    /// show with auto-download fetches its newest unplayed episodes.
    fn after_refresh(&mut self, url: &str) {
        let options = self.prefs.show(url);
        let Some(show) = self.shows.iter().find(|s| s.url == url) else { return };
        let store = &self.store;
        let drop: Vec<String> = library::prune(&show.episodes, options.keep, |u| store.downloaded(u), self.playing.as_deref()).into_iter().map(str::to_string).collect();
        let want: Vec<String> = if options.auto_download {
            library::auto_downloads(&show.episodes, &self.library, options.keep, |u| store.downloaded(u)).into_iter().map(str::to_string).collect()
        } else {
            Vec::new()
        };
        self.store.forget_episodes(drop.iter().map(String::as_str));
        for episode in want {
            self.download(&episode, false);
        }
    }

    fn next_msg(&self) -> Option<Msg> {
        self.inbox.lock().ok()?.pop_front()
    }

    fn pump(&mut self) {
        let mut any = false;
        while let Some(msg) = self.next_msg() {
            // download progress changes no list, so it does not rebuild the rows
            any |= !matches!(msg, Msg::Progress(..));
            self.dirty = true;
            match msg {
                Msg::Feed(url, r) => {
                    self.loading.retain(|u| *u != url);
                    self.refresh.finished(r.is_ok());
                    match r {
                        Ok((title, episodes)) => {
                            if let Some(show) = self.shows.iter_mut().find(|s| s.url == url) {
                                if !title.is_empty() {
                                    show.title = title;
                                }
                                show.episodes = episodes;
                                self.store.save(&self.shows);
                                self.apply_chapters();
                                self.after_refresh(&url);
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
                    let wanted = self.play_after.as_deref() == Some(url.as_str());
                    if wanted {
                        self.play_after = None;
                    }
                    match r {
                        Ok(path) => {
                            self.bounce_ready = true;
                            if wanted {
                                self.play_file(&url, path);
                            }
                        }
                        Err(e) => self.error = format!("Cannot download: {e}"),
                    }
                    self.next_download();
                }
                Msg::Search(r) => {
                    self.searching = false;
                    match r {
                        Ok(hits) => self.results = hits,
                        Err(e) => self.error = format!("Cannot search: {e}"),
                    }
                }
                Msg::Charts(r) => {
                    self.charts_loading = false;
                    match r {
                        Ok(hits) => self.charts = hits,
                        Err(e) => self.error = format!("Cannot load the Top Charts: {e}"),
                    }
                }
                Msg::Transcript(url, r) => {
                    if self.transcript_loading.as_deref() == Some(url.as_str()) {
                        self.transcript_loading = None;
                    }
                    match r {
                        Ok(cues) => {
                            self.transcripts.retain(|(u, _)| *u != url);
                            self.transcripts.push((url, cues));
                            if self.transcripts.len() > TRANSCRIPTS_KEPT {
                                self.transcripts.remove(0);
                            }
                        }
                        Err(e) => self.error = format!("Cannot load the transcript: {e}"),
                    }
                    self.wrapped = None;
                }
                Msg::Chapters(url, r) => {
                    // a missing or unreadable chapters file is not worth an error line: the episode just has none
                    if let Some(list) = r.ok().filter(|l| !l.is_empty()) {
                        self.json_chapters.retain(|(u, _)| *u != url);
                        self.json_chapters.push((url, list));
                        if self.json_chapters.len() > chapters::KEPT {
                            self.json_chapters.remove(0);
                        }
                        self.apply_chapters();
                        self.wrapped = None;
                    }
                }
            }
        }
        if any {
            self.rebuild();
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
        if self.library.in_queue(url) {
            self.library.dequeue(url);
            self.library.save(&self.store.dir);
        }
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
        let was_sheet = self.sheet;
        if stop && self.playing.is_some() {
            self.audio.stop();
            self.released = self.playing_path.clone();
        }
        self.playing_path = None;
        self.started = false;
        if let Some(url) = self.playing.take() {
            let ms = if finished || self.position_ms < 5_000.0 { None } else { Some(self.position_ms as u64) };
            self.set_resume(&url, ms);
            if finished {
                self.library.set_played(&url, true);
                self.library.dequeue(&url);
                self.library.save(&self.store.dir);
            }
        }
        self.position_ms = 0.0;
        self.sheet = false;
        self.sheet_panel = SheetPanel::None;
        self.scrubbing = None;
        self.sleep_at = None;
        self.paused = false;
        if finished {
            self.advance(was_sheet);
        } else {
            self.rebuild();
        }
    }

    /// Continues with the next episode of Up Next, keeping the Now Playing sheet open when it was.
    fn advance(&mut self, keep_sheet: bool) {
        if let Some(next) = self.library.take_next() {
            self.library.save(&self.store.dir);
            self.error.clear();
            self.download(&next, true);
            self.sheet = keep_sheet;
        }
        self.rebuild();
    }

    /// Skips to the next episode of Up Next (Controls > Next Episode).
    fn next_episode(&mut self) {
        if self.library.queue().is_empty() {
            return;
        }
        let was_sheet = self.sheet;
        self.stop_playback();
        self.advance(was_sheet);
    }

    /// Sets the playback rate and sends it to the audio service, which time-stretches the audio (the pitch stays).
    fn set_rate(&mut self, rate: f64) {
        self.rate = rate;
        self.audio.set_rate(rate as f32);
    }

    /// Follows the service: the episode's position and state, another app taking over, and an
    /// episode that is already playing when the app starts.
    fn sync_audio(&mut self, t: f64) {
        self.dirty |= self.audio.pump();
        self.dirty |= self.audio.tick(t);
        if self.audio.connects() != self.rate_sent_at {
            self.rate_sent_at = self.audio.connects();
            self.audio.set_rate(self.rate as f32);
        }
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
        ui.pt(self.side_pt(ui))
    }

    /// Keep the stock sidebar width on desktop, but let content take over on compact windows.
    fn side_pt(&self, ui: &Ui) -> f32 {
        SIDEBAR_W.min((self.w as f32 / ui.pt(1.0)) * 0.4).max(0.0)
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

    /// 0 skip back, 1 play/pause, 2 skip forward; then speed, sleep, chapters and transcript (3 to 6) on a second row
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
            let w4 = (cw - 3.0 * gap) / 4.0;
            (x0 + (w4 + gap) * (n - 3) as f32, sc.1 + sc.3 + ui.pt(SPACE_XXL) + h + gap, w4, h)
        }
    }

    /// The speed menu's card: one row per rate in `transport::RATES`, opening above the Speed button and as wide as it.
    fn speed_menu_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let b = self.sheet_button(ui, 3);
        let h = ui.pt(SPACE_XXS) * 2.0 + ui.pt(SPEED_ROW_PT) * transport::RATES.len() as f32;
        (b.0, b.1 - ui.pt(SPACE_S) - h, b.2, h)
    }

    /// One row of the speed menu.
    fn speed_item(&self, ui: &Ui, i: usize) -> (f32, f32, f32, f32) {
        let (x, y, w, _) = self.speed_menu_rect(ui);
        (x, y + ui.pt(SPACE_XXS) + ui.pt(SPEED_ROW_PT) * i as f32, w, ui.pt(SPEED_ROW_PT))
    }

    /// Draws the speed menu over the sheet's controls: the current rate's row is selected, and each row names its rate.
    fn draw_speed_menu(&mut self, s: &mut Surface, ui: &Ui) {
        let (x, y, w, h) = self.speed_menu_rect(ui);
        s.fill_rrect(x, y, w, h, ui.pt(SPACE_M), CARD_BG.now());
        let current = transport::rate_index(self.rate);
        for (i, rate) in transport::RATES.iter().enumerate() {
            let r = self.speed_item(ui, i);
            if current == Some(i) {
                s.fill_rrect(r.0 + ui.pt(SPACE_XXS), r.1, r.2 - ui.pt(SPACE_XXS) * 2.0, r.3, ui.pt(m::SELECTION_RADIUS), SELECTION.now());
            }
            self.centred(s, ui, &format!("{rate}x"), MAC_BODY_PT, LABEL.now(), r);
        }
    }

    // navigation --------------------------------------------------------

    /// Shows `view` from its top with nothing selected.
    fn go(&mut self, view: View) {
        self.view = view;
        self.confirm_remove = false;
        self.sheet = false;
        self.speed_menu = false;
        self.field = None;
        self.scroll = Scroller::new();
        self.selected = None;
        self.selected_key = None;
        self.error.clear();
        self.notice.clear();
        self.rebuild();
        self.dirty = true;
    }

    /// Shows a panel (settings, episode detail) that Back leaves for the view it came from.
    fn open_panel(&mut self, view: View) {
        if !matches!(self.view, View::Settings | View::ShowSettings(_) | View::Detail(..)) {
            self.back = self.view;
        }
        self.go(view);
    }

    fn open_show(&mut self, i: usize) {
        self.go(View::Episodes(i));
        let url = self.shows[i].url.clone();
        if self.shows[i].episodes.is_empty() {
            if let Some((_, eps)) = self.store.cached(&url) {
                self.shows[i].episodes = eps;
                self.rebuild();
            }
        }
        self.fetch_feed(&url);
    }

    fn open_search(&mut self, ui: &Ui) {
        self.go(View::Search);
        if self.search_field.is_none() {
            self.search_field = Some(TextField::new(ui, "Search shows", MAC_BODY_PT, false, 128));
        }
    }

    /// Shows the Top Charts, fetching them the first time (and again after a failure).
    fn open_charts(&mut self) {
        self.go(View::TopCharts);
        if self.charts.is_empty() && !self.charts_loading {
            self.charts_loading = true;
            self.error.clear();
            self.rebuild();
            let (net, inbox, url) = (self.net.clone(), self.inbox.clone(), search::charts_url());
            DispatchQueue::global(Qos::Utility).async_(move || {
                let r = net.get(&url, SEARCH_LIMIT).map_err(|e| e.to_string()).map(|bytes| search::parse_gpodder(&String::from_utf8_lossy(&bytes)));
                deliver(&inbox, Msg::Charts(r));
            });
        }
    }

    /// The directory hits the current view lists: the Top Charts, or the search results.
    fn hits(&self) -> &[search::Hit] {
        if self.view == View::TopCharts { &self.charts } else { &self.results }
    }

    /// The sidebar row `id` was chosen.
    fn navigate(&mut self, ui: &Ui, id: u32) {
        match id {
            SIDE_SHOWS => self.go(View::Shows),
            SIDE_NEW => self.go(View::New),
            SIDE_UP_NEXT => self.go(View::UpNext),
            SIDE_DOWNLOADED => self.go(View::Downloaded),
            SIDE_SEARCH => self.open_search(ui),
            SIDE_CHARTS => self.open_charts(),
            SIDE_SETTINGS => self.open_panel(View::Settings),
            _ => {
                if let Some(i) = id.checked_sub(SIDE_SHOW).map(|i| i as usize).filter(|&i| i < self.shows.len()) {
                    self.open_show(i);
                }
            }
        }
    }

    /// The sidebar row that stands for the current view.
    fn side_id(&self) -> u32 {
        match self.view {
            View::Shows => SIDE_SHOWS,
            View::Episodes(i) | View::ShowSettings(i) => SIDE_SHOW + i as u32,
            View::New => SIDE_NEW,
            View::UpNext => SIDE_UP_NEXT,
            View::Downloaded => SIDE_DOWNLOADED,
            View::Search => SIDE_SEARCH,
            View::TopCharts => SIDE_CHARTS,
            View::Settings => SIDE_SETTINGS,
            View::Detail(s, _) => SIDE_SHOW + s as u32,
        }
    }

    // subscriptions -----------------------------------------------------

    /// Follows the show at `url` (once) and returns its place in the list.
    fn follow(&mut self, input: &str, title: Option<&str>) -> Option<usize> {
        let url = feed::normalize(input)?;
        let i = match self.shows.iter().position(|s| s.url == url) {
            Some(i) => i,
            None => {
                let title = title.filter(|t| !t.is_empty()).map_or_else(|| url.trim_start_matches("https://").trim_start_matches("http://").to_string(), str::to_string);
                self.shows.push(Show { url, title, episodes: Vec::new() });
                self.store.save(&self.shows);
                self.shows.len() - 1
            }
        };
        Some(i)
    }

    fn subscribe(&mut self, input: &str) {
        match self.follow(input, None) {
            Some(i) => self.open_show(i),
            None => self.error = "Enter a feed URL".into(),
        }
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
        self.library.forget(&urls);
        self.library.save(&self.store.dir);
        self.dl_queue.retain(|u| !urls.contains(&u.as_str()));
        self.prefs.forget_show(&show.url);
        self.prefs.save(&self.store.dir);
        self.store.save(&self.shows);
        self.go(View::Shows);
    }

    // lists -------------------------------------------------------------

    /// Reads every show's cached episodes (New Episodes, Up Next and Downloaded look across shows).
    fn ensure_all(&mut self) {
        if self.cached_all {
            return;
        }
        for show in &mut self.shows {
            if show.episodes.is_empty() {
                if let Some((_, eps)) = self.store.cached(&show.url) {
                    show.episodes = eps;
                }
            }
        }
        self.cached_all = true;
    }

    fn find_episode(&self, url: &str) -> Option<(usize, usize)> {
        self.shows.iter().enumerate().find_map(|(si, s)| s.episodes.iter().position(|e| e.url == url).map(|ei| (si, ei)))
    }

    fn episode_url(&self, (s, e): (usize, usize)) -> Option<String> {
        self.shows.get(s)?.episodes.get(e).map(|e| e.url.clone())
    }

    /// What identifies a row across rebuilds: the show, episode or feed address.
    fn row_key(&self, row: &Row) -> String {
        let key = match *row {
            Row::Show(i) => self.shows.get(i).map(|s| s.url.clone()),
            Row::Ep(s, e) => self.episode_url((s, e)),
            Row::Hit(i) => self.hits().get(i).map(|h| h.feed.clone()),
        };
        key.unwrap_or_default()
    }

    /// Rebuilds the rows of the current view from what it lists.
    fn rebuild(&mut self) {
        if matches!(self.view, View::New | View::UpNext | View::Downloaded) {
            self.ensure_all();
        }
        if let View::Detail(..) = self.view {
            match self.find_episode(&self.detail_url) {
                Some((s, e)) => self.view = View::Detail(s, e),
                None => self.view = self.back,
            }
        }
        let rows: Vec<Row> = match self.view {
            View::Shows => (0..self.shows.len()).map(Row::Show).collect(),
            View::Episodes(si) => self.shows.get(si).map_or_else(Vec::new, |s| {
                let mut order = library::newest_first(&s.episodes);
                if self.prefs.show(&s.url).oldest_first {
                    order.reverse();
                }
                order.into_iter().map(|ei| Row::Ep(si, ei)).collect()
            }),
            View::New => library::latest_filtered(&self.shows, &self.library, self.new_filter, |u| self.store.downloaded(u), NEW_LIMIT).into_iter().map(|(s, e)| Row::Ep(s, e)).collect(),
            View::UpNext => self.library.queue().iter().filter_map(|u| self.find_episode(u)).map(|(s, e)| Row::Ep(s, e)).collect(),
            View::Downloaded => {
                let mut all: Vec<(i64, usize, usize)> = Vec::new();
                for (si, show) in self.shows.iter().enumerate() {
                    for (ei, e) in show.episodes.iter().enumerate() {
                        if self.store.downloaded(&e.url) {
                            all.push((e.ts, si, ei));
                        }
                    }
                }
                all.sort_by_key(|&(ts, si, ei)| (std::cmp::Reverse(ts), si, ei));
                all.into_iter().map(|(_, s, e)| Row::Ep(s, e)).collect()
            }
            View::Search => (0..self.results.len()).map(Row::Hit).collect(),
            View::TopCharts => (0..self.charts.len()).map(Row::Hit).collect(),
            View::Settings | View::ShowSettings(_) | View::Detail(..) => Vec::new(),
        };
        self.rows = rows;
        self.selected = self.selected_key.clone().and_then(|k| self.rows.iter().position(|r| self.row_key(r) == k));
        if self.selected.is_none() {
            self.selected_key = None;
        }
        self.wrapped = None;
        self.dirty = true;
    }

    fn select(&mut self, i: usize) {
        self.selected = Some(i);
        self.selected_key = self.rows.get(i).map(|r| self.row_key(r));
    }

    /// Moves the selection by `delta` rows and scrolls it into view.
    fn move_selection(&mut self, ui: &Ui, delta: i32) {
        let n = self.count();
        if n == 0 || !matches!(self.view, View::Shows | View::Episodes(_) | View::New | View::UpNext | View::Downloaded | View::Search | View::TopCharts) {
            return;
        }
        let next = match self.selected {
            Some(i) => (i as i32 + delta).clamp(0, n as i32 - 1) as usize,
            None => 0,
        };
        self.select(next);
        let r = self.row_rect(ui, next);
        let (top, bottom) = (self.row_top(ui), self.controls_top(ui));
        if r.1 < top {
            self.scroll.jump_to(self.scroll.offset() - (top - r.1));
        } else if r.1 + r.3 > bottom {
            self.scroll.jump_to(self.scroll.offset() + (r.1 + r.3 - bottom));
        }
    }

    /// The episode a command acts on: the one open in the detail, or the selected row.
    fn selected_episode(&self) -> Option<(usize, usize)> {
        if let View::Detail(s, e) = self.view {
            return Some((s, e));
        }
        match self.rows.get(self.selected?)? {
            Row::Ep(s, e) => Some((*s, *e)),
            _ => None,
        }
    }

    fn save_library(&self) {
        self.library.save(&self.store.dir);
    }

    fn episode_action(&mut self, action: EpAction) {
        let Some(at) = self.selected_episode() else { return };
        let Some(url) = self.episode_url(at) else { return };
        match action {
            EpAction::Play => {
                self.error.clear();
                self.download(&url, true);
            }
            EpAction::Download => {
                if !self.store.downloaded(&url) {
                    self.error.clear();
                    self.download(&url, false);
                }
            }
            EpAction::RemoveDownload => self.remove_download(&url),
            EpAction::PlayNext => {
                self.library.play_next(&url);
                self.save_library();
            }
            EpAction::PlayLast => {
                self.library.play_last(&url);
                self.save_library();
            }
            EpAction::Unqueue => {
                self.library.dequeue(&url);
                self.save_library();
            }
            EpAction::TogglePlayed => {
                let played = !self.library.is_played(&url);
                self.library.set_played(&url, played);
                self.save_library();
            }
            EpAction::Details => {
                self.detail_url = url;
                self.open_panel(View::Detail(at.0, at.1));
            }
            EpAction::MoveUp | EpAction::MoveDown => {
                self.library.move_in_queue(&url, action == EpAction::MoveUp);
                self.save_library();
            }
        }
        self.rebuild();
        self.dirty = true;
    }

    /// Marks every episode of the show on screen played, or unplayed when they are all played already.
    fn mark_show_played(&mut self) {
        let View::Episodes(si) = self.view else { return };
        let Some(show) = self.shows.get(si) else { return };
        let urls: Vec<&str> = show.episodes.iter().map(|e| e.url.as_str()).collect();
        let played = !urls.iter().all(|u| self.library.is_played(u));
        self.library.set_all_played(urls, played);
        self.save_library();
        self.rebuild();
        self.dirty = true;
    }

    // search, transcripts, files ---------------------------------------

    fn run_search(&mut self) {
        let term = self.search_field.as_ref().map(|f| f.text().to_string()).unwrap_or_default();
        let asks = search::requests(&term);
        if asks.is_empty() {
            return;
        }
        self.searching = true;
        self.results.clear();
        self.error.clear();
        self.rebuild();
        let (net, inbox) = (self.net.clone(), self.inbox.clone());
        DispatchQueue::global(Qos::Utility).async_(move || {
            // both directories; one that fails leaves the other's answer, and only both failing is an error
            let mut answers = Vec::new();
            let mut failed = None;
            for (source, url) in asks {
                match net.get(&url, SEARCH_LIMIT) {
                    Ok(bytes) => answers.push(search::parse_from(source, &String::from_utf8_lossy(&bytes))),
                    Err(e) => failed = Some(e.to_string()),
                }
            }
            let r = match failed {
                Some(e) if answers.is_empty() => Err(e),
                _ => Ok(search::merge(&term, answers)),
            };
            deliver(&inbox, Msg::Search(r));
        });
    }

    fn follow_hit(&mut self, i: usize) {
        let Some(hit) = self.hits().get(i).cloned() else { return };
        if let Some(at) = self.follow(&hit.feed, Some(&hit.title)) {
            let url = self.shows[at].url.clone();
            self.fetch_feed(&url);
            self.notice = format!("Following {}", hit.title);
        }
    }

    /// The transcript of the playing episode: its address.
    fn playing_episode(&self) -> Option<&feed::Episode> {
        let url = self.playing.as_deref()?;
        self.shows.iter().flat_map(|s| s.episodes.iter()).find(|e| e.url == url)
    }

    fn fetch_transcript(&mut self, url: &str) {
        if self.transcripts.iter().any(|(u, _)| u == url) || self.transcript_loading.as_deref() == Some(url) {
            return;
        }
        self.transcript_loading = Some(url.to_string());
        let (net, inbox, url) = (self.net.clone(), self.inbox.clone(), url.to_string());
        DispatchQueue::global(Qos::Utility).async_(move || {
            let r = net.get(&url, TRANSCRIPT_LIMIT).map_err(|e| e.to_string()).map(|bytes| transcript::parse(&String::from_utf8_lossy(&bytes)));
            deliver(&inbox, Msg::Transcript(url, r));
        });
    }

    /// Fetches the chapters file of the playing episode and of the open episode page when the feed links to one
    /// instead of listing the chapters itself.
    fn want_chapters(&mut self) {
        let mut want: Vec<(String, String)> = Vec::new();
        let open = self.detail_episode().map(|(_, e)| e);
        for ep in self.playing_episode().into_iter().chain(open) {
            if chapters::wanted(ep, &self.chapters_asked) && !want.iter().any(|(u, _)| *u == ep.url) {
                want.push((ep.url.clone(), ep.chapters_url.clone()));
            }
        }
        for (episode, link) in want {
            self.chapters_asked.push(episode.clone());
            if self.chapters_asked.len() > CHAPTERS_ASKED {
                self.chapters_asked.remove(0);
            }
            let (net, inbox) = (self.net.clone(), self.inbox.clone());
            DispatchQueue::global(Qos::Utility).async_(move || {
                let r = net.get(&link, CHAPTERS_LIMIT).map_err(|e| e.to_string()).map(|bytes| chapters::parse(&String::from_utf8_lossy(&bytes)));
                deliver(&inbox, Msg::Chapters(episode, r));
            });
        }
    }

    /// Gives the episodes the chapters read from their linked files (a feed refresh replaces the episodes).
    fn apply_chapters(&mut self) {
        for (url, list) in &self.json_chapters {
            for show in &mut self.shows {
                for ep in show.episodes.iter_mut().filter(|e| e.url == *url && e.chapters.is_empty()) {
                    ep.chapters = list.clone();
                }
            }
        }
    }

    /// Opens or closes a panel of the Now Playing sheet.
    fn toggle_panel(&mut self, panel: SheetPanel) {
        self.sheet_panel = if self.sheet_panel == panel { SheetPanel::None } else { panel };
        self.panel_scroll = Scroller::new();
        self.followed = None;
        self.wrapped = None;
        if self.sheet_panel == SheetPanel::Transcript {
            if let Some(url) = self.playing_episode().map(|e| e.transcript.clone()).filter(|u| !u.is_empty()) {
                self.fetch_transcript(&url);
            }
        }
    }

    /// The followed shows as an OPML document.
    fn opml_text(&self) -> String {
        let feeds: Vec<(String, String)> = self.shows.iter().map(|s| (s.url.clone(), s.title.clone())).collect();
        opml::write(&feeds)
    }

    /// The app loop has a file panel to open (it needs the runtime context).
    pub fn take_file_job(&mut self) -> Option<FileJob> {
        self.want_file.take()
    }

    pub fn begin_import(&mut self, request: u32) {
        self.import = Some(request);
    }

    pub fn begin_export(&mut self, request: u32) {
        self.export = Some((request, self.opml_text().into_bytes()));
    }

    pub fn is_import(&self, request: u32) -> bool {
        self.import == Some(request)
    }

    /// The bytes waiting for the save panel's file `request`.
    pub fn take_export(&mut self, request: u32) -> Option<Vec<u8>> {
        match self.export.take() {
            Some((r, bytes)) if r == request => Some(bytes),
            other => {
                self.export = other;
                None
            }
        }
    }

    /// A file panel ended without a file (cancelled or refused).
    pub fn file_failed(&mut self, request: u32) {
        if self.import == Some(request) {
            self.import = None;
        }
        if self.export.as_ref().is_some_and(|(r, _)| *r == request) {
            self.export = None;
        }
    }

    /// The export was written.
    pub fn exported(&mut self) {
        self.notice = format!("Exported {} subscriptions", self.shows.len());
        self.dirty = true;
    }

    /// The subscriptions file arrived: follows the shows it lists that are new.
    pub fn imported(&mut self, bytes: &[u8]) {
        self.import = None;
        let known: Vec<&str> = self.shows.iter().map(|s| s.url.as_str()).collect();
        let fresh = opml::new_feeds(&opml::parse(&String::from_utf8_lossy(bytes)), &known);
        for (url, title) in &fresh {
            self.follow(url, Some(title));
            self.fetch_feed(url);
        }
        self.notice = match fresh.len() {
            0 => "No new subscriptions in the file".to_string(),
            1 => "Imported 1 subscription".to_string(),
            n => format!("Imported {n} subscriptions"),
        };
        self.rebuild();
        self.dirty = true;
    }

    // menus -------------------------------------------------------------

    /// The app's own menus: File, View, Controls and Episode, with their key equivalents.
    pub fn main_menu(&self) -> Vec<AppMenu> {
        let url = self.selected_episode().and_then(|at| self.episode_url(at));
        let has_ep = url.is_some();
        let (downloaded, played, queued) = url.as_deref().map_or((false, false, false), |u| (self.store.downloaded(u), self.library.is_played(u), self.library.in_queue(u)));
        let playing = self.playing.is_some();
        let has_queue = !self.library.queue().is_empty();
        let show = match self.view {
            View::Episodes(si) => self.shows.get(si),
            _ => None,
        };
        let show_has_eps = show.is_some_and(|s| !s.episodes.is_empty());
        let show_all_played = show.is_some_and(|s| s.episodes.iter().all(|e| self.library.is_played(&e.url)));
        let st = self.prefs.settings;
        let file = AppMenu {
            title: "File".into(),
            items: vec![
                AppMenuItem::new(M_ADD, "Add Podcast...").key("cmd+n"),
                AppMenuItem::separator(),
                AppMenuItem::new(M_IMPORT, "Import Subscriptions...").key("cmd+shift+i"),
                AppMenuItem::new(M_EXPORT, "Export Subscriptions...").key("cmd+shift+e").enabled(!self.shows.is_empty()),
            ],
        };
        let view = AppMenu {
            title: "View".into(),
            items: vec![
                AppMenuItem::new(M_REFRESH, "Refresh").key("cmd+r").enabled(!self.shows.is_empty()),
                AppMenuItem::separator(),
                AppMenuItem::new(M_VIEW_SEARCH, "Search Shows").key("cmd+f").checked(self.view == View::Search),
                AppMenuItem::new(M_VIEW_CHARTS, "Top Charts").checked(self.view == View::TopCharts),
                AppMenuItem::new(M_VIEW_NEW, "New Episodes").key("cmd+1").checked(self.view == View::New),
                AppMenuItem::new(M_VIEW_UP_NEXT, "Up Next").key("cmd+2").checked(self.view == View::UpNext),
                AppMenuItem::new(M_VIEW_DOWNLOADED, "Downloaded").key("cmd+3").checked(self.view == View::Downloaded),
                AppMenuItem::new(M_VIEW_SHOWS, "Shows").key("cmd+4").checked(self.view == View::Shows),
                AppMenuItem::separator(),
                AppMenuItem::new(M_SHOW_SETTINGS, "Show Settings...").enabled(matches!(self.view, View::Episodes(_))),
                AppMenuItem::new(M_SETTINGS, "Settings...").key("cmd+,"),
            ],
        };
        // the New Episodes filters sit under New Episodes, one checked at a time
        let mut view = view;
        view.items.splice(4..4, library::NewFilter::ALL.iter().enumerate().map(|(i, f)| AppMenuItem::new(M_NEW_FILTER + i as u32, f.label()).checked(self.new_filter == *f).depth(1)));
        let mut controls = vec![
            AppMenuItem::new(M_PLAY_PAUSE, if self.paused { "Play" } else { "Pause" }).enabled(playing),
            AppMenuItem::new(M_STOP, "Stop").key("cmd+.").enabled(playing),
            AppMenuItem::separator(),
            AppMenuItem::new(M_SKIP_BACK, &format!("Go Back {} Seconds", st.back_s)).key("cmd+left").enabled(playing),
            AppMenuItem::new(M_SKIP_FWD, &format!("Go Forward {} Seconds", st.forward_s)).key("cmd+right").enabled(playing),
            AppMenuItem::new(M_NEXT, "Next Episode").key("cmd+alt+right").enabled(has_queue),
            AppMenuItem::separator(),
            AppMenuItem::new(M_SLOWER, "Slower").key("cmd+[").enabled(playing),
            AppMenuItem::new(M_FASTER, "Faster").key("cmd+]").enabled(playing),
            AppMenuItem::submenu("Playback Speed"),
        ];
        for (i, rate) in transport::RATES.iter().enumerate() {
            controls.push(AppMenuItem::new(M_RATE + i as u32, &format!("{rate}x")).checked(transport::rate_index(self.rate) == Some(i)).enabled(playing).depth(1));
        }
        controls.push(AppMenuItem::submenu("Sleep Timer"));
        for (i, minutes) in SLEEPS.iter().enumerate() {
            let title = if *minutes == 0 { "Off".to_string() } else { format!("{minutes} minutes") };
            controls.push(AppMenuItem::new(M_SLEEP + i as u32, &title).checked(self.sleep == i).enabled(playing).depth(1));
        }
        let episode = AppMenu {
            title: "Episode".into(),
            items: vec![
                AppMenuItem::new(M_EP_PLAY, "Play Episode").enabled(has_ep),
                AppMenuItem::new(M_EP_DETAILS, "Show Episode Details").key("cmd+i").enabled(has_ep),
                AppMenuItem::separator(),
                AppMenuItem::new(M_EP_DOWNLOAD, "Download").key("cmd+d").enabled(has_ep && !downloaded),
                AppMenuItem::new(M_EP_REMOVE_DOWNLOAD, "Remove Download").key("cmd+delete").enabled(downloaded),
                AppMenuItem::separator(),
                AppMenuItem::new(M_EP_PLAY_NEXT, "Play Next").key("cmd+alt+n").enabled(has_ep),
                AppMenuItem::new(M_EP_PLAY_LAST, "Play Last").key("cmd+alt+l").enabled(has_ep),
                AppMenuItem::new(M_EP_UNQUEUE, "Remove from Up Next").enabled(queued),
                AppMenuItem::new(M_EP_UP, "Move Up in Up Next").key("cmd+alt+up").enabled(queued),
                AppMenuItem::new(M_EP_DOWN, "Move Down in Up Next").key("cmd+alt+down").enabled(queued),
                AppMenuItem::new(M_CLEAR_QUEUE, "Clear Up Next").enabled(has_queue),
                AppMenuItem::separator(),
                AppMenuItem::new(M_EP_PLAYED, if played { "Mark as Unplayed" } else { "Mark as Played" }).key("cmd+shift+u").enabled(has_ep),
                AppMenuItem::new(M_SHOW_PLAYED, if show_all_played { "Mark All as Unplayed" } else { "Mark All as Played" }).enabled(show_has_eps),
            ],
        };
        vec![file, view, AppMenu { title: "Controls".into(), items: controls }, episode]
    }

    /// The context menu for a right click at pixel (x, y): selects the episode row there. Empty over nothing.
    pub fn context_items(&mut self, ui: &Ui, x: f32, y: f32) -> Vec<bifrost_app::proto::ContextItem> {
        if self.field.is_some() || self.sheet || y < self.row_top(ui) || y >= self.controls_top(ui) {
            return Vec::new();
        }
        let Some(i) = (0..self.count()).find(|&i| Self::inside(self.row_rect(ui, i), x, y)) else { return Vec::new() };
        if !matches!(self.rows.get(i), Some(Row::Ep(..))) {
            return Vec::new();
        }
        self.select(i);
        self.dirty = true;
        let url = self.selected_episode().and_then(|at| self.episode_url(at));
        let (downloaded, played, queued) = url.as_deref().map_or((false, false, false), |u| (self.store.downloaded(u), self.library.is_played(u), self.library.in_queue(u)));
        let item = |id: u32, label: &str, enabled: bool| bifrost_app::proto::ContextItem { id, label: label.into(), enabled };
        vec![
            item(M_EP_PLAY, "Play Episode", true),
            item(M_EP_DETAILS, "Show Episode Details", true),
            item(M_EP_PLAY_NEXT, "Play Next", true),
            item(M_EP_PLAY_LAST, "Play Last", true),
            item(M_EP_UNQUEUE, "Remove from Up Next", queued),
            item(if downloaded { M_EP_REMOVE_DOWNLOAD } else { M_EP_DOWNLOAD }, if downloaded { "Remove Download" } else { "Download" }, true),
            item(M_EP_PLAYED, if played { "Mark as Unplayed" } else { "Mark as Played" }, true),
        ]
    }

    /// A menu item (or context menu item) was chosen.
    pub fn menu_action(&mut self, ui: &Ui, id: u32) {
        let t = self.now;
        match id {
            M_ADD => self.activate(ui, Target::Add, t),
            M_IMPORT => self.want_file = Some(FileJob::Import),
            M_EXPORT => self.want_file = Some(FileJob::Export),
            M_REFRESH => self.activate(ui, Target::Refresh, t),
            M_VIEW_NEW => self.navigate(ui, SIDE_NEW),
            _ if (M_NEW_FILTER..M_NEW_FILTER + library::NewFilter::ALL.len() as u32).contains(&id) => {
                self.new_filter = library::NewFilter::ALL[(id - M_NEW_FILTER) as usize];
                self.rebuild();
            }
            M_VIEW_UP_NEXT => self.navigate(ui, SIDE_UP_NEXT),
            M_VIEW_DOWNLOADED => self.navigate(ui, SIDE_DOWNLOADED),
            M_VIEW_SHOWS => self.navigate(ui, SIDE_SHOWS),
            M_VIEW_SEARCH => self.navigate(ui, SIDE_SEARCH),
            M_VIEW_CHARTS => self.navigate(ui, SIDE_CHARTS),
            M_SETTINGS => self.navigate(ui, SIDE_SETTINGS),
            M_SHOW_SETTINGS => self.activate(ui, Target::ShowSettings, t),
            M_PLAY_PAUSE => self.activate(ui, Target::PlayPause, t),
            M_STOP => self.activate(ui, Target::Stop, t),
            M_SKIP_BACK => self.activate(ui, Target::SkipBack, t),
            M_SKIP_FWD => self.activate(ui, Target::SkipFwd, t),
            M_NEXT => self.next_episode(),
            M_SLOWER | M_FASTER => {
                if let Some(i) = transport::rate_index(self.rate) {
                    let to = if id == M_SLOWER { i.saturating_sub(1) } else { (i + 1).min(transport::RATES.len() - 1) };
                    self.set_rate(transport::RATES[to]);
                }
            }
            M_CLEAR_QUEUE => {
                self.library.clear_queue();
                self.save_library();
                self.rebuild();
            }
            M_EP_PLAY => self.episode_action(EpAction::Play),
            M_EP_DOWNLOAD => self.episode_action(EpAction::Download),
            M_EP_REMOVE_DOWNLOAD => self.episode_action(EpAction::RemoveDownload),
            M_EP_PLAY_NEXT => self.episode_action(EpAction::PlayNext),
            M_EP_PLAY_LAST => self.episode_action(EpAction::PlayLast),
            M_EP_UNQUEUE => self.episode_action(EpAction::Unqueue),
            M_EP_PLAYED => self.episode_action(EpAction::TogglePlayed),
            M_EP_DETAILS => self.episode_action(EpAction::Details),
            M_EP_UP => self.episode_action(EpAction::MoveUp),
            M_EP_DOWN => self.episode_action(EpAction::MoveDown),
            M_SHOW_PLAYED => self.mark_show_played(),
            _ => {
                if let Some(i) = id.checked_sub(M_RATE).map(|i| i as usize).filter(|&i| i < transport::RATES.len()) {
                    self.set_rate(transport::RATES[i]);
                } else if let Some(i) = id.checked_sub(M_SLEEP).map(|i| i as usize).filter(|&i| i < SLEEPS.len()) {
                    self.sleep = i;
                    self.sleep_at = (SLEEPS[i] > 0).then(|| t + SLEEPS[i] as f64 * 60_000.0);
                }
            }
        }
        self.dirty = true;
    }

    // layout ------------------------------------------------------------

    /// Top of the floating mini player (the bottom of the list when nothing plays).
    fn controls_top(&self, ui: &Ui) -> f32 {
        self.h as f32 - if self.playing.is_some() { ui.pt(PLAYER_H + 2.0 * SPACE_M) } else { 0.0 }
    }

    fn margin(&self, ui: &Ui) -> f32 {
        ui.pt(SPACE_XL)
    }

    /// How many rows the scrolling list has.
    fn count(&self) -> usize {
        match self.view {
            View::Settings | View::ShowSettings(_) => self.option_rows().len(),
            View::Detail(..) => 0,
            _ => self.rows.len(),
        }
    }

    fn row_h(&self, ui: &Ui) -> f32 {
        ui.pt(if matches!(self.view, View::Settings | View::ShowSettings(_)) { OPT_ROW_PT } else { ROW })
    }

    /// Where the rows start: under the search field in Search.
    fn row_top(&self, ui: &Ui) -> f32 {
        self.list_top(ui) + if self.view == View::Search { ui.pt(SEARCH_BAR_PT) } else { 0.0 }
    }

    /// Where the scrolling content ends, unscrolled.
    fn content_end(&self, ui: &Ui) -> f32 {
        match self.view {
            View::Detail(..) => self.list_top(ui) + self.body_h,
            _ => self.row_top(ui) + self.row_h(ui) * self.count() as f32,
        }
    }

    fn row_rect(&self, ui: &Ui, i: usize) -> (f32, f32, f32, f32) {
        let m = self.margin(ui);
        (self.side(ui) + m, self.row_top(ui) + self.row_h(ui) * i as f32 - self.scroll.offset(), self.content_w(ui) - 2.0 * m, self.row_h(ui))
    }

    /// The Follow button of a search result row.
    fn follow_rect(&self, ui: &Ui, i: usize) -> (f32, f32, f32, f32) {
        let r = self.row_rect(ui, i);
        let (w, h) = (ui.pt(88.0), ui.pt(28.0));
        (r.0 + r.2 - w - ui.pt(SPACE_S), r.1 + (r.3 - h) / 2.0, w, h)
    }

    /// The search field's frame (the capsule is drawn a little wider).
    fn search_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let m = self.margin(ui);
        (self.side(ui) + m + ui.pt(SPACE_L), self.list_top(ui) + ui.pt(SPACE_S), self.content_w(ui) - 2.0 * m - ui.pt(2.0 * SPACE_L), ui.pt(SPACE_XXXL))
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

    // episode detail geometry (pixels from the top of the scrolling content)

    /// The buttons of the episode detail: 0 play, 1 download, 2 up next, 3 played.
    fn detail_button(&self, ui: &Ui, n: usize) -> (f32, f32, f32, f32) {
        let m = self.margin(ui);
        let gap = ui.pt(SPACE_S);
        let w = (self.content_w(ui) - 2.0 * m - 3.0 * gap) / 4.0;
        (self.side(ui) + m + (w + gap) * n as f32, self.list_top(ui) - self.scroll.offset() + ui.pt(DETAIL_BUTTONS_PT), w, ui.pt(widgets::CAPSULE_PT))
    }

    /// Offsets from the content top of the chapter list, the notes, and the end of the content.
    fn detail_layout(&self, ui: &Ui, chapters: usize, lines: usize) -> (f32, f32, f32) {
        let chap_y = ui.pt(DETAIL_BUTTONS_PT + widgets::CAPSULE_PT + SPACE_XL);
        let notes_y = if chapters > 0 { chap_y + ui.pt(24.0 + CHAPTER_ROW_PT * chapters as f32 + SPACE_L) } else { chap_y };
        (chap_y, notes_y, notes_y + ui.pt(24.0 + LINE_PT * lines.max(1) as f32 + 2.0 * SPACE_XL))
    }

    fn detail_episode(&self) -> Option<(&Show, &feed::Episode)> {
        let View::Detail(s, e) = self.view else { return None };
        let show = self.shows.get(s)?;
        Some((show, show.episodes.get(e)?))
    }

    // Now Playing sheet panel geometry

    fn panel_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let a = self.art_rect(ui);
        let m = self.margin(ui);
        (self.side(ui) + m, a.1, self.content_w(ui) - 2.0 * m, a.3)
    }

    /// The text width inside the panel's card.
    fn panel_text_w(&self, ui: &Ui) -> f32 {
        self.panel_rect(ui).2 - 2.0 * ui.pt(SPACE_L)
    }

    /// Wraps the notes of the open detail, or the transcript of the open panel, for the width they draw at, and
    /// measures the content that scrolls.
    fn ensure_wrapped(&mut self, ui: &Ui) {
        let measure = |t: &str| ui.measure(t, MAC_BODY_PT, Weight::Regular);
        if self.sheet && self.sheet_panel == SheetPanel::Transcript && self.playing.is_some() {
            let Some(link) = self.playing_episode().map(|e| e.transcript.clone()) else { return };
            let width = self.panel_text_w(ui);
            let key = format!("t:{link}");
            if self.wrapped.as_ref().is_some_and(|w| w.0 == key && w.1 == width.to_bits()) {
                return;
            }
            let mut lines = Vec::new();
            if let Some((_, cues)) = self.transcripts.iter().find(|(u, _)| *u == link) {
                for (i, cue) in cues.iter().enumerate() {
                    for line in wrap::wrap(&cue.text, width, &measure) {
                        lines.push((line, Some(i)));
                    }
                }
            }
            self.panel_h = ui.pt(LINE_PT) * lines.len() as f32 + 2.0 * ui.pt(SPACE_L);
            self.wrapped = Some((key, width.to_bits(), lines));
        } else if let Some((_, ep)) = self.detail_episode().filter(|_| !self.sheet) {
            let width = self.content_w(ui) - 2.0 * self.margin(ui);
            let key = format!("d:{}", ep.url);
            if self.wrapped.as_ref().is_some_and(|w| w.0 == key && w.1 == width.to_bits()) {
                return;
            }
            let (notes, chapters) = (ep.notes.clone(), ep.chapters.len());
            let lines: Vec<(String, Option<usize>)> = wrap::wrap(&notes, width, &measure).into_iter().map(|l| (l, None)).collect();
            self.body_h = self.detail_layout(ui, chapters, lines.len()).2;
            self.wrapped = Some((key, width.to_bits(), lines));
        }
    }

    // hit testing and input ---------------------------------------------

    /// The transcript cue whose text is under `y`, when the transcript panel shows timed lines.
    fn transcript_cue_at(&self, ui: &Ui, y: f32) -> Option<usize> {
        let link = self.playing_episode()?.transcript.clone();
        let (key, _, lines) = self.wrapped.as_ref()?;
        if *key != format!("t:{link}") {
            return None;
        }
        let p = self.panel_rect(ui);
        let line = transcript::line_at(y - p.1 - ui.pt(SPACE_L) + self.panel_scroll.offset(), ui.pt(LINE_PT), lines.len())?;
        let cue = lines.get(line)?.1?;
        let cues = &self.transcripts.iter().find(|(u, _)| *u == link)?.1;
        transcript::start_of(cues, cue).map(|_| cue)
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
                View::New => "New Episodes".into(),
                View::UpNext => "Up Next".into(),
                View::Downloaded => "Downloaded".into(),
                View::Search => "Search".into(),
                View::TopCharts => "Top Charts".into(),
                View::Settings => "Settings".into(),
                View::ShowSettings(i) => self.shows.get(i).map_or("Settings".into(), |s| format!("{} Settings", s.title)),
                View::Detail(..) => self.detail_episode().map_or(String::new(), |(_, e)| e.title.clone()),
            }
        }
    }

    /// The unified toolbar (`NativeToolbar`): the title, then the actions for the current screen.
    fn bar(&self, ui: &Ui) -> Toolbar {
        let room = (self.w as f32 / ui.pt(1.0) - self.side_pt(ui) - 220.0).max(0.0);
        let title_w = (ui.measure(&self.title(), MAC_TITLE3_PT, Weight::Bold) / ui.s + 4.0).min(room);
        let panel = matches!(self.view, View::Settings | View::ShowSettings(_) | View::Detail(..));
        let mut items = Vec::new();
        if panel && self.field.is_none() && !(self.sheet && self.playing.is_some()) {
            items.push(ToolbarItem::icon(BAR_BACK, "Back", "chevron.left"));
        }
        items.push(ToolbarItem::custom(BAR_TITLE, "Title", title_w));
        items.push(ToolbarItem::flexible_space());
        if self.field.is_some() {
            items.push(ToolbarItem::button(BAR_CANCEL, "Cancel"));
            items.push(ToolbarItem::button(BAR_DONE, "Subscribe"));
        } else if self.sheet && self.playing.is_some() {
            items.push(ToolbarItem::button(BAR_CLOSE, "Done"));
        } else if !panel {
            if let View::Episodes(_) = self.view {
                let label = if self.confirm_remove { "Confirm Remove" } else { "Remove Show" };
                items.push(ToolbarItem::icon(BAR_SHOW_SETTINGS, "Show Settings", "ellipsis.circle"));
                items.push(ToolbarItem::icon(BAR_REMOVE, label, "trash").selected(self.confirm_remove));
            }
            items.push(ToolbarItem::icon(BAR_REFRESH, "Refresh", "arrow.clockwise").enabled(!self.shows.is_empty()));
            items.push(ToolbarItem::icon(BAR_ADD, "Add Podcast", "plus"));
        }
        Toolbar::new(items)
    }

    /// The toolbar rectangle in points.
    fn bar_bounds(&self, ui: &Ui) -> Rect {
        let x = self.side_pt(ui) + widgets::TOOLBAR_GAP_PT + SPACE_XS;
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
        let queued = self.library.queue().len() as u32;
        let mut items = vec![
            SidebarItem::header("Library"),
            SidebarItem::row(SIDE_SEARCH, "Search").with_symbol("magnifyingglass"),
            SidebarItem::row(SIDE_CHARTS, "Top Charts").with_symbol("list.number"),
            SidebarItem::row(SIDE_NEW, "New Episodes").with_symbol("star"),
            SidebarItem::row(SIDE_UP_NEXT, "Up Next").with_symbol("music.note.list").with_count(queued),
            SidebarItem::row(SIDE_DOWNLOADED, "Downloaded").with_symbol("arrow.down.circle"),
            SidebarItem::row(SIDE_SHOWS, "Shows").with_symbol("list.bullet").with_count(self.shows.len() as u32),
            SidebarItem::row(SIDE_SETTINGS, "Settings").with_symbol("ellipsis.circle"),
        ];
        if !self.shows.is_empty() {
            items.push(SidebarItem::header("Subscriptions"));
            for (i, sh) in self.shows.iter().enumerate() {
                items.push(SidebarItem::row(SIDE_SHOW + i as u32, &sh.title).with_symbol("globe"));
            }
        }
        let mut list = SourceList::new(items, 32.0, 30.0);
        list.set_selected(Some(self.side_id()));
        list
    }

    fn sidebar_bounds(&self, ui: &Ui) -> Rect {
        Rect { x: 0.0, y: 0.0, w: self.side_pt(ui), h: self.h as f32 / ui.s }
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
                BAR_REFRESH => Some(Target::Refresh),
                BAR_SHOW_SETTINGS => Some(Target::ShowSettings),
                BAR_BACK => Some(Target::Back),
                _ => None,
            };
        }
        if self.field.is_some() {
            return None;
        }
        if self.sheet && self.playing.is_some() {
            if self.speed_menu {
                if let Some(i) = (0..transport::RATES.len()).find(|&i| Self::inside(self.speed_item(ui, i), x, y)) {
                    return Some(Target::Rate(i));
                }
            }
            let sc = self.scrub_rect(ui);
            if Self::inside((sc.0 - ui.pt(SPACE_M), sc.1, sc.2 + ui.pt(SPACE_XL), sc.3), x, y) {
                return Some(Target::Scrub);
            }
            let targets = [Target::SkipBack, Target::PlayPause, Target::SkipFwd, Target::Speed, Target::Sleep, Target::Chapters, Target::Transcript];
            if let Some(t) = targets.into_iter().enumerate().find(|&(n, _)| Self::inside(self.sheet_button(ui, n), x, y)).map(|(_, t)| t) {
                return Some(t);
            }
            if self.sheet_panel != SheetPanel::None && Self::inside(self.panel_rect(ui), x, y) {
                if self.sheet_panel == SheetPanel::Chapters {
                    let p = self.panel_rect(ui);
                    let count = self.playing_episode().map_or(0, |e| e.chapters.len());
                    let row = ((y - p.1 - ui.pt(SPACE_S) + self.panel_scroll.offset()) / ui.pt(CHAPTER_ROW_PT)).floor();
                    if row >= 0.0 && (row as usize) < count {
                        return Some(Target::Chapter(row as usize));
                    }
                }
                if self.sheet_panel == SheetPanel::Transcript {
                    if let Some(cue) = self.transcript_cue_at(ui, y) {
                        return Some(Target::Line(cue));
                    }
                }
                return Some(Target::Panel);
            }
            return None;
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
            if self.view == View::Search && y < self.row_top(ui) {
                return Some(Target::SearchField);
            }
            if let View::Detail(..) = self.view {
                if let Some(n) = (0..4).find(|&n| Self::inside(self.detail_button(ui, n), x, y)) {
                    return Some(Target::Detail(n));
                }
                return Some(Target::List);
            }
            if y >= self.row_top(ui) {
                for i in 0..self.count() {
                    if Self::inside(self.row_rect(ui, i), x, y) {
                        if matches!(self.view, View::Search | View::TopCharts) && Self::inside(self.follow_rect(ui, i), x, y) {
                            return Some(Target::Follow(i));
                        }
                        return Some(if matches!(self.view, View::Settings | View::ShowSettings(_)) { Target::Opt(i) } else { Target::Row(i) });
                    }
                }
            }
            return Some(Target::List);
        }
        None
    }

    /// The rows of the settings list: (label, value).
    fn option_rows(&self) -> Vec<(String, String)> {
        match self.view {
            View::Settings => {
                let st = self.prefs.settings;
                vec![
                    ("Skip Back".into(), format!("{} seconds", st.back_s)),
                    ("Skip Forward".into(), format!("{} seconds", st.forward_s)),
                    ("Refresh Feeds".into(), st.refresh_label()),
                ]
            }
            View::ShowSettings(i) => {
                let p = self.shows.get(i).map_or_else(Default::default, |s| self.prefs.show(&s.url));
                vec![
                    ("Download New Episodes".into(), if p.auto_download { "On" } else { "Off" }.into()),
                    ("Keep Downloads".into(), p.keep_label()),
                    ("Episode Order".into(), p.order_label().into()),
                ]
            }
            _ => Vec::new(),
        }
    }

    /// A settings row was chosen: its value moves to the next choice.
    fn change_option(&mut self, i: usize) {
        match self.view {
            View::Settings => {
                match i {
                    0 => self.prefs.settings.cycle_back(),
                    1 => self.prefs.settings.cycle_forward(),
                    _ => {
                        self.prefs.settings.cycle_refresh();
                        self.last_refresh = Some(self.now);
                    }
                }
                self.prefs.save(&self.store.dir);
            }
            View::ShowSettings(s) => {
                let Some(url) = self.shows.get(s).map(|s| s.url.clone()) else { return };
                let mut p = self.prefs.show(&url);
                match i {
                    0 => p.auto_download = !p.auto_download,
                    1 => p.cycle_keep(),
                    _ => p.oldest_first = !p.oldest_first,
                }
                self.prefs.set_show(&url, p);
                self.prefs.save(&self.store.dir);
                self.after_refresh(&url);
                self.rebuild();
            }
            _ => {}
        }
    }

    fn activate(&mut self, ui: &Ui, target: Target, t: f64) {
        if target != Target::Remove {
            self.confirm_remove = false;
        }
        if !matches!(target, Target::Speed | Target::Rate(_)) {
            self.speed_menu = false;
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
            Target::Side(id) => self.navigate(ui, id),
            Target::Remove => {
                if let View::Episodes(i) = self.view {
                    if self.confirm_remove {
                        self.remove(i);
                    } else {
                        self.confirm_remove = true;
                    }
                }
            }
            Target::Refresh => {
                if !self.shows.is_empty() {
                    self.error.clear();
                    self.refresh_all();
                }
            }
            Target::ShowSettings => {
                if let View::Episodes(i) = self.view {
                    self.open_panel(View::ShowSettings(i));
                }
            }
            Target::Back => {
                self.back_out();
            }
            Target::Opt(i) => self.change_option(i),
            Target::Detail(n) => match n {
                0 => self.episode_action(EpAction::Play),
                1 => {
                    let downloaded = self.selected_episode().and_then(|at| self.episode_url(at)).is_some_and(|u| self.store.downloaded(&u));
                    self.episode_action(if downloaded { EpAction::RemoveDownload } else { EpAction::Download });
                }
                2 => {
                    let queued = self.selected_episode().and_then(|at| self.episode_url(at)).is_some_and(|u| self.library.in_queue(&u));
                    self.episode_action(if queued { EpAction::Unqueue } else { EpAction::PlayNext });
                }
                _ => self.episode_action(EpAction::TogglePlayed),
            },
            Target::Follow(i) => self.follow_hit(i),
            Target::SearchField => {}
            Target::Row(i) => match self.rows.get(i).copied() {
                Some(Row::Show(si)) => self.open_show(si),
                Some(Row::Ep(..)) => {
                    // a click selects the episode; a second click on it plays
                    let again = self.selected == Some(i) && self.last_click.0 == i && t - self.last_click.1 < DOUBLE_CLICK_MS;
                    self.select(i);
                    self.last_click = (i, t);
                    if again {
                        self.episode_action(EpAction::Play);
                    }
                }
                Some(Row::Hit(_)) => self.select(i),
                None => {}
            },
            Target::Stop => self.stop_playback(),
            Target::SkipBack => self.skip(true, self.prefs.settings.back_s as f64),
            Target::Sheet => self.sheet = true,
            Target::CloseSheet => self.sheet = false,
            Target::Scrub => {}
            Target::PlayPause => {
                if self.playing.is_some() {
                    self.audio.toggle();
                    self.paused = !self.paused;
                }
            }
            Target::Speed => self.speed_menu = !self.speed_menu,
            Target::Rate(i) => {
                let rate = transport::RATES.get(i).copied().unwrap_or(self.rate);
                self.set_rate(rate);
                self.speed_menu = false;
            }
            Target::Sleep => {
                self.sleep = (self.sleep + 1) % SLEEPS.len();
                self.sleep_at = (SLEEPS[self.sleep] > 0).then(|| t + SLEEPS[self.sleep] as f64 * 60_000.0);
            }
            Target::Chapters => self.toggle_panel(SheetPanel::Chapters),
            Target::Transcript => self.toggle_panel(SheetPanel::Transcript),
            Target::Chapter(i) => {
                if let Some(ms) = self.playing_episode().and_then(|e| e.chapters.get(i)).map(|c| c.start_ms) {
                    self.seek_to(ms as f64);
                }
            }
            Target::Panel => {}
            Target::Line(cue) => {
                let ms = self.playing_episode().map(|e| e.transcript.clone()).and_then(|link| {
                    let cues = &self.transcripts.iter().find(|(u, _)| *u == link)?.1;
                    transcript::start_of(cues, cue)
                });
                if let Some(ms) = ms {
                    self.seek_to(ms as f64);
                }
            }
            Target::SkipFwd => self.skip(false, self.prefs.settings.forward_s as f64),
            Target::List => {}
        }
        self.dirty = true;
    }

    fn back_out(&mut self) {
        let to = self.back;
        self.go(to);
    }

    fn finish_add(&mut self) {
        let text = self.field.as_ref().map(|f| f.text().to_string()).unwrap_or_default();
        self.field = None;
        self.subscribe(&text);
    }

    pub fn key(&mut self, ui: &Ui, out: Out, t: f64) {
        let searching = self.field.is_none() && self.view == View::Search && self.search_field.is_some() && !(self.sheet && self.playing.is_some());
        if self.field.is_some() {
            if matches!(out, Out::Hide) {
                self.activate(ui, Target::Cancel, t);
            } else if matches!(out, Out::Enter) {
                self.finish_add();
            } else if let Some(f) = &mut self.field {
                f.edit(ui, &out, t);
            }
        } else if searching {
            match out {
                Out::Enter => self.run_search(),
                Out::Named("Down") if !self.rows.is_empty() => self.move_selection(ui, 1),
                Out::Named("Up") if !self.rows.is_empty() => self.move_selection(ui, -1),
                _ => {
                    if let Some(f) = &mut self.search_field {
                        f.edit(ui, &out, t);
                    }
                }
            }
        } else {
            // transport keys, the in-app stand-in for the system's remote commands: space plays or pauses, arrows skip
            match &out {
                Out::Text(s) if s == " " => self.activate(ui, Target::PlayPause, t),
                Out::Named("Left") => self.skip(true, self.prefs.settings.back_s as f64),
                Out::Named("Right") => self.skip(false, self.prefs.settings.forward_s as f64),
                Out::Named("Down") if !self.sheet => self.move_selection(ui, 1),
                Out::Named("Up") if !self.sheet => self.move_selection(ui, -1),
                Out::Enter if !self.sheet => match self.selected.and_then(|i| self.rows.get(i)).copied() {
                    Some(Row::Show(si)) => self.open_show(si),
                    Some(Row::Ep(..)) => self.episode_action(EpAction::Play),
                    Some(Row::Hit(i)) => self.follow_hit(i),
                    None => {}
                },
                Out::Hide => {
                    if self.sheet && self.sheet_panel != SheetPanel::None {
                        self.sheet_panel = SheetPanel::None;
                    } else if self.sheet {
                        self.sheet = false;
                    } else if matches!(self.view, View::Settings | View::ShowSettings(_) | View::Detail(..)) {
                        self.back_out();
                    }
                }
                _ => {}
            }
        }
        self.dirty = true;
    }

    /// Whether a text field has the keys (the add sheet's or the search field), so Edit > Cut, Copy, Paste and Select All
    /// apply to it.
    pub fn editing_text(&self) -> bool {
        self.field_kind().is_some()
    }

    /// Runs an Edit command (`SelectAll`, `Copy`, `Cut`, `Paste`, `Undo`, `Redo`) on the text field that has the keys;
    /// false when none has them.
    pub fn edit_command(&mut self, ui: &Ui, name: &'static str, t: f64) -> bool {
        let Some(search) = self.field_kind() else { return false };
        let Some(f) = self.field_mut(search) else { return false };
        f.edit(ui, &Out::Named(name), t);
        self.dirty = true;
        true
    }

    /// Which text field takes touches now: the add sheet's (false) or the search field (true).
    fn field_kind(&self) -> Option<bool> {
        if self.field.is_some() {
            Some(false)
        } else if self.view == View::Search && self.search_field.is_some() && !(self.sheet && self.playing.is_some()) {
            Some(true)
        } else {
            None
        }
    }

    fn field_frame(&self, ui: &Ui, search: bool) -> (f32, f32, f32, f32) {
        if search { self.search_rect(ui) } else { self.field_rect(ui) }
    }

    fn field_mut(&mut self, search: bool) -> Option<&mut TextField> {
        if search { self.search_field.as_mut() } else { self.field.as_mut() }
    }

    /// The scroller a drag in the current screen moves.
    fn drag_scroller(&mut self) -> &mut Scroller {
        if self.sheet && self.playing.is_some() { &mut self.panel_scroll } else { &mut self.scroll }
    }

    pub fn touch(&mut self, ui: &Ui, event: Event, t: f64) {
        let (x, y) = match event {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            _ => return,
        };
        let w = self.w as f32;
        match event {
            Event::Down(..) => {
                self.dirty = true;
                if let Some(search) = self.field_kind() {
                    let frame = self.field_frame(ui, search);
                    // the search field only owns the band above its results
                    let band = (self.side(ui), self.list_top(ui), self.content_w(ui), self.row_top(ui) - self.list_top(ui));
                    if !search || Self::inside(band, x, y) {
                        if let Some(f) = self.field_mut(search) {
                            if f.touch(ui, event, frame, w, t) {
                                self.field_touch = true;
                                return;
                            }
                        }
                    }
                }
                self.pressed = self.hit(ui, x, y);
                self.reordering = None;
                self.hold = match self.pressed {
                    Some(Target::Row(i)) if self.view == View::UpNext => self.row_url(i).map(|url| (url, t, y)),
                    _ => None,
                };
                if self.pressed == Some(Target::Scrub) {
                    self.scrubbing = Some(self.scrub_ms(ui, x));
                    return;
                }
                if matches!(self.pressed, Some(Target::Row(_) | Target::List | Target::Panel | Target::Line(_) | Target::Chapter(_) | Target::Opt(_) | Target::Detail(_) | Target::Follow(_))) {
                    self.scrolling = true;
                    self.drag_scroller().down(y, t);
                }
            }
            Event::Move(..) => {
                if let Some(url) = self.reordering.clone() {
                    if let Some(Target::Row(j)) = self.hit(ui, x, y) {
                        self.move_in_up_next(&url, j);
                    }
                    self.dirty = true;
                } else if self.field_touch {
                    if let Some(search) = self.field_kind() {
                        let frame = self.field_frame(ui, search);
                        if let Some(f) = self.field_mut(search) {
                            f.touch(ui, event, frame, w, t);
                        }
                    }
                    self.dirty = true;
                } else if self.scrubbing.is_some() {
                    self.scrubbing = Some(self.scrub_ms(ui, x));
                    self.dirty = true;
                } else if self.scrolling {
                    let sc = self.drag_scroller();
                    sc.moved(y, t);
                    let dragging = sc.dragging();
                    if dragging {
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
                self.hold = None;
                if self.reordering.take().is_some() {
                    self.save_library();
                    self.pressed = None;
                    self.scrolling = false;
                    return;
                }
                if std::mem::take(&mut self.field_touch) {
                    if let Some(search) = self.field_kind() {
                        let frame = self.field_frame(ui, search);
                        if let Some(f) = self.field_mut(search) {
                            f.touch(ui, event, frame, w, t);
                        }
                    }
                    return;
                }
                if self.scrubbing.take().is_some() {
                    self.pressed = None;
                    let ms = self.scrub_ms(ui, x);
                    self.seek_to(ms);
                    return;
                }
                let tap = if std::mem::take(&mut self.scrolling) { self.drag_scroller().up(y, t) } else { true };
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
        self.want_chapters();
        self.check_hold(t);
        if self.refresh.pump() {
            self.dirty = true;
        }
        match (self.prefs.settings.refresh_ms(), self.last_refresh) {
            (_, None) => {
                if self.shows.is_empty() || self.prefs.settings.refresh_ms().is_none() {
                    self.last_refresh = Some(t);
                } else {
                    self.refresh_all();
                }
            }
            (Some(ms), Some(at)) if t - at >= ms => self.refresh_all(),
            _ => {}
        }
        self.ensure_wrapped(ui);
        self.scroll.set_max((self.content_end(ui) - self.controls_top(ui)).max(0.0));
        self.dirty |= self.scroll.tick(t);
        self.tick_panel(ui, t);
        if let Some(f) = &mut self.field {
            self.dirty |= f.tick(ui, t);
            if f.focused {
                self.dirty = true;
            }
        }
        if self.field.is_none() && self.view == View::Search {
            if let Some(f) = &mut self.search_field {
                self.dirty |= f.tick(ui, t);
                if f.focused {
                    self.dirty = true;
                }
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

    /// A held Up Next row turns into a drag once the press has lasted `HOLD_MS` without the list scrolling.
    fn check_hold(&mut self, t: f64) {
        let Some((url, at, y)) = self.hold.clone() else { return };
        if self.pressed.is_none() || self.drag_scroller().dragging() {
            self.hold = None;
        } else if t - at >= HOLD_MS {
            self.hold = None;
            // the press never became a scroll: end its scroll touch without a fling
            self.drag_scroller().up(y, t);
            self.scrolling = false;
            self.pressed = None;
            self.reordering = Some(url);
            self.dirty = true;
        }
    }

    /// The address of the episode in row `i`, when that row is an episode.
    fn row_url(&self, i: usize) -> Option<String> {
        match self.rows.get(i)? {
            Row::Ep(s, e) => self.episode_url((*s, *e)),
            _ => None,
        }
    }

    /// Drags the Up Next episode `url` to the place of row `j`.
    fn move_in_up_next(&mut self, url: &str, j: usize) {
        let Some(target) = self.row_url(j).and_then(|u| self.library.queue().iter().position(|q| *q == u)) else { return };
        if self.library.move_to(url, target) {
            self.rebuild();
        }
    }

    /// Sizes the Now Playing panel's scroll range, and keeps the spoken line of a transcript in view.
    fn tick_panel(&mut self, ui: &Ui, t: f64) {
        if !(self.sheet && self.playing.is_some()) || self.sheet_panel == SheetPanel::None {
            return;
        }
        let view_h = self.panel_rect(ui).3;
        let content = match self.sheet_panel {
            SheetPanel::Chapters => ui.pt(CHAPTER_ROW_PT) * self.playing_episode().map_or(0, |e| e.chapters.len()) as f32 + 2.0 * ui.pt(SPACE_S),
            _ => self.panel_h,
        };
        self.panel_scroll.set_max((content - view_h).max(0.0));
        self.dirty |= self.panel_scroll.tick(t);
        if self.sheet_panel == SheetPanel::Transcript && !self.panel_scroll.dragging() {
            let active = self.playing_episode().map(|e| e.transcript.clone()).and_then(|link| {
                let cues = &self.transcripts.iter().find(|(u, _)| *u == link)?.1;
                transcript::active(cues, self.position_ms as u64)
            });
            if active != self.followed {
                self.followed = active;
                let first = active.and_then(|c| self.wrapped.as_ref()?.2.iter().position(|(_, cue)| *cue == Some(c)));
                if let Some(line) = first {
                    let target = ui.pt(LINE_PT) * line as f32 - view_h / 3.0;
                    self.panel_scroll.jump_to(target.max(0.0));
                }
            }
        }
    }

    pub fn next_wake(&self, t: f64) -> Option<f64> {
        // a playback clock only: downloads and feeds arrive through `deliver` (a main-queue wake), not by polling
        let mut due = self.playing.is_some().then_some(t + POLL_MS);
        let hold = self.hold.as_ref().map(|(_, at, _)| at + HOLD_MS);
        // the service socket is only read between frames: once a second while idle, to notice an episode playing
        // This wake is a one-second media-service clock interval, not a layout/resolution constant.
        let listening = if self.audio.connected() { Some(t + AUDIO_WAKE_INTERVAL_S) } else { self.audio.next_wake(t) };
        due = [due, listening].into_iter().flatten().reduce(f64::min);
        let refresh = self.last_refresh.zip(self.prefs.settings.refresh_ms()).map(|(at, ms)| at + ms);
        let search = if self.view == View::Search { self.search_field.as_ref().and_then(|f| f.next_wake(t)) } else { None };
        for wake in [self.field.as_ref().and_then(|f| f.next_wake(t)), search, refresh, hold, self.scroll.animating().then_some(t), self.panel_scroll.animating().then_some(t)] {
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
            self.wrapped = None;
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

    /// A push button: a capsule plate with tinted text (`NativeButton`, rounded bezel). The label is shortened to fit.
    fn button(&mut self, s: &mut Surface, ui: &Ui, target: Target, label: &str, color: u32, r: (f32, f32, f32, f32)) {
        let fill = if self.pressed == Some(target) { ROW_PRESSED.now() } else { FIELD_BG.now() };
        s.fill_rrect(r.0, r.1, r.2, r.3, r.3 / 2.0, fill);
        let label = ui.fit(label, MAC_BODY_PT, (r.2 - ui.pt(SPACE_M)).max(0.0));
        self.centred(s, ui, &label, MAC_BODY_PT, color, r);
    }

    /// A button that stays lit while its panel is open.
    fn toggle_button(&mut self, s: &mut Surface, ui: &Ui, target: Target, label: &str, on: bool, r: (f32, f32, f32, f32)) {
        if on {
            s.fill_rrect(r.0, r.1, r.2, r.3, r.3 / 2.0, SELECTION.now());
            let label = ui.fit(label, MAC_BODY_PT, (r.2 - ui.pt(SPACE_M)).max(0.0));
            self.centred(s, ui, &label, MAC_BODY_PT, WHITE, r);
        } else {
            self.button(s, ui, target, label, LINK, r);
        }
    }

    /// The chapters of the playing episode or its transcript, on a card where the artwork would be.
    fn draw_panel(&mut self, s: &mut Surface, ui: &Ui) {
        let p = self.panel_rect(ui);
        s.fill_rrect(p.0, p.1, p.2, p.3, ui.pt(SPACE_L), CARD_BG.now());
        let (pad, off) = (ui.pt(SPACE_L), self.panel_scroll.offset());
        let inner_top = p.1 + ui.pt(SPACE_S);
        let (lo, hi) = (p.1, p.1 + p.3);
        match self.sheet_panel {
            SheetPanel::Chapters => {
                let chapters = self.playing_episode().map(|e| e.chapters.clone()).unwrap_or_default();
                if chapters.is_empty() {
                    self.centred(s, ui, "No chapters for this episode", MAC_BODY_PT, LABEL_2.now(), p);
                    return;
                }
                let current = feed::current_chapter(&chapters, self.position_ms as u64);
                let row_h = ui.pt(CHAPTER_ROW_PT);
                for (i, chapter) in chapters.iter().enumerate() {
                    let y = inner_top + row_h * i as f32 - off;
                    if y < lo || y + row_h > hi {
                        continue;
                    }
                    let on = current == Some(i);
                    if on {
                        s.fill_rrect(p.0 + ui.pt(SPACE_XS), y, p.2 - ui.pt(SPACE_S), row_h, ui.pt(m::SELECTION_RADIUS), SELECTION.now());
                    }
                    let (ink, dim) = if on { (WHITE, WHITE) } else { (LABEL.now(), LABEL_2.now()) };
                    let clock = springboard::shell::music::nowplaying::clock(chapter.start_ms as f64);
                    let ty = y + (row_h - ui.pt(MAC_BODY_PT)) / 2.0 - ui.pt(SPACE_XXS);
                    self.text(s, ui, &clock, MAC_BODY_PT, Weight::Regular, dim, p.0 + pad, ty);
                    let room = p.2 - 2.0 * pad - ui.pt(64.0);
                    let title = ui.fit(&chapter.title, MAC_BODY_PT, room);
                    self.text(s, ui, &title, MAC_BODY_PT, Weight::Regular, ink, p.0 + pad + ui.pt(64.0), ty);
                }
            }
            _ => {
                let link = self.playing_episode().map(|e| e.transcript.clone()).unwrap_or_default();
                if link.is_empty() {
                    self.centred(s, ui, "No transcript for this episode", MAC_BODY_PT, LABEL_2.now(), p);
                    return;
                }
                let lines = match &self.wrapped {
                    Some((key, _, lines)) if *key == format!("t:{link}") => lines.clone(),
                    _ => Vec::new(),
                };
                if lines.is_empty() {
                    let msg = if self.transcript_loading.as_deref() == Some(link.as_str()) { "Loading transcript..." } else { "The transcript is not available" };
                    self.centred(s, ui, msg, MAC_BODY_PT, LABEL_2.now(), p);
                    return;
                }
                let active = self.transcripts.iter().find(|(u, _)| *u == link).and_then(|(_, cues)| transcript::active(cues, self.position_ms as u64));
                let line_h = ui.pt(LINE_PT);
                for (j, (line, cue)) in lines.iter().enumerate() {
                    let y = p.1 + pad + line_h * j as f32 - off;
                    if y < lo || y + line_h > hi {
                        continue;
                    }
                    let on = active.is_some() && *cue == active;
                    let (ink, weight) = if on { (LINK, Weight::Semibold) } else { (LABEL.now(), Weight::Regular) };
                    self.text(s, ui, line, MAC_BODY_PT, weight, ink, p.0 + pad, y);
                }
            }
        }
    }

    fn draw_sheet(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        let url = self.playing.clone().unwrap_or_default();
        let found = self.shows.iter().find_map(|sh| {
            sh.episodes.iter().find(|e| e.url == url).map(|e| (sh.title.clone(), e.title.clone(), e.date.clone(), e.duration.clone()))
        });
        let (show, title, released, feed_length) = found.unwrap_or_default();
        let (x0, cw) = (self.side(ui), self.content_w(ui));
        let m = self.margin(ui);
        let a = self.art_rect(ui);
        if self.sheet_panel == SheetPanel::None {
            // artwork: the Podcasts mark on a rounded plate
            let side = a.2 as usize;
            let art = crate::icon::icon(side);
            s.fill_rrect(a.0, a.1, a.2, a.3, ui.pt(SPACE_L), CARD_BG.now());
            s.blit(&art, a.0 as i32, a.1 as i32, 255, 255);
        } else {
            self.draw_panel(s, ui);
        }
        let title = ui.fit(&title, MAC_TITLE3_PT, cw - 2.0 * m);
        self.centred(s, ui, &title, MAC_TITLE3_PT, LABEL.now(), (x0, a.1 + a.3 + ui.pt(SPACE_S), cw, ui.pt(30.0)));
        let show = ui.fit(&show, MAC_SUBHEAD_PT, cw - 2.0 * m);
        self.centred(s, ui, &show, MAC_SUBHEAD_PT, LABEL_2.now(), (x0, a.1 + a.3 + ui.pt(36.0), cw, ui.pt(SPACE_XXL)));
        // episode info: type, codec, length and release date, one footnote line above the scrubber
        let line = ui.fit(&info::summary(&url, self.duration_ms, &feed_length, &released), MAC_FOOTNOTE_PT, cw - 2.0 * m);
        self.centred(s, ui, &line, MAC_FOOTNOTE_PT, LABEL_2.now(), (x0, a.1 + a.3 + ui.pt(58.0), cw, ui.pt(SPACE_L)));
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
        let (back, fwd) = (self.prefs.settings.back_s, self.prefs.settings.forward_s);
        self.button(s, ui, Target::SkipBack, &format!("Back {back}"), LINK, self.sheet_button(ui, 0));
        self.button(s, ui, Target::PlayPause, if self.paused { "Play" } else { "Pause" }, LINK, self.sheet_button(ui, 1));
        self.button(s, ui, Target::SkipFwd, &format!("Fwd {fwd}"), LINK, self.sheet_button(ui, 2));
        self.button(s, ui, Target::Speed, &transport::speed_label(self.rate), LINK, self.sheet_button(ui, 3));
        let label = self.sleep_label(t);
        self.button(s, ui, Target::Sleep, &label, LINK, self.sheet_button(ui, 4));
        let panel = self.sheet_panel;
        self.toggle_button(s, ui, Target::Chapters, "Chapters", panel == SheetPanel::Chapters, self.sheet_button(ui, 5));
        self.toggle_button(s, ui, Target::Transcript, "Transcript", panel == SheetPanel::Transcript, self.sheet_button(ui, 6));
        if self.speed_menu {
            self.draw_speed_menu(s, ui);
        }
    }

    /// The label under the row's title: date, length and the state of its download.
    fn episode_line(&self, si: usize, ei: usize) -> String {
        let (show, e) = (&self.shows[si], &self.shows[si].episodes[ei]);
        let mut sub = if matches!(self.view, View::Episodes(_)) { String::new() } else { format!("{}  ", show.title) };
        sub.push_str(&e.date);
        if !e.duration.is_empty() {
            sub = format!("{sub}  {}", e.duration);
        }
        if self.downloading.as_deref() == Some(e.url.as_str()) {
            sub = match self.progress {
                Some((at, Some(total))) if total > 0 => format!("{sub}  Downloading {}%", at * 100 / total),
                Some((at, _)) => format!("{sub}  Downloading {} MB", at >> 20),
                None => format!("{sub}  Downloading..."),
            };
        } else if self.dl_queue.contains(&e.url) {
            sub = format!("{sub}  Waiting to download");
        } else if self.store.downloaded(&e.url) {
            sub = format!("{sub}  Downloaded");
        }
        if self.view != View::UpNext && self.library.in_queue(&e.url) {
            sub = format!("{sub}  Up Next");
        }
        sub
    }

    fn draw_rows(&mut self, s: &mut Surface, ui: &Ui) {
        if matches!(self.view, View::Settings | View::ShowSettings(_)) {
            self.draw_options(s, ui);
            return;
        }
        let x0 = self.side(ui);
        let cw = self.content_w(ui);
        let bottom = self.controls_top(ui);
        let top = self.row_top(ui);
        let n = self.count();
        if n == 0 {
            let status = self.refresh.text();
            let busy = !self.loading.is_empty();
            let msg = match self.view {
                View::Shows if busy => status.unwrap_or_else(|| "Loading...".into()),
                View::Shows => "No Podcasts. Click Add and paste a feed URL.".into(),
                View::Episodes(_) if busy => status.unwrap_or_else(|| "Loading...".into()),
                View::Episodes(_) => "No Episodes".into(),
                View::New => "No new episodes".into(),
                View::UpNext => "Up Next is empty. Choose Play Next or Play Last on an episode.".into(),
                View::Downloaded => "No downloaded episodes".into(),
                View::Search if self.searching => "Searching...".into(),
                View::Search if self.search_field.as_ref().is_some_and(|f| !f.is_empty()) => "No shows found".into(),
                View::Search => "Search for shows by name, author or network and press Return.".into(),
                View::TopCharts if self.charts_loading => "Loading the Top Charts...".into(),
                View::TopCharts => "The Top Charts could not be loaded. Choose Top Charts again to retry.".into(),
                _ => String::new(),
            };
            let msg = ui.fit(&msg, MAC_TITLE3_PT, cw - 2.0 * self.margin(ui));
            self.centred(s, ui, &msg, MAC_TITLE3_PT, LABEL_2.now(), (x0, top, cw, (bottom - top).max(0.0)));
        }
        for i in 0..n {
            let r = self.row_rect(ui, i);
            if r.1 + r.3 < top || r.1 >= bottom {
                continue;
            }
            let row = self.rows[i];
            let (title, sub, active, unplayed) = match row {
                Row::Show(si) => {
                    let sh = &self.shows[si];
                    (sh.title.clone(), format!("{} episodes", sh.episodes.len()), false, false)
                }
                Row::Ep(si, ei) => {
                    let e = &self.shows[si].episodes[ei];
                    (e.title.clone(), self.episode_line(si, ei), self.playing.as_deref() == Some(e.url.as_str()), !self.library.is_played(&e.url))
                }
                Row::Hit(hi) => {
                    let hit = &self.hits()[hi];
                    (hit.title.clone(), hit.author.clone(), false, false)
                }
            };
            // the playing episode sits on the accent selection, as in Golden Gate Podcasts
            let ink = if active { WHITE } else if matches!(row, Row::Ep(..)) && !unplayed { LABEL_2.now() } else { LABEL.now() };
            if active {
                s.fill_rrect(r.0, r.1.max(0.0), r.2, r.3, ui.pt(m::SELECTION_RADIUS), SELECTION.now());
            } else if self.pressed == Some(Target::Row(i)) || self.selected == Some(i) {
                s.fill_rrect(r.0, r.1.max(0.0), r.2, r.3, ui.pt(m::SELECTION_RADIUS), ROW_PRESSED.now());
            } else if i + 1 < n {
                // hairline inset to the text start, none under the last row
                s.fill_rect((r.0 + ui.pt(SPACE_L)) as usize, (r.1 + r.3 - 1.0).max(0.0) as usize, (r.2 - ui.pt(SPACE_L)) as usize, 1, SEPARATOR_C.now());
            }
            if unplayed && !active {
                // an unplayed episode carries the accent dot
                s.fill_rrect(r.0 + ui.pt(SPACE_XS), r.1 + ui.pt(SPACE_M + 5.0), ui.pt(SPACE_S), ui.pt(SPACE_S), ui.pt(SPACE_XS), LINK);
            }
            let room = if matches!(row, Row::Hit(_)) { r.2 - ui.pt(SPACE_XXXL) - ui.pt(96.0) } else { r.2 - ui.pt(SPACE_XXXL) };
            let title = ui.fit(&title, MAC_BODY_PT, room);
            self.text(s, ui, &title, MAC_BODY_PT, Weight::Semibold, ink, r.0 + ui.pt(SPACE_L), r.1 + ui.pt(SPACE_M));
            let sub = ui.fit(&sub, MAC_SUBHEAD_PT, room);
            let sub_ink = if active { WHITE } else { LABEL_2.now() };
            self.text(s, ui, &sub, MAC_SUBHEAD_PT, Weight::Regular, sub_ink, r.0 + ui.pt(SPACE_L), r.1 + ui.pt(38.0));
            if let Row::Hit(hi) = row {
                let followed = self.shows.iter().any(|sh| sh.url == self.hits()[hi].feed);
                let f = self.follow_rect(ui, i);
                if followed {
                    self.centred(s, ui, "Following", MAC_SUBHEAD_PT, LABEL_2.now(), f);
                } else {
                    self.button(s, ui, Target::Follow(i), "Follow", LINK, f);
                }
            }
        }
        if self.view == View::Search {
            self.draw_search_bar(s, ui);
        }
    }

    /// The search field over its band (rows scroll under the band, not over the field).
    fn draw_search_bar(&mut self, s: &mut Surface, ui: &Ui) {
        let (x0, cw) = (self.side(ui), self.content_w(ui));
        s.fill_rect(x0 as usize, self.list_top(ui) as usize, cw as usize, (self.row_top(ui) - self.list_top(ui)) as usize, CONTENT_BG.now());
        let f = self.search_rect(ui);
        let pad = ui.pt(SPACE_S);
        // search fields are capsules (CLAUDE.md, Radii)
        s.fill_rrect(f.0 - pad, f.1, f.2 + 2.0 * pad, f.3, f.3 / 2.0, FIELD_BG.now());
        let t = self.now;
        if let Some(field) = &self.search_field {
            field.draw(s, ui, f, t);
        }
    }

    /// A grouped list of settings: the label, then the current choice in the tint; a row cycles through its choices.
    fn draw_options(&mut self, s: &mut Surface, ui: &Ui) {
        let rows = self.option_rows();
        let (top, bottom) = (self.row_top(ui), self.controls_top(ui));
        for (i, (label, value)) in rows.iter().enumerate() {
            let r = self.row_rect(ui, i);
            if r.1 + r.3 < top || r.1 >= bottom {
                continue;
            }
            if self.pressed == Some(Target::Opt(i)) {
                s.fill_rrect(r.0, r.1.max(0.0), r.2, r.3, ui.pt(m::SELECTION_RADIUS), ROW_PRESSED.now());
            } else if i + 1 < rows.len() {
                s.fill_rect((r.0 + ui.pt(SPACE_L)) as usize, (r.1 + r.3 - 1.0).max(0.0) as usize, (r.2 - ui.pt(SPACE_L)) as usize, 1, SEPARATOR_C.now());
            }
            let ty = r.1 + (r.3 - ui.pt(MAC_BODY_PT)) / 2.0 - ui.pt(SPACE_XXS);
            self.text(s, ui, label, MAC_BODY_PT, Weight::Regular, LABEL.now(), r.0 + ui.pt(SPACE_L), ty);
            let value = ui.fit(value, MAC_BODY_PT, r.2 / 2.0);
            let vw = ui.measure(&value, MAC_BODY_PT, Weight::Regular);
            self.text(s, ui, &value, MAC_BODY_PT, Weight::Regular, LINK, r.0 + r.2 - vw - ui.pt(SPACE_L), ty);
        }
        let hint = match self.view {
            View::Settings => "Skip intervals apply to the buttons, the arrow keys and the Control Center tile.",
            _ => "Downloads beyond the number to keep are deleted when the feed updates.",
        };
        let y = self.row_top(ui) + self.row_h(ui) * rows.len() as f32 - self.scroll.offset() + ui.pt(SPACE_M);
        let hint = ui.fit(hint, MAC_FOOTNOTE_PT, self.content_w(ui) - 2.0 * self.margin(ui));
        self.text(s, ui, &hint, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), self.side(ui) + self.margin(ui) + ui.pt(SPACE_L), y);
    }

    /// An episode's page: title, the action buttons, chapters and notes.
    fn draw_detail(&mut self, s: &mut Surface, ui: &Ui) {
        let Some((show, ep)) = self.detail_episode() else { return };
        let (show_title, title, date, duration, url) = (show.title.clone(), ep.title.clone(), ep.date.clone(), ep.duration.clone(), ep.url.clone());
        let chapters = ep.chapters.clone();
        let (top, bottom) = (self.list_top(ui), self.controls_top(ui));
        let x = self.side(ui) + self.margin(ui);
        let width = self.content_w(ui) - 2.0 * self.margin(ui);
        let origin = top - self.scroll.offset();
        let title = ui.fit(&title, MAC_TITLE3_PT, width);
        self.text(s, ui, &title, MAC_TITLE3_PT, Weight::Bold, LABEL.now(), x, origin + ui.pt(SPACE_S));
        let mut sub = show_title;
        for part in [&date, &duration] {
            if !part.is_empty() {
                sub = format!("{sub}  {part}");
            }
        }
        let sub = ui.fit(&sub, MAC_SUBHEAD_PT, width);
        self.text(s, ui, &sub, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), x, origin + ui.pt(34.0));
        let downloaded = self.store.downloaded(&url);
        let labels = [
            "Play".to_string(),
            if downloaded { "Remove Download" } else { "Download" }.to_string(),
            if self.library.in_queue(&url) { "Remove from Up Next" } else { "Play Next" }.to_string(),
            if self.library.is_played(&url) { "Mark Unplayed" } else { "Mark Played" }.to_string(),
        ];
        for (n, label) in labels.iter().enumerate() {
            self.button(s, ui, Target::Detail(n), label, LINK, self.detail_button(ui, n));
        }
        let lines = self.wrapped.as_ref().map(|w| w.2.clone()).unwrap_or_default();
        let (chap_y, notes_y, _) = self.detail_layout(ui, chapters.len(), lines.len());
        if !chapters.is_empty() {
            self.text(s, ui, "Chapters", MAC_FOOTNOTE_PT, Weight::Semibold, LABEL_2.now(), x, origin + chap_y);
            let current = if self.playing.as_deref() == Some(url.as_str()) { feed::current_chapter(&chapters, self.position_ms as u64) } else { None };
            for (i, chapter) in chapters.iter().enumerate() {
                let y = origin + chap_y + ui.pt(24.0 + CHAPTER_ROW_PT * i as f32);
                if y + ui.pt(CHAPTER_ROW_PT) < top || y >= bottom {
                    continue;
                }
                let clock = springboard::shell::music::nowplaying::clock(chapter.start_ms as f64);
                let ink = if current == Some(i) { LINK } else { LABEL.now() };
                self.text(s, ui, &clock, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), x, y);
                let name = ui.fit(&chapter.title, MAC_BODY_PT, width - ui.pt(64.0));
                self.text(s, ui, &name, MAC_BODY_PT, Weight::Regular, ink, x + ui.pt(64.0), y);
            }
        }
        self.text(s, ui, "Notes", MAC_FOOTNOTE_PT, Weight::Semibold, LABEL_2.now(), x, origin + notes_y);
        let line_h = ui.pt(LINE_PT);
        if lines.iter().all(|(l, _)| l.is_empty()) {
            self.text(s, ui, "No notes", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), x, origin + notes_y + ui.pt(SPACE_XXL));
        }
        for (j, (line, _)) in lines.iter().enumerate() {
            let y = origin + notes_y + ui.pt(SPACE_XXL) + line_h * j as f32;
            if line.is_empty() || y + line_h < top || y >= bottom {
                continue;
            }
            self.text(s, ui, line, MAC_BODY_PT, Weight::Regular, LABEL.now(), x, y);
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
        let (back, fwd) = (self.prefs.settings.back_s, self.prefs.settings.forward_s);
        self.button(s, ui, Target::SkipBack, &format!("-{back}s"), LINK, self.player_rect(ui, 0));
        self.button(s, ui, Target::Stop, "Stop", DESTRUCTIVE, self.player_rect(ui, 1));
        self.button(s, ui, Target::SkipFwd, &format!("+{fwd}s"), LINK, self.player_rect(ui, 2));
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        self.dirty = false;
        self.now = t;
        let style = (ui.s.to_bits(), ui.text_scale.to_bits(), ui.bold);
        if style != self.cache_style {
            self.cache.clear();
            self.cache_style = style;
            self.wrapped = None;
        }
        if self.cache.len() > 256 {
            self.cache.clear();
        }
        self.ensure_wrapped(ui);
        s.fill_rect(0, 0, self.w as usize, self.h as usize, CONTENT_BG.now());
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
            if let View::Detail(..) = self.view {
                self.draw_detail(s, ui);
            } else {
                self.draw_rows(s, ui);
            }
            self.draw_player(s, ui);
        }
        if !self.error.is_empty() || !self.notice.is_empty() {
            let (text, ink) = if self.error.is_empty() { (self.notice.clone(), LABEL_2.now()) } else { (self.error.clone(), DESTRUCTIVE) };
            let msg = ui.fit(&text, MAC_FOOTNOTE_PT, cw - 2.0 * m);
            self.centred(s, ui, &msg, MAC_FOOTNOTE_PT, ink, (x0, ui.pt(UNIFIED_BAR_PT), cw, ui.pt(LIST_TOP - UNIFIED_BAR_PT)));
        }
        // the band and sidebar repaint over any row that scrolled under them
        let pressed_side = match self.pressed {
            Some(Target::Side(id)) => Some(id),
            _ => None,
        };
        widgets::source_list(s, ui, self.sidebar_bounds(ui), &self.sidebar(), SIDEBAR_SCROLL, pressed_side);
        s.fill_rect((x0 - 1.0).max(0.0) as usize, 0, 1, self.h as usize, SEPARATOR_C.now());
        let w_pt = self.w as f32 / ui.s;
        s.fill_rect(x0 as usize, 0, cw as usize, ui.pt(UNIFIED_BAR_PT) as usize, CONTENT_BG.now());
        let side = self.side_pt(ui);
        widgets::toolbar_band(s, ui, Rect { x: side, y: 0.0, w: w_pt - side, h: UNIFIED_BAR_PT });
        let pressed_bar = match self.pressed {
            Some(Target::Add) => Some(BAR_ADD),
            Some(Target::Remove) => Some(BAR_REMOVE),
            Some(Target::Cancel) => Some(BAR_CANCEL),
            Some(Target::Done) => Some(BAR_DONE),
            Some(Target::CloseSheet) => Some(BAR_CLOSE),
            Some(Target::Refresh) => Some(BAR_REFRESH),
            Some(Target::ShowSettings) => Some(BAR_SHOW_SETTINGS),
            Some(Target::Back) => Some(BAR_BACK),
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

    fn test_ui() -> Ui {
        let font = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf");
        let data: &'static [u8] = Box::leak(std::fs::read(font).unwrap().into_boxed_slice());
        Ui::new(bifrost_app::kit::Font::parse(data).unwrap(), 1.0)
    }

    fn podcasts(name: &str, ui: &Ui) -> Podcasts {
        Podcasts::new(ui, 1000, 700, scratch(name), Arc::new(Broken), &scratch(&format!("{name}-svc")))
    }

    #[test]
    fn select_all_in_the_search_field_lets_typing_replace_the_term() {
        let ui = test_ui();
        let mut p = podcasts("select-all", &ui);
        assert!(!p.editing_text(), "no field has the keys on the shows list");
        assert!(!p.edit_command(&ui, "SelectAll", 0.0));
        p.open_search(&ui);
        assert!(p.editing_text());
        p.key(&ui, Out::Text("Relay FM".into()), 1.0);
        assert!(p.edit_command(&ui, "SelectAll", 2.0));
        p.key(&ui, Out::Text("Accidental Tech Podcast".into()), 3.0);
        assert_eq!(p.search_field.as_ref().map(|f| f.text().to_string()).as_deref(), Some("Accidental Tech Podcast"), "the selected term is replaced, not appended to");
    }

    #[test]
    fn the_top_charts_list_the_directory_shows_with_follow_buttons() {
        let ui = test_ui();
        let mut p = podcasts("charts", &ui);
        p.navigate(&ui, SIDE_CHARTS);
        assert_eq!(p.view, View::TopCharts);
        assert!(p.charts_loading, "the first visit fetches the list");
        assert_eq!(p.sidebar().selected(), Some(SIDE_CHARTS));
        let hits = vec![search::Hit { title: "This American Life".into(), author: "This American Life".into(), feed: "https://tal.test/rss".into() }, search::Hit { title: "Radiolab".into(), author: "WNYC Studios".into(), feed: "https://radiolab.test/rss".into() }];
        p.inbox.lock().unwrap().push_back(Msg::Charts(Ok(hits)));
        p.pump();
        assert!(!p.charts_loading);
        assert_eq!(p.rows, [Row::Hit(0), Row::Hit(1)]);
        assert_eq!(p.row_key(&Row::Hit(1)), "https://radiolab.test/rss", "a chart row stands for its feed");
        assert!(p.results.is_empty(), "the charts are not search results");
        p.open_search(&ui);
        assert!(p.rows.is_empty(), "Search lists its own results");
        p.navigate(&ui, SIDE_CHARTS);
        assert!(!p.charts_loading && p.rows.len() == 2, "the charts are kept, not fetched again");
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
