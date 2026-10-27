//! Keyboard shortcuts, editing and page key input.
use super::*;

impl Browser {
    pub(super) fn on_edit_command(&mut self, ctx: &Ctx, command: bifrost_app::controls::Command) -> bool {
        use bifrost_app::controls::Command;
        if self.sheet.open || self.overview.open || self.pageui.active() || self.lib.captures_keys() {
            return false;
        }
        let clipboard = ctx.pasteboard_string().unwrap_or_default();
        if self.editing {
            if let Some(text) = self.address.command(command, &clipboard) {
                ctx.set_pasteboard_string(&text);
            }
            self.refresh_suggestions();
            return true;
        }
        let (key, modifiers) = match command {
            Command::Cut => (45, CHORD_CTRL),
            Command::Copy => (46, CHORD_CTRL),
            Command::Paste => (47, CHORD_CTRL),
            Command::SelectAll => (30, CHORD_CTRL),
            Command::Undo => (44, CHORD_CTRL),
            Command::Redo => (44, CHORD_CTRL | CHORD_SHIFT),
        };
        if let Some(pool) = &mut self.pool {
            pool.set_clipboard(&clipboard);
            pool.key(&KeyInput::chord(key, modifiers));
        }
        false
    }

    pub(super) fn on_key(&mut self, ctx: &Ctx, k: KeyInput) -> bool {
        let view = self.view();
        if let Some((KEY_TAB, mods)) = k.as_chord() {
            if mods & CHORD_CTRL != 0 {
                self.cycle_tab(mods & CHORD_SHIFT == 0);
                self.place_web(ctx);
                return true;
            }
        }
        if let Some(redraw) = self.inspector.key(&k, self.pool.as_mut(), view) {
            if self.inspector.take_relayout() {
                self.place_web(ctx);
            }
            return redraw;
        }
        if let Some((key, mods)) = k.as_chord() {
            if key == KEY_R && mods & (CHORD_CTRL | CHORD_META) != 0 && mods & CHORD_SHIFT != 0 && mods & CHORD_ALT == 0 {
                self.toggle_reader();
                return true;
            }
            if self.dev_key(key, mods) {
                self.place_web(ctx);
                return true;
            }
            if self.system_chord(ctx, key, mods) {
                return true;
            }
            if self.lib_chord(ctx, key, mods) {
                return true;
            }
            if self.editing && mods & (CHORD_CTRL | CHORD_META) != 0 && mods & CHORD_ALT == 0 {
                use bifrost_app::controls::Command as Edit;
                let command = match (key, mods & CHORD_SHIFT != 0) {
                    (30, false) => Some(Edit::SelectAll),
                    (45, false) => Some(Edit::Cut),
                    (46, false) => Some(Edit::Copy),
                    (47, false) => Some(Edit::Paste),
                    (44, false) => Some(Edit::Undo),
                    (44, true) => Some(Edit::Redo),
                    _ => None,
                };
                if let Some(command) = command {
                    return self.on_edit_command(ctx, command);
                }
            }
            let cmd = mods & (CHORD_CTRL | CHORD_META) != 0 && mods & CHORD_SHIFT == 0;
            let shift = mods & CHORD_SHIFT != 0;
            let before = (self.strip_shown(), self.find.open);
            if mods & (CHORD_CTRL | CHORD_META) != 0 {
                let used = match (key, shift) {
                    (KEY_F, false) => {
                        self.editing = false;
                        self.show_find();
                        true
                    }
                    (KEY_G, s) if self.find.open => {
                        self.find_step(if s { FindDirection::Backward } else { FindDirection::Forward });
                        true
                    }
                    (KEY_T, true) => {
                        self.reopen_closed();
                        true
                    }
                    (KEY_N, true) => {
                        self.open_private_window();
                        true
                    }
                    (KEY_LEFTBRACE, true) => {
                        self.cycle_tab(false);
                        true
                    }
                    (KEY_RIGHTBRACE, true) => {
                        self.cycle_tab(true);
                        true
                    }
                    (KEY_D, false) if mods & CHORD_ALT != 0 => {
                        self.duplicate_tab();
                        true
                    }
                    (KEY_1..=KEY_9, false) if mods & CHORD_ALT == 0 => {
                        self.select_numbered((key - KEY_1 + 1) as usize);
                        true
                    }
                    _ => false,
                };
                if used {
                    if before != (self.strip_shown(), self.find.open) {
                        self.place_web(ctx);
                    }
                    return true;
                }
            }
            let handled = cmd && match key {
                KEY_COMMA => {
                    self.editing = false;
                    self.sheet.show(&self.prefs);
                    true
                }
                KEY_L => {
                    self.begin_editing();
                    true
                }
                KEY_T => {
                    self.activate(NEW_TAB);
                    true
                }
                KEY_W => {
                    self.close_tab(self.current);
                    true
                }
                KEY_R => {
                    self.activate(RELOAD);
                    true
                }
                _ => false,
            };
            if before != (self.strip_shown(), self.find.open) {
                self.place_web(ctx);
            }
            if handled {
                return true;
            }
        }
        if self.find.open && !self.editing {
            let was = self.find.open;
            let used = self.find_key(&k);
            if was != self.find.open {
                self.place_web(ctx);
            }
            if used {
                return true;
            }
        }
        if !self.editing && matches!(k, KeyInput::Code(1 | 27)) {
            if let (Some(v), Some(p)) = (self.view(), self.pool.as_mut()) {
                if self.reader.exit(p, v) {
                    return true;
                }
            }
        }
        if self.downloads.open && matches!(k, KeyInput::Code(1 | 27)) {
            self.downloads.open = false;
            return true;
        }
        if self.overview.open && matches!(k, KeyInput::Code(1 | 27)) {
            self.overview.open = false;
            return true;
        }
        if self.editing {
            match k {
                KeyInput::Text(s) => {
                    self.address.push_str(&s);
                    self.refresh_suggestions();
                }
                KeyInput::Code(8) => {
                    self.address.pop();
                    self.refresh_suggestions();
                }
                KeyInput::Code(46) => {
                    self.address.delete();
                    self.refresh_suggestions();
                }
                KeyInput::Code(40) => {
                    let n = self.sugg.len();
                    self.sel = if n == 0 { None } else { Some(self.sel.map_or(0, |i| (i + 1) % n)) };
                }
                KeyInput::Code(38) => {
                    let n = self.sugg.len();
                    self.sel = if n == 0 { None } else { Some(self.sel.map_or(n - 1, |i| (i + n - 1) % n)) };
                }
                KeyInput::Code(13) => self.commit_address(),
                KeyInput::Code(1) | KeyInput::Code(27) => {
                    self.editing = false;
                    self.sugg.clear();
                    self.sync_address();
                }
                KeyInput::Code(37) => self.address.move_caret(false),
                KeyInput::Code(39) => self.address.move_caret(true),
                _ => return false,
            }
            return true;
        }
        if let Some(p) = &mut self.pool {
            p.set_clipboard(ctx.pasteboard_string().as_deref().unwrap_or(""));
            p.key(&k);
        }
        false
    }
}
