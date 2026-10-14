//! Day, Week and Year views (the Day/Week/Month/Year segmented group is in the toolbar, `chrome.rs`).
use bifrost_app::kit::ui::{MAC_CAPTION1_PT};
use bifrost_app::layout::top_inset;
use bifrost_app::kit::ui::{SPACE_HUGE, SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XS, SPACE_XXL, SPACE_XXS, SPACE_XXXL};
use bifrost_app::kit::ui::MAC_SUBHEAD_PT;
use springboard::layout::in_rect as within;
use super::layout;
use super::store::{self, CalendarEvent};
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
/// The hour row's height in points at the default zoom; View > Zoom In / Out step it by `ZOOM_STEP` between the limits.
pub(super) const HOUR_H: f32 = 48.0;
const ZOOM_STEP: f32 = 12.0;
const ZOOM_MIN: f32 = 24.0;
const ZOOM_MAX: f32 = 96.0;

/// The hour height after one zoom step (`dir` > 0 zooms in, < 0 out), kept between the limits.
pub(super) fn zoomed(hour_h: f32, dir: i32) -> f32 {
    (hour_h + ZOOM_STEP * dir.signum() as f32).clamp(ZOOM_MIN, ZOOM_MAX)
}
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

/// What a drag in the Day or Week grid does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum DragKind {
    /// Sizing a new event from the minute that was pressed.
    Create { press_min: u32 },
    /// Moving a saved event; `grab` is the minutes between its start and the pressed point.
    Move { id: u64, grab: u32, duration: u32 },
}

/// A drag in progress. It only counts as one once the pointer has moved (`moved`); a plain tap stays a tap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Drag {
    pub kind: DragKind,
    pub date: Date,
    pub span: layout::DragSpan,
    pub moved: bool,
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

    /// Days shown as columns: the selected day, or its week.
    fn view_days(&self) -> Vec<Date> {
        if self.view == View::Day {
            return vec![self.selected];
        }
        let start = self.week_start();
        (0..7).map(|i| start.shift_days(i)).collect()
    }

    /// The first day of the shown week (Settings > Week starts on).
    fn week_start(&self) -> Date {
        self.selected.shift_days(-self.lead_days(self.selected))
    }

    /// Scroll range of the hour grid.
    pub(super) fn time_scroll_max(&self, ui: &Ui) -> f32 {
        ui.pt(GRID0 + ALLDAY_H + 24.0 * self.hour_h + 16.0) - self.bottom(ui)
    }

    /// Initial offset showing the day's start hour (Settings > Day starts at) at the top.
    pub(super) fn scroll_to_morning(&mut self, ui: &Ui) {
        self.scroll.jump_to(ui.pt(self.prefs.day_start as f32 * self.hour_h));
    }

    /// View > Zoom In / Zoom Out: changes the hour height and keeps the hour at the top of the grid where it was.
    pub(super) fn zoom(&mut self, ui: &Ui, dir: i32) {
        let next = zoomed(self.hour_h, dir);
        if next == self.hour_h {
            return;
        }
        let offset = self.scroll.offset() * next / self.hour_h;
        self.hour_h = next;
        self.scroll.set_max(self.time_scroll_max(ui));
        self.scroll.jump_to(offset);
    }

    /// Whether another zoom step in `dir` is possible (the menu items dim at the limits).
    pub(super) fn can_zoom(&self, dir: i32) -> bool {
        self.time_view() && zoomed(self.hour_h, dir) != self.hour_h
    }

    /// The minute of the day under pixel row `y` (1 minute resolution), clamped to the day.
    fn minute_under(&self, ui: &Ui, y: f32) -> u32 {
        let hours_top = ui.pt(GRID0) - self.scroll.offset() + ui.pt(ALLDAY_H);
        layout::y_minute(y, hours_top, ui.pt(self.hour_h), 1)
    }

    /// The day column under pixel `x`.
    fn date_at(&self, ui: &Ui, x: f32) -> Date {
        if self.view != View::Week {
            return self.selected;
        }
        let (gut, col_w) = self.col_geometry(ui);
        self.week_start().shift_days(layout::column_at(x, gut, col_w, 7) as i32)
    }

    /// A press in the Day or Week hour rows: on empty space it starts sizing a new event, on a timed event that does not
    /// repeat it starts moving it. The hour gutter and the all-day row still scroll.
    pub(super) fn drag_start(&self, ui: &Ui, x: f32, y: f32, pressed: Option<Target>) -> Option<Drag> {
        if !self.time_view() || self.editor.is_some() || self.goto.is_some() || self.settings || self.search.is_some() || self.storage_blocked {
            return None;
        }
        let (gut, _) = self.col_geometry(ui);
        let hours_top = ui.pt(GRID0) - self.scroll.offset() + ui.pt(ALLDAY_H);
        if x < gut || y < hours_top {
            return None;
        }
        let minute = self.minute_under(ui, y);
        match pressed? {
            Target::Agenda => Some(Drag {
                kind: DragKind::Create { press_min: minute },
                date: self.date_at(ui, x),
                span: layout::drag_create(minute, minute, 15),
                moved: false,
            }),
            Target::Row(id) => {
                let e = self.events.iter().find(|e| e.id == id)?;
                if e.all_day || e.repeat != 0 {
                    return None;
                }
                let (duration, grab) = (e.duration as u32, minute.saturating_sub(e.minute as u32));
                Some(Drag {
                    kind: DragKind::Move { id, grab, duration },
                    date: e.date,
                    span: layout::drag_move(duration, grab, minute, 15),
                    moved: false,
                })
            }
            _ => None,
        }
    }

    /// The pointer moved: follows it once it left the press point's slop. True when the drag is under way.
    pub(super) fn drag_update(&mut self, ui: &Ui, x: f32, y: f32) -> bool {
        let Some(mut d) = self.drag else { return false };
        if !d.moved {
            let (sx, sy) = self.start.unwrap_or((x, y));
            if (x - sx).hypot(y - sy) <= ui.pt(SPACE_M) {
                return false;
            }
            d.moved = true;
        }
        let now = self.minute_under(ui, y);
        match d.kind {
            DragKind::Create { press_min } => d.span = layout::drag_create(press_min, now, 15),
            DragKind::Move { grab, duration, .. } => {
                d.span = layout::drag_move(duration, grab, now, 15);
                d.date = self.date_at(ui, x);
            }
        }
        self.drag = Some(d);
        true
    }

    /// The pointer lifted after a drag: a sized span opens the new event's editor, a moved event is saved at its new time.
    pub(super) fn drag_finish(&mut self, ui: &Ui, x: f32, y: f32) {
        self.drag_update(ui, x, y);
        let Some(d) = self.drag.take() else { return };
        self.dirty = true;
        match d.kind {
            DragKind::Create { .. } => {
                let event = self.blank_event(d.date, d.span.start as u16, (d.span.end - d.span.start) as u16);
                self.selected = d.date;
                self.open_draft(ui, event, false);
                self.fit_scroll(ui);
            }
            DragKind::Move { id, .. } => self.move_event(id, d.date, d.span.start as u16),
        }
    }

    /// Saves the event `id` at a new date and start time, keeping its length.
    pub(super) fn move_event(&mut self, id: u64, date: Date, minute: u16) {
        let mut events = self.events.clone();
        let Some(e) = events.iter_mut().find(|e| e.id == id) else { return };
        if e.date == date && e.minute == minute {
            return;
        }
        e.date = date;
        e.minute = minute;
        store::sort(&mut events);
        match store::save(&self.path, &events) {
            Ok(()) => {
                self.events = events;
                self.changes.post();
                self.selected = date;
                self.error = None;
            }
            Err(e) => {
                eprintln!("springboard: calendar: {e}");
                self.error = Some("Could not save. The event stays where it was.".into());
            }
        }
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
        let hh = ui.pt(self.hour_h);
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
        let hh = ui.pt(self.hour_h);
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
            let moving = match self.drag {
                Some(Drag { kind: DragKind::Move { id, .. }, moved: true, .. }) => Some(id),
                _ => None,
            };
            for (id, x, y, rw, rh) in rects {
                if moving == Some(id) {
                    continue;
                }
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
            // the block being dragged: a new event being sized, or the event being moved
            if let Some(d) = self.drag.filter(|d| d.moved) {
                if let Some(i) = days.iter().position(|x| *x == d.date) {
                    let bx = gut + col_w * i as f32 + ui.pt(SPACE_XXS);
                    let by = layout::minute_y(d.span.start as f32, hours_top, hh);
                    let bh = (d.span.end - d.span.start) as f32 / 60.0 * hh;
                    g.fill_rrect(bx, by, col_w - ui.pt(SPACE_XS), bh, ui.pt(SPACE_XS), CARD_BG.now());
                    g.fill_rrect(bx, by, ui.pt(SPACE_XS), bh, ui.pt(SPACE_XS) / 2.0, cal_colour());
                    let label = format!("{} - {}", super::time_label(d.span.start as u16), super::time_label(d.span.end as u16));
                    self.text(&mut g, ui, &label, MAC_CALLOUT_PT, LABEL.now(), bx + ui.pt(SPACE_S), by + ui.pt(SPACE_S));
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
            let name = &WEEKDAYS[(i as usize + self.first_weekday()) % 7][..3].to_uppercase();
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

#[cfg(test)]
mod zoom_tests {
    use super::*;

    #[test]
    fn zoom_steps_and_clamps() {
        assert_eq!(zoomed(48.0, 1), 60.0);
        assert_eq!(zoomed(48.0, -1), 36.0);
        assert_eq!(zoomed(96.0, 1), 96.0);
        assert_eq!(zoomed(24.0, -1), 24.0);
        assert_eq!(zoomed(48.0, 0), 48.0);
    }
}

#[cfg(test)]
mod drag_tests {
    use super::*;
    use springboard::shell::clock::Wall;

    fn calendar(name: &str) -> (Calendar, Ui, std::path::PathBuf) {
        let ui = crate::test_ui();
        let dir = std::env::temp_dir().join(format!("springboard-calendar-drag-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let c = Calendar::new(&ui, 1200, 1920, dir.join("events.tsv"), Wall { unix: 0, offset: 0, at: 0.0 });
        (c, ui, dir)
    }

    fn event(selected: Date, repeat: u8, all_day: bool) -> CalendarEvent {
        CalendarEvent {
            id: 1,
            date: selected,
            minute: 540,
            duration: 60,
            all_day,
            title: "Dentist".into(),
            location: String::new(),
            notes: String::new(),
            repeat,
            alert: 0,
        }
    }

    /// Pixel row of `minute` in the hour grid, a little below its line so rounding cannot tip it into the minute before.
    fn y_of(c: &Calendar, ui: &Ui, minute: f32) -> f32 {
        let hours_top = ui.pt(GRID0) - c.scroll.offset() + ui.pt(ALLDAY_H);
        hours_top + ui.pt(c.hour_h) * minute / 60.0 + 2.0
    }

    #[test]
    fn dragging_empty_space_sizes_a_new_event_and_opens_the_editor() {
        let (mut c, ui, dir) = calendar("create");
        c.activate(&ui, Target::View(View::Day), 0.0);
        let x = ui.pt(GUTTER + 100.0);
        let (y0, y1) = (y_of(&c, &ui, 600.0), y_of(&c, &ui, 690.0));
        c.start = Some((x, y0));
        c.drag = c.drag_start(&ui, x, y0, Some(Target::Agenda));
        assert!(c.drag.is_some());
        assert!(!c.drag_update(&ui, x, y0), "a press without movement is a tap");
        assert!(c.drag_update(&ui, x, y1));
        c.drag_finish(&ui, x, y1);
        let draft = c.editor.as_ref().expect("the editor opens on the sized event");
        assert!(!draft.existing);
        assert_eq!((draft.event.minute, draft.event.duration), (600, 105));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn dragging_an_event_moves_it_and_keeps_its_length() {
        let (mut c, ui, dir) = calendar("move");
        c.activate(&ui, Target::View(View::Day), 0.0);
        c.events = vec![event(c.selected, 0, false)];
        store::save(&c.path, &c.events).unwrap();
        let x = ui.pt(GUTTER + 100.0);
        let (y0, y1) = (y_of(&c, &ui, 555.0), y_of(&c, &ui, 675.0));
        c.start = Some((x, y0));
        c.drag = c.drag_start(&ui, x, y0, Some(Target::Row(1)));
        assert!(matches!(c.drag, Some(Drag { kind: DragKind::Move { id: 1, .. }, .. })));
        assert!(c.drag_update(&ui, x, y1));
        c.drag_finish(&ui, x, y1);
        assert_eq!((c.events[0].minute, c.events[0].duration), (660, 60));
        assert_eq!(store::load(&c.path).unwrap()[0].minute, 660, "the move is saved");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn recurring_and_all_day_events_are_not_dragged_and_the_gutter_still_scrolls() {
        let (mut c, ui, dir) = calendar("skip");
        c.activate(&ui, Target::View(View::Day), 0.0);
        let y = y_of(&c, &ui, 555.0);
        c.events = vec![event(c.selected, 2, false)];
        assert!(c.drag_start(&ui, ui.pt(GUTTER + 100.0), y, Some(Target::Row(1))).is_none());
        c.events = vec![event(c.selected, 0, true)];
        assert!(c.drag_start(&ui, ui.pt(GUTTER + 100.0), y, Some(Target::Row(1))).is_none());
        assert!(c.drag_start(&ui, ui.pt(GUTTER - 10.0), y, Some(Target::Agenda)).is_none(), "the hour gutter scrolls");
        let _ = std::fs::remove_dir_all(dir);
    }
}
