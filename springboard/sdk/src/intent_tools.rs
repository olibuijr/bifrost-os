//! App intents as model tools. An intent an app declares (`App::intents()`) can be offered to the on-device model as a
//! tool named after the intent's id: its parameters become the tool's argument schema, and a call runs the intent with
//! the arguments the model chose. The app supplies the function that performs the intent, so the model and the control
//! socket (`intent.<id>`) share one code path. An intent that needs authentication is never offered to the model.
//!
//! The model layer is [`crate::language_model`]; the intent model is [`crate::intents`].

use crate::intents::{Authentication, Intent, Param, ParamType};
use crate::language_model::Tool;
use bifrost_core::json::Json;
use bifrost_ui::generation_schema::{Kind, Property, Value};

/// The argument schema for one parameter type. Types a schema cannot express (a date, a URL, an entity id) are text;
/// their exact form is checked with [`ParamType::accepts`] when the call runs.
pub fn kind_of(ty: &ParamType) -> Kind {
    match ty {
        ParamType::Int | ParamType::Double => Kind::Number(None),
        ParamType::Bool => Kind::Bool,
        ParamType::Enum(cases) => Kind::Choice(cases.clone()),
        ParamType::Currency => Kind::Object(vec![
            Property::new("amount", Kind::Number(None)).described("the amount"),
            Property::new("code", Kind::Text).described("ISO 4217 currency code, three capitals"),
        ]),
        ParamType::Placemark => Kind::Object(vec![
            Property::new("latitude", Kind::Number(Some((-90.0, 90.0)))).described("degrees"),
            Property::new("longitude", Kind::Number(Some((-180.0, 180.0)))).described("degrees"),
            Property::new("name", Kind::Text).described("place name").optional(),
        ]),
        ParamType::Measurement => Kind::Object(vec![
            Property::new("value", Kind::Number(None)).described("the amount"),
            Property::new("unit", Kind::Text).described("the unit, such as km"),
        ]),
        _ => Kind::Text,
    }
}

fn property_of(p: &Param) -> Property {
    let prop = Property::new(&p.name, kind_of(&p.ty)).described(&p.title);
    if p.optional {
        prop.optional()
    } else {
        prop
    }
}

fn to_json(v: &Value) -> Json {
    match v {
        Value::Null => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Number(n) => Json::Num(*n),
        Value::Text(s) => Json::Str(s.clone()),
        Value::Array(items) => Json::Arr(items.iter().map(to_json).collect()),
        Value::Object(fields) => Json::Obj(fields.iter().map(|(k, v)| (k.clone(), to_json(v))).collect()),
    }
}

/// The tool for `intent`, or None when the intent needs authentication, which the model never gets to run. `run` performs
/// the intent with the arguments as a JSON object (the same object `intent.<id>` receives) and returns the text the model
/// reads. Arguments that the schema accepts but the parameter type does not (a bad date or URL) are refused before `run`.
pub fn tool_for_intent(intent: &Intent, run: impl Fn(&Json) -> Result<String, String> + 'static) -> Option<Tool> {
    if !matches!(intent.authentication, Authentication::AlwaysAllowed) {
        return None;
    }
    let description = if intent.description.trim().is_empty() { intent.title.clone() } else { intent.description.clone() };
    let schema = Kind::Object(intent.params.iter().map(property_of).collect());
    let params: Vec<Param> = intent.params.clone();
    Some(Tool::new(&intent.id, &description, schema, move |args| {
        let json = to_json(args);
        for p in &params {
            match json.get(&p.name) {
                Some(v) if !p.ty.accepts(v) => return Err(format!("`{}` does not fit {}", p.name, p.ty.name())),
                None if !p.optional => return Err(format!("`{}` is required", p.name)),
                _ => {}
            }
        }
        run(&json)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language_model::{ToolCall, ToolSet};
    use std::cell::Cell;
    use std::rc::Rc;

    fn open_note() -> Intent {
        Intent::new("open-note", "Open Note", "Opens a note by its number.")
            .param(Param::new("number", "Number", ParamType::Int))
            .param(Param::new("folder", "Folder", ParamType::Enum(vec!["work".into(), "home".into()])))
            .param(Param::new("link", "Link", ParamType::Url).optional())
    }

    fn call(name: &str, arguments: &str) -> ToolCall {
        ToolCall { name: name.into(), arguments: arguments.into() }
    }

    #[test]
    fn an_intent_becomes_a_tool_named_after_its_id_and_runs_with_the_arguments() {
        let runs = Rc::new(Cell::new(0));
        let seen = runs.clone();
        let tool = tool_for_intent(&open_note(), move |args| {
            seen.set(seen.get() + 1);
            Ok(format!("{:?}", args.get("number")))
        })
        .expect("no authentication, so the model is offered the tool");
        let mut set = ToolSet::new();
        set.register(tool).unwrap();
        assert_eq!(set.names(), vec!["open-note"]);
        assert_eq!(set.call(&call("open-note", "{\"number\":4,\"folder\":\"work\"}")), Ok("Some(Num(4.0))".to_string()));
        assert_eq!(runs.get(), 1);
    }

    #[test]
    fn an_argument_the_parameter_type_refuses_stops_the_call_before_the_intent_runs() {
        let runs = Rc::new(Cell::new(0));
        let seen = runs.clone();
        let tool = tool_for_intent(&open_note(), move |_| {
            seen.set(seen.get() + 1);
            Ok(String::new())
        })
        .unwrap();
        let mut set = ToolSet::new();
        set.register(tool).unwrap();
        // the schema accepts any text for a URL, but `not a link` is not a URL
        let out = set.call(&call("open-note", "{\"number\":4,\"folder\":\"work\",\"link\":\"not a link\"}"));
        assert!(out.is_err(), "{out:?}");
        assert_eq!(runs.get(), 0);
    }

    #[test]
    fn a_required_parameter_that_is_missing_is_refused_and_an_optional_one_may_be_left_out() {
        let mut set = ToolSet::new();
        set.register(tool_for_intent(&open_note(), |_| Ok("opened".into())).unwrap()).unwrap();
        assert!(set.call(&call("open-note", "{\"folder\":\"home\"}")).is_err(), "number is required");
        assert_eq!(set.call(&call("open-note", "{\"number\":2,\"folder\":\"home\"}")), Ok("opened".to_string()));
    }

    #[test]
    fn an_intent_that_needs_authentication_is_not_offered_to_the_model() {
        let mut locked = open_note();
        locked.authentication = Authentication::RequiresAuthentication;
        assert!(tool_for_intent(&locked, |_| Ok(String::new())).is_none());
    }

    #[test]
    fn the_schema_maps_each_parameter_type_to_its_argument_kind() {
        assert_eq!(kind_of(&ParamType::Int), Kind::Number(None));
        assert_eq!(kind_of(&ParamType::Enum(vec!["open".into(), "closed".into()])), Kind::Choice(vec!["open".into(), "closed".into()]));
        assert_eq!(kind_of(&ParamType::Url), Kind::Text);
        let Kind::Object(currency) = kind_of(&ParamType::Currency) else { panic!("a currency is an object") };
        assert_eq!(currency.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), vec!["amount", "code"]);
    }
}
