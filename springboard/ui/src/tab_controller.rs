//! Tab bar controller model (`NativeTabBarController`, `NativeTab`, `NativeTabGroup`, `NativeSearchTab`): an ordered list of screens,
//! one selected. On a compact width the tabs sit in a bar along the bottom edge, and a group is one bar item that opens
//! its screens as a list. On a wide width the bar becomes a source-list sidebar (`SourceList`), with each group as a
//! header over its screens. A search tab sits at the trailing end of the bar. Pure model: the SDK draws it, the shell
//! supplies the bounds, and the keyboard and pointer both go through `step`, `select_index` and `tap`.
use crate::geometry::Edge;
use crate::sidebar::{SidebarItem, SourceList};
use crate::Rect;

/// Height of the tab bar on a compact width, in points.
pub const TAB_BAR_PT: f32 = 49.0;
/// Width from which the sidebar replaces the tab bar, in points.
pub const SIDEBAR_MIN_WIDTH_PT: f32 = 768.0;
/// Width of the sidebar, in points.
pub const SIDEBAR_WIDTH_PT: f32 = 280.0;
/// Height of a sidebar row and of a group header, in points.
pub const SIDEBAR_ROW_PT: f32 = 36.0;
pub const SIDEBAR_HEADER_PT: f32 = 28.0;

/// Which chrome the tabs use at the current width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presentation {
    /// A bar along the bottom edge.
    TabBar,
    /// A source list along the leading edge.
    Sidebar,
}

impl Presentation {
    /// The presentation for a window `width_pt` points wide.
    pub fn for_width(width_pt: f32) -> Self {
        if width_pt >= SIDEBAR_MIN_WIDTH_PT {
            Presentation::Sidebar
        } else {
            Presentation::TabBar
        }
    }
}

/// What a tab is.
#[derive(Clone, Debug, PartialEq)]
pub enum TabKind {
    /// A screen with its own content (`NativeTab`).
    Screen,
    /// Screens shown together under one title (`NativeTabGroup`). Groups do not nest.
    Group(Vec<Tab>),
    /// The search tab, placed at the trailing end of the bar (`NativeSearchTab`).
    Search,
}

/// One tab. Ids are unique across the whole controller, groups included.
#[derive(Clone, Debug, PartialEq)]
pub struct Tab {
    pub id: u32,
    pub title: String,
    pub symbol: Option<String>,
    pub enabled: bool,
    pub kind: TabKind,
}

impl Tab {
    pub fn screen(id: u32, title: &str) -> Self {
        Tab { id, title: title.to_string(), symbol: None, enabled: true, kind: TabKind::Screen }
    }
    pub fn group(id: u32, title: &str, children: Vec<Tab>) -> Self {
        Tab { id, title: title.to_string(), symbol: None, enabled: true, kind: TabKind::Group(children) }
    }
    pub fn search(id: u32, title: &str) -> Self {
        Tab { id, title: title.to_string(), symbol: None, enabled: true, kind: TabKind::Search }
    }
    pub fn with_symbol(mut self, name: &str) -> Self {
        self.symbol = Some(name.to_string());
        self
    }
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

/// The result of a pointer tap on the bar or sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tap {
    /// A screen was selected; `changed` is false when it was already selected.
    Selected { id: u32, changed: bool },
    /// A group bar item was tapped on a compact width: its screens are now presented as a list (`open_group`).
    OpenedGroup(u32),
    /// Nothing tappable under the pointer.
    Missed,
}

/// Where the parts of the controller go inside the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabLayout {
    /// The screen area, between the bar and the sidebar.
    pub content: Rect,
    /// The tab bar along the bottom edge, present on a compact width.
    pub bar: Option<Rect>,
    /// The sidebar along the leading edge, present on a wide width.
    pub sidebar: Option<Rect>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TabBarController {
    tabs: Vec<Tab>,
    selected: Option<u32>,
    open_group: Option<u32>,
}

impl TabBarController {
    /// A controller over `tabs`, with the first selectable screen selected.
    pub fn new(tabs: Vec<Tab>) -> Self {
        let mut c = TabBarController { tabs, selected: None, open_group: None };
        c.selected = c.leaves().first().map(|t| t.id);
        c
    }

    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn selected(&self) -> Option<u32> {
        self.selected
    }

    /// The selected screen, or `None` when no screen is selectable.
    pub fn selected_tab(&self) -> Option<&Tab> {
        let id = self.selected?;
        self.leaves().into_iter().find(|t| t.id == id)
    }

    /// True while the search tab is the selected screen: the SDK shows its search field.
    pub fn is_search_selected(&self) -> bool {
        self.selected_tab().is_some_and(|t| matches!(t.kind, TabKind::Search))
    }

    /// The group whose screens are presented as a list on a compact width, if any.
    pub fn open_group(&self) -> Option<u32> {
        self.open_group
    }

    /// The enabled screens of the group `group_id`, in order, for the list that a compact width presents.
    pub fn group_screens(&self, group_id: u32) -> Vec<&Tab> {
        match self.tabs.iter().find(|t| t.id == group_id).map(|t| &t.kind) {
            Some(TabKind::Group(children)) => children.iter().filter(|t| t.enabled).collect(),
            _ => Vec::new(),
        }
    }

    /// Dismisses the presented group list without selecting anything.
    pub fn close_group(&mut self) {
        self.open_group = None;
    }

    /// Replaces the tabs. The selection stays when it is still a selectable screen, else the first screen is selected.
    pub fn set_tabs(&mut self, tabs: Vec<Tab>) {
        self.tabs = tabs;
        if self.open_group.is_some_and(|g| !self.tabs.iter().any(|t| t.id == g)) {
            self.open_group = None;
        }
        if !self.selected.is_some_and(|s| self.leaves().iter().any(|t| t.id == s)) {
            self.selected = self.leaves().first().map(|t| t.id);
        }
    }

    /// Selects the enabled screen `id`; true when the selection changed. A group id, a disabled tab or an unknown id
    /// is refused.
    pub fn select(&mut self, id: u32) -> bool {
        if self.selected == Some(id) || !self.leaves().iter().any(|t| t.id == id) {
            return false;
        }
        self.selected = Some(id);
        self.open_group = None;
        true
    }

    /// Selects the `n`th selectable screen (0-based): the Command-1 to Command-9 shortcut.
    pub fn select_index(&mut self, n: usize) -> bool {
        match self.leaves().get(n).map(|t| t.id) {
            Some(id) => self.select(id),
            None => false,
        }
    }

    /// Moves the selection to the next or previous screen. It wraps around, as the arrow keys and Tab do in a tab bar.
    /// True when the selection changed.
    pub fn step(&mut self, backwards: bool) -> bool {
        let ids: Vec<u32> = self.leaves().iter().map(|t| t.id).collect();
        if ids.is_empty() {
            return false;
        }
        let next = match self.selected.and_then(|s| ids.iter().position(|i| *i == s)) {
            Some(i) if backwards => (i + ids.len() - 1) % ids.len(),
            Some(i) => (i + 1) % ids.len(),
            None if backwards => ids.len() - 1,
            None => 0,
        };
        self.select(ids[next])
    }

    /// The window's areas for the current width.
    pub fn layout(&self, bounds: Rect) -> TabLayout {
        match Presentation::for_width(bounds.w) {
            Presentation::TabBar => {
                let (content, bar) = bounds.divide(TAB_BAR_PT, Edge::Bottom);
                TabLayout { content, bar: Some(bar), sidebar: None }
            }
            Presentation::Sidebar => {
                let (sidebar, content) = bounds.divide(SIDEBAR_WIDTH_PT, Edge::Left);
                TabLayout { content, bar: None, sidebar: Some(sidebar) }
            }
        }
    }

    /// The bar items with their rectangles: the screens and groups share the bar's width evenly, and the search tab is
    /// a square at the trailing end. Empty when the controller has no bar items.
    pub fn bar_items(&self, bar: Rect) -> Vec<(u32, Rect)> {
        let (search, regular): (Vec<&Tab>, Vec<&Tab>) = self.tabs.iter().partition(|t| matches!(t.kind, TabKind::Search));
        let search_w = if search.is_empty() { 0.0 } else { bar.h.min(bar.w) };
        let region = (bar.w - search_w).max(0.0);
        let item_w = if regular.is_empty() { 0.0 } else { region / regular.len() as f32 };
        let mut out: Vec<(u32, Rect)> = regular
            .iter()
            .enumerate()
            .map(|(i, t)| (t.id, Rect { x: bar.x + i as f32 * item_w, y: bar.y, w: item_w, h: bar.h }))
            .collect();
        if let Some(s) = search.first() {
            out.push((s.id, Rect { x: bar.right() - search_w, y: bar.y, w: search_w, h: bar.h }));
        }
        out
    }

    /// The sidebar as a source list: groups are headers over their screens. Search and screen tabs are rows.
    pub fn sidebar(&self) -> SourceList {
        let mut items = Vec::new();
        for tab in &self.tabs {
            match &tab.kind {
                TabKind::Group(children) => {
                    items.push(SidebarItem::header(&tab.title));
                    items.extend(children.iter().map(row));
                }
                _ => items.push(row(tab)),
            }
        }
        let mut list = SourceList::new(items, SIDEBAR_ROW_PT, SIDEBAR_HEADER_PT);
        list.set_selected(self.selected);
        list
    }

    /// Handles a pointer tap at `(x, y)` in the window, given the window's bounds. A group bar item opens its list on a
    /// compact width; on the sidebar a group is only a header.
    pub fn tap(&mut self, bounds: Rect, x: f32, y: f32) -> Tap {
        let layout = self.layout(bounds);
        if let Some(bar) = layout.bar {
            if !bar.contains(x, y) {
                return Tap::Missed;
            }
            let hit = self.bar_items(bar).into_iter().find(|(_, r)| r.contains(x, y)).map(|(id, _)| id);
            // The kind of the tapped item, copied out so no borrow of the tabs is held while the state changes.
            let target = hit.and_then(|id| {
                self.tabs.iter().find(|t| t.id == id).map(|t| (id, t.enabled, matches!(t.kind, TabKind::Group(_))))
            });
            return match target {
                Some((_, false, _)) | None => Tap::Missed,
                Some((id, true, true)) => {
                    self.open_group = Some(id);
                    Tap::OpenedGroup(id)
                }
                Some((id, true, false)) => Tap::Selected { id, changed: self.select(id) },
            };
        }
        if let Some(side) = layout.sidebar {
            return match self.sidebar().hit(side, 0.0, x, y) {
                Some(id) => Tap::Selected { id, changed: self.select(id) },
                None => Tap::Missed,
            };
        }
        Tap::Missed
    }

    /// The screens that are selectable, in presentation order: group children follow their group.
    fn leaves(&self) -> Vec<&Tab> {
        let mut out = Vec::new();
        for tab in &self.tabs {
            match &tab.kind {
                TabKind::Group(children) => out.extend(children.iter().filter(|t| t.enabled)),
                _ if tab.enabled => out.push(tab),
                _ => {}
            }
        }
        out
    }
}

fn row(tab: &Tab) -> SidebarItem {
    let item = SidebarItem::row(tab.id, &tab.title);
    let item = match &tab.symbol {
        Some(name) => item.with_symbol(name),
        None => item,
    };
    if tab.enabled {
        item
    } else {
        item.disabled()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(w: f32, h: f32) -> Rect {
        Rect { x: 0.0, y: 0.0, w, h }
    }

    fn sample() -> TabBarController {
        TabBarController::new(vec![
            Tab::screen(1, "Home").with_symbol("house"),
            Tab::group(2, "Library", vec![Tab::screen(20, "Books"), Tab::screen(21, "Music").disabled(), Tab::screen(22, "Films")]),
            Tab::screen(3, "Settings"),
            Tab::search(4, "Search"),
        ])
    }

    #[test]
    fn first_screen_is_selected_and_groups_are_not_selectable() {
        let mut c = sample();
        assert_eq!(c.selected(), Some(1));
        assert!(!c.select(2), "a group is not a screen");
        assert!(!c.select(21), "a disabled screen is refused");
        assert!(!c.select(99), "an unknown id is refused");
        assert_eq!(c.selected(), Some(1));
    }

    #[test]
    fn step_wraps_and_skips_disabled_and_group_children_are_in_order() {
        let mut c = sample();
        let order: Vec<u32> = std::iter::from_fn(|| {
            let id = c.selected()?;
            c.step(false);
            Some(id)
        })
        .take(5)
        .collect();
        assert_eq!(order, vec![1, 20, 22, 3, 4]);
        assert!(c.step(false));
        assert_eq!(c.selected(), Some(1), "wraps to the first screen");
        assert!(c.step(true));
        assert_eq!(c.selected(), Some(4), "backwards wraps to the last screen");
    }

    #[test]
    fn select_index_follows_presentation_order() {
        let mut c = sample();
        assert!(c.select_index(2));
        assert_eq!(c.selected(), Some(22));
        assert!(!c.select_index(9));
    }

    #[test]
    fn search_tab_reports_search_selected() {
        let mut c = sample();
        assert!(!c.is_search_selected());
        c.select(4);
        assert!(c.is_search_selected());
    }

    #[test]
    fn presentation_switches_at_the_sidebar_width() {
        assert_eq!(Presentation::for_width(SIDEBAR_MIN_WIDTH_PT - 1.0), Presentation::TabBar);
        assert_eq!(Presentation::for_width(SIDEBAR_MIN_WIDTH_PT), Presentation::Sidebar);
    }

    #[test]
    fn compact_layout_puts_the_bar_at_the_bottom() {
        let l = sample().layout(bounds(390.0, 844.0));
        let bar = l.bar.expect("bar on compact width");
        assert_eq!(bar, Rect { x: 0.0, y: 844.0 - TAB_BAR_PT, w: 390.0, h: TAB_BAR_PT });
        assert_eq!(l.content.h, 844.0 - TAB_BAR_PT);
        assert!(l.sidebar.is_none());
    }

    #[test]
    fn wide_layout_puts_the_sidebar_on_the_leading_edge() {
        let l = sample().layout(bounds(1200.0, 800.0));
        assert!(l.bar.is_none());
        let side = l.sidebar.expect("sidebar on wide width");
        assert_eq!(side.w, SIDEBAR_WIDTH_PT);
        assert_eq!(l.content.x, SIDEBAR_WIDTH_PT);
        assert_eq!(l.content.w, 1200.0 - SIDEBAR_WIDTH_PT);
    }

    #[test]
    fn bar_items_share_the_width_and_the_search_tab_trails() {
        let c = sample();
        let items = c.bar_items(Rect { x: 0.0, y: 0.0, w: 400.0, h: 50.0 });
        // Three screens and one group share the 350 points left of the square search tab.
        assert_eq!(items.len(), 4);
        let search = items.iter().find(|(id, _)| *id == 4).unwrap().1;
        assert_eq!(search, Rect { x: 350.0, y: 0.0, w: 50.0, h: 50.0 });
        let first = items.iter().find(|(id, _)| *id == 1).unwrap().1;
        assert!((first.w - 350.0 / 3.0).abs() < 0.01, "three regular items share the width");
    }

    #[test]
    fn tapping_a_group_on_compact_width_opens_it_and_a_child_closes_it() {
        let mut c = sample();
        let b = bounds(390.0, 844.0);
        let bar = c.layout(b).bar.unwrap();
        let group_rect = c.bar_items(bar).into_iter().find(|(id, _)| *id == 2).unwrap().1;
        let (gx, gy) = (group_rect.x + 1.0, group_rect.y + 1.0);
        assert_eq!(c.tap(b, gx, gy), Tap::OpenedGroup(2));
        assert_eq!(c.open_group(), Some(2));
        let screens: Vec<u32> = c.group_screens(2).iter().map(|t| t.id).collect();
        assert_eq!(screens, vec![20, 22], "disabled children are left out of the list");
        assert!(c.select(22));
        assert_eq!(c.open_group(), None, "selecting a child dismisses the list");
    }

    #[test]
    fn tapping_a_disabled_or_empty_bar_area_misses() {
        let mut c = sample();
        let b = bounds(390.0, 844.0);
        assert_eq!(c.tap(b, 10.0, 10.0), Tap::Missed, "above the bar");
        let mut d = TabBarController::new(vec![Tab::screen(1, "Only").disabled(), Tab::screen(2, "Next")]);
        let bar = d.layout(b).bar.unwrap();
        let first = d.bar_items(bar)[0].1;
        assert_eq!(d.tap(b, first.x + 1.0, first.y + 1.0), Tap::Missed);
        assert_eq!(d.selected(), Some(2));
    }

    #[test]
    fn sidebar_tap_selects_a_row_and_headers_do_not_hit() {
        let mut c = sample();
        let b = bounds(1200.0, 800.0);
        let side = c.layout(b).sidebar.unwrap();
        // Row 0 is "Home" at the top of the list; the header of the group sits below it.
        assert_eq!(c.tap(b, side.x + 5.0, side.y + SIDEBAR_ROW_PT / 2.0), Tap::Selected { id: 1, changed: false });
        let header_y = side.y + SIDEBAR_ROW_PT + 2.0;
        assert_eq!(c.tap(b, side.x + 5.0, header_y), Tap::Missed);
        let books_y = side.y + SIDEBAR_ROW_PT + SIDEBAR_HEADER_PT + 2.0;
        assert_eq!(c.tap(b, side.x + 5.0, books_y), Tap::Selected { id: 20, changed: true });
        assert_eq!(c.sidebar().selected(), Some(20));
    }

    #[test]
    fn set_tabs_keeps_a_valid_selection_and_reselects_otherwise() {
        let mut c = sample();
        c.select(3);
        c.set_tabs(vec![Tab::screen(3, "Settings"), Tab::screen(5, "New")]);
        assert_eq!(c.selected(), Some(3));
        c.set_tabs(vec![Tab::screen(5, "New")]);
        assert_eq!(c.selected(), Some(5));
        c.set_tabs(Vec::new());
        assert_eq!(c.selected(), None);
        assert!(!c.step(false));
    }
}
