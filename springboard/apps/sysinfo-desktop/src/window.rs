//! The System Information window's state and geometry: the sidebar of categories, the toolbar with Refresh, the detail
//! card and its scroll. Layout is in points; the drawing and the hit tests read the same rectangles, so a tap lands on
//! what is drawn.

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

/// What a press or a tap landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Nothing,
    Toolbar(u32),
    Row(Category),
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
        }
    }

    /// Reads the machine again. The scroll stays where it is, clamped to the new length.
    pub fn reload(&mut self) {
        self.report = Report::collect(&Roots::system());
        self.loaded = true;
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
    }

    pub fn rows(&self) -> Vec<model::Row> {
        model::rows(self.selected, &self.report)
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
            items.push(SidebarItem::header(section.title()));
            for category in model::in_section(section) {
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
        Target::Content
    }

    /// The action of a target. True when the window changed.
    pub fn activate(&mut self, target: Target) -> bool {
        match target {
            Target::Toolbar(B_REFRESH) => {
                self.reload();
                true
            }
            Target::Row(category) => self.select(category),
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
}
