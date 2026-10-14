//! The Activity Monitor window's state and behaviour: the tab, the filter, the sort, the search, the selection, the
//! sampling clock, the sheets (signal, inspector, sample report, diagnostics, CPU and GPU history) and the geometry
//! that drawing and pointer input share. Points for the kit (`ui.pt`), pixels only where the surface is drawn.
//!
//! Things only the shell can do (the save panel, the pasteboard, the window list, the Dock tile, Reveal in Files)
//! are queued as [`Effect`]s; `main.rs` runs them after each event, so this module stays free of the runtime and
//! testable.

use crate::diagnostics;
use crate::inspect::{self, Inspection, Section};
use crate::model::{self, Baseline, Column, DockIcon, Filter, History, Scope, Signal, Sort, SortKey, Tab, UpdateFrequency, SIGNALS};
use crate::sample::{ProcSample, Sampler, Snapshot};
use crate::sample_proc;
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::sheet::{self, ButtonRole, Hit, Sheet, SheetButton, SHEET_PAD_PT};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::{Node, Rect, Role, TextField};
use bifrost_app::kit::ui::{MAC_BODY_PT, SPACE_S};
use bifrost_app::kit::{Scroller, TextCache, Ui, Weight};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets::{CAPSULE_PT, TOOLBAR_GAP_PT};
use bifrost_app::TouchKind;
use std::collections::HashSet;
use std::path::Path;

/// Sidebar width (points) and the inset of the content from it and from the window edge.
pub const SIDE_PT: f32 = 180.0;
const INSET_PT: f32 = 16.0;
/// The tab control: its height and its widest size.
const SEG_H_PT: f32 = 24.0;
const SEG_MAX_PT: f32 = 520.0;
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
/// Height of a row in the signal list and of a line in the report and file lists (points).
pub const SIGNAL_ROW_PT: f32 = 20.0;
pub const LINE_PT: f32 = 14.0;
/// Height of the title line of a free-form sheet, which the content starts below (points).
const SHEET_TITLE_PT: f32 = 26.0;

/// Toolbar item ids (also the published control ids).
pub const B_QUIT: u32 = 1;
pub const B_SEARCH: u32 = 2;
pub const B_FORCE: u32 = 3;
pub const B_INSPECT: u32 = 4;
pub const B_SAMPLE: u32 = 5;
/// Sidebar row ids.
const ROW_ALL: u32 = 1;
const ROW_MINE: u32 = 2;
const ROW_OTHER: u32 = 3;
const ROW_SYSTEM: u32 = 4;
const ROW_ACTIVE: u32 = 5;
const ROW_INACTIVE: u32 = 6;
const ROW_WINDOWED: u32 = 7;
const ROW_GPU: u32 = 8;
const ROW_SELECTED: u32 = 9;
/// Menu item ids for the app's own menu.
pub const MENU_QUIT: u32 = 1;
pub const MENU_FIND: u32 = 2;
pub const MENU_FORCE: u32 = 3;
pub const MENU_INSPECT: u32 = 4;
pub const MENU_COPY: u32 = 5;
pub const MENU_SAMPLE: u32 = 6;
pub const MENU_SIGNAL_SHEET: u32 = 7;
pub const MENU_DIAGNOSTICS: u32 = 8;
pub const MENU_DELTAS: u32 = 9;
pub const MENU_CPU_HISTORY: u32 = 10;
pub const MENU_CPU_USAGE: u32 = 11;
pub const MENU_GPU_HISTORY: u32 = 12;
/// Update Frequency radio items: MENU_FREQ + index into `UpdateFrequency::ALL`.
pub const MENU_FREQ: u32 = 20;
/// Send Signal items: MENU_SIGNAL + index into `model::SIGNALS`.
pub const MENU_SIGNAL: u32 = 40;
/// Filter items of the View menu: MENU_FILTER + index into `Filter::ALL`.
pub const MENU_FILTER: u32 = 60;
/// Column items of the View menu that hide a default column: MENU_COLUMN + index into `model::OPTIONAL_COLUMNS`.
pub const MENU_COLUMN: u32 = 80;
/// Column items that add a column: MENU_EXTRA + index into `model::EXTRA_COLUMNS`.
pub const MENU_EXTRA: u32 = 100;
/// Dock Icon radio items: MENU_DOCK + index into `DockIcon::ALL`.
pub const MENU_DOCK: u32 = 120;

/// Sheet button ids.
pub const SB_DEFAULT: u32 = 100;
pub const SB_CANCEL: u32 = 101;
pub const SB_COPY: u32 = 102;
pub const SB_REVEAL: u32 = 103;
pub const SB_SAMPLE: u32 = 104;

/// Where a press started, so the release can act on the same thing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    None,
    Sidebar,
    Toolbar(u32),
    Tab(usize),
    Head(usize),
    Table,
}

/// The sheet over the window, if any. Only one is open at a time.
pub enum Overlay {
    /// Process > Send Signal to Process: the signals as a list, `choice` an index into `SIGNALS`.
    Signal { choice: usize },
    /// The inspector for `Monitor::inspector`.
    Inspector,
    /// A sample under way.
    Sampling(sample_proc::Job),
    /// A finished sample: the text, and the process it belongs to for the file name.
    Report { pid: u32, name: String, text: String },
    /// System diagnostics under way.
    Diagnostics(diagnostics::Job),
    CpuHistory,
    CpuUsage,
    GpuHistory,
}

/// Something only the shell can do, queued for `main.rs`.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Show the save panel and write `bytes` to the file chosen.
    SaveFile { name: String, bytes: Vec<u8> },
    CopyText(String),
    /// Show the file at this path in Files.
    Reveal(String),
    /// Ask the shell for the window list, to know which processes own a window.
    ListWindows,
    /// Set the Dock tile: the badge text and the progress bar.
    Dock { badge: String, progress: Option<f32> },
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
    /// The process being inspected (its pid and the last reading), shown in the inspector sheet.
    pub inspector: Option<(u32, Inspection)>,
    pub inspector_section: Section,
    pub frequency: UpdateFrequency,
    /// View > Columns: bit i set hides `model::OPTIONAL_COLUMNS[i]`.
    pub hidden_columns: i64,
    /// View > Columns: bit i set shows `model::EXTRA_COLUMNS[i]`.
    pub extra_columns: i64,
    /// View > Show Deltas for Process: the counters at the moment it was switched on.
    pub baseline: Option<Baseline>,
    /// The processes Selected Processes keeps.
    pub pinned: Vec<u32>,
    /// The processes with a window, from the shell's window list.
    pub windowed: HashSet<u32>,
    pub dock: DockIcon,
    pub overlay: Option<Overlay>,
    /// Scroll of the report and the file list inside a sheet.
    pub sheet_scroll: Scroller,
    /// The sheet button under a press, for its pressed look.
    pub sheet_press: Option<u32>,
    sheet_drag: bool,
    effects: Vec<Effect>,
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
            inspector_section: Section::Memory,
            frequency: UpdateFrequency::VeryOften,
            hidden_columns: 0,
            extra_columns: 0,
            baseline: None,
            pinned: Vec::new(),
            windowed: HashSet::new(),
            dock: DockIcon::Application,
            overlay: None,
            sheet_scroll: Scroller::new(),
            sheet_press: None,
            sheet_drag: false,
            effects: Vec::new(),
        }
    }

    pub fn resize(&mut self, w: f32, h: f32) {
        self.w = w;
        self.h = h;
    }

    /// The effects queued since the last call, for `main.rs` to carry out.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    /// The sheet's busy work (a sample, the diagnostics) wants frames while it runs.
    fn overlay_busy(&self) -> bool {
        matches!(self.overlay, Some(Overlay::Sampling(_)) | Some(Overlay::Diagnostics(_)))
    }

    /// Reads the machine when the update interval has passed and moves a running sample or diagnostics along. True
    /// when the window needs a new frame.
    pub fn tick(&mut self, t: f64) -> bool {
        let mut changed = self.scroll.tick(t) | self.sheet_scroll.tick(t);
        changed |= self.poll_jobs();
        let due = self.last_sample.map_or(true, |last| t - last >= self.frequency.millis());
        if !due {
            return changed || self.overlay_busy();
        }
        self.last_sample = Some(t);
        self.sync_sampler();
        let snap = self.sampler.sample(t);
        self.history.record(&snap);
        if self.dock != DockIcon::Application {
            let (badge, progress) = self.dock.tile(&snap);
            self.effects.push(Effect::Dock { badge, progress });
        }
        self.snapshot = Some(snap);
        if self.filter == Filter::Windowed {
            self.effects.push(Effect::ListWindows);
        }
        self.rebuild();
        self.refresh_inspector();
        true
    }

    /// Moves a running sample or diagnostics to its next stage. True when something changed.
    fn poll_jobs(&mut self) -> bool {
        let mut next = None;
        match &mut self.overlay {
            Some(Overlay::Sampling(job)) => {
                if job.poll() {
                    let text = job.report.clone().unwrap_or_default();
                    next = Some(Overlay::Report { pid: job.pid, name: job.name.clone(), text });
                }
            }
            Some(Overlay::Diagnostics(job)) => {
                if job.poll() {
                    match job.result.take() {
                        Some(Ok(bytes)) => {
                            self.effects.push(Effect::SaveFile { name: diagnostics::FILE_NAME.into(), bytes });
                            self.status = "System diagnostics are ready to save".into();
                        }
                        Some(Err(e)) => self.status = format!("System diagnostics failed: {e}"),
                        None => {}
                    }
                    self.overlay = None;
                    return true;
                }
            }
            _ => {}
        }
        match next {
            Some(report) => {
                self.sheet_scroll.jump_top();
                self.overlay = Some(report);
                true
            }
            None => false,
        }
    }

    /// Turns on the readings that cost something only while a view needs them.
    fn sync_sampler(&mut self) {
        let gpu_column = self.extra_columns & (1 << extra_index(SortKey::Gpu)) != 0;
        self.sampler.gpu_enabled = self.tab == Tab::Gpu || self.filter == Filter::Gpu || gpu_column || matches!(self.overlay, Some(Overlay::GpuHistory));
        let sleep_column = self.extra_columns & (1 << extra_index(SortKey::PreventsSleep)) != 0;
        self.sampler.energy_enabled = self.tab == Tab::Energy || sleep_column;
    }

    /// Re-reads the inspected process. The inspector closes when the process has exited.
    fn refresh_inspector(&mut self) {
        let Some((pid, _)) = self.inspector else { return };
        let data = inspect::inspect(pid);
        if data.command.is_empty() && data.exe.is_empty() {
            self.inspector = None;
            if matches!(self.overlay, Some(Overlay::Inspector)) {
                self.overlay = None;
            }
            self.status = "The inspected process has exited".into();
        } else {
            self.inspector = Some((pid, data));
        }
    }

    /// The time of the next reading, so the app wakes for it (and sooner while a job shows its progress).
    pub fn next_wake(&self, t: f64) -> f64 {
        let due = self.last_sample.map_or(t, |last| last + self.frequency.millis()).max(t);
        if self.overlay_busy() {
            due.min(t + 100.0)
        } else {
            due
        }
    }

    /// Recomputes the table rows from the last reading and the filter, search and sort.
    pub fn rebuild(&mut self) {
        let query = self.query.to_string().to_lowercase();
        self.rows = match &self.snapshot {
            Some(s) => {
                let scope = Scope {
                    filter: self.filter,
                    own_uid: self.sampler.own_uid,
                    query: &query,
                    sort: self.sort,
                    pinned: &self.pinned,
                    windowed: &self.windowed,
                    baseline: self.baseline.as_ref(),
                };
                model::visible(&s.procs, &scope)
            }
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
        self.sync_sampler();
        self.rebuild();
    }

    /// Chooses a filter. Selected Processes keeps the process selected now and needs one.
    pub fn set_filter(&mut self, filter: Filter) {
        if filter == Filter::Selected {
            match self.selected {
                Some(pid) => self.pinned = vec![pid],
                None if self.pinned.is_empty() => {
                    self.status = "Select a process first".into();
                    return;
                }
                None => {}
            }
        }
        self.filter = filter;
        self.scroll.jump_top();
        self.sync_sampler();
        self.rebuild();
    }

    /// The shell's window list: the processes that own a window.
    pub fn set_windowed(&mut self, pids: impl IntoIterator<Item = u32>) {
        self.windowed = pids.into_iter().collect();
        self.rebuild();
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

    /// Process > Send Signal to Process...: opens the sheet that lists the signals.
    pub fn open_signal_sheet(&mut self) {
        if !self.can_quit_selected() {
            if self.has_selected() {
                self.status = "Only processes you own can be sent a signal".into();
            }
            return;
        }
        // TERM is the signal people mean most often
        let choice = SIGNALS.iter().position(|s| s.number == SIGTERM).unwrap_or(0);
        self.overlay = Some(Overlay::Signal { choice });
    }

    /// Changes how often the machine is read; the next reading is due at once.
    pub fn set_frequency(&mut self, f: UpdateFrequency) {
        self.frequency = f;
    }

    /// View > Columns: shows or hides one default column.
    pub fn toggle_column(&mut self, index: usize) {
        if index < model::OPTIONAL_COLUMNS.len() {
            self.hidden_columns ^= 1 << index;
        }
    }

    /// Whether default column `index` is shown.
    pub fn column_shown(&self, index: usize) -> bool {
        self.hidden_columns & (1 << index) == 0
    }

    /// View > Columns: adds or removes one extra column.
    pub fn toggle_extra(&mut self, index: usize) {
        if index < model::EXTRA_COLUMNS.len() {
            self.extra_columns ^= 1 << index;
            self.sync_sampler();
        }
    }

    pub fn extra_shown(&self, index: usize) -> bool {
        self.extra_columns & (1 << index) != 0
    }

    /// View > Show Deltas for Process: the counter columns count from now, or back to the totals.
    pub fn toggle_deltas(&mut self) {
        self.baseline = match (&self.baseline, &self.snapshot) {
            (None, Some(s)) => Some(Baseline::capture(&s.procs)),
            _ => None,
        };
        self.rebuild();
    }

    /// View > Dock Icon: what the Dock tile shows. Going back to the application icon clears the tile.
    pub fn set_dock(&mut self, dock: DockIcon) {
        self.dock = dock;
        if dock == DockIcon::Application {
            self.effects.push(Effect::Dock { badge: String::new(), progress: None });
        } else if let Some(s) = &self.snapshot {
            let (badge, progress) = dock.tile(s);
            self.effects.push(Effect::Dock { badge, progress });
        }
    }

    /// The selected row as tab-separated text, for Edit > Copy.
    pub fn copy_text(&self) -> Option<String> {
        self.selected_proc().map(|p| model::row_text(p, self.tab))
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
            if matches!(self.overlay, Some(Overlay::Inspector)) {
                self.overlay = None;
            }
            return;
        }
        let Some(pid) = self.selected else { return };
        self.inspector = Some((pid, inspect::inspect(pid)));
        self.inspector_section = Section::Memory;
        self.sheet_scroll.jump_top();
        self.overlay = Some(Overlay::Inspector);
    }

    /// Process > Sample Process: samples the selected process for ten seconds and shows the report.
    pub fn start_sample(&mut self) {
        let target = match (&self.overlay, &self.inspector) {
            (Some(Overlay::Inspector), Some((pid, d))) => Some((*pid, name_of(d, *pid))),
            _ => self.selected_proc().map(|p| (p.pid, p.name.clone())),
        };
        let Some((pid, name)) = target else { return };
        self.overlay = Some(Overlay::Sampling(sample_proc::Job::start(Path::new("/proc"), pid, &name, sample_proc::Options::default())));
    }

    /// View > Run System Diagnostics.
    pub fn start_diagnostics(&mut self) {
        let scratch = std::env::temp_dir().join(format!("activity-monitor-{}", std::process::id()));
        self.overlay = Some(Overlay::Diagnostics(diagnostics::Job::start(scratch)));
    }

    /// Window > CPU Usage, CPU History or GPU History: opens the panel, or closes it when it is the open one.
    pub fn toggle_panel(&mut self, panel: Overlay) {
        let same = matches!(
            (&self.overlay, &panel),
            (Some(Overlay::CpuHistory), Overlay::CpuHistory) | (Some(Overlay::CpuUsage), Overlay::CpuUsage) | (Some(Overlay::GpuHistory), Overlay::GpuHistory)
        );
        self.overlay = if same { None } else { Some(panel) };
        self.sheet_scroll.jump_top();
        self.sync_sampler();
    }

    // --- sheets ---

    /// The window in points, the parent of a sheet.
    pub fn window_rect(&self, ui: &Ui) -> Rect {
        let k = ui.pt(1.0);
        Rect { x: 0.0, y: 0.0, w: self.w / k, h: self.h / k }
    }

    /// The sheet for the open overlay, sized for a window of `w` by `h` points.
    pub fn sheet_for(&self, w: f32, h: f32) -> Option<Sheet> {
        let overlay = self.overlay.as_ref()?;
        let big = ((w - 80.0).clamp(440.0, 720.0), (h - UNIFIED_BAR_PT - 40.0).clamp(240.0, 520.0));
        let done = |title: &str| SheetButton::new(SB_CANCEL, title, ButtonRole::Cancel);
        Some(match overlay {
            Overlay::Signal { .. } => {
                let height = SHEET_PAD_PT * 2.0 + SHEET_TITLE_PT + SIGNALS.len() as f32 * SIGNAL_ROW_PT + 12.0 + sheet::BUTTON_H_PT;
                Sheet::new("Send Signal to Process", (300.0, height), vec![SheetButton::new(SB_DEFAULT, "Send", ButtonRole::Default), done("Cancel")])
            }
            Overlay::Inspector => {
                let (pid, d) = self.inspector.as_ref()?;
                let title = format!("{} ({pid})", name_of(d, *pid));
                Sheet::new(
                    &title,
                    big,
                    vec![done("Done"), SheetButton::new(SB_SAMPLE, "Sample", ButtonRole::Normal), SheetButton::new(SB_COPY, "Copy", ButtonRole::Normal), SheetButton::new(SB_REVEAL, "Reveal in Files", ButtonRole::Normal)],
                )
            }
            Overlay::Sampling(job) => Sheet::new(&format!("Sampling {}", job.name), (340.0, 130.0), vec![done("Stop")]),
            Overlay::Report { pid, name, .. } => Sheet::new(
                &format!("Sample of {name} ({pid})"),
                big,
                vec![done("Done"), SheetButton::new(SB_DEFAULT, "Save...", ButtonRole::Default), SheetButton::new(SB_COPY, "Copy", ButtonRole::Normal)],
            ),
            Overlay::Diagnostics(_) => Sheet::new("Running System Diagnostics", (340.0, 130.0), vec![done("Cancel")]),
            Overlay::CpuHistory => Sheet::new("CPU History", big, vec![done("Done")]),
            Overlay::CpuUsage => {
                let cores = self.snapshot.as_ref().map_or(1, |s| s.cores.len().max(1)) as f32;
                let height = (SHEET_PAD_PT * 2.0 + SHEET_TITLE_PT + cores * 20.0 + 12.0 + sheet::BUTTON_H_PT).min(big.1);
                Sheet::new("CPU Usage", (340.0, height), vec![done("Done")])
            }
            Overlay::GpuHistory => Sheet::new("GPU History", (520.0_f32.min(big.0), 260.0), vec![done("Done")]),
        })
    }

    pub fn current_sheet(&self, ui: &Ui) -> Option<Sheet> {
        let p = self.window_rect(ui);
        self.sheet_for(p.w, p.h)
    }

    /// The area of a sheet between its title and its buttons, where the content is drawn and tapped.
    pub fn sheet_content(&self, ui: &Ui, sheet: &Sheet) -> Rect {
        let l = sheet.layout(self.window_rect(ui), UNIFIED_BAR_PT, 1.0);
        let y = l.panel.y + SHEET_PAD_PT + SHEET_TITLE_PT;
        let buttons_top = l.buttons.first().map(|(_, r)| r.y).unwrap_or(l.panel.y + l.panel.h - SHEET_PAD_PT);
        Rect { x: l.panel.x + SHEET_PAD_PT, y, w: l.panel.w - SHEET_PAD_PT * 2.0, h: (buttons_top - 12.0 - y).max(0.0) }
    }

    /// The segmented control at the top of the inspector's content.
    pub fn inspector_segments(content: Rect) -> Rect {
        Rect { x: content.x, y: content.y, w: content.w, h: SEG_H_PT }
    }

    /// The signal under a point of the signal sheet.
    fn signal_at(&self, ui: &Ui, sheet: &Sheet, x: f32, y: f32) -> Option<usize> {
        let c = self.sheet_content(ui, sheet);
        if x < c.x || x >= c.x + c.w || y < c.y {
            return None;
        }
        let i = ((y - c.y) / SIGNAL_ROW_PT) as usize;
        (i < SIGNALS.len()).then_some(i)
    }

    /// What a sheet button does.
    pub fn sheet_button(&mut self, id: u32) {
        let Some(overlay) = self.overlay.take() else { return };
        match (overlay, id) {
            (Overlay::Signal { choice }, SB_DEFAULT) => {
                self.send_signal(SIGNALS[choice.min(SIGNALS.len() - 1)]);
            }
            (Overlay::Signal { .. }, _) => {}
            (Overlay::Inspector, SB_CANCEL) => {
                self.inspector = None;
            }
            (Overlay::Inspector, SB_COPY) => {
                if let Some((pid, d)) = &self.inspector {
                    self.effects.push(Effect::CopyText(inspect::report(*pid, d)));
                }
                self.overlay = Some(Overlay::Inspector);
            }
            (Overlay::Inspector, SB_REVEAL) => {
                match self.inspector.as_ref().map(|(_, d)| d.exe.clone()).filter(|e| !e.is_empty()) {
                    Some(exe) => self.effects.push(Effect::Reveal(exe)),
                    None => self.status = "The executable of this process is not readable".into(),
                }
                self.overlay = Some(Overlay::Inspector);
            }
            (Overlay::Inspector, SB_SAMPLE) => {
                self.overlay = Some(Overlay::Inspector);
                self.start_sample();
            }
            (Overlay::Inspector, _) => self.overlay = Some(Overlay::Inspector),
            (Overlay::Sampling(job), SB_CANCEL) => {
                // stop early: the report covers what was taken
                job.cancel();
                self.overlay = Some(Overlay::Sampling(job));
            }
            (Overlay::Report { pid, name, text }, SB_DEFAULT) => {
                self.effects.push(Effect::SaveFile { name: sample_proc::file_name(&name, pid), bytes: text.clone().into_bytes() });
                self.overlay = Some(Overlay::Report { pid, name, text });
            }
            (Overlay::Report { pid, name, text }, SB_COPY) => {
                self.effects.push(Effect::CopyText(text.clone()));
                self.overlay = Some(Overlay::Report { pid, name, text });
            }
            (Overlay::Report { .. }, SB_CANCEL) => {}
            (Overlay::Diagnostics(_), SB_CANCEL) => {
                // the job finishes on its own thread and its result is dropped
            }
            (other @ (Overlay::CpuHistory | Overlay::CpuUsage | Overlay::GpuHistory), id) if id != SB_CANCEL => self.overlay = Some(other),
            (_, _) => {}
        }
        self.sync_sampler();
    }

    /// A tap on the content of a sheet: a signal row or an inspector segment.
    fn sheet_tap(&mut self, ui: &Ui, sheet: &Sheet, x: f32, y: f32) {
        let signals = matches!(self.overlay, Some(Overlay::Signal { .. }));
        let inspector = matches!(self.overlay, Some(Overlay::Inspector));
        if signals {
            if let Some(i) = self.signal_at(ui, sheet, x, y) {
                self.overlay = Some(Overlay::Signal { choice: i });
            }
        } else if inspector {
            let seg = Monitor::inspector_segments(self.sheet_content(ui, sheet));
            if let Some(i) = sheet::segment_hit(seg, Section::ALL.len(), x, y) {
                self.inspector_section = Section::from_index(i);
                self.sheet_scroll.jump_top();
            }
        }
    }

    /// A key while a sheet is open: Return answers the default button, Escape the cancel button. Every key is consumed.
    pub fn sheet_key(&mut self, ui_size: (f32, f32), code: u32) -> bool {
        let Some(sheet) = self.sheet_for(ui_size.0, ui_size.1) else { return false };
        let answer = match code {
            13 => sheet.default_button(),
            1 | 27 => sheet.cancel_button(),
            _ => None,
        };
        if let Some(id) = answer {
            self.sheet_button(id);
        }
        true
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
        let cols = model::visible_columns(self.tab, self.hidden_columns, self.extra_columns);
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
            SidebarItem::row(ROW_WINDOWED, Filter::Windowed.label()).with_symbol("macwindow"),
            SidebarItem::row(ROW_GPU, Filter::Gpu.label()).with_symbol("cpu"),
            SidebarItem::row(ROW_SELECTED, Filter::Selected.label()).with_symbol("checkmark.circle"),
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
            Filter::Gpu => ROW_GPU,
            Filter::Selected => ROW_SELECTED,
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
            ROW_GPU => Filter::Gpu,
            ROW_SELECTED => Filter::Selected,
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
        Toolbar::new(vec![
            ToolbarItem::button(B_QUIT, "Quit Process").enabled(self.can_quit_selected()),
            ToolbarItem::button(B_FORCE, "Force Quit").enabled(self.can_quit_selected()),
            ToolbarItem::button(B_INSPECT, "Inspect").enabled(self.has_selected()),
            ToolbarItem::button(B_SAMPLE, "Sample").enabled(self.has_selected()),
            ToolbarItem::flexible_space(),
            ToolbarItem::custom(B_SEARCH, "Search", SEARCH_PT),
        ])
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
            B_SAMPLE => self.start_sample(),
            B_SEARCH => self.search_focused = true,
            _ => {}
        }
    }

    pub fn focus_search(&mut self) {
        self.search_focused = true;
    }

    /// Typed text goes to the search field while it has focus. True when a frame is needed.
    pub fn text(&mut self, text: &str) -> bool {
        if self.overlay.is_some() || !self.search_focused {
            return false;
        }
        self.query.push_str(text);
        self.rebuild();
        true
    }

    /// Editing keys (shell key codes: 8 backspace, 1 escape). True when a frame is needed.
    pub fn key(&mut self, ui: &Ui, code: u32) -> bool {
        if self.overlay.is_some() {
            let p = self.window_rect(ui);
            return self.sheet_key((p.w, p.h), code);
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

    /// A touch while a sheet is open: it blocks the window under it.
    fn sheet_touch(&mut self, ui: &Ui, sheet: &Sheet, kind: TouchKind, x: f32, y: f32, t: f64) {
        let parent = self.window_rect(ui);
        match kind {
            TouchKind::Down => match sheet.hit(parent, UNIFIED_BAR_PT, 1.0, x, y) {
                Hit::Button(id) => self.sheet_press = Some(id),
                Hit::Panel => {
                    self.sheet_scroll.down(y, t);
                    self.sheet_drag = true;
                }
                Hit::Blocked | Hit::Outside => {}
            },
            TouchKind::Move => {
                if self.sheet_drag {
                    self.sheet_scroll.moved(y, t);
                }
            }
            TouchKind::Up | TouchKind::Cancel => {
                let press = self.sheet_press.take();
                let dragging = std::mem::replace(&mut self.sheet_drag, false);
                if let Some(id) = press {
                    if sheet.hit(parent, UNIFIED_BAR_PT, 1.0, x, y) == Hit::Button(id) {
                        self.sheet_button(id);
                    }
                } else if dragging && self.sheet_scroll.up(y, t) {
                    self.sheet_tap(ui, sheet, x, y);
                }
            }
        }
    }

    /// A touch. Pixels in, points used inside. True when a frame is needed.
    pub fn touch(&mut self, ui: &Ui, kind: TouchKind, x: f32, y: f32, t: f64) -> bool {
        let k = ui.pt(1.0);
        let (x, y) = (x / k, y / k);
        if let Some(sheet) = self.current_sheet(ui) {
            self.sheet_touch(ui, &sheet, kind, x, y, t);
            return true;
        }
        match kind {
            TouchKind::Down => {
                let g = self.geo(ui);
                let seg = self.seg_rect(ui);
                self.press = if x < SIDE_PT {
                    Press::Sidebar
                } else if y < UNIFIED_BAR_PT {
                    self.toolbar_hit(ui, x, y).map_or(Press::None, Press::Toolbar)
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
                            self.select_filter(id);
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
                _ => nodes.push(Node { id: id as u64, role: Role::Button, label: item.label.clone(), value: String::new(), bounds: r, enabled: item.enabled && self.overlay.is_none() }),
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
        let deltas = if self.baseline.is_some() { "   Showing deltas" } else { "" };
        format!("{procs}   {cpu}   {mem}{deltas}")
    }
}

/// The index of an extra column in `model::EXTRA_COLUMNS`.
fn extra_index(key: SortKey) -> usize {
    model::EXTRA_COLUMNS.iter().position(|c| c.0 == key).unwrap_or(0)
}

/// A process's name for a title: the first word of its command line, as its base file name, or the pid.
fn name_of(d: &Inspection, pid: u32) -> String {
    let exe = if d.exe.is_empty() { d.command.split_whitespace().next().unwrap_or("") } else { d.exe.as_str() };
    match exe.rsplit('/').next() {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => format!("Process {pid}"),
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

    fn me() -> ProcSample {
        ProcSample { pid: std::process::id(), name: "self".into(), user: "me".into(), uid: 0, threads: 1, ..ProcSample::default() }
    }

    fn with_rows(m: &mut Monitor, rows: Vec<ProcSample>) {
        m.rows = rows;
    }

    #[test]
    fn the_inspector_needs_a_selected_process() {
        let mut m = Monitor::new(800.0, 600.0);
        assert!(!m.has_selected());
        m.toggle_inspector();
        assert!(m.inspector.is_none() && m.overlay.is_none());
    }

    #[test]
    fn the_inspector_opens_on_the_selected_pid_and_closes_again() {
        let mut m = Monitor::new(800.0, 600.0);
        m.selected = Some(std::process::id());
        m.toggle_inspector();
        let (pid, data) = m.inspector.clone().expect("open");
        assert_eq!(pid, std::process::id());
        assert!(!data.exe.is_empty());
        assert!(matches!(m.overlay, Some(Overlay::Inspector)));
        m.toggle_inspector();
        assert!(m.inspector.is_none() && m.overlay.is_none());
    }

    #[test]
    fn the_inspector_sheet_is_titled_with_the_process_and_has_the_reveal_and_sample_buttons() {
        let mut m = Monitor::new(800.0, 600.0);
        m.selected = Some(std::process::id());
        m.toggle_inspector();
        let sheet = m.sheet_for(1000.0, 700.0).expect("sheet");
        assert!(sheet.title.contains(&format!("({})", std::process::id())));
        let ids: Vec<u32> = sheet.buttons.iter().map(|b| b.id).collect();
        assert_eq!(ids, vec![SB_CANCEL, SB_SAMPLE, SB_COPY, SB_REVEAL]);
        assert_eq!(sheet.cancel_button(), Some(SB_CANCEL));
    }

    #[test]
    fn inspector_copy_and_reveal_queue_effects_and_keep_the_sheet() {
        let mut m = Monitor::new(800.0, 600.0);
        m.selected = Some(std::process::id());
        m.toggle_inspector();
        m.sheet_button(SB_COPY);
        m.sheet_button(SB_REVEAL);
        let effects = m.take_effects();
        assert!(matches!(&effects[0], Effect::CopyText(t) if t.starts_with("Process ")));
        assert!(matches!(&effects[1], Effect::Reveal(p) if p.starts_with('/')));
        assert!(matches!(m.overlay, Some(Overlay::Inspector)));
        m.sheet_button(SB_CANCEL);
        assert!(m.overlay.is_none() && m.inspector.is_none());
    }

    #[test]
    fn the_signal_sheet_needs_a_signalable_selection_and_starts_on_term() {
        let mut m = Monitor::new(800.0, 600.0);
        m.open_signal_sheet();
        assert!(m.overlay.is_none());
        // a process of another user cannot be signalled
        m.sampler.own_uid = 1;
        let mut other = me();
        other.pid = 1;
        with_rows(&mut m, vec![other]);
        m.selected = Some(1);
        m.open_signal_sheet();
        assert!(m.overlay.is_none() && m.status.contains("Only processes you own"));
        // one of the user's own can
        let mut mine = me();
        mine.uid = 1;
        mine.pid = std::process::id() + 1;
        let pid = mine.pid;
        with_rows(&mut m, vec![mine]);
        m.selected = Some(pid);
        m.open_signal_sheet();
        let Some(Overlay::Signal { choice }) = m.overlay else { panic!("sheet") };
        assert_eq!(SIGNALS[choice].name, "TERM");
        let sheet = m.sheet_for(900.0, 700.0).unwrap();
        assert_eq!(sheet.default_button(), Some(SB_DEFAULT));
        // Cancel closes without sending
        m.sheet_button(SB_CANCEL);
        assert!(m.overlay.is_none());
        assert!(m.status.contains("Only processes you own") || !m.status.starts_with("Sent"));
    }

    #[test]
    fn escape_and_return_answer_the_sheet_and_every_key_is_consumed() {
        let mut m = Monitor::new(800.0, 600.0);
        m.selected = Some(std::process::id());
        m.toggle_inspector();
        assert!(m.sheet_key((900.0, 700.0), 65));
        assert!(m.overlay.is_some());
        assert!(m.sheet_key((900.0, 700.0), 27));
        assert!(m.overlay.is_none());
        assert!(!m.sheet_key((900.0, 700.0), 27));
    }

    #[test]
    fn a_sample_runs_to_a_report_that_can_be_saved_and_copied() {
        let mut m = Monitor::new(800.0, 600.0);
        m.rows = vec![ProcSample { pid: std::process::id(), name: "self".into(), ..ProcSample::default() }];
        m.selected = Some(std::process::id());
        m.start_sample();
        assert!(matches!(m.overlay, Some(Overlay::Sampling(_))));
        if let Some(Overlay::Sampling(job)) = &m.overlay {
            job.cancel();
        }
        let mut finished = false;
        for _ in 0..500 {
            m.poll_jobs();
            if matches!(m.overlay, Some(Overlay::Report { .. })) {
                finished = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(finished);
        m.sheet_button(SB_DEFAULT);
        m.sheet_button(SB_COPY);
        let effects = m.take_effects();
        assert!(matches!(&effects[0], Effect::SaveFile { name, bytes } if name.starts_with("Sample of self") && !bytes.is_empty()));
        assert!(matches!(&effects[1], Effect::CopyText(_)));
        m.sheet_button(SB_CANCEL);
        assert!(m.overlay.is_none());
    }

    #[test]
    fn panels_toggle_and_replace_each_other() {
        let mut m = Monitor::new(800.0, 600.0);
        m.toggle_panel(Overlay::CpuHistory);
        assert!(matches!(m.overlay, Some(Overlay::CpuHistory)));
        m.toggle_panel(Overlay::CpuHistory);
        assert!(m.overlay.is_none());
        m.toggle_panel(Overlay::CpuUsage);
        m.toggle_panel(Overlay::GpuHistory);
        assert!(matches!(m.overlay, Some(Overlay::GpuHistory)));
        assert!(m.sampler.gpu_enabled || !m.sampler.gpu_enabled);
        m.sheet_button(SB_CANCEL);
        assert!(m.overlay.is_none());
    }

    #[test]
    fn selected_processes_pins_the_selection_and_needs_one() {
        let mut m = Monitor::new(800.0, 600.0);
        m.set_filter(Filter::Selected);
        assert_eq!(m.filter, Filter::All);
        assert!(m.status.contains("Select a process"));
        m.selected = Some(5);
        m.set_filter(Filter::Selected);
        assert_eq!((m.filter, m.pinned.clone()), (Filter::Selected, vec![5]));
    }

    #[test]
    fn the_window_list_feeds_the_windowed_filter() {
        let mut m = Monitor::new(800.0, 600.0);
        m.set_windowed([4, 9]);
        assert!(m.windowed.contains(&9) && !m.windowed.contains(&5));
    }

    #[test]
    fn show_deltas_needs_a_reading_and_toggles_a_baseline() {
        let mut m = Monitor::new(800.0, 600.0);
        m.toggle_deltas();
        assert!(m.baseline.is_none());
        m.tick(1000.0);
        m.toggle_deltas();
        assert!(m.baseline.is_some());
        assert!(m.footer_text().ends_with("Showing deltas"));
        m.toggle_deltas();
        assert!(m.baseline.is_none());
    }

    #[test]
    fn extra_columns_toggle_and_switch_on_their_readings() {
        let mut m = Monitor::new(800.0, 600.0);
        let gpu = extra_index(SortKey::Gpu);
        assert!(!m.extra_shown(gpu));
        m.toggle_extra(gpu);
        assert!(m.extra_shown(gpu) && m.sampler.gpu_enabled);
        m.toggle_extra(gpu);
        assert!(!m.sampler.gpu_enabled);
        m.toggle_extra(99);
        assert_eq!(m.extra_columns, 0);
    }

    #[test]
    fn the_gpu_and_energy_tabs_switch_on_their_readings() {
        let mut m = Monitor::new(800.0, 600.0);
        m.set_tab(Tab::Gpu);
        assert!(m.sampler.gpu_enabled && !m.sampler.energy_enabled);
        m.set_tab(Tab::Energy);
        assert!(m.sampler.energy_enabled && !m.sampler.gpu_enabled);
        m.set_tab(Tab::Cpu);
        assert!(!m.sampler.energy_enabled);
    }

    #[test]
    fn the_dock_tile_follows_the_choice_and_clears_with_the_icon() {
        let mut m = Monitor::new(800.0, 600.0);
        m.tick(1000.0);
        m.take_effects();
        m.set_dock(DockIcon::Cpu);
        assert!(matches!(&m.take_effects()[0], Effect::Dock { badge, progress: Some(_) } if badge.ends_with('%')));
        m.tick(2000.0);
        assert!(m.take_effects().iter().any(|e| matches!(e, Effect::Dock { .. })));
        m.set_dock(DockIcon::Application);
        assert_eq!(m.take_effects(), vec![Effect::Dock { badge: String::new(), progress: None }]);
        m.tick(3000.0);
        assert!(m.take_effects().is_empty());
    }

    #[test]
    fn the_windowed_filter_asks_for_the_window_list_on_each_reading() {
        let mut m = Monitor::new(800.0, 600.0);
        m.filter = Filter::Windowed;
        m.tick(1000.0);
        assert!(m.take_effects().contains(&Effect::ListWindows));
    }

    #[test]
    fn copy_text_follows_the_selection_and_tab() {
        let mut m = Monitor::new(800.0, 600.0);
        assert_eq!(m.copy_text(), None);
        m.rows = vec![crate::sample::ProcSample { pid: 7, name: "sh".into(), user: "me".into(), uid: 1, cpu: 1.0, rss: 0, threads: 2, ..Default::default() }];
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
    fn toggling_a_column_hides_and_shows_it() {
        let mut m = Monitor::new(800.0, 600.0);
        assert!(m.column_shown(1));
        m.toggle_column(1);
        assert!(!m.column_shown(1));
        m.toggle_column(1);
        assert!(m.column_shown(1));
        m.toggle_column(9);
        assert_eq!(m.hidden_columns, 0);
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
}
