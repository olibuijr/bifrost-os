//! App Intents: show a date, show an event, create an event. They are served on the app's control socket as
//! `intent.<id>` (and declared in `manifest.toml` for Search), so scripts and the shell can drive the calendar.
use super::store::{self, CalendarEvent};
use super::{Calendar, Date};
use bifrost_app::control::{RpcError, INVALID_PARAMS};
use bifrost_app::intents::{self, Entity, EntityQuery, Intent, IntentResult, Param, ParamType};
use bifrost_app::Ui;
use bifrost_core::json::Json;

/// The entity type of an event.
pub const EVENT: &str = "Event";

/// The intents the calendar declares.
pub fn declared() -> Vec<Intent> {
    vec![
        Intent::new("show-date", "Show Date", "Opens the calendar on a date.")
            .foreground()
            .param(Param::new("date", "Date", ParamType::Date)),
        Intent::new("show-event", "Show Event", "Opens an event on its day.")
            .foreground()
            .param(Param::new("event", "Event", ParamType::Entity(EVENT.into()))),
        Intent::new("create-event", "Create Event", "Adds an event to the calendar.")
            .param(Param::new("title", "Title", ParamType::String))
            .param(Param::new("start", "Start", ParamType::Date).optional())
            .param(Param::new("minutes", "Length in minutes", ParamType::Int).optional())
            .param(Param::new("location", "Location", ParamType::String).optional())
            .param(Param::new("notes", "Notes", ParamType::String).optional())
            .param(Param::new("all_day", "All-day", ParamType::Bool).optional()),
    ]
}

/// A date parameter: `2026-10-10`, or `2026-10-10T14:30` (and anything after the minutes). The minute of the day is
/// there only when the text carries a time. None when it is not a real date or time.
pub fn parse_when(text: &str) -> Option<(Date, Option<u16>)> {
    let date = Date { year: text.get(0..4)?.parse().ok()?, month: text.get(5..7)?.parse().ok()?, day: text.get(8..10)?.parse().ok()? };
    if !date.valid() || text.get(4..5)? != "-" || text.get(7..8)? != "-" {
        return None;
    }
    if text.len() == 10 {
        return Some((date, None));
    }
    if text.get(10..11)? != "T" || text.get(13..14)? != ":" {
        return None;
    }
    let (h, m): (u16, u16) = (text.get(11..13)?.parse().ok()?, text.get(14..16)?.parse().ok()?);
    (h < 24 && m < 60).then_some((date, Some(h * 60 + m)))
}

/// What the `create-event` intent asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewEvent {
    pub title: String,
    pub date: Date,
    /// None: 09:00.
    pub minute: Option<u16>,
    /// None: the default length from the settings.
    pub minutes: Option<u16>,
    pub location: String,
    pub notes: String,
    pub all_day: bool,
}

impl Calendar {
    /// Shows `date`, leaving whatever was open (the event form, a sheet or a search).
    pub fn show_date(&mut self, ui: &Ui, date: Date) {
        self.leave_modes(ui);
        self.selected = date;
        self.fit_scroll(ui);
        if self.time_view() {
            self.scroll_to_morning(ui);
        } else {
            self.scroll.jump_top();
        }
        self.dirty = true;
    }

    /// Opens event `id` on its day. False when there is no such event.
    pub fn show_event(&mut self, ui: &Ui, id: u64) -> bool {
        let Some(date) = self.events.iter().find(|e| e.id == id).map(|e| e.date) else { return false };
        self.leave_modes(ui);
        self.selected = date;
        self.open(ui, Some(id));
        self.dirty = true;
        true
    }

    /// Closes the event form, the Go to Date sheet and the search, so the calendar can show something else.
    fn leave_modes(&mut self, ui: &Ui) {
        self.goto = None;
        self.quick_line = None;
        self.close();
        self.end_search(ui);
    }

    /// Adds an event and saves it. The calendar shows its day.
    pub fn create_event(&mut self, new: NewEvent) -> Result<CalendarEvent, String> {
        if self.storage_blocked {
            return Err("Saved events could not be read. Save is unavailable.".into());
        }
        if self.events.len() >= store::MAX_EVENTS {
            return Err("Calendar is full".into());
        }
        let event = CalendarEvent {
            id: store::next_id(&self.events),
            date: new.date,
            minute: if new.all_day { 0 } else { new.minute.unwrap_or(9 * 60) },
            duration: if new.all_day { 1440 } else { new.minutes.unwrap_or(self.prefs.duration).clamp(15, 1440) },
            all_day: new.all_day,
            title: new.title.trim().to_string(),
            location: new.location.trim().to_string(),
            notes: new.notes.trim().to_string(),
            repeat: 0,
            alert: if new.all_day { self.prefs.alert_all_day } else { self.prefs.alert },
        };
        if !event.valid() {
            return Err("The event needs a title of up to 160 characters, without line breaks".into());
        }
        let mut events = self.events.clone();
        events.push(event.clone());
        store::sort(&mut events);
        store::save(&self.path, &events).map_err(|e| e.to_string())?;
        self.events = events;
        self.changes.post();
        self.selected = event.date;
        self.picked = Some(event.id);
        self.dirty = true;
        Ok(event)
    }

    /// The events an `Event` entity query asks for: the next ones (suggested), those with these ids, or those whose
    /// title holds the text.
    pub fn event_entities(&self, query: &EntityQuery) -> Vec<Entity> {
        let from = self.today;
        let wanted: Vec<&CalendarEvent> = match query {
            EntityQuery::Suggested => self.events.iter().filter(|e| e.date >= from).take(10).collect(),
            EntityQuery::Ids(ids) => self.events.iter().filter(|e| ids.iter().any(|i| *i == e.id.to_string())).collect(),
            EntityQuery::String(text) => {
                let text = text.to_lowercase();
                self.events.iter().filter(|e| e.title.to_lowercase().contains(&text)).take(50).collect()
            }
        };
        wanted
            .into_iter()
            .map(|e| {
                let when = if e.all_day { format!("{} all-day", e.date.iso()) } else { format!("{} {}", e.date.iso(), super::time_label(e.minute)) };
                Entity::new(EVENT, &e.id.to_string(), &e.title).subtitle(&when)
            })
            .collect()
    }
}

fn date_param(params: &Json, name: &str) -> Result<(Date, Option<u16>), RpcError> {
    let text = intents::str_param(params, name)?;
    parse_when(text).ok_or_else(|| RpcError::new(INVALID_PARAMS, format!("parameter {name} is not a date")))
}

fn text_param(params: &Json, name: &str) -> String {
    params.get(name).and_then(Json::as_str).unwrap_or("").to_string()
}

/// Runs intent `id` on the calendar.
pub fn perform(c: &mut Calendar, ui: &Ui, id: &str, params: &Json) -> Result<IntentResult, RpcError> {
    match id {
        "show-date" => {
            let (date, _) = date_param(params, "date")?;
            c.show_date(ui, date);
            Ok(IntentResult::value(Json::Str(date.iso())).with_dialog(&format!("Showing {}", date.label())).continue_in_foreground())
        }
        "show-event" => {
            let raw = intents::str_param(params, "event")?;
            let event_id: u64 = raw.parse().map_err(|_| RpcError::new(INVALID_PARAMS, "parameter event must be an event id"))?;
            if !c.show_event(ui, event_id) {
                return Err(intents::failed(format!("no event {raw}")));
            }
            Ok(IntentResult::value(Json::Str(raw.to_string())).with_dialog("Showing the event").continue_in_foreground())
        }
        "create-event" => {
            let title = intents::str_param(params, "title")?.to_string();
            let (date, minute) = match params.get("start").filter(|v| **v != Json::Null) {
                Some(_) => date_param(params, "start")?,
                None => (c.selected(), None),
            };
            let minutes = params.get("minutes").and_then(Json::number).map(|m| m.clamp(15.0, 1440.0) as u16);
            let all_day = matches!(params.get("all_day"), Some(Json::Bool(true)));
            let made = c
                .create_event(NewEvent { title, date, minute, minutes, location: text_param(params, "location"), notes: text_param(params, "notes"), all_day })
                .map_err(intents::failed)?;
            let when = if made.all_day { format!("{} (all-day)", made.date.iso()) } else { format!("{} {}", made.date.iso(), super::time_label(made.minute)) };
            Ok(IntentResult::value(Json::Str(made.id.to_string())).with_dialog(&format!("Created {} on {when}", made.title)))
        }
        _ => Err(RpcError::new(INVALID_PARAMS, format!("no intent {id}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(month: u32, day: u32) -> Date {
        Date { year: 2026, month, day }
    }
    fn obj(fields: &[(&str, Json)]) -> Json {
        Json::Obj(fields.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
    }
    fn s(text: &str) -> Json {
        Json::Str(text.to_string())
    }

    #[test]
    fn dates_and_times_parse_and_bad_ones_do_not() {
        assert_eq!(parse_when("2026-10-10"), Some((d(10, 10), None)));
        assert_eq!(parse_when("2026-10-10T14:30"), Some((d(10, 10), Some(870))));
        assert_eq!(parse_when("2026-10-10T14:30:00Z"), Some((d(10, 10), Some(870))));
        assert_eq!(parse_when("2026-10-10T00:00"), Some((d(10, 10), Some(0))));
        for bad in ["", "2026-13-01", "2026-02-30", "2026/10/10", "2026-10-10X14:30", "2026-10-10T24:00", "2026-10-10T12:60", "2026-10-1", "2026-10-10T1"] {
            assert_eq!(parse_when(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_declared_intents_validate_their_parameters() {
        let all = declared();
        let find = |id: &str| all.iter().find(|i| i.id == id).unwrap();
        assert!(find("show-date").validate(&obj(&[("date", s("2026-10-10"))])).is_ok());
        assert!(find("show-date").validate(&obj(&[])).is_err(), "the date is required");
        assert!(find("show-date").validate(&obj(&[("date", s("tomorrow"))])).is_err());
        assert!(find("show-event").validate(&obj(&[("event", s("3"))])).is_ok());
        assert!(find("create-event").validate(&obj(&[("title", s("Dentist"))])).is_ok());
        let full = obj(&[("title", s("Dentist")), ("start", s("2026-10-12T09:30")), ("minutes", Json::Num(45.0)), ("location", s("Laugavegur 5")), ("notes", s("x")), ("all_day", Json::Bool(false))]);
        assert!(find("create-event").validate(&full).is_ok());
        assert!(find("create-event").validate(&obj(&[("title", s("x")), ("colour", s("red"))])).is_err(), "unknown parameter");
        // the foreground intents may continue in the foreground, the background one may not
        assert!(find("show-date").supports(bifrost_app::intents::IntentMode::Foreground));
        assert!(!find("create-event").supports(bifrost_app::intents::IntentMode::Foreground));
    }

    #[test]
    fn the_manifest_lists_the_same_intents() {
        let manifest = include_str!("../../manifest.toml");
        for i in declared() {
            assert!(manifest.contains(&i.manifest_toml()), "manifest.toml declares {}:\n{}", i.id, i.manifest_toml());
        }
    }
}
