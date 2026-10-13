//! The Settings sheet, the search screen, calendar archives and the Edit-menu actions (cut, copy, paste an event).
use super::*;
use std::fs;

impl Calendar {
    // ---- Settings -------------------------------------------------------------------------------------------------

    /// The Settings sheet's card: left edge and width in points.
    pub(super) fn settings_card(&self, ui: &Ui) -> (f32, f32) {
        let wp = self.w as f32 / ui.s;
        let w = SET_W.min(wp - 16.0).max(0.0);
        ((wp - w) / 2.0, w)
    }

    /// The sheet's rows as (target, top pt, height pt): the five settings, then Done. The first slot holds the title.
    pub(super) fn settings_rows() -> [(Target, f32, f32); 6] {
        let row = |t, i: f32| (t, SET_TOP + SET_ROW * (i + 1.0), SET_ROW);
        [
            row(Target::SetStep(SetRow::WeekStart, 0), 0.0),
            row(Target::SetStep(SetRow::DayStart, 0), 1.0),
            row(Target::SetStep(SetRow::Duration, 0), 2.0),
            row(Target::SetStep(SetRow::Alert, 0), 3.0),
            row(Target::SetStep(SetRow::AlertAllDay, 0), 4.0),
            row(Target::SetDone, 5.0),
        ]
    }

    /// The text of a settings row: what it sets and its value now.
    pub(super) fn settings_label(&self, row: SetRow) -> String {
        let p = &self.prefs;
        match row {
            SetRow::WeekStart => format!("Week starts on {}", WEEKDAYS[p.week_start as usize % 7]),
            SetRow::DayStart => format!("Day starts at {}", time_label(p.day_start as u16 * 60)),
            SetRow::Duration => format!("New event lasts {} min", p.duration),
            SetRow::Alert => format!("Alert: {}", store::ALERTS[p.alert as usize]),
            SetRow::AlertAllDay => format!("All-day alert: {}", store::ALERTS[p.alert_all_day as usize]),
        }
    }

    /// The sheet is modal: a press on a row's left or right edge steps it, on Done closes it, elsewhere does nothing.
    pub(super) fn hit_settings(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let (xp, yp) = (x / ui.s, y / ui.s);
        let (left, w) = self.settings_card(ui);
        if xp < left || xp > left + w {
            return None;
        }
        let (target, ..) = Self::settings_rows().into_iter().find(|(_, top, h)| (*top..*top + *h).contains(&yp))?;
        match target {
            Target::SetStep(row, _) => {
                let rel = (xp - left) / w.max(1.0);
                if rel < 0.15 {
                    Some(Target::SetStep(row, -1))
                } else if rel > 0.85 {
                    Some(Target::SetStep(row, 1))
                } else {
                    None
                }
            }
            other => Some(other),
        }
    }

    pub(super) fn step_setting(&mut self, row: SetRow, n: i32) {
        match row {
            SetRow::WeekStart => self.prefs.step_week_start(n),
            SetRow::DayStart => self.prefs.step_day_start(n),
            SetRow::Duration => self.prefs.step_duration(n),
            SetRow::Alert => self.prefs.alert = prefs::Prefs::step_alert(self.prefs.alert, n),
            SetRow::AlertAllDay => self.prefs.alert_all_day = prefs::Prefs::step_alert(self.prefs.alert_all_day, n),
        }
        self.save_prefs();
    }

    pub(super) fn draw_settings(&mut self, s: &mut Surface, ui: &Ui) {
        if !self.settings {
            return;
        }
        let (left, w) = self.settings_card(ui);
        let rows = Self::settings_rows();
        let (top, bottom) = (SET_TOP, rows[5].1 + rows[5].2);
        s.fill_rrect(ui.pt(left), ui.pt(top), ui.pt(w), ui.pt(bottom - top), ui.pt(CARD_R), CARD_BG.now());
        s.stroke_rrect(ui.pt(left), ui.pt(top), ui.pt(w), ui.pt(bottom - top), ui.pt(CARD_R), 1.0, SEPARATOR_C.now());
        let cx = ui.pt(left + w / 2.0);
        self.centre(s, ui, "Settings", MAC_BODY_PT, Weight::Semibold, LABEL.now(), cx, ui.pt(top + SET_ROW / 2.0));
        for (target, row_top, h) in rows {
            let mid = ui.pt(row_top + h / 2.0);
            s.fill_rrect(ui.pt(left + SPACE_L), ui.pt(row_top), ui.pt(w - 2.0 * SPACE_L), ui.pt(0.5).max(1.0), 0.0, SEPARATOR_C.now());
            match target {
                Target::SetStep(row, _) => {
                    let text = ui.fit(&self.settings_label(row), MAC_BODY_PT, ui.pt(w * 0.68));
                    self.centre(s, ui, "\u{2212}", MAC_TITLE2_PT, Weight::Regular, accent(), ui.pt(left + w * 0.075), mid);
                    self.centre(s, ui, &text, MAC_BODY_PT, Weight::Regular, LABEL.now(), cx, mid);
                    self.centre(s, ui, "+", MAC_TITLE2_PT, Weight::Regular, accent(), ui.pt(left + w * 0.925), mid);
                }
                _ => self.centre(s, ui, "Done", MAC_BODY_PT, Weight::Semibold, accent(), cx, mid),
            }
        }
    }

    // ---- Search ---------------------------------------------------------------------------------------------------

    /// The query field's frame in px.
    pub(super) fn query_frame(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        (ui.pt(36.0), ui.pt(SEARCH_FIELD_TOP), self.w as f32 - ui.pt(72.0), ui.pt(44.0))
    }

    /// Edit > Find: shows the search screen with the query field focused.
    pub(super) fn open_search(&mut self, ui: &Ui) {
        self.query.set_text(ui, "");
        self.query.focused = true;
        self.search = Some(Search { hits: vec![] });
        self.scroll.jump_top();
        self.dirty = true;
    }

    pub(super) fn close_search(&mut self) {
        self.search = None;
        self.query.focused = false;
        self.scroll.jump_top();
    }

    /// Looks the query up again after it changed.
    pub(super) fn refresh_hits(&mut self) {
        let hits = if self.search.is_some() { self.search(self.query.text()) } else { vec![] };
        if let Some(s) = &mut self.search {
            s.hits = hits;
        }
    }

    /// The scroll range of the result list.
    pub(super) fn search_scroll_max(&self, ui: &Ui) -> f32 {
        let n = self.search.as_ref().map_or(0, |s| s.hits.len());
        (ui.pt(SEARCH_LIST_TOP + n as f32 * SEARCH_ROW + 16.0) - self.bottom(ui)).max(0.0)
    }

    /// The query field or the result row under the point (px, column coordinates).
    pub(super) fn hit_search(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let _ = x;
        let yp = y / ui.s;
        if (SEARCH_FIELD_TOP..SEARCH_FIELD_TOP + 44.0).contains(&yp) {
            return Some(Target::SearchField);
        }
        if yp < SEARCH_LIST_TOP {
            return None;
        }
        let row = ((yp - SEARCH_LIST_TOP + self.scroll.offset() / ui.s) / SEARCH_ROW).floor();
        let hits = self.search.as_ref()?.hits.len();
        (row >= 0.0 && (row as usize) < hits).then_some(Target::SearchHit(row as usize))
    }

    /// Shows the day of result `i` and opens its event when the store holds it.
    pub(super) fn open_hit(&mut self, ui: &Ui, i: usize) {
        let Some(hit) = self.search.as_ref().and_then(|s| s.hits.get(i)).cloned() else { return };
        self.close_search();
        self.selected = hit.date;
        let id = self.events.iter().find(|e| e.date == hit.date && e.minute == hit.minute && e.title == hit.title).map(|e| e.id);
        if let Some(id) = id {
            self.open(ui, Some(id));
        }
    }

    /// A key while the search screen is up: Return opens the first result, other keys edit the query.
    pub(super) fn search_key(&mut self, ui: &Ui, out: Out, t: f64) {
        match out {
            Out::Enter => self.open_hit(ui, 0),
            Out::Hide => self.query.focused = false,
            out => {
                self.query.focused = true;
                if self.query.edit(ui, &out, t) {
                    self.refresh_hits();
                    self.scroll.jump_top();
                }
            }
        }
        self.fit_scroll(ui);
        self.dirty = true;
    }

    pub(super) fn draw_search(&mut self, s: &mut Surface, ui: &Ui) {
        let Some(search) = &self.search else { return };
        let hits = search.hits.clone();
        let w = self.w as f32;
        // the query field, drawn with the kit's text field look
        let (x, y, fw, fh) = self.query_frame(ui);
        let mut field = bifrost_app::controls::TextField::new();
        field.push_str(self.query.text());
        field.home();
        for _ in 0..self.query.text().get(..self.query.caret()).map_or(self.query.caret(), |p| p.chars().count()) {
            field.move_caret(true);
        }
        let k = ui.pt(1.0);
        let r = bifrost_app::controls::Rect { x: x / k, y: y / k + SPACE_S, w: fw / k, h: fh / k - 2.0 * SPACE_S };
        bifrost_app::widgets::text_field(s, ui, &field, r, "Search events", self.query.focused);
        if hits.is_empty() {
            if !self.query.text().trim().is_empty() {
                self.centre(s, ui, "No results", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), w / 2.0, ui.pt(SEARCH_LIST_TOP + 30.0));
            }
            return;
        }
        let bottom = self.bottom(ui);
        let off = self.scroll.offset();
        let mut g = s.rows(ui.pt(SEARCH_LIST_TOP) as usize, bottom as usize);
        for (i, hit) in hits.iter().enumerate() {
            let top = ui.pt(SEARCH_LIST_TOP + SEARCH_ROW * i as f32) - off;
            if top + ui.pt(SEARCH_ROW) < ui.pt(SEARCH_LIST_TOP) || top > bottom {
                continue;
            }
            let when = if hit.all_day {
                format!("{} {} {}", hit.date.day, &MONTHS[(hit.date.month - 1) as usize][..3], hit.date.year)
            } else {
                format!("{} {} {}  {}", hit.date.day, &MONTHS[(hit.date.month - 1) as usize][..3], hit.date.year, time_label(hit.minute))
            };
            let mid = top + ui.pt(SEARCH_ROW / 2.0);
            let right_edge = w - ui.pt(36.0);
            let title_w = (right_edge - ui.pt(36.0) - ui.pt(170.0)).max(ui.pt(60.0));
            let title = ui.fit(&hit.title, MAC_BODY_PT, title_w);
            if hit.location.is_empty() {
                self.text(&mut g, ui, &title, MAC_BODY_PT, LABEL.now(), ui.pt(36.0), mid);
            } else {
                self.text(&mut g, ui, &title, MAC_BODY_PT, LABEL.now(), ui.pt(36.0), mid - ui.pt(8.0));
                let place = ui.fit(&hit.location, MAC_SUBHEAD_PT, title_w);
                self.text(&mut g, ui, &place, MAC_SUBHEAD_PT, LABEL_2.now(), ui.pt(36.0), mid + ui.pt(9.0));
            }
            self.right(&mut g, ui, &when, LABEL_2.now(), right_edge, mid);
            g.fill_rrect(ui.pt(36.0), top + ui.pt(SEARCH_ROW), w - ui.pt(72.0), ui.pt(0.5).max(1.0), 0.0, SEPARATOR_C.now());
        }
    }

    // ---- Archive --------------------------------------------------------------------------------------------------

    /// The whole calendar as an archive: the event store and the rich sidecar in one file (`archive.rs`).
    pub fn export_archive(&self) -> String {
        let events = fs::read_to_string(&self.path).unwrap_or_default();
        let ics = fs::read_to_string(exchange::rich_path(&self.path)).unwrap_or_default();
        archive::pack(&events, &ics)
    }

    /// File > Export Calendar Archive: writes Calendar Archive.bfcal next to the other exports.
    pub(super) fn export_archive_file(&mut self) {
        self.notice = None;
        let text = self.export_archive();
        self.save_export_as("Calendar Archive", archive::EXTENSION, &text);
    }

    /// File > Import Calendar Archive: replaces the calendar with the archive's contents and says what came of it.
    /// The calendar as it was is kept as events.tsv.bak beside the store.
    pub fn restore_archive(&mut self, name: &str, text: &str) {
        self.notice = None;
        self.error = None;
        self.dirty = true;
        match self.try_restore(text) {
            Ok(1) => self.notice = Some(format!("Restored 1 event from {name}")),
            Ok(n) => self.notice = Some(format!("Restored {n} events from {name}")),
            Err(e) => self.error = Some(e),
        }
    }

    fn try_restore(&mut self, text: &str) -> Result<usize, String> {
        if self.storage_blocked {
            return Err("Saved events could not be read. Restore is unavailable.".into());
        }
        let (events_text, ics_text) = archive::unpack(text)?;
        let previous = fs::read_to_string(&self.path).ok();
        if let Some(old) = &previous {
            let _ = springboard::shell::fsutil::write_atomic(&self.path.with_extension("tsv.bak"), old);
        }
        let loaded = if events_text.trim().is_empty() {
            let _ = fs::remove_file(&self.path);
            vec![]
        } else {
            springboard::shell::fsutil::write_atomic(&self.path, &events_text).map_err(|e| e.to_string())?;
            match store::load(&self.path) {
                Ok(events) => events,
                Err(e) => {
                    // put the calendar back as it was
                    match &previous {
                        Some(old) => {
                            let _ = springboard::shell::fsutil::write_atomic(&self.path, old);
                        }
                        None => {
                            let _ = fs::remove_file(&self.path);
                        }
                    }
                    return Err(format!("The archive holds no readable events ({e})"));
                }
            }
        };
        let rich = exchange::rich_path(&self.path);
        if ics_text.trim().is_empty() {
            let _ = fs::remove_file(&rich);
        } else {
            springboard::shell::fsutil::write_atomic(&rich, &ics_text).map_err(|e| e.to_string())?;
        }
        self.events = loaded;
        self.undo = None;
        self.changes.post();
        Ok(self.events.len())
    }

    // ---- Edit menu ------------------------------------------------------------------------------------------------

    /// Edit > Copy and Cut act on the saved event being edited, while no text field has the focus.
    pub fn can_copy_event(&self) -> bool {
        self.editor.as_ref().is_some_and(|d| d.existing) && self.focused().is_none()
    }

    /// Edit > Paste adds the pasteboard's event to the selected day, from the calendar views.
    pub fn can_paste_event(&self) -> bool {
        self.editor.is_none() && self.goto.is_none() && !self.settings && self.search.is_none() && !self.storage_blocked
    }

    /// Event > Cut Event: copies the saved event being edited and deletes it (Undo brings it back).
    pub fn cut_event(&mut self, ui: &Ui, t: f64) -> bool {
        if !self.copy_event() {
            return false;
        }
        self.commit(ui, true, t);
        if self.editor.is_none() {
            self.notice = Some("Cut event".into());
        }
        true
    }

    // ---- Navigation for intents and control methods ---------------------------------------------------------------

    /// Shows `date` in the current view (the `show-date` intent).
    pub fn go_to(&mut self, date: Date) {
        self.selected = date;
        self.scroll.jump_top();
        self.dirty = true;
    }

    /// Re-reads the event store after another process (an intent) changed it.
    pub fn reload(&mut self) {
        if let Ok(events) = store::load(&self.path) {
            self.events = events;
            self.changes.post();
            self.dirty = true;
        }
    }
}
