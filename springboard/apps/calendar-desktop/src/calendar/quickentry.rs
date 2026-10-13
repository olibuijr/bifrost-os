//! Event > Quick Event: a sheet with one line to type what the event is ("Lunch with Ana tomorrow at 12:30 at Laugavegur 5").
//! It shows how the sentence is read (`quick::parse`) and Add saves the event at once, on the day and time it names.
use super::goto::date_title;
use super::intents::NewEvent;
use super::quick::{self, Quick};
use super::*;
use bifrost_app::controls::{Node, Rect, Role, TextField as Line};

/// The line's top and a row's height (points), as in the event form.
const TOP: f32 = 108.0;
const ROW: f32 = 44.0;

/// The rows of the preview under the line: how the sentence reads. A part the sentence leaves out shows what the event
/// would get (the shown day, 09:00) or "None".
pub fn preview(q: &Quick, shown: Date) -> Vec<(&'static str, String)> {
    vec![
        ("Title", if q.title.trim().is_empty() { "None yet".to_string() } else { q.title.trim().to_string() }),
        ("Day", date_title(q.date.unwrap_or(shown))),
        ("Time", time_label(q.minute.unwrap_or(9 * 60))),
        ("Place", q.location.clone().unwrap_or_else(|| "None".to_string())),
    ]
}

impl Calendar {
    /// Whether a sheet (Go to Date, Quick Event) is open: the window behind it takes no commands.
    pub(super) fn sheet_open(&self) -> bool {
        self.goto.is_some() || self.quick_line.is_some()
    }

    /// Opens the sheet with an empty line.
    pub(super) fn open_quick(&mut self) {
        self.quick_line = Some(Line::new());
        self.search_focused = false;
        self.focus(None);
    }

    /// The text in the sheet's line, while the sheet is open.
    pub fn quick_text(&self) -> Option<&str> {
        self.quick_line.as_deref()
    }

    /// The preview rows for what is typed now.
    pub fn quick_preview(&self) -> Vec<(&'static str, String)> {
        let text = self.quick_text().unwrap_or("");
        preview(&quick::parse(text, self.today), self.selected)
    }

    /// A key typed in the sheet: text and Backspace edit the line, Left and Right move in it, Return adds the event.
    pub(super) fn quick_key(&mut self, ui: &Ui, out: Out, t: f64) {
        let Some(line) = &mut self.quick_line else { return };
        match out {
            Out::Text(text) => line.push_str(&text),
            Out::Backspace => line.pop(),
            Out::Named("Left") => line.move_caret(false),
            Out::Named("Right") => line.move_caret(true),
            Out::Enter => {
                self.quick_add(ui, t);
                return;
            }
            _ => return,
        }
        self.error = None;
        self.dirty = true;
    }

    /// Text pasted into the line (Edit > Paste): the first line of it.
    pub(super) fn quick_paste(&mut self, text: &str) {
        if let Some(line) = &mut self.quick_line {
            line.push_str(text.lines().next().unwrap_or(""));
            self.error = None;
            self.dirty = true;
        }
    }

    /// Add: saves the event the sentence describes and closes the sheet. The sheet stays, with a message, when the
    /// sentence names no event or the event cannot be saved.
    pub(super) fn quick_add(&mut self, ui: &Ui, _t: f64) {
        let text = self.quick_text().unwrap_or("").to_string();
        let q = quick::parse(&text, self.today);
        if q.title.trim().is_empty() {
            self.error = Some("Type what the event is, for example: Lunch tomorrow at 12:30".into());
            return;
        }
        let new = NewEvent {
            title: q.title,
            date: q.date.unwrap_or(self.selected),
            minute: q.minute,
            minutes: None,
            location: q.location.unwrap_or_default(),
            notes: String::new(),
            all_day: false,
        };
        match self.create_event(new) {
            Ok(e) => {
                self.quick_line = None;
                self.error = None;
                self.notice = Some(format!("Added {} on {} at {}", e.title, e.date.iso(), time_label(e.minute)));
                self.fit_scroll(ui);
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// The sheet: the line, then how it reads.
    pub(super) fn draw_quick(&mut self, s: &mut Surface, ui: &Ui) {
        let Some(line) = &self.quick_line else { return };
        let w = self.w as f32;
        let k = ui.pt(1.0);
        let field = Rect { x: 36.0, y: TOP + SPACE_S, w: w / k - 72.0, h: ROW - 2.0 * SPACE_S };
        bifrost_app::widgets::text_field(s, ui, line, field, "Lunch with Ana tomorrow at 12:30", true);
        let rows = self.quick_preview();
        let top = TOP + ROW + 20.0;
        s.fill_rrect(ui.pt(SPACE_XL), ui.pt(top), w - ui.pt(SPACE_HUGE), ui.pt(ROW * rows.len() as f32), ui.pt(CARD_R), CARD_BG.now());
        for (i, (name, value)) in rows.iter().enumerate() {
            let mid = ui.pt(top + ROW * i as f32 + ROW / 2.0);
            if i > 0 {
                s.fill_rrect(ui.pt(36.0), ui.pt(top + ROW * i as f32), w - ui.pt(56.0), ui.pt(0.5).max(1.0), 0.0, SEPARATOR_C.now());
            }
            self.text(s, ui, name, MAC_BODY_PT, LABEL.now(), ui.pt(36.0), mid);
            let fit = ui.fit(value, MAC_BODY_PT, w - ui.pt(150.0));
            self.right(s, ui, &fit, LABEL_2.now(), w - ui.pt(36.0), mid);
        }
        let y = top + ROW * rows.len() as f32 + 28.0;
        if let Some(error) = self.error.clone() {
            self.centre(s, ui, &error, MAC_BODY_PT, Weight::Semibold, LABEL.now(), w / 2.0, ui.pt(y));
        } else {
            let hint = "Day names, dates, times and an address are read from the sentence";
            self.centre(s, ui, hint, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), w / 2.0, ui.pt(y));
        }
    }

    /// The line as a text field for assistive technology.
    pub(super) fn quick_nodes(&self, ui: &Ui) -> Vec<Node> {
        let Some(line) = &self.quick_line else { return vec![] };
        let k = ui.pt(1.0);
        vec![Node {
            id: 250,
            role: Role::TextField,
            label: "Quick Event".into(),
            value: line.to_string(),
            bounds: Rect { x: self.x0 / k + 36.0, y: TOP, w: self.w as f32 / k - 72.0, h: ROW },
            enabled: true,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(month: u32, day: u32) -> Date {
        Date { year: 2026, month, day }
    }

    #[test]
    fn the_preview_reads_the_sentence_and_fills_in_what_it_leaves_out() {
        // said on Tuesday 13 October, "next Tuesday" is the 20th
        let q = quick::parse("Lunch with Ana next Tuesday at 12:30 at Laugavegur 5", d(10, 13));
        let rows = preview(&q, d(10, 13));
        assert_eq!(rows[0], ("Title", "Lunch with Ana".to_string()));
        assert_eq!(rows[1], ("Day", "Tuesday, October 20, 2026".to_string()));
        assert_eq!(rows[2], ("Time", "12:30".to_string()));
        assert_eq!(rows[3].0, "Place");
        assert!(rows[3].1.contains("Laugavegur"));
        let bare = preview(&quick::parse("Dentist", d(10, 13)), d(10, 9));
        assert_eq!(bare[1].1, "Friday, October 9, 2026", "the shown day");
        assert_eq!(bare[2].1, "09:00");
        assert_eq!(bare[3].1, "None");
        assert_eq!(preview(&quick::parse("", d(10, 13)), d(10, 13))[0].1, "None yet");
    }
}
