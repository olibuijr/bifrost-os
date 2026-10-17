//! The Console window's state and behaviour: the views, the streaming, the search and the table selection, and the
//! geometry that drawing and hit tests share. Points for the kit (`Rect`, `ui.pt`), pixels only where drawing.

use crate::activity;
use crate::columns::{self, ColumnSet};
use crate::cursor;
use crate::draw;
use crate::find;
use crate::kernel::{self, KernelReports};
use crate::spin::{self, SpinReports};
use crate::opened::OpenedLog;
use crate::rows::{Extra, Rows};
use crate::export;
use crate::diagnostics::{self, DailySource, HangSource};
use crate::model::{self, Buffer, Levels, LogEntry, Query, SavedSearch, MAX_ENTRIES};
use crate::signposts;
use crate::security::{self, Security};
use crate::source::{Coredumps, Follow, Journal, Loaded, LogFile, Sink, Source};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::{Node, Rect, Role, TextField};
use bifrost_app::kit::ui::{MAC_BODY_PT, MAC_SUBHEAD_PT, SPACE_S};
use bifrost_app::kit::{Scroller, TextCache, Ui, Weight};
use bifrost_app::layout::{traffic_lights_trailing, UNIFIED_BAR_PT};
use bifrost_app::widgets::{CAPSULE_PT, TOOLBAR_GAP_PT};
use bifrost_app::dispatch::DispatchQueue;
use bifrost_app::{KeyInput, TouchKind};
use bifrost_core::log_cursor::Position;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Sidebar width and its row heights (points). The rows start under the toolbar band, as in Notes.
pub const SIDE_PT: f32 = 190.0;
/// The toolbar's search field width (points).
const SEARCH_PT: f32 = 220.0;
const SIDE_ROW_PT: f32 = 28.0;
const SIDE_HEADER_PT: f32 = 26.0;
pub const SIDE_SCROLL: f32 = -(UNIFIED_BAR_PT + 4.0);
/// Table rows, the table header and the status footer (points).
pub const ROW_PT: f32 = 22.0;
/// Text inset in a table cell (points), which the expanded rows wrap inside.
pub const CELL_PAD_PT: f32 = 8.0;
pub const HEAD_PT: f32 = 22.0;
pub const FOOT_PT: f32 = 24.0;
/// The detail pane's width range, shown only when a row is selected and the window has room for it.
const PANE_MIN_PT: f32 = 260.0;
const PANE_MAX_PT: f32 = 360.0;
/// The table that stays beside the pane: its narrow columns (Type, Time, Process, Message) need this much.
const TABLE_MIN_PT: f32 = 380.0;
const PANE_ROOM_PT: f32 = PANE_MIN_PT + TABLE_MIN_PT;
/// Entries read when a view opens.
const LOAD: usize = 500;
/// Messages waiting for the window while the reader runs ahead of it (the rest are dropped).
const PENDING_MAX: usize = 20_000;

// Sidebar row ids: the live journal, Crash Reports, Hang Diagnostics, Daily Summary, Signposts and Security Events,
// then log file `i` as `FILE_ROW + i`, saved search `i` as `SAVED_ROW + i`.
const ROW_LIVE: u32 = 1;
const ROW_CRASH: u32 = 2;
const ROW_SIGNPOST: u32 = 3;
const ROW_SECURITY: u32 = 4;
const ROW_HANG: u32 = 5;
const ROW_DAILY: u32 = 6;
const ROW_KERNEL: u32 = 7;
const ROW_SPIN: u32 = 8;
const FILE_ROW: u32 = 100;
const SAVED_ROW: u32 = 200;
const OPENED_ROW: u32 = 300;
/// Entries read when an opened file or a captured archive is shown: all it holds.
const OPENED_LOAD: usize = MAX_ENTRIES;
/// Lines of a message an expanded row shows at most, and the height of each line after the first (points).
const EXPAND_LINES: usize = 12;
pub const EXPAND_LINE_PT: f32 = 15.0;

// Toolbar item ids (also the published control ids).
pub const B_STREAM: u32 = 1;
pub const B_CLEAR: u32 = 2;
pub const B_INFO: u32 = 3;
pub const B_DEBUG: u32 = 4;
pub const B_SEARCH: u32 = 5;

/// What the sidebar shows. `File` indexes `Console::files`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Live,
    Crashes,
    Hangs,
    Daily,
    Signposts,
    Security,
    Kernel,
    Spins,
    File(usize),
    /// An opened file or a captured archive: indexes `Console::opened`.
    Opened(usize),
}

pub use crate::columns::Col;

/// Where the table and the panes are, in points. `cols` holds each column's x and width.
pub struct Geo {
    pub w: f32,
    pub h: f32,
    pub table_x: f32,
    pub table_right: f32,
    pub table_top: f32,
    pub table_bottom: f32,
    /// The detail pane's x (points) when it is shown.
    pub pane_x: Option<f32>,
    pub cols: Vec<(Col, f32, f32)>,
}

/// A pointer press, held until its release decides what it was.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Press {
    None,
    Toolbar(u32),
    Sidebar,
    Table,
}

pub struct Console {
    pub w: f32,
    pub h: f32,
    pub files: Vec<PathBuf>,
    pub view: View,
    source: Box<dyn Source>,
    pub buffer: Buffer,
    /// Sequence numbers of the shown entries, oldest first.
    pub visible: Vec<u64>,
    pub query: Query,
    /// Searches kept in the sidebar, in the order they were saved. Choosing one shows the live journal with its text.
    pub saved: Vec<SavedSearch>,
    pub search: TextField,
    pub search_focused: bool,
    pub levels: Levels,
    /// Live view: the journal is followed (Start) or paused.
    pub streaming: bool,
    follow: Option<Follow>,
    pending: Arc<Mutex<Vec<LogEntry>>>,
    pub selected: Option<u64>,
    /// Messages marked with Log > Mark Message, by sequence. When any are marked, Save Messages writes them.
    pub marked: std::collections::BTreeSet<u64>,
    /// The selected entry's extra detail text (`coredumpctl info` for a crash), with the sequence it belongs to.
    pub detail: Option<(u64, String)>,
    /// The selected entry's position in the log's sequence, as text (`Position::encode`).
    pub position: Option<String>,
    /// Walk mode: only the messages after this position are shown (set by the Log menu from the selection).
    pub after: Option<Position>,
    /// The Signposts view's footer: the totals of the intervals it shows.
    pub signpost_line: String,
    pub scroll: Scroller,
    /// Keep the newest row in view as messages arrive (set when a live view opens, cleared by the user's touch).
    pub stick: bool,
    /// The last load or stream error, or a note, for the footer.
    pub status: String,
    /// The power service's line (thermal state, battery, Low Power Mode), shown after the count. Empty when unknown.
    pub power_note: String,
    pub labels: TextCache,
    press: Press,
    /// Which columns the table shows (View > Visible Columns).
    pub columns: ColumnSet,
    /// View > Show Sidebar and View > Show Details.
    pub show_sidebar: bool,
    pub show_details: bool,
    /// View > Hide Toolbar: the toolbar's items and search field are neither drawn nor reachable.
    pub show_toolbar: bool,
    /// Sequence numbers of the rows whose whole message is shown (View > Expand Message).
    expanded: BTreeSet<u64>,
    /// The text Find Next and Find Previous step to; empty means the search field's.
    pub find_text: String,
    /// A row to scroll into view at the next draw (set by Find Next and Find Previous).
    reveal: Option<u64>,
    /// Files opened and archives captured, shown under Opened Files in the sidebar.
    pub opened: Vec<OpenedLog>,
    /// The systemd unit invocation the table is narrowed to (View > Show Only This Activity), if any.
    pub activity: Option<String>,
}

impl Console {
    pub fn new(w: f32, h: f32, files: Vec<PathBuf>) -> Self {
        Console {
            w,
            h,
            files,
            view: View::Live,
            source: Box::new(Journal),
            buffer: Buffer::new(MAX_ENTRIES),
            visible: Vec::new(),
            query: Query::default(),
            saved: Vec::new(),
            search: TextField::new(),
            search_focused: false,
            levels: Levels { info: false, debug: false },
            streaming: true,
            follow: None,
            pending: Arc::new(Mutex::new(Vec::new())),
            selected: None,
            marked: std::collections::BTreeSet::new(),
            detail: None,
            position: None,
            after: None,
            signpost_line: String::new(),
            scroll: Scroller::new(),
            stick: false,
            status: String::new(),
            power_note: String::new(),
            labels: TextCache::default(),
            press: Press::None,
            columns: ColumnSet::default(),
            show_sidebar: true,
            show_details: true,
            show_toolbar: true,
            expanded: BTreeSet::new(),
            find_text: String::new(),
            reveal: None,
            opened: Vec::new(),
            activity: None,
        }
    }

    pub fn resize(&mut self, w: f32, h: f32) {
        self.w = w;
        self.h = h;
    }

    // --- views and the journal ---

    /// Shows `view`: reads its entries and, for the live journal while streaming, follows it.
    pub fn open_view(&mut self, view: View) {
        self.follow = None;
        if let Ok(mut p) = self.pending.lock() {
            p.clear();
        }
        self.source = match view {
            View::Live => Box::new(Journal),
            View::Crashes => Box::new(Coredumps),
            View::Hangs => Box::new(HangSource),
            View::Daily => Box::new(DailySource),
            View::Signposts => Box::new(Journal),
            View::Security => Box::new(Security),
            View::Kernel => Box::new(KernelReports::default()),
            View::Spins => Box::new(SpinReports::default()),
            View::File(i) => Box::new(LogFile { path: self.files.get(i).cloned().unwrap_or_default() }),
            View::Opened(i) => Box::new(Loaded { entries: self.opened.get(i).map(|o| o.entries.clone()).unwrap_or_default() }),
        };
        self.view = view;
        self.buffer.clear();
        self.visible.clear();
        self.selected = None;
        self.detail = None;
        self.position = None;
        self.after = None;
        self.expanded.clear();
        self.reveal = None;
        self.status.clear();
        self.scroll.jump_top();
        let limit = if matches!(view, View::Opened(_)) { OPENED_LOAD } else { LOAD };
        let (loaded, _timing) = signposts::timed(|| self.source.load(limit));
        match loaded {
            Ok(entries) => {
                for e in entries {
                    self.buffer.push(e);
                }
            }
            Err(err) => self.status = err,
        }
        match view {
            View::Security => self.status = security::NOTE.to_string(),
            View::Hangs => self.status = diagnostics::HANG_NOTE.to_string(),
            View::Daily => self.status = diagnostics::DAILY_NOTE.to_string(),
            View::Kernel if self.status.is_empty() => self.status = kernel::NOTE.to_string(),
            View::Spins if self.status.is_empty() => self.status = spin::NOTE.to_string(),
            _ => {}
        }
        self.rebuild();
        self.stick = self.is_live() && self.streaming;
        if self.is_live() && self.streaming {
            self.start_follow();
        }
    }

    pub fn is_live(&self) -> bool {
        self.view == View::Live
    }

    fn start_follow(&mut self) {
        let pending = self.pending.clone();
        let queued = Arc::new(AtomicBool::new(false));
        let sink: Sink = Box::new(move |e| {
            if let Ok(mut p) = pending.lock() {
                if p.len() < PENDING_MAX {
                    p.push(e);
                }
            }
            // one main-queue wake per batch: the main queue clears `queued` before `tick`, which drains `pending`
            if !queued.swap(true, Ordering::AcqRel) {
                let queued = queued.clone();
                DispatchQueue::main().async_(move || queued.store(false, Ordering::Release));
            }
        });
        match self.source.follow(sink) {
            Ok(Some(f)) => self.follow = Some(f),
            Ok(None) => {}
            Err(err) => {
                self.streaming = false;
                self.status = err;
            }
        }
    }

    /// Start or Pause. Only the live journal streams.
    pub fn toggle_streaming(&mut self) {
        if !self.is_live() {
            return;
        }
        if self.streaming {
            self.streaming = false;
            self.follow = None;
        } else {
            self.streaming = true;
            self.stick = true;
            self.start_follow();
        }
    }

    /// Moves the messages the reader queued into the buffer. True when the table changed.
    pub fn drain(&mut self) -> bool {
        let batch: Vec<LogEntry> = match self.pending.lock() {
            Ok(mut p) => std::mem::take(&mut *p),
            Err(_) => return false,
        };
        let mut changed = !batch.is_empty();
        for e in batch {
            let shown = self.levels.shows(e.level) && self.query.matches(&e) && activity::shows(self.activity.as_deref(), &e);
            let seq = self.buffer.push(e);
            if shown {
                self.visible.push(seq);
            }
        }
        if changed {
            if self.after.is_some() {
                self.rebuild();
            } else {
                self.trim_visible();
            }
        }
        if self.follow.as_ref().is_some_and(|f| f.finished()) {
            self.follow = None;
            self.streaming = false;
            self.status = "The journal stopped".into();
            changed = true;
        }
        changed
    }

    /// Drops the sequence numbers of entries the buffer has let go.
    fn trim_visible(&mut self) {
        match self.buffer.first_seq() {
            Some(first) => {
                let k = self.visible.partition_point(|s| *s < first);
                self.visible.drain(..k);
            }
            None => self.visible.clear(),
        }
    }

    /// Recomputes what is shown from the toggles and the search.
    pub fn rebuild(&mut self) {
        let levels = self.levels;
        let query = &self.query;
        let activity = self.activity.as_deref();
        let signposts_only = self.view == View::Signposts;
        let mut visible: Vec<u64> = self
            .buffer
            .iter()
            .filter(|(_, e)| levels.shows(e.level) && query.matches(e) && activity::shows(activity, e) && (!signposts_only || signposts::parse(&e.message).is_some()))
            .map(|(s, _)| s)
            .collect();
        if let Some(from) = self.after {
            let newer = cursor::after(&self.positions(), from);
            visible.retain(|s| newer.contains(s));
        }
        self.visible = visible;
        self.signpost_line = if signposts_only { signposts::summary_line(&signposts::summarize(self.visible.iter().filter_map(|s| self.buffer.get(*s)))) } else { String::new() };
    }

    /// Every buffered message with its position in the log's sequence (`cursor::positions`).
    fn positions(&self) -> Vec<(Position, u64)> {
        let all: Vec<(u64, &LogEntry)> = self.buffer.iter().collect();
        cursor::positions(&all)
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.visible.clear();
        self.selected = None;
        self.detail = None;
        self.position = None;
        self.after = None;
        self.expanded.clear();
        self.reveal = None;
        if let Ok(mut p) = self.pending.lock() {
            p.clear();
        }
    }

    // --- the sidebar ---

    pub fn sidebar(&self) -> SourceList {
        let mut items = vec![SidebarItem::header("Live"), SidebarItem::row(ROW_LIVE, "This Mac").with_symbol("desktopcomputer")];
        items.push(SidebarItem::header("Reports"));
        items.push(SidebarItem::row(ROW_CRASH, "Crash Reports").with_symbol("xmark.circle.fill"));
        items.push(SidebarItem::row(ROW_HANG, "Hang Diagnostics").with_symbol("exclamationmark.triangle"));
        items.push(SidebarItem::row(ROW_DAILY, "Daily Summary").with_symbol("calendar"));
        items.push(SidebarItem::row(ROW_SIGNPOST, "Signposts").with_symbol("clock"));
        items.push(SidebarItem::row(ROW_KERNEL, "Kernel Reports").with_symbol("flag"));
        items.push(SidebarItem::row(ROW_SPIN, "Spin Reports").with_symbol("arrow.triangle.2.circlepath"));
        items.push(SidebarItem::row(ROW_SECURITY, "Security Events").with_symbol("info.circle"));
        // the section appears once a search is saved, so the default sidebar is unchanged
        if !self.saved.is_empty() {
            items.push(SidebarItem::header("Saved Searches"));
            for (i, s) in self.saved.iter().enumerate() {
                items.push(SidebarItem::row(SAVED_ROW + i as u32, &s.text).with_symbol("magnifyingglass"));
            }
        }
        if !self.opened.is_empty() {
            items.push(SidebarItem::header("Opened Files"));
            for (i, o) in self.opened.iter().enumerate() {
                items.push(SidebarItem::row(OPENED_ROW + i as u32, &o.name).with_symbol("doc"));
            }
        }
        items.push(SidebarItem::header("Log Files"));
        for (i, f) in self.files.iter().enumerate() {
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            items.push(SidebarItem::row(FILE_ROW + i as u32, &name).with_symbol("doc"));
        }
        let mut list = SourceList::new(items, SIDE_ROW_PT, SIDE_HEADER_PT);
        list.set_selected(Some(match self.view {
            View::Live => ROW_LIVE,
            View::Crashes => ROW_CRASH,
            View::Hangs => ROW_HANG,
            View::Daily => ROW_DAILY,
            View::Signposts => ROW_SIGNPOST,
            View::Security => ROW_SECURITY,
            View::Kernel => ROW_KERNEL,
            View::Spins => ROW_SPIN,
            View::File(i) => FILE_ROW + i as u32,
            View::Opened(i) => OPENED_ROW + i as u32,
        }));
        list
    }

    fn sidebar_hit(&self, ui: &Ui, x: f32, y: f32) -> Option<u32> {
        if !self.show_sidebar || x >= SIDE_PT {
            return None;
        }
        let bounds = Rect { x: 0.0, y: 0.0, w: SIDE_PT, h: self.h / ui.pt(1.0) };
        self.sidebar().hit(bounds, SIDE_SCROLL, x, y)
    }

    fn select_row(&mut self, id: u32) {
        if id >= OPENED_ROW {
            let i = (id - OPENED_ROW) as usize;
            if i < self.opened.len() && self.view != View::Opened(i) {
                self.open_view(View::Opened(i));
            }
            return;
        }
        if id >= SAVED_ROW {
            self.open_saved((id - SAVED_ROW) as usize);
            return;
        }
        let view = match id {
            ROW_LIVE => View::Live,
            ROW_CRASH => View::Crashes,
            ROW_SIGNPOST => View::Signposts,
            ROW_SECURITY => View::Security,
            ROW_HANG => View::Hangs,
            ROW_DAILY => View::Daily,
            ROW_KERNEL => View::Kernel,
            ROW_SPIN => View::Spins,
            id if id >= FILE_ROW && ((id - FILE_ROW) as usize) < self.files.len() => View::File((id - FILE_ROW) as usize),
            _ => return,
        };
        if view != self.view {
            self.open_view(view);
        }
    }

    /// Chooses a saved search: shows the live journal with the saved text in the search field.
    pub fn open_saved(&mut self, i: usize) {
        let Some(text) = self.saved.get(i).map(|s| s.text.clone()) else { return };
        if !self.is_live() {
            self.open_view(View::Live);
        }
        self.search.clear();
        self.search.push_str(&text);
        self.search_focused = false;
        self.search_changed();
    }

    /// Whether the search field holds a search that is not saved yet (the Save Search item).
    pub fn can_save_search(&self) -> bool {
        let text = self.search.trim();
        !text.is_empty() && !self.saved.iter().any(|s| s.text == text)
    }

    /// Whether the search field holds a saved search (the Delete Saved Search item).
    pub fn can_delete_search(&self) -> bool {
        let text = self.search.trim();
        self.saved.iter().any(|s| s.text == text)
    }

    /// Log menu: Save Search. Keeps the search field's text as a sidebar row.
    fn keep_search(&mut self) {
        let text = self.search.to_string();
        model::save_search(&mut self.saved, &text);
    }

    /// Log menu: Delete Saved Search. Removes the sidebar row whose text is the search field's.
    fn drop_search(&mut self) {
        let text = self.search.to_string();
        model::remove_search(&mut self.saved, &text);
    }

    // --- geometry (points) ---

    fn w_pt(&self, ui: &Ui) -> f32 {
        self.w / ui.pt(1.0)
    }

    /// The sidebar's width in points: nothing while it is hidden.
    pub fn side_w(&self) -> f32 {
        if self.show_sidebar {
            SIDE_PT
        } else {
            0.0
        }
    }

    pub fn geo(&self, ui: &Ui) -> Geo {
        let w = self.w_pt(ui);
        let h = self.h / ui.pt(1.0);
        let table_x = self.side_w();
        let pane_w = (w * 0.34).clamp(PANE_MIN_PT, PANE_MAX_PT);
        let pane = self.selected.is_some() && self.show_details && w - table_x >= PANE_ROOM_PT;
        let table_right = if pane { w - pane_w } else { w };
        let table_w = (table_right - table_x).max(0.0);
        Geo {
            w,
            h,
            table_x,
            table_right,
            table_top: UNIFIED_BAR_PT + HEAD_PT,
            table_bottom: (h - FOOT_PT).max(UNIFIED_BAR_PT + HEAD_PT),
            pane_x: pane.then_some(table_right),
            cols: columns::layout(table_x, table_w, self.columns),
        }
    }

    /// The toolbar band over the table and the pane. With the sidebar hidden it starts past the traffic lights.
    pub fn band(&self, ui: &Ui) -> Rect {
        let w = self.w_pt(ui);
        let x = if self.show_sidebar { SIDE_PT + 12.0 } else { traffic_lights_trailing() + TOOLBAR_GAP_PT };
        Rect { x, y: (UNIFIED_BAR_PT - CAPSULE_PT) / 2.0, w: (w - x - 12.0).max(0.0), h: CAPSULE_PT }
    }

    fn bar(&self) -> Toolbar {
        let live = self.is_live();
        Toolbar::new(vec![
            if self.streaming && live {
                ToolbarItem::icon(B_STREAM, "Pause", "pause.fill").enabled(live)
            } else {
                ToolbarItem::icon(B_STREAM, "Start", "play.fill").enabled(live)
            },
            ToolbarItem::button(B_CLEAR, "Clear"),
            ToolbarItem::flexible_space(),
            ToolbarItem::button(B_INFO, "Info").selected(self.levels.info).in_group(1),
            ToolbarItem::button(B_DEBUG, "Debug").selected(self.levels.debug).in_group(1),
            // the search field is always open, as in the stock Console (NativeSearchField in the toolbar)
            ToolbarItem::custom(B_SEARCH, "Search", SEARCH_PT),
        ])
    }

    pub fn bar_layout(&self, ui: &Ui) -> (Toolbar, ToolbarLayout) {
        let bar = self.bar();
        let layout = bar.layout_with(self.band(ui), item_w(ui), CAPSULE_PT, TOOLBAR_GAP_PT);
        (bar, layout)
    }

    /// The enabled toolbar item under a point.
    fn toolbar_hit(&self, ui: &Ui, x: f32, y: f32) -> Option<u32> {
        if !self.show_toolbar || y >= UNIFIED_BAR_PT {
            return None;
        }
        let (bar, layout) = self.bar_layout(ui);
        let id = layout.hit(x, y)?;
        bar.items.iter().find(|i| i.id == id).filter(|i| i.enabled).map(|_| id)
    }

    /// The table row (sequence number) under a point, given the current scroll.
    pub(crate) fn row_at(&self, ui: &Ui, y: f32) -> Option<u64> {
        let g = self.geo(ui);
        if y < g.table_top || y > g.table_bottom {
            return None;
        }
        let index = self.table_rows(ui, &g).locate(y - g.table_top + self.scroll.offset());
        self.visible.get(index).copied()
    }

    /// The row heights: every row is `ROW_PT` tall and an expanded one is taller by the lines its message needs, so
    /// the extra lines are measured here against the Message column's width.
    pub fn table_rows(&self, ui: &Ui, g: &Geo) -> Rows {
        let message_w = g.cols.iter().find(|c| c.0 == Col::Message).map(|c| (c.2 - 2.0 * CELL_PAD_PT).max(0.0));
        let extras: Vec<Extra> = match message_w {
            Some(w) if !self.expanded.is_empty() => self
                .expanded
                .iter()
                .filter_map(|seq| {
                    let index = self.visible.binary_search(seq).ok()?;
                    let message = &self.buffer.get(*seq)?.message;
                    let lines = draw::wrap(ui, message, MAC_SUBHEAD_PT, w).len().clamp(1, EXPAND_LINES);
                    Some((index, (lines - 1) as f32 * EXPAND_LINE_PT))
                })
                .collect(),
            _ => Vec::new(),
        };
        Rows::new(ROW_PT, extras)
    }

    /// The largest scroll offset for the rows, and sets it on the scroller.
    pub fn fit_scroll(&mut self, ui: &Ui) -> f32 {
        let g = self.geo(ui);
        let content = self.table_rows(ui, &g).content_height(self.visible.len());
        let max = (content - (g.table_bottom - g.table_top)).max(0.0);
        self.scroll.set_max(max);
        max
    }

    /// Scrolls to the row Find Next or Find Previous chose, now that the geometry and the scroll range are known.
    pub fn apply_reveal(&mut self, ui: &Ui, max: f32) {
        let Some(seq) = self.reveal.take() else { return };
        let Ok(index) = self.visible.binary_search(&seq) else { return };
        let g = self.geo(ui);
        let to = self.table_rows(ui, &g).reveal(index, self.scroll.offset(), g.table_bottom - g.table_top).min(max);
        self.scroll.jump_to(to);
    }

    // --- selection and the detail text ---

    fn select(&mut self, seq: u64) {
        self.selected = Some(seq);
        self.detail = None;
        self.position = cursor::position_of(&self.positions(), seq).map(|p| p.encode());
        if let Some(e) = self.buffer.get(seq).cloned() {
            if let Some(text) = self.source.detail(&e) {
                self.detail = Some((seq, text));
            }
        }
    }

    // --- toolbar, search and keys ---

    fn activate(&mut self, id: u32) {
        match id {
            B_STREAM => self.toggle_streaming(),
            B_CLEAR => self.clear(),
            B_INFO => {
                self.levels.info = !self.levels.info;
                self.rebuild();
            }
            B_DEBUG => {
                self.levels.debug = !self.levels.debug;
                self.rebuild();
            }
            B_SEARCH => self.search_focused = true,
            _ => {}
        }
    }

    pub fn focus_search(&mut self) {
        self.search_focused = true;
    }

    fn search_changed(&mut self) {
        self.query = Query::parse(&self.search);
        self.rebuild();
    }

    /// Typed text goes to the search field while it has focus. True when a frame is needed.
    pub fn text(&mut self, text: &str) -> bool {
        if !self.search_focused {
            return false;
        }
        self.search.push_str(text);
        self.search_changed();
        true
    }

    /// Editing and navigation keys (shell key codes: 8 backspace, 1 escape). True when a frame is needed.
    pub fn key(&mut self, code: u32) -> bool {
        if !self.search_focused {
            return false;
        }
        match code {
            8 => {
                self.search.pop();
                self.search_changed();
            }
            1 => {
                if self.search.is_empty() {
                    self.search_focused = false;
                } else {
                    self.search.clear();
                    self.search_changed();
                }
            }
            _ => return false,
        }
        true
    }

    pub fn key_input(&mut self, input: KeyInput) -> bool {
        match input {
            KeyInput::Text(text) => self.text(&text),
            KeyInput::Code(code) => self.key(code),
        }
    }

    // --- pointer ---

    /// A touch. Pixels in, points used inside. True when a frame is needed.
    pub fn touch(&mut self, ui: &Ui, kind: TouchKind, x: f32, y: f32, t: f64) -> bool {
        let k = ui.pt(1.0);
        let (x, y) = (x / k, y / k);
        match kind {
            TouchKind::Down => {
                let g = self.geo(ui);
                self.press = if x < self.side_w() {
                    Press::Sidebar
                } else if y < UNIFIED_BAR_PT {
                    self.toolbar_hit(ui, x, y).map_or(Press::None, Press::Toolbar)
                } else if y >= g.table_top && y <= g.table_bottom && x >= g.table_x && x < g.table_right {
                    self.stick = false;
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
                            self.select_row(id);
                        }
                    }
                    Press::Table => {
                        // `up` reports a tap (no drag): that selects the row under the finger
                        if self.scroll.up(y, t) {
                            if let Some(seq) = self.row_at(ui, y) {
                                self.select(seq);
                            }
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
        if !self.show_toolbar {
            return Vec::new();
        }
        let (bar, layout) = self.bar_layout(ui);
        let mut nodes = Vec::new();
        for (id, r) in layout.visible {
            let Some(item) = bar.items.iter().find(|i| i.id == id) else { continue };
            match id {
                B_SEARCH => nodes.push(Node { id: id as u64, role: Role::TextField, label: "Search".into(), value: self.search.to_string(), bounds: r, enabled: true }),
                _ => nodes.push(Node { id: id as u64, role: Role::Button, label: item.label.clone(), value: String::new(), bounds: r, enabled: item.enabled }),
            }
        }
        nodes
    }

    /// The rows the table shows, for the drawing code.
    pub fn row_range(&self, rows: &Rows, g: &Geo) -> (usize, usize) {
        let offset = self.scroll.offset();
        let first = rows.locate(offset);
        let last = rows.locate(offset + (g.table_bottom - g.table_top)) + 2;
        (first.min(self.visible.len()), last.min(self.visible.len()))
    }

    /// Whether the scroller is still moving, so the window draws again.
    pub fn animating(&mut self, t: f64) -> bool {
        self.scroll.tick(t)
    }

    /// A menu item's action (`View`, `Edit`). True when a frame is needed.
    pub fn menu(&mut self, id: u32) -> bool {
        match id {
            MENU_STREAM => self.toggle_streaming(),
            MENU_INFO => self.activate(B_INFO),
            MENU_DEBUG => self.activate(B_DEBUG),
            MENU_CLEAR => self.clear(),
            MENU_FIND => self.focus_search(),
            MENU_AFTER => self.toggle_after(),
            MENU_SUBSYSTEM => self.show_subsystem(),
            MENU_SAVE_SEARCH => self.keep_search(),
            MENU_MARK => self.toggle_mark(),
            MENU_CLEAR_MARKS => self.clear_marks(),
            MENU_DELETE_SEARCH => self.drop_search(),
            MENU_RELOAD => self.reload(),
            MENU_SIDEBAR => self.show_sidebar = !self.show_sidebar,
            MENU_DETAILS => self.show_details = !self.show_details,
            MENU_TOOLBAR => self.toggle_toolbar(),
            MENU_ACTIVITY => self.toggle_activity(),
            MENU_EXPAND => self.toggle_expanded(),
            MENU_COLLAPSE_ALL => self.expanded.clear(),
            MENU_FIND_NEXT => self.find_step(true),
            MENU_FIND_PREVIOUS => self.find_step(false),
            MENU_USE_SELECTION => self.use_selection_for_find(),
            id if (MENU_COLUMN..MENU_COLUMN + Col::ALL.len() as u32).contains(&id) => {
                self.columns.toggle(Col::ALL[(id - MENU_COLUMN) as usize]);
            }
            id if (MENU_RECENT..MENU_RECENT + MAX_OPENED as u32).contains(&id) => {
                let i = (id - MENU_RECENT) as usize;
                // the menu lists the newest first
                if let Some(i) = self.opened.len().checked_sub(1 + i) {
                    self.select_row(OPENED_ROW + i as u32);
                }
            }
            _ => return false,
        }
        true
    }

    /// View > Reload: reads the current view again.
    pub fn reload(&mut self) {
        self.open_view(self.view);
    }

    /// View > Expand Message / Collapse Message: shows the selected message whole in the table, or in one line again.
    pub fn toggle_expanded(&mut self) {
        let Some(seq) = self.selected else { return };
        if !self.expanded.remove(&seq) {
            self.expanded.insert(seq);
        }
    }

    /// Whether the selected message is shown whole (the menu item then reads Collapse).
    pub fn selected_expanded(&self) -> bool {
        self.selected.is_some_and(|s| self.expanded.contains(&s))
    }

    /// Whether any message is shown whole (Collapse All Messages).
    pub fn any_expanded(&self) -> bool {
        !self.expanded.is_empty()
    }

    /// The query Find Next and Find Previous match with: the find text, else the search field's.
    fn find_query(&self) -> Query {
        if self.find_text.trim().is_empty() {
            Query::parse(&self.search)
        } else {
            Query::parse(&self.find_text)
        }
    }

    /// Edit > Find Next and Find Previous: selects the next or previous shown message that matches the find text, wrapping
    /// at the ends, and scrolls to it. The table keeps every row; only the selection moves.
    pub fn find_step(&mut self, forward: bool) {
        let query = self.find_query();
        let found = find::step(&self.visible, self.selected, forward, |seq| self.buffer.get(seq).is_some_and(|e| query.matches(e)));
        match found {
            Some(seq) => {
                self.select(seq);
                self.reveal = Some(seq);
                self.stick = false;
                self.status.clear();
            }
            None => self.status = "No message matches.".into(),
        }
    }

    /// Edit > Use Selection for Find: the selected message's first line becomes the find text.
    pub fn use_selection_for_find(&mut self) {
        let Some(text) = self.selected_entry().map(find::text_for).filter(|t| !t.is_empty()) else { return };
        self.status = format!("Find: {text}");
        self.find_text = text;
    }

    /// File > Open and Capture Archive: the log joins the Opened Files section and is shown. The oldest one goes when
    /// `MAX_OPENED` are held; its grant is returned for the caller to release.
    pub fn add_opened(&mut self, log: OpenedLog) -> Option<bifrost_app::proto::GrantId> {
        let dropped = if self.opened.len() >= MAX_OPENED { self.opened.remove(0).grant } else { None };
        self.opened.push(log);
        self.open_view(View::Opened(self.opened.len() - 1));
        dropped
    }

    /// The opened log in view, if the view is one.
    pub fn active_opened(&self) -> Option<&OpenedLog> {
        match self.view {
            View::Opened(i) => self.opened.get(i),
            _ => None,
        }
    }

    /// Forgets the opened log that holds `grant` (it was moved to the Trash). The live journal is shown if it was in view.
    pub fn forget_opened(&mut self, grant: bifrost_app::proto::GrantId) {
        let viewing = matches!(self.view, View::Opened(_));
        self.opened.retain(|o| o.grant != Some(grant));
        if viewing {
            self.open_view(View::Live);
        }
    }

    /// The text File > Save Messages writes: the selected message, or every message the window shows.
    pub fn messages_to_save(&self) -> String {
        if !self.marked.is_empty() {
            return export::text(self.buffer.iter().filter(|(seq, _)| self.marked.contains(seq)).map(|(_, e)| e));
        }
        match self.selected_entry() {
            Some(e) => export::text([e]),
            None => export::text(self.visible.iter().filter_map(|s| self.buffer.get(*s))),
        }
    }

    /// Log > Mark Message: adds the selected message to the marked set, or takes it out again. Marked messages are what
    /// Save Messages writes. Nothing happens with no message selected.
    pub fn toggle_mark(&mut self) {
        let Some(seq) = self.selected else { return };
        if !self.marked.remove(&seq) {
            self.marked.insert(seq);
        }
        self.status = match self.marked.len() {
            0 => "No messages marked.".into(),
            n => format!("{n} marked for Save Messages."),
        };
    }

    /// Log > Clear Marks: forgets every marked message.
    pub fn clear_marks(&mut self) {
        self.marked.clear();
    }

    /// Whether the selected message is marked (the Log menu then reads Unmark Message).
    pub fn selected_marked(&self) -> bool {
        self.selected.is_some_and(|s| self.marked.contains(&s))
    }

    /// Save Messages without a save panel (a shell that has none): writes to `EXPORT_DIR` under `home`. The saved path, or
    /// the error, goes to the status line.
    pub fn save_messages(&mut self, home: &Path, now_secs: i64) {
        let text = self.messages_to_save();
        if text.is_empty() {
            self.status = "No messages to save.".into();
            return;
        }
        self.status = match export::save(&home.join(export::EXPORT_DIR), &export::file_name(now_secs), &text) {
            Ok(path) => format!("Saved {}", path.display()),
            Err(e) => format!("Could not save: {e}"),
        };
    }

    /// View > Hide Toolbar and Show Toolbar. Hiding also gives up the search field's focus, which it no longer shows.
    pub fn toggle_toolbar(&mut self) {
        self.show_toolbar = !self.show_toolbar;
        if !self.show_toolbar {
            self.search_focused = false;
        }
    }

    /// View > Show Only This Activity: narrows the table to the selected message's invocation. Again shows every activity.
    pub fn toggle_activity(&mut self) {
        if self.activity.is_some() {
            self.activity = None;
        } else {
            self.activity = self.selected_entry().and_then(activity::filter_for);
        }
        self.rebuild();
    }

    /// Log menu: walk mode. Shows the messages after the selected one, or back to every message.
    pub fn toggle_after(&mut self) {
        if self.after.take().is_none() {
            self.after = self.selected.and_then(|seq| cursor::position_of(&self.positions(), seq));
        }
        self.rebuild();
    }

    /// Log menu: narrows the search to the selected message's subsystem, as a `subsystem:` token appended to any search.
    pub fn show_subsystem(&mut self) {
        let Some(subsystem) = self.selected_entry().map(|e| e.subsystem.clone()) else { return };
        if subsystem.is_empty() {
            return;
        }
        let token = if subsystem.contains(char::is_whitespace) { format!("subsystem:\"{subsystem}\"") } else { format!("subsystem:{subsystem}") };
        if !self.search.is_empty() {
            self.search.push_str(" ");
        }
        self.search.push_str(&token);
        self.search_changed();
    }

    /// The footer's count and streaming state, then the power line when the power service has answered.
    pub fn footer_text(&self) -> String {
        let base = self.footer_base();
        if self.power_note.is_empty() {
            base
        } else {
            format!("{base} · {}", self.power_note)
        }
    }

    fn footer_base(&self) -> String {
        let shown = self.visible.len();
        let total = self.buffer.len();
        let count = match (shown, total) {
            (0, 0) => "No messages".to_string(),
            (s, t) if s == t => format!("{} messages", group(t)),
            (s, t) => format!("{} of {} messages", group(s), group(t)),
        };
        if self.view == View::Signposts {
            if self.signpost_line.is_empty() { count } else { format!("{count} · {}", self.signpost_line) }
        } else if !self.is_live() {
            count
        } else if self.streaming {
            format!("{count} · Streaming")
        } else {
            format!("{count} · Paused")
        }
    }

    pub fn empty_message(&self) -> &'static str {
        match self.view {
            View::Live => "No Messages",
            View::Crashes => "No Crash Reports",
            View::Hangs => "No Hang Diagnostics",
            View::Daily => "No Crashes or Hangs",
            View::Signposts => "No Signposts",
            View::Security => "No Security Events",
            View::Kernel => "No Kernel Reports",
            View::Spins => "No Spin Reports",
            View::File(_) | View::Opened(_) => "No Log Entries",
        }
    }

    pub fn selected_entry(&self) -> Option<&LogEntry> {
        self.selected.and_then(|s| self.buffer.get(s))
    }

    pub fn selected_detail(&self) -> Option<&str> {
        match (&self.detail, self.selected) {
            (Some((seq, text)), Some(sel)) if *seq == sel => Some(text.as_str()),
            _ => None,
        }
    }

    /// Scrolls to the newest row while `stick` is set (live streaming).
    pub fn set_stick_bottom(&mut self, max: f32) {
        if self.stick {
            self.scroll.jump_to(max);
        }
    }
}

/// The toolbar's width for each item: glyph buttons are capsules, titled ones fit their label (as in Notes).
fn item_w(ui: &Ui) -> impl Fn(&ToolbarItem) -> f32 + '_ {
    move |it| {
        if it.symbol.is_some() {
            CAPSULE_PT
        } else {
            (ui.measure(&it.label, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0) + 2.0 * SPACE_S * 1.5).max(CAPSULE_PT)
        }
    }
}

/// "1,234" with a comma every three digits.
fn group(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub const MENU_STREAM: u32 = 1;
pub const MENU_INFO: u32 = 2;
pub const MENU_DEBUG: u32 = 3;
pub const MENU_CLEAR: u32 = 4;
pub const MENU_FIND: u32 = 5;
pub const MENU_AFTER: u32 = 6;
pub const MENU_SUBSYSTEM: u32 = 7;
/// Log menu: Copy Message. Handled by the app (it needs the shell's pasteboard service), not by `menu`.
pub const MENU_COPY: u32 = 8;
/// Log menu: Save Search and Delete Saved Search, for the search field's text.
pub const MENU_SAVE_SEARCH: u32 = 9;
pub const MENU_DELETE_SEARCH: u32 = 10;
/// Log menu: Save Messages. Handled by the app (it needs the home directory), not by `menu`.
pub const MENU_SAVE_MESSAGES: u32 = 11;
/// View menu: Reload, Show Sidebar, Show Details, Expand or Collapse Message, Collapse All Messages.
pub const MENU_RELOAD: u32 = 12;
pub const MENU_SIDEBAR: u32 = 13;
pub const MENU_DETAILS: u32 = 14;
pub const MENU_EXPAND: u32 = 15;
pub const MENU_COLLAPSE_ALL: u32 = 16;
/// View menu: Show Only This Activity, which again shows every activity when one is narrowed.
pub const MENU_ACTIVITY: u32 = 24;
/// View menu: Hide Toolbar or Show Toolbar.
pub const MENU_TOOLBAR: u32 = 25;
/// Edit menu: Find Next, Find Previous, Use Selection for Find.
pub const MENU_FIND_NEXT: u32 = 17;
pub const MENU_FIND_PREVIOUS: u32 = 18;
pub const MENU_USE_SELECTION: u32 = 19;
/// File menu: Open, Capture Archive, Reveal in Files, Move to Trash. Handled by the app (they need the shell's
/// document and workspace services), not by `menu`.
pub const MENU_OPEN: u32 = 20;
pub const MENU_CAPTURE: u32 = 21;
pub const MENU_REVEAL: u32 = 22;
pub const MENU_TRASH: u32 = 23;
/// View > Visible Columns: item `MENU_COLUMN + i` toggles `Col::ALL[i]`.
pub const MENU_COLUMN: u32 = 30;
/// File > Open Recent: item `MENU_RECENT + i` shows the i-th newest opened log.
pub const MENU_RECENT: u32 = 40;
/// Log > Mark Message (toggles the selected message) and Log > Clear Marks.
pub const MENU_MARK: u32 = 26;
pub const MENU_CLEAR_MARKS: u32 = 27;
/// Logs held under Opened Files; opening one more drops the oldest.
pub const MAX_OPENED: usize = 8;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_are_grouped() {
        assert_eq!(group(0), "0");
        assert_eq!(group(999), "999");
        assert_eq!(group(1_000), "1,000");
        assert_eq!(group(50_000), "50,000");
        assert_eq!(group(1_234_567), "1,234,567");
    }

}
