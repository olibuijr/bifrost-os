//! The Clock's connections to the shell's system services.
//!
//! - `widgets_timeline`: the World Clock widget. The chosen cities are written as a timeline of one entry per minute
//!   for the next hour; Notification Centre shows the entry on screen. The shell asks for a new timeline when the
//!   hour is over (`reload` event), and the app answers then.
//! - `dock`: while a timer runs the Dock icon shows an indeterminate progress bar; when a timer finishes the icon
//!   bounces once.
//!
//! Calls are made only on changes: the chosen cities, a timer that starts, stops or finishes, or the shell's reload
//! request. Nothing is sent every frame. Replies are not waited for. A shell without a service answers with an error,
//! which the app logs and otherwise ignores.

use crate::clock::tz::City;
use bifrost_app::controls::timeline::ReloadPolicy;
use bifrost_app::widgets_timeline::{self as wt, Entry};
use bifrost_app::{dock_tile, Ctx};

/// The timeline kind the World Clock widget reads.
pub const WORLD_KIND: &str = "world-clock";
/// The widget's title line.
const WORLD_TITLE: &str = "World Clock";
/// Seconds between two entries: one entry per minute, so the minutes on screen stay right.
const STEP_SECS: i64 = 60;
/// Entries per timeline: one hour, under the shell's cap of 64 entries.
const ENTRIES: i64 = 60;
/// Cities one widget line shows, so the line stays short.
const WIDGET_CITIES: usize = 4;

/// The timeline entries for `cities` from the minute that holds `unix`, one per minute for an hour.
pub fn world_entries(cities: &[&City], unix: i64) -> Vec<Entry> {
    let start = unix.div_euclid(STEP_SECS) * STEP_SECS;
    (0..ENTRIES)
        .map(|i| {
            let date = start + i * STEP_SECS;
            Entry { date: date as f64, title: WORLD_TITLE.into(), text: line(cities, date) }
        })
        .collect()
}

/// The widget's line at `unix`: the first cities with their local time, for example "Reykjavik 12:00, London 13:00".
pub fn line(cities: &[&City], unix: i64) -> String {
    cities
        .iter()
        .take(WIDGET_CITIES)
        .map(|city| {
            let now = city.local(unix);
            format!("{} {:02}:{:02}", city.name, now.hour, now.min)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// What the app last sent to the shell, so a change is sent once.
#[derive(Default)]
pub struct System {
    /// the city names the World Clock timeline was last sent with; None until the first send
    shown: Option<Vec<&'static str>>,
    /// whether the Dock icon shows the timer progress bar
    dock_bar: bool,
}

impl System {
    /// Sends the World Clock timeline when the chosen cities differ from the ones last sent, or always when `force`
    /// is set (the shell asked for a new timeline). No cities removes the timeline.
    pub fn sync_world(&mut self, ctx: &Ctx, cities: &[&City], unix: i64, force: bool) {
        let names: Vec<&'static str> = cities.iter().map(|city| city.name).collect();
        if !force && self.shown.as_deref() == Some(names.as_slice()) {
            return;
        }
        if names.is_empty() {
            wt::remove(ctx, WORLD_KIND);
        } else {
            wt::reload(ctx, WORLD_KIND, &world_entries(cities, unix), ReloadPolicy::AtEnd);
        }
        self.shown = Some(names);
    }

    /// Shows the Dock progress bar while a timer runs and clears it when none does. `finished` is set when a timer
    /// ran out since the last call; the Dock icon then bounces once.
    pub fn timers(&mut self, ctx: &Ctx, running: bool, finished: bool) {
        if finished {
            dock_tile::bounce(ctx, false);
        }
        if running == self.dock_bar {
            return;
        }
        self.dock_bar = running;
        if running {
            dock_tile::set_indeterminate(ctx);
        } else {
            dock_tile::set_progress(ctx, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::tz::{self, CITIES};
    use bifrost_app::proto::{Configure, ToShell};

    fn city(name: &str) -> &'static City {
        CITIES.iter().find(|c| c.name == name).expect("a supported city")
    }

    fn test_ctx() -> Ctx {
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(crate::test_ui(), cfg)
    }

    /// Sat 3 Oct 2026 12:00:30 UTC: Reykjavik is on UTC, London on summer time (UTC+1).
    fn noon() -> i64 {
        tz::days_from_civil(2026, 10, 3) * 86_400 + 12 * 3600 + 30
    }

    /// The (method, kind) of every widgets_timeline or dock call the context queued.
    fn calls(ctx: &Ctx) -> Vec<(String, String)> {
        ctx.take_sent()
            .into_iter()
            .filter_map(|m| match m {
                ToShell::ServiceCall { service, method, .. } => Some((service, method)),
                _ => None,
            })
            .collect()
    }

    fn pair(service: &str, method: &str) -> (String, String) {
        (service.to_string(), method.to_string())
    }

    #[test]
    fn the_widget_line_shows_local_times_of_the_first_cities() {
        let cities = [city("Reykjavik"), city("London"), city("New York"), city("Tokyo"), city("Sydney")];
        assert_eq!(line(&cities, noon()), "Reykjavik 12:00, London 13:00, New York 08:00, Tokyo 21:00");
        assert_eq!(line(&[], noon()), "");
    }

    #[test]
    fn the_timeline_has_one_entry_a_minute_from_the_current_minute() {
        let cities = [city("Reykjavik")];
        let entries = world_entries(&cities, noon());
        assert_eq!(entries.len(), 60);
        assert_eq!(entries[0].date, (noon() - 30) as f64, "the entry on screen starts at the minute");
        assert_eq!(entries[1].date - entries[0].date, 60.0);
        assert_eq!(entries[0].title, "World Clock");
        assert_eq!(entries[0].text, "Reykjavik 12:00");
        assert!(entries.iter().all(|e| e.text.starts_with("Reykjavik ")));
    }

    #[test]
    fn the_world_timeline_is_sent_once_per_change_and_removed_when_empty() {
        let ctx = test_ctx();
        let mut system = System::default();
        let cities = [city("Reykjavik"), city("London")];
        system.sync_world(&ctx, &cities, noon(), false);
        assert_eq!(calls(&ctx), vec![pair("widgets_timeline", "reload")], "the first sync sends");
        system.sync_world(&ctx, &cities, noon() + 5, false);
        assert!(calls(&ctx).is_empty(), "the same cities send nothing");
        system.sync_world(&ctx, &cities, noon(), true);
        assert_eq!(calls(&ctx), vec![pair("widgets_timeline", "reload")], "a reload request always answers");
        system.sync_world(&ctx, &[city("Tokyo")], noon(), false);
        assert_eq!(calls(&ctx), vec![pair("widgets_timeline", "reload")], "a changed list is sent");
        system.sync_world(&ctx, &[], noon(), false);
        assert_eq!(calls(&ctx), vec![pair("widgets_timeline", "remove")], "no cities removes the timeline");
        system.sync_world(&ctx, &[], noon(), false);
        assert!(calls(&ctx).is_empty());
    }

    #[test]
    fn the_widget_request_carries_the_kind_and_the_entries() {
        let ctx = test_ctx();
        System::default().sync_world(&ctx, &[city("Reykjavik")], noon(), false);
        match ctx.take_sent().as_slice() {
            [ToShell::ServiceCall { service, method, body, .. }] => {
                assert_eq!((service.as_str(), method.as_str()), ("widgets_timeline", "reload"));
                assert!(body.contains("\"kind\":\"world-clock\""), "{body}");
                assert!(body.contains("\"policy\":\"at_end\""), "{body}");
                assert!(body.contains("Reykjavik 12:00"), "{body}");
            }
            other => panic!("one reload call expected, got {other:?}"),
        }
    }

    #[test]
    fn the_dock_bar_follows_the_timer_state_and_is_sent_on_flips_only() {
        let ctx = test_ctx();
        let mut system = System::default();
        system.timers(&ctx, false, false);
        assert!(calls(&ctx).is_empty(), "no timer, no call");
        system.timers(&ctx, true, false);
        assert_eq!(calls(&ctx), vec![pair("dock", "progress")]);
        system.timers(&ctx, true, false);
        assert!(calls(&ctx).is_empty(), "still running: nothing new");
        system.timers(&ctx, false, false);
        assert_eq!(calls(&ctx), vec![pair("dock", "progress")], "the bar clears when nothing runs");
    }

    #[test]
    fn a_finished_timer_bounces_the_dock_icon_once() {
        let ctx = test_ctx();
        let mut system = System::default();
        system.timers(&ctx, true, false);
        ctx.take_sent();
        system.timers(&ctx, false, true);
        assert_eq!(calls(&ctx), vec![pair("dock", "bounce"), pair("dock", "progress")]);
        system.timers(&ctx, false, false);
        assert!(calls(&ctx).is_empty());
    }
}
