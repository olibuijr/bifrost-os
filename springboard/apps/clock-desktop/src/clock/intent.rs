//! App Intent `world-clock-time`: the time in a World Clock city, or on this
//! device when no city is named. Search lists it as a shortcut. App Intent
//! `next-alarm`: when the next alarm rings. App Intent `set-alarm`: adds an alarm for a time, with an optional label and
//! repeat.

use bifrost_app::control::{RpcError, INVALID_PARAMS};
use bifrost_app::intents::{self, Intent, IntentResult, Param, ParamType};
use bifrost_core::json::Json;
use springboard::shell::clock::alarm::{Alarm, EVERY_DAY, WEEKDAYS, WEEKEND};
use springboard::shell::clock::tz::{self, City, CITIES};
use springboard::shell::fuzzy::fold;

pub const TIME_INTENT: &str = "world-clock-time";
pub const NEXT_ALARM_INTENT: &str = "next-alarm";
pub const SET_ALARM_INTENT: &str = "set-alarm";

/// The repeat choices `set-alarm` offers.
pub const REPEATS: [&str; 4] = ["once", "daily", "weekdays", "weekends"];

/// The longest alarm label `set-alarm` keeps, in characters.
const LABEL_MAX: usize = 40;

/// The intents this app declares to the shell.
pub fn intents() -> Vec<Intent> {
    vec![
        Intent::new(TIME_INTENT, "World Clock Time", "Reports the time in a World Clock city, or on this device when no city is named.")
            .param(Param::new("city", "City", ParamType::String).optional())
            .shortcut(),
        Intent::new(NEXT_ALARM_INTENT, "Next Alarm", "Reports when the next alarm rings, or that none is set.").shortcut(),
        Intent::new(SET_ALARM_INTENT, "Set Alarm", "Adds an alarm that rings at the given time.")
            .param(Param::new("hour", "Hour", ParamType::Int))
            .param(Param::new("minute", "Minute", ParamType::Int))
            .param(Param::new("label", "Label", ParamType::String).optional())
            .param(Param::new("repeat", "Repeat", ParamType::Enum(REPEATS.iter().map(|r| r.to_string()).collect())).optional())
            .shortcut(),
    ]
}

/// A whole number parameter in `0..=max`, or the reason it is refused.
fn whole(params: &Json, name: &str, max: u64) -> Result<u32, RpcError> {
    match params.get(name) {
        Some(Json::Num(n)) if n.is_finite() && n.fract() == 0.0 && *n >= 0.0 && *n <= max as f64 => Ok(*n as u32),
        _ => Err(RpcError::new(INVALID_PARAMS, format!("{name} must be a whole number from 0 to {max}"))),
    }
}

/// The alarm `set-alarm` asks for: `hour` and `minute` of the day, a `label` (trimmed, cut at 40 characters, "Alarm" when
/// blank) and a `repeat` choice from [`REPEATS`] (once when missing). The alarm is on and has no id yet.
pub fn alarm_from_params(params: &Json) -> Result<Alarm, RpcError> {
    let mut alarm = Alarm::new(whole(params, "hour", 23)?, whole(params, "minute", 59)?);
    if let Some(label) = params.get("label").and_then(Json::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        alarm.label = label.chars().take(LABEL_MAX).collect();
    }
    alarm.repeat = match params.get("repeat") {
        None | Some(Json::Null) => 0,
        Some(Json::Str(r)) => match r.as_str() {
            "once" => 0,
            "daily" => EVERY_DAY,
            "weekdays" => WEEKDAYS,
            "weekends" => WEEKEND,
            other => return Err(RpcError::new(INVALID_PARAMS, format!("repeat {other} is not one of once, daily, weekdays, weekends"))),
        },
        Some(_) => return Err(RpcError::new(INVALID_PARAMS, "repeat must be text")),
    };
    Ok(alarm)
}

/// What the intent says when the alarm is set: "Alarm set for 07:30 (Weekdays)".
pub fn alarm_set_text(alarm: &Alarm) -> String {
    match alarm.repeat_text().as_str() {
        "Never" => format!("Alarm set for {}", alarm.time_text()),
        repeat => format!("Alarm set for {} ({})", alarm.time_text(), repeat),
    }
}

/// Runs the alarm intent. `next` is the summary the Alarms page shows for the next armed alarm ("Alarm in 7 hr 5 min"),
/// or `None` when no alarm is armed.
pub fn perform_alarm(id: &str, next: Option<String>) -> Result<IntentResult, RpcError> {
    if id != NEXT_ALARM_INTENT {
        return Err(RpcError::new(INVALID_PARAMS, format!("no intent {id}")));
    }
    let text = next.unwrap_or_else(|| "No alarm is set".to_string());
    Ok(IntentResult::value(Json::Str(text.clone())).with_dialog(&text))
}

/// Runs the intent at Unix time `unix`, with the device at `home_offset` seconds from UTC.
pub fn perform(id: &str, params: &Json, unix: i64, home_offset: i32) -> Result<IntentResult, RpcError> {
    if id != TIME_INTENT {
        return Err(RpcError::new(INVALID_PARAMS, format!("no intent {id}")));
    }
    let text = match params.get("city").and_then(Json::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        None => {
            let here = tz::civil(unix, home_offset);
            format!("{:02}:{:02} here", here.hour, here.min)
        }
        Some(name) => {
            let city = find_city(name).ok_or_else(|| intents::failed(format!("No World Clock city named {name}")))?;
            let (word, _) = tz::relative(city, unix, home_offset);
            let local = city.local(unix);
            format!("{} {:02}:{:02}, {}", city.name, local.hour, local.min, word)
        }
    };
    Ok(IntentResult::value(Json::Str(text.clone())).with_dialog(&text))
}

/// The supported city whose name matches `name`, ignoring case and accents.
pub fn find_city(name: &str) -> Option<&'static City> {
    let wanted = fold(name.trim());
    CITIES.iter().find(|c| fold(c.name) == wanted)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-10-14 12:00 UTC.
    fn unix() -> i64 {
        tz::days_from_civil(2026, 10, 14) * 86_400 + 12 * 3600
    }

    fn named(city: &str) -> Json {
        Json::Obj(vec![("city".into(), Json::Str(city.into()))])
    }

    fn text(result: &IntentResult) -> String {
        result.value.as_str().unwrap_or_default().to_string()
    }

    #[test]
    fn next_alarm_reports_the_summary_or_none() {
        let armed = perform_alarm(NEXT_ALARM_INTENT, Some("Alarm in 7 hr 5 min".into())).unwrap();
        assert_eq!(text(&armed), "Alarm in 7 hr 5 min");
        let none = perform_alarm(NEXT_ALARM_INTENT, None).unwrap();
        assert_eq!(text(&none), "No alarm is set");
        assert!(perform_alarm(TIME_INTENT, None).is_err());
    }

    fn args(fields: &[(&str, Json)]) -> Json {
        Json::Obj(fields.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
    }

    #[test]
    fn set_alarm_is_declared_with_two_required_numbers_and_two_optional_choices() {
        let list = intents();
        let set = list.iter().find(|i| i.id == SET_ALARM_INTENT).expect("declared");
        assert!(set.shortcut);
        let names: Vec<(&str, bool)> = set.params.iter().map(|p| (p.name.as_str(), p.optional)).collect();
        assert_eq!(names, vec![("hour", false), ("minute", false), ("label", true), ("repeat", true)]);
        // the declaration accepts what the builder reads and refuses a repeat the builder does not know
        assert!(set.validate(&args(&[("hour", Json::Num(7.0)), ("minute", Json::Num(30.0)), ("repeat", Json::Str("weekdays".into()))])).is_ok());
        assert!(set.validate(&args(&[("hour", Json::Num(7.0)), ("minute", Json::Num(30.0)), ("repeat", Json::Str("monthly".into()))])).is_err());
        assert!(set.validate(&args(&[("hour", Json::Num(7.0))])).is_err(), "the minute is required");
    }

    #[test]
    fn set_alarm_builds_an_alarm_that_is_on_with_the_given_time_label_and_repeat() {
        let a = alarm_from_params(&args(&[
            ("hour", Json::Num(6.0)),
            ("minute", Json::Num(45.0)),
            ("label", Json::Str("  Gym  ".into())),
            ("repeat", Json::Str("weekdays".into())),
        ]))
        .unwrap();
        assert_eq!((a.hour, a.min), (6, 45));
        assert_eq!(a.label, "Gym");
        assert_eq!(a.repeat, WEEKDAYS);
        assert!(a.on);
        assert_eq!(alarm_set_text(&a), "Alarm set for 06:45 (Weekdays)");
    }

    #[test]
    fn set_alarm_defaults_to_one_ring_labelled_alarm() {
        let a = alarm_from_params(&args(&[("hour", Json::Num(0.0)), ("minute", Json::Num(0.0)), ("label", Json::Str("   ".into()))])).unwrap();
        assert_eq!(a.label, "Alarm");
        assert_eq!(a.repeat, 0);
        assert_eq!(alarm_set_text(&a), "Alarm set for 00:00");
        let daily = alarm_from_params(&args(&[("hour", Json::Num(9.0)), ("minute", Json::Num(5.0)), ("repeat", Json::Str("daily".into()))])).unwrap();
        assert_eq!(daily.repeat, EVERY_DAY);
        let weekend = alarm_from_params(&args(&[("hour", Json::Num(9.0)), ("minute", Json::Num(5.0)), ("repeat", Json::Str("weekends".into()))])).unwrap();
        assert_eq!(weekend.repeat, WEEKEND);
    }

    #[test]
    fn set_alarm_refuses_times_off_the_clock_and_long_labels_are_cut() {
        for (h, m) in [(24.0, 0.0), (7.0, 60.0), (-1.0, 0.0), (7.5, 0.0), (7.0, f64::NAN)] {
            let bad = args(&[("hour", Json::Num(h)), ("minute", Json::Num(m))]);
            assert_eq!(alarm_from_params(&bad).unwrap_err().code, INVALID_PARAMS, "{h}:{m}");
        }
        assert!(alarm_from_params(&args(&[("minute", Json::Num(1.0))])).is_err(), "no hour");
        let long = "x".repeat(100);
        let a = alarm_from_params(&args(&[("hour", Json::Num(1.0)), ("minute", Json::Num(1.0)), ("label", Json::Str(long))])).unwrap();
        assert_eq!(a.label.chars().count(), 40);
        assert!(alarm_from_params(&args(&[("hour", Json::Num(1.0)), ("minute", Json::Num(1.0)), ("repeat", Json::Str("hourly".into()))])).is_err());
    }

    #[test]
    fn declares_the_next_alarm_intent_as_a_shortcut() {
        let declared = intents();
        assert!(declared.iter().any(|i| i.id == NEXT_ALARM_INTENT && i.shortcut));
    }

    #[test]
    fn declares_one_runnable_shortcut_with_optional_city() {
        let list = intents();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].id, TIME_INTENT);
        assert!(list[0].shortcut);
        assert_eq!(list[0].params.len(), 1);
        assert!(list[0].params[0].optional);
    }

    #[test]
    fn city_time_is_local_to_the_city() {
        // Tokyo is UTC+9 with no daylight saving
        let r = perform(TIME_INTENT, &named("Tokyo"), unix(), 0).unwrap();
        assert_eq!(text(&r), "Tokyo 21:00, Today");
        assert_eq!(r.dialog.as_deref(), Some("Tokyo 21:00, Today"));
    }

    #[test]
    fn city_name_matches_ignoring_case() {
        let r = perform(TIME_INTENT, &named("  TOKYO "), unix(), 0).unwrap();
        assert_eq!(text(&r), "Tokyo 21:00, Today");
    }

    #[test]
    fn daylight_saving_is_applied() {
        // Sydney is on daylight time from October 4 2026
        let r = perform(TIME_INTENT, &named("Sydney"), unix(), 0).unwrap();
        assert_eq!(text(&r), "Sydney 23:00, Today");
    }

    #[test]
    fn no_city_reports_device_time() {
        let r = perform(TIME_INTENT, &Json::Obj(vec![]), unix(), 0).unwrap();
        assert_eq!(text(&r), "12:00 here");
    }

    #[test]
    fn blank_city_counts_as_no_city() {
        let r = perform(TIME_INTENT, &named("   "), unix(), 3600).unwrap();
        assert_eq!(text(&r), "13:00 here");
    }

    #[test]
    fn unknown_city_is_an_error() {
        assert!(perform(TIME_INTENT, &named("Atlantis"), unix(), 0).is_err());
    }

    #[test]
    fn other_intent_ids_are_rejected() {
        assert!(perform("new-clock", &Json::Obj(vec![]), unix(), 0).is_err());
    }

    #[test]
    fn relative_word_follows_the_date_line() {
        // Auckland is on daylight time (UTC+13), so it is already tomorrow at 12:00 UTC
        let r = perform(TIME_INTENT, &named("Auckland"), unix(), 0).unwrap();
        assert!(text(&r).ends_with("Tomorrow"), "{}", text(&r));
    }
}
