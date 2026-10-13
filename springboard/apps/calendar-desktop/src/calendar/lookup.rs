//! Searching events: the toolbar's search field and the results list that replaces the grid while the field holds a
//! query. The matching itself is `search.rs`; a result opens its event on its day.
use super::search::Hit;
use super::*;
use bifrost_app::controls::{Node, Rect, Role};

/// One result row, in points.
const ROW_PT: f32 = 52.0;

/// "Mon, 5 Oct 2026  ·  10:00  ·  Room 4": when a result is, and where.
pub fn hit_detail(h: &Hit) -> String {
    let d = h.date;
    let mut text = format!("{}, {} {} {}", &WEEKDAYS[d.weekday()][..3], d.day, &MONTHS[(d.month - 1) as usize][..3], d.year);
    text.push_str(&if h.all_day { "  \u{b7}  all-day".to_string() } else { format!("  \u{b7}  {}", time_label(h.minute)) });
    if !h.location.is_empty() {
        text.push_str("  \u{b7}  ");
        text.push_str(&h.location);
    }
    text
}

/// The header of the results: how many events matched.
pub fn count_label(n: usize) -> String {
    match n {
        0 => "No results".to_string(),
        1 => "1 result".to_string(),
        n => format!("{n} results"),
    }
}

impl Calendar {
    /// Whether the results list is showing: the field holds a query and no sheet is open.
    pub fn searching(&self) -> bool {
        !self.search_box.is_empty() && self.editor.is_none() && self.goto.is_none()
    }

    /// The matches for the query in the field.
    pub fn search_hits(&self) -> &[Hit] {
        &self.search_hits
    }

    /// The query in the field.
    pub fn search_query(&self) -> &str {
        &self.search_box
    }

    /// Recomputes the matches after the query changed, and scrolls the list back to its top.
    fn refresh_search(&mut self, ui: &Ui) {
        self.search_hits = if self.search_box.is_empty() { vec![] } else { self.search(&self.search_box) };
        self.scroll.jump_top();
        self.fit_scroll(ui);
        self.dirty = true;
    }

    /// Puts the cursor in the search field (Cmd+F, a click on it).
    pub(super) fn focus_search(&mut self) {
        self.search_focused = true;
    }

    /// Empties the field and shows the grid again.
    pub(super) fn end_search(&mut self, ui: &Ui) {
        self.search_box.clear();
        self.search_hits.clear();
        self.search_focused = false;
        self.scroll.jump_top();
        self.fit_scroll(ui);
    }

    /// The clear button in the field: empties it and keeps the cursor there.
    pub(super) fn clear_search(&mut self, ui: &Ui) {
        self.end_search(ui);
        self.search_focused = true;
    }

    /// A key typed while the search field has the cursor. Return and Hide leave the field; the results stay.
    pub(super) fn search_key(&mut self, ui: &Ui, out: Out) {
        match out {
            Out::Text(text) => self.search_box.push_str(&text),
            Out::Backspace => self.search_box.pop(),
            Out::Named("Left") => self.search_box.move_caret(false),
            Out::Named("Right") => self.search_box.move_caret(true),
            Out::Enter | Out::Hide => self.search_focused = false,
            _ => return,
        }
        self.refresh_search(ui);
    }

    /// Text pasted into the field (Edit > Paste while it has the cursor): the first line.
    pub(super) fn search_paste(&mut self, ui: &Ui, text: &str) {
        self.search_box.push_str(text.lines().next().unwrap_or(""));
        self.refresh_search(ui);
    }

    /// Opens result `i`: shows its day and, when the event is saved, opens it.
    pub(super) fn open_hit(&mut self, ui: &Ui, i: usize) {
        let Some(h) = self.search_hits.get(i).cloned() else { return };
        self.end_search(ui);
        self.selected = h.date;
        let saved = self.events.iter().find(|e| e.date == h.date && e.minute == h.minute && e.all_day == h.all_day && e.title == h.title).map(|e| e.id);
        if let Some(id) = saved {
            self.open(ui, Some(id));
        }
    }

    /// How far the results list scrolls, in px.
    pub(super) fn results_scroll_max(&self, ui: &Ui) -> f32 {
        ui.pt(GRID_TOP + ROW_PT * self.search_hits.len() as f32) - self.bottom(ui)
    }

    /// The result row under content pixel `(x, y)`.
    pub(super) fn hit_results(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let top = ui.pt(GRID_TOP);
        if x < 0.0 || y < top {
            return None;
        }
        let i = ((y - top + self.scroll.offset()) / ui.pt(ROW_PT)).floor() as usize;
        (i < self.search_hits.len()).then_some(Target::SearchHit(i))
    }

    /// The results: the header, then one row per match (title, then when and where), or "No results".
    pub(super) fn draw_results(&mut self, s: &mut Surface, ui: &Ui) {
        let w = self.w as f32;
        let top = ui.pt(GRID_TOP) - self.scroll.offset();
        let hits = self.search_hits.clone();
        {
            let mut g = s.rows(ui.pt(GRID_TOP) as usize, self.bottom(ui) as usize);
            if hits.is_empty() {
                self.centre(&mut g, ui, "No results", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), w / 2.0, ui.pt(GRID_TOP + 40.0));
            }
            for (i, h) in hits.iter().enumerate() {
                let y = top + ui.pt(ROW_PT) * i as f32;
                let room = w - ui.pt(SPACE_HUGE);
                let title = ui.fit(&h.title, MAC_BODY_PT, room);
                self.text(&mut g, ui, &title, MAC_BODY_PT, LABEL.now(), ui.pt(SPACE_XL), y + ui.pt(ROW_PT * 0.34));
                let detail = ui.fit(&hit_detail(h), MAC_SUBHEAD_PT, room);
                self.text(&mut g, ui, &detail, MAC_SUBHEAD_PT, LABEL_2.now(), ui.pt(SPACE_XL), y + ui.pt(ROW_PT * 0.7));
                g.fill_rrect(ui.pt(SPACE_XL), y + ui.pt(ROW_PT) - ui.pt(0.5).max(1.0), w - ui.pt(SPACE_HUGE), ui.pt(0.5).max(1.0), 0.0, SEPARATOR_C.now());
            }
        }
        // the large title: "Search" and what was found
        let cy = ui.pt(chrome::HEADER_MID_PT);
        let a = self.labels.get(ui, "Search", MAC_LARGE_TITLE_PT, Weight::Bold, LABEL.now()).clone();
        s.blit(&a, ui.pt(SPACE_XL) as i32, (cy - a.h as f32 / 2.0) as i32, 255, 255);
        let count = format!(" {}", count_label(hits.len()));
        self.text(s, ui, &count, MAC_LARGE_TITLE_PT, LABEL_2.now(), ui.pt(SPACE_XL) + a.w as f32, cy);
    }

    /// The results as accessible buttons, one per row that is on screen, and the field itself.
    pub(super) fn search_nodes(&self, ui: &Ui) -> Vec<Node> {
        let k = ui.pt(1.0);
        let mut out = vec![];
        if let Some(r) = self.search_rect(ui) {
            out.push(Node { id: 240, role: Role::TextField, label: "Search".into(), value: self.search_box.to_string(), bounds: r, enabled: true });
        }
        if !self.searching() {
            return out;
        }
        let (x0, off) = (self.x0 / k, self.scroll.offset() / k);
        for (i, h) in self.search_hits.iter().enumerate() {
            let y = GRID_TOP + ROW_PT * i as f32 - off;
            if y + ROW_PT < GRID_TOP || y > self.bottom(ui) / k {
                continue;
            }
            out.push(Node {
                id: 400 + i as u64,
                role: Role::Button,
                label: h.title.clone(),
                value: hit_detail(h),
                bounds: Rect { x: x0, y, w: self.w as f32 / k, h: ROW_PT },
                enabled: true,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(day: u32, minute: u16, all_day: bool, location: &str) -> Hit {
        Hit { date: Date { year: 2026, month: 10, day }, minute, all_day, title: "Lunch".into(), location: location.into() }
    }

    #[test]
    fn a_result_says_when_and_where() {
        assert_eq!(hit_detail(&hit(5, 750, false, "")), "Mon, 5 Oct 2026  \u{b7}  12:30");
        assert_eq!(hit_detail(&hit(6, 750, false, "Cafe Paris")), "Tue, 6 Oct 2026  \u{b7}  12:30  \u{b7}  Cafe Paris");
        assert_eq!(hit_detail(&hit(7, 0, true, "")), "Wed, 7 Oct 2026  \u{b7}  all-day");
    }

    #[test]
    fn the_header_counts_the_matches() {
        assert_eq!(count_label(0), "No results");
        assert_eq!(count_label(1), "1 result");
        assert_eq!(count_label(14), "14 results");
    }
}
