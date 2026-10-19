//! Application menus and their actions.
use super::*;

impl Browser {
    pub(super) fn main_menu_anchor(&self, ui: &Ui, w: u32) -> Rect {
        self.bar_layout.as_ref().and_then(|layout| layout.rect_of(MAIN_MENU))
            .unwrap_or_else(|| {
                let bounds = Self::bar_bounds(ui, w);
                Rect { x: (bounds.x + bounds.w - widgets::CAPSULE_PT).max(0.0), y: 0.0, w: widgets::CAPSULE_PT, h: UNIFIED_BAR_PT }
            })
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
            let title = if id == ids::PAGE { "View" } else { "Develop" };
            let mut items = vec![Item::new(ids::BACK, "‹  Main Menu"), Item::separator()];
            if id == ids::DEVELOP {
                items.push(Item::new(ids::INSPECTOR, "Web Inspector").shortcut("⌥⌘I").checked(self.inspector.is_open()));
                items.push(Item::separator());
            }
            if let Some(menu) = self.menus().into_iter().find(|menu| menu.title == title) {
                for item in menu.items {
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

    /// File > Share, View > Show Reader and the History menu (Back and Forward entries of the selected tab).
    pub(super) fn menus(&self) -> Vec<AppMenu> {
        let reader_ok = self.view().is_some_and(|v| self.reader.button(v));
        let reader_on = self.view().is_some_and(|v| self.reader.is_active(v));
        let page = self.view().is_some();
        let finished = self.downloads.items.iter().any(|i| i.state != downloads::State::Active);
        vec![
            AppMenu {
                title: "File".into(),
                items: vec![
                    AppMenuItem::new(MENU_SHARE, "Share\u{2026}").enabled(self.web_url().is_some()),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_AUTOFILL, "AutoFill Password").enabled(self.view().is_some_and(|v| self.autofill.has_form(v))),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_DOWNLOADS, "Show Downloads").enabled(!self.downloads.items.is_empty()),
                    AppMenuItem::new(MENU_CLEAR_DOWNLOADS, "Clear Finished Downloads").enabled(finished),
                ],
            },
            AppMenu {
                title: "View".into(),
                items: vec![
                    AppMenuItem::new(MENU_READER, "Show Reader").key("cmd+shift+r").checked(reader_on).enabled(reader_ok),
                    AppMenuItem::new(MENU_OVERVIEW, "Show All Tabs").key("cmd+shift+backslash").checked(self.overview.open).enabled(!self.tabs.is_empty()),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_ZOOM_IN, "Zoom In").key("cmd+=").enabled(page),
                    AppMenuItem::new(MENU_ZOOM_OUT, "Zoom Out").key("cmd+-").enabled(page),
                    AppMenuItem::new(MENU_ACTUAL_SIZE, "Actual Size").key("cmd+0").enabled(page),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_FIND, "Find in Page\u{2026}").key("cmd+f").enabled(page),
                    AppMenuItem::new(MENU_FIND_NEXT, "Find Next").key("cmd+g").enabled(self.find.open),
                    AppMenuItem::new(MENU_FIND_PREV, "Find Previous").key("cmd+shift+g").enabled(self.find.open),
                ],
            },
            AppMenu { title: "Develop".into(), items: self.dev.menu_items(page) },
            AppMenu { title: "History".into(), items: self.history_menu() },
        ]
    }

    pub(super) fn on_menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        match id {
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
            _ => match tabdiscard::history_target(id) {
                Some(target) => {
                    self.jump_to(target);
                    true
                }
                None => false,
            },
        }
    }
}
