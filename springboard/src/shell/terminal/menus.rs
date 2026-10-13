//! Menus and panels: the More menu, panel input and actions.
use super::*;

impl Shell {
    pub(crate) fn terminal_panel_input(&mut self, out: &Out, t: f64) -> bool {
        if *out == Out::Named("More") {
            self.terminal_action(6, t);
            return true;
        }
        let paste = matches!(out, Out::Named("Paste")).then(|| {
            Out::Text(
                clipboard::text()
                    .unwrap_or_default()
                    .replace(['\r', '\n'], " "),
            )
        });
        let out = paste.as_ref().unwrap_or(out);
        match self.terminal_ui.panel {
            Panel::Find => {
                if let Some(field) = &mut self.terminal_ui.query {
                    let enter = field.edit(&self.ui, out, t);
                    self.terminal_find(Direction::Older, !enter);
                }
                true
            }
            Panel::Form => {
                let i = self.terminal_ui.focused;
                if matches!(out, Out::Named("Tab") | Out::Enter) {
                    self.terminal_ui.fields[i].focused = false;
                    self.terminal_ui.focused = (i + 1) % 6;
                    self.terminal_ui.fields[self.terminal_ui.focused].focused = true;
                } else {
                    self.terminal_ui.fields[i].edit(&self.ui, out, t);
                }
                true
            }
            Panel::Command => {
                if matches!(out, Out::Enter) {
                    self.terminal_run_command(t);
                } else if let Some(field) = &mut self.terminal_ui.command {
                    field.edit(&self.ui, out, t);
                }
                true
            }
            Panel::Title | Panel::Shell | Panel::Colors => {
                if matches!(out, Out::Enter) {
                    match self.terminal_ui.panel {
                        Panel::Title => self.terminal_apply_title(t),
                        Panel::Shell => self.terminal_apply_shell(t),
                        _ => self.terminal_apply_colors(t),
                    }
                } else if let Some(field) = &mut self.terminal_ui.entry {
                    field.edit(&self.ui, out, t);
                }
                true
            }
            Panel::None => false,
            _ => true,
        }
    }
    pub(super) fn terminal_panel_rect(&self) -> (f32, f32, f32, f32) {
        let bottom = if self.keyboard.shown() {
            self.kb_top()
        } else {
            self.h as f32 - super::super::ui::home_zone(&self.ui)
        };
        layout::panel(
            self.w as f32,
            self.h as f32,
            self.ui.s,
            bottom,
            self.terminal_ui.panel == Panel::Form,
            matches!(self.terminal_ui.panel, Panel::More | Panel::Help),
        )
    }

    pub(super) fn terminal_close_panel(&mut self, t: f64) {
        self.terminal_ui.panel = Panel::None;
        self.terminal_ui.pending_signal = None;
        self.terminal_ui.confirm_others = false;
        self.terminal_ui.down = None;
        self.set_keyboard(self.terminal_ui.restore_keyboard, t);
        self.resize_terminals();
        for term in self.term.iter_mut() {
            term.clear_find();
        }
        self.dirty = true;
    }

    pub(super) fn terminal_action(&mut self, i: usize, t: f64) {
        if matches!(i, 0 | 5 | 6) && matches!(self.terminal_ui.panel, Panel::None | Panel::Find) {
            self.terminal_ui.restore_keyboard = self.keyboard.shown();
        }
        if i == 6 && self.terminal_ui.panel == Panel::More {
            self.terminal_close_panel(t);
            return;
        }
        match i {
            0 => {
                let path = self.terminal_config_path();
                match termcore::sshconfig::read_file(&path) {
                    Ok(text) => {
                        self.terminal_ui.config = Config::parse(&text);
                        self.terminal_ui.config_loaded = true;
                        self.terminal_ui.message.clear();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        self.terminal_ui.config = Config::parse("");
                        self.terminal_ui.config_loaded = true;
                        self.terminal_ui.message.clear();
                    }
                    Err(e) => {
                        self.terminal_ui.config_loaded = false;
                        self.terminal_ui.message = format!("Could not read SSH config: {e}")
                    }
                }
                self.terminal_ui.panel = Panel::Connections;
                self.terminal_ui.scroll = 0.0;
                self.set_keyboard(false, t);
            }
            1 => {
                self.terminal_ui.panel = Panel::Find;
                self.terminal_ui.query.get_or_insert_with(|| {
                    TextField::new(&self.ui, "Search scrollback", 14.0, false, 256)
                });
                self.set_keyboard(true, t);
            }
            2 | 3 => {
                self.terminal_ui.settings.zoom = termcore::zoom::step(
                    self.terminal_ui.settings.zoom,
                    if i == 2 { -1 } else { 1 },
                );
                self.terminal_appearance();
            }
            4 => {
                let (theme, profile) = profiles::next_scheme(
                    self.terminal_ui.settings.theme,
                    self.terminal_ui.profile,
                    THEMES.len(),
                    profile_table::PROFILES.len() + usize::from(self.terminal_ui.custom_colors.is_some()),
                );
                self.terminal_ui.settings.theme = theme;
                self.terminal_ui.profile = profile;
                self.terminal_appearance();
            }
            5 => {
                self.terminal_ui.panel = Panel::Processes;
                self.terminal_ui.scroll = 0.0;
                self.terminal_ui.message.clear();
                self.terminal_ui.sample(t);
                self.set_keyboard(false, t);
            }
            6 => {
                self.terminal_ui.panel = Panel::More;
                self.terminal_ui.scroll = 0.0;
                self.terminal_ui.message.clear();
                self.set_keyboard(false, t);
            }
            _ => {}
        }
        self.resize_terminals();
        self.dirty = true;
    }
    pub(super) fn terminal_menu_action(&mut self, i: usize, t: f64) {
        match i {
            0 => self.terminal_action(1, t),
            1 => self.terminal_action(2, t),
            2 => self.terminal_action(3, t),
            3 => self.terminal_action(4, t),
            4 => self.terminal_action(5, t),
            5 => {
                if let Some(dir) = self.term.closed.pop() {
                    self.new_terminal_tab(Some(dir));
                    self.terminal_close_panel(t);
                } else {
                    self.terminal_ui.message = "No recently closed tabs".into();
                }
            }
            // A new tab in the folder the current shell is in.
            6 => {
                let dir = self.term.as_ref().and_then(|term| term.cwd());
                self.new_terminal_tab(dir);
                self.terminal_close_panel(t);
            }
            7 => {
                if let Some(term) = self.term.as_mut() {
                    term.select_all();
                }
                self.terminal_close_panel(t);
            }
            8 => {
                if let Some(text) = self.term.as_ref().and_then(|term| term.selected_text()) {
                    clipboard::set_text(&text);
                    self.terminal_ui.message = "Selection copied".into();
                } else {
                    self.terminal_ui.message = "Select terminal text first".into();
                }
            }
            9 => {
                self.terminal_close_panel(t);
                self.type_out(Out::Named("Paste"), t);
            }
            10 => {
                if let Some(text) = self.term.as_ref().map(|term| term.transcript()) {
                    self.terminal_ui.message = self.terminal_save_text("terminal-scrollback", &text);
                }
            }
            11 => {
                if let Some(term) = self.term.as_mut() {
                    term.clear_history();
                }
                self.terminal_close_panel(t);
            }
            // Ctrl+L: the shell redraws its prompt at the top of the screen.
            12 => {
                if let Some(term) = self.term.as_mut() {
                    term.write(b"\x0c");
                }
                self.terminal_close_panel(t);
            }
            13 => {
                if let Some(term) = self.term.as_mut() {
                    term.reset();
                }
                self.terminal_close_panel(t);
            }
            14 => self.terminal_action(0, t),
            16 => {
                self.terminal_ui.cursor = self.terminal_ui.cursor.next();
                self.terminal_cursor_changed();
            }
            17 => {
                self.terminal_ui.blink = !self.terminal_ui.blink;
                self.terminal_cursor_changed();
            }
            18 => self.terminal_toggle_secure(),
            19 => self.terminal_open_command(t),
            20 => self.terminal_split(t),
            21 => self.terminal_close_split(t),
            22 => self.terminal_save_group(),
            23 => self.terminal_open_group(t),
            item::TITLE => self.terminal_open_title(t),
            item::CLOSE_OTHERS => self.terminal_close_others(t),
            item::MOVE_LEFT => self.terminal_move_tab(true),
            item::MOVE_RIGHT => self.terminal_move_tab(false),
            item::MARK => {
                self.terminal_close_panel(t);
                self.terminal_toggle_mark();
            }
            item::PREVIOUS_MARK | item::NEXT_MARK => {
                self.terminal_close_panel(t);
                self.terminal_jump_mark(i == item::NEXT_MARK);
            }
            item::FIND_SELECTION => self.terminal_use_selection_for_find(t),
            item::PASTE_ESCAPED => self.terminal_paste_escaped(t),
            item::PASTE_SELECTION => self.terminal_paste_selection(t),
            item::SEARCH_WEB => self.terminal_search_web(t),
            item::MAN_PAGE => self.terminal_man_page(t),
            item::SAVE_SELECTION => {
                let text = self.term.as_ref().and_then(|term| term.selected_text()).filter(|text| !text.is_empty());
                self.terminal_ui.message = match text {
                    Some(text) => self.terminal_save_text("terminal-selection", &text),
                    None => "Select terminal text first".into(),
                };
            }
            item::MOUSE => self.terminal_toggle_mouse(),
            item::LOGIN_SHELL => self.terminal_open_shell_panel(t),
            item::GROUP_LAUNCH => self.terminal_toggle_group_launch(),
            item::HELP => self.terminal_open_help(t),
            item::COLORS => self.terminal_open_colors(t),
            // The first web link in the selection opens in the browser.
            15 => {
                let link = self
                    .term
                    .as_ref()
                    .and_then(|term| term.selected_text())
                    .and_then(|text| links::first_link(&text));
                self.terminal_ui.message = match link {
                    Some(url) => match self.desktop_open_url(&url, t) {
                        Ok(()) => "Opened link".into(),
                        Err(e) => format!("Could not open link: {e}"),
                    },
                    None => "No link in selection".into(),
                };
            }
            _ => {}
        }
    }
}
