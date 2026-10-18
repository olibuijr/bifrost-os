//! Device activity reports (the report a `DeviceActivityReport` view shows): foreground time over a run of days,
//! ranked by app, with the pickups, the notifications and the hour-by-hour split, built from the shell's per-day
//! usage files (`screen_usage`, `bifrost_ui::os_services::usage_record`).
//!
//! Privacy rules of this model:
//! - A report is built from a filter. [`Scope::OwnApp`] is the default for an app: only that app's seconds and
//!   notifications come out, and the hourly split and the pickups are left at zero, so the report cannot be used to
//!   learn how the person uses the rest of the device (the same rule `screen_usage::own_seconds` keeps).
//! - [`Scope::AllApps`] is for the Settings Screen Time page and for report extensions the person has opened from it.
//! - Nothing leaves the device: this reads local day files and builds the numbers; it opens no connection.

use crate::screen_usage::system_usage_dir;
use bifrost_ui::os_services::usage_record::UsageDay;
use std::io;
use std::path::Path;

/// How the report splits time: one bar per day, or one bar per hour of the combined days.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Segment {
    Daily,
    Hourly,
}

/// Whose usage the report may show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Only the app with this key (its lowercase name as the tracker writes it).
    OwnApp(String),
    /// Every app, with pickups and the hourly split.
    AllApps,
}

/// What to report on: which days (days since the Unix epoch, local), whose usage and how to split it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityFilter {
    pub days: Vec<i64>,
    pub scope: Scope,
    pub segment: Segment,
}

impl ActivityFilter {
    /// The `n` days ending `today`, for the whole device, split by day.
    pub fn last_days(today: i64, n: i64, scope: Scope) -> Self {
        ActivityFilter { days: bifrost_ui::os_services::usage_record::window_days(today, n.max(1)), scope, segment: Segment::Daily }
    }
}

/// One app's line in a report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppActivity {
    pub app: String,
    pub secs: u32,
    pub notifications: u32,
}

/// The numbers a report view draws.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActivityReport {
    /// Foreground seconds of the apps in scope.
    pub total_secs: u32,
    /// Times the device was picked up (zero for an own-app report).
    pub pickups: u32,
    /// Notifications of the apps in scope.
    pub notifications: u32,
    /// Apps with foreground time or notifications, most used first (ties by key).
    pub apps: Vec<AppActivity>,
    /// Foreground seconds per local hour (all zero for an own-app report).
    pub hourly: [u32; 24],
    /// Seconds per requested day, in the filter's order.
    pub per_day: Vec<(i64, u32)>,
}

/// The report for the `(day, usage)` pairs under `filter`. Days the filter does not list are ignored; a listed day with
/// no usage counts zero.
pub fn build_report(usage: &[(i64, UsageDay)], filter: &ActivityFilter) -> ActivityReport {
    let mut sum = UsageDay::default();
    let mut per_day = Vec::new();
    for &day in &filter.days {
        let u = usage.iter().find(|(d, _)| *d == day).map(|(_, u)| u);
        let secs = u.map_or(0, |u| match &filter.scope {
            Scope::OwnApp(key) => u.app_secs(key),
            Scope::AllApps => u.total(),
        });
        per_day.push((day, secs));
        if let Some(u) = u {
            sum.add(u);
        }
    }
    let mut report = ActivityReport { per_day, ..Default::default() };
    match &filter.scope {
        Scope::OwnApp(key) => {
            let (secs, notifs) = (sum.app_secs(key), sum.notifs.get(key).copied().unwrap_or(0));
            report.total_secs = secs;
            report.notifications = notifs;
            if secs > 0 || notifs > 0 {
                report.apps.push(AppActivity { app: key.clone(), secs, notifications: notifs });
            }
        }
        Scope::AllApps => {
            report.total_secs = sum.total();
            report.notifications = sum.notification_total();
            report.pickups = sum.pickups;
            report.hourly = sum.hours;
            let mut keys: Vec<&String> = sum.secs.keys().chain(sum.notifs.keys()).collect();
            keys.sort();
            keys.dedup();
            report.apps = keys
                .into_iter()
                .map(|k| AppActivity { app: k.clone(), secs: sum.app_secs(k), notifications: sum.notifs.get(k).copied().unwrap_or(0) })
                .filter(|a| a.secs > 0 || a.notifications > 0)
                .collect();
            report.apps.sort_by(|a, b| b.secs.cmp(&a.secs).then_with(|| a.app.cmp(&b.app)));
        }
    }
    report
}

/// Reads the day files in `dir` for the filter's days and builds the report. A missing day file is a day with no usage.
pub fn read_report(dir: &Path, filter: &ActivityFilter) -> io::Result<ActivityReport> {
    let mut usage = Vec::new();
    for &day in &filter.days {
        match std::fs::read_to_string(dir.join(format!("{day}.txt"))) {
            Ok(text) => usage.push((day, UsageDay::parse(&text))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(build_report(&usage, filter))
}

/// The report from the shell's own usage directory.
pub fn read_system_report(filter: &ActivityFilter) -> io::Result<ActivityReport> {
    read_report(&system_usage_dir(), filter)
}

/// A duration as a person reads it: `"0 min"`, `"5 min"`, `"1 h"`, `"1 h 5 min"`. Seconds round down to the minute.
pub fn format_duration(secs: u32) -> String {
    let (h, m) = (secs / 3600, secs % 3600 / 60);
    match (h, m) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

impl ActivityReport {
    /// The busiest hour (0 to 23) and its seconds; `None` when no hour has time.
    pub fn busiest_hour(&self) -> Option<(usize, u32)> {
        self.hourly.iter().enumerate().filter(|(_, &s)| s > 0).max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(&a.0))).map(|(h, &s)| (h, s))
    }

    /// The report as list rows for a view: the total first, then one row per app with its time.
    pub fn rows(&self) -> Vec<(String, String)> {
        let mut rows = vec![("Total".to_string(), format_duration(self.total_secs))];
        rows.extend(self.apps.iter().map(|a| (a.app.clone(), format_duration(a.secs))));
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> UsageDay {
        UsageDay::parse(text)
    }

    fn week() -> Vec<(i64, UsageDay)> {
        vec![
            (100, day("pickups=4\napp.terminal=3600\napp.photos=600\nhour.9=3000\nhour.20=1200\nnotif.photos=2\n")),
            (101, day("pickups=1\napp.terminal=120\nnotif.mail=5\nhour.9=120\n")),
        ]
    }

    fn all(days: Vec<i64>) -> ActivityFilter {
        ActivityFilter { days, scope: Scope::AllApps, segment: Segment::Daily }
    }

    #[test]
    fn an_all_apps_report_ranks_apps_and_sums_the_days() {
        let r = build_report(&week(), &all(vec![100, 101]));
        assert_eq!(r.total_secs, 4320);
        assert_eq!((r.pickups, r.notifications), (5, 7));
        let order: Vec<&str> = r.apps.iter().map(|a| a.app.as_str()).collect();
        assert_eq!(order, ["terminal", "photos", "mail"], "mail has notifications only and ranks last");
        assert_eq!(r.apps[0].secs, 3720);
        assert_eq!(r.per_day, [(100, 4200), (101, 120)]);
        assert_eq!(r.busiest_hour(), Some((9, 3120)));
    }

    #[test]
    fn an_own_app_report_shows_nothing_about_the_rest_of_the_device() {
        let f = ActivityFilter { days: vec![100, 101], scope: Scope::OwnApp("photos".into()), segment: Segment::Daily };
        let r = build_report(&week(), &f);
        assert_eq!(r.total_secs, 600);
        assert_eq!(r.notifications, 2);
        assert_eq!(r.apps, [AppActivity { app: "photos".into(), secs: 600, notifications: 2 }]);
        assert_eq!((r.pickups, r.hourly), (0, [0; 24]), "pickups and the hourly split stay private");
        assert_eq!(r.per_day, [(100, 600), (101, 0)]);
        assert_eq!(r.busiest_hour(), None);
    }

    #[test]
    fn days_outside_the_filter_and_days_without_files_count_zero() {
        let r = build_report(&week(), &all(vec![101, 102]));
        assert_eq!(r.total_secs, 120);
        assert_eq!(r.per_day, [(101, 120), (102, 0)]);
        assert_eq!(build_report(&[], &all(vec![])), ActivityReport::default());
    }

    #[test]
    fn reading_the_day_files_builds_the_same_report() {
        let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-device-activity")).join(format!("r-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (d, u) in week() {
            std::fs::write(dir.join(format!("{d}.txt")), u.to_text()).unwrap();
        }
        let filter = all(vec![99, 100, 101]);
        assert_eq!(read_report(&dir, &filter).unwrap(), build_report(&week(), &filter));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn durations_read_as_hours_and_minutes() {
        assert_eq!(format_duration(0), "0 min");
        assert_eq!(format_duration(59), "0 min");
        assert_eq!(format_duration(300), "5 min");
        assert_eq!(format_duration(3600), "1 h");
        assert_eq!(format_duration(3900), "1 h 5 min");
    }

    #[test]
    fn rows_list_the_total_then_each_app() {
        let rows = build_report(&week(), &all(vec![100])).rows();
        assert_eq!(rows[0], ("Total".to_string(), "1 h 10 min".to_string()));
        assert_eq!(rows[1], ("terminal".to_string(), "1 h".to_string()));
        assert_eq!(rows.len(), 3);
        assert_eq!(ActivityFilter::last_days(10, 3, Scope::AllApps).days, [8, 9, 10]);
    }
}
