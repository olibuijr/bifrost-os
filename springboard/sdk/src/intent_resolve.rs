//! Parameter resolution for App Intents: the steps that run before `perform_intent`.
//!
//! The resolution model asks, for each parameter, whether the value is usable: it is `success`, `needs a value`,
//! `unsupported`, `not required`, or it needs a `disambiguation` among choices. After every parameter is settled, a
//! final confirmation step can run before the intent is performed. This module is the pure model of those steps.
//! It does not talk to the user; the caller asks the question and calls [`plan`] again with the answer.
//!
//! - [`resolve_param`]: one parameter, one [`Resolution`].
//! - [`plan`]: all parameters of an [`Intent`], in declaration order. The first parameter that is not settled decides
//!   the [`Plan`]: ask for a value, let the user choose, or stop with a reason. When everything is settled the plan is
//!   [`Plan::Confirm`] (when the intent asks for confirmation) or [`Plan::Ready`] with the normalized parameters.
//!
//! Mirrors the official articles on resolving parameters and confirming the details of an intent. See
//! docs/bifrost-ui.md "App Intents".

use crate::intents::{Entity, Intent, Param, ParamType};
use bifrost_core::json::Json;

/// The resolution result for one parameter: success, not required, and the rest.
#[derive(Clone, Debug, PartialEq)]
pub enum Resolution {
    /// The value is usable as given (normalized: an entity id is returned as the matched entity's id).
    Success(Json),
    /// The parameter is optional and absent: it is left out of the normalized parameters.
    NotRequired,
    /// A required value is missing; the caller must ask for it.
    NeedsValue,
    /// The value is well formed but cannot be fulfilled, with the reason to show.
    Unsupported(String),
    /// The value matches several choices; the caller must let the user pick one of these.
    Disambiguate(Vec<Entity>),
}

/// Resolves one parameter. `choices` are the entities that the caller found for an entity parameter (for example
/// the result of `entity.query` with a string); they are ignored for other types.
pub fn resolve_param(p: &Param, value: Option<&Json>, choices: &[Entity]) -> Resolution {
    let value = match value {
        None | Some(Json::Null) if p.optional => return Resolution::NotRequired,
        None | Some(Json::Null) => return Resolution::NeedsValue,
        Some(v) => v,
    };
    if !p.ty.accepts(value) {
        return Resolution::Unsupported(format!("{} must be {}", p.name, p.ty.name()));
    }
    match &p.ty {
        ParamType::Entity(ty) => {
            let id = value.as_str().unwrap_or_default();
            if let Some(hit) = choices.iter().find(|e| e.id == id && e.ty == *ty) {
                return Resolution::Success(Json::Str(hit.id.clone()));
            }
            match choices.len() {
                0 => Resolution::Unsupported(format!("no {ty} matches {id}")),
                1 => Resolution::Success(Json::Str(choices[0].id.clone())),
                _ => Resolution::Disambiguate(choices.iter().filter(|e| e.ty == *ty).cloned().collect()),
            }
        }
        _ => Resolution::Success(value.clone()),
    }
}

/// The next step for an intent, after its parameters were resolved.
#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
    /// Every parameter is settled and no confirmation is needed: perform with these parameters.
    Ready(Json),
    /// Every parameter is settled; the user must confirm these parameters before the intent is performed.
    Confirm(Json),
    /// `param` has no value yet: ask the user for it.
    Ask { param: String },
    /// `param` matches several choices: let the user pick one.
    Choose { param: String, options: Vec<Entity> },
    /// `param` cannot be fulfilled, or the parameters are malformed (unknown name, wrong type): stop with `reason`.
    Blocked { param: String, reason: String },
}

/// Plans an intent from its raw parameters. `choices` maps a parameter name to the entities the caller found for
/// it; a parameter missing from the list has no choices. A missing required value is a question ([`Plan::Ask`]),
/// not an error, so this does not call [`Intent::validate`]. Unknown names and a non-object request are reported
/// with an empty `param` in [`Plan::Blocked`].
pub fn plan(intent: &Intent, params: &Json, choices: &[(&str, Vec<Entity>)], needs_confirmation: bool) -> Plan {
    let fields: &[(String, Json)] = match params {
        Json::Obj(f) => f,
        Json::Null => &[],
        _ => return Plan::Blocked { param: String::new(), reason: "params must be an object".into() },
    };
    if let Some((k, _)) = fields.iter().find(|(k, _)| !intent.params.iter().any(|p| &p.name == k)) {
        return Plan::Blocked { param: String::new(), reason: format!("unknown parameter {k}") };
    }
    let mut normalized = Vec::new();
    for p in &intent.params {
        let given = fields.iter().find(|(k, _)| k == &p.name).map(|(_, v)| v);
        let found: &[Entity] = choices.iter().find(|(n, _)| *n == p.name).map(|(_, c)| c.as_slice()).unwrap_or(&[]);
        match resolve_param(p, given, found) {
            Resolution::Success(v) => normalized.push((p.name.clone(), v)),
            Resolution::NotRequired => {}
            Resolution::NeedsValue => return Plan::Ask { param: p.name.clone() },
            Resolution::Disambiguate(options) => return Plan::Choose { param: p.name.clone(), options },
            Resolution::Unsupported(reason) => return Plan::Blocked { param: p.name.clone(), reason },
        }
    }
    let params = Json::Obj(normalized);
    if needs_confirmation {
        Plan::Confirm(params)
    } else {
        Plan::Ready(params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(id: &str, title: &str) -> Entity {
        Entity::new("Folder", id, title)
    }

    fn move_note() -> Intent {
        Intent::new("move-note", "Move Note", "Moves a note to a folder")
            .param(Param::new("text", "Note", ParamType::String))
            .param(Param::new("folder", "Folder", ParamType::Entity("Folder".into())))
            .param(Param::new("due", "Due", ParamType::Date).optional())
    }

    fn s(v: &str) -> Json {
        Json::Str(v.into())
    }

    #[test]
    fn missing_required_value_asks_and_optional_value_is_not_required() {
        let p = Param::new("text", "Note", ParamType::String);
        assert_eq!(resolve_param(&p, None, &[]), Resolution::NeedsValue);
        assert_eq!(resolve_param(&p, Some(&Json::Null), &[]), Resolution::NeedsValue);
        let due = Param::new("due", "Due", ParamType::Date).optional();
        assert_eq!(resolve_param(&due, None, &[]), Resolution::NotRequired);
    }

    #[test]
    fn entity_with_one_match_resolves_and_several_matches_disambiguate() {
        let p = Param::new("folder", "Folder", ParamType::Entity("Folder".into()));
        let one = [folder("f1", "Work")];
        assert_eq!(resolve_param(&p, Some(&s("f1")), &one), Resolution::Success(s("f1")));
        let two = [folder("f1", "Work"), folder("f2", "Work archive")];
        match resolve_param(&p, Some(&s("work")), &two) {
            Resolution::Disambiguate(opts) => assert_eq!(opts.len(), 2),
            other => panic!("expected disambiguation, got {other:?}"),
        }
        assert!(matches!(resolve_param(&p, Some(&s("zz")), &[]), Resolution::Unsupported(_)));
    }

    #[test]
    fn plan_asks_for_the_first_missing_parameter_in_declaration_order() {
        let i = move_note();
        let params = Json::Obj(vec![]);
        assert_eq!(plan(&i, &params, &[], false), Plan::Ask { param: "text".into() });
        let params = Json::Obj(vec![("text".into(), s("milk"))]);
        assert_eq!(plan(&i, &params, &[], false), Plan::Ask { param: "folder".into() });
    }

    #[test]
    fn plan_lets_the_user_choose_between_entity_matches() {
        let i = move_note();
        let params = Json::Obj(vec![("text".into(), s("milk")), ("folder".into(), s("work"))]);
        let choices = [("folder", vec![folder("f1", "Work"), folder("f2", "Work archive")])];
        match plan(&i, &params, &choices, false) {
            Plan::Choose { param, options } => {
                assert_eq!(param, "folder");
                assert_eq!(options.len(), 2);
            }
            other => panic!("expected choice, got {other:?}"),
        }
    }

    #[test]
    fn plan_is_ready_with_normalized_params_or_asks_to_confirm() {
        let i = move_note();
        let params = Json::Obj(vec![("text".into(), s("milk")), ("folder".into(), s("f1"))]);
        let choices = [("folder", vec![folder("f1", "Work")])];
        let ready = Json::Obj(vec![("text".into(), s("milk")), ("folder".into(), s("f1"))]);
        assert_eq!(plan(&i, &params, &choices, false), Plan::Ready(ready.clone()));
        assert_eq!(plan(&i, &params, &choices, true), Plan::Confirm(ready));
    }

    #[test]
    fn plan_blocks_malformed_requests_and_unfulfillable_values() {
        let i = move_note();
        let unknown = Json::Obj(vec![("text".into(), s("a")), ("folder".into(), s("f1")), ("color".into(), s("red"))]);
        match plan(&i, &unknown, &[], false) {
            Plan::Blocked { param, reason } => {
                assert!(param.is_empty());
                assert!(reason.contains("color"));
            }
            other => panic!("expected block, got {other:?}"),
        }
        let bad_date = Json::Obj(vec![("text".into(), s("a")), ("folder".into(), s("f1")), ("due".into(), s("someday"))]);
        assert!(matches!(plan(&i, &bad_date, &[("folder", vec![folder("f1", "Work")])], false), Plan::Blocked { .. }));
        let no_match = Json::Obj(vec![("text".into(), s("a")), ("folder".into(), s("nope"))]);
        match plan(&i, &no_match, &[], false) {
            Plan::Blocked { param, .. } => assert_eq!(param, "folder"),
            other => panic!("expected block on folder, got {other:?}"),
        }
    }
}
