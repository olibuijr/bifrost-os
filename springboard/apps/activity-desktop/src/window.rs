//! The Activity Monitor window's state and behaviour: the tab, the filter, the sort, the search, the selection, the
//! sampling clock and the geometry that drawing and pointer input share. Points for the kit (`ui.pt`), pixels only
//! where the surface is drawn.

use crate::inspect::{self, Inspection};
use crate::job::{Job, JobKind, Outcome};
use crate::model::{self, Column, ColumnSet, EnergyAverages, Filter, History, Signal, Sort, SortKey, Tab, UpdateFrequency};
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
use std::collections::HashMap;
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

/// Toolbar item ids (also the published control ids).
pub const B_QUIT: u32 = 1;
pub const B_SEARCH: u32 = 2;
pub const B_FORCE: u32 = 3;
pub const B_INSPECT: u32 = 4;
pub const B_STOP: u32 = 5;
pub const B_REPORT_CLOSE: u32 = 6;
pub const B_REPORT_SAVE: u32 = 7;
pub const B_REPORT_COPY: u32 = 8;
/// Sidebar row ids.
const ROW_ALL: u32 = 1;
const ROW_MINE: u32 = 2;
const ROW_OTHER: u32 = 3;
const ROW_SYSTEM: u32 = 4;
const ROW_ACTIVE: u32 = 5;
const ROW_INACTIVE: u32 = 6;
const ROW_SELECTED: u32 = 7;
const ROW_GPU: u32 = 8;
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
/// Update Frequency radio items: MENU_FREQ + index into `UpdateFrequency::ALL`.
pub const MENU_FREQ: u32 = 20;
/// Send Signal items: MENU_SIGNAL + index into `model::SIGNALS`.
pub const MENU_SIGNAL: u32 = 40;
/// Filter items of the View menu: MENU_FILTER + index into `Filter::ALL`.
pub const MENU_FILTER: u32 = 60;
/// View > Columns items: MENU_COLUMN + index into `Tab::all_columns()` of the current tab.
pub const MENU_COLUMN: u32 = 100;

/// Height of one line of the report view (points) and how often a running job is looked at (milliseconds).
pub const REPORT_LINE_PT: f32 = 16.0;
const JOB_POLL_MS: f64 = 200.0;

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

/// Where a press started, so the release can act on the same thing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    None,
    Report,
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
    /// The process being inspected (its pid and the last reading), shown in place of the graph.
    pub inspector: Option<(u32, Inspection)>,
    pub frequency: UpdateFrequency,
    /// Which optional columns the user turned on or off (View > Columns).
    pub columns: ColumnSet,
    /// Processes shown relative to the moment Show Deltas was turned on for them, by pid.
    pub baselines: HashMap<u32, ProcSample>,
    /// The process whose family the Selected Processes filter shows.
    pub scope: Option<u32>,
    /// Running energy means by process name, for the 12 hr Average column.
    pub averages: EnergyAverages,
    /// The CPU graph shows one strip per core (View > CPU History) instead of the total.
    pub cpu_history: bool,
    /// The background task running now (a process sample or the diagnostics), if any.
    pub job: Option<Job>,
    /// The report shown in place of the table.
    pub report: Option<Report>,
    /// A save the app layer should ask the shell for; taken with `take_request`.
    pub request: Option<Request>,
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
            frequency: UpdateFrequency::VeryOften,
            columns: ColumnSet::default(),
            baselines: HashMap::new(),
            scope: None,
            averages: EnergyAverages::default(),
            cpu_history: false,
            job: None,
            report: None,
            request: None,
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
        self.history.record(&snap);
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
        // a running job is looked at often enough for its progress to move
        if self.job.is_some() {
            reading.min(t + JOB_POLL_MS)
        } else {
            reading
        }
    }

    // --- background jobs and the report ---

    pub fn job_running(&self) -> bool {
        self.job.is_some()
    }

    /// Process > Sample Process: looks at the selected process's threads for ten seconds.
    pub fn start_sample(&mut self) {
        if self.job.is_some() {
            return;
        }
        let Some(p) = self.selected_proc().cloned() else { return };
        let (pid, name) = (p.pid, p.name.clone());
        self.report = None;
        self.job = Some(Job::start(JobKind::Sample, &format!("Sampling {name}"), move |progress| {
            sample_proc::run(pid, &name, sample_proc::DURATION_SECS, sample_proc::INTERVAL_MS, progress)
        }));
        self.status.clear();
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
            if self.filter == Filter::Selected && !self.scope.map_or(false, |root| s.procs.iter().any(|p| p.pid == root)) {
                self.filter = Filter::All;
                self.scope = None;
                self.status = "The selected process has exited".into();
                self.sync_scans();
            }
        }
        let only = match (self.filter, self.scope, &self.snapshot) {
            (Filter::Selected, Some(root), Some(s)) => Some(model::descendants(&s.procs, root)),
            _ => None,
        };
        self.rows = match &self.snapshot {
            Some(s) => model::visible_within(&s.procs, self.filter, self.sampler.own_uid, &query, self.sort, only.as_ref()),
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
        self.scroll.jump_top();
        self.sync_scans();
        self.rebuild();
    }

    /// Tells the sampler which costly readings the window needs now: the GPU on the GPU tab and under the GPU filter,
    /// the sleep blockers on the Energy tab while its Preventing Sleep column is shown. A reading that has just been
    /// turned on is taken at once.
    fn sync_scans(&mut self) {
        let gpu = self.tab == Tab::Gpu || self.filter == Filter::Gpu;
        let sleep = self.tab == Tab::Energy && self.tab.columns(&self.columns).iter().any(|c| c.key == Some(SortKey::PreventingSleep));
        let turned_on = (gpu && !self.sampler.scan_gpu) || (sleep && !self.sampler.scan_sleep);
        self.sampler.scan_gpu = gpu;
        self.sampler.scan_sleep = sleep;
        if turned_on {
            self.last_sample = None;
        }
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
            self.baselines.insert(p.pid, p);
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

    /// Sends SIGTERM to the selected process (its own user's, checked here as well as in the toolbar).
    pub fn quit_selected(&mut self) {
        self.signal_selected(SIGTERM, "quit");
    }

    /// Sends SIGKILL to the selected process. Same ownership rule as Quit: a process that cannot be quit cannot be forced.
    pub fn force_quit_selected(&mut self) {
        self.signal_selected(SIGKILL, "force quit");
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
        self.selected_proc().map(|p| model::row_text(p, self.tab, &self.columns, self.baselines.get(&p.pid)))
    }

    fn signal_selected(&mut self, sig: i32, verb: &str) {
        let Some(p) = self.selected_proc().cloned() else { return };
        if !self.can_quit_selected() {
            self.status = format!("Only processes you own can be {verb}");
            return;
        }
        // SAFETY: kill(2) takes a pid and a signal number and touches no memory of ours.
        let rc = unsafe { kill(p.pid as i32, sig) };
        self.status = if rc == 0 { format!("Sent {verb} to {}", p.name) } else { format!("Could not {verb} {}", p.name) };
    }

    /// Whether a process row is selected (any process can be inspected; the reading says what is denied).
    pub fn has_selected(&self) -> bool {
        self.selected_proc().is_some()
    }

    /// Opens the inspector on the selected process, or closes it when it is open.
    pub fn toggle_inspector(&mut self) {
        if self.inspector.take().is_some() {
            return;
        }
        let Some(pid) = self.selected else { return };
        self.inspector = Some((pid, inspect::inspect(pid)));
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
            SidebarItem::row(ROW_SELECTED, Filter::Selected.label()).with_symbol("checkmark.circle"),
            SidebarItem::row(ROW_GPU, Filter::Gpu.label()).with_symbol("desktopcomputer"),
        ];
        let mut list = SourceList::new(items, 28.0, 26.0);
        list.set_selected(Some(match self.filter {
            Filter::All => ROW_ALL,
            Filter::Mine => ROW_MINE,
            Filter::Others => ROW_OTHER,
            Filter::System => ROW_SYSTEM,
            Filter::Active => ROW_ACTIVE,
            Filter::Inactive => ROW_INACTIVE,
            Filter::Selected => ROW_SELECTED,
            Filter::Gpu => ROW_GPU,
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
            ROW_SELECTED => Filter::Selected,
            ROW_GPU => Filter::Gpu,
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
        // Escape leaves the report
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

    /// A touch. Pixels in, points used inside. True when a frame is needed.
    pub fn touch(&mut self, ui: &Ui, kind: TouchKind, x: f32, y: f32, t: f64) -> bool {
        let k = ui.pt(1.0);
        let (x, y) = (x / k, y / k);
        match kind {
            TouchKind::Down => {
                let g = self.geo(ui);
                let seg = self.seg_rect(ui);
                self.press = if x < SIDE_PT {
                    Press::Sidebar
                } else if y < UNIFIED_BAR_PT {
                    self.toolbar_hit(ui, x, y).map_or(Press::None, Press::Toolbar)
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
                            self.select_filter(id);
                        }
                    }
                    Press::Report => {
                        if let Some(report) = &mut self.report {
                            report.scroll.up(y, t);
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
    fn a_running_job_wakes_the_window_sooner() {
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
        assert_eq!(m.next_wake(1100.0), 1300.0);
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
}
