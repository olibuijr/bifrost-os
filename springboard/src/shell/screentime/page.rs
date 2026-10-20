//! Settings › Screen Time and its sub-pages (Downtime, App Limits, Always
//! Allowed), built as list specs from the recorded days and the config.

use super::super::list::{Group, Item, Spec, Tint};
use super::super::pages::{Action, PageId};
use super::super::passcode::Purpose;
use super::super::{AppId, apps};
use super::chart::{duration, Chart};
use super::config::{Config, Downtime};
use super::usage::{weekday, Day};

/// What a Screen Time row does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StEdit {
    /// Day (false) or Week (true) chart
    Range(bool),
    /// next daily-limit step for an app
    Cycle(AppId),
    DowntimeSwitch,
    DowntimeFrom,
    DowntimeTo,
    /// toggles an app's Always Allowed entry
    Allow(AppId),
    /// set, change or remove the Screen Time passcode
    Passcode(Purpose, bool),
}

impl StEdit {
    /// Whether the Screen Time passcode must be entered first (when one is
    /// set). Looking at usage and managing the passcode itself (which asks
    /// for the old code on its own) are free.
    pub fn needs_passcode(&self) -> bool {
        !matches!(self, StEdit::Range(_) | StEdit::Passcode(..))
    }
}

/// What the pages are built from.
pub struct View<'a> {
    pub weekly: bool,
    /// the last seven days, oldest first; the final one is today
    pub days: &'a [(i64, Day)],
    pub cfg: &'a Config,
    pub passcode_on: bool,
    pub clock_24h: bool,
}

const WEEKDAYS: [&str; 7] = ["S", "M", "T", "W", "T", "F", "S"];

fn clock(minute: u16, h24: bool) -> String {
    crate::status::clock_in((minute / 60) as i32, (minute % 60) as i32, h24)
}

/// Apps Screen Time manages, in launcher order (never Settings).
pub fn managed_apps() -> impl Iterator<Item = (AppId, &'static str)> {
    apps().iter().filter(|a| a.id != AppId::Settings).map(|a| (a.id, a.name))
}

/// The chart for the day or the week.
pub fn chart(v: &View) -> Chart {
    let today = v.days.last().map(|(_, d)| d.clone()).unwrap_or_default();
    if v.weekly {
        let totals: Vec<u32> = v.days.iter().map(|(_, d)| d.total()).collect();
        let avg = totals.iter().sum::<u32>() / totals.len().max(1) as u32;
        let labels = v.days.iter().enumerate().map(|(i, (day, _))| (i, WEEKDAYS[weekday(*day)].to_string())).collect();
        Chart { caption: "DAILY AVERAGE".into(), figure: duration(avg), current: Some(totals.len().saturating_sub(1)), bars: totals, labels }
    } else {
        let name = |h: u16| if v.clock_24h { format!("{h:02}") } else { clock(h * 60, false).replace(":00", "") };
        let labels = [0u16, 6, 12, 18].iter().map(|&h| (h as usize, name(h))).collect();
        Chart { caption: "TODAY".into(), figure: duration(today.total()), bars: today.hours.to_vec(), labels, current: None }
    }
}

/// The days the lists below the chart cover: today, or the whole week summed.
fn shown(v: &View) -> Day {
    let mut sum = Day::default();
    let take = if v.weekly { v.days.len() } else { 1 };
    for (_, d) in v.days.iter().rev().take(take) {
        sum.add(d);
    }
    sum
}

pub fn spec(id: PageId, v: &View) -> Spec {
    let page = |title: &str, groups: Vec<Group>| Spec { id, title: title.into(), large: false, groups };
    match id {
        PageId::StDowntime => {
            let d = v.cfg.downtime();
            page(
                "Downtime",
                vec![
                    Group::new(vec![Item::new("Scheduled").switch(d.on, Action::ScreenTime(StEdit::DowntimeSwitch))])
                        .footer("During Downtime only apps you choose to always allow can be used. A window can run past midnight."),
                    Group::new(vec![
                        Item::new("From").value(clock(d.start, v.clock_24h)).action(Action::ScreenTime(StEdit::DowntimeFrom)),
                        Item::new("To").value(clock(d.end, v.clock_24h)).action(Action::ScreenTime(StEdit::DowntimeTo)),
                    ])
                    .header("SCHEDULE"),
                    Group::new(vec![Item::new("Always Allowed").link(PageId::StAllowed)]),
                ],
            )
        }
        PageId::StLimits => {
            let used = v.days.last().map(|(_, d)| d.clone()).unwrap_or_default();
            let rows = managed_apps()
                .map(|(app, name)| {
                    let limit = v.cfg.limit_mins(app).map_or("None".to_string(), |m| duration(m * 60));
                    let note = if v.cfg.limit_mins(app).is_some() { format!("{limit}  ·  {} used", duration(used.app_secs(app))) } else { limit };
                    Item::new(name).value(note).action(Action::ScreenTime(StEdit::Cycle(app)))
                })
                .collect();
            page("App Limits", vec![Group::new(rows).footer("Tap an app to step its daily limit. When it is reached, a Time Limit screen covers the app. Settings is never limited.")])
        }
        PageId::StAllowed => {
            let rows = managed_apps().map(|(app, name)| Item::new(name).check(v.cfg.allowed(app), Action::ScreenTime(StEdit::Allow(app)))).collect();
            page("Always Allowed", vec![Group::new(rows).header("ALLOWED DURING DOWNTIME").footer("Settings is always allowed.")])
        }
        _ => {
            let sum = shown(v);
            let mut groups = vec![
                Group::new(vec![Item::chart(chart(v))]),
                // a view switch is a segmented control; its segments send Range(false) and Range(true) (`pages::chosen`)
                Group::new(vec![Item::new("Range").segmented(vec!["Day".into(), "Week".into()], usize::from(v.weekly), Action::ScreenTime(StEdit::Range(v.weekly)))]),
            ];
            let ranked = sum.ranked();
            let rows = if ranked.is_empty() {
                vec![Item::new("No Activity").tint(Tint::Dim)]
            } else {
                ranked.iter().map(|&(app, secs)| Item::new(apps().iter().find(|a| a.id == app).map_or("", |a| a.name)).value(duration(secs))).collect()
            };
            groups.push(Group::new(rows).header(if v.weekly { "MOST USED THIS WEEK" } else { "MOST USED TODAY" }));
            groups.push(Group::new(vec![Item::new("Pickups").value(sum.pickups.to_string()), Item::new("Notifications").value(sum.notif_total().to_string())]).header("ACTIVITY"));
            let dt = v.cfg.downtime();
            let limits = managed_apps().filter(|&(a, _)| v.cfg.limit_mins(a).is_some()).count();
            groups.push(Group::new(vec![
                Item::new("Downtime").value(downtime_text(dt, v.clock_24h)).link(PageId::StDowntime),
                Item::new("App Limits").value(if limits == 0 { "None".to_string() } else { limits.to_string() }).link(PageId::StLimits),
                Item::new("Always Allowed").link(PageId::StAllowed),
            ]));
            // Placeholder: content and privacy restrictions have no enforcement yet.
            groups.push(
                Group::new(vec![Item::new("Content & Privacy Restrictions").value("Not Available").tint(Tint::Dim)])
                    .footer("Content and privacy restrictions are not enforced on this device yet. Downtime, App Limits and Always Allowed are."),
            );
            groups.push(if v.passcode_on {
                Group::new(vec![
                    Item::new("Change Screen Time Passcode").tint(Tint::Blue).action(Action::ScreenTime(StEdit::Passcode(Purpose::Change, false))),
                    Item::new("Turn Off Screen Time Passcode").tint(Tint::Blue).action(Action::ScreenTime(StEdit::Passcode(Purpose::TurnOff, false))),
                ])
                .footer("The passcode is needed to change limits and Downtime, and to ignore a limit.")
            } else {
                Group::new(vec![Item::new("Use Screen Time Passcode").tint(Tint::Blue).action(Action::ScreenTime(StEdit::Passcode(Purpose::TurnOn, false)))])
                    .footer("Use a passcode to secure Screen Time settings and to allow more time when limits expire.")
            });
            page("Screen Time", groups)
        }
    }
}

/// "Off" or "10 PM to 7 AM".
pub fn downtime_text(d: Downtime, h24: bool) -> String {
    if d.on {
        format!("{} to {}", clock(d.start, h24), clock(d.end, h24))
    } else {
        "Off".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::list::{Accessory, Body};

    fn days(today: Day) -> Vec<(i64, Day)> {
        let mut v: Vec<(i64, Day)> = (0..6).map(|i| (100 + i, Day::default())).collect();
        v.push((106, today));
        v
    }

    fn view<'a>(d: &'a [(i64, Day)], cfg: &'a Config, weekly: bool, passcode_on: bool) -> View<'a> {
        View { weekly, days: d, cfg, passcode_on, clock_24h: true }
    }

    #[test]
    fn main_page_lists_usage_most_used_first() {
        let mut t = Day::default();
        t.secs.insert("files".into(), 600);
        t.secs.insert("photos".into(), 1200);
        t.pickups = 3;
        t.notifs.insert("photos".into(), 2);
        let (d, cfg) = (days(t), Config::memory());
        let s = spec(PageId::ScreenTime, &view(&d, &cfg, false, false));
        assert!(matches!(s.groups[0].items[0].body, Body::Chart(_)));
        let used = &s.groups[2];
        assert_eq!((used.items[0].label.as_str(), used.items[0].value.as_str()), ("Photos", "20m"));
        assert_eq!(used.items[1].label, "Files");
        let act = &s.groups[3];
        assert_eq!((act.items[0].value.as_str(), act.items[1].value.as_str()), ("3", "2"));
    }

    #[test]
    fn day_and_week_are_one_segmented_control() {
        let (d, cfg) = (days(Day::default()), Config::memory());
        for weekly in [false, true] {
            let s = spec(PageId::ScreenTime, &view(&d, &cfg, weekly, false));
            assert_eq!(s.groups[1].items.len(), 1, "one row, not a checklist");
            let row = &s.groups[1].items[0];
            assert_eq!(row.accessory, Accessory::Segmented);
            let seg = row.segments.as_ref().expect("segments");
            assert_eq!((seg.choices.clone(), seg.selected), (vec!["Day".to_string(), "Week".to_string()], usize::from(weekly)));
        }
    }

    #[test]
    fn content_and_privacy_is_an_honest_placeholder() {
        let (d, cfg) = (days(Day::default()), Config::memory());
        let s = spec(PageId::ScreenTime, &view(&d, &cfg, false, false));
        let row = s.groups.iter().flat_map(|g| g.items.iter()).find(|i| i.label == "Content & Privacy Restrictions").expect("placeholder row");
        assert_eq!(row.value, "Not Available");
        assert!(matches!(row.action, Action::None), "no action: nothing is enforced yet");
    }

    #[test]
    fn empty_day_says_no_activity_and_the_week_sums_all_days() {
        let (d, cfg) = (days(Day::default()), Config::memory());
        let s = spec(PageId::ScreenTime, &view(&d, &cfg, false, false));
        assert_eq!(s.groups[2].items[0].label, "No Activity");
        let mut w = days(Day::default());
        w[0].1.secs.insert("files".into(), 60);
        w[6].1.secs.insert("files".into(), 120);
        let s = spec(PageId::ScreenTime, &view(&w, &cfg, true, false));
        assert_eq!(s.groups[2].items[0].value, "3m");
        assert_eq!(s.groups[1].items[1].accessory, Accessory::Check(true));
    }

    #[test]
    fn charts_have_24_hours_or_7_days() {
        let mut t = Day::default();
        t.hours[9] = 300;
        t.secs.insert("files".into(), 300);
        let (d, cfg) = (days(t), Config::memory());
        let c = chart(&view(&d, &cfg, false, false));
        assert_eq!((c.bars.len(), c.bars[9], c.figure.as_str()), (24, 300, "5m"));
        let w = chart(&view(&d, &cfg, true, false));
        assert_eq!((w.bars.len(), w.bars[6], w.current), (7, 300, Some(6)));
        assert_eq!(w.labels[0].1, WEEKDAYS[weekday(100)]);
    }

    #[test]
    fn limits_page_shows_each_limit_and_never_settings() {
        let mut cfg = Config::memory();
        cfg.set_limit(AppId::Files, Some(30));
        let mut t = Day::default();
        t.secs.insert("files".into(), 600);
        let d = days(t);
        let s = spec(PageId::StLimits, &view(&d, &cfg, false, false));
        let items = &s.groups[0].items;
        assert!(items.iter().all(|i| i.label != "Settings"));
        let notes = items.iter().find(|i| i.label == "Files").unwrap();
        assert_eq!(notes.value, "30m  ·  10m used");
        assert_eq!(notes.action, Action::ScreenTime(StEdit::Cycle(AppId::Files)));
        assert_eq!(items.iter().find(|i| i.label == "Photos").unwrap().value, "None");
    }

    #[test]
    fn downtime_page_shows_the_window() {
        let mut cfg = Config::memory();
        cfg.set_downtime(Downtime { on: true, start: 22 * 60, end: 7 * 60 });
        let d = days(Day::default());
        let s = spec(PageId::StDowntime, &view(&d, &cfg, false, false));
        assert_eq!(s.groups[0].items[0].accessory, Accessory::Switch(true));
        assert_eq!((s.groups[1].items[0].value.as_str(), s.groups[1].items[1].value.as_str()), ("22:00", "7:00"));
        assert_eq!(downtime_text(cfg.downtime(), true), "22:00 to 7:00");
        assert_eq!(downtime_text(Downtime { on: false, ..cfg.downtime() }, true), "Off");
    }

    #[test]
    fn passcode_rows_follow_the_passcode_state() {
        let (d, cfg) = (days(Day::default()), Config::memory());
        let off = spec(PageId::ScreenTime, &view(&d, &cfg, false, false));
        assert_eq!(off.groups.last().unwrap().items[0].label, "Use Screen Time Passcode");
        let on = spec(PageId::ScreenTime, &view(&d, &cfg, false, true));
        assert_eq!(on.groups.last().unwrap().items.len(), 2);
        assert!(!StEdit::Range(true).needs_passcode() && StEdit::DowntimeSwitch.needs_passcode());
    }

    #[test]
    fn always_allowed_lists_checks() {
        let mut cfg = Config::memory();
        cfg.set_allowed(AppId::Files, true);
        let d = days(Day::default());
        let s = spec(PageId::StAllowed, &view(&d, &cfg, false, false));
        let items = &s.groups[0].items;
        assert_eq!(items.iter().find(|i| i.label == "Files").unwrap().accessory, Accessory::Check(true));
        assert_eq!(items.iter().find(|i| i.label == "Photos").unwrap().accessory, Accessory::Check(false));
    }
}
