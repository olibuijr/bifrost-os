//! Per-app usage records for one day: foreground seconds per app and per local hour, pickups, and notifications per app.
//!
//! The system's usage tracker keeps one `key=value` file per day. [`UsageDay::parse`] and [`UsageDay::to_text`] read and
//! write that format, and skip lines they do not know. App keys are the lowercase app names the tracker uses.
//! [`window_days`] gives the day numbers of a chart history.

use std::collections::BTreeMap;

/// One day of usage.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsageDay {
    /// Foreground seconds per app key.
    pub secs: BTreeMap<String, u32>,
    /// Foreground seconds per local hour, 0 to 23.
    pub hours: [u32; 24],
    /// Times the device was picked up.
    pub pickups: u32,
    /// Notifications received per app key.
    pub notifs: BTreeMap<String, u32>,
}

impl UsageDay {
    /// Reads a day file. Lines without a known key or a number are skipped.
    pub fn parse(text: &str) -> Self {
        let mut day = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let Ok(n) = value.trim().parse::<u32>() else { continue };
            let key = key.trim();
            if let Some(app) = key.strip_prefix("app.") {
                day.secs.insert(app.to_string(), n);
            } else if let Some(app) = key.strip_prefix("notif.") {
                day.notifs.insert(app.to_string(), n);
            } else if let Some(hour) = key.strip_prefix("hour.").and_then(|h| h.parse::<usize>().ok()).filter(|&h| h < 24) {
                day.hours[hour] = n;
            } else if key == "pickups" {
                day.pickups = n;
            }
        }
        day
    }

    /// The day file text. Zero hours are left out.
    pub fn to_text(&self) -> String {
        let mut out = format!("pickups={}\n", self.pickups);
        for (app, n) in &self.secs {
            out += &format!("app.{app}={n}\n");
        }
        for (hour, n) in self.hours.iter().enumerate().filter(|(_, &n)| n > 0) {
            out += &format!("hour.{hour}={n}\n");
        }
        for (app, n) in &self.notifs {
            out += &format!("notif.{app}={n}\n");
        }
        out
    }

    /// Total foreground seconds.
    pub fn total(&self) -> u32 {
        self.secs.values().sum()
    }

    /// Foreground seconds for one app key.
    pub fn app_secs(&self, app: &str) -> u32 {
        self.secs.get(app).copied().unwrap_or(0)
    }

    /// Notifications received, all apps.
    pub fn notification_total(&self) -> u32 {
        self.notifs.values().sum()
    }

    /// Adds another day's counts to this one, app by app and hour by hour.
    pub fn add(&mut self, other: &UsageDay) {
        for (app, n) in &other.secs {
            *self.secs.entry(app.clone()).or_default() += n;
        }
        for (app, n) in &other.notifs {
            *self.notifs.entry(app.clone()).or_default() += n;
        }
        for (mine, theirs) in self.hours.iter_mut().zip(other.hours.iter()) {
            *mine += theirs;
        }
        self.pickups += other.pickups;
    }

    /// Apps with foreground time, most used first. Ties sort by key.
    pub fn ranked(&self) -> Vec<(&str, u32)> {
        let mut apps: Vec<(&str, u32)> = self.secs.iter().filter(|(_, &n)| n > 0).map(|(app, &n)| (app.as_str(), n)).collect();
        apps.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        apps
    }
}

/// The `n` day numbers that end at `today`, oldest first. Day numbers count days since the Unix epoch.
pub fn window_days(today: i64, n: i64) -> Vec<i64> {
    (today - n + 1..=today).collect()
}
