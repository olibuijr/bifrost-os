//! File > Print draws the grid the window shows onto the page: the month as a table of days with each day's events in its
//! cell, the week and the day as columns on an hourly grid with a block for each event, and the year as twelve small months
//! with the days that have events marked. The page is a PDF content stream (the printing service takes pages of
//! `PdfPage`), laid out for landscape paper inside the margins of a `PrintInfo`. The drawing is pure: it works from the
//! days, the events and a function that says whether an event falls on a day, so it is host-testable without a printer.
//! Two standard fonts are used (`FONTS`: Helvetica as `/F0` and Helvetica-Bold as `/F1`); text is written in WinAnsi, so
//! the letters of Western European languages print and anything else prints as `?`.
use super::layout::{self, Slot};
use super::store::CalendarEvent;
use super::timeview::View;
use super::{time_label, Date, MONTHS, WEEKDAYS};
use bifrost_app::print::{Orientation, PdfPage, PrintInfo};

/// The standard fonts the pages use, in the order of their names in the content: `/F0` and `/F1`.
pub const FONTS: [&str; 2] = ["Helvetica", "Helvetica-Bold"];

/// Helvetica's advance widths for the characters from space to tilde, in thousandths of the type size.
const WIDTHS: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, // space to /
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, // 0 to 9
    278, 278, 584, 584, 584, 556, 1015, // : to @
    667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, // A to M
    722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, // N to Z
    278, 278, 278, 469, 556, 333, // [ to `
    556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, // a to m
    556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, // n to z
    334, 260, 334, 584, // { to ~
];

/// The width of `text` set in Helvetica at `size` points. A character outside ASCII counts as an average one.
pub fn text_width(text: &str, size: f32) -> f32 {
    let units: u32 = text
        .chars()
        .map(|c| match c {
            ' '..='~' => WIDTHS[(c as u32 - 32) as usize] as u32,
            _ => 556,
        })
        .sum();
    units as f32 * size / 1000.0
}

/// `text` cut to fit `max_w` points, with `...` in place of what was cut. Text that fits is returned as it is.
pub fn fit(text: &str, size: f32, max_w: f32) -> String {
    if text_width(text, size) <= max_w {
        return text.to_string();
    }
    let dots = text_width("...", size);
    let mut out = String::new();
    for c in text.chars() {
        let next = format!("{out}{c}");
        if text_width(&next, size) + dots > max_w {
            break;
        }
        out = next;
    }
    format!("{}...", out.trim_end())
}

/// A character as the WinAnsi byte a PDF string holds: ASCII, the Latin-1 letters, a few typographic marks, else `?`.
fn winansi(c: char) -> u8 {
    match c {
        ' '..='~' => c as u8,
        '\u{a0}'..='\u{ff}' => c as u32 as u8,
        '\u{20ac}' => 0x80,
        '\u{2026}' => 0x85,
        '\u{2018}' => 0x91,
        '\u{2019}' => 0x92,
        '\u{201c}' => 0x93,
        '\u{201d}' => 0x94,
        '\u{2022}' => 0x95,
        '\u{2013}' => 0x96,
        '\u{2014}' => 0x97,
        _ => b'?',
    }
}

/// One page's content stream, drawn with coordinates in points from the page's top left corner (the PDF's own origin is
/// the bottom left, so every y is turned here).
struct Canvas {
    page_h: f32,
    out: Vec<u8>,
}

impl Canvas {
    fn new(page_h: f32) -> Self {
        Canvas { page_h, out: Vec::new() }
    }

    fn put(&mut self, text: String) {
        self.out.extend(text.into_bytes());
    }

    /// A filled rectangle in grey level `gray` (0 black, 1 white).
    fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, gray: f32) {
        let y = self.page_h - y - h;
        self.put(format!("{gray:.3} g {x:.2} {y:.2} {w:.2} {h:.2} re f\n"));
    }

    /// A line `width` points thick in grey level `gray`.
    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, gray: f32) {
        let (y1, y2) = (self.page_h - y1, self.page_h - y2);
        self.put(format!("{gray:.3} G {width:.2} w {x1:.2} {y1:.2} m {x2:.2} {y2:.2} l S\n"));
    }

    /// The outline of a rectangle.
    fn frame(&mut self, x: f32, y: f32, w: f32, h: f32, width: f32, gray: f32) {
        let y = self.page_h - y - h;
        self.put(format!("{gray:.3} G {width:.2} w {x:.2} {y:.2} {w:.2} {h:.2} re S\n"));
    }

    /// `text` with its left end at `x` and its baseline at `y`, set at `size` points in Helvetica or, with `bold`, in
    /// Helvetica-Bold, in grey level `gray`.
    fn text(&mut self, x: f32, y: f32, size: f32, bold: bool, gray: f32, text: &str) {
        let y = self.page_h - y;
        self.put(format!("BT {gray:.3} g /F{} {size:.2} Tf {x:.2} {y:.2} Td (", u8::from(bold)));
        for c in text.chars() {
            match winansi(c) {
                b @ (b'(' | b')' | b'\\') => self.out.extend([b'\\', b]),
                b => self.out.push(b),
            }
        }
        self.put(") Tj ET\n".to_string());
    }

    /// `text` with its right end at `x`.
    fn text_right(&mut self, x: f32, y: f32, size: f32, bold: bool, gray: f32, text: &str) {
        let w = text_width(text, size) * if bold { 1.05 } else { 1.0 };
        self.text(x - w, y, size, bold, gray, text);
    }

    /// `text` centred on `x`.
    fn text_centre(&mut self, x: f32, y: f32, size: f32, bold: bool, gray: f32, text: &str) {
        let w = text_width(text, size) * if bold { 1.05 } else { 1.0 };
        self.text(x - w / 2.0, y, size, bold, gray, text);
    }

    fn page(self, width: f32, height: f32) -> PdfPage {
        PdfPage { width: width as f64, height: height as f64, content: self.out, images: Vec::new() }
    }
}

/// What to print: the view, its title, the days it covers and the settings that shape the grid.
pub struct Grid {
    pub view: View,
    /// The heading: "October 2026", "Oct 4 - 10, 2026", "Sunday, October 4".
    pub title: String,
    /// The day the view is on.
    pub selected: Date,
    /// The days the Day and Week views show as columns.
    pub days: Vec<Date>,
    /// The weekday number (0 is Sunday) the week starts on, for the Month and Year grids.
    pub first_weekday: usize,
    /// The working day: the hours the time grid always shows (it grows to take in events outside them).
    pub day_start: u32,
    pub day_end: u32,
}

/// The paper the grid is printed on: landscape, with a half inch margin, as most calendars print.
pub fn paper() -> PrintInfo {
    PrintInfo { orientation: Orientation::Landscape, margins: [36.0; 4], ..PrintInfo::default() }
}

const GRAY_TEXT: f32 = 0.35;
const GRAY_FAINT: f32 = 0.6;
const GRAY_LINE: f32 = 0.7;
const GRAY_BLOCK: f32 = 0.9;
const BLACK: f32 = 0.0;
/// Space under the title before the grid starts.
const TITLE_H: f32 = 34.0;
/// The line height of an event in a month cell, and its type size.
const CELL_LINE: f32 = 10.5;
const CELL_SIZE: f32 = 7.5;

/// "09:00 Title" for a timed event, the title alone for an all-day one.
fn event_line(e: &CalendarEvent) -> String {
    if e.all_day {
        e.title.clone()
    } else {
        format!("{} {}", time_label(e.minute), e.title)
    }
}

/// The events of `events` on `day`: all-day ones first, then by start, then by id.
fn on_day<'a>(events: &'a [CalendarEvent], occurs: &dyn Fn(&CalendarEvent, Date) -> bool, day: Date) -> Vec<&'a CalendarEvent> {
    let mut out: Vec<&CalendarEvent> = events.iter().filter(|e| occurs(e, day)).collect();
    out.sort_by_key(|e| (!e.all_day, e.minute, e.id));
    out
}

/// The pages of `grid`: one page, drawn on the paper `info` describes. `occurs` says whether an event falls on a day.
pub fn pages(grid: &Grid, events: &[CalendarEvent], occurs: &dyn Fn(&CalendarEvent, Date) -> bool, info: &PrintInfo) -> Vec<PdfPage> {
    let (paper_w, paper_h) = info.paper_size();
    let (left, top, width, height) = info.imageable();
    let mut c = Canvas::new(paper_h);
    c.text(left, top + 18.0, 18.0, true, BLACK, &grid.title);
    let area = (left, top + TITLE_H, width, (height - TITLE_H).max(0.0));
    match grid.view {
        View::Month => month(&mut c, grid, events, occurs, area),
        View::Year => year(&mut c, grid, events, occurs, area),
        View::Day | View::Week => time_grid(&mut c, grid, events, occurs, area),
    }
    vec![c.page(paper_w, paper_h)]
}

/// The weekday name's first three letters.
fn short_weekday(wd: usize) -> &'static str {
    &WEEKDAYS[wd % 7][..3]
}

/// The day numbers of the month of `selected` as a table: the date shown in each of the cells of its weeks, which start on
/// `first_weekday`. The first row is the week of the 1st; the last is the week of the last day.
pub fn month_cells(selected: Date, first_weekday: usize) -> Vec<Vec<Date>> {
    let first = Date { day: 1, ..selected };
    let lead = (first.weekday() + 7 - first_weekday % 7) % 7;
    let rows = (lead + first.month_len() as usize).div_ceil(7);
    let start = first.shift_days(-(lead as i32));
    (0..rows).map(|r| (0..7).map(|c| start.shift_days((r * 7 + c) as i32)).collect()).collect()
}

fn month(c: &mut Canvas, grid: &Grid, events: &[CalendarEvent], occurs: &dyn Fn(&CalendarEvent, Date) -> bool, area: (f32, f32, f32, f32)) {
    let (x0, y0, w, h) = area;
    let head_h = 16.0;
    let cells = month_cells(grid.selected, grid.first_weekday);
    let col_w = w / 7.0;
    let row_h = ((h - head_h) / cells.len() as f32).max(1.0);
    for col in 0..7 {
        let name = short_weekday(grid.first_weekday + col);
        c.text_right(x0 + col_w * (col as f32 + 1.0) - 4.0, y0 + 11.0, 8.0, true, GRAY_TEXT, name);
    }
    let top = y0 + head_h;
    let room = (((row_h - 16.0) / CELL_LINE).floor().max(0.0)) as usize;
    for (r, row) in cells.iter().enumerate() {
        for (k, day) in row.iter().enumerate() {
            let (x, y) = (x0 + col_w * k as f32, top + row_h * r as f32);
            let in_month = day.month == grid.selected.month && day.year == grid.selected.year;
            c.frame(x, y, col_w, row_h, 0.5, GRAY_LINE);
            let number = day.day.to_string();
            c.text_right(x + col_w - 4.0, y + 11.0, 9.0, in_month, if in_month { BLACK } else { GRAY_FAINT }, &number);
            let today = on_day(events, occurs, *day);
            let shown = if today.len() <= room { today.len() } else { room.saturating_sub(1) };
            for (i, e) in today.iter().take(shown).enumerate() {
                let line = fit(&event_line(e), CELL_SIZE, col_w - 6.0);
                c.text(x + 3.0, y + 22.0 + CELL_LINE * i as f32, CELL_SIZE, false, if in_month { BLACK } else { GRAY_TEXT }, &line);
            }
            if today.len() > shown && room > 0 {
                let more = format!("{} more", today.len() - shown);
                c.text(x + 3.0, y + 22.0 + CELL_LINE * shown as f32, CELL_SIZE, false, GRAY_TEXT, &more);
            }
        }
    }
}

/// The hours the time grid covers: the working day, taken wider to hold every timed event of `days`.
pub fn hour_range(day_start: u32, day_end: u32, spans: &[(u32, u32)]) -> (u32, u32) {
    let (mut h0, mut h1) = (day_start.min(23), day_end.clamp(1, 24));
    for &(start, end) in spans {
        h0 = h0.min(start / 60);
        h1 = h1.max(end.div_ceil(60).min(24));
    }
    if h1 <= h0 {
        h1 = (h0 + 1).min(24);
        h0 = h1 - 1;
    }
    (h0, h1)
}

/// "9", "10", ... the label of an hour line.
fn hour_label(h: u32) -> String {
    match h {
        0 | 24 => "12a".to_string(),
        1..=11 => format!("{h}a"),
        12 => "12p".to_string(),
        _ => format!("{}p", h - 12),
    }
}

fn time_grid(c: &mut Canvas, grid: &Grid, events: &[CalendarEvent], occurs: &dyn Fn(&CalendarEvent, Date) -> bool, area: (f32, f32, f32, f32)) {
    let (x0, y0, w, h) = area;
    let gutter = 26.0;
    let n = grid.days.len().max(1);
    let col_w = (w - gutter) / n as f32;
    let per_day: Vec<Vec<&CalendarEvent>> = grid.days.iter().map(|d| on_day(events, occurs, *d)).collect();
    // the day names
    let head_h = 18.0;
    for (i, day) in grid.days.iter().enumerate() {
        let name = format!("{} {}", short_weekday(day.weekday()), day.day);
        c.text_centre(x0 + gutter + col_w * (i as f32 + 0.5), y0 + 12.0, 9.0, true, BLACK, &name);
    }
    // the all-day band: a line per all-day event of the busiest day, at most four
    let most_all_day = per_day.iter().map(|d| d.iter().filter(|e| e.all_day).count()).max().unwrap_or(0).min(4);
    let band_h = if most_all_day == 0 { 6.0 } else { 4.0 + CELL_LINE * most_all_day as f32 };
    let band_top = y0 + head_h;
    for (i, day_events) in per_day.iter().enumerate() {
        let x = x0 + gutter + col_w * i as f32;
        for (k, e) in day_events.iter().filter(|e| e.all_day).take(4).enumerate() {
            let y = band_top + 2.0 + CELL_LINE * k as f32;
            c.fill(x + 1.5, y, col_w - 3.0, CELL_LINE - 1.5, GRAY_BLOCK);
            c.text(x + 3.0, y + 6.5, CELL_SIZE, false, BLACK, &fit(&e.title, CELL_SIZE, col_w - 7.0));
        }
    }
    if most_all_day > 0 {
        c.text(x0, band_top + 8.0, 6.5, false, GRAY_TEXT, "all-day");
    }
    // the hours
    let spans: Vec<(u32, u32)> = per_day.iter().flat_map(|d| d.iter().filter(|e| !e.all_day).map(|e| (e.minute as u32, (e.minute as u32 + e.duration as u32).min(1440)))).collect();
    let (h0, h1) = hour_range(grid.day_start, grid.day_end, &spans);
    let grid_top = band_top + band_h;
    let hour_h = ((y0 + h - grid_top) / (h1 - h0) as f32).max(1.0);
    for hour in h0..=h1 {
        let y = grid_top + hour_h * (hour - h0) as f32;
        c.line(x0 + gutter, y, x0 + w, y, 0.4, GRAY_LINE);
        if hour < h1 {
            c.text_right(x0 + gutter - 3.0, y + 7.0, 6.5, false, GRAY_TEXT, &hour_label(hour));
        }
    }
    for i in 0..=n {
        let x = x0 + gutter + col_w * i as f32;
        c.line(x, band_top, x, grid_top + hour_h * (h1 - h0) as f32, 0.4, GRAY_LINE);
    }
    c.line(x0 + gutter, grid_top, x0 + w, grid_top, 0.6, GRAY_TEXT);
    // the blocks, side by side where they overlap
    for (i, day_events) in per_day.iter().enumerate() {
        let timed: Vec<&&CalendarEvent> = day_events.iter().filter(|e| !e.all_day).collect();
        let day_spans: Vec<(u32, u32)> = timed.iter().map(|e| (e.minute as u32, (e.minute as u32 + e.duration as u32).min(1440))).collect();
        let slots: Vec<Slot> = layout::layout(&day_spans, 30);
        for ((e, span), slot) in timed.iter().zip(&day_spans).zip(slots) {
            let (x, bw) = layout::slot_x(slot, x0 + gutter + col_w * i as f32, col_w, 1.5);
            let y = grid_top + (span.0 as f32 / 60.0 - h0 as f32) * hour_h;
            let bh = ((span.1 - span.0) as f32 / 60.0 * hour_h - 1.0).max(9.0);
            c.fill(x, y, bw, bh, GRAY_BLOCK);
            c.fill(x, y, 2.0, bh, GRAY_TEXT);
            let text_w = bw - 6.0;
            if text_w > 8.0 {
                c.text(x + 4.0, y + 7.0, CELL_SIZE, true, BLACK, &fit(&e.title, CELL_SIZE, text_w));
                if bh >= 18.0 {
                    let when = format!("{} - {}", time_label(e.minute), time_label(((e.minute as u32 + e.duration as u32) % 1440) as u16));
                    c.text(x + 4.0, y + 7.0 + CELL_LINE, CELL_SIZE, false, GRAY_TEXT, &fit(&when, CELL_SIZE, text_w));
                }
                if bh >= 28.0 && !e.location.is_empty() {
                    c.text(x + 4.0, y + 7.0 + 2.0 * CELL_LINE, CELL_SIZE, false, GRAY_TEXT, &fit(&e.location, CELL_SIZE, text_w));
                }
            }
        }
    }
}

fn year(c: &mut Canvas, grid: &Grid, events: &[CalendarEvent], occurs: &dyn Fn(&CalendarEvent, Date) -> bool, area: (f32, f32, f32, f32)) {
    let (x0, y0, w, h) = area;
    let (cols, rows) = (4usize, 3usize);
    let (cw, ch) = (w / cols as f32, h / rows as f32);
    for m in 1..=12u32 {
        let idx = (m - 1) as usize;
        let (x, y) = (x0 + cw * (idx % cols) as f32, y0 + ch * (idx / cols) as f32);
        let first = Date { year: grid.selected.year, month: m, day: 1 };
        c.text(x + 6.0, y + 11.0, 10.0, true, BLACK, MONTHS[idx]);
        let (dw, dh) = ((cw - 12.0) / 7.0, ((ch - 34.0) / 7.0).min(14.0));
        for col in 0..7 {
            c.text_centre(x + 6.0 + dw * (col as f32 + 0.5), y + 24.0, 6.5, true, GRAY_TEXT, &short_weekday(grid.first_weekday + col)[..1]);
        }
        let lead = (first.weekday() + 7 - grid.first_weekday % 7) % 7;
        for day in 1..=first.month_len() {
            let slot = lead + day as usize - 1;
            let (col, row) = (slot % 7, slot / 7);
            let date = Date { day, ..first };
            let busy = events.iter().any(|e| occurs(e, date));
            let (cx, top) = (x + 6.0 + dw * (col as f32 + 0.5), y + 28.0 + dh * row as f32);
            if busy {
                c.fill(cx - dw / 2.0 + 0.5, top, dw - 1.0, dh - 1.0, GRAY_BLOCK);
            }
            c.text_centre(cx, top + dh - 4.0, 7.0, busy, BLACK, &day.to_string());
        }
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
    fn on_date(e: &CalendarEvent, day: Date) -> bool {
        e.occurs_on(day)
    }
    fn text_of(page: &PdfPage) -> String {
        String::from_utf8_lossy(&page.content).into_owned()
    }
    fn grid(view: View, selected: Date, days: Vec<Date>) -> Grid {
        Grid { view, title: "Title".into(), selected, days, first_weekday: 0, day_start: 8, day_end: 18 }
    }

    #[test]
    fn text_widths_follow_helvetica() {
        // the standard widths: a space is 278 units, a capital M 833, a digit 556
        assert!((text_width(" ", 10.0) - 2.78).abs() < 1e-4);
        assert!((text_width("M", 10.0) - 8.33).abs() < 1e-4);
        assert!((text_width("2026", 10.0) - 22.24).abs() < 1e-3);
        assert_eq!(text_width("", 12.0), 0.0);
        assert_eq!(WIDTHS.len(), 95, "space to tilde");
        // a letter outside ASCII counts as an average one
        assert!((text_width("\u{fe}", 10.0) - 5.56).abs() < 1e-4);
    }

    #[test]
    fn long_text_is_cut_with_dots_and_short_text_is_left_alone() {
        assert_eq!(fit("Standup", 10.0, 200.0), "Standup");
        let cut = fit("A very long meeting title indeed", 10.0, 60.0);
        assert!(cut.ends_with("...") && cut.len() < 32, "{cut}");
        assert!(text_width(&cut, 10.0) <= 60.0 + 1e-3, "{cut}");
        assert_eq!(fit("anything", 10.0, 1.0), "...", "no room even for a letter");
    }

    #[test]
    fn text_is_written_in_winansi_with_the_escapes_a_pdf_needs() {
        let mut c = Canvas::new(100.0);
        c.text(10.0, 20.0, 9.0, false, 0.0, "a(b)\\c \u{fe}\u{f0} \u{4e2d}");
        let bytes = c.out.clone();
        let want: Vec<u8> = b"(a\\(b\\)\\\\c ".iter().copied().chain([0xfe, 0xf0, b' ', b'?', b')']).collect();
        assert!(bytes.windows(want.len()).any(|w| w == want.as_slice()), "{:?}", String::from_utf8_lossy(&bytes));
        // y is counted from the top: baseline 20 from the top of a page 100 high is 80 from the bottom
        assert!(String::from_utf8_lossy(&bytes).contains("10.00 80.00 Td"));
    }

    #[test]
    fn the_month_table_has_the_rows_its_weeks_need() {
        // October 2026 starts on a Thursday: five weeks starting Sunday, and still five starting Monday
        let sunday = month_cells(d(10, 15), 0);
        assert_eq!((sunday.len(), sunday[0][0], sunday[0][4], sunday[4][6]), (5, d(9, 27), d(10, 1), d(10, 31)));
        let monday = month_cells(d(10, 15), 1);
        assert_eq!((monday.len(), monday[0][0], monday[0][3]), (5, d(9, 28), d(10, 1)));
        // February 2026 starts on a Sunday and has 28 days: four rows when weeks start Sunday, five when Monday
        assert_eq!(month_cells(d(2, 10), 0).len(), 4);
        assert_eq!(month_cells(d(2, 10), 1).len(), 5);
        // a month that spills into a sixth row: August 2026 starts on a Saturday
        assert_eq!(month_cells(d(8, 1), 0).len(), 6);
        for row in month_cells(d(10, 1), 0) {
            assert_eq!(row.len(), 7);
        }
    }

    #[test]
    fn a_month_page_lists_each_days_events_in_its_cell() {
        let events = vec![ev(1, d(10, 6), 600, 30, "Dentist"), ev(2, d(10, 6), 480, 30, "Run")];
        let pages = pages(&grid(View::Month, d(10, 4), vec![]), &events, &on_date, &paper());
        assert_eq!(pages.len(), 1);
        let (w, h) = paper().paper_size();
        assert_eq!((pages[0].width, pages[0].height), (w as f64, h as f64));
        assert!(w > h, "landscape");
        let t = text_of(&pages[0]);
        assert!(t.contains("(Title) Tj"), "the heading");
        assert!(t.contains("(08:00 Run) Tj") && t.contains("(10:00 Dentist) Tj"));
        // Run (08:00) is listed above Dentist (10:00): the first cell line is drawn first
        assert!(t.find("(08:00 Run)").unwrap() < t.find("(10:00 Dentist)").unwrap());
        assert!(t.contains("(Sun) Tj") && t.contains("(Sat) Tj"), "the weekday names");
        assert!(t.contains(" re S"), "the cells are framed");
    }

    #[test]
    fn a_busy_cell_ends_in_how_many_are_left_out() {
        let events: Vec<CalendarEvent> = (1..=30).map(|id| ev(id, d(10, 6), 540, 30, &format!("Item {id}"))).collect();
        let pages = pages(&grid(View::Month, d(10, 4), vec![]), &events, &on_date, &paper());
        let t = text_of(&pages[0]);
        assert!(t.contains(" more) Tj"), "{t}");
        assert!(!t.contains("(09:00 Item 30)"), "the last items do not fit the cell");
    }

    #[test]
    fn the_hour_grid_grows_to_hold_early_and_late_events() {
        assert_eq!(hour_range(8, 18, &[]), (8, 18));
        assert_eq!(hour_range(8, 18, &[(300, 360)]), (5, 18), "an event at 05:00 starts the grid at 5");
        assert_eq!(hour_range(8, 18, &[(1200, 1290)]), (8, 22), "an event to 21:30 ends it at 22");
        assert_eq!(hour_range(8, 18, &[(0, 1440)]), (0, 24));
        assert_eq!(hour_range(8, 18, &[(600, 660)]), (8, 18), "an event inside the day changes nothing");
        let (a, b) = hour_range(20, 20, &[]);
        assert!(b > a, "the range is never empty");
    }

    #[test]
    fn a_week_page_has_a_column_per_day_and_a_block_per_event() {
        let days: Vec<Date> = (4..=10).map(|n| d(10, n)).collect();
        let mut holiday = ev(3, d(10, 6), 0, 1440, "Trip");
        holiday.all_day = true;
        let events = vec![ev(1, d(10, 6), 600, 60, "Dentist"), holiday, ev(2, d(10, 7), 630, 60, "Call")];
        let page = &pages(&grid(View::Week, d(10, 7), days), &events, &on_date, &paper())[0];
        let t = text_of(page);
        assert!(t.contains("(Sun 4) Tj") && t.contains("(Sat 10) Tj"), "{t}");
        assert!(t.contains("(Dentist) Tj") && t.contains("(Call) Tj") && t.contains("(Trip) Tj"));
        assert!(t.contains("(10:00 - 11:00) Tj"), "the block of a one hour event names its times");
        assert!(t.contains("(all-day) Tj"), "the band has its label");
        assert!(t.contains("(8a) Tj") && t.contains("(5p) Tj"), "the hour labels of the working day");
    }

    #[test]
    fn overlapping_blocks_share_the_column() {
        let a = ev(1, d(10, 6), 600, 60, "Dentist");
        let b = ev(2, d(10, 6), 630, 60, "Call");
        let page = &pages(&grid(View::Day, d(10, 6), vec![d(10, 6)]), &[a, b], &on_date, &paper())[0];
        let t = text_of(page);
        // the two blocks are filled side by side: two different left edges
        let fills: Vec<&str> = t.lines().filter(|l| l.starts_with("0.900 g ")).collect();
        assert_eq!(fills.len(), 2, "{t}");
        assert_ne!(fills[0].split_whitespace().nth(2), fills[1].split_whitespace().nth(2));
    }

    #[test]
    fn a_year_page_has_twelve_months_and_marks_the_days_with_events() {
        let events = vec![ev(1, d(3, 14), 600, 60, "Pi day")];
        let page = &pages(&grid(View::Year, d(6, 1), vec![]), &events, &on_date, &paper())[0];
        let t = text_of(page);
        for m in MONTHS {
            assert!(t.contains(&format!("({m}) Tj")), "{m}");
        }
        assert_eq!(t.lines().filter(|l| l.starts_with("0.900 g ")).count(), 1, "one day is marked");
    }

    #[test]
    fn an_empty_calendar_still_prints_its_grid() {
        for view in [View::Day, View::Week, View::Month, View::Year] {
            let days = if view == View::Week { (4..=10).map(|n| d(10, n)).collect() } else { vec![d(10, 4)] };
            let pages = pages(&grid(view, d(10, 4), days), &[], &on_date, &paper());
            assert_eq!(pages.len(), 1);
            assert!(!pages[0].content.is_empty());
        }
    }
}
