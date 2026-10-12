//! Day, Week and Year views (the Day/Week/Month/Year segmented group is in the toolbar, `chrome.rs`).
use bifrost_app::kit::ui::{MAC_CAPTION1_PT};
use bifrost_app::layout::top_inset;
use bifrost_app::kit::ui::{SPACE_HUGE, SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XS, SPACE_XXL, SPACE_XXS, SPACE_XXXL};
use bifrost_app::kit::ui::MAC_SUBHEAD_PT;
use springboard::layout::in_rect as within;
use super::layout;
use super::store::CalendarEvent;
use super::{
    red, Calendar, Date, Target, MONTHS, WEEKDAYS,
};
use springboard::gfx::Surface;
use springboard::shell::clock::tz;
use springboard::shell::ui::{Ui, Weight, WHITE};
use bifrost_app::kit::ui::{CARD_BG, CONTENT_BG, FIELD_BG, LABEL, LABEL_2, SEPARATOR_C};
use bifrost_app::kit::ui::{MAC_BODY_PT, MAC_CALLOUT_PT, MAC_TITLE3_PT};
use super::chrome::cal_colour;

const GRID0: f32 = 192.0;
/// Week view: the weekday names, the day badge and the day number (points below the top).
const WEEKDAY_Y_PT: f32 = 150.0;
const DAY_BADGE_Y_PT: f32 = 158.0;
const DAY_NUM_Y_PT: f32 = 173.0;
/// Year view: the top of the month grid.
const YEAR_TOP_PT: f32 = 140.0;
const ALLDAY_H: f32 = 28.0;
const HOUR_H: f32 = 48.0;
const GUTTER: f32 = 52.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Day,
    Week,
    Month,
    Year,
}

impl View {
    pub fn name(self) -> &'static str {
        match self {
            View::Day => "day",
            View::Week => "week",
            View::Month => "month",
            View::Year => "year",
        }
    }
    pub(super) fn label(self) -> &'static str {
        match self {
            View::Day => "Day",
            View::Week => "Week",
            View::Month => "Month",
            View::Year => "Year",
        }
    }
}

fn hour_label(h: u32) -> String {
    match h {
        0 => "12 AM".into(),
        1..=11 => format!("{h} AM"),
        12 => "Noon".into(),
        _ => format!("{} PM", h - 12),
    }
}

impl Calendar {
    pub(super) fn time_view(&self) -> bool {
        matches!(self.view, View::Day | View::Week)
    }

    /// Days shown as columns: the selected day, or its Sunday-first week.
    fn view_days(&self) -> Vec<Date> {
        if self.view == View::Day {
            return vec![self.selected];
        }
        let start = self.selected.shift_days(-(self.selected.weekday() as i32));
        (0..7).map(|i| start.shift_days(i)).collect()
    }

    fn week_start(&self) -> Date {
        self.selected.shift_days(-(self.selected.weekday() as i32))
    }

    /// Scroll range of the hour grid.
    pub(super) fn time_scroll_max(&self, ui: &Ui) -> f32 {
        ui.pt(GRID0 + ALLDAY_H + 24.0 * HOUR_H + 16.0) - self.bottom(ui)
    }

    /// Initial offset showing the morning (8 AM) at the top.
    pub(super) fn scroll_to_morning(&mut self, ui: &Ui) {
        self.scroll.jump_to(ui.pt(8.0 * HOUR_H));
    }

    /// Steps by the active view's unit.
    pub(super) fn step_view(&mut self, dir: i32) {
        self.selected = match self.view {
            View::Day => self.selected.shift_days(dir),
            View::Week => self.selected.shift_days(7 * dir),
            View::Month => self.selected.shift_months(dir),
            View::Year => self.selected.shift_months(12 * dir),
        };
    }

    fn col_geometry(&self, ui: &Ui) -> (f32, f32) {
        let n = if self.view == View::Day { 1.0 } else { 7.0 };
        let gut = ui.pt(GUTTER);
        (gut, (self.w as f32 - gut - ui.pt(SPACE_S)) / n)
    }

    /// Screen rectangles `(id, x, y, w, h)` of every event in the grid.
    fn time_rects(&self, ui: &Ui) -> Vec<(u64, f32, f32, f32, f32)> {
        let (gut, col_w) = self.col_geometry(ui);
        let top = ui.pt(GRID0) - self.scroll.offset();
        let hours_top = top + ui.pt(ALLDAY_H);
        let hh = ui.pt(HOUR_H);
        let mut out = vec![];
        for (i, d) in self.view_days().iter().enumerate() {
            let x0 = gut + col_w * i as f32;
            let (all, timed): (Vec<&CalendarEvent>, Vec<&CalendarEvent>) = self
                .events
                .iter()
                .filter(|e| e.occurs_on(*d))
                .partition(|e| e.all_day);
            let spans: Vec<(u32, u32)> = all.iter().map(|_| (0, 1)).collect();
            for (e, slot) in all.iter().zip(layout::layout(&spans, 1)) {
                let (x, w) = layout::slot_x(slot, x0, col_w, ui.pt(SPACE_XXS));
                out.push((e.id, x, top + ui.pt(SPACE_XXS), w, ui.pt(ALLDAY_H - 4.0)));
            }
            let spans: Vec<(u32, u32)> = timed
                .iter()
                .map(|e| (e.minute as u32, (e.minute as u32 + e.duration as u32).min(1440)))
                .collect();
            for ((e, span), slot) in timed.iter().zip(&spans).zip(layout::layout(&spans, 30)) {
                let (x, w) = layout::slot_x(slot, x0, col_w, ui.pt(SPACE_XXS));
                let y = layout::minute_y(span.0 as f32, hours_top, hh);
                let h = ((span.1 - span.0) as f32 / 60.0 * hh - ui.pt(1.0)).max(ui.pt(SPACE_L));
                out.push((e.id, x, y, w, h));
            }
        }
        out
    }

    /// Hit test for the Day/Week/Year screens (the toolbar and the header capsules are `chrome::bar_target`).
    pub(super) fn hit_view(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        if self.view == View::Month {
            return None;
        }
        let xp = x / ui.s;
        let yp = y / ui.s;
        let wp = self.w as f32 / ui.s;
        if self.view == View::Year {
            if yp >= 140.0 {
                let (cols, rows) = self.year_dims();
                let cw = (wp - 40.0) / cols as f32;
                let ch = (self.bottom(ui) / ui.s - 140.0) / rows as f32;
                let col = (((xp - 20.0) / cw).floor() as i32).clamp(0, cols as i32 - 1) as u32;
                let row = (((yp - 140.0) / ch).floor() as i32).clamp(0, rows as i32 - 1) as u32;
                return Some(Target::MonthOf(row * cols + col + 1));
            }
            return None;
        }
        if (134.0..GRID0).contains(&yp) {
            let col = if self.view == View::Week {
                let (gut, col_w) = self.col_geometry(ui);
                (((x - gut) / col_w).floor() as i32).clamp(0, 6)
            } else {
                ((x / (self.w as f32 / 7.0)) as i32).clamp(0, 6)
            };
            return Some(Target::Pick(self.week_start().shift_days(col)));
        }
        if yp >= GRID0 {
            for (id, rx, ry, rw, rh) in self.time_rects(ui) {
                if within(x, y, (rx, ry, rw, rh)) {
                    return Some(Target::Row(id));
                }
            }
            return Some(Target::Agenda);
        }
        None
    }

    fn year_dims(&self) -> (u32, u32) {
        if self.w > self.h {
            (4, 3)
        } else {
            (3, 4)
        }
    }

    pub(super) fn range_label(&self) -> String {
        match self.view {
            View::Day => format!(
                "{}, {} {}",
                WEEKDAYS[self.selected.weekday()],
                MONTHS[(self.selected.month - 1) as usize],
                self.selected.day
            ),
            View::Week => {
                let a = self.week_start();
                let b = a.shift_days(6);
                let m = |d: Date| &MONTHS[(d.month - 1) as usize][..3];
                if a.month == b.month {
                    format!("{} {} - {}, {}", m(a), a.day, b.day, b.year)
                } else {
                    format!("{} {} - {} {}, {}", m(a), a.day, m(b), b.day, b.year)
                }
            }
            _ => self.selected.year.to_string(),
        }
    }

    pub(super) fn draw_time(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        let w = self.w as f32;
        let bottom = self.bottom(ui);
        let (gut, col_w) = self.col_geometry(ui);
        let days = self.view_days();
        let n = days.len();
        let off = self.scroll.offset();
        let top = ui.pt(GRID0) - off;
        let hours_top = top + ui.pt(ALLDAY_H);
        let hh = ui.pt(HOUR_H);
        let rects = self.time_rects(ui);
        let c = tz::civil(self.wall.now(t), self.wall.offset);
        let now_min = c.hour * 60 + c.min;
        {
            let mut g = s.rows(ui.pt(GRID0) as usize, bottom as usize);
            g.fill_rrect(0.0, ui.pt(GRID0), w, bottom - ui.pt(GRID0), 0.0, CONTENT_BG.now());
            for h in 0..=24u32 {
                let y = hours_top + hh * h as f32;
                g.fill_rrect(gut, y, w - gut, ui.pt(0.5), 0.0, SEPARATOR_C.now());
                if (1..24).contains(&h) {
                    self.text(&mut g, ui, &hour_label(h), MAC_SUBHEAD_PT, LABEL_2.now(), ui.pt(SPACE_S), y);
                }
            }
            g.fill_rrect(gut, top + ui.pt(ALLDAY_H), w - gut, ui.pt(0.5), 0.0, SEPARATOR_C.now());
            self.text(&mut g, ui, "all-day", MAC_SUBHEAD_PT, LABEL_2.now(), ui.pt(SPACE_S), top + ui.pt(ALLDAY_H / 2.0));
            if n > 1 {
                for i in 0..=n {
                    g.fill_rrect(
                        gut + col_w * i as f32,
                        top,
                        ui.pt(0.5),
                        ui.pt(ALLDAY_H) + hh * 24.0,
                        0.0,
                        SEPARATOR_C.now(),
                    );
                }
            }
            let by_id: Vec<(u64, String, u16, bool)> = self
                .events
                .iter()
                .map(|e| (e.id, e.title.clone(), e.minute, e.all_day))
                .collect();
            for (id, x, y, rw, rh) in rects {
                let Some((_, title, minute, all_day)) = by_id.iter().find(|e| e.0 == id) else {
                    continue;
                };
                g.fill_rrect(x, y, rw, rh, ui.pt(SPACE_XS), FIELD_BG.now());
                g.fill_rrect(x, y, ui.pt(SPACE_XS), rh, ui.pt(SPACE_XS) / 2.0, cal_colour());
                let text_w = rw - ui.pt(SPACE_M);
                if text_w > ui.pt(SPACE_L) && rh >= ui.pt(SPACE_L) {
                    let fit = ui.fit(title, MAC_CAPTION1_PT, text_w);
                    self.text(&mut g, ui, &fit, MAC_CALLOUT_PT, LABEL.now(), x + ui.pt(SPACE_S), y + ui.pt(SPACE_S));
                    if !*all_day && rh >= ui.pt(SPACE_XXXL) {
                        let tm = super::time_label(*minute);
                        self.text(&mut g, ui, &tm, MAC_SUBHEAD_PT, LABEL_2.now(), x + ui.pt(SPACE_S), y + ui.pt(SPACE_XXL));
                    }
                }
            }
            if let Some(i) = days.iter().position(|d| *d == self.today) {
                if let Some(y) = layout::now_line_y(now_min, hours_top, hh) {
                    g.fill_rrect(gut + col_w * i as f32, y, col_w, ui.pt(1.5), 0.0, red());
                    g.fill_rrect(
                        gut + col_w * i as f32 - ui.pt(SPACE_XS),
                        y - ui.pt(SPACE_XS),
                        ui.pt(SPACE_S),
                        ui.pt(SPACE_S),
                        ui.pt(SPACE_XS),
                        red(),
                    );
                }
            }
        }
        s.fill_rrect(0.0, ui.pt(top_inset()), w, ui.pt(GRID0 - top_inset()), 0.0, CONTENT_BG.now());
        self.draw_header(s, ui);
        let start = self.week_start();
        for i in 0..7 {
            let d = start.shift_days(i);
            let cx = if self.view == View::Week {
                gut + col_w * (i as f32 + 0.5)
            } else {
                w / 7.0 * (i as f32 + 0.5)
            };
            let name = &WEEKDAYS[i as usize][..3].to_uppercase();
            self.centre(s, ui, name, MAC_SUBHEAD_PT, Weight::Medium, LABEL_2.now(), cx, ui.pt(WEEKDAY_Y_PT));
            let sel = d == self.selected;
            let today = d == self.today;
            if sel || today {
                s.fill_rrect(
                    cx - ui.pt(SPACE_L),
                    ui.pt(DAY_BADGE_Y_PT),
                    ui.pt(30.0),
                    ui.pt(30.0),
                    ui.pt(SPACE_L),
                    if today { red() } else { CARD_BG.now() },
                );
            }
            self.centre(
                s,
                ui,
                &d.day.to_string(),
                MAC_TITLE3_PT,
                if sel { Weight::Semibold } else { Weight::Regular },
                if today { WHITE } else { LABEL.now() },
                cx,
                ui.pt(DAY_NUM_Y_PT),
            );
        }
        s.fill_rrect(0.0, ui.pt(GRID0) - ui.pt(0.5), w, ui.pt(0.5), 0.0, SEPARATOR_C.now());
    }

    pub(super) fn draw_year(&mut self, s: &mut Surface, ui: &Ui) {
        self.draw_header(s, ui);
        let w = self.w as f32;
        let (cols, rows) = self.year_dims();
        let cw = (w - ui.pt(SPACE_HUGE)) / cols as f32;
        let ch = (self.bottom(ui) - ui.pt(YEAR_TOP_PT)) / rows as f32;
        for m in 1..=12u32 {
            let idx = m - 1;
            let x0 = ui.pt(SPACE_XL) + cw * (idx % cols) as f32;
            let y0 = ui.pt(YEAR_TOP_PT) + ch * (idx / cols) as f32;
            let current = self.today.year == self.selected.year && self.today.month == m;
            self.text(
                s,
                ui,
                MONTHS[idx as usize],
                MAC_BODY_PT,
                if current { red() } else { LABEL.now() },
                x0 + ui.pt(SPACE_S),
                y0 + ui.pt(SPACE_M),
            );
            let first = Date { year: self.selected.year, month: m, day: 1 };
            let mini_w = (cw - ui.pt(SPACE_L)) / 7.0;
            let mini_h = ((ch - ui.pt(36.0)) / 6.0).min(ui.pt(SPACE_XXL));
            for day in 1..=first.month_len() {
                let col = (first.weekday() + day as usize - 1) % 7;
                let row = (first.weekday() + day as usize - 1) / 7;
                let cx = x0 + ui.pt(SPACE_S) + mini_w * (col as f32 + 0.5);
                let cy = y0 + ui.pt(36.0) + mini_h * (row as f32 + 0.5);
                let is_today = current && day == self.today.day;
                if is_today {
                    let r = mini_h.min(mini_w) / 2.0;
                    s.fill_rrect(cx - r, cy - r, r * 2.0, r * 2.0, r, red());
                }
                self.centre(
                    s,
                    ui,
                    &day.to_string(),
                    MAC_CAPTION1_PT,
                    Weight::Regular,
                    if is_today { WHITE } else { LABEL.now() },
                    cx,
                    cy,
                );
            }
        }
    }
}
