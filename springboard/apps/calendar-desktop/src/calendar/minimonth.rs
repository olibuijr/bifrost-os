//! The calendar list's mini month (Golden Gate Calendar): a month at the bottom of the sidebar with previous and next
//! chevrons, the weekday initials, today's red marker, the selected day's plate and a dot under the days with events. A
//! click on a day shows it; the chevrons page the mini month alone, and it follows the selected day again on the next
//! move of the main view. Drawn with the kit's `widgets::mini_month` (`NativeDatePicker`, graphical style).
//!
//! Apple's mini month also shows the leading and trailing days of the next and previous months in grey; the kit's
//! mini month lays out the month's own days only, so those are left out here.
use super::*;
use bifrost_app::controls::calendar::MiniMonth;
use bifrost_app::controls::Rect;
use bifrost_app::widgets;

/// The mini month's side inset and its gap above the window's bottom edge (points).
const INSET: f32 = SPACE_M;
const BOTTOM_GAP: f32 = SPACE_M;
/// The mini month lays out eight rows (title, weekdays, six weeks) of square cells.
const ROWS: f32 = 8.0;

/// The mini month in a sidebar `side_w` points wide whose window is `win_h` points tall, when it fits below the
/// calendar rows that end at `rows_bottom` (points).
pub(super) fn mini_bounds(side_w: f32, win_h: f32, rows_bottom: f32) -> Option<Rect> {
    let w = side_w - 2.0 * INSET;
    let h = w / 7.0 * ROWS;
    let y = win_h - BOTTOM_GAP - h;
    (w > 0.0 && y >= rows_bottom + SPACE_S).then_some(Rect { x: INSET, y, w, h })
}

impl Calendar {
    /// The first day of the month the mini month shows: the paged one, or the selected day's month.
    pub(super) fn mini_month_shown(&self) -> Date {
        self.mini_shown.unwrap_or(Date { day: 1, ..self.selected })
    }

    /// The mini month's place (points, window coordinates), while the calendar list is shown and there is room.
    pub(super) fn mini_rect(&self, ui: &Ui) -> Option<Rect> {
        if self.x0 <= 0.0 {
            return None;
        }
        let k = ui.pt(1.0);
        mini_bounds(self.x0 / k, self.h as f32 / k, self.sidebar_bottom(ui))
    }

    pub(super) fn mini_layout(&self, ui: &Ui) -> Option<MiniMonth> {
        let r = self.mini_rect(ui)?;
        let m = self.mini_month_shown();
        Some(MiniMonth::new(r, m.year as i32, m.month, self.prefs.weekday_at(0) as u32))
    }

    /// The mini month's target under window pixel `(x, y)`: a chevron, or a day of the shown month.
    pub(super) fn mini_target(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let l = self.mini_layout(ui)?;
        let k = ui.pt(1.0);
        let (px, py) = (x / k, y / k);
        if l.prev.contains(px, py) {
            return Some(Target::MiniPrev);
        }
        if l.next.contains(px, py) {
            return Some(Target::MiniNext);
        }
        let m = self.mini_month_shown();
        l.hit_day(px, py).map(|day| Target::MiniDay(Date { day, ..m }))
    }

    /// The chevrons page the mini month by a month; a day shows that day in the main view.
    pub(super) fn mini_tap(&mut self, target: Target) {
        match target {
            Target::MiniPrev | Target::MiniNext => {
                let step = if target == Target::MiniPrev { -1 } else { 1 };
                self.mini_shown = Some(self.mini_month_shown().shift_months(step));
            }
            Target::MiniDay(day) => {
                self.selected = day;
                self.mini_shown = None;
                self.scroll.jump_top();
            }
            _ => {}
        }
    }

    /// The days of the shown month that have an event shown in the grid (the mini month's dots).
    pub(super) fn mini_marked(&self, m: Date) -> Vec<u32> {
        (1..=m.month_len()).filter(|d| {
            let date = Date { day: *d, ..m };
            self.shown_events().any(|e| self.occurs(&e, date))
        }).collect()
    }

    /// Draws the mini month in the calendar list (`draw_chrome` calls it after the source list).
    pub(super) fn draw_mini_month(&self, s: &mut Surface, ui: &Ui) {
        let Some(r) = self.mini_rect(ui) else { return };
        let m = self.mini_month_shown();
        let same = |d: Date| (d.year == m.year && d.month == m.month).then_some(d.day);
        let title = format!("{} {}", MONTHS[(m.month - 1) as usize], m.year);
        let marked = self.mini_marked(m);
        widgets::mini_month(s, ui, r, m.year as i32, m.month, self.prefs.weekday_at(0) as u32, &title, same(self.today), same(self.selected), &marked);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mini_month_sits_at_the_bottom_of_the_sidebar_when_it_fits() {
        let r = mini_bounds(200.0, 780.0, 300.0).expect("room under the calendar rows");
        assert_eq!(r.x, INSET);
        assert!((r.y + r.h - (780.0 - BOTTOM_GAP)).abs() < 0.01, "on the bottom edge");
        assert!((r.w / 7.0 - r.h / ROWS).abs() < 0.01, "square cells");
        assert!(mini_bounds(200.0, 400.0, 300.0).is_none(), "a short window keeps the calendar rows");
        assert!(mini_bounds(0.0, 780.0, 0.0).is_none(), "no sidebar");
    }
}
