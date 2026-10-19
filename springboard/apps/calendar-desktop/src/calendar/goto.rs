//! Go > Go to Date: a sheet with day, month and year steppers that sets the shown date, and a row to type the date in
//! ("2026-12-24", "24.12.2026", "24 Dec", "next Friday"). It reuses the event form's look (grouped rows, minus and plus at
//! the sides) and its toolbar (Cancel, title, Go). What is typed moves the steppers as it is read; Return or Go takes it.
use super::quick;
use super::*;
use bifrost_app::controls::{Node, Rect, Role};

/// The first row's top and one row's height (points), as in the event form.
const TOP: f32 = 108.0;
const ROW: f32 = 44.0;
/// How far from each side a touch counts as the minus or plus (points), as in the event form.
const SIDE: f32 = 100.0;

/// "Sunday, October 4, 2026".
pub fn date_title(d: Date) -> String {
    format!("{}, {} {}, {}", WEEKDAYS[d.weekday()], MONTHS[(d.month - 1) as usize], d.day, d.year)
}

/// The sheet's three values: the day of the month, the month's name and the year.
pub fn values(d: Date) -> (String, String, String) {
    (d.day.to_string(), MONTHS[(d.month - 1) as usize].to_string(), d.year.to_string())
}

/// The month number (1 to 12) a word names: the full name or its first three letters ("dec"), in any case, or "sept".
fn month_of(word: &str) -> Option<u32> {
    let w = word.to_ascii_lowercase();
    if w.len() < 3 {
        return None;
    }
    MONTHS
        .iter()
        .position(|m| {
            let name = m.to_ascii_lowercase();
            name == w || (w.len() == 3 && name.starts_with(&w)) || (w == "sept" && name == "september")
        })
        .map(|i| i as u32 + 1)
}

/// A date written with a month name: "24 Dec 2026", "December 24, 2026" and "dec 24th" are read. The numbers are the day
/// (one or two digits, an ordinal ending allowed) and the year (four digits); a date with no year is in `year`. None
/// unless there is exactly one month, one day and at most one year.
fn by_month_name(text: &str, year: i64) -> Option<Date> {
    let (mut month, mut day, mut named_year) = (None, None, None);
    for raw in text.split(|c: char| c.is_whitespace() || c == ',') {
        let word = raw.trim_matches('.').to_ascii_lowercase();
        if word.is_empty() {
            continue;
        }
        if let Some(m) = month_of(&word) {
            if month.replace(m).is_some() {
                return None;
            }
            continue;
        }
        let digits = word.trim_end_matches(|c: char| c.is_ascii_alphabetic());
        let ending = &word[digits.len()..];
        if !ending.is_empty() && !["st", "nd", "rd", "th"].contains(&ending) {
            return None;
        }
        let n: i64 = digits.parse().ok()?;
        let taken = if digits.len() == 4 { named_year.replace(n) } else { day.replace(n) };
        if taken.is_some() {
            return None;
        }
    }
    let date = Date { year: named_year.unwrap_or(year), month: month?, day: u32::try_from(day?).ok()? };
    date.valid().then_some(date)
}

/// The date `text` names: `2026-12-24` or `24.12.2026`, a day and a month name with an optional year ("24 Dec",
/// "December 24, 2026"; with no year it is in `year`), or one of the words the quick event reader knows for a day
/// ("today", "tomorrow", a weekday, "next Friday") counted from `today`. None for anything else, a time of day included.
pub fn read_date(text: &str, today: Date, year: i64) -> Option<Date> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if let Some(d) = quick::parse_date(t) {
        return Some(d);
    }
    if let Some(d) = by_month_name(t, year) {
        return Some(d);
    }
    let q = quick::parse(t, today);
    if q.title.trim().is_empty() && q.minute.is_none() {
        q.date
    } else {
        None
    }
}

/// `d` moved by one step of a stepper row: days, months or years (the day is kept inside the new month).
pub fn step(d: Date, target: Target) -> Date {
    match target {
        Target::GoDay(n) => d.shift_days(n),
        Target::GoMonth(n) => d.shift_months(n),
        Target::GoYear(n) => d.shift_months(12 * n),
        _ => d,
    }
}

impl Calendar {
    /// Opens the sheet on the shown date, with an empty date row.
    pub(super) fn open_goto(&mut self) {
        self.goto = Some(self.selected);
        self.goto_line.clear();
        self.focus(None);
    }

    /// The sheet's rows as (target, top pt, height pt): the three steppers, a Today shortcut in its own group, and the row
    /// the date is typed in.
    pub(super) fn goto_rows(&self) -> Vec<(Target, f32, f32)> {
        if self.goto.is_none() {
            return vec![];
        }
        vec![
            (Target::GoDay(0), TOP, ROW),
            (Target::GoMonth(0), TOP + ROW, ROW),
            (Target::GoYear(0), TOP + 2.0 * ROW, ROW),
            (Target::GoToday, TOP + 3.0 * ROW + 20.0, ROW),
            (Target::GoField, TOP + 4.0 * ROW + 40.0, ROW),
        ]
    }

    /// The date typed in the sheet's row, read when it changes: a date that reads moves the steppers to it. A row that does
    /// not read yet leaves them where they are (the reason waits for Go).
    pub(super) fn goto_read(&mut self) {
        let text = self.goto_line.to_string();
        let shown = self.goto.unwrap_or(self.selected);
        if let Some(date) = read_date(&text, self.today, shown.year) {
            self.goto = Some(date);
        }
        self.error = None;
    }

    /// The target under window-column pixel `(x, y)`: the left or right side of a stepper row, or the Today row.
    pub(super) fn hit_goto(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let (xp, yp, wp) = (x / ui.s, y / ui.s, self.w as f32 / ui.s);
        let row = self.goto_rows().into_iter().find(|r| (r.1..r.1 + r.2).contains(&yp))?;
        let side = |n: i32| {
            if xp < SIDE {
                Some(-n)
            } else if xp > wp - SIDE {
                Some(n)
            } else {
                None
            }
        };
        match row.0 {
            Target::GoDay(_) => side(1).map(Target::GoDay),
            Target::GoMonth(_) => side(1).map(Target::GoMonth),
            Target::GoYear(_) => side(1).map(Target::GoYear),
            other => Some(other),
        }
    }

    /// Moves the sheet's date by one step. A date typed before is dropped from the row: the steppers rule now.
    pub(super) fn goto_step(&mut self, target: Target) {
        if let Some(d) = self.goto {
            self.goto = Some(step(d, target));
            self.goto_line.clear();
        }
    }

    /// Go: shows the chosen date and closes the sheet. A date typed in the row that does not read keeps the sheet open and
    /// says what to type.
    pub(super) fn goto_confirm(&mut self, ui: &Ui) {
        if !self.goto_line.trim().is_empty() {
            let text = self.goto_line.to_string();
            let shown = self.goto.unwrap_or(self.selected);
            match read_date(&text, self.today, shown.year) {
                Some(date) => self.goto = Some(date),
                None => {
                    self.error = Some("Type a date such as 2026-12-24, 24.12.2026 or 24 Dec".into());
                    return;
                }
            }
        }
        self.goto_line.clear();
        let Some(date) = self.goto.take() else { return };
        self.selected = date;
        if self.time_view() {
            self.scroll_to_morning(ui);
        } else {
            self.scroll.jump_top();
        }
    }

    pub(super) fn draw_goto(&mut self, s: &mut Surface, ui: &Ui) {
        let Some(date) = self.goto else { return };
        let w = self.w as f32;
        let rows = self.goto_rows();
        for (first, last) in [(0, 2), (3, 3), (4, 4)] {
            let (top, bottom) = (rows[first].1, rows[last].1 + rows[last].2);
            s.fill_rrect(ui.pt(SPACE_XL), ui.pt(top), w - ui.pt(SPACE_HUGE), ui.pt(bottom - top), ui.pt(CARD_R), CARD_BG.now());
        }
        for r in &rows[1..3] {
            s.fill_rrect(ui.pt(36.0), ui.pt(r.1), w - ui.pt(56.0), ui.pt(0.5).max(1.0), 0.0, SEPARATOR_C.now());
        }
        self.centre(s, ui, &date_title(date), MAC_TITLE3_PT, Weight::Semibold, LABEL.now(), w / 2.0, ui.pt(84.0));
        let (day, month, year) = values(date);
        for (target, top, h) in rows {
            let mid = ui.pt(top + h / 2.0);
            match target {
                Target::GoDay(_) => self.step_row(s, ui, &day, mid),
                Target::GoMonth(_) => self.step_row(s, ui, &month, mid),
                Target::GoYear(_) => self.step_row(s, ui, &year, mid),
                Target::GoToday => self.centre(s, ui, "Today", MAC_BODY_PT, Weight::Regular, colors::LINK.now(), w / 2.0, mid),
                Target::GoField => {
                    let k = ui.pt(1.0);
                    let field = Rect { x: 36.0, y: top + SPACE_S, w: w / k - 72.0, h: h - 2.0 * SPACE_S };
                    bifrost_app::widgets::text_field(s, ui, &self.goto_line, field, "Type a date: 2026-12-24, 24 Dec", true);
                    let below = ui.pt(top + h + 20.0);
                    match self.error.clone() {
                        Some(error) => self.centre(s, ui, &error, MAC_BODY_PT, Weight::Semibold, LABEL.now(), w / 2.0, below),
                        None => self.centre(s, ui, "Day, month and year move as you type", MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), w / 2.0, below),
                    }
                }
                _ => {}
            }
        }
    }

    /// The sheet's controls for assistive technology: a decrease and an increase button for each stepper, and Today.
    pub(super) fn goto_nodes(&self, ui: &Ui) -> Vec<Node> {
        let Some(date) = self.goto else { return vec![] };
        let k = ui.pt(1.0);
        let (x0, wp) = (self.x0 / k, self.w as f32 / k);
        let (day, month, year) = values(date);
        let mut out = vec![];
        for (i, (target, top, h)) in self.goto_rows().into_iter().enumerate() {
            let (name, value) = match target {
                Target::GoDay(_) => ("Day", day.clone()),
                Target::GoMonth(_) => ("Month", month.clone()),
                Target::GoYear(_) => ("Year", year.clone()),
                Target::GoField => {
                    out.push(Node { id: 231, role: Role::TextField, label: "Date".into(), value: self.goto_line.to_string(), bounds: Rect { x: x0 + 36.0, y: top, w: wp - 72.0, h }, enabled: true });
                    continue;
                }
                _ => {
                    out.push(Node { id: 230, role: Role::Button, label: "Today".into(), value: String::new(), bounds: Rect { x: x0 + 36.0, y: top, w: wp - 72.0, h }, enabled: true });
                    continue;
                }
            };
            let id = 220 + 2 * i as u64;
            out.push(Node { id, role: Role::Button, label: format!("Previous {name}"), value: value.clone(), bounds: Rect { x: x0, y: top, w: SIDE, h }, enabled: true });
            out.push(Node { id: id + 1, role: Role::Button, label: format!("Next {name}"), value, bounds: Rect { x: x0 + wp - SIDE, y: top, w: SIDE, h }, enabled: true });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(year: i64, month: u32, day: u32) -> Date {
        Date { year, month, day }
    }

    #[test]
    fn the_title_names_the_weekday() {
        assert_eq!(date_title(d(2026, 10, 4)), "Sunday, October 4, 2026");
        assert_eq!(date_title(d(2024, 2, 29)), "Thursday, February 29, 2024");
    }

    #[test]
    fn the_values_are_day_month_year() {
        assert_eq!(values(d(2026, 3, 9)), ("9".to_string(), "March".to_string(), "2026".to_string()));
    }

    #[test]
    fn a_typed_date_reads_in_the_forms_people_write_it() {
        let today = d(2026, 10, 4);
        // the two numeric forms
        assert_eq!(read_date("2026-12-24", today, 2026), Some(d(2026, 12, 24)));
        assert_eq!(read_date("24.12.2026", today, 2026), Some(d(2026, 12, 24)));
        assert_eq!(read_date("  24.12.2026 ", today, 2026), Some(d(2026, 12, 24)));
        // a month name, in either order, with or without the year, an ordinal ending and commas
        assert_eq!(read_date("24 Dec 2026", today, 2026), Some(d(2026, 12, 24)));
        assert_eq!(read_date("December 24, 2027", today, 2026), Some(d(2027, 12, 24)));
        assert_eq!(read_date("dec 24th", today, 2031), Some(d(2031, 12, 24)), "no year is the shown year");
        assert_eq!(read_date("1 sept 2026", today, 2026), Some(d(2026, 9, 1)));
        assert_eq!(read_date("MARCH 3", today, 2024), Some(d(2024, 3, 3)));
        // words for a day, counted from today (a Sunday)
        assert_eq!(read_date("tomorrow", today, 2026), Some(d(2026, 10, 5)));
        assert_eq!(read_date("today", today, 2026), Some(today));
        assert_eq!(read_date("friday", today, 2026), Some(d(2026, 10, 9)));
    }

    #[test]
    fn a_typed_date_that_is_not_one_is_refused() {
        let today = d(2026, 10, 4);
        for bad in ["", "   ", "2026", "feb 30", "31 april", "32 dec", "2026-13-01", "24.12", "dec dec 24", "24 25 dec", "dec 24 2026 2027", "lunch at noon", "tomorrow at 5pm", "0 dec", "24xx dec"] {
            assert_eq!(read_date(bad, today, 2026), None, "{bad:?}");
        }
        assert_eq!(read_date("feb 29", today, 2027), None, "2027 has no 29 February");
        assert_eq!(read_date("feb 29", today, 2028), Some(d(2028, 2, 29)));
    }

    #[test]
    fn month_names_need_three_letters() {
        assert_eq!(month_of("jan"), Some(1));
        assert_eq!(month_of("January"), Some(1));
        assert_eq!(month_of("sept"), Some(9));
        assert_eq!(month_of("ju"), None);
        assert_eq!(month_of("jux"), None);
        assert_eq!(month_of("decx"), None);
    }

    #[test]
    fn steppers_keep_the_date_valid() {
        assert_eq!(step(d(2026, 1, 31), Target::GoMonth(1)), d(2026, 2, 28));
        assert_eq!(step(d(2024, 2, 29), Target::GoYear(1)), d(2025, 2, 28));
        assert_eq!(step(d(2024, 2, 29), Target::GoYear(-4)), d(2020, 2, 29));
        assert_eq!(step(d(2026, 12, 31), Target::GoDay(1)), d(2027, 1, 1));
        assert_eq!(step(d(2026, 1, 1), Target::GoDay(-1)), d(2025, 12, 31));
        assert_eq!(step(d(9999, 12, 31), Target::GoYear(1)), d(9999, 12, 31));
        assert_eq!(step(d(2026, 6, 1), Target::Today), d(2026, 6, 1));
    }
}
