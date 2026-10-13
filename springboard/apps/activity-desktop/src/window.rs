//! The Activity Monitor window's state and behaviour: the tab, the filter, the sort, the search, the selection, the
//! sampling clock and the geometry that drawing and pointer input share. Points for the kit (`ui.pt`), pixels only
//! where the surface is drawn.

use crate::model::{self, Column, Filter, History, Sort, SortKey, Tab};
use crate::sample::{ProcSample, Sampler, Snapshot};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::sheet;
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::{Node, Rect, Role, TextField};
use bifrost_app::kit::ui::{MAC_BODY_PT, SPACE_S};
use bifrost_app::kit::{Scroller, TextCache, Ui, Weight};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets::{CAPSULE_PT, TOOLBAR_GAP_PT};
use bifrost_app::TouchKind;

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
/// Readings are taken once a second.
pub const SAMPLE_MS: f64 = 1000.0;

/// Toolbar item ids (also the published control ids).
pub const B_QUIT: u32 = 1;
pub const B_SEARCH: u32 = 2;
/// Sidebar row ids.
const ROW_ALL: u32 = 1;
const ROW_MINE: u32 = 2;
const ROW_OTHER: u32 = 3;
/// Menu item ids for the app's own menu.
pub const MENU_QUIT: u32 = 1;
pub const MENU_FIND: u32 = 2;

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
        }
    }

    pub fn resize(&mut self, w: f32, h: f32) {
        self.w = w;
        self.h = h;
    }

    /// Reads the machine when a second has passed. True when the window needs a new frame.
    pub fn tick(&mut self, t: f64) -> bool {
        let moving = self.scroll.tick(t);
        let due = self.last_sample.map_or(true, |last| t - last >= SAMPLE_MS);
        if !due {
            return moving;
        }
        self.last_sample = Some(t);
        let snap = self.sampler.sample(t);
        self.history.record(&snap);
        self.snapshot = Some(snap);
        self.rebuild();
        true
    }

    /// The time of the next reading, so the app wakes for it.
    pub fn next_wake(&self, t: f64) -> f64 {
        self.last_sample.map_or(t, |last| last + SAMPLE_MS).max(t)
    }

    /// Recomputes the table rows from the last reading and the filter, search and sort.
    pub fn rebuild(&mut self) {
        let query = self.query.to_string().to_lowercase();
        self.rows = match &self.snapshot {
            Some(s) => model::visible(&s.procs, self.filter, self.sampler.own_uid, &query, self.sort),
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
        self.rebuild();
    }

    pub fn set_filter(&mut self, filter: Filter) {
        self.filter = filter;
        self.scroll.jump_top();
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
        let Some(p) = self.selected_proc().cloned() else { return };
        if !self.can_quit_selected() {
            self.status = "Only processes you own can be quit".into();
            return;
        }
        // SAFETY: kill(2) takes a pid and a signal number and touches no memory of ours.
        let rc = unsafe { kill(p.pid as i32, SIGTERM) };
        self.status = if rc == 0 { format!("Sent quit to {}", p.name) } else { format!("Could not quit {}", p.name) };
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
        let cols = self.tab.columns();
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
            SidebarItem::row(ROW_ALL, "All Processes").with_symbol("list.bullet"),
            SidebarItem::row(ROW_MINE, "My Processes").with_symbol("person"),
            SidebarItem::row(ROW_OTHER, "Other Users").with_symbol("person.2"),
        ];
        let mut list = SourceList::new(items, 28.0, 26.0);
        list.set_selected(Some(match self.filter {
            Filter::All => ROW_ALL,
            Filter::Mine => ROW_MINE,
            Filter::Others => ROW_OTHER,
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
const SIGTERM: i32 = 15;
