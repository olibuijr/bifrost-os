//! The whole user interface without Android: lock screen, SpringBoard and
//! built-in apps, driven by plain touch/key events and drawn into a Surface.

pub mod pasteboard;
pub mod battery;
pub mod browser;
pub mod accessibility;
pub mod atspi;
pub mod account;
pub mod applib;
pub mod apphost;
pub mod folder_actions;
pub mod touch_frames;
pub mod intents;
pub mod media_intents;
pub mod kinds;
mod apps;
mod appgrid;
pub mod overview;
mod audiosvc;
pub mod assistive_touch;
pub mod store;
pub mod weather;
pub mod evkeys;
pub mod focus;
pub mod fsutil;
pub mod homeconf;
mod folders;
mod dock_stack;
mod homeedit;
mod jiggle;
mod quick_actions;
pub mod camera;
pub mod magnifier;
pub mod calendar;
pub mod contactextras;
pub mod contactstore;
pub mod books;
pub mod cache;
pub mod clipboard;
mod services_route;
mod look_up_popover;
pub mod chord;
pub mod clock;
pub mod control_center;
pub mod controls;
pub mod fuzzy;
pub mod edit;
pub mod editmenu;
pub mod gestures;
pub mod glyphs;
pub mod icons;
pub mod row_glyphs;
pub mod keyboard;
pub mod textinput;
pub mod compose_window;
pub mod kb_layouts;
pub mod kb_capslock;
pub mod kb_input;
pub mod input_methods;
pub mod kb_clicks;
pub mod alert_sound;
pub mod hwkeys;
pub mod backup;
pub mod backup_page;
pub mod timemachine;
pub mod md;
pub mod sha256;
pub mod share;
pub mod update;
pub mod pacman_update;
pub mod ota;
pub mod notify;
pub mod usernotif;
pub mod extras;
pub mod desktop_gate;
pub mod spotindex;
pub mod sharing;
pub mod forms;
pub mod users_page;
pub mod mouse_page;
pub mod sharing_page;
pub mod print_scan_page;
pub mod workspace;
pub mod services;
pub mod chrome_host;
pub mod dwell;
pub mod sticky_keys;
pub mod game_controllers_page;
mod now_playing_menu;
pub mod dnd;
pub mod photos;
pub mod music;
pub mod list;
pub mod low_battery;
pub mod crash_alert;
pub mod permission_sheet;
pub mod options;
pub mod pageedit;
pub mod pages;
mod login_items_page;
pub mod axmap;
pub mod power_menu;
#[cfg(feature = "desktop-wayland")]
mod compose_sheet;
mod recent_items;
#[cfg(feature = "desktop-wayland")]
mod print_sheet;
mod character_viewer;
mod keyboard_viewer;
pub mod session;
pub mod prefs;
pub mod markup;
pub mod screenshot;
/// Screen recording: the frames the shell draws, encoded to a movie (ffmpeg or the built-in AVI writer).
pub mod screenrec;
/// The screenshot toolbar's capture session: Shift-Cmd-5 and `screencapture -i -w` (see `capture.rs`).
pub mod capture;
pub mod screentime;
pub mod login;
pub mod passcode;
pub mod rotate;
pub mod settings;
pub mod settings_load;
// the settings App Intents catalogue; Search does not list it yet, so unused items are expected for now
#[allow(dead_code)]
pub mod settings_intents;
// action intents (Wi-Fi, wallpaper); not wired to Search yet
#[allow(dead_code)]
pub mod settings_actions;
pub mod settings_search;
pub mod screensaver;
pub mod storage;
pub mod storage_page;
pub mod gpu_page;
pub mod quicksearch;
pub mod quicksearch_math;
pub mod switcher;
pub mod touch_gestures;
pub mod textarea;
pub mod termtouch;
pub mod terminal;
pub mod textfield;
pub mod today;
pub mod video;
pub mod ui;
pub mod splitview;
pub mod stage_strip;
mod desk;
mod desk_occlusion;
pub mod window_fx;
pub mod wake;
pub mod desktop_network;
pub mod disk_image;
mod desktop_bar;
#[cfg(feature = "desktop-wayland")]
pub mod docpanel;
pub mod preview_panel;
mod desktop_metrics;
mod desktop_dock;
pub mod defaultapps;
pub mod volume_hud;
pub mod input_source_hud;
pub mod input_memory;
pub mod fcitx_bridge;
pub mod sound_page;
pub mod datetime_page;
pub mod diag;
pub mod diagnostics_page;
pub mod passwords_page;
pub mod internet_accounts_page;
pub mod timeset_page;
pub mod night_shift;
pub mod notify_page;
pub mod wallpaper_page;
pub mod wallpaper_schedule;
pub mod widgets;
pub mod widgetgallery;
pub mod wifi_page;

use crate::deildu::{self, Deildu};
use crate::files::{self, Files};
use crate::gfx::{rgba, squircle_mask, threads, Damage, Image, Surface, View};
use crate::layout::{Layout, Rect};
use crate::net::wifi_config::NetworkConfig;
use crate::pager::Pager;
use crate::scene::{self, dock_background, Chrome, HomeState, Scene, Tile};
use crate::status;
use crate::term::{self, Terminal};
use applib::AppLibrary;
use control_center::ControlCenter;
use homeconf::{Home, HomeConf};
use pageedit::PageEditor;
use widgetgallery::Gallery;
use photos::Photos;
use screenshot::Overlay;
use std::path::{Path, PathBuf};
use gestures::HomeGesture;
use std::sync::Arc;
use keyboard::{Keyboard, Out};
use options::Options;
use prefs::Prefs;
use settings::Settings;
use quicksearch::QuickSearch;
use switcher::Switcher;
use std::rc::Rc;
use today::{Action, Today};
use widgets::deildu::Played;
use ui::{centred, home_indicator, home_zone, Ui, Weight, BLACK, WHITE};
use crate::media::route::Route;
use crate::media::volume::Volume;
use volume_hud::VolumeHud;
use input_source_hud::SourceHud;
use wallpaper_page::Wallpaper;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
    /// Active physical panel mode, independent of app window geometry.
    pub display_mode: Option<(u32, u32, u32)>,
    /// the refresh rates the panel offers at its current size, fastest first (Settings > Displays > Refresh Rate)
    pub display_rates: Vec<f32>,
    pub brightness_supported: bool,
    pub battery_present: bool,
    pub desktop_network: Option<desktop_network::Snapshot>,
    pub hour: i32,
    pub min: i32,
    pub wday: i32,
    pub mday: i32,
    pub mon: i32,
    pub battery: u8,
    pub charging: bool,
    /// Wi-Fi connected; the Wi-Fi glyph and join/leave notices follow this alone
    pub online: bool,
    /// a wired Ethernet link has an address (shown as a LAN glyph)
    pub wired: bool,
    /// a wireless adapter exists (false on a desktop without one)
    pub wifi_adapter: bool,
    /// 0..=3 signal bars of the current network
    pub wifi_bars: u8,
    /// Wi-Fi row text, e.g. "WIFI" or "Off"
    pub wifi: String,
    pub ip: String,
    /// 0..=1
    pub brightness: f32,
    pub model: String,
    pub kernel: String,
    pub memory: String,
    pub wifi_on: bool,
    /// scan results, strongest first
    pub networks: Vec<Network>,
    pub current_ssid: Option<String>,
    /// Wi-Fi lost and being recovered automatically (bare::wifi_health)
    pub wifi_notice: Option<String>,
    /// saved networks in priority order (first is joined first)
    pub saved: Vec<SavedNetwork>,
    /// scanned networks that sign in with a username (802.1X)
    pub enterprise: Vec<String>,
    /// id of the chosen wallpaper (`wallpapers/<id>.raw`), "" for the default
    pub wallpaper: String,
    // -- General › About --
    /// the device's hostname
    pub name: String,
    /// Wi-Fi MAC, "AA:BB:..."
    pub mac: String,
    pub router: String,
    pub cpu: String,
    pub storage_total: u64,
    pub storage_free: u64,
    pub uptime_min: i64,
    pub serial: String,
    /// songs, videos, photos on the device
    pub media_counts: [u32; 3],
    pub timezone: String,
    /// Date & Time › Set Automatically is off
    pub time_manual: bool,
    /// the last network time sync, "Synced 2 min ago, offset +0.3 s"
    pub time_sync: String,
    /// battery samples, oldest first (see `battery`)
    pub battery_history: std::sync::Arc<Vec<battery::Sample>>,
    /// health rows from sysfs power_supply (capacity, cycles, condition)
    pub battery_health: Vec<(String, String)>,
    /// the CPU profile in effect, for Battery › Power Mode's footer
    pub power_profile: String,
    /// the Bluetooth radio: power, scan results, paired and connected devices
    pub bt: crate::bare::bluetooth::Status,
    /// the sign-in page of the joined network, while it has one (see `net::captive`)
    pub captive_portal: Option<String>,
}

/// A saved Wi-Fi network as Known Networks lists it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SavedNetwork {
    pub ssid: String,
    pub auto_join: bool,
    /// why it is not connected: "Incorrect Password", "No Internet Connection"; empty when fine
    pub note: String,
    /// the security type the network was saved with; None when the source does not say
    pub security: Option<crate::net::wifi_config::Security>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Network {
    pub ssid: String,
    /// 0..=3
    pub bars: u8,
    pub secure: bool,
    pub saved: bool,
}

/// True when `new` differs from `old` only in fields that change on their own clock and that no
/// always-visible surface draws (uptime, free storage, memory text, battery history, media counts,
/// sync text, CPU name, brightness).
pub fn ambient_only(old: &Info, new: &Info) -> bool {
    let mut n = new.clone();
    n.uptime_min = old.uptime_min;
    n.storage_free = old.storage_free;
    n.memory = old.memory.clone();
    n.battery_history = old.battery_history.clone();
    n.media_counts = old.media_counts;
    n.time_sync = old.time_sync.clone();
    n.cpu = old.cpu.clone();
    n.brightness = old.brightness;
    n == *old
}

pub use apps::{apps, AppDef, AppId, APPS};

/// How many of `APPS` sit in the dock (the rest fill the grid).
const IN_DOCK: usize = 2;

#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Brightness(f32),
    ScreenOff,
    /// explicit Sleep (menu): suspend the system after the display is off
    SleepSystem,
    ScreenOn,
    Wifi(WifiAction),
    /// a Bluetooth change: power, scan, pair, connect, forget
    Bluetooth(crate::bare::bluetooth::Cmd),
    /// Settings › Bluetooth: send the sound to the connected Bluetooth audio device (`Shell::use_bluetooth_output`)
    BluetoothOutput,
    /// media volume 0..=1 from the Control Center slider
    Volume(f32),
    /// a timer or alarm went off: sound it
    Alarm,
    /// arm the RTC to wake the device at this Unix time (0 = disarm)
    WakeAt(i64),
    /// the user picked a wallpaper: map `wallpapers/<id>.raw` ("" = the
    /// default wallpaper.raw) and `set_wallpaper`; the shell persists the id
    Wallpaper(String),
    /// the lock screen's own wallpaper: an id as above, "default" for
    /// wallpaper.raw, "" to follow the home screen again
    LockWallpaper(String),
    /// Dark Appearance Dims Wallpaper changed
    WallpaperDim(bool),
    /// the idle timeout changed (ms, 0 = never); the shell already applies it
    AutoLock(u32),
    /// Energy: minutes of display-off before the system sleeps (0 = never); the binary applies it live
    SystemSleepAfter(u32),
    /// quit with exit code 3 so the launcher starts the shell again
    Restart,
    /// power off, reboot or hand back to Android; the binary flushes first
    Power(power_menu::PowerAction),
    /// Rotation Lock toggled in Control Center: persist as `rotation_lock`
    RotationLock(bool),
    /// a key press asked for a click and Keyboard Clicks is on: the shell binary plays it
    KeyClick(kb_clicks::Click),
    #[cfg(feature = "desktop-wayland")]
    ActivateForeign(u32),
    #[cfg(feature="desktop-wayland")]
    ForeignWindowAction(u32,bool),
    #[cfg(feature="desktop-wayland")]
    ForeignWindowMaximize(u32,bool),
    #[cfg(feature="desktop-wayland")]
    DesktopShortcut(u16),
}

#[derive(Clone, Debug, PartialEq)]
pub enum WifiAction {
    Scan,
    Enable(bool),
    /// joins a saved network
    Select(String),
    Join { ssid: String, psk: Option<String> },
    /// removes a saved network
    Forget(String),
    /// joins a network typed in on the "Other…" sheet (hidden, WPA3, 802.1X)
    JoinOther(NetworkConfig),
    SetAutoJoin { ssid: String, on: bool },
    /// one place up or down the priority list
    Move { ssid: String, up: bool },
}

pub use bifrost_ui::Event;

/// A tap on the dark screen wakes it: down and up within this time and distance.
const TAP_WAKE_MS: f64 = 300.0;
const TAP_WAKE_PX: f32 = 40.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Screen {
    Lock,
    Home,
    App(AppId),
    Switcher,
}

#[derive(Clone, Copy, Debug)]
enum Anim {
    /// lock screen sliding away (to = 1) or back (to = 0)
    Unlock { from: f32, to: f32 },
    /// the app's frame zooms from `rect` to the full screen
    Open { app: AppId, rect: Rect },
    /// the app's frame zooms from `from` into its icon at `rect`
    Close { from: Rect, rect: Rect },
}


/// What the last frame showed; the next frame redraws only what differs.
/// `chrome` is the status-bar generation, `card` the launch card's pixel box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drawn {
    Nothing,
    /// `today` is the lock pager's offset and the Today View's generation
    Lock { lift: i32, chrome: u32, notes: u32, today: (i32, u32) },
    Home { state: HomeState, chrome: u32, card: Option<[i32; 4]>, badges: u32 },
    /// the Terminal with everything around its grid as `key` says (see `terminal_key`)
    Term { key: u64 },
    /// Settings' list page: everything but the scroll as `key` says, scrolled to `off` px
    Settings { key: u64, off: i32 },
    /// an installed app's frame; the surface still holds it, so only damage is copied
    Ext(u8),
    App,
}

pub struct Shell {
    pub w: i32,
    pub h: i32,
    ui: Ui,
    layout: Layout,
    /// the caller's mapping of the home still (stays mapped; never owned here)
    home_src: View<'static>,
    /// desktop: the home and lock stills scaled to cover the panel (see `rotate::fit_wallpaper`); the shell
    /// owns them and reads them through `rotate::still`. `fit_home` is freed once the GPU holds the texture and
    /// rebuilt only while the lock screen or Today needs it; `fit_lock` and `wpx.lock_dim` exist only while locked
    fit_home: Option<Image>,
    fit_lock: Option<Image>,
    /// the GPU presenter has the wallpaper texture; stop handing it the view
    wallpaper_uploaded: bool,
    /// The wallpaper under the menu bar is light: the bar draws dark text (`desktop_bar::wallpaper_is_light`).
    bar_on_light: bool,
    /// lock/Today buffers were just freed: the main loop hands the heap back once it idles
    heap_trim_due: bool,
    /// the packed stills offered by Settings › Wallpaper
    wallpapers: Rc<Vec<Wallpaper>>,
    /// lock wallpaper and dimmed copies
    wpx: wallpaper_page::Extra,
    dock_bg: Option<Image>,
    dock: Vec<Tile>,
    grid: Vec<Tile>,
    /// every app's masked icon at the standard size (apps() order), the source
    /// the tiles are resampled from under Display Zoom
    base_icons: Vec<Image>,
    zoom: f32,
    pager: Pager,
    /// persisted home-screen edits (widgets, page order, hidden pages)
    conf: HomeConf,
    /// the visible pages built from `conf`; `home_grid` is their tiles in cell order
    home: Home,
    home_grid: Vec<Tile>,
    home_widgets: Vec<scene::HomeWidget>,
    /// the App Library, the last home page
    library: AppLibrary,
    /// bumped whenever the widgets or the page set change
    home_gen: u32,
    /// the current touch started on the library page
    lib_touch: bool,
    /// a home touch that may become a long press
    hold: Option<homeedit::Hold>,
    jiggle: Option<jiggle::Jiggle>,
    /// grid apps taken off the home pages (kept in the App Library)
    removed: Vec<usize>,
    page_editor: Option<PageEditor>,
    /// folders on the home grid and the one open
    folders: folders::Folders,
    folder_view: Option<folders::View>,
    gallery: Option<Gallery>,
    /// the visible page the gallery adds widgets to
    gallery_page: usize,
    chrome: Chrome,
    info: Info,
    screen: Screen,
    screen_on: bool,
    anim: Option<(Anim, f64)>,
    /// lock-screen drag: (start y, current lift in px)
    lock_drag: Option<(f32, f32)>,
    /// where the lock-screen touch landed, and whether it turned horizontal
    lock_x0: f32,
    lock_axis: Option<bool>,
    /// the passcode keypad over the lock screen, after the unlock swipe
    lock_pad: Option<passcode::Lock>,
    /// the login window over the lock screen, the root helper behind it and the unlocked session's token
    login: Option<login::Login>,
    login_backend: Option<std::sync::Arc<dyn login::Backend>>,
    session_token: Option<String>,
    /// Screen Time: today's usage, the Time Limit cover, and per-second gates
    screentime: screentime::usage::Tracker,
    shield: Option<screentime::shield::Shield>,
    st_checked: i64,
    st_blocked_at: i64,
    /// the parsed screentime.ini, reloaded only when the file changes on disk
    st_cfg: Option<(screentime::config::Config, Option<std::time::SystemTime>)>,
    /// trims the caches while /data is nearly full
    storage_guard: storage::Guard,
    /// the Login Items page's extension model, re-read only when the installed apps change
    login_ext_cache: login_items_page::ExtensionsCache,
    /// slides the Today View over the lock screen (one page plus the leading one)
    lock_pager: Pager,
    /// the Today View on page -1 of the home screen and the lock screen
    today: Today,
    /// the current touch started on the Today page
    today_touch: bool,
    /// the Today tiles need a refresh (info or terminal output changed)
    today_stale: bool,
    /// wall-clock time, for the Today widgets
    wall: clock::Wall,
    /// go-home swipe from the indicator
    home_gesture: Option<HomeGesture>,
    /// the app frame that zooms during Open/Close and the go-home swipe
    transition: Option<Image>,
    /// start of a touch on the dark screen: (x, y, t)
    dark_tap: Option<(f32, f32, f64)>,
    cc: ControlCenter,
    /// the volume-key HUD; `hud_drawn` says the last frame had it
    hud: VolumeHud,
    hud_drawn: bool,
    /// the input source HUD; `input_hud_drawn` says the last frame had it
    input_hud: SourceHud,
    input_hud_drawn: bool,
    /// the source each app keeps (Settings > Keyboard > Keyboards, "Remember Input Source for Each App")
    input_memory: input_memory::InputMemory,
    /// Developer › Performance HUD; `perf_drawn` says the last frame had it
    perf: diag::PerfHud,
    perf_drawn: bool,
    switcher: Switcher,
    /// the search card pulled down over the home screen
    quicksearch: QuickSearch,
    /// the built-in compose method over the Search field and its candidate window
    compose_ime: compose_window::ComposeWindow,
    notify: notify::Notify,
    /// `UNUserNotificationCenter` state of the external apps (categories, pending and delivered requests)
    usernotif: usernotif::Registry,
    /// login items (`SMAppService`) and the menu-bar extras the apps published (`NativeStatusItem`)
    extras: extras::Registry,
    /// the share sheet an app asked for (`sharing`)
    app_share: sharing::State,
    app_workspace: workspace::State,
    /// System services apps call by name (`services/`).
    pub(crate) services: Vec<Box<dyn services::SystemService>>,
    /// Input events pass through the session event taps and the source state tables (`services::event_services`).
    input_taps: services::event_services::InputTaps,
    /// the drag and drop session between app windows, and the types each window registered (`dnd`)
    dnd: dnd::State,
    assistive_touch: assistive_touch::AssistiveTouch,
    /// an app hiding the status bar and home indicator
    fullscreen: bool,
    /// persisted Settings (`settings.ini`), owned here so one writer exists
    prefs: Prefs,
    /// Screen Zoom (Option+Cmd+8, =, -): the final-frame magnifier the GPU presenter draws (`accessibility::zoom`)
    screen_zoom: accessibility::zoom::ScreenZoom,
    /// the background update checker; set once the data directory is loaded
    ota: Option<ota::Ota>,
    /// last time (ms) the apps registry was stat'ed for outside changes
    disk_check_ms: f64,
    /// Display & Brightness › Auto-Lock, ms; 0 never sleeps
    idle_off_ms: f64,
    /// Lock Screen > Require Password: when the display went off with a grace, and the screen it returns to (see `display_off`)
    grace_from: Option<(f64, Screen)>,
    /// Settings > Screen Saver: idle activation and the saver on screen (see `screensaver`)
    saver: screensaver::ScreenSaver,
    /// the menu bar clock's options (Date & Time), see `options::Options::clock_style`
    clock_style: bifrost_ui::menu_extras::ClockStyle,
    lock_time: Option<Image>,
    lock_date: Option<Image>,
    clock: clock::Clock,
    settings: Option<Settings>,
    /// SF Mono for the terminal
    mono: &'static [u8],
    /// the Terminal's shell, alive in the background once opened
    term: term::Sessions,
    terminal_ui: terminal::TerminalUi,
    term_cmd: &'static [&'static str],
    /// the Deildu library, alive in the background once opened
    deildu: Option<Deildu>,
    deildu_dir: std::path::PathBuf,
    deildu_transport: Arc<dyn deildu::http::Transport>,
    /// the running installed apps by slot (see `apphost`)
    ext: Vec<Option<apphost::AppHost>>,
    /// Apps that were sent Quit and are not gone yet (reaped in `ext_tick`, killed at their deadline).
    dying: Vec<apphost::AppHost>,
    /// desktop session restoration (`session`): the layout file and the replay state
    session: session::Runtime,
    /// the installed app in the foreground, to send Suspend/Resume on change
    ext_fg: Option<u8>,
    store: Option<store::AppStore>,
    camera: Option<camera::Camera>,
    magnifier: Option<magnifier::Magnifier>,
    /// the Browser app, alive once opened
    browser: Option<browser::Browser>,
    /// the Photos library, alive in the background once opened
    photos: Option<Photos>,
    /// directories Photos scans; the first receives screenshots
    photo_dirs: Vec<PathBuf>,
    /// where settings and caches live (`set_data_dir`)
    data_dir: PathBuf,
    /// the screenshot animation over the screen
    shot: Option<Overlay>,
    /// the Markup editor opened from the screenshot thumbnail
    markup: Option<markup::Markup>,
    /// a picture being read and decoded on a worker for the Markup editor, with the path it came from
    markup_load: Option<(PathBuf, std::sync::mpsc::Receiver<Option<crate::gfx::Image>>)>,
    /// a touch that started on the screenshot thumbnail
    shot_tap: bool,
    /// The screenshot toolbar's capture session, open while it is on screen.
    capture: Option<capture::Session>,
    /// the screen recording in progress (the toolbar's Record buttons, `rec start`, `screencapture -v`)
    rec: Option<screenrec::Session>,
    /// the Options menu's choices, kept across capture sessions
    capture_prefs: capture::Prefs,
    /// the display's refresh rate and whether the GPU path is up (`set_display`): the recording's frame rate
    rec_display: (u32, bool),
    /// the loop's clock (ms) at its last pass, for recordings that finish between passes
    rec_clock: f64,
    /// finished recordings whose audio take is still on its way from the audio service
    rec_mux: Vec<screenrec::PendingMux>,
    /// audio muxes running on their threads
    rec_muxing: usize,
    /// the muxed movies come back here, so the loop opens them on the shell thread
    rec_ready: (std::sync::mpsc::Sender<std::path::PathBuf>, std::sync::mpsc::Receiver<std::path::PathBuf>),
    /// the Files browser, alive in the background once opened
    files: Option<Files>,
    /// the roots Files offers (tests point them at a temp dir)
    files_locations: Vec<files::fs::Location>,
    /// the audio service (bifrostd): Now Playing, transport and volume go through it
    audio: crate::svc::audio::Client,
    audio_synced: u32,
    svc_labels: Option<audiosvc::SvcLabels>,
    svc_touch: audiosvc::SvcTouch,
    /// samples are zeroed while this is on; off by default (the volume control sets the level)
    music_muted: bool,
    keyboard: Keyboard,
    /// desktop or touch tablet: decides whether the on-screen keyboard can appear (`FormFactor::allows_onscreen_keyboard`)
    form: bifrost_ui::form_factor::FormFactor,
    /// a physical keyboard is attached; on a tablet it keeps the on-screen keyboard down
    hw_keyboard: bool,
    /// the touchscreen fingers that are down, so the pointer stream does not report them twice (`touch_frames`)
    touch_contacts: touch_frames::Contacts,
    /// the focused text field of a Wayland client (the window server's input method), and the requests the keyboard
    /// made for it since the last `take_input_method_out`
    im: textinput::ImField,
    im_out: Vec<textinput::ImOut>,
    /// the device has a vibrator (`bare::vibrator::vibrator_available`)
    haptics: bool,
    /// a touch that started on the keyboard
    kb_touch: bool,
    /// the desktop window the on-screen keyboard lifted: (app, y before the lift, y after it)
    desk_kb_lift: Option<(AppId, i32, i32)>,
    /// touch on the terminal grid: scroll, tap, long-press selection, menu
    term_touch: termtouch::TermTouch,
    last_input: f64,
    /// when Settings last reported a content change from its tick (loads landing, timers): with `last_input`, the desk
    /// keeps a focused Settings window's cached pixels until one of them is newer than the cache
    settings_changed: f64,
    /// where a bottom-left corner swipe started (General › Gestures)
    corner_start: Option<(f32, f32)>,
    /// the slide-to-power-off sheet (Power held for a second)
    power_menu: Option<power_menu::PowerMenu>,
    /// the compose post sheet opened from the share menu
    #[cfg(feature = "desktop-wayland")]
    compose: Option<compose_sheet::ComposeOverlay>,
    /// File > Print…: the print sheet over the captured desktop (see `print_sheet.rs`)
    #[cfg(feature = "desktop-wayland")]
    print_sheet: Option<print_sheet::PrintSheet>,
    /// Edit > Emoji & Symbols: the character viewer panel (see `character_viewer.rs`)
    char_viewer: Option<character_viewer::CharacterViewer>,
    /// Option+Cmd+F5: the Accessibility Shortcut panel with a checkbox per chosen feature (see `accessibility::shortcut_panel`)
    a11y_panel: Option<accessibility::shortcut_panel::ShortcutPanel>,
    /// Input Sources > Show Keyboard Viewer: the keyboard board panel (see `keyboard_viewer.rs`)
    keyboard_viewer: Option<keyboard_viewer::KeyboardViewer>,
    /// the share sheet, the nearby service and the incoming-file prompt
    share: Option<share::Sheet>,
    share_svc: Option<crate::net::share::Service>,
    share_prompt: Option<share::prompt::Prompt>,
    look_up: Option<look_up_popover::Popover>,
    /// transfer rows the progress card showed at the last poll
    share_rows: usize,
    /// the "Low Battery" alert on screen
    low_alert: Option<low_battery::Alert>,
    /// the Allow / Don't Allow sheet for an app's privacy request (see `permission_sheet.rs`)
    permission_sheet: Option<permission_sheet::Sheet>,
    /// the "quit unexpectedly" alert for an app that failed (see `crash_alert.rs`)
    crash_alert: Option<crash_alert::Alert>,
    low_monitor: low_battery::Monitor,
    /// the announcement banner: the text apps post with `announce`, shown at the top of the screen for a few seconds
    announce: accessibility::banner::Banner,
    /// Guided Access: the session an app holds (`accessibility::guided`).
    guided: accessibility::guided::Guided,
    /// the speech engine that speaks the announcements while the screen reader is on (`accessibility::speech`)
    speech: accessibility::speech::Speaker,
    /// the screen reader's cursor over the front Settings page (`accessibility::reader`)
    reader: accessibility::reader::Reader,
    battery_checked: f64,
    dirty: bool,
    perf_only: bool,
    /// Overview, the Apps grid and the desktops (see `overview.rs`)
    overview: overview::Overview,
    desktop_menu:Option<desktop_bar::Menu>,
    /// Default-app registry and its Settings view, rebuilt when the set of apps changes (keyed by the app count).
    defaults:Option<(usize,bifrost_ui::launch::Registry,Vec<defaultapps::CategoryView>)>,
    /// The highlighted row of the open bar menu (keyboard or pointer), with the menu it belongs to.
    #[cfg(feature = "desktop-wayland")]
    desktop_menu_row:Option<(desktop_bar::Menu,usize)>,
    /// Recent Items: the apps opened last, listed in the leading system menu (`recent_items`).
    recent_list:bifrost_ui::shared_list::SharedList,
    /// Type-to-select state of the open bar menu (`NativeMenu` type-ahead): the kit model keeps the typed prefix.
    #[cfg(feature = "desktop-wayland")]
    desktop_menu_typed:Option<bifrost_ui::menu::Menu>,
    /// Width of the open bar menu, keyed by a hash of its rows and the scale (measuring every title on each pointer move was half the hover cost).
    #[cfg(feature = "desktop-wayland")]
    desktop_menu_width:std::cell::Cell<Option<(u64,usize)>>,
    /// the blurred menu-bar backdrop, rebuilt only when its size or wallpaper changes
    bar_backdrop:std::cell::RefCell<Option<((usize,usize,usize),Image)>>,
    /// Rendered strings of the desktop chrome (menu bar, menus, window titles, dock label), drawn every frame.
    chrome_text:std::cell::RefCell<bifrost_render::cache::TextCache>,
    /// Blurred menu-bar label shadows, keyed by (text, size, weight, blur radius): blurred once, not every frame.
    bar_shadows:std::cell::RefCell<std::collections::HashMap<(String,u32,std::mem::Discriminant<Weight>,usize),Image>>,
    /// what the last desktop frame drew outside the transparent content layer (GPU presenter only):
    /// the next desktop frame damages these and its own instead of the whole frame
    desk_regions:Option<Vec<(i32,i32,i32,i32)>>,
    /// set while a partial GPU desktop frame draws: the only rectangles the wallpaper layer clears
    desk_clear:Option<Vec<(i32,i32,i32,i32)>>,
    /// A full desktop frame has been drawn since the last frame of any other screen (the HUD-only fast path needs it).
    desk_drawn:bool,
    desktop_context:Option<desktop_bar::ContextMenu>,
    /// the open or save panel an app asked for (`docpanel`), shown as `desktop_menu = Document`
    #[cfg(feature = "desktop-wayland")]
    doc_panel:Option<docpanel::Panel>,
    /// the document broker: `None` until first asked for, then `Some(None)` when there is no installed account
    #[cfg(feature = "desktop-wayland")]
    doc_broker:Option<Option<docpanel::Broker>>,
    /// the floating preview panel (`preview_panel`), shown by an app's `ToShell::Preview` or the `quick-look` service
    quick_look:preview_panel::Panel,
    desktop_metrics: desktop_metrics::Snapshot,
    desktop_metrics_at:f64,
    metrics_sampler: desktop_metrics::Sampler,
    metrics_running: bool,
    /// the second app of a Split View / Slide Over, its menu and the pane scratch pixels
    /// desktop mode: floating windows and a dock (None on the bifrost)
    desk: Option<desk::Desk>,
    /// opens once the first desktop frame is on screen: login items and restored windows start after it
    desktop_gate: desktop_gate::DesktopGate,
    multi: Option<splitview::Multi>,
    multi_menu: Option<splitview::Menu>,
    multi_buf: Vec<u32>,
    /// desktop: the Today frost and the dimmed home still are built on demand (about 14 MiB each at 2560x1440)
    today_frost_stale: bool,
    // -- damage tracking (see `draw`) --
    drawn: Drawn,
    /// bumped whenever the status bar's images or glyphs change
    chrome_gen: u32,
    damage: Damage,
    rows: Vec<(usize, usize)>,
    /// `apps()` indices the dock shows: the fixed ones, then the recent apps
    dock_apps: Vec<usize>,
    /// the App Library icon at the end of the dock
    lib_dock: Option<Image>,
    /// where a touch began on it
    lib_down: Option<(f32, f32)>,
    /// `apps()` indices, most recently opened first
    recent_apps: Vec<usize>,
    /// orientation, the turn in progress and the letterbox (see `rotate`)
    rot: rotate::Rotate,
}

impl Shell {
    /// `mono` is the terminal font (SFUIMono.ttf); `icon` loads an icon by asset name at the requested size.
    pub fn new(w: i32, h: i32, font: crate::font::Font<'static>, mono: &'static [u8], wallpaper: View<'static>, icon: impl Fn(&str, usize) -> Option<Image>, info: Info) -> Self {
        let layout = Layout::new(w, h, IN_DOCK + 1).with_dock_sep(IN_DOCK);
        let home_src = wallpaper;
        let fit_home = rotate::fit_wallpaper(home_src, w, h);
        let wallpaper = rotate::still(home_src, &fit_home);
        let ui = Ui::new(font, layout.s);
        let size = layout.icon as usize;
        let mask = squircle_mask(size);
        // installed apps' icons are PNGs: they decode in parallel, one thread per app, before any tile is built
        let all = apps();
        let decoded: Vec<Option<Image>> = apps::decode_icons(all, size);
        let tile = |a: &AppDef, decoded: Option<Image>| {
            // the Terminal's own artwork outranks a stale packaged `icons/*.raw`
            let own = (a.id == AppId::Terminal).then(|| a.icon_image.map(|f| f(size))).flatten();
            let mut img = decoded.or(own).or_else(|| icon(a.icon, size)).or_else(|| a.icon_image.map(|f| f(size))).unwrap_or_else(|| scene::placeholder_icon(size));
            img.apply_mask(&mask);
            Tile { icon: img, label: ui.styled(&ui.fit(a.name, 12.0, layout.label_w as f32), 12.0, Weight::Regular, WHITE, true) }
        };
        let mut decoded = decoded.into_iter();
        let mut tiles: Vec<Tile> = all.iter().map(|a| tile(a, decoded.next().flatten())).collect();
        let grid: Vec<Tile> = tiles.split_off(IN_DOCK);
        let dock: Vec<Tile> = tiles;
        let base_icons: Vec<Image> = dock.iter().chain(&grid).map(|t| t.icon.clone()).collect();
        let library = AppLibrary::new(&ui, &layout, &base_icons);
        let pager = Pager::new(Layout::pages(grid.len()) + 1, w as f32);
        let keyboard = Keyboard::new(w as f32, layout.s, true);
        let clock = clock::Clock::new(&ui, w, h);
        let quicksearch = QuickSearch::new(&ui, w as f32, layout.s);
        let mut today = Today::new(&ui, w, h, mono);
        today.set_wallpaper(wallpaper);
        let mut pager = pager;
        pager.set_leading(true);
        let mut lock_pager = Pager::new(1, w as f32);
        lock_pager.set_leading(true);
        let mut sh = Shell {
            assistive_touch: assistive_touch::AssistiveTouch::new(&ui, w, h),
            w,
            h,
            dock_bg: dock_background(wallpaper, &layout),
            ui,
            layout,
            home_src,
            fit_home,
            fit_lock: None,
            wallpaper_uploaded: false,
            bar_on_light: false,
            heap_trim_due: false,
            wallpapers: Rc::new(Vec::new()),
            wpx: Default::default(),
            dock,
            grid,
            base_icons,
            zoom: 1.0,
            pager,
            conf: HomeConf::default(),
            home: Home::default(),
            home_grid: Vec::new(),
            home_widgets: Vec::new(),
            library,
            home_gen: 0,
            lib_touch: false,
            hold: None,
            jiggle: None,
            removed: Vec::new(),
            page_editor: None,
            folders: folders::Folders::default(),
            folder_view: None,
            gallery: None,
            gallery_page: 0,
            chrome: Chrome::default(),
            info: Info::default(),
            // An unlock at boot is the login: the desktop starts, no lock screen (task 10).
            screen: if login::take_autologin().is_some() { Screen::Home } else { Screen::Lock },
            screen_on: true,
            anim: None,
            lock_drag: None,
            lock_x0: 0.0,
            lock_axis: None,
            lock_pad: None,
            login: None,
            login_backend: login::Privd::from_env().map(|p| std::sync::Arc::new(p) as std::sync::Arc<dyn login::Backend>),
            session_token: None,
            screentime: screentime::usage::Tracker::new(deildu::data_dir().parent().map(|p| p.join("screentime")).unwrap_or_default()),
            shield: None,
            st_checked: 0,
            st_blocked_at: 0,
            st_cfg: None,
            storage_guard: storage::Guard::default(),
            login_ext_cache: login_items_page::ExtensionsCache::default(),
            lock_pager,
            today,
            today_touch: false,
            today_stale: true,
            wall: clock::Wall::default(),
            home_gesture: None,
            transition: None,
            dark_tap: None,
            cc: ControlCenter::default(),
            hud: VolumeHud::default(),
            hud_drawn: false,
            input_hud: SourceHud::default(),
            input_hud_drawn: false,
            input_memory: input_memory::InputMemory::default(),
            perf: diag::PerfHud::default(),
            perf_drawn: false,
            switcher: Switcher::default(),
            quicksearch,
            compose_ime: compose_window::ComposeWindow::default(),
            notify: notify::Notify::default(),
            usernotif: usernotif::Registry::default(),
            extras: extras::Registry::default(),
            app_share: sharing::State::default(),
            app_workspace: workspace::State::default(),
            services: services::all(),
            input_taps: Default::default(),
            dnd: dnd::State::default(),
            fullscreen: false,
            prefs: Prefs::memory(),
            screen_zoom: Default::default(),
            ota: None,
            disk_check_ms: 0.0,
            idle_off_ms: Options::default().auto_lock_ms as f64,
            grace_from: None,
            saver: screensaver::ScreenSaver::new(),
            clock_style: bifrost_ui::menu_extras::ClockStyle::default(),
            lock_time: None,
            lock_date: None,
            clock,
            settings: None,
            mono,
            term: term::Sessions::default(),
            terminal_ui: terminal::TerminalUi::default(),
            term_cmd: if std::path::Path::new("/system/bin/sh").exists() {
                term::SHELL
            } else if std::path::Path::new("/bin/bash").exists() {
                &["/bin/bash", "-l"]
            } else {
                &["/bin/sh"]
            },
            deildu: None,
            deildu_dir: deildu::data_dir(),
            deildu_transport: Arc::new(deildu::http::Tls::new()),
            ext: Vec::new(),
            dying: Vec::new(),
            session: session::Runtime::default(),
            ext_fg: None,
            store: None,
            camera: None,
            magnifier: None,
            browser: None,
            photos: None,
            photo_dirs: photos::library::dirs(),
            data_dir: deildu::data_dir().parent().map(Path::to_path_buf).unwrap_or_default(),
            shot: None,
            markup: None,
            markup_load: None,
            shot_tap: false,
            capture: None,
            rec: None,
            capture_prefs: capture::Prefs::default(),
            rec_display: (screenrec::FPS, false),
            rec_clock: 0.0,
            rec_mux: Vec::new(),
            rec_muxing: 0,
            rec_ready: std::sync::mpsc::channel(),
            files: None,
            files_locations: files::fs::locations(),
            audio: crate::svc::audio::Client::for_root(&apphost::root()),
            audio_synced: 0,
            svc_labels: None,
            svc_touch: Default::default(),
            music_muted: false,
            keyboard,
            form: bifrost_ui::form_factor::FormFactor::current(),
            hw_keyboard: false,
            touch_contacts: touch_frames::Contacts::default(),
            im: Default::default(),
            im_out: Vec::new(),
            haptics: false,
            kb_touch: false,
            desk_kb_lift: None,
            term_touch: termtouch::TermTouch::default(),
            last_input: 0.0,
            settings_changed: 0.0,
            corner_start: None,
            power_menu: None,
            #[cfg(feature = "desktop-wayland")]
            compose: None,
            #[cfg(feature = "desktop-wayland")]
            print_sheet: None,
            char_viewer: None,
            a11y_panel: None,
            keyboard_viewer: None,
            share: None,
            share_svc: None,
            share_prompt: None,
            look_up: None,
            share_rows: 0,
            low_alert: None,
            permission_sheet: None,
            crash_alert: None,
            low_monitor: low_battery::Monitor::default(),
            announce: Default::default(),
            guided: Default::default(),
            speech: accessibility::speech::Speaker::detect(),
            reader: Default::default(),
            battery_checked: f64::NEG_INFINITY,
            dirty: true,
            perf_only: false,
            overview:Default::default(),
            desktop_menu:None,defaults:None,recent_list:recent_items::new(),desktop_context:None,
            #[cfg(feature = "desktop-wayland")]
            desktop_menu_row:None,
            #[cfg(feature = "desktop-wayland")]
            desktop_menu_typed:None,
            #[cfg(feature = "desktop-wayland")]
            desktop_menu_width:std::cell::Cell::new(None),
            #[cfg(feature = "desktop-wayland")]
            doc_panel:None,
            #[cfg(feature = "desktop-wayland")]
            doc_broker:None,
            quick_look:Default::default(),
            bar_backdrop:std::cell::RefCell::new(None),chrome_text:std::cell::RefCell::new(bifrost_render::cache::TextCache::for_screen(w.max(0) as usize, h.max(0) as usize)),bar_shadows:Default::default(),desk_regions:None,desk_clear:None,desk_drawn:false,
            desktop_metrics:Default::default(),desktop_metrics_at:0.,metrics_sampler:Default::default(),metrics_running:false,
            desk: Some(desk::Desk::new()),
            desktop_gate: Default::default(),
            multi: None,
            multi_menu: None,
            multi_buf: Vec::new(),
            today_frost_stale: true,
            drawn: Drawn::Nothing,
            chrome_gen: 0,
            damage: Damage::new(w as usize, h as usize),
            rows: Vec::new(),
            dock_apps: (0..IN_DOCK).collect(),
            lib_dock: None,
            lib_down: None,
            recent_apps: Vec::new(),
            rot: rotate::Rotate::new(),
        };
        sh.rebuild_home();
        if let Some(d)=&mut sh.desk {d.dock_preferences=desktop_dock::Preferences::load(&sh.data_dir);d.dock_visible=!d.dock_preferences.autohide;}
        sh.desktop_export_apps();
        sh.rebuild_lib_dock();
        sh.set_info(info);
        sh
    }

    /// True while a built-in app is on screen (it may need live status).
    pub fn in_app(&self) -> bool {
        matches!(self.screen, Screen::App(_))
    }

    pub fn screen_on(&self) -> bool {
        self.screen_on
    }

    /// Audio is playing or a download is running: the device must not suspend.
    pub fn keeps_awake(&self) -> bool {
        self.audio.snapshot().playing
            || self.files.as_ref().is_some_and(Files::is_playing)
            || self.deildu.as_ref().is_some_and(Deildu::active)
    }

    /// Asks for one frame with no change of its own (the presenter owes glass that must be presented).
    pub fn request_frame(&mut self) {
        self.dirty = true;
    }

    pub fn needs_frame(&self) -> bool {
        let files = self.screen == Screen::App(AppId::Files) && self.files.as_ref().is_some_and(Files::animating);
        self.screen_on && (self.dirty || bifrost_render::appearance::fading() || self.anim.is_some() || self.turning() || self.pager.animating() || self.cc.animating() || self.switcher.animating() || self.keyboard.animating() || self.quicksearch.animating() || self.notify.animating() || self.lock_pager.animating() || self.today.animating() || self.home_animating() || files)
    }

    /// Why `needs_frame` holds, for the frame trace: the first condition that does ("-" when none).
    pub fn frame_reason(&self) -> String {
        let files = self.screen == Screen::App(AppId::Files) && self.files.as_ref().is_some_and(Files::animating);
        let reasons = [
            (!self.screen_on, "screen-off"),
            (self.dirty, "dirty"),
            (self.anim.is_some(), "anim"),
            (self.turning(), "turn"),
            (self.pager.animating(), "pager"),
            (self.cc.animating(), "cc"),
            (self.switcher.animating(), "switcher"),
            (self.keyboard.animating(), "keyboard"),
            (self.quicksearch.animating(), "quicksearch"),
            (self.notify.animating(), "notify"),
            (self.lock_pager.animating(), "lock-pager"),
            (self.today.animating(), "today"),
            (self.home_animating(), "home"),
            (files, "files"),
        ];
        let screen = format!("{:?}", self.screen).to_lowercase();
        let why: Vec<&str> = reasons.iter().filter(|r| r.0).map(|r| r.1).collect();
        format!("{screen}:{}", if why.is_empty() { "-".to_string() } else { why.join("+") })
    }

    /// Loads persisted app state (the Clock's alarms, the Today View's
    /// widgets) from `dir` and saves there from now on. Returns the RTC
    /// wake effect for the next alarm.
    pub fn set_data_dir(&mut self, dir: &str, t: f64) -> Vec<Effect> {
        self.quicksearch.load(format!("{dir}/quicksearch.ini"));
        self.notify_load(&format!("{dir}/notifications.ini"));
        self.today.load(dir);
        self.today_stale = true;
        clipboard::attach(format!("{dir}/{}", clipboard::FILE));
        self.data_dir = PathBuf::from(dir);
        self.st_load();
        self.prefs = Prefs::load(format!("{dir}/settings.ini"));
        // the Setup Assistant's choices reach settings.ini once, before the look is read below
        if let Err(e) = crate::setup::apply_pending(Path::new(dir), &mut self.prefs) {
            eprintln!("springboard: setup choices: {e}");
        }
        if let Some(v) = self.prefs.get_parsed::<f32>(settings::LIQUID_GLASS_PREF) { bifrost_render::glass::set_clarity(v); }
        // start in the saved scheme with no fade
        {
            let mode = bifrost_render::appearance::Mode::parse(self.prefs.get(settings::APPEARANCE_PREF).unwrap_or("dark"));
            bifrost_render::appearance::publish_mode(mode);
            bifrost_render::appearance::set_accent(bifrost_render::appearance::Accent::parse(self.prefs.get(settings::ACCENT_PREF).unwrap_or("blue")));
            let (unix, offset) = self.wall_now(0.0);
            bifrost_render::appearance::jump_to(mode.is_light(((unix + offset as i64).rem_euclid(86_400) / 3600) as u32));
        }
        services::text_services::load(&self.prefs);
        self.conf = HomeConf::load(&self.prefs);
        self.folders = folders::Folders::load(&self.prefs);
        self.removed = jiggle::load_removed(&self.prefs);
        self.extras = extras::Registry::from_pref(self.prefs.get(extras::PREF_KEY));
        self.rebuild_home();
        self.terminal_ui.load(&self.prefs);
        self.terminal_follow_scheme();
        self.ui.symbols = crate::symbols::Symbols::load(dir);
        self.apply_options();
        self.ota_enable();
        // the Clock app's files: the shell service shares them with the package
        let data = PathBuf::from(dir).join("appdata").join(clock::APP_ID);
        apphost::migrate_legacy(Path::new(dir), clock::APP_ID, &data);
        let _ = std::fs::create_dir_all(&data);
        // first run: the Setup Assistant opens over the desktop until its state file says the run finished
        if crate::setup::needs_window(Path::new(dir)) {
            if let Some(slot) = apphost::slot_of(crate::setup::APP_ID) {
                self.open(AppId::External(slot as u8), t);
            }
        }
        let mut fx = self.clock.load(&data.join("clock.ini").to_string_lossy(), t);
        fx.push(Effect::AutoLock(self.idle_off_ms as u32));
        fx
    }

    /// The options as Settings last saved them.
    fn options(&self) -> Options {
        self.settings.as_ref().map_or_else(|| Options::load(&self.prefs), |st| st.options().clone())
    }

    /// The alert tone (an index into `options::ALERT_SOUNDS`) for a timer or alarm that just fired: the Settings
    /// Alert Sound choice, or `None` while the output is muted (the clock is muted at zero volume too).
    pub fn alert_tone(&self) -> Option<usize> {
        (self.cc.volume > 0.0).then(|| self.options().alert_sound)
    }

    /// Tells the shell whether the device has a vibrator; Settings shows
    /// Keyboard Haptics only then. Host default is false.
    pub fn set_haptics_available(&mut self, on: bool) {
        self.haptics = on;
        if let Some(st) = &mut self.settings {
            st.set_haptics_available(on);
        }
    }

    /// Key clicks queued since the last call, kept only while Keyboard Clicks is on.
    fn gated_clicks(&mut self) -> Vec<kb_clicks::Click> {
        let clicks = self.keyboard.take_clicks();
        if self.options().keyboard_clicks { clicks } else { Vec::new() }
    }

    /// Low Power Mode as the user left it; the binary caps the backlight and
    /// the frame rate while it is on.
    pub fn low_power(&self) -> bool {
        self.options().low_power
    }

    /// Battery › Power Mode as the user left it; the binary maps it to the CPU profile.
    pub fn power_pref(&self) -> crate::bare::cpuprofile::Pref {
        self.options().power_profile
    }

    fn set_low_power(&mut self, on: bool) {
        match &mut self.settings {
            Some(st) => st.set_low_power(&self.ui, &mut self.prefs, on),
            None => {
                let mut o = Options::load(&self.prefs);
                o.low_power = on;
                o.save(&mut self.prefs);
            }
        }
        self.apply_options();
    }

    /// Writes out everything that is only saved lazily; called before the
    /// bifrost powers off, reboots or hands back to Android.
    pub fn flush(&mut self) {
        let o = self.options();
        o.save(&mut self.prefs);
        if let Some(b) = &mut self.browser {
            b.flush();
        }
    }

    /// Closes the sheets that draw their own full frame (Control Centre,
    /// Search, Notification Centre, the volume HUD) so a modal can show.
    fn clear_sheets(&mut self) {
        self.cc.hide();
        self.hud.hide();
        self.quicksearch.hide();
        self.close_home_overlays();
        self.notify_sleep();
    }

    /// Power held for a second: shows the slide-to-power-off sheet.
    pub fn open_power_menu(&mut self, t: f64) {
        if !self.screen_on || self.power_menu.is_some() {
            return;
        }
        self.last_input = t;
        self.clear_sheets();
        self.power_menu = Some(power_menu::PowerMenu::new(&self.ui));
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// Looks at the battery about once a second: raises or withdraws the
    /// "Low Battery" alert and asks for a clean shutdown at the bottom.
    fn low_battery_tick(&mut self, t: f64) -> Vec<Effect> {
        if t - self.battery_checked < low_battery::CHECK_MS {
            return vec![];
        }
        self.battery_checked = t;
        let (percent, charging) = (self.info.battery, self.info.charging);
        if self.low_alert.as_ref().is_some_and(|a| !low_battery::relevant(a.level, percent, charging)) {
            self.low_alert = None;
            self.dirty = true;
            self.drawn = Drawn::Nothing;
        }
        let low_power = self.low_power();
        match self.low_monitor.update(percent, charging, low_power) {
            low_battery::Verdict::Show(level) => {
                self.clear_sheets();
                self.low_alert = Some(low_battery::Alert::new(&self.ui, level, percent, low_power));
                self.dirty = true;
                self.drawn = Drawn::Nothing;
                vec![]
            }
            low_battery::Verdict::Shutdown => vec![Effect::Power(power_menu::PowerAction::Off)],
            low_battery::Verdict::Nothing => vec![],
        }
    }

    /// Shows the next app privacy request as an Allow / Don't Allow sheet when none is on screen.
    fn permission_tick(&mut self, t: f64) {
        if self.permission_sheet.is_some() {
            return;
        }
        let Some(prompt) = services::privacy_permissions::take_prompt() else { return };
        self.permission_sheet = permission_sheet::Sheet::new(&self.ui, prompt, t.max(0.0) as u64);
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// Whether the compose post sheet is up. The share menu opens it, and that menu is the desktop's.
    #[cfg(feature = "desktop-wayland")]
    fn compose_showing(&self) -> bool {
        self.compose.is_some()
    }
    #[cfg(not(feature = "desktop-wayland"))]
    fn compose_showing(&self) -> bool {
        false
    }
    /// Whether the print sheet is up. File > Print is the desktop's menu.
    #[cfg(feature = "desktop-wayland")]
    fn print_showing(&self) -> bool {
        self.print_sheet.is_some()
    }
    #[cfg(not(feature = "desktop-wayland"))]
    fn print_showing(&self) -> bool {
        false
    }

    /// Touches for the power sheet and the low-battery alert, which sit on
    /// top of everything: None when neither is up (or for a non-touch event).
    fn modal_event(&mut self, ev: Event) -> Option<Vec<Effect>> {
        let (x, y) = match ev {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            _ => return None,
        };
        if let Some(fx) = self.share_modal(ev) {
            return Some(fx);
        }
        if self.notify.ask.is_some() {
            // the first-time notification alert takes every touch until the person answers it
            return self.usernotif_ask_event(ev);
        }
        #[cfg(feature = "desktop-wayland")]
        if let Some(sheet) = &mut self.compose {
            let outcome = sheet.event(&self.ui, self.w, self.h, ev);
            self.drawn = Drawn::Nothing;
            return Some(match outcome {
                compose_sheet::Outcome::Finish(send) => {
                    self.compose_done(send);
                    vec![]
                }
                compose_sheet::Outcome::Idle | compose_sheet::Outcome::Redraw => vec![],
            });
        }
        if let Some(menu) = &mut self.power_menu {
            let outcome = menu.event(&self.ui, self.w, self.h, ev);
            self.drawn = Drawn::Nothing;
            return Some(match outcome {
                power_menu::Outcome::Pick(action) => {
                    self.power_menu = None;
                    vec![Effect::Power(action)]
                }
                power_menu::Outcome::Dismiss => {
                    self.power_menu = None;
                    vec![]
                }
                power_menu::Outcome::Idle | power_menu::Outcome::Redraw => vec![],
            });
        }
        if let Some(fx) = self.keyboard_viewer_touch(ev) {
            return Some(fx);
        }
        if self.char_viewer.is_some() {
            // the character viewer takes every touch until Done
            return Some(self.char_viewer_event(ev));
        }
        if self.a11y_panel.is_some() {
            // the Accessibility Shortcut panel takes every touch until Done
            return Some(self.a11y_panel_event(ev));
        }
        #[cfg(feature = "desktop-wayland")]
        if self.print_sheet.is_some() {
            // the print sheet takes every touch until Cancel or Print
            if let Event::Up(x, y) = ev {
                self.print_sheet_touch(x, y);
            }
            return Some(vec![]);
        }
        if self.crash_alert.is_some() {
            // the crash alert takes every touch until one of its buttons answers it
            if matches!(ev, Event::Up(..)) {
                self.crash_alert_touch(x, y);
            }
            self.drawn = Drawn::Nothing;
            self.dirty = true;
            return Some(vec![]);
        }
        if self.permission_sheet.is_some() {
            // the sheet takes every touch until the person answers it
            if matches!(ev, Event::Up(..)) {
                let hit = self.permission_sheet.as_ref().and_then(|s| s.hit(&self.ui, self.w, self.h, x, y));
                if let Some(button) = hit {
                    if let Some(sheet) = self.permission_sheet.take() {
                        // the question has no expiry, so the clock does not matter to the answer
                        let _ = sheet.finish(button, 0);
                    }
                }
            }
            self.drawn = Drawn::Nothing;
            self.dirty = true;
            return Some(vec![]);
        }
        let alert = self.low_alert.as_ref()?;
        if matches!(ev, Event::Up(..)) {
            match alert.hit(&self.ui, self.w, self.h, x, y) {
                Some(low_battery::Button::LowPowerMode) => {
                    self.low_alert = None;
                    self.set_low_power(true);
                }
                Some(low_battery::Button::Dismiss) => self.low_alert = None,
                None => {}
            }
            self.drawn = Drawn::Nothing;
        }
        Some(vec![])
    }

    /// Applies the persisted options to the shell: idle timeout, clock
    /// style, text size and weight, keyboard auto-capitalisation.
    /// Status-bar glyphs that follow settings: percent text, yellow battery, moon.
    fn sync_status_flags(&mut self, o: &Options) {
        let c = &mut self.chrome;
        if (c.hide_percent, c.low_power, c.dnd) != (!o.battery_percent, o.low_power, o.dnd) {
            let power_changed = c.low_power != o.low_power;
            (c.hide_percent, c.low_power, c.dnd) = (!o.battery_percent, o.low_power, o.dnd);
            if power_changed {
                // `ProcessInfo.isLowPowerModeEnabled` for apps (bifrost_app::ProcessInfo reads this file)
                let _ = std::fs::write("/run/bifrost/power-state", if o.low_power { "low_power=1\n" } else { "low_power=0\n" });
            }
            self.chrome_gen += 1;
            self.dirty = true;
        }
    }

    fn apply_options(&mut self) {
        let perf = self.prefs.get(settings::PERF_HUD_PREF) == Some("1");
        if perf != self.perf.enabled {
            self.perf.enabled = perf;
            self.dirty = true;
        }
        self.rebuild_dock(false);
        let o = self.options();
        self.notify.set_dnd(o.dnd);
        self.sync_status_flags(&o);
        if self.assistive_touch.set_enabled(o.assistive_touch) {
            self.dirty = true;
            self.drawn = Drawn::Nothing;
        }
        self.idle_off_ms = low_battery::auto_lock_ms(o.auto_lock_ms, o.low_power) as f64;
        self.saver.configure(o.screensaver_module, o.screensaver_minutes);
        if self.clock_style != o.clock_style() {
            self.clock_style = o.clock_style();
            self.chrome.time = None;
            let info = std::mem::take(&mut self.info);
            self.set_info(info);
        }
        let a = self.settings.as_ref().map_or_else(|| accessibility::AccessibilitySettings::load(&self.prefs), |st| st.accessibility().clone());
        a.publish();
        let zoom = accessibility::display_zoom();
        // desktop: Settings > Display changed the interface scale, so the home layout follows
        if crate::layout::adaptive() && (crate::layout::base_scale(self.w, self.h) - self.layout.s).abs() > f32::EPSILON {
            self.apply_zoom(zoom);
        }
        let s = self.layout.s * zoom;
        if (s - self.ui.s).abs() > f32::EPSILON {
            self.ui.s = s;
            self.keyboard.set_scale(s);
            if let Some(st) = &mut self.settings {
                st.relayout(&self.ui);
            }
            self.dirty = true;
        }
        if (zoom - self.zoom).abs() > f32::EPSILON {
            self.apply_zoom(zoom);
        }
        if o.text_scale != self.ui.text_scale || o.bold_text != self.ui.bold {
            self.set_text_scale(o.text_scale, o.bold_text);
        }
        self.keyboard.set_kb_prefs(kb_capslock::KbPrefs { enable_caps_lock: o.enable_caps_lock, period_shortcut: o.period_shortcut });
        kb_input::set(o.key_prefs());
        if let Some(d) = &mut self.deildu {
            d.set_auto_caps(o.auto_caps);
        }
    }

    /// Adds home-screen tiles for apps installed since start-up (`apphost::refresh`); they join
    /// the end of the grid and the App Library like any app.
    pub(super) fn installed_changed(&mut self) {
        let size = self.layout.icon as usize;
        let mask = squircle_mask(size);
        let all = apps();
        if all.len() <= self.base_icons.len() {
            return;
        }
        for a in &all[self.base_icons.len()..] {
            let mut icon = apphost::icon_image(a.icon, size).unwrap_or_else(|| scene::placeholder_icon(size));
            icon.apply_mask(&mask);
            let label = self.ui.styled(&self.ui.fit(a.name, 12.0, self.layout.label_w as f32), 12.0, Weight::Regular, WHITE, true);
            self.base_icons.push(icon.clone());
            self.grid.push(Tile { icon, label });
        }
        self.pager.set_pages(Layout::pages(self.grid.len()) + 1);
        self.rebuild_library();
        self.rebuild_home();
        self.drawn = Drawn::Nothing;
        self.dirty = true;
    }

    /// Display Zoom for the home screen: re-lays the grid and dock out and
    /// resamples the icons and labels to match.
    fn apply_zoom(&mut self, zoom: f32) {
        self.zoom = zoom;
        self.layout = Layout::zoomed(self.w, self.h, self.dock_apps.len() + 1, zoom).with_dock_sep(IN_DOCK);
        let size = self.layout.icon as usize;
        let (ui, layout) = (&self.ui, &self.layout);
        let tile = |i: usize| {
            let base = &self.base_icons[i];
            let icon = if base.w == size { base.clone() } else { base.resized(size, size) };
            Tile { icon, label: ui.styled(&ui.fit(apps()[i].name, 12.0, layout.label_w as f32), 12.0, Weight::Regular, WHITE, true) }
        };
        let (dock, grid) = (self.dock_apps.iter().map(|&i| tile(i)).collect(), (IN_DOCK..apps().len()).map(tile).collect());
        self.dock = dock;
        self.grid = grid;
        self.rebuild_library();
        self.rebuild_home();
        self.rebuild_lib_dock();
        if !self.desk_active() {
            self.dock_bg = dock_background(self.home_wp(), &self.layout);
        }
        self.drawn = Drawn::Nothing;
        self.dirty = true;
    }

    /// Dynamic Type: scales every `Ui` text by `k` (0.85..=1.3) and renders
    /// `bold` one weight heavier; open apps re-render at the next draw.
    pub fn set_text_scale(&mut self, k: f32, bold: bool) {
        self.ui.text_scale = k.clamp(ui::TEXT_SCALE_MIN, ui::TEXT_SCALE_MAX);
        self.ui.bold = bold;
        self.keyboard.clear_labels();
        if let Some(st) = &mut self.settings {
            st.relayout(&self.ui);
        }
        self.rebuild_library();
        self.dirty = true;
    }

    /// Anchors wall-clock time (Unix seconds, local UTC offset in seconds)
    /// to the monotonic clock at `t`; call about once a second.
    pub fn set_wall(&mut self, unix: i64, offset: i32, t: f64) {
        self.wall = clock::Wall { unix, offset, at: t };
        self.clock.set_wall(unix, offset, t);
        if let Some(c) = &mut self.camera {
            c.set_wall(self.wall);
        }
        self.notify.set_wall(unix, offset, t);
        self.poll_share();
        if let Some(p) = &mut self.photos {
            p.set_offset(&self.ui, offset);
        }
        if let Some(f) = &mut self.files {
            f.set_wall(unix, offset);
        }
    }

    /// Unix seconds and UTC offset at monotonic `t` (the system clock before `set_wall`).
    fn wall_now(&self, t: f64) -> (i64, i32) {
        let w = &self.wall;
        if w.unix == 0 {
            return (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64), 0);
        }
        (w.unix + ((t - w.at) / 1000.0).floor() as i64, w.offset)
    }

    /// When the shell next needs `tick` without any input (caret blink,
    /// countdowns, alarms), in monotonic ms; None when nothing is pending.
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        let mut due = wake::Wake::default();
        due.add(self.clock.next_wake(t, self.screen_on));
        if bifrost_render::appearance::fading() { due.add(Some(t)); }
        if let Some(when)=self.desk.as_ref().and_then(|d|d.dock_hide_at){due.add(Some(when));}
        if self.dock_animating(t) || self.fx_animating(t) { due.add(Some(t)); }
        if self.metrics_active() {
            // the CPU/RAM read joins a wake-up that is already planned inside its tolerance window
            let menu = self.metrics_menu_open();
            let when = desktop_metrics::coalesce(desktop_metrics::deadline(self.desktop_metrics_at, menu).max(t), desktop_metrics::tolerance_ms(menu), due.get());
            due.add(Some(when));
        }
        if let Some(at) = self.dying.iter().filter_map(apphost::AppHost::quit_deadline).reduce(f64::min) {
            due.add(Some(at));
        }
        if let Some(w) = self.screentime_wake(t) {
            due.add(Some(w));
        }
        if let Some(w) = self.lock_pad_wake(t).filter(|_| self.screen_on) {
            due.add(Some(w));
        }
        if let Some(w) = self.login_wake(t).filter(|_| self.screen_on) {
            due.add(Some(w));
        }
        if self.screen_on && !self.is_locked() {
            if let Some(w) = self.assistive_touch.next_wake(t) {
                due.add(Some(w));
            }
        }
        if let Some(w) = self.cc.next_wake().filter(|_| self.screen_on) {
            due.add(Some(w));
        }
        if self.markup_load.is_some() {
            due.add(Some(t + 50.0));
        }
        if self.screen_on {
            if let Some(w) = self.quicksearch.next_wake(t) {
                due.add(Some(w));
            }
        }
        if self.screen_on && self.screen == Screen::App(AppId::Settings) {
            if let Some(w) = self.settings.as_ref().and_then(|st| st.next_wake(t)) {
                due.add(Some(w));
            }
        }
        if self.desk.as_ref().is_some_and(|d| d.dock_slide.borrow().active(t / 1000.0)) {
            due.add(Some(t + 16.0));
        }
        if let Some(w) = self.perf.next_wake().filter(|_| self.screen_on) {
            due.add(Some(w));
        }
        if let Some(w) = self.hud.next_wake(t).filter(|_| self.screen_on) {
            due.add(Some(w));
        }
        if let Some(w) = self.input_hud.next_wake(t).filter(|_| self.screen_on) {
            due.add(Some(w));
        }
        let app_wake = if self.screen_on {
            match self.screen {
                // the app's socket is in the poll set (`ext_fds`): its frames wake the loop themselves
                Screen::App(AppId::External(_)) => None,
                Screen::App(AppId::AppStore) => self.store.as_ref().and_then(|c| c.next_wake(t)),
                Screen::App(AppId::Camera) => self.camera.as_ref().and_then(|c| c.next_wake(t)),
                Screen::App(AppId::Magnifier) => self.magnifier.as_ref().and_then(|c| c.next_wake(t)),
                Screen::App(AppId::Browser) => self.browser.as_ref().and_then(|b| b.next_wake(t)),
                _ => None,
            }
        } else {
            None
        };
        if let Some(w) = app_wake {
            due.add(Some(w));
        }
        if let Some(w) = self.notify.next_wake(t) {
            due.add(Some(w));
        }
        if let Some(w) = self.home_wake().filter(|_| self.screen_on) {
            due.add(Some(w));
        }
        if self.screen_on && self.screen == Screen::App(AppId::Terminal) {
            if let Some(w) = self.terminal_ui.next_wake() {
                due.add(Some(w));
            }
            if let Some(w) = self.term_touch.next_wake() {
                due.add(Some(w));
            }
            if self.terminal_ui.blink {
                let w = term::next_blink(t);
                due.add(Some(w));
            }
        }
        if self.screen_on && self.screen == Screen::App(AppId::Photos) {
            if let Some(w) = self.photos.as_ref().and_then(Photos::next_wake) {
                due.add(Some(w));
            }
        }
        if let Some(w) = self.shot.as_ref().filter(|_| self.screen_on).and_then(|o| o.next_wake(t)) {
            due.add(Some(w));
        }
        // a running countdown redraws once a second and captures when it ends
        if let Some((_, cd)) = self.capture.as_ref().and_then(|c| c.pending) {
            let w = if cd.done(t) { t } else { (t + 1.0).min(cd.end()) };
            due.add(Some(w));
        }
        if self.screen_on && self.screen == Screen::App(AppId::Files) {
            if let Some(w) = self.files.as_ref().and_then(|f| f.next_wake(t)) {
                due.add(Some(w));
            }
        }
        if let Some(w) = self.audio_next_wake(t) {
            due.add(Some(w));
        }
        if std::env::var_os("SPRINGBOARD_WAKE_TRACE").is_some() {
            if let (Some(at), Some(src)) = (due.get(), due.source()) {
                eprintln!("springboard: wake in {:.0} ms from {}:{}", at - t, src.file(), src.line());
            }
        }
        due.get()
    }

    /// The media volume 0..=1 the Control Center slider shows.
    pub fn volume(&self) -> f32 {
        self.cc.volume
    }

    /// Makes the connected Bluetooth audio device the default output through the audio-devices service. An error
    /// (no such device connected) comes back as text for the log.
    pub fn use_bluetooth_output(&mut self, t: f64) -> Result<(), String> {
        let body = format!("{{\"role\":\"output\",\"id\":\"{}\"}}", services::audio_devices::BLUETOOTH);
        self.service_call("bifrost-shell", "audio-devices", "set_default", &body, t).map(|_| ())
    }

    /// Syncs the slider to a level set elsewhere (the persisted one at
    /// start); no HUD, no effect.
    pub fn set_volume(&mut self, level: f32) {
        self.cc.volume = Volume::new(level).level();
        self.audio_volume(self.cc.volume);
        self.dirty = true;
    }

    /// The output route changed (headphone jack): the slider and HUD glyph
    /// follow, `level` is the new route's remembered volume, and the HUD
    /// announces the route when `announce` and the screen shows something.
    pub fn set_route(&mut self, route: Route, level: f32, announce: bool, t: f64) {
        self.cc.route = route;
        self.hud.set_route(route);
        self.set_volume(level);
        if announce && self.screen_on && !self.cc_active() {
            self.hud.show_route(self.cc.volume, route, t);
        }
    }

    /// Persists settings.ini entries for the binary (per-route volumes).
    pub fn save_prefs(&mut self, entries: &[(&str, String)]) {
        for (key, value) in entries {
            if self.prefs.get(key) != Some(value.as_str()) {
                if let Err(e) = self.prefs.set(key, value) {
                    eprintln!("springboard: settings {key}: {e}");
                }
            }
        }
    }

    /// Pauses music playback (headphones unplugged); never starts any.
    pub fn pause_music(&mut self) {
        self.audio_pause();
    }

    /// A volume key: one bifrost step, the HUD when the screen shows something
    /// other than Control Center (whose slider moves instead), and the
    /// effect for the device. Works on every screen; never wakes it.
    fn volume_key(&mut self, up: bool, t: f64) -> Vec<Effect> {
        let v = Volume::new(self.cc.volume);
        let v = if up { v.up() } else { v.down() };
        self.cc.volume = v.level();
        self.audio_volume(self.cc.volume);
        if self.screen_on && !self.cc_active() {
            self.hud.show(v.level(), t);
        }
        vec![Effect::Volume(v.level())]
    }

    /// Adds the HUD's box to `damage` while it shows or has just gone.
    fn hud_damage(&self, damage: &mut Damage, t: f64) {
        if self.perf.enabled || self.perf_drawn {
            let (x, y, w, h) = self.perf_rect();
            damage.add(x.floor() as i32, y.floor() as i32, w.ceil() as i32 + 1, h.ceil() as i32 + 1);
        }
        if self.hud.visible(t) || self.hud_drawn {
            let (x, y, w, h) = VolumeHud::damage_rect(&self.ui, self.w);
            damage.add(x.floor() as i32, y.floor() as i32, w.ceil() as i32 + 1, h.ceil() as i32 + 1);
        }
        if self.input_hud.visible(t) || self.input_hud_drawn {
            let (x, y, w, h) = SourceHud::rect(&self.ui, self.w);
            damage.add(x.floor() as i32, y.floor() as i32, w.ceil() as i32 + 1, h.ceil() as i32 + 1);
        }
    }

    /// Activity reported by the compositor includes external Linux apps.
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_activity(&mut self,t:f64) {self.last_input=t;}

    pub fn is_locked(&self) -> bool {
        self.screen == Screen::Lock
    }

    /// key=value lines for remote inspection.
    pub fn describe(&self) -> String {
        let screen = match self.screen {
            Screen::Lock => "lock".to_string(),
            Screen::Home => "home".to_string(),
            Screen::App(a) => format!("app:{a:?}").to_lowercase(),
            Screen::Switcher => "switcher".to_string(),
        };
        let deildu = self.deildu.as_ref().map(Deildu::describe).unwrap_or_default();
        let photos = self.photos.as_ref().map(Photos::describe).unwrap_or_default();
        let files = self.files.as_ref().map(Files::describe).unwrap_or_default();
        let calculator = self.describe_ext();
        let camera = self.camera.as_ref().map(|c| c.describe() + "\n").unwrap_or_default();
        let camera = format!("{camera}{}", self.magnifier.as_ref().map(magnifier::Magnifier::describe).unwrap_or_default());
        let browser = self.browser.as_ref().map(browser::Browser::describe).unwrap_or_default();
        let desktop = self.desk.as_ref().map(|d| d.wins.iter().map(|w| format!("window={:?},{},{},{},{}\n",w.app,w.r.x,w.r.y,w.r.w,w.r.h)).collect::<String>()).unwrap_or_default();
        let desktop=if let Some(d)=&self.desk {let m=self.desktop_metrics.clone();format!("{desktop}dock_position={:?}\ndock_size={}\ndock_autohide={}\ndock_visible={}\ndock_pinned={}\nmenu={:?}\ncpu_usage={:.1}\nmemory_usage={:.1}\n",d.dock_preferences.position,d.dock_preferences.size,d.dock_preferences.autohide,self.dock_is_visible(),d.dock_preferences.pinned.join(","),self.desktop_menu,m.cpu,m.memory_percent())}else{desktop};
        let store = self.store.as_ref().map(store::AppStore::describe).unwrap_or_default();
        format!(
            "screen={screen}\nscreen_on={}\nanimating={}\npage={}\ntoday={}\ncc={}\ncards={}\nvolume={:.3}\nhud={}\nquicksearch={}\n{}\n{}{}{deildu}{photos}{files}{calculator}{camera}{browser}{desktop}{store}",
            self.screen_on,
            self.anim.is_some() || self.turning() || self.pager.animating() || self.cc.animating() || self.switcher.animating(),
            self.pager.page(),
            self.pager.leading() || self.lock_pager.leading(),
            self.cc.describe(),
            self.switcher.describe(),
            self.cc.volume,
            if self.hud.shown() { "shown" } else { "hidden" },
            self.quicksearch.describe(),
            self.notify.describe(),
            today::describe(&self.today),
            self.describe_orientation(),
        )
    }

    /// Refreshes clock/battery-derived text; cheap when nothing changed.
    pub fn set_info(&mut self, info: Info) {
        if info == self.info {
            return;
        }
        let ui = &self.ui;
        let clock = status::clock_in(info.hour, info.min, self.clock_style.use_24h);
        let short_date = status::date_text(info.wday, info.mday, info.mon);
        let minute_changed = (info.hour, info.min, info.mday) != (self.info.hour, self.info.min, self.info.mday) || self.chrome.time.is_none();
        if minute_changed {
            self.chrome.time = Some(ui.chrome(&clock, 12.0, Weight::Semibold, WHITE));
            self.chrome.date = Some(ui.chrome(&short_date, 12.0, Weight::Semibold, WHITE));
            self.lock_time = Some(ui.chrome(&clock, 110.0, Weight::Semibold, WHITE));
            self.lock_date = Some(ui.chrome(&lock_date_text(&info), 22.0, Weight::Semibold, WHITE));
        }
        if info.battery != self.info.battery || self.chrome.percent.is_none() {
            self.chrome.percent = Some(ui.chrome(&format!("{}%", info.battery), 12.0, Weight::Medium, WHITE));
        }
        let glyphs = |i: &Info| (i.battery, i.charging, i.online, i.wired, i.wifi_bars, i.battery_present);
        if minute_changed || glyphs(&info) != glyphs(&self.info) {
            self.chrome_gen += 1;
        }
        self.chrome.battery = info.battery;
        self.chrome.charging = info.charging;
        self.chrome.online = info.online;
        self.chrome.wired = info.wired;
        // a desktop without a battery shows none, rather than a made-up 100%
        self.chrome.no_battery = info.desktop_network.is_some() && !info.battery_present;
        self.chrome.wifi_bars = info.wifi_bars;
        self.storage_check(info.storage_free);
        self.notify_info(&info);
        if let Some(st) = &mut self.settings {
            st.refresh(&self.ui, &info);
        }
        // fields no idle desktop surface shows (uptime, free disk, memory, ...) change all the time:
        // they refresh the pages that list them but never cost a frame of their own
        let quiet = self.desk_active() && self.settings.is_none() && self.desktop_menu.is_none() && !self.cc_active() && !self.center_active()
            && !self.quicksearch.active() && ambient_only(&self.info, &info);
        if !quiet {
            self.perf_only = false;
        }
        self.info = info;
        self.today_stale = true;
        if !quiet {
            self.dirty = true;
        }
    }

    /// Re-renders the Today widgets from the current information; repaints
    /// if the page is in view.
    fn refresh_today(&mut self, t: f64) {
        self.today_stale = false;
        let term = self.term.as_ref().filter(|t| t.alive()).map(|t| t.tail(widgets::terminal::LINES));
        let src = today::Sources { info: &self.info, term, unix: self.wall.now(t) };
        let changed = self.today.refresh(&self.ui, &src, t);
        if changed && !self.home_widgets.is_empty() {
            self.rebuild_widgets();
            self.home_gen = self.home_gen.wrapping_add(1);
            self.dirty = true;
        }
        if changed && self.today_in_view() {
            self.dirty = true;
        }
    }

    /// True while the Today page shows (home or lock screen).
    /// Desktop footprint: everything full-screen is built when a screen needs it and freed after. The dimmed home
    /// still and the Today frost exist only while the lock screen or Today can show; the lock screen's own fitted
    /// and dimmed stills only while locked; the fitted home still is freed once the GPU presenter holds its texture.
    fn lazy_wallpapers(&mut self) {
        if !self.desk_active() {
            return;
        }
        let in_view = self.today_in_view();
        let lock = self.screen == Screen::Lock;
        let idle = !lock && !in_view && self.anim.is_none();
        if (lock || in_view) && self.fit_home.is_none() {
            // released after the GPU upload: refit from the mapping
            self.fit_home = rotate::fit_wallpaper(self.home_src, self.w, self.h);
        }
        if self.wpx.dim && self.wpx.home_dim.is_none() && (lock || (self.today_frost_stale && in_view)) {
            self.wpx.home_dim = Some(wallpaper_page::dimmed(self.home_still()));
            self.dirty = true;
        }
        if self.today_frost_stale && in_view {
            let home = self.wpx.home_dim.as_ref().map_or(rotate::still(self.home_src, &self.fit_home), |i| i.view());
            self.today.set_wallpaper(home);
            self.today_frost_stale = false;
            self.dirty = true;
        }
        if lock {
            if let Some(src) = self.wpx.lock {
                if self.fit_lock.is_none() {
                    self.fit_lock = rotate::fit_wallpaper(src, self.w, self.h);
                }
                if self.wpx.dim && self.wpx.lock_dim.is_none() {
                    self.wpx.lock_dim = Some(wallpaper_page::dimmed(rotate::still(src, &self.fit_lock)));
                }
            }
        }
        if idle {
            self.release_lock_buffers();
            if self.wallpaper_uploaded && self.fit_home.take().is_some() {
                self.heap_trim_due = true;
            }
        }
    }

    /// Drops the dimmed home/lock stills and the fitted lock still (re-decoded lazily on the next lock).
    pub(super) fn release_lock_buffers(&mut self) {
        let held = self.wpx.home_dim.is_some() || self.wpx.lock_dim.is_some() || self.fit_lock.is_some();
        self.wpx.home_dim = None;
        self.wpx.lock_dim = None;
        self.fit_lock = None;
        if held {
            self.heap_trim_due = true;
        }
    }

    /// True once after lock/Today buffers were freed: the loop then calls the allocator trim.
    pub fn take_heap_trim_due(&mut self) -> bool {
        std::mem::take(&mut self.heap_trim_due)
    }

    /// The home still: the fitted copy, or the caller's mapping when it already fits.
    pub(super) fn home_still(&self) -> View<'_> {
        rotate::still(self.home_src, &self.fit_home)
    }

    /// The wallpaper and where it sits on screen, for the GPU presenter; None once uploaded.
    pub fn desktop_wallpaper_once(&self) -> Option<(View<'_>, i32)> {
        (!self.wallpaper_uploaded).then(|| {
            let wp = self.home_still();
            (wp, (self.w - wp.w as i32) / 2)
        })
    }

    /// The presenter holds the texture now: `lazy_wallpapers` frees the fitted CPU copy when idle.
    pub fn note_wallpaper_uploaded(&mut self) {
        if !self.wallpaper_uploaded {
            self.wallpaper_uploaded = true;
            self.dirty = true;
        }
    }

    fn today_in_view(&self) -> bool {
        match self.screen {
            Screen::Home => self.pager.offset() < 0.0,
            Screen::Lock => self.lock_pager.offset() < 0.0,
            _ => false,
        }
    }

    /// Opens Deildu on the detail screen of the title the Today View remembered.
    fn open_deildu(&mut self, played: &Played, t: f64) {
        self.ensure_deildu();
        if let Some(d) = &mut self.deildu {
            let item = deildu::site::Item { id: played.id.clone(), title: played.title.clone(), url: played.url.clone(), info: String::new(), torrent_url: String::new() };
            d.open_item(&self.ui, item);
        }
        self.open(AppId::Deildu, t);
    }

    /// Acts on what a tap on the Today page asked for.
    fn act(&mut self, action: Action, t: f64) {
        match action {
            Action::OpenQuickSearch => self.quicksearch_show(t),
            Action::OpenCalendar if self.screen == Screen::Lock => {}
            Action::OpenCalendar => {
                if let Some(i) = apphost::slot_of("is.olibuijr.calendar") {
                    self.open(AppId::External(i as u8), t);
                }
            }
            Action::OpenWeather if self.screen == Screen::Lock => {}
            Action::OpenWeather => {
                if let Some(slot) = apphost::slot_of("is.olibuijr.weather") {
                    self.open(AppId::External(slot as u8), t)
                }
            }
            Action::OpenDeildu(_) if self.screen == Screen::Lock => {}
            Action::OpenDeildu(p) => self.open_deildu(&p, t),
        }
    }

    /// The wallpaper catalogue shown by Settings › Wallpaper.
    pub fn set_wallpapers(&mut self, wallpapers: Vec<Wallpaper>) {
        self.wallpapers = Rc::new(wallpapers);
    }

    /// Idle trim (Snow Leopard): drops rebuildable caches before the heap is handed back.
    pub fn trim_caches(&mut self) {
        self.chrome_text.borrow_mut().clear();
        self.bar_shadows.borrow_mut().clear();
        if bifrost_render::glass::gpu() {
            *self.bar_backdrop.borrow_mut() = None;
        }
    }

    /// Swaps the wallpaper for lock and home. The caller keeps `wp` mapped;
    /// only the dock backdrop is rebuilt here, nothing is copied per frame.
    pub fn set_wallpaper(&mut self, wp: View<'static>) {
        // the dim path re-enters with the current still
        if !std::ptr::eq(wp.px.as_ptr(), self.home_src.px.as_ptr()) || self.fit_home.is_none() {
            self.home_src = wp;
            self.fit_home = rotate::fit_wallpaper(wp, self.w, self.h);
        }
        self.wallpaper_uploaded = false;
        *self.bar_backdrop.borrow_mut() = None;
        self.bar_on_light = desktop_bar::wallpaper_is_light(self.home_still(), self.desktop_bar_height());
        if self.desk_active() {
            // desktop: the dimmed copy and the Today frost are full-screen images only the lock screen and the
            // Today page use; `lazy_wallpapers` builds them when one is on screen
            self.wpx.home_dim = None;
            self.today.drop_frost();
            self.today_frost_stale = true;
            self.dock_bg = dock_background(self.home_wp(), &self.layout);
        } else {
            self.wpx.home_dim = self.wpx.dim.then(|| wallpaper_page::dimmed(self.home_still()));
            self.dock_bg = dock_background(self.home_wp(), &self.layout);
            let home = self.wpx.home_dim.as_ref().map_or(rotate::still(self.home_src, &self.fit_home), |i| i.view());
            self.today.set_wallpaper(home);
        }
        self.wallpaper_changed();
    }

    /// The lock screen's own wallpaper; None follows the home screen. On the desktop the fitted and dimmed
    /// copies are built by `lazy_wallpapers` while the lock screen shows.
    pub fn set_lock_wallpaper(&mut self, wp: Option<View<'static>>) {
        self.wpx.lock = wp;
        self.fit_lock = None;
        self.wpx.lock_dim = None;
        if !self.desk_active() {
            if let Some(v) = wp {
                self.fit_lock = rotate::fit_wallpaper(v, self.w, self.h);
                self.wpx.lock_dim = self.wpx.dim.then(|| wallpaper_page::dimmed(rotate::still(v, &self.fit_lock)));
            }
        }
        self.dirty = true;
    }

    /// Dark Appearance Dims Wallpaper.
    pub fn set_wallpaper_dim(&mut self, on: bool) {
        self.wpx.dim = on;
        self.wpx.lock_dim = None;
        if !self.desk_active() {
            if let Some(v) = self.wpx.lock.filter(|_| on) {
                self.wpx.lock_dim = Some(wallpaper_page::dimmed(rotate::still(v, &self.fit_lock)));
            }
        }
        self.set_wallpaper(self.home_src);
    }

    /// The home still, dimmed when the setting asks for it.
    pub(super) fn home_wp(&self) -> View<'_> {
        self.wpx.home_dim.as_ref().map_or(self.home_still(), |i| i.view())
    }

    /// The lock screen's frame wallpaper: its own still in portrait.
    fn lock_wp(&self) -> View<'_> {
        if self.orientation().is_landscape() {
            return self.wallpaper();
        }
        match (&self.wpx.lock_dim, self.wpx.lock) {
            (Some(i), _) => i.view(),
            (None, Some(v)) => rotate::still(v, &self.fit_lock),
            _ => self.wallpaper(),
        }
    }

    fn icon_rect(&self, app: AppId) -> Rect {
        let i = apps().iter().position(|a| a.id == app).unwrap_or(0);
        if let Some(d) = self.dock_apps.iter().position(|&a| a == i) {
            self.layout.dock_icons[d]
        } else {
            self.layout.cells[self.grid_cell(i)]
        }
    }

    /// Rebuilds the App Library dock icon from the recent apps (padded with the grid's first).
    fn rebuild_lib_dock(&mut self) {
        let mut ids: Vec<usize> = Vec::new();
        for i in self.recent_apps.iter().copied().filter(|&i| i >= IN_DOCK).chain(IN_DOCK..apps().len()) {
            if !ids.contains(&i) {
                ids.push(i);
            }
        }
        let icons: Vec<&Image> = ids.iter().take(4).filter_map(|&i| self.base_icons.get(i)).collect();
        self.lib_dock = Some(folders::library_icon(self.layout.icon as usize, &icons));
    }

    /// Creates the Deildu app if it is not running yet.
    fn ensure_deildu(&mut self) {
        if self.deildu.is_some() {
            return;
        }
        match Deildu::new(&self.ui, self.portrait_size().0, self.portrait_size().1, self.deildu_dir.clone(), self.deildu_transport.clone()) {
            Ok(mut d) => {
                d.set_player(Box::new(video::MediaPlayer::new(self.portrait_size().0, self.portrait_size().1)));
                self.deildu = Some(d);
            }
            Err(e) => eprintln!("springboard: deildu: {e}"),
        }
    }

    /// Launches (or resumes) an app: its frame zooms out of its icon.
    fn open(&mut self, app: AppId, t: f64) {
        if self.desk_active() {
            return self.desk_open(app, t);
        }
        self.quicksearch.note_launch(app);
        self.clear_notifications(app);
        self.fullscreen = false;
        self.end_multi();
        self.init_app(app, t);
        self.transition = Some(self.render_app_frame(app));
        self.anim = Some((Anim::Open { app, rect: self.icon_rect(app) }, t));
        if let Some(i) = apps().iter().position(|a| a.id == app) {
            self.recent_apps.retain(|&r| r != i);
            self.recent_apps.insert(0, i);
            self.recent_apps.truncate(8);
            self.rebuild_dock(false);
        }
    }

    /// The dock: the fixed apps, then (when Settings allows) up to three
    /// recently used apps that are not already in it, after a thin separator.
    fn rebuild_dock(&mut self, force: bool) {
        let extra = self.recent_apps.iter().copied().filter(|&i| i >= IN_DOCK).take(if self.options().dock_recents { 3 } else { 0 });
        let dock_apps: Vec<usize> = (0..IN_DOCK).chain(extra).collect();
        if !force && dock_apps == self.dock_apps {
            return;
        }
        self.dock_apps = dock_apps;
        self.layout = Layout::zoomed(self.w, self.h, self.dock_apps.len() + 1, self.zoom).with_dock_sep(IN_DOCK);
        let size = self.layout.icon as usize;
        let (ui, layout) = (&self.ui, &self.layout);
        self.dock = self
            .dock_apps
            .iter()
            .map(|&i| {
                let base = &self.base_icons[i];
                let icon = if base.w == size { base.clone() } else { base.resized(size, size) };
                Tile { icon, label: ui.styled(&ui.fit(apps()[i].name, 12.0, layout.label_w as f32), 12.0, Weight::Regular, WHITE, true) }
            })
            .collect();
        self.rebuild_lib_dock();
        if !self.desk_active() {
            self.dock_bg = dock_background(self.home_wp(), &self.layout);
        }
        self.drawn = Drawn::Nothing;
        self.dirty = true;
    }

    /// Creates the app's state if it is not running yet.
    fn init_app(&mut self, app: AppId, t: f64) {
        if app == AppId::Photos {
            self.ensure_photos();
        }
        if app == AppId::Settings && self.settings.is_none() {
            let mut st = Settings::new(&self.ui, &self.info, &self.prefs, self.w, self.h, self.wallpapers.clone());
            st.set_data_dir(self.data_dir.clone());
            st.set_haptics_available(self.haptics);
            if let Some(o) = &self.ota {
                st.set_update_badge(&self.ui, o.os_badge() > 0);
                st.mirror_update(&self.ui, &o.updater);
            }
            self.settings = Some(st);
        }
        if app == AppId::Settings {
            let muted = apps().iter().map(|a| a.id).filter(|&a| self.notify.store().is_muted(a)).collect();
            let app_notifs: Vec<(AppId, notify_page::AppNotif)> = apps().iter().filter_map(|a| self.prefs.get(&notify_page::pref_key(a.id)).map(|v| (a.id, notify_page::AppNotif::decode(v)))).collect();
            let focus_allowed = apps().iter().map(|a| a.id).filter(|&a| self.notify.store().is_focus_allowed(a)).collect();
            let schedule = self.notify.store().schedule();
            if let Some(st) = &mut self.settings {
                st.set_notify(&self.ui, schedule, muted, app_notifs, focus_allowed);
            }
        }
        if app == AppId::Terminal {
            self.open_terminal(t);
        }
        if app == AppId::Deildu {
            self.ensure_deildu();
        }
        if let AppId::External(n) = app {
            self.ext_open(n as usize);
        }
        if app == AppId::AppStore && self.store.is_none() {
            self.store = Some(store::AppStore::new(&self.ui, self.portrait_size().0, self.portrait_size().1, self.data_dir.clone()));
        } else if app == AppId::AppStore {
            if let Some(st) = &mut self.store {
                st.refresh_soon();
            }
        }
        if app == AppId::Browser && self.browser.is_none() {
            self.browser = Some(browser::Browser::new(&self.ui, self.portrait_size().0, self.portrait_size().1, self.data_dir.join("Browser"), self.deildu_transport.clone(), self.wall, t));
        }
        if app == AppId::Camera && self.camera.is_none() {
            let dir = self.photo_dirs.first().cloned().unwrap_or_else(|| self.data_dir.join("Pictures"));
            self.camera = Some(camera::Camera::new(&self.ui, self.portrait_size().0, self.portrait_size().1, dir, self.wall));
        }
        if app == AppId::Magnifier && self.magnifier.is_none() {
            self.magnifier = Some(magnifier::Magnifier::new(&self.ui, self.portrait_size().0, self.portrait_size().1));
        }
        if app == AppId::Files {
            self.open_files();
        }
    }

    /// Files › a song: opens it in the Music package, which plays it through the audio
    /// service; false when Music is not installed.
    fn play_in_music(&mut self, path: &std::path::Path, t: f64) -> bool {
        let Some(slot) = apphost::slot_of("is.olibuijr.music") else { return false };
        if let Screen::App(app) = self.screen {
            let frame = self.render_app_frame(app);
            self.store_card(app, &frame);
        }
        let ext = AppId::External(slot as u8);
        self.init_app(ext, t);
        self.ext_send(slot, &bifrost_proto::ToApp::Open { url: format!("file://{}", path.display()) });
        self.fullscreen = false;
        self.screen = Screen::App(ext);
        self.dirty = true;
        true
    }

    /// Files › a picture: opens it in the Photos viewer; false when the
    /// Photos library does not hold the file.
    fn show_in_photos(&mut self, path: &std::path::Path) -> bool {
        self.ensure_photos();
        if !self.photos.as_mut().is_some_and(|p| p.show(&self.ui, path)) {
            return false;
        }
        if let Screen::App(app) = self.screen {
            let frame = self.render_app_frame(app);
            self.store_card(app, &frame);
        }
        self.fullscreen = false;
        self.screen = Screen::App(AppId::Photos);
        self.dirty = true;
        true
    }

    /// Files › a PDF or EPUB: opens the Books package (the path cannot be passed yet).
    fn open_book(&mut self, _path: &std::path::Path, t: f64) {
        let Some(slot) = apphost::slot_of("is.olibuijr.books") else { return };
        if let Screen::App(app) = self.screen {
            let frame = self.render_app_frame(app);
            self.store_card(app, &frame);
        }
        let ext = AppId::External(slot as u8);
        self.init_app(ext, t);
        self.fullscreen = false;
        self.screen = Screen::App(ext);
        self.dirty = true;
    }

    /// Creates the Files app (with the shell's media player) if it is not running.
    fn open_files(&mut self) {
        if self.files.is_some() {
            return;
        }
        match Files::with_locations(&self.ui, self.portrait_size().0, self.portrait_size().1, self.mono, self.files_locations.clone()) {
            Ok(mut f) => {
                f.set_player(Box::new(video::MediaPlayer::new(self.portrait_size().0, self.portrait_size().1)));
                f.set_wall(self.wall.unix, self.wall.offset);
                f.set_handoff(true);
                self.files = Some(f);
            }
            Err(e) => eprintln!("springboard: files: {e}"),
        }
    }

    /// Opens the Terminal with its shell in `dir` (Files › Share › Open in
    /// Terminal): a running shell is sent there with `cd`.
    pub fn open_terminal_at(&mut self, dir: &std::path::Path, t: f64) {
        self.open_terminal(t);
        if let Some(term) = self.term.as_mut() {
            let quoted = dir.to_string_lossy().replace('\'', "'\\''");
            term.write(format!("cd '{quoted}'\n").as_bytes());
        }
        if let Screen::App(app) = self.screen {
            let frame = self.render_app_frame(app);
            self.store_card(app, &frame);
        }
        self.screen = Screen::App(AppId::Terminal);
        self.dirty = true;
    }

    /// Falls back to Files' own preview when another app cannot open the file.
    fn preview_in_files(&mut self, entry: &files::fs::Entry, t: f64) {
        if let Some(f) = &mut self.files {
            f.preview_entry(&self.ui, entry, t);
        }
    }

    /// Shows a text file (a crash log) in Files' viewer.
    fn view_log(&mut self, path: &Path, t: f64) {
        let Ok(meta) = std::fs::metadata(path) else { return };
        self.open_files();
        let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let entry = files::fs::Entry { kind: files::fs::kind_of(&name, false), name, path: path.to_path_buf(), is_dir: false, size: meta.len(), mtime: 0, items: None };
        if let Screen::App(app) = self.screen {
            let frame = self.render_app_frame(app);
            self.store_card(app, &frame);
        }
        self.preview_in_files(&entry, t);
        self.screen = Screen::App(AppId::Files);
        self.dirty = true;
    }

    /// Plays a video file on the player screen Files and Deildu share.
    pub fn play_file(&mut self, path: &std::path::Path) {
        self.open_files();
        if let Some(f) = &mut self.files {
            f.play(&self.ui, path);
        }
        self.screen = Screen::App(AppId::Files);
        self.dirty = true;
    }

    /// Leaves the app: its frame zooms into its icon and becomes its switcher card.
    fn go_home(&mut self, t: f64) {
        if self.desk_active() {
            return self.desk_show_desktop();
        }
        if let Screen::App(app) = self.screen {
            let full = Rect { x: 0, y: 0, w: self.w, h: self.h };
            let frame = self.render_app_frame(app);
            self.store_card(app, &frame);
            self.transition = Some(frame);
            self.anim = Some((Anim::Close { from: full, rect: self.icon_rect(app) }, t));
            self.screen = Screen::Home;
        }
    }

    /// Turns the screen off and relocks, dropping every overlay.
    /// Input while the screen is off: a wake request or a quick tap turns it on.
    fn dark_touch(&mut self, ev: Event, t: f64) -> Vec<Effect> {
        let wake = match ev {
            Event::Wake => true,
            Event::Down(x, y) => {
                self.dark_tap = Some((x, y, t));
                false
            }
            Event::Up(x, y) => self.dark_tap.take().is_some_and(|(x0, y0, t0)| t - t0 <= TAP_WAKE_MS && (x - x0).abs() <= TAP_WAKE_PX && (y - y0).abs() <= TAP_WAKE_PX),
            _ => false,
        };
        if wake {
            self.dark_tap = None;
            self.screen_on = true;
            self.screen_woke(t);
            return vec![Effect::ScreenOn];
        }
        vec![]
    }

    /// menu > Lock Screen, also the control verb `lock`.
    pub fn lock_screen(&mut self) -> Vec<Effect> {
        self.sleep_screen()
    }

    fn sleep_screen(&mut self) -> Vec<Effect> {
        self.grace_from = None;
        self.power_menu = None;
        self.shield = None;
        self.screentime.flush();
        self.assistive_touch.dismiss();
        self.screen_on = false;
        self.screen = Screen::Lock;
        self.lock_pad = None;
        self.login_reset();
        self.anim = None;
        self.home_gesture = None;
        self.transition = None;
        self.end_turn();
        self.cc.hide();
        self.hud.hide();
        self.quicksearch.hide();
        self.close_home_overlays();
        self.switcher_sleep();
        self.notify_sleep();
        self.leave_today();
        vec![Effect::ScreenOff]
    }

    /// The display goes off from idle or the power key. With a Require Password grace the screen is not locked yet:
    /// it stays as it was until `screen_woke` sees the display come back after the grace. A lock already on stays.
    fn display_off(&mut self, t: f64) -> Vec<Effect> {
        let before = self.screen;
        let grace = self.options().lock_grace_s;
        let fx = self.sleep_screen();
        if grace > 0 && before != Screen::Lock {
            self.grace_from = Some((t, before));
            self.screen = before;
        }
        fx
    }

    /// The display came on: inside the grace the screen returns as it was, after it the screen is locked.
    fn screen_woke(&mut self, t: f64) {
        let Some((off, before)) = self.grace_from.take() else { return };
        let grace_s = self.options().lock_grace_s;
        self.screen = if options::grace_lapsed(off, t, grace_s) { Screen::Lock } else { before };
    }

    /// Puts both pagers back on their first page and resets the Today View.
    fn leave_today(&mut self) {
        self.today.reset();
        self.library.reset();
        self.today_touch = false;
        for p in [&mut self.pager, &mut self.lock_pager] {
            p.set_leading(false);
            p.set_leading(true);
        }
    }

    pub fn event(&mut self, ev: Event, t: f64) -> Vec<Effect> {
        // session event taps see the pointer first and may drop or move it (`services::event_services`)
        let Some(ev) = self.tap_pointer_event(ev) else { return vec![] };
        self.last_input = t;
        self.dirty = true;
        // a touch or key ends a screen saver and is not passed on (the display power key still sleeps)
        if ev != Event::Power && self.saver_wake(t) {
            return vec![];
        }
        self.size_current_player_app();
        if ev == Event::Power {
            return if self.screen_on {
                self.display_off(t)
            } else {
                self.screen_on = true;
                self.screen_woke(t);
                vec![Effect::ScreenOn]
            };
        }
        if let Event::VolumeUp | Event::VolumeDown = ev {
            return self.volume_key(ev == Event::VolumeUp, t);
        }
        if !self.screen_on {
            return self.dark_touch(ev, t);
        }
        if ev == Event::Wake {
            return vec![];
        }
        if ev == Event::Screenshot {
            self.take_screenshot(t);
            return vec![];
        }
        if self.corner_swipe(ev) {
            self.take_screenshot(t);
            return vec![];
        }
        if let Some(fx) = self.dnd_event(ev, t) {
            return fx;
        }
        if let Some(fx) = self.shield_event(ev, t) {
            return fx;
        }
        if let Some(fx) = self.look_up_event(ev, t) {
            return fx;
        }
        if let Some(fx) = self.modal_event(ev) {
            return fx;
        }
        if self.clock.alert().is_some() {
            let ev = self.letterbox_event(ev);
            return self.clock.touch(&self.ui, ev, t);
        }
        if self.anim.is_some() {
            return vec![];
        }
        // During a turn new touches are ignored, but a lifted finger must still reach the app:
        // otherwise a held control (a scrub) never sees its Up and stays held.
        if self.turning() && !matches!(ev, Event::Up(..)) {
            return vec![];
        }
        if !self.is_locked() {
            let response = self.assistive_touch.event(ev, t, &self.ui, self.w, self.h);
            self.dirty |= response.changed;
            if response.consumed {
                return response.action.map_or_else(Vec::new, |action| self.assistive_action(action, t));
            }
        }
        if self.capture.is_some() && matches!(ev, Event::Down(..) | Event::Move(..) | Event::Up(..)) {
            return self.capture_touch(ev, t);
        }
        if self.markup_touch(ev, t) {
            return vec![];
        }
        if self.shot_touch(ev, t) {
            return vec![];
        }
        if let Some(fx)=self.overview_event(ev,t) {return fx;}
        // the preview card takes its pointer in every build, not only with the desktop bar; a locked screen does not
        let quick_look_took = !self.is_locked() && match ev {
            Event::Down(x, y) => self.quick_look_pointer(x, y, false),
            Event::Up(x, y) => self.quick_look_pointer(x, y, true),
            _ => false,
        };
        if quick_look_took {
            return vec![];
        }
        #[cfg(feature="desktop-wayland")]
        if let Some(fx)=self.desktop_bar_event(ev,t) {return fx;}
        if let Some(fx) = self.notify_event(ev, t) {
            return fx;
        }
        if let Some(fx) = self.cc_event(ev, t) {
            return fx;
        }
        if let Some(fx) = self.jiggle_event(ev, t) {
            return fx;
        }
        if let Some(fx) = self.overlay_event(ev, t) {
            return fx;
        }
        if let Some(fx) = self.quicksearch_event(ev, t) {
            return fx;
        }
        if self.desk_active() && matches!(self.screen, Screen::Home | Screen::App(_)) {
            if let Some(fx) = self.desk_keyboard_touch(ev, t) {
                return fx;
            }
            if let Some(fx) = self.desk_event(ev, t) {
                return fx;
            }
        }
        match self.screen {
            Screen::Lock => self.lock_touch(ev, t),
            Screen::Home => {
                self.home_touch(ev, t);
                vec![]
            }
            Screen::App(app) => self.app_touch(app, ev, t),
            Screen::Switcher => {
                self.switcher_touch(ev, t);
                vec![]
            }
        }
    }

    /// The lock screen: a vertical drag lifts it to unlock, a horizontal one
    /// slides the Today View in from the left; once that page is in, its
    /// touches go to the page.
    fn lock_touch(&mut self, ev: Event, t: f64) -> Vec<Effect> {
        // as for keys: the first click after a lock reaches the login window even before a tick opened it; with a
        // login backend the lock screen takes no swipe (a drag would lift it over the desktop)
        self.login_ensure(t);
        if self.login_open() || self.login_backend.is_some() {
            self.login_show(t);
            return self.login_touch(ev, t);
        }
        if self.lock_pad.is_some() {
            return self.lock_pad_touch(ev, t);
        }
        if self.lock_notes_event(ev, t) {
            return vec![];
        }
        if matches!(ev, Event::Down(..)) {
            self.today_touch = self.lock_pager.leading();
        }
        if self.today_touch {
            if let Some(a) = today_page_touch(&mut self.today, &mut self.lock_pager, ev, t) {
                self.act(a, t);
            }
            return vec![];
        }
        if let Some(fx) = self.music_lock_event(ev) {
            return fx;
        }
        match ev {
            Event::Down(x, y) => {
                self.lock_drag = Some((y, 0.0));
                self.lock_x0 = x;
                self.lock_axis = None;
                self.lock_pager.down(x, y, t, None);
            }
            Event::Move(x, y) => {
                let Some((y0, _)) = self.lock_drag else { return vec![] };
                if self.lock_axis.is_none() && ((x - self.lock_x0).abs() > today::SLOP || (y - y0).abs() > today::SLOP) {
                    self.lock_axis = Some((x - self.lock_x0).abs() > (y - y0).abs());
                }
                match self.lock_axis {
                    Some(true) => self.lock_pager.moved(x, y, t),
                    Some(false) => self.lock_drag = Some((y0, (y0 - y).max(0.0))),
                    None => {}
                }
            }
            Event::Up(x, y) => {
                if self.lock_axis == Some(true) {
                    self.lock_drag = None;
                    self.lock_pager.up(x, y, t, None);
                } else if let Some((_, lift)) = self.lock_drag.take() {
                    let from = lift / self.h as f32;
                    if from > 0.2 {
                        self.begin_unlock(from, t);
                    } else {
                        self.anim = Some((Anim::Unlock { from, to: 0.0 }, t));
                    }
                }
            }
            Event::Power | Event::Wake | Event::VolumeUp | Event::VolumeDown | Event::Screenshot => {}
        }
        vec![]
    }

    fn home_touch(&mut self, ev: Event, t: f64) {
        if self.home_gesture_event(ev, t) {
            return;
        }
        if matches!(ev, Event::Down(..)) {
            self.today_touch = self.pager.leading();
            self.lib_touch = !self.today_touch && self.on_library();
        }
        if self.today_touch {
            if let Some(a) = today_page_touch(&mut self.today, &mut self.pager, ev, t) {
                self.act(a, t);
            }
            return;
        }
        if self.lib_touch {
            self.library_touch(ev, t);
            return;
        }
        self.hold_touch(ev, t);
        let hit = |s: &Self, x, y| scene::hit(&s.layout, s.pager.offset(), s.dock.len(), s.home_grid.len(), x, y).filter(|&slot| s.slot_app(slot).is_some() || s.slot_folder(slot).is_some());
        match ev {
            Event::Down(x, y) => {
                let h = hit(self, x, y);
                self.lib_down = self.layout.dock_icons.get(self.dock.len()).filter(|r| r.contains(x, y)).map(|_| (x, y));
                self.pager.down(x, y, t, h);
            }
            Event::Move(x, y) => self.pager.moved(x, y, t),
            Event::Up(x, y) => {
                let h = hit(self, x, y);
                // slots run dock first, then the visible grid cells; `slot_app` maps them to apps()
                let lib_tap = self.lib_down.take().is_some_and(|(dx, dy)| (x - dx).abs() < 12.0 && (y - dy).abs() < 12.0 && self.layout.dock_icons.get(self.dock.len()).is_some_and(|r| r.contains(x, y)));
                if lib_tap {
                    self.pager.cancel(t);
                    self.pager.go(self.pager.pages - 1, t);
                    self.dirty = true;
                } else if let Some(slot) = self.pager.up(x, y, t, h) {
                    if let Some(app) = self.slot_app(slot) {
                        self.open(apps()[app].id, t);
                    } else if let Some(f) = self.slot_folder(slot) {
                        self.open_folder(f);
                    }
                }
            }
            Event::Power | Event::Wake | Event::VolumeUp | Event::VolumeDown | Event::Screenshot => {}
        }
    }

    fn app_touch(&mut self, app: AppId, ev: Event, t: f64) -> Vec<Effect> {
        match self.split_touch(app, ev, t) {
            Some(fx) => fx,
            None => self.app_touch_single(app, ev, t),
        }
    }

    fn app_touch_single(&mut self, app: AppId, ev: Event, t: f64) -> Vec<Effect> {
        // the home swipe works in frame coordinates; the apps laid out for the portrait panel
        // see touches mapped into their letterbox
        let frame_ev = ev;
        let ev = if self.letterboxed(app) { self.letterbox_event(ev) } else { ev };
        match ev {
            _ if self.home_gesture_event(frame_ev, t) => {}
            _ if app == AppId::Terminal => self.terminal_touch(ev, t),
            _ if app == AppId::Deildu => {
                if let Some(d) = &mut self.deildu {
                    d.touch(&self.ui, ev, t);
                }
            }
            _ if app == AppId::Photos => {
                let (shared, camera) = self.photos.as_mut().map_or((None, false), |p| {
                    p.touch(&self.ui, ev, t);
                    (p.take_share(), p.take_camera_request())
                });
                if camera {
                    self.open(AppId::Camera, t);
                }
                if let Some(path) = shared {
                    let anchor = (self.w as f32 / 2.0, self.h as f32 / 3.0);
                    self.open_share(share::Item::File(path), anchor);
                }
            }
            _ if app == AppId::Files => {
                let requests = self.files.as_mut().map(|f| f.touch(&self.ui, ev, t)).unwrap_or_default();
                for r in requests {
                    match r {
                        files::Request::OpenTerminal(dir) => self.open_terminal_at(&dir, t),
                        files::Request::Play(path) => self.play_file(&path),
                        files::Request::Share(path) => {
                            let anchor = (self.w as f32 / 2.0, self.h as f32 / 3.0);
                            self.open_share(share::Item::File(path), anchor);
                        }
                        files::Request::OpenPhoto(e) => {
                            if !self.show_in_photos(&e.path) {
                                self.preview_in_files(&e, t);
                            }
                        }
                        files::Request::OpenBook(path) => self.open_book(&path, t),
                        files::Request::Markup(path) => self.markup_file(&path),
                        files::Request::PlayAudio(e) => {
                            if !self.play_in_music(&e.path, t) {
                                self.preview_in_files(&e, t);
                            }
                        }
                    }
                }
            }
            _ if matches!(app, AppId::External(_)) => {
                if let AppId::External(n) = app {
                    self.ext_touch(n as usize, ev, t);
                }
            }
            _ if app == AppId::AppStore => {
                let launch = self.store.as_mut().and_then(|c| {
                    c.touch(&self.ui, ev, t);
                    c.take_launch()
                });
                // a tap on the Search field raises the on-screen keyboard (desktops have a hardware one)
                if self.store.as_mut().is_some_and(|c| c.take_keyboard_request()) {
                    self.set_keyboard(true, t);
                }
                if let Some(l) = launch {
                    // OPEN: an app the shell registered at start-up opens at once; one installed
                    // since then is registered by restarting the shell (apphost reads once)
                    if apphost::slot_of(&l.id).is_none() && apphost::refresh() {
                        self.installed_changed();
                    }
                    match apphost::slot_of(&l.id) {
                        Some(slot) => {
                            let ext = AppId::External(slot as u8);
                            if self.desk_active() { self.desk_open(ext, t); }
                            else {
                                self.init_app(ext, t);
                                self.screen = Screen::App(ext);
                                self.dirty = true;
                            }
                        }
                        None => eprintln!("springboard: installed app {} has no usable manifest", l.id),
                    }
                }
            }
            _ if app == AppId::Camera => {
                if let Some(c) = &mut self.camera {
                    c.touch(&self.ui, ev, t);
                }
            }
            _ if app == AppId::Magnifier => {
                if let Some(c) = &mut self.magnifier {
                    c.touch(&self.ui, ev, t);
                }
            }
            _ if app == AppId::Browser => {
                if let Some(b) = &mut self.browser {
                    b.touch(&self.ui, ev, t);
                }
            }
            _ if app == AppId::Settings => {
                self.ensure_defaults();
                let logins = self.login_item_views();
                let pending = self.login_pending_views();
                let ext = self.login_ext_cache.get();
                let background_off = login_items_page::background_off_from_pref(self.prefs.get(login_items_page::BACKGROUND_OFF_KEY));
                let Some(st) = &mut self.settings else { return vec![] };
                st.set_login_items(logins, pending);
                st.set_login_extensions(ext, background_off);
                if let Some(dock) = self.desk.as_ref().map(|d| d.dock_preferences.clone()) {
                    st.set_dock(dock);
                }
                if let Some((_, _, views)) = &self.defaults {
                    st.set_defaults(views.clone());
                }
                let before = st.options().clone();
                let fx = st.touch(&self.ui, &mut self.prefs, ev, t);
                let changed = *st.options() != before || !fx.is_empty();
                let changes = st.take_notify_changes();
                let view = st.take_view_request();
                let dock_changes = st.take_dock_changes();
                let default_changes = st.take_defaults_changes();
                let login_removed = st.take_login_removed();
                let login_moved = st.take_login_moved();
                let login_approved = st.take_login_approved();
                let preview = st.take_screensaver_preview();
                for change in changes {
                    match change {
                        notify_page::NotifyChange::Schedule(s) => self.set_dnd_schedule(s),
                        notify_page::NotifyChange::ToggleMute(a) => self.toggle_mute(a),
                        notify_page::NotifyChange::FocusAllowed(a, on) => self.set_focus_allowed(a, on),
                        notify_page::NotifyChange::App(a, n) => {
                            let _ = self.prefs.set(&notify_page::pref_key(a), &n.encode());
                            self.store_changed();
                        }
                    }
                }
                if let Some(f) = fx.iter().rev().find_map(|f| if let Effect::Brightness(b) = f { Some(*b) } else { None }) {
                    self.info.brightness = f;
                }
                if let Some(path) = view {
                    self.view_log(&path, t);
                }
                self.apply_default_changes(default_changes);
                self.login_items_remove(login_removed);
                self.login_items_approve(login_approved);
                self.login_items_move(login_moved);
                if self.desk.is_some() {
                    for change in dock_changes {
                        let _ = self.dock_change(change, t);
                    }
                }
                if changed || matches!(ev, Event::Up(..)) {
                    self.apply_options();
                }
                if preview {
                    self.saver_preview(t);
                }
                return fx;
            }
            _ => {}
        }
        vec![]
    }

    /// Advances animations and idle timeout; returns effects to apply.
    /// Follows Settings > Appearance: Light, Dark, or Auto by the local hour; the shell redraws while the scheme fades.
    fn appearance_tick(&mut self, t: f64) {
        let mode = bifrost_render::appearance::Mode::parse(self.prefs.get(settings::APPEARANCE_PREF).unwrap_or("dark"));
        bifrost_render::appearance::publish_mode(mode);
        pages::DeskPrefs::load(&self.prefs).apply();
        let accent = bifrost_render::appearance::Accent::parse(self.prefs.get(settings::ACCENT_PREF).unwrap_or("blue"));
        if accent != bifrost_render::appearance::accent() {
            bifrost_render::appearance::set_accent(accent);
            self.dirty = true;
            self.chrome_gen += 1;
        }
        let (unix, offset) = self.wall_now(t);
        let hour = ((unix + offset as i64).rem_euclid(86_400) / 3600) as u32;
        bifrost_render::appearance::set_light(mode.is_light(hour), t);
        #[cfg(feature = "desktop-wayland")]
        {
            let light = bifrost_render::appearance::target_is_light();
            let (dark_c, light_c) = accent.colours();
            // The effective scheme and accent for native apps' portal (bifrost-portal); written on change only.
            crate::appearance_portal::publish(crate::appearance_portal::State::new(light, if light { light_c } else { dark_c }), || {
                if !crate::appearance_portal::is_root() { return None; }
                crate::bare::app_policy::Account::installed().ok().map(|a| (a.uid, a.gid))
            });
        }
        if bifrost_render::appearance::tick(t) || bifrost_render::appearance::fading() {
            self.dirty = true;
            self.chrome_gen += 1;
        }
    }

    /// Whether the CPU and RAM readings are read at all: a desktop with a readout or menu showing, the screen on and
    /// the shell unlocked. Otherwise there is no read and no wake-up.
    fn metrics_active(&self) -> bool {
        self.desk_active() && self.screen_on && !self.is_locked() && self.metrics_wanted()
    }

    /// Reads the host counters when the deadline has passed; marks the shell dirty only when the shown text changed.
    fn metrics_tick(&mut self, t: f64) {
        if !self.metrics_active() {
            if self.metrics_running {
                self.metrics_running = false;
                self.metrics_sampler.pause(); // the next baseline must not span the pause
            }
            return;
        }
        let menu = self.metrics_menu_open();
        if self.metrics_running && !desktop_metrics::is_due(t, self.desktop_metrics_at, menu) {
            return;
        }
        self.metrics_running = true;
        self.desktop_metrics_at = t;
        let key = |m: &desktop_metrics::Snapshot| if menu { m.shown() } else { m.bar_key() };
        let before = key(&self.desktop_metrics);
        self.metrics_sampler.sample(&mut self.desktop_metrics, menu);
        if key(&self.desktop_metrics) != before {
            self.dirty = true; // redraw only when the shown text changed
        }
    }

    pub fn tick(&mut self, t: f64) -> Vec<Effect> {
        keyboard::pump();
        self.appearance_tick(t);
        self.terminal_follow_scheme();
        self.dock_tick(t);
        self.fx_tick(t);
        self.metrics_tick(t);
        let mut fx = self.tick_inner(t);
        fx.extend(self.gated_clicks().into_iter().map(Effect::KeyClick));
        fx
    }

    fn tick_inner(&mut self, t: f64) -> Vec<Effect> {
        if self.chrome.rot_lock != self.rotation_locked() {
            self.chrome.rot_lock = self.rotation_locked();
            self.chrome_gen += 1;
            self.dirty = true;
        }
        self.input_memory_tick();
        self.guided_tick();
        self.split_tick();
        self.markup_tick();
        self.ota_tick(t);
        if self.screen_on && !self.is_locked() {
            self.dirty |= self.assistive_touch.tick(t);
        }
        self.saver_tick(t);
        if self.screen_on && self.idle_off_ms > 0.0 && t - self.last_input > self.idle_off_ms && self.last_input > 0.0 {
            return self.display_off(t);
        }
        self.lock_pad_tick(t);
        self.login_tick(t);
        self.screentime_tick(t);
        self.size_current_player_app();
        self.home_tick(t);
        if self.pager.tick(t) | self.lock_pager.tick(t) | self.today.tick(t) {
            self.dirty = true;
        }
        self.lazy_wallpapers();
        if self.today.editing() && !self.today_in_view() {
            self.today.reset();
        }
        if let Some(item) = self.deildu.as_ref().and_then(Deildu::last_played) {
            let played = Played { id: item.id.clone(), title: item.title.clone(), url: item.url.clone() };
            if self.today.note_played(&played) {
                self.today_stale = true;
            }
        }
        if self.today_stale || self.today.due(t) {
            self.refresh_today(t);
        }
        self.tick_turn(t);
        self.update_orientation(t);
        let cc_fx = self.cc_tick(t);
        self.switcher_tick(t);
        if self.hud.tick(t) {
            self.dirty = true;
        }
        if self.input_hud.tick(t) {
            self.dirty = true;
        }
        let perf_changed = self.perf.roll(t);
        if self.desk.as_ref().is_some_and(|d| d.dock_slide.borrow().active(t / 1000.0)) {
            self.dirty = true;
        }
        self.dirty |= self.quicksearch_tick(t);
        if let Some((a, start)) = self.anim {
            self.dirty = true;
            if t - start >= accessibility::anim_ms(gestures::SPRING_MS) {
                self.anim = None;
                self.transition = None;
                match a {
                    Anim::Unlock { to, .. } if to >= 1.0 => {
                        self.screen = Screen::Home;
                        self.leave_today();
                        // the lock screen's buffers are gone: hand their pages back to the system
                        #[cfg(target_env = "gnu")]
                        {
                            unsafe extern "C" {
                                fn malloc_trim(pad: usize) -> i32;
                            }
                            unsafe { malloc_trim(0) };
                        }
                    }
                    Anim::Open { app, .. } => self.screen = Screen::App(app),
                    _ => {}
                }
            }
        }
        if self.screen == Screen::App(AppId::Terminal) && self.term.is_none() && self.anim.is_none() {
            self.go_home(t);
        }
        if self.screen == Screen::App(AppId::Deildu) {
            match &mut self.deildu {
                Some(d) => self.dirty |= d.tick(&self.ui, t),
                None if self.anim.is_none() => self.go_home(t),
                None => {}
            }
        }
        let mut effects = cc_fx;
        let typing_on_hardware = self.desk_active();
        if let Some(p) = &mut self.photos {
            if self.screen == Screen::App(AppId::Photos) {
                p.set_hardware_keyboard(typing_on_hardware);
                self.dirty |= p.tick(&self.ui, t);
            }
            effects.extend(p.take_effects());
        } else if self.screen == Screen::App(AppId::Photos) && self.anim.is_none() {
            self.go_home(t);
        }
        if let Some(o) = &self.shot {
            if o.done(t) {
                self.shot = None;
                self.dirty = true;
            } else if o.animating(t) {
                self.dirty = true;
            }
        }
        if self.capture.as_ref().is_some_and(|c| c.due(t)) {
            self.finish_capture(t);
        }
        if self.capture.is_some() {
            self.dirty = true;
        }
        if self.screen == Screen::App(AppId::Files) {
            match &mut self.files {
                Some(f) => self.dirty |= f.tick(&self.ui, t),
                None if self.anim.is_none() => self.go_home(t),
                None => {}
            }
        }
        self.audio_tick(t);
        if self.login_owns_keyboard() && self.keyboard.animating() {
            self.dirty = true;
        }
        for out in self.keyboard.tick(t) {
            // on the lock screen the login window owns the keys
            if self.login_owns_keyboard() {
                effects.extend(self.login_key(&out, t));
            } else {
                self.type_out(out, t);
            }
            self.dirty = true;
        }
        if self.screen == Screen::App(AppId::Terminal) {
            self.dirty |= self.terminal_ui.tick(t);
            if self.terminal_ui.blink {
                let on = term::blink_phase(t);
                for tab in self.term.iter_mut() {
                    self.dirty |= tab.set_blink_phase(on);
                }
            }
            if let Some(term) = self.term.as_mut() {
                self.dirty |= self.term_touch.tick(term, t);
            }
        }
        self.ext_tick(t);
        if self.screen == Screen::App(AppId::AppStore) {
            let mut grew = false;
            if let Some(c) = &mut self.store {
                self.dirty |= c.tick(&self.ui, t);
                grew = c.take_registry_changed();
            }
            // an install finished: its icon joins the home screen right away
            if grew && apphost::refresh() {
                self.installed_changed();
            }
        }
        self.tick_camera(t);
        self.tick_magnifier(t);
        if self.screen == Screen::App(AppId::Browser) {
            if let Some(b) = &mut self.browser {
                self.dirty |= b.tick(&self.ui, t);
            }
        }
        if self.screen == Screen::App(AppId::Settings) {
            if let Some(st) = &mut self.settings {
                let changed = st.tick(&self.ui, t);
                if changed {
                    self.settings_changed = t;
                }
                self.dirty |= changed;
                // Install Now: the update's backup is in, so the restart goes ahead
                if st.take_install_ready() {
                    effects.push(Effect::Restart);
                }
            }
        }
        self.clock.set_muted(self.cc.volume <= 0.0);
        // shell tests stand in for the Clock app by writing its files, so they count as the app running
        let running = cfg!(test) || apphost::slot_of(clock::MANIFEST_ID).and_then(|k| self.ext.get(k).and_then(Option::as_ref)).is_some_and(|h| h.running());
        if running && !self.clock.app_running {
            self.clock.next_poll_now(t);
        }
        self.clock.app_running = running;
        let (clock_effects, changed) = self.clock.tick(&self.ui, t);
        effects.extend(clock_effects);
        if changed {
            self.dirty = true;
        }
        if effects.contains(&Effect::Alarm) {
            self.dirty = true;
            self.last_input = t;
            if !self.screen_on {
                self.screen_on = true;
                self.screen_woke(t);
                effects.push(Effect::ScreenOn);
            }
            self.notify_alarm();
        }
        effects.extend(self.notify_tick(t));
        effects.extend(self.low_battery_tick(t));
        self.announce_tick(t);
        self.permission_tick(t);
        self.usernotif_ask_tick(t);
        // app requests for the Bluetooth radio (services/bluetooth_service.rs)
        effects.extend(services::bluetooth_service::take_commands().into_iter().map(Effect::Bluetooth));
        self.perf_only = perf_changed && !self.dirty;
        self.dirty |= perf_changed;
        effects
    }

    fn progress(&self, t: f64) -> f32 {
        self.anim.map_or(1.0, |(_, s)| (((t - s) / accessibility::anim_ms(gestures::SPRING_MS)).clamp(0.0, 1.0)) as f32)
    }

    /// Draws what changed since the last frame into `s` and returns it as
    /// damage for the presenter; a frame with nothing new draws nothing.
    /// The home and lock screens compare against `drawn` and paint only the
    /// touched rows, spread over worker threads; apps repaint in full.
    pub fn draw(&mut self, s: &mut Surface, t: f64) -> &Damage {
        // the audio service connects only once the desktop has been drawn (lazy; socket activation starts it then)
        self.audio.arm();
        self.dirty = false;
        let mut damage = std::mem::take(&mut self.damage);
        let mut rows = std::mem::take(&mut self.rows);
        damage.clear();
        if self.saver_draw(s, t) {
            damage.all();
            self.drawn = Drawn::Nothing;
            self.damage = damage;
            self.rows = rows;
            return &self.damage;
        }
        #[cfg(feature = "desktop-wayland")]
        if std::mem::take(&mut self.perf_only) && std::env::var_os("SPRINGBOARD_WAYLAND").is_some()
            && self.desk_active() && !self.is_locked() && self.anim.is_none()
            && self.desktop_menu.is_none() && !self.cc_active() && !self.center_active() && !self.assistive_visible() && !self.quick_look.is_visible()
            && self.desk_drawn && self.retain_wayland_windows() {
            let (_,y,_,h)=self.perf_rect();
            let top=y.floor().max(0.) as usize;
            let bottom=((y+h).ceil() as usize+1).min(s.h);
            self.draw_desktop_background(&mut s.rows(top,bottom));
            self.draw_perf(&mut s.rows(top,bottom));
            damage.add(0,top as i32,s.w as i32,(bottom-top) as i32);
            self.damage=damage; self.rows=rows;
            return &self.damage;
        }
        let prev_regions = self.desk_regions.take();
        self.desk_drawn = false;
        if self.draw_turn(s, t) {
            damage.all();
            self.drawn = Drawn::App;
            self.damage = damage;
            self.rows = rows;
            return &self.damage;
        }
        if self.assistive_visible() {
            // Repaint beneath the movable translucent control, including
            // its old position, so dragging and closing cannot leave trails.
            self.drawn = Drawn::Nothing;
            damage.all();
        }
        self.notify_prepare();
        if let Some(sh) = self.shield.as_ref().filter(|_| !self.st_suspended()) {
            damage.all();
            sh.draw(s, &self.ui, t);
            self.drawn = Drawn::App;
            self.damage = damage;
            self.rows = rows;
            return &self.damage;
        }
        // on the desktop both panels are live overlays drawn with the desk (below)
        if (self.cc_active() || self.center_active()) && !self.desk_active() {
            damage.all();
            if self.center_active() {
                self.draw_center(s, t);
            } else {
                self.draw_cc(s, t);
            }
            self.drawn = Drawn::App;
            self.draw_assistive(s, t);
            self.damage = damage;
            self.rows = rows;
            return &self.damage;
        }
        // on the desktop Search floats over the live desk (an overlay surface; see desktop_chrome_frames)
        if self.quicksearch.active() && !self.desk_active() {
            damage.all();
            let started = std::time::Instant::now();
            self.draw_quicksearch(s, t);
            quicksearch_draw_timing(started.elapsed());
            self.drawn = Drawn::App;
            self.draw_assistive(s, t);
            self.damage = damage;
            self.rows = rows;
            return &self.damage;
        }
        if self.page_editor.is_some() || self.gallery.is_some() || self.folder_view.is_some() || self.jiggle.is_some() {
            damage.all();
            self.draw_home_overlay(s);
            self.drawn = Drawn::App;
            self.draw_assistive(s, t);
            self.damage = damage;
            self.rows = rows;
            return &self.damage;
        }
        if self.notify_damage_all() {
            damage.all();
        }
        if self.announce_damage() || self.reader_damage() {
            damage.all();
        }
        // the preview panel is drawn over the whole desktop: a change repaints the frame
        if self.quick_look.take_dirty() {
            damage.all();
        }
        let ease = gestures::ease_out_quartic(self.progress(t));
        let full = Rect { x: 0, y: 0, w: self.w, h: self.h };
        // The app shrinking under the finger repaints like the home zoom: only
        // the old and new rectangles over home, not the whole screen.
        let gesture_zoom = match (self.screen, self.home_gesture) {
            (Screen::App(_), Some(g)) => Some(self.gesture_rect(&g)).filter(|r| r.w < self.w),
            _ => None,
        };
        let screen = if gesture_zoom.is_some() { Screen::Home } else { self.screen };
        self.drawn = match (screen, self.anim) {
            (Screen::Lock, anim) => {
                let lift = match anim {
                    Some((Anim::Unlock { from, to }, _)) => (from + (to - from) * ease) * self.h as f32,
                    _ => self.lock_drag.map_or(0.0, |(_, lift)| lift),
                };
                let lift = lift.round().max(0.0);
                let today_off = self.lock_pager.offset().round();
                let now = Drawn::Lock { lift: lift as i32, chrome: self.chrome_gen, notes: self.notify.lock_state(), today: (today_off as i32, self.today.gen()) };
                if self.drawn != now || self.lock_pad.is_some() || self.login.is_some() {
                    damage.all();
                }
                self.hud_damage(&mut damage, t);
                damage.rows(&mut rows);
                // Desktop unlock must reveal the same desktop shown after the
                // transition, rather than the bifrost's icon grid.
                let desktop = self.desk_active();
                if desktop && lift > 0.0 {
                    self.draw_desk(s, t);
                }
                let (scene, lock, today, layout) = (self.scene(), self.lock_view(), &self.today, &self.layout);
                let (pad, pad_ui, signin) = (self.lock_pad.as_ref(), &self.ui, self.login.as_ref());
                s.draw_rows(&rows, threads(), &|s| {
                    // only the strip the lock screen has uncovered shows home
                    if lift > 0.0 && !desktop {
                        scene.draw(&mut s.band(lock.h as usize - lift as usize));
                    }
                    lock.draw(s, lift);
                    today.draw(s, layout, today_off);
                    if let Some(pad) = pad {
                        pad.draw(s, pad_ui, t);
                    }
                    if let Some(l) = signin {
                        l.draw(s, pad_ui, t);
                    }
                });
                if self.login_owns_keyboard() && (self.keyboard.shown() || self.keyboard.animating()) {
                    let kb_top = self.kb_top();
                    self.keyboard.draw(s, &self.ui, kb_top, t);
                    let below = (kb_top + self.keyboard.h() + self.keyboard.offset(t)) as usize;
                    s.fill_rect(0, below, self.w as usize, self.h as usize, keyboard::BG);
                }
                now
            }
            (Screen::Switcher, anim) => {
                damage.all();
                self.draw_switcher(s, t);
                if let Some((Anim::Open { rect, .. }, _)) = anim {
                    let r = self.zoom_rect(rect, full, ease);
                    draw_zoom(self.transition.as_ref(), s, r, self.zoom_radius(r.w as f32), Self::zoom_alpha(true, ease));
                }
                Drawn::App
            }
            (Screen::Home | Screen::App(_), _) if self.desk_active() => {
                // with the GPU drawing the wallpaper, only the bar, the dock and menus are ours
                #[cfg(feature = "desktop-wayland")]
                let now = self.desk_regions_now();
                #[cfg(not(feature = "desktop-wayland"))]
                let now: Option<Vec<(i32, i32, i32, i32)>> = None;
                let now = self.overview_regions(now);
                if let (Some(prev), Some(cur)) = (&prev_regions, &now) {
                    self.desk_clear = Some(prev.iter().chain(cur).copied().collect());
                }
                // what the regions held before this draw: pixels that come out the same are not damage
                let before = self.desk_clear.as_ref().filter(|_| s.y0 == 0).map(|r| Damage::snapshot_rects(s.px, s.stride, s.w, s.h, r));
                self.draw_desk(s, t);
                self.draw_overview(s, t);
                // Control Center and Notification Center float over the live desktop, at the menus' layer
                if self.center_active() {
                    self.draw_center(s, t);
                } else if self.cc_active() {
                    self.draw_cc(s, t);
                }
                if self.quicksearch.active() {
                    self.quicksearch.draw(s, &self.ui, self.w, self.h, t);
                    if self.quicksearch.animating() {
                        damage.all();
                    }
                }
                // the compose candidate window floats above the windows
                self.draw_compose(s, &self.ui, self.w, self.h);
                match self.desk_clear.take() {
                    Some(rects) => match before {
                        Some(b) => damage.union(&Damage::changed_in(&rects, &b, s.px, s.stride, s.w, s.h)),
                        None => for (x, y, w, h) in rects { damage.add(x, y, w, h); },
                    },
                    None => damage.all(),
                }
                // the panels fade and slide in: repaint everything until they settle
                if self.cc.animating() || self.notify.animating() {
                    damage.all();
                }
                self.desk_regions = now;
                self.desk_drawn = true;
                Drawn::Nothing
            }
            (Screen::App(_), _) if self.home_gesture.is_some() => {
                damage.all();
                self.draw_home_gesture(s);
                Drawn::App
            }
            (Screen::App(AppId::External(n)), None) if self.multi.is_none() && self.drawn == Drawn::Ext(n) && !self.ext_kb_visible() && self.ext_has_frame(n as usize) => {
                // only the app's damaged rectangles changed since the last frame
                self.ext_blit_damage(s, n as usize, &mut damage);
                Drawn::Ext(n)
            }
            (Screen::App(AppId::Terminal), None) if let Some(key) = self.terminal_key(t) => {
                // only output changed: repaint and send just the grid rows that changed
                if self.drawn == (Drawn::Term { key }) {
                    if let Some(term) = self.term.as_mut() {
                        let (x, _, w, _) = term.area();
                        for row in term.repaint() {
                            let (y, h) = term.row_span(row);
                            term.copy_rows(s, y, h);
                            damage.add(x, y, w, h);
                        }
                    }
                } else {
                    damage.all();
                    self.draw_app(s, AppId::Terminal, t);
                }
                Drawn::Term { key }
            }
            (Screen::App(AppId::Settings), None) if let Some((key, off)) = self.settings_key(t) => {
                match self.drawn {
                    Drawn::Settings { key: k, off: prev } if k == key => {
                        // same page, same content: nothing, or the old pixels shifted plus a strip
                        if prev != off {
                            damage.all();
                            if !self.draw_settings_scrolled(s, t, off - prev) {
                                self.draw_app(s, AppId::Settings, t);
                            }
                        }
                    }
                    _ => {
                        damage.all();
                        self.draw_app(s, AppId::Settings, t);
                    }
                }
                Drawn::Settings { key, off }
            }
            (Screen::App(app), _) => {
                damage.all();
                if self.multi.is_some() {
                    self.draw_multi(s, app, t);
                } else {
                    self.draw_app(s, app, t);
                }
                self.draw_split_ui(s, app);
                match app {
                    AppId::External(n) if self.multi.is_none() => Drawn::Ext(n),
                    _ => Drawn::App,
                }
            }
            (Screen::Home, anim) => {
                let zoom = gesture_zoom.or(match anim {
                    Some((Anim::Open { rect, .. }, _)) => Some(self.zoom_rect(rect, full, ease)),
                    Some((Anim::Close { from, rect }, _)) => Some(self.zoom_rect(from, rect, ease)),
                    _ => None,
                });
                let bounds = zoom.map(|r| [r.x, r.y, r.x + r.w, r.y + r.h]);
                let radius = zoom.map_or(0.0, |r| self.zoom_radius(r.w as f32));
                let transition = self.transition.as_ref();
                let alpha = Self::zoom_alpha(matches!(anim, Some((Anim::Open { .. }, _))), ease);
                let scene = self.scene();
                let badges = self.badge_view();
                let now = Drawn::Home { state: scene.state(), chrome: self.chrome_gen, card: bounds, badges: self.notify.badge_gen() };
                match self.drawn {
                    Drawn::Home { state, chrome, card: before, badges: had } => {
                        scene.damage(Some(state), &mut damage);
                        if chrome != self.chrome_gen {
                            damage.add(0, 0, self.w, self.layout.status_h);
                        }
                        if had != self.notify.badge_gen() {
                            damage.all();
                        }
                        if before != bounds {
                            for [x0, y0, x1, y1] in [before, bounds].into_iter().flatten() {
                                damage.add(x0, y0, x1 - x0, y1 - y0);
                            }
                        }
                    }
                    _ => damage.all(),
                }
                self.hud_damage(&mut damage, t);
                damage.rows(&mut rows);
                // a partial repaint cuts at the damaged rectangles, not whole rows (badges stick out of
                // their tiles' rectangles, so they and any zoom card keep the row repaint)
                if !damage.is_all() && zoom.is_none() && badges.is_empty() && scene.can_clip() {
                    let areas = damage.areas();
                    s.draw_rows(&rows, threads(), &|s| scene.draw_clipped(s, areas));
                    now
                } else {
                s.draw_rows(&rows, threads(), &|s| {
                    scene.draw(s);
                    badges.draw(s);
                    if let Some(r) = zoom {
                        draw_zoom(transition, s, r, radius, alpha);
                    }
                });
                now
                }
            }
        };
        if self.clock.alert().is_none() {
            self.draw_banner(s, t);
        }
        self.draw_announce(s);
        self.draw_reader(s);
        if self.dnd.session.is_some() {
            damage.all();
            self.dnd_draw(s);
        }
        if self.clock.alert().is_some() {
            damage.all();
            self.draw_letterboxed(s, |sh, s| sh.clock.draw_alert(s, &sh.ui, t));
            home_indicator(s, &self.ui, self.w, self.h, 255);
        }
        self.hud_drawn = self.hud.visible(t);
        if self.hud_drawn {
            self.hud.draw(s, &self.ui, self.w, t);
        }
        self.input_hud_drawn = self.input_hud.visible(t);
        if self.input_hud_drawn {
            self.input_hud.draw(s, &self.ui, self.w, t);
        }
        if let Some(o) = &self.shot {
            damage.all();
            o.draw(s, t);
        }
        if let Some(c) = &self.capture {
            damage.all();
            let frames = self.capture_frames();
            c.draw(s, &self.ui, &frames, self.w as i32, self.h as i32, t);
        }
        if let Some(m) = &self.markup {
            damage.all();
            self.drawn = Drawn::Nothing;
            m.draw(s, &self.ui);
        }
        if self.share.is_some() || self.share_prompt.is_some() || self.look_up.is_some() {
            damage.all();
            self.drawn = Drawn::Nothing;
        }
        if self.low_alert.is_some() || self.power_menu.is_some() || self.permission_sheet.is_some() || self.crash_alert.is_some() || self.notify.ask.is_some() || self.compose_showing() || self.print_showing() || self.char_viewer.is_some() || self.keyboard_viewer.is_some() || self.a11y_panel.is_some() {
            // modal: repaint everything under it every frame, and again once it is gone
            damage.all();
            self.drawn = Drawn::Nothing;
            if let Some(a) = &self.low_alert {
                a.draw(s, &self.ui, self.w, self.h);
            }
            if let Some(m) = &self.power_menu {
                m.draw(s, &self.ui, self.w, self.h);
            }
            if let Some(p) = &self.permission_sheet {
                p.draw(s, &self.ui, self.w, self.h);
            }
            if let Some(a) = &self.crash_alert {
                a.draw(s, &self.ui, self.w, self.h);
            }
            if let Some(a) = &self.notify.ask {
                a.draw(s, &self.ui, self.w, self.h);
            }
            #[cfg(feature = "desktop-wayland")]
            if let Some(c) = &self.compose {
                c.draw(s, &self.ui, self.w, self.h);
            }
            #[cfg(feature = "desktop-wayland")]
            if let Some(p) = &self.print_sheet {
                p.draw(s, &self.ui, self.w, self.h);
            }
            if let Some(v) = &self.char_viewer {
                v.draw(s, &self.ui, self.w, self.h);
            }
            if let Some(v) = &self.keyboard_viewer {
                v.draw(s, &self.ui, self.w, self.h, self.keyboard_viewer_layout());
            }
            if let Some(p) = &self.a11y_panel {
                p.draw(s, &self.ui, self.w, self.h);
            }
        }
        self.draw_share(s);
        self.draw_look_up(s);
        self.draw_assistive(s, t);
        self.damage = damage;
        self.rows = rows;
        &self.damage
    }

    /// Everything the Terminal frame shows apart from its grid, hashed; None when something
    /// over or around the grid moves (keyboard, menu, selection, HUD, input just now, a
    /// letterbox, Split View), and the frame is then repainted in full.
    fn terminal_key(&self, t: f64) -> Option<u64> {
        use std::hash::{Hash, Hasher};
        let term = self.term.as_ref()?;
        let quiet = self.multi.is_none() && self.home_gesture.is_none() && self.transition.is_none() && !self.letterboxed(AppId::Terminal)
            && !self.keyboard.animating() && !self.kb_touch && self.term_touch.menu().is_none() && term.plain()
            && !self.hud.visible(t) && !self.input_hud.visible(t) && !self.perf.enabled && !self.assistive_visible() && self.drawn != Drawn::Nothing
            && self.clock.alert().is_none() && self.shot.is_none() && self.markup.is_none() && self.share.is_none() && self.share_prompt.is_none() && self.look_up.is_none()
            && self.low_alert.is_none() && self.power_menu.is_none() && self.permission_sheet.is_none() && self.crash_alert.is_none() && self.notify.ask.is_none() && !self.print_showing() && self.char_viewer.is_none() && self.keyboard_viewer.is_none() && self.a11y_panel.is_none() && !self.notify_damage_all() && !self.announce_damage() && !self.reader_damage();
        if !quiet {
            return None;
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.w, self.h, self.fullscreen, self.chrome_gen, self.last_input.to_bits(), self.keyboard.shown(), self.kb_top().to_bits(), term.area()).hash(&mut h);
        (self.term.len(), self.term.active_index()).hash(&mut h);
        for (i, tm) in self.term.iter().enumerate() {
            (tm.title(i), tm.unread, tm.bell).hash(&mut h);
        }
        Some(h.finish())
    }

    /// Settings' list page when only its scroll can differ from the last frame: the key of
    /// everything else (see `Settings::scroll_key`) and the offset. None while anything around
    /// the page moves (see `terminal_key`); the frame is then repainted in full.
    fn settings_key(&self, t: f64) -> Option<(u64, i32)> {
        let quiet = self.multi.is_none() && self.home_gesture.is_none() && self.transition.is_none() && !self.letterboxed(AppId::Settings)
            && !self.kb_touch && !self.fullscreen
            && !self.hud.visible(t) && !self.hud_drawn && !self.input_hud.visible(t) && !self.input_hud_drawn && !self.perf.enabled && !self.perf_drawn && !self.assistive_visible() && self.drawn != Drawn::Nothing
            && self.clock.alert().is_none() && self.shot.is_none() && self.markup.is_none() && self.share.is_none() && self.share_prompt.is_none() && self.look_up.is_none()
            && self.low_alert.is_none() && self.power_menu.is_none() && self.permission_sheet.is_none() && self.crash_alert.is_none() && self.notify.ask.is_none() && !self.print_showing() && self.char_viewer.is_none() && self.keyboard_viewer.is_none() && self.a11y_panel.is_none() && !self.notify_damage_all() && !self.announce_damage() && !self.reader_damage();
        if !quiet {
            return None;
        }
        let (key, off) = self.settings.as_ref()?.scroll_key(&self.ui, t)?;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        use std::hash::{Hash, Hasher};
        (key, self.w, self.h, self.chrome_gen).hash(&mut h);
        Some((h.finish(), off))
    }

    /// Scrolls the previous Settings frame in place by `d` px (content moves up by `d`): the
    /// pixels between the bar and the home indicator are moved, and only the newly exposed
    /// strip, the bar (blur over moving content) and the indicator zone are drawn. False
    /// (nothing touched) when the surface is not the whole frame.
    fn draw_settings_scrolled(&mut self, s: &mut Surface, t: f64, d: i32) -> bool {
        if s.y0 != 0 || s.w != self.w as usize || s.h != self.h as usize {
            return false;
        }
        let ui = &self.ui;
        let Some(st) = &self.settings else { return false };
        let Some(view) = st.page_view() else { return false };
        let (w, h, stride) = (s.w, s.h, s.stride);
        let top = (list::nav_bottom(ui).ceil() as usize + 1).min(h);
        let bot = h.saturating_sub(ui.pt(14.0).ceil() as usize).max(top);
        let n = d.unsigned_abs() as usize;
        let mut spans: Vec<(usize, usize)> = vec![];
        if n >= bot - top {
            spans.push((top, bot));
        } else if d > 0 {
            // rows [top + n, bot) move up to [top, bot - n)
            s.px.copy_within((top + n) * stride..(bot - 1) * stride + w, top * stride);
            spans.push((bot - n, bot));
        } else {
            s.px.copy_within(top * stride..(bot - n - 1) * stride + w, (top + n) * stride);
            spans.push((top, top + n));
        }
        if bot < h {
            spans.push((bot, h));
        }
        // the bar's blur needs the whole strip on one surface
        s.draw_rows(&[(0, top)], 1, &|s| view.draw(s, ui, t));
        s.draw_rows(&spans, threads(), &|s| view.draw(s, ui, t));
        self.draw_app_status(s);
        home_indicator(s, &self.ui, self.w, self.h, 255);
        true
    }

    fn assistive_visible(&self) -> bool {
        self.screen_on && !self.is_locked() && self.clock.alert().is_none() && self.assistive_touch.enabled()
    }

    /// The performance overlay's box under the status bar, in px.
    pub(super) fn perf_rect(&self) -> (f32, f32, f32, f32) {
        #[cfg(feature="desktop-wayland")]
        let top=if self.desk_active(){self.desktop_bar_height() as f32+self.ui.pt(4.)}else{self.ui.pt(scene::STATUS_PT+4.)};
        #[cfg(not(feature="desktop-wayland"))]
        let top=self.ui.pt(scene::STATUS_PT+4.);
        (self.ui.pt(8.0), top, self.ui.pt(150.0), self.ui.pt(4.0 * 14.0 + 10.0))
    }

    /// Frame drawn in `draw_ms`: feeds the performance HUD.
    pub fn perf_frame(&mut self, draw_ms: f64) {
        if self.perf.enabled {
            self.perf.frame(draw_ms);
        }
    }

    fn draw_perf(&mut self, s: &mut Surface) {
        self.perf_drawn = self.perf.enabled && self.screen_on;
        if !self.perf_drawn {
            return;
        }
        let (x, y, w, h) = self.perf_rect();
        s.fill_rrect(x, y, w, h, self.ui.pt(4.0), rgba(0, 0, 0, 200));
        for (i, line) in self.perf.lines().iter().enumerate() {
            let img = self.ui.text(line, 11.0, Weight::Medium, WHITE);
            s.blit(&img, (x + self.ui.pt(6.0)) as i32, (y + self.ui.pt(5.0 + 14.0 * i as f32)) as i32, 255, 255);
        }
    }

    fn draw_assistive(&mut self, s: &mut Surface, t: f64) {
        self.draw_perf(s);
        if self.assistive_visible() {
            self.assistive_touch.draw(s, &self.ui, self.w, self.h, t);
        }
    }

    fn assistive_action(&mut self, action: assistive_touch::Action, t: f64) -> Vec<Effect> {
        use assistive_touch::Action;
        match action {
            Action::Home => {
                self.cc.hide();
                self.notify_sleep();
                self.quicksearch.hide();
                self.close_home_overlays();
                if self.screen == Screen::Switcher {
                    self.switcher_sleep();
                    self.screen = Screen::Home;
                }
                self.go_home(t);
                self.leave_today();
                self.pager.home(t);
            }
            Action::AppSwitcher => self.open_app_switcher(t),
            Action::ControlCenter => self.open_control_center(t),
            Action::NotificationCenter => self.open_notification_center(t),
            Action::Lock => return self.sleep_screen(),
            Action::Screenshot => self.take_screenshot(t),
            Action::Rotate => {
                self.dirty = true;
                return self.toggle_rotation_lock(t);
            }
            Action::VolumeUp => {
                self.dirty = true;
                return self.volume_key(true, t);
            }
            Action::VolumeDown => {
                self.dirty = true;
                return self.volume_key(false, t);
            }
            Action::Device | Action::Back => {}
        }
        self.dirty = true;
        self.drawn = Drawn::Nothing;
        vec![]
    }

    fn scene(&self) -> Scene<'_> {
        Scene {
            layout: &self.layout,
            wallpaper: self.wallpaper(),
            dock_bg: if self.desk_active() { None } else { self.dock_bg.as_ref() },
            dock: if self.desk_active() { &[] } else { &self.dock },
            grid: if self.desk_active() { &[] } else { &self.home_grid },
            pager: &self.pager,
            chrome: &self.chrome,
            today: if self.desk_active() { None } else { Some(&self.today) },
            extras: if self.desk_active() { None } else { Some(self.extras()) },
        }
    }

    /// Where Now Playing is heard: the Bluetooth sink, else the wired or built-in route.
    fn route_label(&self) -> String {
        match crate::media::route::bluetooth_sink(&self.info.bt) {
            Some(n) => n.to_string(),
            None if self.cc.route == Route::Headphones => "Headphones".into(),
            None => "Speakers".into(),
        }
    }

    fn lock_view(&self) -> LockView<'_> {
        LockView { w: self.w, h: self.h, wallpaper: self.lock_wp(), ui: &self.ui, date: self.lock_date.as_ref(), time: self.lock_time.as_ref(), chrome: &self.chrome, music: self.svc_lock_card().filter(|_| self.svc_active()).map(|mut v| { v.route = self.route_label(); v }), notes: self.lock_draw(), status_inset: if self.login.is_some() { login::status_inset(&self.ui) } else { 0 } }
    }

    /// The open/close zoom card for the GPU compositor, when a plain one is
    /// playing (home screen, no overlay, no Reduce Motion fade); the same
    /// rect, radius and ease `draw` uses. Read-only.
    pub fn zoom_view(&self, t: f64) -> Option<ZoomView<'_>> {
        if self.zoom_blocker().is_some() {
            return None;
        }
        let ease = gestures::ease_out_quartic(self.progress(t));
        let full = Rect { x: 0, y: 0, w: self.w, h: self.h };
        let (rect, opening) = match self.anim? {
            (Anim::Open { rect, .. }, _) => (self.zoom_rect(rect, full, ease), true),
            (Anim::Close { from, rect }, _) => (self.zoom_rect(from, rect, ease), false),
            _ => return None,
        };
        let card = self.transition.as_ref()?;
        if Self::zoom_alpha(opening, ease) != 255 {
            return None;
        }
        Some(ZoomView { card, rect, radius: self.zoom_radius(rect.w as f32), opening })
    }

    /// Why the GPU zoom cannot play right now (None when it can); for the frame trace.
    pub fn zoom_blocker(&self) -> Option<&'static str> {
        if let Some(b) = self.overlay_blocker() {
            return Some(b);
        }
        [("not-home", self.screen != Screen::Home), ("home-gesture", self.home_gesture.is_some())].iter().find(|c| c.1).map(|c| c.0)
    }

    /// The overlays that paint over the app or home frame on the CPU, which the GPU
    /// compositors (zoom, switcher) cannot reproduce.
    pub(super) fn overlay_blocker(&self) -> Option<&'static str> {
        let checks: [(&'static str, bool); 18] = [
            ("markup", self.markup.is_some()),
            ("shield", self.shield.is_some()),
            ("control-center", self.cc_active()),
            ("notification-center", self.center_active()),
            ("quicksearch", self.quicksearch.active()),
            ("page-editor", self.page_editor.is_some()),
            ("gallery", self.gallery.is_some()),
            ("folder", self.folder_view.is_some()),
            ("assistive-touch", self.assistive_visible()),
            ("share", self.share.is_some()),
            ("share-prompt", self.share_prompt.is_some()),
            ("low-alert", self.low_alert.is_some()),
            ("permission-sheet", self.permission_sheet.is_some()),
            ("crash-alert", self.crash_alert.is_some()),
            ("compose-sheet", self.compose_showing()),
            ("power-menu", self.power_menu.is_some()),
            ("screenshot", self.shot.is_some()),
            ("clock-alert", self.clock.alert().is_some()),
        ];
        checks.iter().find(|c| c.1).map(|c| c.0)
    }

    /// What the next frame belongs to, for the slow-frame log: (screen, animation).
    pub fn jank_context(&self) -> (String, &'static str) {
        let screen = match self.screen {
            Screen::Lock => "lock".to_string(),
            Screen::Home => "home".to_string(),
            Screen::App(a) => format!("app:{a:?}").to_lowercase(),
            Screen::Switcher => "switcher".to_string(),
        };
        let anim = match self.anim {
            Some((Anim::Open { .. }, _)) => "open",
            Some((Anim::Close { .. }, _)) => "close",
            _ if self.turning() => "turn",
            _ if self.screen == Screen::Switcher || self.switcher.animating() => "switcher",
            _ if self.pager.animating() || self.today.animating() || self.home_animating() => "scroll",
            _ => "none",
        };
        (screen, anim)
    }

    /// Zoom state for the frame trace.
    pub fn zoom_debug(&self) -> String {
        format!("card={} reduce_motion={} screen_home={}", self.transition.is_some(), accessibility::reduce_motion(), self.screen == Screen::Home)
    }

    /// Whether an open/close zoom is in flight.
    pub fn zoom_active(&self) -> bool {
        matches!(self.anim, Some((Anim::Open { .. } | Anim::Close { .. }, _)))
    }

    /// The home screen alone (wallpaper, icons, badges) drawn into `s`.
    pub fn render_home(&self, s: &mut Surface) {
        self.draw_home(s);
    }

    /// Asks the app in front for a video layer before `draw`. `gpu` says whether the GPU can
    /// draw video at all (apps then stop converting pictures on their video threads); `compose`
    /// whether the caller can compose this frame (false withdraws a layer granted earlier in
    /// the same frame, without touching `gpu`). Some means
    /// the coming `draw` paints only the app's overlay, transparent where the picture goes, and
    /// the caller must compose the picture under it; None means `draw` paints a complete frame.
    /// Only a plain fullscreen app frame qualifies: no animation, turn, gesture, split view or
    /// system overlay that reads or replaces the app's pixels.
    pub fn video_layer(&mut self, gpu: bool, compose: bool) -> Option<crate::media::VideoLayer> {
        let plain = compose
            && self.screen_on
            && self.anim.is_none()
            && self.home_gesture.is_none()
            && self.multi.is_none()
            && !self.turning()
            && !self.cc_active()
            && !self.center_active()
            && !self.quicksearch.active()
            && self.page_editor.is_none()
            && self.gallery.is_none()
            && self.folder_view.is_none()
            && !(self.shield.is_some() && !self.st_suspended());
        let front = match self.screen {
            Screen::App(app) => Some(app),
            _ => None,
        };
        // a letterboxed app (portrait column in landscape) is not drawn in the logical frame
        // the layer's coordinates refer to
        let ok = |app: AppId, letterboxed: bool| plain && front == Some(app) && !letterboxed;
        let deildu_ok = ok(AppId::Deildu, self.letterboxed(AppId::Deildu));
        let files_ok = ok(AppId::Files, self.letterboxed(AppId::Files));
        // every player app hears `gpu`; only the one in front may hand out a layer
        let deildu = self.deildu.as_mut().and_then(|d| d.video_layer(gpu, deildu_ok));
        let files = self.files.as_mut().and_then(|f| f.video_layer(gpu, files_ok));
        match front {
            Some(AppId::Deildu) => deildu,
            Some(AppId::Files) => files,
            _ => None,
        }
    }

    /// Screen Zoom as the presenter draws it each frame (Option+Cmd+8 and the zoom steps).
    pub fn screen_zoom(&self) -> accessibility::zoom::ScreenZoom {
        self.screen_zoom
    }

    /// Cursor Size as a pointer scale: the open Settings' slider while it is open (not yet saved), else the saved value.
    pub fn cursor_scale(&self) -> f32 {
        match &self.settings {
            Some(st) => st.accessibility().cursor_scale(),
            None => accessibility::AccessibilitySettings::load(&self.prefs).cursor_scale(),
        }
    }

    /// Repaints everything on the next `draw` (after the GPU path drew frames the CPU did not see).
    pub fn force_full_redraw(&mut self) {
        self.perf_only = false;
        self.desk_regions = None;
        self.drawn = Drawn::Nothing;
        self.dirty = true;
    }

    /// The launch-zoom card `k` of the way from `from` to `to`.
    fn zoom_rect(&self, from: Rect, to: Rect, k: f32) -> Rect {
        if accessibility::reduce_motion() {
            // Reduce Motion: no zoom, the larger frame fades instead (see `zoom_alpha`)
            return if from.w >= to.w { from } else { to };
        }
        let lerp = |a: i32, b: i32| (a as f32 + (b as f32 - a as f32) * k).round() as i32;
        Rect { x: lerp(from.x, to.x), y: lerp(from.y, to.y), w: lerp(from.w, to.w), h: lerp(from.h, to.h) }
    }

    /// Opacity of the launch card: always opaque while it zooms, a
    /// cross-fade under Reduce Motion (`opening` fades it in, else out).
    fn zoom_alpha(opening: bool, k: f32) -> u32 {
        if !accessibility::reduce_motion() {
            return 255;
        }
        // the shared fade: the card is the incoming content when opening, the outgoing one when closing
        let fade = bifrost_ui::transition::Transition::fade().ease(bifrost_ui::transition::Ease::Linear);
        let fade = if opening { fade } else { fade.reversed() };
        (fade.placement(k, 0.0, 0.0).over_alpha * 255.0) as u32
    }

    /// The home screen in full (used by overlays that composite over it).
    pub(crate) fn draw_home(&self, s: &mut Surface) {
        if self.desk_active() {
            self.draw_desktop_background(s);
            return;
        }
        self.scene().draw(s);
        self.badge_view().draw(s);
    }

    fn draw_app(&mut self, s: &mut Surface, app: AppId, t: f64) {
        if !(app == AppId::Settings && self.draw_settings_banded(s, t)) {
            self.draw_app_body(s, app, t);
        }
        if !self.fullscreen {
            self.draw_app_status(s);
            home_indicator(s, &self.ui, self.w, self.h, 255);
        }
    }

    /// Draws Settings' list page in row bands over worker threads, then its overlays on this
    /// thread. False (nothing drawn) when Settings is mid-slide, letterboxed or not a plain list.
    fn draw_settings_banded(&mut self, s: &mut Surface, t: f64) -> bool {
        if self.letterboxed(AppId::Settings) || s.y0 != 0 || s.w != self.w as usize || s.h != self.h as usize {
            return false;
        }
        let ui = &self.ui;
        let Some(st) = &mut self.settings else { return false };
        let Some(view) = st.page_view() else { return false };
        s.draw_rows(&[(0, s.h)], threads(), &|s| view.draw(s, ui, t));
        st.draw_top(s, ui, t);
        true
    }

    /// The app's own pixels: laid out for `s`, or for the portrait panel and letterboxed in
    /// landscape (see `rotate`).
    fn draw_app_body(&mut self, s: &mut Surface, app: AppId, t: f64) {
        if self.letterboxed(app) {
            let (pw, ph) = self.portrait_size();
            self.size_player_app(app, pw, ph);
            self.draw_letterboxed(s, |sh, s| sh.draw_app_inner(s, app, t));
        } else {
            let (w, h) = (self.w, self.h);
            self.size_player_app(app, w, h);
            self.draw_app_inner(s, app, t);
        }
    }

    /// A portrait-only app is letterboxed in landscape, except while it shows the fullscreen
    /// video player: video uses the whole landscape screen.
    fn letterboxed(&self, app: AppId) -> bool {
        let playing = match app {
            AppId::Deildu => self.deildu.as_ref().is_some_and(|d| d.fullscreen()),
            AppId::Files => self.files.as_ref().is_some_and(|f| f.fullscreen()),
            _ => false,
        };
        rotate::portrait_only(app) && !playing
    }

    /// Deildu lays itself out for the size it is drawn at (portrait column or full landscape).
    fn size_player_app(&mut self, app: AppId, w: i32, h: i32) {
        match app {
            AppId::Deildu => {
                if let Some(d) = &mut self.deildu {
                    d.set_size(w, h);
                }
            }
            AppId::Files => {
                if let Some(f) = &mut self.files {
                    f.set_size(w, h);
                }
            }
            _ => {}
        }
    }

    /// Sizes Deildu or Files for the surface they are drawn at right now (letterboxed portrait
    /// column, or the full screen while the player is up), ahead of a touch or tick reaching them.
    fn size_current_player_app(&mut self) {
        if let Screen::App(app @ (AppId::Deildu | AppId::Files)) = self.screen {
            let (w, h) = if self.letterboxed(app) { self.portrait_size() } else { (self.w, self.h) };
            self.size_player_app(app, w, h);
        }
    }

    fn draw_app_inner(&mut self, s: &mut Surface, app: AppId, t: f64) {
        match app {
            AppId::Settings => {
                if let Some(st) = &mut self.settings {
                    st.draw(s, &self.ui, t);
                }
            }
            AppId::Terminal => self.draw_terminal(s, t),
            AppId::Deildu => match &mut self.deildu {
                Some(d) => {
                    self.fullscreen = d.fullscreen();
                    d.draw(s, &self.ui, t);
                }
                None => s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, BLACK),
            },
            AppId::Photos => match &mut self.photos {
                Some(p) => {
                    self.fullscreen = p.fullscreen();
                    p.draw(s, &self.ui, t);
                }
                None => s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, BLACK),
            },
            AppId::Files => match &mut self.files {
                Some(f) => {
                    self.fullscreen = f.fullscreen();
                    f.draw(s, &self.ui, t);
                }
                None => s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, BLACK),
            },
            AppId::External(n) => self.ext_draw(s, n as usize, t),
            AppId::AppStore => match &mut self.store {
                Some(c) => c.draw(s, &self.ui, t),
                None => s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, BLACK),
            },
            AppId::Camera => match &mut self.camera {
                Some(c) => c.draw(s, &self.ui, t),
                None => s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, BLACK),
            },
            AppId::Magnifier => match &mut self.magnifier {
                Some(c) => c.draw(s, &self.ui, t),
                None => s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, BLACK),
            },
            AppId::Browser => match &mut self.browser {
                Some(b) => b.draw(s, &self.ui, t),
                None => s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, BLACK),
            },
        }
    }

    fn draw_app_status(&self, s: &mut Surface) {
        #[cfg(feature="desktop-wayland")]
        if self.desk_active() && !self.is_locked() {self.draw_desktop_bar(s);return;}
        let ui = &self.ui;
        let mid = (ui.pt(scene::STATUS_PT) / 2.0) as i32;
        // time then date, as the bifrost status bar shows them in apps too
        let mut x = ui.pt(scene::STATUS_LEFT_PT) as i32;
        for img in [&self.chrome.time, &self.chrome.date].into_iter().flatten() {
            s.blit(img, x, mid - img.h as i32 / 2, 255, 255);
            x += img.w as i32 + ui.pt(7.5) as i32;
        }
        scene::draw_status_right(s, self.w, mid, ui.s, &self.chrome);
    }
}

/// Feeds a touch to the Today page and the pager it rides on: the page
/// takes vertical drags and taps, the pager horizontal swipes.
fn today_page_touch(today: &mut Today, pager: &mut Pager, ev: Event, t: f64) -> Option<Action> {
    match ev {
        Event::Down(x, y) => {
            pager.down(x, y, t, None);
            today.down(x, y, t);
            None
        }
        Event::Move(x, y) => {
            if today.moved(x, y, t) {
                pager.cancel(t);
            } else if !today.owns() {
                pager.moved(x, y, t);
            }
            None
        }
        Event::Up(x, y) => {
            if today.owns() {
                pager.cancel(t);
                today.up(x, y, t)
            } else {
                today.up(x, y, t);
                pager.up(x, y, t, None);
                None
            }
        }
        Event::Power | Event::Wake | Event::VolumeUp | Event::VolumeDown | Event::Screenshot => None,
    }
}

/// One frame of the open/close zoom, for the GPU compositor.
pub struct ZoomView<'a> {
    pub card: &'a Image,
    pub rect: Rect,
    pub radius: f32,
    pub opening: bool,
}

/// The bifrost launch zoom: the app's frame (a black card before it has one)
/// in `rect` with corner radius `r`.
fn draw_zoom(transition: Option<&Image>, s: &mut Surface, rect: Rect, r: f32, alpha: u32) {
    match transition {
        Some(img) => switcher::draw_scaled(s, img, rect, r, alpha),
        None => s.fill_rrect(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32, r, BLACK),
    }
}

/// The Terminal app: a pty shell that keeps running at home, typed into
/// with the docked keyboard.
impl Shell {
    /// Fds to poll (the pty, Deildu's and Files' job pipes); call `pump` when one is readable.
    pub fn poll_fds(&self) -> Vec<std::ffi::c_int> {
        self.term.iter().map(|t| t.fd()).chain(self.deildu.iter().flat_map(|d| d.fds())).chain(self.photos.iter().flat_map(|p| p.fds())).chain(self.files.iter().flat_map(|f| f.fds())).chain([self.audio.fd()]).chain([if self.desk_active(){desktop_network::fd()}else{-1}]).chain(self.store.iter().flat_map(|n| n.fds())).chain(self.ext_fds()).chain(keyboard::poll_fd()).collect()
    }

    /// `poll_fds` without the audio-service client's always-present fd.
    #[cfg(test)]
    pub(crate) fn app_poll_fds(&self) -> Vec<std::ffi::c_int> {
        let audio = self.audio.fd();
        self.poll_fds().into_iter().filter(|f| *f != audio).collect()
    }

    /// Feeds shell output to the terminal and finished jobs to Deildu; true
    /// if either changed. Repaints only when that app is on screen; an
    /// exited shell is dropped.
    pub fn pump(&mut self) -> bool {
        let mut changed = false;
        let (c, exits) = self.term.pump();
        if exits > 0 { self.notify_term_exit(); }
        if c && self.screen == Screen::App(AppId::Terminal) { self.dirty = true; }
        if let Some(text) = self.term.as_mut().and_then(|t| t.take_clipboard()) { clipboard::set_text(&text); }
        self.today_stale |= c;
        changed |= c;
        if let Some(d) = &mut self.deildu {
            let c = d.pump(&self.ui);
            if c && self.screen == Screen::App(AppId::Deildu) {
                self.dirty = true;
            }
            changed |= c;
        }
        if let Some(p) = &mut self.photos {
            let c = p.pump(&self.ui);
            if c && self.screen == Screen::App(AppId::Photos) {
                self.dirty = true;
            }
            changed |= c;
        }
        if let Some(f) = &mut self.files {
            let c = f.pump(&self.ui);
            if c && self.screen == Screen::App(AppId::Files) {
                self.dirty = true;
            }
            changed |= c;
        }
        if let Some(n) = &mut self.store {
            let c = n.pump();
            if c && self.screen == Screen::App(AppId::AppStore) {
                self.dirty = true;
            }
            changed |= c;
        }
        if self.store.as_mut().is_some_and(|s|s.take_registry_changed()) && apphost::refresh() {
            self.installed_changed();
        }
        changed |= self.audio_pump();
        changed
    }

    /// Top of the docked keyboard, which sits above the home-indicator zone.
    fn kb_top(&self) -> f32 {
        self.h as f32 - home_zone(&self.ui) - self.keyboard.h()
    }

    /// Terminal px rect: below the status bar, above the keyboard or the home zone.
    fn term_area(&self) -> (i32, i32, i32, i32) {
        let top = self.ui.pt(scene::STATUS_PT + terminal::HEADER_PT + self.terminal_ui.search_height()).round() as i32;
        let bottom = if self.keyboard.shown() { self.kb_top() } else { self.h as f32 - home_zone(&self.ui) } as i32;
        (0, top, self.w, bottom - top)
    }

    /// Shows the keyboard and (re)launches the shell if it is not running.
    fn open_terminal(&mut self, t: f64) {
        // set_keyboard refuses where the form factor has no on-screen keyboard
        self.set_keyboard(true, t);
        if self.term.as_ref().is_some_and(|t| t.alive()) {
            return;
        }
        self.new_terminal_tab(None);
    }

    fn new_terminal_tab(&mut self, dir: Option<String>) -> bool {
        self.spawn_terminal_tab(self.term_cmd, dir)
    }

    /// Opens a tab running `argv` (the login shell, or a New command line); `dir` is cd'd into first.
    fn spawn_terminal_tab(&mut self, argv: &[&str], dir: Option<String>) -> bool {
        let size = self.ui.pt(termcore::zoom::SIZES[self.terminal_ui.settings.zoom] as f32);
        match Terminal::spawn(self.mono, self.term_area(), size, argv) {
            Ok(mut term) => {
                term.appearance(self.terminal_ui.settings.theme, size);
                term.set_cursor(Some(self.terminal_ui.cursor.shape()), self.terminal_ui.blink);
                if let Some(dir) = dir { term.write(format!("cd '{}'\r", dir.replace('\'', "'\\''")).as_bytes()); }
                self.term.push(term);
                self.term_touch = termtouch::TermTouch::default();
                true
            }
            Err(e) => { self.terminal_ui.message = format!("Could not open shell: {e}"); false },
        }
    }

    /// An input-method batch from the window server. Focus on a Wayland client's text field raises the on-screen
    /// keyboard (the form policy decides whether it shows), and losing focus lowers it. The field's text and purpose
    /// are kept for the keyboard's requests (`type_out`, `ext_forward_composition`).
    pub fn input_method_batch(&mut self, batch: textinput::ImBatch, t: f64) {
        match self.im.apply(batch) {
            Some(true) => self.set_keyboard(true, t),
            Some(false) => self.set_keyboard(false, t),
            None => {}
        }
        self.dirty = true;
    }

    /// The requests the keyboard made for a Wayland client's field since the last call; the window server's input
    /// method sends them (`bare::kms`).
    pub fn take_input_method_out(&mut self) -> Vec<textinput::ImOut> {
        std::mem::take(&mut self.im_out)
    }

    /// Whether the on-screen keyboard may be raised now: the form factor, the UI in front and any attached
    /// hardware keyboard decide.
    pub(super) fn keyboard_allowed(&self) -> bool {
        self.form.allows_onscreen_keyboard(self.desk_active(), self.hw_keyboard)
    }

    /// The form factor this shell runs as.
    pub fn form(&self) -> bifrost_ui::form_factor::FormFactor {
        self.form
    }

    /// A physical keyboard attached or removed. Attaching hides a raised on-screen keyboard and keeps it down while
    /// the keyboard stays; removing it lets text fields raise it again (they do so on the next focus).
    pub fn set_hardware_keyboard(&mut self, attached: bool, t: f64) {
        if self.hw_keyboard == attached {
            return;
        }
        self.hw_keyboard = attached;
        if attached {
            self.set_keyboard(false, t);
            if matches!(self.screen, Screen::App(AppId::External(_))) {
                self.ext_set_keyboard(false, t);
            }
        }
        self.dirty = true;
    }

    /// Slides the keyboard in or out; the terminal takes its final size at once.
    fn set_keyboard(&mut self, shown: bool, t: f64) {
        // A desktop has a hardware keyboard and Golden Gate has no on-screen one: never raise it there. A tablet
        // raises it with the desktop UI up too, unless a hardware keyboard is attached. Guarding here, not in each
        // caller, matters: bringing an app with a focused text field to the front used to raise it, and its top row
        // showed as grey boxes along the bottom of the window.
        let shown = shown && self.keyboard_allowed();
        if self.keyboard.shown() != shown {
            self.keyboard.set_shown(shown, t);
            self.resize_terminals();
        }
        // the desktop's front window follows the keyboard up and back down
        self.desk_keyboard_lift();
    }

    fn terminal_touch(&mut self, ev: Event, t: f64) {
        if self.terminal_chrome_touch(ev, t) { return; }
        let top = self.kb_top();
        match ev {
            Event::Down(x, y) if self.keyboard.shown() && y >= top => {
                self.kb_touch = true;
                self.keyboard.down(x, y - top, t);
            }
            Event::Down(x, y) => {
                self.select_pane_at(x);
                if let Some(term) = self.term.as_mut() {
                    self.term_touch.down(&self.ui, term, x, y, t);
                }
            }
            Event::Move(x, y) if self.kb_touch => self.keyboard.moved(x, y - top),
            Event::Up(x, y) if self.kb_touch => {
                self.kb_touch = false;
                if let Some(out) = self.keyboard.up(x, y - top, t) {
                    self.type_out(out, t);
                }
            }
            Event::Move(x, y) => {
                if let Some(term) = self.term.as_mut() {
                    self.term_touch.moved(&self.ui, term, x, y);
                }
            }
            Event::Up(x, y) => {
                let tap = match self.term.as_mut() {
                    Some(term) => self.term_touch.up(&self.ui, term, x, y, self.w as f32),
                    None => false,
                };
                if let Some(url) = self.term_touch.take_link() {
                    self.terminal_ui.message = match self.desktop_open_url(&url, t) {
                        Ok(()) => "Opened link".into(),
                        Err(e) => format!("Could not open link: {e}"),
                    };
                    self.dirty = true;
                }
                if tap {
                    self.set_keyboard(true, t);
                }
            }
            Event::Power | Event::Wake | Event::VolumeUp | Event::VolumeDown | Event::Screenshot => {}
        }
    }

    fn type_out(&mut self, out: Out, t: f64) {
        // a Wayland client's text field has focus: the keyboard types into it through the window server's input method
        if self.im.active {
            if out == Out::Hide {
                self.set_keyboard(false, t);
            } else {
                let requests = textinput::outs_for(&self.im, &out);
                self.im_out.extend(requests);
            }
            return;
        }
        if matches!(self.screen, Screen::App(AppId::External(_))) {
            self.ext_type_out(out, t);
            return;
        }
        if out == Out::Hide {
            self.set_keyboard(false, t);
            return;
        }
        // the App Store's Search field takes the typing; nothing else on that screen does
        if self.screen == Screen::App(AppId::AppStore) {
            if let Some(c) = self.store.as_mut() {
                c.key(out);
            }
            return;
        }
        if self.terminal_panel_input(&out, t) { return; }
        let Some(term) = self.term.as_mut() else { return };
        self.term_touch.dismiss();
        match out {
            Out::Text(s) => term.write(s.as_bytes()),
            Out::Backspace => term.key("Backspace"),
            Out::Enter => term.key("Enter"),
            Out::Named("Paste") => {
                if let Some(text) = clipboard::text() {
                    term.paste(&text);
                }
            }
            Out::Named(n) => term.key(n),
            Out::Hide => {}
        }
    }

    fn draw_terminal(&mut self, s: &mut Surface, t: f64) {
        // the canvas covers the area, so only the strips around it need paint
        let (_, top, w, h) = self.term_area();
        let bg = term::theme_background(self.terminal_ui.settings.theme);
        let (w, top, bottom) = (w as usize, top as usize, (top + h) as usize);
        s.fill_rect(0, 0, w, top, bg);
        if self.term.is_some() {
            self.draw_panes(s);
            self.term_touch.draw(s, &self.ui);
        } else {
            s.fill_rect(0, top, w, bottom - top, bg);
        }
        s.fill_rect(0, bottom, w, self.h as usize, bg);
        self.draw_terminal_chrome(s, t);
        if self.keyboard.shown() || self.keyboard.animating() {
            let kb_top = self.kb_top();
            self.keyboard.draw(s, &self.ui, kb_top, t);
            let below = (kb_top + self.keyboard.h() + self.keyboard.offset(t)) as usize;
            s.fill_rect(0, below, w, self.h as usize, keyboard::BG);
        }
    }
}

/// Photos and screenshots.
impl Shell {
    /// Starts the camera once its app is on screen, releases it as soon as it is not, and opens a
    /// tapped last-photo thumbnail in Photos. The app touches no device before the user is in it.
    fn tick_magnifier(&mut self, t: f64) {
        let on = self.screen_on && self.screen == Screen::App(AppId::Magnifier);
        if let Some(m) = &mut self.magnifier {
            if on {
                m.resume();
                self.dirty |= m.tick(&self.ui, t);
            } else {
                m.stop();
            }
        }
    }

    fn tick_camera(&mut self, t: f64) {
        let on_camera = self.screen_on && self.screen == Screen::App(AppId::Camera);
        let mut open = None;
        if let Some(c) = &mut self.camera {
            if on_camera {
                c.resume(&self.ui);
                self.dirty |= c.tick(&self.ui, t);
                open = c.take_open_photo();
            } else {
                c.stop();
            }
        }
        if let Some(path) = open {
            self.show_in_photos(&path);
        }
    }

    /// Creates the Photos app on first use, or rescans its folders.
    fn ensure_photos(&mut self) {
        match &mut self.photos {
            Some(p) => p.rescan(),
            None => {
                let offset = self.wall.offset;
                match Photos::new(&self.ui, self.portrait_size().0, self.portrait_size().1, self.data_dir.clone(), self.photo_dirs.clone(), offset) {
                    Ok(p) => self.photos = Some(p),
                    Err(e) => eprintln!("springboard: photos: {e}"),
                }
            }
        }
    }

    /// Saves the screen as a PNG in the Pictures folder (on a thread), shows
    /// it in Photos at once and plays the capture animation.
    /// A swipe up and inwards from the bottom-left corner, when General › Gestures allows it.
    fn corner_swipe(&mut self, ev: Event) -> bool {
        let zone = self.ui.pt(40.0);
        match ev {
            Event::Down(x, y) => {
                self.corner_start = (self.options().corner_swipe && x < zone && y > self.h as f32 - zone).then_some((x, y));
                false
            }
            Event::Up(x, y) => self.corner_start.take().is_some_and(|(x0, y0)| x - x0 > zone && y0 - y > zone),
            _ => false,
        }
    }

    /// Draws the whole screen into a fresh image (screenshots and the display capture service).
    pub(crate) fn capture_frame(&mut self, t: f64) -> Image {
        let mut img = Image::new(self.w as usize, self.h as usize);
        self.drawn = Drawn::Nothing;
        self.draw(&mut Surface { w: img.w, h: img.h, stride: img.w, px: &mut img.px, y0: 0 }, t);
        self.drawn = Drawn::Nothing;
        self.dirty = true;
        img
    }

    fn take_screenshot(&mut self, t: f64) {
        let whole = crate::layout::Rect { x: 0, y: 0, w: self.w as i32, h: self.h as i32 };
        self.take_capture(whole, None, t);
    }

    /// Captures `region` of the screen (the whole screen when it is all of it), draws the pointer into it when `pointer`
    /// names where it is, and saves it with the capture animation. The frame is drawn without the toolbar or the old
    /// thumbnail.
    fn take_capture(&mut self, region: crate::layout::Rect, pointer: Option<(f32, f32)>, t: f64) {
        self.shot = None;
        // the toolbar is set aside while the frame is drawn, so it stays out of the picture
        let open = self.capture.take();
        let full = self.capture_frame(t);
        self.capture = open;
        let whole = region.x == 0 && region.y == 0 && region.w as usize == full.w && region.h as usize == full.h;
        let mut img = if whole { full } else { full.crop(region.x as usize, region.y as usize, region.w as usize, region.h as usize) };
        if let Some((x, y)) = pointer {
            capture::draw_pointer(&mut img, x - region.x as f32, y - region.y as f32, self.ui.s);
        }
        self.save_capture(img, t);
    }

    /// Saves a captured picture to Photos and starts the thumbnail's animation over it.
    fn save_capture(&mut self, img: Image, t: f64) {
        let (unix, offset) = self.wall_now(t);
        let name = screenshot::file_name(&photos::library::civil(unix, offset));
        let dir = self.photo_dirs.first().cloned().unwrap_or_else(|| self.data_dir.join("Pictures"));
        let bytes = screenshot::encode(&img);
        let path = dir.join(&name);
        let photo = photos::library::Photo { path: path.clone(), mtime: unix, size: bytes.len() as u64 };
        crate::bare::sys::spawn_named("photo-save", move || {
            crate::gfx::affinity::background_thread();
            if let Err(e) = screenshot::write(&dir, &name, &bytes, unix) {
                eprintln!("springboard: screenshot: {e}");
            }
        });
        if self.photos.is_none() {
            self.ensure_photos();
        }
        if let Some(p) = &mut self.photos {
            p.add_photo(&self.ui, photo, img.clone());
        }
        self.shot = Some(Overlay::new(img, path, self.ui.s, t));
    }

    /// The windows on the desk that a window capture can pick, back to front, as layout rectangles.
    fn capture_frames(&self) -> Vec<crate::layout::Rect> {
        self.desk.as_ref().map_or_else(Vec::new, |d| {
            d.wins.iter().filter(|w| !w.minimized).map(|w| crate::layout::Rect { x: w.r.x, y: w.r.y, w: w.r.w, h: w.r.h }).collect()
        })
    }

    /// Opens the toolbar's capture session with `mode` (`select`, `window` or `screen`): `screencapture -i`, `-w`, or the
    /// whole screen, from the control port. The capture is saved to Photos when the user finishes the selection.
    pub fn capture_request(&mut self, mode: &str) -> Result<(), String> {
        if !self.screen_on || self.is_locked() {
            return Err("locked".into());
        }
        let pick = match mode {
            "select" => capture::Pick::Region,
            "window" => capture::Pick::Window,
            "screen" => capture::Pick::Screen,
            other => return Err(format!("no capture mode {other}")),
        };
        let mut session = capture::Session::with_prefs(self.capture_prefs);
        session.pick = pick;
        self.capture = Some(session);
        self.dirty = true;
        Ok(())
    }

    /// A touch while the toolbar's session is open: its buttons, or else the screen, where a press starts a selection
    /// (or picks a window or the screen) and a release takes the capture, after its timer.
    fn capture_touch(&mut self, ev: Event, t: f64) -> Vec<Effect> {
        let (w, h, s) = (self.w as i32, self.h as i32, self.ui.s);
        let frames = self.capture_frames();
        let Some(sess) = self.capture.as_mut() else { return vec![] };
        if sess.pending.is_some() {
            // the countdown runs; the screen waits for it
            return vec![];
        }
        match ev {
            Event::Down(x, y) => {
                if let Some(row) = sess.option_at(&self.ui, w, h, x, y) {
                    sess.prefs.choose(row);
                    self.capture_prefs = sess.prefs;
                    if row == capture::OptionRow::Microphone {
                        self.ask_microphone();
                    }
                } else if sess.on_menu(&self.ui, w, h, x, y) {
                    // the card's padding does nothing
                } else if sess.menu && !sess.on_toolbar(&self.ui, w, h, x, y) {
                    // a press outside the open menu only closes it
                    sess.menu = false;
                } else {
                    match sess.tool_at(&self.ui, w, h, x, y) {
                        Some(tool) => sess.pressed = Some(tool),
                        None => sess.press(x, y),
                    }
                }
            }
            Event::Move(x, y) => sess.moved(x, y),
            Event::Up(x, y) => {
                if let Some(pressed) = sess.pressed.take() {
                    if sess.tool_at(&self.ui, w, h, x, y) == Some(pressed) && sess.apply(pressed) {
                        self.capture = None;
                    }
                    return vec![];
                }
                if sess.on_toolbar(&self.ui, w, h, x, y) {
                    return vec![];
                }
                if let Some(r) = sess.release(x, y, &frames, w, h, s) {
                    sess.pending = Some((r, capture::Countdown::new(t, sess.prefs.timer)));
                    if sess.due(t) {
                        self.finish_capture(t);
                    }
                }
            }
            _ => {}
        }
        vec![]
    }

    /// Ends the toolbar's session and takes its chosen capture (its countdown has run out).
    fn finish_capture(&mut self, t: f64) {
        let Some(sess) = self.capture.take() else { return };
        let Some((region, _)) = sess.pending else { return };
        if sess.record {
            let whole = region.x <= 0 && region.y <= 0 && region.w >= self.w as i32 && region.h >= self.h as i32;
            let area = (!whole).then(|| (region.x.max(0) as u32, region.y.max(0) as u32, region.w.max(1) as u32, region.h.max(1) as u32));
            if let Err(e) = self.recording_start(area, None, t) {
                eprintln!("springboard: recording: {e}");
            }
            return;
        }
        let pointer = sess.prefs.pointer.then_some(sess.at);
        self.take_capture(region, pointer, t);
    }

    /// A touch on the screenshot thumbnail; true when it was consumed. A tap opens the capture in Photos (not from the
    /// lock screen); a drag moves the thumbnail and is not a tap.
    fn shot_touch(&mut self, ev: Event, t: f64) -> bool {
        match ev {
            Event::Down(x, y) if self.shot.as_ref().is_some_and(|o| o.hit(x, y, t)) => {
                self.shot_tap = true;
                if let Some(o) = self.shot.as_mut() {
                    o.grab(x, y);
                }
                true
            }
            Event::Move(x, y) if self.shot_tap => {
                if let Some(o) = self.shot.as_mut() {
                    o.drag_to(x, y);
                }
                true
            }
            Event::Up(x, y) if self.shot_tap => {
                self.shot_tap = false;
                if self.shot.as_mut().is_some_and(|o| o.release()) {
                    self.dirty = true;
                    return true;
                }
                if let Some(o) = self.shot.take().filter(|o| o.hit(x, y, t)) {
                    if matches!(self.screen, Screen::Lock | Screen::Switcher) {
                        return true;
                    }
                    self.markup = Some(markup::Markup::new(o.frame().clone(), o.path, self.ui.s));
                    self.dirty = true;
                }
                true
            }
            _ => false,
        }
    }

    /// Preview's Markup button: opens a picture in the editor; Done saves
    /// a PNG (a copy named "... Markup.png" unless it already is a PNG).
    fn markup_file(&mut self, path: &Path) {
        // the file read and the decode can take a while on a large picture, so they run on a worker;
        // `markup_tick` opens the editor when the picture lands
        let source = path.to_path_buf();
        let (tx, rx) = std::sync::mpsc::channel();
        crate::bare::sys::spawn_named("markup-load", move || {
            let _ = tx.send(std::fs::read(&source).ok().and_then(|b| crate::cg::imageio::decode(&b)));
        });
        self.markup_load = Some((path.to_path_buf(), rx));
    }

    /// Opens the Markup editor once the worker has decoded the picture (a failed read or decode opens nothing).
    fn markup_tick(&mut self) {
        let Some((_, rx)) = &self.markup_load else { return };
        let decoded = match rx.try_recv() {
            Ok(decoded) => decoded,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
        };
        let Some((path, _)) = self.markup_load.take() else { return };
        let Some(img) = decoded else { return };
        let target = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")) {
            path
        } else {
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            path.with_file_name(format!("{stem} Markup.png"))
        };
        self.markup = Some(markup::Markup::new(img, target, self.ui.s));
        self.dirty = true;
    }

    /// Touches go to the Markup editor while it is open; Done saves the
    /// marked-up PNG over the capture and refreshes it in Photos.
    fn markup_touch(&mut self, ev: Event, t: f64) -> bool {
        let Some(m) = &mut self.markup else { return false };
        self.dirty = true;
        match m.touch(&self.ui, ev, t) {
            Some(markup::Outcome::Done) => {
                if let Some(m) = self.markup.take() {
                    let img = m.image().clone();
                    let bytes = screenshot::encode(&img);
                    let (unix, _) = self.wall_now(t);
                    let path = m.path;
                    let dir = path.parent().map(PathBuf::from).unwrap_or_default();
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let photo = photos::library::Photo { path: path.clone(), mtime: unix, size: bytes.len() as u64 };
                    if let Err(e) = screenshot::write(&dir, &name, &bytes, unix) {
                        eprintln!("springboard: markup: {e}");
                    }
                    self.ensure_photos();
                    if let Some(p) = &mut self.photos {
                        p.add_photo(&self.ui, photo, img);
                    }
                }
            }
            Some(markup::Outcome::Cancel) => self.markup = None,
            None => {}
        }
        true
    }
}

/// The lock screen's parts, borrowed so worker threads can draw it.
struct LockView<'a> {
    w: i32,
    h: i32,
    wallpaper: View<'a>,
    ui: &'a Ui,
    date: Option<&'a Image>,
    time: Option<&'a Image>,
    chrome: &'a Chrome,
    notes: notify::LockDraw<'a>,
    /// the Now Playing card while a song is loaded
    music: Option<music::LockCardView<'a>>,
    /// room kept at the right of the status icons for the login window's power button (0 without it)
    status_inset: i32,
}

impl LockView<'_> {
    fn draw(&self, s: &mut Surface, lift: f32) {
        let dy = -lift.round() as i32;
        let wp = self.wallpaper;
        s.copy(wp, (self.w - wp.w as i32) / 2, dy);
        let cx = self.w / 2;
        let ui = self.ui;
        // the date above a large clock, as the reference's login window and lock screen draw them
        let mut clock_top = dy + ui.pt(52.0) as i32;
        if let Some(d) = self.date {
            centred(s, d, cx, clock_top);
            clock_top += d.h as i32;
        }
        if let Some(tm) = self.time {
            centred(s, tm, cx, clock_top - ui.pt(6.0) as i32);
        }
        self.notes.draw(s, ui, self.w, dy);
        let dots_y = (dy + self.h) as f32 - ui.pt(26.0);
        // a keyboard machine unlocks with Return, not a swipe; the login window asks for the password instead
        if self.status_inset == 0 {
            let hint = ui.text("Press Return to unlock", 15.0, Weight::Regular, rgba(255, 255, 255, 200));
            centred(s, &hint, cx, dots_y as i32);
        }
        if let Some(card) = &self.music {
            card.draw(s, ui, dy);
        }
        // the bifrost mini (Home button) shows no home indicator on the lock screen
        scene::draw_status_right(s, self.w - self.status_inset, dy + (ui.pt(scene::STATUS_PT) / 2.0) as i32, ui.s, self.chrome);
    }
}

/// The date over the lock clock, short as the reference has it: "Thu Sep 10".
fn lock_date_text(i: &Info) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{} {} {}", DAYS[i.wday.rem_euclid(7) as usize], MONTHS[i.mon.rem_euclid(12) as usize], i.mday)
}


#[cfg(any(test, feature = "testkit"))]
pub mod tests;

#[cfg(test)]
pub(crate) mod frame_budget;

#[cfg(test)]
mod term_fast_tests;

/// Average `draw_quicksearch` cost, logged to stderr (the journal) every 120 frames.
/// Measures the glass card's per-frame cost on the desktop; see docs/desktop-chrome.md.
static QUICKSEARCH_US: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static QUICKSEARCH_N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn quicksearch_draw_timing(d: std::time::Duration) {
    use std::sync::atomic::Ordering::Relaxed;
    QUICKSEARCH_US.fetch_add(d.as_micros() as u64, Relaxed);
    if QUICKSEARCH_N.fetch_add(1, Relaxed) + 1 == 120 {
        let avg = QUICKSEARCH_US.swap(0, Relaxed) as f64 / 120.0 / 1000.0;
        QUICKSEARCH_N.store(0, Relaxed);
        eprintln!("springboard: quicksearch draw avg {avg:.2} ms over 120 frames");
    }
}

#[cfg(test)]
mod idle_tests {
    use super::*;
    #[test]
    fn ambient_fields_alone_are_not_a_visible_change() {
        let a = Info::default();
        let mut b = a.clone();
        b.uptime_min += 1;
        b.storage_free += 4096;
        b.memory = "1.2 GB".into();
        b.brightness = 0.3;
        assert!(ambient_only(&a, &b));
        b.min += 1;
        assert!(!ambient_only(&a, &b));
        let mut c = a.clone();
        c.battery = 50;
        assert!(!ambient_only(&a, &c));
    }
}
