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
use crate::shell::apphost::unquote;
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

/// The Focus that is on now: the name of the Focus mode on (Do Not Disturb, Work, ...), or None when no Focus is on.
fn active_focus(shell: &Shell) -> Option<String> {
    shell.notify.active_mode().map(|mode| mode.name)
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
                let active = active_focus(shell);
                view.activate(active.as_deref());
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
            filters.activate(active.as_deref());
        }
        let before = self.last.replace(active.clone());
        match before {
            Some(prev) if prev != active => {
                // the apps' declared intents run on a worker thread, off the frame
                let installed = crate::shell::apphost::installed();
                let declared: Vec<(String, String)> = installed.iter().flat_map(|i| i.manifest.focus_intents.iter().map(|intent| (i.manifest.id.clone(), intent.clone()))).collect();
                let focus = active.clone().unwrap_or_default();
                for (app, intent) in focus_calls(&declared, |id| crate::shell::apphost::slot_of(id).is_some()) {
                    let focus = focus.clone();
                    let _ = crate::bare::sys::spawn_named("focus-intent", move || {
                        if let Err(e) = crate::shell::intents::run_with(&app, &intent, &[("focus", focus.as_str())], std::time::Duration::from_secs(2)) {
                            eprintln!("focus_filter: intent {intent} of {app}: {e}");
                        }
                    });
                }
                self.apps.iter().map(|(app, _)| Event { app: Some(app.clone()), event: EVENT_CHANGED.to_string(), body: changed_body(active.as_deref()) }).collect()
            }
            _ => Vec::new(),
        }
    }
}

/// One `[[focus_filter]]` table of a manifest, already checked: an id, a title, and the value keys the app keeps for
/// each Focus (its `set_context` values). A Settings page shows one row of fields per declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterDeclaration {
    pub id: String,
    pub title: String,
    /// the value keys, in declaration order
    pub params: Vec<String>,
    /// an `[[intent]]` id of the app the shell runs when the Focus changes (`focus` = the Focus name, "" when none is
    /// on). Only a running app gets the call; an unsafe id is dropped and the declaration kept.
    pub intent: Option<String>,
}

/// The intent calls a Focus change makes: each declared `(app id, intent id)` whose app is running. The caller passes
/// the Focus name (`""` when none is on). An app that is not running is not started for a change of Focus.
pub fn focus_calls(declared: &[(String, String)], is_running: impl Fn(&str) -> bool) -> Vec<(String, String)> {
    declared.iter().filter(|(app, _)| is_running(app.as_str())).cloned().collect()
}

/// A word the manifest may hold as an id or a value key: letters, digits, dashes and underscores, up to 40 characters.
fn safe_word(text: &str) -> bool {
    !text.is_empty() && text.len() <= 40 && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Reads the `[[focus_filter]]` tables of a manifest:
///
/// ```toml
/// [[focus_filter]]
/// id = "notebook"
/// title = "Notebook"
/// params = ["folder", "room"]
/// ```
///
/// A table with an unsafe id or key, a blank title or no keys is dropped, so one bad table does not hide the others.
pub fn parse_declarations(text: &str) -> Vec<FilterDeclaration> {
    #[derive(Default)]
    struct Draft {
        id: String,
        title: String,
        params: String,
        intent: String,
    }
    let mut drafts: Vec<Draft> = Vec::new();
    let mut in_table = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && !line.contains('=') {
            in_table = line == "[[focus_filter]]";
            if in_table {
                drafts.push(Draft::default());
            }
            continue;
        }
        if !in_table {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let Some(draft) = drafts.last_mut() else { continue };
        match key.trim() {
            "id" => draft.id = unquote(value),
            "title" => draft.title = unquote(value),
            "params" => draft.params = value.trim().to_string(),
            "intent" => draft.intent = unquote(value),
            _ => {}
        }
    }
    drafts
        .into_iter()
        .filter_map(|d| {
            let inner = d.params.trim().trim_start_matches('[').trim_end_matches(']');
            let params: Vec<String> = inner.split(',').map(|p| unquote(p.trim())).filter(|p| !p.is_empty()).collect();
            let checked = safe_word(&d.id) && !d.title.trim().is_empty() && !params.is_empty() && params.iter().all(|p| safe_word(p));
            let intent = Some(d.intent).filter(|i| safe_word(i));
            checked.then(|| FilterDeclaration { id: d.id, title: d.title.trim().to_string(), params, intent })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::tests::shell;

    #[test]
    fn a_declared_intent_is_read_and_an_unsafe_one_is_dropped_with_its_table() {
        let text = "[[focus_filter]]\nid = \"notebook\"\ntitle = \"Notebook\"\nparams = [\"folder\"]\nintent = \"apply-focus\"\n\
                    [[focus_filter]]\nid = \"mail\"\ntitle = \"Mail\"\nparams = [\"account\"]\nintent = \"bad intent\"\n";
        let decls = parse_declarations(text);
        assert_eq!(decls.len(), 2, "an unsafe intent does not drop the declaration");
        assert_eq!(decls[0].intent.as_deref(), Some("apply-focus"));
        assert_eq!(decls[1].intent, None);
    }

    #[test]
    fn a_focus_change_calls_only_the_running_apps_intents() {
        let declared = vec![("is.olibuijr.notes".to_string(), "apply-focus".to_string()), ("is.olibuijr.mail".to_string(), "apply-focus".to_string())];
        assert_eq!(focus_calls(&declared, |id| id == "is.olibuijr.notes"), vec![("is.olibuijr.notes".to_string(), "apply-focus".to_string())]);
        assert!(focus_calls(&declared, |_| false).is_empty(), "no running app is started for a Focus change");
    }

    #[test]
    fn a_manifest_carries_its_focus_intents() {
        let m = crate::shell::apphost::Manifest::parse("id = \"a.b\"\nname = \"Notes\"\nexec = \"x\"\n[[focus_filter]]\nid = \"n\"\ntitle = \"N\"\nparams = [\"f\"]\nintent = \"apply-focus\"\n").unwrap();
        assert_eq!(m.focus_intents, vec!["apply-focus".to_string()]);
    }

    #[test]
    fn filter_declarations_come_from_the_manifest_and_bad_tables_are_dropped() {
        let text = "name = \"Notes\"\n[[focus_filter]]\nid = \"notebook\"\ntitle = \"Notebook\"\nparams = [\"folder\", \"room\"]\n\
                    [[focus_filter]]\nid = \"bad id\"\ntitle = \"Spaces\"\nparams = [\"a\"]\n\
                    [[focus_filter]]\nid = \"blank\"\ntitle = \" \"\nparams = [\"a\"]\n\
                    [[focus_filter]]\nid = \"none\"\ntitle = \"No values\"\nparams = []\n\
                    [[control]]\nkind = \"button\"\ntitle = \"Other\"\nintent = \"x\"\n";
        let decls = parse_declarations(text);
        assert_eq!(decls, vec![FilterDeclaration { id: "notebook".into(), title: "Notebook".into(), params: vec!["folder".into(), "room".into()], intent: None }]);
    }

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

    #[test]
    fn a_work_focus_hands_the_apps_its_work_values_and_the_change_is_pushed() {
        let mut sh = shell();
        let mut svc = FocusFilterService::default();
        store(&mut svc, &mut sh, "app.a", r#"{"focus":"Work","values":{"folder":"Projects"}}"#).unwrap();
        store(&mut svc, &mut sh, "app.a", r#"{"focus":"Do Not Disturb","values":{"mode":"Quiet"}}"#).unwrap();
        assert!(sh.set_focus_mode(Some("work")));
        assert!(svc.tick(&mut sh, 0.0).is_empty(), "the first tick records Work");
        let reply = svc.call(&mut sh, "app.a", "current", &Json::Null, 0.0).unwrap();
        assert_eq!(reply.str_field("focus").as_deref(), Some("Work"), "the mode's name is the Focus name");
        let values = reply.get("values").expect("values");
        assert_eq!(values.str_field("folder").as_deref(), Some("Projects"));
        assert!(sh.set_focus_mode(None));
        let off = svc.tick(&mut sh, 1.0);
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].body.get("focus"), Some(&Json::Null));
    }
}
