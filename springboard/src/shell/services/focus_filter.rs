//! The `focus_filter` service: an app stores its Focus filter values for each Focus (the App Intents "Focus filters" idea)
//! and reads the values that apply while that Focus is on. The shell's Focus is Do Not Disturb (`shell::focus`), named
//! `"Do Not Disturb"` here. Each app has its own `FocusFilters` (`bifrost_ui::focus_filter`), so an app reads only its own
//! contexts. The SDK client is `bifrost_app::focus_filter`.
//!
//! Calls (JSON params):
//! - `set_context` `{focus: string, values: {key: string}}` stores the caller's values for `focus`; replies null.
//! - `remove_context` `{focus: string}` removes them; replies `{removed: bool}`.
//! - `current` replies `{focus: string|null, values: {key: string}|null}`: the Focus that is on, and the caller's values
//!   for it. Both are null when no Focus is on, and `values` is null when the caller has no context for it.
//!
//! Event to the app that owns the contexts: `changed` `{focus: string|null}` when the Focus that is on changes. The first
//! tick only records the state, so nothing is sent at start-up.
use super::{Event, Shell, SystemService};
use bifrost_core::json::Json;
use bifrost_ui::focus_filter::{AppContext, FocusFilters};

/// The name the service is registered under.
pub const SERVICE: &str = "focus_filter";
/// The Focus name the shell uses for Do Not Disturb. Apps store their values under this name.
pub const DO_NOT_DISTURB: &str = "Do Not Disturb";
/// The event pushed when the Focus that is on changes.
pub const EVENT_CHANGED: &str = "changed";

/// The shell's contexts for every app that has stored one, and the Focus seen by the last tick.
#[derive(Default)]
pub struct FocusFilterService {
    apps: Vec<(String, FocusFilters)>,
    /// The Focus on at the last tick. None until the first tick.
    last: Option<Option<String>>,
}

/// The Focus that is on now: Do Not Disturb when it is on.
fn active_focus(shell: &Shell) -> Option<&'static str> {
    if shell.dnd() {
        Some(DO_NOT_DISTURB)
    } else {
        None
    }
}

/// The `focus` name in `params`; it must not be empty.
fn focus_param(params: &Json) -> Result<String, String> {
    match params.str_field("focus") {
        Some(f) if !f.trim().is_empty() => Ok(f),
        _ => Err("focus_filter: a focus name is needed".into()),
    }
}

/// The `values` object in `params`, each value text.
fn values_param(params: &Json) -> Result<Vec<(String, String)>, String> {
    match params.get("values") {
        Some(Json::Obj(fields)) => fields
            .iter()
            .map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())).ok_or_else(|| format!("focus_filter: value {k} must be text")))
            .collect(),
        _ => Err("focus_filter: values must be an object of text values".into()),
    }
}

/// The reply body for the caller's `context`, if it has one.
fn context_json(context: Option<&AppContext>) -> Json {
    match context {
        Some(c) => Json::Obj(c.pairs().iter().map(|(k, v)| (k.clone(), Json::Str(v.clone()))).collect()),
        None => Json::Null,
    }
}

/// The `changed` event body for the Focus `focus`.
fn changed_body(focus: Option<&str>) -> Json {
    Json::Obj(vec![("focus".to_string(), focus.map_or(Json::Null, |f| Json::Str(f.to_string())))])
}

impl FocusFilterService {
    /// The caller's contexts, created on the first store.
    fn filters_mut(&mut self, app: &str) -> &mut FocusFilters {
        let index = match self.apps.iter().position(|(a, _)| a == app) {
            Some(i) => i,
            None => {
                self.apps.push((app.to_string(), FocusFilters::new()));
                self.apps.len() - 1
            }
        };
        &mut self.apps[index].1
    }
}

impl SystemService for FocusFilterService {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        match method {
            "set_context" => {
                let focus = focus_param(params)?;
                let mut context = AppContext::new();
                for (key, value) in values_param(params)? {
                    context.set(&key, &value);
                }
                self.filters_mut(app_id).set_context(&focus, context);
                Ok(Json::Null)
            }
            "remove_context" => {
                let focus = focus_param(params)?;
                let removed = self.filters_mut(app_id).remove(&focus);
                Ok(Json::Obj(vec![("removed".to_string(), Json::Bool(removed))]))
            }
            "current" => {
                // a copy, so reading never creates an entry for an app that has stored nothing
                let mut view = self.apps.iter().find(|(a, _)| a == app_id).map(|(_, f)| f.clone()).unwrap_or_default();
                view.activate(active_focus(shell));
                let focus = view.active().map_or(Json::Null, |f| Json::Str(f.to_string()));
                let values = context_json(view.current());
                Ok(Json::Obj(vec![("focus".to_string(), focus), ("values".to_string(), values)]))
            }
            _ => Err(format!("focus_filter has no method {method}")),
        }
    }

    fn tick(&mut self, shell: &mut Shell, _t: f64) -> Vec<Event> {
        let active = active_focus(shell);
        for (_, filters) in self.apps.iter_mut() {
            filters.activate(active);
        }
        let before = self.last.replace(active.map(str::to_string));
        match before {
            Some(prev) if prev.as_deref() != active => self
                .apps
                .iter()
                .map(|(app, _)| Event { app: Some(app.clone()), event: EVENT_CHANGED.to_string(), body: changed_body(active) })
                .collect(),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::tests::shell;

    fn body(text: &str) -> Json {
        bifrost_core::json::parse(text).expect("valid json")
    }

    fn store(svc: &mut FocusFilterService, sh: &mut Shell, app: &str, text: &str) -> Result<Json, String> {
        svc.call(sh, app, "set_context", &body(text), 0.0)
    }

    #[test]
    fn with_no_focus_on_the_current_reply_is_null() {
        let mut sh = shell();
        let mut svc = FocusFilterService::default();
        store(&mut svc, &mut sh, "app.a", r#"{"focus":"Do Not Disturb","values":{"mode":"Quiet"}}"#).unwrap();
        let reply = svc.call(&mut sh, "app.a", "current", &Json::Null, 0.0).unwrap();
        assert_eq!(reply.get("focus"), Some(&Json::Null));
        assert_eq!(reply.get("values"), Some(&Json::Null));
    }

    #[test]
    fn turning_do_not_disturb_on_gives_the_owner_its_values() {
        let mut sh = shell();
        let mut svc = FocusFilterService::default();
        store(&mut svc, &mut sh, "app.a", r#"{"focus":"Do Not Disturb","values":{"mode":"Quiet","room":"3"}}"#).unwrap();
        sh.set_dnd(true);
        let reply = svc.call(&mut sh, "app.a", "current", &Json::Null, 0.0).unwrap();
        assert_eq!(reply.str_field("focus").as_deref(), Some("Do Not Disturb"));
        let values = reply.get("values").expect("values");
        assert_eq!(values.str_field("mode").as_deref(), Some("Quiet"));
        assert_eq!(values.str_field("room").as_deref(), Some("3"));
    }

    #[test]
    fn an_app_only_sees_its_own_values() {
        let mut sh = shell();
        let mut svc = FocusFilterService::default();
        store(&mut svc, &mut sh, "app.a", r#"{"focus":"Do Not Disturb","values":{"mode":"Quiet"}}"#).unwrap();
        sh.set_dnd(true);
        let other = svc.call(&mut sh, "app.b", "current", &Json::Null, 0.0).unwrap();
        assert_eq!(other.str_field("focus").as_deref(), Some("Do Not Disturb"), "the Focus is shared");
        assert_eq!(other.get("values"), Some(&Json::Null), "the values are not");
    }

    #[test]
    fn remove_context_reports_whether_there_was_one() {
        let mut sh = shell();
        let mut svc = FocusFilterService::default();
        store(&mut svc, &mut sh, "app.a", r#"{"focus":"Do Not Disturb","values":{"mode":"Quiet"}}"#).unwrap();
        let first = svc.call(&mut sh, "app.a", "remove_context", &body(r#"{"focus":"Do Not Disturb"}"#), 0.0).unwrap();
        assert_eq!(first.get("removed"), Some(&Json::Bool(true)));
        let again = svc.call(&mut sh, "app.a", "remove_context", &body(r#"{"focus":"Do Not Disturb"}"#), 0.0).unwrap();
        assert_eq!(again.get("removed"), Some(&Json::Bool(false)));
    }

    #[test]
    fn bad_params_are_refused_with_the_field_named() {
        let mut sh = shell();
        let mut svc = FocusFilterService::default();
        assert!(store(&mut svc, &mut sh, "app.a", r#"{"values":{}}"#).unwrap_err().contains("focus"));
        assert!(store(&mut svc, &mut sh, "app.a", r#"{"focus":"  ","values":{}}"#).unwrap_err().contains("focus"));
        assert!(store(&mut svc, &mut sh, "app.a", r#"{"focus":"Work"}"#).unwrap_err().contains("values"));
        assert!(store(&mut svc, &mut sh, "app.a", r#"{"focus":"Work","values":{"room":3}}"#).unwrap_err().contains("room"));
        assert!(svc.call(&mut sh, "app.a", "fly", &Json::Null, 0.0).unwrap_err().contains("fly"));
    }

    #[test]
    fn a_focus_change_is_pushed_once_to_each_app_with_a_context() {
        let mut sh = shell();
        let mut svc = FocusFilterService::default();
        store(&mut svc, &mut sh, "app.a", r#"{"focus":"Do Not Disturb","values":{"mode":"Quiet"}}"#).unwrap();
        store(&mut svc, &mut sh, "app.b", r#"{"focus":"Do Not Disturb","values":{"mode":"Open"}}"#).unwrap();
        assert!(svc.tick(&mut sh, 0.0).is_empty(), "the first tick only records the Focus");
        assert!(svc.tick(&mut sh, 1.0).is_empty(), "no change, no event");
        sh.set_dnd(true);
        let events = svc.tick(&mut sh, 2.0);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].app.as_deref(), Some("app.a"));
        assert_eq!(events[1].app.as_deref(), Some("app.b"));
        assert_eq!(events[0].event, EVENT_CHANGED);
        assert_eq!(events[0].body.str_field("focus").as_deref(), Some("Do Not Disturb"));
        assert!(svc.tick(&mut sh, 3.0).is_empty(), "one event per change");
        sh.set_dnd(false);
        let off = svc.tick(&mut sh, 4.0);
        assert_eq!(off.len(), 2);
        assert_eq!(off[0].body.get("focus"), Some(&Json::Null));
    }
}
