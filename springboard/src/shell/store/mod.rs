//! The App Store: browse, product pages, GET -> progress ring -> OPEN, and an
//! Updates tab, laid out like the bifrost 14 App Store (Today / Apps / Games /
//! Updates / Search tab bar, large titles, rows with icon, name, summary and a
//! capsule button in the tint colour). The catalogue comes from
//! `bifrost.olibuijr.com` on worker threads (cached for offline); downloads
//! are verified and installed by `install`. The UI thread only reads memory.

pub mod api;
pub mod install;
pub mod json;
pub mod net;

use super::cache::TextCache;
use super::glyphs::{arc, disc, paint, ring, segment};
use super::ui::{home_zone, Ui, Weight, CARD_RADIUS_PT, CAPSULE_RADIUS, SECONDARY, STORY_HUES, TINT, WHITE};
use super::ui::{CONTENT_BG, CARD_BG, SEPARATOR_C, LABEL, FIELD_BG, MATERIAL_TITLEBAR_SOLID};
use super::Event;
use crate::deildu::jobs::Pool;
use crate::deildu::scroll::Scroller;
use crate::gfx::{pack, squircle_mask, Image, Surface};
use crate::scene::STATUS_PT;
use api::StoreApp;
use bifrost_ui::entitlement::Ledger;
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

/// True when an icon that failed at `failed_at` (None: never) may be fetched at `t`.
fn icon_due(failed_at: Option<f64>, t: f64) -> bool {
    failed_at.is_none_or(|at| t - at >= ICON_RETRY_MS)
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
fn cell() -> u32 { CARD_BG.now() }
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
}

const TABS: [(Tab, &str); 5] = [(Tab::Today, "Today"), (Tab::Games, "Games"), (Tab::Apps, "Apps"), (Tab::Updates, "Updates"), (Tab::Search, "Search")];

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
    Note(String),
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
            Row::Note(_) => 64.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Btn {
    Get,
    Open,
    Update,
    Progress(f32),
    /// system components are preinstalled: nothing to tap unless an update is on offer
    None,
}

enum Done {
    Catalog(Result<Vec<StoreApp>, String>),
    Icon { id: String, image: Option<Image> },
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
    tab: Tab,
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
    /// the last time `maintain` ran
    clock: f64,
    jobs: HashMap<String, Job>,
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
}

/// What the product page and rows call a system package's category.
const SYSTEM_COMPONENT: &str = "System Component";

fn category_label(a: &StoreApp) -> &str {
    if a.is_system() { SYSTEM_COMPONENT } else { &a.category }
}

fn is_game(a: &StoreApp) -> bool {
    a.category.eq_ignore_ascii_case("games")
}

impl AppStore {
    /// `root` is `/var/lib/springboard` (the store cache and `apps/` live below it).
    pub fn new(ui: &Ui, w: i32, h: i32, root: PathBuf) -> Self {
        let catalog = api::load_cache(&root.join("store-cache.json")).unwrap_or_default();
        let installed = install::installed_in(&root);
        let desktop = true;
        let sidebar = if desktop { (ui.pt(200.0) as i32).min(w / 3) } else { 0 };
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
            clock: 0.0,
            jobs: HashMap::new(),
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
        }
    }

    pub fn set_size(&mut self, ui: &Ui, w: i32, h: i32) {
        self.sidebar = if self.desktop { (ui.pt(200.0) as i32).min(w / 3) } else { 0 };
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
        match self.installed_version(&a.id) {
            Some(v) if api::update_available(v, &a.version) => Btn::Update,
            Some(_) if a.is_system() => Btn::None,
            Some(_) => Btn::Open,
            None if a.is_preinstalled() => Btn::None,
            None => Btn::Get,
        }
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
        let mut ledger = Ledger::new();
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
            }
            Tab::Search => {
                r.push(Row::Title("Search"));
                r.push(Row::Chips);
                let cats = self.categories();
                let list: Vec<usize> = all.into_iter().filter(|&i| self.category.is_none_or(|c| cats.get(c) == Some(&self.catalog[i].category))).collect();
                r.extend(by_name(list, &self.catalog).into_iter().map(Row::App));
            }
        }
        if self.catalog.is_empty() && matches!(self.tab, Tab::Today | Tab::Apps | Tab::Games | Tab::Search) {
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
                Done::Icon { id, image } => {
                    self.icon_pending.remove(&id);
                    self.icon_px.retain(|(i, _), _| *i != id);
                    match image {
                        Some(img) => {
                            self.icon_failed.remove(&id);
                            self.icons.insert(id, Some(img));
                        }
                        // not cached as missing: it is asked for again after ICON_RETRY_MS
                        None => {
                            self.icon_failed.insert(id, self.clock);
                        }
                    }
                }
                // a cancelled job was already removed; a newer job for the same app must stay
                Done::Installed { result: Err(e), .. } if e == install::CANCELLED => {}
                Done::Installed { id, result } => {
                    self.jobs.remove(&id);
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
                }
            }
        }
        if changed {
            self.dirty = true;
        }
        changed
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
            self.icon_pending.insert(id.clone());
            if let Some(p) = &self.pool {
                p.spawn(move || {
                    crate::gfx::affinity::background_thread();
                    let bytes = match local.filter(|p| p.is_file()) {
                        Some(p) => std::fs::read(p).ok(),
                        None if !icon.is_empty() => net::get(&net::absolute(&icon), 1_000_000).ok(),
                        None => None,
                    };
                    Done::Icon { id, image: bytes.and_then(|b| crate::imgdec::decode(&b)) }
                });
            }
        }
    }

    fn start(&mut self, id: &str) {
        let Some(a) = self.find(id).cloned() else { return };
        if self.jobs.contains_key(id) {
            return;
        }
        self.failed.remove(id);
        let done = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs.insert(id.to_string(), Job { done: done.clone(), size: a.size, cancel: cancel.clone() });
        let root = self.root.clone();
        if let Some(p) = &self.pool {
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
                Done::Installed { id: a.id, result }
            });
        } else {
            self.jobs.remove(id);
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
            self.dirty = true;
        }
    }

    fn press_button(&mut self, id: &str) {
        match self.find(id).map(|a| self.button(a)) {
            Some(Btn::None) => {}
            // a paid app needs a purchase, and the shell has no purchase sheet yet: it is never installed here
            Some(Btn::Get) if self.find(id).is_some_and(|a| a.is_paid()) => {
                self.failed.insert(id.to_string(), "Purchase is not available yet".into());
            }
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
        ui.pt(150.0 + 84.0 + 40.0 + 22.0 * p.desc.len() as f32 + self.rollback_height(&p.id) + self.whatsnew_height() + 60.0)
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
        let y = self.top(ui) - self.scroll.offset() + ui.pt(150.0 + 84.0 + 40.0 + 22.0 * n_desc as f32 + 8.0);
        (ui.pt(MARGIN), y, self.card_w(ui), ui.pt(44.0))
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
        (self.top(ui) - self.scroll.offset() + ui.pt(150.0 + 84.0 + 40.0 + 22.0 * n as f32 + 16.0 + 78.0 + 22.0 * nl as f32 + 8.0), ui.pt(44.0))
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
        if y >= bar && y < bar + ui.pt(TAB_BAR) {
            let i = (x / (self.w as f32 / TABS.len() as f32)) as usize;
            return TABS.get(i).map(|(t, _)| Target::Tab(*t));
        }
        if y < self.top(ui) {
            return (self.page.is_some() && x < ui.pt(140.0)).then_some(Target::Back);
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

    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) {
        let (mut x, y) = match ev {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            _ => return,
        };
        if self.desktop {
            if x < self.sidebar as f32 {
                if matches!(ev, Event::Up(..)) && y >= ui.pt(76.0) {
                    let i = ((y - ui.pt(76.0)) / ui.pt(44.0)) as usize;
                    if let Some((tab, _)) = TABS.get(i) { self.activate(ui, Target::Tab(*tab)); }
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
        self.dirty |= was || moving || progress;
        self.dirty
    }

    pub fn next_wake(&self, t: f64) -> Option<f64> {
        if self.scroll.animating() {
            return Some(t);
        }
        (!self.jobs.is_empty()).then(|| (self.progress_at + PROGRESS_MS).max(t))
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
                    _ => "UPDATE",
                };
                s.fill_rrect(x, y, w, h, CAPSULE_RADIUS, fill);
                let lw = ui.measure(label, 15.0, Weight::Bold);
                self.put(s, ui, label, 15.0, Weight::Bold, TINT, false, x + (w - lw) / 2.0, mid);
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
                    let b = if a.is_system() { Btn::None } else { Btn::Open };
                    self.draw_app_row(&mut c, ui, y, h, &a.id, &a.name, &format!("Version {}", a.version), if a.is_system() { SYSTEM_COMPONENT } else { "" }, b, last);
                }
            }
            y += h;
        }
    }

    fn draw_today_head(&mut self, s: &mut Surface, ui: &Ui, y: f32) {
        let x = ui.pt(TODAY_MARGIN);
        self.put(s, ui, &date_label(), 15.0, Weight::Semibold, SECONDARY, false, x, y + ui.pt(30.0));
        self.put(s, ui, "Today", 34.0, Weight::Bold, label(), false, x, y + ui.pt(70.0));
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
                for (k, line) in ui.wrap(&a.summary, 17.0, Weight::Regular, text_w).into_iter().take(2).enumerate() {
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
        let mid = ui.pt(STATUS_PT + 22.0);
        if let Some((back_tab, id)) = self.page.as_ref().map(|p| (p.back, p.id.clone())) {
            let history = self.page.as_ref().is_some_and(|p| p.history);
            let prev = if history { self.find(&id).map(|a| a.name.clone()).unwrap_or_default() } else { TABS.iter().find(|(t, _)| *t == back_tab).map_or("Back", |t| t.1).to_string() };
            let back = format!("\u{2039} {prev}");
            self.put(s, ui, &back, 17.0, Weight::Regular, TINT, false, ui.pt(MARGIN), mid);
            if history {
                let tw = ui.measure("Version History", 17.0, Weight::Semibold);
                self.put(s, ui, "Version History", 17.0, Weight::Semibold, label(), false, (self.w as f32 - tw) / 2.0, mid);
            } else if self.scroll.offset() > ui.pt(60.0) {
                let name = self.find(&id).map(|a| a.name.clone()).unwrap_or_default();
                let tw = ui.measure(&name, 17.0, Weight::Semibold);
                self.put(s, ui, &name, 17.0, Weight::Semibold, label(), false, (self.w as f32 - tw) / 2.0, mid);
            }
        } else if self.tab != Tab::Today && self.scroll.offset() > ui.pt(40.0) {
            let name = TABS.iter().find(|(t, _)| *t == self.tab).map_or("", |t| t.1);
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
            s.fill_rect(0, 0, self.sidebar as usize, self.h as usize, cell());
            s.blit(&image, self.sidebar, 0, 255, 255);
            self.put(s, ui, "App Store", 20.0, Weight::Semibold, label(), false, ui.pt(20.0), ui.pt(40.0));
            let current = self.page.as_ref().map_or(self.tab, |p| p.back);
            for (i, (tab, name)) in TABS.iter().enumerate() {
                let y = ui.pt(76.0 + i as f32 * 44.0);
                if *tab == current { s.fill_rrect(ui.pt(8.0), y, self.sidebar as f32 - ui.pt(16.0), ui.pt(40.0), ui.pt(8.0), super::ui::SIDEBAR_SELECTED); }
                tab_glyph(s, ui, *tab, (ui.pt(28.0), y + ui.pt(20.0)), if *tab == current { WHITE } else { TINT });
                self.put(s, ui, name, 15.0, Weight::Regular, label(), false, ui.pt(52.0), y + ui.pt(20.0));
            }
            s.fill_rect(self.sidebar as usize - 1, 0, 1, self.h as usize, sep());
        } else { self.draw_pane(s, ui); }
        let _ = t;
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

fn tab_glyph(s: &mut Surface, ui: &Ui, tab: Tab, (cx, cy): (f32, f32), color: u32) {
    let u = ui.pt(1.0);
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
    fn a_failed_icon_is_retried_after_a_minute_not_never() {
        assert!(icon_due(None, 0.0));
        assert!(!icon_due(Some(1_000.0), 1_000.0 + ICON_RETRY_MS - 1.0));
        assert!(icon_due(Some(1_000.0), 1_000.0 + ICON_RETRY_MS));
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
        for tab in [Tab::Today, Tab::Games, Tab::Apps, Tab::Updates, Tab::Search] {
            s.tab = tab;
            let ids = listed(&s);
            assert!(!ids.iter().any(|i| i == "is.o.update" || i == "is.o.audio"), "{tab:?} lists a service");
        }
        assert_eq!(s.update_count(), 0);
        let plan = api::system_plan(&s.catalog, &[], "9.9.9");
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
        assert_eq!(rows[at + 1..], [Row::App(1), Row::App(0)], "chess (bought first) then calc, both in the catalogue");
    }

    #[test]
    fn a_paid_app_is_not_installed_without_a_purchase() {
        let mut s = store();
        s.catalog[1].price = Some(bifrost_ui::purchase::Price::new(990, 0, "ISK"));
        let chess = s.catalog[1].clone();
        s.press_button(&chess.id);
        assert!(s.jobs.is_empty());
        assert_eq!(s.failed.get(&chess.id).map(String::as_str), Some("Purchase is not available yet"));
        assert_eq!(s.button(&chess), Btn::Get);
    }
}
