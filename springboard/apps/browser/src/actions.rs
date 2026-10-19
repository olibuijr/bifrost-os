//! Browser actions shared by keyboard, menus and controls.
use super::*;

impl Browser {
    /// Cmd+D, Cmd+Y, Cmd+Option+B and Cmd+Shift+B. True when the chord was one of them.
    pub(super) fn lib_chord(&mut self, ctx: &Ctx, key: u32, mods: u32) -> bool {
        if mods & (CHORD_CTRL | CHORD_META) == 0 {
            return false;
        }
        let (alt, shift) = (mods & CHORD_ALT != 0, mods & CHORD_SHIFT != 0);
        match (key, alt, shift) {
            (KEY_D, false, false) => {
                self.editing = false;
                self.downloads.open = false;
                if let Some(s) = self.view().and_then(|v| self.pool.as_ref()?.state(v)) {
                    let (u, t) = (s.url.clone(), s.title.clone());
                    let r = self.address_rect(&ctx.ui, ctx.cfg.w);
                    self.lib.bookmark_page(&mut self.store, &u, &t, store::now(), r.x + r.w / 2.0);
                }
            }
            (KEY_Y, false, false) | (KEY_B, true, false) => {
                let mode = if key == KEY_Y { library::Mode::History } else { library::Mode::Bookmarks };
                if self.lib.open && self.lib.mode == mode {
                    self.lib.close_all(&mut self.store);
                } else {
                    self.editing = false;
                    self.downloads.open = false;
                    self.lib.show_panel(mode);
                }
            }
            (KEY_B, false, true) => {
                self.lib.favbar = !self.lib.favbar;
                self.lib.save(ctx);
                self.place_web(ctx);
            }
            _ => return false,
        }
        true
    }

    /// Settings > Import Bookmarks: the newest bookmark export in ~/Downloads is added (links already bookmarked stay).
    pub(super) fn import_bookmarks(&mut self) {
        let msg = match library::newest_export(&home().join("Downloads")) {
            None => "No bookmark export found in Downloads.".to_string(),
            Some(path) => match fs::read_to_string(&path) {
                Ok(html) => format!("Imported {} bookmarks.", library::import_html(&mut self.store, &html, store::now())),
                Err(_) => "The bookmark export could not be read.".to_string(),
            },
        };
        self.sheet.set_status(&msg);
    }

    /// Applies what the Settings sheet reported.
    pub(super) fn sheet_action(&mut self, ctx: &Ctx, a: prefs::Action) {
        match a {
            prefs::Action::None => {}
            prefs::Action::Changed => self.prefs.save(ctx),
            prefs::Action::RendererChanged => {
                self.prefs.save(ctx);
                self.restart_renderer();
                self.place_web(ctx);
                self.sheet.set_status(if self.error.is_some() { "Rendering could not start. Try CPU fallback." } else { "Rendering changed; open pages are reloading." });
            }
            prefs::Action::ClearHistory => {
                self.clear_browsing_history();
                self.sheet.set_status("History cleared.");
            }
            prefs::Action::ImportBookmarks => self.import_bookmarks(),
            prefs::Action::ClearData => {
                match self.clear_website_data() {
                    Ok(()) => self.sheet.set_status("Website data cleared."),
                    Err(error) => {
                        ctx.log(&format!("Website data clear failed: {error}"));
                        self.sheet.set_status("Website data could not be cleared.");
                    }
                }
                self.place_web(ctx);
            }
            prefs::Action::Close => {
                self.sheet.open = false;
                self.prefs.save(ctx);
            }
        }
    }

    /// A press on the page held still for `LONG_PRESS_S`: cancel the touch and open the context menu there.
    pub(super) fn fire_long_press(&mut self) {
        let Some((_, x, y, id)) = self.long.take() else { return };
        self.swallow = true;
        self.pageui.last_point = (x, y);
        if let Some(p) = &mut self.pool {
            p.touch(TouchKind::Cancel, id, x, y);
            if p.contains(x, y) {
                p.secondary_click(x, y);
            }
        }
    }

    /// Share sends the page address to the shell's share menu (Copy and the apps that take links). The shell
    /// answers with the chosen target; the app keeps no state for it.
    pub(super) fn share_page(&self, ctx: &Ctx) -> bool {
        if let Some(url) = self.web_url() {
            // under the toolbar's Share button when it is on the bar, else at the window's corner
            let anchor = self.bar_layout.as_ref().and_then(|l| l.rect_of(SHARE)).unwrap_or(Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 });
            let _ = bifrost_app::share::show_picker(ctx, &[bifrost_app::share::Payload::Url(url)], anchor);
        }
        false
    }

    /// File > AutoFill Password for the selected tab (autofill.rs).
    pub(super) fn autofill_page(&mut self, ctx: &Ctx) -> bool {
        let Some(v) = self.view() else { return false };
        let url = self.pool.as_ref().and_then(|p| p.state(v)).map(|s| s.url.clone()).unwrap_or_default();
        self.autofill.start(ctx, v, &url)
    }

    /// View > Show All Tabs and Cmd+Shift+backslash (overview.rs). Opening it closes the downloads list.
    pub(super) fn toggle_overview(&mut self) {
        self.overview.open = !self.overview.open && !self.tabs.is_empty();
        if self.overview.open {
            self.downloads.open = false;
        }
    }

    pub(super) fn toggle_reader(&mut self) {
        let (Some(v), Some(p)) = (self.view(), self.pool.as_mut()) else { return };
        let url = p.state(v).map(|s| s.url.clone()).unwrap_or_default();
        self.reader.toggle(p, v, &url, self.dark.unwrap_or(false));
    }

    /// Develop menu shortcuts: Cmd+Option+U View Source, Cmd+Shift+R Reload From Origin, Cmd+Option+R Responsive Design Mode.
    pub(super) fn dev_key(&mut self, key: u32, mods: u32) -> bool {
        if mods & (CHORD_CTRL | CHORD_META) == 0 {
            return false;
        }
        let Some(v) = self.view() else { return false };
        match (key, mods & CHORD_ALT != 0, mods & CHORD_SHIFT != 0) {
            (KEY_U, true, _) => {
                let url = self.pool.as_ref().and_then(|p| p.state(v)).map(|s| s.url.clone()).unwrap_or_default();
                if url.starts_with("http") {
                    // curl runs on a thread; tick() opens the source tab when it finishes.
                    // the result lands in the slot and wakes the idle loop; tick() collects it
                    let u = url.clone();
                    let slot = inbox::spawn(move || devmenu::fetch_source(&u));
                    self.sources.push((url, slot));
                }
            }
            (KEY_R, true, true) => {
                if let Some(p) = &mut self.pool {
                    p.reload_ignoring_cache(v);
                }
            }
            (KEY_R, true, false) => self.dev.toggle_responsive(),
            _ => return false,
        }
        true
    }

    /// The pointer shape over the chrome: an I-beam over the address field, unless a sheet or a page menu is up.
    pub(super) fn update_pointer(&mut self, ctx: &Ctx, x: f32, y: f32) {
        let ui = &ctx.ui;
        let (px, py) = (x / ui.pt(1.0), y / ui.pt(1.0));
        let field = self.address_rect(ui, ctx.cfg.w);
        // the page menu icon and reload / stop are buttons in the field: the pointer stays an arrow over them
        let on_button = self.field_shows_page() && (Self::page_menu_rect(field).contains(px, py) || Self::reload_rect(field).contains(px, py));
        let over = !self.sheet.open && !self.pageui.active() && py < UNIFIED_BAR_PT && field.contains(px, py) && !on_button;
        if let Some(change) = self.pointer.update(over) {
            system::send_pointer(ctx, change);
        }
    }

    /// Cmd+Shift+C copies the page's link (URL and title); Cmd+Shift+K copies the newest downloaded image.
    pub(super) fn system_chord(&mut self, ctx: &Ctx, key: u32, mods: u32) -> bool {
        if mods & (CHORD_CTRL | CHORD_META) == 0 || mods & CHORD_SHIFT == 0 || mods & CHORD_ALT != 0 {
            return false;
        }
        match key {
            KEY_C => {
                let page = self.view().and_then(|v| self.pool.as_ref()?.state(v)).map(|s| (s.url.clone(), s.title.clone()));
                match page {
                    Some((url, title)) if url.starts_with("http") => {
                        self.clip = system::send_copy(ctx, system::Clip::Link { url, title });
                        true
                    }
                    _ => false,
                }
            }
            KEY_K => {
                let image = system::newest_image(&self.downloads.items).and_then(|i| system::read_image(&i.path));
                match image {
                    Some((uti, bytes)) => {
                        self.clip = system::send_copy(ctx, system::Clip::Image { uti, bytes });
                        true
                    }
                    None => false,
                }
            }
            KEY_BACKSLASH => {
                self.toggle_overview();
                true
            }
            _ => false,
        }
    }

    /// A toolbar press or an accessibility activation. Share needs the context for the shell's picker; everything
    /// else is `activate`.
    pub(super) fn activate_in(&mut self, ctx: &Ctx, id: u32) {
        if id == SHARE {
            self.downloads.open = false;
            self.share_page(ctx);
        } else {
            self.activate(id);
        }
    }

    pub(super) fn activate(&mut self, id: u32) {
        if id == MAIN_MENU {
            self.toggle_main_menu();
            return;
        }
        if id != DOWNLOADS {
            self.downloads.open = false;
        }
        let view = self.view();
        match (id, &mut self.pool, view) {
            (BACK, Some(p), Some(v)) => match self.discard.back_target(v).map(str::to_string).filter(|_| !p.state(v).is_some_and(|s| s.can_go_back)) {
                // a revived tab has no engine history behind it: load the app-side previous entry
                Some(url) => p.load(v, &url),
                None => p.go_back(v),
            },
            (FORWARD, Some(p), Some(v)) => p.go_forward(v),
            (RELOAD, Some(p), Some(v)) => {
                p.reload(v);
            }
            (RELOAD, None, _) => {
                self.storm.clear();
                self.start()
            }
            // Servo 0.7.0 ignores the engine's stop command (the field still shows Stop until the load ends)
            (STOP, Some(p), Some(v)) => p.stop_loading(v),
            (OVERVIEW, _, _) => self.toggle_overview(),
            (findbar::PREV, _, _) => self.find_step(FindDirection::Backward),
            (findbar::NEXT, _, _) => self.find_step(FindDirection::Forward),
            (findbar::DONE, _, _) => self.hide_find(),
            // a click on the find field or the rest of the bar moves the typing there, off the address field
            (findbar::FIELD, _, _) => {
                self.editing = false;
                self.sugg.clear();
                self.sync_address();
            }
            (NEW_TAB, _, _) => {
                if self.pool.is_none() {
                    self.start();
                } else {
                    self.new_tab(START);
                }
            }
            (DOWNLOADS, _, _) => self.downloads.open = !self.downloads.open,
            (ADDRESS, _, _) => self.begin_editing(),
            _ => {}
        }
    }

    /// Develop menu: Responsive Design Mode, its size and pixel ratio, and the user agent preset.
    pub(super) fn dev_menu_action(&mut self, ctx: &Ctx, id: u32) {
        match devmenu::pick(id) {
            Some(devmenu::Pick::Responsive) => self.dev.toggle_responsive(),
            Some(devmenu::Pick::UserAgent(i)) => self.set_user_agent(i),
            Some(devmenu::Pick::Size(i)) => self.dev.preset(i),
            Some(devmenu::Pick::Dpr(d)) => self.dev.set_dpr(d),
            None => return,
        }
        self.place_web(ctx);
    }

    /// Develop > User Agent. Servo updates the preference for subsequent requests.
    pub(super) fn set_user_agent(&mut self, i: usize) {
        self.dev.set_ua(i);
        let ua = self.dev.user_agent();
        self.ctl.ua = ua.map(String::from);
        if let Some(p) = self.pool.as_mut() {
            p.set_user_agent(ua);
        }
    }
}
