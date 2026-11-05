//! Application menus and their actions.
//!
//! The Browser publishes File, View, History, Bookmarks and Develop (`App::main_menu`). The shell merges File and View into its own
//! menus of the same names and puts the others after View, before Window (Golden Gate order: File, Edit, View, History, Bookmarks,
//! Develop, Window, Help; see `desktop_bar::bar_after_app`), so the order here is only the order inside those groups.
use super::*;

/// Item ids of the File, History and Bookmarks menus (`App::menu_action`). The fixed ones sit between the View menu's ids
/// (`main.rs`, 1 to 12) and `tabdiscard::NAV_MENU_BASE`; each list takes a range of its own (`id = base + index`), clear of the
/// History jumps (`NAV_MENU_BASE` and 50 more) and the Develop menu (1400 and up).
const MENU_NEW_TAB: u32 = 20;
const MENU_NEW_PRIVATE: u32 = 21;
const MENU_OPEN_LOCATION: u32 = 22;
const MENU_CLOSE_TAB: u32 = 23;
const MENU_REOPEN_TAB: u32 = 24;
const MENU_SHOW_HISTORY: u32 = 25;
const MENU_BACK: u32 = 26;
const MENU_FORWARD: u32 = 27;
const MENU_SHOW_BOOKMARKS: u32 = 28;
const MENU_ADD_BOOKMARK: u32 = 29;
const MENU_FAVORITES_BAR: u32 = 30;
/// History > Recently Closed: item `CLOSED_MENU_BASE + n` reopens the n-th newest closed tab.
const CLOSED_MENU_BASE: u32 = 1100;
const CLOSED_MENU_MAX: usize = 8;
/// History > Recently Visited: item `VISITED_MENU_BASE + n` opens the n-th most recently visited page in the tab.
const VISITED_MENU_BASE: u32 = 1200;
const VISITED_MENU_MAX: usize = 10;
/// Bookmarks > Favorites: item `BOOKMARK_MENU_BASE + n` opens the n-th bookmark in the tab.
const BOOKMARK_MENU_BASE: u32 = 1300;
const BOOKMARK_MENU_MAX: usize = 20;
/// Characters of a page title kept in a menu label.
const LABEL_MAX: usize = 60;

/// What the History menu shows, gathered from the selected tab, the closed-tab stack and the history store.
#[derive(Default)]
pub(super) struct HistoryMenu {
    /// Earlier and later pages of the selected tab as (index into its list, address), nearest first (`Browser::history_pages`).
    pub back: Vec<(usize, String)>,
    pub forward: Vec<(usize, String)>,
    pub can_back: bool,
    pub can_forward: bool,
    /// Labels of the recently closed tabs, newest first.
    pub closed: Vec<String>,
    /// Labels of the recently visited pages, newest first.
    pub visited: Vec<String>,
}

/// The Edit menu's find items (as the reference browser's Edit > Find): Find in Page is open to any page, Find Next and Find Previous while a
/// find is open.
pub(super) fn find_items(page: bool, find_open: bool) -> Vec<AppMenuItem> {
    vec![
        AppMenuItem::new(MENU_FIND, "Find in Page\u{2026}").key("cmd+f").enabled(page),
        AppMenuItem::new(MENU_FIND_NEXT, "Find Next").key("cmd+g").enabled(find_open),
        AppMenuItem::new(MENU_FIND_PREV, "Find Previous").key("cmd+shift+g").enabled(find_open),
    ]
}

/// The History menu: Show All History, Reopen Last Closed Tab, Back and Forward (each with the selected tab's earlier or later
/// pages under it, a jump per page), then the recently closed tabs and the recently visited pages.
pub(super) fn history_items(h: &HistoryMenu) -> Vec<AppMenuItem> {
    let page = |(index, url): &(usize, String)| AppMenuItem::new(tabdiscard::NAV_MENU_BASE + *index as u32, &tabdiscard::history_label(url)).depth(1);
    let mut items = vec![
        AppMenuItem::new(MENU_SHOW_HISTORY, "Show All History").key("cmd+y"),
        AppMenuItem::new(MENU_REOPEN_TAB, "Reopen Last Closed Tab").key("cmd+shift+t").enabled(!h.closed.is_empty()),
        AppMenuItem::separator(),
        AppMenuItem::new(MENU_BACK, "Back").key("cmd+[").enabled(h.can_back),
    ];
    items.extend(h.back.iter().map(page));
    items.push(AppMenuItem::new(MENU_FORWARD, "Forward").key("cmd+]").enabled(h.can_forward));
    items.extend(h.forward.iter().map(page));
    list_section(&mut items, "Recently Closed", CLOSED_MENU_BASE, &h.closed);
    list_section(&mut items, "Recently Visited", VISITED_MENU_BASE, &h.visited);
    items
}

/// What the Bookmarks menu shows.
#[derive(Default)]
pub(super) struct BookmarksMenu {
    /// A web page is open, so it can be bookmarked.
    pub page: bool,
    /// The Favorites bar is showing.
    pub favorites_bar: bool,
    /// Labels of the bookmarks (the Favorites bar's buttons), in the order they were added.
    pub favorites: Vec<String>,
}

/// The Bookmarks menu: Show Bookmarks, Add Bookmark, the Favorites bar switch and the bookmarks themselves.
pub(super) fn bookmarks_items(b: &BookmarksMenu) -> Vec<AppMenuItem> {
    let mut items = vec![
        AppMenuItem::new(MENU_SHOW_BOOKMARKS, "Show Bookmarks").key("cmd+alt+b"),
        AppMenuItem::new(MENU_ADD_BOOKMARK, "Add Bookmark").key("cmd+d").enabled(b.page),
        AppMenuItem::separator(),
        AppMenuItem::new(MENU_FAVORITES_BAR, if b.favorites_bar { "Hide Favorites Bar" } else { "Show Favorites Bar" }).key("cmd+shift+b"),
    ];
    list_section(&mut items, "Favorites", BOOKMARK_MENU_BASE, &b.favorites);
    items
}

/// File > Close Tab. It owns Cmd+W while the window has another tab to show; on the last tab it is disabled and has no key, so the
/// shell takes Cmd+W and closes the window (`Shell::app_owns_chord`).
pub(super) fn close_tab_item(tabs: usize) -> AppMenuItem {
    let item = AppMenuItem::new(MENU_CLOSE_TAB, "Close Tab");
    if tabs > 1 { item.key("cmd+w") } else { item.enabled(false) }
}

/// A separator, a header and the labels as its indented items (`base + index`); nothing for an empty list.
fn list_section(items: &mut Vec<AppMenuItem>, header: &str, base: u32, labels: &[String]) {
    if labels.is_empty() {
        return;
    }
    items.push(AppMenuItem::separator());
    items.push(AppMenuItem::submenu(header));
    items.extend(labels.iter().enumerate().map(|(n, label)| AppMenuItem::new(base + n as u32, label).depth(1)));
}

/// `Some(n)` when `id` is `base + n` with `n < len`.
fn in_range(id: u32, base: u32, len: usize) -> Option<usize> {
    id.checked_sub(base).map(|n| n as usize).filter(|&n| n < len)
}

/// A menu label for a page: its title, else its address (`history_label`); a tab that never loaded a page is "New Tab". Control
/// characters become spaces and a long title is cut with an ellipsis.
fn page_label(title: &str, url: &str) -> String {
    let title: String = title.trim().chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    if title.is_empty() {
        return if url.is_empty() || url == START { "New Tab".to_string() } else { tabdiscard::history_label(url) };
    }
    let mut label: String = title.chars().take(LABEL_MAX).collect();
    if title.chars().count() > LABEL_MAX {
        label.push('\u{2026}');
    }
    label
}

/// The `n` most recently visited entries, newest first, without sorting the whole history every time the menus are built.
fn recent_pages(history: &[store::Entry], n: usize) -> Vec<&store::Entry> {
    let mut top: Vec<&store::Entry> = Vec::with_capacity(n + 1);
    for entry in history {
        if top.len() == n && top.last().is_some_and(|oldest| oldest.last >= entry.last) {
            continue;
        }
        let at = top.iter().position(|kept| kept.last < entry.last).unwrap_or(top.len());
        top.insert(at, entry);
        top.truncate(n);
    }
    top
}

impl Browser {
    /// The page menu opens under the icon at the address field's leading edge (Alt+F and F10 open it there too).
    pub(super) fn main_menu_anchor(&self, ui: &Ui, w: u32) -> Rect {
        Self::page_menu_rect(self.address_rect(ui, w))
    }

    fn main_menu_items(&self) -> Vec<bifrost_app::controls::menu::MenuItem> {
        use bifrost_app::controls::menu::MenuItem as Item;
        use browser_menu as id;
        let page = self.view().is_some() && self.pool.as_ref().is_some_and(|pool| pool.alive());
        vec![
            Item::new(id::NEW, "New Tab").shortcut("⌘T"),
            Item::new(id::PRIVATE, "New Private Window").shortcut("⇧⌘N"),
            Item::new(id::REOPEN, "Reopen Closed Tab").shortcut("⇧⌘T").enabled(!self.session.closed.is_empty()),
            Item::new(id::CLOSE, "Close Tab").shortcut("⌘W").enabled(!self.tabs.is_empty()),
            Item::new(MENU_OVERVIEW, "Show All Tabs").checked(self.overview.open).enabled(!self.tabs.is_empty()),
            Item::separator(),
            Item::new(id::BOOKMARK, "Bookmark This Page").shortcut("⌘D").enabled(self.web_url().is_some()),
            Item::new(id::BOOKMARKS, "Bookmarks").shortcut("⌥⌘B"),
            Item::new(id::HISTORY, "History").shortcut("⌘Y"),
            Item::new(MENU_DOWNLOADS, "Downloads").enabled(!self.downloads.items.is_empty()),
            Item::separator(),
            Item::new(MENU_FIND, "Find in Page…").shortcut("⌘F").enabled(page),
            Item::new(id::PAGE, "Page Actions  ›").enabled(page),
            Item::new(id::DEVELOP, "Developer Tools  ›").enabled(page),
            Item::separator(),
            Item::new(id::SETTINGS, "Settings…").shortcut("⌘,"),
        ]
    }

    pub(super) fn toggle_main_menu(&mut self) {
        if self.main_menu.open() {
            self.main_menu.close();
            return;
        }
        self.editing = false;
        self.sync_address();
        self.overview.open = false;
        self.downloads.open = false;
        self.lib.close_all(&mut self.store);
        self.press = None;
        self.long = None;
        if let Some(pool) = self.pool.as_mut() { pool.mouse_leave(); }
        self.main_menu.show(self.main_menu_items());
    }

    pub(super) fn main_menu_pick(&mut self, ctx: &Ctx, id: u32) {
        use bifrost_app::controls::menu::MenuItem as Item;
        use browser_menu as ids;
        if !self.main_menu.allows(id) { return; }
        if id == ids::BACK {
            self.main_menu.show(self.main_menu_items());
            return;
        }
        if id == ids::PAGE || id == ids::DEVELOP {
            let groups: &[&str] = if id == ids::PAGE { &["View", "Edit"] } else { &["Develop"] };
            let mut items = vec![Item::new(ids::BACK, "‹  Main Menu"), Item::separator()];
            if id == ids::DEVELOP {
                items.push(Item::new(ids::INSPECTOR, "Web Inspector").shortcut("⌥⌘I").checked(self.inspector.is_open()));
                items.push(Item::separator());
            }
            let menus = self.menus();
            for (n, group) in groups.iter().enumerate() {
                if n > 0 { items.push(Item::separator()); }
                for menu in menus.iter().filter(|menu| menu.title == *group) {
                    for item in &menu.items {
                        if item.separator { items.push(Item::separator()); }
                        else {
                            let mut row = Item::new(item.id, &item.title).enabled(item.enabled && !item.submenu).checked(item.checked);
                            if !item.key.is_empty() {
                                row.shortcut = Some(item.key.replace("cmd+", "⌘").replace("shift+", "⇧").replace("alt+", "⌥").replace("backslash", "\\").to_uppercase());
                            }
                            items.push(row);
                        }
                    }
                }
            }
            if id == ids::PAGE {
                items.push(Item::separator());
                items.push(Item::new(MENU_SHARE, "Share…").enabled(self.web_url().is_some()));
            }
            self.main_menu.show(items);
            return;
        }
        self.main_menu.close();
        match id {
            ids::NEW => self.activate(NEW_TAB),
            ids::PRIVATE => self.open_private_window(),
            ids::REOPEN => self.reopen_closed(),
            ids::CLOSE => self.close_tab(self.current),
            ids::BOOKMARK => self.toggle_bookmark(),
            ids::BOOKMARKS | ids::HISTORY => {
                self.editing = false;
                self.lib.show_panel(if id == ids::BOOKMARKS { library::Mode::Bookmarks } else { library::Mode::History });
            }
            ids::SETTINGS => { self.editing = false; self.sheet.show(&self.prefs); }
            ids::INSPECTOR => self.inspector.toggle(self.pool.as_mut()),
            _ => { self.on_menu_action(ctx, id); }
        }
        self.place_web(ctx);
    }

    /// The History menu of the selected tab, the closed-tab stack and the history store (`history_items`).
    fn history_menu(&self) -> Vec<AppMenuItem> {
        let (back, forward) = self.history_pages();
        let view = self.view();
        history_items(&HistoryMenu {
            back,
            forward,
            can_back: view.is_some_and(|v| self.can_back(v)),
            can_forward: view.and_then(|v| self.pool.as_ref()?.state(v)).is_some_and(|s| s.can_go_forward),
            closed: self.session.closed.newest().take(CLOSED_MENU_MAX).map(|t| page_label(&t.title, &t.url)).collect(),
            visited: recent_pages(&self.store.history, VISITED_MENU_MAX).into_iter().map(|e| page_label(&e.title, &e.url)).collect(),
        })
    }

    /// The Bookmarks menu: the bookmarks are the Favorites bar's buttons (`bookmarks_items`).
    fn bookmarks_menu(&self) -> Vec<AppMenuItem> {
        bookmarks_items(&BookmarksMenu {
            page: self.web_url().is_some(),
            favorites_bar: self.lib.favbar,
            favorites: self.store.bookmarks.iter().take(BOOKMARK_MENU_MAX).map(|e| page_label(&e.title, &e.url)).collect(),
        })
    }

    /// A modal surface (Settings sheet, page dialog or the toolbar menu) is up: it takes the keys, so the app menu's key
    /// equivalents do not act behind it.
    fn menu_blocked(&self) -> bool {
        self.sheet.open || self.main_menu.open() || self.pageui.active()
    }

    /// File, Edit, View, History, Bookmarks and Develop (the shell orders them; see the module comment).
    pub(super) fn menus(&self) -> Vec<AppMenu> {
        let reader_ok = self.view().is_some_and(|v| self.reader.button(v));
        let reader_on = self.view().is_some_and(|v| self.reader.is_active(v));
        let page = self.view().is_some();
        let finished = self.downloads.items.iter().any(|i| i.state != downloads::State::Active);
        vec![
            AppMenu {
                title: "File".into(),
                items: vec![
                    AppMenuItem::new(MENU_NEW_TAB, "New Tab").key("cmd+t"),
                    AppMenuItem::new(MENU_NEW_PRIVATE, "New Private Window").key("cmd+shift+n"),
                    AppMenuItem::new(MENU_OPEN_LOCATION, "Open Location\u{2026}").key("cmd+l"),
                    AppMenuItem::separator(),
                    close_tab_item(self.tabs.len()),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_SHARE, "Share\u{2026}").enabled(self.web_url().is_some()),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_AUTOFILL, "AutoFill Password").enabled(self.view().is_some_and(|v| self.autofill.has_form(v))),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_DOWNLOADS, "Show Downloads").enabled(!self.downloads.items.is_empty()),
                    AppMenuItem::new(MENU_CLEAR_DOWNLOADS, "Clear Finished Downloads").enabled(finished),
                ],
            },
            AppMenu { title: "Edit".into(), items: find_items(page, self.find.open) },
            AppMenu {
                title: "View".into(),
                items: vec![
                    AppMenuItem::new(MENU_READER, "Show Reader").key("cmd+shift+r").checked(reader_on).enabled(reader_ok),
                    AppMenuItem::new(MENU_OVERVIEW, "Show All Tabs").key("cmd+shift+backslash").checked(self.overview.open).enabled(!self.tabs.is_empty()),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_RELOAD, "Reload Page").key("cmd+r").enabled(page),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_ZOOM_IN, "Zoom In").key("cmd+=").enabled(page),
                    AppMenuItem::new(MENU_ZOOM_OUT, "Zoom Out").key("cmd+-").enabled(page),
                    AppMenuItem::new(MENU_ACTUAL_SIZE, "Actual Size").key("cmd+0").enabled(page),
                ],
            },
            AppMenu { title: "History".into(), items: self.history_menu() },
            AppMenu { title: "Bookmarks".into(), items: self.bookmarks_menu() },
            AppMenu { title: "Develop".into(), items: self.dev.menu_items(page) },
        ]
    }

    pub(super) fn on_menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        match id {
            // the fixed File, History and Bookmarks items do nothing behind a modal surface (a key equivalent would reach them)
            MENU_NEW_TAB | MENU_NEW_PRIVATE | MENU_OPEN_LOCATION | MENU_CLOSE_TAB | MENU_REOPEN_TAB | MENU_SHOW_HISTORY | MENU_BACK
            | MENU_FORWARD | MENU_SHOW_BOOKMARKS | MENU_ADD_BOOKMARK | MENU_FAVORITES_BAR if self.menu_blocked() => false,
            MENU_NEW_TAB => {
                self.activate(NEW_TAB);
                self.place_web(ctx);
                true
            }
            MENU_NEW_PRIVATE => {
                self.open_private_window();
                true
            }
            MENU_OPEN_LOCATION => {
                self.begin_editing();
                true
            }
            // Cmd+W: closes the tab shown. The last tab never gets here with the key: Close Tab is disabled then and the shell
            // closes the window (`close_tab_item`); the menu item itself is disabled too.
            MENU_CLOSE_TAB => {
                self.close_tab(self.current);
                self.place_web(ctx);
                true
            }
            MENU_REOPEN_TAB => {
                self.reopen_closed();
                self.place_web(ctx);
                true
            }
            MENU_SHOW_HISTORY | MENU_SHOW_BOOKMARKS => {
                self.editing = false;
                self.downloads.open = false;
                self.lib.show_panel(if id == MENU_SHOW_HISTORY { library::Mode::History } else { library::Mode::Bookmarks });
                true
            }
            MENU_BACK => {
                self.activate(BACK);
                true
            }
            MENU_FORWARD => {
                self.activate(FORWARD);
                true
            }
            // the same actions as Cmd+D and Cmd+Shift+B (`lib_chord`)
            MENU_ADD_BOOKMARK => self.lib_chord(ctx, KEY_D, CHORD_META),
            MENU_FAVORITES_BAR => self.lib_chord(ctx, KEY_B, CHORD_META | CHORD_SHIFT),
            MENU_READER => {
                self.toggle_reader();
                true
            }
            MENU_SHARE => self.share_page(ctx),
            MENU_FIND => {
                self.editing = false;
                if !self.find.open {
                    self.show_find();
                }
                true
            }
            MENU_FIND_NEXT => {
                self.find_step(FindDirection::Forward);
                true
            }
            MENU_FIND_PREV => {
                self.find_step(FindDirection::Backward);
                true
            }
            MENU_RELOAD => {
                self.activate(RELOAD);
                true
            }
            MENU_ZOOM_IN => {
                self.zoom_by(1);
                true
            }
            MENU_ZOOM_OUT => {
                self.zoom_by(-1);
                true
            }
            MENU_ACTUAL_SIZE => {
                self.zoom_by(0);
                true
            }
            MENU_DOWNLOADS => {
                self.downloads.open = !self.downloads.items.is_empty();
                true
            }
            MENU_CLEAR_DOWNLOADS => {
                self.downloads.clear_finished();
                true
            }
            MENU_AUTOFILL => self.autofill_page(ctx),
            MENU_OVERVIEW => {
                self.toggle_overview();
                true
            }
            _ if devmenu::pick(id).is_some() => {
                self.dev_menu_action(ctx, id);
                true
            }
            _ => self.on_list_menu_action(ctx, id),
        }
    }

    /// The lists of the History and Bookmarks menus: a jump to an earlier or later page of the tab, a closed tab, a visited page
    /// or a bookmark (the ids are `base + index`, see `history_items` and `bookmarks_items`).
    fn on_list_menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        if let Some(target) = tabdiscard::history_target(id) {
            self.jump_to(target);
            return true;
        }
        if self.menu_blocked() {
            return false;
        }
        if let Some(n) = in_range(id, CLOSED_MENU_BASE, CLOSED_MENU_MAX) {
            let reopened = self.reopen_nth_closed(n);
            self.place_web(ctx);
            return reopened;
        }
        let url = if let Some(n) = in_range(id, VISITED_MENU_BASE, VISITED_MENU_MAX) {
            recent_pages(&self.store.history, VISITED_MENU_MAX).get(n).map(|e| e.url.clone())
        } else if let Some(n) = in_range(id, BOOKMARK_MENU_BASE, BOOKMARK_MENU_MAX) {
            self.store.bookmarks.get(n).map(|e| e.url.clone())
        } else {
            return false;
        };
        match url {
            Some(url) => {
                self.lib_open(&url);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(items: &[AppMenuItem]) -> Vec<&str> {
        items.iter().filter(|i| !i.separator).map(|i| i.title.as_str()).collect()
    }

    fn item<'a>(items: &'a [AppMenuItem], title: &str) -> &'a AppMenuItem {
        items.iter().find(|i| i.title == title).unwrap_or_else(|| panic!("no item {title}"))
    }

    #[test]
    fn fixed_ids_are_distinct_and_the_lists_stay_in_their_ranges() {
        let fixed = [
            MENU_NEW_TAB, MENU_NEW_PRIVATE, MENU_OPEN_LOCATION, MENU_CLOSE_TAB, MENU_REOPEN_TAB, MENU_SHOW_HISTORY, MENU_BACK, MENU_FORWARD,
            MENU_SHOW_BOOKMARKS, MENU_ADD_BOOKMARK, MENU_FAVORITES_BAR,
        ];
        for (i, a) in fixed.iter().enumerate() {
            assert!(*a > MENU_OVERVIEW && *a < tabdiscard::NAV_MENU_BASE, "id {a} collides with the View ids or the History jumps");
            assert!(fixed[i + 1..].iter().all(|b| b != a), "id {a} is used twice");
        }
        let lists = [(CLOSED_MENU_BASE, CLOSED_MENU_MAX), (VISITED_MENU_BASE, VISITED_MENU_MAX), (BOOKMARK_MENU_BASE, BOOKMARK_MENU_MAX)];
        for (i, (base, len)) in lists.iter().enumerate() {
            for n in 0..*len {
                let id = *base + n as u32;
                assert!(tabdiscard::history_target(id).is_none(), "{id} is a History jump");
                assert!(devmenu::pick(id).is_none(), "{id} is a Develop item");
                assert_eq!(in_range(id, *base, *len), Some(n));
                for (other, other_len) in &lists[i + 1..] {
                    assert!(in_range(id, *other, *other_len).is_none(), "{id} is in two lists");
                }
            }
            assert_eq!(in_range(*base + *len as u32, *base, *len), None, "one past the end is not an item");
        }
        assert_eq!(in_range(5, CLOSED_MENU_BASE, CLOSED_MENU_MAX), None, "below the base is not an item");
    }

    #[test]
    fn find_is_an_edit_item_open_to_any_page_and_next_or_previous_need_a_find() {
        let items = find_items(true, false);
        assert_eq!(titles(&items), vec!["Find in Page\u{2026}", "Find Next", "Find Previous"]);
        assert_eq!((item(&items, "Find in Page\u{2026}").id, item(&items, "Find in Page\u{2026}").key.as_str()), (MENU_FIND, "cmd+f"));
        assert!(item(&items, "Find in Page\u{2026}").enabled);
        assert!(!item(&items, "Find Next").enabled && !item(&items, "Find Previous").enabled, "no find open yet");
        let items = find_items(false, true);
        assert!(!item(&items, "Find in Page\u{2026}").enabled, "no page to search");
        assert!(item(&items, "Find Next").enabled && item(&items, "Find Previous").enabled);
        assert_eq!(item(&items, "Find Previous").key, "cmd+shift+g");
    }

    #[test]
    fn close_tab_owns_cmd_w_only_while_there_is_another_tab() {
        let two = close_tab_item(2);
        assert!(two.enabled && !two.separator);
        assert_eq!(two.key, "cmd+w");
        // the last tab: disabled and keyless, so the shell closes the window with Cmd+W
        for tabs in [1, 0] {
            let last = close_tab_item(tabs);
            assert!(!last.enabled, "{tabs} tab(s): Close Tab is disabled");
            assert!(last.key.is_empty(), "{tabs} tab(s): the chord is left to the shell");
        }
        assert_eq!(close_tab_item(1).id, MENU_CLOSE_TAB);
    }

    #[test]
    fn the_history_menu_has_the_usual_items_and_ids_that_map_back() {
        let menu = HistoryMenu {
            back: vec![(1, "https://a.example/".into()), (0, "https://b.example/x".into())],
            forward: vec![(3, "https://c.example".into())],
            can_back: true,
            can_forward: true,
            closed: vec!["Closed one".into(), "Closed two".into()],
            visited: vec!["Visited".into()],
        };
        let items = history_items(&menu);
        assert_eq!(
            titles(&items),
            vec![
                "Show All History", "Reopen Last Closed Tab", "Back", "a.example", "b.example/x", "Forward", "c.example", "Recently Closed", "Closed one",
                "Closed two", "Recently Visited", "Visited"
            ]
        );
        assert_eq!(item(&items, "Show All History").key, "cmd+y");
        assert_eq!(item(&items, "Reopen Last Closed Tab").key, "cmd+shift+t");
        assert!(item(&items, "Reopen Last Closed Tab").enabled);
        assert_eq!((item(&items, "Back").key.as_str(), item(&items, "Back").id), ("cmd+[", MENU_BACK));
        assert_eq!((item(&items, "Forward").key.as_str(), item(&items, "Forward").id), ("cmd+]", MENU_FORWARD));
        // a page of the tab's list jumps there; closed tabs and visited pages count from the newest
        assert_eq!(tabdiscard::history_target(item(&items, "a.example").id), Some(1));
        assert_eq!(tabdiscard::history_target(item(&items, "c.example").id), Some(3));
        assert_eq!(item(&items, "Closed one").id, CLOSED_MENU_BASE);
        assert_eq!(item(&items, "Closed two").id, CLOSED_MENU_BASE + 1);
        assert_eq!(item(&items, "Visited").id, VISITED_MENU_BASE);
        // the pages and lists sit under their entry, headers are not actions
        assert_eq!(item(&items, "a.example").depth, 1);
        assert!(item(&items, "Recently Closed").submenu);
        // every action has an id of its own
        let mut ids: Vec<u32> = items.iter().filter(|i| i.enabled && !i.submenu && !i.separator).map(|i| i.id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }

    #[test]
    fn the_fullest_menus_fit_the_protocol_limits() {
        // ten earlier and ten later pages, the closed and visited lists full: the shell drops items past 48 per menu
        let entries = |n: usize| (0..n).map(|i| (i, format!("https://site{i}.example/"))).collect::<Vec<_>>();
        let full = HistoryMenu {
            back: entries(10),
            forward: entries(10),
            can_back: true,
            can_forward: true,
            closed: (0..CLOSED_MENU_MAX).map(|i| format!("closed {i}")).collect(),
            visited: (0..VISITED_MENU_MAX).map(|i| format!("visited {i}")).collect(),
        };
        let limit = bifrost_app::proto::MAX_APP_MENU_ITEMS;
        assert!(history_items(&full).len() <= limit, "History has {} items", history_items(&full).len());
        let favorites = (0..BOOKMARK_MENU_MAX).map(|i| format!("bookmark {i}")).collect();
        assert!(bookmarks_items(&BookmarksMenu { page: true, favorites_bar: true, favorites }).len() <= limit);
    }

    #[test]
    fn an_empty_history_menu_greys_out_what_has_no_data() {
        let items = history_items(&HistoryMenu::default());
        assert_eq!(titles(&items), vec!["Show All History", "Reopen Last Closed Tab", "Back", "Forward"]);
        assert!(!item(&items, "Reopen Last Closed Tab").enabled, "nothing closed yet");
        assert!(!item(&items, "Back").enabled && !item(&items, "Forward").enabled);
        assert!(item(&items, "Show All History").enabled);
    }

    #[test]
    fn the_bookmarks_menu_lists_the_favorites_and_toggles_the_bar() {
        let menu = BookmarksMenu { page: true, favorites_bar: false, favorites: vec!["One".into(), "Two".into()] };
        let items = bookmarks_items(&menu);
        assert_eq!(titles(&items), vec!["Show Bookmarks", "Add Bookmark", "Show Favorites Bar", "Favorites", "One", "Two"]);
        assert_eq!(item(&items, "Show Bookmarks").key, "cmd+alt+b");
        assert_eq!(item(&items, "Add Bookmark").key, "cmd+d");
        assert!(item(&items, "Add Bookmark").enabled);
        assert_eq!(item(&items, "Show Favorites Bar").key, "cmd+shift+b");
        assert_eq!((item(&items, "One").id, item(&items, "Two").id), (BOOKMARK_MENU_BASE, BOOKMARK_MENU_BASE + 1));
        // no page to bookmark, the bar showing, no bookmarks yet
        let items = bookmarks_items(&BookmarksMenu { page: false, favorites_bar: true, favorites: Vec::new() });
        assert_eq!(titles(&items), vec!["Show Bookmarks", "Add Bookmark", "Hide Favorites Bar"]);
        assert!(!item(&items, "Add Bookmark").enabled);
    }

    #[test]
    fn labels_are_the_title_then_the_address_and_a_blank_tab_is_new_tab() {
        assert_eq!(page_label("  Example\tDomain\n", "https://example.com/"), "Example Domain");
        assert_eq!(page_label("", "https://www.example.com/a/"), "www.example.com/a");
        assert_eq!(page_label("", START), "New Tab");
        assert_eq!(page_label("", ""), "New Tab");
        assert_eq!(page_label("Home", START), "Home");
        let long = "x".repeat(LABEL_MAX + 5);
        assert_eq!(page_label(&long, "https://a"), format!("{}\u{2026}", "x".repeat(LABEL_MAX)));
        assert_eq!(page_label(&"y".repeat(LABEL_MAX), "https://a"), "y".repeat(LABEL_MAX), "a title that fits is not cut");
    }

    fn visit(url: &str, last: u64) -> store::Entry {
        store::Entry { url: url.to_string(), title: String::new(), visits: 1, last }
    }

    #[test]
    fn recent_pages_are_the_newest_n_in_order() {
        let history = vec![visit("a", 10), visit("b", 50), visit("c", 30), visit("d", 40), visit("e", 20)];
        let urls = |n: usize| recent_pages(&history, n).into_iter().map(|e| e.url.as_str()).collect::<Vec<_>>();
        assert_eq!(urls(3), vec!["b", "d", "c"]);
        assert_eq!(urls(10), vec!["b", "d", "c", "e", "a"]);
        assert!(urls(0).is_empty());
        assert!(recent_pages(&[], 3).is_empty());
        // equal times keep the history's order
        let ties = vec![visit("x", 5), visit("y", 5), visit("z", 5)];
        assert_eq!(recent_pages(&ties, 2).into_iter().map(|e| e.url.as_str()).collect::<Vec<_>>(), vec!["x", "y"]);
    }
}
