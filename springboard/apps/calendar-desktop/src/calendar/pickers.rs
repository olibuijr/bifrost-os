//! The event form's pickers. Repeat and Alert expand inline below their rows, as Starts and Ends do; Calendar, Time Zone
//! and Travel Time cycle in place. The form edits copies (`Draft`), and `commit` (`calendar.rs`) saves them: the preset
//! fields go to the store, the rest to the extras (`extras.rs`).
use super::alerts::{ALERT_MINUTES, REPEAT_RULES};
use super::extras::{self, Extra};
use super::recur::{self, Freq, Rule};
use super::*;

/// The frequencies the Frequency row cycles through. None is Never.
const FREQS: [Option<Freq>; 5] = [None, Some(Freq::Daily), Some(Freq::Weekly), Some(Freq::Monthly), Some(Freq::Yearly)];
/// Occurrences an "After" end starts with, and the most a rule may name.
const COUNT_START: u32 = 10;
const COUNT_MAX: i32 = 999;
/// The custom alarm the Alert picker's stepper starts on, in minutes.
const CUSTOM_ALARM_START: u32 = 30;

/// The rule an event starts with: its own RRULE when it has one, else its preset.
pub(super) fn rule_of(event: &CalendarEvent, extra: &Extra) -> Option<Rule> {
    if let Some(text) = &extra.rrule {
        return recur::parse_rrule(text);
    }
    if event.repeat == 0 {
        return None;
    }
    REPEAT_RULES.get(event.repeat as usize).and_then(|r| recur::parse_rrule(r))
}

/// The repeat a rule is saved as: its preset number and no text, or 0 with the rule's own text when no preset is that rule.
pub(super) fn repeat_fields(rule: Option<&Rule>) -> (u8, Option<String>) {
    let Some(rule) = rule else { return (0, None) };
    let text = recur::to_rrule(rule);
    match REPEAT_RULES.iter().position(|p| !p.is_empty() && *p == text.as_str()) {
        Some(k) => (k as u8, None),
        None => (0, Some(text)),
    }
}

/// The alert a list of alarms is saved as: the preset of its first alarm (0 when it has none), and the list itself when
/// it is more than that preset.
pub(super) fn alarm_fields(alarms: &[u32]) -> (u8, Option<Vec<u32>>) {
    let list = extras::clean_alarms(alarms);
    let alert = list.first().and_then(|m| ALERT_MINUTES.iter().position(|a| *a == Some(*m))).unwrap_or(0) as u8;
    let same = extras::preset_alarms(alert) == list;
    (alert, (!same).then_some(list))
}

/// The weekday column (0 is the row's first point) under window point `xp`, of a row `wp` points wide.
pub(super) fn weekday_column(xp: f32, wp: f32) -> u8 {
    (xp / (wp / 7.0)).floor().clamp(0.0, 6.0) as u8
}

/// The rows of the Repeat picker under its row: the frequency, then (when the event repeats) the interval, the weekdays
/// for a weekly rule, the end and its stepper.
pub(super) fn repeat_targets(d: &Draft) -> Vec<Target> {
    if !d.repeat_open {
        return vec![];
    }
    let mut out = vec![Target::RepeatFreq];
    if let Some(rule) = &d.rule {
        out.push(Target::RepeatEvery(0));
        if rule.freq == Freq::Weekly {
            out.push(Target::RepeatDays);
        }
        out.push(Target::RepeatEnds);
        match (rule.count, rule.until) {
            (Some(_), _) => out.push(Target::RepeatCount(0)),
            (None, Some(_)) => out.push(Target::RepeatUntil(0)),
            _ => {}
        }
    }
    out
}

/// The rows of the Alert picker under its row: the seven presets (the first clears the list), the custom alarm's stepper
/// and its add row, then a row per alarm that is not a preset.
pub(super) fn alarm_targets(d: &Draft) -> Vec<Target> {
    if !d.alert_open {
        return vec![];
    }
    let mut out: Vec<Target> = (0..ALERT_MINUTES.len() as u8).map(Target::AlarmPreset).collect();
    out.push(Target::AlarmStep(0));
    out.push(Target::AlarmAdd);
    out.extend(d.alarms.iter().enumerate().filter(|(_, m)| !ALERT_MINUTES.contains(&Some(**m))).map(|(i, _)| Target::AlarmDrop(i)));
    out
}

/// The Repeat row's value: Never, a preset's name, or the rule's own description.
pub(super) fn repeat_label(rule: Option<&Rule>) -> String {
    match rule {
        None => "Never".into(),
        Some(r) => match repeat_fields(Some(r)) {
            (k, None) => store::REPEATS[k as usize].to_string(),
            _ => recur::describe(Some(r)),
        },
    }
}

/// The Alert row's value: None, one alarm's name, or how many alarms there are.
pub(super) fn alarm_label(alarms: &[u32]) -> String {
    match alarms {
        [] => "None".into(),
        [one] => extras::alarm_title(*one),
        many => format!("{} alerts", many.len()),
    }
}

/// The rule after a change of frequency. Never clears it. A frequency keeps the interval and the end of the rule it
/// replaces, and drops the weekdays that only a weekly rule uses.
fn with_freq(rule: Option<Rule>, freq: Option<Freq>) -> Option<Rule> {
    let freq = freq?;
    Some(match rule {
        Some(mut r) => {
            r.freq = freq;
            if freq != Freq::Weekly {
                r.by_day.clear();
            }
            r
        }
        None => Rule::new(freq),
    })
}

/// Whether weekday `wd` (0 is Sunday) is one the weekly rule repeats on. With no weekdays named the rule repeats on the
/// day the event starts.
fn day_on(d: &Draft, wd: u8) -> bool {
    match &d.rule {
        Some(r) if r.by_day.is_empty() => wd == d.event.date.weekday() as u8,
        Some(r) => r.by_day.iter().any(|(_, w)| *w == wd),
        None => false,
    }
}

/// The Frequency row's value.
fn freq_label(rule: Option<&Rule>) -> &'static str {
    match rule.map(|r| r.freq) {
        None => "Never",
        Some(Freq::Daily) => "Daily",
        Some(Freq::Weekly) => "Weekly",
        Some(Freq::Monthly) => "Monthly",
        Some(Freq::Yearly) => "Yearly",
    }
}

/// The Ends row's value.
fn ends_label(rule: Option<&Rule>) -> String {
    match rule {
        Some(Rule { count: Some(c), .. }) => format!("After {c} times"),
        Some(Rule { until: Some(u), .. }) => format!("Until {}", short_date(*u)),
        _ => "Never".into(),
    }
}

/// "3 Oct 2026": the day, the short month name and the year.
fn short_date(d: Date) -> String {
    format!("{} {} {}", d.day, &MONTHS[(d.month - 1) as usize][..3], d.year)
}

/// The Time Zone row's name for the choice `tzid` (see `extras::ZONES`).
fn zone_index(tzid: Option<&str>) -> usize {
    extras::ZONES.iter().position(|z| *z == tzid).unwrap_or(0)
}

impl Calendar {
    /// The form's draft for `event`: its working rule, alarms and extras. A new event starts on the default calendar.
    pub(super) fn new_draft(&self, event: CalendarEvent, existing: bool, invitees: Vec<Participant>) -> Draft {
        let extra = if existing {
            self.extras.events.get(&event.id).cloned().unwrap_or_default()
        } else {
            Extra { calendar: self.extras.default_id(), ..Extra::default() }
        };
        let rule = rule_of(&event, &extra);
        let alarms = extra.alarms.clone().unwrap_or_else(|| extras::preset_alarms(event.alert));
        Draft {
            event,
            existing,
            open: None,
            invitees,
            repeat_open: false,
            alert_open: false,
            rule,
            alarms,
            custom_alarm: CUSTOM_ALARM_START,
            extra,
            invitee_line: None,
        }
    }

    /// A tap on a picker row or one of its controls. The rows that expand open and close; the controls change the
    /// working copies in the draft. The form stays open and `commit` saves the result.
    pub(super) fn picker_tap(&mut self, target: Target) {
        self.focus(None);
        let prefs = self.prefs;
        let Some(d) = self.editor.as_mut() else { return };
        match target {
            Target::Repeat => d.repeat_open = !d.repeat_open,
            Target::Alert => d.alert_open = !d.alert_open,
            Target::RepeatFreq => {
                let at = FREQS.iter().position(|f| *f == d.rule.as_ref().map(|r| r.freq)).unwrap_or(0);
                d.rule = with_freq(d.rule.take(), FREQS[(at + 1) % FREQS.len()]);
            }
            Target::RepeatEvery(n) => {
                if let Some(r) = d.rule.as_mut() {
                    r.interval = (r.interval as i32 + n).clamp(1, 99) as u32;
                }
            }
            Target::RepeatDay(col) => {
                let wd = prefs.weekday_at(col as usize) as u8;
                let start = d.event.date.weekday() as u8;
                if let Some(r) = d.rule.as_mut() {
                    // the implicit start day becomes an explicit one before the first change; a rule keeps one day
                    if r.by_day.is_empty() {
                        r.by_day.push((0, start));
                    }
                    match r.by_day.iter().position(|(_, w)| *w == wd) {
                        Some(i) if r.by_day.len() > 1 => {
                            r.by_day.remove(i);
                        }
                        Some(_) => {}
                        None => r.by_day.push((0, wd)),
                    }
                    r.by_day.sort_by_key(|(_, w)| *w);
                }
            }
            Target::RepeatEnds => {
                let start = d.event.date;
                if let Some(r) = d.rule.as_mut() {
                    match (r.count, r.until) {
                        (None, None) => r.count = Some(COUNT_START),
                        (Some(_), _) => {
                            r.count = None;
                            r.until = Some(start.shift_months(12));
                        }
                        (None, Some(_)) => r.until = None,
                    }
                }
            }
            Target::RepeatCount(n) => {
                if let Some(c) = d.rule.as_mut().and_then(|r| r.count.as_mut()) {
                    *c = (*c as i32 + n).clamp(1, COUNT_MAX) as u32;
                }
            }
            Target::RepeatUntil(n) => {
                let start = d.event.date;
                if let Some(r) = d.rule.as_mut() {
                    if let Some(u) = r.until {
                        let next = u.shift_months(n);
                        r.until = Some(if next < start { start } else { next });
                    }
                }
            }
            Target::AlarmPreset(k) => {
                if k == 0 {
                    d.alarms.clear();
                } else if let Some(Some(m)) = ALERT_MINUTES.get(k as usize).copied() {
                    match d.alarms.iter().position(|a| *a == m) {
                        Some(i) => {
                            d.alarms.remove(i);
                        }
                        None => d.alarms.push(m),
                    }
                    d.alarms = extras::clean_alarms(&d.alarms);
                }
            }
            Target::AlarmStep(n) => {
                let next = d.custom_alarm as i64 + n as i64 * extras::ALARM_STEP as i64;
                d.custom_alarm = next.clamp(0, extras::ALARM_MAX as i64) as u32;
            }
            Target::AlarmAdd => {
                d.alarms.push(d.custom_alarm);
                d.alarms = extras::clean_alarms(&d.alarms);
            }
            Target::AlarmDrop(i) => {
                if i < d.alarms.len() {
                    d.alarms.remove(i);
                }
            }
            Target::Zone => {
                let i = zone_index(d.extra.tzid.as_deref());
                d.extra.tzid = extras::ZONES[(i + 1) % extras::ZONES.len()].map(str::to_string);
            }
            Target::Travel => {
                let i = extras::TRAVEL.iter().position(|t| *t == d.extra.travel).unwrap_or(0);
                d.extra.travel = extras::TRAVEL[(i + 1) % extras::TRAVEL.len()];
            }
            _ => {}
        }
    }

    /// One picker row of the form, at `top` points with height `h`.
    pub(super) fn draw_picker(&mut self, s: &mut Surface, ui: &Ui, target: Target, d: &Draft, top: f32, h: f32) {
        let w = self.w as f32;
        let mid = ui.pt(top + h / 2.0);
        let prefs = self.prefs;
        match target {
            Target::Repeat => self.value_row(s, ui, "Repeat", &repeat_label(d.rule.as_ref()), mid),
            Target::Alert => self.value_row(s, ui, "Alert", &alarm_label(&d.alarms), mid),
            Target::RepeatFreq => self.value_row(s, ui, "Frequency", freq_label(d.rule.as_ref()), mid),
            Target::RepeatEvery(_) => {
                let title = recur::describe(d.rule.as_ref());
                self.step_row(s, ui, &title, mid);
            }
            Target::RepeatDays => {
                for col in 0..7u8 {
                    let wd = prefs.weekday_at(col as usize) as u8;
                    let colour = if day_on(d, wd) { colors::LINK.now() } else { LABEL_3.now() };
                    let x = w * (col as f32 + 0.5) / 7.0;
                    self.centre(s, ui, &WEEKDAYS[wd as usize][..1], MAC_BODY_PT, Weight::Semibold, colour, x, mid);
                }
            }
            Target::RepeatEnds => self.value_row(s, ui, "Ends", &ends_label(d.rule.as_ref()), mid),
            Target::RepeatCount(_) => {
                let title = match d.rule.as_ref().and_then(|r| r.count) {
                    Some(c) => format!("After {c} times"),
                    None => "After 1 time".to_string(),
                };
                self.step_row(s, ui, &title, mid);
            }
            Target::RepeatUntil(_) => {
                let title = match d.rule.as_ref().and_then(|r| r.until) {
                    Some(u) => format!("Until {}", short_date(u)),
                    None => "Until".to_string(),
                };
                self.step_row(s, ui, &title, mid);
            }
            Target::AlarmPreset(k) => {
                let (label, on) = if k == 0 {
                    ("None".to_string(), d.alarms.is_empty())
                } else {
                    let m = ALERT_MINUTES.get(k as usize).copied().flatten().unwrap_or(0);
                    (extras::alarm_title(m), d.alarms.contains(&m))
                };
                self.text(s, ui, &label, MAC_BODY_PT, LABEL.now(), ui.pt(36.0), mid);
                if on {
                    self.right(s, ui, "\u{2713}", colors::LINK.now(), w - ui.pt(36.0), mid);
                }
            }
            Target::AlarmStep(_) => {
                let title = format!("Custom: {}", extras::alarm_title(d.custom_alarm));
                self.step_row(s, ui, &title, mid);
            }
            Target::AlarmAdd => self.centre(s, ui, "Add Alert", MAC_BODY_PT, Weight::Regular, colors::LINK.now(), w / 2.0, mid),
            Target::AlarmDrop(i) => {
                let name = d.alarms.get(i).map_or_else(String::new, |m| extras::alarm_title(*m));
                self.text(s, ui, &name, MAC_BODY_PT, LABEL.now(), ui.pt(36.0), mid);
                self.right(s, ui, "Remove", colors::SYSTEM_RED.now(), w - ui.pt(36.0), mid);
            }
            Target::Calendar => {
                let cur = if self.extras.exists(d.extra.calendar) { d.extra.calendar } else { self.extras.default_id() };
                let name = self.extras.name_of(cur);
                self.value_row(s, ui, "Calendar", &name, mid);
                // the calendar's colour dot before its name, as in the pop-up menu it opens (`calmenu.rs`)
                let text_w = ui.measure(&name, MAC_BODY_PT, Weight::Regular).min(w * 0.55);
                let d = ui.pt(SPACE_S);
                let x = w - ui.pt(36.0) - text_w - ui.pt(SPACE_XS) - d;
                s.fill_rrect(x, mid - d / 2.0, d, d, d / 2.0, self.calendar_colour(cur));
            }
            Target::Zone => self.value_row(s, ui, "Time Zone", extras::zone_title(d.extra.tzid.as_deref()), mid),
            Target::Travel => self.value_row(s, ui, "Travel Time", &extras::travel_title(d.extra.travel), mid),
            _ => {}
        }
    }

    /// A row with its name on the left and its value on the right, as the Starts row shows its date.
    fn value_row(&mut self, s: &mut Surface, ui: &Ui, name: &str, value: &str, mid: f32) {
        let w = self.w as f32;
        self.text(s, ui, name, MAC_BODY_PT, LABEL.now(), ui.pt(36.0), mid);
        let fit = ui.fit(value, MAC_BODY_PT, w * 0.55);
        self.right(s, ui, &fit, LABEL_2.now(), w - ui.pt(36.0), mid);
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(month: u32, day: u32) -> Date {
        Date { year: 2026, month, day }
    }

    fn draft(rule: Option<Rule>, alarms: Vec<u32>) -> Draft {
        let event = CalendarEvent { id: 1, date: d(10, 13), minute: 540, duration: 60, all_day: false, title: "x".into(), location: String::new(), notes: String::new(), repeat: 0, alert: 0, uid: String::new() };
        Draft {
            event,
            existing: false,
            open: None,
            invitees: vec![],
            repeat_open: true,
            alert_open: true,
            rule,
            alarms,
            custom_alarm: CUSTOM_ALARM_START,
            extra: Extra::default(),
            invitee_line: None,
        }
    }

    #[test]
    fn every_preset_repeat_is_saved_as_its_number() {
        for k in 1..REPEAT_RULES.len() {
            let rule = recur::parse_rrule(REPEAT_RULES[k]).expect("preset parses");
            assert_eq!(repeat_fields(Some(&rule)), (k as u8, None), "preset {k}");
        }
        assert_eq!(repeat_fields(None), (0, None));
    }

    #[test]
    fn a_custom_rule_is_saved_as_its_text() {
        let mut rule = Rule::new(Freq::Weekly);
        rule.by_day = vec![(0, 1), (0, 3)];
        let (k, text) = repeat_fields(Some(&rule));
        assert_eq!(k, 0);
        assert_eq!(text.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO,WE"));
        let mut counted = Rule::new(Freq::Daily);
        counted.count = Some(3);
        assert_eq!(repeat_fields(Some(&counted)).1.as_deref(), Some("FREQ=DAILY;COUNT=3"));
    }

    #[test]
    fn the_repeat_label_names_presets_and_custom_rules() {
        assert_eq!(repeat_label(None), "Never");
        assert_eq!(repeat_label(recur::parse_rrule("FREQ=WEEKLY;INTERVAL=2").as_ref()), "Every 2 Weeks");
        let mut rule = Rule::new(Freq::Monthly);
        rule.count = Some(4);
        assert_eq!(repeat_label(Some(&rule)), "Every month", "a count makes it custom, described by the rule");
    }

    #[test]
    fn alarms_are_saved_as_the_preset_of_the_first_or_as_a_list() {
        assert_eq!(alarm_fields(&[]), (0, None));
        assert_eq!(alarm_fields(&[5]), (2, None));
        assert_eq!(alarm_fields(&[0]), (1, None));
        assert_eq!(alarm_fields(&[25]), (0, Some(vec![25])));
        assert_eq!(alarm_fields(&[60, 5, 60]), (2, Some(vec![5, 60])));
    }

    #[test]
    fn the_alert_label_names_one_alarm_or_counts_them() {
        assert_eq!(alarm_label(&[]), "None");
        assert_eq!(alarm_label(&[1440]), "1 day before");
        assert_eq!(alarm_label(&[5, 60, 1440]), "3 alerts");
    }

    #[test]
    fn frequency_changes_keep_the_end_and_drop_weekdays() {
        let mut weekly = Rule::new(Freq::Weekly);
        weekly.interval = 2;
        weekly.by_day = vec![(0, 2)];
        weekly.count = Some(5);
        let monthly = with_freq(Some(weekly), Some(Freq::Monthly)).unwrap();
        assert_eq!((monthly.freq, monthly.interval, monthly.count), (Freq::Monthly, 2, Some(5)));
        assert!(monthly.by_day.is_empty());
        assert_eq!(with_freq(Some(monthly), None), None, "Never clears the rule");
        assert_eq!(with_freq(None, Some(Freq::Yearly)).map(|r| r.freq), Some(Freq::Yearly));
    }

    #[test]
    fn the_weekday_row_maps_points_to_columns() {
        assert_eq!(weekday_column(0.0, 700.0), 0);
        assert_eq!(weekday_column(99.0, 700.0), 0);
        assert_eq!(weekday_column(100.0, 700.0), 1);
        assert_eq!(weekday_column(699.0, 700.0), 6);
        assert_eq!(weekday_column(1000.0, 700.0), 6);
    }

    #[test]
    fn the_repeat_rows_appear_only_when_open_and_follow_the_rule() {
        let mut d0 = draft(None, vec![]);
        d0.repeat_open = false;
        assert!(repeat_targets(&d0).is_empty());
        let never = draft(None, vec![]);
        assert_eq!(repeat_targets(&never), vec![Target::RepeatFreq]);
        let weekly = draft(Some(Rule::new(Freq::Weekly)), vec![]);
        assert_eq!(repeat_targets(&weekly), vec![Target::RepeatFreq, Target::RepeatEvery(0), Target::RepeatDays, Target::RepeatEnds]);
        let mut counted = draft(Some(Rule { count: Some(3), ..Rule::new(Freq::Daily) }), vec![]);
        counted.repeat_open = true;
        assert_eq!(repeat_targets(&counted).last(), Some(&Target::RepeatCount(0)));
    }

    #[test]
    fn the_alert_rows_list_presets_then_custom_alarms() {
        let d1 = draft(None, vec![5, 25]);
        let rows = alarm_targets(&d1);
        assert_eq!(rows.len(), 7 + 2 + 1, "seven presets, the stepper and add, then the custom alarm");
        assert_eq!(rows[9], Target::AlarmDrop(1), "25 minutes is not a preset, 5 is");
        assert!(!alarm_targets(&draft(None, vec![])).is_empty());
        let mut closed = draft(None, vec![]);
        closed.alert_open = false;
        assert!(alarm_targets(&closed).is_empty());
    }

    #[test]
    fn the_day_on_follows_the_start_day_until_days_are_named() {
        // 13 October 2026 is a Tuesday
        let implicit = draft(Some(Rule::new(Freq::Weekly)), vec![]);
        assert!(day_on(&implicit, 2));
        assert!(!day_on(&implicit, 1));
        let named = draft(Some(Rule { by_day: vec![(0, 1), (0, 5)], ..Rule::new(Freq::Weekly) }), vec![]);
        assert!(day_on(&named, 5) && !day_on(&named, 2));
        assert!(!day_on(&draft(None, vec![]), 2));
    }

    #[test]
    fn zones_are_found_in_their_choice_list() {
        assert_eq!(zone_index(None), 0);
        assert_eq!(zone_index(Some("UTC")), 1);
        assert_eq!(zone_index(Some("Atlantic/Reykjavik")), 2);
        assert_eq!(zone_index(Some("Mars/Base")), 0);
    }
}
