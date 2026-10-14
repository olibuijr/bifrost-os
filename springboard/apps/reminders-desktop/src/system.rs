//! The shell services the app uses beyond its own window: the Today widget timeline (`widgets_timeline`). The Dock badge
//! (`dock`) and the due alerts (`notification-schedule`, see `alerts.rs`) are called from `main.rs` and `alerts.rs`.
//! The builders here are pure; [`reload_today`] takes a `Ctx` and returns the service request id.
use crate::model::{self, Reminder};
use bifrost_app::controls::timeline::ReloadPolicy;
use bifrost_app::widgets_timeline::{self as wt, Entry};
use bifrost_app::Ctx;

/// The kind the Today timeline is stored under in the shell.
pub const WIDGET_KIND: &str = "today";
/// Reminder rows on the timeline, after the summary row.
pub const MAX_ROWS: usize = 8;

/// The Today timeline: a summary row at `now`, then each open reminder due today or earlier, soonest first. A reminder
/// that is already overdue is dated `now` and reads "Overdue". Completed and undated reminders are left out.
pub fn today_entries(items: &[Reminder], today: i64, now: i64, all_day_min: u16, offset_at: model::OffsetAt) -> Vec<Entry> {
    let mut due: Vec<(i64, &Reminder)> = items
        .iter()
        .filter(|r| !r.completed && r.due.is_some_and(|d| d <= today))
        .filter_map(|r| model::due_unix(r, all_day_min, offset_at).map(|at| (at, r)))
        .collect();
    due.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)));
    let summary = match due.len() {
        0 => "Nothing due".to_string(),
        1 => "1 due".to_string(),
        n => format!("{n} due"),
    };
    let mut out = vec![Entry { date: now as f64, title: "Today".into(), text: summary }];
    for &(at, r) in due.iter().take(MAX_ROWS) {
        let text = if at < now { "Overdue".to_string() } else { model::clock_label(r.time.unwrap_or(all_day_min)) };
        out.push(Entry { date: at.max(now) as f64, title: r.title.clone(), text });
    }
    out
}

/// The unix second the next local day starts: when the timeline asks to be reloaded.
pub fn next_midnight(today: i64, offset_at: model::OffsetAt) -> f64 {
    springboard::shell::clock::alarm::local_to_unix(today + 1, 0, offset_at) as f64
}

/// Replaces the Today timeline with the reminders of `today` as seen at `now`, reloading at the next midnight. Returns
/// the request id; an older shell without `widgets_timeline` answers with an error, which the app ignores.
pub fn reload_today(ctx: &Ctx, items: &[Reminder], today: i64, now: i64, all_day_min: u16, offset_at: model::OffsetAt) -> u32 {
    let entries = today_entries(items, today, now, all_day_min, offset_at);
    wt::reload(ctx, WIDGET_KIND, &entries, ReloadPolicy::After(next_midnight(today, offset_at)))
}

#[cfg(test)]
pub(crate) fn test_ctx() -> Ctx {
    use bifrost_app::kit::{Font, Ui};
    use bifrost_app::Configure;
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
    let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
    let ui = Ui::new(Font::parse(data).unwrap(), 1.0);
    let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
    Ctx::testing(ui, cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::proto::ToShell;
    use bifrost_core::json::{parse, Json};

    const TODAY: i64 = 20_000;
    const NOON: u16 = 720;
    const DAY: u16 = model::DEFAULT_ALERT_MIN;

    fn at_day(id: u64, title: &str, day: i64, min: u16) -> Reminder {
        let mut r = Reminder::new(id, title);
        r.due = Some(day);
        r.time = Some(min);
        r
    }

    fn zero(_: i64) -> i32 {
        0
    }

    #[test]
    fn the_summary_row_counts_open_reminders_due_today_or_earlier() {
        let now = TODAY * 86_400 + 9 * 3600;
        let mut done = at_day(4, "done", TODAY, NOON);
        done.completed = true;
        let items = vec![at_day(1, "later", TODAY + 1, NOON), at_day(2, "today", TODAY, NOON), at_day(3, "late", TODAY - 2, NOON), done, Reminder::new(5, "undated")];
        let rows = today_entries(&items, TODAY, now, DAY, &zero);
        assert_eq!(rows[0].title, "Today");
        assert_eq!(rows[0].text, "2 due");
        assert_eq!(rows[0].date, now as f64);
        let titles: Vec<&str> = rows[1..].iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, vec!["late", "today"], "soonest first, no later, completed or undated reminders");
    }

    #[test]
    fn overdue_rows_sit_at_now_and_read_overdue() {
        let now = TODAY * 86_400 + 13 * 3600;
        let rows = today_entries(&[at_day(1, "old", TODAY - 1, 60)], TODAY, now, DAY, &zero);
        assert_eq!(rows[1].text, "Overdue");
        assert_eq!(rows[1].date, now as f64);
    }

    #[test]
    fn a_future_row_shows_its_clock_time_at_its_due_moment() {
        let now = TODAY * 86_400;
        let rows = today_entries(&[at_day(1, "standup", TODAY, 9 * 60 + 15)], TODAY, now, DAY, &zero);
        assert_eq!(rows[1].text, model::clock_label(9 * 60 + 15));
        assert_eq!(rows[1].date, (TODAY * 86_400 + (9 * 60 + 15) * 60) as f64);
    }

    #[test]
    fn a_date_only_reminder_is_listed_at_the_all_day_time() {
        let mut r = Reminder::new(1, "birthday");
        r.due = Some(TODAY);
        let now = TODAY * 86_400;
        let rows = today_entries(&[r.clone()], TODAY, now, 8 * 60, &zero);
        assert_eq!(rows[1].text, model::clock_label(8 * 60));
        assert_eq!(rows[1].date, (now + 8 * 3600) as f64);
        let rows = today_entries(&[r], TODAY, now, 17 * 60, &zero);
        assert_eq!(rows[1].text, model::clock_label(17 * 60));
    }

    #[test]
    fn an_empty_day_says_nothing_is_due_and_keeps_the_summary_row() {
        let rows = today_entries(&[], TODAY, 100, DAY, &zero);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].text, "Nothing due");
    }

    #[test]
    fn the_rows_are_capped() {
        let items: Vec<Reminder> = (1..=20).map(|i| at_day(i, "r", TODAY, 600 + i as u16)).collect();
        let rows = today_entries(&items, TODAY, TODAY * 86_400, DAY, &zero);
        assert_eq!(rows.len(), 1 + MAX_ROWS);
        assert_eq!(rows[0].text, "20 due", "the summary still counts all of them");
    }

    #[test]
    fn the_reload_is_due_at_the_next_local_midnight() {
        assert_eq!(next_midnight(TODAY, &zero), ((TODAY + 1) * 86_400) as f64);
        // a zone 3600 s east starts the next day an hour earlier in unix time
        let east = |_: i64| 3_600;
        assert_eq!(next_midnight(TODAY, &east), ((TODAY + 1) * 86_400 - 3_600) as f64);
    }

    #[test]
    fn reload_today_sends_the_timeline_to_the_widgets_service() {
        let ctx = test_ctx();
        let id = reload_today(&ctx, &[at_day(1, "standup", TODAY, 540)], TODAY, TODAY * 86_400, DAY, &zero);
        let sent = ctx.take_sent();
        assert_eq!(sent.len(), 1);
        match &sent[0] {
            ToShell::ServiceCall { request, service, method, body } => {
                assert_eq!(*request, id);
                assert_eq!(service, "widgets_timeline");
                assert_eq!(method, "reload");
                let json: Json = parse(body).unwrap();
                assert_eq!(json.str_field("kind").as_deref(), Some("today"));
                assert_eq!(json.str_field("policy").as_deref(), Some("after"));
                assert_eq!(json.get("reload_at").and_then(Json::number), Some(((TODAY + 1) * 86_400) as f64));
                let entries = json.get("entries").and_then(Json::as_array).unwrap();
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[1].str_field("title").as_deref(), Some("standup"));
            }
            other => panic!("expected a service call, got {other:?}"),
        }
    }
}
