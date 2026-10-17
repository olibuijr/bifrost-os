//! App Intent `world-clock-time`: the time in a World Clock city, or on this
//! device when no city is named. Search lists it as a shortcut. App Intent
//! `next-alarm`: when the next alarm rings.

use bifrost_app::control::{RpcError, INVALID_PARAMS};
use bifrost_app::intents::{self, Intent, IntentResult, Param, ParamType};
use bifrost_core::json::Json;
use springboard::shell::clock::tz::{self, City, CITIES};
use springboard::shell::fuzzy::fold;

pub const TIME_INTENT: &str = "world-clock-time";
pub const NEXT_ALARM_INTENT: &str = "next-alarm";

/// The intents this app declares to the shell.
pub fn intents() -> Vec<Intent> {
    vec![
        Intent::new(TIME_INTENT, "World Clock Time", "Reports the time in a World Clock city, or on this device when no city is named.")
            .param(Param::new("city", "City", ParamType::String).optional())
            .shortcut(),
        Intent::new(NEXT_ALARM_INTENT, "Next Alarm", "Reports when the next alarm rings, or that none is set.").shortcut(),
    ]
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

    #[test]
    fn declares_the_next_alarm_intent_as_a_shortcut() {
        let declared = intents();
        assert!(declared.iter().any(|i| i.id == NEXT_ALARM_INTENT && i.shortcut));
    }

    #[test]
    fn declares_one_runnable_shortcut_with_optional_city() {
        let list = intents();
        assert_eq!(list.len(), 2);
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
