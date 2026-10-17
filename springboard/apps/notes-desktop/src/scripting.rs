//! Notes' scripting answers (`[[script.*]]` in manifest.toml). `make new note with properties {title, body}` arrives as
//! the AppEvent `notes.make` and becomes a new note through the Create Note intent; `get name of every note` (and the
//! other properties and specifiers) arrive as the core event `getd` with an `object` record and are read from the same
//! note list `notes.list` returns. Anything else is refused with a reason.

use bifrost_app::control::{RpcError, INVALID_PARAMS, METHOD_NOT_FOUND};
use bifrost_core::json::Json;

/// The properties a script can read from a note: `name` is its title, the rest are the summary fields.
pub const PROPERTIES: &[&str] = &["name", "id", "folder", "modified"];

/// The text of a new note from `make new note` parameters. The note's title is its first line, so a title and a body
/// become `title` + newline + `body`; either may be missing.
pub fn note_text(params: &Json) -> Result<String, RpcError> {
    let field = |key: &str| -> Result<String, RpcError> {
        match params.get(key) {
            None | Some(Json::Null) => Ok(String::new()),
            Some(Json::Str(s)) => Ok(s.clone()),
            Some(_) => Err(RpcError::new(INVALID_PARAMS, format!("{key} must be text"))),
        }
    };
    let title = field("title")?;
    let body = field("body")?;
    Ok(match (title.is_empty(), body.is_empty()) {
        (true, _) => body,
        (false, true) => title,
        (false, false) => format!("{title}\n{body}"),
    })
}

/// One property of a note summary (`notes.list` row): `None` is the whole row.
fn read(row: &Json, property: Option<&str>) -> Result<Json, RpcError> {
    match property {
        None => Ok(row.clone()),
        Some("name") => Ok(row.get("title").cloned().unwrap_or(Json::Null)),
        Some(p) if PROPERTIES.contains(&p) => Ok(row.get(p).cloned().unwrap_or(Json::Null)),
        Some(p) => Err(RpcError::new(INVALID_PARAMS, format!("notes have no property {p}"))),
    }
}

/// Reads a `getd` event: `object` is the specifier record (`{class, all | index | name | whose: {property, value}}`) and
/// `property` the property asked for, or none for the object itself. `rows` is the `notes.list` result. An `all`
/// specifier gives a list; the others give one value (the first match).
pub fn get_property(rows: &Json, object: &Json, property: Option<&str>) -> Result<Json, RpcError> {
    let class = object.str_field("class").unwrap_or_default();
    if class != "note" {
        return Err(RpcError::new(INVALID_PARAMS, format!("no such class {class}")));
    }
    let all: &[Json] = rows.as_array().unwrap_or(&[]);
    let title_is = |row: &&Json, want: &str| row.str_field("title").as_deref() == Some(want);
    if object.get("all").is_some() {
        let values: Result<Vec<Json>, RpcError> = all.iter().map(|r| read(r, property)).collect();
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
            _ => return Err(RpcError::new(INVALID_PARAMS, "a note can be picked by name only")),
        }
    } else {
        return Err(RpcError::new(INVALID_PARAMS, "the object names no note"));
    };
    match found {
        Some(row) => read(row, property),
        None => Err(RpcError::new(INVALID_PARAMS, "no such note")),
    }
}

/// The reply for an event the dictionary does not declare.
pub fn unhandled(method: &str) -> RpcError {
    RpcError::new(METHOD_NOT_FOUND, format!("Notes does not handle {method}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::json::parse;

    fn j(text: &str) -> Json {
        parse(text).unwrap()
    }

    fn rows() -> Json {
        j(r#"[{"id":"a","title":"Shopping","folder":null,"modified":3},{"id":"b","title":"Ideas","folder":"Work","modified":2}]"#)
    }

    #[test]
    fn a_new_note_takes_its_title_as_the_first_line() {
        assert_eq!(note_text(&j(r#"{"title":"Milk","body":"two litres"}"#)).unwrap(), "Milk\ntwo litres");
        assert_eq!(note_text(&j(r#"{"body":"only"}"#)).unwrap(), "only");
        assert_eq!(note_text(&j(r#"{"title":"Only"}"#)).unwrap(), "Only");
        assert_eq!(note_text(&j("{}")).unwrap(), "");
        assert!(note_text(&j(r#"{"title":3}"#)).is_err());
    }

    #[test]
    fn every_note_gives_a_list_of_the_property() {
        let object = j(r#"{"class":"note","all":true}"#);
        assert_eq!(get_property(&rows(), &object, Some("name")).unwrap(), j(r#"["Shopping","Ideas"]"#));
    }

    #[test]
    fn an_index_or_a_name_gives_one_note() {
        let by_index = j(r#"{"class":"note","index":2}"#);
        assert_eq!(get_property(&rows(), &by_index, Some("folder")).unwrap(), Json::Str("Work".into()));
        let by_name = j(r#"{"class":"note","name":"Shopping"}"#);
        assert_eq!(get_property(&rows(), &by_name, Some("id")).unwrap(), Json::Str("a".into()));
        let whole = get_property(&rows(), &by_name, None).unwrap();
        assert_eq!(whole.str_field("title").as_deref(), Some("Shopping"));
    }

    #[test]
    fn a_whose_clause_picks_by_name_and_nothing_else() {
        let object = j(r#"{"class":"note","whose":{"property":"name","value":"Ideas"}}"#);
        assert_eq!(get_property(&rows(), &object, Some("modified")).unwrap(), Json::Num(2.0));
        let other = j(r#"{"class":"note","whose":{"property":"folder","value":"Work"}}"#);
        assert!(get_property(&rows(), &other, Some("name")).is_err());
    }

    #[test]
    fn unknown_classes_properties_and_objects_are_refused() {
        assert!(get_property(&rows(), &j(r#"{"class":"window","index":1}"#), Some("name")).is_err());
        assert!(get_property(&rows(), &j(r#"{"class":"note","index":1}"#), Some("body")).is_err(), "no body property yet");
        assert!(get_property(&rows(), &j(r#"{"class":"note","index":9}"#), Some("name")).is_err());
        assert!(get_property(&rows(), &j(r#"{"class":"note","index":0}"#), Some("name")).is_err());
    }
}
