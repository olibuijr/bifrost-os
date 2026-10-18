//! Reminders' scripting answers (`[[script.*]]` in manifest.toml). `make new reminder with properties {name, due}` arrives
//! as the AppEvent `reminders.make` and becomes a reminder through the Add Reminder intent; `make new list` arrives as
//! `reminders.makelist` and goes through Create List. `get name of every reminder` (and the other properties and
//! specifiers) arrive as the core event `getd` with an `object` record and are read from `rows`, the snapshot built
//! here from the open store. Setting properties and deleting reminders from a script are refused for now.

use crate::model::{ListDef, Reminder};
use bifrost_core::json::Json;

use bifrost_app::control::{RpcError, INVALID_PARAMS, METHOD_NOT_FOUND};

/// The properties a script can read from a reminder: `name` is its title; `due` is the local day number (days since
/// 1970-01-01), or missing when the reminder has no due date; `list` is the list's name.
pub const PROPERTIES: &[&str] = &["name", "id", "notes", "completed", "flagged", "priority", "due", "list"];

/// One reminder as a script sees it (the row `getd` reads from).
pub fn row(r: &Reminder, lists: &[ListDef]) -> Json {
    let list = lists.iter().find(|l| l.id == r.list).map_or_else(String::new, |l| l.name.clone());
    Json::Obj(vec![
        ("id".into(), Json::Num(r.id as f64)),
        ("name".into(), Json::Str(r.title.clone())),
        ("notes".into(), Json::Str(r.notes.clone())),
        ("completed".into(), Json::Bool(r.completed)),
        ("flagged".into(), Json::Bool(r.flagged)),
        ("priority".into(), Json::Num(f64::from(r.priority))),
        ("due".into(), r.due.map_or(Json::Null, |d| Json::Num(d as f64))),
        ("list".into(), Json::Str(list)),
    ])
}

/// The snapshot `getd` reads: every reminder, top level and nested, in store order.
pub fn rows(items: &[Reminder], lists: &[ListDef]) -> Json {
    Json::Arr(items.iter().map(|r| row(r, lists)).collect())
}

/// The title and due text of a `make new reminder` (`title` from `name`, the due day text from `due`). Either may be
/// missing; a title is required before the reminder is made, so an empty one is refused by the intent.
pub fn reminder_params(params: &Json) -> Result<Json, RpcError> {
    let text = |key: &str| -> Result<Option<String>, RpcError> {
        match params.get(key) {
            None | Some(Json::Null) => Ok(None),
            Some(Json::Str(s)) => Ok(Some(s.clone())),
            Some(_) => Err(RpcError::new(INVALID_PARAMS, format!("{key} must be text"))),
        }
    };
    let title = text("name")?.or(text("title")?).unwrap_or_default();
    if title.trim().is_empty() {
        return Err(RpcError::new(INVALID_PARAMS, "a reminder needs a name"));
    }
    let mut fields = vec![("title".to_string(), Json::Str(title))];
    if let Some(due) = text("due")? {
        fields.push(("due".into(), Json::Str(due)));
    }
    Ok(Json::Obj(fields))
}

/// The name of a `make new list` (`name`), refused when missing.
pub fn list_params(params: &Json) -> Result<Json, RpcError> {
    match params.get("name") {
        Some(Json::Str(s)) if !s.trim().is_empty() => Ok(Json::Obj(vec![("name".into(), Json::Str(s.clone()))])),
        Some(Json::Str(_)) | None | Some(Json::Null) => Err(RpcError::new(INVALID_PARAMS, "a list needs a name")),
        Some(_) => Err(RpcError::new(INVALID_PARAMS, "name must be text")),
    }
}

/// One property of a row: `None` is the whole row.
fn read(row: &Json, property: Option<&str>, known: &[&str], class: &str) -> Result<Json, RpcError> {
    match property {
        None => Ok(row.clone()),
        Some("name") => Ok(row.get("name").cloned().unwrap_or(Json::Null)),
        Some(p) if known.contains(&p) => Ok(row.get(p).cloned().unwrap_or(Json::Null)),
        Some(p) => Err(RpcError::new(INVALID_PARAMS, format!("{class}s have no property {p}"))),
    }
}

/// Reads a `getd` event for reminders. `object` is the specifier record (`{class, all | index | name | whose:
/// {property, value}}`) and `property` the property asked for, or none for the object itself. `rows` is the snapshot
/// from `rows`. An `all` specifier gives a list; the others give one value (the first match).
pub fn get_property(rows: &Json, object: &Json, property: Option<&str>) -> Result<Json, RpcError> {
    let class = object.str_field("class").unwrap_or_default();
    if class != "reminder" {
        return Err(RpcError::new(INVALID_PARAMS, format!("no such class {class}")));
    }
    let all: &[Json] = rows.as_array().unwrap_or(&[]);
    let title_is = |row: &&Json, want: &str| row.str_field("name").as_deref() == Some(want);
    if object.get("all").is_some() {
        let values: Result<Vec<Json>, RpcError> = all.iter().map(|r| read(r, property, PROPERTIES, "reminder")).collect();
        return values.map(Json::Arr);
    }
    let found: Option<&Json> = if let Some(n) = object.get("index").and_then(Json::number) {
        let k = n as usize;
        if k == 0 {
            None
        } else {
            all.get(k - 1)
        }
    } else if let Some(name) = object.str_field("name") {
        all.iter().find(|r| title_is(r, &name))
    } else if let Some(whose) = object.get("whose") {
        match whose.str_field("property").as_deref() {
            Some("name") => {
                let value = whose.str_field("value").unwrap_or_default();
                all.iter().find(|r| title_is(r, &value))
            }
            _ => return Err(RpcError::new(INVALID_PARAMS, "a reminder can be picked by name only")),
        }
    } else {
        return Err(RpcError::new(INVALID_PARAMS, "the object names no reminder"));
    };
    match found {
        Some(row) => read(row, property, PROPERTIES, "reminder"),
        None => Err(RpcError::new(INVALID_PARAMS, "no such reminder")),
    }
}

/// The reply for an event the dictionary does not declare.
pub fn unhandled(method: &str) -> RpcError {
    RpcError::new(METHOD_NOT_FOUND, format!("Reminders does not handle {method}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::json::parse;

    fn j(text: &str) -> Json {
        parse(text).unwrap()
    }

    fn snapshot() -> Json {
        j(r#"[{"id":1,"name":"Milk","notes":"","completed":false,"flagged":true,"priority":2,"due":20000,"list":"Shopping"},
              {"id":2,"name":"Post letter","notes":"","completed":true,"flagged":false,"priority":0,"due":null,"list":"Home"}]"#)
    }

    #[test]
    fn a_new_reminder_needs_a_name_and_takes_an_optional_due() {
        let made = reminder_params(&j(r#"{"name":"Call mum","due":"tomorrow"}"#)).unwrap();
        assert_eq!(made.str_field("title").as_deref(), Some("Call mum"));
        assert_eq!(made.str_field("due").as_deref(), Some("tomorrow"));
        let bare = reminder_params(&j(r#"{"name":"Milk"}"#)).unwrap();
        assert!(bare.get("due").is_none());
        assert!(reminder_params(&j(r#"{"name":"  "}"#)).is_err());
        assert!(reminder_params(&j("{}")).is_err());
        assert!(reminder_params(&j(r#"{"name":3}"#)).is_err());
    }

    #[test]
    fn a_new_list_needs_a_name() {
        assert_eq!(list_params(&j(r#"{"name":"Work"}"#)).unwrap().str_field("name").as_deref(), Some("Work"));
        assert!(list_params(&j("{}")).is_err());
    }

    #[test]
    fn every_reminder_gives_a_list_of_the_property() {
        let object = j(r#"{"class":"reminder","all":true}"#);
        assert_eq!(get_property(&snapshot(), &object, Some("name")).unwrap(), j(r#"["Milk","Post letter"]"#));
        assert_eq!(get_property(&snapshot(), &object, Some("completed")).unwrap(), j("[false,true]"));
    }

    #[test]
    fn an_index_or_a_name_gives_one_reminder() {
        let by_index = j(r#"{"class":"reminder","index":2}"#);
        assert_eq!(get_property(&snapshot(), &by_index, Some("list")).unwrap(), Json::Str("Home".into()));
        let by_name = j(r#"{"class":"reminder","name":"Milk"}"#);
        assert_eq!(get_property(&snapshot(), &by_name, Some("due")).unwrap(), Json::Num(20000.0));
        let whole = get_property(&snapshot(), &by_name, None).unwrap();
        assert_eq!(whole.str_field("name").as_deref(), Some("Milk"));
    }

    #[test]
    fn a_missing_due_date_reads_as_null() {
        let object = j(r#"{"class":"reminder","index":2}"#);
        assert_eq!(get_property(&snapshot(), &object, Some("due")).unwrap(), Json::Null);
    }

    #[test]
    fn a_whose_clause_picks_by_name_and_nothing_else() {
        let object = j(r#"{"class":"reminder","whose":{"property":"name","value":"Milk"}}"#);
        assert_eq!(get_property(&snapshot(), &object, Some("flagged")).unwrap(), Json::Bool(true));
        let other = j(r#"{"class":"reminder","whose":{"property":"list","value":"Home"}}"#);
        assert!(get_property(&snapshot(), &other, Some("name")).is_err());
    }

    #[test]
    fn unknown_classes_properties_and_objects_are_refused() {
        assert!(get_property(&snapshot(), &j(r#"{"class":"note","index":1}"#), Some("name")).is_err());
        assert!(get_property(&snapshot(), &j(r#"{"class":"reminder","index":1}"#), Some("body")).is_err());
        assert!(get_property(&snapshot(), &j(r#"{"class":"reminder","index":9}"#), Some("name")).is_err());
        assert!(get_property(&snapshot(), &j(r#"{"class":"reminder","index":0}"#), Some("name")).is_err());
    }
}
