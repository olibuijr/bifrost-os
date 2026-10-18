//! The lock screen's widget strip, under the clock: the next calendar event
//! and the battery. (The Now Playing card, with its transport controls, sits
//! right under the strip and the notification stacks under that.) The strip
//! only shows; a tap on it goes through the passcode flow like a tap on a
//! notification: unlock (asking for the passcode if one is set), then open
//! the app. While locked behind a passcode the event title is hidden.

use super::super::calendar::store::CalendarEvent;
use super::super::calendar::Date;
use super::super::ui::{Ui, Weight, SECONDARY, WHITE};
use super::super::AppId;
use super::list::{CARD, CARD_W_PT};
use crate::gfx::{pack, Image, Surface};

/// Top of the strip, under the clock.
pub const TOP_PT: f32 = 205.0;
pub const H_PT: f32 = 60.0;
/// between the strip and whatever follows it
pub const BELOW_PT: f32 = 10.0;
const GAP_PT: f32 = 8.0;
const BATTERY_W_PT: f32 = 104.0;
const RADIUS_PT: f32 = 6.0;
const PAD_PT: f32 = 14.0;
const GREEN: u32 = pack(48, 209, 88, 255);
const RED: u32 = pack(255, 69, 58, 255);

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// What a finger can land on in the strip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Event,
    Battery,
    /// the weather slot (see `weather_slot`); the strip has no panel for it yet
    Weather,
}

impl Target {
    /// The app a tap opens, once unlocked.
    pub fn app(self) -> AppId {
        match self {
            Target::Event => super::super::apphost::slot_of("is.olibuijr.calendar").map_or(AppId::Settings, |i| AppId::External(i as u8)),
            Target::Battery => AppId::Settings,
            Target::Weather => crate::shell::apphost::slot_of("is.olibuijr.weather").map_or(AppId::Settings, |n| AppId::External(n as u8)),
        }
    }
}

/// The weather slot hook: what a lock screen weather panel would show, from
/// the summary the Weather app saved in `dir` (None until it has run). The
/// text is "Reykjavik 7\u{b0} Rain"; hidden behind a passcode it is only
/// "Weather", like the calendar event title.
pub fn weather_slot(dir: &std::path::Path, private: bool) -> Option<String> {
    let summary = super::super::weather::data::Summary::load(dir)?;
    Some(if private { "Weather".to_string() } else { summary.line() })
}

/// The first event that has not ended yet at `minute` of `today` (all-day
/// events of today count), earliest first.
pub fn next_event(events: &[CalendarEvent], today: Date, minute: u16) -> Option<&CalendarEvent> {
    events
        .iter()
        .filter(|e| e.date > today || (e.date == today && (e.all_day || e.minute + e.duration > minute)))
        .min_by_key(|e| (e.date, !e.all_day, e.minute))
}

/// "Today 14:30", "Tomorrow, all day" or "Mon 5 Oct 09:00".
pub fn when_label(e: &CalendarEvent, today: Date) -> String {
    let day = if e.date == today {
        "Today".to_string()
    } else if e.date == today.shift_days(1) {
        "Tomorrow".to_string()
    } else {
        format!("{} {} {}", DAYS[e.date.weekday() % 7], e.date.day, MONTHS[(e.date.month as usize - 1) % 12])
    };
    if e.all_day {
        format!("{day}, all day")
    } else {
        format!("{day} {:02}:{:02}", e.minute / 60, e.minute % 60)
    }
}

/// The strip rendered for one battery level and next event.
pub struct Strip {
    title: Image,
    when: Option<Image>,
    percent: Image,
    state: Image,
}

impl Strip {
    /// `event` is (title, when) of the next event; `private` hides its title.
    pub fn new(ui: &Ui, event: Option<(&str, &str)>, private: bool, battery: u8, charging: bool) -> Self {
        let text_w = ui.pt(CARD_W_PT - BATTERY_W_PT - GAP_PT - 2.0 * PAD_PT);
        let (title, when) = match event {
            Some((t, w)) => (if private { "Calendar event" } else { t }, Some(ui.text(w, 13.0, Weight::Regular, SECONDARY))),
            None => ("No upcoming events", None),
        };
        let colour = if charging { GREEN } else if battery <= 20 { RED } else { WHITE };
        Strip {
            title: ui.text(&ui.fit(title, 15.0, text_w), 15.0, if when.is_some() { Weight::Semibold } else { Weight::Regular }, if when.is_some() { WHITE } else { SECONDARY }),
            when,
            percent: ui.text(&format!("{battery}%"), 20.0, Weight::Semibold, colour),
            state: ui.text(if charging { "Charging" } else { "Battery" }, 12.0, Weight::Regular, SECONDARY),
        }
    }

    /// (event panel, battery panel) as (x, y, w, h) on a `w` px wide screen, lifted by nothing.
    fn rects(ui: &Ui, w: i32) -> [(f32, f32, f32, f32); 2] {
        let total = ui.pt(CARD_W_PT).min(w as f32 - ui.pt(32.0));
        let x = ((w as f32 - total) / 2.0).round();
        let (y, h, gap) = (ui.pt(TOP_PT), ui.pt(H_PT), ui.pt(GAP_PT));
        let bw = ui.pt(BATTERY_W_PT);
        [(x, y, total - bw - gap, h), (x + total - bw, y, bw, h)]
    }

    /// Where the strip ends, in px: the next element starts `BELOW_PT` under it.
    pub fn bottom(ui: &Ui) -> f32 {
        ui.pt(TOP_PT + H_PT)
    }

    pub fn hit(ui: &Ui, w: i32, x: f32, y: f32) -> Option<Target> {
        let [e, b] = Self::rects(ui, w);
        let inside = |(rx, ry, rw, rh): (f32, f32, f32, f32)| x >= rx && x < rx + rw && y >= ry && y < ry + rh;
        if inside(e) {
            Some(Target::Event)
        } else if inside(b) {
            Some(Target::Battery)
        } else {
            None
        }
    }

    /// Draws the strip lifted with the lock screen by `dy`.
    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, dy: i32) {
        let [(ex, ey, ew, eh), (bx, by, bw, bh)] = Self::rects(ui, w);
        let ey = ey + dy as f32;
        let by = by + dy as f32;
        s.fill_rrect(ex, ey, ew, eh, ui.pt(RADIUS_PT), CARD);
        s.fill_rrect(bx, by, bw, bh, ui.pt(RADIUS_PT), CARD);
        let tx = (ex + ui.pt(PAD_PT)) as i32;
        match &self.when {
            Some(when) => {
                s.blit(&self.title, tx, (ey + eh * 0.36 - self.title.h as f32 / 2.0).round() as i32, 255, 255);
                s.blit(when, tx, (ey + eh * 0.70 - when.h as f32 / 2.0).round() as i32, 255, 255);
            }
            None => s.blit(&self.title, tx, (ey + (eh - self.title.h as f32) / 2.0).round() as i32, 255, 255),
        }
        let cx = bx + bw / 2.0;
        s.blit(&self.percent, (cx - self.percent.w as f32 / 2.0).round() as i32, (by + bh * 0.36 - self.percent.h as f32 / 2.0).round() as i32, 255, 255);
        s.blit(&self.state, (cx - self.state.w as f32 / 2.0).round() as i32, (by + bh * 0.74 - self.state.h as f32 / 2.0).round() as i32, 255, 255);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::tests::ui;

    fn d(year: i64, month: u32, day: u32) -> Date {
        Date { year, month, day }
    }

    fn ev(id: u64, date: Date, minute: u16, duration: u16, all_day: bool, title: &str) -> CalendarEvent {
        CalendarEvent { id, date, minute, duration, all_day, title: title.into(), location: String::new(), notes: String::new(), repeat: 0, alert: 0, uid: String::new() }
    }

    #[test]
    fn the_next_event_skips_finished_ones_and_picks_the_earliest() {
        let today = d(2026, 10, 4);
        let events = vec![
            ev(1, today, 8 * 60, 60, false, "Breakfast"),
            ev(2, today, 15 * 60, 30, false, "Dentist"),
            ev(3, d(2026, 10, 5), 9 * 60, 30, false, "Standup"),
            ev(4, d(2026, 10, 3), 12 * 60, 30, false, "Yesterday"),
        ];
        assert_eq!(next_event(&events, today, 10 * 60).map(|e| e.id), Some(2), "breakfast is over");
        assert_eq!(next_event(&events, today, 8 * 60 + 59).map(|e| e.id), Some(1), "still running");
        assert_eq!(next_event(&events, today, 16 * 60).map(|e| e.id), Some(3), "tomorrow's is next");
        assert_eq!(next_event(&events[3..], today, 0), None);
        assert_eq!(next_event(&[], today, 0), None);
    }

    #[test]
    fn an_all_day_event_of_today_leads_the_day() {
        let today = d(2026, 10, 4);
        let events = vec![ev(1, today, 18 * 60, 60, false, "Dinner"), ev(2, today, 0, 1440, true, "Holiday")];
        assert_eq!(next_event(&events, today, 17 * 60).map(|e| e.id), Some(2));
        assert_eq!(next_event(&events, today, 23 * 60 + 59).map(|e| e.id), Some(2), "all day lasts all day");
    }

    #[test]
    fn labels_name_today_tomorrow_or_the_date() {
        let today = d(2026, 10, 4);
        assert_eq!(when_label(&ev(1, today, 14 * 60 + 30, 30, false, "x"), today), "Today 14:30");
        assert_eq!(when_label(&ev(1, d(2026, 10, 5), 0, 1440, true, "x"), today), "Tomorrow, all day");
        assert_eq!(when_label(&ev(1, d(2026, 10, 12), 9 * 60, 30, false, "x"), today), "Mon 12 Oct 09:00");
        assert_eq!(when_label(&ev(1, d(2026, 11, 1), 9 * 60 + 5, 30, false, "x"), d(2026, 10, 31)), "Tomorrow 09:05", "month ends roll over");
    }

    #[test]
    fn the_weather_slot_reads_the_saved_summary_and_respects_privacy() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("target/test-lock-weather-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(weather_slot(&dir, false), None, "nothing before the app has run");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("summary.txt"), "Oslo\t7\t61\t1\t9\t2\tC").unwrap();
        assert_eq!(weather_slot(&dir, false).as_deref(), Some("Oslo 7\u{b0} Rain"));
        assert_eq!(weather_slot(&dir, true).as_deref(), Some("Weather"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_strip_sits_under_the_clock_and_its_panels_are_tappable() {
        let ui = ui();
        let [(ex, ey, ew, eh), (bx, by, bw, _)] = Strip::rects(&ui, 1200);
        assert_eq!(ey, ui.pt(TOP_PT));
        assert_eq!(by, ey);
        assert_eq!(ex + ew + ui.pt(GAP_PT), bx, "the gap sits between the panels");
        assert_eq!(bx + bw - ex, ui.pt(CARD_W_PT), "as wide as a notification card");
        assert_eq!(Strip::bottom(&ui), ey + eh);
        assert_eq!(Strip::hit(&ui, 1200, ex + 10.0, ey + 10.0), Some(Target::Event));
        assert_eq!(Strip::hit(&ui, 1200, bx + 10.0, ey + 10.0), Some(Target::Battery));
        assert_eq!(Strip::hit(&ui, 1200, ex + ew + 2.0, ey + 10.0), None, "the gap is nothing");
        assert_eq!(Strip::hit(&ui, 1200, ex + 10.0, ey + eh + 1.0), None);
        assert_eq!(Target::Battery.app(), AppId::Settings);
        let narrow = Strip::rects(&ui, 500);
        assert!(narrow[0].0 >= 0.0 && narrow[1].0 + narrow[1].2 <= 500.0, "fits a narrow screen");
    }

    #[test]
    fn draws_event_battery_and_hides_a_private_title() {
        let ui = ui();
        let (w, h) = (1200usize, 1920usize);
        let paint = |strip: &Strip| {
            let mut buf = vec![0u32; w * h];
            strip.draw(&mut Surface { w, h, stride: w, px: &mut buf, y0: 0 }, &ui, 1200, 0);
            buf
        };
        let open = paint(&Strip::new(&ui, Some(("Dentist", "Today 15:00")), false, 80, false));
        let hidden = paint(&Strip::new(&ui, Some(("Dentist", "Today 15:00")), true, 80, false));
        let none = paint(&Strip::new(&ui, None, false, 15, true));
        let [(ex, ey, ew, eh), _] = Strip::rects(&ui, 1200);
        let at = (ey + eh / 2.0) as usize * w + (ex + ew / 2.0) as usize;
        assert_ne!(open[at], 0, "the panel paints");
        assert_ne!(open, hidden, "the private strip differs");
        assert_ne!(open, none);
    }
}
