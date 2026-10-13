//! The System Information window's state and geometry: the sidebar of categories, the toolbar with Refresh and the
//! search field, the item list and the detail card with their scrolls, the find bar, the report that is open, and the
//! "Preparing system profile" sheet while the facts are read. Layout is in points; the drawing and the hit tests read
//! the same rectangles, so a tap lands on what is drawn.

use crate::export::Format;
use crate::extras::Extras;
use crate::model::{self, Category, Content, Row, Section, Stored, SECTIONS};
use crate::recents::{self, Recent};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::search;
use bifrost_app::controls::sheet::{ButtonRole, Hit, Sheet, SheetButton};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::{Rect, TextField};
use bifrost_app::kit::{TextCache, Ui};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::proto::{DocErrorCode, GrantId};
use bifrost_app::widgets;
use bifrost_core::sysinfo::{Report, Roots, Summary};
use std::sync::mpsc::{self, Receiver, TryRecvError};

/// The sidebar width (points).
pub const SIDE_PT: f32 = 220.0;
pub const ROW_PT: f32 = 26.0;
pub const HEAD_PT: f32 = 24.0;
pub const PAD_PT: f32 = 20.0;
/// One detail row (points).
pub const LINE_PT: f32 = 28.0;
/// The key column of the detail card (points).
pub const KEY_PT: f32 = 260.0;
/// Where the detail card starts, below the title and its line (points, from the top of the content).
pub const CARD_TOP_PT: f32 = 82.0;
/// One arrow-key step of the detail card (points).
pub const STEP_PT: f32 = 40.0;
/// The item list shows this many lines before it scrolls.
pub const LIST_MAX_ROWS: usize = 8;
/// The gap between the item list and the detail card (points).
pub const PANE_GAP_PT: f32 = 12.0;
/// The toolbar's search field (points).
pub const SEARCH_W_PT: f32 = 200.0;

/// The toolbar's Refresh button and search field, and the loading sheet's Cancel button.
pub const B_REFRESH: u32 = 1;
pub const B_SEARCH: u32 = 2;
pub const B_CANCEL: u32 = 3;
/// Section header ids are `SECTION_BASE` plus the section's index; sidebar row ids are `ROW_BASE` plus the category's
/// index, so neither meets the toolbar ids.
pub const SECTION_BASE: u64 = 50;
pub const ROW_BASE: u64 = 100;

/// The File, Edit, View, Window and Help menu items, so `main.rs` and the tests name the same ids.
pub const MENU_SAVE: u32 = 1;
pub const MENU_REFRESH: u32 = 2;
pub const MENU_LESS: u32 = 3;
pub const MENU_COPY: u32 = 4;
pub const MENU_SELECT_ALL: u32 = 5;
pub const MENU_FIND: u32 = 6;
pub const MENU_FIND_NEXT: u32 = 7;
pub const MENU_FIND_PREVIOUS: u32 = 8;
pub const MENU_HELP: u32 = 9;
pub const MENU_OPEN: u32 = 10;
pub const MENU_CLEAR_RECENTS: u32 = 11;
pub const MENU_THIS_COMPUTER: u32 = 12;
pub const MENU_SPEAK: u32 = 13;
pub const MENU_NEXT_CATEGORY: u32 = 14;
pub const MENU_PREVIOUS_CATEGORY: u32 = 15;
pub const MENU_OVERVIEW: u32 = 16;
pub const MENU_SAVE_RTF: u32 = 17;
pub const MENU_SAVE_JSON: u32 = 18;
/// Open Recent entry `i` is `MENU_RECENT + i`.
pub const MENU_RECENT: u32 = 200;

/// The preference key that keeps the collapsed sidebar sections.
pub const COLLAPSED_KEY: &str = "sidebar.collapsed";

/// Which pane the arrow keys act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    List,
    Detail,
}

/// The rows picked in the detail card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    None,
    One(usize),
    All,
}

/// What a press or a tap landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Nothing,
    Toolbar(u32),
    /// The search field, and its clear button.
    Search,
    SearchClear,
    Row(Category),
    /// A section header of the sidebar (its disclosure).
    Section(Section),
    /// Empty space in the sidebar.
    SideArea,
    /// A line of the item list, by its index in the items shown.
    Item(usize),
    ListArea,
    /// A line of the detail card, by its index in the rows shown.
    Line(usize),
    DetailArea,
    Content,
    /// The Cancel button of the loading sheet.
    Cancel,
}

/// What the window shows for the selected category: its content after the find, and which item of a list is picked.
#[derive(Clone, Debug)]
pub struct View {
    pub content: Content,
    pub item: usize,
}

impl View {
    /// The rows of the detail card: the picked item's facts, or the card's own rows.
    pub fn detail(&self) -> &[Row] {
        self.content.detail_of(self.item)
    }
}

/// The two panes of the content area, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Panes {
    pub list: Option<Rect>,
    pub detail: Rect,
}

/// A report waiting for the file the save panel will give.
pub struct PendingExport {
    pub request: u32,
    /// The form the menu item asked for; the file name's extension overrides it.
    pub format: Format,
    pub contents: Vec<(Category, Content)>,
}

/// A read of the machine on a worker thread.
pub struct Loading {
    rx: Receiver<(Report, Extras)>,
    /// The clock of the last tick, for the progress bar.
    pub t: f64,
}

fn spawn_load() -> Receiver<(Report, Extras)> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let report = Report::collect(&Roots::system());
        let extras = Extras::collect();
        let _ = tx.send((report, extras)); // the window may have cancelled and gone on
    });
    rx
}

pub struct SysInfo {
    pub report: Report,
    pub selected: Category,
    /// The picked line of the item list.
    pub item: usize,
    /// How far the detail card, the item list and the sidebar are scrolled, in points.
    pub scroll: f32,
    pub list_scroll: f32,
    pub side_scroll: f32,
    pub labels: TextCache,
    /// The window size, in the units the layout uses.
    pub size: (f32, f32),
    pub loaded: bool,
    /// Whether the first read has been started.
    pub started: bool,
    pub load: Option<Loading>,
    /// The pointer's last y while dragging a pane, and whether the drag has moved (a moved press is not a tap).
    pub drag_y: Option<f32>,
    pub dragged: bool,
    /// The target a press started on; the release activates it only when it lands on the same one.
    pub press: Target,
    /// Facts beyond the shared report: host name, uptime, modules, startup items, language, hardware detail.
    pub extras: Extras,
    /// Show Less Information: only the Overview.
    pub mini: bool,
    /// The find field, while the find bar is open.
    pub find: Option<TextField>,
    pub selection: Selection,
    pub focus: Focus,
    /// The save panel request and the report waiting for its file.
    pub export: Option<PendingExport>,
    /// A report opened from a file, shown in place of the live facts.
    pub stored: Option<Stored>,
    /// The open panel and reopen requests waiting for their file, with the grant of a recent report.
    pub pending_open: Vec<(u32, Option<GrantId>)>,
    pub recents: Vec<Recent>,
    pub recents_loaded: bool,
    /// The sections folded in the sidebar, and whether that changed since it was saved.
    pub collapsed: Vec<Section>,
    pub prefs_dirty: bool,
    /// A line that replaces the category's blurb until the next action (a file that would not open).
    pub notice: Option<String>,
    /// Whether the keyboard was asked for, so the request is sent only when it changes.
    pub keyboard_wanted: bool,
}

impl SysInfo {
    pub fn new() -> SysInfo {
        SysInfo {
            report: Report::default(),
            selected: Category::Overview,
            item: 0,
            scroll: 0.0,
            list_scroll: 0.0,
            side_scroll: 0.0,
            labels: TextCache::default(),
            size: (1200.0, 780.0),
            loaded: false,
            started: false,
            load: None,
            drag_y: None,
            dragged: false,
            press: Target::Nothing,
            extras: Extras::default(),
            mini: false,
            find: None,
            selection: Selection::None,
            focus: Focus::Sidebar,
            export: None,
            stored: None,
            pending_open: Vec::new(),
            recents: Vec::new(),
            recents_loaded: false,
            collapsed: Vec::new(),
            prefs_dirty: false,
            notice: None,
            keyboard_wanted: false,
        }
    }

    // ------------------------------------------------------------ reading the machine

    pub fn is_loading(&self) -> bool {
        self.load.is_some()
    }

    /// Starts reading the machine on a worker thread. False when a read is already running.
    pub fn start_load(&mut self) -> bool {
        if self.load.is_some() {
            return false;
        }
        self.started = true;
        self.load = Some(Loading { rx: spawn_load(), t: 0.0 });
        true
    }

    /// Checks on the read at clock `t`. True when the window changed (the facts landed or the progress moved).
    pub fn poll_load(&mut self, t: f64) -> bool {
        let Some(load) = self.load.as_mut() else { return false };
        load.t = t;
        match load.rx.try_recv() {
            Ok((report, extras)) => {
                self.load = None;
                self.apply(report, extras);
                true
            }
            Err(TryRecvError::Empty) => true,
            Err(TryRecvError::Disconnected) => {
                self.load = None;
                self.loaded = true;
                self.notice = Some("The system profile could not be read. Choose View > Refresh Information to try again.".to_string());
                true
            }
        }
    }

    /// Cancel on the loading sheet. The result of the read is dropped when it arrives.
    pub fn cancel_load(&mut self) -> bool {
        if self.load.take().is_none() {
            return false;
        }
        if !self.loaded {
            self.notice = Some("Reading the system profile was cancelled. Choose View > Refresh Information to read it again.".to_string());
        }
        true
    }

    /// Takes a finished read. The scroll stays where it is, clamped to the new length.
    pub fn apply(&mut self, report: Report, extras: Extras) {
        self.report = report;
        self.extras = extras;
        self.loaded = true;
        self.clamp_scrolls();
    }

    /// Reads the machine again: a report that is open gives way to this computer's facts.
    pub fn refresh(&mut self) -> bool {
        if self.load.is_some() {
            return false;
        }
        self.stored = None;
        self.notice = None;
        self.start_load();
        true
    }

    // ------------------------------------------------------------ content

    /// The text typed into the find bar, or "".
    pub fn query(&self) -> &str {
        self.find.as_deref().unwrap_or("")
    }

    /// The content of a category before any find: from the open report, else from this computer.
    pub fn base_content(&self, category: Category) -> Content {
        match &self.stored {
            Some(s) => s.content(category),
            None => model::content(category, &self.report, &self.extras),
        }
    }

    /// What the window shows now: the selected category's content after the find.
    pub fn view(&self) -> View {
        let content = self.base_content(self.selected).filtered(self.query());
        let item = self.item.min(content.items.len().saturating_sub(1));
        View { content, item }
    }

    /// The summary rows of the selected category after the find (the card, or one line per item).
    pub fn rows(&self) -> Vec<Row> {
        self.view().content.summary_rows()
    }

    /// The categories the sidebar lists: all of them (those a saved report holds), or the Overview alone in Show Less
    /// Information.
    pub fn visible_categories(&self) -> Vec<Category> {
        if self.mini {
            return vec![Category::Overview];
        }
        model::ALL.iter().copied().filter(|c| self.stored.as_ref().map_or(true, |s| s.has(*c))).collect()
    }

    /// The categories the arrow keys visit: the visible ones outside folded sections, and the selected one.
    pub fn navigable_categories(&self) -> Vec<Category> {
        self.visible_categories().into_iter().filter(|c| *c == self.selected || !self.is_collapsed(c.section())).collect()
    }

    pub fn toggle_mini(&mut self) -> bool {
        self.mini = !self.mini;
        self.selection = Selection::None;
        if self.mini {
            self.selected = Category::Overview;
        }
        self.item = 0;
        self.scroll = 0.0;
        self.list_scroll = 0.0;
        self.side_scroll = 0.0;
        true
    }

    // ------------------------------------------------------------ find

    pub fn begin_find(&mut self) -> bool {
        if self.find.is_none() {
            self.find = Some(TextField::new());
        }
        true
    }

    pub fn end_find(&mut self) -> bool {
        let was = self.find.take().is_some();
        self.clamp_scrolls();
        was
    }

    fn after_find_change(&mut self) {
        self.selection = Selection::None;
        self.item = 0;
        self.scroll = 0.0;
        self.list_scroll = 0.0;
    }

    /// Types into the find bar. False when the bar is closed.
    pub fn find_type(&mut self, text: &str) -> bool {
        let Some(field) = &mut self.find else { return false };
        field.push_str(text);
        self.after_find_change();
        true
    }

    pub fn find_backspace(&mut self) -> bool {
        let Some(field) = &mut self.find else { return false };
        let had = !field.is_empty();
        field.pop();
        self.after_find_change();
        had
    }

    /// Find Next and Find Previous: moves to the next category with a match. False when there is none.
    pub fn find_step(&mut self, forward: bool) -> bool {
        let q = self.query().to_string();
        if q.trim().is_empty() || self.mini {
            return false;
        }
        let found = match &self.stored {
            Some(stored) => {
                let n = model::ALL.len();
                let from = self.selected.index();
                (1..=n)
                    .map(|step| if forward { (from + step) % n } else { (from + n - step % n) % n })
                    .filter_map(Category::from_index)
                    .find(|c| stored.has(*c) && stored.content(*c).matches(&q))
            }
            None => model::next_match(self.selected, &q, forward, &self.report, &self.extras),
        };
        match found {
            Some(c) => self.select(c),
            None => false,
        }
    }

    // ------------------------------------------------------------ selection and copy

    pub fn select_all(&mut self) -> bool {
        self.selection = Selection::All;
        true
    }

    /// Whether Copy has something to copy: rows picked in the card, or an item picked in a list that has the focus.
    pub fn can_copy(&self) -> bool {
        self.selection != Selection::None || self.focus == Focus::List
    }

    /// The text Copy puts on the clipboard: the picked rows as `key: value` lines; with the item list focused and no
    /// row picked, the picked item's line. None when there is nothing to copy.
    pub fn copy_text(&self) -> Option<String> {
        let view = self.view();
        let line = |(k, v): &Row| format!("{k}: {v}");
        let lines: Vec<String> = match self.selection {
            Selection::All => view.detail().iter().map(line).collect(),
            Selection::One(i) => view.detail().get(i).map(line).into_iter().collect(),
            Selection::None if self.focus == Focus::List && view.content.list => {
                view.content.items.get(view.item).map(|i| format!("{}: {}", i.title.trim_start(), i.summary)).into_iter().collect()
            }
            Selection::None => return None,
        };
        (!lines.is_empty()).then(|| lines.join("\n"))
    }

    pub fn is_selected(&self, index: usize) -> bool {
        match self.selection {
            Selection::None => false,
            Selection::All => true,
            Selection::One(i) => i == index,
        }
    }

    // ------------------------------------------------------------ the report that is open or saved

    /// What Save writes: the open report, else this computer's facts (the Overview alone in Show Less Information).
    pub fn export_contents(&self) -> Vec<(Category, Content)> {
        let visible = self.visible_categories();
        match &self.stored {
            Some(s) => s.contents.iter().filter(|(c, _)| visible.contains(c)).cloned().collect(),
            None => model::contents(&self.report, &self.extras, self.mini),
        }
    }

    /// The report as plain text, as Save writes it by default.
    pub fn report_text(&self) -> String {
        model::text_of(&self.export_contents())
    }

    /// Shows a report read from a file in place of the live facts.
    pub fn show_stored(&mut self, stored: Stored) {
        self.notice = None;
        self.mini = false;
        let first = model::ALL.iter().copied().find(|c| stored.has(*c));
        self.stored = Some(stored);
        if !self.stored.as_ref().is_some_and(|s| s.has(self.selected)) {
            if let Some(c) = first {
                self.selected = c;
            }
        }
        self.item = 0;
        self.selection = Selection::None;
        self.scroll = 0.0;
        self.list_scroll = 0.0;
        self.side_scroll = 0.0;
    }

    /// File > Show This Computer: the live facts again, without reading them anew.
    pub fn show_live(&mut self) -> bool {
        if self.stored.take().is_none() {
            return false;
        }
        self.notice = None;
        self.item = 0;
        self.selection = Selection::None;
        self.scroll = 0.0;
        self.list_scroll = 0.0;
        true
    }

    /// The serial number as it is spoken: "Serial number" and then every letter and digit on its own. None without one.
    pub fn serial_speech(&self) -> Option<String> {
        speech_for_serial(&Summary::from_report(&self.report).serial)
    }

    // ------------------------------------------------------------ navigation

    pub fn select(&mut self, category: Category) -> bool {
        if category == self.selected {
            return false;
        }
        self.selected = category;
        self.item = 0;
        self.selection = Selection::None;
        self.scroll = 0.0;
        self.list_scroll = 0.0;
        true
    }

    /// Moves the sidebar selection by `delta` rows among the categories the arrow keys visit. True when it moved.
    pub fn step_category(&mut self, delta: i32) -> bool {
        let list = self.navigable_categories();
        let Some(at) = list.iter().position(|c| *c == self.selected) else { return false };
        let next = (at as i32 + delta).clamp(0, list.len() as i32 - 1) as usize;
        let moved = self.select(list[next]);
        if moved {
            self.keep_selected_row_in_view();
        }
        moved
    }

    /// Moves the picked line of the item list by `delta`. True when it moved.
    pub fn step_item(&mut self, delta: i32) -> bool {
        let view = self.view();
        if !view.content.list {
            return false;
        }
        let last = view.content.items.len() as i32 - 1;
        let next = (view.item as i32 + delta).clamp(0, last.max(0)) as usize;
        if next == view.item {
            return false;
        }
        self.item = next;
        self.selection = Selection::None;
        self.scroll = 0.0;
        let panes = self.panes(&view);
        if let Some(list) = panes.list {
            self.list_scroll = scroll_into_view(self.list_scroll, next, list.h);
        }
        true
    }

    /// Tab: sidebar, then the item list (when the category has one), then the card, then the sidebar again.
    pub fn toggle_focus(&mut self) -> bool {
        let has_list = self.view().content.list;
        self.focus = match (self.focus, has_list) {
            (Focus::Sidebar, true) => Focus::List,
            (Focus::Sidebar, false) => Focus::Detail,
            (Focus::List, _) => Focus::Detail,
            (Focus::Detail, _) => Focus::Sidebar,
        };
        true
    }

    // ------------------------------------------------------------ sections

    pub fn is_collapsed(&self, section: Section) -> bool {
        self.collapsed.contains(&section)
    }

    /// Folds or unfolds a sidebar section. The choice is saved with the preferences.
    pub fn toggle_section(&mut self, section: Section) -> bool {
        match self.collapsed.iter().position(|s| *s == section) {
            Some(i) => {
                self.collapsed.remove(i);
            }
            None => self.collapsed.push(section),
        }
        self.prefs_dirty = true;
        self.side_scroll = self.side_scroll.clamp(0.0, self.max_side_scroll());
        true
    }

    pub fn collapsed_text(&self) -> String {
        format_collapsed(&self.collapsed)
    }

    pub fn set_collapsed_text(&mut self, text: &str) {
        self.collapsed = parse_collapsed(text);
    }

    // ------------------------------------------------------------ geometry

    pub fn toolbar(&self) -> Toolbar {
        Toolbar::new(vec![ToolbarItem::button(B_REFRESH, "Refresh").enabled(!self.is_loading()), ToolbarItem::flexible_space(), ToolbarItem::custom(B_SEARCH, "Search", SEARCH_W_PT)])
    }

    pub fn toolbar_layout(&self, ui: &Ui) -> ToolbarLayout {
        widgets::toolbar_layout(ui, self.bar_rect(), &self.toolbar())
    }

    /// The search field in the toolbar, if it fits.
    pub fn search_rect(&self, ui: &Ui) -> Option<Rect> {
        self.toolbar_layout(ui).rect_of(B_SEARCH).map(|r| Rect { x: r.x, y: r.y + 3.0, w: r.w, h: (r.h - 6.0).max(0.0) })
    }

    pub fn sidebar(&self) -> SourceList {
        let visible = self.visible_categories();
        let mut items = Vec::new();
        for section in SECTIONS {
            let cats: Vec<Category> = model::in_section(section).into_iter().filter(|c| visible.contains(c)).collect();
            if cats.is_empty() {
                continue;
            }
            items.push(SidebarItem::section(SECTION_BASE as u32 + section.index() as u32, section.title(), self.is_collapsed(section)));
            for category in cats {
                items.push(SidebarItem::row(ROW_BASE as u32 + category.index() as u32, category.title()).with_symbol(category.symbol()));
            }
        }
        let mut list = SourceList::new(items, ROW_PT, HEAD_PT);
        list.set_selected(Some(ROW_BASE as u32 + self.selected.index() as u32));
        list
    }

    pub fn bar_rect(&self) -> Rect {
        Rect { x: 0.0, y: 0.0, w: self.size.0, h: UNIFIED_BAR_PT }
    }

    pub fn window_rect(&self) -> Rect {
        Rect { x: 0.0, y: 0.0, w: self.size.0, h: self.size.1 }
    }

    pub fn sidebar_rect(&self) -> Rect {
        Rect { x: 0.0, y: UNIFIED_BAR_PT, w: SIDE_PT, h: (self.size.1 - UNIFIED_BAR_PT).max(0.0) }
    }

    /// The left edge of the detail content, and its width.
    pub fn content_x(&self) -> f32 {
        SIDE_PT + PAD_PT
    }

    pub fn content_w(&self) -> f32 {
        (self.size.0 - SIDE_PT - 2.0 * PAD_PT).max(0.0)
    }

    /// The top of the content cards, in points.
    pub fn card_top(&self) -> f32 {
        UNIFIED_BAR_PT + CARD_TOP_PT
    }

    /// The item list and the detail card for `view`. A card category has no list, and its card takes the whole area.
    pub fn panes(&self, view: &View) -> Panes {
        let (x, w) = (self.content_x(), self.content_w());
        let top = self.card_top();
        let bottom = self.size.1 - PAD_PT;
        let avail = (bottom - top).max(0.0);
        if !view.content.list {
            return Panes { list: None, detail: Rect { x, y: top, w, h: avail } };
        }
        let wanted = view.content.items.len().min(LIST_MAX_ROWS) as f32 * LINE_PT + 12.0;
        let list_h = wanted.min((avail * 0.45).max(LINE_PT + 12.0)).min(avail);
        let detail_y = top + list_h + PANE_GAP_PT;
        Panes { list: Some(Rect { x, y: top, w, h: list_h }), detail: Rect { x, y: detail_y, w, h: (bottom - detail_y).max(0.0) } }
    }

    /// The index of the detail line under the window point `y` (points), if any.
    pub fn line_at(&self, view: &View, y: f32) -> Option<usize> {
        row_at(&self.panes(view).detail, self.scroll, view.detail().len(), y)
    }

    /// The index of the list line under the window point `y` (points), if any.
    pub fn item_at(&self, view: &View, y: f32) -> Option<usize> {
        let list = self.panes(view).list?;
        row_at(&list, self.list_scroll, view.content.items.len(), y)
    }

    /// How far the detail card can scroll: the rows beyond what the pane shows, in points.
    pub fn max_scroll(&self) -> f32 {
        let view = self.view();
        max_scroll_of(view.detail().len(), self.panes(&view).detail.h)
    }

    pub fn max_list_scroll(&self) -> f32 {
        let view = self.view();
        match self.panes(&view).list {
            Some(list) => max_scroll_of(view.content.items.len(), list.h),
            None => 0.0,
        }
    }

    pub fn max_side_scroll(&self) -> f32 {
        (self.sidebar().content_height() - self.sidebar_rect().h).max(0.0)
    }

    fn clamp_scrolls(&mut self) {
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
        self.list_scroll = self.list_scroll.clamp(0.0, self.max_list_scroll());
        self.side_scroll = self.side_scroll.clamp(0.0, self.max_side_scroll());
    }

    fn keep_selected_row_in_view(&mut self) {
        let height = self.sidebar_rect().h;
        self.side_scroll = self.sidebar().scroll_to_selected(height, self.side_scroll);
    }

    /// Moves the detail card by `delta` points (positive scrolls down), within the card's length.
    pub fn scroll_by(&mut self, delta: f32) -> bool {
        let next = (self.scroll + delta).clamp(0.0, self.max_scroll());
        let moved = (next - self.scroll).abs() >= f32::EPSILON;
        self.scroll = next;
        moved
    }

    pub fn scroll_list_by(&mut self, delta: f32) -> bool {
        let next = (self.list_scroll + delta).clamp(0.0, self.max_list_scroll());
        let moved = (next - self.list_scroll).abs() >= f32::EPSILON;
        self.list_scroll = next;
        moved
    }

    pub fn scroll_side_by(&mut self, delta: f32) -> bool {
        let next = (self.side_scroll + delta).clamp(0.0, self.max_side_scroll());
        let moved = (next - self.side_scroll).abs() >= f32::EPSILON;
        self.side_scroll = next;
        moved
    }

    /// A drag of `delta` points (positive scrolls down) in the pane the press started on.
    pub fn drag_scroll(&mut self, press: Target, delta: f32) -> bool {
        match press {
            Target::Row(_) | Target::Section(_) | Target::SideArea => self.scroll_side_by(delta),
            Target::Item(_) | Target::ListArea => self.scroll_list_by(delta),
            Target::Line(_) | Target::DetailArea | Target::Content => self.scroll_by(delta),
            _ => false,
        }
    }

    // ------------------------------------------------------------ the loading sheet

    /// The sheet shown while the machine is read.
    pub fn load_sheet(&self) -> Sheet {
        Sheet::new("Preparing system profile", (340.0, 128.0), vec![SheetButton::new(B_CANCEL, "Cancel", ButtonRole::Cancel)])
    }

    // ------------------------------------------------------------ hit tests and actions

    /// What is under the point `(x, y)`, in points.
    pub fn hit(&self, ui: &Ui, x: f32, y: f32) -> Target {
        if self.is_loading() {
            return match self.load_sheet().hit(self.window_rect(), UNIFIED_BAR_PT, 1.0, x, y) {
                Hit::Button(id) if id == B_CANCEL => Target::Cancel,
                _ => Target::Nothing,
            };
        }
        if y < UNIFIED_BAR_PT {
            let bar = self.toolbar();
            return self
                .toolbar_layout(ui)
                .visible
                .iter()
                .find(|(_, r)| r.contains(x, y))
                .and_then(|(id, _)| bar.items.iter().find(|i| i.id == *id).filter(|i| i.enabled).map(|i| i.id))
                .map_or(Target::Nothing, |id| {
                    if id != B_SEARCH {
                        return Target::Toolbar(id);
                    }
                    match self.search_rect(ui) {
                        Some(r) if search::hits_clear(r, !self.query().is_empty(), x, y) => Target::SearchClear,
                        _ => Target::Search,
                    }
                });
        }
        if x < SIDE_PT {
            let list = self.sidebar();
            let rect = self.sidebar_rect();
            if let Some(id) = list.hit_section(rect, self.side_scroll, x, y) {
                return Self::target_of_id(id as u64);
            }
            return match list.hit(rect, self.side_scroll, x, y) {
                Some(id) => Self::target_of_id(id as u64),
                None => Target::SideArea,
            };
        }
        let view = self.view();
        let panes = self.panes(&view);
        if panes.list.is_some_and(|l| l.contains(x, y)) {
            return match self.item_at(&view, y) {
                Some(i) => Target::Item(i),
                None => Target::ListArea,
            };
        }
        if panes.detail.contains(x, y) {
            return match self.line_at(&view, y) {
                Some(i) => Target::Line(i),
                None => Target::DetailArea,
            };
        }
        Target::Content
    }

    /// The action of a target. True when the window changed.
    pub fn activate(&mut self, target: Target) -> bool {
        if !matches!(target, Target::Nothing | Target::Content | Target::SideArea | Target::ListArea | Target::DetailArea) {
            self.notice = None;
        }
        match target {
            Target::Toolbar(B_REFRESH) => self.refresh(),
            Target::Search => self.begin_find(),
            Target::SearchClear => self.end_find(),
            Target::Row(category) => {
                self.focus = Focus::Sidebar;
                self.select(category)
            }
            Target::Section(section) => self.toggle_section(section),
            Target::Item(index) => {
                self.focus = Focus::List;
                let view = self.view();
                if index >= view.content.items.len() {
                    return false;
                }
                let changed = index != view.item;
                self.item = index;
                self.selection = Selection::None;
                self.scroll = 0.0;
                changed
            }
            Target::Line(index) => {
                self.focus = Focus::Detail;
                self.selection = Selection::One(index);
                true
            }
            Target::Cancel => self.cancel_load(),
            _ => false,
        }
    }

    /// The target a published node id names (the same ids the accessibility nodes publish).
    pub fn target_of_id(id: u64) -> Target {
        if id == B_REFRESH as u64 {
            return Target::Toolbar(B_REFRESH);
        }
        if id == B_SEARCH as u64 {
            return Target::Search;
        }
        if id == B_CANCEL as u64 {
            return Target::Cancel;
        }
        if id >= ROW_BASE {
            return Category::from_index((id - ROW_BASE) as usize).map_or(Target::Nothing, Target::Row);
        }
        if id >= SECTION_BASE {
            return SECTIONS.get((id - SECTION_BASE) as usize).copied().map_or(Target::Nothing, Target::Section);
        }
        Target::Nothing
    }

    // ------------------------------------------------------------ the line under the title

    /// The line under the title: a notice, the find hint, the open report's name, or the category's blurb.
    pub fn subtitle(&self) -> String {
        if let Some(n) = &self.notice {
            return n.clone();
        }
        if let Some(q) = &self.find {
            return format!("Find: {q}|  (Return finds the next category, Escape closes)");
        }
        if let Some(s) = &self.stored {
            return format!("Saved report {}. Refresh Information shows this computer again.", s.name);
        }
        self.selected.blurb().to_string()
    }

    // ------------------------------------------------------------ recent reports

    /// Reads the recent reports from the saved text (once).
    pub fn load_recents(&mut self, saved: Option<String>) {
        if !self.recents_loaded {
            self.recents_loaded = true;
            self.recents = recents::parse(&saved.unwrap_or_default());
        }
    }

    /// Notes a report as the newest recent one. Returns the grants that dropped off, for the caller to release.
    pub fn note_recent(&mut self, grant: GrantId, name: &str) -> Vec<GrantId> {
        recents::note(&mut self.recents, grant, name)
    }

    pub fn recents_text(&self) -> String {
        recents::format(&self.recents)
    }
}

/// What the line under the title says when a report could not be opened.
pub fn open_failure(code: DocErrorCode) -> &'static str {
    match code {
        DocErrorCode::Stale => "That report was replaced since it was last opened. Open it again with File > Open Report.",
        DocErrorCode::NotFound => "That report is gone.",
        DocErrorCode::Revoked | DocErrorCode::Denied => "System Information may no longer open that report. Open it again with File > Open Report.",
        DocErrorCode::Unsupported => "System Information is not allowed to open files.",
        DocErrorCode::Cancelled => "",
    }
}

/// Whether a failed reopen means the recent entry can never work again and should leave the list.
pub fn forgets_recent(code: DocErrorCode) -> bool {
    matches!(code, DocErrorCode::Stale | DocErrorCode::NotFound | DocErrorCode::Revoked | DocErrorCode::Denied)
}

/// The row of a pane that holds the window point `y`: `pane` is the pane's rectangle, `scroll` how far it is scrolled,
/// `count` the number of rows.
pub fn row_at(pane: &Rect, scroll: f32, count: usize, y: f32) -> Option<usize> {
    if y < pane.y || y >= pane.y + pane.h {
        return None;
    }
    let local = y - pane.y - 6.0 + scroll;
    if local < 0.0 {
        return None;
    }
    let index = (local / LINE_PT) as usize;
    (index < count).then_some(index)
}

/// How far a pane of `rows` lines can scroll in a pane `pane_h` tall.
pub fn max_scroll_of(rows: usize, pane_h: f32) -> f32 {
    (rows as f32 * LINE_PT + 12.0 - pane_h).max(0.0)
}

/// The scroll that brings line `index` fully into a pane `pane_h` tall, from `scroll`.
pub fn scroll_into_view(scroll: f32, index: usize, pane_h: f32) -> f32 {
    let top = 6.0 + index as f32 * LINE_PT;
    let bottom = top + LINE_PT;
    if top < scroll {
        top.max(0.0)
    } else if bottom > scroll + pane_h {
        (bottom - pane_h).max(0.0)
    } else {
        scroll
    }
}

/// The folded sections as saved: their titles, comma separated.
pub fn format_collapsed(sections: &[Section]) -> String {
    sections.iter().map(|s| s.title()).collect::<Vec<_>>().join(",")
}

pub fn parse_collapsed(text: &str) -> Vec<Section> {
    let mut out = Vec::new();
    for part in text.split(',') {
        if let Some(s) = SECTIONS.iter().copied().find(|s| s.title() == part.trim()) {
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}

/// "Serial number 1 2 3 A B" for the serial `1 2 3 a b`: every letter and digit alone, letters in capitals, other
/// characters left out. None when nothing is left to say.
pub fn speech_for_serial(serial: &str) -> Option<String> {
    let chars: Vec<String> = serial.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase().to_string()).collect();
    (!chars.is_empty()).then(|| format!("Serial number {}", chars.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::kv;

    #[test]
    fn target_ids_round_trip_the_published_nodes() {
        assert_eq!(SysInfo::target_of_id(B_REFRESH as u64), Target::Toolbar(B_REFRESH));
        assert_eq!(SysInfo::target_of_id(B_SEARCH as u64), Target::Search);
        assert_eq!(SysInfo::target_of_id(B_CANCEL as u64), Target::Cancel);
        for (i, c) in model::ALL.iter().enumerate() {
            assert_eq!(SysInfo::target_of_id(ROW_BASE + i as u64), Target::Row(*c));
        }
        for (i, s) in SECTIONS.iter().enumerate() {
            assert_eq!(SysInfo::target_of_id(SECTION_BASE + i as u64), Target::Section(*s));
        }
        assert_eq!(SysInfo::target_of_id(ROW_BASE + model::ALL.len() as u64), Target::Nothing);
        assert_eq!(SysInfo::target_of_id(SECTION_BASE + SECTIONS.len() as u64), Target::Nothing);
    }

    #[test]
    fn selecting_a_category_resets_the_scroll_once() {
        let mut w = SysInfo::new();
        w.scroll = 50.0;
        w.item = 3;
        assert!(w.select(Category::Usb));
        assert_eq!((w.scroll, w.item), (0.0, 0));
        assert!(!w.select(Category::Usb), "the same category is not a change");
    }

    #[test]
    fn scroll_stops_at_the_ends() {
        let mut w = SysInfo::new();
        w.report = Report::default();
        assert!(!w.scroll_by(-10.0), "already at the top");
        assert_eq!(w.max_scroll(), 0.0, "a short list does not scroll");
        assert!(!w.scroll_by(STEP_PT));
    }

    #[test]
    fn the_card_sits_below_the_title() {
        let w = SysInfo::new();
        assert!(w.card_top() > UNIFIED_BAR_PT);
        assert_eq!(w.content_x(), SIDE_PT + PAD_PT);
    }

    fn loaded() -> SysInfo {
        let mut w = SysInfo::new();
        w.extras = Extras { hostname: "midget".into(), modules: vec![crate::extras::Module { name: "snd".into(), size_bytes: 2048, used_by: 1 }], ..Default::default() };
        w
    }

    fn with_usb() -> SysInfo {
        let mut w = loaded();
        let dev = |port: &str, product: &str| bifrost_core::sysinfo::UsbDevice { port: port.into(), product: product.into(), vendor_id: 1, product_id: 2, ..Default::default() };
        w.report.usb = vec![dev("usb1", "Host"), dev("1-1", "Hub"), dev("1-1.1", "Mouse"), dev("1-2", "Keyboard")];
        w.select(Category::Usb);
        w
    }

    #[test]
    fn find_filters_rows_and_reports_no_match() {
        let mut w = loaded();
        w.select(Category::System);
        w.begin_find();
        w.find_type("midg");
        assert_eq!(w.rows(), vec![kv("Computer Name", "midget")]);
        w.find_type("zzz");
        assert_eq!(w.rows(), vec![kv("Find", "No matches in this category")]);
        assert!(w.find_backspace());
        assert!(w.end_find());
        assert!(w.rows().len() > 1, "closing find restores every row");
    }

    #[test]
    fn find_next_jumps_to_the_category_with_the_match() {
        let mut w = loaded();
        w.begin_find();
        w.find_type("snd");
        assert!(w.find_step(true));
        assert_eq!(w.selected, Category::Extensions);
        assert!(!w.find_step(true), "the only match is already shown");
    }

    #[test]
    fn find_in_an_open_report_visits_only_the_categories_it_holds() {
        let mut w = loaded();
        w.show_stored(Stored::of(
            "r.json",
            vec![(Category::Overview, Content::card(vec![kv("Model", "Alpha")])), (Category::Storage, Content::card(vec![kv("Disk", "Beta needle")]))],
        ));
        assert_eq!(w.visible_categories(), vec![Category::Overview, Category::Storage]);
        w.begin_find();
        w.find_type("needle");
        assert!(w.find_step(true));
        assert_eq!(w.selected, Category::Storage);
        w.find_type("zzz");
        assert!(!w.find_step(true));
    }

    #[test]
    fn copy_puts_the_picked_rows_on_the_clipboard() {
        let mut w = loaded();
        w.select(Category::System);
        assert_eq!(w.copy_text(), None);
        assert!(!w.can_copy());
        assert!(w.activate(Target::Line(3)));
        assert!(w.can_copy());
        assert_eq!(w.copy_text(), Some("Computer Name: midget".to_string()));
        w.select_all();
        assert_eq!(w.copy_text().unwrap().lines().count(), w.rows().len());
        w.select(Category::Usb);
        assert_eq!(w.selection, Selection::None, "a new category clears the pick");
    }

    #[test]
    fn copying_from_the_item_list_gives_the_picked_item() {
        let mut w = with_usb();
        assert!(w.activate(Target::Item(1)));
        assert_eq!(w.focus, Focus::List);
        assert!(w.can_copy());
        let text = w.copy_text().unwrap();
        assert!(text.starts_with("Hub: "), "{text}");
        w.activate(Target::Line(0));
        assert_eq!(w.copy_text().unwrap(), "Product: Hub");
    }

    #[test]
    fn show_less_information_keeps_only_the_overview() {
        let mut w = loaded();
        w.select(Category::System);
        assert!(w.toggle_mini());
        assert_eq!(w.selected, Category::Overview);
        assert_eq!(w.visible_categories(), vec![Category::Overview]);
        assert!(!w.report_text().contains("Operating System ("));
        assert!(w.toggle_mini());
        assert!(w.report_text().contains("Operating System ("));
    }

    #[test]
    fn arrow_steps_move_the_sidebar_selection_within_the_list() {
        let mut w = loaded();
        assert!(!w.step_category(-1), "already first");
        assert!(w.step_category(1));
        assert_eq!(w.selected, Category::Processor);
        w.select(Category::Language);
        assert!(!w.step_category(1), "already last");
        assert!(w.toggle_focus());
        assert_eq!(w.focus, Focus::Detail);
    }

    #[test]
    fn arrows_skip_the_categories_of_folded_sections() {
        let mut w = loaded();
        w.select(Category::Sata);
        assert!(w.toggle_section(Section::Network));
        assert!(w.step_category(1));
        assert_eq!(w.selected, Category::System, "the Network section is folded");
        assert!(w.step_category(-1));
        assert_eq!(w.selected, Category::Sata);
    }

    #[test]
    fn tab_visits_the_list_only_when_the_category_has_one() {
        let mut w = with_usb();
        assert_eq!(w.focus, Focus::Sidebar);
        w.toggle_focus();
        assert_eq!(w.focus, Focus::List);
        w.toggle_focus();
        assert_eq!(w.focus, Focus::Detail);
        w.toggle_focus();
        assert_eq!(w.focus, Focus::Sidebar);
        w.select(Category::System);
        w.toggle_focus();
        assert_eq!(w.focus, Focus::Detail, "a card category has no list");
    }

    #[test]
    fn a_list_category_splits_into_list_and_detail_panes() {
        let w = with_usb();
        let view = w.view();
        let panes = w.panes(&view);
        let list = panes.list.expect("USB is a list");
        assert_eq!(list.y, w.card_top());
        assert!(panes.detail.y >= list.y + list.h + PANE_GAP_PT - 0.01);
        assert!(panes.detail.h > 0.0);
        assert_eq!(view.detail()[0].0, "Product");
        let card = loaded().panes(&loaded().view());
        assert!(card.list.is_none(), "the Overview is a card");
        assert_eq!(card.detail.y, loaded().card_top());
    }

    #[test]
    fn stepping_the_item_changes_the_detail_and_keeps_the_line_in_view() {
        let mut w = with_usb();
        assert!(w.step_item(1));
        assert_eq!(w.view().detail()[0], kv("Product", "Hub"));
        assert!(w.step_item(2));
        assert_eq!(w.view().detail()[0], kv("Product", "Keyboard"));
        assert!(!w.step_item(1), "already last");
        assert!(w.step_item(-10));
        assert_eq!(w.item, 0);
        assert!(!w.step_item(-1));
        w.select(Category::System);
        assert!(!w.step_item(1), "a card has no items");
    }

    #[test]
    fn a_find_narrows_the_list_and_resets_the_pick() {
        let mut w = with_usb();
        w.step_item(2);
        w.begin_find();
        w.find_type("keyb");
        assert_eq!(w.view().content.items.len(), 1);
        assert_eq!(w.view().item, 0);
        assert_eq!(w.view().detail()[0], kv("Product", "Keyboard"));
    }

    #[test]
    fn rows_are_found_by_their_pane_and_scroll() {
        let pane = Rect { x: 0.0, y: 100.0, w: 300.0, h: 100.0 };
        assert_eq!(row_at(&pane, 0.0, 5, 100.0 + 6.0 + LINE_PT * 1.5), Some(1));
        assert_eq!(row_at(&pane, LINE_PT, 5, 100.0 + 6.0 + LINE_PT * 0.5), Some(1), "scrolled by a line");
        assert_eq!(row_at(&pane, 0.0, 5, 99.0), None, "above the pane");
        assert_eq!(row_at(&pane, 0.0, 5, 200.0), None, "below the pane");
        assert_eq!(row_at(&pane, 0.0, 1, 100.0 + 6.0 + LINE_PT * 1.5), None, "past the last row");
        assert_eq!(row_at(&pane, 0.0, 5, 103.0), None, "the padding above the first row");
        assert_eq!(max_scroll_of(2, 200.0), 0.0);
        assert_eq!(max_scroll_of(10, 100.0), 10.0 * LINE_PT + 12.0 - 100.0);
    }

    #[test]
    fn scrolling_brings_a_line_into_view() {
        let h = 4.0 * LINE_PT + 12.0;
        assert_eq!(scroll_into_view(0.0, 1, h), 0.0, "already visible");
        assert_eq!(scroll_into_view(0.0, 6, h), 7.0 * LINE_PT + 6.0 - h, "a line below scrolls down to it");
        assert_eq!(scroll_into_view(500.0, 0, h), 6.0, "a line above scrolls up to it");
    }

    #[test]
    fn a_point_on_the_card_names_its_line() {
        let w = loaded();
        let view = w.view();
        let top = w.card_top() + 6.0;
        assert_eq!(w.line_at(&view, top + LINE_PT * 1.5), Some(1));
        assert_eq!(w.line_at(&view, top - 3.0), None);
        assert_eq!(w.line_at(&view, top + LINE_PT * 40.0), None);
    }

    #[test]
    fn folding_a_section_hides_its_rows_and_is_remembered() {
        let mut w = loaded();
        let rows = |w: &SysInfo| w.sidebar().items.iter().filter(|i| matches!(i, SidebarItem::Row { .. })).count();
        assert!(w.sidebar().items.iter().any(|i| matches!(i, SidebarItem::Section { collapsed: false, .. })));
        assert_eq!(w.sidebar().content_height(), (3.0 * HEAD_PT) + (model::ALL.len() as f32 * ROW_PT));
        assert!(w.activate(Target::Section(Section::Hardware)));
        assert!(w.is_collapsed(Section::Hardware));
        assert!(w.prefs_dirty);
        assert_eq!(rows(&w), model::ALL.len(), "the rows stay in the list; the list hides them");
        assert!(w.sidebar().content_height() < (3.0 * HEAD_PT) + (model::ALL.len() as f32 * ROW_PT));
        assert_eq!(w.collapsed_text(), "Hardware");
        let mut fresh = SysInfo::new();
        fresh.set_collapsed_text(&w.collapsed_text());
        assert!(fresh.is_collapsed(Section::Hardware));
        assert!(w.toggle_section(Section::Hardware));
        assert!(!w.is_collapsed(Section::Hardware));
    }

    #[test]
    fn the_folded_sections_round_trip_their_text() {
        assert_eq!(parse_collapsed("Hardware, Software,Bogus,Hardware"), vec![Section::Hardware, Section::Software]);
        assert_eq!(format_collapsed(&[Section::Network]), "Network");
        assert!(parse_collapsed("").is_empty());
        assert_eq!(parse_collapsed(&format_collapsed(&SECTIONS)), SECTIONS.to_vec());
    }

    #[test]
    fn the_sidebar_scrolls_to_keep_the_selection_visible() {
        let mut w = loaded();
        w.size = (1200.0, 300.0);
        assert!(w.max_side_scroll() > 0.0, "28 rows do not fit in 300 points");
        w.select(Category::Usb);
        for _ in 0..20 {
            w.step_category(1);
        }
        assert!(w.side_scroll > 0.0);
        assert!(w.side_scroll <= w.max_side_scroll());
        assert!(w.scroll_side_by(-10_000.0));
        assert_eq!(w.side_scroll, 0.0);
    }

    #[test]
    fn a_dragged_pane_is_the_one_that_scrolls() {
        let mut w = with_usb();
        w.size = (1200.0, 300.0);
        assert!(w.drag_scroll(Target::Row(Category::Usb), 20.0));
        assert!(w.side_scroll > 0.0);
        assert_eq!(w.scroll, 0.0);
        assert!(!w.drag_scroll(Target::Toolbar(B_REFRESH), 20.0));
    }

    #[test]
    fn an_open_report_replaces_the_live_facts_until_refreshed() {
        let mut w = loaded();
        w.select(Category::System);
        w.show_stored(Stored::of("saved.json", vec![(Category::Overview, Content::card(vec![kv("Model", "Remote 9")]))]));
        assert_eq!(w.selected, Category::Overview, "the selection moves to a category the report holds");
        assert_eq!(w.rows(), vec![kv("Model", "Remote 9")]);
        assert!(w.subtitle().contains("saved.json"));
        assert_eq!(w.export_contents().len(), 1);
        assert!(w.show_live());
        assert!(w.rows().iter().any(|(k, _)| k == "Model"));
        assert_ne!(w.rows(), vec![kv("Model", "Remote 9")]);
        assert!(!w.show_live(), "already live");
    }

    #[test]
    fn a_notice_replaces_the_blurb_until_the_next_action() {
        let mut w = loaded();
        assert_eq!(w.subtitle(), Category::Overview.blurb());
        w.notice = Some("That file is not a system report".into());
        assert_eq!(w.subtitle(), "That file is not a system report");
        assert!(w.activate(Target::Row(Category::Memory)));
        assert_eq!(w.subtitle(), Category::Memory.blurb());
        w.begin_find();
        w.find_type("ab");
        assert!(w.subtitle().starts_with("Find: ab|"));
    }

    #[test]
    fn a_failed_open_explains_itself_and_forgets_dead_recents() {
        assert!(open_failure(DocErrorCode::Stale).contains("replaced"));
        assert!(open_failure(DocErrorCode::NotFound).contains("gone"));
        assert_eq!(open_failure(DocErrorCode::Cancelled), "");
        assert!(forgets_recent(DocErrorCode::Stale));
        assert!(forgets_recent(DocErrorCode::NotFound));
        assert!(!forgets_recent(DocErrorCode::Unsupported), "a missing permission is not the file's fault");
        assert!(!forgets_recent(DocErrorCode::Cancelled));
    }

    #[test]
    fn the_serial_is_spoken_one_character_at_a_time() {
        assert_eq!(speech_for_serial("ab-12"), Some("Serial number A B 1 2".to_string()));
        assert_eq!(speech_for_serial(" - "), None);
        assert_eq!(speech_for_serial(""), None);
        let mut w = SysInfo::new();
        assert_eq!(w.serial_speech(), None);
        w.report.firmware.serial = "C02X1".into();
        assert_eq!(w.serial_speech(), Some("Serial number C 0 2 X 1".to_string()));
    }

    #[test]
    fn a_finished_read_replaces_the_facts_and_a_cancelled_one_is_dropped() {
        let mut w = SysInfo::new();
        assert!(!w.cancel_load(), "nothing is loading");
        assert!(!w.poll_load(0.0));
        let (tx, rx) = mpsc::channel();
        w.load = Some(Loading { rx, t: 0.0 });
        assert!(w.is_loading());
        assert!(w.poll_load(10.0), "still waiting: the progress moves");
        let mut report = Report::default();
        report.os_name = "Test OS".into();
        tx.send((report, Extras::default())).unwrap();
        assert!(w.poll_load(20.0));
        assert!(!w.is_loading());
        assert!(w.loaded);
        assert_eq!(w.report.os_name, "Test OS");

        let (tx, rx) = mpsc::channel::<(Report, Extras)>();
        w.load = Some(Loading { rx, t: 0.0 });
        assert!(w.cancel_load());
        assert!(tx.send((Report::default(), Extras::default())).is_err(), "the cancelled read has no one to tell");
        assert!(w.notice.is_none(), "the facts already shown stay, with no complaint");
    }

    #[test]
    fn cancelling_the_first_read_says_how_to_read_again() {
        let mut w = SysInfo::new();
        let (_tx, rx) = mpsc::channel::<(Report, Extras)>();
        w.load = Some(Loading { rx, t: 0.0 });
        assert!(w.activate(Target::Cancel));
        assert!(w.notice.as_deref().is_some_and(|n| n.contains("Refresh Information")));
    }

    #[test]
    fn a_dead_worker_ends_the_loading_sheet() {
        let mut w = SysInfo::new();
        let (tx, rx) = mpsc::channel::<(Report, Extras)>();
        w.load = Some(Loading { rx, t: 0.0 });
        drop(tx);
        assert!(w.poll_load(1.0));
        assert!(!w.is_loading());
        assert!(w.notice.is_some());
    }

    #[test]
    fn recent_reports_load_once_and_report_what_falls_off() {
        let mut w = SysInfo::new();
        let saved = recents::format(&[Recent { grant: [1; 16], name: "A".into() }]);
        w.load_recents(Some(saved));
        w.load_recents(Some(String::new()));
        assert_eq!(w.recents.len(), 1, "the second call does not overwrite");
        assert!(w.note_recent([2; 16], "B").is_empty());
        assert_eq!(w.recents[0].name, "B");
        assert!(w.recents_text().contains("\tA\n"));
    }
}
