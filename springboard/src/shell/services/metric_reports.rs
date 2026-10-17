//! MetricKit daily reports for apps (`MXMetricManager`): the shell keeps each app's usage totals and, once a day has
//! passed, sends the subscribed app its daily payload as a `didReceiveDailyReport` event. The model is
//! `bifrost_core::app_metrics` (totals) and `bifrost_core::daily_metric_report` (schedule, payload, archive); this
//! module feeds the totals and runs the schedule.
//!
//! Feeding: the shell calls [`note_launch`] when it starts an app, [`note_foreground`] when the app is shown, and
//! [`note_close`] when it ends the app. Background time is not tracked yet, so an app counts as foreground until it
//! is closed.
//!
//! Methods (params are a JSON object, the app id is the caller):
//! - `subscribe {}` starts daily events for the app; `unsubscribe {}` stops them.
//! - `reports {}` returns `{reports: [payload]}`, the archived payloads (newest last, at most a week).
//!
//! Events: `didReceiveDailyReport` with the payload object of `bifrost_core::daily_metric_report::payload_json`.
use super::{Event, Shell, SystemService};
use crate::shell::services::location::now_unix;
use bifrost_core::app_metrics::{Ending, Metrics, Phase, Totals};
use bifrost_core::daily_metric_report::{payload_json, DailyPayload, PastPayloads, ReportSchedule};
use bifrost_core::json::Json;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

/// The service name.
pub const SERVICE: &str = "metrics";
/// The event that carries a daily payload.
pub const EVENT_DAILY: &str = "didReceiveDailyReport";

thread_local! {
    static METRICS: RefCell<Metrics> = RefCell::new(Metrics::new());
    static SCHEDULES: RefCell<BTreeMap<String, ReportSchedule>> = const { RefCell::new(BTreeMap::new()) };
    static ARCHIVE: RefCell<BTreeMap<String, PastPayloads>> = const { RefCell::new(BTreeMap::new()) };
    static SUBSCRIBERS: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
}

/// Milliseconds from the Unix epoch for a time in seconds (never negative).
fn millis(t: f64) -> u64 {
    (t.max(0.0) * 1000.0) as u64
}

/// The app was started: its launch clock begins.
pub fn note_launch(app: &str) {
    let now = millis(now_unix());
    METRICS.with(|m| m.borrow_mut().begin_launch(app, now));
}

/// The app was shown: its foreground time begins.
pub fn note_foreground(app: &str) {
    let now = millis(now_unix());
    METRICS.with(|m| m.borrow_mut().set_phase(app, Phase::Foreground, now));
}

/// The app was shown after its first frame: the launch that began with [`note_launch`] ends here.
pub fn note_first_draw(app: &str) {
    let now = millis(now_unix());
    METRICS.with(|m| m.borrow_mut().first_draw(app, now));
}

/// The app ended normally: its open phase closes and the run counts as a normal ending.
pub fn note_close(app: &str) {
    let now = millis(now_unix());
    // Called from `AppHost::drop`, which runs when a thread's keyboard service is torn down; `METRICS` may already
    // be destroyed by then, so a plain `with` would panic inside a thread-local destructor and abort the process.
    let _ = METRICS.try_with(|m| {
        let mut m = m.borrow_mut();
        m.close(app, now);
        m.record_ending(app, Ending::Normal);
    });
}

/// Subscribes an app to daily events. `false` when it already was.
pub fn subscribe(app: &str) -> bool {
    SUBSCRIBERS.with(|s| s.borrow_mut().insert(app.to_string()))
}

/// Unsubscribes an app. `false` when it was not subscribed.
pub fn unsubscribe(app: &str) -> bool {
    SUBSCRIBERS.with(|s| s.borrow_mut().remove(app))
}

/// The payloads archived for an app, oldest first.
pub fn archived(app: &str) -> Vec<DailyPayload> {
    ARCHIVE.with(|a| a.borrow().get(app).map(|p| p.all().to_vec()).unwrap_or_default())
}

/// Runs the schedule for every subscribed app at `now_ms` and returns one event per payload that came due. Each due
/// payload is archived. The first check of an app only takes its baseline, so the first payload comes a day later.
pub fn due(now_ms: u64) -> Vec<Event> {
    let apps: Vec<String> = SUBSCRIBERS.with(|s| s.borrow().iter().cloned().collect());
    let mut out = Vec::new();
    for app in apps {
        let totals: Totals = METRICS.with(|m| m.borrow().totals(&app).cloned()).unwrap_or_default();
        let payload = SCHEDULES.with(|s| s.borrow_mut().entry(app.clone()).or_default().poll(now_ms, &totals));
        if let Some(payload) = payload {
            let body = payload_json(&payload);
            ARCHIVE.with(|a| a.borrow_mut().entry(app.clone()).or_default().push(payload));
            out.push(Event { app: Some(app), event: EVENT_DAILY.into(), body });
        }
    }
    out
}

/// The `metrics` service: subscriptions and the archive. Reports are cut in [`due`], on each tick.
pub struct MetricReports;

impl SystemService for MetricReports {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, _params: &Json, _t: f64) -> Result<Json, String> {
        match method {
            "subscribe" => Ok(Json::Obj(vec![("subscribed".into(), Json::Bool(subscribe(app_id)))])),
            "unsubscribe" => Ok(Json::Obj(vec![("stopped".into(), Json::Bool(unsubscribe(app_id)))])),
            "reports" => {
                let list = archived(app_id).iter().map(payload_json).collect();
                Ok(Json::Obj(vec![("reports".into(), Json::Arr(list))]))
            }
            _ => Err(format!("metrics has no method {method}")),
        }
    }

    fn tick(&mut self, _shell: &mut Shell, _t: f64) -> Vec<Event> {
        due(millis(now_unix()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::daily_metric_report::REPORT_INTERVAL_MS;

    #[test]
    fn a_subscribed_app_gets_one_payload_a_day_with_its_foreground_time() {
        // Each test runs on its own thread, so the thread-local state starts empty.
        let mut m = Metrics::new();
        m.set_phase("a", Phase::Foreground, 0);
        METRICS.with(|s| *s.borrow_mut() = m);
        assert!(subscribe("a"));
        assert!(!subscribe("a"), "a second subscribe changes nothing");
        assert!(due(0).is_empty(), "the first check only takes the baseline");
        METRICS.with(|s| s.borrow_mut().close("a", 3_600_000));
        assert!(due(REPORT_INTERVAL_MS - 1).is_empty(), "less than a day is not a report");
        let events = due(REPORT_INTERVAL_MS);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, EVENT_DAILY);
        assert_eq!(events[0].app.as_deref(), Some("a"));
        assert!(matches!(events[0].body.get("foreground_ms"), Some(Json::Num(n)) if *n == 3_600_000.0));
        assert_eq!(archived("a").len(), 1);
        assert!(due(REPORT_INTERVAL_MS + 1).is_empty(), "the next report waits another day");
    }

    #[test]
    fn an_unsubscribed_app_gets_nothing_and_its_archive_stays() {
        assert!(!unsubscribe("never"));
        subscribe("b");
        due(0);
        assert_eq!(due(REPORT_INTERVAL_MS).len(), 1);
        assert!(unsubscribe("b"));
        assert!(due(2 * REPORT_INTERVAL_MS).is_empty());
        assert_eq!(archived("b").len(), 1);
    }

    #[test]
    fn launch_and_close_feed_the_totals() {
        note_launch("c");
        note_foreground("c");
        note_close("c");
        let totals = METRICS.with(|m| m.borrow().totals("c").cloned()).unwrap();
        assert_eq!(totals.endings[Ending::Normal as usize], 1);
    }

    #[test]
    fn millis_never_goes_negative() {
        assert_eq!(millis(-5.0), 0);
        assert_eq!(millis(1.5), 1500);
    }
}
