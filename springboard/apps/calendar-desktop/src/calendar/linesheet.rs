//! The line sheet: one line of text for a calendar's name (New Calendar, Rename Calendar) or the address of a calendar
//! subscription (New Calendar Subscription). It has the sheet toolbar (Cancel, the title, Done) and Return accepts it.
//! A sheet that cannot take its line says why and stays open.
use super::*;
use bifrost_app::controls::{Node, Rect, Role, TextField as Line};

/// The line's top and a row's height (points), as in the event form.
const TOP: f32 = 108.0;
const ROW: f32 = 44.0;

/// What the line is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// A new calendar named by the line.
    NewCalendar,
    /// The calendar with this id gets the line as its name.
    Rename(u32),
    /// The line is the address of a calendar to subscribe to.
    Subscribe,
}

impl Purpose {
    pub(super) fn title(self) -> &'static str {
        match self {
            Purpose::NewCalendar => "New Calendar",
            Purpose::Rename(_) => "Rename Calendar",
            Purpose::Subscribe => "New Calendar Subscription",
        }
    }

    fn placeholder(self) -> &'static str {
        match self {
            Purpose::NewCalendar | Purpose::Rename(_) => "Calendar name",
            Purpose::Subscribe => "Address, such as webcal://example.com/calendar.ics",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Purpose::NewCalendar | Purpose::Rename(_) => "A calendar keeps its own events and colour, and can be hidden",
            Purpose::Subscribe => "A subscribed calendar is read-only and is refreshed about once an hour",
        }
    }
}

/// The open line sheet: what it is for and the text typed so far.
pub struct LineSheet {
    pub purpose: Purpose,
    pub line: Line,
}

impl Calendar {
    /// Opens the line sheet for `purpose` with `text` in its line (the name of a calendar, or the address to subscribe to).
    pub(super) fn open_line(&mut self, purpose: Purpose, text: &str) {
        let mut line = Line::new();
        line.push_str(text.lines().next().unwrap_or("").trim());
        self.line_sheet = Some(LineSheet { purpose, line });
        self.search_focused = false;
        self.error = None;
        self.notice = None;
        self.focus(None);
    }

    /// The text in the line sheet, while it is open.
    pub fn line_text(&self) -> Option<String> {
        self.line_sheet.as_ref().map(|s| s.line.to_string())
    }

    /// A key typed in the line: text and Backspace edit it, Left and Right move in it, Return accepts it.
    pub(super) fn line_key(&mut self, ui: &Ui, out: Out, t: f64) {
        let Some(sheet) = &mut self.line_sheet else { return };
        match out {
            Out::Text(text) => sheet.line.push_str(&text),
            Out::Backspace => sheet.line.pop(),
            Out::Named("Left") => sheet.line.move_caret(false),
            Out::Named("Right") => sheet.line.move_caret(true),
            Out::Enter => {
                self.line_done(ui, t);
                return;
            }
            _ => return,
        }
        self.error = None;
        self.dirty = true;
    }

    /// Text pasted into the line (Edit > Paste): the first line of it.
    pub(super) fn line_paste(&mut self, text: &str) {
        if let Some(sheet) = &mut self.line_sheet {
            sheet.line.push_str(text.lines().next().unwrap_or(""));
            self.error = None;
            self.dirty = true;
        }
    }

    /// Done: applies the sheet. The sheet closes when the change is made; a refusal keeps it open with the reason.
    pub(super) fn line_done(&mut self, _ui: &Ui, _t: f64) {
        let Some(sheet) = self.line_sheet.take() else { return };
        let text = sheet.line.to_string();
        let purpose = sheet.purpose;
        let outcome: Result<String, String> = match purpose {
            Purpose::NewCalendar => match self.extras.new_calendar(&text) {
                Some(_) => {
                    self.save_extras();
                    Ok(format!("Made the calendar {}", extras::clean_name(&text)))
                }
                None if extras::clean_name(&text).is_empty() => Err("Type a name for the calendar".into()),
                None => Err("There is no room for another calendar".into()),
            },
            Purpose::Rename(cal) => {
                if self.extras.rename(cal, &text) {
                    self.save_extras();
                    Ok(format!("Renamed the calendar to {}", extras::clean_name(&text)))
                } else {
                    Err("Type a name for the calendar".into())
                }
            }
            Purpose::Subscribe => self.subscribe(&text).map(|name| format!("Subscribed to {name}")),
        };
        match outcome {
            Ok(message) => {
                self.error = None;
                self.notice = Some(message);
                self.dirty = true;
            }
            Err(message) => {
                self.line_sheet = Some(sheet);
                self.error = Some(message);
                self.dirty = true;
            }
        }
    }

    /// The sheet: the line, then what it does.
    pub(super) fn draw_line(&mut self, s: &mut Surface, ui: &Ui) {
        let w = self.w as f32;
        let k = ui.pt(1.0);
        let Some(sheet) = &self.line_sheet else { return };
        let purpose = sheet.purpose;
        let field = Rect { x: 36.0, y: TOP + SPACE_S, w: w / k - 72.0, h: ROW - 2.0 * SPACE_S };
        bifrost_app::widgets::text_field(s, ui, &sheet.line, field, purpose.placeholder(), true);
        let y = TOP + ROW + 28.0;
        if let Some(error) = self.error.clone() {
            self.centre(s, ui, &error, MAC_BODY_PT, Weight::Semibold, LABEL.now(), w / 2.0, ui.pt(y));
        } else {
            self.centre(s, ui, purpose.hint(), MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), w / 2.0, ui.pt(y));
        }
    }

    /// The line as a text field for assistive technology.
    pub(super) fn line_nodes(&self, ui: &Ui) -> Vec<Node> {
        let Some(sheet) = &self.line_sheet else { return vec![] };
        let k = ui.pt(1.0);
        vec![Node {
            id: 252,
            role: Role::TextField,
            label: sheet.purpose.title().into(),
            value: sheet.line.to_string(),
            bounds: Rect { x: self.x0 / k + 36.0, y: TOP, w: self.w as f32 / k - 72.0, h: ROW },
            enabled: true,
        }]
    }
}

/// The calendar menu's actions: the sheets they open and the deletion of a calendar.
impl Calendar {
    /// File > New Calendar Subscription...: the sheet opens with `text` (the pasteboard's address) in its line.
    pub fn open_subscription(&mut self, text: &str) {
        self.open_line(Purpose::Subscribe, text);
    }

    /// File > Rename Calendar: the sheet opens with the calendar's name. False for the built-in calendar, an unknown id
    /// and while another change is open.
    pub fn open_rename(&mut self, cal: u32) -> bool {
        if !self.settings_enabled() || cal == extras::DEFAULT_CALENDAR || !self.extras.exists(cal) {
            return false;
        }
        let name = self.extras.name_of(cal);
        self.open_line(Purpose::Rename(cal), &name);
        true
    }

    /// File > Delete Calendar: takes away a calendar the user made. A calendar that still holds events is not removed;
    /// the message says to delete them first, so no event goes without the user choosing it.
    pub fn delete_calendar(&mut self, cal: u32) -> bool {
        if !self.settings_enabled() || cal == extras::DEFAULT_CALENDAR || !self.extras.exists(cal) {
            return false;
        }
        let name = self.extras.name_of(cal);
        if self.events.iter().any(|e| self.extras.calendar_of(e.id) == cal) {
            self.error = Some(format!("{name} has events. Delete them first."));
            self.dirty = true;
            return true;
        }
        if !self.extras.remove(cal) {
            return false;
        }
        self.save_extras();
        self.error = None;
        self.notice = Some(format!("Deleted the calendar {name}"));
        self.dirty = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_purpose_has_its_own_words() {
        assert_eq!(Purpose::NewCalendar.title(), "New Calendar");
        assert_eq!(Purpose::Rename(2).title(), "Rename Calendar");
        assert_eq!(Purpose::Subscribe.title(), "New Calendar Subscription");
        assert!(Purpose::Subscribe.placeholder().starts_with("Address"));
    }
}
