//! Go > Go to Date: a sheet with day, month and year steppers that sets the shown date. It reuses the event form's
//! look (grouped rows, minus and plus at the sides) and its toolbar (Cancel, title, Go).
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
    /// Opens the sheet on the shown date.
    pub(super) fn open_goto(&mut self) {
        self.goto = Some(self.selected);
        self.focus(None);
    }

    /// The sheet's rows as (target, top pt, height pt): the three steppers, then a Today shortcut in its own group.
    pub(super) fn goto_rows(&self) -> Vec<(Target, f32, f32)> {
        if self.goto.is_none() {
            return vec![];
        }
        vec![
            (Target::GoDay(0), TOP, ROW),
            (Target::GoMonth(0), TOP + ROW, ROW),
            (Target::GoYear(0), TOP + 2.0 * ROW, ROW),
            (Target::GoToday, TOP + 3.0 * ROW + 20.0, ROW),
        ]
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

    /// Moves the sheet's date by one step.
    pub(super) fn goto_step(&mut self, target: Target) {
        if let Some(d) = self.goto {
            self.goto = Some(step(d, target));
        }
    }

    /// Go: shows the chosen date and closes the sheet.
    pub(super) fn goto_confirm(&mut self, ui: &Ui) {
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
        for (first, last) in [(0, 2), (3, 3)] {
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
