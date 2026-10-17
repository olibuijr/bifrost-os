//! Daily metric reports as a pure model. The shell keeps cumulative per-app totals (`bifrost_core::app_metrics`). A
//! [`ReportSchedule`] takes a snapshot of those totals and, once a day has passed since the last snapshot, cuts a
//! [`DailyPayload`]: the difference between the two snapshots over the window between them. Completed payloads go into
//! a [`PastPayloads`] archive that keeps the newest few.
//!
//! The caller supplies the time in milliseconds from a fixed origin and the current totals, so nothing here reads a
//! clock. Each report covers the time since the previous delivery, which is one day when the schedule is polled daily.
//! When the totals move backwards (the shell restarted and the counters started again), the difference saturates at
//! zero for that field rather than wrapping.

use crate::app_metrics::Totals;
use crate::json::Json;

/// The length of one report window, in milliseconds (24 hours).
pub const REPORT_INTERVAL_MS: u64 = 24 * 60 * 60 * 1000;

/// The newest payloads a [`PastPayloads`] archive keeps (a week of daily reports).
pub const MAX_PAST_PAYLOADS: usize = 7;

/// The time span a payload covers, in milliseconds from the caller's origin. `begin_ms` is the previous delivery time
/// and `end_ms` is the delivery time that closed the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub begin_ms: u64,
    pub end_ms: u64,
}

/// One daily report: the totals accrued inside its window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DailyPayload {
    pub window: Window,
    pub totals: Totals,
}

/// Field-by-field `later - earlier`, saturating at zero.
fn difference(later: &Totals, earlier: &Totals) -> Totals {
    let mut endings = later.endings;
    for (slot, old) in endings.iter_mut().zip(earlier.endings.iter()) {
        *slot = slot.saturating_sub(*old);
    }
    let mut first_draw = later.first_draw;
    for (slot, old) in first_draw.iter_mut().zip(earlier.first_draw.iter()) {
        *slot = slot.saturating_sub(*old);
    }
    Totals {
        foreground_ms: later.foreground_ms.saturating_sub(earlier.foreground_ms),
        background_ms: later.background_ms.saturating_sub(earlier.background_ms),
        endings,
        first_draw,
    }
}

/// Keeps the last snapshot and its time. The first poll only takes the snapshot and returns nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReportSchedule {
    baseline: Option<(u64, Totals)>,
}

impl ReportSchedule {
    pub fn new() -> ReportSchedule {
        ReportSchedule::default()
    }

    /// Checks the schedule at `now_ms` with the current `totals`. While less than [`REPORT_INTERVAL_MS`] has passed
    /// since the last snapshot, this returns `None` and keeps that snapshot. Once the interval has passed it returns the
    /// payload for the window and takes a new snapshot at `now_ms`.
    pub fn poll(&mut self, now_ms: u64, totals: &Totals) -> Option<DailyPayload> {
        let Some((since, earlier)) = self.baseline.clone() else {
            self.baseline = Some((now_ms, totals.clone()));
            return None;
        };
        if now_ms.saturating_sub(since) < REPORT_INTERVAL_MS {
            return None;
        }
        self.baseline = Some((now_ms, totals.clone()));
        Some(DailyPayload {
            window: Window { begin_ms: since, end_ms: now_ms },
            totals: difference(totals, &earlier),
        })
    }
}

/// The newest completed payloads, oldest first. Pushing past [`MAX_PAST_PAYLOADS`] drops the oldest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PastPayloads {
    items: Vec<DailyPayload>,
}

impl PastPayloads {
    pub fn new() -> PastPayloads {
        PastPayloads::default()
    }

    pub fn push(&mut self, payload: DailyPayload) {
        self.items.push(payload);
        if self.items.len() > MAX_PAST_PAYLOADS {
            let extra = self.items.len() - MAX_PAST_PAYLOADS;
            self.items.drain(..extra);
        }
    }

    /// The archived payloads, oldest first.
    pub fn all(&self) -> &[DailyPayload] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// The payload as the JSON object an app receives: the window in milliseconds, the foreground and background time, the
/// ending counts (indexed as `Totals::endings`) and the first-draw counts per bucket.
pub fn payload_json(p: &DailyPayload) -> Json {
    let counts = |xs: &[u32]| Json::Arr(xs.iter().map(|n| Json::Num(*n as f64)).collect());
    Json::Obj(vec![
        ("begin_ms".into(), Json::Num(p.window.begin_ms as f64)),
        ("end_ms".into(), Json::Num(p.window.end_ms as f64)),
        ("foreground_ms".into(), Json::Num(p.totals.foreground_ms as f64)),
        ("background_ms".into(), Json::Num(p.totals.background_ms as f64)),
        ("endings".into(), counts(&p.totals.endings)),
        ("first_draw".into(), counts(&p.totals.first_draw)),
    ])
}

/// Reads a payload back from [`payload_json`]. `None` for another shape, or when a count list has the wrong length.
pub fn payload_from_json(body: &Json) -> Option<DailyPayload> {
    let num = |k: &str| match body.get(k)? {
        Json::Num(n) if *n >= 0.0 => Some(*n as u64),
        _ => None,
    };
    let counts = |k: &str, n: usize| -> Option<Vec<u32>> {
        let items = body.get(k)?.as_array()?;
        if items.len() != n {
            return None;
        }
        items.iter().map(|v| match v {
            Json::Num(c) if *c >= 0.0 => Some(*c as u32),
            _ => None,
        }).collect()
    };
    let endings: [u32; 6] = counts("endings", 6)?.try_into().ok()?;
    let first_draw: [u32; 7] = counts("first_draw", 7)?.try_into().ok()?;
    Some(DailyPayload {
        window: Window { begin_ms: num("begin_ms")?, end_ms: num("end_ms")? },
        totals: Totals { foreground_ms: num("foreground_ms")?, background_ms: num("background_ms")?, endings, first_draw },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn totals(fg: u64, bg: u64, endings0: u32, draw2: u32) -> Totals {
        let mut t = Totals { foreground_ms: fg, background_ms: bg, ..Totals::default() };
        t.endings[0] = endings0;
        t.first_draw[2] = draw2;
        t
    }

    #[test]
    fn the_first_poll_only_takes_a_snapshot() {
        let mut s = ReportSchedule::new();
        assert!(s.poll(5_000, &totals(10, 0, 0, 0)).is_none());
        assert!(s.poll(5_000 + REPORT_INTERVAL_MS - 1, &totals(99, 0, 0, 0)).is_none());
    }

    #[test]
    fn a_report_is_cut_exactly_one_interval_after_the_snapshot() {
        let mut s = ReportSchedule::new();
        s.poll(1_000, &totals(0, 0, 0, 0));
        let p = s.poll(1_000 + REPORT_INTERVAL_MS, &totals(3_000, 500, 2, 1)).unwrap();
        assert_eq!(p.window, Window { begin_ms: 1_000, end_ms: 1_000 + REPORT_INTERVAL_MS });
        assert_eq!(p.totals.foreground_ms, 3_000);
        assert_eq!(p.totals.background_ms, 500);
        assert_eq!(p.totals.endings[0], 2);
        assert_eq!(p.totals.first_draw[2], 1);
    }

    #[test]
    fn the_payload_is_the_difference_of_the_two_snapshots() {
        let mut s = ReportSchedule::new();
        s.poll(0, &totals(5_000, 100, 4, 3));
        let p = s.poll(REPORT_INTERVAL_MS, &totals(9_000, 250, 6, 3)).unwrap();
        assert_eq!(p.totals.foreground_ms, 4_000);
        assert_eq!(p.totals.background_ms, 150);
        assert_eq!(p.totals.endings[0], 2);
        assert_eq!(p.totals.first_draw[2], 0);
    }

    #[test]
    fn the_next_window_starts_where_the_last_one_ended() {
        let mut s = ReportSchedule::new();
        s.poll(0, &totals(0, 0, 0, 0));
        s.poll(REPORT_INTERVAL_MS, &totals(100, 0, 0, 0)).unwrap();
        let second = s.poll(2 * REPORT_INTERVAL_MS, &totals(350, 0, 0, 0)).unwrap();
        assert_eq!(second.window.begin_ms, REPORT_INTERVAL_MS);
        assert_eq!(second.window.end_ms, 2 * REPORT_INTERVAL_MS);
        assert_eq!(second.totals.foreground_ms, 250);
    }

    #[test]
    fn counters_that_moved_backwards_saturate_at_zero() {
        let mut s = ReportSchedule::new();
        s.poll(0, &totals(9_000, 0, 5, 0));
        let p = s.poll(REPORT_INTERVAL_MS, &totals(1_000, 0, 1, 0)).unwrap();
        assert_eq!(p.totals.foreground_ms, 0);
        assert_eq!(p.totals.endings[0], 0);
    }

    #[test]
    fn the_archive_keeps_the_newest_seven_in_order() {
        let mut archive = PastPayloads::new();
        assert!(archive.is_empty());
        for i in 0..9u64 {
            archive.push(DailyPayload { window: Window { begin_ms: i, end_ms: i + 1 }, totals: Totals::default() });
        }
        assert_eq!(archive.len(), MAX_PAST_PAYLOADS);
        assert_eq!(archive.all()[0].window.begin_ms, 2);
        assert_eq!(archive.all()[6].window.begin_ms, 8);
    }

    #[test]
    fn a_payload_survives_the_json_an_app_receives() {
        let mut t = totals(5000, 700, 2, 3);
        t.endings[1] = 4;
        let p = DailyPayload { window: Window { begin_ms: 10, end_ms: 86_400_010 }, totals: t };
        let body = payload_json(&p);
        assert!(matches!(body.get("foreground_ms"), Some(Json::Num(n)) if *n == 5000.0));
        assert_eq!(payload_from_json(&body), Some(p));
        assert!(payload_from_json(&Json::Null).is_none());
        let short = Json::Obj(vec![("begin_ms".into(), Json::Num(1.0))]);
        assert!(payload_from_json(&short).is_none());
    }
}
