//! The Settings app: a navigation stack of `list` pages (plus the
//! Wallpaper grid) built from `pages::spec`, with the bifrost push/pop slide,
//! the Wi-Fi password sheet, confirmation alerts, and the dispatch of every
//! row's action into persisted `Options` and `Effect`s for the device.

use super::accessibility::shortcut::Feature;
use super::accessibility::{AccessibilitySettings, HOLD_MS, ZOOMS};
use super::datetime_page::{PickerEvent, ZonePicker};
use super::timeset_page::{TimeEvent, TimePicker};
use super::keyboard::Out;
use super::settings_load::Loaded;
use super::settings_search::{self, Search};
use super::night_shift::{NightEdit, NightShift};
use super::notify_page::{self, NotifyChange, NotifyEdit, NotifyPrefs};
use super::list::{Group, Hit, Item, ListPage, NavBar, Pane, DETAIL_MARGIN_PT, SIDEBAR_MARGIN_PT, SIDEBAR_PT};
use super::options::{Options, AUTO_LOCK, GAME_MODES, REQUIRE_AFTER, SYSTEM_SLEEP, ICON_SIZES, POWER_MODES, TEXT_SIZES};
use super::pages::{spec, Action, Command, Model, PageId, Pick, Slider, Toggle};
use super::power_menu::PowerAction;
use super::passcode::{self, Lock, LockOut, Overlay, OverlayOut};
use super::screentime::{self, config::Config as StConfig, page::{StEdit, View as StView}, usage as st_usage};
use super::prefs::Prefs;
use super::storage::Scanner;
use super::pacman_update::Removal;
use bifrost_ui::transition::{Ease, Subtype, Transition};
use super::storage_page::{self, DevState, PageState};
use super::update::{self, Updater};
use super::{backup, backup_page, diag, diagnostics_page};
use super::ui::{AlertCard, Ui, Weight, CONTENT_BG, SEPARATOR_C, SIDEBAR_BG};
use super::live_wallpaper::{Look, LOOK_PREF};
use super::wallpaper_page::{Import, ShowOn, Tap, Target, Wallpaper, WallpaperPage, SHOW_ON_PREF};
use super::controls::{notch, slider_value};
pub use super::list::fill_row;
use super::wifi_page::{choose, Choice, Sheet, SheetHit, BLUE};
use super::{Effect, Event, Info, WifiAction};
use crate::bare::bluetooth::{Cmd as BtCmd, State as BtState};
use crate::gfx::{Image, Surface};
use std::path::PathBuf;
use std::rc::Rc;

pub use super::list::{GROUP_GAP_PT, MARGIN_PT, RADIUS_PT, ROW_PT};
pub use super::list::HIGHLIGHT;
/// The rows Search offers as "Settings" results, by their names.
pub const PAGES: [&str; 10] = ["Wi-Fi", "Display & Brightness", "Wallpaper", "About", "Battery", "Focus", "Do Not Disturb", "Notifications", "Accessibility", "AssistiveTouch"];
/// The push/pop slide, as NativeNavigationController times it.
pub const PUSH_MS: f64 = 350.0;
/// How far the page underneath slides left while one is pushed over it.
const PARALLAX: f32 = 0.3;
/// Frames at least this wide get the bifrost's sidebar + detail layout.
pub(crate) const SPLIT_MIN_PT: f32 = 600.0;
/// Desktop frames: the sidebar grows with the width up to this, and the detail rows stay within
/// a column of `DETAIL_MAX_PT`.
const SIDEBAR_MAX_PT: f32 = 420.0;
const DETAIL_MAX_PT: f32 = 760.0;
use super::icons::RED;
/// The pref behind Display & Brightness › Interface Scale on a desktop: "auto" or a factor ("1.25").
pub const DISPLAY_SCALE_PREF: &str = "display_scale";
/// The pref behind Displays › Appearance: "light", "dark" or "auto" (default dark).
pub const APPEARANCE_PREF: &str = "appearance_mode";
pub const ACCENT_PREF: &str = "accent_color";
/// The pref behind Developer › Performance HUD ("1" = on).
pub const PERF_HUD_PREF: &str = "perf_hud";
/// The pref behind Developer › Zero-Copy GPU Output ("1" = on; read once at start).
pub const GPU_SCANOUT_PREF: &str = "gpu_scanout";
/// Displays › Liquid Glass clarity, 0 (clear) to 1 (tinted).
pub const LIQUID_GLASS_PREF: &str = "liquid_glass";
/// Appearance > Icon & widget style: the variant key (`default`, `dark`, `tinted`).
pub const ICON_STYLE_PREF: &str = "icon_style";

/// Publishes the icon style: the variant, with the current accent's colour as the Tinted tint. Icons drawn after this
/// use it; the shell rebuilds its icons on its next frame (`Shell::refresh_icon_style`).
pub fn apply_icon_style(variant: bifrost_render::icon_doc::Variant) {
    let (dark, _) = bifrost_render::appearance::accent().colours();
    let tint = [(dark & 255) as u8, ((dark >> 8) & 255) as u8, ((dark >> 16) & 255) as u8];
    bifrost_render::icon_doc::set_style(variant, tint);
}

enum View {
    List(ListPage),
    Wallpaper(WallpaperPage),
}

struct Entry {
    id: PageId,
    title: String,
    nav: NavBar,
    view: View,
}

struct Slide {
    push: bool,
    start: f64,
    /// the page sliding out on a pop
    leaving: Option<Entry>,
    /// (underneath, moving) pages, rendered once on the first frame
    imgs: Option<(Image, Image)>,
}

/// What the top page made of a touch.
enum Outcome {
    Nothing,
    Pop,
    /// slider (group, item) dragged; true on release
    Slide(usize, usize, bool),
    Activate(Hit),
    /// a tap on the Wallpaper page
    Wallpaper(Tap),
}

/// A confirmation alert for a command (NativeAlertController, alert style).
pub struct Alert {
    pub command: Command,
    card: AlertCard,
    /// kept for the element map (`ax`): what the alert says and what its second button is called
    pub title: String,
    pub message: String,
    pub confirm: String,
}

/// What a touch on the alert landed on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AlertHit {
    Cancel,
    Confirm,
    /// the "Reopen windows when logging back in" checkbox
    Reopen,
}

impl Alert {
    pub fn new(ui: &Ui, command: Command) -> Self {
        // the removal's text comes from `Removal`, which refuses a package it cannot remove
        if let Command::RemovePackage(name) = &command {
            let (title, message, confirm) = match Removal::ask(name) {
                Ok(r) => (r.title(), r.body(), r.confirm_label()),
                Err(e) => ("Could Not Remove".to_string(), e, "OK"),
            };
            let buttons = vec![ui.text("Cancel", 17.0, Weight::Regular, BLUE), ui.text(confirm, 17.0, Weight::Semibold, RED)];
            return Alert { card: AlertCard::new(ui, &title, &message, buttons), command, title: title.clone(), message: message.clone(), confirm: confirm.to_string() };
        }
        let (title, message, confirm, destructive) = match &command {
            Command::ResetSettings => ("Reset All Settings?", "Every setting goes back to its default. Saved Wi-Fi networks are kept.".to_string(), "Reset", true),
            Command::EraseContent => ("Erase All Content and Settings?", "Your files, notes, accounts, passwords and settings are removed from this computer when BifrOSt restarts, and it starts again as new. Local backups in the data folder are removed too. The system stays installed. This cannot be undone.".to_string(), "Erase", true),
            Command::Restart => ("Restart BifrOSt?", "The shell quits and starts again.".to_string(), "Restart", true),
            Command::ShutDown => ("Shut Down?", "The computer powers off.".to_string(), "Shut Down", true),
            Command::Reboot => ("Restart?", "The computer restarts.".to_string(), "Restart", true),
            Command::Forget(ssid) => ("Forget Wi-Fi Network?", format!("“{ssid}” will no longer join automatically."), "Forget", true),
            Command::ForgetBluetooth(_) => ("Forget Bluetooth Device?", "This computer will no longer pair with it automatically.".to_string(), "Forget", true),
            Command::RemoveTextReplacement(shortcut) => ("Remove Text Replacement?", format!("“{shortcut}” will no longer expand to its phrase."), "Remove", true),
            Command::Account(danger) => {
                let (title, message, confirm) = danger.alert();
                (title, message, confirm, true)
            }
            Command::ClearCache(c) => (c.label(), c.footer().to_string(), "Clear", false),
            Command::RootedOff => (
                "Turn Off Rooted Debugging?",
                "Bare mode at boot needs it: with it off, the next boot starts without root debugging, the home app can no longer hand off to BifrOSt, and only Android starts. Turn it back on before rebooting if you rely on bare mode.".to_string(),
                "Turn Off",
                true,
            ),
            Command::BackUpNow => ("Back Up Now?", "Notes, Contacts, Calendar, Reminders, settings and notifications are saved to one file.".to_string(), "Back Up", false),
            Command::RestoreBackup => ("Restore Latest Backup?", "Notes, Contacts, Calendar, Reminders and settings are replaced by the backup, and BifrOSt restarts.".to_string(), "Restore", true),
            Command::DeleteBackups => ("Delete All Backups?", "Every backup file is removed. Your data is not changed, but a deleted backup cannot be restored.".to_string(), "Delete", true),
            Command::CheckUpdate => ("Check for Update?", "BifrOSt asks the update server whether a new version exists.".to_string(), "Check", false),
            Command::DownloadUpdate => ("Download Update?", "The new version is downloaded and checked; nothing is installed until you restart.".to_string(), "Download", false),
            Command::ShareDiagnostics => ("Share with Developer?", "The build, crash logs and heartbeat are sent to the developer's update server on your network.".to_string(), "Share", false),
            Command::DownloadSpeechModel => ("Download Speech Model?", format!("Dictation needs a model of about {} MB, stored on this computer. Your speech is recognized here and never uploaded.", bifrost_ui::whisper::MODEL_SIZE_BYTES / 1_000_000), "Download", false),
            Command::RemoveSpeechModel => ("Remove Speech Model?", "Dictation stops working until the model is downloaded again.".to_string(), "Remove", true),
            Command::DeleteCrashLogs => ("Delete All Crash Logs?", "Every crash log on this computer is removed.".to_string(), "Delete", true),
            Command::InstallUpdate => ("Restart to Install?", "BifrOSt restarts into the new version. If it fails within a minute, the previous version comes back.".to_string(), "Restart", false),
            Command::RollbackSystem => ("Roll Back to Previous System?", "The system returns to the version before the last update. Your data is kept. The shell restarts.".to_string(), "Roll Back", true),
            Command::Notice(reason) => ("Could Not Make the Change", reason.clone(), "OK", false),
            Command::RemovePackage(_) => ("Remove Package?", String::new(), "Remove", true),
        };
        let buttons = vec![ui.text("Cancel", 17.0, Weight::Regular, BLUE), ui.text(confirm, 17.0, Weight::Semibold, if destructive { RED } else { BLUE })];
        Alert { card: AlertCard::new(ui, title, &message, buttons), command, title: title.to_string(), message: message.clone(), confirm: confirm.to_string() }
    }

    /// Adds the "Reopen windows when logging back in" checkbox to a restart, shut down or update restart alert.
    pub fn with_reopen(mut self, ui: &Ui, checked: bool) -> Self {
        if matches!(self.command, Command::Restart | Command::ShutDown | Command::Reboot | Command::InstallUpdate) {
            self.card = self.card.with_checkbox(ui, "Reopen windows when logging back in", checked);
        }
        self
    }

    /// The checkbox state, `None` when the alert has none.
    pub fn reopen(&self) -> Option<bool> {
        self.card.checked()
    }

    pub fn hit(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<AlertHit> {
        if self.card.hit_check(ui, w, h, x, y) {
            return Some(AlertHit::Reopen);
        }
        self.card.hit(ui, w, h, x, y).map(|i| if i == 0 { AlertHit::Cancel } else { AlertHit::Confirm })
    }

    /// Centre of the checkbox row, for tests.
    pub fn reopen_centre(&self, ui: &Ui, w: i32, h: i32) -> Option<(f32, f32)> {
        self.card.check_centre(ui, w, h)
    }

    /// Centre of a button, for tests.
    pub fn button(&self, ui: &Ui, w: i32, h: i32, which: AlertHit) -> (f32, f32) {
        match which {
            AlertHit::Reopen => self.card.check_centre(ui, w, h).unwrap_or((0.0, 0.0)),
            _ => self.card.button_centre(ui, w, h, if which == AlertHit::Cancel { 0 } else { 1 }),
        }
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        self.card.draw(s, ui, w, h);
    }
}

/// A list page borrowed for banded drawing (see `Settings::page_view`).
pub struct PageView<'a> {
    page: &'a ListPage,
    w: i32,
    h: i32,
    pressed: Option<Hit>,
}

impl PageView<'_> {
    pub fn draw(&self, s: &mut Surface, ui: &Ui, t: f64) {
        s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, CONTENT_BG.now());
        self.page.draw(s, ui, self.pressed, t);
    }
}

/// What the pages read off the UI thread (see `settings_load`): each keeps the last value it landed, so a page
/// opens at once from it and rebuilds when a fresh read lands.
#[derive(Default)]
struct Loads {
    fonts: Loaded<super::pages::FontsView>,
    gpus: Loaded<Vec<super::gpu_page::Gpu>>,
    /// General > Startup Disk: the firmware boot table (`startup_disk_page::View::load`)
    startup: Loaded<super::startup_disk_page::View>,
    crashes: Loaded<Vec<super::diag::CrashEntry>>,
    /// failed app and system installs, newest first (`install_log::recent`)
    install_failures: Loaded<Vec<crate::install_log::Failure>>,
    latest_backup: Loaded<Option<u64>>,
    /// the packages Storage may remove (`pacman_update::removable_packages`)
    removable: Loaded<Vec<String>>,
    logins: Loaded<Vec<crate::credentials::Login>>,
    accounts: Loaded<(Vec<bifrost_ui::internet_accounts::Account>, bifrost_ui::account_store::Contents)>,
    users: Loaded<super::users_page::View>,
    sharing: Loaded<super::sharing_page::View>,
    printers: Loaded<super::print_scan_page::View>,
    /// a Print & Scan edit that finished on its worker thread; `poll_loads` acts on it once
    print_done: Loaded<super::print_scan_page::Landed>,
    screentime: Loaded<(StConfig, Vec<(i64, st_usage::Day)>)>,
    /// Settings > BifrOSt Account: the signed-in account and its devices (`account_page::load`)
    account_view: Loaded<super::account_page::View>,
    /// a sign-in or sign-out that finished on its worker thread; `poll_loads` acts on it once
    account_done: Loaded<super::account_page::Landed>,
    /// Network > Diagnose: the last finished run of the connection probes
    net_diag: Loaded<Option<bifrost_core::net_probe::Report>>,
}

impl Loads {
    /// Takes the reads that landed; true when any value changed.
    fn poll(&mut self) -> bool {
        let mut landed = false;
        landed |= self.fonts.poll();
        landed |= self.gpus.poll();
        landed |= self.startup.poll();
        landed |= self.crashes.poll();
        landed |= self.install_failures.poll();
        landed |= self.latest_backup.poll();
        landed |= self.removable.poll();
        landed |= self.logins.poll();
        landed |= self.accounts.poll();
        landed |= self.users.poll();
        landed |= self.sharing.poll();
        landed |= self.printers.poll();
        landed |= self.screentime.poll();
        landed |= self.account_view.poll();
        landed |= self.net_diag.poll();
        landed
    }

    /// A read is running.
    fn busy(&self) -> bool {
        self.fonts.busy() || self.gpus.busy() || self.crashes.busy() || self.install_failures.busy() || self.latest_backup.busy() || self.logins.busy() || self.accounts.busy() || self.users.busy() || self.sharing.busy() || self.printers.busy() || self.print_done.busy() || self.screentime.busy() || self.net_diag.busy() || self.startup.busy() || self.account_view.busy()
    }
}

pub struct Settings {
    /// the detail pane's width (the whole frame in the phone layout)
    w: i32,
    h: i32,
    /// screen height / width (the Wallpaper grid and preview take this shape)
    screen_aspect: f32,
    /// the frame's width, and the sidebar's (0 in the phone layout)
    full_w: i32,
    side_w: i32,
    /// the detail pane's side margin: wide desktop frames keep the rows in a readable column
    detail_margin_pt: f32,
    /// the split view's left column: the root list, always on screen
    sidebar: Option<ListPage>,
    side_pressed: Option<Hit>,
    /// the finger that is down landed in the sidebar
    side_touch: bool,
    /// false keeps the phone's single column on a wide frame
    split_allowed: bool,
    /// the drawn sidebar and what it was drawn from (selection, press, page gen, offset, size)
    side_cache: Option<(u64, Image)>,
    info: Info,
    opts: Options,
    /// the device has a vibrator
    haptics: bool,
    a11y: AccessibilitySettings,
    wallpapers: Rc<Vec<Wallpaper>>,
    /// Dark Appearance Dims Wallpaper
    wallpaper_dim: bool,
    /// Dynamic Wallpaper (`wallpaper_dynamic` pref): the shell changes the still with the time of day
    wallpaper_dynamic: bool,
    /// Live Look (`wallpaper_look` pref): the desktop's animated or time-of-day wallpaper, or the still
    wallpaper_look: Look,
    /// Show On (`wallpaper_show_on` pref, desktop): where a click on a thumbnail puts the wallpaper
    wallpaper_show_on: ShowOn,
    /// Add Photo… or Add Folder… was clicked; the shell brings up its open panel (`take_wallpaper_import`)
    wallpaper_import: Option<Import>,
    stack: Vec<Entry>,
    /// the pages Back left, the next one last: the toolbar pill's forward half opens them again; any other navigation
    /// clears them (a browser's forward list)
    forward: Vec<PageId>,
    transition: Option<Slide>,
    sheet: Option<Sheet>,
    /// the Time Zone picker over Date & Time
    zone_picker: Option<ZonePicker>,
    time_picker: Option<TimePicker>,
    /// notification state shown on Focus and Notifications, and the edits not yet applied by the shell
    notify: NotifyPrefs,
    notify_out: Vec<NotifyChange>,
    /// the desktop dock as the shell holds it (Desktop & Dock) and the edits not yet applied there
    dock: super::desktop_dock::Preferences,
    dock_out: Vec<super::desktop_dock::Change>,
    /// Desktop & Dock extras, Overview and the Appearance extras (they live in settings.ini)
    desk: super::pages::DeskPrefs,
    /// Screen Saver: the chosen saver's options with their saved values (`screensaver::load_sheet`)
    saver_sheet: bifrost_ui::screensaver::ConfigureSheet,
    /// the Default Apps page's categories as the shell holds them, and the choices not yet applied there
    defaults: Vec<super::defaultapps::CategoryView>,
    defaults_out: Vec<(usize, String)>,
    /// the Login Items page's rows as the shell holds them, and the ids removed there
    login_items: Vec<(String, String)>,
    /// the Firewall page's rows as the shell holds them, and the changes made there since the last call
    firewall: Vec<super::firewall_sheet::FirewallView>,
    firewall_out: Vec<(String, super::firewall_sheet::Edit)>,
    /// the Login Items page's requests waiting for approval, and the ids the user allowed there
    login_pending: Vec<(String, String)>,
    login_approve_out: Vec<String>,
    /// the ids the user moved one place earlier in Start Order
    login_moved: Vec<String>,
    /// the Login Items & Extensions page's background agents and app extensions (`login_items_page::scan_installed`), with the agents the user turned off
    login_ext: super::login_items_page::LoginExtensions,
    /// the pages' reads off the UI thread (fonts, graphics, crash logs, ...)
    loads: Loads,
    /// Privacy > Location Services: the switches and System location, read from the prefs when Settings opened
    location: super::location_page::Store,
    /// Displays > Refresh Rate: the choice in the display-refresh file, read when the Displays pages open
    refresh: super::services::display_services::RefreshChoice,
    /// Displays > Resolution: the size in the display-resolution file (None is Native), read when the Displays pages open
    resolution: Option<(u32, u32)>,
    sound: super::sound_page::SoundView,
    /// Sound: the saved output, input and sound-effects device UIDs (`sound_page::PREF_*`), chosen again when the
    /// device list is read
    sound_saved: (String, String, String),
    /// the headphones are plugged in (the output route), so the analog output is listed as Headphones
    headphones: bool,
    login_out: Vec<String>,
    /// the Focus time row the time-of-day picker is editing
    notify_edge: Option<NotifyEdit>,
    night: NightShift,
    /// the Night Shift time being picked: true for From
    night_edge: Option<bool>,
    /// the probes Network > Diagnose runs (the system's own; tests swap in a fake)
    net_probe: std::sync::Arc<dyn bifrost_core::net_probe::Probe + Send + Sync>,
    /// Privacy > Allow Apps From (see `launch_gate`)
    allow_from: bifrost_core::launch_policy::Setting,
    /// Battery > Schedule: the repeating wake and power-on times
    wake: bifrost_core::wake_schedule::Schedule,
    /// the Schedule entry whose time the time-of-day picker is editing
    wake_edge: Option<usize>,
    alert: Option<Alert>,
    pressed: Option<Hit>,
    /// a thumbnail held on the Wallpaper page
    pressed_thumb: Option<usize>,
    /// what the finger landed on in the password sheet
    pressed_sheet: Option<SheetHit>,
    /// the slider row being dragged on the top page
    drag: Option<(usize, usize)>,
    airplane: bool,
    wifi_on: bool,
    brightness: f32,
    /// the network whose detail page is open
    detail: String,
    /// where passcode.ini and the measured caches live
    data_dir: PathBuf,
    /// the bifrost Storage measurement, run on a worker thread
    scan: Scanner,
    /// the Back Up Now copy in flight; its result sets the backup note
    backup_rx: Option<std::sync::mpsc::Receiver<Result<usize, String>>>,
    /// a package removal running on a worker: its result
    package_rx: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    /// the backup copied before Install Now restarts the system, while it runs (its file count, or why it failed)
    install_rx: Option<std::sync::mpsc::Receiver<Result<usize, String>>>,
    /// the backup is in and the restart can go ahead (taken by the shell through `take_install_ready`)
    install_ready: bool,
    /// Developer switches as read from the Android properties
    dev: DevState,
    updater: Updater,
    /// an OS update is on offer (set by the shell's background checker)
    update_badge: bool,
    backup_note: String,
    /// Analytics & Diagnostics: the result of the last share or delete
    diag_note: String,
    /// the upload's answer, while one runs
    diag_rx: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    /// Startup Disk: the entry whose change waits for the administrator approval on the password sheet
    startup_pending: Option<String>,
    /// a crash log the user tapped, for the shell to show (see `take_view_request`)
    view_request: Option<PathBuf>,
    /// Settings > Screen Saver > Preview was tapped; the shell starts the saver (see `take_screensaver_preview`)
    screensaver_preview: bool,
    /// last download percent drawn, so the page rebuilds only when it moves
    update_pct: u64,
    /// stack depth when only the top page was last re-rendered: pages beneath it are stale, and
    /// `tick` brings them up to date once a pop exposes one (0 = none are stale)
    stale_depth: usize,
    /// the keypad over Settings while a passcode is set, changed or turned off
    passcode: Option<Overlay>,
    /// the login whose password waits for the passcode before it is shown
    pw_pending: Option<(String, String)>,
    /// the login whose password the Passwords page shows: site, account, password
    pw_revealed: Option<(String, String, String)>,
    /// Settings > Internet Accounts: why the last change did not happen
    ia_note: String,
    /// Settings > BifrOSt Account: the value of the root row (`account_page::label`; blank until the first read)
    account_label: String,
    /// Settings > BifrOSt Account: why the last sign-in or sign-out did not happen
    account_note: String,
    /// the account read is started once per Settings window
    account_started: bool,
    /// Screen Time: weekly chart shown, the Downtime row the time picker edits (true = From), and the passcode prompt guarding an edit
    st_weekly: bool,
    st_edge: Option<bool>,
    st_gate: Option<(Lock, StEdit)>,
    /// the search field over the root page
    search: Search,
    /// what Search finds, while it has a query (see `settings_search::index`)
    index: Option<Vec<settings_search::Entry>>,
    /// Users & Groups: the last error from privd, shown under the list ("" when none)
    users_note: String,
}

/// Display name of the wallpaper with this id ("" when unknown).
fn wallpaper_name(list: &[Wallpaper], id: &str) -> String {
    list.iter().find(|w| w.id == id).map(|w| w.name.clone()).unwrap_or_default()
}

/// The page slide as the shared push transition: a pop plays it backwards.
fn slide(push: bool) -> Transition {
    let e = Transition::push(Subtype::FromRight).parallax(PARALLAX, 60.0 / 255.0).ease(Ease::OutCubic);
    if push { e } else { e.reversed() }
}

/// The page an arrow key moves the search highlight to: the next (or previous) hit on another page than the highlighted
/// one, since every hit of a page is filled together. Down from no highlight is the first hit, Up the last.
fn step_hit(hits: &[PageId], at: Option<usize>, down: bool) -> Option<PageId> {
    let Some(at) = at else {
        return if down { hits.first() } else { hits.last() }.copied();
    };
    let cur = hits[at];
    if down { hits[at + 1..].iter().find(|p| **p != cur) } else { hits[..at].iter().rev().find(|p| **p != cur) }.copied()
}

impl Settings {
    /// Lays the app out for a pane of `w` x `h` px (Split View, Slide Over).
    pub fn set_size(&mut self, ui: &Ui, w: i32, h: i32) {
        if (self.full_w, self.h) != (w, h) {
            self.full_w = w;
            self.h = h;
            self.apply_mode(ui);
            self.search.resize(ui, self.w);
            self.rebuild(ui);
        }
    }

    /// Chooses the layout for the frame: bifrost's split view (sidebar + detail) when it is wide
    /// enough, else the phone's single column, carrying the open page across.
    fn apply_mode(&mut self, ui: &Ui) {
        let split = self.split_allowed && self.full_w as f32 >= ui.pt(SPLIT_MIN_PT);
        // a wide desktop frame widens the sidebar a little and keeps the detail rows in a column
        // of at most DETAIL_MAX_PT; the bifrost (768 pt wide portrait, 1024 landscape) is unchanged
        let wide = split && crate::layout::adaptive();
        // the sidebar is a fixed 232 pt column (NativeSplitViewItem min = max thickness); a wide window gives the detail pane the rest
        let side_pt = SIDEBAR_PT;
        let _ = (wide, SIDEBAR_MAX_PT);
        self.side_w = if split { ui.pt(side_pt) as i32 } else { 0 };
        self.w = self.full_w - self.side_w;
        self.detail_margin_pt = if wide { ((self.w as f32 / ui.s - DETAIL_MAX_PT) / 2.0).max(DETAIL_MARGIN_PT) } else { DETAIL_MARGIN_PT };
        if split {
            if self.stack.first().is_some_and(|e| e.id == PageId::Root) {
                self.stack.remove(0);
            }
            if self.stack.is_empty() {
                let entry = self.build(ui, PageId::General, None);
                self.stack.push(entry);
            }
            // a resize keeps where the sidebar was scrolled to (clamped to the new height)
            let offset = self.sidebar.as_ref().map_or(0.0, ListPage::offset);
            let mut sb = self.build_sidebar(ui);
            sb.jump_to(offset);
            self.sidebar = Some(sb);
        } else {
            // the split view's default detail page (General, opened by itself) is not a page the
            // user navigated to: the single column starts at the root list again
            if self.sidebar.is_some() && self.stack.len() == 1 && self.stack[0].id == PageId::General {
                self.stack.clear();
            }
            self.sidebar = None;
            if self.stack.first().is_none_or(|e| e.id != PageId::Root) {
                let root = self.build(ui, PageId::Root, None);
                self.stack.insert(0, root);
            }
        }
    }

    /// Turns the split view on or off for wide frames (previews and tests of the single column).
    pub fn allow_split(&mut self, ui: &Ui, on: bool) {
        self.split_allowed = on;
        self.apply_mode(ui);
        self.rebuild(ui);
    }

    fn pane(&self) -> Pane {
        if self.side_w > 0 {
            // desktop: no status bar; the toolbar zone above the cards is 52 pt (the back/forward pill sits in it)
            Pane { margin_pt: self.detail_margin_pt, nav_pt: 52.0, sidebar: false, status_pt: 0.0, header: false }
        } else {
            Pane::PHONE
        }
    }

    /// The sidebar's rows: the root page's, which a desktop arranges in the Golden Gate groups with Appearance and
    /// Keyboard as panes of their own (`pages::desktop_sidebar`).
    fn sidebar_spec(&self) -> super::list::Spec {
        let mut root = spec(PageId::Root, &self.model());
        if self.info.display_mode.is_some() {
            root = super::pages::desktop_sidebar(root, self.info.desktop_network.is_some());
        }
        root
    }

    fn build_sidebar(&self, ui: &Ui) -> ListPage {
        let pane = Pane { margin_pt: SIDEBAR_MARGIN_PT, nav_pt: 0.0, sidebar: true, status_pt: 0.0, header: false };
        let mut root = self.sidebar_spec();
        // like bifrost, the sidebar shows a value only for Wi-Fi and Bluetooth; the rest is plain
        for item in root.groups.iter_mut().flat_map(|g| g.items.iter_mut()) {
            if item.label != "Network" && item.label != "Wi-Fi" && item.label != "Bluetooth" {
                item.value.clear();
            }
        }
        if let Some(results) = self.search_results() {
            root = results;
        }
        let mut page = ListPage::in_pane(ui, root, self.side_w, self.h, None, pane);
        page.select(self.side_selected());
        page
    }

    /// `wallpapers` is the packed catalogue; `info.wallpaper` names the
    /// current one; `prefs` holds the persisted options.
    pub fn new(ui: &Ui, info: &Info, prefs: &Prefs, w: i32, h: i32, wallpapers: Rc<Vec<Wallpaper>>) -> Self {
        let opts = Options::load(prefs);
        let saver_sheet = super::screensaver::load_sheet(opts.screensaver_module, prefs);
        crate::bare::pointer_prefs::set(opts.pointer_tuning());
        let mut st = Settings {
            w,
            h,
            screen_aspect: h as f32 / w.max(1) as f32,
            full_w: w,
            side_w: 0,
            detail_margin_pt: DETAIL_MARGIN_PT,
            sidebar: None,
            side_pressed: None,
            side_touch: false,
            split_allowed: true,
            side_cache: None,
            info: info.clone(),
            opts,
            haptics: false,
            a11y: AccessibilitySettings::load(prefs),
            wallpapers,
            wallpaper_dim: prefs.get("wallpaper_dim") != Some("0"),
            wallpaper_dynamic: prefs.get("wallpaper_dynamic") == Some("1"),
            wallpaper_look: Look::parse(prefs.get(LOOK_PREF).unwrap_or("")),
            wallpaper_show_on: ShowOn::parse(prefs.get(SHOW_ON_PREF).unwrap_or("")),
            wallpaper_import: None,
            stack: vec![],
            forward: Vec::new(),
            transition: None,
            sheet: None,
            zone_picker: None,
            time_picker: None,
            notify: NotifyPrefs::default(),
            notify_out: Vec::new(),
            dock: super::desktop_dock::Preferences::default(),
            dock_out: Vec::new(),
            desk: super::pages::DeskPrefs::load(prefs),
            saver_sheet,
            defaults: Vec::new(),
            defaults_out: Vec::new(),
            login_items: Vec::new(),
            login_pending: Vec::new(),
            firewall: Vec::new(),
            firewall_out: Vec::new(),
            login_approve_out: Vec::new(),
            login_moved: Vec::new(),
            login_ext: Default::default(),
            login_out: Vec::new(),
            loads: Loads::default(),
            location: super::location_page::Store::load(prefs),
            refresh: super::services::display_services::RefreshChoice::Max,
            resolution: None,
            sound: Default::default(),
            sound_saved: (
                prefs.get(super::sound_page::PREF_OUTPUT).unwrap_or("").to_string(),
                prefs.get(super::sound_page::PREF_INPUT).unwrap_or("").to_string(),
                prefs.get(super::sound_page::PREF_EFFECTS).unwrap_or("").to_string(),
            ),
            headphones: false,
            notify_edge: None,
            night: NightShift::load(prefs),
            night_edge: None,
            net_probe: std::sync::Arc::new(bifrost_core::net_probe::SystemProbe::default()),
            allow_from: prefs.get(super::launch_gate::SETTING_PREF).and_then(bifrost_core::launch_policy::Setting::parse).unwrap_or(bifrost_core::launch_policy::Setting::StoreAndIdentified),
            wake: super::wake_page::load(prefs),
            wake_edge: None,
            alert: None,
            pw_pending: None,
            pw_revealed: None,
            ia_note: String::new(),
            account_label: String::new(),
            account_note: String::new(),
            account_started: false,
            pressed: None,
            pressed_thumb: None,
            pressed_sheet: None,
            drag: None,
            airplane: false,
            wifi_on: info.wifi_on,
            brightness: info.brightness,
            detail: String::new(),
            data_dir: super::apphost::root(),
            scan: Scanner::default(),
            backup_rx: None,
            package_rx: None,
            install_rx: None,
            install_ready: false,
            dev: DevState { perf_hud: prefs.get(PERF_HUD_PREF) == Some("1"), gpu_scanout: crate::bare::scanout::pref_on(prefs.get(GPU_SCANOUT_PREF)), ..DevState::read() },
            updater: Updater::new(super::apphost::root(), prefs.get(update::URL_PREF)),
            update_badge: false,
            backup_note: String::new(),
            diag_note: String::new(),
            diag_rx: None,
            startup_pending: None,
            view_request: None,
            screensaver_preview: false,
            update_pct: 0,
            stale_depth: 0,
            passcode: None,
            st_weekly: false,
            st_edge: None,
            st_gate: None,
            search: Search::new(ui, w),
            index: None,
            users_note: String::new(),
        };
        let root = st.build(ui, PageId::Root, None);
        st.stack.push(root);
        st.apply_mode(ui);
        st
    }

    /// Whether the device has a vibrator (Keyboard Haptics row).
    pub fn set_haptics_available(&mut self, on: bool) {
        self.haptics = on;
    }

    /// Points the storage and passcode pages at the shell's data directory.
    pub fn set_data_dir(&mut self, dir: impl Into<PathBuf>) {
        self.data_dir = dir.into();
        if !self.updater.busy() {
            let url = self.updater.url().to_string();
            self.updater = Updater::new(self.data_dir.clone(), Some(&url));
        }
    }

    /// Shows or hides the red badges on General and Software Update.
    pub fn set_update_badge(&mut self, ui: &Ui, on: bool) {
        if self.update_badge != on {
            self.update_badge = on;
            self.rebuild(ui);
        }
    }

    /// Automatic Updates as the user left it.
    pub fn auto_update(&self) -> bool {
        self.opts.auto_update
    }

    /// Adopts the background checker's offer, download or staged update.
    pub fn mirror_update(&mut self, ui: &Ui, o: &Updater) {
        if matches!(o.state, update::State::Available(_) | update::State::Downloading | update::State::Staged(_)) && self.updater.state != o.state && !self.updater.busy() {
            self.updater.mirror(o);
            self.update_pct = 0;
            self.rebuild(ui);
        }
    }

    fn model(&self) -> Model<'_> {
        let now = self.info.battery_history.last().map_or(0, |s| s.unix);
        Model { info: &self.info, opts: &self.opts, a11y: &self.a11y, airplane: self.airplane, haptics: self.haptics, wifi_on: self.wifi_on, brightness: self.brightness, wallpaper: self.wallpaper_name(), detail: &self.detail, now, update_badge: self.update_badge, dock: self.dock.clone(), defaults: &self.defaults, login_items: &self.login_items, login_pending: &self.login_pending, firewall: &self.firewall, ext: &self.login_ext, fonts: &self.loads.fonts.value, refresh: self.refresh, resolution: self.resolution, sound: &self.sound, sharing: self.loads.sharing.get(), printers: self.loads.printers.get(), users: &self.loads.users.value, account: &self.account_label }
    }

    fn wallpaper_name(&self) -> String {
        wallpaper_name(&self.wallpapers, &self.info.wallpaper)
    }

    /// The spec a page is built from: the shared page table, or the page's own source when its rows depend on live state
    /// (updates, storage, Wi-Fi, notifications). `build` and the element map both start here, so they cannot disagree.
    fn spec_for(&self, id: PageId) -> super::list::Spec {
        match id {
            PageId::Display if self.info.display_mode.is_some() => super::pages::desktop_display(&self.model()),
            PageId::HomeScreen => super::pages::desktop_dock_page(&self.model(), &self.desk),
            PageId::Appearance => super::pages::appearance_page(&self.model(), &self.desk),
            PageId::HotCorners => super::pages::hot_corners_page(&self.desk),
            PageId::ScreenSaver => super::pages::screen_saver_page(&self.opts, &self.saver_sheet),
            PageId::Overview => super::pages::overview_page(&self.desk),
            PageId::General => {
                let mut s = spec(id, &self.model());
                // before the Restart / Shut Down group
                let at = s.groups.len().saturating_sub(1);
                s.groups.insert(at, super::pages::restoration_group(&self.desk));
                s
            }
            PageId::Notifications => notify_page::spec(&self.notify),
            PageId::NotificationApp => notify_page::app_spec(&self.notify, &self.detail),
            PageId::NightShift => self.night.spec(self.opts.clock_24h),
            PageId::Network => {
                let mut s = spec(id, &self.model());
                let (host, _) = super::net_diagnose_page::target_of(self.updater.url());
                s.groups.extend(super::net_diagnose_page::groups(self.loads.net_diag.busy(), self.loads.net_diag.get().and_then(|r| r.as_ref()), &host));
                s
            }
            PageId::Privacy => {
                let mut s = spec(id, &self.model());
                let blocked = super::launch_gate::read_blocked(&self.data_dir);
                let mut extra = super::launch_gate::groups(self.allow_from, blocked.as_ref());
                // the blocked program comes first, the setting after the page's own rows
                if blocked.is_some() {
                    s.groups.insert(0, extra.remove(0));
                }
                s.groups.extend(extra);
                s
            }
            PageId::WakeSchedule => {
                let (unix, off) = crate::bare::sys::wall_time();
                super::wake_page::spec(&self.wake, self.opts.clock_24h, unix, i64::from(off))
            }
            PageId::Energy => {
                // the wake Schedule sits with the sleep timeout
                let mut s = spec(id, &self.model());
                s.groups.push(Group::new(vec![Item::new("Schedule").value(super::wake_page::summary(&self.wake)).link(PageId::WakeSchedule)]));
                s
            }
            PageId::Focus => {
                let mut s = spec(id, &self.model());
                let prefs = NotifyPrefs { clock_24h: self.opts.clock_24h, ..self.notify.clone() };
                s.groups.insert(0, notify_page::status_group(prefs.active.as_deref()));
                s.groups.push(notify_page::modes_group(&prefs));
                s.groups.extend(notify_page::add_mode_group(&prefs));
                s.groups.push(notify_page::schedule_group(&prefs));
                s
            }
            PageId::Graphics => super::gpu_page::spec(self.loads.gpus.get().map(Vec::as_slice)),
            PageId::StartupDisk => super::startup_disk_page::spec(self.loads.startup.get(), self.startup_pending.as_deref()),
            PageId::Sharing => super::pages::sharing_spec(&self.opts, self.loads.sharing.get()),
            PageId::PrintScan => super::print_scan_page::spec_pending(self.loads.printers.get(), self.loads.print_done.busy()),
            PageId::GameControllers | PageId::GameController(_) => super::game_controllers_page::spec(id, &super::services::gamepad_service::snapshot()),
            PageId::Assistant => {
                let prefs = super::prefs::Prefs::load(self.data_dir.join("settings.ini"));
                super::assistant_page::spec(&super::assistant::Settings::load(&prefs), &super::assistant_page::intent_apps())
            }
            PageId::Storage | PageId::Passcode | PageId::Developer => {
                let passcode_on = passcode::Store::load(self.data_dir.join("passcode.ini")).enabled();
                let removable = self.loads.removable.get().map_or(&[][..], |v| v.as_slice());
                storage_page::spec(id, &PageState { usage: self.scan.usage.as_ref(), passcode_on, dev: &self.dev, removable })
            }
            PageId::Passwords => {
                let passcode_on = passcode::Store::load(self.data_dir.join("passcode.ini")).enabled();
                let revealed = self.pw_revealed.as_ref().map(|(site, account, password)| super::passwords_page::Shown { site: site.as_str(), account: account.as_str(), password: password.as_str() });
                super::passwords_page::spec(&super::passwords_page::PageState { logins: &self.loads.logins.value, revealed, offset: crate::bare::sys::wall_time().1, passcode_on })
            }
            PageId::InternetAccounts => super::internet_accounts_page::spec(&super::internet_accounts_page::PageState { accounts: &self.loads.accounts.value.0, store: &self.loads.accounts.value.1, note: &self.ia_note }),
            PageId::Account => super::account_page::spec(&super::account_page::PageState {
                view: self.loads.account_view.get(),
                note: &self.account_note,
                busy: self.loads.account_done.busy(),
            }),
            PageId::Diagnostics => diagnostics_page::spec(&diagnostics_page::PageState { crashes: &self.loads.crashes.value, installs: &self.loads.install_failures.value, offset: crate::bare::sys::wall_time().1, note: &self.diag_note, sharing: self.diag_rx.is_some() }),
            PageId::Users | PageId::UserDetail | PageId::Groups | PageId::GroupDetail => super::users_page::spec(id, &self.loads.users.value, &self.detail, &self.users_note, &self.opts),
            PageId::ScreenTime | PageId::StDowntime | PageId::StLimits | PageId::StAllowed => self.st_spec(id),
            PageId::PrivacyPermissions => super::pages::permissions_page(&super::services::privacy_permissions::rows()),
            PageId::AppPanes(slot) => super::settings_panes::spec(slot),
            PageId::Root => {
                // the root lists the apps that declare settings panes after the built-in groups; in the split layout the
                // root is the sidebar, so Search and the page paths see the panes it lists (Appearance, Keyboard)
                let mut s = if self.side_w > 0 { self.sidebar_spec() } else { spec(id, &self.model()) };
                s.groups.extend(super::settings_panes::root_groups());
                s
            }
            PageId::LocationServices => super::location_page::spec(&self.location, super::services::location::now_unix()),
            PageId::Backup | PageId::SoftwareUpdate => backup_page::spec(id, &backup_page::PageState { updater: &self.updater, auto_update: self.opts.auto_update, backup_before_update: self.opts.backup_before_update, backup_note: &self.backup_note, latest_backup: self.loads.latest_backup.value, destination: &self.opts.backup_destination }),
            _ => spec(id, &self.model()),
        }
    }

    fn build(&self, ui: &Ui, id: PageId, back: Option<&str>) -> Entry {
        if id == PageId::Wallpaper {
            let title = "Wallpaper".to_string();
            let mut page = WallpaperPage::new(ui, self.w, self.screen_aspect, &self.wallpapers, &self.info.wallpaper, self.wallpaper_dim);
            page.set_dynamic(self.wallpaper_dynamic);
            page.set_look(self.wallpaper_look);
            // the desktop applies a thumbnail on click (Golden Gate); the tablet keeps its preview with the Set buttons
            page.set_desktop(ui, super::pages::pointer_form());
            page.set_show_on(self.wallpaper_show_on);
            return Entry { id, nav: NavBar::new(ui, &title, back).height(self.pane().nav_pt), title, view: View::Wallpaper(page) };
        }
        let spec = self.spec_for(id);
        let mut spec = spec;
        if id == PageId::Root {
            if let Some(results) = self.search_results() {
                spec = results;
            }
        }
        let title = spec.title.clone();
        // a page opened with no back target is a first-level page: it gets the header card
        let pane = Pane { header: back.is_none(), ..self.pane() };
        Entry { id, nav: NavBar::new(ui, &title, back).height(pane.nav_pt).status(pane.status_pt).pill(ui, pane.status_pt == 0.0 && !pane.sidebar), title, view: View::List(ListPage::in_pane(ui, spec, self.w, self.h, back, pane)) }
    }

    /// Settings > Passwords: a tap on a login asks for the passcode when one is set, then shows the password; Copy,
    /// Hide and Delete act on the revealed login.
    fn passwords_act(&mut self, ui: &Ui, act: super::passwords_page::Act) -> Vec<Effect> {
        use super::passwords_page::Act;
        match act {
            Act::Reveal { site, account } => {
                if passcode::Store::load(self.data_dir.join("passcode.ini")).enabled() {
                    self.pw_pending = Some((site, account));
                    self.passcode = Some(Overlay::new(&self.data_dir.join("passcode.ini"), passcode::Purpose::Reveal, false, self.w, self.h));
                } else {
                    self.reveal_password(ui, site, account);
                }
            }
            Act::Copy => {
                if let Some((_, _, password)) = &self.pw_revealed {
                    super::clipboard::set_text(password);
                }
            }
            Act::Hide => self.pw_revealed = None,
            Act::Delete { site, account } => {
                let _ = crate::credentials::remove(&super::apphost::root(), &site, &account);
                let root = super::apphost::root();
                self.loads.logins.start("logins-read", move || crate::credentials::logins(&root));
                if self.pw_revealed.as_ref().is_some_and(|(s, a, _)| *s == site && *a == account) {
                    self.pw_revealed = None;
                }
            }
        }
        self.rebuild(ui);
        vec![]
    }

    /// Settings > Internet Accounts: Add opens the first prompt of the flow; a service switch or Remove changes the
    /// shared list at once. A change that fails shows its reason on the page.
    fn ia_act(&mut self, ui: &Ui, act: super::internet_accounts_page::Act, t: f64) -> Vec<Effect> {
        use super::internet_accounts_page::{self as ia, Act};
        match act {
            Act::Add(kind) => {
                self.ia_note.clear();
                self.sheet = Some(Sheet::form(ui, self.w, t, super::forms::Prompt::new_account_address(kind)));
                return vec![];
            }
            Act::Toggle { id, service } => {
                self.ia_note = match ia::list_path() {
                    Ok(path) => ia::toggle(&path, &id, service).err().unwrap_or_default(),
                    Err(e) => e,
                };
            }
            Act::Remove { id } => {
                self.ia_note = match ia::list_path() {
                    Ok(path) => ia::remove(&path, &super::apphost::root(), &id).err().unwrap_or_default(),
                    Err(e) => e,
                };
            }
            Act::ForgetRefusal { app, account_type } => {
                self.ia_note = ia::forget_refusal(&app, &account_type).err().unwrap_or_default();
            }
        }
        self.loads.accounts.start("accounts-read", ia::load);
        self.rebuild_visible(ui);
        vec![]
    }

    fn reveal_password(&mut self, ui: &Ui, site: String, account: String) {
        if let Some(password) = crate::credentials::secret(&super::apphost::root(), &site, &account) {
            self.pw_revealed = Some((site, account, password));
        }
        self.rebuild(ui);
    }

    /// Re-renders every page on the stack (new info, text size), keeping
    /// each one's scroll position.
    fn rebuild(&mut self, ui: &Ui) {
        self.stale_depth = 0;
        self.rebuild_pages(ui, 0..self.stack.len());
        self.rebuild_sidebar(ui);
    }

    /// Re-renders only what is on screen: the top page and the sidebar. The pages beneath
    /// are brought up to date by `tick` when a pop or a jump exposes them.
    fn rebuild_visible(&mut self, ui: &Ui) {
        self.stale_depth = if self.stack.len() > 1 { self.stack.len() } else { 0 };
        self.rebuild_pages(ui, self.stack.len().saturating_sub(1)..self.stack.len());
        self.rebuild_sidebar(ui);
    }

    fn rebuild_pages(&mut self, ui: &Ui, pages: std::ops::Range<usize>) {
        for k in pages {
            let back = (k > 0).then(|| self.stack[k - 1].title.clone());
            let offset = match &self.stack[k].view {
                View::List(p) => p.offset(),
                View::Wallpaper(_) => 0.0,
            };
            // a pop-up menu that is open stays open when a background read lands and the page is built again
            let open_label = match &self.stack[k].view {
                View::List(p) => p.open_popup_label(),
                View::Wallpaper(_) => None,
            };
            let mut entry = self.build(ui, self.stack[k].id, back.as_deref());
            if let View::List(p) = &mut entry.view {
                p.jump_to(offset);
                if let Some((g, i)) = open_label.and_then(|l| p.find(&l)) {
                    p.open_popup(ui, g, i);
                }
            }
            self.stack[k] = entry;
        }
        self.sync_forward();
    }

    fn rebuild_sidebar(&mut self, ui: &Ui) {
        if let Some(old) = &self.sidebar {
            let offset = old.offset();
            let mut sb = self.build_sidebar(ui);
            sb.jump_to(offset);
            self.sidebar = Some(sb);
        }
    }

    /// Re-renders text for new device info, keeping the page and any sheet.
    pub fn refresh(&mut self, ui: &Ui, info: &Info) {
        self.wifi_on = info.wifi_on;
        if self.drag.is_none() {
            self.brightness = info.brightness;
        }
        if self.info != *info {
            let sink_changed = self.info.bt.audio_sink() != info.bt.audio_sink();
            self.info = info.clone();
            // a Bluetooth headset that connects or leaves is an output that comes or goes on Sound
            if sink_changed && self.page() == PageId::Sounds {
                self.reload_sound();
            }
            self.rebuild_visible(ui);
        }
    }

    /// The live output level and route, from the shell (the Control Center slider, the volume keys, the headphone
    /// jack). Sound's Output volume shows it, not the saved pref, and follows it while the page is open.
    pub fn set_audio(&mut self, ui: &Ui, level: f32, headphones: bool) {
        let level = level.clamp(0.0, 1.0);
        let route = self.headphones != headphones;
        if (self.sound.volume - level).abs() < 1e-4 && !route {
            return;
        }
        self.sound.volume = level;
        self.headphones = headphones;
        // the page follows a level set elsewhere, but not the drag of its own slider (which set the level)
        if self.page() == PageId::Sounds && self.drag.is_none() {
            if route {
                self.reload_sound();
            }
            self.rebuild_visible(ui);
        }
    }

    /// Reads the Sound page's devices again (ALSA, the jack, Bluetooth) and selects the saved ones.
    fn reload_sound(&mut self) {
        let (volume, level) = (self.sound.volume, self.sound.level);
        self.sound = super::sound_page::read_view(self.headphones, self.info.bt.audio_sink());
        self.sound.volume = volume;
        self.sound.level = level;
        let (o, i, e) = &self.sound_saved;
        let some = |s: &str| (!s.is_empty()).then(|| s.to_string());
        let (o, i, e) = (some(o), some(i), some(e));
        self.sound.choose(o.as_deref(), i.as_deref(), e.as_deref());
    }

    /// Re-renders after a text size or weight change.
    pub fn relayout(&mut self, ui: &Ui) {
        self.rebuild(ui);
    }

    /// The options as the user has set them.
    pub fn options(&self) -> &Options {
        &self.opts
    }

    /// Publishes the Mouse and Trackpad choices to the input layer (`bare::pointer_prefs`).
    fn apply_pointer(&self) {
        crate::bare::pointer_prefs::set(self.opts.pointer_tuning());
    }

    /// Publishes the keyboard choices the hardware-key path uses (see `kb_input`).
    fn apply_keys(&self) {
        super::kb_input::set(self.opts.key_prefs());
    }

    /// The accessibility settings as the user has set them.
    pub fn accessibility(&self) -> &AccessibilitySettings {
        &self.a11y
    }

    /// Replaces the accessibility settings (the Accessibility Shortcut switches them while Settings is open).
    pub(in crate::shell) fn set_accessibility(&mut self, a: AccessibilitySettings) {
        self.a11y = a;
    }

    /// Replaces the accessibility settings and redraws the pages, for a change the shell made on its own (the
    /// on-screen keyboard was closed, so the Accessibility Keyboard switch must show it).
    pub(in crate::shell) fn refresh_accessibility(&mut self, ui: &Ui, a: AccessibilitySettings) {
        self.a11y = a;
        self.rebuild_visible(ui);
    }

    /// Synchronizes a Control Center Focus change while retaining the open page.
    pub(super) fn set_dnd(&mut self, ui: &Ui, on: bool) {
        self.opts.dnd = on;
        self.rebuild(ui);
    }

    /// Night Shift changed outside Settings (Control Center's Display detail): show it, so a later edit here does not
    /// save the old state back.
    pub(super) fn set_night_shift(&mut self, ui: &Ui, night: NightShift) {
        self.night = night;
        self.rebuild(ui);
    }

    /// Loads the notification state (schedule, muted apps) from the shell.
    pub(super) fn set_notify(&mut self, ui: &Ui, schedule: super::notify::Schedule, muted: Vec<super::AppId>, apps: Vec<(super::AppId, notify_page::AppNotif)>, focus_allowed: Vec<super::AppId>) {
        self.notify = NotifyPrefs { schedule, muted, clock_24h: self.opts.clock_24h, apps, focus_allowed, ..self.notify.clone() };
        self.rebuild(ui);
    }

    /// Loads the Focus mode names and the mode that is on by hand. The page is rebuilt by the call that follows
    /// (`set_notify` or `set_dnd`).
    pub(super) fn set_focus_modes(&mut self, names: Vec<String>, on: Option<usize>, active: Option<String>) {
        self.notify.modes = names;
        self.notify.mode_on = on;
        self.notify.active = active;
    }

    /// Loads Settings > Notifications > Notification Center's options from the shell. The page is rebuilt by the call
    /// that follows (`set_notify`).
    pub(super) fn set_notify_center(&mut self, center: notify_page::CenterPrefs) {
        self.notify.center = center;
    }

    /// The crash log the user tapped, for the shell to open in Files' viewer.
    pub(super) fn take_view_request(&mut self) -> Option<PathBuf> {
        self.view_request.take()
    }

    /// Settings > Screen Saver > Preview: true once after the row is tapped.
    pub(super) fn take_screensaver_preview(&mut self) -> bool {
        std::mem::take(&mut self.screensaver_preview)
    }

    /// Settings > Wallpaper > Add Photo… or Add Folder…: once after the click, for the shell's open panel.
    pub(super) fn take_wallpaper_import(&mut self) -> Option<Import> {
        self.wallpaper_import.take()
    }

    /// The pictures the open panel added are in the catalogue now (`wallpapers`, shared with the shell): the Wallpaper
    /// page lists them, and the first of `added` is applied as a click on its thumbnail would (Show On decides where).
    pub(super) fn wallpapers_added(&mut self, ui: &Ui, prefs: &mut Prefs, wallpapers: Rc<Vec<Wallpaper>>, added: &[String]) -> Vec<Effect> {
        self.wallpapers = wallpapers;
        let Some(top) = self.stack.iter().rposition(|e| e.id == PageId::Wallpaper) else { return vec![] };
        let back = top.checked_sub(1).map(|i| self.stack[i].title.clone());
        let page = self.build(ui, PageId::Wallpaper, back.as_deref());
        self.stack[top] = page;
        if top + 1 != self.stack.len() {
            return vec![];
        }
        let Some(i) = added.first().and_then(|id| self.wallpapers.iter().position(|w| &w.id == id)) else { return vec![] };
        self.wallpaper_tap(ui, prefs, Tap::Set(i, self.wallpaper_show_on.target()))
    }

    /// Edits made on the notification pages, for the shell to apply.
    pub(super) fn take_notify_changes(&mut self) -> Vec<NotifyChange> {
        std::mem::take(&mut self.notify_out)
    }

    /// The shell's dock state, copied before each event so the page shows what the bar menu left.
    pub(super) fn set_dock(&mut self, dock: super::desktop_dock::Preferences) {
        self.dock = dock;
    }

    /// Settings > Fonts: each role's default family and saved override (the data dir's settings.ini, as the
    /// shell reads it at start), and the font library in the data dir. Read on a worker, since the faces are
    /// parsed from the files; the page fills in on a later tick.
    fn load_fonts(&mut self) {
        let dir = self.data_dir.to_string_lossy().into_owned();
        self.loads.fonts.start("fonts-read", move || {
            use bifrost_render::fontprefs::{family, font_override, installed, library, FontRole};
            super::pages::FontsView {
                defaults: FontRole::ALL.map(|r| family(&format!("{dir}/{}", r.file())).unwrap_or_default()),
                overrides: FontRole::ALL.map(|r| font_override(&dir, r)),
                names: FontRole::ALL.map(|r| font_override(&dir, r).as_deref().and_then(family)),
                library: installed(&[library(&dir)]),
            }
        });
    }

    /// Starts the reads a page shows, on worker threads: the page opens at once from the last values read, and
    /// `tick` rebuilds it when the new ones land. Anything a thread reads is taken from the paths here, on the UI thread.
    fn start_loads(&mut self, id: PageId) {
        let dir = self.data_dir.clone();
        match id {
            PageId::Fonts | PageId::FontPick(_) => self.load_fonts(),
            PageId::Display | PageId::RefreshRate | PageId::Resolution => {
                self.refresh = super::services::display_services::RefreshChoice::load(&dir);
                self.resolution = super::services::display_services::load_resolution(&dir);
            }
            PageId::Graphics => self.loads.gpus.start("gpu-read", super::gpu_page::read_system),
            PageId::StartupDisk => self.loads.startup.start("startup-disk-read", super::startup_disk_page::View::load),
            PageId::Diagnostics => {
                let installs = dir.clone();
                self.loads.crashes.start("diag-read", move || diag::load(&dir));
                // the user's own log, and the root daemons' (privd installs, the updater) in the system data directory
                let system = std::path::PathBuf::from(crate::privd::STATE_DIR);
                self.loads.install_failures.start("install-read", move || crate::install_log::recent_in(&[&installs, &system], diagnostics_page::MAX_ROWS));
            }
            PageId::Backup | PageId::SoftwareUpdate => {
                let root = backup::destination_root(&dir, &self.opts.backup_destination);
                self.loads.latest_backup.start("backup-read", move || backup::latest(&root).map(|(_, t)| t))
            }
            PageId::Storage => self.loads.removable.start("removable-read", super::pacman_update::removable_packages),
            PageId::Passwords => {
                let root = super::apphost::root();
                self.loads.logins.start("logins-read", move || crate::credentials::logins(&root));
            }
            PageId::InternetAccounts => self.loads.accounts.start("accounts-read", super::internet_accounts_page::load),
            PageId::Account => self.start_account_view(),
            PageId::Users | PageId::UserDetail | PageId::Groups | PageId::GroupDetail => self.loads.users.start("users-read", super::users_page::View::load),
            PageId::Sharing => self.loads.sharing.start("sharing-read", super::sharing_page::View::load),
            PageId::PrintScan => self.loads.printers.start("print-scan-read", super::print_scan_page::View::load),
            PageId::ScreenTime | PageId::StDowntime | PageId::StLimits | PageId::StAllowed => {
                let (unix, off) = crate::bare::sys::wall_time();
                self.loads.screentime.start("screentime-read", move || {
                    let cfg = StConfig::load(&dir.join("screentime.ini"));
                    let days = st_usage::history(&dir.join("screentime"), st_usage::day_of(unix, off), 7);
                    (cfg, days)
                });
            }
            _ => {}
        }
    }

    /// Takes the reads that landed and rebuilds the page on top when one did; true when a frame is needed.
    fn poll_loads(&mut self, ui: &Ui, t: f64) -> bool {
        if !self.account_started {
            self.account_started = true;
            self.start_account_view();
        }
        let mut frame = self.loads.poll();
        if self.loads.account_done.poll() {
            self.account_landed(ui, t);
            frame = true;
        }
        if self.loads.print_done.poll() {
            self.print_landed(ui, t);
            frame = true;
        }
        self.account_label = super::account_page::label(self.loads.account_view.get());
        if frame {
            self.rebuild_visible(ui);
        }
        frame
    }

    /// Runs a Print & Scan edit on a worker thread (the scheduler can take seconds to answer); the page shows
    /// "Working…" until `print_landed` has the answer. A newer edit replaces the wait for an older one.
    fn start_print_edit(&mut self, ui: &Ui, work: impl FnOnce() -> Result<Option<super::forms::Prompt>, String> + Send + 'static) {
        self.loads.print_done.start("print-scan-edit", move || super::print_scan_page::run_edit(work));
        self.rebuild_visible(ui);
    }

    /// A Print & Scan edit finished: re-read the printers (also after a failure: the scheduler may have done the
    /// change anyway), then show the next prompt or the reason it did not happen.
    fn print_landed(&mut self, ui: &Ui, t: f64) {
        use super::print_scan_page::Landed;
        if let Landed::Edited(r) = std::mem::take(&mut self.loads.print_done.value) {
            self.loads.printers.start("print-scan-read", super::print_scan_page::View::load);
            self.edit_result(ui, t, r);
        }
    }

    /// Reads the BifrOSt Account page's account on a worker thread: the stored session, its name and devices.
    fn start_account_view(&mut self) {
        let root = super::apphost::root();
        self.loads.account_view.start("account-read", move || super::account_page::load(&root));
    }

    /// A sign-in or sign-out finished. A refused sign-in reopens the password prompt with the cloud's reason; a finished
    /// one reloads the account. A sign-out the cloud did not confirm says so on the page.
    fn account_landed(&mut self, ui: &Ui, t: f64) {
        use super::account_page::Landed;
        match self.loads.account_done.value.clone() {
            Landed::Idle => {}
            Landed::SignedIn { result: Ok(_), .. } => {
                self.account_note.clear();
                self.start_account_view();
            }
            Landed::SignedIn { email, result: Err(e) } => {
                self.sheet = Some(Sheet::form(ui, self.w, t, super::account_page::password_prompt(&email, &e)));
            }
            Landed::SignedOut(result) => {
                self.account_note = result.err().unwrap_or_default();
                self.start_account_view();
            }
        }
    }

    /// Settings > BifrOSt Account: Sign In opens the email sheet; Sign Out runs on a worker thread.
    fn account_act(&mut self, ui: &Ui, act: super::account_page::Act, t: f64) -> Vec<Effect> {
        use super::account_page::Act;
        match act {
            Act::SignIn => {
                self.account_note.clear();
                self.sheet = Some(Sheet::form(ui, self.w, t, super::account_page::email_prompt("")));
            }
            Act::SignOut => {
                let root = super::apphost::root();
                self.account_note.clear();
                self.loads.account_done.start("account-sign-out", move || super::account_page::Landed::SignedOut(super::account_page::sign_out(&root)));
                self.rebuild_visible(ui);
            }
        }
        vec![]
    }

    pub(super) fn set_defaults(&mut self, v: Vec<super::defaultapps::CategoryView>) {
        self.defaults = v;
    }

    /// The Firewall page's rows as the `app_firewall` service holds them.
    pub(super) fn set_firewall(&mut self, v: Vec<super::firewall_sheet::FirewallView>) {
        self.firewall = v;
    }

    /// The Firewall page changes made since the last call: (app id, change), in order.
    pub(super) fn take_firewall_edits(&mut self) -> Vec<(String, super::firewall_sheet::Edit)> {
        std::mem::take(&mut self.firewall_out)
    }

    pub(super) fn set_login_items(&mut self, v: Vec<(String, String)>, pending: Vec<(String, String)>) {
        self.login_items = v;
        self.login_pending = pending;
    }

    /// The Login Items & Extensions page's model: the installed apps' background agents and extensions, and the
    /// `login.background.off` ids read from the prefs.
    pub(super) fn set_login_extensions(&mut self, mut ext: super::login_items_page::LoginExtensions, background_off: Vec<String>) {
        ext.background_off = background_off;
        self.login_ext = ext;
    }

    /// The Login Items ids the user moved earlier since the last call, in order.
    pub(super) fn take_login_moved(&mut self) -> Vec<String> {
        std::mem::take(&mut self.login_moved)
    }

    pub(super) fn take_login_removed(&mut self) -> Vec<String> {
        std::mem::take(&mut self.login_out)
    }

    /// The Login Items requests the user allowed since the last call.
    pub(super) fn take_login_approved(&mut self) -> Vec<String> {
        std::mem::take(&mut self.login_approve_out)
    }

    pub(super) fn take_defaults_changes(&mut self) -> Vec<(usize, String)> {
        std::mem::take(&mut self.defaults_out)
    }

    pub(super) fn take_dock_changes(&mut self) -> Vec<super::desktop_dock::Change> {
        std::mem::take(&mut self.dock_out)
    }

    /// True once the update's backup is in and the shell should restart into the new version (taken once).
    pub(super) fn take_install_ready(&mut self) -> bool {
        std::mem::take(&mut self.install_ready)
    }

    /// Takes the update backup's answer when it is in: a success makes the restart ready, a failure says why not.
    fn poll_install(&mut self, ui: &Ui) -> bool {
        let Some(rx) = &self.install_rx else { return false };
        let res = match rx.try_recv() {
            Ok(res) => res,
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("the backup stopped early".to_string()),
        };
        self.install_rx = None;
        match res {
            Ok(n) => {
                self.backup_note = format!("Backed up {n} files before the update.");
                self.update_badge = false;
                self.install_ready = true;
            }
            // the data is never replaced unsaved: the install stops here
            Err(e) => self.backup_note = format!("Update not installed: backup failed: {e}"),
        }
        self.rebuild(ui);
        true
    }

    fn notify_edit(&mut self, ui: &Ui, edit: NotifyEdit, g: usize, i: usize, t: f64) {
        match edit {
            NotifyEdit::Scheduled => {
                self.notify.schedule.enabled = !self.notify.schedule.enabled;
                self.notify_out.push(NotifyChange::Schedule(self.notify.schedule));
                let on = self.notify.schedule.enabled;
                if let Some(p) = self.list_mut() {
                    p.set_switch(g, i, on, t);
                }
            }
            NotifyEdit::Mute(app) => {
                if self.notify.is_muted(app) {
                    self.notify.muted.retain(|&a| a != app);
                } else {
                    self.notify.muted.push(app);
                }
                self.notify_out.push(NotifyChange::ToggleMute(app));
                let on = self.notify.is_muted(app);
                if self.page() == PageId::NotificationApp {
                    self.rebuild(ui);
                } else if let Some(p) = self.list_mut() {
                    p.set_switch(g, i, on, t);
                }
            }
            NotifyEdit::App(app, field) => {
                let mut n = self.notify.app(app);
                n.apply(field);
                self.notify.apps.retain(|(a, _)| *a != app);
                self.notify.apps.push((app, n));
                self.notify_out.push(NotifyChange::App(app, n));
                self.rebuild(ui);
            }
            NotifyEdit::FocusAllowed(app) => {
                let on = !self.notify.focus_allows(app);
                self.notify.focus_allowed.retain(|&a| a != app);
                if on {
                    self.notify.focus_allowed.push(app);
                }
                self.notify_out.push(NotifyChange::FocusAllowed(app, on));
                self.rebuild(ui);
            }
            NotifyEdit::Mode(i) => {
                // the mode that is on is tapped again: Focus goes off
                let next = if self.notify.mode_on == Some(i) { None } else { Some(i) };
                self.notify.mode_on = next;
                self.notify_out.push(NotifyChange::Mode(next));
                self.rebuild(ui);
            }
            NotifyEdit::Center(field) => {
                self.notify.center.apply(field);
                self.notify_out.push(NotifyChange::Center(self.notify.center));
                self.rebuild(ui);
            }
            NotifyEdit::AddMode => {
                // the shell adds the mode and sends the new list back (`set_focus_modes`); the page shows it at once
                if let Some(name) = notify_page::next_preset(&self.notify.modes) {
                    self.notify.modes.push(name.to_string());
                    self.notify_out.push(NotifyChange::AddMode(name.to_string()));
                    self.rebuild(ui);
                }
            }
            NotifyEdit::From | NotifyEdit::To => {
                let (minute, title) = if edit == NotifyEdit::From { (self.notify.schedule.start, "From") } else { (self.notify.schedule.end, "To") };
                self.notify_edge = Some(edit);
                self.time_picker = Some(TimePicker::of_day(ui, self.w, self.h, minute, title));
            }
        }
    }

    /// A Battery > Schedule row was tapped: Time opens the picker, the others edit the schedule at once.
    fn wake_edit(&mut self, ui: &Ui, prefs: &mut Prefs, edit: super::wake_page::WakeEdit) -> Vec<Effect> {
        use super::wake_page::{self, WakeEdit};
        if let WakeEdit::Time(i) = edit {
            if wake_page::apply(&mut self.wake, edit) {
                self.wake_edge = Some(i);
                self.time_picker = Some(TimePicker::of_day(ui, self.w, self.h, wake_page::minute_of(&self.wake, i), "Time"));
            }
            return vec![];
        }
        if !wake_page::apply(&mut self.wake, edit) {
            return vec![];
        }
        self.wake_changed(ui, prefs)
    }

    /// The time picker chose `minute` for Schedule entry `index`.
    fn wake_set_minute(&mut self, ui: &Ui, prefs: &mut Prefs, index: usize, minute: u16) -> Vec<Effect> {
        if super::wake_page::set_minute(&mut self.wake, index, minute) {
            self.wake_changed(ui, prefs)
        } else {
            vec![]
        }
    }

    /// Keeps the schedule, redraws the page and asks the shell binary to arm the clock for the next moment.
    fn wake_changed(&mut self, ui: &Ui, prefs: &mut Prefs) -> Vec<Effect> {
        super::wake_page::save(&self.wake, prefs);
        self.rebuild(ui);
        let (unix, off) = crate::bare::sys::wall_time();
        vec![Effect::WakeSchedule(super::wake_page::next_at(&self.wake, unix, i64::from(off)))]
    }

    /// The Screen Time pages from the day files and the config on disk.
    fn st_spec(&self, id: PageId) -> super::list::Spec {
        let (cfg, days) = &self.loads.screentime.value;
        let passcode_on = screentime::passcode(&self.data_dir).enabled();
        screentime::page::spec(id, &StView { weekly: self.st_weekly, days: days.as_slice(), cfg, passcode_on, clock_24h: self.opts.clock_24h })
    }

    /// A Screen Time row was tapped: the passcode keypad first when it guards the edit.
    fn st_edit(&mut self, ui: &Ui, edit: StEdit) {
        let pass = screentime::passcode(&self.data_dir);
        if let StEdit::Passcode(purpose, custom) = edit {
            self.passcode = Some(Overlay::with_store(pass, purpose, custom, self.w, self.h));
        } else if edit.needs_passcode() && pass.enabled() {
            self.st_gate = Some((Lock::with_store(pass, self.w, self.h), edit));
        } else {
            self.st_apply(ui, edit);
        }
    }

    fn st_apply(&mut self, ui: &Ui, edit: StEdit) {
        let mut cfg = StConfig::load(&self.data_dir.join("screentime.ini"));
        match edit {
            StEdit::Range(weekly) => self.st_weekly = weekly,
            StEdit::Cycle(app) => {
                cfg.cycle_limit(app);
            }
            StEdit::DowntimeSwitch => {
                let d = cfg.downtime();
                cfg.set_downtime(screentime::config::Downtime { on: !d.on, ..d });
            }
            StEdit::DowntimeFrom | StEdit::DowntimeTo => {
                let from = edit == StEdit::DowntimeFrom;
                let d = cfg.downtime();
                self.st_edge = Some(from);
                self.time_picker = Some(TimePicker::of_day(ui, self.w, self.h, if from { d.start } else { d.end }, if from { "From" } else { "To" }));
                return;
            }
            StEdit::Allow(app) => cfg.set_allowed(app, !cfg.allowed(app)),
            StEdit::Passcode(..) => return,
        }
        self.rebuild(ui);
    }

    fn st_set_edge(&mut self, ui: &Ui, from: bool, minute: u16) {
        let mut cfg = StConfig::load(&self.data_dir.join("screentime.ini"));
        let d = cfg.downtime();
        cfg.set_downtime(if from { screentime::config::Downtime { start: minute % 1440, ..d } } else { screentime::config::Downtime { end: minute % 1440, ..d } });
        self.rebuild(ui);
    }

    /// The open alert, if any (the element map lists its buttons).
    pub fn ax_alert(&self) -> Option<&Alert> {
        self.alert.as_ref()
    }

    /// Answers the open alert as a tap on its button would.
    pub fn ax_answer(&mut self, ui: &Ui, prefs: &mut Prefs, confirm: bool, t: f64) -> Result<Vec<Effect>, String> {
        let alert = self.alert.take().ok_or("no alert is open")?;
        if confirm {
            Ok(self.run(ui, prefs, alert.command, t))
        } else {
            Ok(Vec::new())
        }
    }

    /// A page's spec from the live model: the element map (`axmap`) walks these.
    pub fn ax_spec(&self, id: PageId) -> super::list::Spec {
        self.spec_for(id)
    }

    /// Presses the row at a label path (`["General", "Software Update"]`) exactly as a tap would: the first label is a
    /// sidebar row in the split layout (a row of the root page otherwise), each next one a row of the page just opened.
    pub fn ax_press(&mut self, ui: &Ui, prefs: &mut Prefs, labels: &[String], t: f64) -> Result<Vec<Effect>, String> {
        let mut fx = Vec::new();
        let mut rest = labels;
        self.sheet = None;
        self.pressed = None;
        if self.sidebar.is_some() {
            let first = labels.first().ok_or("empty path")?;
            let (g, i) = self.sidebar.as_ref().and_then(|sb| sb.find(first)).ok_or(format!("no sidebar row {first:?}"))?;
            fx.extend(self.sidebar_activate(ui, prefs, Hit::Row(g, i), t));
            rest = &labels[1..];
        } else {
            self.stack.truncate(1);
        }
        for label in rest {
            let (g, i) = self.list().and_then(|p| p.find(label)).ok_or(format!("no row {label:?} on {:?}", self.page()))?;
            fx.extend(self.activate(ui, prefs, Hit::Row(g, i), t));
        }
        Ok(fx)
    }

    /// The page on top of the stack.
    pub fn page(&self) -> PageId {
        self.stack.last().map_or(PageId::Root, |e| e.id)
    }

    /// The pages on the stack, root first.
    pub fn stack(&self) -> Vec<PageId> {
        self.stack.iter().map(|e| e.id).collect()
    }

    /// The split layout's sidebar list, if the window is wide enough for one.
    pub fn sidebar_list(&self) -> Option<&ListPage> {
        self.sidebar.as_ref()
    }

    /// The top page's list, if it is one.
    pub fn list(&self) -> Option<&ListPage> {
        match &self.stack.last()?.view {
            View::List(p) => Some(p),
            View::Wallpaper(_) => None,
        }
    }

    fn list_mut(&mut self) -> Option<&mut ListPage> {
        match &mut self.stack.last_mut()?.view {
            View::List(p) => Some(p),
            View::Wallpaper(_) => None,
        }
    }

    /// Screen centre of the top page's row labelled `label`.
    pub fn row_centre(&self, ui: &Ui, label: &str) -> Option<(f32, f32)> {
        let p = self.list()?;
        let (g, i) = p.find(label)?;
        let (x, y) = p.row_centre(ui, g, i);
        // the page lays out in pane coordinates; the split view puts the pane right of the sidebar
        Some((x + self.pane_x(), y))
    }

    /// Screen x where the page pane starts: the sidebar's width in the split layout, else 0.
    pub fn pane_x(&self) -> f32 {
        self.side_w as f32
    }

    /// Screen centre of the sidebar row labelled `label` (split layout only).
    pub fn sidebar_centre(&self, ui: &Ui, label: &str) -> Option<(f32, f32)> {
        let p = self.sidebar.as_ref()?;
        let (g, i) = p.find(label)?;
        Some(p.row_centre(ui, g, i))
    }

    /// Jumps to the page a row name stands for (rows without a page of
    /// their own show the root), dropping any sheet; Wi-Fi starts a scan.
    pub fn open_page(&mut self, ui: &Ui, name: &str, t: f64) -> Vec<Effect> {
        let id = match name {
            "Network" => PageId::Network,
            "Assistant" => PageId::Assistant,
            "Ethernet" => PageId::Ethernet,
            "Wi-Fi" => PageId::Wifi,
            "Bluetooth" => PageId::Bluetooth,
            "Location Services" => PageId::LocationServices,
            "Display & Brightness" => PageId::Display,
            "Night Shift" => PageId::NightShift,
            "Wallpaper" => PageId::Wallpaper,
            "Screen Saver" => PageId::ScreenSaver,
            "About" => PageId::About,
            "Battery" => PageId::Battery,
            "Focus" | "Do Not Disturb" => PageId::Focus,
            "Notifications" => PageId::Notifications,
            "Software Update" => PageId::SoftwareUpdate,
            "Fonts" => PageId::Fonts,
            "Appearance" => PageId::Appearance,
            "Keyboard" => PageId::Keyboard,
            "Accessibility" | "AssistiveTouch" => PageId::Accessibility,
            _ => match settings_search::alias(name).or_else(|| settings_search::find_page(&self.build_index(), name)) {
                Some(id) => id,
                None => {
                    self.reset_overlays();
                    if self.stack.is_empty() {
                        self.push(ui, PageId::General, t);
                    }
                    return vec![];
                }
            },
        };
        self.open_page_id(ui, id, t)
    }

    /// Drops the sheet, pickers, press and slide before a jump to another page.
    fn reset_overlays(&mut self) {
        self.sheet = None;
        self.zone_picker = None;
        self.time_picker = None;
        self.pressed = None;
        self.transition = None;
    }

    /// The pages from a first-level pane (a sidebar row) down to `id`, the pane first and `id` last: what Back walks up
    /// through. `id` alone when no page links to it.
    fn path_to(&self, id: PageId) -> Vec<PageId> {
        settings_search::path_to(|p| self.spec_for(p), id).unwrap_or_else(|| vec![id])
    }

    /// Opens the pane `id` directly (a deep link, an App Intent of the Settings panes, a Search result). The pages that
    /// lead to it go under it, so Back walks up to its pane: in the split layout the pane's sidebar row is selected and
    /// the pane is the detail's first page; the single column stacks them on the root.
    pub fn open_page_id(&mut self, ui: &Ui, id: PageId, t: f64) -> Vec<Effect> {
        // a computer without a battery has no Battery pane: its power settings are Energy
        let id = if id == PageId::Battery && !super::pages::has_battery(&self.info) { PageId::Energy } else { id };
        self.reset_overlays();
        if id == PageId::Root {
            // the root is the single column's base; the split layout keeps its pane
            if self.side_w == 0 {
                self.stack.truncate(1);
                if self.stack.is_empty() {
                    self.push(ui, PageId::Root, t);
                }
                self.forward.clear();
                self.sync_forward();
            }
            return vec![];
        }
        let path = self.path_to(id);
        self.stack.truncate(usize::from(self.side_w == 0));
        for p in path {
            self.push(ui, p, t);
        }
        match id {
            PageId::Wifi => vec![Effect::Wifi(WifiAction::Scan)],
            PageId::Bluetooth => vec![Effect::Bluetooth(BtCmd::Scan)],
            _ => vec![],
        }
    }

    /// Developer preview: opens a stack of pages by `PageId` name, e.g. `General>About` (tests only).
    #[cfg(test)]
    pub fn preview_open(&mut self, ui: &Ui, path: &str, t: f64) {
        use PageId::*;
        self.stack.truncate(usize::from(self.side_w == 0));
        for name in path.split('>') {
            let id = [Network, Wifi, Bluetooth, Sounds, Focus, Notifications, General, About, SoftwareUpdate, Storage, DateTime, Keyboard, Language, Fonts, Dictionary, DefaultApps, LoginItems,
                HomeScreen, Battery, WakeSchedule, ControlCentre, MenuBar, Accessibility, Privacy, Passcode, Passwords, InternetAccounts, Developer, ScreenTime, Backup, Wallpaper, Nearby, NightShift, InterfaceScale, Graphics, AutoLock, Energy, Vpn, Legal,
                BackgroundRefresh, Display, Appearance, Diagnostics, DisplayZoom, TouchHold, DwellControl, SlowKeys, AccessibilityShortcut, ScreenReaderSpeech]
            .into_iter()
            .find(|id| format!("{id:?}") == name);
            if let Some(id) = id {
                self.push(ui, id, t);
            }
        }
    }

    /// Lays the pages out for a new frame size (the device turned), keeping the page, the open
    /// sheet and what was typed into it.
    pub fn resize(&mut self, ui: &Ui, w: i32, h: i32) {
        self.pressed = None;
        self.set_size(ui, w, h);
        if let Some(sh) = &mut self.sheet {
            sh.keyboard.resize(w as f32);
        }
    }

    /// The SSID whose password sheet is open.
    pub fn sheet(&self) -> Option<&str> {
        self.sheet.as_ref().map(|s| s.ssid.as_str())
    }

    /// True while a password is being typed into the open sheet (the Wi-Fi password): no event tap may see these keys.
    pub fn secret_field_focused(&self) -> bool {
        self.sheet.as_ref().is_some_and(|s| s.takes_secret())
    }

    /// Turns Low Power Mode on or off from outside the page (the low-battery
    /// alert's button), saving it and refreshing the open pages.
    pub fn set_low_power(&mut self, ui: &Ui, prefs: &mut Prefs, on: bool) {
        self.opts.low_power = on;
        self.opts.save(prefs);
        self.rebuild(ui);
    }

    pub fn alert(&self) -> Option<&Alert> {
        self.alert.as_ref()
    }

    pub fn brightness(&self) -> f32 {
        self.brightness
    }

    /// Whether a push or pop is mid-flight.
    pub fn transitioning(&self) -> bool {
        self.transition.is_some()
    }

    /// Advances the slide, the top page's scroll and the sheet's keyboard;
    /// true when a frame is needed.
    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        let mut frame = false;
        if let Some(tr) = &self.transition {
            if t - tr.start >= PUSH_MS {
                self.transition = None;
            }
            frame = true;
        }
        if let Some(p) = self.list_mut() {
            frame |= p.tick(t);
        }
        if let Some(p) = &mut self.sidebar {
            frame |= p.tick(t);
        }
        frame |= self.zone_picker.as_mut().is_some_and(|p| p.tick(ui, t));
        frame |= self.time_picker.as_mut().is_some_and(|p| p.tick(t));
        if self.scan.poll() {
            self.rebuild(ui);
            frame = true;
        }
        frame |= self.poll_loads(ui, t);
        frame |= self.poll_install(ui);
        if let Some(res) = self.backup_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.backup_rx = None;
            self.backup_note = match res {
                Ok(n) => format!("Backed up {n} files."),
                Err(e) => format!("Backup failed: {e}"),
            };
            let dir = backup::destination_root(&self.data_dir, &self.opts.backup_destination);
            self.loads.latest_backup.start("backup-read", move || backup::latest(&dir).map(|(_, t)| t));
            self.rebuild(ui);
            frame = true;
        }
        if let Some(res) = self.package_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.package_rx = None;
            match res {
                Ok(()) => self.loads.removable.start("removable-read", super::pacman_update::removable_packages),
                Err(e) => self.alert = Some(Alert::new(ui, Command::Notice(e))),
            }
            self.rebuild(ui);
            frame = true;
        }
        if self.stale_depth > 0 && self.transition.is_none() && self.stack.len() < self.stale_depth {
            self.rebuild(ui);
            frame = true;
        }
        let pct = (self.updater.fraction() * 100.0) as u64;
        if self.updater.poll(super::pages::BUILD.version, super::pages::BUILD.git) {
            self.update_pct = pct;
            self.rebuild(ui);
            frame = true;
        } else if self.updater.state == update::State::Downloading && pct != self.update_pct {
            self.update_pct = pct;
            // only the progress row changes while the bytes arrive
            let bar = backup_page::progress_bar(&self.updater);
            if !self.list_mut().is_some_and(|p| p.set_bar(bar)) {
                self.rebuild_visible(ui);
            }
            frame = true;
        }
        if self.startup_disk_poll(ui) {
            frame = true;
        }
        if let Some(res) = self.diag_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.diag_rx = None;
            self.diag_note = match res {
                Ok(()) => "Shared with the developer.".into(),
                Err(e) => format!("Could not share: {e}"),
            };
            self.rebuild(ui);
            frame = true;
        }
        // an open controller test refreshes its rows when a controller's state moves
        if let Some(PageId::GameController(id)) = self.stack.last().map(|e| e.id) {
            if super::game_controllers_page::take_change() {
                let rows = super::game_controllers_page::live_rows_for(&super::services::gamepad_service::snapshot(), id);
                if let Some(p) = self.list_mut() {
                    for (label, value) in rows {
                        if let Some((g, i)) = p.find(label) {
                            p.set_value(ui, g, i, &value);
                        }
                    }
                }
                frame = true;
            }
        }
        let (changed, search_frame) = self.search.tick(ui, t);
        if changed {
            self.requery(ui);
        }
        frame |= changed | search_frame;
        frame |= self.passcode.as_ref().is_some_and(|o| o.animating(t));
        frame |= self.st_gate.as_mut().is_some_and(|(l, _)| l.refresh(crate::bare::sys::wall_time().0) | l.animating(t));
        frame | self.sheet.as_mut().is_some_and(|sh| sh.tick(ui, t).1)
    }

    /// When the app next needs a frame on its own.
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        if self.transition.is_some() || self.list().is_some_and(|p| p.animating(t)) || self.sidebar.as_ref().is_some_and(|p| p.animating(t)) || self.passcode.as_ref().is_some_and(|o| o.animating(t)) || self.st_gate.as_ref().is_some_and(|(l, _)| l.animating(t)) {
            return Some(t);
        }
        if self.scan.busy() || self.updater.busy() || self.diag_rx.is_some() || self.backup_rx.is_some() || self.package_rx.is_some() || self.install_rx.is_some() || self.loads.busy() {
            return Some(t + 100.0);
        }
        self.search.next_wake(t).or_else(|| self.zone_picker.as_ref().and_then(|p| p.next_wake(t))).or_else(|| self.time_picker.as_ref().and_then(|p| p.next_wake(t))).or_else(|| self.sheet.as_ref().and_then(|sh| sh.next_wake(t)))
    }

    fn push(&mut self, ui: &Ui, id: PageId, t: f64) {
        // a page opened by any way but Forward starts a new history: nothing is ahead of it
        self.forward.clear();
        self.start_loads(id);
        match id {
            PageId::Storage => self.scan.start(self.data_dir.clone()),
            PageId::Developer => {
                let (perf_hud, gpu_scanout) = (self.dev.perf_hud, self.dev.gpu_scanout);
                self.dev = DevState { perf_hud, gpu_scanout, ..DevState::read() };
            }
            PageId::Diagnostics => self.diag_note.clear(),
            PageId::SoftwareUpdate if matches!(self.updater.state, update::State::Idle | update::State::UpToDate | update::State::Failed(_)) => self.updater.check(),
            PageId::Backup => self.backup_note.clear(),
            PageId::Users => self.users_note.clear(),
            PageId::Sounds => self.reload_sound(),
            _ => {}
        }
        let back = self.stack.last().map(|e| e.title.clone());
        let entry = self.build(ui, id, back.as_deref());
        self.stack.push(entry);
        if self.stack.len() > 1 {
            self.transition = Some(Slide { push: true, start: t, leaving: None, imgs: None });
        }
        self.sync_forward();
    }

    /// Esc: backs out of the page on top (an open sheet or picker swallows it); false at the root.
    pub fn back_page(&mut self, t: f64) -> bool {
        if self.stack.len() <= 1 {
            return false;
        }
        if self.transition.is_none() && !self.overlay_open() {
            self.go_back(t);
        }
        true
    }

    /// Back (the pill's back half, Esc, Cmd+[): pops the top page and keeps it for Forward.
    fn go_back(&mut self, t: f64) {
        if self.stack.len() > 1 {
            let id = self.page();
            self.pop(t);
            self.forward.push(id);
            self.sync_forward();
        }
    }

    /// Forward (the pill's forward half, Cmd+]): opens again the page Back left last; false when there is none.
    pub fn forward_page(&mut self, ui: &Ui, t: f64) -> bool {
        if self.transition.is_some() || self.overlay_open() {
            return !self.forward.is_empty();
        }
        let Some(id) = self.forward.pop() else { return false };
        let ahead = std::mem::take(&mut self.forward);
        self.push(ui, id, t);
        self.forward = ahead;
        self.sync_forward();
        true
    }

    /// Whether the pill's forward half is enabled (a page Back left can be opened again).
    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Shows the forward state on the top page's toolbar pill.
    fn sync_forward(&mut self) {
        let on = !self.forward.is_empty();
        if let Some(top) = self.stack.last_mut() {
            top.nav.set_forward(on);
            if let View::List(p) = &mut top.view {
                p.set_forward(on);
            }
        }
    }

    /// Takes the top page off the stack (Back, or a page whose subject is gone, which Forward must not bring back).
    fn pop(&mut self, t: f64) {
        if self.stack.len() > 1 {
            let leaving = self.stack.pop();
            self.transition = Some(Slide { push: false, start: t, leaving, imgs: None });
        }
        self.sync_forward();
    }

    /// The scroll wheel over the window at window x `x` moved the content `dy` px: the sidebar or the page under it
    /// scrolls, and nothing is pressed (a notch over a slider must not set or save it). False when no list is there to
    /// take it (an overlay, a slide or the wallpaper grid), so the drag emulated for the wheel scrolls it as before.
    pub fn wheel(&mut self, x: f32, dy: f32) -> bool {
        if self.transition.is_some() || self.overlay_open() {
            return false;
        }
        let list = if self.side_w > 0 && x < self.side_w as f32 { self.sidebar.as_mut() } else { self.list_mut() };
        let Some(list) = list else { return false };
        list.wheel(dy);
        true
    }

    /// Handles a touch; `prefs` receives every persisted change.
    pub fn touch(&mut self, ui: &Ui, prefs: &mut Prefs, ev: Event, t: f64) -> Vec<Effect> {
        let (x, y) = match ev {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            Event::Power | Event::Wake | Event::VolumeUp | Event::VolumeDown | Event::Screenshot => return vec![],
        };
        if self.transition.is_some() {
            return vec![];
        }
        let (ev, x, y) = if self.side_w > 0 {
            if matches!(ev, Event::Down(..)) {
                self.side_touch = x < self.side_w as f32 && !self.overlay_open();
            }
            if self.side_touch {
                // the sidebar's search field first (sidebar x and y are already local), then its rows
                let frame = self.sidebar.as_ref().and_then(|sb| sb.search_frame(ui));
                let (taken, changed) = self.search.touch(ui, ev, frame, self.h, t);
                if changed {
                    self.requery(ui);
                }
                if taken {
                    return vec![];
                }
                return self.sidebar_touch(ui, prefs, ev, x, y, t);
            }
            let dx = self.side_w as f32;
            let ev = match ev {
                Event::Down(..) => Event::Down(x - dx, y),
                Event::Move(..) => Event::Move(x - dx, y),
                _ => Event::Up(x - dx, y),
            };
            (ev, x - dx, y)
        } else {
            (ev, x, y)
        };
        if let Some(picker) = &mut self.time_picker {
            let mut wake_fx = Vec::new();
            match picker.touch(ui, ev, t, |u| crate::bare::timezone::offset_at(u).unwrap_or(0)) {
                TimeEvent::None => {}
                TimeEvent::Cancel => {
                    self.time_picker = None;
                    self.notify_edge = None;
                    self.st_edge = None;
                    self.night_edge = None;
                    self.wake_edge = None;
                }
                TimeEvent::SetMinute(m) => {
                    self.time_picker = None;
                    if let Some(index) = self.wake_edge.take() {
                        wake_fx = self.wake_set_minute(ui, prefs, index, m);
                    }
                    if let Some(from) = self.st_edge.take() {
                        self.st_set_edge(ui, from, m);
                    }
                    if let Some(from) = self.night_edge.take() {
                        self.night.set_edge(from, m);
                        self.night.publish();
                        self.night.save(prefs);
                        self.rebuild(ui);
                    }
                    if let Some(edge) = self.notify_edge.take() {
                        if notify_page::set_edge(&mut self.notify.schedule, edge, m) {
                            self.notify_out.push(NotifyChange::Schedule(self.notify.schedule));
                            self.rebuild(ui);
                        }
                    }
                }
                TimeEvent::Set(unix) => self.apply_time(ui, unix),
            }
            return wake_fx;
        }
        if let Some(picker) = &mut self.zone_picker {
            match picker.touch(ui, ev, t) {
                PickerEvent::None => {}
                PickerEvent::Cancel => self.zone_picker = None,
                PickerEvent::Pick(zone) => self.pick_zone(ui, prefs, &zone),
            }
            return vec![];
        }
        if let Some((lock, edit)) = &mut self.st_gate {
            match lock.touch(ui, ev, t) {
                LockOut::Open => {}
                LockOut::Cancelled => self.st_gate = None,
                LockOut::Unlocked => {
                    let edit = *edit;
                    self.st_gate = None;
                    self.st_apply(ui, edit);
                }
            }
            return vec![];
        }
        if let Some(o) = &mut self.passcode {
            match o.touch(ui, ev, t) {
                OverlayOut::Open => {}
                OverlayOut::Cancelled => {
                    self.passcode = None;
                    self.pw_pending = None;
                }
                OverlayOut::Done => {
                    self.passcode = None;
                    match self.pw_pending.take() {
                        Some((site, account)) => self.reveal_password(ui, site, account),
                        None => self.rebuild(ui),
                    }
                }
            }
            return vec![];
        }
        if self.sheet.is_some() {
            return self.sheet_touch(ui, ev, x, y, t);
        }
        if let Some(alert) = &self.alert {
            if let Event::Up(..) = ev {
                match alert.hit(ui, self.w, self.h, x, y) {
                    Some(AlertHit::Confirm) => {
                        let cmd = self.alert.take().map(|a| a.command).expect("alert");
                        return self.run(ui, prefs, cmd, t);
                    }
                    Some(AlertHit::Cancel) => self.alert = None,
                    Some(AlertHit::Reopen) => {
                        if let Some(a) = self.alert.as_mut() {
                            let on = !a.reopen().unwrap_or(true);
                            a.card.set_checked(on);
                            let _ = prefs.set(super::session::REOPEN_PREF, if on { "1" } else { "0" });
                        }
                    }
                    None => {}
                }
            }
            return vec![];
        }
        if self.stack.len() == 1 && self.side_w == 0 {
            let frame = self.list().and_then(|p| p.search_frame(ui));
            let (taken, changed) = self.search.touch(ui, ev, frame, self.h, t);
            if changed {
                self.requery(ui);
            }
            if taken {
                return vec![];
            }
        }
        match self.page_touch(ui, ev, x, y, t) {
            Outcome::Nothing => vec![],
            Outcome::Pop => {
                self.go_back(t);
                vec![]
            }
            Outcome::Slide(g, i, release) => self.slide(ui, prefs, g, i, x, release),
            Outcome::Activate(hit) => self.activate(ui, prefs, hit, t),
            Outcome::Wallpaper(tap) => self.wallpaper_tap(ui, prefs, tap),
        }
    }

    fn overlay_open(&self) -> bool {
        self.time_picker.is_some() || self.zone_picker.is_some() || self.st_gate.is_some() || self.passcode.is_some() || self.sheet.is_some() || self.alert.is_some()
    }

    /// A touch that began on the sidebar: scroll it, or open the row that was tapped.
    fn sidebar_touch(&mut self, ui: &Ui, prefs: &mut Prefs, ev: Event, x: f32, y: f32, t: f64) -> Vec<Effect> {
        let Some(sb) = &mut self.sidebar else { return vec![] };
        match ev {
            Event::Down(..) => {
                self.side_pressed = sb.hit(ui, x, y);
                sb.scroll_down(y, t);
            }
            Event::Move(..) => {
                sb.scroll_moved(y, t);
                if sb.dragging() {
                    self.side_pressed = None;
                }
            }
            _ => {
                let tap = sb.scroll_up(y, t);
                let hit = sb.hit(ui, x, y);
                self.pressed = None;
                match self.side_pressed.take() {
                    Some(p) if tap && Some(p) == hit => return self.sidebar_activate(ui, prefs, p, t),
                    _ => {}
                }
            }
        }
        vec![]
    }

    fn sidebar_activate(&mut self, ui: &Ui, prefs: &mut Prefs, hit: Hit, t: f64) -> Vec<Effect> {
        let Hit::Row(g, i) = hit else { return vec![] };
        let Some(item) = self.sidebar.as_ref().map(|p| p.item(g, i).clone()) else { return vec![] };
        match item.action {
            // a Search result may be a page inside a pane (Night Shift): it goes on its pane, so Back leads there
            Action::Push(id) if !self.search.query().trim().is_empty() => self.open_page_id(ui, id, t),
            Action::Push(id) => self.select(ui, id, t),
            Action::Toggle(which) => self.toggle(ui, prefs, which, g, i, t),
            _ => vec![],
        }
    }

    /// Shows `id` as the detail pane's first page (tapping the open one again pops back to it).
    fn select(&mut self, ui: &Ui, id: PageId, t: f64) -> Vec<Effect> {
        self.pressed = None;
        self.transition = None;
        if self.stack.first().is_some_and(|e| e.id == id) {
            self.stack.truncate(1);
            self.forward.clear();
            self.sync_forward();
            return vec![];
        }
        self.stack.clear();
        self.push(ui, id, t);
        match id {
            PageId::Wifi => vec![Effect::Wifi(WifiAction::Scan)],
            PageId::Bluetooth => vec![Effect::Bluetooth(BtCmd::Scan)],
            _ => vec![],
        }
    }

    /// Routes a touch through the top page's own hit testing and scrolling.
    fn page_touch(&mut self, ui: &Ui, ev: Event, x: f32, y: f32, t: f64) -> Outcome {
        let (w, h) = (self.w, self.h);
        let Some(top) = self.stack.last_mut() else { return Outcome::Nothing };
        match &mut top.view {
            View::Wallpaper(page) => {
                let back = top.nav.hit_back(ui, x, y);
                match ev {
                    Event::Down(..) => self.pressed_thumb = if page.previewing() { None } else { page.hit(ui, w, h, x, y) },
                    Event::Up(..) => {
                        let held = self.pressed_thumb.take();
                        if back {
                            return Outcome::Pop;
                        }
                        if page.previewing() {
                            if let Some(tap) = page.tap(ui, w, h, x, y) {
                                return Outcome::Wallpaper(tap);
                            }
                        } else if let Some(tap @ (Tap::Dynamic | Tap::Look(_) | Tap::ShowOn(_) | Tap::Add(_))) = page.tap(ui, w, h, x, y) {
                            // the Dynamic Wallpaper switch, the Live Look row and the desktop's Show On row and add
                            // buttons sit above the grid and are not thumbnails
                            return Outcome::Wallpaper(tap);
                        } else if let Some(i) = held.filter(|&i| page.hit(ui, w, h, x, y) == Some(i)) {
                            return Outcome::Wallpaper(Tap::Thumb(i));
                        }
                    }
                    _ => {}
                }
                Outcome::Nothing
            }
            View::List(page) => match ev {
                Event::Down(..) => {
                    self.pressed = page.hit(ui, x, y);
                    // with a pointer only the switch (or its label) flips a switch row, not a click on its empty middle
                    if let Some(Hit::Row(g, i)) = self.pressed {
                        if super::pages::pointer_form() && !page.on_switch_or_label(ui, g, i, x) {
                            self.pressed = None;
                        }
                    }
                    if let Some(Hit::Slider(g, i)) = self.pressed {
                        self.drag = Some((g, i));
                        return Outcome::Slide(g, i, false);
                    }
                    page.scroll_down(y, t);
                    Outcome::Nothing
                }
                Event::Move(..) => {
                    if let Some((g, i)) = self.drag {
                        return Outcome::Slide(g, i, false);
                    }
                    page.scroll_moved(y, t);
                    if page.dragging() {
                        self.pressed = None;
                    }
                    Outcome::Nothing
                }
                _ => {
                    if let Some((g, i)) = self.drag.take() {
                        self.pressed = None;
                        return Outcome::Slide(g, i, true);
                    }
                    let tap = page.scroll_up(y, t);
                    let hit = page.hit(ui, x, y);
                    match self.pressed.take() {
                        Some(p) if tap && Some(p) == hit => Outcome::Activate(p),
                        _ => Outcome::Nothing,
                    }
                }
            },
        }
    }

    fn sheet_touch(&mut self, ui: &Ui, ev: Event, x: f32, y: f32, t: f64) -> Vec<Effect> {
        let (w, h) = (self.w, self.h);
        let Some(sh) = &mut self.sheet else { return vec![] };
        if sh.field_touch(ui, w, h, ev, t) {
            return vec![];
        }
        let kb_top = sh.keyboard_top(ui, h).unwrap_or(f32::MAX);
        let hit = sh.hit(ui, w, h, x, y);
        match ev {
            Event::Down(..) => {
                self.pressed_sheet = hit;
                if hit == Some(SheetHit::Keyboard) {
                    sh.keyboard.down(x, y - kb_top, t);
                }
            }
            Event::Move(..) => {
                if self.pressed_sheet == Some(SheetHit::Keyboard) {
                    sh.keyboard.moved(x, y - kb_top);
                }
            }
            _ => {
                let pressed = self.pressed_sheet.take();
                if pressed == Some(SheetHit::Keyboard) {
                    let out = sh.keyboard.up(x, y - kb_top, t);
                    return out.map_or(vec![], |o| self.key(ui, o, t));
                }
                if pressed.is_some() && pressed == hit {
                    match hit {
                        Some(SheetHit::Cancel) => self.sheet = None,
                        Some(SheetHit::Join) => return self.join(ui, t),
                        Some(SheetHit::Field) => sh.press(ui, w, h, x, y, t),
                        _ => {}
                    }
                }
            }
        }
        vec![]
    }

    /// Applies a key from the password sheet's keyboard.
    /// Every pane and setting Search finds, walked from the root page's live specs (see `settings_search::index`).
    fn build_index(&self) -> Vec<settings_search::Entry> {
        settings_search::index(|id| self.spec_for(id))
    }

    /// The root page's rows while Search has a query: its matches. None when there is no query.
    fn search_results(&self) -> Option<super::list::Spec> {
        let query = self.search.query();
        if query.trim().is_empty() {
            return None;
        }
        Some(settings_search::results(self.index.as_deref().unwrap_or(&[]), query))
    }

    /// The page of each search hit, in the order the results list them (empty without a query).
    fn hit_pages(&self) -> Vec<PageId> {
        let query = self.search.query();
        if query.trim().is_empty() {
            return Vec::new();
        }
        settings_search::matches(self.index.as_deref().unwrap_or(&[]), query).iter().map(|e| e.page).collect()
    }

    /// The hit the sidebar fills: the first one on the deepest open page that is a hit (one opened by a tap or an
    /// arrow key), so Return opens what is highlighted.
    fn highlighted_hit(&self, hits: &[PageId]) -> Option<usize> {
        self.stack.iter().rev().find_map(|e| hits.iter().position(|p| *p == e.id))
    }

    /// The page whose sidebar rows are filled: the highlighted hit while Search has a query, else the open pane.
    fn side_selected(&self) -> PageId {
        let hits = self.hit_pages();
        match self.highlighted_hit(&hits) {
            Some(i) => hits[i],
            None => self.stack.first().map_or(PageId::Root, |e| e.id),
        }
    }

    /// Rebuilds the root page for a changed search query, from the top.
    fn requery(&mut self, ui: &Ui) {
        // the index is built when a query starts (so it shows the rows as they are now) and dropped when it ends
        if self.search.query().trim().is_empty() {
            self.index = None;
        } else if self.index.is_none() {
            self.index = Some(self.build_index());
        }
        // a query replaces the root list with its matches
        if self.stack.first().is_some_and(|e| e.id == PageId::Root) {
            self.rebuild_pages(ui, 0..1);
        }
        self.rebuild_sidebar(ui);
        // a filtered sidebar starts at its top; the detail pane keeps its place
        if let Some(sb) = &mut self.sidebar {
            sb.jump_to(0.0);
        } else if let Some(p) = self.list_mut() {
            p.jump_to(0.0);
        }
    }

    /// Whether typing reaches a text field, by the routing of `key`: the zone picker's query while it is up, else the
    /// focused field of the sheet, else the sidebar or root search while it is open.
    pub fn text_focused(&self) -> bool {
        if let Some(p) = &self.zone_picker {
            return p.text_focused();
        }
        match &self.sheet {
            Some(sh) => sh.text_focused(),
            None => self.search.active && (self.stack.len() == 1 || self.side_w > 0),
        }
    }

    pub fn key(&mut self, ui: &Ui, out: Out, t: f64) -> Vec<Effect> {
        if let Some(p) = &mut self.zone_picker {
            p.key(ui, out, t);
            return vec![];
        }
        if self.sheet.is_none() && self.search.active && (self.stack.len() == 1 || self.side_w > 0) {
            // Up and Down step through the hits, opening each; Return opens the highlighted hit, else the first (the
            // field keeps its query, so the results stay in the sidebar)
            let hits = self.hit_pages();
            if let Out::Named(dir @ ("Up" | "Down")) = out {
                return match step_hit(&hits, self.highlighted_hit(&hits), dir == "Down") {
                    Some(id) => self.open_page_id(ui, id, t),
                    None => vec![],
                };
            }
            let target = matches!(out, Out::Enter).then(|| hits.get(self.highlighted_hit(&hits).unwrap_or(0)).copied()).flatten();
            if self.search.edit(ui, out, t) {
                self.requery(ui);
            }
            if let Some(id) = target {
                return self.open_page_id(ui, id, t);
            }
            return vec![];
        }
        let submit = self.sheet.as_mut().is_some_and(|sh| sh.edit(ui, out, t));
        if submit {
            self.join(ui, t)
        } else {
            vec![]
        }
    }

    fn join(&mut self, ui: &Ui, t: f64) -> Vec<Effect> {
        // a one-field prompt (`forms`) finishes its flow here: the next prompt, a change through privd, or an alert
        if let Some((purpose, text)) = self.sheet.as_ref().and_then(|sh| sh.form_result()) {
            self.sheet = None;
            let r = match purpose {
                super::forms::Purpose::ComputerName | super::forms::Purpose::LocalHostname | super::forms::Purpose::ScreenSharingAccount { .. } => {
                    let r = super::sharing_page::finish(&purpose, &text).map(|_| None);
                    self.loads.sharing.start("sharing-read", super::sharing_page::View::load);
                    r
                }
                super::forms::Purpose::PrinterAddress => {
                    self.start_print_edit(ui, move || super::print_scan_page::finish(&purpose, &text).map(|_| None));
                    return vec![];
                }
                _ => return self.form_done(ui, &purpose, &text, t),
            };
            self.edit_result(ui, t, r);
            return vec![];
        }
        match self.sheet.take_if(|sh| sh.can_join()) {
            Some(sh) => vec![Effect::Wifi(sh.action())],
            None => vec![],
        }
    }

    /// A one-field prompt was submitted: privd gets the change, or the flow opens its next prompt.
    /// A Location Services change: the switches and System location update the page now, and the shell saves the pref.
    fn location_act(&mut self, ui: &Ui, edit: super::location_page::Edit, t: f64) -> Vec<Effect> {
        use super::location_page::Edit;
        let pairs = match edit {
            Edit::Services(on) => self.location.set_services(on),
            Edit::Precise { app, on } => self.location.set_precise(&app, on),
            Edit::Access { app, choice } => {
                super::services::privacy_permissions::set_location_access(&app, choice);
                vec![]
            }
            Edit::ClearLocation => self.location.clear_manual(),
            Edit::SetLocation => {
                let current = self.location.manual.map_or_else(String::new, |(la, _)| format!("{la}"));
                self.sheet = Some(Sheet::form(ui, self.w, t, super::forms::Prompt::set_latitude(&current)));
                return vec![];
            }
        };
        self.rebuild_visible(ui);
        if pairs.is_empty() {
            vec![]
        } else {
            vec![Effect::Location(pairs)]
        }
    }

    fn form_done(&mut self, ui: &Ui, purpose: &super::forms::Purpose, text: &str, t: f64) -> Vec<Effect> {
        if super::location_page::owns(purpose) {
            return match super::location_page::step(&mut self.location, purpose, text) {
                Ok(super::location_page::Flow::Next(next)) => {
                    self.sheet = Some(Sheet::form(ui, self.w, t, next));
                    vec![]
                }
                Ok(super::location_page::Flow::Save(pairs)) => {
                    self.rebuild_visible(ui);
                    vec![Effect::Location(pairs)]
                }
                Err(e) => {
                    self.edit_result(ui, t, Err(e));
                    vec![]
                }
            };
        }
        if let super::forms::Purpose::BluetoothPairing { .. } = purpose {
            return vec![Effect::Bluetooth(crate::bare::bluetooth::Cmd::AnswerPairing(Some(text.to_string())))];
        }
        if super::account_page::owns(purpose) {
            match super::account_page::next_prompt(purpose, text) {
                super::account_page::Step::Ask(next) => self.sheet = Some(Sheet::form(ui, self.w, t, next)),
                super::account_page::Step::SignIn { email, password } => {
                    let root = super::apphost::root();
                    let for_done = email.clone();
                    self.account_note.clear();
                    self.loads.account_done.start("account-sign-in", move || {
                        let result = super::account_page::sign_in(&root, &email, &password);
                        super::account_page::Landed::SignedIn { email: for_done, result }
                    });
                }
            }
            self.rebuild_visible(ui);
            return vec![];
        }
        if super::internet_accounts_page::owns(purpose) {
            let r =super::internet_accounts_page::list_path().and_then(|path| super::internet_accounts_page::finish(&path, &super::apphost::root(), purpose, text));
            match r {
                Ok(Some(next)) => self.sheet = Some(Sheet::form(ui, self.w, t, next)),
                Ok(None) => self.ia_note.clear(),
                Err(e) => self.ia_note = e,
            }
            self.rebuild_visible(ui);
            return vec![];
        }
        let finished = super::users_page::finish(purpose, text);
        self.loads.users.start("users-read", super::users_page::View::load);
        match finished {
            Ok(Some(next)) => self.sheet = Some(Sheet::form(ui, self.w, t, next)),
            Ok(None) => self.users_note.clear(),
            Err(e) => self.users_note = e,
        }
        self.rebuild_visible(ui);
        vec![]
    }

    /// Users & Groups: an edit applies now, or opens the prompt that asks for what it still needs.
    fn users_act(&mut self, ui: &Ui, edit: super::users_page::Edit, t: f64) -> Vec<Effect> {
        let view = self.loads.users.get().cloned().unwrap_or_else(super::users_page::View::load);
        match super::users_page::apply(&edit, &view) {
            Ok(Some(prompt)) => {
                self.users_note.clear();
                self.sheet = Some(Sheet::form(ui, self.w, t, prompt));
            }
            Ok(None) => self.users_note.clear(),
            Err(e) => self.users_note = e,
        }
        self.loads.users.start("users-read", super::users_page::View::load);
        self.rebuild_visible(ui);
        vec![]
    }

    /// A Startup Disk row was tapped: the change waits for an administrator's approval on the shell's password sheet
    /// (`local_auth::queue`), and `startup_disk_poll` finishes it when the sheet answers.
    fn startup_disk_act(&mut self, ui: &Ui, edit: super::startup_disk_page::Edit) -> Vec<Effect> {
        use super::services::local_auth;
        use super::startup_disk_page as page;
        let picked = match self.loads.startup.get() {
            Some(v) => page::choose(v, &edit).map(|d| (d.id.clone(), page::reason(d))),
            None => Err("The list of systems has not been read yet.".to_string()),
        };
        match picked {
            Ok((id, reason)) if self.startup_pending.is_none() => {
                // an answer left from an earlier sheet must not approve this change
                let _ = local_auth::take_result(page::APP);
                if local_auth::is_waiting(page::APP) || !local_auth::queue(page::APP, &reason) {
                    self.alert = Some(Alert::new(ui, Command::Notice("An approval is already waiting. Finish it first.".into())));
                } else {
                    self.startup_pending = Some(id);
                }
            }
            Ok(_) => {}
            Err(e) => self.alert = Some(Alert::new(ui, Command::Notice(e))),
        }
        self.rebuild_visible(ui);
        vec![]
    }

    /// Takes the password sheet's answer for a waiting Startup Disk change: an approval sets the entry through privd,
    /// a cancel does nothing, anything else says why nothing changed. True when the page changed.
    fn startup_disk_poll(&mut self, ui: &Ui) -> bool {
        use super::startup_disk_page as page;
        let Some(id) = self.startup_pending.clone() else { return false };
        let Some(answer) = super::services::local_auth::take_result(page::APP) else { return false };
        self.startup_pending = None;
        match page::outcome(answer) {
            page::Outcome::Approved => match page::apply(&id) {
                Ok(()) => self.loads.startup.start("startup-disk-read", page::View::load),
                Err(e) => self.alert = Some(Alert::new(ui, Command::Notice(e))),
            },
            page::Outcome::Cancelled => {}
            page::Outcome::Refused(why) => self.alert = Some(Alert::new(ui, Command::Notice(why))),
        }
        self.rebuild_visible(ui);
        true
    }

    /// Shows what a page edit asked for: the next prompt on a sheet, the page rebuilt, or an alert with the reason
    /// the change did not happen.
    fn edit_result(&mut self, ui: &Ui, t: f64, r: Result<Option<super::forms::Prompt>, String>) {
        match r {
            Ok(Some(p)) => self.sheet = Some(Sheet::form(ui, self.w, t, p)),
            Ok(None) => self.rebuild_visible(ui),
            Err(e) => self.alert = Some(Alert::new(ui, Command::Notice(e))),
        }
    }

    /// Saves a Language & Region change to `settings.ini`, publishes it for the menu bar clock and the
    /// pages, and asks privd to set the system locale (best effort: a locale that is not generated
    /// leaves the system as it was, and the shell and apps still use the saved choice).
    fn change_region(&mut self, prefs: &mut Prefs, edit: impl FnOnce(&mut bifrost_ui::region::Region)) {
        let mut r = bifrost_ui::region::current();
        let before = r.languages.clone();
        edit(&mut r);
        let languages_changed = r.languages != before;
        let languages = r.languages.join(",");
        let region = r.region.clone();
        let weekday = r.first_weekday.map(|d| d.to_string()).unwrap_or_default();
        let currency = r.currency.clone().unwrap_or_default();
        if let Err(e) = prefs.set_many(&[("languages", languages.as_str()), ("region", region.as_str()), ("first_weekday", weekday.as_str()), ("currency", currency.as_str())]) {
            eprintln!("springboard: settings.ini: {e}");
        }
        let locale = r.identifier();
        bifrost_ui::region::publish(r);
        if languages_changed {
            // Running apps watch this generation (bifrost_sdk::locale::LanguageWatch) and reload their strings.
            if let Err(e) = bifrost_core::language_generation::announce_language_change(&super::apphost::root()) {
                eprintln!("springboard: language generation: {e}");
            }
        }
        crate::bare::sys::spawn_named("set-locale", move || {
            let _ = crate::privd_client::connect().set_locale(&locale, &locale);
        });
    }

    /// Applies a zone chosen in the picker: the clock and the page follow at once.
    fn pick_zone(&mut self, ui: &Ui, prefs: &mut Prefs, zone: &str) {
        self.zone_picker = None;
        if !crate::bare::timezone::set(zone) {
            return;
        }
        if let Err(e) = prefs.set("tz", zone) {
            eprintln!("springboard: settings.ini: {e}");
        }
        let (hour, min, ..) = crate::bare::sys::local_time();
        self.info.timezone = zone.to_string();
        self.info.hour = hour;
        self.info.min = min;
        self.rebuild(ui);
    }

    /// Sets the clock to a time chosen in the picker (manual mode only).
    fn apply_time(&mut self, ui: &Ui, unix: i64) {
        self.time_picker = None;
        if let Err(e) = crate::bare::sys::set_realtime(unix, 0) {
            eprintln!("springboard: set clock: {e}");
            return;
        }
        let (hour, min, ..) = crate::bare::sys::local_time();
        self.info.hour = hour;
        self.info.min = min;
        self.rebuild(ui);
    }

    fn wallpaper_tap(&mut self, ui: &Ui, prefs: &mut Prefs, tap: Tap) -> Vec<Effect> {
        let (w, h) = (self.w, self.h);
        let Some(View::Wallpaper(page)) = self.stack.last_mut().map(|e| &mut e.view) else { return vec![] };
        let save = |prefs: &mut Prefs, k: &str, v: &str| {
            if let Err(e) = prefs.set(k, v) {
                eprintln!("springboard: settings.ini: {e}");
            }
        };
        match tap {
            // the desktop applies the wallpaper at once, where Show On says (Golden Gate's Wallpaper pane)
            Tap::Thumb(i) if page.desktop() => {
                let target = page.show_on().target();
                self.wallpaper_tap(ui, prefs, Tap::Set(i, target))
            }
            Tap::Thumb(i) => {
                page.open_preview(ui, w, h, i);
                vec![]
            }
            Tap::ShowOn(o) => {
                page.set_show_on(o);
                self.wallpaper_show_on = o;
                save(prefs, SHOW_ON_PREF, o.pref());
                vec![]
            }
            Tap::Add(kind) => {
                self.wallpaper_import = Some(kind);
                vec![]
            }
            Tap::Cancel => {
                page.close_preview();
                vec![]
            }
            Tap::Dynamic => {
                let on = !page.dynamic();
                page.set_dynamic(on);
                self.wallpaper_dynamic = on;
                save(prefs, "wallpaper_dynamic", if on { "1" } else { "0" });
                // off: the still the user picked comes back at once; on: the shell changes it on its next tick
                if on {
                    vec![]
                } else {
                    vec![Effect::Wallpaper(self.info.wallpaper.clone())]
                }
            }
            Tap::Look(look) => {
                page.set_look(look);
                self.wallpaper_look = look;
                save(prefs, LOOK_PREF, look.pref());
                vec![Effect::LiveWallpaper(look.pref().to_string())]
            }
            Tap::Dim => {
                let on = !page.dim();
                page.set_dim(on);
                self.wallpaper_dim = on;
                save(prefs, "wallpaper_dim", if on { "1" } else { "0" });
                vec![Effect::WallpaperDim(on)]
            }
            Tap::Set(i, target) => {
                let Some(id) = page.id_of(i).map(String::from) else { return vec![] };
                page.close_preview();
                let mut fx = Vec::new();
                if target == Target::Lock {
                    save(prefs, "wallpaper_lock", &id);
                    return vec![Effect::LockWallpaper(id)];
                }
                page.select(i);
                // Home alone pins a lock screen that still follows the home screen
                if target == Target::Home && prefs.get("wallpaper_lock").unwrap_or("").is_empty() {
                    let old = if self.info.wallpaper.is_empty() { "default".to_string() } else { self.info.wallpaper.clone() };
                    save(prefs, "wallpaper_lock", &old);
                    fx.push(Effect::LockWallpaper(old));
                }
                if target == Target::Both {
                    save(prefs, "wallpaper_lock", "");
                    fx.push(Effect::LockWallpaper(String::new()));
                }
                self.info.wallpaper = id.clone();
                save(prefs, "wallpaper", &id);
                let name = self.wallpaper_name();
                self.set_root_value(ui, "Wallpaper", &name);
                fx.push(Effect::Wallpaper(id));
                fx
            }
        }
    }

    /// Updates a value on the root page without rebuilding it.
    fn set_root_value(&mut self, ui: &Ui, label: &str, value: &str) {
        let root = match &mut self.sidebar {
            Some(sb) => Some(sb),
            None => match self.stack.first_mut().map(|e| &mut e.view) {
                Some(View::List(root)) => Some(root),
                _ => None,
            },
        };
        if let Some(root) = root {
            if let Some((g, i)) = root.find(label) {
                root.set_value(ui, g, i, value);
            }
        }
    }

    /// Updates a value on the page under the top one (the one a pick
    /// returns to).
    fn set_parent_value(&mut self, ui: &Ui, label: &str, value: &str) {
        let n = self.stack.len();
        if n < 2 {
            return;
        }
        if let View::List(p) = &mut self.stack[n - 2].view {
            if let Some((g, i)) = p.find(label) {
                p.set_value(ui, g, i, value);
            }
        }
    }

    /// A slider drag at `x`: updates the row and emits the live effect;
    /// `release` commits the value.
    fn slide(&mut self, ui: &Ui, prefs: &mut Prefs, g: usize, i: usize, x: f32, release: bool) -> Vec<Effect> {
        let Some(page) = self.list() else { return vec![] };
        let Action::Slider(which) = page.item(g, i).action.clone() else { return vec![] };
        let (tx, _, tw) = page.track(ui, g, i);
        let v = slider_value(x, tx, tw);
        match which {
            Slider::Brightness => {
                self.brightness = v;
                self.list_mut().expect("list").set_slider(g, i, v);
                vec![Effect::Brightness(v)]
            }
            Slider::Volume => {
                self.opts.volume = v;
                self.sound.volume = v;
                self.list_mut().expect("list").set_slider(g, i, v);
                if release {
                    self.opts.save(prefs);
                }
                vec![Effect::Volume(v)]
            }
            Slider::Warmth => {
                self.night.warmth = v;
                self.night.publish();
                self.list_mut().expect("list").set_slider(g, i, v);
                if release {
                    self.night.save(prefs);
                }
                vec![]
            }
            Slider::CursorSize => {
                let v = v.clamp(0., 1.);
                self.a11y.cursor_size = v;
                self.list_mut().expect("list").set_slider(g, i, v);
                if release {
                    self.a11y.save(prefs);
                }
                vec![]
            }
            Slider::DockSize => {
                let v = v.clamp(0., 1.);
                let size = 32. + v * 48.;
                self.list_mut().expect("list").set_slider(g, i, v);
                let delta = size - self.dock.size;
                if delta != 0. {
                    self.dock.size = size;
                    self.dock_out.push(super::desktop_dock::Change::Size(delta));
                }
                vec![]
            }
            Slider::DockMagnify => {
                let v = v.clamp(0., 1.);
                self.list_mut().expect("list").set_slider(g, i, v);
                // 0..1 maps onto the dock's 48..128 pt magnified size; the dock applies the change
                let delta = (v - self.desk.magnify_size) * 80.;
                self.desk.magnify_size = v;
                if delta != 0. {
                    self.dock_out.push(super::desktop_dock::Change::MagnifiedSize(delta));
                }
                if release {
                    let _ = prefs.set(super::pages::DOCK_MAGNIFY_SIZE_PREF, &format!("{v:.3}"));
                }
                vec![]
            }
            Slider::KeyRepeatDelay | Slider::KeyRepeatRate => {
                let v = v.clamp(0., 1.);
                match which {
                    Slider::KeyRepeatDelay => self.opts.key_repeat_delay_ms = super::options::repeat_delay_from_slider(v),
                    _ => self.opts.key_repeat_rate = super::options::repeat_rate_from_slider(v),
                }
                self.list_mut().expect("list").set_slider(g, i, v);
                self.apply_keys();
                if release {
                    self.opts.save(prefs);
                }
                vec![]
            }
            Slider::MouseSpeed | Slider::MouseScroll | Slider::TrackpadSpeed => {
                let v = v.clamp(0., 1.);
                let speed = super::options::speed_from_slider(v);
                match which {
                    Slider::MouseSpeed => self.opts.mouse_speed = speed,
                    Slider::MouseScroll => self.opts.mouse_scroll = speed,
                    _ => self.opts.trackpad_speed = speed,
                }
                self.list_mut().expect("list").set_slider(g, i, v);
                self.apply_pointer();
                if release {
                    self.opts.save(prefs);
                }
                vec![]
            }
            Slider::MouseDoubleClick => {
                let v = v.clamp(0., 1.);
                self.opts.double_click_ms = super::mouse_page::double_click_from_slider(v);
                self.list_mut().expect("list").set_slider(g, i, v);
                self.apply_pointer();
                if release {
                    self.opts.save(prefs);
                }
                vec![]
            }
            Slider::LiquidGlass => {
                let v = v.clamp(0., 1.);
                self.list_mut().expect("list").set_slider(g, i, v);
                bifrost_render::glass::set_clarity(v);
                if release {
                    let _ = prefs.set(LIQUID_GLASS_PREF, &format!("{v:.3}"));
                }
                vec![]
            }
            Slider::TextSize => {
                let step = notch(v, TEXT_SIZES.len());
                self.list_mut().expect("list").set_slider(g, i, step as f32 / (TEXT_SIZES.len() - 1) as f32);
                // applied live while dragging; persisted on release (`save` skips unchanged keys)
                self.opts.text_scale = TEXT_SIZES[step];
                if release {
                    self.opts.save(prefs);
                }
                vec![]
            }
        }
    }

    fn activate(&mut self, ui: &Ui, prefs: &mut Prefs, hit: Hit, t: f64) -> Vec<Effect> {
        // (group, item, the pop-up choice when the touch chose one from an open menu)
        let (g, i, choice) = match hit {
            Hit::Back => {
                self.go_back(t);
                return vec![];
            }
            Hit::Forward => {
                self.forward_page(ui, t);
                return vec![];
            }
            Hit::Row(g, i) => (g, i, None),
            Hit::Choice(k) => match self.list_mut().and_then(|p| p.close_popup()) {
                Some((g, i)) => (g, i, Some(k)),
                None => return vec![],
            },
            Hit::Dismiss => {
                if let Some(p) = self.list_mut() {
                    p.close_popup();
                }
                return vec![];
            }
            // a segment of a segmented row chooses like a pop-up's choice does
            Hit::Segment(g, i, k) => (g, i, Some(k)),
            Hit::Slider(..) => return vec![],
        };
        let Some(page) = self.list() else { return vec![] };
        let item = page.item(g, i).clone();
        if choice.is_none() && item.popup.is_some() {
            // a pop-up row opens its menu; the menu's choice comes back as `Hit::Choice`
            if let Some(p) = self.list_mut() {
                p.open_popup(ui, g, i);
            }
            return vec![];
        }
        if choice.is_none() && item.segments.is_some() {
            // a segmented row is chosen by its segments (`Hit::Segment`), never as a whole
            return vec![];
        }
        if !item.tappable() {
            return vec![];
        }
        // a choice sends the row's action with the chosen index
        let action = match choice {
            Some(k) => match super::pages::chosen(&item.action, k) {
                Some(action) => {
                    // the button or the segment shows the new choice at once
                    if let Some(p) = self.list_mut() {
                        p.set_choice(ui, g, i, k);
                    }
                    action
                }
                None => return vec![],
            },
            None => item.action.clone(),
        };
        match action {
            Action::None | Action::Slider(_) => vec![],
            Action::Push(id) => {
                if matches!(id, PageId::WifiDetail | PageId::BluetoothDetail | PageId::NotificationApp | PageId::UserDetail | PageId::GroupDetail) {
                    self.detail = item.label.clone();
                }
                self.push(ui, id, t);
                match id {
                    PageId::Wifi => vec![Effect::Wifi(WifiAction::Scan)],
                    PageId::Bluetooth => vec![Effect::Bluetooth(BtCmd::Scan)],
                    _ => vec![],
                }
            }
            Action::Users(edit) => self.users_act(ui, edit, t),
            Action::Firewall(id, edit) => {
                // the row shows the new choice at once; the shell saves it and reads the page back
                if let Some(v) = self.firewall.iter_mut().find(|v| v.app_id == id) {
                    match edit {
                        super::firewall_sheet::Edit::Allow => v.policy = bifrost_core::app_firewall::Policy::Allow,
                        super::firewall_sheet::Edit::Block => v.policy = bifrost_core::app_firewall::Policy::Block,
                        super::firewall_sheet::Edit::Forget => {}
                    }
                }
                if edit == super::firewall_sheet::Edit::Forget {
                    self.firewall.retain(|v| v.app_id != id);
                }
                self.firewall_out.push((id, edit));
                self.rebuild_visible(ui);
                vec![]
            }
            Action::LoginItemRemove(id) => {
                self.login_items.retain(|(a, _)| *a != id);
                self.login_out.push(id);
                self.rebuild_visible(ui);
                vec![]
            }
            Action::GameController(edit) => {
                let _ = super::game_controllers_page::apply(prefs, &edit);
                self.rebuild_visible(ui);
                vec![]
            }
            Action::Assistant(edit) => {
                let _ = super::assistant_page::apply(prefs, &edit);
                self.rebuild_visible(ui);
                vec![]
            }
            Action::AppPane(slot, pane) => {
                // the app opens the pane in its own window; a refusal is logged and leaves Settings as it is
                if let Err(e) = super::settings_panes::open(slot, &pane) {
                    eprintln!("springboard: settings pane {pane}: {e}");
                }
                vec![]
            }
            Action::BackgroundAgentToggle(id) => {
                let off = &mut self.login_ext.background_off;
                match off.iter().position(|a| *a == id) {
                    Some(i) => {
                        off.remove(i);
                    }
                    None => off.push(id),
                }
                let _ = prefs.set(super::login_items_page::BACKGROUND_OFF_KEY, &super::login_items_page::background_off_to_pref(&off[..]));
                self.rebuild_visible(ui);
                vec![]
            }
            Action::ExtensionToggle(key) => {
                let off = &mut self.login_ext.extensions_off;
                match off.iter().position(|k| *k == key) {
                    Some(i) => {
                        off.remove(i);
                    }
                    None => off.push(key),
                }
                let _ = prefs.set(super::login_items_page::EXTENSIONS_OFF_KEY, &off.join(","));
                self.rebuild_visible(ui);
                vec![]
            }
            Action::RefreshToggle(id) => {
                let off = &mut self.login_ext.refresh_off;
                match off.iter().position(|a| *a == id) {
                    Some(i) => {
                        off.remove(i);
                    }
                    None => off.push(id),
                }
                let _ = prefs.set(super::services::background::PREF_OFF, &super::services::background::off_to_pref(&off[..]));
                self.rebuild_visible(ui);
                vec![]
            }
            Action::LoginItemMove(id) => {
                if let Some(i) = self.login_items.iter().position(|(a, _)| *a == id) {
                    if i > 0 {
                        self.login_items.swap(i - 1, i);
                        self.login_moved.push(id);
                    }
                }
                self.rebuild_visible(ui);
                vec![]
            }
            Action::LoginItemApprove(id) => {
                if let Some(i) = self.login_pending.iter().position(|(a, _)| *a == id) {
                    let (a, name) = self.login_pending.remove(i);
                    self.login_items.push((a, name));
                    self.login_approve_out.push(id);
                }
                self.rebuild_visible(ui);
                vec![]
            }
            Action::Font(role, path) => {
                if let Err(e) = prefs.set(role.pref(), path.as_deref().unwrap_or("")) {
                    eprintln!("springboard: settings.ini: {e}");
                }
                let i = role.index();
                self.loads.fonts.value.names[i] = path.as_deref().and_then(bifrost_render::fontprefs::family);
                self.loads.fonts.value.overrides[i] = path;
                self.rebuild_visible(ui);
                vec![]
            }
            Action::DefaultApp(category, app) => {
                if let Some(view) = self.defaults.get_mut(category) {
                    view.current = Some(app.clone());
                }
                self.defaults_out.push((category, app));
                self.rebuild_visible(ui);
                vec![]
            }
            Action::Toggle(which) => self.toggle(ui, prefs, which, g, i, t),
            Action::Passcode(purpose, custom) => {
                self.passcode = Some(Overlay::new(&self.data_dir.join("passcode.ini"), purpose, custom, self.w, self.h));
                vec![]
            }
            Action::Passwords(act) => self.passwords_act(ui, act),
            Action::InternetAccounts(act) => self.ia_act(ui, act, t),
            Action::Account(act) => self.account_act(ui, act, t),
            Action::BluetoothPrompt { addr, passkey } => {
                self.sheet = Some(Sheet::form(ui, self.w, t, super::forms::Prompt::bluetooth_pairing(&addr, passkey)));
                vec![]
            }
            Action::Pick(pick) => {
                self.list_mut().expect("list").set_check(g, i);
                match pick {
                    Pick::ScreenSaverModule(k) => {
                        self.opts.screensaver_module = k.min(super::screensaver::SAVERS.len() - 1);
                        self.opts.save(prefs);
                        // the options on the page are the new saver's; the page is rebuilt to show them
                        self.saver_sheet = super::screensaver::load_sheet(self.opts.screensaver_module, prefs);
                        self.rebuild_visible(ui);
                        vec![]
                    }
                    Pick::ScreenSaverOption(n, c) => {
                        let key = self.saver_sheet.keys().get(n).map(|k| k.to_string());
                        let choice = key.as_deref().and_then(|k| self.saver_sheet.choices(k)).and_then(|list| list.get(c).cloned());
                        if let (Some(key), Some(choice)) = (key, choice) {
                            if self.saver_sheet.set_from_text(&key, &choice) {
                                super::screensaver::save_option(prefs, self.opts.screensaver_module, &key, &choice);
                            }
                        }
                        vec![]
                    }
                    Pick::AlertSound(k) => {
                        self.opts.alert_sound = k.min(super::options::ALERT_SOUNDS.len() - 1);
                        self.opts.save(prefs);
                        // choosing a sound plays it once (the binary skips it while the output is muted)
                        vec![Effect::AlertSound(self.opts.alert_sound)]
                    }
                    Pick::ScreenSaverStart(k) => {
                        self.opts.screensaver_minutes = super::screensaver::START_AFTER[k.min(super::screensaver::START_AFTER.len() - 1)].1;
                        self.opts.save(prefs);
                        vec![]
                    }
                    Pick::ScreenSaverPreview => {
                        self.screensaver_preview = true;
                        vec![]
                    }
                    Pick::AutoLock(k) => {
                        self.opts.auto_lock_ms = AUTO_LOCK[k].1;
                        self.opts.save(prefs);
                        let label = self.opts.auto_lock_label();
                        self.set_parent_value(ui, "Auto-Lock", label);
                        vec![Effect::AutoLock(self.opts.auto_lock_ms)]
                    }
                    Pick::RequirePassword(k) => {
                        self.opts.lock_grace_s = REQUIRE_AFTER[k.min(REQUIRE_AFTER.len() - 1)].1;
                        self.opts.save(prefs);
                        let label = self.opts.lock_grace_label();
                        self.set_parent_value(ui, "Require Password", label);
                        vec![]
                    }
                    Pick::SystemSleep(k) => {
                        self.opts.system_sleep_min = SYSTEM_SLEEP[k].1;
                        self.opts.save(prefs);
                        let label = self.opts.system_sleep_label();
                        self.set_parent_value(ui, "Energy", label);
                        vec![Effect::SystemSleepAfter(self.opts.system_sleep_min)]
                    }
                    Pick::AllowAppsFrom(k) => {
                        let (_, setting) = super::launch_gate::SETTINGS[k.min(super::launch_gate::SETTINGS.len() - 1)];
                        self.allow_from = setting;
                        let _ = prefs.set(super::launch_gate::SETTING_PREF, setting.label());
                        vec![]
                    }
                    Pick::GameMode(k) => {
                        self.opts.game_mode = GAME_MODES[k.min(GAME_MODES.len() - 1)].1;
                        self.opts.save(prefs);
                        vec![]
                    }
                    Pick::PowerMode(k) => {
                        self.opts.power_profile = POWER_MODES[k.min(POWER_MODES.len() - 1)].1;
                        self.opts.save(prefs);
                        vec![]
                    }
                    Pick::DisplayZoom(k) => {
                        self.a11y.zoom = k.min(ZOOMS.len() - 1);
                        self.a11y.save(prefs);
                        self.set_parent_value(ui, "Display Zoom", ZOOMS[self.a11y.zoom].0);
                        vec![]
                    }
                    Pick::Appearance(k) => {
                        use bifrost_render::appearance::{mode_label, publish_mode, Mode};
                        let mode = [Mode::Light, Mode::Dark, Mode::Auto][k.min(2)];
                        if let Err(e) = prefs.set(APPEARANCE_PREF, mode.key()) {
                            eprintln!("springboard: settings.ini: {e}");
                        }
                        publish_mode(mode);
                        self.set_parent_value(ui, "Appearance", mode_label(mode));
                        vec![]
                    }
                    Pick::Accent(k) => {
                        use bifrost_render::appearance::{set_accent, Accent};
                        let accent = Accent::ALL[k.min(Accent::ALL.len() - 1)];
                        if let Err(e) = prefs.set(ACCENT_PREF, accent.key()) {
                            eprintln!("springboard: settings.ini: {e}");
                        }
                        set_accent(accent);
                        // the Tinted icons take the accent as their tint
                        apply_icon_style(bifrost_render::icon_doc::current_style().0);
                        vec![]
                    }
                    Pick::MinimizeEffect(k) => {
                        self.desk.minimize_effect = k.min(super::pages::MINIMIZE_EFFECTS.len() - 1);
                        let _ = prefs.set(super::pages::DOCK_MINIMIZE_PREF, &self.desk.minimize_effect.to_string());
                        vec![]
                    }
                    Pick::HotCorner(c, a) => {
                        let c = c.min(3);
                        self.desk.corners[c] = a.min(super::pages::CORNER_ACTIONS.len() - 1);
                        let _ = prefs.set(super::pages::HOT_CORNER_PREFS[c], &self.desk.corners[c].to_string());
                        vec![]
                    }
                    Pick::Highlight(k) => {
                        self.desk.highlight = k.min(bifrost_render::appearance::Accent::ALL.len() - 1);
                        let _ = prefs.set(super::pages::HIGHLIGHT_PREF, &self.desk.highlight.to_string());
                        self.desk.apply();
                        vec![]
                    }
                    Pick::SidebarIcons(k) => {
                        self.desk.sidebar_icons = k.min(super::pages::SIDEBAR_ICON_SIZES.len() - 1);
                        let _ = prefs.set(super::pages::SIDEBAR_ICON_PREF, &self.desk.sidebar_icons.to_string());
                        self.desk.apply();
                        // the sidebar's row pitch is laid out when it is built: build it again at the new size now
                        self.rebuild_sidebar(ui);
                        vec![]
                    }
                    Pick::Remap(slot, target) => {
                        let target = target.min(super::kb_input::REMAP_TARGETS.len() - 1);
                        match slot {
                            0 => self.opts.remap_caps_lock = target,
                            1 => self.opts.remap_control = target,
                            2 => self.opts.remap_option = target,
                            _ => self.opts.remap_command = target,
                        }
                        self.opts.save(prefs);
                        self.apply_keys();
                        vec![]
                    }
                    Pick::TrackpadSecondary(k) => {
                        self.opts.trackpad_secondary = k.min(super::options::TRACKPAD_SECONDARY.len() - 1);
                        self.opts.save(prefs);
                        self.apply_pointer();
                        vec![]
                    }
                    Pick::BackupDestination(k) => {
                        if let Some((_, path)) = super::backup_page::destination_choices().get(k) {
                            self.opts.backup_destination = path.clone();
                            self.opts.save(prefs);
                            // the newest backup is read from the new place
                            let root = backup::destination_root(&self.data_dir, &self.opts.backup_destination);
                            self.loads.latest_backup.start("backup-read", move || backup::latest(&root).map(|(_, t)| t));
                        }
                        vec![]
                    }
                    Pick::SpotlightExclude(k) => {
                        if self.opts.spotlight.remove_excluded(k) {
                            self.opts.save(prefs);
                            super::spotlight_scope::install(self.opts.spotlight.clone());
                        }
                        vec![]
                    }
                    Pick::MouseSecondary(k) => {
                        self.opts.swap_buttons = super::mouse_page::swaps_buttons(k);
                        self.opts.save(prefs);
                        self.apply_pointer();
                        vec![]
                    }
                    Pick::ScrollBars(k) => {
                        self.desk.scroll_bars = k.min(super::pages::SCROLL_BAR_MODES.len() - 1);
                        let _ = prefs.set(super::pages::SCROLL_BARS_PREF, &self.desk.scroll_bars.to_string());
                        self.desk.apply();
                        vec![]
                    }
                    Pick::GlassStyle(k) => {
                        let v = if k == 0 { 1.0 } else { 0.0 };
                        bifrost_render::glass::set_clarity(v);
                        let _ = prefs.set(LIQUID_GLASS_PREF, &format!("{v:.3}"));
                        vec![]
                    }
                    Pick::IconStyle(k) => {
                        use bifrost_render::icon_doc::Variant;
                        let variant = [Variant::Default, Variant::Dark, Variant::Tinted][k.min(2)];
                        if let Err(e) = prefs.set(ICON_STYLE_PREF, variant.key()) {
                            eprintln!("springboard: settings.ini: {e}");
                        }
                        apply_icon_style(variant);
                        vec![]
                    }
                    Pick::RefreshRate(k) => {
                        use super::services::display_services::{choice_at, request_refresh};
                        let choice = choice_at(k, &self.info.display_rates);
                        self.refresh = choice;
                        if let Err(e) = choice.save(&self.data_dir) {
                            eprintln!("springboard: display-refresh: {e}");
                        }
                        if let Some((w, h, _)) = self.info.display_mode {
                            request_refresh(choice, w, h);
                        }
                        self.set_parent_value(ui, "Refresh Rate", &choice.label());
                        vec![]
                    }
                    Pick::Resolution(k) => {
                        use super::services::display_services::{request_refresh, resolution_at, resolution_label, save_resolution, RefreshChoice};
                        let choice = resolution_at(k, &self.info.display_sizes);
                        self.resolution = choice;
                        if let Err(e) = save_resolution(&self.data_dir, choice) {
                            eprintln!("springboard: display-resolution: {e}");
                        }
                        // Native is the largest size the panel offers; the window server picks the fastest refresh there
                        if let Some((w, h)) = choice.or(self.info.display_sizes.first().copied()) {
                            request_refresh(RefreshChoice::Max, w, h);
                        }
                        self.set_parent_value(ui, "Resolution", &resolution_label(choice));
                        vec![]
                    }
                    Pick::InterfaceScale(k) => {
                        let choice = k.checked_sub(1).and_then(|i| crate::layout::SCALE_CHOICES.get(i).copied());
                        crate::layout::set_user_scale(choice);
                        if let Err(e) = prefs.set(DISPLAY_SCALE_PREF, &choice.map_or_else(|| "auto".to_string(), |s| s.to_string())) {
                            eprintln!("springboard: settings.ini: {e}");
                        }
                        self.set_parent_value(ui, "Interface Scale", &super::pages::scale_label(choice));
                        vec![]
                    }
                    Pick::TouchHold(k) => {
                        self.a11y.hold = k.min(HOLD_MS.len() - 1);
                        self.a11y.save(prefs);
                        self.set_parent_value(ui, "Touch Accommodations", HOLD_MS[self.a11y.hold].0);
                        vec![]
                    }
                    Pick::Dwell(k) => {
                        self.a11y.dwell = k.min(super::accessibility::dwell::DELAYS_MS.len() - 1);
                        self.a11y.save(prefs);
                        self.set_parent_value(ui, "Dwell Control", super::accessibility::dwell::DELAYS_MS[self.a11y.dwell].0);
                        vec![]
                    }
                    Pick::Slow(k) => {
                        self.a11y.slow = k.min(super::slow_keys::DELAYS_MS.len() - 1);
                        self.a11y.save(prefs);
                        self.set_parent_value(ui, "Slow Keys", super::slow_keys::DELAYS_MS[self.a11y.slow].0);
                        vec![]
                    }
                    Pick::SpeechRate(k) => {
                        self.a11y.speech_rate = super::accessibility::SPEECH_RATES[k.min(super::accessibility::SPEECH_RATES.len() - 1)].1;
                        self.a11y.save(prefs);
                        vec![]
                    }
                    Pick::SpeechPitch(k) => {
                        self.a11y.speech_pitch = super::accessibility::SPEECH_PITCHES[k.min(super::accessibility::SPEECH_PITCHES.len() - 1)].1;
                        self.a11y.save(prefs);
                        vec![]
                    }
                    Pick::SpeechPunctuation(k) => {
                        self.a11y.speech_punctuation = k.min(super::accessibility::SPEECH_PUNCTUATION.len() - 1);
                        self.a11y.save(prefs);
                        vec![]
                    }
                    Pick::SpeechVoice(k) => {
                        self.a11y.speech_voice = super::accessibility::SPEECH_VOICES[k.min(super::accessibility::SPEECH_VOICES.len() - 1)].1.to_string();
                        self.a11y.save(prefs);
                        self.set_parent_value(ui, "Speech", super::accessibility::SPEECH_VOICES[super::pages::speech_voice_index(&self.a11y.speech_voice)].0);
                        vec![]
                    }
                    Pick::AccessibilityShortcut(k) => {
                        // checks the feature in panel order: a tapped feature joins or leaves the choice
                        if let Some(&f) = Feature::ALL.get(k) {
                            let on = self.a11y.shortcut.contains(&f);
                            self.a11y.shortcut = Feature::ALL.into_iter().filter(|x| if *x == f { !on } else { self.a11y.shortcut.contains(x) }).collect();
                            self.a11y.save(prefs);
                            self.set_parent_value(ui, "Accessibility Shortcut", &super::pages::shortcut_label(&self.a11y.shortcut));
                        }
                        vec![]
                    }
                    Pick::Nearby(k) => {
                        self.opts.nearby = k.min(2);
                        self.opts.save(prefs);
                        vec![]
                    }
                    Pick::Language(k) => {
                        let code = bifrost_ui::region::LANGUAGES[k.min(bifrost_ui::region::LANGUAGES.len() - 1)].0;
                        self.change_region(prefs, |r| {
                            r.languages.retain(|l| l.as_str() != code);
                            r.languages.insert(0, code.to_string());
                        });
                        self.rebuild_visible(ui);
                        vec![]
                    }
                    Pick::Region(k) => {
                        let code = bifrost_ui::region::REGIONS[k.min(bifrost_ui::region::REGIONS.len() - 1)].0;
                        self.change_region(prefs, |r| r.region = code.to_string());
                        let name = bifrost_ui::region::current().region_name();
                        self.set_parent_value(ui, "Region", &name);
                        self.rebuild_visible(ui);
                        vec![]
                    }
                    Pick::FirstWeekday(k) => {
                        let day = match k {
                            1 => Some(0),
                            2 => Some(1),
                            _ => None,
                        };
                        self.change_region(prefs, |r| r.first_weekday = day);
                        self.rebuild_visible(ui);
                        vec![]
                    }
                    Pick::DockPosition(k) => {
                        let position = [super::desktop_dock::Position::Bottom, super::desktop_dock::Position::Left, super::desktop_dock::Position::Right][k.min(2)];
                        self.dock.position = position;
                        self.dock_out.push(super::desktop_dock::Change::Position(position));
                        vec![]
                    }
                    Pick::SoundOutput(k) | Pick::SoundInput(k) => {
                        let input = matches!(pick, Pick::SoundInput(_));
                        if let Some((key, uid)) = super::sound_page::pick_pref(&self.sound, input, k) {
                            if let Err(e) = prefs.set(key, &uid) {
                                eprintln!("settings: {key}: {e}");
                            }
                            if input {
                                self.sound.input = Some(k);
                                self.sound_saved.1 = uid;
                            } else {
                                self.sound.output = Some(k);
                                self.sound_saved.0 = uid;
                            }
                            // the check moves to the chosen device
                            self.rebuild_visible(ui);
                        }
                        vec![]
                    }
                    Pick::EffectsOutput(k) => {
                        let uid = super::sound_page::effects_pref(&self.sound, k);
                        if let Err(e) = prefs.set(super::sound_page::PREF_EFFECTS, &uid) {
                            eprintln!("settings: {}: {e}", super::sound_page::PREF_EFFECTS);
                        }
                        self.sound.effects = k.checked_sub(1).filter(|&i| i < self.sound.outputs.len());
                        self.sound_saved.2 = uid;
                        vec![]
                    }
                    Pick::IconSize(k) => {
                        self.opts.icon_size = k.min(ICON_SIZES.len() - 1);
                        self.opts.save(prefs);
                        vec![]
                    }
                }
            }
            Action::Location(edit) => self.location_act(ui, edit, t),
            Action::Sharing(edit) => {
                // the view the page last showed; the services are read again on a worker below
                let view = self.loads.sharing.get().cloned().unwrap_or_else(super::sharing_page::View::load);
                let r = super::sharing_page::apply(&edit, &view);
                // a service that just started or stopped shows its new state at once; the read below confirms it
                if let (super::sharing_page::Edit::Service { id, on }, Ok(None)) = (&edit, &r) {
                    if let Some(v) = self.loads.sharing.get_mut() {
                        v.set_service(*id, *on);
                    }
                }
                // the page keeps the last answer until the new read lands
                self.loads.sharing.start("sharing-read", super::sharing_page::View::load);
                self.edit_result(ui, t, r);
                vec![]
            }
            Action::StartupDisk(edit) => self.startup_disk_act(ui, edit),
            Action::PrintScan(edit) => {
                match edit {
                    // opening the address prompt touches no service
                    super::print_scan_page::Edit::Add => {
                        let r = super::print_scan_page::apply(&edit);
                        self.edit_result(ui, t, r);
                    }
                    super::print_scan_page::Edit::SetDefault(_) => self.start_print_edit(ui, move || super::print_scan_page::apply(&edit)),
                }
                vec![]
            }
            Action::Notify(edit) => {
                self.notify_edit(ui, edit, g, i, t);
                vec![]
            }
            Action::Wake(edit) => self.wake_edit(ui, prefs, edit),
            Action::OpenAnyway => {
                // the blocked program is remembered for this code; the page drops the block and says to open it again
                super::launch_gate::open_anyway(&self.data_dir, prefs);
                self.rebuild(ui);
                vec![]
            }
            Action::NetDiagnose => {
                if !self.loads.net_diag.busy() {
                    let (host, port) = super::net_diagnose_page::target_of(self.updater.url());
                    let probe = self.net_probe.clone();
                    // the resolve and connect have time limits, but they still run off the UI thread
                    self.loads.net_diag.start("net-diagnose", move || Some(bifrost_core::net_probe::run(&*probe, &bifrost_core::net_probe::Target::new(&host, port))));
                    self.rebuild_visible(ui);
                }
                vec![]
            }
            Action::Night(edit) => {
                match edit {
                    NightEdit::Scheduled | NightEdit::Manual => {
                        let on = if edit == NightEdit::Scheduled { &mut self.night.scheduled } else { &mut self.night.manual };
                        *on = !*on;
                        let on = *on;
                        self.night.publish();
                        self.night.save(prefs);
                        if let Some(p) = self.list_mut() {
                            p.set_switch(g, i, on, t);
                        }
                        self.set_parent_value(ui, "Night Shift", self.night.label());
                    }
                    NightEdit::From | NightEdit::To => {
                        let from = edit == NightEdit::From;
                        self.night_edge = Some(from);
                        self.time_picker = Some(TimePicker::of_day(ui, self.w, self.h, if from { self.night.start } else { self.night.end }, if from { "From" } else { "To" }));
                    }
                }
                vec![]
            }
            Action::ScreenTime(edit) => {
                self.st_edit(ui, edit);
                vec![]
            }
            Action::SetTime => {
                let (unix, off) = crate::bare::sys::wall_time();
                let (_, mo, d) = crate::bare::timezone::civil_from_days((unix + off as i64).div_euclid(86_400));
                let y = crate::bare::timezone::civil_from_days((unix + off as i64).div_euclid(86_400)).0;
                self.time_picker = Some(TimePicker::new(ui, self.w, self.h, (y, mo, d, self.info.hour as u32, self.info.min as u32)));
                vec![]
            }
            Action::TimeZone => {
                self.zone_picker = Some(ZonePicker::new(ui, self.w, self.h, &self.info.timezone, t));
                vec![]
            }
            Action::Run(cmd) => {
                self.alert = Some(Alert::new(ui, cmd).with_reopen(ui, prefs.get_bool(super::session::REOPEN_PREF, true)));
                vec![]
            }
            Action::JoinOther => {
                self.sheet = Some(Sheet::other(ui, None, self.w, t));
                vec![]
            }
            Action::Bluetooth(c) => vec![Effect::Bluetooth(c)],
            // the network service runs nmcli on its own thread; the page shows the new state on its next read
            Action::Vpn { name, up } => {
                use super::desktop_network::{Request, VpnAction};
                super::desktop_network::send(Request::Vpn(if up { VpnAction::Up(name) } else { VpnAction::Down(Some(name)) }));
                vec![]
            }
            Action::AudioOutput => vec![Effect::BluetoothOutput],
            Action::OpenFile(path) => {
                self.view_request = Some(path);
                vec![]
            }
            Action::Wifi(a) => {
                if let WifiAction::SetAutoJoin { on, .. } = &a {
                    self.list_mut().expect("list").set_switch(g, i, *on, t);
                }
                vec![Effect::Wifi(a)]
            }
            Action::Network(ssid) => match self.info.networks.iter().find(|n| n.ssid == ssid).map(choose) {
                // a network that signs in with a username needs the fuller sheet
                Some(Choice::Password(ssid)) if self.info.enterprise.contains(&ssid) => {
                    self.sheet = Some(Sheet::other(ui, Some(ssid), self.w, t));
                    vec![]
                }
                Some(Choice::Act(a)) => vec![Effect::Wifi(a)],
                Some(Choice::Password(ssid)) => {
                    self.sheet = Some(Sheet::new(ui, ssid, self.w, t));
                    vec![]
                }
                None => vec![],
            },
        }
    }

    fn toggle(&mut self, ui: &Ui, prefs: &mut Prefs, which: Toggle, g: usize, i: usize, t: f64) -> Vec<Effect> {
        let flip = |b: &mut bool| {
            *b = !*b;
            *b
        };
        let mut effects = vec![];
        let on = match which {
            Toggle::Airplane => {
                let on = flip(&mut self.airplane);
                self.wifi_on = !on;
                effects.push(Effect::Wifi(WifiAction::Enable(!on)));
                let value = if on { "Off".to_string() } else { self.info.wifi.clone() };
                self.set_root_value(ui, "Wi-Fi", &value);
                on
            }
            Toggle::Wifi => {
                let on = flip(&mut self.wifi_on);
                effects.push(Effect::Wifi(WifiAction::Enable(on)));
                if on {
                    effects.push(Effect::Wifi(WifiAction::Scan));
                }
                let value = if on { self.info.wifi.clone() } else { "Off".to_string() };
                self.set_root_value(ui, "Wi-Fi", &value);
                // the network groups appear and disappear with the switch
                let back = self.stack.get(self.stack.len().wrapping_sub(2)).map(|e| e.title.clone());
                let entry = self.build(ui, PageId::Wifi, back.as_deref());
                *self.stack.last_mut().expect("top") = entry;
                on
            }
            Toggle::Bluetooth => {
                // with no controller the switch stays off; the wish is remembered by the radio thread
                let on = !self.info.bt.on() && self.info.bt.state != BtState::Unavailable;
                effects.push(Effect::Bluetooth(BtCmd::Enable(!self.info.bt.on())));
                if on {
                    effects.push(Effect::Bluetooth(BtCmd::Scan));
                }
                if self.info.bt.state != BtState::Unavailable {
                    self.info.bt.state = if on { BtState::On } else { BtState::Off };
                }
                self.set_root_value(ui, "Bluetooth", self.info.bt.summary());
                // the device groups appear and disappear with the switch
                let back = self.stack.get(self.stack.len().wrapping_sub(2)).map(|e| e.title.clone());
                let entry = self.build(ui, PageId::Bluetooth, back.as_deref());
                *self.stack.last_mut().expect("top") = entry;
                on
            }
            Toggle::KeyboardClicks => flip(&mut self.opts.keyboard_clicks),
            Toggle::KeyboardHaptics => flip(&mut self.opts.keyboard_haptics),
            Toggle::LockSound => flip(&mut self.opts.lock_sound),
            Toggle::Clock24 => flip(&mut self.opts.clock_24h),
            Toggle::ClockDate => flip(&mut self.opts.clock_date),
            Toggle::ClockDay => flip(&mut self.opts.clock_day),
            Toggle::Layout(i) => super::kb_layouts::toggle(super::kb_layouts::Layout::ALL[i.min(super::kb_layouts::Layout::ALL.len() - 1)]),
            Toggle::RememberInputSource => super::kb_layouts::set_remember(!super::kb_layouts::remembers()),
            Toggle::AutoCaps => flip(&mut self.opts.auto_caps),
            Toggle::EnableCapsLock => flip(&mut self.opts.enable_caps_lock),
            Toggle::PeriodShortcut => flip(&mut self.opts.period_shortcut),
            Toggle::BoldText => flip(&mut self.opts.bold_text),
            Toggle::AutoJoin => flip(&mut self.opts.auto_join),
            Toggle::Dnd => flip(&mut self.opts.dnd),
            Toggle::NaturalScroll => {
                let on = flip(&mut self.opts.natural_scroll);
                self.apply_pointer();
                on
            }
            Toggle::TapToClick => {
                let on = flip(&mut self.opts.tap_to_click);
                self.apply_pointer();
                on
            }
            Toggle::MouseAcceleration => {
                let on = flip(&mut self.opts.mouse_accel);
                self.apply_pointer();
                on
            }
            Toggle::LoginNamePassword => flip(&mut self.opts.login_name_password),
            Toggle::LoginHints => flip(&mut self.opts.login_hints),
            Toggle::FunctionStandard => {
                let on = flip(&mut self.opts.function_standard);
                self.apply_keys();
                on
            }
            Toggle::TabNavigation => {
                let on = flip(&mut self.opts.tab_navigation);
                self.apply_keys();
                on
            }
            Toggle::Shortcut(i) => {
                let bit = 1u32 << i.min(super::kb_input::BINDINGS.len() - 1);
                self.opts.shortcuts_off ^= bit;
                self.apply_keys();
                self.opts.shortcuts_off & bit == 0
            }
            Toggle::AssistiveTouch => flip(&mut self.opts.assistive_touch),
            Toggle::ReduceMotion => flip(&mut self.a11y.reduce_motion),
            Toggle::IncreaseContrast => flip(&mut self.a11y.increase_contrast),
            Toggle::ReduceTransparency => flip(&mut self.a11y.reduce_transparency),
            Toggle::ButtonShapes => flip(&mut self.a11y.button_shapes),
            Toggle::DimFlashingLights => flip(&mut self.a11y.dim_flashing_lights),
            Toggle::GuidedAccess => flip(&mut self.a11y.guided_access),
            Toggle::ScreenReader => flip(&mut self.a11y.screen_reader),
            Toggle::SwitchScan => flip(&mut self.a11y.switch_scan),
            Toggle::StickyKeys => flip(&mut self.a11y.sticky_keys),
            Toggle::MouseKeys => flip(&mut self.a11y.mouse_keys),
            Toggle::AccessibilityKeyboard => flip(&mut self.a11y.accessibility_keyboard),
            Toggle::InputMenu => flip(&mut self.opts.input_menu),
            Toggle::MotionCues => flip(&mut self.a11y.motion_cues),
            Toggle::SwitchControl => flip(&mut self.a11y.switch_control),
            Toggle::VoiceControl => flip(&mut self.a11y.voice_control),
            Toggle::LiveCaptions => flip(&mut self.a11y.live_captions),
            Toggle::AutoTime => {
                let on = flip(&mut self.info.time_manual);
                // the switch shows "automatic", the flag stores "manual"
                let auto = !on;
                crate::net::ntp::set_auto(auto);
                if let Err(e) = prefs.set("time_auto", if auto { "1" } else { "0" }) {
                    eprintln!("springboard: settings.ini: {e}");
                }
                auto
            }
            Toggle::LowPower => flip(&mut self.opts.low_power),
            Toggle::MenuExtra(e) => {
                self.opts.menu_extras ^= e.bit();
                self.opts.menu_extras & e.bit() != 0
            }
            Toggle::Handoff => flip(&mut self.opts.handoff),
            Toggle::UniversalClipboard => flip(&mut self.opts.universal_clipboard),
            Toggle::PipAuto => flip(&mut self.opts.pip_auto),
            Toggle::AutoBrightness => flip(&mut self.opts.auto_brightness),
            Toggle::BgRefresh => flip(&mut self.opts.bg_refresh),
            Toggle::SuspendBackground => flip(&mut self.opts.suspend_background),
            Toggle::FourFinger => flip(&mut self.opts.four_finger),
            Toggle::CornerSwipe => flip(&mut self.opts.corner_swipe),
            Toggle::Permission(media, index) => match super::apphost::installed().get(index) {
                Some(app) => {
                    let id = app.manifest.id.clone();
                    let allowed = !super::services::privacy_permissions::access(&id).can_capture(media);
                    // a restricted app refuses the change; the switch then shows the answer that stands
                    let _ = super::services::privacy_permissions::set_by_user(&id, media, allowed);
                    super::services::privacy_permissions::access(&id).can_capture(media)
                }
                None => false,
            },
            Toggle::Contacts(index) => match super::apphost::installed().get(index) {
                Some(app) => {
                    let id = app.manifest.id.clone();
                    let allowed = !super::services::privacy_permissions::contacts_allowed(&id);
                    super::services::privacy_permissions::set_contacts_by_user(&id, allowed)
                }
                None => false,
            },
            Toggle::Calendars(index) => match super::apphost::installed().get(index) {
                Some(app) => {
                    let id = app.manifest.id.clone();
                    let allowed = !super::services::calendar::allowed(&id);
                    super::services::privacy_permissions::set_calendars_by_user(&id, allowed)
                }
                None => false,
            },
            Toggle::Accessibility(index) => match super::apphost::installed().get(index) {
                Some(app) => {
                    let id = app.manifest.id.clone();
                    let allowed = !super::services::privacy_permissions::accessibility_allowed(&id);
                    super::services::privacy_permissions::set_accessibility_by_user(&id, allowed)
                }
                None => false,
            },
            Toggle::Usb(index) => match super::apphost::installed().get(index) {
                Some(app) => {
                    super::services::usb_access::revoke_app(&app.manifest.id);
                    false
                }
                None => false,
            },
            Toggle::Location(index) => match super::apphost::installed().get(index) {
                Some(app) => {
                    let id = app.manifest.id.clone();
                    let allowed = !super::services::privacy_permissions::location_status(&id).is_authorized();
                    // a restricted app refuses the change; the switch then shows the status that stands
                    super::services::privacy_permissions::set_location_by_user(&id, allowed).is_authorized()
                }
                None => false,
            },
            Toggle::SpotlightCategory(c) => {
                let on = !self.opts.spotlight.allows(c);
                self.opts.spotlight.set_allowed(c, on);
                self.opts.save(prefs);
                super::spotlight_scope::install(self.opts.spotlight.clone());
                on
            }
            Toggle::BatteryPercent => flip(&mut self.opts.battery_percent),
            Toggle::DockRecents => flip(&mut self.opts.dock_recents),
            Toggle::DockAutohide => {
                self.dock.autohide = !self.dock.autohide;
                self.dock_out.push(super::desktop_dock::Change::Autohide);
                self.dock.autohide
            }
            Toggle::DockMagnify => {
                let on = flip(&mut self.desk.magnify);
                let _ = prefs.set(super::pages::DOCK_MAGNIFY_PREF, if on { "1" } else { "0" });
                // the dock owns magnification (desktop-dock.ini): this pref only mirrors it for the page
                self.dock_out.push(super::desktop_dock::Change::Magnification(on));
                on
            }
            Toggle::OverviewGroup => {
                let on = flip(&mut self.desk.overview_group);
                let _ = prefs.set(super::pages::OVERVIEW_GROUP_PREF, if on { "1" } else { "0" });
                on
            }
            Toggle::OverviewSwitch => {
                let on = flip(&mut self.desk.overview_switch);
                let _ = prefs.set(super::pages::OVERVIEW_SWITCH_PREF, if on { "1" } else { "0" });
                on
            }
            Toggle::SaverOption(n) => {
                let key = self.saver_sheet.keys().get(n).map(|k| k.to_string());
                match key {
                    Some(key) if self.saver_sheet.is_bool(key.as_str()) => {
                        let on = !self.saver_sheet.bool_or(&key, false);
                        let text = if on { "true" } else { "false" };
                        if self.saver_sheet.set_from_text(&key, text) {
                            super::screensaver::save_option(prefs, self.opts.screensaver_module, &key, text);
                        }
                        on
                    }
                    _ => false,
                }
            }
            Toggle::CloseOnQuit => {
                let on = flip(&mut self.desk.close_on_quit);
                let _ = prefs.set(super::session::CLOSE_ON_QUIT_PREF, if on { "1" } else { "0" });
                on
            }
            Toggle::AutoUpdate => flip(&mut self.opts.auto_update),
            Toggle::BackupBeforeUpdate => flip(&mut self.opts.backup_before_update),
            Toggle::RootedDebug => {
                // turning it off asks first; the switch moves only once confirmed
                if self.dev.rooted {
                    self.alert = Some(Alert::new(ui, Command::RootedOff));
                    return effects;
                }
                self.dev.set_rooted(true);
                true
            }
            Toggle::PerfHud => {
                self.dev.perf_hud = !self.dev.perf_hud;
                if let Err(e) = prefs.set(PERF_HUD_PREF, if self.dev.perf_hud { "1" } else { "0" }) {
                    eprintln!("springboard: settings.ini: {e}");
                }
                self.dev.perf_hud
            }
            Toggle::GpuScanout => {
                self.dev.gpu_scanout = !self.dev.gpu_scanout;
                if let Err(e) = prefs.set(GPU_SCANOUT_PREF, if self.dev.gpu_scanout { "1" } else { "0" }) {
                    eprintln!("springboard: settings.ini: {e}");
                }
                self.dev.gpu_scanout
            }
            Toggle::WifiAdb => {
                let on = !self.dev.wifi_adb;
                self.dev.set_wifi_adb(on);
                // shows the restart row and the 'takes effect after restart' footer
                self.rebuild(ui);
                on
            }
        };
        if !matches!(which, Toggle::Airplane | Toggle::Wifi | Toggle::Bluetooth) {
            self.opts.save(prefs);
            self.a11y.save(prefs);
        }
        if which == Toggle::Dnd {
            self.set_root_value(ui, "Focus", if on { "On" } else { "Off" });
        }
        let target = if self.side_touch { self.sidebar.as_mut() } else { self.list_mut() };
        if let Some(p) = target {
            p.set_switch(g, i, on, t);
            if which == Toggle::Wifi {
                p.flipped_at(g, i, t);
            }
        }
        effects
    }

    /// Carries out a confirmed command.
    fn run(&mut self, ui: &Ui, prefs: &mut Prefs, cmd: Command, t: f64) -> Vec<Effect> {
        match cmd {
            Command::ResetSettings => {
                if let Err(e) = prefs.clear() {
                    eprintln!("springboard: settings.ini: {e}");
                }
                self.opts = Options::default();
                self.a11y = AccessibilitySettings::default();
                self.dev.perf_hud = false;
                self.dev.gpu_scanout = false;
                self.info.wallpaper.clear();
                self.wallpaper_dim = true;
                self.wallpaper_dynamic = false;
                self.wallpaper_look = Look::Still;
                self.wallpaper_show_on = ShowOn::All;
                self.rebuild(ui);
                vec![Effect::AutoLock(self.opts.auto_lock_ms), Effect::Volume(self.opts.volume), Effect::Wallpaper(String::new()), Effect::LockWallpaper(String::new()), Effect::WallpaperDim(true), Effect::LiveWallpaper(Look::Still.pref().to_string())]
            }
            Command::EraseContent => match bifrost_core::erase_content::request(&self.data_dir) {
                Ok(()) => vec![Effect::Restart],
                Err(e) => {
                    eprintln!("springboard: erase not started: {e}");
                    vec![]
                }
            },
            Command::Notice(_) => vec![],
            Command::RemovePackage(name) => {
                let name = name.clone();
                self.package_rx = Some(read_on_worker("package-remove", move || Removal::ask(&name).and_then(|r| r.run())));
                vec![]
            }
            Command::Restart => vec![Effect::Restart],
            Command::ShutDown => vec![Effect::Power(PowerAction::Off)],
            Command::Reboot => vec![Effect::Power(PowerAction::Restart)],
            Command::ClearCache(what) => {
                self.scan.start_with(self.data_dir.clone(), Some(what));
                vec![]
            }
            Command::BackUpNow => {
                // the copy of every data file runs on a worker; the note reports the result when it lands
                let dir = self.data_dir.clone();
                let dest = backup::destination_root(&dir, &self.opts.backup_destination);
                self.backup_rx = Some(read_on_worker("backup-now", move || backup::back_up_to(&dir, &dest).map(|(_, n)| n).map_err(|e| e.to_string())));
                self.backup_note = "Backing up…".into();
                self.rebuild(ui);
                vec![]
            }
            Command::RestoreBackup => match backup::latest(&backup::destination_root(&self.data_dir, &self.opts.backup_destination)).map(|(p, _)| backup::stage(&self.data_dir, &p)) {
                Some(Ok(_)) => vec![Effect::Restart],
                other => {
                    self.backup_note = format!("Restore failed: {}", other.map_or("no backup".to_string(), |r| r.map_or_else(|e| e.to_string(), |_| String::new())));
                    self.rebuild(ui);
                    vec![]
                }
            },
            Command::DeleteBackups => {
                let dir = backup::destination_root(&self.data_dir, &self.opts.backup_destination);
                self.backup_note = match backup::delete_backups(&dir) {
                    Ok(n) => format!("Deleted {n} backups."),
                    Err(e) => format!("Delete failed: {e}"),
                };
                self.loads.latest_backup.start("backup-read", move || backup::latest(&dir).map(|(_, t)| t));
                self.rebuild(ui);
                vec![]
            }
            Command::CheckUpdate => {
                self.updater.check();
                self.rebuild(ui);
                vec![]
            }
            Command::DownloadUpdate => {
                self.updater.start_download();
                self.update_pct = 0;
                self.rebuild(ui);
                vec![]
            }
            Command::InstallUpdate => {
                if self.install_rx.is_some() {
                    return vec![];
                }
                if self.opts.backup_before_update {
                    // the copy runs on a worker; `poll_install` restarts once it is in, and a failed backup stops the install
                    // so the data is never replaced unsaved
                    let dir = self.data_dir.clone();
                    let dest = backup::destination_root(&dir, &self.opts.backup_destination);
                    self.install_rx = Some(read_on_worker("update-backup", move || backup::back_up_to(&dir, &dest).map(|(_, n)| n).map_err(|e| e.to_string())));
                    self.backup_note = "Backing up before the update…".into();
                    self.rebuild(ui);
                    return vec![];
                }
                self.update_badge = false;
                vec![Effect::Restart]
            }
            Command::RollbackSystem => {
                match super::pacman_update::rollback() {
                    // the guard stops this shell while it rolls back; nothing to show until then
                    Ok(()) => self.backup_note = "Rolling back to the previous system…".into(),
                    Err(e) => self.backup_note = format!("Rollback not started: {e}"),
                }
                self.rebuild(ui);
                vec![]
            }
            Command::ShareDiagnostics => {
                let (unix, _) = crate::bare::sys::wall_time();
                let name = format!("diag-{}-{unix}.txt", self.info.serial.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>());
                let url = diag::upload_url(self.updater.url(), &name);
                let (dir, build) = (self.data_dir.clone(), format!("{} {}", super::pages::BUILD.version, super::pages::BUILD.git));
                let (tx, rx) = std::sync::mpsc::channel();
                // the bundle reads the logs and heartbeat, so it is built on the worker with the upload
                std::thread::spawn(move || {
                    let body = diag::bundle(&dir, &build, unix);
                    let _ = tx.send(diag::post(&url, body.as_bytes()).map_err(|e| e.to_string()));
                });
                self.diag_rx = Some(rx);
                self.diag_note.clear();
                self.rebuild(ui);
                vec![]
            }
            Command::DownloadSpeechModel => {
                // the consent is recorded and the download starts in a thread; a failed download is shown on the page
                let _ = super::services::recognizer::grant_and_download(&super::services::recognizer::user_data_dir());
                self.rebuild(ui);
                vec![]
            }
            Command::RemoveSpeechModel => {
                let _ = super::services::recognizer::remove_model(&super::services::recognizer::user_data_dir());
                self.rebuild(ui);
                vec![]
            }
            Command::DeleteCrashLogs => {
                let n = diag::delete_all(&self.data_dir);
                let dir = self.data_dir.clone();
                self.loads.crashes.start("diag-read", move || diag::load(&dir));
                self.diag_note = format!("Deleted {n} crash log file{}.", if n == 1 { "" } else { "s" });
                self.rebuild(ui);
                vec![]
            }
            Command::RootedOff => {
                self.dev.set_rooted(false);
                self.rebuild(ui);
                vec![]
            }
            Command::Forget(ssid) => {
                self.pop(t);
                vec![Effect::Wifi(WifiAction::Forget(ssid))]
            }
            Command::RemoveTextReplacement(shortcut) => {
                super::services::text_services::remove_replacement(prefs, &shortcut);
                self.rebuild(ui);
                Vec::new()
            }
            Command::ForgetBluetooth(addr) => {
                self.pop(t);
                vec![Effect::Bluetooth(BtCmd::Forget(addr))]
            }
            Command::Account(danger) => {
                match super::users_page::run(&danger) {
                    Ok(()) => {
                        self.users_note.clear();
                        self.loads.users.start("users-read", super::users_page::View::load);
                        // the deleted account's or group's page has nothing left to show
                        if matches!(self.page(), PageId::UserDetail | PageId::GroupDetail) {
                            self.pop(t);
                        }
                        self.rebuild(ui);
                    }
                    Err(e) => {
                        self.users_note = e;
                        self.rebuild_visible(ui);
                    }
                }
                vec![]
            }
        }
    }

    /// Renders an entry to an offscreen frame for the slide.
    fn render(&self, entry: &Entry, ui: &Ui, t: f64) -> Image {
        let mut img = Image::new(self.w as usize, self.h as usize);
        let mut s = Surface { w: img.w, h: img.h, stride: img.w, px: &mut img.px, y0: 0 };
        self.draw_entry(&mut s, entry, ui, None, t);
        img
    }

    fn draw_entry(&self, s: &mut Surface, entry: &Entry, ui: &Ui, pressed: Option<Hit>, t: f64) {
        s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, CONTENT_BG.now());
        match &entry.view {
            View::List(p) => p.draw(s, ui, pressed, t),
            View::Wallpaper(p) => {
                p.draw(s, ui, self.w, self.h, self.pressed_thumb);
                entry.nav.draw(s, ui, self.w, 0.0, 1.0);
            }
        }
    }

    /// The plain list page as a `Sync` view that worker threads can draw in bands, or `None`
    /// while a slide is running or the page is not a list (those draw serially in `draw`).
    pub fn page_view(&self) -> Option<PageView<'_>> {
        if self.transition.is_some() || self.side_w > 0 {
            return None;
        }
        match &self.stack.last()?.view {
            View::List(page) => Some(PageView { page, w: self.w, h: self.h, pressed: self.pressed }),
            View::Wallpaper(_) => None,
        }
    }

    /// When the next frame is the previous one with the list scrolled and nothing else changed:
    /// a hash of everything but the scroll offset, and the offset in px. None while a slide,
    /// overlay, keyboard or switch animation runs, or the page is not a plain list.
    pub fn scroll_key(&self, ui: &Ui, t: f64) -> Option<(u64, i32)> {
        use std::hash::{Hash, Hasher};
        if self.side_w > 0 || self.transition.is_some() || self.sheet.is_some() || self.zone_picker.is_some() || self.time_picker.is_some() || self.passcode.is_some() || self.st_gate.is_some() || self.alert.is_some() || !self.search.idle() {
            return None;
        }
        let p = self.list()?;
        let off = p.draw_offset() as i32;
        if !p.scroll_only(ui, t, off) {
            return None;
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (p.gen(), self.stack.len(), self.w, self.h, self.pressed, self.search.query()).hash(&mut h);
        Some((h.finish(), off))
    }

    /// Scrolls the top list page straight to `offset` px.
    pub fn scroll_to(&mut self, offset: f32) {
        if let Some(p) = self.list_mut() {
            p.jump_to(offset);
        }
    }

    /// Everything `page_view` leaves out, drawn over the banded page: the search field and
    /// keyboard, then the alert, sheets and pickers.
    pub fn draw_top(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        self.draw_chrome(s, ui, t);
        self.draw_overlays(s, ui, t);
    }

    /// Width of the sidebar column in px (0 without one): the desk declares it as a `.sidebar` material.
    pub fn sidebar_px(&self) -> i32 {
        if self.sidebar.is_some() { self.side_w } else { 0 }
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        if self.side_w == 0 {
            return self.draw_pane(s, ui, t);
        }
        let selected = self.side_selected();
        let (side_w, h) = (self.side_w as usize, self.h as usize);
        // the sidebar rarely changes while the pane slides or scrolls: draw it once and reuse it
        if let Some(sb) = &mut self.sidebar {
            sb.select(selected);
            use std::hash::{Hash, Hasher};
            let mut k = std::collections::hash_map::DefaultHasher::new();
            (format!("{selected:?}{:?}", self.side_pressed), sb.gen(), sb.draw_offset().to_bits(), side_w, h, bifrost_render::appearance::scheme().to_bits(), vibrant(), bifrost_render::ui::scroll_bars_mode()).hash(&mut k);
            let key = k.finish();
            let fresh = !sb.animating(t) && self.side_cache.as_ref().is_some_and(|(k, i)| *k == key && (i.w, i.h) == (side_w, h));
            if !fresh {
                let mut img = self.side_cache.take().map(|(_, i)| i).filter(|i| (i.w, i.h) == (side_w, h)).unwrap_or_else(|| Image::new(side_w, h));
                {
                    let mut ss = Surface { w: img.w, h: img.h, stride: img.w, px: &mut img.px, y0: 0 };
                    sidebar_fill(&mut ss, 0, 0, side_w, h);
                    sb.draw(&mut ss, ui, self.side_pressed, t);
                    ss.fill_rrect(side_w as f32 - 1.0, 0.0, 1.0, h as f32, 0.0, SEPARATOR_C.now());
                }
                self.side_cache = Some((key, img));
            }
            if let Some((_, img)) = &self.side_cache {
                s.copy(img.view(), 0, 0);
            }
        }
        // the sidebar's search field is live, so it draws over the cached column
        if let Some(frame) = self.sidebar.as_ref().and_then(|sb| sb.search_frame(ui)) {
            self.search.draw_field(s, ui, frame, t);
        }
        // scrolled content passes under the traffic-light zone, which stays clear
        if self.sidebar.as_ref().is_some_and(|sb| sb.draw_offset() > 0.0) {
            sidebar_fill(s, 0, 0, side_w.saturating_sub(1), ui.pt(bifrost_ui::prefs_layout::m::SEARCH_Y - 9.0) as usize);
        }
        // the pane draws straight into the frame through a column view (same stride, offset by the
        // sidebar): no 6 MB pane buffer, no full-pane blend blit
        let (stride, y0, pane_w) = (s.stride, s.y0, self.w as usize);
        let rows = s.h;
        let mut ps = Surface { w: pane_w, h: rows, stride, px: &mut s.px[side_w..], y0 };
        let banded = match (&self.transition, self.stack.last().map(|e| &e.view)) {
            (None, Some(View::List(page))) => Some(PageView { page, w: self.w, h: self.h, pressed: self.pressed }),
            _ => None,
        };
        match banded {
            // a still list page: worker threads draw it in row bands (the bar's blur needs
            // its whole strip on one surface), then the chrome and overlays go on top serially
            Some(view) if y0 == 0 => {
                let top = (super::list::nav_bottom(ui).ceil() as usize + 1).min(h);
                ps.draw_rows(&[(0, top)], 1, &|s| view.draw(s, ui, t));
                ps.draw_rows(&[(top, h)], crate::gfx::threads(), &|s| view.draw(s, ui, t));
                self.draw_chrome(&mut ps, ui, t);
                self.draw_overlays(&mut ps, ui, t);
            }
            _ => self.draw_pane(&mut ps, ui, t),
        }
    }

    fn draw_pane(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        let Some(top) = self.stack.last() else { return };
        match self.transition.take() {
            Some(mut tr) => {
                let n = self.stack.len();
                if tr.imgs.is_none() {
                    // the page that stays underneath and the one that moves over it
                    let (under, over) = if tr.push { (&self.stack[n - 2], top) } else { (top, tr.leaving.as_ref().expect("leaving")) };
                    tr.imgs = Some((self.render(under, ui, t), self.render(over, ui, t)));
                    // the slide's clock starts with its first frame: building the page and these
                    // renders take a while, and a slide timed from the tap would skip its start
                    tr.start = tr.start.max(t);
                }
                let pl = slide(tr.push).placement(((t - tr.start) / PUSH_MS) as f32, self.w as f32, 0.0);
                let (under_img, over_img) = tr.imgs.as_ref().expect("rendered");
                s.blit(under_img, pl.under_x.round() as i32, 0, 255 - (pl.shade * 255.0).round() as u32, 255);
                // the moving page is opaque: a row copy, not a per-pixel blend
                let ox = (pl.over_x.round() as i32).clamp(-(over_img.w as i32), self.w);
                s.copy(over_img.view(), ox, 0);
                self.transition = Some(tr);
            }
            None => {
                self.draw_entry(s, top, ui, self.pressed, t);
                self.draw_chrome(s, ui, t);
            }
        }
        self.draw_overlays(s, ui, t);
    }

    /// The root page's search capsule and keyboard (only when no slide is running).
    fn draw_chrome(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        if self.transition.is_none() && self.stack.len() == 1 && self.side_w == 0 {
            if let Some(frame) = self.list().and_then(|p| p.search_frame(ui)) {
                self.search.draw_field(s, ui, frame, t);
            }
            self.search.draw_keyboard(s, ui, self.w, self.h, t);
        }
    }

    fn draw_overlays(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        if let Some(a) = &self.alert {
            a.draw(s, ui, self.w, self.h);
        }
        if let Some(sh) = &mut self.sheet {
            sh.draw(s, ui, self.w, self.h, t);
        }
        if let Some(p) = &mut self.zone_picker {
            p.draw(s, ui, t);
        }
        if let Some(p) = &mut self.time_picker {
            p.draw(s, ui, t);
        }
        if let Some(o) = &self.passcode {
            o.draw(s, ui, t);
        }
        if let Some((l, _)) = &self.st_gate {
            l.draw(s, ui, t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::pack;
    use crate::shell::Network;

    fn info() -> Info {
        Info { battery: 42, charging: true, online: false, wifi: "Off".into(), model: "Midget".into(), kernel: "6.12".into(), memory: "2.4 GB".into(), brightness: 0.5, ..Info::default() }
    }

    fn net(ssid: &str, secure: bool, saved: bool) -> Network {
        Network { ssid: ssid.into(), bars: 2, secure, saved }
    }

    fn wifi_info() -> Info {
        Info {
            wifi: "Home".into(),
            wifi_on: true,
            current_ssid: Some("Home".into()),
            networks: vec![net("Home", true, true), net("Cafe", false, false), net("Office", true, true), net("Neighbour", true, false)],
            ..info()
        }
    }

    /// The single-column layout: the page logic tests (push/pop, rows) predate the split view.
    fn settings(ui: &Ui, info: &Info) -> (Settings, Prefs) {
        let mut st = Settings::new(ui, info, &Prefs::memory(), 1200, 1920, Rc::new(vec![]));
        st.allow_split(ui, false);
        (st, Prefs::memory())
    }

    fn tap(st: &mut Settings, ui: &Ui, prefs: &mut Prefs, x: f32, y: f32) -> Vec<Effect> {
        let mut fx = st.touch(ui, prefs, Event::Down(x, y), 0.0);
        fx.extend(st.touch(ui, prefs, Event::Up(x, y), 10.0));
        fx
    }

    /// Taps the row labelled `label` on the top page and settles the slide.
    fn tap_row(st: &mut Settings, ui: &Ui, prefs: &mut Prefs, label: &str) -> Vec<Effect> {
        // root rows live in the sidebar in the bifrost split layout
        let (x, y) = st.row_centre(ui, label).or_else(|| st.sidebar_centre(ui, label)).unwrap_or_else(|| panic!("no row {label} on {:?}", st.page()));
        let fx = tap(st, ui, prefs, x, y);
        settle(st, ui);
        fx
    }

    /// Chooses the `k`th choice of the pop-up row labelled `label`: taps the row to open its menu, then picks from the menu.
    fn pick(st: &mut Settings, ui: &Ui, prefs: &mut Prefs, label: &str, k: usize) -> Vec<Effect> {
        tap_row(st, ui, prefs, label);
        let fx = st.activate(ui, prefs, Hit::Choice(k), 0.0);
        settle(st, ui);
        fx
    }

    fn settle(st: &mut Settings, ui: &Ui) {
        st.tick(ui, 1e9);
        // the pages' reads run on worker threads (see `settings_load`): wait for them, as a frame would
        let began = std::time::Instant::now();
        while st.loads.busy() && began.elapsed() < std::time::Duration::from_secs(30) {
            std::thread::sleep(std::time::Duration::from_millis(1));
            st.tick(ui, 1e9);
        }
    }

    fn back(st: &mut Settings, ui: &Ui, prefs: &mut Prefs) {
        tap(st, ui, prefs, ui.pt(40.0), ui.pt(42.0));
        settle(st, ui);
    }

    /// A data dir with the packed default fonts and one library font (a copy of the system font).
    fn font_dir(tag: &str) -> (PathBuf, String) {
        let dir = std::env::temp_dir().join(format!("settings-fonts-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("fonts")).unwrap();
        let assets = concat!(env!("CARGO_MANIFEST_DIR"), "/assets");
        for f in ["SFUI.ttf", "SFUIMono.ttf"] {
            std::fs::copy(format!("{assets}/{f}"), dir.join(f)).unwrap();
        }
        let lib = dir.join("fonts/Other.ttf");
        std::fs::copy(format!("{assets}/SFUI.ttf"), &lib).unwrap();
        (dir, lib.to_string_lossy().into_owned())
    }

    /// Lets the Fonts read on its worker land, as the frames would.
    fn settle_fonts(st: &mut Settings, ui: &Ui) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while st.loads.fonts.busy() && std::time::Instant::now() < end {
            st.tick(ui, 0.0);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        st.tick(ui, 0.0);
    }

    #[test]
    fn a_deep_link_from_another_app_opens_its_pane() {
        let ui = super::super::tests::ui();
        let (mut st, _prefs) = settings(&ui, &wifi_info());
        st.open_page(&ui, "Wi-Fi Settings", 0.0);
        assert_eq!(st.page(), PageId::Wifi);
        st.open_page(&ui, "Battery", 0.0);
        assert_eq!(st.page(), PageId::Battery);
        st.open_page(&ui, "Print & Scan", 0.0);
        assert_eq!(st.page(), PageId::PrintScan);
        assert!(st.open_page(&ui, "No Such Pane", 0.0).is_empty());
    }

    /// Opens General > Startup Disk with a boot table of two usable systems landed.
    fn startup_page(ui: &Ui) -> (Settings, Prefs) {
        use super::super::startup_disk_page::{Disk, View};
        let (mut st, mut prefs) = settings(ui, &wifi_info());
        tap_row(&mut st, ui, &mut prefs, "General");
        tap_row(&mut st, ui, &mut prefs, "Startup Disk");
        assert_eq!(st.page(), PageId::StartupDisk);
        let disk = |id: &str, label: &str, default: bool| Disk { id: id.into(), label: label.into(), active: true, current: default, next: false, default };
        st.loads.startup.set(View { disks: vec![disk("0001", "BifrOSt", true), disk("0003", "Rescue", false)], error: None });
        st.rebuild_visible(ui);
        (st, prefs)
    }

    #[test]
    fn the_screen_sharing_switch_asks_for_an_account_before_anything_starts() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = settings(&ui, &wifi_info());
        tap_row(&mut st, &ui, &mut prefs, "Sharing");
        assert_eq!(st.page(), PageId::Sharing);
        st.loads.sharing.set(super::super::sharing_page::View { computer: "Midget".into(), services: vec![("screen_sharing", false)], screen_user: None });
        st.rebuild_visible(&ui);
        tap_row(&mut st, &ui, &mut prefs, "Screen Sharing");
        assert!(st.sheet.is_some(), "the account prompt is up");
        assert!(st.alert().is_none(), "nothing was sent to privd");
    }

    #[test]
    fn startup_disk_is_a_general_row_that_lists_the_systems() {
        let ui = super::super::tests::ui();
        let (st, _prefs) = startup_page(&ui);
        let spec = st.spec_for(PageId::StartupDisk);
        let labels: Vec<&str> = spec.groups[0].items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["BifrOSt", "Rescue"]);
        assert_eq!(spec.groups[0].items[1].action, Action::StartupDisk(super::super::startup_disk_page::Edit::Choose("0003".into())));
    }

    #[test]
    fn choosing_a_system_waits_for_the_password_sheet_and_a_cancel_changes_nothing() {
        use super::super::services::local_auth::{self, Failure};
        use super::super::startup_disk_page::APP;
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = startup_page(&ui);
        tap_row(&mut st, &ui, &mut prefs, "Rescue");
        assert_eq!(st.startup_pending.as_deref(), Some("0003"));
        let request = local_auth::take_request().expect("the sheet is queued");
        assert_eq!(request.app, APP);
        assert!(request.reason.contains("Rescue"));
        // while the sheet is up a second tap does not queue another
        assert!(local_auth::take_request().is_none());
        local_auth::finish(APP, Err(Failure::UserCancel));
        st.tick(&ui, 2e9);
        assert_eq!(st.startup_pending, None);
        assert!(st.alert().is_none(), "a cancel says nothing");
    }

    #[test]
    fn a_wrong_password_is_reported_and_sets_nothing() {
        use super::super::services::local_auth::{self, Failure};
        use super::super::startup_disk_page::APP;
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = startup_page(&ui);
        tap_row(&mut st, &ui, &mut prefs, "Rescue");
        assert!(local_auth::take_request().is_some());
        local_auth::finish(APP, Err(Failure::AuthenticationFailed));
        st.tick(&ui, 2e9);
        assert_eq!(st.startup_pending, None);
        assert!(st.alert().is_some(), "the person is told the change did not happen");
    }

    #[test]
    fn the_firewall_page_hands_the_shell_each_change() {
        use super::super::firewall_sheet::{Edit, FirewallView};
        use bifrost_core::app_firewall::Policy;
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = settings(&ui, &info());
        st.set_firewall(vec![FirewallView { app_id: "is.example.chat".into(), name: "Chat".into(), policy: Policy::Block, ports: "tcp/8080".into() }]);
        st.open_page_id(&ui, PageId::Firewall, 0.0);
        settle(&mut st, &ui);
        assert_eq!(st.page(), PageId::Firewall);
        tap_row(&mut st, &ui, &mut prefs, "Allow Incoming Connections");
        assert_eq!(st.take_firewall_edits(), vec![("is.example.chat".to_string(), Edit::Allow)]);
        assert_eq!(st.firewall[0].policy, Policy::Allow, "the row shows the choice at once");
        assert!(st.take_firewall_edits().is_empty());
        tap_row(&mut st, &ui, &mut prefs, "Forget This Choice");
        assert_eq!(st.take_firewall_edits(), vec![("is.example.chat".to_string(), Edit::Forget)]);
        assert!(st.firewall.is_empty(), "a forgotten choice leaves the page");
    }

    #[test]
    fn picking_a_library_font_saves_the_override_the_shell_reads() {
        use bifrost_render::fontprefs::{family, FontRole};
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = settings(&ui, &wifi_info());
        let (dir, lib) = font_dir("pick");
        st.set_data_dir(&dir);
        let name = family(&lib).expect("the library font has a family name");
        for label in ["General", "Fonts"] {
            tap_row(&mut st, &ui, &mut prefs, label);
        }
        settle_fonts(&mut st, &ui);
        tap_row(&mut st, &ui, &mut prefs, "System Font");
        assert_eq!(st.page(), PageId::FontPick(FontRole::System));
        tap_row(&mut st, &ui, &mut prefs, &name);
        assert_eq!(prefs.get(FontRole::System.pref()), Some(lib.as_str()), "saved under the key the shell reads at start");
        assert_eq!(st.loads.fonts.value.overrides[FontRole::System.index()].as_deref(), Some(lib.as_str()));
        let default = format!("{} (Default)", st.loads.fonts.value.defaults[FontRole::System.index()]);
        tap_row(&mut st, &ui, &mut prefs, &default);
        assert_eq!(prefs.get(FontRole::System.pref()), Some(""), "Default clears the override");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Developer preview: `PREVIEW_DIR=/tmp FONTS_DIR=<packed assets> cargo test preview_fonts_pages -- --ignored`
    /// writes fonts.png and fonts-system.png, the desktop Fonts pages drawn with the packed default font.
    #[test]
    #[ignore]
    fn preview_fonts_pages() {
        let (Some(out), Some(fonts)) = (std::env::var_os("PREVIEW_DIR"), std::env::var("FONTS_DIR").ok()) else { return };
        let data: &'static [u8] = Box::leak(std::fs::read(format!("{fonts}/SFUI.ttf")).unwrap().into_boxed_slice());
        let ui = Ui::new(crate::font::Font::parse(data).unwrap(), 1.5);
        let (w, h) = (1500usize, 1050usize);
        let info = Info { display_mode: Some((2048, 1152, 60)), ..wifi_info() };
        let mut prefs = Prefs::memory();
        let mut st = Settings::new(&ui, &info, &prefs, w as i32, h as i32, Rc::new(vec![]));
        st.set_data_dir(&fonts);
        st.open_page(&ui, "Fonts", 0.0);
        settle(&mut st, &ui);
        let shot = |st: &mut Settings, name: &str| {
            let mut img = crate::gfx::Image::new(w, h);
            st.draw(&mut Surface { w, h, stride: w, px: &mut img.px, y0: 0 }, &ui, 1e9);
            std::fs::write(std::path::Path::new(&out).join(name), crate::shell::screenshot::encode(&img)).unwrap();
        };
        shot(&mut st, "fonts.png");
        tap_row(&mut st, &ui, &mut prefs, "System Font");
        shot(&mut st, "fonts-system.png");
    }

    #[test]
    fn banded_page_draw_equals_the_single_thread_draw() {
        let ui = super::super::tests::ui();
        let (st, _) = settings(&ui, &wifi_info());
        let (w, h) = (1200usize, 1920usize);
        let mut one = vec![0u32; w * h];
        st.page_view().expect("plain list").draw(&mut Surface { w, h, stride: w, px: &mut one, y0: 0 }, &ui, 1e9);
        let mut many = vec![0u32; w * h];
        let view = st.page_view().unwrap();
        Surface { w, h, stride: w, px: &mut many, y0: 0 }.draw_rows(&[(0, h)], 4, &|s| view.draw(s, &ui, 1e9));
        assert!(one == many, "banded draw differs from single-thread draw");
    }

    #[test]
    fn rows_push_pages_and_back_pops_them_with_a_slide() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = settings(&ui, &wifi_info());
        assert_eq!(st.page(), PageId::Root);
        let (x, y) = st.row_centre(&ui, "Wi-Fi").unwrap();
        let fx = tap(&mut st, &ui, &mut prefs, x, y);
        assert_eq!(fx, vec![Effect::Wifi(WifiAction::Scan)]);
        assert_eq!(st.page(), PageId::Wifi);
        assert!(st.transitioning());
        assert_eq!(st.next_wake(20.0), Some(20.0), "the slide wants frames");
        assert!(tap(&mut st, &ui, &mut prefs, x, y).is_empty(), "touches wait for the slide");
        let mut buf = vec![0u32; 1200 * 1920];
        st.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 100.0);
        assert!(st.tick(&ui, 200.0));
        assert!(st.tick(&ui, 1000.0), "one last frame draws the settled page");
        assert!(!st.transitioning());
        assert!(!st.tick(&ui, 1100.0));
        assert_eq!(st.stack(), vec![PageId::Root, PageId::Wifi]);
        back(&mut st, &ui, &mut prefs);
        assert_eq!(st.page(), PageId::Root);
        assert!(tap_row(&mut st, &ui, &mut prefs, "Battery").is_empty(), "the Battery row is a plain link");
        assert_eq!(st.page(), PageId::Battery);
        back(&mut st, &ui, &mut prefs);
        back(&mut st, &ui, &mut prefs);
        assert_eq!(st.stack(), vec![PageId::Root], "the root never pops");
    }

    #[test]
    fn every_page_is_reachable_and_draws() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = settings(&ui, &wifi_info());
        let mut buf = vec![0u32; 1200 * 1920];
        let mut draw = |st: &mut Settings| st.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 1e9);
        let routes: Vec<(Vec<&str>, PageId)> = vec![
            (vec!["Sounds"], PageId::Sounds),
            (vec!["Focus"], PageId::Focus),
            (vec!["Notifications"], PageId::Notifications),
            (vec!["General"], PageId::General),
            (vec!["General", "About"], PageId::About),
            (vec!["General", "Software Update"], PageId::SoftwareUpdate),
            (vec!["General", "Date & Time"], PageId::DateTime),
            (vec!["General", "Default Apps"], PageId::DefaultApps),
            (vec!["General", "Login Items & Extensions"], PageId::LoginItems),
            (vec!["General", "Game Controllers"], PageId::GameControllers),
            (vec!["General", "Fonts"], PageId::Fonts),
            (vec!["General", "Fonts", "System Font"], PageId::FontPick(bifrost_render::fontprefs::FontRole::System)),
            (vec!["General", "Keyboard"], PageId::Keyboard),
            (vec!["General", "Keyboard", "Keyboards"], PageId::Keyboards),
            (vec!["General", "Keyboard", "Text Replacements"], PageId::TextReplacements),
            (vec!["General", "Keyboard", "Modifier Keys"], PageId::ModifierKeys),
            (vec!["General", "Language & Region"], PageId::Language),
            (vec!["General", "Language & Region", "Region"], PageId::Region),
            (vec!["General", "Transfer or Reset Tab"], PageId::Reset),
            (vec!["Control Center"], PageId::ControlCentre),
            (vec!["Menu Bar"], PageId::MenuBar),
            (vec!["Display & Brightness"], PageId::Display),
            (vec!["Display & Brightness", "Auto-Lock"], PageId::AutoLock),
            (vec!["Home Screen & Dock"], PageId::HomeScreen),
            (vec!["Home Screen & Dock", "Hot Corners"], PageId::HotCorners),
            (vec!["Overview"], PageId::Overview),
            (vec!["Sharing"], PageId::Sharing),
            (vec!["Print & Scan"], PageId::PrintScan),
            (vec!["Sharing", "Nearby Share"], PageId::Nearby),
            (vec!["Sharing", "AirPlay & Handoff"], PageId::AirPlayHandoff),
            (vec!["Lock Screen"], PageId::LockScreen),
            (vec!["Trackpad"], PageId::Trackpad),
            (vec!["Mouse"], PageId::Mouse),
            (vec!["Users & Groups"], PageId::Users),
            (vec!["Keyboard Shortcuts"], PageId::Shortcuts),
            (vec!["Accessibility"], PageId::Accessibility),
            (vec!["Wallpaper"], PageId::Wallpaper),
            (vec!["Screen Saver"], PageId::ScreenSaver),
            (vec!["Battery"], PageId::Battery),
            (vec!["Privacy"], PageId::Privacy),
            (vec!["Privacy", "App Permissions"], PageId::PrivacyPermissions),
            (vec!["Privacy", "Location Services"], PageId::LocationServices),
            (vec!["Privacy", "Analytics & Diagnostics"], PageId::Diagnostics),
            (vec!["General", "bifrost Storage"], PageId::Storage),
            (vec!["General", "Backup"], PageId::Backup),
            (vec!["Passcode"], PageId::Passcode),
            (vec!["Passwords"], PageId::Passwords),
            (vec!["Internet Accounts"], PageId::InternetAccounts),
            (vec!["Developer"], PageId::Developer),
            (vec!["Wi-Fi", "Home"], PageId::WifiDetail),
        ];
        let desk = true;
        let routes: Vec<(Vec<&str>, PageId)> = routes
            .into_iter()
            .filter(|(r, _)| !(desk && r.last() == Some(&"Transfer or Reset Tab")) && (desk || r.last() != Some(&"Default Apps")))
            .map(|(r, id)| if desk && r.last() == Some(&"bifrost Storage") { (vec!["General", "Storage"], id) } else { (r, id) })
            .collect();
        for (route, id) in routes {
            for label in &route {
                tap_row(&mut st, &ui, &mut prefs, label);
            }
            assert_eq!(st.page(), id, "{route:?}");
            draw(&mut st);
            for _ in 0..route.len() {
                back(&mut st, &ui, &mut prefs);
            }
            assert_eq!(st.page(), PageId::Root);
        }
    }

    #[test]
    fn a_drag_off_the_row_cancels_the_tap_and_scrolls() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = settings(&ui, &wifi_info());
        let (x, y) = st.row_centre(&ui, "Wi-Fi").unwrap();
        st.touch(&ui, &mut prefs, Event::Down(x, y), 0.0);
        for k in 1..=5 {
            st.touch(&ui, &mut prefs, Event::Move(x, y - 80.0 * k as f32), k as f64 * 30.0);
        }
        assert!(st.touch(&ui, &mut prefs, Event::Up(x, y - 400.0), 160.0).is_empty());
        assert_eq!(st.page(), PageId::Root);
        assert!(st.list().unwrap().offset() > 0.0, "the list scrolled");
    }

    #[test]
    fn airplane_mode_switches_wifi_off_and_on() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = settings(&ui, &wifi_info());
        assert_eq!(tap_row(&mut st, &ui, &mut prefs, "Airplane Mode"), vec![Effect::Wifi(WifiAction::Enable(false))]);
        let root = st.list().unwrap();
        let (g, i) = root.find("Wi-Fi").unwrap();
        assert_eq!(root.item(g, i).value, "Off");
        // group 0 is the BifrOSt Account row, so the Airplane Mode switch is the first row of group 1
        assert_eq!(root.item(1, 0).accessory, super::super::list::Accessory::Switch(true));
        assert_eq!(tap_row(&mut st, &ui, &mut prefs, "Airplane Mode"), vec![Effect::Wifi(WifiAction::Enable(true))]);
        assert_eq!(st.list().unwrap().item(g, i).value, "Home");
    }

    #[test]
    fn wifi_switch_toggles_and_hides_the_networks() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = settings(&ui, &wifi_info());
        tap_row(&mut st, &ui, &mut prefs, "Wi-Fi");
        // the switch, the networks, and the Country or Region group
        assert_eq!(st.list().unwrap().spec().groups.len(), 3);
        // the switch animates right after the tap; the settle that follows reloads the page and ends it
        let (x, y) = st.row_centre(&ui, "Wi-Fi").unwrap();
        let fx = tap(&mut st, &ui, &mut prefs, x, y);
        assert_eq!(fx, vec![Effect::Wifi(WifiAction::Enable(false))]);
        assert!(st.list().unwrap().animating(50.0), "the switch animates");
        settle(&mut st, &ui);
        assert_eq!(st.list().unwrap().spec().groups.len(), 1, "no networks while off");
        let fx = tap_row(&mut st, &ui, &mut prefs, "Wi-Fi");
        assert_eq!(fx, vec![Effect::Wifi(WifiAction::Enable(true)), Effect::Wifi(WifiAction::Scan)]);
        assert_eq!(st.list().unwrap().spec().groups.len(), 3, "the switch, the networks and Country or Region are back");
    }

    #[test]
    fn networks_join_select_or_ask_for_a_password() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = settings(&ui, &wifi_info());
        tap_row(&mut st, &ui, &mut prefs, "Wi-Fi");
        assert_eq!(tap_row(&mut st, &ui, &mut prefs, "Cafe"), vec![Effect::Wifi(WifiAction::Join { ssid: "Cafe".into(), psk: None })]);
        assert_eq!(tap_row(&mut st, &ui, &mut prefs, "Office"), vec![Effect::Wifi(WifiAction::Select("Office".into()))]);
        assert!(tap_row(&mut st, &ui, &mut prefs, "Neighbour").is_empty());
        assert_eq!(st.sheet(), Some("Neighbour"));
    }

    #[test]
    fn other_row_opens_the_sheet_that_joins_a_hidden_network() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = settings(&ui, &wifi_info());
        tap_row(&mut st, &ui, &mut prefs, "Wi-Fi");
        assert!(tap_row(&mut st, &ui, &mut prefs, "Other…").is_empty());
        assert!(st.sheet().is_some());
        st.key(&ui, Out::Text("Ghost".into()), 0.0);
        st.key(&ui, Out::Named("Tab"), 0.0);
        st.key(&ui, Out::Text("secret12".into()), 0.0);
        let fx = st.key(&ui, Out::Enter, 0.0);
        assert!(matches!(&fx[..], [Effect::Wifi(WifiAction::JoinOther(c))] if c.ssid == "Ghost" && c.hidden && c.password == "secret12"));
        assert_eq!(st.sheet(), None);
    }

    #[test]
    fn a_scanned_enterprise_network_opens_the_full_sheet() {
        let ui = super::super::tests::ui();
        let info = Info { enterprise: vec!["Neighbour".into()], ..wifi_info() };
        let (mut st, mut prefs) = settings(&ui, &info);
        tap_row(&mut st, &ui, &mut prefs, "Wi-Fi");
        assert!(tap_row(&mut st, &ui, &mut prefs, "Neighbour").is_empty());
        assert_eq!(st.sheet(), Some("Neighbour"));
        assert!(st.sheet.as_ref().unwrap().config().unwrap().security.is_enterprise());
    }

    fn sheet_open(ui: &Ui) -> (Settings, Prefs) {
        let (mut st, mut prefs) = settings(ui, &wifi_info());
        tap_row(&mut st, ui, &mut prefs, "Wi-Fi");
        tap_row(&mut st, ui, &mut prefs, "Neighbour");
        (st, prefs)
    }

    #[test]
    fn join_needs_eight_characters() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = sheet_open(&ui);
        let (jx, jy) = st.sheet.as_ref().unwrap().join_button(&ui, 1200, 1920);
        st.key(&ui, Out::Text("secret1".into()), 0.0);
        assert!(tap(&mut st, &ui, &mut prefs, jx, jy).is_empty(), "7 characters");
        assert!(st.key(&ui, Out::Enter, 0.0).is_empty());
        st.key(&ui, Out::Text("2".into()), 0.0);
        let join = vec![Effect::Wifi(WifiAction::Join { ssid: "Neighbour".into(), psk: Some("secret12".into()) })];
        assert_eq!(tap(&mut st, &ui, &mut prefs, jx, jy), join);
        assert_eq!(st.sheet(), None);
        let (mut st, _) = sheet_open(&ui);
        st.key(&ui, Out::Text("secret12".into()), 0.0);
        assert_eq!(st.key(&ui, Out::Enter, 0.0), join, "return submits");
    }

    #[test]
    fn cancel_closes_the_sheet_without_effects() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = sheet_open(&ui);
        let (cx, cy) = st.sheet.as_ref().unwrap().cancel_button(&ui, 1200, 1920);
        assert!(tap(&mut st, &ui, &mut prefs, cx, cy).is_empty());
        assert_eq!(st.sheet(), None);
        assert_eq!(st.page(), PageId::Wifi);
    }

    #[test]
    fn hidden_keyboard_returns_when_the_field_is_tapped() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = sheet_open(&ui);
        st.key(&ui, Out::Hide, 0.0);
        let sh = st.sheet.as_ref().unwrap();
        assert!(sh.keyboard_top(&ui, 1920).is_none());
        let (fx, fy) = sh.field(&ui, 1200, 1920);
        tap(&mut st, &ui, &mut prefs, fx, fy);
        assert!(st.sheet.as_ref().unwrap().keyboard_top(&ui, 1920).is_some());
    }

    #[test]
    fn long_press_on_the_password_field_offers_paste() {
        use super::super::clipboard;
        let _g = clipboard::tests::exclusive();
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = sheet_open(&ui);
        clipboard::set_text("hunter22");
        let (fx, fy) = st.sheet.as_ref().unwrap().field(&ui, 1200, 1920);
        st.touch(&ui, &mut prefs, Event::Down(fx, fy), 1000.0);
        assert!(st.next_wake(1000.0).is_some_and(|w| w <= 1400.0), "the long press wants a tick");
        assert!(st.tick(&ui, 1400.0));
        assert!(st.touch(&ui, &mut prefs, Event::Up(fx, fy), 1450.0).is_empty());
        let menu = st.sheet.as_ref().unwrap().field.menu().expect("Paste over the field");
        let (mx, my, _, mh) = menu.rect();
        assert_eq!(menu.hit(mx + 5.0, my + 5.0), Some(super::super::edit::Action::Paste));
        assert!(tap(&mut st, &ui, &mut prefs, mx + 5.0, my + mh / 2.0).is_empty());
        let sh = st.sheet.as_ref().unwrap();
        assert_eq!(sh.password(), "hunter22");
        assert!(sh.can_join() && sh.field.menu().is_none());
    }

    #[test]
    fn typing_on_the_docked_keyboard_edits_the_password() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = sheet_open(&ui);
        let top = st.sheet.as_ref().unwrap().keyboard_top(&ui, 1920).unwrap();
        let (x, y) = st.sheet.as_ref().unwrap().keyboard.centre("q").unwrap();
        let y = top + y;
        tap(&mut st, &ui, &mut prefs, x, y);
        assert_eq!(st.sheet.as_ref().unwrap().password(), "q");
    }

    fn stills() -> Rc<Vec<Wallpaper>> {
        let thumb = |c| Image { w: 320, h: 180, px: vec![pack(c, c, c, 255); 320 * 180] };
        Rc::new(vec![
            Wallpaper { id: "a".into(), name: "Desert Peak Day".into(), thumb: thumb(10) },
            Wallpaper { id: "b".into(), name: "Lake The Lake Night".into(), thumb: thumb(20) },
        ])
    }

    #[test]
    fn dynamic_wallpaper_switch_persists_and_turning_it_off_restores_the_picked_still() {
        let ui = super::super::tests::ui();
        let mut prefs = Prefs::memory();
        let mut st = Settings::new(&ui, &Info { wallpaper: "a".into(), ..info() }, &prefs, 1200, 1920, stills());
        st.allow_split(&ui, false);
        assert!(tap_row(&mut st, &ui, &mut prefs, "Wallpaper").is_empty());
        let (x, y, w, h) = super::super::wallpaper_page::dynamic_row(&ui, 1200);
        assert!(tap(&mut st, &ui, &mut prefs, x + w / 2.0, y + h / 2.0).is_empty(), "on: the shell changes the still on its next tick");
        assert_eq!(prefs.get("wallpaper_dynamic"), Some("1"));
        assert_eq!(tap(&mut st, &ui, &mut prefs, x + w / 2.0, y + h / 2.0), vec![Effect::Wallpaper("a".into())], "off: the picked still comes back");
        assert_eq!(prefs.get("wallpaper_dynamic"), Some("0"));
    }

    #[test]
    fn live_look_row_picks_an_animated_wallpaper_persists_it_and_reset_goes_back_to_the_still() {
        let ui = super::super::tests::ui();
        let mut prefs = Prefs::memory();
        let mut st = Settings::new(&ui, &Info { wallpaper: "a".into(), ..info() }, &prefs, 1200, 1920, stills());
        st.allow_split(&ui, false);
        assert!(tap_row(&mut st, &ui, &mut prefs, "Wallpaper").is_empty());
        let centre = |i: usize| {
            let (x, y, w, h) = super::super::wallpaper_page::look_segment(&ui, 1200, i);
            (x + w / 2.0, y + h / 2.0)
        };
        for (i, look) in Look::ALL.into_iter().enumerate() {
            let (x, y) = centre(i);
            assert_eq!(tap(&mut st, &ui, &mut prefs, x, y), vec![Effect::LiveWallpaper(look.pref().to_string())], "{look:?}");
            assert_eq!(prefs.get("wallpaper_look"), Some(look.pref()));
            let View::Wallpaper(page) = &st.stack.last().unwrap().view else { panic!("wallpaper page") };
            assert_eq!(page.look(), look, "the selected plate follows the tap");
        }
        let (x, y) = centre(1);
        tap(&mut st, &ui, &mut prefs, x, y);
        let mut buf = vec![0u32; 1200 * 1920];
        st.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 1e9);
        // a fresh Settings reads the stored look back
        let again = Settings::new(&ui, &Info { wallpaper: "a".into(), ..info() }, &prefs, 1200, 1920, stills());
        assert_eq!(again.wallpaper_look, Look::Drift);
        let fx = st.run(&ui, &mut prefs, Command::ResetSettings, 0.0);
        assert!(fx.contains(&Effect::LiveWallpaper("still".into())));
        assert_eq!(st.wallpaper_look, Look::Still);
        assert_eq!(prefs.get("wallpaper_look"), None);
    }

    #[test]
    fn on_the_desktop_a_click_applies_a_wallpaper_where_show_on_says_and_the_add_buttons_ask_for_the_panel() {
        use bifrost_ui::form_factor::FormFactor;
        let ui = super::super::tests::ui();
        super::super::pages::set_test_form(FormFactor::Desktop);
        let mut prefs = Prefs::memory();
        let mut st = Settings::new(&ui, &Info { wallpaper: "a".into(), ..info() }, &prefs, 1200, 1920, stills());
        st.allow_split(&ui, false); // single-column geometry
        assert!(tap_row(&mut st, &ui, &mut prefs, "Wallpaper").is_empty());
        let View::Wallpaper(page) = &st.stack.last().unwrap().view else { panic!("wallpaper page") };
        assert!(page.desktop());
        let (cx, cy, cw, ch) = page.cell(&ui, 1200, 1);
        // a click applies it at once, to the desktop and the lock screen together: no preview
        assert_eq!(tap(&mut st, &ui, &mut prefs, cx + cw / 2.0, cy + ch / 2.0), vec![Effect::LockWallpaper(String::new()), Effect::Wallpaper("b".into())]);
        let View::Wallpaper(page) = &st.stack.last().unwrap().view else { panic!("wallpaper page") };
        assert!(!page.previewing(), "the desktop has no Set Lock Screen / Home / Both preview");
        assert_eq!((prefs.get("wallpaper"), st.info.wallpaper.as_str()), (Some("b"), "b"));
        // Lock Screen Only: the next click sets the lock screen alone, and the choice is kept
        let (sx, sy, sw, sh) = super::super::wallpaper_page::show_on_segment(&ui, 1200, 1);
        assert!(tap(&mut st, &ui, &mut prefs, sx + sw / 2.0, sy + sh / 2.0).is_empty());
        assert_eq!(prefs.get(SHOW_ON_PREF), Some("lock"));
        let (cx, cy, cw, ch) = match &st.stack.last().unwrap().view {
            View::Wallpaper(page) => page.cell(&ui, 1200, 0),
            _ => panic!("wallpaper page"),
        };
        assert_eq!(tap(&mut st, &ui, &mut prefs, cx + cw / 2.0, cy + ch / 2.0), vec![Effect::LockWallpaper("a".into())]);
        assert_eq!((prefs.get("wallpaper_lock"), st.info.wallpaper.as_str()), (Some("a"), "b"), "the desktop keeps its wallpaper");
        assert_eq!(Settings::new(&ui, &info(), &prefs, 1200, 1920, stills()).wallpaper_show_on, ShowOn::LockScreen);
        // Add Photo… and Add Folder… ask the shell for its open panel once
        for (i, kind) in Import::ALL.into_iter().enumerate() {
            let (bx, by, bw, bh) = super::super::wallpaper_page::add_button(&ui, 1200, i);
            assert!(tap(&mut st, &ui, &mut prefs, bx + bw / 2.0, by + bh / 2.0).is_empty());
            assert_eq!(st.take_wallpaper_import(), Some(kind));
            assert_eq!(st.take_wallpaper_import(), None);
        }
        // a picture added through the panel joins the grid and is applied where Show On says
        let mut more = (*stills()).clone();
        more.push(Wallpaper { id: "photo_fjord".into(), name: "Fjord".into(), thumb: Image { w: 320, h: 180, px: vec![pack(1, 2, 3, 255); 320 * 180] } });
        let fx = st.wallpapers_added(&ui, &mut prefs, Rc::new(more), &["photo_fjord".to_string()]);
        assert_eq!(fx, vec![Effect::LockWallpaper("photo_fjord".into())]);
        let View::Wallpaper(page) = &st.stack.last().unwrap().view else { panic!("wallpaper page") };
        assert_eq!(page.len(), 3, "the grid lists the added picture");
        super::super::pages::set_test_form(FormFactor::Tablet);
    }

    #[test]
    fn wallpaper_row_opens_the_grid_and_a_tap_picks_and_persists_a_wallpaper() {
        let ui = super::super::tests::ui();
        let mut prefs = Prefs::memory();
        let mut st = Settings::new(&ui, &Info { wallpaper: "a".into(), ..info() }, &prefs, 1200, 1920, stills());
        st.allow_split(&ui, false); // single-column geometry
        assert!(tap_row(&mut st, &ui, &mut prefs, "Wallpaper").is_empty());
        assert_eq!(st.page(), PageId::Wallpaper);
        let (cx, cy, cw, ch) = super::super::wallpaper_page::cell(&ui, 1200, 9.0 / 16.0, 1);
        assert!(tap(&mut st, &ui, &mut prefs, cx + cw / 2.0, cy + ch / 2.0).is_empty(), "opens the preview");
        let View::Wallpaper(page) = &st.stack.last().unwrap().view else { panic!("wallpaper page") };
        let (bx, by, bw, bh) = page.button(&ui, 1200, 1920, 2);
        assert_eq!(tap(&mut st, &ui, &mut prefs, bx + bw / 2.0, by + bh / 2.0), vec![Effect::LockWallpaper(String::new()), Effect::Wallpaper("b".into())]);
        assert_eq!(prefs.get("wallpaper"), Some("b"));
        assert_eq!(st.info.wallpaper, "b");
        let mut buf = vec![0u32; 1200 * 1920];
        st.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 1e9);
        back(&mut st, &ui, &mut prefs);
        assert_eq!(st.page(), PageId::Root);
        let root = st.list().unwrap();
        let (g, i) = root.find("Wallpaper").unwrap();
        assert_eq!(root.item(g, i).value, "Lake The Lake Night");
        st.refresh(&ui, &Info { wallpaper: "b".into(), ..info() });
        assert_eq!(wallpaper_name(&st.wallpapers, "zzz"), "");
    }

    #[test]
    fn resize_keeps_page_sheet_and_text_and_widens_the_keyboard() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = sheet_open(&ui);
        st.key(&ui, Out::Text("abc".into()), 0.0);
        let before = st.sheet.as_ref().unwrap().keyboard.h();
        st.resize(&ui, 1920, 1200);
        assert_eq!(st.page(), PageId::Wifi);
        assert_eq!(st.sheet.as_ref().unwrap().password(), "abc");
        assert_eq!(st.sheet.as_ref().unwrap().keyboard.w, 1920.0);
        assert!(st.sheet.as_ref().unwrap().keyboard.h() > before);
        let mut buf = vec![0u32; 1920 * 1200];
        st.draw(&mut Surface { w: 1920, h: 1200, stride: 1920, px: &mut buf, y0: 0 }, &ui, 0.5);
        assert!(st.touch(&ui, &mut prefs, Event::Down(1900.0, 600.0), 1.0).is_empty());
        st.resize(&ui, 1200, 1920);
        assert_eq!(st.sheet.as_ref().unwrap().keyboard.h(), before);
    }

    #[test]
    fn refresh_keeps_page_sheet_and_scroll() {
        let ui = super::super::tests::ui();
        let (mut st, prefs) = sheet_open(&ui);
        st.key(&ui, Out::Text("abc".into()), 0.0);
        st.refresh(&ui, &Info { battery: 7, ..wifi_info() });
        assert_eq!(st.page(), PageId::Wifi);
        assert_eq!(st.sheet.as_ref().unwrap().password(), "abc");
        // a short screen makes About scroll
        let mut st = Settings::new(&ui, &wifi_info(), &prefs, 1200, 900, Rc::new(vec![]));
        st.allow_split(&ui, false); // single-column geometry
        let mut prefs2 = Prefs::memory();
        tap_row(&mut st, &ui, &mut prefs2, "General");
        tap_row(&mut st, &ui, &mut prefs2, "About");
        st.touch(&ui, &mut prefs2, Event::Down(600.0, 800.0), 0.0);
        for k in 1..=6 {
            st.touch(&ui, &mut prefs2, Event::Move(600.0, 800.0 - 100.0 * k as f32), k as f64 * 30.0);
        }
        st.touch(&ui, &mut prefs2, Event::Up(600.0, 200.0), 1000.0);
        settle(&mut st, &ui);
        let offset = st.list().unwrap().offset();
        assert!(offset > 0.0);
        st.refresh(&ui, &Info { battery: 9, ..wifi_info() });
        assert_eq!(st.list().unwrap().offset(), offset, "scroll survives a refresh");
        assert_eq!(st.stack(), vec![PageId::Root, PageId::General, PageId::About]);
    }

    #[test]
    fn switches_persist_their_options() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = settings(&ui, &info());
        tap_row(&mut st, &ui, &mut prefs, "Sounds");
        assert!(tap_row(&mut st, &ui, &mut prefs, "Keyboard Clicks").is_empty());
        assert_eq!(prefs.get("keyboard_clicks"), Some("0"));
        tap_row(&mut st, &ui, &mut prefs, "Lock Sound");
        assert_eq!(prefs.get("lock_sound"), Some("0"));
        // Alert sound is a pop-up (the Golden Gate Sound pane): its second choice is Chime
        assert_eq!(pick(&mut st, &ui, &mut prefs, "Alert sound", 1), vec![Effect::AlertSound(1)], "choosing an alert sound plays it once");
        assert_eq!(prefs.get("alert_sound"), Some("1"));
        assert_eq!(st.options().alert_sound, 1);
        back(&mut st, &ui, &mut prefs);
        tap_row(&mut st, &ui, &mut prefs, "General");
        tap_row(&mut st, &ui, &mut prefs, "Date & Time");
        tap_row(&mut st, &ui, &mut prefs, "24-Hour Time");
        assert_eq!(prefs.get("clock_24h"), Some("0"));
        assert!(!st.options().clock_24h);
        back(&mut st, &ui, &mut prefs);
        tap_row(&mut st, &ui, &mut prefs, "Keyboard");
        tap_row(&mut st, &ui, &mut prefs, "Auto-Capitalisation");
        assert_eq!(prefs.get("auto_caps"), Some("0"));
        back(&mut st, &ui, &mut prefs);
        back(&mut st, &ui, &mut prefs);
        tap_row(&mut st, &ui, &mut prefs, "Display & Brightness");
        tap_row(&mut st, &ui, &mut prefs, "Bold Text");
        assert_eq!(prefs.get("bold_text"), Some("1"));
        assert_eq!(Options::load(&prefs), *st.options(), "the file matches the live options");
    }

    #[test]
    fn focus_and_assistive_touch_switches_persist_and_reload() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = settings(&ui, &info());
        tap_row(&mut st, &ui, &mut prefs, "Focus");
        assert_eq!(st.page(), PageId::Focus);
        assert!(tap_row(&mut st, &ui, &mut prefs, "Do Not Disturb").is_empty());
        assert_eq!(prefs.get("dnd"), Some("1"));
        back(&mut st, &ui, &mut prefs);
        let list = st.list().unwrap();
        let (g, i) = list.find("Focus").unwrap();
        assert_eq!(list.item(g, i).value, "On");
        tap_row(&mut st, &ui, &mut prefs, "Accessibility");
        assert!(tap_row(&mut st, &ui, &mut prefs, "AssistiveTouch").is_empty());
        assert_eq!(prefs.get("assistive_touch"), Some("1"));
        let again = Settings::new(&ui, &info(), &prefs, 1200, 1920, Rc::new(vec![]));
        assert!(again.options().dnd);
        assert!(again.options().assistive_touch);
        assert_eq!(Options::load(&prefs), *st.options());
    }

    #[test]
    fn auto_lock_picks_persist_and_update_the_display_page() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = settings(&ui, &info());
        tap_row(&mut st, &ui, &mut prefs, "Display & Brightness");
        tap_row(&mut st, &ui, &mut prefs, "Auto-Lock");
        assert_eq!(st.page(), PageId::AutoLock);
        assert_eq!(tap_row(&mut st, &ui, &mut prefs, "Never"), vec![Effect::AutoLock(0)]);
        assert_eq!(prefs.get("auto_lock_ms"), Some("0"));
        let p = st.list().unwrap();
        assert_eq!(p.item(0, 4).accessory, super::super::list::Accessory::Check(true));
        assert_eq!(p.item(0, 2).accessory, super::super::list::Accessory::Check(false));
        back(&mut st, &ui, &mut prefs);
        let d = st.list().unwrap();
        let (g, i) = d.find("Auto-Lock").unwrap();
        assert_eq!(d.item(g, i).value, "Never");
        assert_eq!(tap_row(&mut st, &ui, &mut prefs, "Auto-Lock"), vec![]);
        assert_eq!(tap_row(&mut st, &ui, &mut prefs, "30 seconds"), vec![Effect::AutoLock(30_000)]);
    }

    #[test]
    fn require_password_picks_persist_and_update_the_lock_screen_row() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = settings(&ui, &info());
        tap_row(&mut st, &ui, &mut prefs, "Lock Screen");
        tap_row(&mut st, &ui, &mut prefs, "Require Password");
        assert_eq!(st.page(), PageId::RequirePassword);
        assert_eq!(tap_row(&mut st, &ui, &mut prefs, "1 minute"), vec![]);
        assert_eq!(prefs.get("lock_grace_seconds"), Some("60"));
        assert_eq!(st.options().lock_grace_s, 60);
        assert_eq!(tap_row(&mut st, &ui, &mut prefs, "Immediately"), vec![]);
        assert_eq!(prefs.get("lock_grace_seconds"), Some("0"));
    }

    #[test]
    fn energy_picks_persist_and_apply_without_restart() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = settings(&ui, &info());
        tap_row(&mut st, &ui, &mut prefs, "Battery");
        tap_row(&mut st, &ui, &mut prefs, "Energy");
        assert_eq!(st.page(), PageId::Energy);
        assert_eq!(pick(&mut st, &ui, &mut prefs, "Sleep After", 3), vec![Effect::SystemSleepAfter(15)]);
        assert_eq!(prefs.get("system_sleep_minutes"), Some("15"));
        assert_eq!(crate::bare::sleep::system_sleep_after(prefs.get("system_sleep_minutes")), Some(900_000.0));
        assert_eq!(pick(&mut st, &ui, &mut prefs, "Sleep After", 6), vec![Effect::SystemSleepAfter(0)]);
        assert_eq!(crate::bare::sleep::system_sleep_after(prefs.get("system_sleep_minutes")), None);
    }

    #[test]
    fn privacy_offers_open_anyway_for_a_blocked_program_and_the_allow_setting() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = settings(&ui, &info());
        let dir = std::env::temp_dir().join(format!("settings-launch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let prog = dir.join("tool");
        std::fs::write(&prog, "#!/bin/sh\n").unwrap();
        st.set_data_dir(&dir);
        tap_row(&mut st, &ui, &mut prefs, "Privacy");
        assert!(st.row_centre(&ui, "Open Anyway").is_none(), "nothing waits");
        assert!(st.row_centre(&ui, "Allow Apps From").is_some());
        back(&mut st, &ui, &mut prefs);
        super::super::launch_gate::write_blocked(&dir, &prog, "\"tool\" cannot be opened because its developer cannot be identified.");
        tap_row(&mut st, &ui, &mut prefs, "Privacy");
        assert_eq!(tap_row(&mut st, &ui, &mut prefs, "Open Anyway"), vec![]);
        assert!(st.row_centre(&ui, "Open Anyway").is_none(), "the block is lifted");
        let remembered = std::fs::read_to_string(dir.join("launch_overrides.txt")).unwrap();
        assert!(remembered.starts_with(&format!("{}\t", prog.display())), "{remembered:?}");
        // the setting persists and the check follows the tap
        assert_eq!(pick(&mut st, &ui, &mut prefs, "Allow Apps From", 2), vec![]);
        assert_eq!(prefs.get("allow_apps_from"), Some("anywhere"));
        assert_eq!(super::super::launch_gate::load_policy(&prefs, &dir).setting(), bifrost_core::launch_policy::Setting::Anywhere);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Probes that answer from a table instead of the network.
    struct FakeProbe {
        resolves: bool,
    }

    impl bifrost_core::net_probe::Probe for FakeProbe {
        fn link_states(&self) -> Vec<(String, String)> {
            vec![("lo".into(), "unknown".into()), ("wlan0".into(), "up".into())]
        }
        fn global_addresses(&self) -> String {
            "3: wlan0    inet 192.168.1.20/24 brd 192.168.1.255 scope global dynamic wlan0\n".into()
        }
        fn resolve(&self, _host: &str, port: u16) -> Vec<std::net::SocketAddr> {
            if self.resolves { vec![std::net::SocketAddr::from(([192, 168, 1, 1], port))] } else { Vec::new() }
        }
        fn connect(&self, _addr: std::net::SocketAddr) -> bool {
            true
        }
    }

    #[test]
    fn network_diagnose_runs_the_probe_off_the_draw_thread_and_shows_the_diagnosis() {
        let ui = super::super::tests::ui();
        // the Network row is the desktop's own network page, so the snapshot has to be there
        let info = Info { desktop_network: Some(super::super::desktop_network::Snapshot::default()), ..info() };
        let (mut st, mut prefs) = settings(&ui, &info);
        st.net_probe = std::sync::Arc::new(FakeProbe { resolves: false });
        tap_row(&mut st, &ui, &mut prefs, "Network");
        assert_eq!(st.page(), PageId::Network);
        assert!(st.row_centre(&ui, "Diagnose").is_some());
        assert!(st.row_centre(&ui, "Name Lookup").is_none(), "no result before a run");
        // tap without settling: the run is on its worker, the page says so
        let (x, y) = st.row_centre(&ui, "Diagnose").unwrap();
        assert_eq!(tap(&mut st, &ui, &mut prefs, x, y), vec![]);
        assert!(st.loads.net_diag.busy() || st.loads.net_diag.get().is_some());
        settle(&mut st, &ui);
        let report = st.loads.net_diag.get().and_then(|r| r.as_ref()).expect("the run landed");
        assert_eq!(report.diagnosis.stage, bifrost_core::net_diagnostic::Stage::Name);
        // the steps and the diagnosis are on the page
        assert!(st.row_centre(&ui, "Name Lookup").is_some());
        assert!(st.row_centre(&ui, "Connection").is_some());
        assert!(st.row_centre(&ui, report.diagnosis.title).is_some());
    }

    #[test]
    fn battery_schedule_adds_and_edits_times_and_arms_the_wake() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = settings(&ui, &info());
        tap_row(&mut st, &ui, &mut prefs, "Battery");
        tap_row(&mut st, &ui, &mut prefs, "Energy");
        tap_row(&mut st, &ui, &mut prefs, "Schedule");
        assert_eq!(st.page(), PageId::WakeSchedule);
        let fx = tap_row(&mut st, &ui, &mut prefs, "Add Time");
        assert!(matches!(fx.as_slice(), [Effect::WakeSchedule(at)] if *at > 0), "{fx:?}");
        assert_eq!(prefs.get("wake_schedule"), Some("31,420,wake,1"));
        tap_row(&mut st, &ui, &mut prefs, "Repeat");
        assert_eq!(prefs.get("wake_schedule"), Some("96,420,wake,1"));
        tap_row(&mut st, &ui, &mut prefs, "Action");
        assert_eq!(prefs.get("wake_schedule"), Some("96,420,poweron,1"));
        // the time row opens the time-of-day picker; its answer moves the entry and arms the clock again
        assert_eq!(tap_row(&mut st, &ui, &mut prefs, "Time"), vec![]);
        assert!(st.time_picker.is_some());
        let fx = st.wake_set_minute(&ui, &mut prefs, 0, 8 * 60);
        assert!(matches!(fx.as_slice(), [Effect::WakeSchedule(at)] if *at > 0), "{fx:?}");
        assert_eq!(prefs.get("wake_schedule"), Some("96,480,poweron,1"));
        st.time_picker = None;
        // a fresh Settings reads it back and the Battery page shows it
        let reloaded = Settings::new(&ui, &info(), &prefs, 1200, 1920, Rc::new(vec![]));
        assert_eq!(super::super::wake_page::summary(&reloaded.wake), "1 time");
        // off disarms (0), and Remove deletes the entry
        let fx = tap_row(&mut st, &ui, &mut prefs, "Enabled");
        assert_eq!(fx, vec![Effect::WakeSchedule(0)]);
        let fx = tap_row(&mut st, &ui, &mut prefs, "Remove");
        assert_eq!(fx, vec![Effect::WakeSchedule(0)]);
        assert_eq!(prefs.get("wake_schedule"), Some(""));
    }

    #[test]
    fn screen_saver_picks_persist_and_preview_is_queued_once() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = settings(&ui, &info());
        tap_row(&mut st, &ui, &mut prefs, "Screen Saver");
        assert_eq!(st.page(), PageId::ScreenSaver);
        assert_eq!(pick(&mut st, &ui, &mut prefs, "Screen Saver", 1), vec![]);
        assert_eq!(prefs.get("screensaver_module"), Some("1"));
        let ten = super::super::screensaver::START_AFTER.iter().position(|(_, min)| *min == 10).unwrap();
        assert_eq!(pick(&mut st, &ui, &mut prefs, "Start After", ten), vec![]);
        assert_eq!(prefs.get("screensaver_minutes"), Some("10"));
        assert!(!st.take_screensaver_preview());
        tap_row(&mut st, &ui, &mut prefs, "Preview");
        assert!(st.take_screensaver_preview(), "Preview asks the shell to start the saver");
        assert!(!st.take_screensaver_preview(), "the request is taken once");
    }

    #[test]
    fn screen_saver_options_are_saved_per_saver() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = settings(&ui, &info());
        tap_row(&mut st, &ui, &mut prefs, "Screen Saver");
        // Clock is the first saver: its switch and its choices are on the page
        tap_row(&mut st, &ui, &mut prefs, "Show Date");
        assert_eq!(prefs.get("ScreenSaver.Clock.show_date"), Some("false"));
        pick(&mut st, &ui, &mut prefs, "Move Every", 0);
        assert_eq!(prefs.get("ScreenSaver.Clock.move_every"), Some("30s"));
        assert_eq!(prefs.get("ScreenSaver.Colours.speed"), None, "another saver's keys are untouched");
        // the next saver shows its own options
        pick(&mut st, &ui, &mut prefs, "Screen Saver", 1);
        pick(&mut st, &ui, &mut prefs, "Colour Change", 2);
        assert_eq!(prefs.get("ScreenSaver.Colours.speed"), Some("fast"));
        assert_eq!(prefs.get("ScreenSaver.Clock.move_every"), Some("30s"));
    }

    fn drag_slider(st: &mut Settings, ui: &Ui, prefs: &mut Prefs, label_row_above: Option<&str>, g: usize, i: usize, from: f32, to: f32) -> Vec<Effect> {
        let _ = label_row_above;
        let (tx, ty, tw) = st.list().unwrap().track(ui, g, i);
        let mut fx = st.touch(ui, prefs, Event::Down(tx + tw * from, ty), 0.0);
        fx.extend(st.touch(ui, prefs, Event::Move(tx + tw * to, ty + 200.0), 50.0));
        fx.extend(st.touch(ui, prefs, Event::Up(tx + tw * to, ty + 200.0), 100.0));
        fx
    }

    #[test]
    fn sliders_emit_live_effects_and_persist_on_release() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = settings(&ui, &info());
        tap_row(&mut st, &ui, &mut prefs, "Display & Brightness");
        let fx = drag_slider(&mut st, &ui, &mut prefs, None, 0, 0, 0.25, 0.75);
        assert_eq!(fx, vec![Effect::Brightness(0.25), Effect::Brightness(0.75), Effect::Brightness(0.75)]);
        assert_eq!(st.brightness(), 0.75);
        assert_eq!(st.page(), PageId::Display, "dragging never taps");
        // text size snaps to the seven notches and persists on release
        let (g, i) = st.list().unwrap().find("Bold Text").map(|(g, i)| (g, i - 1)).unwrap();
        assert!(drag_slider(&mut st, &ui, &mut prefs, None, g, i, 0.5, 1.0).is_empty());
        assert_eq!(st.options().text_scale, TEXT_SIZES[6]);
        assert_eq!(prefs.get("text_scale"), Some("1.30"));
        drag_slider(&mut st, &ui, &mut prefs, None, g, i, 1.0, 0.0);
        assert_eq!(st.options().text_scale, TEXT_SIZES[0]);
        back(&mut st, &ui, &mut prefs);
        tap_row(&mut st, &ui, &mut prefs, "Sounds");
        // the Output volume slider sits under the output devices
        let g = st.list().unwrap().spec().groups.iter().position(|g| g.header.as_deref() == Some("OUTPUT VOLUME")).expect("Output volume");
        let fx = drag_slider(&mut st, &ui, &mut prefs, None, g, 0, 0.5, 0.0);
        assert_eq!(fx.last(), Some(&Effect::Volume(0.0)));
        assert_eq!(prefs.get("volume"), Some("0.00"));
    }

    #[test]
    fn the_restart_alert_offers_reopen_windows_and_remembers_the_choice() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = settings(&ui, &wifi_info());
        tap_row(&mut st, &ui, &mut prefs, "General");
        tap_row(&mut st, &ui, &mut prefs, "Restart");
        assert_eq!(st.alert().unwrap().reopen(), Some(true), "on by default");
        let (cx, cy) = st.alert().unwrap().reopen_centre(&ui, 1200, 1920).expect("a checkbox row");
        assert!(tap(&mut st, &ui, &mut prefs, cx, cy).is_empty(), "the checkbox does not confirm");
        assert_eq!(st.alert().unwrap().reopen(), Some(false));
        assert_eq!(prefs.get(super::super::session::REOPEN_PREF), Some("0"));
        let (cx, cy) = st.alert().unwrap().button(&ui, 1200, 1920, AlertHit::Confirm);
        assert_eq!(tap(&mut st, &ui, &mut prefs, cx, cy), vec![Effect::Power(PowerAction::Restart)]);
    }

    #[test]
    fn close_windows_when_quitting_is_a_general_setting() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = settings(&ui, &wifi_info());
        tap_row(&mut st, &ui, &mut prefs, "General");
        assert_eq!(prefs.get(super::super::session::CLOSE_ON_QUIT_PREF), None);
        tap_row(&mut st, &ui, &mut prefs, "Close windows when quitting an application");
        assert_eq!(prefs.get(super::super::session::CLOSE_ON_QUIT_PREF), Some("1"));
        tap_row(&mut st, &ui, &mut prefs, "Close windows when quitting an application");
        assert_eq!(prefs.get(super::super::session::CLOSE_ON_QUIT_PREF), Some("0"));
    }

    #[test]
    fn forget_network_from_the_detail_page() {
        let ui = super::super::tests::ui();
        let (mut st, mut prefs) = settings(&ui, &wifi_info());
        tap_row(&mut st, &ui, &mut prefs, "Wi-Fi");
        tap_row(&mut st, &ui, &mut prefs, "Home");
        assert_eq!(st.page(), PageId::WifiDetail);
        assert_eq!(st.list().unwrap().spec().title, "Home");
        assert_eq!(tap_row(&mut st, &ui, &mut prefs, "Auto-Join"), vec![Effect::Wifi(WifiAction::SetAutoJoin { ssid: "Home".into(), on: false })]);
        tap_row(&mut st, &ui, &mut prefs, "Forget This Network");
        let (cx, cy) = st.alert().unwrap().button(&ui, 1200, 1920, AlertHit::Confirm);
        assert_eq!(tap(&mut st, &ui, &mut prefs, cx, cy), vec![Effect::Wifi(WifiAction::Forget("Home".into()))]);
        settle(&mut st, &ui);
        assert_eq!(st.page(), PageId::Wifi, "back to the list");
    }
}

/// Behind-window vibrancy for the sidebar column: on the GPU presenter (and without Reduce Transparency) the desk
/// declares the column a `.sidebar` material and the presenter blurs the desktop under it.
fn vibrant() -> bool {
    super::apphost::vibrancy_available()
}

/// Paints the sidebar column: the premultiplied translucent `MATERIAL_SIDEBAR` tint written over the pixels (not
/// blended, so the cached column and the frame keep its alpha for the presenter) with vibrancy, else opaque
/// `SIDEBAR_BG`.
fn sidebar_fill(s: &mut Surface, x: usize, y: usize, w: usize, h: usize) {
    let c = if vibrant() {
        let c = super::ui::MATERIAL_SIDEBAR.now();
        bifrost_render::gfx::rgba((c & 255) as u8, (c >> 8 & 255) as u8, (c >> 16 & 255) as u8, (c >> 24) as u8)
    } else {
        SIDEBAR_BG.now()
    };
    s.fill_rect(x, y, w, h, c);
}

/// Runs `read` on a worker thread; the result arrives on the returned channel, so the UI thread never waits on it.
fn read_on_worker<T: Send + 'static>(name: &str, read: impl FnOnce() -> T + Send + 'static) -> std::sync::mpsc::Receiver<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    crate::bare::sys::spawn_named(name, move || {
        let _ = tx.send(read());
    });
    rx
}

#[cfg(test)]
mod read_on_worker_tests {
    use super::read_on_worker;

    #[test]
    fn a_read_runs_off_the_caller_and_delivers_its_value() {
        let caller = std::thread::current().id();
        let rx = read_on_worker("test-read", move || (7usize, std::thread::current().id()));
        let (value, worker) = rx.recv().expect("the worker sends its result");
        assert_eq!(value, 7);
        assert_ne!(worker, caller, "the read must not run on the calling thread");
    }
}

/// E2E wave 3 (Settings): navigation history, Escape, the Sound page, search and the sidebar order.
#[cfg(test)]
mod e2e_w3_tests {
    use super::*;
    use bifrost_ui::prefs_layout::m;

    fn info() -> Info {
        Info { battery: 42, wifi: "Home".into(), wifi_on: true, model: "Midget".into(), brightness: 0.5, ..Info::default() }
    }

    /// The desktop split layout (sidebar + detail).
    fn split(ui: &Ui) -> (Settings, Prefs) {
        let st = Settings::new(ui, &info(), &Prefs::memory(), 1200, 1920, Rc::new(vec![]));
        assert!(st.sidebar_list().is_some(), "1200 px at this scale is wide enough for the split layout");
        (st, Prefs::memory())
    }

    fn settle(st: &mut Settings, ui: &Ui) {
        st.tick(ui, 1e9);
    }

    fn tap(st: &mut Settings, ui: &Ui, prefs: &mut Prefs, x: f32, y: f32) -> Vec<Effect> {
        let mut fx = st.touch(ui, prefs, Event::Down(x, y), 0.0);
        fx.extend(st.touch(ui, prefs, Event::Up(x, y), 10.0));
        settle(st, ui);
        fx
    }

    /// The centre of the back (`false`) or forward (`true`) half of the toolbar pill, in window px.
    fn pill(st: &Settings, ui: &Ui, forward: bool) -> (f32, f32) {
        let k = if forward { 0.75 } else { 0.25 };
        (st.pane_x() + ui.pt(m::TOOLBAR_X + m::TOOLBAR_PILL_W * k), ui.pt(m::TOOLBAR_Y + m::TOOLBAR_PILL_H / 2.0))
    }

    /// W3-3: a page opened in the split layout (a deep link, a Search hit) had no back target, so the pill and Esc did
    /// nothing; it now goes on its pane, and the pill walks back and forward through the history.
    #[test]
    fn the_split_layout_pill_goes_back_and_forward() {
        let ui = crate::shell::tests::ui();
        let (mut st, mut prefs) = split(&ui);
        st.open_page(&ui, "About", 0.0);
        settle(&mut st, &ui);
        assert_eq!(st.stack(), vec![PageId::General, PageId::About], "About goes on its pane");
        let nav = st.list().expect("list").nav();
        assert!(nav.has_back() && !nav.has_forward(), "only the back half is enabled");
        let (bx, by) = pill(&st, &ui, false);
        tap(&mut st, &ui, &mut prefs, bx, by);
        assert_eq!(st.stack(), vec![PageId::General], "the back half pops About");
        assert!(st.can_go_forward() && st.list().expect("list").nav().has_forward(), "About is ahead now");
        assert!(!st.list().expect("list").nav().has_back(), "General is the first page: nothing behind it");
        let (fx, fy) = pill(&st, &ui, true);
        tap(&mut st, &ui, &mut prefs, fx, fy);
        assert_eq!(st.stack(), vec![PageId::General, PageId::About], "the forward half opens About again");
        assert!(!st.can_go_forward());
        // a tap on the dimmed forward half does nothing
        tap(&mut st, &ui, &mut prefs, fx, fy);
        assert_eq!(st.page(), PageId::About);
    }

    /// W3-3: Esc pops a sub-page in the split layout, and is not used at the pane (so the shell leaves it alone).
    #[test]
    fn escape_pops_a_split_sub_page_and_stops_at_its_pane() {
        let ui = crate::shell::tests::ui();
        let (mut st, _) = split(&ui);
        st.open_page(&ui, "Software Update", 0.0);
        settle(&mut st, &ui);
        assert_eq!(st.stack(), vec![PageId::General, PageId::SoftwareUpdate]);
        assert!(st.back_page(0.0));
        settle(&mut st, &ui);
        assert_eq!(st.stack(), vec![PageId::General]);
        assert!(!st.back_page(0.0), "nothing to pop at the pane");
    }

    /// A new navigation (a sidebar row, a deep link) clears what was ahead.
    #[test]
    fn a_new_page_clears_the_forward_history() {
        let ui = crate::shell::tests::ui();
        let (mut st, _) = split(&ui);
        st.open_page(&ui, "About", 0.0);
        settle(&mut st, &ui);
        st.back_page(0.0);
        settle(&mut st, &ui);
        assert!(st.can_go_forward());
        st.open_page(&ui, "Bluetooth", 0.0);
        settle(&mut st, &ui);
        assert!(!st.can_go_forward());
        assert!(!st.forward_page(&ui, 0.0));
    }

    /// The single column stacks the same way, on the root.
    #[test]
    fn the_single_column_stacks_a_deep_link_on_its_pane() {
        let ui = crate::shell::tests::ui();
        let (mut st, _) = split(&ui);
        st.allow_split(&ui, false);
        st.open_page(&ui, "About", 0.0);
        settle(&mut st, &ui);
        assert_eq!(st.stack(), vec![PageId::Root, PageId::General, PageId::About]);
    }

    /// W3-13: with a pointer a click on the empty middle of a switch row did flip the switch; now only the switch or its
    /// label does. A finger (the tablet form) still flips it from anywhere on the row.
    #[test]
    fn a_pointer_flips_a_switch_only_on_the_switch_or_its_label() {
        use bifrost_ui::form_factor::FormFactor;
        let ui = crate::shell::tests::ui();
        let (mut st, mut prefs) = split(&ui);
        st.open_page(&ui, "Focus", 0.0);
        settle(&mut st, &ui);
        let (g, i) = st.list().expect("list").find("Do Not Disturb").expect("the switch row");
        let (rx, ry, rw, rh) = st.list().expect("list").row_rect(&ui, g, i);
        let (px, mid) = (st.pane_x(), ry + rh / 2.0);
        let middle = px + rx + rw * 0.55;
        let switch = px + rx + rw - ui.pt(16.0 + 19.0);
        let before = st.options().dnd;
        super::super::pages::set_test_form(FormFactor::Desktop);
        tap(&mut st, &ui, &mut prefs, middle, mid);
        assert_eq!(st.options().dnd, before, "a click on the row's empty middle does nothing");
        tap(&mut st, &ui, &mut prefs, switch, mid);
        assert_eq!(st.options().dnd, !before, "a click on the switch flips it");
        super::super::pages::set_test_form(FormFactor::Tablet);
        tap(&mut st, &ui, &mut prefs, middle, mid);
        assert_eq!(st.options().dnd, before, "a finger flips it from anywhere on the row");
    }

    /// N3: Return opened the first match even when another hit was highlighted (opened by Down or a tap).
    #[test]
    fn return_in_search_opens_the_highlighted_hit() {
        let ui = crate::shell::tests::ui();
        let (mut st, _) = split(&ui);
        st.search.active = true;
        st.key(&ui, Out::Text("display".into()), 0.0);
        let hits = st.hit_pages();
        let first = *hits.first().expect("a match");
        let second = *hits.iter().find(|p| **p != first).expect("hits on two pages");
        st.open_page_id(&ui, first, 0.0);
        settle(&mut st, &ui);
        st.key(&ui, Out::Named("Down"), 0.0);
        settle(&mut st, &ui);
        assert_eq!(st.page(), second, "Down opens the next hit");
        assert_eq!(st.side_selected(), second, "and the sidebar fills it");
        st.key(&ui, Out::Enter, 0.0);
        settle(&mut st, &ui);
        assert_eq!(st.page(), second, "Return opens the highlighted hit, not the first");
        st.key(&ui, Out::Named("Up"), 0.0);
        settle(&mut st, &ui);
        assert_eq!(st.page(), first, "Up goes back");
    }

    /// N17: the wheel scrolls the list under the pointer itself (the shell then drops the drag emulated for it, which
    /// pressed and saved the slider under the pointer).
    #[test]
    fn the_wheel_scrolls_the_list_under_the_pointer() {
        let ui = crate::shell::tests::ui();
        let (mut st, _) = split(&ui);
        st.set_size(&ui, 1200, 700);
        assert!(st.wheel(10.0, -40.0), "the sidebar takes the wheel");
        assert_eq!(st.sidebar_list().expect("split").offset(), 40.0);
        st.open_page_id(&ui, PageId::Appearance, 0.0);
        settle(&mut st, &ui);
        let before = st.list().expect("list").offset();
        assert!(st.wheel(st.sidebar_px() as f32 + 10.0, -40.0), "the page takes the wheel");
        assert!(st.list().expect("list").offset() > before, "and scrolls down");
        assert_eq!(st.sidebar_list().expect("split").offset(), 40.0, "the sidebar stays");
    }

    /// N4: a window resize rebuilt the sidebar from its top.
    #[test]
    fn a_resize_keeps_the_sidebar_scroll() {
        let ui = crate::shell::tests::ui();
        let (mut st, _) = split(&ui);
        st.set_size(&ui, 1200, 700);
        st.sidebar.as_mut().expect("split").jump_to(150.0);
        let before = st.sidebar_list().expect("split").offset();
        assert!(before > 0.0, "the sidebar scrolls at this height");
        st.set_size(&ui, 1210, 680);
        assert_eq!(st.sidebar_list().expect("split").offset(), before);
    }

    /// N12: a new Sidebar icon size moved the sidebar rows only at the next relayout.
    #[test]
    fn the_sidebar_icon_size_applies_at_once() {
        let ui = crate::shell::tests::ui();
        let keep = bifrost_render::ui::sidebar_icon_size();
        bifrost_render::ui::set_sidebar_icon_size(1);
        let (mut st, mut prefs) = split(&ui);
        st.open_page_id(&ui, PageId::Appearance, 0.0);
        settle(&mut st, &ui);
        let (_, before) = st.sidebar_centre(&ui, "Appearance").expect("the Appearance row");
        let (x, y) = st.row_centre(&ui, "Sidebar icon size").expect("the size row");
        tap(&mut st, &ui, &mut prefs, x, y);
        st.activate(&ui, &mut prefs, Hit::Choice(2), 0.0);
        settle(&mut st, &ui);
        let (_, after) = st.sidebar_centre(&ui, "Appearance").expect("the Appearance row");
        bifrost_render::ui::set_sidebar_icon_size(keep);
        assert!(after > before, "Large rows move the row down at once: {before} -> {after}");
    }

    #[test]
    fn arrow_keys_step_over_the_hits_of_the_highlighted_page() {
        let hits = [PageId::Display, PageId::Display, PageId::NightShift, PageId::About];
        assert_eq!(step_hit(&hits, None, true), Some(PageId::Display));
        assert_eq!(step_hit(&hits, None, false), Some(PageId::About));
        assert_eq!(step_hit(&hits, Some(0), true), Some(PageId::NightShift));
        assert_eq!(step_hit(&hits, Some(2), false), Some(PageId::Display));
        assert_eq!(step_hit(&hits, Some(3), true), None);
        assert_eq!(step_hit(&hits, Some(0), false), None);
    }

    /// W3-11: Return in the search field did nothing; it opens the first match.
    #[test]
    fn return_in_search_opens_the_first_match() {
        let ui = crate::shell::tests::ui();
        let (mut st, _) = split(&ui);
        st.search.active = true;
        st.key(&ui, Out::Text("about".into()), 0.0);
        let first = settings_search::first_hit(st.index.as_deref().unwrap_or(&[]), "about").expect("a match");
        st.key(&ui, Out::Enter, 0.0);
        settle(&mut st, &ui);
        assert_eq!(st.page(), first, "Return opened the first match");
        assert_eq!(first, PageId::About);
    }

    /// The Output volume slider's value on the open Sound page.
    fn sound_slider(st: &Settings) -> f32 {
        let spec = st.list().expect("list").spec();
        let g = spec.groups.iter().find(|g| g.header.as_deref() == Some("OUTPUT VOLUME")).expect("the output volume group");
        match g.items[0].body {
            super::super::list::Body::Slider { value, .. } => value,
            _ => panic!("not a slider"),
        }
    }

    /// W3-9: Sound showed the saved "volume" pref (0) while the system played at 0.644; it shows the live level, and
    /// follows it while open.
    #[test]
    fn sound_shows_and_follows_the_live_output_level() {
        let ui = crate::shell::tests::ui();
        let (mut st, _) = split(&ui);
        st.set_audio(&ui, 0.644, false);
        st.open_page(&ui, "Sound", 0.0);
        settle(&mut st, &ui);
        assert_eq!(st.page(), PageId::Sounds);
        assert!((sound_slider(&st) - 0.644).abs() < 1e-4, "the live level");
        st.set_audio(&ui, 0.25, false);
        assert!((sound_slider(&st) - 0.25).abs() < 1e-4, "a level set by the keys or Control Center shows at once");
        let spec = st.list().expect("list").spec();
        assert!(spec.groups.iter().any(|g| g.header.as_deref() == Some("OUTPUT") && !g.items.is_empty()), "the output devices are listed");
        assert!(spec.groups[0].items.iter().any(|i| i.label == "Play sound effects through"));
    }

    /// W3-12: the desktop sidebar was one flat list; it follows the Golden Gate groups, with Appearance and Keyboard as
    /// panes of their own.
    #[test]
    fn the_desktop_sidebar_follows_the_golden_gate_groups() {
        let ui = crate::shell::tests::ui();
        let desk = Info { display_mode: Some((2560, 1440, 60)), ..info() };
        let mut st = Settings::new(&ui, &desk, &Prefs::memory(), 1200, 1920, Rc::new(vec![]));
        let groups: Vec<Vec<String>> = st.sidebar_list().expect("split").spec().groups.iter().map(|g| g.items.iter().map(|i| i.label.clone()).collect()).collect();
        assert_eq!(groups[0], ["BifrOSt Account"]);
        assert_eq!(groups[1][..2], ["Wi-Fi", "Bluetooth"], "connections first; no Airplane Mode on a desktop");
        assert_eq!(groups[2], ["General", "Sharing", "Accessibility", "Appearance", "Desktop & Dock", "Overview", "Displays", "Menu Bar", "Control Center", "Assistant", "Search", "Wallpaper", "Screen Saver"]);
        assert_eq!(groups[3], ["Notifications", "Sound", "Focus", "Screen Time"]);
        assert_eq!(groups[4][..2], ["Lock Screen", "Privacy & Security"]);
        assert_eq!(groups[4].last().map(String::as_str), Some("Users & Groups"));
        assert_eq!(groups[5], ["Passwords", "Internet Accounts"]);
        assert_eq!(groups[6], ["Keyboard", "Keyboard Shortcuts", "Mouse", "Trackpad", "Printers & Scanners"]);
        assert_eq!(groups.last().expect("groups"), &["Developer"]);
        // Appearance and Keyboard open as first-level panes (their sidebar row selected), not under Displays or General
        st.open_page(&ui, "Appearance", 0.0);
        settle(&mut st, &ui);
        assert_eq!(st.stack(), vec![PageId::Appearance]);
        st.open_page(&ui, "Keyboard", 0.0);
        settle(&mut st, &ui);
        assert_eq!(st.stack(), vec![PageId::Keyboard]);
        // the tablet's root list keeps its own order
        let tablet = Settings::new(&ui, &info(), &Prefs::memory(), 1200, 1920, Rc::new(vec![]));
        let first = tablet.sidebar_list().expect("split").spec().groups[1].items[0].label.clone();
        assert_eq!(first, "Airplane Mode");
    }

    /// W3-3: a Search result in the sidebar that is a page inside a pane (Night Shift) opens on its pane, with a back
    /// target, instead of as a first page with none.
    #[test]
    fn a_sidebar_search_hit_inside_a_pane_has_a_back_target() {
        let ui = crate::shell::tests::ui();
        let (mut st, mut prefs) = split(&ui);
        st.search.active = true;
        st.key(&ui, Out::Text("night shift".into()), 0.0);
        let label = st.sidebar_list().expect("split").spec().groups[0].items[0].label.clone();
        let want = settings_search::first_hit(st.index.as_deref().unwrap_or(&[]), "night shift").expect("a match");
        assert_eq!(want, PageId::NightShift);
        let (x, y) = st.sidebar_centre(&ui, &label).expect("the result row");
        tap(&mut st, &ui, &mut prefs, x, y);
        assert_eq!(st.page(), PageId::NightShift);
        assert!(st.stack().len() >= 2, "Night Shift sits on its pane: {:?}", st.stack());
        assert!(st.list().expect("list").nav().has_back());
        assert!(st.back_page(0.0), "Esc goes back to the pane");
    }
    #[test]
    fn dock_changes_from_a_filled_settings_window_use_the_output_size() {
        use crate::shell::{AppId, Event, desktop_dock::{Change, Position}, desk::Desk};
        let mut sh = crate::shell::tests::desktop_shell();
        sh.desk_open(AppId::Settings, 1000.0);
        let m = sh.desk_metrics();
        let index = sh.desk.as_ref().unwrap().index_of(AppId::Settings).unwrap();
        sh.desk.as_mut().unwrap().toggle_zoom(&m, index);
        for change in [Change::Position(Position::Left), Change::Position(Position::Right), Change::Position(Position::Bottom), Change::Autohide] {
            let m = sh.desk_metrics();
            let body = sh.desk.as_ref().unwrap().wins[index].body(&m);
            // This queue is consumed during a real Settings content event, inside with_size before the fix.
            sh.settings.as_mut().unwrap().dock_out.push(change);
            sh.desk_event(Event::Down((body.x + body.w / 2) as f32, (body.y + body.h / 2) as f32), 2000.0);
            let expected = Desk::area(&sh.desk_metrics());
            assert_eq!(sh.desk.as_ref().unwrap().wins[index].r, expected);
        }
    }

}
