//! Window controller: the chrome tap routing (tab bar, header, panels).
use super::*;

impl Shell {
    pub(super) fn terminal_chrome_tap(&mut self, x: f32, y: f32, t: f64) {
        self.term_touch = termtouch::TermTouch::default();
        let top = self.ui.pt(scene::STATUS_PT);
        let body_top = top + self.ui.pt(HEADER_PT);
        let width = self.w as f32;
        let panel = self.terminal_ui.panel;
        let pending_close = self.terminal_ui.pending_close.take();
        if matches!(panel, Panel::None | Panel::Find) && y < body_top {
            use layout::HeaderHit;
            let g = layout::Header::new(width, self.ui.s, self.term.len());
            match g.hit(x, width) {
                Some(HeaderHit::New) => {
                    self.new_terminal_tab(None);
                }
                Some(HeaderHit::Connections) => self.terminal_action(0, t),
                Some(HeaderHit::Processes) => self.terminal_action(5, t),
                Some(HeaderHit::Menu) => self.terminal_action(6, t),
                Some(HeaderHit::Tab(i)) => {
                    self.term.select(i);
                    self.terminal_ui.message.clear();
                }
                // A tab running a job asks first, as Terminal does: a second tap on × closes it.
                Some(HeaderHit::Close(i)) => {
                    let job = self.term.iter().nth(i).and_then(|tab| tab.job());
                    match job {
                        Some(job) if pending_close != Some(i) => {
                            self.terminal_ui.pending_close = Some(i);
                            self.terminal_ui.message = format!("{job} is running. Tap × again to close the tab");
                        }
                        _ => self.close_terminal_tab(i),
                    }
                }
                None => {}
            }
            return;
        }
        if panel == Panel::Find {
            if x >= width - self.ui.pt(44.0) {
                self.terminal_ui.message.clear();
                self.terminal_close_panel(t);
            } else if x >= width - self.ui.pt(88.0) {
                self.terminal_find(Direction::Newer, false);
            } else if x >= width - self.ui.pt(132.0) {
                self.terminal_find(Direction::Older, false);
            } else if let Some(field) = &mut self.terminal_ui.query {
                field.tap(&self.ui, x - self.ui.pt(34.0), t);
                self.set_keyboard(true, t);
            }
            return;
        }
        let (px, py, pw, ph) = self.terminal_panel_rect();
        if x < px || x >= px + pw || y < py || y >= py + ph {
            self.terminal_close_panel(t);
            return;
        }
        let (lx, ly, w, h) = (
            (x - px) / self.ui.s,
            (y - py) / self.ui.s,
            pw / self.ui.s,
            ph / self.ui.s,
        );
        match panel {
            Panel::Connections => {
                use termcore::connui::ListHit;
                match termcore::connui::list_hit(
                    w as f64,
                    h as f64,
                    lx as f64,
                    ly as f64,
                    (self.terminal_ui.scroll / self.ui.s) as f64,
                    self.terminal_ui.config.entries().len(),
                    self.terminal_ui.config_loaded,
                ) {
                    ListHit::Close => self.terminal_close_panel(t),
                    ListHit::New if self.terminal_ui.config_loaded => {
                        self.terminal_ui.form(&self.ui, Entry::default(), false);
                        self.set_keyboard(true, t);
                    }
                    ListHit::Import => self.terminal_import(),
                    ListHit::Row(i) => {
                        if let Some(e) = self.terminal_ui.config.entries().get(i).cloned() {
                            self.terminal_connect(&e.alias, t);
                        }
                    }
                    ListHit::Edit(i) if self.terminal_ui.config_loaded => {
                        if let Some(e) = self.terminal_ui.config.entries().get(i).cloned() {
                            self.terminal_ui.form(&self.ui, e, true);
                            self.set_keyboard(true, t);
                        }
                    }
                    _ => {}
                }
            }
            Panel::Form => {
                use termcore::connui::FormHit;
                match termcore::connui::form_hit(
                    w as f64,
                    lx as f64,
                    ly as f64,
                    self.terminal_ui.editing.is_some(),
                ) {
                    FormHit::Cancel => {
                        self.terminal_ui.panel = Panel::Connections;
                        self.set_keyboard(false, t);
                    }
                    FormHit::Save => self.terminal_save_form(t),
                    FormHit::Delete => {
                        if !self.terminal_ui.delete_confirm {
                            self.terminal_ui.delete_confirm = true;
                            self.terminal_ui.message =
                                "Tap Delete again to remove this connection".into();
                        } else if let Some(alias) = self.terminal_ui.editing.clone() {
                            let mut config = self.terminal_ui.config.clone();
                            match config
                                .remove(&alias)
                                .map_err(|e| e.to_string())
                                .and_then(|_| {
                                    self.save_terminal_config(&config)
                                        .map_err(|e| e.to_string())
                                }) {
                                Ok(()) => {
                                    self.terminal_ui.config = config;
                                    self.terminal_ui.panel = Panel::Connections;
                                    self.terminal_ui.message = "Connection deleted".into();
                                    self.set_keyboard(false, t);
                                }
                                Err(e) => self.terminal_ui.message = e,
                            }
                        }
                    }
                    FormHit::None => {
                        for i in 0..6 {
                            let (fx, fy, fw, fh) = termcore::connui::field_rect(w as f64, i);
                            if lx as f64 >= fx
                                && (lx as f64) < fx + fw
                                && ly as f64 >= fy
                                && (ly as f64) < fy + fh
                            {
                                self.terminal_ui.fields[self.terminal_ui.focused].focused = false;
                                self.terminal_ui.focused = i;
                                self.terminal_ui.fields[i].tap(
                                    &self.ui,
                                    self.ui.pt(lx - fx as f32 - 8.0),
                                    t,
                                );
                                self.terminal_ui.fields[i].focused = true;
                                self.set_keyboard(true, t);
                                break;
                            }
                        }
                    }
                }
            }
            Panel::Processes => {
                if ly < 44.0 {
                    if lx >= w - 44.0 {
                        self.terminal_close_panel(t);
                    } else if lx >= 218.0 && lx < 300.0 {
                        self.terminal_ui.cpu_sort = true;
                        self.terminal_ui.scroll = 0.0;
                    } else if lx >= 136.0 && lx < 218.0 {
                        self.terminal_ui.cpu_sort = false;
                        self.terminal_ui.scroll = 0.0;
                    }
                } else if ly >= h - 56.0 {
                    if let Some(p) = self
                        .terminal_ui
                        .processes
                        .iter()
                        .find(|p| Some((p.info.pid, p.start)) == self.terminal_ui.selected)
                        .cloned()
                    {
                        let force = lx >= w - 120.0;
                        if lx < w - 240.0
                            || !termcore::procs::killable(p.info.pid, std::process::id())
                        {
                            return;
                        }
                        if self.terminal_ui.pending_signal != Some(force) {
                            self.terminal_ui.pending_signal = Some(force);
                            self.terminal_ui.message = format!(
                                "{} {}? Tap again to confirm",
                                if force { "Force kill" } else { "Terminate" },
                                p.info.name
                            );
                        } else {
                            self.terminal_ui.message = match process::signal(&p, force) {
                                Ok(()) => format!(
                                    "{} sent to {} ({})",
                                    if force { "SIGKILL" } else { "SIGTERM" },
                                    p.info.name,
                                    p.info.pid
                                ),
                                Err(e) => format!("Could not signal: {e}"),
                            };
                            self.terminal_ui.pending_signal = None;
                            self.terminal_ui.sample(t);
                        }
                    }
                } else {
                    let i = ((ly - 44.0 + self.terminal_ui.scroll / self.ui.s) / 40.0) as usize;
                    if let Some(&index) = self.terminal_ui.order().get(i) {
                        let p = &self.terminal_ui.processes[index];
                        self.terminal_ui.selected = Some((p.info.pid, p.start));
                        self.terminal_ui.pending_signal = None;
                        self.terminal_ui.message.clear();
                    }
                }
            }
            Panel::Command => {
                if ly < 44.0 && lx >= w - 44.0 {
                    self.terminal_close_panel(t);
                } else if ly >= h - 56.0 && lx >= w - 120.0 {
                    self.terminal_run_command(t);
                } else if ly >= 44.0 && ly < h - 56.0 {
                    self.set_keyboard(true, t);
                }
            }
            Panel::Look => {
                if ly < 44.0 && lx >= w - 44.0 {
                    self.terminal_close_panel(t);
                } else if ly >= 44.0 {
                    let row = ((ly - 44.0 + self.terminal_ui.scroll / self.ui.s) / 44.0) as usize;
                    self.terminal_look_tap(row, lx, w);
                }
            }
            Panel::More => {
                if ly < 44.0 && lx >= w - 80.0 {
                    self.terminal_close_panel(t);
                } else if ly >= 44.0 && ly < h - 56.0 {
                    let i = ((ly - 44.0 + self.terminal_ui.scroll / self.ui.s) / 44.0) as usize;
                    self.terminal_menu_action(i, t);
                }
            }
            _ => {}
        }
    }

}
