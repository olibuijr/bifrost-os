//! The toolbar: back and forward, title, view switcher, sort, new folder, select and search (NativeToolbar items).
use crate::*;

/// Toolbar item ids; they are also the semantic control ids the shell sees (and does not drag the window over).
pub(crate) const BAR_BACK: u32 = 1;

pub(crate) const BAR_FORWARD: u32 = 2;

pub(crate) const BAR_ICONS: u32 = 3;

pub(crate) const BAR_SEARCH: u32 = 4;

pub(crate) const BAR_NEW_FOLDER: u32 = 7;

pub(crate) const BAR_SELECT: u32 = 8;

pub(crate) const BAR_TITLE: u32 = 9;

pub(crate) const BAR_SORT: u32 = 11;

pub(crate) const BAR_LIST: u32 = 12;

pub(crate) const BAR_COLUMNS: u32 = 13;

pub(crate) const BAR_GALLERY: u32 = 14;

/// The toolbar width (pt) from which the search field stays open; narrower, it collapses to a button.
pub(crate) const SEARCH_FIELD_MIN_BAR_W: f32 = 640.;

impl Files {
    /// The search field shows while it has focus or a query, and always on a wide toolbar, as Files's does (the
    /// field is what the toolbar publishes to assistive tech). On a narrow toolbar it is a round search button.
    pub(crate) fn search_open(&self) -> bool {
        self.searching || !self.query.is_empty() || self.bar_bounds().w >= SEARCH_FIELD_MIN_BAR_W
    }
    pub(crate) fn bar_bounds(&self) -> Rect {
        let x = if self.sidebar_hidden { traffic_lights_trailing() + widgets::TOOLBAR_GAP_PT } else { SIDEBAR_W + 12. };
        let h = widgets::CAPSULE_PT;
        Rect { x, y: (HEADER_H - h) / 2., w: (self.w - x - 16.).max(0.), h }
    }
    /// The window title: the open folder's name, or while a query searches a tree, the place it searches
    /// (`Searching "This Computer"`).
    pub(crate) fn title(&self) -> String {
        if self.scope_bar_shown() {
            return format!("Searching \u{201C}{}\u{201D}", self.scope_label(self.effective_scope()));
        }
        self.folder_title()
    }
    /// The open folder's name as the title and the Folder scope plate show it.
    pub(crate) fn folder_title(&self) -> String {
        if self.path == self.home {
            self.home_name()
        } else if self.in_bin() {
            "Trash".into()
        } else if let Some(tag) = tag_filter(&self.path) {
            tag.map_or("Tags", |t| t.label()).into()
        } else if let Some(i) = smart::folder_index(&self.path) {
            self.smart.get(i).map_or("Smart Folder", |f| f.name.as_str()).into()
        } else if provider::is_path(&self.path) {
            provider::shared().title(&self.path)
        } else if self.in_recents() {
            "Recents".into()
        } else {
            self.path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or("Computer".into())
        }
    }
    /// The Files toolbar: back and forward, the title, the view modes, sort, new folder and select, search.
    pub(crate) fn bar(&self) -> Toolbar {
        let writable = !self.in_recents() && !self.in_bin();
        Toolbar::new(vec![
            ToolbarItem::icon(BAR_BACK, "Back", "chevron.left").in_group(1).enabled(self.at > 0),
            ToolbarItem::icon(BAR_FORWARD, "Forward", "chevron.right").in_group(1).enabled(self.at + 1 < self.history.len()),
            ToolbarItem::custom(BAR_TITLE, "Title", self.title_w.min(self.title_room())),
            ToolbarItem::flexible_space(),
            ToolbarItem::icon(BAR_ICONS, "as Icons", "square.grid.2x2").in_group(2).selected(self.grid),
            ToolbarItem::icon(BAR_LIST, "as List", "list.bullet").in_group(2).selected(self.is_list()),
            ToolbarItem::icon(BAR_COLUMNS, "as Columns", "rectangle.split.3x1").in_group(2).selected(self.columns),
            ToolbarItem::icon(BAR_GALLERY, "as Gallery", "rectangle.grid.1x2").in_group(2).selected(self.vo.gallery),
            ToolbarItem::icon(BAR_SORT, &format!("View Options (sorted by {})", self.sort.label()), "arrow.up.arrow.down").in_group(3),
            ToolbarItem::icon(BAR_NEW_FOLDER, "New Folder", "folder.badge.plus").in_group(3).enabled(writable),
            ToolbarItem::icon(BAR_SELECT, "Select", "checkmark.circle").in_group(3).selected(self.select_mode),
            ToolbarItem::custom(BAR_SEARCH, "Search", self.search_w()),
        ])
    }
    /// The search field narrows in a narrow window so the title keeps its room; collapsed it is a round button.
    pub(crate) fn search_w(&self) -> f32 {
        if !self.search_open() { widgets::CAPSULE_PT } else if self.bar_bounds().w >= SEARCH_FIELD_MIN_BAR_W { 180. } else { 140. }
    }
    /// What the title may take: the bar less every other item and the gaps between groups.
    pub(crate) fn title_room(&self) -> f32 {
        let fixed = 9. * widgets::CAPSULE_PT + self.search_w() + 5. * widgets::TOOLBAR_GAP_PT; // the flexible space has a gap on each side
        (self.bar_bounds().w - fixed).max(40.)
    }
    pub(crate) fn bar_layout(&self) -> ToolbarLayout {
        self.bar().layout_with(self.bar_bounds(), |_| widgets::CAPSULE_PT, widgets::CAPSULE_PT, widgets::TOOLBAR_GAP_PT)
    }
    pub(crate) fn search_rect(&self) -> Option<Rect> {
        self.bar_layout().rect_of(BAR_SEARCH).map(|r| if self.search_open() { Rect { x: r.x, y: r.y + 4., w: r.w, h: r.h - 8. } } else { r })
    }
}
