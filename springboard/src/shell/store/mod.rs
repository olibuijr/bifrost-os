//! The App Store: browse, product pages, GET -> progress ring -> OPEN, and an
//! Updates tab, laid out like the bifrost 14 App Store (Today / Apps / Games /
//! Updates / Search tab bar (Search has a query field), large titles, rows with icon, name, summary and a
//! capsule button in the tint colour). The catalogue comes from
//! `bifrost.olibuijr.com` on worker threads (cached for offline); downloads
//! are verified and installed by `install`. The UI thread only reads memory.

pub mod api;
pub mod coordination;
pub mod install;
pub mod json;
pub mod net;
/// In-app products listed from the catalogue, with price, ownership and what the store can offer.
pub mod products;
mod review_sheet;
mod sheet;

use super::cache::TextCache;
use super::glyphs::{arc, disc, paint, ring, segment};
use super::ui::{home_zone, Ui, Weight, CARD_RADIUS_PT, CAPSULE_RADIUS, SECONDARY, STORY_HUES, TINT, WHITE};
use super::ui::{CONTENT_BG, CARD_BG, SEPARATOR_C, LABEL, FIELD_BG, MATERIAL_HUD_SOLID, MATERIAL_TITLEBAR_SOLID};
use super::keyboard::Out;
use super::Event;
use crate::deildu::jobs::Pool;
use crate::deildu::scroll::Scroller;
use crate::gfx::{pack, rgba, squircle_mask, Image, Surface};
use crate::scene::STATUS_PT;
use api::StoreApp;
use bifrost_ui::entitlement::Ledger;
use bifrost_ui::purchase::Outcome;
use review_sheet::{ReviewOut, ReviewSheet};
use sheet::{sheet_buttons, Sheet, SheetOut};
use install::InstalledApp;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

const MARGIN: f32 = 20.0;
const NAV_BOTTOM: f32 = STATUS_PT + 44.0;
const TAB_BAR: f32 = 49.0;
const ROW: f32 = 88.0;
const ICON: f32 = 60.0;
const BTN_W: f32 = 76.0;
const BTN_H: f32 = 30.0;
const REFRESH_MS: f64 = 10.0 * 60_000.0;
const RETRY_MS: f64 = 30_000.0;
/// An icon that failed to load is tried again after this long.
const ICON_RETRY_MS: f64 = 60_000.0;
/// Download progress rings redraw at most this often (4 Hz).
const PROGRESS_MS: f64 = 250.0;
/// A product page's screenshot strip: the picture height, and the strip with its gaps (pt).
const SHOT_H: f32 = 260.0;
const SHOTS_PT: f32 = 276.0;

/// True when an icon that failed at `failed_at` (None: never) may be fetched at `t`.
fn icon_due(failed_at: Option<f64>, t: f64) -> bool {
    failed_at.is_none_or(|at| t - at >= ICON_RETRY_MS)
}

/// Where the store keeps an app's icon once it has loaded: `<root>/icon-cache/<id>.png`, so a fetch that fails
/// later (no network yet, a slow server) still shows the real icon instead of a letter tile.
fn icon_cache_path(root: &std::path::Path, id: &str) -> PathBuf {
    let safe: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).collect();
    root.join("icon-cache").join(format!("{safe}.png"))
}

/// An app's icon from disk: the installed copy first, then the copy cached from an earlier download, with `true` when
/// it came from the cache (which the catalogue may have changed since). Read and decoded on a store worker ahead of
/// any download, never on the UI thread, so the icons appear a moment after opening instead of letter tiles while the
/// network catches up (E2E11 9). Each source must decode before it counts, so a damaged installed icon falls through
/// to the cache.
fn local_icon(local: Option<&std::path::Path>, cache: &std::path::Path) -> Option<(Image, bool)> {
    let decode = |b: &[u8]| crate::cg::imageio::decode(b);
    local
        .filter(|p| p.is_file())
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| decode(&b))
        .map(|img| (img, false))
        .or_else(|| std::fs::read(cache).ok().and_then(|b| decode(&b)).map(|img| (img, true)))
}

/// An icon that is on disk nowhere: the catalogue's download, written to `cache` once it decodes so the next
/// opening paints it at once. An undecodable or failed download leaves the cache alone.
fn fetch_icon(fetch: impl FnOnce() -> Option<Vec<u8>>, cache: &std::path::Path) -> Option<Image> {
    let bytes = fetch()?;
    let img = crate::cg::imageio::decode(&bytes)?;
    if let Some(dir) = cache.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(cache, &bytes);
    Some(img)
}
/// Height of one line of release notes under an update row.
const NOTE_LINE: f32 = 18.0;
const TODAY_HEAD: f32 = 127.0;
const CARD_H: f32 = 410.0;
const CARD_GAP: f32 = 34.0;
const TODAY_MARGIN: f32 = 34.0;
const LIST_ROW: f32 = 71.0;
const LIST_TOP: f32 = 119.0;
fn bg() -> u32 { CONTENT_BG.now() }
/// Where the reviews written on this PC are kept, under the store root.
const REVIEWS_FILE: &str = "store-reviews.json";

fn cell() -> u32 { CARD_BG.now() }

/// Paints the desktop sidebar column as Settings does: with behind-window vibrancy the premultiplied translucent
/// `MATERIAL_SIDEBAR` tint is written over the pixels (not blended, so the frame keeps its alpha and the presenter
/// blurs the desktop under the `.sidebar` material the shell declares, `take_wayland_materials`), else opaque.
fn sidebar_fill(s: &mut Surface, w: usize, h: usize) {
    let c = if super::apphost::vibrancy_available() {
        let c = super::ui::MATERIAL_SIDEBAR.now();
        rgba((c & 255) as u8, (c >> 8 & 255) as u8, (c >> 16 & 255) as u8, (c >> 24) as u8)
    } else {
        cell()
    };
    s.fill_rect(0, 0, w, h, c);
}
fn fill() -> u32 { FIELD_BG.now() }
fn sep() -> u32 { SEPARATOR_C.now() }
fn label() -> u32 { LABEL.now() }
fn bar_bg() -> u32 { MATERIAL_TITLEBAR_SOLID.now() }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tab {
    Today,
    Apps,
    Games,
    Updates,
    Search,
    /// a desktop sidebar shelf: the apps of a few related catalogue categories
    Shelf(Shelf),
    /// every catalogue category with its apps (desktop sidebar)
    Categories,
}

/// The desktop sidebar's shelves (the reference's Create, Work, Play and Develop). Each gathers the catalogue
/// categories that fit it; a category of the catalogue no shelf names is still listed under Categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shelf {
    Create,
    Work,
    Play,
    Develop,
}

impl Shelf {
    pub fn title(self) -> &'static str {
        match self {
            Shelf::Create => "Create",
            Shelf::Work => "Work",
            Shelf::Play => "Play",
            Shelf::Develop => "Develop",
        }
    }

    /// The catalogue categories on the shelf, in the order they are listed (matched without regard to case).
    pub fn categories(self) -> &'static [&'static str] {
        match self {
            Shelf::Create => &["Graphics & Design", "Photo & Video", "Music"],
            Shelf::Work => &["Productivity", "Business", "Finance", "Reference", "Education"],
            Shelf::Play => &["Games", "Entertainment", "Sports"],
            Shelf::Develop => &["Developer Tools", "Utilities"],
        }
    }
}

const TABS: [(Tab, &str); 5] = [(Tab::Today, "Today"), (Tab::Games, "Games"), (Tab::Apps, "Apps"), (Tab::Updates, "Updates"), (Tab::Search, "Search")];

/// The desktop sidebar, under its search field: Discover (the Today cards), the shelves, Categories and Updates.
const SIDEBAR: [(Tab, &str); 7] = [
    (Tab::Today, "Discover"),
    (Tab::Shelf(Shelf::Create), "Create"),
    (Tab::Shelf(Shelf::Work), "Work"),
    (Tab::Shelf(Shelf::Play), "Play"),
    (Tab::Shelf(Shelf::Develop), "Develop"),
    (Tab::Categories, "Categories"),
    (Tab::Updates, "Updates"),
];

/// The desktop sidebar's geometry (points): the search capsule under the traffic lights, the rows under it, and the
/// account chip at the foot.
const SIDE_SEARCH_Y: f32 = 46.0;
const SIDE_SEARCH_H: f32 = 28.0;
const SIDE_ROWS_Y: f32 = 88.0;
const SIDE_ROW_H: f32 = 36.0;
const SIDE_ACCOUNT_H: f32 = 56.0;
const SIDE_INSET: f32 = 10.0;

/// The desktop window is full-size (its sidebar runs to the top edge under the traffic lights): the pane's content
/// starts under the unified toolbar band, where the back control sits (`super::desk::UNIFIED_BAR_PT`).
const DESKTOP_BAR_PT: f32 = super::desk::UNIFIED_BAR_PT;
/// How wide a press on the pane's back control may be (pt from the pane's left edge).
const BACK_HIT_PT: f32 = 140.0;

thread_local! {
    /// The width (pt from the pane's left edge) of the back control the desktop window last drew in its toolbar band,
    /// 0 while no page is open and so no back control is shown. `draw_bars` keeps it; the shell's drag zone reads it
    /// (`back_control_pt`), so the band only stops dragging where the back control is.
    static BACK_SHOWN_PT: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
}

/// The width (pt) of the back control in the desktop window's toolbar band, 0 when none is shown.
pub fn back_control_pt() -> f32 {
    BACK_SHOWN_PT.with(|c| c.get())
}

pub(super) fn set_back_control_pt(w: f32) {
    BACK_SHOWN_PT.with(|c| c.set(w.max(0.0)));
}

/// The sidebar width (px) of a desktop App Store window `w` px wide at `pt` px per point.
fn sidebar_px(w: i32, pt: f32) -> i32 {
    ((200.0 * pt) as i32).min(w / 3)
}

/// The desktop window's controls in its unified toolbar band, in window points, as the shell's drag zone needs them:
/// the sidebar's search capsule and, while a page is open, the pane's back control (`back_pt` wide from the pane's
/// left edge, `back_control_pt`; 0 when none is shown). The rest of the band moves the window.
pub fn toolbar_controls(w_px: i32, pt: f32, back_pt: f32) -> Vec<bifrost_proto::Node> {
    use bifrost_proto::{Node, Role};
    let pt = pt.max(0.01);
    let side = sidebar_px(w_px, pt) as f32 / pt;
    let node = |id, role, label: &str, bounds| Node { id, role, label: label.into(), value: String::new(), bounds, enabled: true };
    let mut controls = vec![node(1, Role::TextField, "Search", bifrost_ui::Rect { x: SIDE_INSET, y: SIDE_SEARCH_Y, w: (side - 2.0 * SIDE_INSET).max(0.0), h: SIDE_SEARCH_H })];
    if back_pt > 0.0 {
        controls.push(node(2, Role::Button, "Back", bifrost_ui::Rect { x: side, y: 0.0, w: back_pt.min(BACK_HIT_PT), h: DESKTOP_BAR_PT }));
    }
    controls
}

/// What a click in the desktop sidebar lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SideHit {
    Search,
    Tab(Tab),
    Account,
}

/// The name a tab goes by: its sidebar name on the desktop, else its tab bar name.
fn tab_name(tab: Tab, desktop: bool) -> &'static str {
    let side = SIDEBAR.iter().find(|(t, _)| desktop && *t == tab);
    side.or_else(|| TABS.iter().find(|(t, _)| *t == tab)).or_else(|| SIDEBAR.iter().find(|(t, _)| *t == tab)).map_or("", |t| t.1)
}

/// The initials of an account name for the sidebar chip: the first letters of the first two words, upper case.
fn initials(name: &str) -> String {
    name.split_whitespace().filter_map(|w| w.chars().next()).take(2).flat_map(char::to_uppercase).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    Tab(Tab),
    Back,
    Page(String),
    Button(String),
    UpdateAll,
    Chip(usize),
    /// "more" under an update's notes
    More(String),
    /// "Version History" on a product page
    History,
    /// "Roll Back to <previous>" on a product page
    Rollback(String),
    /// "Restore Purchases" at the foot of the Updates tab
    Restore,
    /// the Search tab's query field: tapping it raises the keyboard
    SearchField,
    /// "Remove" on an installed app's page: the first tap asks, the second removes
    Remove(String),
    /// "Rate" on an installed app's page: opens the rating sheet
    Rate(String),
}

/// A Today card built from the catalogue.
#[derive(Clone, Debug, PartialEq)]
enum Card {
    Story { app: usize, tag: &'static str },
    List { eyebrow: &'static str, title: &'static str, apps: Vec<usize> },
}

#[cfg(test)]
impl Card {
    fn apps(&self) -> Vec<usize> {
        match self {
            Card::Story { app, .. } => vec![*app],
            Card::List { apps, .. } => apps.clone(),
        }
    }
}

/// One line of a tab's scrolling content.
#[derive(Clone, Debug, PartialEq)]
enum Row {
    Title(&'static str),
    Header(String, bool),
    TodayHead,
    /// one or two story/list cards side by side
    Cards(Vec<Card>),
    App(usize),
    Local(usize),
    /// a pending update: catalogue index, note lines shown, "more" link offered
    Update(usize, usize, bool),
    Chips,
    /// the query field of the Search tab
    Search,
    Note(String),
    /// "Restore Purchases" at the foot of the Updates tab
    Restore,
}

impl Row {
    fn height(&self) -> f32 {
        match self {
            Row::Title(_) => 56.0,
            Row::Header(..) => 48.0,
            Row::TodayHead => TODAY_HEAD,
            Row::Cards(_) => CARD_H + CARD_GAP,
            Row::App(_) | Row::Local(_) => ROW,
            Row::Update(_, n, _) => ROW + if *n > 0 { NOTE_LINE * *n as f32 + 6.0 } else { 0.0 },
            Row::Chips => 52.0,
            Row::Search => 52.0,
            Row::Note(_) => 64.0,
            Row::Restore => 52.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Btn {
    Get,
    Open,
    Update,
    Progress(f32),
    /// the package was built for another SDK protocol than this shell speaks: it cannot run, so nothing to tap
    NeedsUpdate,
    /// system components are preinstalled: nothing to tap unless an update is on offer
    None,
}

/// The button for catalogue entry `a` given the installed version (if any) and the shell's SDK protocol.
pub fn button_for(a: &StoreApp, installed: Option<&str>, shell: u32) -> Btn {
    match installed {
        Some(v) if api::update_available(v, &a.version) && a.runs_on(shell) => Btn::Update,
        Some(_) | None if !a.runs_on(shell) && !a.is_preinstalled() => Btn::NeedsUpdate,
        Some(v) if api::update_available(v, &a.version) => Btn::None,
        Some(_) if a.is_system() => Btn::None,
        Some(_) => Btn::Open,
        None if a.is_preinstalled() => Btn::None,
        None => Btn::Get,
    }
}

enum Done {
    Catalog(Result<Vec<StoreApp>, String>),
    Icon { id: String, image: Option<Image> },
    /// an icon read from disk on a worker ([`local_icon`]); `url` and `cache` are what a download would need
    LocalIcon { id: String, image: Option<Image>, from_cache: bool, url: String, cache: PathBuf },
    Shot { url: String, image: Option<Image> },
    Installed { id: String, result: Result<InstalledApp, String> },
}

struct Job {
    done: Arc<AtomicU64>,
    size: u64,
    /// set by Cancel: the worker stops at its next chunk and keeps the `.part` file
    cancel: Arc<AtomicBool>,
}

struct Page {
    id: String,
    back: Tab,
    desc: Vec<String>,
    list_scroll: f32,
    /// the newest release's notes, wrapped to the page width
    whatsnew: Vec<String>,
    /// the Version History page is showing instead of the product page
    history: bool,
    /// every release for the history page: (version, date, wrapped note lines)
    hist: Vec<(String, String, Vec<String>)>,
}

pub struct AppStore {
    desktop: bool,
    sidebar: i32,
    w: i32,
    h: i32,
    root: PathBuf,
    pool: Option<Pool<Done>>,
    catalog: Vec<StoreApp>,
    installed: Vec<InstalledApp>,
    registry_changed: bool,
    pub(super) tab: Tab,
    page: Option<Page>,
    category: Option<usize>,
    fetching: bool,
    fetched_at: Option<f64>,
    tried_at: f64,
    error: Option<String>,
    icons: HashMap<String, Option<Image>>,
    icon_px: HashMap<(String, usize), Image>,
    icon_pending: HashSet<String>,
    /// when each icon that would not load failed (the clock of `maintain`)
    icon_failed: HashMap<String, f64>,
    /// icons painted from the cache whose catalogue copy has been fetched again this session (once each)
    icon_revalidated: HashSet<String>,
    /// the last time `maintain` ran
    clock: f64,
    jobs: HashMap<String, Job>,
    /// queue of installs: one download runs at a time, in request order
    coord: coordination::Coordinator,
    /// the last time download progress was redrawn
    progress_at: f64,
    failed: HashMap<String, String>,
    launch: Option<InstalledApp>,
    /// system packages installed or updated here, for the shell to hot-reload
    system_installed: Vec<InstalledApp>,
    scroll: Scroller,
    pressed: Option<Target>,
    scrolling: bool,
    labels: TextCache,
    /// update rows whose notes are expanded ("more")
    expanded: HashSet<String>,
    dirty: bool,
    /// purchases recorded by the purchase sheet this session (merged with the installed apps' records)
    bought: Ledger,
    /// the open purchase sheet of a paid app
    sheet: Option<Sheet>,
    /// the open rating sheet of an installed app
    review: Option<ReviewSheet>,
    /// the reviews written on this PC, kept in `store-reviews.json` (`bifrost_core::app_review::ReviewBook`)
    reviews: bifrost_core::app_review::ReviewBook,
    /// the result line under Restore Purchases
    restored: Option<String>,
    /// the Search tab's query, typed from the on-screen or a hardware keyboard
    query: String,
    /// set by a tap on the query field, read once by the shell (it raises the keyboard)
    keyboard_wanted: bool,
    /// the app whose Remove was tapped once: the next tap removes it
    confirm_remove: Option<String>,
    /// the local account's name, for the sidebar's account chip (never sent anywhere)
    account: String,
    /// decoded screenshots by URL, and each one resized to the strip height in px
    shots: HashMap<String, Image>,
    shot_px: HashMap<(String, usize), Image>,
    shot_pending: HashSet<String>,
    /// when each screenshot that would not load failed (retried like icons)
    shot_failed: HashMap<String, f64>,
}

/// What the product page and rows call a system package's category.
const SYSTEM_COMPONENT: &str = "System Component";

fn category_label(a: &StoreApp) -> &str {
    if a.is_system() { SYSTEM_COMPONENT } else { &a.category }
}

fn is_game(a: &StoreApp) -> bool {
    a.category.eq_ignore_ascii_case("games")
}

/// Whether an entry matches a search: every word of the query appears in its name, summary, category or id (any case).
fn matches_query(a: &StoreApp, query: &str) -> bool {
    let hay = format!("{} {} {} {}", a.name, a.summary, a.category, a.id).to_lowercase();
    query.split_whitespace().all(|w| hay.contains(&w.to_lowercase()))
}

/// Remove in two taps: the first asks (the button reads "Confirm"), the second removes. True on the second tap.
fn remove_step(confirm: &mut Option<String>, id: &str) -> bool {
    if confirm.as_deref() == Some(id) {
        *confirm = None;
        true
    } else {
        *confirm = Some(id.to_string());
        false
    }
}

impl AppStore {
    /// `root` is `/var/lib/springboard` (the store cache and `apps/` live below it).
    pub fn new(ui: &Ui, w: i32, h: i32, root: PathBuf) -> Self {
        let catalog = api::load_cache(&root.join("store-cache.json")).unwrap_or_default();
        let installed = install::installed_in(&root);
        let reviews = bifrost_core::app_review::ReviewBook::from_text(&std::fs::read_to_string(root.join(REVIEWS_FILE)).unwrap_or_default());
        let desktop = true;
        let sidebar = if desktop { sidebar_px(w, ui.pt(1.0)) } else { 0 };
        AppStore {
            desktop,
            sidebar,
            w: w - sidebar,
            h,
            root,
            pool: Pool::new(3).ok(),
            catalog,
            installed,
            tab: Tab::Today,
            page: None,
            category: None,
            fetching: false,
            fetched_at: None,
            tried_at: f64::NEG_INFINITY,
            error: None,
            icons: HashMap::new(),
            icon_px: HashMap::new(),
            icon_pending: HashSet::new(),
            icon_failed: HashMap::new(),
            icon_revalidated: HashSet::new(),
            clock: 0.0,
            jobs: HashMap::new(),
            coord: coordination::Coordinator::new(coordination::current_uid()),
            progress_at: f64::NEG_INFINITY,
            failed: HashMap::new(),
            launch: None,
            system_installed: vec![],
            registry_changed: false,
            scroll: Scroller::new(),
            pressed: None,
            scrolling: false,
            labels: TextCache::default(),
            expanded: HashSet::new(),
            dirty: true,
            bought: Ledger::new(),
            sheet: None,
            review: None,
            reviews,
            restored: None,
            query: String::new(),
            keyboard_wanted: false,
            confirm_remove: None,
            account: crate::shell::account::load().full_name,
            shots: HashMap::new(),
            shot_px: HashMap::new(),
            shot_pending: HashSet::new(),
            shot_failed: HashMap::new(),
        }
    }

    pub fn set_size(&mut self, ui: &Ui, w: i32, h: i32) {
        self.sidebar = if self.desktop { sidebar_px(w, ui.pt(1.0)) } else { 0 };
        self.w = (w - self.sidebar).max(1);
        self.h = h.max(1);
        self.fit_scroll(ui);
        self.dirty = true;
    }

    pub fn needs_frame(&self) -> bool {
        self.dirty || !self.jobs.is_empty() || bifrost_render::appearance::fading()
    }

    pub fn describe(&self) -> String {
        format!("store_tab={:?}\nstore_apps={}\nstore_installed={}\nstore_jobs={}\nstore_error={}\n", self.tab, self.catalog.len(), self.installed.len(), self.jobs.len(), self.error.as_deref().unwrap_or(""))
    }

    pub fn fds(&self) -> Vec<std::ffi::c_int> {
        self.pool.iter().map(|p| p.fd()).collect()
    }

    /// The app the user tapped OPEN on; the shell hands it to the app host.
    pub fn take_launch(&mut self) -> Option<InstalledApp> {
        self.launch.take()
    }

    /// System packages installed or updated since the last call; the shell reloads what they provide.
    pub fn take_system_installed(&mut self) -> Vec<InstalledApp> {
        std::mem::take(&mut self.system_installed)
    }

    /// The Updates tab badge: installed apps and system components with a newer store version.
    pub fn update_count(&self) -> usize {
        self.update_list().len()
    }

    /// How many apps `apps/installed.json` lists.
    pub fn take_registry_changed(&mut self) -> bool { std::mem::take(&mut self.registry_changed) }

    pub fn installed_count(&self) -> usize {
        self.installed.len()
    }

    /// Re-reads `apps/installed.json` (e.g. after a removal elsewhere).
    pub fn refresh_installed(&mut self) {
        self.installed = install::installed_in(&self.root);
        self.dirty = true;
    }

    // ---- state ----

    fn installed_version(&self, id: &str) -> Option<&str> {
        self.installed.iter().find(|i| i.id == id).map(|i| i.version.as_str())
    }

    pub fn button(&self, a: &StoreApp) -> Btn {
        if let Some(j) = self.jobs.get(&a.id) {
            return Btn::Progress(if j.size == 0 { 0.0 } else { (j.done.load(Ordering::Relaxed) as f32 / j.size as f32).min(1.0) });
        }
        if a.is_service() {
            return Btn::None;
        }
        button_for(a, self.installed_version(&a.id), bifrost_proto::SDK_VERSION)
    }

    fn find(&self, id: &str) -> Option<&StoreApp> {
        self.catalog.iter().find(|a| a.id == id)
    }

    fn categories(&self) -> Vec<String> {
        let mut c: Vec<String> = self.catalog.iter().map(|a| a.category.clone()).collect();
        c.sort();
        c.dedup();
        c
    }

    fn update_list(&self) -> Vec<usize> {
        (0..self.catalog.len()).filter(|&i| !self.catalog[i].is_service() && matches!(self.installed_version(&self.catalog[i].id), Some(v) if api::update_available(v, &self.catalog[i].version))).collect()
    }

    /// The installed apps the customer has purchased on `day`, newest purchase first (ties by name). Every installed app
    /// (not a background service) is a free purchase recorded on its first install day, held in the entitlement ledger.
    fn purchased(&self, day: i64) -> Vec<usize> {
        let mut ledger = self.bought.clone();
        // a service stays hidden whether the install record or the catalogue entry says so
        for i in self.installed.iter().filter(|i| !i.is_service() && !self.catalog.iter().any(|a| a.id == i.id && a.is_service())) {
            ledger.record(&i.id, i.purchased, None);
        }
        let mut out: Vec<(i64, String, usize)> = ledger
            .current_entitlements(day)
            .into_iter()
            .filter_map(|t| {
                let idx = self.installed.iter().position(|i| i.id == t.product)?;
                Some((t.purchased, self.installed[idx].name.to_lowercase(), idx))
            })
            .collect();
        out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        out.into_iter().map(|(_, _, i)| i).collect()
    }

    /// Story and list cards for Today from the catalogue, newest release first.
    fn today_cards(&self, mut apps: Vec<usize>) -> Vec<Card> {
        let date = |i: usize| self.catalog[i].changelog.first().map(|r| r.date.clone()).unwrap_or_default();
        apps.sort_by(|&a, &b| date(b).cmp(&date(a)).then_with(|| self.catalog[a].name.to_lowercase().cmp(&self.catalog[b].name.to_lowercase())));
        let tag = |i: usize| if self.catalog[i].changelog.len() > 1 { "UPDATED" } else { "NOW AVAILABLE" };
        let mut cards = vec![];
        let mut rest = apps.into_iter();
        if let Some(first) = rest.next() {
            cards.push(Card::Story { app: first, tag: tag(first) });
        }
        let list: Vec<usize> = rest.by_ref().take(4).collect();
        if !list.is_empty() {
            cards.push(Card::List { eyebrow: "OUR FAVOURITES", title: "Essential apps", apps: list });
        }
        cards.extend(rest.map(|app| Card::Story { app, tag: tag(app) }));
        cards
    }

    fn rows(&self) -> Vec<Row> {
        let mut r = vec![];
        let by_name = |mut v: Vec<usize>, cat: &[StoreApp]| {
            v.sort_by_key(|&i| cat[i].name.to_lowercase());
            v
        };
        // system components are listed under "System" (Apps tab) and on the Updates tab only
        let all: Vec<usize> = (0..self.catalog.len()).filter(|&i| !self.catalog[i].is_preinstalled()).collect();
        let system: Vec<usize> = (0..self.catalog.len()).filter(|&i| self.catalog[i].is_preinstalled() && !self.catalog[i].is_service()).collect();
        match self.tab {
            Tab::Today => {
                r.push(Row::TodayHead);
                r.extend(self.today_cards(all).chunks(2).map(|c| Row::Cards(c.to_vec())));
            }
            Tab::Apps | Tab::Games => {
                let games = self.tab == Tab::Games;
                r.push(Row::Title(if games { "Games" } else { "Apps" }));
                let list: Vec<usize> = all.into_iter().filter(|&i| is_game(&self.catalog[i]) == games).collect();
                if !list.is_empty() {
                    r.push(Row::Header(if games { "Top Games" } else { "Top Apps" }.into(), false));
                }
                if list.is_empty() && !self.catalog.is_empty() {
                    r.push(Row::Note(if games { "No games in the store yet." } else { "No apps in the store yet." }.into()));
                }
                r.extend(by_name(list, &self.catalog).into_iter().map(Row::App));
                if !games && !system.is_empty() {
                    r.push(Row::Header("System".into(), false));
                    r.extend(by_name(system, &self.catalog).into_iter().map(Row::App));
                }
            }
            Tab::Updates => {
                r.push(Row::Title("Updates"));
                let ups = self.update_list();
                r.push(Row::Header("Available Updates".into(), !ups.is_empty() && ups.iter().any(|&i| !self.jobs.contains_key(&self.catalog[i].id))));
                if ups.is_empty() {
                    r.push(Row::Note("All apps are up to date.".into()));
                }
                r.extend(ups.iter().map(|&i| {
                    let n = self.catalog[i].changelog.first().map_or(0, |c| c.notes.len());
                    let open = self.expanded.contains(&self.catalog[i].id);
                    Row::Update(i, if open { n } else { n.min(2) }, !open && n > 2)
                }));
                let rest: Vec<usize> = self.purchased(today_days()).into_iter().filter(|&i| !ups.iter().any(|&u| self.catalog[u].id == self.installed[i].id)).collect();
                if !rest.is_empty() {
                    r.push(Row::Header("Purchased".into(), false));
                    for i in rest {
                        match self.catalog.iter().position(|a| a.id == self.installed[i].id) {
                            Some(c) => r.push(Row::App(c)),
                            None => r.push(Row::Local(i)),
                        }
                    }
                }
                if let Some(msg) = &self.restored {
                    r.push(Row::Note(msg.clone()));
                }
                r.push(Row::Restore);
            }
            Tab::Shelf(shelf) => {
                r.push(Row::Title(shelf.title()));
                let mut any = false;
                for cat in shelf.categories() {
                    let list: Vec<usize> = all.iter().copied().filter(|&i| self.catalog[i].category.eq_ignore_ascii_case(cat)).collect();
                    if !list.is_empty() {
                        any = true;
                        r.push(Row::Header((*cat).into(), false));
                        r.extend(by_name(list, &self.catalog).into_iter().map(Row::App));
                    }
                }
                if !any && !self.catalog.is_empty() {
                    r.push(Row::Note(format!("No {} apps in the store yet.", shelf.title().to_lowercase())));
                }
            }
            Tab::Categories => {
                r.push(Row::Title("Categories"));
                for cat in self.categories() {
                    let list: Vec<usize> = all.iter().copied().filter(|&i| self.catalog[i].category == cat).collect();
                    if !list.is_empty() {
                        r.push(Row::Header(if cat.is_empty() { "Other".into() } else { cat }, false));
                        r.extend(by_name(list, &self.catalog).into_iter().map(Row::App));
                    }
                }
                if !system.is_empty() {
                    r.push(Row::Header("System".into(), false));
                    r.extend(by_name(system, &self.catalog).into_iter().map(Row::App));
                }
            }
            Tab::Search => {
                r.push(Row::Title("Search"));
                // on the desktop the query field is the sidebar's, above the sections
                if !self.desktop {
                    r.push(Row::Search);
                }
                r.push(Row::Chips);
                let cats = self.categories();
                let q = self.query.trim();
                let list: Vec<usize> = all.into_iter().filter(|&i| self.category.is_none_or(|c| cats.get(c) == Some(&self.catalog[i].category)) && matches_query(&self.catalog[i], q)).collect();
                if list.is_empty() && !q.is_empty() && !self.catalog.is_empty() {
                    r.push(Row::Note(format!("No results for \u{201c}{q}\u{201d}.")));
                }
                r.extend(by_name(list, &self.catalog).into_iter().map(Row::App));
            }
        }
        if self.catalog.is_empty() && matches!(self.tab, Tab::Today | Tab::Apps | Tab::Games | Tab::Search | Tab::Shelf(_) | Tab::Categories) {
            let msg = match (&self.error, self.fetching) {
                (_, true) => "Loading the store\u{2026}".to_string(),
                (Some(e), false) => format!("Cannot connect to the store. {e}"),
                _ => "No apps yet.".to_string(),
            };
            r.push(Row::Note(msg));
        }
        r
    }

    // ---- worker results ----

    pub fn pump(&mut self) -> bool {
        let done = self.pool.as_mut().map(|p| p.drain()).unwrap_or_default();
        let changed = !done.is_empty();
        for d in done {
            match d {
                Done::Catalog(Ok(apps)) => {
                    self.fetching = false;
                    self.error = None;
                    self.catalog = apps;
                }
                Done::Catalog(Err(e)) => {
                    self.fetching = false;
                    self.error = Some(e);
                }
                Done::Shot { url, image } => {
                    self.shot_pending.remove(&url);
                    match image {
                        Some(img) => {
                            self.shot_failed.remove(&url);
                            self.shot_px.retain(|(u, _), _| *u != url);
                            self.shots.insert(url, img);
                        }
                        None => {
                            self.shot_failed.insert(url, self.clock);
                        }
                    }
                }
                Done::Icon { id, image } => {
                    self.icon_pending.remove(&id);
                    match image {
                        Some(img) => {
                            self.icon_px.retain(|(i, _), _| *i != id);
                            self.icon_failed.remove(&id);
                            self.icons.insert(id, Some(img));
                        }
                        // a revalidation that failed keeps the cached icon and is not retried
                        None if self.icons.contains_key(&id) => {}
                        // not cached as missing: it is asked for again after ICON_RETRY_MS
                        None => {
                            self.icon_failed.insert(id, self.clock);
                        }
                    }
                }
                Done::LocalIcon { id, image, from_cache, url, cache } => self.local_icon_done(id, image, from_cache, url, cache),
                // a cancelled job was already removed; a newer job for the same app must stay
                Done::Installed { result: Err(e), .. } if e == install::CANCELLED => {}
                Done::Installed { id, result } => {
                    self.jobs.remove(&id);
                    self.coord.finish(&id, result.as_ref().map(|_| ()).map_err(|e| e.clone()));
                    match result {
                        Ok(app) => {
                            if app.is_preinstalled() {
                                self.system_installed.push(app);
                            }
                            self.registry_changed = true;
                            self.failed.remove(&id);
                            self.installed = install::installed_in(&self.root);
                            self.icons.remove(&id);
                        }
                        Err(e) => {
                            self.failed.insert(id, e);
                        }
                    }
                    self.start_queued();
                }
            }
        }
        if changed {
            self.dirty = true;
        }
        changed
    }

    /// A new App Store window starts on the front page (Today, the desktop's Discover), as the reference opens it:
    /// not on the product page, category or sheet the last window was closed on (W5-20). Installs keep running.
    pub fn show_front_page(&mut self) {
        self.tab = Tab::Today;
        self.page = None;
        self.category = None;
        self.sheet = None;
        self.review = None;
        self.confirm_remove = None;
        self.pressed = None;
        self.scrolling = false;
        self.scroll.jump_top();
        self.dirty = true;
    }

    /// Fetches the catalogue again on the next tick (opening the store, the Updates tab), so a
    /// version published a moment ago shows up without waiting for the 10-minute refresh.
    pub fn refresh_soon(&mut self) {
        if !self.fetching {
            self.tried_at = f64::NEG_INFINITY;
            self.dirty = true;
        }
    }

    fn maintain(&mut self, t: f64) {
        let wait = if self.catalog.is_empty() || self.error.is_some() { RETRY_MS } else { REFRESH_MS };
        if !self.fetching && t - self.tried_at >= wait {
            self.tried_at = t;
            self.fetching = true;
            self.fetched_at = Some(t);
            let cache = self.root.join("store-cache.json");
            match &self.pool {
                Some(p) => p.spawn(move || {
                    crate::gfx::affinity::background_thread();
                    Done::Catalog(api::fetch(&cache))
                }),
                None => self.fetching = false,
            }
        }
        self.clock = t;
        self.sync_coordinator();
        self.fetch_shots(t);
        // only the icons still to fetch are copied out of the lists
        let wanted: Vec<(String, String)> = self
            .catalog
            .iter()
            .map(|a| (a.id.as_str(), a.icon.as_str()))
            .chain(self.installed.iter().map(|i| (i.id.as_str(), "")))
            .filter(|(id, _)| !self.icons.contains_key(*id) && !self.icon_pending.contains(*id) && icon_due(self.icon_failed.get(*id).copied(), t))
            .map(|(id, icon)| (id.to_string(), icon.to_string()))
            .collect();
        for (id, icon) in wanted {
            if self.icon_pending.contains(&id) {
                continue;
            }
            let local = self.installed.iter().find(|i| i.id == id).map(|i| i.icon_path.clone());
            // the catalogue's URL for this id (an installed app's own row carries none)
            let url = if icon.is_empty() { self.catalog.iter().find(|a| a.id == id).map(|a| a.icon.clone()).unwrap_or_default() } else { icon };
            let cache = icon_cache_path(&self.root, &id);
            let Some(p) = &self.pool else {
                // no workers: nothing reads the disk or the network, so the letter tile stays until the retry
                self.icon_failed.insert(id, t);
                continue;
            };
            // the installed or cached icon is read and decoded on a worker, ahead of the downloads, so the tick never
            // waits on the disk; the letter tile shows until it answers (a frame or two). The network is asked only
            // when neither is on disk, or to revalidate a cached copy (`local_icon_done`).
            self.icon_pending.insert(id.clone());
            let fallback = (id.clone(), url.clone(), cache.clone());
            p.spawn_front_or(
                move || {
                    crate::gfx::affinity::background_thread();
                    let found = local_icon(local.as_deref(), &cache);
                    let from_cache = found.as_ref().is_some_and(|(_, c)| *c);
                    Done::LocalIcon { id, image: found.map(|(img, _)| img), from_cache, url, cache }
                },
                move || Done::LocalIcon { id: fallback.0, image: None, from_cache: false, url: fallback.1, cache: fallback.2 },
            );
        }
    }

    /// The worker's answer for an icon on disk. A found icon is painted; one from the cache is then fetched from the
    /// catalogue once more in the background (once per session), so an icon the catalogue changed replaces the cached
    /// copy. With nothing on disk the catalogue's copy is downloaded, or, without a URL, the retry waits as for a
    /// failed fetch rather than asking every tick.
    fn local_icon_done(&mut self, id: String, image: Option<Image>, from_cache: bool, url: String, cache: PathBuf) {
        let download = match image {
            Some(img) => {
                self.icon_failed.remove(&id);
                self.icon_px.retain(|(i, _), _| *i != id);
                self.icons.insert(id.clone(), Some(img));
                from_cache && !url.is_empty() && self.icon_revalidated.insert(id.clone())
            }
            None if url.is_empty() => {
                self.icon_failed.insert(id.clone(), self.clock);
                false
            }
            None => true,
        };
        self.dirty = true;
        match &self.pool {
            Some(p) if download => {
                // still pending: its `Done::Icon` clears it
                let fallback = id.clone();
                p.spawn_or(
                    move || {
                        crate::gfx::affinity::background_thread();
                        let fetch = || net::get(&net::absolute(&url), 1_000_000).ok();
                        Done::Icon { image: fetch_icon(fetch, &cache), id }
                    },
                    move || Done::Icon { id: fallback, image: None },
                );
            }
            _ => {
                self.icon_pending.remove(&id);
            }
        }
    }

    /// The open product page's screenshots, each fetched once (a failed one is retried like an icon).
    fn fetch_shots(&mut self, t: f64) {
        let urls: Vec<String> = self.page.as_ref().filter(|p| !p.history).and_then(|p| self.find(&p.id)).map(|a| a.screenshots.clone()).unwrap_or_default();
        for url in urls {
            if self.shots.contains_key(&url) || self.shot_pending.contains(&url) || !icon_due(self.shot_failed.get(&url).copied(), t) {
                continue;
            }
            self.shot_pending.insert(url.clone());
            if let Some(p) = &self.pool {
                let abs = net::absolute(&url);
                p.spawn(move || {
                    crate::gfx::affinity::background_thread();
                    let bytes = net::get(&abs, 4_000_000).ok();
                    Done::Shot { url, image: bytes.and_then(|b| crate::cg::imageio::decode(&b)) }
                });
            }
        }
    }

    /// Queues an install with the coordinator. The UI keeps its progress ring per job; the download itself
    /// starts when `start_queued` finds no other download running.
    fn start(&mut self, id: &str) {
        let Some(a) = self.find(id).cloned() else { return };
        if self.jobs.contains_key(id) {
            return;
        }
        self.failed.remove(id);
        if self.coord.request(&a.id, coordination::current_uid(), a.size).is_err() {
            return;
        }
        let done = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs.insert(id.to_string(), Job { done, size: a.size, cancel });
        self.start_queued();
    }

    /// Starts the coordinator's next queued install when no download is running. Without a worker pool
    /// nothing can run, so the job is finished as failed rather than left queued.
    fn start_queued(&mut self) {
        while let Some(id) = self.coord.start_next() {
            let Some(a) = self.find(&id).cloned() else {
                self.coord.finish(&id, Err("the app is no longer in the catalogue".into()));
                self.jobs.remove(&id);
                continue;
            };
            let Some(job) = self.jobs.get(&id) else {
                self.coord.finish(&id, Err("no job for the install".into()));
                continue;
            };
            let (done, cancel) = (job.done.clone(), job.cancel.clone());
            let root = self.root.clone();
            match &self.pool {
                Some(p) => {
                    p.spawn(move || {
                        crate::gfx::affinity::background_thread();
                        let result = (|| {
                            let app = install::install_download(&root, &a.id, &net::absolute(&a.package), a.size, &a.sha256, install::shell_version(), &done, &cancel)?;
                            if app.id != a.id || app.is_preinstalled() != a.is_preinstalled() {
                                let _ = install::uninstall(&root, &app.id);
                                return Err("package id does not match the store entry".to_string());
                            }
                            Ok(app)
                        })();
                        if let Err(e) = &result {
                            crate::install_log::record(&crate::setup::shell_data_dir(), "app install", &format!("{}: {e}", a.id));
                        }
                        Done::Installed { id: a.id, result }
                    });
                    return;
                }
                None => {
                    self.coord.finish(&id, Err("no worker pool".into()));
                    self.jobs.remove(&id);
                }
            }
        }
    }

    /// Copies each download counter into the coordinator, which keeps the job's bytes and phase. A phase
    /// change marks the screen dirty; byte counts only feed the progress ring, which already redraws at
    /// its own rate.
    fn sync_coordinator(&mut self) {
        let counts: Vec<(String, u64)> = self.jobs.iter().map(|(id, j)| (id.clone(), j.done.load(Ordering::Relaxed))).collect();
        for (id, done) in counts {
            self.coord.progress(&id, done);
        }
        if self.coord.drain_events().iter().any(|e| matches!(e, coordination::Event::Phase { .. })) {
            self.dirty = true;
        }
    }

    /// Restores the kept previous version of an installed app.
    fn rollback(&mut self, id: &str) {
        let root = self.root.clone();
        match install::rollback(&root, id) {
            Ok(()) => {
                self.registry_changed = true;
                self.installed = install::installed_in(&root);
                self.icons.remove(id);
                self.failed.remove(id);
            }
            Err(e) => {
                self.failed.insert(id.to_string(), e);
            }
        }
    }

    /// Stops a download: the worker halts at its next chunk and its `.part` file stays,
    /// so the next GET resumes from it.
    fn cancel(&mut self, id: &str) {
        if let Some(j) = self.jobs.remove(id) {
            j.cancel.store(true, Ordering::Relaxed);
            self.coord.cancel(id);
            self.start_queued();
            self.dirty = true;
        }
    }

    /// Opens the purchase sheet for a paid app: nothing is downloaded until the customer confirms.
    fn open_sheet(&mut self, id: &str) {
        let Some(a) = self.find(id).cloned() else { return };
        let Some(price) = a.price.clone() else { return };
        self.sheet = Some(Sheet::open(&self.root, &a.id, &a.name, price));
        self.dirty = true;
    }

    /// A confirmed purchase: recorded in the entitlement ledger, then the paid package is installed.
    fn finish_purchase(&mut self) {
        let Some(open) = self.sheet.take() else { return };
        let Sheet { id, mut purchase, .. } = open;
        let tx = self.bought.record(&id, today_days(), None);
        if purchase.complete(Outcome::Success(tx)) && purchase.may_install() {
            self.failed.remove(&id);
            self.start(&id);
        }
    }

    /// Restore Purchases: re-reads the install records, which hold the purchases of this PC, and says how many there are.
    fn restore_purchases(&mut self) {
        self.refresh_installed();
        self.restored = Some(match self.purchased(today_days()).len() {
            0 => "No purchases to restore.".to_string(),
            1 => "Restored 1 purchase.".to_string(),
            n => format!("Restored {n} purchases."),
        });
        self.dirty = true;
    }

    /// Whether the page offers Remove: an installed app the customer chose, not a system part, a service or a
    /// preinstalled package, and not being downloaded now.
    fn can_remove(&self, id: &str) -> bool {
        !self.jobs.contains_key(id) && self.installed.iter().any(|i| i.id == id && !i.is_system() && !i.is_preinstalled() && !i.is_service())
    }

    /// Uninstalls an app and refreshes the Installed list and the home screen.
    fn remove(&mut self, id: &str) {
        let root = self.root.clone();
        match install::uninstall(&root, id) {
            Ok(()) => {
                self.registry_changed = true;
                self.installed = install::installed_in(&root);
                self.failed.remove(id);
                self.expanded.remove(id);
            }
            Err(e) => {
                self.failed.insert(id.to_string(), e);
            }
        }
    }

    /// Whether typing reaches the query field: the Search tab with no product page or sheet open (see `key`).
    /// The desktop sidebar's width in px (0 on the phone layout): the shell declares this column a `.sidebar` material.
    #[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
    pub fn sidebar_px(&self) -> i32 {
        if self.desktop { self.sidebar } else { 0 }
    }

    pub fn text_focused(&self) -> bool {
        self.tab == Tab::Search && self.page.is_none() && self.sheet.is_none()
    }

    /// Typed text for the Search tab (from the on-screen keyboard or a hardware keyboard); other keys are ignored.
    pub fn key(&mut self, out: Out) {
        if let Some(r) = self.review.as_mut() {
            match out {
                Out::Text(s) => {
                    r.type_text(&s);
                }
                Out::Backspace => {
                    r.backspace();
                }
                Out::Enter => {
                    r.type_text("\n");
                }
                _ => return,
            }
            self.dirty = true;
            return;
        }
        if self.tab != Tab::Search || self.page.is_some() || self.sheet.is_some() {
            return;
        }
        match out {
            Out::Text(s) => self.query.push_str(&s),
            Out::Backspace => {
                self.query.pop();
            }
            _ => return,
        }
        self.scroll.jump_top();
        self.dirty = true;
    }

    /// The desktop sidebar's search capsule (px, window coordinates).
    fn side_search_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let x = ui.pt(SIDE_INSET);
        (x, ui.pt(SIDE_SEARCH_Y), (self.sidebar as f32 - 2.0 * x).max(0.0), ui.pt(SIDE_SEARCH_H))
    }

    /// The desktop sidebar's row `i` (px).
    fn side_row_rect(&self, ui: &Ui, i: usize) -> (f32, f32, f32, f32) {
        let x = ui.pt(SIDE_INSET);
        (x, ui.pt(SIDE_ROWS_Y + i as f32 * SIDE_ROW_H), (self.sidebar as f32 - 2.0 * x).max(0.0), ui.pt(SIDE_ROW_H - 2.0))
    }

    /// The account chip at the foot of the desktop sidebar (px).
    fn side_account_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let x = ui.pt(SIDE_INSET);
        (x, self.h as f32 - ui.pt(SIDE_ACCOUNT_H), (self.sidebar as f32 - 2.0 * x).max(0.0), ui.pt(SIDE_ACCOUNT_H - SIDE_INSET))
    }

    /// What a click at `(x, y)` (px) in the desktop sidebar lands on.
    fn side_hit(&self, ui: &Ui, x: f32, y: f32) -> Option<SideHit> {
        let inside = |r: (f32, f32, f32, f32)| x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3;
        if inside(self.side_search_rect(ui)) {
            return Some(SideHit::Search);
        }
        if inside(self.side_account_rect(ui)) {
            return Some(SideHit::Account);
        }
        (0..SIDEBAR.len()).find(|&i| inside(self.side_row_rect(ui, i))).map(|i| SideHit::Tab(SIDEBAR[i].0))
    }

    /// A click on the sidebar's search field: the Search results take the pane and the typing goes to the query.
    fn focus_search(&mut self) {
        if self.tab != Tab::Search || self.page.is_some() {
            self.tab = Tab::Search;
            self.page = None;
            self.scroll.jump_top();
        }
        self.keyboard_wanted = true;
        self.dirty = true;
    }

    /// True once after a tap on the query field: the shell raises the on-screen keyboard for it.
    pub fn take_keyboard_request(&mut self) -> bool {
        std::mem::take(&mut self.keyboard_wanted)
    }

    fn press_button(&mut self, id: &str) {
        match self.find(id).map(|a| self.button(a)) {
            Some(Btn::None | Btn::NeedsUpdate) => {}
            // a paid app needs a purchase, and the shell has no purchase sheet yet: it is never installed here
            Some(Btn::Get) if self.find(id).is_some_and(|a| a.is_paid()) => self.open_sheet(id),
            Some(Btn::Get | Btn::Update) => self.start(id),
            Some(Btn::Progress(_)) => self.cancel(id),
            Some(Btn::Open) => self.launch = self.installed.iter().find(|i| i.id == id).cloned(),
            _ => {
                // a store-less installed app opens too
                if let Some(i) = self.installed.iter().find(|i| i.id == id && !i.is_system()) {
                    self.launch = Some(i.clone());
                }
            }
        }
    }

    // ---- layout and input ----

    fn bottom(&self, ui: &Ui) -> f32 {
        if self.desktop { self.h as f32 - ui.pt(16.0) } else { self.h as f32 - home_zone(ui) - ui.pt(TAB_BAR) }
    }

    fn top(&self, ui: &Ui) -> f32 {
        // the desktop window has no title bar. Today (Discover) starts at the top edge as the reference does, its
        // large title under the empty toolbar band; the other tabs and the pages start under the band, which holds
        // the back control and the inline title
        if self.desktop {
            return if self.page.is_none() && self.tab == Tab::Today { 0.0 } else { ui.pt(DESKTOP_BAR_PT) };
        }
        if self.page.is_none() && self.tab == Tab::Today {
            return ui.pt(STATUS_PT);
        }
        ui.pt(NAV_BOTTOM)
    }

    fn card_w(&self, ui: &Ui) -> f32 {
        self.w as f32 - ui.pt(2.0 * MARGIN)
    }

    fn page_height(&self, ui: &Ui) -> f32 {
        let Some(p) = self.page.as_ref() else { return 0.0 };
        if p.history {
            return ui.pt(24.0 + p.hist.iter().map(|(_, _, l)| 40.0 + 22.0 * l.len() as f32 + 16.0).sum::<f32>() + 60.0);
        }
        ui.pt(150.0 + 84.0 + 40.0 + 22.0 * p.desc.len() as f32 + self.rollback_height(&p.id) + self.whatsnew_height() + 60.0) + ui.pt(self.shots_pt())
    }

    /// Height (pt) of the What's New block on the product page; 0 without release notes.
    /// The kept previous version of an installed (non-system) app, if a rollback is possible.
    fn rollback_to(&self, id: &str) -> Option<&str> {
        self.installed.iter().find(|i| i.id == id && !i.is_system()).and_then(|i| i.previous.as_deref())
    }

    /// Extra page height (pt) taken by the "Roll Back" row.
    fn rollback_height(&self, id: &str) -> f32 {
        if self.rollback_to(id).is_some() { 52.0 } else { 0.0 }
    }

    /// Rect (px) of the "Roll Back" row: below the description, above What's New.
    fn rollback_rect(&self, ui: &Ui, n_desc: usize) -> (f32, f32, f32, f32) {
        let y = self.top(ui) - self.scroll.offset() + ui.pt(150.0 + 84.0 + 40.0 + self.shots_pt() + 22.0 * n_desc as f32 + 8.0);
        (ui.pt(MARGIN), y, self.card_w(ui), ui.pt(44.0))
    }

    /// Height (pt) of the screenshot strip on the open product page; 0 without screenshots.
    fn shots_pt(&self) -> f32 {
        match self.page.as_ref().and_then(|p| self.find(&p.id)) {
            Some(a) if !a.screenshots.is_empty() => SHOTS_PT,
            _ => 0.0,
        }
    }

    /// Rect (px) of the Remove button: right-hand side of the product page's button row.
    fn remove_btn_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (bw, bh) = (ui.pt(BTN_W + 8.0), ui.pt(BTN_H + 2.0));
        (self.w as f32 - ui.pt(MARGIN) - bw, self.top(ui) - self.scroll.offset() + ui.pt(100.0), bw, bh)
    }

    /// Rect (px) of the Rate button: beside the Get or Open button of an installed app's page.
    fn rate_btn_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (x, y, w, h) = self.page_btn_rect(ui);
        (x + w + ui.pt(12.0), y, w, h)
    }

    fn whatsnew_height(&self) -> f32 {
        match self.page.as_ref() {
            Some(p) if !p.hist.is_empty() => 16.0 + 78.0 + 22.0 * p.whatsnew.len() as f32 + 8.0 + 44.0,
            _ => 0.0,
        }
    }

    /// Top of the "Version History" row (px, scroll applied).
    fn history_row(&self, ui: &Ui) -> (f32, f32) {
        let n = self.page.as_ref().map_or(0, |p| p.desc.len());
        let nl = self.page.as_ref().map_or(0, |p| p.whatsnew.len());
        (self.top(ui) - self.scroll.offset() + ui.pt(150.0 + 84.0 + 40.0 + self.shots_pt() + 22.0 * n as f32 + 16.0 + 78.0 + 22.0 * nl as f32 + 8.0), ui.pt(44.0))
    }

    fn content_height(&self, ui: &Ui) -> f32 {
        if self.page.is_some() {
            return self.page_height(ui);
        }
        ui.pt(self.rows().iter().map(Row::height).sum::<f32>() + 24.0)
    }

    fn fit_scroll(&mut self, ui: &Ui) {
        let view = (self.bottom(ui) - self.top(ui)).max(0.0);
        let max = (self.content_height(ui) - view).max(0.0);
        self.scroll.set_max(max);
    }

    fn btn_rect(&self, ui: &Ui, row_y: f32, row_h: f32) -> (f32, f32, f32, f32) {
        let (bw, bh) = (ui.pt(BTN_W), ui.pt(BTN_H));
        (self.w as f32 - ui.pt(MARGIN) - bw, row_y + (row_h - bh) / 2.0, bw, bh)
    }

    fn page_btn_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (bw, bh) = (ui.pt(BTN_W + 8.0), ui.pt(BTN_H + 2.0));
        (ui.pt(MARGIN + 100.0 + 16.0), self.top(ui) - self.scroll.offset() + ui.pt(100.0), bw, bh)
    }

    /// Visits each row with its top y (in px, scroll applied).
    fn each_row(&self, ui: &Ui, mut f: impl FnMut(&Row, f32, f32)) {
        let mut y = self.top(ui) - self.scroll.offset();
        for r in self.rows() {
            let h = ui.pt(r.height());
            f(&r, y, h);
            y += h;
        }
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let bar = self.bottom(ui);
        // the tab bar is the tablet's; the desktop has its sidebar instead
        if !self.desktop && y >= bar && y < bar + ui.pt(TAB_BAR) {
            let i = (x / (self.w as f32 / TABS.len() as f32)) as usize;
            return TABS.get(i).map(|(t, _)| Target::Tab(*t));
        }
        if y < self.top(ui) {
            return (self.page.is_some() && x < ui.pt(BACK_HIT_PT)).then_some(Target::Back);
        }
        if y >= bar {
            return None;
        }
        if let Some(p) = &self.page {
            if p.history {
                return None;
            }
            let (hy, hh) = self.history_row(ui);
            if !p.hist.is_empty() && y >= hy && y < hy + hh {
                return Some(Target::History);
            }
            if self.rollback_to(&p.id).is_some() {
                let (rx, ry, rw, rh) = self.rollback_rect(ui, p.desc.len());
                if x >= rx && x < rx + rw && y >= ry && y < ry + rh {
                    return Some(Target::Rollback(p.id.clone()));
                }
            }
            if self.can_remove(&p.id) {
                let (rx, ry, rw, rh) = self.remove_btn_rect(ui);
                if x >= rx && x < rx + rw && y >= ry && y < ry + rh {
                    return Some(Target::Remove(p.id.clone()));
                }
            }
            if self.installed_version(&p.id).is_some() {
                let (rx, ry, rw, rh) = self.rate_btn_rect(ui);
                if x >= rx && x < rx + rw && y >= ry && y < ry + rh {
                    return Some(Target::Rate(p.id.clone()));
                }
            }
            let (bx, by, bw, bh) = self.page_btn_rect(ui);
            return (x >= bx && x < bx + bw && y >= by && y < by + bh).then(|| Target::Button(p.id.clone()));
        }
        let mut found = None;
        let ids: Vec<(String, Option<String>)> = self.rows_ids();
        let mut n = 0;
        self.each_row(ui, |row, ry, rh| {
            let idx = n;
            n += 1;
            if found.is_some() || y < ry || y >= ry + rh {
                return;
            }
            match row {
                Row::App(_) | Row::Local(_) | Row::Update(..) => {
                    let (bx, by, bw, bh) = self.btn_rect(ui, ry, if matches!(row, Row::Update(..)) { ui.pt(ROW) } else { rh });
                    let id = ids[idx].0.clone();
                    let on_btn = x >= bx - ui.pt(8.0) && x < bx + bw + ui.pt(8.0) && y >= by - ui.pt(8.0) && y < by + bh + ui.pt(8.0);
                    found = Some(if matches!(row, Row::Update(_, _, true)) && y >= ry + ui.pt(ROW) && x > self.w as f32 - ui.pt(MARGIN + 70.0) {
                        Target::More(id)
                    } else if on_btn {
                        Target::Button(id)
                    } else if matches!(row, Row::Local(_)) {
                        Target::Button(id)
                    } else {
                        Target::Page(id)
                    });
                }
                Row::Cards(cards) => {
                    'cards: for (col, card) in cards.iter().enumerate() {
                        for ((cx, cy, cw, ch), id, button) in self.card_items(ui, ry, col, card).into_iter().rev() {
                            if x >= cx && x < cx + cw && y >= cy && y < cy + ch {
                                found = Some(if button { Target::Button(id) } else { Target::Page(id) });
                                break 'cards;
                            }
                        }
                    }
                }
                Row::Header(_, true) if x > self.w as f32 - ui.pt(130.0) => found = Some(Target::UpdateAll),
                Row::Restore => found = Some(Target::Restore),
                Row::Search => found = Some(Target::SearchField),
                Row::Chips => {
                    let n = self.categories().len() + 1;
                    let cw = self.card_w(ui) / n as f32;
                    let i = ((x - ui.pt(MARGIN)) / cw).floor();
                    if i >= 0.0 && (i as usize) < n {
                        found = Some(Target::Chip(i as usize));
                    }
                }
                _ => {}
            }
        });
        found
    }

    /// Row id per row (parallel to `rows()`): the app id for app rows.
    fn rows_ids(&self) -> Vec<(String, Option<String>)> {
        self.rows()
            .iter()
            .map(|r| match r {
                Row::App(i) | Row::Update(i, ..) => (self.catalog[*i].id.clone(), None),
                Row::Local(i) => (self.installed[*i].id.clone(), None),
                _ => (String::new(), None),
            })
            .collect()
    }

    fn card_w_px(&self, ui: &Ui) -> f32 {
        (self.w as f32 - ui.pt(3.0 * TODAY_MARGIN)) / 2.0
    }

    /// Hit areas of a Today card (rect in px, app id, is a GET button); buttons come last.
    fn card_items(&self, ui: &Ui, ry: f32, col: usize, card: &Card) -> Vec<((f32, f32, f32, f32), String, bool)> {
        let cw = self.card_w_px(ui);
        let x0 = ui.pt(TODAY_MARGIN) + col as f32 * (cw + ui.pt(TODAY_MARGIN));
        let mut pages = vec![];
        let mut buttons = vec![];
        match card {
            Card::Story { app, .. } => {
                let id = self.catalog[*app].id.clone();
                pages.push(((x0, ry, cw, ui.pt(CARD_H)), id.clone(), false));
                buttons.push(((x0 + cw - ui.pt(16.0 + 70.0), ry + ui.pt(358.0), ui.pt(70.0), ui.pt(26.0)), id, true));
            }
            Card::List { apps, .. } => {
                for (i, &a) in apps.iter().enumerate() {
                    let top = ry + ui.pt(LIST_TOP + LIST_ROW * i as f32);
                    let id = self.catalog[a].id.clone();
                    pages.push(((x0, top, cw, ui.pt(LIST_ROW)), id.clone(), false));
                    buttons.push(((x0 + cw - ui.pt(20.0 + 66.0), top + ui.pt((LIST_ROW - 28.0) / 2.0), ui.pt(66.0), ui.pt(28.0)), id, true));
                }
            }
        }
        pages.extend(buttons);
        pages
    }

    fn activate(&mut self, ui: &Ui, target: Target) {
        match target {
            Target::Tab(t) => {
                if self.tab == t && self.page.is_none() {
                    self.scroll.jump_top();
                } else {
                    self.tab = t;
                    if t == Tab::Updates {
                        self.refresh_soon();
                    }
                    self.page = None;
                    self.scroll.jump_top();
                }
            }
            Target::History => {
                if let Some(p) = self.page.as_mut() {
                    p.history = true;
                }
                self.scroll.jump_top();
            }
            Target::More(id) => {
                self.expanded.insert(id);
            }
            Target::Back if self.page.as_ref().is_some_and(|p| p.history) => {
                if let Some(p) = self.page.as_mut() {
                    p.history = false;
                }
                self.scroll.jump_top();
            }
            Target::Back => {
                self.confirm_remove = None;
                if let Some(p) = self.page.take() {
                    self.tab = p.back;
                    self.fit_scroll(ui);
                    self.scroll.jump_to(p.list_scroll);
                }
            }
            Target::Page(id) => {
                if let Some(a) = self.find(&id).cloned() {
                    let text = if a.description.is_empty() { a.summary.clone() } else { a.description.clone() };
                    let desc = ui.wrap(&text, 17.0, Weight::Regular, self.card_w(ui));
                    let hist: Vec<(String, String, Vec<String>)> = a.changelog.iter().map(|r| (r.version.clone(), r.date.clone(), wrap_notes(ui, &r.notes, self.card_w(ui)))).collect();
                    let whatsnew = hist.first().map(|h| h.2.clone()).unwrap_or_default();
                    self.page = Some(Page { id, back: self.tab, desc, list_scroll: self.scroll.offset(), whatsnew, history: false, hist });
                    self.scroll.jump_top();
                }
            }
            Target::Button(id) => self.press_button(&id),
            Target::Rollback(id) => self.rollback(&id),
            Target::Restore => self.restore_purchases(),
            Target::SearchField => self.keyboard_wanted = true,
            Target::Rate(id) => self.open_review(&id),
            Target::Remove(id) => {
                if remove_step(&mut self.confirm_remove, &id) {
                    self.remove(&id);
                }
            }
            Target::UpdateAll => {
                for i in self.update_list() {
                    let id = self.catalog[i].id.clone();
                    self.start(&id);
                }
            }
            Target::Chip(i) => self.category = if i == 0 || self.category == Some(i - 1) { None } else { Some(i - 1) },
        }
        self.fit_scroll(ui);
        self.dirty = true;
    }

    /// While a purchase sheet is up it takes every touch; the pane's coordinates skip the sidebar on desktop.
    fn touch_sheet(&mut self, ui: &Ui, ev: Event, t: f64) {
        let dx = if self.desktop { self.sidebar as f32 } else { 0.0 };
        let ev = match ev {
            Event::Down(x, y) => Event::Down(x - dx, y),
            Event::Move(x, y) => Event::Move(x - dx, y),
            Event::Up(x, y) => Event::Up(x - dx, y),
            other => other,
        };
        let (w, h, now) = (self.w, self.h, crate::shell::clock::tz::unix_now());
        let out = self.sheet.as_mut().and_then(|s| s.touch(ui, w, h, ev, t, now));
        self.dirty = true;
        match out {
            Some(SheetOut::Confirmed) => self.finish_purchase(),
            Some(SheetOut::Cancelled) => self.sheet = None,
            None => {}
        }
    }

    /// Opens the rating sheet for an installed app, with the review kept before to edit.
    fn open_review(&mut self, id: &str) {
        let Some(a) = self.find(id).cloned() else { return };
        self.review = Some(ReviewSheet::open(&a.id, &a.name, self.reviews.get(&a.id)));
        self.dirty = true;
    }

    /// While the rating sheet is up it takes every touch; a tap on a field asks the shell for the keyboard.
    fn touch_review(&mut self, ui: &Ui, ev: Event) {
        let dx = if self.desktop { self.sidebar as f32 } else { 0.0 };
        let ev = match ev {
            Event::Down(x, y) => Event::Down(x - dx, y),
            Event::Move(x, y) => Event::Move(x - dx, y),
            Event::Up(x, y) => Event::Up(x - dx, y),
            other => other,
        };
        let (w, h) = (self.w, self.h);
        let before = self.review.as_ref().and_then(ReviewSheet::focus);
        let out = self.review.as_mut().and_then(|r| r.touch(ui, w, h, ev));
        self.dirty = true;
        match out {
            Some(ReviewOut::Send(review)) => {
                if let Some(open) = self.review.take() {
                    self.keep_review(&open.id, &review);
                }
            }
            Some(ReviewOut::Cancelled) => self.review = None,
            None => {
                let now = self.review.as_ref().and_then(ReviewSheet::focus);
                if now.is_some() && now != before {
                    self.keyboard_wanted = true;
                }
            }
        }
    }

    /// Keeps a sent review in the book and writes the book. Nothing is sent anywhere: the catalogue takes no reviews yet.
    fn keep_review(&mut self, id: &str, review: &bifrost_core::app_review::Review) {
        if self.reviews.set(id, review).is_ok() {
            let _ = std::fs::create_dir_all(&self.root);
            if let Err(e) = std::fs::write(self.root.join(REVIEWS_FILE), self.reviews.to_text()) {
                eprintln!("springboard: {REVIEWS_FILE}: {e}");
            }
        }
    }

    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) {
        if self.review.is_some() {
            self.touch_review(ui, ev);
            return;
        }
        if self.sheet.is_some() {
            self.touch_sheet(ui, ev, t);
            return;
        }
        let (mut x, y) = match ev {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            _ => return,
        };
        if self.desktop {
            if x < self.sidebar as f32 {
                if matches!(ev, Event::Up(..)) {
                    match self.side_hit(ui, x, y) {
                        Some(SideHit::Search) => self.focus_search(),
                        Some(SideHit::Tab(tab)) => self.activate(ui, Target::Tab(tab)),
                        // the account's apps: what it bought and what has an update
                        Some(SideHit::Account) => self.activate(ui, Target::Tab(Tab::Updates)),
                        None => {}
                    }
                }
                self.dirty = true;
                return;
            }
            x -= self.sidebar as f32;
        }
        self.dirty = true;
        match ev {
            Event::Down(..) => {
                self.pressed = self.hit(ui, x, y);
                if y >= self.top(ui) && y < self.bottom(ui) {
                    self.scrolling = true;
                    self.scroll.down(y, t);
                }
            }
            Event::Move(..) => {
                if self.scrolling {
                    self.scroll.moved(y, t);
                    if self.scroll.dragging() {
                        self.pressed = None;
                    }
                }
            }
            Event::Up(..) => {
                let tap = if std::mem::take(&mut self.scrolling) { self.scroll.up(y, t) } else { true };
                let pressed = self.pressed.take();
                if tap {
                    if let Some(target) = pressed.filter(|p| self.hit(ui, x, y).as_ref() == Some(p)) {
                        self.activate(ui, target);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        self.maintain(t);
        self.fit_scroll(ui);
        let was = self.scroll.animating();
        let moving = self.scroll.tick(t);
        let progress = !self.jobs.is_empty() && t - self.progress_at >= PROGRESS_MS;
        if progress {
            self.progress_at = t;
        }
        let shaking = self.sheet.as_ref().is_some_and(|s| s.animating(t));
        self.dirty |= was || moving || progress || shaking;
        self.dirty
    }

    pub fn next_wake(&self, t: f64) -> Option<f64> {
        if self.scroll.animating() {
            return Some(t);
        }
        // a failed icon is fetched again when its retry is due, without waiting for input to tick the store; one whose
        // retry is in flight waits for the reply instead, so the loop does not spin until it arrives
        let retry = self.icon_failed.iter().filter(|(id, _)| !self.icon_pending.contains(*id)).map(|(_, at)| (at + ICON_RETRY_MS).max(t)).reduce(f64::min);
        let progress = (!self.jobs.is_empty()).then(|| (self.progress_at + PROGRESS_MS).max(t));
        match (progress, retry) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    // ---- drawing ----

    #[allow(clippy::too_many_arguments)]
    fn put(&mut self, s: &mut Surface, ui: &Ui, text: &str, size: f32, weight: Weight, color: u32, right: bool, x: f32, mid: f32) {
        let img = self.labels.get(ui, text, size, weight, color);
        let left = if right { x - img.w as f32 } else { x };
        s.blit(img, left.round() as i32, (mid - img.h as f32 / 2.0).round() as i32, 255, 255);
    }

    /// The app's icon at `px`, squircle-masked; a tinted tile until it arrives.
    fn draw_icon(&mut self, s: &mut Surface, ui: &Ui, id: &str, name: &str, x: f32, y: f32, px: usize) {
        let key = (id.to_string(), px);
        if !self.icon_px.contains_key(&key) {
            if let Some(Some(src)) = self.icons.get(id) {
                let mut img = src.resized(px, px);
                img.apply_mask(&squircle_mask(px));
                self.icon_px.insert(key.clone(), img);
            }
        }
        if let Some(img) = self.icon_px.get(&key) {
            s.blit(img, x.round() as i32, y.round() as i32, 255, 255);
            return;
        }
        s.fill_rrect(x, y, px as f32, px as f32, px as f32 * 0.225, fill());
        let letter: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
        self.put(s, ui, &letter, px as f32 / ui.pt(1.0) * 0.45, Weight::Semibold, SECONDARY, false, x + px as f32 * 0.36, y + px as f32 / 2.0);
    }

    fn draw_button(&mut self, s: &mut Surface, ui: &Ui, r: (f32, f32, f32, f32), b: Btn) {
        self.draw_button_on(s, ui, r, b, fill());
    }

    fn draw_button_on(&mut self, s: &mut Surface, ui: &Ui, (x, y, w, h): (f32, f32, f32, f32), b: Btn, fill: u32) {
        let mid = y + h / 2.0;
        match b {
            Btn::None => {}
            Btn::Progress(f) => {
                let r = h * 0.38;
                let c = (x + w / 2.0, mid);
                let bounds = (c.0 - r - 4.0, mid - r - 4.0, 2.0 * r + 8.0, 2.0 * r + 8.0);
                paint(s, bounds, sep(), ring(c, r, ui.pt(2.5)));
                paint(s, bounds, TINT, arc(c, r, ui.pt(2.5), 0.0, (f.max(0.02)) * std::f32::consts::TAU - 0.0001));
                let q = r * 0.42;
                paint(s, bounds, TINT, segment((c.0 - q, c.1 - q), (c.0 + q, c.1 + q), ui.pt(0.1)));
                s.fill_rrect(c.0 - q, c.1 - q, 2.0 * q, 2.0 * q, ui.pt(2.0), TINT);
            }
            _ => {
                let label = match b {
                    Btn::Get => "GET",
                    Btn::Open => "OPEN",
                    Btn::NeedsUpdate => "Needs update",
                    _ => "UPDATE",
                };
                s.fill_rrect(x, y, w, h, CAPSULE_RADIUS, fill);
                let size = if b == Btn::NeedsUpdate { 11.0 } else { 15.0 };
                let lw = ui.measure(label, size, Weight::Bold);
                self.put(s, ui, label, size, Weight::Bold, TINT, false, x + (w - lw) / 2.0, mid);
            }
        }
    }

    fn draw_app_row(&mut self, s: &mut Surface, ui: &Ui, y: f32, h: f32, id: &str, name: &str, line2: &str, line3: &str, b: Btn, last: bool) {
        let x = ui.pt(MARGIN);
        let ipx = ui.pt(ICON) as usize;
        self.draw_icon(s, ui, id, name, x, y + (h - ipx as f32) / 2.0, ipx);
        let tx = x + ipx as f32 + ui.pt(14.0);
        let max = self.w as f32 - tx - ui.pt(MARGIN + BTN_W + 12.0);
        let name = ui.fit(name, 17.0, max);
        self.put(s, ui, &name, 17.0, Weight::Regular, label(), false, tx, y + h / 2.0 - ui.pt(17.0));
        let l2 = ui.fit(line2, 13.0, max);
        self.put(s, ui, &l2, 13.0, Weight::Regular, SECONDARY, false, tx, y + h / 2.0 + ui.pt(2.0));
        if !line3.is_empty() {
            let l3 = ui.fit(line3, 13.0, max);
            self.put(s, ui, &l3, 13.0, Weight::Regular, SECONDARY, false, tx, y + h / 2.0 + ui.pt(20.0));
        }
        let r = self.btn_rect(ui, y, h);
        self.draw_button(s, ui, r, b);
        if !last {
            s.fill_rect(tx as usize, (y + h - 1.0).max(0.0) as usize, (self.w as f32 - tx - ui.pt(MARGIN)) as usize, 1, sep());
        }
    }

    fn draw_content(&mut self, s: &mut Surface, ui: &Ui) {
        let (top, bottom) = (self.top(ui) as usize, self.bottom(ui) as usize);
        let rows = self.rows();
        let mut c = s.rows(top, bottom);
        let mut y = self.top(ui) - self.scroll.offset();
        let (x, w) = (ui.pt(MARGIN), self.card_w(ui));
        for (n, row) in rows.iter().enumerate() {
            let h = ui.pt(row.height());
            if y + h < top as f32 || y > bottom as f32 {
                y += h;
                continue;
            }
            let last = rows.get(n + 1).is_none_or(|r| !matches!(r, Row::App(_) | Row::Local(_) | Row::Update(..)));
            match row {
                Row::Title(t) => self.put(&mut c, ui, t, 34.0, Weight::Bold, label(), false, x, y + h * 0.58),
                Row::Header(t, all) => {
                    self.put(&mut c, ui, t, 22.0, Weight::Bold, label(), false, x, y + h * 0.6);
                    if *all {
                        self.put(&mut c, ui, "Update All", 17.0, Weight::Regular, TINT, true, self.w as f32 - x, y + h * 0.6);
                    }
                }
                Row::Note(t) => {
                    let t = ui.fit(t, 15.0, w);
                    self.put(&mut c, ui, &t, 15.0, Weight::Regular, SECONDARY, false, x, y + h / 2.0);
                }
                Row::Restore => {
                    let text = "Restore Purchases";
                    let tw = ui.measure(text, 17.0, Weight::Regular);
                    self.put(&mut c, ui, text, 17.0, Weight::Regular, TINT, false, (self.w as f32 - tw) / 2.0, y + h / 2.0);
                }
                Row::Search => {
                    let fy = y + ui.pt(6.0);
                    c.fill_rrect(x, fy, w, ui.pt(40.0), CAPSULE_RADIUS, fill());
                    let (text, ink) = if self.query.is_empty() { ("Search apps and games".to_string(), SECONDARY) } else { (self.query.clone(), label()) };
                    let text = ui.fit(&text, 17.0, w - ui.pt(32.0));
                    self.put(&mut c, ui, &text, 17.0, Weight::Regular, ink, false, x + ui.pt(16.0), fy + ui.pt(20.0));
                }
                Row::Chips => {
                    let mut names = vec!["All".to_string()];
                    names.extend(self.categories());
                    let cw = w / names.len() as f32;
                    for (i, name) in names.iter().enumerate() {
                        let on = self.category.map_or(i == 0, |c| c + 1 == i);
                        let (fill, ink) = if on { (TINT, WHITE) } else { (cell(), SECONDARY) };
                        let cx = x + i as f32 * cw;
                        c.fill_rrect(cx + ui.pt(2.0), y + ui.pt(8.0), cw - ui.pt(4.0), ui.pt(34.0), CAPSULE_RADIUS, fill);
                        let label = ui.fit(name, 14.0, cw - ui.pt(14.0));
                        let lw = ui.measure(&label, 14.0, Weight::Semibold);
                        self.put(&mut c, ui, &label, 14.0, Weight::Semibold, ink, false, cx + (cw - lw) / 2.0, y + ui.pt(25.0));
                    }
                }
                Row::TodayHead => self.draw_today_head(&mut c, ui, y),
                Row::Cards(cards) => {
                    for (col, card) in cards.iter().enumerate() {
                        self.draw_card(&mut c, ui, y, col, card);
                    }
                }
                Row::App(i) => {
                    let a = self.catalog[*i].clone();
                    let b = self.button(&a);
                    let (l2, l3) = self.app_lines(&a);
                    self.draw_app_row(&mut c, ui, y, h, &a.id, &a.name, &l2, &l3, b, last);
                }
                Row::Update(i, shown, more) => {
                    let a = self.catalog[*i].clone();
                    let b = self.button(&a);
                    let (l2, l3) = self.app_lines(&a);
                    self.draw_app_row(&mut c, ui, y, ui.pt(ROW), &a.id, &a.name, &l2, &l3, b, true);
                    let tx = x + ui.pt(ICON + 14.0);
                    let lines: Vec<String> = a.changelog.first().map(|r| r.notes.iter().take(*shown).cloned().collect()).unwrap_or_default();
                    for (k, note) in lines.iter().enumerate() {
                        let reserve = if *more && k + 1 == lines.len() { 56.0 } else { 0.0 };
                        let t = ui.fit(note, 13.0, self.w as f32 - tx - ui.pt(MARGIN + reserve));
                        let my = y + ui.pt(ROW + 4.0 + NOTE_LINE * k as f32 + NOTE_LINE / 2.0);
                        self.put(&mut c, ui, &t, 13.0, Weight::Regular, label(), false, tx, my);
                        if reserve > 0.0 {
                            self.put(&mut c, ui, "more", 13.0, Weight::Regular, TINT, true, self.w as f32 - x, my);
                        }
                    }
                    if !last {
                        c.fill_rect(tx as usize, (y + h - 1.0).max(0.0) as usize, (self.w as f32 - tx - x) as usize, 1, sep());
                    }
                }
                Row::Local(i) => {
                    let a = self.installed[*i].clone();
                    let b = if a.is_system() {
                        Btn::None
                    } else if self.find(&a.id).is_some_and(|s| !s.runs_on(bifrost_proto::SDK_VERSION)) {
                        Btn::NeedsUpdate
                    } else {
                        Btn::Open
                    };
                    self.draw_app_row(&mut c, ui, y, h, &a.id, &a.name, &format!("Version {}", a.version), if a.is_system() { SYSTEM_COMPONENT } else { "" }, b, last);
                }
            }
            y += h;
        }
    }

    fn draw_today_head(&mut self, s: &mut Surface, ui: &Ui, y: f32) {
        let x = ui.pt(TODAY_MARGIN);
        self.put(s, ui, &date_label(), 15.0, Weight::Semibold, SECONDARY, false, x, y + ui.pt(30.0));
        self.put(s, ui, tab_name(Tab::Today, self.desktop), 34.0, Weight::Bold, label(), false, x, y + ui.pt(70.0));
        if self.desktop {
            // the account sits at the foot of the sidebar on the desktop
            s.fill_rect(x as usize, (y + ui.pt(95.0)) as usize, (self.w as f32 - 2.0 * x) as usize, 1, sep());
            return;
        }
        let (cx, cy, r) = (self.w as f32 - x - ui.pt(16.0), y + ui.pt(70.0), ui.pt(15.5));
        paint(s, (cx - r - 4.0, cy - r - 4.0, 2.0 * r + 8.0, 2.0 * r + 8.0), TINT, ring((cx, cy), r, ui.pt(2.0)));
        s.fill_rrect(cx - ui.pt(5.0), cy - ui.pt(11.0), ui.pt(10.0), ui.pt(10.0), ui.pt(5.0), TINT);
        s.fill_rrect(cx - ui.pt(9.0), cy + ui.pt(2.0), ui.pt(18.0), ui.pt(9.0), ui.pt(4.5), TINT);
        s.fill_rect(x as usize, (y + ui.pt(95.0)) as usize, (self.w as f32 - 2.0 * x) as usize, 1, sep());
    }

    fn draw_card(&mut self, s: &mut Surface, ui: &Ui, y: f32, col: usize, card: &Card) {
        let cw = self.card_w_px(ui);
        let x0 = ui.pt(TODAY_MARGIN) + col as f32 * (cw + ui.pt(TODAY_MARGIN));
        let radius = ui.pt(CARD_RADIUS_PT);
        match card {
            Card::Story { app, tag } => {
                let a = self.catalog[*app].clone();
                let hue = STORY_HUES[a.id.bytes().fold(0usize, |h, b| h.wrapping_mul(31).wrapping_add(b as usize)) % STORY_HUES.len()];
                let (dark, light) = (shade(hue, 0.55), tint_toward_white(hue, 0.6));
                s.fill_rrect(x0, y, cw, ui.pt(CARD_H), radius, hue);
                s.fill_rrect(x0 + ui.pt(6.0), y + ui.pt(6.0), cw - ui.pt(12.0), ui.pt(210.0), ui.pt(8.0), dark);
                let ipx = ui.pt(120.0) as usize;
                self.draw_icon(s, ui, &a.id, &a.name, x0 + (cw - ipx as f32) / 2.0, y + ui.pt(50.0), ipx);
                let tw = ui.measure(tag, 15.0, Weight::Semibold);
                s.fill_rrect(x0, y, tw + ui.pt(32.0), ui.pt(28.0), radius, hue);
                self.put(s, ui, tag, 15.0, Weight::Semibold, WHITE, false, x0 + ui.pt(16.0), y + ui.pt(14.0));
                let text_w = cw - ui.pt(32.0);
                let title = ui.fit(&a.name, 24.0, text_w);
                self.put(s, ui, &title, 24.0, Weight::Bold, WHITE, false, x0 + ui.pt(16.0), y + ui.pt(236.0));
                for (k, line) in wrap_clamped(ui, &a.summary, 17.0, Weight::Regular, text_w, 2).into_iter().enumerate() {
                    self.put(s, ui, &line, 17.0, Weight::Regular, light, false, x0 + ui.pt(16.0), y + ui.pt(268.0 + 20.0 * k as f32));
                }
                s.fill_rect((x0 + ui.pt(16.0)) as usize, (y + ui.pt(325.0)) as usize, text_w as usize, 1, shade(light, 0.6));
                let ipx = ui.pt(44.0) as usize;
                self.draw_icon(s, ui, &a.id, &a.name, x0 + ui.pt(16.0), y + ui.pt(341.0), ipx);
                let tx = x0 + ui.pt(16.0 + 44.0 + 12.0);
                let max = cw - ui.pt(16.0 + 44.0 + 12.0 + 70.0 + 24.0);
                let name = ui.fit(&a.name, 15.0, max);
                self.put(s, ui, &name, 15.0, Weight::Semibold, WHITE, false, tx, y + ui.pt(355.0));
                let cat = ui.fit(category_label(&a), 13.0, max);
                self.put(s, ui, &cat, 13.0, Weight::Regular, light, false, tx, y + ui.pt(373.0));
                let b = self.button(&a);
                let r = self.card_items(ui, y, col, card).pop().map(|i| i.0).unwrap_or_default();
                self.draw_button_on(s, ui, r, b, WHITE);
            }
            Card::List { eyebrow, title, apps } => {
                s.fill_rrect(x0, y, cw, ui.pt(CARD_H), radius, cell());
                let x = x0 + ui.pt(20.0);
                self.put(s, ui, eyebrow, 15.0, Weight::Semibold, SECONDARY, false, x, y + ui.pt(25.0));
                for (k, line) in ui.wrap(title, 30.0, Weight::Bold, cw - ui.pt(40.0)).into_iter().take(2).enumerate() {
                    self.put(s, ui, &line, 30.0, Weight::Bold, label(), false, x, y + ui.pt(57.0 + 33.0 * k as f32));
                }
                let items = self.card_items(ui, y, col, card);
                let n = apps.len();
                for (i, &ai) in apps.iter().enumerate() {
                    let a = self.catalog[ai].clone();
                    let top = y + ui.pt(LIST_TOP + LIST_ROW * i as f32);
                    let ipx = ui.pt(48.0) as usize;
                    self.draw_icon(s, ui, &a.id, &a.name, x, top + ui.pt((LIST_ROW - 48.0) / 2.0), ipx);
                    let tx = x + ui.pt(48.0 + 14.0);
                    let max = cw - ui.pt(20.0 + 48.0 + 14.0 + 66.0 + 12.0 + 20.0);
                    let name = ui.fit(&a.name, 17.0, max);
                    self.put(s, ui, &name, 17.0, Weight::Regular, label(), false, tx, top + ui.pt(26.0));
                    let sum = ui.fit(&a.summary, 13.0, max);
                    self.put(s, ui, &sum, 13.0, Weight::Regular, SECONDARY, false, tx, top + ui.pt(46.0));
                    let b = self.button(&a);
                    self.draw_button_on(s, ui, items[n + i].0, b, fill());
                    if i + 1 < n {
                        s.fill_rect(tx as usize, (top + ui.pt(LIST_ROW) - 1.0) as usize, (x0 + cw - ui.pt(20.0) - tx) as usize, 1, sep());
                    }
                }
            }
        }
    }

    fn app_lines(&self, a: &StoreApp) -> (String, String) {
        if let Some(e) = self.failed.get(&a.id) {
            return (format!("Failed: {e}"), String::new());
        }
        match self.installed_version(&a.id) {
            Some(v) if api::update_available(v, &a.version) => (format!("Version {} \u{2192} {}", v, a.version), a.summary.clone()),
            _ => (a.summary.clone(), row_detail(category_label(a), &a.version, a.size)),
        }
    }

    fn draw_page(&mut self, s: &mut Surface, ui: &Ui) {
        let Some(page) = self.page.as_ref() else { return };
        let Some(a) = self.find(&page.id).cloned() else { return };
        let desc = page.desc.clone();
        if page.history {
            return self.draw_history(s, ui);
        }
        let whatsnew = page.whatsnew.clone();
        let latest = a.changelog.first().cloned();
        let (top, bottom) = (self.top(ui) as usize, self.bottom(ui) as usize);
        let mut c = s.rows(top, bottom);
        let x = ui.pt(MARGIN);
        let y0 = self.top(ui) - self.scroll.offset();
        let ipx = ui.pt(100.0) as usize;
        self.draw_icon(&mut c, ui, &a.id, &a.name, x, y0 + ui.pt(8.0), ipx);
        let tx = x + ipx as f32 + ui.pt(16.0);
        let name = ui.fit(&a.name, 22.0, self.w as f32 - tx - x);
        self.put(&mut c, ui, &name, 22.0, Weight::Bold, label(), false, tx, y0 + ui.pt(28.0));
        let cat = ui.fit(category_label(&a), 15.0, self.w as f32 - tx - x);
        self.put(&mut c, ui, &cat, 15.0, Weight::Regular, SECONDARY, false, tx, y0 + ui.pt(58.0));
        let b = self.button(&a);
        let r = self.page_btn_rect(ui);
        self.draw_button(&mut c, ui, r, b);
        if self.installed_version(&a.id).is_some() {
            let (rx, ry, rw, rh) = self.rate_btn_rect(ui);
            c.fill_rrect(rx, ry, rw, rh, CAPSULE_RADIUS, fill());
            let lw = ui.measure("Rate", 15.0, Weight::Bold);
            self.put(&mut c, ui, "Rate", 15.0, Weight::Bold, TINT, false, rx + (rw - lw) / 2.0, ry + rh / 2.0);
        }
        if self.can_remove(&a.id) {
            let (rx, ry, rw, rh) = self.remove_btn_rect(ui);
            let text = if self.confirm_remove.as_deref() == Some(a.id.as_str()) { "Confirm" } else { "Remove" };
            c.fill_rrect(rx, ry, rw, rh, CAPSULE_RADIUS, fill());
            let lw = ui.measure(text, 15.0, Weight::Bold);
            self.put(&mut c, ui, text, 15.0, Weight::Bold, super::ui::SYSTEM_RED, false, rx + (rw - lw) / 2.0, ry + rh / 2.0);
        }
        let mut y = y0 + ui.pt(150.0);
        let w = self.card_w(&*ui);
        c.fill_rect(x as usize, y as usize, w as usize, 1, sep());
        let cells = [("VERSION", a.version.clone()), ("SIZE", size_text(a.size)), ("PRICE", a.price_text()), ("AGE", a.age_text()), ("CATEGORY", category_label(&a).to_string())];
        let cw = w / cells.len() as f32;
        for (i, (k, v)) in cells.iter().enumerate() {
            let cx = x + cw * i as f32 + cw / 2.0;
            let kw = ui.measure(k, 11.0, Weight::Semibold);
            self.put(&mut c, ui, k, 11.0, Weight::Semibold, SECONDARY, false, cx - kw / 2.0, y + ui.pt(22.0));
            let v = ui.fit(v, 17.0, cw - ui.pt(8.0));
            let vw = ui.measure(&v, 17.0, Weight::Semibold);
            self.put(&mut c, ui, &v, 17.0, Weight::Semibold, label(), false, cx - vw / 2.0, y + ui.pt(48.0));
        }
        y += ui.pt(84.0);
        c.fill_rect(x as usize, y as usize, w as usize, 1, sep());
        if self.shots_pt() > 0.0 {
            self.draw_shots(&mut c, ui, &a.id, x, y + ui.pt(16.0));
        }
        y += ui.pt(self.shots_pt());
        y += ui.pt(40.0);
        for line in &desc {
            self.put(&mut c, ui, line, 17.0, Weight::Regular, label(), false, x, y);
            y += ui.pt(22.0);
        }
        if let Some(prev) = self.rollback_to(&a.id).map(str::to_string) {
            let (rx, ry, rw, _) = self.rollback_rect(ui, desc.len());
            c.fill_rect(rx as usize, ry as usize, rw as usize, 1, sep());
            self.put(&mut c, ui, &format!("Roll Back to {prev}"), 17.0, Weight::Regular, TINT, false, rx, ry + ui.pt(22.0));
            y += ui.pt(52.0);
        }
        if let Some(r) = latest {
            y += ui.pt(16.0);
            self.put(&mut c, ui, "What\u{2019}s New", 22.0, Weight::Bold, label(), false, x, y + ui.pt(16.0));
            self.put(&mut c, ui, &format!("Version {}", r.version), 15.0, Weight::Regular, SECONDARY, false, x, y + ui.pt(52.0));
            self.put(&mut c, ui, &api::relative_date(&r.date, today_days()), 15.0, Weight::Regular, SECONDARY, true, x + w, y + ui.pt(52.0));
            y += ui.pt(78.0);
            for line in &whatsnew {
                self.put(&mut c, ui, line, 17.0, Weight::Regular, label(), false, x, y);
                y += ui.pt(22.0);
            }
            y += ui.pt(8.0);
            c.fill_rect(x as usize, y as usize, w as usize, 1, sep());
            self.put(&mut c, ui, "Version History", 17.0, Weight::Regular, TINT, false, x, y + ui.pt(22.0));
            self.put(&mut c, ui, "\u{203a}", 22.0, Weight::Regular, SECONDARY, true, x + w, y + ui.pt(22.0));
            y += ui.pt(44.0);
            c.fill_rect(x as usize, y as usize, w as usize, 1, sep());
        }
        if let Some(e) = self.failed.get(&a.id).cloned() {
            let e = ui.fit(&format!("Install failed: {e}"), 13.0, w);
            self.put(&mut c, ui, &e, 13.0, Weight::Regular, super::ui::SYSTEM_RED, false, x, y + ui.pt(24.0));
        }
    }

    /// The screenshot strip: each picture scaled to `SHOT_H` tall, a tinted placeholder until it arrives.
    fn draw_shots(&mut self, s: &mut Surface, ui: &Ui, id: &str, x: f32, y: f32) {
        let urls = match self.find(id) {
            Some(a) => a.screenshots.clone(),
            None => return,
        };
        let h = ui.pt(SHOT_H) as usize;
        let gap = ui.pt(12.0);
        let mut cx = x;
        for url in urls {
            let key = (url.clone(), h);
            if !self.shot_px.contains_key(&key) {
                if let Some(src) = self.shots.get(&url) {
                    let w = ((h as f32 * src.w as f32 / src.h.max(1) as f32).round() as usize).max(1);
                    let img = src.resized(w, h);
                    self.shot_px.insert(key.clone(), img);
                }
            }
            match self.shot_px.get(&key) {
                Some(img) => {
                    s.blit(img, cx.round() as i32, y.round() as i32, 255, 255);
                    cx += img.w as f32 + gap;
                }
                None => {
                    let w = ui.pt(SHOT_H * 9.0 / 16.0);
                    s.fill_rrect(cx, y, w, h as f32, ui.pt(12.0), fill());
                    cx += w + gap;
                }
            }
            if cx > self.w as f32 {
                break;
            }
        }
    }

    /// The Version History page: every release, newest first.
    fn draw_history(&mut self, s: &mut Surface, ui: &Ui) {
        let Some(page) = self.page.as_ref() else { return };
        let hist = page.hist.clone();
        let (top, bottom) = (self.top(ui) as usize, self.bottom(ui) as usize);
        let mut c = s.rows(top, bottom);
        let (x, w) = (ui.pt(MARGIN), self.card_w(ui));
        let today = today_days();
        let mut y = self.top(ui) - self.scroll.offset() + ui.pt(24.0);
        for (version, date, lines) in &hist {
            c.fill_rect(x as usize, y.max(0.0) as usize, w as usize, 1, sep());
            self.put(&mut c, ui, &format!("Version {version}"), 17.0, Weight::Semibold, label(), false, x, y + ui.pt(22.0));
            self.put(&mut c, ui, &api::relative_date(date, today), 15.0, Weight::Regular, SECONDARY, true, x + w, y + ui.pt(22.0));
            y += ui.pt(40.0);
            for line in lines {
                self.put(&mut c, ui, line, 17.0, Weight::Regular, label(), false, x, y + ui.pt(8.0));
                y += ui.pt(22.0);
            }
            y += ui.pt(16.0);
        }
    }

    fn draw_bars(&mut self, s: &mut Surface, ui: &Ui) {
        let top = self.top(ui) as usize;
        s.fill_rect(0, 0, self.w as usize, top, bg());
        let mid = if self.desktop { ui.pt(DESKTOP_BAR_PT / 2.0) } else { ui.pt(STATUS_PT + 22.0) };
        if let Some((back_tab, id)) = self.page.as_ref().map(|p| (p.back, p.id.clone())) {
            let history = self.page.as_ref().is_some_and(|p| p.history);
            let prev = if history { self.find(&id).map(|a| a.name.clone()).unwrap_or_default() } else { Some(tab_name(back_tab, self.desktop)).filter(|n| !n.is_empty()).unwrap_or("Back").to_string() };
            let back = format!("\u{2039} {prev}");
            self.put(s, ui, &back, 17.0, Weight::Regular, TINT, false, ui.pt(MARGIN), mid);
            if self.desktop {
                // the band stops dragging over the back control only (the label and a little room either side)
                set_back_control_pt(MARGIN + ui.measure(&back, 17.0, Weight::Regular) / ui.pt(1.0) + 8.0);
            }
            if history {
                let tw = ui.measure("Version History", 17.0, Weight::Semibold);
                self.put(s, ui, "Version History", 17.0, Weight::Semibold, label(), false, (self.w as f32 - tw) / 2.0, mid);
            } else if self.scroll.offset() > ui.pt(60.0) {
                let name = self.find(&id).map(|a| a.name.clone()).unwrap_or_default();
                let tw = ui.measure(&name, 17.0, Weight::Semibold);
                self.put(s, ui, &name, 17.0, Weight::Semibold, label(), false, (self.w as f32 - tw) / 2.0, mid);
            }
        } else if self.desktop {
            set_back_control_pt(0.0);
        }
        if self.page.is_none() && self.tab != Tab::Today && self.scroll.offset() > ui.pt(40.0) {
            let name = tab_name(self.tab, self.desktop);
            let tw = ui.measure(name, 17.0, Weight::Semibold);
            self.put(s, ui, name, 17.0, Weight::Semibold, label(), false, (self.w as f32 - tw) / 2.0, mid);
            s.fill_rect(0, top.saturating_sub(1), self.w as usize, 1, sep());
        }
        if self.desktop { return; }
        // tab bar
        let by = self.bottom(ui);
        let bh = ui.pt(TAB_BAR);
        s.fill_rect(0, by as usize, self.w as usize, (bh + home_zone(ui)) as usize + 1, bar_bg());
        s.fill_rect(0, by as usize, self.w as usize, 1, sep());
        let cw = self.w as f32 / TABS.len() as f32;
        let current = if self.page.is_some() { self.page.as_ref().map(|p| p.back).unwrap_or(self.tab) } else { self.tab };
        let badge = self.update_count();
        for (i, (tab, name)) in TABS.iter().enumerate() {
            let col = if *tab == current { TINT } else { SECONDARY };
            let cx = cw * i as f32 + cw / 2.0;
            let cy = by + ui.pt(17.0);
            tab_glyph(s, ui, *tab, (cx, cy), col);
            let lw = ui.measure(name, 10.0, Weight::Medium);
            self.put(s, ui, name, 10.0, Weight::Medium, col, false, cx - lw / 2.0, by + ui.pt(39.0));
            if *tab == Tab::Updates && badge > 0 {
                s.fill_rrect(cx + ui.pt(8.0), by + ui.pt(2.0), ui.pt(18.0), ui.pt(18.0), CAPSULE_RADIUS, super::ui::SYSTEM_RED);
                let n = badge.min(99).to_string();
                let nw = ui.measure(&n, 12.0, Weight::Semibold);
                self.put(s, ui, &n, 12.0, Weight::Semibold, WHITE, false, cx + ui.pt(17.0) - nw / 2.0, by + ui.pt(11.0));
            }
        }
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        if self.desktop {
            let mut image = Image::new(self.w as usize, self.h as usize);
            let mut pane = Surface { w: image.w, h: image.h, stride: image.w, px: &mut image.px, y0: 0 };
            self.draw_pane(&mut pane, ui);
            sidebar_fill(s, self.sidebar as usize, self.h as usize);
            s.blit(&image, self.sidebar, 0, 255, 255);
            self.draw_sidebar(s, ui);
            s.fill_rect(self.sidebar as usize - 1, 0, 1, self.h as usize, sep());
        } else { self.draw_pane(s, ui); }
        let _ = t;
    }

    /// The desktop sidebar: the search capsule (it shows the query), the sections with the Updates count, and the
    /// local account's chip at the foot.
    fn draw_sidebar(&mut self, s: &mut Surface, ui: &Ui) {
        use super::ui::{MAC_BODY_PT, MAC_FOOTNOTE_PT, SIDEBAR_SELECTED};
        let (fx, fy, fw, fh) = self.side_search_rect(ui);
        // search fields are capsules
        s.fill_rrect(fx, fy, fw, fh, fh / 2.0, fill());
        let (gx, gy) = (fx + ui.pt(14.0), fy + fh / 2.0);
        let u = ui.pt(1.0);
        let b = (gx - 8.0 * u, gy - 8.0 * u, 16.0 * u, 16.0 * u);
        paint(s, b, SECONDARY, ring((gx - 1.0 * u, gy - 1.0 * u), 4.5 * u, 1.4 * u));
        paint(s, b, SECONDARY, segment((gx + 2.5 * u, gy + 2.5 * u), (gx + 6.0 * u, gy + 6.0 * u), 1.6 * u));
        let searching = self.tab == Tab::Search && self.page.is_none();
        let (text, ink) = if self.query.is_empty() { ("Search".to_string(), SECONDARY) } else { (self.query.clone(), label()) };
        let room = fw - ui.pt(36.0);
        let text = ui.fit(&text, MAC_BODY_PT, room);
        let shown = if searching { format!("{text}|") } else { text };
        self.put(s, ui, &shown, MAC_BODY_PT, Weight::Regular, ink, false, fx + ui.pt(26.0), gy);
        let current = self.page.as_ref().map_or(self.tab, |p| p.back);
        let badge = self.update_count();
        for (i, (tab, name)) in SIDEBAR.iter().enumerate() {
            let (x, y, w, h) = self.side_row_rect(ui, i);
            let on = *tab == current;
            if on {
                s.fill_rrect(x, y, w, h, ui.pt(9.0), SIDEBAR_SELECTED);
            }
            let mid = y + h / 2.0;
            side_glyph(s, ui, *tab, (x + ui.pt(16.0), mid), if on { WHITE } else { TINT });
            self.put(s, ui, name, MAC_BODY_PT, Weight::Regular, if on { WHITE } else { label() }, false, x + ui.pt(34.0), mid);
            if *tab == Tab::Updates && badge > 0 {
                let n = badge.min(99).to_string();
                self.put(s, ui, &n, MAC_FOOTNOTE_PT, Weight::Semibold, if on { WHITE } else { SECONDARY }, true, x + w - ui.pt(10.0), mid);
            }
        }
        // the local account, as the reference's account chip: initials on a disc, then the name
        let (ax, ay, aw, ah) = self.side_account_rect(ui);
        s.fill_rect(ax as usize, ay as usize, aw as usize, 1, sep());
        let r = ui.pt(14.0);
        let (cx, cy) = (ax + ui.pt(6.0) + r, ay + ah / 2.0);
        s.fill_rrect(cx - r, cy - r, 2.0 * r, 2.0 * r, r, SECONDARY);
        let account = self.account.clone();
        let letters = initials(&account);
        let lw = ui.measure(&letters, MAC_FOOTNOTE_PT, Weight::Semibold);
        self.put(s, ui, &letters, MAC_FOOTNOTE_PT, Weight::Semibold, WHITE, false, cx - lw / 2.0, cy);
        if badge > 0 {
            let d = ui.pt(14.0);
            s.fill_rrect(cx + r - d * 0.6, cy - r - d * 0.3, d, d, d / 2.0, super::ui::SYSTEM_RED);
        }
        let name = ui.fit(&account, MAC_BODY_PT, aw - 2.0 * r - ui.pt(18.0));
        self.put(s, ui, &name, MAC_BODY_PT, Weight::Semibold, label(), false, cx + r + ui.pt(10.0), cy);
    }

    fn draw_pane(&mut self, s: &mut Surface, ui: &Ui) {
        self.dirty = false;
        s.fill_rect(0, 0, self.w as usize, self.h as usize, bg());
        if self.page.is_some() {
            self.draw_page(s, ui);
        } else {
            self.draw_content(s, ui);
        }
        self.draw_bars(s, ui);
        if self.sheet.is_some() {
            self.draw_sheet(s, ui);
        }
        if self.review.is_some() {
            self.draw_review(s, ui);
        }
    }

    /// The purchase sheet: a card with Buy and Cancel, or the passcode keypad once Buy asked for it.
    fn draw_sheet(&mut self, s: &mut Surface, ui: &Ui) {
        let (w, h, t) = (self.w, self.h, self.clock);
        let Some(open) = self.sheet.as_ref() else { return };
        if let Some(pad) = open.keypad() {
            pad.draw(s, ui, w, h, MATERIAL_HUD_SOLID.now(), t);
            return;
        }
        let title = format!("Buy {}", open.name);
        let price = open.purchase.price.display();
        let message = if open.message.is_empty() { "Confirm to buy this app.".to_string() } else { open.message.clone() };
        let (cx, cy, cw, ch) = sheet::sheet_card(ui, w, h);
        let (buy, cancel) = sheet_buttons(ui, w, h);
        s.fill_rect(0, 0, w as usize, h as usize, rgba(0, 0, 0, 110));
        s.fill_rrect(cx, cy, cw, ch, ui.pt(13.0), cell());
        let mid = cx + cw / 2.0;
        for (text, size, weight, color, y) in [
            (title.as_str(), 17.0, Weight::Semibold, label(), cy + ui.pt(40.0)),
            (price.as_str(), 22.0, Weight::Bold, label(), cy + ui.pt(84.0)),
            (message.as_str(), 13.0, Weight::Regular, SECONDARY, cy + ui.pt(112.0)),
        ] {
            let tw = ui.measure(text, size, weight);
            self.put(s, ui, text, size, weight, color, false, mid - tw / 2.0, y);
        }
        s.fill_rrect(buy.0, buy.1, buy.2, buy.3, CAPSULE_RADIUS, TINT);
        s.fill_rrect(cancel.0, cancel.1, cancel.2, cancel.3, CAPSULE_RADIUS, fill());
        for (text, r, ink) in [("Buy", buy, WHITE), ("Cancel", cancel, TINT)] {
            let tw = ui.measure(text, 17.0, Weight::Semibold);
            self.put(s, ui, text, 17.0, Weight::Semibold, ink, false, r.0 + (r.2 - tw) / 2.0, r.1 + r.3 / 2.0);
        }
    }
}

/// `text` wrapped to at most `max_lines` lines of `w` px. When it does not fit, the last line ends at a whole word
/// with an ellipsis, so a description is never cut in the middle of a sentence without saying so.
fn wrap_clamped(ui: &Ui, text: &str, pt: f32, weight: Weight, w: f32, max_lines: usize) -> Vec<String> {
    let mut lines = ui.wrap(text, pt, weight, w);
    if lines.len() <= max_lines || max_lines == 0 {
        return lines;
    }
    lines.truncate(max_lines);
    let last = lines.pop().unwrap_or_default();
    let mut words: Vec<&str> = last.split_whitespace().collect();
    loop {
        let kept = words.join(" ");
        let candidate = format!("{}\u{2026}", kept.trim_end_matches([',', ';', ':', '-', '\u{2013}']));
        if words.is_empty() || ui.measure(&candidate, pt, weight) <= w {
            lines.push(candidate);
            return lines;
        }
        words.pop();
    }
}

/// Each note wrapped on its own, bulleted, so notes stay separate lines.
fn wrap_notes(ui: &Ui, notes: &[String], w: f32) -> Vec<String> {
    notes.iter().flat_map(|n| ui.wrap(&format!("\u{2022} {n}"), 17.0, Weight::Regular, w)).collect()
}

fn channels(c: u32) -> [u32; 3] {
    [c & 0xff, (c >> 8) & 0xff, (c >> 16) & 0xff]
}

fn shade(c: u32, f: f32) -> u32 {
    let [r, g, b] = channels(c).map(|v| (v as f32 * f) as u8);
    pack(r, g, b, 255)
}

fn tint_toward_white(c: u32, f: f32) -> u32 {
    let [r, g, b] = channels(c).map(|v| (v as f32 + (255.0 - v as f32) * f) as u8);
    pack(r, g, b, 255)
}

fn today_days() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64 / 86_400).unwrap_or(0)
}

/// The Today tab's date line, e.g. "MONDAY 5 OCTOBER".
fn date_label() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (_, m, d) = crate::bare::timezone::civil_from_days(secs.div_euclid(86_400));
    const MONTHS: [&str; 12] = ["JANUARY", "FEBRUARY", "MARCH", "APRIL", "MAY", "JUNE", "JULY", "AUGUST", "SEPTEMBER", "OCTOBER", "NOVEMBER", "DECEMBER"];
    const DAYS: [&str; 7] = ["MONDAY", "TUESDAY", "WEDNESDAY", "THURSDAY", "FRIDAY", "SATURDAY", "SUNDAY"];
    let weekday = (crate::bare::timezone::weekday(secs.div_euclid(86_400)) as usize + 6) % 7;
    format!("{} {} {}", DAYS[weekday], d, MONTHS[(m - 1) as usize])
}

/// The third line of a store row: category, the version the store has, and the download size.
pub fn row_detail(category: &str, version: &str, bytes: u64) -> String {
    format!("{category} \u{b7} v{version} \u{b7} {}", size_text(bytes))
}

pub fn size_text(bytes: u64) -> String {
    if bytes == 0 { "-".into() } else { bifrost_ui::format::byte_count(bytes) }
}

/// A desktop sidebar glyph: the tab bar's glyph at two thirds of its size, drawn about `(cx, cy)`.
fn side_glyph(s: &mut Surface, ui: &Ui, tab: Tab, (cx, cy): (f32, f32), color: u32) {
    let u = ui.pt(1.0) * 0.66;
    let b = (cx - 14.0 * u, cy - 14.0 * u, 28.0 * u, 28.0 * u);
    let t = 2.6 * u;
    match tab {
        // a star of four strokes: Discover
        Tab::Today => {
            for (dx, dy) in [(0.0, 11.0), (11.0, 0.0), (8.0, 8.0), (8.0, -8.0)] {
                paint(s, b, color, segment((cx - dx * u, cy - dy * u), (cx + dx * u, cy + dy * u), t));
            }
        }
        // a pencil: Create
        Tab::Shelf(Shelf::Create) => {
            paint(s, b, color, segment((cx - 9.0 * u, cy + 9.0 * u), (cx + 7.0 * u, cy - 7.0 * u), t + 1.6 * u));
            paint(s, b, color, segment((cx - 11.0 * u, cy + 11.0 * u), (cx - 8.0 * u, cy + 8.0 * u), t));
        }
        // a paper plane: Work
        Tab::Shelf(Shelf::Work) => {
            let (a, c, d) = ((cx - 11.0 * u, cy), (cx + 11.0 * u, cy - 10.0 * u), (cx + 1.0 * u, cy + 11.0 * u));
            paint(s, b, color, segment(a, c, t));
            paint(s, b, color, segment(c, d, t));
            paint(s, b, color, segment(d, (cx - 1.0 * u, cy + 1.0 * u), t));
            paint(s, b, color, segment((cx - 1.0 * u, cy + 1.0 * u), a, t));
        }
        // a game controller's stick: Play
        Tab::Shelf(Shelf::Play) => tab_glyph_scaled(s, ui, Tab::Games, (cx, cy), color, 0.66),
        // angle brackets: Develop
        Tab::Shelf(Shelf::Develop) => {
            paint(s, b, color, segment((cx - 4.0 * u, cy - 9.0 * u), (cx - 11.0 * u, cy), t));
            paint(s, b, color, segment((cx - 11.0 * u, cy), (cx - 4.0 * u, cy + 9.0 * u), t));
            paint(s, b, color, segment((cx + 4.0 * u, cy - 9.0 * u), (cx + 11.0 * u, cy), t));
            paint(s, b, color, segment((cx + 11.0 * u, cy), (cx + 4.0 * u, cy + 9.0 * u), t));
        }
        // a grid of four tiles: Categories
        Tab::Categories => {
            for (dx, dy) in [(-11.0, -11.0), (2.0, -11.0), (-11.0, 2.0), (2.0, 2.0)] {
                s.stroke_rrect(cx + dx * u, cy + dy * u, 9.0 * u, 9.0 * u, 2.0 * u, t * 0.8, color);
            }
        }
        other => tab_glyph_scaled(s, ui, other, (cx, cy), color, 0.66),
    }
}

fn tab_glyph(s: &mut Surface, ui: &Ui, tab: Tab, centre: (f32, f32), color: u32) {
    tab_glyph_scaled(s, ui, tab, centre, color, 1.0);
}

/// The tab bar's glyph for `tab` about `(cx, cy)`, `k` times its size.
fn tab_glyph_scaled(s: &mut Surface, ui: &Ui, tab: Tab, (cx, cy): (f32, f32), color: u32, k: f32) {
    let u = ui.pt(1.0) * k;
    let b = (cx - 14.0 * u, cy - 14.0 * u, 28.0 * u, 28.0 * u);
    let t = 2.2 * u;
    match tab {
        Tab::Today => {
            s.stroke_rrect(cx - 10.0 * u, cy - 11.0 * u, 20.0 * u, 22.0 * u, 4.0 * u, t, color);
            paint(s, b, color, segment((cx - 5.0 * u, cy - 3.0 * u), (cx + 5.0 * u, cy - 3.0 * u), t));
            paint(s, b, color, segment((cx - 5.0 * u, cy + 3.0 * u), (cx + 2.0 * u, cy + 3.0 * u), t));
        }
        Tab::Apps => {
            s.stroke_rrect(cx - 11.0 * u, cy - 11.0 * u, 15.0 * u, 15.0 * u, 3.0 * u, t, color);
            s.stroke_rrect(cx - 4.0 * u, cy - 4.0 * u, 15.0 * u, 15.0 * u, 3.0 * u, t, color);
        }
        Tab::Games => {
            paint(s, b, color, ring((cx, cy), 10.0 * u, t));
            paint(s, b, color, disc((cx, cy), 2.6 * u));
            paint(s, b, color, segment((cx, cy - 10.0 * u), (cx, cy - 5.0 * u), t));
        }
        Tab::Updates => {
            s.stroke_rrect(cx - 11.0 * u, cy - 11.0 * u, 22.0 * u, 22.0 * u, 5.0 * u, t, color);
            paint(s, b, color, segment((cx, cy - 6.0 * u), (cx, cy + 5.0 * u), t));
            paint(s, b, color, segment((cx - 5.0 * u, cy), (cx, cy + 5.5 * u), t));
            paint(s, b, color, segment((cx + 5.0 * u, cy), (cx, cy + 5.5 * u), t));
        }
        Tab::Search => {
            paint(s, b, color, ring((cx - 2.0 * u, cy - 2.0 * u), 8.0 * u, t));
            paint(s, b, color, segment((cx + 4.0 * u, cy + 4.0 * u), (cx + 11.0 * u, cy + 11.0 * u), t + 0.6 * u));
        }
        // the desktop-only sections are drawn by `side_glyph`; in the tab bar they would read as Apps
        Tab::Shelf(_) | Tab::Categories => {
            s.stroke_rrect(cx - 11.0 * u, cy - 11.0 * u, 15.0 * u, 15.0 * u, 3.0 * u, t, color);
            s.stroke_rrect(cx - 4.0 * u, cy - 4.0 * u, 15.0 * u, 15.0 * u, 3.0 * u, t, color);
        }
    }
}

/// Procedural app icon: the App Store's blue tile with a white "A" of three strokes.
pub fn icon(size: usize) -> Image {
    let mut image = Image { w: size, h: size, px: vec![0; size * size] };
    let (top, bottom) = ((26, 197, 253), (28, 98, 240));
    for y in 0..size {
        let k = y as f32 / size.max(1) as f32;
        let mix = |a: i32, b: i32| (a as f32 + (b - a) as f32 * k) as u8;
        image.px[y * size..(y + 1) * size].fill(pack(mix(top.0, bottom.0), mix(top.1, bottom.1), mix(top.2, bottom.2), 255));
    }
    let mut s = Surface { w: size, h: size, stride: size, px: &mut image.px, y0: 0 };
    let f = size as f32;
    let b = (0.0, 0.0, f, f);
    let white = pack(255, 255, 255, 255);
    let t = f * 0.075;
    paint(&mut s, b, white, segment((f * 0.5, f * 0.2), (f * 0.25, f * 0.72), t));
    paint(&mut s, b, white, segment((f * 0.5, f * 0.2), (f * 0.75, f * 0.72), t));
    paint(&mut s, b, white, segment((f * 0.3, f * 0.6), (f * 0.7, f * 0.6), t));
    image
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::store::api::parse_catalog;

    const SUM: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn a_description_that_does_not_fit_ends_in_an_ellipsis_at_a_word() {
        let ui = crate::shell::tests::ui();
        let text = "A fast, private web browser powered by Servo, with its own BifrOSt profile";
        let w = ui.measure("A fast, private web browser", 17.0, Weight::Regular) + 4.0;
        let lines = wrap_clamped(&ui, text, 17.0, Weight::Regular, w, 2);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].ends_with('\u{2026}'), "{lines:?}");
        assert!(ui.measure(&lines[1], 17.0, Weight::Regular) <= w);
        let kept = lines[1].trim_end_matches('\u{2026}');
        assert!(text.contains(kept) && !kept.ends_with(' '), "cut at a word boundary: {kept:?}");
        let all = wrap_clamped(&ui, text, 17.0, Weight::Regular, w, 4);
        assert!(!all.last().unwrap().ends_with('\u{2026}'), "text that fits is untouched");
        assert_eq!(all.join(" "), text);
    }

    #[test]
    fn a_package_for_another_sdk_protocol_shows_needs_update_not_get_or_open() {
        let mk = |p: u32, v: &str, kind: &str| StoreApp { id: "a".into(), version: v.into(), sdk_protocol: p, kind: kind.into(), ..Default::default() };
        assert_eq!(button_for(&mk(1, "1.0.0", "app"), None, 2), Btn::NeedsUpdate, "no GET for an app that cannot run");
        assert_eq!(button_for(&mk(1, "1.0.0", "app"), Some("1.0.0"), 2), Btn::NeedsUpdate, "no OPEN for an installed one either");
        assert_eq!(button_for(&mk(2, "1.1.0", "app"), Some("1.0.0"), 2), Btn::Update, "a compatible newer version is an update");
        assert_eq!(button_for(&mk(1, "1.1.0", "app"), Some("1.0.0"), 2), Btn::NeedsUpdate, "a newer incompatible one is no update");
        assert_eq!(button_for(&mk(2, "1.0.0", "app"), None, 2), Btn::Get);
        assert_eq!(button_for(&mk(0, "1.0.0", "app"), Some("1.0.0"), 2), Btn::Open, "unknown protocol: assume it runs");
    }

    #[test]
    fn a_failed_icon_is_retried_after_a_minute_not_never() {
        assert!(icon_due(None, 0.0));
        assert!(!icon_due(Some(1_000.0), 1_000.0 + ICON_RETRY_MS - 1.0));
        assert!(icon_due(Some(1_000.0), 1_000.0 + ICON_RETRY_MS));
    }

    /// E2E10 finding 10 and E2E11 9: Discover's Essential apps drew F and R tiles for Files and Remote, first for
    /// good, then for seconds while a download ran although the icon was on disk.
    #[test]
    fn an_icon_comes_from_the_install_then_the_cache_and_the_network_only_when_neither_has_it() {
        let png: &[u8] = include_bytes!("../../../apps/files/icon.png");
        let dir = std::env::temp_dir().join(format!("apps-store-icons-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cache = icon_cache_path(&dir, "is.o/files");
        assert_eq!(cache, dir.join("icon-cache").join("is.o_files.png"), "an id cannot leave the cache folder");
        let broken = dir.join("icon.png");
        std::fs::write(&broken, b"not a png").unwrap();
        assert!(local_icon(Some(broken.as_path()), &cache).is_none(), "a damaged install and no cache: fetch it");
        assert!(fetch_icon(|| Some(png.to_vec()), &cache).is_some(), "the catalogue's icon is used");
        assert_eq!(std::fs::read(&cache).unwrap(), png, "the downloaded icon is kept");
        assert_eq!(local_icon(Some(broken.as_path()), &cache).map(|(_, c)| c), Some(true), "a damaged installed icon falls through to the cache");
        assert_eq!(local_icon(None, &cache).map(|(_, c)| c), Some(true), "the cached icon is shown without the network");
        assert!(fetch_icon(|| Some(b"junk".to_vec()), &cache).is_none(), "an undecodable download is no icon");
        assert_eq!(std::fs::read(&cache).unwrap(), png, "and does not replace the cached one");
        assert!(fetch_icon(|| None, &cache).is_none(), "a failed download is no icon");
        let good = dir.join("installed.png");
        std::fs::write(&good, png).unwrap();
        std::fs::remove_file(&cache).unwrap();
        assert_eq!(local_icon(Some(good.as_path()), &cache).map(|(_, c)| c), Some(false), "an installed icon needs no cache");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// E2E11 9 and E2E12: an icon already on disk is decoded on a worker, not on the tick (the UI thread), and is in
    /// place as soon as the worker answers, before any download; an app with no icon anywhere waits for the retry
    /// instead of being asked for again every tick.
    #[test]
    fn a_cached_icon_is_decoded_off_the_tick_and_painted_without_a_download() {
        let png: &[u8] = include_bytes!("../../../apps/files/icon.png");
        let mut s = store();
        s.pool = Pool::new(1).ok();
        let dir = std::env::temp_dir().join(format!("apps-store-first-tick-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        s.root = dir.clone();
        s.installed.clear();
        let cache = icon_cache_path(&dir, "is.o.calc");
        std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
        std::fs::write(&cache, png).unwrap();
        s.fetching = true; // no catalogue refresh in this test
        s.maintain(0.0);
        assert!(s.icons.get("is.o.calc").is_none(), "the tick does not read or decode the PNG");
        assert!(s.icon_pending.contains("is.o.calc") && s.icon_pending.contains("is.o.chess"), "the worker does");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !s.icon_pending.is_empty() && std::time::Instant::now() < deadline {
            s.pump();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(matches!(s.icons.get("is.o.calc"), Some(Some(_))), "the cached icon is painted once the worker answers");
        assert!(!s.icon_pending.contains("is.o.calc"), "and nothing is downloaded for it (no catalogue URL)");
        assert!(s.icons.get("is.o.chess").is_none() && !s.icon_pending.contains("is.o.chess"));
        assert_eq!(s.icon_failed.get("is.o.chess"), Some(&0.0), "no icon on disk and no URL: retried later");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_workers_the_tick_reads_no_icon_and_waits_for_the_retry() {
        let mut s = store();
        s.fetching = true;
        s.maintain(0.0);
        assert!(s.icons.get("is.o.calc").is_none() && !s.icon_pending.contains("is.o.calc"));
        assert_eq!(s.icon_failed.get("is.o.calc"), Some(&0.0));
    }

    /// A cached icon is painted first and then fetched from the catalogue once more, so an icon the catalogue
    /// changed shows; it is revalidated once per session, and a failed revalidation keeps the cached icon.
    #[test]
    fn a_cached_icon_is_revalidated_once_after_it_is_painted() {
        let png: &[u8] = include_bytes!("../../../apps/files/icon.png");
        let img = crate::cg::imageio::decode(png).unwrap();
        let mut s = store(); // no workers: the revalidation is decided but not sent
        let cache = PathBuf::from("/nonexistent/icon.png");
        s.icon_pending.insert("is.o.calc".into());
        s.local_icon_done("is.o.calc".into(), Some(img.clone()), true, "/store/calc.png".into(), cache.clone());
        assert!(matches!(s.icons.get("is.o.calc"), Some(Some(_))), "the cached copy is painted first");
        assert!(s.icon_revalidated.contains("is.o.calc"), "then fetched again from the catalogue");
        assert!(!s.icon_pending.contains("is.o.calc"));
        // an installed icon, or a cached one with no catalogue URL, is not fetched
        s.local_icon_done("is.o.chess".into(), Some(img.clone()), false, "/store/chess.png".into(), cache.clone());
        s.local_icon_done("is.o.notes".into(), Some(img), true, String::new(), cache);
        assert!(!s.icon_revalidated.contains("is.o.chess") && !s.icon_revalidated.contains("is.o.notes"));
        // the revalidation failing keeps the painted icon and schedules no retry (which would wake the store)
        s.icon_pending.insert("is.o.calc".into());
        s.pool = Pool::new(1).ok();
        s.pool.as_ref().unwrap().spawn(|| Done::Icon { id: "is.o.calc".into(), image: None });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while s.icon_pending.contains("is.o.calc") && std::time::Instant::now() < deadline {
            s.pump();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(matches!(s.icons.get("is.o.calc"), Some(Some(_))));
        assert!(!s.icon_failed.contains_key("is.o.calc"));
    }

    #[test]
    fn a_failed_icon_wakes_the_store_when_its_retry_is_due() {
        let mut s = store();
        assert_eq!(s.next_wake(0.0), None);
        s.icon_failed.insert("is.o.calc".into(), 1_000.0);
        assert_eq!(s.next_wake(2_000.0), Some(1_000.0 + ICON_RETRY_MS));
    }

    #[test]
    fn an_icon_whose_retry_is_in_flight_does_not_wake_the_store() {
        let mut s = store();
        s.icon_failed.insert("is.o.calc".into(), 1_000.0);
        s.icon_pending.insert("is.o.calc".into());
        assert_eq!(s.next_wake(1_000.0 + ICON_RETRY_MS + 5_000.0), None, "the pending request's reply wakes it instead");
        s.icon_failed.insert("is.o.chess".into(), 3_000.0);
        assert_eq!(s.next_wake(4_000.0), Some(3_000.0 + ICON_RETRY_MS), "another failed icon still schedules its retry");
    }

    fn store() -> AppStore {
        let ui = crate::shell::tests::ui();
        let mut s = AppStore::new(&ui, 1000, 1400, std::env::temp_dir().join(format!("apps-store-ui-{}", std::process::id())));
        s.pool = None;
        s.catalog = parse_catalog(&format!(
            r#"{{"apps":[{{"id":"is.o.calc","name":"Calc","version":"1.1.0","sha256":"{SUM}","package":"/p","size":2000000}},{{"id":"is.o.chess","name":"Chess","version":"1.0.0","sha256":"{SUM}","package":"/q","category":"Games"}}]}}"#
        ))
        .unwrap();
        s
    }

    fn local(id: &str, v: &str) -> InstalledApp {
        InstalledApp { id: id.into(), name: id.into(), version: v.into(), ..Default::default() }
    }

    #[test]
    fn a_new_window_starts_on_the_front_page() {
        let mut s = store();
        s.tab = Tab::Apps;
        s.category = Some(0);
        s.confirm_remove = Some("is.o.calc".into());
        s.page = Some(Page { id: "is.o.calc".into(), back: Tab::Apps, desc: Vec::new(), list_scroll: 0.0, whatsnew: Vec::new(), history: false, hist: Vec::new() });
        s.show_front_page();
        assert_eq!(s.tab, Tab::Today);
        assert!(s.page.is_none() && s.category.is_none() && s.confirm_remove.is_none());
    }

    #[test]
    fn update_rows_show_two_note_lines_until_more_is_tapped() {
        let mut s = store();
        s.catalog[0].changelog = vec![api::Release { version: "1.1.0".into(), date: "2026-10-05".into(), notes: vec!["a".into(), "b".into(), "c".into()] }];
        s.installed = vec![local("is.o.calc", "1.0.0")];
        s.tab = Tab::Updates;
        let find = |s: &AppStore| s.rows().into_iter().find_map(|r| if let Row::Update(i, n, m) = r { Some((i, n, m)) } else { None });
        assert_eq!(find(&s), Some((0, 2, true)));
        s.expanded.insert("is.o.calc".into());
        assert_eq!(find(&s), Some((0, 3, false)));
        assert!(Row::Update(0, 3, false).height() > Row::Update(0, 2, true).height() && Row::Update(0, 0, false).height() == ROW);
    }

    fn with_background_packages() -> AppStore {
        let mut s = store();
        for (id, kind) in [("is.o.update", "service"), ("is.o.audio", "service"), ("is.o.kb", "system"), ("is.o.files", "system-app")] {
            s.catalog.push(StoreApp { id: id.into(), name: id.into(), version: "2.0.0".into(), min_shell: "0.0.0".into(), kind: kind.into(), category: "System".into(), ..Default::default() });
        }
        s
    }

    fn listed(s: &AppStore) -> Vec<String> {
        s.rows().into_iter().filter_map(|r| if let Row::App(i) | Row::Update(i, ..) = r { Some(s.catalog[i].id.clone()) } else { None }).collect()
    }

    #[test]
    fn services_are_hidden_from_every_tab_and_the_badge() {
        let mut s = with_background_packages();
        s.installed = vec![InstalledApp { kind: "service".into(), ..local("is.o.update", "1.0.0") }, local("is.o.audio", "1.0.0")];
        for tab in [Tab::Today, Tab::Games, Tab::Apps, Tab::Updates, Tab::Search, Tab::Categories, Tab::Shelf(Shelf::Develop), Tab::Shelf(Shelf::Play)] {
            s.tab = tab;
            let ids = listed(&s);
            assert!(!ids.iter().any(|i| i == "is.o.update" || i == "is.o.audio"), "{tab:?} lists a service");
        }
        assert_eq!(s.update_count(), 0);
        let plan = api::system_plan(&s.catalog, &[], "9.9.9", &[]);
        assert!(plan.iter().any(|a| a.id == "is.o.update") && plan.iter().any(|a| a.id == "is.o.audio"));
        assert!(api::updates(&s.catalog, &s.installed).is_empty());
    }

    #[test]
    fn background_packages_never_offer_get() {
        let mut s = with_background_packages();
        for a in s.catalog.clone().iter().filter(|a| a.is_preinstalled()) {
            assert_ne!(s.button(a), Btn::Get, "{}", a.id);
        }
        let files = s.catalog.iter().find(|a| a.id == "is.o.files").unwrap().clone();
        s.installed.push(InstalledApp { kind: "system-app".into(), ..local("is.o.files", "2.0.0") });
        assert_eq!(s.button(&files), Btn::Open);
        s.tab = Tab::Apps;
        assert!(listed(&s).contains(&"is.o.kb".to_string()));
    }

    #[test]
    fn button_follows_install_state() {
        let mut s = store();
        let calc = s.catalog[0].clone();
        assert_eq!(s.button(&calc), Btn::Get);
        s.installed.push(local("is.o.calc", "1.0.0"));
        assert_eq!(s.button(&calc), Btn::Update);
        s.installed[0].version = "1.1.0".into();
        assert_eq!(s.button(&calc), Btn::Open);
        s.jobs.insert(calc.id.clone(), Job { done: Arc::new(AtomicU64::new(500_000)), size: 2_000_000, cancel: Arc::new(AtomicBool::new(false)) });
        assert_eq!(s.button(&calc), Btn::Progress(0.25));
    }

    #[test]
    fn tabs_split_apps_and_games_and_updates_list_only_newer() {
        let mut s = store();
        s.tab = Tab::Games;
        assert!(s.rows().iter().any(|r| matches!(r, Row::App(1))) && !s.rows().iter().any(|r| matches!(r, Row::App(0))));
        s.tab = Tab::Updates;
        assert!(s.rows().iter().any(|r| matches!(r, Row::Note(n) if n.contains("up to date"))));
        s.installed.push(local("is.o.calc", "1.0.0"));
        assert_eq!(s.update_list(), vec![0]);
        assert!(s.rows().contains(&Row::Header("Available Updates".into(), true)));
    }

    #[test]
    fn open_hands_the_installed_app_to_the_host() {
        let mut s = store();
        s.installed.push(local("is.o.calc", "1.1.0"));
        s.press_button("is.o.calc");
        assert_eq!(s.take_launch().map(|a| a.id), Some("is.o.calc".to_string()));
        assert!(s.take_launch().is_none());
    }

    #[test]
    fn icon_is_opaque_and_sizes_read_well() {
        assert!(icon(64).px.iter().all(|p| p >> 24 == 255));
        assert_eq!((size_text(0), size_text(2_500_000), size_text(900)), ("-".to_string(), "2.5 MB".to_string(), "900 bytes".to_string()));
        assert_eq!(row_detail("Productivity", "1.7.0", 2_500_000), "Productivity \u{b7} v1.7.0 \u{b7} 2.5 MB", "every row names the version the store has");
        assert_eq!(crate::bare::timezone::civil_from_days(0), (1970, 1, 1));
    }

    fn with_system() -> AppStore {
        let mut s = store();
        s.catalog.push(StoreApp { id: "is.o.kb".into(), name: "Keyboard".into(), version: "1.1.0".into(), kind: "system".into(), category: "System".into(), size: 11_000_000, ..Default::default() });
        s
    }

    #[test]
    fn apps_tab_lists_system_components_in_their_own_section() {
        let mut s = with_system();
        s.tab = Tab::Apps;
        let rows = s.rows();
        let at = rows.iter().position(|r| *r == Row::Header("System".into(), false)).expect("System header");
        assert_eq!(rows[at + 1..], [Row::App(2)]);
        assert!(!rows[..at].contains(&Row::App(2)), "not among Top Apps");
        s.tab = Tab::Games;
        assert!(!s.rows().contains(&Row::App(2)));
        s.tab = Tab::Today;
        assert!(!s.rows().contains(&Row::App(2)) && !s.rows().iter().any(|r| matches!(r, Row::Cards(c) if c.iter().any(|k| k.apps().contains(&2)))));
        s.tab = Tab::Search;
        assert!(!s.rows().contains(&Row::App(2)));
    }

    #[test]
    fn system_components_have_no_get_or_open_only_update_and_count_in_the_badge() {
        let mut s = with_system();
        let kb = s.catalog[2].clone();
        assert_eq!(s.button(&kb), Btn::None, "not installed yet: preinstalled in the background, no GET");
        s.installed.push(InstalledApp { kind: "system".into(), ..local("is.o.kb", "1.1.0") });
        assert_eq!(s.button(&kb), Btn::None, "current: no OPEN");
        s.press_button("is.o.kb");
        assert!(s.take_launch().is_none(), "a system component never opens");
        assert_eq!(s.update_count(), 0);
        s.installed[0].version = "1.0.0".into();
        assert_eq!(s.button(&kb), Btn::Update);
        assert_eq!((s.update_count(), s.update_list()), (1, vec![2]));
        s.installed.push(local("is.o.calc", "1.0.0"));
        assert_eq!(s.update_count(), 2, "apps and system components both count");
        assert_eq!(category_label(&kb), "System Component");
        assert_eq!(category_label(&s.catalog[0]), "Utilities");
    }

    #[test]
    fn purchased_lists_installed_apps_by_purchase_day_and_hides_services() {
        let mut s = store();
        s.installed = vec![
            InstalledApp { purchased: 120, ..local("is.o.chess", "1.0.0") },
            InstalledApp { purchased: 100, ..local("is.o.calc", "1.1.0") },
            InstalledApp { kind: "service".into(), purchased: 90, ..local("is.o.update", "1.0.0") },
            InstalledApp { purchased: 9_999_999, ..local("is.o.later", "1.0.0") },
        ];
        assert_eq!(s.purchased(200), vec![0, 1], "the service and the purchase after today are left out");
        s.tab = Tab::Updates;
        let rows = s.rows();
        let at = rows.iter().position(|r| *r == Row::Header("Purchased".into(), false)).expect("Purchased header");
        assert_eq!(rows[at + 1..at + 3], [Row::App(1), Row::App(0)], "chess (bought first) then calc, both in the catalogue");
        assert_eq!(rows[at + 3], Row::Restore, "Restore Purchases closes the tab");
    }

    #[test]
    fn a_paid_app_opens_the_purchase_sheet_and_installs_nothing_yet() {
        let mut s = store();
        s.catalog[1].price = Some(bifrost_ui::purchase::Price::new(990, 0, "ISK"));
        let chess = s.catalog[1].clone();
        s.press_button(&chess.id);
        assert!(s.jobs.is_empty(), "no download before the customer confirms");
        let open = s.sheet.as_ref().expect("the sheet is up");
        assert_eq!((open.id.as_str(), open.purchase.price.display()), ("is.o.chess", "990 ISK".to_string()));
        assert_eq!(s.button(&chess), Btn::Get);
    }

    #[test]
    fn rating_an_app_keeps_the_review_and_reopens_it_for_editing() {
        let ui = crate::shell::tests::ui();
        let mut s = store();
        let _ = std::fs::remove_file(s.root.join(REVIEWS_FILE));
        s.reviews = bifrost_core::app_review::ReviewBook::new();
        let sidebar = s.sidebar as f32;
        let to_window = |r: (f32, f32, f32, f32)| (r.0 + r.2 / 2.0 + sidebar, r.1 + r.3 / 2.0);
        s.activate(&ui, Target::Rate("is.o.chess".into()));
        assert_eq!(s.review.as_ref().map(|r| r.id.as_str()), Some("is.o.chess"));
        let (x, y) = to_window(review_sheet::rating_rects(&ui, s.w, s.h)[3]);
        s.touch(&ui, Event::Up(x, y), 0.0);
        let (x, y) = to_window(review_sheet::body_rect(&ui, s.w, s.h));
        s.touch(&ui, Event::Up(x, y), 0.0);
        assert!(s.take_keyboard_request(), "a tap on a field raises the keyboard");
        s.key(Out::Text("Fun to play".into()));
        let (x, y) = to_window(review_sheet::buttons(&ui, s.w, s.h).0);
        s.touch(&ui, Event::Up(x, y), 0.0);
        assert!(s.review.is_none(), "Send closes the sheet");
        let kept = s.reviews.get("is.o.chess").expect("the review is in the book");
        assert_eq!((kept.stars, kept.body.as_str()), (4, "Fun to play"));
        // the book is on disk, so a new store opens the sheet with the review to edit
        let again = AppStore::new(&ui, 1000, 1400, s.root.clone());
        assert_eq!(again.reviews.get("is.o.chess"), s.reviews.get("is.o.chess"));
        s.activate(&ui, Target::Rate("is.o.chess".into()));
        assert_eq!(s.review.as_ref().map(|r| (r.stars(), r.body().to_string())), Some((4, "Fun to play".to_string())));
        let (x, y) = to_window(review_sheet::buttons(&ui, s.w, s.h).1);
        s.touch(&ui, Event::Up(x, y), 0.0);
        assert!(s.review.is_none(), "Cancel closes it and keeps the earlier review");
        assert_eq!(s.reviews.get("is.o.chess").map(|r| r.stars), Some(4));
        s.activate(&ui, Target::Rate("is.o.nothing".into()));
        assert!(s.review.is_none(), "an app the catalogue does not list has no sheet");
    }

    #[test]
    fn tapping_buy_records_the_purchase_and_cancel_records_nothing() {
        let ui = crate::shell::tests::ui();
        let mut s = store();
        s.catalog[1].price = Some(bifrost_ui::purchase::Price::new(990, 0, "ISK"));
        let chess = s.catalog[1].clone();
        // the pane sits right of the sidebar on desktop, so window x = sidebar + pane x
        let sidebar = s.sidebar as f32;
        let to_window = |r: (f32, f32, f32, f32)| (r.0 + r.2 / 2.0 + sidebar, r.1 + r.3 / 2.0);
        s.press_button(&chess.id);
        let (cancel_x, cancel_y) = to_window(sheet::sheet_buttons(&ui, s.w, s.h).1);
        s.touch(&ui, Event::Up(cancel_x, cancel_y), 0.0);
        assert!(s.sheet.is_none());
        assert!(!s.bought.is_entitled(&chess.id, today_days()), "a cancelled sheet records nothing");
        s.press_button(&chess.id);
        let (buy_x, buy_y) = to_window(sheet::sheet_buttons(&ui, s.w, s.h).0);
        s.touch(&ui, Event::Up(buy_x, buy_y), 0.0);
        assert!(s.sheet.is_none(), "Buy closes the sheet");
        assert!(s.bought.is_entitled(&chess.id, today_days()), "the purchase is in the ledger");
        assert!(s.jobs.is_empty(), "the test store has no worker pool, so the install is not started");
    }

    #[test]
    fn an_install_request_goes_through_the_coordinator() {
        let mut s = store();
        let calc = s.catalog[0].clone();
        s.start(&calc.id);
        assert!(s.jobs.is_empty(), "no worker pool: the install is not started");
        assert!(s.coord.jobs().is_empty(), "and the coordinator does not keep the failed job");
        let phases: Vec<coordination::Phase> = s
            .coord
            .drain_events()
            .into_iter()
            .filter_map(|e| match e {
                coordination::Event::Phase { phase, .. } => Some(phase),
                _ => None,
            })
            .collect();
        assert_eq!(phases, vec![coordination::Phase::Queued, coordination::Phase::Downloading, coordination::Phase::Failed]);
    }

    #[test]
    fn a_second_install_waits_while_one_downloads() {
        let mut s = store();
        let (calc, chess) = (s.catalog[0].id.clone(), s.catalog[1].id.clone());
        s.coord.request(&calc, coordination::current_uid(), 100).unwrap();
        s.coord.request(&chess, coordination::current_uid(), 100).unwrap();
        s.jobs.insert(calc.clone(), Job { done: Arc::new(AtomicU64::new(0)), size: 100, cancel: Arc::new(AtomicBool::new(false)) });
        s.jobs.insert(chess.clone(), Job { done: Arc::new(AtomicU64::new(0)), size: 100, cancel: Arc::new(AtomicBool::new(false)) });
        assert_eq!(s.coord.start_next().as_deref(), Some(calc.as_str()));
        assert_eq!(s.coord.start_next(), None, "the second download waits its turn");
        // the download counter feeds the coordinator, and the job is still shown with its ring
        s.jobs[&calc].done.store(60, Ordering::Relaxed);
        s.sync_coordinator();
        assert_eq!(s.coord.job(&calc).map(|j| j.done), Some(60));
        assert!(s.jobs.contains_key(&chess), "the waiting install keeps its progress ring");
    }

    #[test]
    fn restore_purchases_reports_the_count_on_the_updates_tab() {
        let ui = crate::shell::tests::ui();
        let mut s = store();
        s.tab = Tab::Updates;
        assert!(s.rows().contains(&Row::Restore));
        s.activate(&ui, Target::Restore);
        assert_eq!(s.restored.as_deref(), Some("No purchases to restore."));
        assert!(s.rows().iter().any(|r| *r == Row::Note("No purchases to restore.".into())));
    }

    #[test]
    fn a_search_matches_every_word_across_name_summary_category_and_id() {
        let mut calc = StoreApp { id: "is.o.calc".into(), name: "Calc".into(), summary: "Adds numbers quickly".into(), category: "Utilities".into(), ..Default::default() };
        assert!(matches_query(&calc, ""), "an empty query matches everything");
        assert!(matches_query(&calc, "CALC") && matches_query(&calc, "numbers utilities"));
        assert!(matches_query(&calc, "o.calc"), "the id is searched too");
        assert!(!matches_query(&calc, "calc chess"), "every word must match");
        calc.summary.clear();
        assert!(!matches_query(&calc, "numbers"));
    }

    #[test]
    fn typing_filters_the_search_tab_and_says_when_nothing_matches() {
        let mut s = store();
        s.tab = Tab::Search;
        assert!(!s.rows().contains(&Row::Search), "on the desktop the query field is the sidebar's");
        s.desktop = false;
        assert!(s.rows().contains(&Row::Search), "the tablet's query field is the first row under the title");
        s.desktop = true;
        s.key(Out::Text("chess".into()));
        assert_eq!(s.query, "chess");
        assert_eq!(listed(&s), vec!["is.o.chess".to_string()]);
        s.key(Out::Backspace);
        s.key(Out::Backspace);
        s.key(Out::Backspace);
        s.key(Out::Backspace);
        s.key(Out::Backspace);
        assert_eq!(listed(&s).len(), 2, "an empty query lists every app again");
        s.key(Out::Text("zzz".into()));
        assert!(s.rows().iter().any(|r| matches!(r, Row::Note(n) if n.starts_with("No results"))));
        s.key(Out::Enter);
        s.key(Out::Named("Up"));
        assert_eq!(s.query, "zzz", "keys other than text and backspace are ignored");
    }

    #[test]
    fn typing_goes_nowhere_off_the_search_tab_or_on_a_product_page() {
        let mut s = store();
        s.tab = Tab::Apps;
        s.key(Out::Text("x".into()));
        assert!(s.query.is_empty(), "the Apps tab has no query field");
        s.tab = Tab::Search;
        s.page = Some(Page { id: "is.o.calc".into(), back: Tab::Search, desc: vec![], list_scroll: 0.0, whatsnew: vec![], history: false, hist: vec![] });
        s.key(Out::Text("x".into()));
        assert!(s.query.is_empty(), "a product page is open");
    }

    #[test]
    fn tapping_the_query_field_asks_for_the_keyboard_once() {
        let ui = crate::shell::tests::ui();
        let mut s = store();
        s.desktop = false;
        s.tab = Tab::Search;
        let mut field_row_y = None;
        s.each_row(&ui, |r, ry, _| {
            if *r == Row::Search {
                field_row_y = Some(ry);
            }
        });
        assert!(field_row_y.is_some(), "the Search row is laid out");
        s.activate(&ui, Target::SearchField);
        assert!(s.take_keyboard_request());
        assert!(!s.take_keyboard_request(), "the request is read once");
    }

    /// A catalogue with one app per shelf category and one with a category no shelf names.
    fn shelved() -> AppStore {
        let mut s = store();
        s.catalog = parse_catalog(&format!(
            r#"{{"apps":[{{"id":"is.o.draw","name":"Draw","version":"1.0.0","sha256":"{SUM}","package":"/a","category":"Graphics & Design"}},
                {{"id":"is.o.words","name":"Words","version":"1.0.0","sha256":"{SUM}","package":"/b","category":"Productivity"}},
                {{"id":"is.o.chess","name":"Chess","version":"1.0.0","sha256":"{SUM}","package":"/c","category":"Games"}},
                {{"id":"is.o.calc","name":"Calc","version":"1.0.0","sha256":"{SUM}","package":"/d","category":"Utilities"}},
                {{"id":"is.o.maps","name":"Maps","version":"1.0.0","sha256":"{SUM}","package":"/e","category":"Travel"}}]}}"#
        ))
        .unwrap();
        s
    }

    #[test]
    fn each_shelf_lists_the_apps_of_its_categories_and_categories_lists_them_all() {
        let mut s = shelved();
        for (shelf, want) in [(Shelf::Create, "is.o.draw"), (Shelf::Work, "is.o.words"), (Shelf::Play, "is.o.chess"), (Shelf::Develop, "is.o.calc")] {
            s.tab = Tab::Shelf(shelf);
            assert_eq!(listed(&s), vec![want.to_string()], "{shelf:?}");
            assert_eq!(s.rows()[0], Row::Title(shelf.title()));
        }
        s.tab = Tab::Categories;
        let mut all = listed(&s);
        all.sort();
        assert_eq!(all, ["is.o.calc", "is.o.chess", "is.o.draw", "is.o.maps", "is.o.words"], "a category no shelf names is still listed");
        assert!(s.rows().contains(&Row::Header("Travel".into(), false)));
        s.catalog.retain(|a| a.id != "is.o.draw");
        s.tab = Tab::Shelf(Shelf::Create);
        assert!(s.rows().iter().any(|r| matches!(r, Row::Note(n) if n.contains("create"))), "an empty shelf says so");
    }

    #[test]
    fn the_desktop_sidebar_has_a_search_field_the_sections_and_the_account_chip() {
        let ui = crate::shell::tests::ui();
        let mut s = shelved();
        let names: Vec<&str> = SIDEBAR.iter().map(|(_, n)| *n).collect();
        assert_eq!(names, ["Discover", "Create", "Work", "Play", "Develop", "Categories", "Updates"]);
        let (fx, fy, fw, fh) = s.side_search_rect(&ui);
        assert!(fy + fh <= s.side_row_rect(&ui, 0).1, "the search field sits above the sections");
        assert_eq!(s.side_hit(&ui, fx + fw / 2.0, fy + fh / 2.0), Some(SideHit::Search));
        let (rx, ry, rw, rh) = s.side_row_rect(&ui, 3);
        assert_eq!(s.side_hit(&ui, rx + rw / 2.0, ry + rh / 2.0), Some(SideHit::Tab(Tab::Shelf(Shelf::Play))));
        let (ax, ay, aw, ah) = s.side_account_rect(&ui);
        assert!(ay > s.side_row_rect(&ui, SIDEBAR.len() - 1).1, "the account chip is at the foot");
        assert_eq!(s.side_hit(&ui, ax + aw / 2.0, ay + ah / 2.0), Some(SideHit::Account));
        // a click on the field opens the results and raises the keyboard; typing filters them
        s.touch(&ui, Event::Up(fx + fw / 2.0, fy + fh / 2.0), 0.0);
        assert_eq!(s.tab, Tab::Search);
        assert!(s.take_keyboard_request() && s.text_focused());
        s.key(Out::Text("draw".into()));
        assert_eq!(listed(&s), vec!["is.o.draw".to_string()]);
        s.touch(&ui, Event::Up(rx + rw / 2.0, ry + rh / 2.0), 0.0);
        assert_eq!(s.tab, Tab::Shelf(Shelf::Play));
        assert_eq!(tab_name(Tab::Today, true), "Discover");
        assert_eq!(tab_name(Tab::Today, false), "Today");
        assert_eq!(tab_name(Tab::Categories, false), "Categories");
    }

    #[test]
    fn the_full_size_desktop_window_reports_its_toolbar_controls_and_starts_its_pane_under_the_band() {
        let ui = crate::shell::tests::ui();
        let mut s = store();
        let pt = ui.pt(1.0);
        // Today starts at the top edge as the reference's Discover does (its large title sits under the empty band);
        // a product page and the other tabs start under the unified toolbar band
        assert_eq!(s.tab, Tab::Today);
        assert_eq!(s.top(&ui), 0.0);
        s.tab = Tab::Apps;
        assert_eq!(s.top(&ui), ui.pt(DESKTOP_BAR_PT));
        s.tab = Tab::Today;
        s.page = Some(Page { id: "is.o.chess".into(), back: Tab::Today, desc: vec![], list_scroll: 0.0, whatsnew: vec![], history: false, hist: vec![] });
        assert_eq!(s.top(&ui), ui.pt(DESKTOP_BAR_PT));
        // the back control in that band takes the press
        assert_eq!(s.hit(&ui, ui.pt(20.0), ui.pt(DESKTOP_BAR_PT / 2.0)), Some(Target::Back));
        // the controls the shell's drag zone leaves to the app are where the window draws them
        assert!(toolbar_controls(s.sidebar + s.w, pt, 0.0).iter().all(|n| n.label != "Back"), "no back control while it is not shown");
        let c = toolbar_controls(s.sidebar + s.w, pt, 90.0);
        let (fx, fy, fw, fh) = s.side_search_rect(&ui);
        let search = c.iter().find(|n| n.label == "Search").expect("the search field");
        assert!((search.bounds.x * pt - fx).abs() < 1.0 && (search.bounds.y * pt - fy).abs() < 1.0, "search at {fx},{fy}");
        assert!((search.bounds.w * pt - fw).abs() < 1.0 && (search.bounds.h * pt - fh).abs() < 1.0);
        let back = c.iter().find(|n| n.label == "Back").expect("the back control");
        assert!((back.bounds.x * pt - s.sidebar as f32).abs() < 1.0, "the back control starts at the pane's left edge");
        assert!(back.bounds.y < DESKTOP_BAR_PT && back.bounds.h >= DESKTOP_BAR_PT);
        assert_eq!(back.bounds.w, 90.0, "the hole covers the back control only");
    }

    #[test]
    fn the_account_chip_shows_the_local_names_initials() {
        assert_eq!(initials("Ada Lovelace"), "AL");
        assert_eq!(initials("owner"), "O");
        assert_eq!(initials("Ólafur Búi Ólafsson"), "ÓB");
        assert_eq!(initials(""), "");
    }

    #[test]
    fn remove_takes_two_taps_and_only_for_an_app_the_customer_chose() {
        let mut confirm = None;
        assert!(!remove_step(&mut confirm, "is.o.calc"), "the first tap only asks");
        assert_eq!(confirm.as_deref(), Some("is.o.calc"));
        assert!(remove_step(&mut confirm, "is.o.calc"), "the second tap removes");
        assert!(confirm.is_none());

        let mut s = with_system();
        s.installed = vec![local("is.o.calc", "1.0.0"), InstalledApp { kind: "system".into(), ..local("is.o.kb", "1.0.0") }];
        assert!(s.can_remove("is.o.calc"));
        assert!(!s.can_remove("is.o.kb"), "a system component is never removed here");
        assert!(!s.can_remove("is.o.chess"), "an app that is not installed has nothing to remove");
        s.jobs.insert("is.o.calc".into(), Job { done: Arc::new(AtomicU64::new(0)), size: 1, cancel: Arc::new(AtomicBool::new(false)) });
        assert!(!s.can_remove("is.o.calc"), "not while it is being downloaded");
    }

    #[test]
    fn the_product_page_reserves_a_strip_only_for_apps_with_screenshots() {
        let ui = crate::shell::tests::ui();
        let mut s = store();
        s.catalog[0].screenshots = vec!["/s/1.png".into()];
        s.page = Some(Page { id: "is.o.calc".into(), back: Tab::Today, desc: vec![], list_scroll: 0.0, whatsnew: vec![], history: false, hist: vec![] });
        assert_eq!(s.shots_pt(), SHOTS_PT);
        s.page = Some(Page { id: "is.o.chess".into(), back: Tab::Today, desc: vec![], list_scroll: 0.0, whatsnew: vec![], history: false, hist: vec![] });
        assert_eq!(s.shots_pt(), 0.0);
        assert!(s.page_height(&ui) > 0.0);
    }
}
