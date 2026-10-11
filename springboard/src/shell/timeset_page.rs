//! Settings › General › Date & Time › Time: manual date and time entry with
//! five scroll wheels (day, month, year, hour, minute). Shown only while
//! "Set Automatically" is off; Settings applies the Unix time it returns.

use crate::layout::in_rect as within;
use super::clock::picker::Picker;
use super::ui::{Ui, Weight, CARD_BG, LABEL};
use super::wifi_page::BLUE;
use super::Event;
use crate::bare::timezone;
use crate::gfx::{rgba, Surface};

pub const FIRST_YEAR: i64 = 2024;
pub const YEARS: usize = 40;
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const CARD_W_PT: f32 = 460.0;
const CARD_H_PT: f32 = 330.0;
const BAR_PT: f32 = 48.0;
const ROW_PT: f32 = 40.0;
const WHEELS: usize = 5;

#[derive(Clone, Debug, PartialEq)]
pub enum TimeEvent {
    None,
    Cancel,
    /// Unix seconds to set the clock to.
    Set(i64),
    /// Minute of the day, from a time-only picker.
    SetMinute(u16),
}

/// Unix time of a local wall-clock time; `offset_at` gives the zone's UTC
/// offset in seconds at a Unix time. The day is clamped to the month.
pub fn to_unix(y: i64, m: u32, d: u32, hour: u32, min: u32, offset_at: impl Fn(i64) -> i32) -> i64 {
    let d = d.clamp(1, timezone::days_in_month(y, m));
    let local = timezone::days_from_civil(y, m, d) * 86_400 + hour as i64 * 3600 + min as i64 * 60;
    let guess = local - offset_at(local) as i64;
    local - offset_at(guess) as i64
}

pub struct TimePicker {
    w: i32,
    h: i32,
    wheels: [Picker; WHEELS],
    /// only the hour and minute wheels (a time of day, no date)
    day_only: bool,
    title: &'static str,
    /// wheel being dragged
    active: Option<usize>,
    pressed: Option<Press>,
    /// each wheel's labels in white, rendered on first use; fading and the grey tint are
    /// applied while blitting, so a frame rasterises no text
    labels: [Vec<Option<crate::gfx::Image>>; WHEELS],
}

#[derive(Clone, Copy, PartialEq)]
enum Press {
    Cancel,
    Done,
    Outside,
}

impl TimePicker {
    /// Opens at the current local time `now` = (year, month 1-12, day, hour, minute).
    pub fn new(ui: &Ui, w: i32, h: i32, now: (i64, u32, u32, u32, u32)) -> Self {
        let (y, mo, d, hr, mi) = now;
        let row = ui.pt(ROW_PT);
        TimePicker {
            w,
            h,
            wheels: [
                Picker::new(31, row, d as usize - 1),
                Picker::new(12, row, mo as usize - 1),
                Picker::new(YEARS, row, (y - FIRST_YEAR).clamp(0, YEARS as i64 - 1) as usize),
                Picker::new(24, row, hr as usize),
                Picker::new(60, row, mi as usize),
            ],
            active: None,
            pressed: None,
            labels: std::array::from_fn(|_| Vec::new()),
            day_only: false,
            title: "Set Date & Time",
        }
    }

    /// A picker for a time of day only (hour and minute wheels).
    pub fn of_day(ui: &Ui, w: i32, h: i32, minute: u16, title: &'static str) -> Self {
        let mut p = Self::new(ui, w, h, (FIRST_YEAR, 1, 1, (minute / 60 % 24) as u32, (minute % 60) as u32));
        p.day_only = true;
        p.title = title;
        p
    }

    /// The chosen minute of the day.
    pub fn minute(&self) -> u16 {
        (self.wheels[3].selected() * 60 + self.wheels[4].selected()) as u16
    }

    /// The wheels in use.
    fn used(&self) -> std::ops::Range<usize> {
        if self.day_only { 3..WHEELS } else { 0..WHEELS }
    }

    /// The chosen wall-clock time as Unix seconds.
    pub fn value(&self, offset_at: impl Fn(i64) -> i32) -> i64 {
        let s: Vec<usize> = self.wheels.iter().map(Picker::selected).collect();
        to_unix(FIRST_YEAR + s[2] as i64, s[1] as u32 + 1, s[0] as u32 + 1, s[3] as u32, s[4] as u32, offset_at)
    }

    fn card(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let w = ui.pt(CARD_W_PT).min(self.w as f32 - ui.pt(32.0));
        let h = ui.pt(CARD_H_PT);
        ((self.w as f32 - w) / 2.0, (self.h as f32 - h) / 3.0, w, h)
    }

    fn wheel_cx(&self, ui: &Ui, i: usize) -> (f32, f32) {
        let (x, _, w, _) = self.card(ui);
        let pad = ui.pt(16.0);
        let used = self.used();
        let cw = (w - 2.0 * pad) / used.len() as f32;
        (x + pad + cw * (i - used.start) as f32, cw)
    }

    fn wheel_at(&self, ui: &Ui, x: f32) -> Option<usize> {
        self.used().find(|&i| {
            let (cx, cw) = self.wheel_cx(ui, i);
            x >= cx && x < cx + cw
        })
    }

    fn centre_y(&self, ui: &Ui) -> f32 {
        let (_, y, _, h) = self.card(ui);
        y + ui.pt(BAR_PT) + (h - ui.pt(BAR_PT)) / 2.0
    }

    fn cancel_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (x, y, _, _) = self.card(ui);
        (x, y, ui.pt(90.0), ui.pt(BAR_PT))
    }

    fn done_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (x, y, w, _) = self.card(ui);
        (x + w - ui.pt(90.0), y, ui.pt(90.0), ui.pt(BAR_PT))
    }

    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64, offset_at: impl Fn(i64) -> i32) -> TimeEvent {
        let cy = self.centre_y(ui);
        match ev {
            Event::Down(x, y) => {
                let card = self.card(ui);
                self.pressed = if within(x, y, self.cancel_rect(ui)) {
                    Some(Press::Cancel)
                } else if within(x, y, self.done_rect(ui)) {
                    Some(Press::Done)
                } else if !within(x, y, card) {
                    Some(Press::Outside)
                } else {
                    None
                };
                if self.pressed.is_none() && y > card.1 + ui.pt(BAR_PT) {
                    self.active = self.wheel_at(ui, x);
                    if let Some(i) = self.active {
                        self.wheels[i].down(y - cy, t);
                    }
                }
            }
            Event::Move(_, y) => {
                if let Some(i) = self.active {
                    self.wheels[i].moved(y - cy, t);
                }
            }
            Event::Up(x, y) => {
                if let Some(i) = self.active.take() {
                    self.wheels[i].up(y - cy, t);
                }
                let hit = if within(x, y, self.cancel_rect(ui)) {
                    Some(Press::Cancel)
                } else if within(x, y, self.done_rect(ui)) {
                    Some(Press::Done)
                } else if !within(x, y, self.card(ui)) {
                    Some(Press::Outside)
                } else {
                    None
                };
                let pressed = self.pressed.take();
                match pressed {
                    Some(Press::Done) if hit == pressed => return if self.day_only { TimeEvent::SetMinute(self.minute()) } else { TimeEvent::Set(self.value(offset_at)) },
                    Some(p @ (Press::Cancel | Press::Outside)) if hit == Some(p) => return TimeEvent::Cancel,
                    _ => {}
                }
            }
            _ => {}
        }
        TimeEvent::None
    }

    /// Advances snaps; true while a wheel moves.
    pub fn tick(&mut self, t: f64) -> bool {
        self.wheels.iter_mut().fold(false, |a, p| p.tick(t) | a)
    }

    pub fn next_wake(&self, t: f64) -> Option<f64> {
        self.wheels.iter().any(Picker::animating).then_some(t)
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, _t: f64) {
        s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, rgba(0, 0, 0, 130));
        let (x, y, w, h) = self.card(ui);
        s.fill_rrect(x, y, w, h, ui.pt(6.0), CARD_BG.now());
        let mid = y + ui.pt(BAR_PT) / 2.0;
        let title = ui.text(self.title, 17.0, Weight::Semibold, LABEL.now());
        s.blit(&title, (x + (w - title.w as f32) / 2.0) as i32, (mid - title.h as f32 / 2.0) as i32, 255, 255);
        let cancel = ui.text("Cancel", 17.0, Weight::Regular, BLUE);
        s.blit(&cancel, (x + ui.pt(16.0)) as i32, (mid - cancel.h as f32 / 2.0) as i32, 255, 255);
        let done = ui.text("Set", 17.0, Weight::Semibold, BLUE);
        s.blit(&done, (x + w - ui.pt(16.0) - done.w as f32) as i32, (mid - done.h as f32 / 2.0) as i32, 255, 255);

        let cy = self.centre_y(ui);
        let row = ui.pt(ROW_PT);
        s.fill_rrect(x + ui.pt(8.0), cy - row / 2.0, w - ui.pt(16.0), row, ui.pt(6.0), super::ui::FIELD_BG.now());
        let top = y + ui.pt(BAR_PT);
        let bottom = y + h - ui.pt(4.0);
        let mut area = s.rows(top as usize, bottom as usize);
        for i in self.used() {
            let (cx, cw) = self.wheel_cx(ui, i);
            let pos = self.wheels[i].position();
            let count = self.wheels[i].count;
            for k in 0..count {
                let d = (k as f32 - pos).abs();
                if d > 2.6 {
                    continue;
                }
                let ry = cy + (k as f32 - pos) * row;
                let fade = (1.0 - d / 2.6).clamp(0.0, 1.0);
                // the selected row is white; the rest grey (tint) and fading (alpha)
                let (tint, alpha) = if d < 0.5 { (255, 255) } else { (142, (255.0 * fade.powf(1.4)) as u32) };
                let labels = &mut self.labels[i];
                if labels.len() != count {
                    labels.resize_with(count, || None);
                }
                let img = labels[k].get_or_insert_with(|| {
                    let text = match i {
                        0 => (k + 1).to_string(),
                        1 => MONTHS[k].to_string(),
                        2 => (FIRST_YEAR + k as i64).to_string(),
                        _ => format!("{k:02}"),
                    };
                    ui.text(&text, 20.0, Weight::Regular, LABEL.now())
                });
                area.blit(img, (cx + (cw - img.w as f32) / 2.0) as i32, (ry - img.h as f32 / 2.0) as i32, tint, alpha);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_time_converts_to_unix() {
        assert_eq!(to_unix(1970, 1, 1, 0, 0, |_| 0), 0);
        assert_eq!(to_unix(2026, 10, 4, 12, 30, |_| 0), 1_791_117_000);
    }

    #[test]
    fn zone_offset_is_subtracted() {
        assert_eq!(to_unix(2026, 10, 4, 12, 30, |_| 3600), 1_791_117_000 - 3600);
    }

    #[test]
    fn day_is_clamped_to_the_month() {
        assert_eq!(to_unix(2025, 2, 31, 0, 0, |_| 0), to_unix(2025, 2, 28, 0, 0, |_| 0));
        assert_eq!(to_unix(2028, 2, 31, 0, 0, |_| 0), to_unix(2028, 2, 29, 0, 0, |_| 0));
    }
}
