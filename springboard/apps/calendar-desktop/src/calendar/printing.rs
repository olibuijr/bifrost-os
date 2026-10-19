//! File > Print: the shown day, week, month or year drawn as a grid on landscape pages (`printgrid.rs`, `print_pages`). The
//! agenda text of the same range (one heading per day with events, one line per event: `agenda_text`, `print_job`) is the
//! list form of the same events.
use super::printgrid;
use super::store::CalendarEvent;
use super::timeview::View;
use super::{time_label, Calendar, Date, MONTHS, WEEKDAYS};
use bifrost_app::print::{PdfPage, PrintInfo};
use std::borrow::Cow;

/// The width of the time column in the agenda: "23:00 - 01:00 +1" and a space.
const TIME_W: usize = 17;

/// The days a view of `selected` covers: the day, its week (starting on `week_start`), the month or the year.
pub fn range_days(view: View, selected: Date, week_start: Date) -> Vec<Date> {
    match view {
        View::Day => vec![selected],
        View::Week => (0..7).map(|i| week_start.shift_days(i)).collect(),
        View::Month => (1..=selected.month_len()).map(|day| Date { day, ..selected }).collect(),
        View::Year => {
            let first = Date { month: 1, day: 1, ..selected };
            let n = Date { month: 12, day: 31, ..selected }.days() - first.days() + 1;
            (0..n as i32).map(|i| first.shift_days(i)).collect()
        }
    }
}

/// "09:00 - 10:00", with "+1" after an end that falls on the next day; "All day" for an all-day event.
pub fn time_range(e: &CalendarEvent) -> String {
    if e.all_day {
        return "All day".to_string();
    }
    let end = e.minute as u32 + e.duration as u32;
    let next = if end >= 1440 { " +1" } else { "" };
    format!("{} - {}{next}", time_label(e.minute), time_label((end % 1440) as u16))
}

/// `title`, underlined, then each of `days` that has events: its name, and a line per event (time and title, then the
/// location and notes when there are some). "No events." when the range has none.
pub fn agenda_text(title: &str, days: &[Date], events: &[CalendarEvent]) -> String {
    let mut out = format!("{title}\n{}\n", "=".repeat(title.chars().count()));
    let indent = " ".repeat(2 + TIME_W);
    let mut any = false;
    for &day in days {
        let mut today: Vec<&CalendarEvent> = events.iter().filter(|e| e.occurs_on(day)).collect();
        if today.is_empty() {
            continue;
        }
        any = true;
        today.sort_by_key(|e| (!e.all_day, e.minute, e.id));
        out.push_str(&format!("\n{} {} {} {}\n", WEEKDAYS[day.weekday()], day.day, MONTHS[(day.month - 1) as usize], day.year));
        for e in today {
            out.push_str(&format!("  {:<width$}{}\n", time_range(e), e.title, width = TIME_W));
            if !e.location.is_empty() {
                out.push_str(&format!("{indent}Location: {}\n", e.location));
            }
            if !e.notes.is_empty() {
                out.push_str(&format!("{indent}Notes: {}\n", e.notes));
            }
        }
    }
    if !any {
        out.push_str("\nNo events.\n");
    }
    out
}

impl Calendar {
    /// The heading of what the window shows: the month and year, the year, or the day or week range.
    fn print_title(&self) -> String {
        match self.view {
            View::Month => format!("{} {}", MONTHS[(self.selected.month - 1) as usize], self.selected.year),
            View::Year => self.selected.year.to_string(),
            _ => self.range_label(),
        }
    }

    /// The job name and the text of the agenda of what the window shows (`agenda_text`). File > Print sends the grid
    /// (`print_pages`); the agenda is the same events as a list, for a destination that takes text.
    pub fn print_job(&self) -> (String, String) {
        let title = self.print_title();
        let days = match self.view {
            View::Week => self.view_days(),
            view => range_days(view, self.selected, self.week_start()),
        };
        let shown: Vec<CalendarEvent> = self.shown_events().map(Cow::into_owned).collect();
        let text = agenda_text(&title, &days, &shown);
        (format!("Calendar {title}"), text)
    }

    /// The job name, the pages and the paper File > Print sends: the grid of what the window shows drawn on a landscape
    /// page (`printgrid.rs`), the events as the grid shows them (a view time zone moves them, `viewzone.rs`).
    pub fn print_pages(&self) -> (String, Vec<PdfPage>, PrintInfo) {
        let title = self.print_title();
        let shown: Vec<CalendarEvent> = self.shown_events().map(Cow::into_owned).collect();
        let grid = printgrid::Grid {
            view: self.view,
            title: title.clone(),
            selected: self.selected,
            days: self.view_days(),
            first_weekday: self.prefs.weekday_at(0),
            day_start: u32::from(self.prefs.day_start),
            day_end: u32::from(self.prefs.day_end),
        };
        let info = printgrid::paper();
        let occurs = |e: &CalendarEvent, day: Date| self.occurs(e, day);
        let pages = printgrid::pages(&grid, &shown, &occurs, &info);
        (format!("Calendar {title}"), pages, info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(month: u32, day: u32) -> Date {
        Date { year: 2026, month, day }
    }
    fn ev(id: u64, date: Date, minute: u16, duration: u16, title: &str) -> CalendarEvent {
        CalendarEvent { id, date, minute, duration, all_day: false, title: title.into(), location: String::new(), notes: String::new(), repeat: 0, alert: 0, uid: String::new() }
    }

    #[test]
    fn each_view_covers_its_days() {
        assert_eq!(range_days(View::Day, d(10, 4), d(10, 4)), vec![d(10, 4)]);
        let week = range_days(View::Week, d(10, 7), d(10, 4));
        assert_eq!((week.len(), week[0], week[6]), (7, d(10, 4), d(10, 10)));
        let month = range_days(View::Month, d(2, 9), d(2, 8));
        assert_eq!((month.len(), month[0], month[27]), (28, d(2, 1), d(2, 28)));
        let leap = range_days(View::Month, Date { year: 2028, month: 2, day: 1 }, d(1, 1));
        assert_eq!(leap.len(), 29);
        assert_eq!(range_days(View::Year, d(6, 1), d(5, 31)).len(), 365);
        assert_eq!(range_days(View::Year, Date { year: 2028, month: 6, day: 1 }, d(5, 31)).len(), 366);
    }

    #[test]
    fn times_read_as_ranges() {
        assert_eq!(time_range(&ev(1, d(10, 4), 540, 60, "x")), "09:00 - 10:00");
        assert_eq!(time_range(&ev(1, d(10, 4), 1380, 120, "x")), "23:00 - 01:00 +1");
        let mut all_day = ev(1, d(10, 4), 0, 1440, "x");
        all_day.all_day = true;
        assert_eq!(time_range(&all_day), "All day");
    }

    #[test]
    fn the_agenda_lists_days_with_events_in_time_order() {
        let mut dentist = ev(2, d(10, 6), 600, 30, "Dentist");
        dentist.location = "Laugavegur 5".into();
        dentist.notes = "Bring the form".into();
        let mut holiday = ev(3, d(10, 6), 0, 1440, "Holiday");
        holiday.all_day = true;
        let events = vec![ev(1, d(10, 4), 540, 60, "Standup"), dentist, holiday, ev(4, d(10, 6), 480, 30, "Run")];
        let days: Vec<Date> = (4..=7).map(|n| d(10, n)).collect();
        let text = agenda_text("Oct 4 - 10, 2026", &days, &events);
        let row = |time: &str, title: &str| format!("  {time:<17}{title}");
        let more = " ".repeat(19);
        let expected = [
            "Oct 4 - 10, 2026".to_string(),
            "=".repeat(16),
            String::new(),
            "Sunday 4 October 2026".to_string(),
            row("09:00 - 10:00", "Standup"),
            String::new(),
            "Tuesday 6 October 2026".to_string(),
            row("All day", "Holiday"),
            row("08:00 - 08:30", "Run"),
            row("10:00 - 10:30", "Dentist"),
            format!("{more}Location: Laugavegur 5"),
            format!("{more}Notes: Bring the form"),
        ]
        .join("\n")
            + "\n";
        assert_eq!(text, expected);
        assert!(text.contains("  09:00 - 10:00    Standup"), "four spaces after a 13 character time");
    }

    #[test]
    fn a_time_that_runs_past_midnight_still_has_room_before_the_title() {
        let text = agenda_text("Night", &[d(10, 4)], &[ev(1, d(10, 4), 1380, 120, "Shift")]);
        assert!(text.contains("23:00 - 01:00 +1 Shift"));
    }

    #[test]
    fn repeating_events_appear_on_each_day_they_occur() {
        let mut daily = ev(1, d(10, 4), 540, 60, "Standup");
        daily.repeat = 1;
        let text = agenda_text("Week", &[d(10, 4), d(10, 5)], &[daily]);
        assert_eq!(text.matches("Standup").count(), 2);
    }

    #[test]
    fn an_empty_range_says_so() {
        assert_eq!(agenda_text("October 2026", &[d(10, 1)], &[]), "October 2026\n============\n\nNo events.\n");
    }
}
