//! Keys and touch: key equivalents, the modal gesture.
use super::*;

impl Shell {

    /// Term key equivalents while Term is in front (see `hwkeys::TermKey`).
    pub(crate) fn terminal_hw_key(&mut self, key: TermKey, t: f64) -> Vec<super::super::Effect> {
        let modal = !matches!(self.terminal_ui.panel, Panel::None | Panel::Find);
        let pending_close = self.terminal_ui.pending_close.take();
        if modal && !matches!(key, TermKey::Find) {
            return vec![];
        }
        let mut fx = vec![];
        match key {
            // Cmd+C never sends ^C: with nothing selected it does nothing
            TermKey::Copy => {
                if let Some(text) = self.term.as_ref().and_then(|term| term.selected_text()) {
                    clipboard::set_text(&text);
                }
            }
            TermKey::SelectAll => {
                if let Some(term) = self.term.as_mut() {
                    term.select_all();
                }
            }
            TermKey::Find => {
                if modal {
                    self.terminal_close_panel(t);
                }
                self.terminal_action(1, t);
            }
            TermKey::FindNext | TermKey::FindPrev => {
                let has_query = self.terminal_ui.query.as_ref().is_some_and(|q| !q.text().is_empty());
                if has_query {
                    self.terminal_find(if key == TermKey::FindNext { Direction::Newer } else { Direction::Older }, false);
                } else {
                    self.terminal_action(1, t);
                }
            }
            TermKey::ClearScrollback => {
                if let Some(term) = self.term.as_mut() {
                    term.clear_history();
                }
            }
            TermKey::Bigger => self.terminal_action(3, t),
            TermKey::Smaller => self.terminal_action(2, t),
            TermKey::ActualSize => {
                self.terminal_ui.settings.zoom = termcore::zoom::DEFAULT;
                self.terminal_appearance();
            }
            TermKey::NewTab => {
                self.new_terminal_tab(None);
            }
            TermKey::CloseTab => {
                let i = self.term.active_index();
                let job = self.term.as_ref().and_then(|tab| tab.job());
                match job {
                    Some(job) if pending_close != Some(i) => {
                        self.terminal_ui.pending_close = Some(i);
                        self.terminal_ui.message = format!("{job} is running. Press Cmd+W again to close the tab");
                    }
                    _ if self.term.len() <= 1 => {
                        #[cfg(feature = "desktop-wayland")]
                        if self.desk_active() {
                            fx = self.desk_window_close_or_minimize(false);
                        } else {
                            self.close_terminal_tab(i);
                        }
                        #[cfg(not(feature = "desktop-wayland"))]
                        self.close_terminal_tab(i);
                    }
                    _ => self.close_terminal_tab(i),
                }
            }
            TermKey::NextTab | TermKey::PrevTab => {
                let n = self.term.len();
                if n > 1 {
                    let at = self.term.active_index();
                    self.term.select(if key == TermKey::NextTab { (at + 1) % n } else { (at + n - 1) % n });
                    self.terminal_ui.message.clear();
                }
            }
            TermKey::Tab(i) => {
                if (i as usize) < self.term.len() {
                    self.term.select(i as usize);
                    self.terminal_ui.message.clear();
                }
            }
        }
        self.dirty = true;
        fx
    }

    // The whole modal gesture belongs to the panel, including outside dismissal.
    pub(crate) fn terminal_chrome_touch(&mut self, ev: Event, t: f64) -> bool {
        let top = self.ui.pt(scene::STATUS_PT + HEADER_PT);
        let bottom = if self.keyboard.shown() {
            self.kb_top()
        } else {
            self.h as f32 - super::super::ui::home_zone(&self.ui)
        };
        let modal = matches!(
            self.terminal_ui.panel,
            Panel::Connections | Panel::Processes | Panel::Form | Panel::More
        );
        match ev {
            Event::Down(x, y)
                if y < top
                    || (modal && y < bottom)
                    || (self.terminal_ui.panel == Panel::Find && y < top + self.ui.pt(ROW)) =>
            {
                self.terminal_ui.down = Some((x, y));
                self.terminal_ui.last_y = y;
                self.terminal_ui.dragged = false;
                true
            }
            Event::Move(x, y) if self.terminal_ui.down.is_some() => {
                let (sx, sy) = self.terminal_ui.down.unwrap();
                if (x - sx).hypot(y - sy) > self.ui.pt(6.0) {
                    self.terminal_ui.dragged = true;
                }
                let (px, py, pw, ph) = self.terminal_panel_rect();
                if self.terminal_ui.dragged
                    && sx >= px
                    && sx < px + pw
                    && sy >= py + self.ui.pt(44.0)
                    && sy < py + ph - self.ui.pt(56.0)
                {
                    let (count, pitch) = match self.terminal_ui.panel {
                        Panel::Connections => (self.terminal_ui.config.entries().len(), 52.0),
                        Panel::Processes => (self.terminal_ui.processes.len(), 40.0),
                        Panel::More => (MENU.len(), 44.0),
                        _ => (0, 0.0),
                    };
                    let limit =
                        (count as f32 * self.ui.pt(pitch) - (ph - self.ui.pt(100.0))).max(0.0);
                    self.terminal_ui.scroll =
                        (self.terminal_ui.scroll + self.terminal_ui.last_y - y).clamp(0.0, limit);
                }
                self.terminal_ui.last_y = y;
                true
            }
            Event::Up(x, y) if self.terminal_ui.down.is_some() => {
                let (sx, sy) = self.terminal_ui.down.take().unwrap();
                if !self.terminal_ui.dragged && (x - sx).hypot(y - sy) < self.ui.pt(12.0) {
                    let old = self.term_area();
                    self.terminal_chrome_tap(x, y, t);
                    let area = self.term_area();
                    if area != old {
                        for term in self.term.iter_mut() {
                            term.resize(area);
                        }
                    }
                }
                true
            }
            _ => false,
        }
    }
}
