//! The Activity Monitor window's state and behaviour: the tab, the filter, the sort, the search, the selection, the
//! sampling clock and the geometry that drawing and pointer input share. Points for the kit (`ui.pt`), pixels only
//! where the surface is drawn.

use crate::inspect::{self, Inspection};
use crate::job::{Job, JobKind, Outcome};
use crate::model::{self, Column, ColumnSet, DockContent, DockMode, EnergyAverages, Filter, History, Signal, Sort, SortKey, Tab, UpdateFrequency};
use crate::sample::{ProcSample, Sampler, Snapshot};
use crate::{diagnostics, sample_proc};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::sheet;
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::{Node, Rect, Role, TextField};
use bifrost_app::kit::ui::{MAC_BODY_PT, SPACE_S};
use bifrost_app::kit::{Scroller, TextCache, Ui, Weight};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets::{CAPSULE_PT, TOOLBAR_GAP_PT};
use bifrost_app::TouchKind;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Sidebar width (points) and the inset of the content from it and from the window edge.
pub const SIDE_PT: f32 = 180.0;
const INSET_PT: f32 = 16.0;
/// The tab control: its height and its widest size.
const SEG_H_PT: f32 = 24.0;
const SEG_MAX_PT: f32 = 460.0;
/// Gap above the tab control, and between the tab control, the graph, the table header.
const GAP_PT: f32 = 10.0;
/// The graph card's height (points).
pub const GRAPH_PT: f32 = 132.0;
/// Table row, header and footer heights (points).
pub const ROW_PT: f32 = 20.0;
pub const HEAD_PT: f32 = 22.0;
pub const FOOT_PT: f32 = 24.0;
/// The sidebar's rows start under the toolbar band, as in Console.
pub const SIDE_SCROLL: f32 = -(UNIFIED_BAR_PT + 4.0);
/// The toolbar's search field width (points).
pub const SEARCH_PT: f32 = 220.0;
/// Readings are taken once a second unless View > Update Frequency says otherwise.
pub const SAMPLE_MS: f64 = 1000.0;
/// The energy means are written to the app's storage this often (milliseconds), and at the first reading.
const AVERAGES_EVERY_MS: f64 = 60_000.0;
/// The file in the app's storage that keeps the energy means across launches.
const AVERAGES_FILE: &str = "energy-averages.txt";

/// Toolbar item ids (also the published control ids).
pub const B_QUIT: u32 = 1;
pub const B_SEARCH: u32 = 2;
pub const B_FORCE: u32 = 3;
pub const B_INSPECT: u32 = 4;
pub const B_STOP: u32 = 5;
pub const B_REPORT_CLOSE: u32 = 6;
pub const B_REPORT_SAVE: u32 = 7;
pub const B_REPORT_COPY: u32 = 8;
pub const B_INSPECT_DONE: u32 = 9;
pub const B_REVEAL: u32 = 10;

/// The Inspect Process segments: Memory, Statistics, and Open Files and Ports.
pub const INSPECT_SEGMENTS: [&str; 3] = ["Memory", "Statistics", "Open Files and Ports"];
/// The inspector's list line height (points), the card's inset, and where its segments and body start below the top
/// of the panel.
pub const INSPECT_ROW_PT: f32 = 16.0;
pub const INSPECT_PAD_PT: f32 = 14.0;
pub const INSPECT_SEG_Y_PT: f32 = 58.0;
pub const INSPECT_BODY_Y_PT: f32 = 94.0;
/// The inspector's segmented control is at most this wide (points).
const INSPECT_SEG_MAX_PT: f32 = 440.0;
/// Sidebar row ids.
const ROW_ALL: u32 = 1;
const ROW_MINE: u32 = 2;
const ROW_OTHER: u32 = 3;
const ROW_SYSTEM: u32 = 4;
const ROW_ACTIVE: u32 = 5;
const ROW_INACTIVE: u32 = 6;
const ROW_SELECTED: u32 = 7;
const ROW_GPU: u32 = 8;
const ROW_WINDOWED: u32 = 9;
const ROW_RECENT: u32 = 10;
/// Menu item ids for the app's own menu.
pub const MENU_QUIT: u32 = 1;
pub const MENU_FIND: u32 = 2;
pub const MENU_FORCE: u32 = 3;
pub const MENU_INSPECT: u32 = 4;
pub const MENU_COPY: u32 = 5;
pub const MENU_DELTAS: u32 = 6;
pub const MENU_CPU_HISTORY: u32 = 7;
pub const MENU_SAMPLE: u32 = 8;
pub const MENU_DIAGNOSTICS: u32 = 9;
pub const MENU_SAVE: u32 = 10;
pub const MENU_CLOSE_REPORT: u32 = 11;
pub const MENU_STOP: u32 = 12;
pub const MENU_SIGNAL_SHEET: u32 = 13;
/// Update Frequency radio items: MENU_FREQ + index into `UpdateFrequency::ALL`.
pub const MENU_FREQ: u32 = 20;
/// Send Signal items: MENU_SIGNAL + index into `model::SIGNALS`.
pub const MENU_SIGNAL: u32 = 40;
/// Filter items of the View menu: MENU_FILTER + index into `Filter::ALL`.
pub const MENU_FILTER: u32 = 60;
/// View > Columns items: MENU_COLUMN + index into `Tab::all_columns()` of the current tab.
pub const MENU_COLUMN: u32 = 100;
/// View > Dock Icon items: MENU_DOCK + index into `DockMode::ALL`.
pub const MENU_DOCK: u32 = 80;
/// Process > Sample Duration items: MENU_SAMPLE_LENGTH + index into `sample_proc::DURATIONS`.
pub const MENU_SAMPLE_LENGTH: u32 = 140;
/// Window > CPU Usage, Window > CPU History and Window > Keep CPU Windows on Top.
pub const MENU_WINDOW_CPU_USAGE: u32 = 160;
pub const MENU_WINDOW_CPU_HISTORY: u32 = 161;
pub const MENU_WINDOW_KEEP_TOP: u32 = 162;

/// Height of one line of the report view (points) and how often a running job is looked at (milliseconds).
pub const REPORT_LINE_PT: f32 = 16.0;

/// Text the window shows in place of the table: a process sample or the diagnostics summary. It scrolls, saves and
/// copies.
pub struct Report {
    pub title: String,
    pub text: String,
    pub lines: Vec<String>,
    pub scroll: Scroller,
}

impl Report {
    pub fn new(title: &str, text: &str) -> Report {
        Report { title: title.to_string(), text: text.to_string(), lines: text.lines().map(str::to_string).collect(), scroll: Scroller::new() }
    }

    /// The file name the save panel suggests: the title with the characters a path cannot hold replaced.
    pub fn save_name(&self) -> String {
        let clean: String = self.title.chars().map(|c| if matches!(c, '/' | ':' | '\\' | '\n') { '-' } else { c }).collect();
        format!("{clean}.txt")
    }
}

/// Something the window needs from the app layer, which owns the shell connection: a save panel for a report or for the
/// diagnostics archive.
#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    SaveReport { name: String, text: String },
    SaveBundle(PathBuf),
}

/// The signal sheet: a card over the window with one row per signal, then Cancel and Send. Points.
pub const SHEET_W_PT: f32 = 340.0;
const SHEET_ROW_PT: f32 = 22.0;
const SHEET_HEAD_PT: f32 = 44.0;
const SHEET_FOOT_PT: f32 = 48.0;
const SHEET_BUTTON_W_PT: f32 = 84.0;
const SHEET_BUTTON_H_PT: f32 = 24.0;
const SHEET_PAD_PT: f32 = 12.0;

/// What a point on the signal sheet is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetHit {
    Row(usize),
    Cancel,
    Send,
}

/// The row of one signal on a sheet whose card is `card`.
pub fn signal_sheet_row(card: Rect, index: usize) -> Rect {
    Rect { x: card.x + SHEET_PAD_PT, y: card.y + SHEET_HEAD_PT + index as f32 * SHEET_ROW_PT, w: card.w - 2.0 * SHEET_PAD_PT, h: SHEET_ROW_PT }
}

/// The Cancel and Send buttons, Send at the right.
pub fn signal_sheet_buttons(card: Rect) -> (Rect, Rect) {
    let y = card.y + card.h - SHEET_FOOT_PT + (SHEET_FOOT_PT - SHEET_BUTTON_H_PT) / 2.0;
    let send = Rect { x: card.x + card.w - SHEET_PAD_PT - SHEET_BUTTON_W_PT, y, w: SHEET_BUTTON_W_PT, h: SHEET_BUTTON_H_PT };
    let cancel = Rect { x: send.x - 8.0 - SHEET_BUTTON_W_PT, ..send };
    (cancel, send)
}

/// What the point is over on a sheet whose card is `card`.
pub fn signal_sheet_hit(card: Rect, x: f32, y: f32) -> Option<SheetHit> {
    let inside = |b: Rect| x >= b.x && x < b.x + b.w && y >= b.y && y < b.y + b.h;
    let (cancel, send) = signal_sheet_buttons(card);
    if inside(cancel) {
        return Some(SheetHit::Cancel);
    }
    if inside(send) {
        return Some(SheetHit::Send);
    }
    (0..model::SIGNALS.len()).find(|i| inside(signal_sheet_row(card, *i))).map(SheetHit::Row)
}

/// The quit confirmation: a card with the question, a line on what is lost, and Cancel, Force Quit and Quit. The
/// default button is the one Return chooses: Force Quit when the menu item was Force Quit Process, else Quit.
const QUIT_HEAD_PT: f32 = 52.0;
const QUIT_FOOT_PT: f32 = 48.0;

/// The state of the quit confirmation: the process it is about and whether Return forces the quit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuitSheet {
    pub pid: u32,
    pub force_default: bool,
}

/// What a point on the quit confirmation is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuitHit {
    Cancel,
    Force,
    Quit,
}

/// The three buttons of a quit confirmation whose card is `card`, left to right: Cancel, Force Quit, Quit.
pub fn quit_sheet_buttons(card: Rect) -> [Rect; 3] {
    let y = card.y + card.h - QUIT_FOOT_PT + (QUIT_FOOT_PT - SHEET_BUTTON_H_PT) / 2.0;
    let quit = Rect { x: card.x + card.w - SHEET_PAD_PT - SHEET_BUTTON_W_PT, y, w: SHEET_BUTTON_W_PT, h: SHEET_BUTTON_H_PT };
    let force = Rect { x: quit.x - 8.0 - SHEET_BUTTON_W_PT, ..quit };
    let cancel = Rect { x: force.x - 8.0 - SHEET_BUTTON_W_PT, ..quit };
    [cancel, force, quit]
}

/// What the point is over on a quit confirmation whose card is `card`.
pub fn quit_sheet_hit(card: Rect, x: f32, y: f32) -> Option<QuitHit> {
    let inside = |b: Rect| x >= b.x && x < b.x + b.w && y >= b.y && y < b.y + b.h;
    let [cancel, force, quit] = quit_sheet_buttons(card);
    if inside(cancel) {
        Some(QuitHit::Cancel)
    } else if inside(force) {
        Some(QuitHit::Force)
    } else if inside(quit) {
        Some(QuitHit::Quit)
    } else {
        None
    }
}

/// Where a press started, so the release can act on the same thing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    None,
    Sheet,
    Report,
    /// A press in the inspector's body (it scrolls by dragging) or on one of its segments.
    Inspect,
    InspectSeg(usize),
    Sidebar,
    Toolbar(u32),
    Tab(usize),
    Head(usize),
    Table,
}

/// Geometry in points. Everything derives from the window size, so a resize moves the parts with it.
pub struct Geo {
    pub w: f32,
    pub h: f32,
    pub content_x: f32,
    pub content_w: f32,
    pub seg_y: f32,
    pub seg_w: f32,
    pub graph_y: f32,
    pub head_y: f32,
    pub table_top: f32,
    pub table_bottom: f32,
}

pub struct Monitor {
    /// Window size in pixels.
    pub w: f32,
    pub h: f32,
    pub tab: Tab,
    pub filter: Filter,
    pub sort: Sort,
    pub query: TextField,
    pub search_focused: bool,
    pub snapshot: Option<Snapshot>,
    /// The process rows the table shows (empty on the Network tab, which lists interfaces).
    pub rows: Vec<ProcSample>,
    pub selected: Option<u32>,
    pub history: History,
    pub scroll: Scroller,
    pub sampler: Sampler,
    pub last_sample: Option<f64>,
    pub status: String,
    pub labels: TextCache,
    pub press: Press,
    /// The process being inspected (its pid and the last reading), shown in place of the graph and the table.
    pub inspector: Option<(u32, Inspection)>,
    /// The inspector segment shown (an index into `INSPECT_SEGMENTS`).
    pub inspect_segment: usize,
    /// Scrolls the inspector's Open Files and Ports list.
    pub inspect_scroll: Scroller,
    /// The program the window asks the app layer to reveal in Files; taken with `take_reveal`.
    pub reveal: Option<PathBuf>,
    pub frequency: UpdateFrequency,
    /// Which optional columns the user turned on or off (View > Columns).
    pub columns: ColumnSet,
    /// Processes shown relative to the moment Show Deltas was turned on for them, by pid.
    pub baselines: HashMap<u32, ProcSample>,
    /// When each baseline was taken, in the sampler's milliseconds (the Since Baseline column).
    pub baseline_at: HashMap<u32, f64>,
    /// The process whose family the Selected Processes filter shows.
    pub scope: Option<u32>,
    /// Running energy means by process name, for the 12 hr Average column.
    pub averages: EnergyAverages,
    /// When the energy means were last written to the app's storage, in the sampler's milliseconds.
    pub averages_saved_at: Option<f64>,
    /// The CPU graph shows one strip per core (View > CPU History) instead of the total.
    pub cpu_history: bool,
    /// How long Sample Process looks, in seconds (one of `sample_proc::DURATIONS`).
    pub sample_seconds: u32,
    /// The background task running now (a process sample or the diagnostics), if any.
    pub job: Option<Job>,
    /// The report shown in place of the table.
    pub report: Option<Report>,
    /// A save the app layer should ask the shell for; taken with `take_request`.
    pub request: Option<Request>,
    /// The signal sheet, with the index of the chosen signal in `model::SIGNALS`.
    pub signal_sheet: Option<usize>,
    /// The quit confirmation, when Quit Process or Force Quit Process has been chosen.
    pub quit_sheet: Option<QuitSheet>,
    /// The pids that own a window on the desktop, as the shell last listed them (for Windowed Processes).
    pub window_owners: HashSet<u32>,
    /// The window list should be asked for again; taken with `take_window_poll`.
    pub window_poll: bool,
    /// What the Dock tile shows (View > Dock Icon).
    pub dock_mode: DockMode,
    /// A new Dock tile to send to the shell; taken with `take_dock_update`.
    pub dock_update: Option<DockContent>,
    /// The app's storage folder, where the diagnostics archive is prepared.
    pub data_dir: PathBuf,
}

impl Monitor {
    pub fn new(w: f32, h: f32) -> Self {
        Monitor {
            w,
            h,
            tab: Tab::Cpu,
            filter: Filter::All,
            sort: Tab::Cpu.default_sort(),
            query: TextField::new(),
            search_focused: false,
            snapshot: None,
            rows: Vec::new(),
            selected: None,
            history: History::default(),
            scroll: Scroller::new(),
            sampler: Sampler::new(),
            last_sample: None,
            status: String::new(),
            labels: TextCache::default(),
            press: Press::None,
            inspector: None,
            inspect_segment: 0,
            inspect_scroll: Scroller::new(),
            reveal: None,
            frequency: UpdateFrequency::VeryOften,
            columns: ColumnSet::default(),
            baselines: HashMap::new(),
            baseline_at: HashMap::new(),
            scope: None,
            averages: EnergyAverages::default(),
            averages_saved_at: None,
            cpu_history: false,
            sample_seconds: sample_proc::DURATION_SECS,
            job: None,
            report: None,
            request: None,
            signal_sheet: None,
            quit_sheet: None,
            window_owners: HashSet::new(),
            window_poll: false,
            dock_mode: DockMode::Off,
            dock_update: None,
            data_dir: PathBuf::new(),
        }
    }

    pub fn resize(&mut self, w: f32, h: f32) {
        self.w = w;
        self.h = h;
    }

    /// Reads the machine when a second has passed. True when the window needs a new frame.
    pub fn tick(&mut self, t: f64) -> bool {
        let mut moving = self.scroll.tick(t);
        if let Some(report) = &mut self.report {
            moving |= report.scroll.tick(t);
        }
        moving |= self.inspect_scroll.tick(t);
        moving |= self.poll_job();
        let due = self.last_sample.map_or(true, |last| t - last >= self.frequency.millis());
        if !due {
            return moving;
        }
        let dt = self.last_sample.map_or(self.frequency.seconds() as f64, |last| (t - last) / 1000.0);
        self.last_sample = Some(t);
        let mut snap = self.sampler.sample(t);
        self.averages.record(&snap.procs, dt);
        for p in &mut snap.procs {
            p.energy_avg = self.averages.mean(&p.name);
        }
        if self.averages_saved_at.map_or(true, |at| t - at >= AVERAGES_EVERY_MS) {
            self.save_averages();
            self.averages_saved_at = Some(t);
        }
        self.history.record(&snap);
        if let Some(pressure) = inspect::read_pressure() {
            self.history.pressure.push(pressure);
        }
        if self.dock_mode != DockMode::Off {
            self.dock_update = Some(model::dock_content(self.dock_mode, &snap, &self.history));
        }
        self.window_poll = self.filter == Filter::Windowed;
        self.snapshot = Some(snap);
        self.rebuild();
        self.refresh_inspector();
        true
    }

    /// Re-reads the inspected process. It closes when the process has exited.
    fn refresh_inspector(&mut self) {
        let Some((pid, _)) = self.inspector else { return };
        let data = inspect::inspect(pid);
        if data.command.is_empty() && data.exe.is_empty() {
            self.inspector = None;
            self.status = "The inspected process has exited".into();
        } else {
            self.inspector = Some((pid, data));
        }
    }

    /// The time of the next reading, so the app wakes for it.
    pub fn next_wake(&self, t: f64) -> f64 {
        let reading = self.last_sample.map_or(t, |last| last + self.frequency.millis()).max(t);
        // Job progress and completion signal the SDK main-queue eventfd; the only timer here is process sampling.
        reading
    }

    // --- background jobs and the report ---

    pub fn job_running(&self) -> bool {
        self.job.is_some()
    }

    /// Process > Sample Process: looks at the selected process's threads for the chosen duration.
    pub fn start_sample(&mut self) {
        if self.job.is_some() {
            return;
        }
        let Some(p) = self.selected_proc().cloned() else { return };
        let (pid, name, seconds) = (p.pid, p.name.clone(), self.sample_seconds);
        self.report = None;
        self.job = Some(Job::start(JobKind::Sample, &format!("Sampling {name}"), move |progress| {
            sample_proc::run(pid, &name, seconds, sample_proc::INTERVAL_MS, progress)
        }));
        self.status.clear();
    }

    /// Process > Sample Duration: the length of the next sample. Only the offered lengths are taken.
    pub fn set_sample_seconds(&mut self, seconds: u32) -> bool {
        if !sample_proc::DURATIONS.contains(&seconds) {
            return false;
        }
        self.sample_seconds = seconds;
        true
    }

    /// View > Run System Diagnostics: gathers the logs and counters into an archive to save.
    pub fn start_diagnostics(&mut self) {
        if self.job.is_some() {
            return;
        }
        let procs = self.snapshot.as_ref().map(|s| s.procs.clone()).unwrap_or_default();
        let dir = self.data_dir.clone();
        self.job = Some(Job::start(JobKind::Diagnostics, "Gathering system diagnostics", move |progress| diagnostics::gather(&dir, &procs, progress)));
        self.status.clear();
    }

    /// Asks the running job to stop; its outcome (Cancelled) arrives through `poll_job`.
    pub fn stop_job(&mut self) {
        if let Some(job) = &self.job {
            job.stop();
            self.status = "Stopping".into();
        }
    }

    /// Takes the outcome of a finished job. True when the window needs a frame (a job is running or has just ended).
    fn poll_job(&mut self) -> bool {
        let Some(job) = &self.job else { return false };
        let Some(outcome) = job.poll() else { return true };
        self.job = None;
        self.status.clear();
        match outcome {
            Outcome::Report { title, text } => self.report = Some(Report::new(&title, &text)),
            Outcome::Bundle(path) => {
                self.request = Some(Request::SaveBundle(path));
                self.status = "Choose where to save the diagnostics".into();
            }
            Outcome::Failed(message) => self.status = message,
            Outcome::Cancelled => self.status = "Stopped".into(),
        }
        true
    }

    /// The progress line of the running job: "4 of 10 seconds" or "3 of 18 items".
    pub fn job_text(&self) -> String {
        let Some(job) = &self.job else { return String::new() };
        let (done, total) = job.counts();
        match (job.kind, total) {
            (_, 0) => "Starting".into(),
            (JobKind::Sample, _) => format!("{} of {} seconds", done / 1000, total / 1000),
            (JobKind::Diagnostics, _) => format!("{done} of {total} items"),
        }
    }

    /// Hands the app layer the save it should ask for, once.
    pub fn take_request(&mut self) -> Option<Request> {
        self.request.take()
    }

    /// Asks for the report to be saved as a text file.
    pub fn save_report(&mut self) {
        if let Some(report) = &self.report {
            self.request = Some(Request::SaveReport { name: report.save_name(), text: report.text.clone() });
        }
    }

    pub fn close_report(&mut self) {
        self.report = None;
    }

    /// The report's area: under the toolbar, down to the footer.
    pub fn report_rect(&self, ui: &Ui) -> Rect {
        let g = self.geo(ui);
        let top = UNIFIED_BAR_PT + GAP_PT;
        Rect { x: g.content_x, y: top, w: g.content_w, h: (g.table_bottom - top).max(0.0) }
    }

    /// Sets the report's scroll range from its line count.
    pub fn fit_report_scroll(&mut self, ui: &Ui) {
        let height = self.report_rect(ui).h;
        if let Some(report) = &mut self.report {
            let content = report.lines.len() as f32 * REPORT_LINE_PT + INSET_PT;
            report.scroll.set_max((content - height).max(0.0));
        }
    }

    /// Recomputes the table rows from the last reading and the filter, search and sort.
    pub fn rebuild(&mut self) {
        let query = self.query.to_string().to_lowercase();
        if let Some(s) = &self.snapshot {
            // a process that ended takes its baseline with it
            self.baselines.retain(|pid, _| s.procs.iter().any(|p| p.pid == *pid));
            self.baseline_at.retain(|pid, _| self.baselines.contains_key(pid));
            if self.filter == Filter::Selected && !self.scope.map_or(false, |root| s.procs.iter().any(|p| p.pid == root)) {
                self.filter = Filter::All;
                self.scope = None;
                self.status = "The selected process has exited".into();
                self.sync_scans();
            }
        }
        let only = match (self.filter, self.scope, &self.snapshot) {
            (Filter::Selected, Some(root), Some(s)) => Some(model::descendants(&s.procs, root)),
            (Filter::Windowed, _, _) => Some(self.window_owners.clone()),
            _ => None,
        };
        self.rows = match &self.snapshot {
            Some(s) => model::visible_scoped(&s.procs, self.filter, self.sampler.own_uid, &query, self.sort, only.as_ref(), &self.baselines),
            None => Vec::new(),
        };
        if let Some(pid) = self.selected {
            if !self.rows.iter().any(|p| p.pid == pid) {
                self.selected = None;
            }
        }
    }

    pub fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.sort = tab.default_sort();
        self.scroll.jump_top();
        self.sync_scans();
        self.rebuild();
    }

    /// Chooses the process filter. Selected Processes needs a selected process: it shows that one and what it started.
    pub fn set_filter(&mut self, filter: Filter) {
        if filter == Filter::Selected {
            let Some(pid) = self.selected else {
                self.status = "Select a process first".into();
                return;
            };
            self.scope = Some(pid);
        } else {
            self.scope = None;
        }
        self.filter = filter;
        self.window_poll = filter == Filter::Windowed;
        self.scroll.jump_top();
        self.sync_scans();
        self.rebuild();
    }

    /// View > Dock Icon. A change puts the new tile (or an empty one, for the plain icon) up for the app layer.
    pub fn set_dock_mode(&mut self, mode: DockMode) {
        if self.dock_mode == mode {
            return;
        }
        self.dock_mode = mode;
        self.dock_update = Some(match &self.snapshot {
            Some(snap) => model::dock_content(mode, snap, &self.history),
            None => DockContent::cleared(),
        });
    }

    /// Hands the app layer the Dock tile to show, once.
    pub fn take_dock_update(&mut self) -> Option<DockContent> {
        self.dock_update.take()
    }

    /// Hands the app layer the request for a fresh window list, once.
    pub fn take_window_poll(&mut self) -> bool {
        std::mem::take(&mut self.window_poll)
    }

    /// The pids that own a window now, from the shell's list.
    pub fn set_window_owners(&mut self, owners: HashSet<u32>) {
        self.window_owners = owners;
        self.rebuild();
    }

    /// Tells the sampler which costly readings the window needs now: the GPU on the GPU tab and under the GPU filter,
    /// the sleep blockers on the Energy tab while its Preventing Sleep column is shown. A reading that has just been
    /// turned on is taken at once.
    fn sync_scans(&mut self) {
        let gpu = self.tab == Tab::Gpu || self.filter == Filter::Gpu;
        let sleep = self.shown_key(SortKey::PreventingSleep);
        let ports = self.shown_key(SortKey::Ports);
        let private = self.shown_key(SortKey::PrivateMem);
        let wakeups = self.shown_key(SortKey::WakeUps);
        let turned_on = (gpu && !self.sampler.scan_gpu) || (sleep && !self.sampler.scan_sleep) || (wakeups && !self.sampler.scan_wakeups);
        self.sampler.scan_gpu = gpu;
        self.sampler.scan_sleep = sleep;
        self.sampler.scan_ports = ports;
        self.sampler.scan_private = private;
        self.sampler.scan_wakeups = wakeups;
        if turned_on {
            self.last_sample = None;
        }
    }

    /// Whether the current tab shows the column with this sort key.
    fn shown_key(&self, key: SortKey) -> bool {
        self.tab.columns(&self.columns).iter().any(|c| c.key == Some(key))
    }

    /// View > Columns: shows or hides the optional column at `index` of the current tab. Returns the new preference
    /// string to store, or None when there is no such column or it cannot be hidden.
    pub fn toggle_column(&mut self, index: usize) -> Option<String> {
        let col = self.tab.all_columns().into_iter().nth(index).filter(|c| !c.required)?;
        self.columns.toggle(self.tab, &col);
        // sorting by a column that is gone would leave the order unexplained
        if !self.tab.columns(&self.columns).iter().any(|c| c.key == Some(self.sort.key)) {
            self.sort = self.tab.default_sort();
        }
        self.sync_scans();
        self.rebuild();
        Some(self.columns.to_pref())
    }

    /// Replaces the shown columns with a stored choice (at startup).
    pub fn set_columns(&mut self, columns: ColumnSet) {
        self.columns = columns;
        self.sync_scans();
    }

    /// Reads the energy means kept from earlier launches, from the app's storage. Nothing is read without a folder.
    pub fn load_averages(&mut self) {
        if self.data_dir.as_os_str().is_empty() {
            return;
        }
        if let Ok(text) = std::fs::read_to_string(self.data_dir.join(AVERAGES_FILE)) {
            self.averages = EnergyAverages::from_text(&text);
        }
    }

    /// Writes the energy means to the app's storage. A write that fails leaves the old file, and the next one tries again.
    pub fn save_averages(&self) {
        if self.data_dir.as_os_str().is_empty() {
            return;
        }
        let _ = std::fs::write(self.data_dir.join(AVERAGES_FILE), self.averages.to_text());
    }

    /// View > CPU History: one strip per core in place of the total.
    pub fn set_cpu_history(&mut self, on: bool) {
        self.cpu_history = on;
    }

    /// Whether the selected process is shown relative to a baseline.
    pub fn deltas_on_selected(&self) -> bool {
        self.selected.map_or(false, |pid| self.baselines.contains_key(&pid))
    }

    /// Process > Show Deltas for Process: counts the selected process's CPU time, bytes and GPU time from now, or
    /// goes back to counting from the start of the process.
    pub fn toggle_deltas(&mut self) {
        let Some(p) = self.selected_proc().cloned() else { return };
        if self.baselines.remove(&p.pid).is_none() {
            self.status = format!("Showing deltas for {}", p.name);
            self.baseline_at.insert(p.pid, self.last_sample.unwrap_or(0.0));
            self.baselines.insert(p.pid, p);
        } else {
            self.baseline_at.remove(&p.pid);
        }
    }

    /// The Since Baseline column's text for a process: how long its baseline has been running as of the last reading,
    /// or nothing when it has none.
    pub fn since_text(&self, pid: u32) -> String {
        match (self.baseline_at.get(&pid), self.last_sample) {
            (Some(at), Some(now)) => model::elapsed_text((now - at) / 1000.0),
            (Some(_), None) => model::elapsed_text(0.0),
            _ => String::new(),
        }
    }

    fn sort_clicked(&mut self, key: SortKey) {
        self.sort = self.sort.clicked(key);
        self.rebuild();
    }

    pub fn selected_proc(&self) -> Option<&ProcSample> {
        let pid = self.selected?;
        self.rows.iter().find(|p| p.pid == pid)
    }

    /// Only processes of the signed-in user can be quit, and never this one.
    pub fn can_quit_selected(&self) -> bool {
        self.selected_proc().map_or(false, |p| p.uid == self.sampler.own_uid && p.pid != std::process::id())
    }

    /// Quit Process: asks first, with the quit confirmation. Only for a process that may be signalled.
    pub fn quit_selected(&mut self) {
        self.open_quit_sheet(false);
    }

    /// Force Quit Process: asks first too, and Return forces the quit.
    pub fn force_quit_selected(&mut self) {
        self.open_quit_sheet(true);
    }

    /// Opens the quit confirmation on the selected process. `force_default` makes Force Quit the button Return chooses.
    pub fn open_quit_sheet(&mut self, force_default: bool) {
        if !self.can_quit_selected() {
            return;
        }
        self.signal_sheet = None;
        self.quit_sheet = self.selected.map(|pid| QuitSheet { pid, force_default });
    }

    /// The name of a process for the sheets' text, or a plain word when it is no longer listed.
    pub fn process_name(&self, pid: u32) -> String {
        self.snapshot.as_ref().and_then(|s| s.procs.iter().find(|p| p.pid == pid)).map_or_else(|| "the process".to_string(), |p| p.name.clone())
    }

    /// The quit confirmation's card, centred on the content like the signal sheet.
    pub fn quit_sheet_rect(&self, ui: &Ui) -> Rect {
        let g = self.geo(ui);
        let h = QUIT_HEAD_PT + QUIT_FOOT_PT;
        let top = UNIFIED_BAR_PT + GAP_PT;
        Rect { x: SIDE_PT + ((g.w - SIDE_PT - SHEET_W_PT) / 2.0).max(0.0), y: ((g.h - h) / 2.0).max(top), w: SHEET_W_PT, h }
    }

    /// A release on the quit confirmation: Cancel closes it; Quit and Force Quit send their signal to that process.
    fn quit_sheet_released(&mut self, ui: &Ui, x: f32, y: f32) {
        let Some(sheet) = self.quit_sheet else { return };
        match quit_sheet_hit(self.quit_sheet_rect(ui), x, y) {
            Some(QuitHit::Cancel) => self.quit_sheet = None,
            Some(QuitHit::Quit) => self.confirm_quit(sheet, false),
            Some(QuitHit::Force) => self.confirm_quit(sheet, true),
            None => {}
        }
    }

    /// Return on the quit confirmation: the default button's action.
    pub fn confirm_quit_default(&mut self) {
        if let Some(sheet) = self.quit_sheet {
            self.confirm_quit(sheet, sheet.force_default);
        }
    }

    /// Closes the confirmation and sends SIGTERM (or SIGKILL when forced) to its process.
    fn confirm_quit(&mut self, sheet: QuitSheet, force: bool) {
        self.quit_sheet = None;
        let (sig, verb) = if force { (SIGKILL, "force quit") } else { (SIGTERM, "quit") };
        self.signal_pid(sheet.pid, sig, verb);
    }

    /// Process > Send Signal...: opens the sheet with TERM chosen. Only for a process that may be signalled.
    pub fn open_signal_sheet(&mut self) {
        if self.can_quit_selected() {
            self.quit_sheet = None;
            self.signal_sheet = model::SIGNALS.iter().position(|s| s.number == SIGTERM).or(Some(0));
        }
    }

    /// Moves the chosen signal on the sheet by `step` rows, stopping at the first and the last.
    fn move_signal_choice(&mut self, step: i32) {
        if let Some(i) = self.signal_sheet {
            let last = model::SIGNALS.len() as i32 - 1;
            self.signal_sheet = Some((i as i32 + step).clamp(0, last) as usize);
        }
    }

    /// Sends the signal chosen on the sheet and closes it (Send, or Return).
    fn send_chosen_signal(&mut self) {
        if let Some(i) = self.signal_sheet.take() {
            self.send_signal(model::SIGNALS[i]);
        }
    }

    /// The sheet's card, centred on the content.
    pub fn signal_sheet_rect(&self, ui: &Ui) -> Rect {
        let g = self.geo(ui);
        let h = SHEET_HEAD_PT + model::SIGNALS.len() as f32 * SHEET_ROW_PT + SHEET_FOOT_PT;
        let top = UNIFIED_BAR_PT + GAP_PT;
        Rect { x: SIDE_PT + ((g.w - SIDE_PT - SHEET_W_PT) / 2.0).max(0.0), y: ((g.h - h) / 2.0).max(top), w: SHEET_W_PT, h }
    }

    /// A release on the sheet: a row chooses its signal, Cancel closes it, Send closes it and signals the process.
    fn sheet_released(&mut self, ui: &Ui, x: f32, y: f32) {
        match signal_sheet_hit(self.signal_sheet_rect(ui), x, y) {
            Some(SheetHit::Row(i)) => self.signal_sheet = Some(i),
            Some(SheetHit::Cancel) => self.signal_sheet = None,
            Some(SheetHit::Send) => self.send_chosen_signal(),
            None => {}
        }
    }

    /// Process > Send Signal: any of the listed signals, same ownership rule as Quit.
    pub fn send_signal(&mut self, sig: Signal) {
        self.signal_selected(sig.number, &format!("signal {}", sig.name));
    }

    /// Changes how often the machine is read; the next reading is due at once.
    pub fn set_frequency(&mut self, f: UpdateFrequency) {
        self.frequency = f;
    }

    /// The selected row as tab-separated text, for Edit > Copy.
    pub fn copy_text(&self) -> Option<String> {
        self.selected_proc().map(|p| model::row_text(p, self.tab, &self.columns, self.baselines.get(&p.pid), &self.since_text(p.pid)))
    }

    fn signal_selected(&mut self, sig: i32, verb: &str) {
        let Some(pid) = self.selected else { return };
        self.signal_pid(pid, sig, verb);
    }

    /// Sends `sig` to one process of the signed-in user. Anything else is refused, and this process is never signalled.
    fn signal_pid(&mut self, pid: u32, sig: i32, verb: &str) {
        let name = self.process_name(pid);
        let owned = self.snapshot.as_ref().and_then(|s| s.procs.iter().find(|p| p.pid == pid)).map_or(false, |p| p.uid == self.sampler.own_uid && p.pid != std::process::id());
        if !owned {
            self.status = format!("Only processes you own can be {verb}");
            return;
        }
        // SAFETY: kill(2) takes a pid and a signal number and touches no memory of ours.
        let rc = unsafe { kill(pid as i32, sig) };
        self.status = if rc == 0 { format!("Sent {verb} to {name}") } else { format!("Could not {verb} {name}") };
    }

    /// Whether a process row is selected (any process can be inspected; the reading says what is denied).
    pub fn has_selected(&self) -> bool {
        self.selected_proc().is_some()
    }

    /// Opens the inspector on the selected process, or closes it when it is open. It opens on the Memory segment.
    pub fn toggle_inspector(&mut self) {
        if self.inspector.take().is_some() {
            return;
        }
        let Some(pid) = self.selected else { return };
        self.report = None;
        self.search_focused = false;
        self.inspect_segment = 0;
        self.inspect_scroll.jump_top();
        self.inspector = Some((pid, inspect::inspect(pid)));
    }

    /// The process the inspector shows, as the last reading has it (None once it has gone from the list).
    pub fn inspected_proc(&self) -> Option<&ProcSample> {
        let (pid, _) = self.inspector.as_ref()?;
        self.snapshot.as_ref()?.procs.iter().find(|p| p.pid == *pid)
    }

    /// The inspector's panel: the report's area, under the toolbar and down to the footer.
    pub fn inspector_rect(&self, ui: &Ui) -> Rect {
        self.report_rect(ui)
    }

    /// The segmented control of the inspector, under the title.
    pub fn inspector_seg_rect(&self, ui: &Ui) -> Rect {
        let r = self.inspector_rect(ui);
        Rect { x: r.x + INSPECT_PAD_PT, y: r.y + INSPECT_SEG_Y_PT, w: (r.w - 2.0 * INSPECT_PAD_PT).min(INSPECT_SEG_MAX_PT), h: SEG_H_PT }
    }

    /// The lines of the Open Files and Ports segment (see `inspect::file_and_port_lines`).
    pub fn inspector_lines(&self) -> Vec<(bool, String)> {
        self.inspector.as_ref().map_or_else(Vec::new, |(_, data)| inspect::file_and_port_lines(data))
    }

    /// Sets the Open Files and Ports list's scroll range from its line count.
    pub fn fit_inspect_scroll(&mut self, ui: &Ui) {
        let r = self.inspector_rect(ui);
        let body = (r.h - INSPECT_BODY_Y_PT - INSPECT_PAD_PT).max(0.0);
        let max = (self.inspector_lines().len() as f32 * INSPECT_ROW_PT - body).max(0.0);
        self.inspect_scroll.set_max(max);
    }

    /// Closes the inspector.
    pub fn close_inspector(&mut self) {
        self.inspector = None;
    }

    /// Reveal in Files: asks the app layer to show the inspected program. The app layer holds the grant for it.
    pub fn reveal_in_files(&mut self) {
        let Some((_, data)) = &self.inspector else { return };
        if data.exe.is_empty() {
            self.status = "This process has no program file to show".into();
            return;
        }
        self.reveal = Some(PathBuf::from(&data.exe));
        self.status = "Showing the program in Files".into();
    }

    /// Hands the app layer the program to reveal, once.
    pub fn take_reveal(&mut self) -> Option<PathBuf> {
        self.reveal.take()
    }

    // --- layout ---

    pub fn geo(&self, ui: &Ui) -> Geo {
        let k = ui.pt(1.0);
        let (w, h) = (self.w / k, self.h / k);
        let content_x = SIDE_PT + INSET_PT;
        let content_w = (w - content_x - INSET_PT).max(0.0);
        let seg_y = UNIFIED_BAR_PT + GAP_PT;
        let graph_y = seg_y + SEG_H_PT + GAP_PT;
        let head_y = graph_y + GRAPH_PT + GAP_PT;
        let table_top = head_y + HEAD_PT;
        Geo {
            w,
            h,
            content_x,
            content_w,
            seg_y,
            seg_w: content_w.min(SEG_MAX_PT),
            graph_y,
            head_y,
            table_top,
            table_bottom: (h - FOOT_PT).max(table_top),
        }
    }

    /// The tab control's rectangle.
    pub fn seg_rect(&self, ui: &Ui) -> Rect {
        let g = self.geo(ui);
        Rect { x: g.content_x, y: g.seg_y, w: g.seg_w, h: SEG_H_PT }
    }

    /// The graph card's rectangle.
    pub fn graph_rect(&self, ui: &Ui) -> Rect {
        let g = self.geo(ui);
        Rect { x: g.content_x, y: g.graph_y, w: g.content_w, h: GRAPH_PT }
    }

    /// The table's columns with their x positions and widths in points, the fill column taking the rest.
    pub fn col_frames(&self, ui: &Ui) -> Vec<(Column, f32, f32)> {
        let g = self.geo(ui);
        let cols = self.tab.columns(&self.columns);
        let fixed: f32 = cols.iter().map(|c| c.width).sum();
        let fill = (g.content_w - fixed).max(80.0);
        let mut x = g.content_x;
        cols.into_iter()
            .map(|c| {
                let w = if c.width == 0.0 { fill } else { c.width };
                let frame = (c.clone(), x, w);
                x += w;
                frame
            })
            .collect()
    }

    /// The number of table lines on the current tab (interfaces on Network, processes elsewhere).
    pub fn line_count(&self) -> usize {
        if self.tab == Tab::Network {
            self.snapshot.as_ref().map_or(0, |s| s.ifaces.len())
        } else {
            self.rows.len()
        }
    }

    /// The table's largest scroll offset, set on the scroller.
    pub fn fit_scroll(&mut self, ui: &Ui) -> f32 {
        let g = self.geo(ui);
        let content = self.line_count() as f32 * ROW_PT;
        let max = (content - (g.table_bottom - g.table_top)).max(0.0);
        self.scroll.set_max(max);
        max
    }

    /// The table row under a point, given the scroll (process rows only).
    fn row_at(&self, ui: &Ui, y: f32) -> Option<u32> {
        let g = self.geo(ui);
        if y < g.table_top || y > g.table_bottom {
            return None;
        }
        let index = ((y - g.table_top + self.scroll.offset()) / ROW_PT).floor();
        if index < 0.0 || self.tab == Tab::Network {
            return None;
        }
        self.rows.get(index as usize).map(|p| p.pid)
    }

    /// The sidebar's source list, with the selected filter.
    pub fn sidebar(&self) -> SourceList {
        let items = vec![
            SidebarItem::header("Processes"),
            SidebarItem::row(ROW_ALL, Filter::All.label()).with_symbol("list.bullet"),
            SidebarItem::row(ROW_MINE, Filter::Mine.label()).with_symbol("person"),
            SidebarItem::row(ROW_OTHER, Filter::Others.label()).with_symbol("person.2"),
            SidebarItem::row(ROW_SYSTEM, Filter::System.label()).with_symbol("gearshape"),
            SidebarItem::row(ROW_ACTIVE, Filter::Active.label()).with_symbol("bolt"),
            SidebarItem::row(ROW_INACTIVE, Filter::Inactive.label()).with_symbol("moon"),
            SidebarItem::row(ROW_WINDOWED, Filter::Windowed.label()).with_symbol("rectangle.grid.1x2"),
            SidebarItem::row(ROW_SELECTED, Filter::Selected.label()).with_symbol("checkmark.circle"),
            SidebarItem::row(ROW_GPU, Filter::Gpu.label()).with_symbol("desktopcomputer"),
            SidebarItem::row(ROW_RECENT, Filter::Recent.label()).with_symbol("clock"),
        ];
        let mut list = SourceList::new(items, 28.0, 26.0);
        list.set_selected(Some(match self.filter {
            Filter::All => ROW_ALL,
            Filter::Mine => ROW_MINE,
            Filter::Others => ROW_OTHER,
            Filter::System => ROW_SYSTEM,
            Filter::Active => ROW_ACTIVE,
            Filter::Inactive => ROW_INACTIVE,
            Filter::Windowed => ROW_WINDOWED,
            Filter::Selected => ROW_SELECTED,
            Filter::Gpu => ROW_GPU,
            Filter::Recent => ROW_RECENT,
        }));
        list
    }

    fn sidebar_hit(&self, ui: &Ui, x: f32, y: f32) -> Option<u32> {
        if x >= SIDE_PT {
            return None;
        }
        let bounds = Rect { x: 0.0, y: 0.0, w: SIDE_PT, h: self.h / ui.pt(1.0) };
        self.sidebar().hit(bounds, SIDE_SCROLL, x, y)
    }

    fn select_filter(&mut self, id: u32) {
        let filter = match id {
            ROW_MINE => Filter::Mine,
            ROW_OTHER => Filter::Others,
            ROW_SYSTEM => Filter::System,
            ROW_ACTIVE => Filter::Active,
            ROW_INACTIVE => Filter::Inactive,
            ROW_WINDOWED => Filter::Windowed,
            ROW_SELECTED => Filter::Selected,
            ROW_GPU => Filter::Gpu,
            ROW_RECENT => Filter::Recent,
            _ => Filter::All,
        };
        self.set_filter(filter);
    }

    /// The toolbar band's rectangle: the capsule row over the content.
    pub fn band(&self, ui: &Ui) -> Rect {
        let w = self.w / ui.pt(1.0);
        Rect { x: SIDE_PT + 12.0, y: (UNIFIED_BAR_PT - CAPSULE_PT) / 2.0, w: (w - SIDE_PT - 24.0).max(0.0), h: CAPSULE_PT }
    }

    fn bar(&self) -> Toolbar {
        if self.inspector.is_some() {
            return Toolbar::new(vec![
                ToolbarItem::button(B_INSPECT_DONE, "Done"),
                ToolbarItem::button(B_REVEAL, "Reveal in Files"),
                ToolbarItem::flexible_space(),
            ]);
        }
        if self.report.is_some() {
            return Toolbar::new(vec![
                ToolbarItem::button(B_REPORT_CLOSE, "Done"),
                ToolbarItem::button(B_REPORT_SAVE, "Save..."),
                ToolbarItem::button(B_REPORT_COPY, "Copy"),
                ToolbarItem::flexible_space(),
            ]);
        }
        let mut items = vec![
            ToolbarItem::button(B_QUIT, "Quit Process").enabled(self.can_quit_selected()),
            ToolbarItem::button(B_FORCE, "Force Quit").enabled(self.can_quit_selected()),
            ToolbarItem::button(B_INSPECT, "Inspect").enabled(self.has_selected()),
        ];
        if self.job.is_some() {
            items.push(ToolbarItem::button(B_STOP, "Stop"));
        }
        items.push(ToolbarItem::flexible_space());
        items.push(ToolbarItem::custom(B_SEARCH, "Search", SEARCH_PT));
        Toolbar::new(items)
    }

    pub fn bar_layout(&self, ui: &Ui) -> (Toolbar, ToolbarLayout) {
        let bar = self.bar();
        let layout = bar.layout_with(self.band(ui), item_w(ui), CAPSULE_PT, TOOLBAR_GAP_PT);
        (bar, layout)
    }

    fn toolbar_hit(&self, ui: &Ui, x: f32, y: f32) -> Option<u32> {
        if y >= UNIFIED_BAR_PT {
            return None;
        }
        let (bar, layout) = self.bar_layout(ui);
        let id = layout.hit(x, y)?;
        bar.items.iter().find(|i| i.id == id).filter(|i| i.enabled).map(|_| id)
    }

    /// Which column header a point is over (the index into `col_frames`).
    fn head_hit(&self, ui: &Ui, x: f32, y: f32) -> Option<usize> {
        let g = self.geo(ui);
        if y < g.head_y || y >= g.table_top {
            return None;
        }
        self.col_frames(ui).iter().position(|(_, cx, cw)| x >= *cx && x < cx + cw)
    }

    // --- actions ---

    fn activate(&mut self, id: u32) {
        match id {
            B_QUIT => self.quit_selected(),
            B_FORCE => self.force_quit_selected(),
            B_INSPECT => self.toggle_inspector(),
            B_STOP => self.stop_job(),
            B_REPORT_CLOSE => self.close_report(),
            B_REPORT_SAVE => self.save_report(),
            B_REPORT_COPY => {
                if let Some(report) = &self.report {
                    bifrost_app::kit::clipboard::set_text(&report.text);
                    self.status = "Copied the report".into();
                }
            }
            B_INSPECT_DONE => self.close_inspector(),
            B_REVEAL => self.reveal_in_files(),
            B_SEARCH => self.search_focused = true,
            _ => {}
        }
    }

    pub fn focus_search(&mut self) {
        self.search_focused = true;
    }

    /// Typed text goes to the search field while it has focus. True when a frame is needed.
    pub fn text(&mut self, text: &str) -> bool {
        if !self.search_focused {
            return false;
        }
        self.query.push_str(text);
        self.rebuild();
        true
    }

    /// Editing keys (shell key codes: 8 backspace, 1 escape). True when a frame is needed.
    pub fn key(&mut self, code: u32) -> bool {
        // the sheets take the keys while they are open: Escape closes, Return confirms, the arrows move the signal
        if self.quit_sheet.is_some() {
            match code {
                1 | 27 => self.quit_sheet = None,
                13 => self.confirm_quit_default(),
                _ => return false,
            }
            return true;
        }
        if self.signal_sheet.is_some() {
            match code {
                1 | 27 => self.signal_sheet = None,
                38 => self.move_signal_choice(-1),
                40 => self.move_signal_choice(1),
                13 => self.send_chosen_signal(),
                _ => return false,
            }
            return true;
        }
        // Escape leaves the inspector and the report
        if self.inspector.is_some() && code == 1 {
            self.close_inspector();
            return true;
        }
        if self.report.is_some() && code == 1 {
            self.report = None;
            return true;
        }
        if !self.search_focused {
            return false;
        }
        match code {
            8 => {
                self.query.pop();
                self.rebuild();
            }
            1 => {
                if self.query.is_empty() {
                    self.search_focused = false;
                } else {
                    self.query.clear();
                    self.rebuild();
                }
            }
            _ => return false,
        }
        true
    }

    /// What a press in the inspector's panel starts: a segment, or a drag of its list.
    fn inspector_press(&mut self, ui: &Ui, x: f32, y: f32, t: f64) -> Press {
        let seg = self.inspector_seg_rect(ui);
        if y >= seg.y && y < seg.y + seg.h && x >= seg.x && x < seg.x + seg.w {
            return sheet::segment_hit(seg, INSPECT_SEGMENTS.len(), x, y).map_or(Press::None, Press::InspectSeg);
        }
        let g = self.geo(ui);
        let body_top = self.inspector_rect(ui).y + INSPECT_BODY_Y_PT;
        if y >= body_top && y < g.table_bottom {
            self.inspect_scroll.down(y, t);
            return Press::Inspect;
        }
        Press::None
    }

    /// A touch. Pixels in, points used inside. True when a frame is needed.
    pub fn touch(&mut self, ui: &Ui, kind: TouchKind, x: f32, y: f32, t: f64) -> bool {
        let k = ui.pt(1.0);
        let (x, y) = (x / k, y / k);
        match kind {
            TouchKind::Down => {
                // the sheet takes every press while it is open
                if self.signal_sheet.is_some() || self.quit_sheet.is_some() {
                    self.press = Press::Sheet;
                    self.search_focused = false;
                    return true;
                }
                let g = self.geo(ui);
                let seg = self.seg_rect(ui);
                self.press = if x < SIDE_PT {
                    Press::Sidebar
                } else if y < UNIFIED_BAR_PT {
                    self.toolbar_hit(ui, x, y).map_or(Press::None, Press::Toolbar)
                } else if self.inspector.is_some() {
                    self.inspector_press(ui, x, y, t)
                } else if self.report.is_some() {
                    // the report fills the content; it scrolls by dragging
                    if y < g.table_bottom {
                        if let Some(report) = &mut self.report {
                            report.scroll.down(y, t);
                        }
                        Press::Report
                    } else {
                        Press::None
                    }
                } else if y >= seg.y && y < seg.y + seg.h && x >= seg.x && x < seg.x + seg.w {
                    sheet::segment_hit(seg, Tab::ALL.len(), x, y).map_or(Press::None, Press::Tab)
                } else if let Some(i) = self.head_hit(ui, x, y) {
                    Press::Head(i)
                } else if y >= g.table_top && y <= g.table_bottom && x >= g.content_x && x < g.content_x + g.content_w {
                    self.scroll.down(y, t);
                    Press::Table
                } else {
                    Press::None
                };
                if self.press != Press::Toolbar(B_SEARCH) {
                    self.search_focused = false;
                }
            }
            TouchKind::Move => {
                if self.press == Press::Table {
                    self.scroll.moved(y, t);
                }
                if self.press == Press::Inspect {
                    self.inspect_scroll.moved(y, t);
                }
                if self.press == Press::Report {
                    if let Some(report) = &mut self.report {
                        report.scroll.moved(y, t);
                    }
                }
            }
            TouchKind::Up | TouchKind::Cancel => {
                let press = std::mem::replace(&mut self.press, Press::None);
                match press {
                    Press::Toolbar(id) => {
                        if self.toolbar_hit(ui, x, y) == Some(id) {
                            self.activate(id);
                        }
                    }
                    Press::Sidebar => {
                        if let Some(id) = self.sidebar_hit(ui, x, y) {
                            self.report = None;
                            self.inspector = None;
                            self.select_filter(id);
                        }
                    }
                    Press::Inspect => {
                        self.inspect_scroll.up(y, t);
                    }
                    Press::InspectSeg(i) => {
                        let seg = self.inspector_seg_rect(ui);
                        if sheet::segment_hit(seg, INSPECT_SEGMENTS.len(), x, y) == Some(i) {
                            self.inspect_segment = i;
                            self.inspect_scroll.jump_top();
                        }
                    }
                    Press::Report => {
                        if let Some(report) = &mut self.report {
                            report.scroll.up(y, t);
                        }
                    }
                    Press::Sheet => {
                        if self.quit_sheet.is_some() {
                            self.quit_sheet_released(ui, x, y);
                        } else {
                            self.sheet_released(ui, x, y);
                        }
                    }
                    Press::Tab(i) => {
                        let seg = self.seg_rect(ui);
                        if sheet::segment_hit(seg, Tab::ALL.len(), x, y) == Some(i) {
                            self.set_tab(Tab::from_index(i));
                        }
                    }
                    Press::Head(i) => {
                        if self.head_hit(ui, x, y) == Some(i) {
                            if let Some(key) = self.col_frames(ui).get(i).and_then(|(c, _, _)| c.key) {
                                self.sort_clicked(key);
                            }
                        }
                    }
                    Press::Table => {
                        // `up` reports a tap (no drag): that selects the row under the finger
                        if self.scroll.up(y, t) {
                            self.selected = self.row_at(ui, y);
                        }
                    }
                    Press::None => {}
                }
            }
        }
        true
    }

    /// Accessibility nodes for the toolbar items and the search field, in points.
    pub fn controls(&self, ui: &Ui) -> Vec<Node> {
        let (bar, layout) = self.bar_layout(ui);
        let mut nodes = Vec::new();
        for (id, r) in layout.visible {
            let Some(item) = bar.items.iter().find(|i| i.id == id) else { continue };
            match id {
                B_SEARCH => nodes.push(Node { id: id as u64, role: Role::TextField, label: "Search".into(), value: self.query.to_string(), bounds: r, enabled: true }),
                _ => nodes.push(Node { id: id as u64, role: Role::Button, label: item.label.clone(), value: String::new(), bounds: r, enabled: item.enabled }),
            }
        }
        nodes
    }

    /// The footer line: counts and the machine's totals.
    pub fn footer_text(&self) -> String {
        let Some(s) = &self.snapshot else { return "Reading the system".into() };
        let procs = format!("{} processes", s.procs.len());
        let cpu = format!("CPU {}", model::percent(s.cpu.busy));
        let mem = format!("Memory {} of {}", model::bytes(s.memory.used()), model::bytes(s.memory.total));
        format!("{procs}   {cpu}   {mem}")
    }
}

fn item_w(ui: &Ui) -> impl Fn(&ToolbarItem) -> f32 + '_ {
    move |it| {
        if it.symbol.is_some() {
            CAPSULE_PT
        } else {
            (ui.measure(&it.label, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0) + 2.0 * SPACE_S * 1.5).max(CAPSULE_PT)
        }
    }
}

extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}
const SIGKILL: i32 = 9;
const SIGTERM: i32 = 15;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inspector_needs_a_selected_process() {
        let mut m = Monitor::new(800.0, 600.0);
        assert!(!m.has_selected());
        m.toggle_inspector();
        assert!(m.inspector.is_none());
    }

    #[test]
    fn the_inspector_opens_on_the_selected_pid_and_closes_again() {
        let mut m = Monitor::new(800.0, 600.0);
        m.selected = Some(std::process::id());
        m.toggle_inspector();
        let (pid, data) = m.inspector.clone().expect("open");
        assert_eq!(pid, std::process::id());
        assert!(!data.exe.is_empty());
        m.toggle_inspector();
        assert!(m.inspector.is_none());
    }

    #[test]
    fn copy_text_follows_the_selection_and_tab() {
        let mut m = Monitor::new(800.0, 600.0);
        assert_eq!(m.copy_text(), None);
        m.rows = vec![crate::sample::ProcSample { pid: 7, name: "sh".into(), user: "me".into(), uid: 1, cpu: 1.0, threads: 2, ..Default::default() }];
        m.selected = Some(7);
        assert_eq!(m.copy_text().as_deref(), Some("sh\tme\t1.0%\t2\t7"));
    }

    #[test]
    fn a_slower_update_frequency_delays_the_next_reading() {
        let mut m = Monitor::new(800.0, 600.0);
        m.last_sample = Some(1000.0);
        assert_eq!(m.next_wake(1500.0), 2000.0);
        m.set_frequency(UpdateFrequency::Normally);
        assert_eq!(m.next_wake(1500.0), 6000.0);
    }

    #[test]
    fn signals_need_a_signalable_selection() {
        let mut m = Monitor::new(800.0, 600.0);
        m.send_signal(model::SIGNALS[0]);
        assert!(m.status.is_empty());
    }

    #[test]
    fn force_quit_without_a_selection_does_nothing() {
        let mut m = Monitor::new(800.0, 600.0);
        m.force_quit_selected();
        m.quit_selected();
        assert!(m.status.is_empty());
    }

    fn proc(pid: u32, ppid: u32, name: &str) -> ProcSample {
        ProcSample { pid, ppid, name: name.into(), ..Default::default() }
    }

    fn with_rows(m: &mut Monitor, procs: Vec<ProcSample>) {
        m.snapshot = Some(Snapshot {
            cpu: Default::default(),
            memory: Default::default(),
            disk_read_bps: 0.0,
            disk_write_bps: 0.0,
            ifaces: Vec::new(),
            procs,
            cores: Vec::new(),
            gpu_busy: None,
        });
        m.rebuild();
    }

    #[test]
    fn selected_processes_shows_the_family_of_the_selection() {
        let mut m = Monitor::new(800.0, 600.0);
        with_rows(&mut m, vec![proc(1, 0, "init"), proc(10, 1, "shell"), proc(11, 10, "ls"), proc(12, 1, "other")]);
        m.set_filter(Filter::Selected);
        assert_eq!(m.filter, Filter::All, "needs a selection");
        assert_eq!(m.status, "Select a process first");
        m.selected = Some(10);
        m.set_filter(Filter::Selected);
        let pids: Vec<u32> = m.rows.iter().map(|p| p.pid).collect();
        assert_eq!(pids, vec![10, 11]);
        // the process ends: the filter gives way to All
        with_rows(&mut m, vec![proc(1, 0, "init"), proc(12, 1, "other")]);
        assert_eq!(m.filter, Filter::All);
        assert_eq!(m.rows.len(), 2);
    }

    #[test]
    fn the_dock_tile_follows_the_mode_and_clears_with_it() {
        let mut m = Monitor::new(800.0, 600.0);
        assert_eq!(m.take_dock_update(), None);
        m.set_dock_mode(DockMode::Off);
        assert_eq!(m.take_dock_update(), None, "no change, nothing to send");
        m.set_dock_mode(DockMode::Cpu);
        assert_eq!(m.take_dock_update(), Some(DockContent::cleared()), "no reading yet");
        with_rows(&mut m, vec![proc(1, 0, "init")]);
        m.set_dock_mode(DockMode::Memory);
        let content = m.take_dock_update().expect("a tile");
        assert!(content.badge.ends_with('%'));
        assert!(content.fraction.is_some());
        m.set_dock_mode(DockMode::Off);
        assert_eq!(m.take_dock_update(), Some(DockContent::cleared()));
    }

    #[test]
    fn windowed_processes_follow_the_window_list() {
        let mut m = Monitor::new(800.0, 600.0);
        with_rows(&mut m, vec![proc(1, 0, "init"), proc(2, 1, "editor"), proc(3, 1, "daemon")]);
        assert!(!m.take_window_poll());
        m.set_filter(Filter::Windowed);
        assert!(m.rows.is_empty(), "no window owners are known yet");
        assert!(m.take_window_poll(), "choosing the filter asks for the list");
        assert!(!m.take_window_poll(), "once");
        m.set_window_owners(HashSet::from([2]));
        assert_eq!(m.rows.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![2]);
        m.set_filter(Filter::All);
        assert_eq!(m.rows.len(), 3);
        assert!(!m.take_window_poll());
    }

    #[test]
    fn deltas_toggle_on_the_selected_process_and_end_with_it() {
        let mut m = Monitor::new(800.0, 600.0);
        with_rows(&mut m, vec![proc(10, 1, "shell"), proc(11, 1, "ls")]);
        m.toggle_deltas();
        assert!(m.baselines.is_empty(), "nothing selected");
        m.selected = Some(10);
        m.toggle_deltas();
        assert!(m.deltas_on_selected());
        m.toggle_deltas();
        assert!(!m.deltas_on_selected());
        m.toggle_deltas();
        with_rows(&mut m, vec![proc(11, 1, "ls")]);
        assert!(m.baselines.is_empty(), "the baseline leaves with the process");
    }

    #[test]
    fn hiding_the_sorted_column_resets_the_sort() {
        let mut m = Monitor::new(800.0, 600.0);
        let index = Tab::Cpu.all_columns().iter().position(|c| c.key == Some(SortKey::Threads)).unwrap();
        m.sort = Sort { key: SortKey::Threads, descending: true };
        let pref = m.toggle_column(index).expect("threads can be hidden");
        assert_eq!(pref, "CPU/Threads");
        assert_eq!(m.sort, Tab::Cpu.default_sort());
        assert!(m.tab.columns(&m.columns).iter().all(|c| c.key != Some(SortKey::Threads)));
        // the % CPU column is required
        let required = Tab::Cpu.all_columns().iter().position(|c| c.key == Some(SortKey::Cpu)).unwrap();
        assert_eq!(m.toggle_column(required), None);
        assert_eq!(m.toggle_column(99), None);
    }

    #[test]
    fn the_gpu_and_sleep_readings_follow_what_is_shown() {
        let mut m = Monitor::new(800.0, 600.0);
        assert!(!m.sampler.scan_gpu && !m.sampler.scan_sleep);
        m.last_sample = Some(5.0);
        m.set_tab(Tab::Gpu);
        assert!(m.sampler.scan_gpu);
        assert_eq!(m.last_sample, None, "a reading just turned on is taken at once");
        m.set_tab(Tab::Energy);
        assert!(!m.sampler.scan_gpu && m.sampler.scan_sleep);
        m.set_tab(Tab::Cpu);
        assert!(!m.sampler.scan_sleep);
        m.set_filter(Filter::Gpu);
        assert!(m.sampler.scan_gpu);
    }

    #[test]
    fn copy_follows_the_shown_columns_and_the_baseline() {
        let mut m = Monitor::new(800.0, 600.0);
        let cpu_time = Tab::Cpu.all_columns().iter().position(|c| c.key == Some(SortKey::CpuTime)).unwrap();
        m.toggle_column(cpu_time);
        let mut p = proc(7, 1, "sh");
        p.user = "me".into();
        p.cpu_ticks = 500;
        m.rows = vec![p.clone()];
        m.selected = Some(7);
        assert_eq!(m.copy_text().as_deref(), Some("sh\tme\t0.0%\t0:05.00\t0\t7"));
        let mut base = p.clone();
        base.cpu_ticks = 200;
        m.baselines.insert(7, base);
        assert_eq!(m.copy_text().as_deref(), Some("sh\tme\t0.0%\t0:03.00\t0\t7"));
    }

    fn wait_for_job(m: &mut Monitor) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while m.job.is_some() {
            m.poll_job();
            assert!(std::time::Instant::now() < deadline, "the job did not finish");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn a_finished_sample_becomes_a_report_that_can_be_saved() {
        let mut m = Monitor::new(800.0, 600.0);
        m.job = Some(Job::start(JobKind::Sample, "Sampling x", |_| Outcome::Report { title: "Sample of a/b".into(), text: "one\ntwo\nthree".into() }));
        assert!(m.job_running());
        wait_for_job(&mut m);
        let report = m.report.as_ref().expect("report");
        assert_eq!(report.lines, vec!["one", "two", "three"]);
        assert_eq!(report.save_name(), "Sample of a-b.txt");
        m.save_report();
        assert_eq!(m.take_request(), Some(Request::SaveReport { name: "Sample of a-b.txt".into(), text: "one\ntwo\nthree".into() }));
        assert_eq!(m.take_request(), None, "a request is handed out once");
        m.close_report();
        assert!(m.report.is_none());
    }

    #[test]
    fn a_finished_bundle_asks_for_a_place_to_save_it() {
        let mut m = Monitor::new(800.0, 600.0);
        m.job = Some(Job::start(JobKind::Diagnostics, "Gathering", |_| Outcome::Bundle(PathBuf::from("/data/bundle.zip"))));
        wait_for_job(&mut m);
        assert_eq!(m.take_request(), Some(Request::SaveBundle(PathBuf::from("/data/bundle.zip"))));
        assert!(m.report.is_none());
    }

    #[test]
    fn a_failed_or_stopped_job_says_so_in_the_footer() {
        let mut m = Monitor::new(800.0, 600.0);
        m.job = Some(Job::start(JobKind::Sample, "Sampling", |_| Outcome::Failed("x ended before it could be sampled".into())));
        wait_for_job(&mut m);
        assert_eq!(m.status, "x ended before it could be sampled");
        m.job = Some(Job::start(JobKind::Sample, "Sampling", |p| {
            while !p.cancelled() {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Outcome::Cancelled
        }));
        m.stop_job();
        wait_for_job(&mut m);
        assert_eq!(m.status, "Stopped");
    }

    #[test]
    fn sampling_needs_a_selection_and_one_job_at_a_time() {
        let mut m = Monitor::new(800.0, 600.0);
        m.start_sample();
        assert!(!m.job_running(), "nothing selected");
        with_rows(&mut m, vec![proc(std::process::id(), 1, "self")]);
        m.selected = Some(std::process::id());
        m.start_sample();
        assert!(m.job_running());
        let label = m.job.as_ref().map(|j| j.label.clone());
        assert_eq!(label.as_deref(), Some("Sampling self"));
        m.start_diagnostics();
        assert_eq!(m.job.as_ref().map(|j| j.kind), Some(JobKind::Sample), "a second job does not replace the first");
        m.stop_job();
        wait_for_job(&mut m);
    }

    #[test]
    fn a_running_job_does_not_add_a_progress_poll_timer() {
        let mut m = Monitor::new(800.0, 600.0);
        m.last_sample = Some(1000.0);
        m.frequency = UpdateFrequency::Normally;
        assert_eq!(m.next_wake(1100.0), 6000.0);
        m.job = Some(Job::start(JobKind::Sample, "Sampling", |p| {
            while !p.cancelled() {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Outcome::Cancelled
        }));
        assert_eq!(m.next_wake(1100.0), 6000.0);
        m.stop_job();
        wait_for_job(&mut m);
    }

    #[test]
    fn the_toolbar_switches_to_the_report_actions_and_escape_leaves() {
        let mut m = Monitor::new(800.0, 600.0);
        let ids = |m: &Monitor| m.bar().items.iter().map(|i| i.id).collect::<Vec<_>>();
        assert!(ids(&m).contains(&B_SEARCH));
        assert!(!ids(&m).contains(&B_STOP));
        m.report = Some(Report::new("T", "a"));
        assert_eq!(&ids(&m)[..3], &[B_REPORT_CLOSE, B_REPORT_SAVE, B_REPORT_COPY]);
        assert!(!ids(&m).contains(&B_SEARCH));
        assert!(m.key(1));
        assert!(m.report.is_none());
    }

    #[test]
    fn job_progress_reads_as_seconds_or_items() {
        let mut m = Monitor::new(800.0, 600.0);
        assert_eq!(m.job_text(), "");
        m.job = Some(Job::start(JobKind::Sample, "Sampling", |p| {
            p.set(4000, 10_000);
            while !p.cancelled() {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Outcome::Cancelled
        }));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while m.job_text() == "Starting" {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(m.job_text(), "4 of 10 seconds");
        m.stop_job();
        wait_for_job(&mut m);
    }

    fn card() -> Rect {
        Rect { x: 100.0, y: 50.0, w: SHEET_W_PT, h: SHEET_HEAD_PT + 8.0 * SHEET_ROW_PT + SHEET_FOOT_PT }
    }

    #[test]
    fn the_signal_sheet_hit_tests_rows_and_buttons() {
        let c = card();
        for i in 0..model::SIGNALS.len() {
            let r = signal_sheet_row(c, i);
            assert_eq!(signal_sheet_hit(c, r.x + 5.0, r.y + r.h / 2.0), Some(SheetHit::Row(i)));
        }
        let (cancel, send) = signal_sheet_buttons(c);
        assert!(cancel.x + cancel.w < send.x, "Cancel sits left of Send");
        assert_eq!(signal_sheet_hit(c, cancel.x + 1.0, cancel.y + 1.0), Some(SheetHit::Cancel));
        assert_eq!(signal_sheet_hit(c, send.x + 1.0, send.y + 1.0), Some(SheetHit::Send));
        assert_eq!(signal_sheet_hit(c, c.x + 1.0, c.y + 1.0), None, "the title area is nothing");
        assert!(send.y + send.h <= c.y + c.h, "the buttons fit in the card");
        let last = signal_sheet_row(c, model::SIGNALS.len() - 1);
        assert!(last.y + last.h <= send.y, "the rows end above the buttons");
    }

    #[test]
    fn the_sheet_only_opens_for_a_process_that_may_be_signalled() {
        let mut m = Monitor::new(800.0, 600.0);
        m.open_signal_sheet();
        assert_eq!(m.signal_sheet, None, "nothing selected");
        let own = m.sampler.own_uid;
        let mut mine = proc(4_000_000, 1, "mine");
        mine.uid = own;
        let mut theirs = proc(4_000_001, 1, "theirs");
        theirs.uid = own.wrapping_add(1);
        with_rows(&mut m, vec![mine, theirs]);
        m.selected = Some(4_000_001);
        m.open_signal_sheet();
        assert_eq!(m.signal_sheet, None, "someone else's process");
        m.selected = Some(4_000_000);
        m.open_signal_sheet();
        assert_eq!(m.signal_sheet.map(|i| model::SIGNALS[i].name), Some("TERM"));
        assert!(m.key(1));
        assert_eq!(m.signal_sheet, None, "Escape closes the sheet");
    }

    fn quit_card() -> Rect {
        Rect { x: 100.0, y: 50.0, w: SHEET_W_PT, h: QUIT_HEAD_PT + QUIT_FOOT_PT }
    }

    #[test]
    fn the_quit_buttons_are_cancel_force_quit_and_quit_left_to_right() {
        let c = quit_card();
        let [cancel, force, quit] = quit_sheet_buttons(c);
        assert!(cancel.x + cancel.w < force.x && force.x + force.w < quit.x, "left to right");
        assert!(quit.x + quit.w <= c.x + c.w && cancel.x >= c.x, "the buttons fit the card");
        assert_eq!(quit_sheet_hit(c, cancel.x + 1.0, cancel.y + 1.0), Some(QuitHit::Cancel));
        assert_eq!(quit_sheet_hit(c, force.x + 1.0, force.y + 1.0), Some(QuitHit::Force));
        assert_eq!(quit_sheet_hit(c, quit.x + 1.0, quit.y + 1.0), Some(QuitHit::Quit));
        assert_eq!(quit_sheet_hit(c, c.x + 1.0, c.y + 1.0), None, "the title is nothing");
    }

    #[test]
    fn quit_asks_first_and_cancel_keeps_the_process() {
        let mut m = Monitor::new(800.0, 600.0);
        let own = m.sampler.own_uid;
        let mut mine = proc(4_000_010, 1, "mine");
        mine.uid = own;
        with_rows(&mut m, vec![mine]);
        m.selected = Some(4_000_010);
        m.quit_selected();
        assert_eq!(m.quit_sheet, Some(QuitSheet { pid: 4_000_010, force_default: false }));
        assert!(m.status.is_empty(), "nothing sent yet");
        assert!(m.key(27), "Escape closes the confirmation");
        assert_eq!(m.quit_sheet, None);
        m.force_quit_selected();
        assert_eq!(m.quit_sheet.map(|q| q.force_default), Some(true));
        assert!(m.key(13), "Return chooses the default");
        assert_eq!(m.quit_sheet, None);
        assert_eq!(m.status, "Could not force quit mine", "the process does not exist, so the kill is refused");
    }

    #[test]
    fn the_quit_confirmation_refuses_other_users_processes() {
        let mut m = Monitor::new(800.0, 600.0);
        let own = m.sampler.own_uid;
        let mut theirs = proc(4_000_011, 1, "theirs");
        theirs.uid = own.wrapping_add(1);
        with_rows(&mut m, vec![theirs]);
        m.selected = Some(4_000_011);
        m.quit_selected();
        assert_eq!(m.quit_sheet, None, "no sheet for someone else's process");
    }

    #[test]
    fn the_signal_sheet_moves_with_the_arrows_and_sends_on_return() {
        let mut m = Monitor::new(800.0, 600.0);
        let own = m.sampler.own_uid;
        let mut mine = proc(4_000_012, 1, "mine");
        mine.uid = own;
        with_rows(&mut m, vec![mine]);
        m.selected = Some(4_000_012);
        m.open_signal_sheet();
        let name = |m: &Monitor| m.signal_sheet.map(|i| model::SIGNALS[i].name);
        assert_eq!(name(&m), Some("TERM"));
        assert!(m.key(40));
        assert_eq!(name(&m), Some("USR1"), "one row down from TERM");
        for _ in 0..20 {
            m.key(40);
        }
        assert_eq!(name(&m), Some("CONT"), "stops at the last signal");
        m.key(38);
        assert_eq!(name(&m), Some("STOP"));
        assert!(!m.key(65), "other keys are not taken by the sheet");
        assert!(m.key(13));
        assert_eq!(m.signal_sheet, None);
        assert_eq!(m.status, "Could not signal STOP mine", "the process does not exist, so the signal is refused");
    }

    #[test]
    fn the_inspector_opens_on_memory_and_closes_with_escape() {
        let mut m = Monitor::new(800.0, 600.0);
        m.selected = Some(std::process::id());
        m.inspect_segment = 2;
        m.toggle_inspector();
        assert_eq!(m.inspect_segment, 0, "a new inspection starts on Memory");
        assert!(m.inspector.is_some());
        let ids = |m: &Monitor| m.bar().items.iter().map(|i| i.id).collect::<Vec<_>>();
        assert_eq!(&ids(&m)[..2], &[B_INSPECT_DONE, B_REVEAL]);
        assert!(!ids(&m).contains(&B_SEARCH));
        assert!(m.key(1));
        assert!(m.inspector.is_none());
    }

    #[test]
    fn reveal_asks_for_the_inspected_program() {
        let mut m = Monitor::new(800.0, 600.0);
        assert_eq!(m.take_reveal(), None);
        m.selected = Some(std::process::id());
        m.toggle_inspector();
        m.reveal_in_files();
        let path = m.take_reveal().expect("the program of this process");
        assert!(!path.as_os_str().is_empty());
        assert_eq!(m.take_reveal(), None, "handed out once");
        m.inspector.as_mut().unwrap().1.exe.clear();
        m.reveal_in_files();
        assert_eq!(m.take_reveal(), None);
        assert_eq!(m.status, "This process has no program file to show");
    }

    #[test]
    fn the_inspector_lists_files_then_ports() {
        let mut m = Monitor::new(800.0, 600.0);
        m.selected = Some(std::process::id());
        m.toggle_inspector();
        let lines = m.inspector_lines();
        assert!(lines.len() >= 2, "a heading for the files and one for the ports");
        assert!(lines[0].0, "the first line is a heading");
    }

    #[test]
    fn the_sample_duration_takes_only_the_offered_lengths() {
        let mut m = Monitor::new(800.0, 600.0);
        assert_eq!(m.sample_seconds, 10);
        assert!(m.set_sample_seconds(30));
        assert_eq!(m.sample_seconds, 30);
        assert!(!m.set_sample_seconds(7));
        assert_eq!(m.sample_seconds, 30);
        // nothing selected: no sample starts, and the length is kept for the next one
        m.start_sample();
        assert!(!m.job_running());
    }

    #[test]
    fn the_energy_means_are_kept_in_the_app_storage() {
        let dir = std::env::current_dir().unwrap().join("target").join("activity-averages-test");
        std::fs::create_dir_all(&dir).unwrap();
        let mut busy = proc(1, 0, "build");
        busy.cpu = 40.0;
        let mut m = Monitor::new(800.0, 600.0);
        m.data_dir = dir.clone();
        m.averages.record(&[busy], 1.0);
        m.save_averages();
        let mut again = Monitor::new(800.0, 600.0);
        again.data_dir = dir.clone();
        again.load_averages();
        assert!((again.averages.mean("build") - 40.0).abs() < 1e-9);
        // without a storage folder nothing is written or read
        let mut bare = Monitor::new(800.0, 600.0);
        bare.load_averages();
        assert_eq!(bare.averages.mean("build"), 0.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_per_process_readings_are_taken_only_while_their_column_is_shown() {
        let mut m = Monitor::new(800.0, 600.0);
        let index = |tab: Tab, key: SortKey| tab.all_columns().iter().position(|c| c.key == Some(key)).unwrap();
        m.set_tab(Tab::Memory);
        assert!(!m.sampler.scan_private, "Private Memory is off by default");
        m.toggle_column(index(Tab::Memory, SortKey::PrivateMem));
        assert!(m.sampler.scan_private);
        m.set_tab(Tab::Cpu);
        assert!(!m.sampler.scan_private);
        m.set_tab(Tab::Energy);
        assert!(!m.sampler.scan_wakeups);
        m.toggle_column(index(Tab::Energy, SortKey::WakeUps));
        assert!(m.sampler.scan_wakeups && m.sampler.scan_sleep);
        m.set_tab(Tab::Cpu);
        m.toggle_column(index(Tab::Cpu, SortKey::Ports));
        assert!(m.sampler.scan_ports && !m.sampler.scan_wakeups);
    }

    #[test]
    fn the_new_columns_read_as_text_and_sort_by_their_figures() {
        let mut small = proc(1, 1000, "small");
        small.ports = Some(2);
        small.private = Some(4096);
        small.wakeups = 1.5;
        let mut large = proc(2, 1000, "large");
        large.ports = Some(9);
        large.private = None;
        large.wakeups = 0.2;
        let rows = model::visible(&[small.clone(), large.clone()], Filter::All, 1000, "", Sort { key: SortKey::Ports, descending: true });
        assert_eq!(rows.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![2, 1]);
        let rows = model::visible(&[small.clone(), large.clone()], Filter::All, 1000, "", Sort { key: SortKey::WakeUps, descending: true });
        assert_eq!(rows.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(model::cell_text(&large, Some(SortKey::PrivateMem), None), "-", "not readable");
        assert_eq!(model::cell_text(&small, Some(SortKey::PrivateMem), None), "4.0 KB");
        assert_eq!(model::cell_text(&small, Some(SortKey::WakeUps), None), "1.5/s");
        assert_eq!(model::cell_text(&small, Some(SortKey::Ports), None), "2");
    }
}
