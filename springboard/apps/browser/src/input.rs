//! Input routing between browser chrome and web content.
use super::*;

impl Browser {
    pub(super) fn on_input(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let ui = &ctx.ui;
        if let Input::Key(key) = &ev {
            if !self.pageui.active() && (key.as_chord() == Some((KEY_F, CHORD_ALT)) || matches!(key, KeyInput::Code(121))) {
                self.sheet.open = false;
                self.toggle_main_menu();
                return true;
            }
        }
        if self.main_menu.open() || (self.main_menu.captures_touch() && matches!(ev, Input::Touch { .. })) {
            let anchor = self.main_menu_anchor(ui, ctx.cfg.w);
            let picked = match &ev {
                Input::Touch { kind, x, y, .. } => self.main_menu.touch(ui, ctx.cfg.w, ctx.cfg.h, anchor, *kind, *x, *y),
                Input::Key(key) => self.main_menu.key(ui, ctx.cfg.w, ctx.cfg.h, anchor, key, self.clock.elapsed().as_secs_f64()),
                Input::Activate(id) if *id >= browser_menu::NODE_BASE => Some((*id - browser_menu::NODE_BASE) as u32),
                Input::Open(_) => { self.main_menu.close(); None }
                _ => None,
            };
            if let Some(id) = picked { self.main_menu_pick(ctx, id); }
            if !matches!(ev, Input::Open(_)) { return true; }
        }
        if let Input::Touch { x, y, .. } = ev {
            self.update_pointer(ctx, x, y);
        }
        // Clear stale tab hover on non-pointer app input; the pointer callback handles viewport leave.
        if !matches!(ev, Input::Touch { kind: TouchKind::Down | TouchKind::Move | TouchKind::Up, .. }) {
            self.hover = None;
        }
        match ev {
            Input::Open(url) => {
                if self.pool.is_some() {
                    self.new_tab(&url);
                } else {
                    self.initial_url = Some(url);
                    self.start();
                }
                self.place_web(ctx);
                true
            }
            Input::Activate(id) if self.pageui.active() => {
                self.pageui.activate_control(id, &mut self.pool);
                true
            }
            Input::Activate(id) if id >= startpage::NODE0 => {
                if let Some(url) = self.start_page.site_of_node(id).map(|s| s.url.clone()) {
                    self.start_open(&url);
                }
                true
            }
            Input::Activate(id) => {
                if id >= TAB0 {
                    self.select((id - TAB0) as usize);
                } else {
                    self.activate_in(ctx, id as u32);
                }
                self.place_web(ctx);
                true
            }
            Input::Context { x, y } => {
                if self.pageui.active() {
                    return false;
                }
                if self.start_swallows_context(x, y) {
                    return true;
                }
                if let Some(p) = self.pool.as_mut().filter(|p| p.contains(x, y)) {
                    self.pageui.last_point = (x, y);
                    p.secondary_click(x, y);
                }
                true
            }
            Input::Touch { kind, x, y, .. } if self.sheet.open => {
                self.hover = None;
                if matches!(kind, TouchKind::Down | TouchKind::Move) {
                    self.sheet.touch_drag(ui, ctx.cfg.w, ctx.cfg.h, kind == TouchKind::Down, x, y);
                }
                if kind == TouchKind::Up {
                    let a = self.sheet.touch_up(ui, ctx.cfg.w, ctx.cfg.h, x, y, &mut self.prefs);
                    self.sheet_action(ctx, a);
                }
                true
            }
            Input::Key(k) if self.sheet.open => {
                let a = match k.as_chord() {
                    Some((KEY_COMMA, mods)) if mods & (CHORD_CTRL | CHORD_META) != 0 => prefs::Action::Close,
                    Some(_) => prefs::Action::None,
                    None => self.sheet.key(&k, &mut self.prefs),
                };
                self.sheet_action(ctx, a);
                true
            }
            // the tab overview covers the content: a card selects its tab, anything else is ignored (overview.rs)
            Input::Touch { kind, x, y, .. } if self.overview.open => {
                self.hover = None;
                if kind == TouchKind::Down {
                    let (px, py) = (x / ui.pt(1.0), y / ui.pt(1.0));
                    let wpt = ctx.cfg.w as f32 / ui.pt(1.0);
                    if let Some(i) = overview::hit(self.tabs.len(), wpt, self.content_top(), px, py) {
                        self.overview.open = false;
                        self.select(i);
                    } else if self.chrome_hit(ui, ctx.cfg.w, x, y) == Some(Press::Bar(OVERVIEW)) {
                        // the toolbar's Show All Tabs button closes the grid it opened
                        self.overview.open = false;
                    }
                }
                true
            }
            Input::Touch { kind, x, y, .. } if self.lib.wants_touch(ui, y, self.fav_top()) => {
                if kind == TouchKind::Up {
                    let (top, t) = (self.fav_top(), self.clock.elapsed().as_secs_f64());
                    let act = self.lib.touch_up(ui, ctx.cfg.w, ctx.cfg.h, x, y, &mut self.store, top, t, store::now());
                    if let library::Act::Open(u) = act {
                        self.lib_open(&u);
                    }
                }
                true
            }
            Input::Key(k) if self.lib.captures_keys() && k.as_chord().is_none() => {
                let hpt = ctx.cfg.h as f32 / ui.pt(1.0);
                if let library::Act::Open(u) = self.lib.key(&k, &mut self.store, store::now(), hpt) {
                    self.lib_open(&u);
                }
                true
            }
            Input::Touch { kind, .. } if self.swallow && self.pageui.active() && matches!(kind, TouchKind::Up | TouchKind::Cancel) => {
                // the finger of a long press lifting must not tap the menu it just opened
                self.swallow = false;
                self.press = None;
                true
            }
            Input::Touch { kind, x, y, .. } if self.pageui.active() => {
                match kind {
                    TouchKind::Down => self.pageui.touch_down(ui, ctx.cfg.w, ctx.cfg.h, x, y),
                    TouchKind::Up => {
                        if let pageui::Outcome::Inspect = self.pageui.touch_up(ui, ctx.cfg.w, ctx.cfg.h, x, y, &mut self.pool) {
                            self.inspect_element();
                            self.place_web(ctx);
                        }
                    }
                    TouchKind::Move => self.pageui.touch_move(ui, ctx.cfg.w, ctx.cfg.h, x, y),
                    TouchKind::Cancel => self.pageui.touch_cancel(),
                }
                true
            }
            Input::Key(k) if self.pageui.active() => {
                if let pageui::Outcome::Inspect = self.pageui.key(&k, &mut self.pool) {
                    self.inspect_element();
                    self.place_web(ctx);
                }
                true
            }
            Input::Touch { kind, id, x, y, .. } => self.on_touch(ctx, kind, id, x, y),
            Input::Key(k) => self.on_key(ctx, k),
            Input::Command(command) => self.on_edit_command(ctx, command),
            _ => false,
        }
    }
}
