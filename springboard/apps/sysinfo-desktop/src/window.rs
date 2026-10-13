//! The System Information window's state and geometry: the sidebar of categories, the toolbar with Refresh, the detail
//! card and its scroll. Layout is in points; the drawing and the hit tests read the same rectangles, so a tap lands on
//! what is drawn.

use crate::extras::Extras;
use crate::model::{self, Category, Section};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::Rect;
use bifrost_app::kit::{TextCache, Ui};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets;
use bifrost_core::sysinfo::{Report, Roots};

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

/// The Refresh button in the toolbar.
pub const B_REFRESH: u32 = 1;
/// Sidebar row ids are `ROW_BASE` plus the category's index, so they never meet the toolbar ids.
pub const ROW_BASE: u64 = 100;

/// The File, Edit, View and Help menu items, so `main.rs` and the tests name the same ids.
pub const MENU_SAVE: u32 = 1;
pub const MENU_REFRESH: u32 = 2;
pub const MENU_LESS: u32 = 3;
pub const MENU_COPY: u32 = 4;
pub const MENU_SELECT_ALL: u32 = 5;
pub const MENU_FIND: u32 = 6;
pub const MENU_FIND_NEXT: u32 = 7;
pub const MENU_FIND_PREVIOUS: u32 = 8;
pub const MENU_HELP: u32 = 9;

/// Which pane the arrow keys act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
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
    Row(Category),
    /// A line of the detail card, by its index in the rows shown.
    Line(usize),
    Content,
}

pub struct SysInfo {
    pub report: Report,
    pub selected: Category,
    /// How far the detail card is scrolled, in points.
    pub scroll: f32,
    pub labels: TextCache,
    /// The window size, in the units the layout uses.
    pub size: (f32, f32),
    pub loaded: bool,
    /// The pointer's last y while dragging the card, and whether the drag has moved (a moved press is not a tap).
    pub drag_y: Option<f32>,
    pub dragged: bool,
    /// The target a press started on; the release activates it only when it lands on the same one.
    pub press: Target,
    /// Facts beyond the shared report: host name, uptime, modules, startup items, language.
    pub extras: Extras,
    /// Show Less Information: only the Overview.
    pub mini: bool,
    /// The text typed into Find, while the find bar is open.
    pub find: Option<String>,
    pub selection: Selection,
    pub focus: Focus,
    /// The save panel request and the report bytes waiting for its file.
    pub export: Option<(u32, Vec<u8>)>,
}

impl SysInfo {
    pub fn new() -> SysInfo {
        SysInfo {
            report: Report::default(),
            selected: Category::Overview,
            scroll: 0.0,
            labels: TextCache::default(),
            size: (1200.0, 780.0),
            loaded: false,
            drag_y: None,
            dragged: false,
            press: Target::Nothing,
            extras: Extras::default(),
            mini: false,
            find: None,
            selection: Selection::None,
            focus: Focus::Sidebar,
            export: None,
        }
    }

    /// Reads the machine again. The scroll stays where it is, clamped to the new length.
    pub fn reload(&mut self) {
        self.report = Report::collect(&Roots::system());
        self.extras = Extras::collect();
        self.loaded = true;
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
    }

    /// The rows of the selected category that the Find text keeps. A find with no hit shows one line that says so.
    pub fn rows(&self) -> Vec<model::Row> {
        let all = model::rows(self.selected, &self.report, &self.extras);
        let Some(query) = self.find.as_deref().filter(|q| !q.trim().is_empty()) else { return all };
        let kept: Vec<model::Row> = all.into_iter().filter(|r| model::row_matches(r, query)).collect();
        if kept.is_empty() {
            vec![model::kv("Find", "No matches in this category")]
        } else {
            kept
        }
    }

    /// The categories the sidebar lists: all of them, or the Overview alone in Show Less Information.
    pub fn visible_categories(&self) -> Vec<Category> {
        if self.mini {
            vec![Category::Overview]
        } else {
            model::ALL.to_vec()
        }
    }

    pub fn toggle_mini(&mut self) -> bool {
        self.mini = !self.mini;
        self.selection = Selection::None;
        if self.mini {
            self.selected = Category::Overview;
        }
        self.scroll = 0.0;
        true
    }

    pub fn begin_find(&mut self) -> bool {
        if self.find.is_none() {
            self.find = Some(String::new());
        }
        true
    }

    pub fn end_find(&mut self) -> bool {
        let was = self.find.take().is_some();
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
        was
    }

    /// Types into the find bar. False when the bar is closed.
    pub fn find_type(&mut self, text: &str) -> bool {
        let Some(q) = &mut self.find else { return false };
        q.push_str(text);
        self.selection = Selection::None;
        self.scroll = 0.0;
        true
    }

    pub fn find_backspace(&mut self) -> bool {
        let Some(q) = &mut self.find else { return false };
        let had = q.pop().is_some();
        self.scroll = 0.0;
        had
    }

    /// Find Next and Find Previous: moves to the next category with a match. False when there is none.
    pub fn find_step(&mut self, forward: bool) -> bool {
        let Some(q) = self.find.clone().filter(|q| !q.trim().is_empty()) else { return false };
        if self.mini {
            return false;
        }
        match model::next_match(self.selected, &q, forward, &self.report, &self.extras) {
            Some(c) => self.select(c),
            None => false,
        }
    }

    pub fn select_all(&mut self) -> bool {
        self.selection = Selection::All;
        true
    }

    /// The text Copy puts on the clipboard: the picked rows as `key: value` lines, or None when nothing is picked.
    pub fn copy_text(&self) -> Option<String> {
        let rows = self.rows();
        let lines: Vec<String> = match self.selection {
            Selection::None => return None,
            Selection::All => rows.iter().map(|(k, v)| format!("{k}: {v}")).collect(),
            Selection::One(i) => rows.get(i).map(|(k, v)| format!("{k}: {v}")).into_iter().collect(),
        };
        (!lines.is_empty()).then(|| lines.join("
"))
    }

    pub fn is_selected(&self, index: usize) -> bool {
        match self.selection {
            Selection::None => false,
            Selection::All => true,
            Selection::One(i) => i == index,
        }
    }

    /// The report as text for Save As: every category, or the Overview in Show Less Information.
    pub fn report_text(&self) -> String {
        model::report_text(&self.report, &self.extras, self.mini)
    }

    /// Moves the sidebar selection by `delta` rows among the visible categories. True when it moved.
    pub fn step_category(&mut self, delta: i32) -> bool {
        let list = self.visible_categories();
        let at = list.iter().position(|c| *c == self.selected).unwrap_or(0) as i32;
        let next = (at + delta).clamp(0, list.len() as i32 - 1) as usize;
        self.select(list[next])
    }

    pub fn toggle_focus(&mut self) -> bool {
        self.focus = if self.focus == Focus::Sidebar { Focus::Detail } else { Focus::Sidebar };
        true
    }

    /// The index of the detail line under the content-space point `y` (points, from the window top), if any.
    pub fn line_at(&self, y: f32) -> Option<usize> {
        let local = y - self.card_top() - 6.0 + self.scroll;
        if local < 0.0 {
            return None;
        }
        let index = (local / LINE_PT) as usize;
        (index < self.rows().len()).then_some(index)
    }

    /// How far the card can scroll: the rows beyond what the window shows, in points.
    pub fn max_scroll(&self) -> f32 {
        let content = self.rows().len() as f32 * LINE_PT + 12.0;
        let visible = (self.size.1 - UNIFIED_BAR_PT - CARD_TOP_PT - PAD_PT).max(0.0);
        (content - visible).max(0.0)
    }

    pub fn select(&mut self, category: Category) -> bool {
        if category == self.selected {
            return false;
        }
        self.selected = category;
        self.selection = Selection::None;
        self.scroll = 0.0;
        true
    }

    pub fn toolbar(&self) -> Toolbar {
        Toolbar::new(vec![ToolbarItem::button(B_REFRESH, "Refresh")])
    }

    pub fn toolbar_layout(&self, ui: &Ui) -> ToolbarLayout {
        widgets::toolbar_layout(ui, self.bar_rect(), &self.toolbar())
    }

    pub fn sidebar(&self) -> SourceList {
        let mut items = Vec::new();
        for section in [Section::Hardware, Section::Software] {
            if model::in_section(section).iter().all(|c| !self.visible_categories().contains(c)) {
                continue;
            }
            items.push(SidebarItem::header(section.title()));
            for category in model::in_section(section).into_iter().filter(|c| self.visible_categories().contains(c)) {
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

    /// The top of the detail card, in points.
    pub fn card_top(&self) -> f32 {
        UNIFIED_BAR_PT + CARD_TOP_PT
    }

    /// What is under the point `(x, y)`, in points.
    pub fn hit(&self, ui: &Ui, x: f32, y: f32) -> Target {
        if y < UNIFIED_BAR_PT {
            let bar = self.toolbar();
            return self
                .toolbar_layout(ui)
                .visible
                .iter()
                .find(|(_, r)| r.contains(x, y))
                .and_then(|(id, _)| bar.items.iter().find(|i| i.id == *id).filter(|i| i.enabled).map(|i| Target::Toolbar(i.id)))
                .unwrap_or(Target::Nothing);
        }
        if x < SIDE_PT {
            let list = self.sidebar();
            return match list.hit(self.sidebar_rect(), 0.0, x, y) {
                Some(id) => Self::target_of_id(id as u64),
                None => Target::Nothing,
            };
        }
        match self.line_at(y) {
            Some(index) if x >= self.content_x() => Target::Line(index),
            _ => Target::Content,
        }
    }

    /// The action of a target. True when the window changed.
    pub fn activate(&mut self, target: Target) -> bool {
        match target {
            Target::Toolbar(B_REFRESH) => {
                self.reload();
                true
            }
            Target::Row(category) => {
                self.focus = Focus::Sidebar;
                self.select(category)
            }
            Target::Line(index) => {
                self.focus = Focus::Detail;
                self.selection = Selection::One(index);
                true
            }
            _ => false,
        }
    }

    /// The target a published node id names (the same ids the accessibility nodes publish).
    pub fn target_of_id(id: u64) -> Target {
        if id == B_REFRESH as u64 {
            return Target::Toolbar(B_REFRESH);
        }
        if id >= ROW_BASE {
            return Category::from_index((id - ROW_BASE) as usize).map_or(Target::Nothing, Target::Row);
        }
        Target::Nothing
    }

    /// Moves the card by `delta` points (positive scrolls down), within the card's length.
    pub fn scroll_by(&mut self, delta: f32) -> bool {
        let next = (self.scroll + delta).clamp(0.0, self.max_scroll());
        if (next - self.scroll).abs() < f32::EPSILON {
            return false;
        }
        self.scroll = next;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_ids_round_trip_the_published_nodes() {
        assert_eq!(SysInfo::target_of_id(B_REFRESH as u64), Target::Toolbar(B_REFRESH));
        for (i, c) in model::ALL.iter().enumerate() {
            assert_eq!(SysInfo::target_of_id(ROW_BASE + i as u64), Target::Row(*c));
        }
        assert_eq!(SysInfo::target_of_id(ROW_BASE + model::ALL.len() as u64), Target::Nothing);
    }

    #[test]
    fn selecting_a_category_resets_the_scroll_once() {
        let mut w = SysInfo::new();
        w.scroll = 50.0;
        assert!(w.select(Category::Usb));
        assert_eq!(w.scroll, 0.0);
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

    #[test]
    fn find_filters_rows_and_reports_no_match() {
        let mut w = loaded();
        w.select(Category::System);
        w.begin_find();
        w.find_type("midg");
        assert_eq!(w.rows(), vec![model::kv("Computer Name", "midget")]);
        w.find_type("zzz");
        assert_eq!(w.rows(), vec![model::kv("Find", "No matches in this category")]);
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
    fn copy_puts_the_picked_rows_on_the_clipboard() {
        let mut w = loaded();
        w.select(Category::System);
        assert_eq!(w.copy_text(), None);
        assert!(w.activate(Target::Line(3)));
        assert_eq!(w.copy_text(), Some("Computer Name: midget".to_string()));
        w.select_all();
        assert_eq!(w.copy_text().unwrap().lines().count(), w.rows().len());
        w.select(Category::Usb);
        assert_eq!(w.selection, Selection::None, "a new category clears the pick");
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
    fn a_point_on_the_card_names_its_line() {
        let w = loaded();
        let top = w.card_top() + 6.0;
        assert_eq!(w.line_at(top + LINE_PT * 1.5), Some(1));
        assert_eq!(w.line_at(top - 3.0), None);
        assert_eq!(w.line_at(top + LINE_PT * 40.0), None);
    }
}
