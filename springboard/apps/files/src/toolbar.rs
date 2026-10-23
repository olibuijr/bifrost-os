//! The toolbar: back and forward, title, view switcher, group and sort, share, tags, the action menu and search (NativeToolbar items).
use crate::*;

/// Toolbar item ids; they are also the semantic control ids the shell sees (and does not drag the window over).
pub(crate) const BAR_BACK: u32 = 1;

pub(crate) const BAR_FORWARD: u32 = 2;

pub(crate) const BAR_ICONS: u32 = 3;

pub(crate) const BAR_SEARCH: u32 = 4;

pub(crate) const BAR_TITLE: u32 = 9;

pub(crate) const BAR_SORT: u32 = 11;

pub(crate) const BAR_LIST: u32 = 12;

pub(crate) const BAR_COLUMNS: u32 = 13;

pub(crate) const BAR_GALLERY: u32 = 14;

pub(crate) const BAR_SHARE: u32 = 15;

pub(crate) const BAR_TAGS: u32 = 16;

pub(crate) const BAR_MORE: u32 = 17;

/// The view pop-up that replaces the four view buttons while a query is active (one capsule: the current view's glyph
/// and a chevron).
pub(crate) const BAR_VIEW: u32 = 18;

/// The toolbar width (pt) from which the search field stays open; narrower, it collapses to a button.
pub(crate) const SEARCH_FIELD_MIN_BAR_W: f32 = 640.;

/// What the bar's fixed items take (points): ten glyph capsules (back, forward, four views, the pop-up, share, tags,
/// action), the pop-up's chevron, and six gaps (the flexible space has one on each side). The title and the search
/// slot share the rest.
const BAR_FIXED_W: f32 = 10. * widgets::CAPSULE_PT + widgets::TOOLBAR_MENU_CHEVRON_PT + 6. * widgets::TOOLBAR_GAP_PT;

/// The same while a query is active and the four views are one pop-up: seven capsules (back, forward, the two pop-ups,
/// share, tags, action), the two chevrons and the same six gaps.
const BAR_FIXED_W_SEARCH: f32 = 7. * widgets::CAPSULE_PT + 2. * widgets::TOOLBAR_MENU_CHEVRON_PT + 6. * widgets::TOOLBAR_GAP_PT;

/// The least width the title keeps.
const TITLE_MIN_W: f32 = 40.;

/// The widest the open search field gets.
const SEARCH_FIELD_W: f32 = 180.;

/// What the title of a search keeps before the field widens: `Searching "This Computer"` at the title size and a
/// little over (points).
const SEARCH_TITLE_W: f32 = 240.;

/// While a query is active the field gives way to the title down to this width (points)...
const SEARCH_ACTIVE_MIN_W: f32 = 100.;

/// ...and widens into what is left up to this one (the reference's grows once the view buttons have collapsed).
const SEARCH_ACTIVE_MAX_W: f32 = 280.;

/// The four views, in the order of the buttons and the View menu: the toolbar id, the glyph and the title.
pub(crate) const VIEWS: [(u32, &str, &str); 4] = [
    (BAR_ICONS, "square.grid.2x2", "as Icons"),
    (BAR_LIST, "list.bullet", "as List"),
    (BAR_COLUMNS, "rectangle.split.3x1", "as Columns"),
    (BAR_GALLERY, "rectangle.grid.1x2", "as Gallery"),
];

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
        } else if applications::is_store_dir(&self.path) {
            "Applications".into()
        } else if self.in_recents() {
            "Recents".into()
        } else {
            self.path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or("Computer".into())
        }
    }
    /// While a query is active the four view buttons collapse into one view pop-up and the search field widens, as the
    /// reference's toolbar does; the room this frees is what the title of the search (`Searching "This Computer"`)
    /// needs.
    pub(crate) fn view_popup(&self) -> bool {
        !self.query.trim().is_empty()
    }
    /// Whether view `i` of `VIEWS` is the one showing (what the button's segment and the menu's check mark follow).
    pub(crate) fn view_on(&self, i: usize) -> bool {
        match i {
            0 => self.grid,
            1 => self.is_list(),
            2 => self.columns,
            _ => self.vo.gallery,
        }
    }
    /// The glyph and the title of the view showing now, for the view pop-up.
    fn view_face(&self) -> (&'static str, &'static str) {
        let i = (0..VIEWS.len()).find(|&i| self.view_on(i)).unwrap_or(1);
        (VIEWS[i].1, VIEWS[i].2)
    }
    /// The Files toolbar (Golden Gate's Files): back and forward, the title, the view modes, the Group and Sort pop-up,
    /// Share, Tags and the action menu, then search. New Folder is File > New Folder (Shift+Cmd+N) and the menus. While a
    /// query is active the four view buttons are one pop-up (`view_popup`).
    pub(crate) fn bar(&self) -> Toolbar {
        let selected = !self.selection.is_empty();
        // a provider's stand-in paths are not files the shell can hand to another app
        let can_share = selected && !provider::is_path(&self.path);
        let mut items = vec![
            ToolbarItem::icon(BAR_BACK, "Back", "chevron.left").in_group(1).enabled(self.at > 0),
            ToolbarItem::icon(BAR_FORWARD, "Forward", "chevron.right").in_group(1).enabled(self.at + 1 < self.history.len()),
            ToolbarItem::custom(BAR_TITLE, "Title", self.title_w.min(self.title_room())),
            ToolbarItem::flexible_space(),
        ];
        if self.view_popup() {
            let (symbol, name) = self.view_face();
            items.push(ToolbarItem::menu(BAR_VIEW, &format!("View ({name})"), Some(symbol)));
        } else {
            for (i, (id, symbol, name)) in VIEWS.iter().enumerate() {
                items.push(ToolbarItem::icon(*id, name, symbol).in_group(2).selected(self.view_on(i)));
            }
        }
        items.extend(vec![
            ToolbarItem::menu(BAR_SORT, &format!("Group and Sort (sorted by {})", self.sort.label()), Some("arrow.up.arrow.down")),
            ToolbarItem::icon(BAR_SHARE, "Share", "square.and.arrow.up").in_group(3).enabled(can_share),
            ToolbarItem::icon(BAR_TAGS, "Tags", "tag").in_group(3).enabled(selected),
            ToolbarItem::icon(BAR_MORE, "Action", "ellipsis").in_group(3),
            ToolbarItem::custom(BAR_SEARCH, "Search", self.search_w()),
        ]);
        Toolbar::new(items)
    }
    /// What the fixed items of the bar take now: less while the views are one pop-up.
    fn bar_fixed_w(&self) -> f32 {
        if self.view_popup() { BAR_FIXED_W_SEARCH } else { BAR_FIXED_W }
    }
    /// The search field narrows in a narrow window so the title keeps its room; collapsed it is a round button. While a
    /// query is active the title of the search keeps its room first and the field widens into what is left.
    pub(crate) fn search_w(&self) -> f32 {
        if !self.search_open() {
            return widgets::CAPSULE_PT;
        }
        let room = self.bar_bounds().w - self.bar_fixed_w();
        if self.view_popup() {
            return (room - SEARCH_TITLE_W).clamp(SEARCH_ACTIVE_MIN_W, SEARCH_ACTIVE_MAX_W);
        }
        (room - TITLE_MIN_W).clamp(widgets::CAPSULE_PT, SEARCH_FIELD_W)
    }
    /// What the title may take: the bar less every other item and the gaps between groups, and a point of slack so that
    /// rounding in the layout never pushes the last item into the overflow menu.
    pub(crate) fn title_room(&self) -> f32 {
        (self.bar_bounds().w - self.bar_fixed_w() - self.search_w() - 1.).max(TITLE_MIN_W)
    }
    /// Where each item sits. The pop-up is a capsule and its chevron wide, as `widgets::toolbar` draws it.
    pub(crate) fn bar_layout(&self) -> ToolbarLayout {
        let width = |it: &ToolbarItem| if it.menu { widgets::CAPSULE_PT + widgets::TOOLBAR_MENU_CHEVRON_PT } else { widgets::CAPSULE_PT };
        self.bar().layout_with(self.bar_bounds(), width, widgets::CAPSULE_PT, widgets::TOOLBAR_GAP_PT)
    }
    pub(crate) fn search_rect(&self) -> Option<Rect> {
        self.bar_layout().rect_of(BAR_SEARCH).map(|r| if self.search_open() { Rect { x: r.x, y: r.y + 4., w: r.w, h: r.h - 8. } } else { r })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::fixture;

    fn ids(files: &Files) -> Vec<u32> {
        files.bar().items.iter().map(|i| i.id).collect()
    }

    fn font_ui() -> Ui {
        let font = springboard::font::Font::parse(Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice())).unwrap();
        Ui::new(font, 1.25)
    }

    #[test]
    fn outside_a_search_the_bar_keeps_the_four_view_buttons() {
        let files = fixture(vec![]);
        assert!(!files.view_popup());
        let on_bar = ids(&files);
        for id in [BAR_ICONS, BAR_LIST, BAR_COLUMNS, BAR_GALLERY] {
            assert!(on_bar.contains(&id), "{id} is on the bar");
        }
        assert!(!on_bar.contains(&BAR_VIEW), "no pop-up beside the buttons");
        let bar = files.bar();
        let icons = bar.items.iter().find(|i| i.id == BAR_ICONS).unwrap();
        assert!(icons.selected && icons.group == 2 && !icons.menu, "the current view is the chosen segment of one capsule");
        assert_eq!(files.bar_fixed_w(), BAR_FIXED_W);
    }

    #[test]
    fn while_a_query_is_active_the_views_are_one_pop_up_showing_the_current_view() {
        let mut files = fixture(vec![]);
        files.query.push_str("aqua");
        assert!(files.view_popup());
        assert_eq!(files.bar_fixed_w(), BAR_FIXED_W_SEARCH);
        let on_bar = ids(&files);
        for id in [BAR_ICONS, BAR_LIST, BAR_COLUMNS, BAR_GALLERY] {
            assert!(!on_bar.contains(&id), "{id} is folded into the pop-up");
        }
        let face = |files: &Files| {
            let bar = files.bar();
            let view = bar.items.iter().find(|i| i.id == BAR_VIEW).expect("the view pop-up is on the bar");
            assert!(view.menu && view.enabled && view.group == 0, "its own capsule, with a chevron");
            (view.symbol.clone().unwrap_or_default(), view.label.clone())
        };
        assert_eq!(face(&files), ("square.grid.2x2".to_string(), "View (as Icons)".to_string()), "the icon view's glyph");
        files.set_view(false, false);
        assert_eq!(face(&files), ("list.bullet".to_string(), "View (as List)".to_string()));
        files.set_view(false, true);
        assert_eq!(face(&files), ("rectangle.split.3x1".to_string(), "View (as Columns)".to_string()));
        files.set_gallery();
        assert_eq!(face(&files), ("rectangle.grid.1x2".to_string(), "View (as Gallery)".to_string()));
        // it sits between the title and the Group and Sort pop-up, as wide as a pull-down
        let layout = files.bar_layout();
        let (view, sort) = (layout.rect_of(BAR_VIEW).unwrap(), layout.rect_of(BAR_SORT).unwrap());
        assert!(view.right() < sort.x, "the views come before Group and Sort");
        assert_eq!(view.w, widgets::CAPSULE_PT + widgets::TOOLBAR_MENU_CHEVRON_PT);
        // a query of spaces is no search, and clearing the query brings the four buttons back
        files.query.clear();
        files.query.push_str("  ");
        assert!(!files.view_popup());
        files.query.clear();
        assert!(ids(&files).contains(&BAR_LIST) && !ids(&files).contains(&BAR_VIEW));
    }

    #[test]
    fn the_collapsed_views_give_the_title_of_a_search_its_room_and_the_bar_still_fits() {
        let mut files = fixture(vec![]);
        for w in [768., 960., 1280.] {
            files.w = w;
            files.query.clear();
            let before = files.title_room();
            files.query.push_str("aqua");
            let after = files.title_room();
            assert!(after >= before.min(SEARCH_TITLE_W - 1.), "{w} pt window: {after} pt for the title while searching, {before} before");
            if w < 1000. {
                assert!(after > before, "{w} pt window: the freed room goes to the title ({after} against {before})");
            }
            let layout = files.bar_layout();
            assert!(layout.overflow.is_empty(), "{w} pt window: nothing overflows");
            assert!(layout.rect_of(BAR_SEARCH).is_some() && layout.rect_of(BAR_VIEW).is_some(), "{w} pt window: the field and the pop-up show");
            assert!(files.search_w() >= SEARCH_ACTIVE_MIN_W && files.search_w() <= SEARCH_ACTIVE_MAX_W, "{w} pt window: the field stays in its range");
        }
    }

    #[test]
    fn the_whole_title_of_a_search_fits_the_window_where_the_re_test_cut_it() {
        let ui = font_ui();
        let mut files = fixture(vec![]);
        files.w = 947.;
        files.query.push_str("Reports");
        assert_eq!(files.title(), "Searching \u{201C}This Computer\u{201D}");
        let want = ui.measure(&files.title(), MAC_TITLE3_PT, Weight::Bold) / ui.pt(1.) + 4.;
        assert!(want <= files.title_room(), "the title wants {want} pt and the bar gives it {}", files.title_room());
    }
}
