//! Scripting dictionaries: what an app can be told to do from a script. A manifest declares them as
//! `[[script.command]]` tables (`name`, `event = "class.id"`, `params = [...]`, `result`) and `[[script.class]]`
//! tables (`name`, `properties = [...]`, `elements = [...]`). A command is sent to the app as the AppEvent
//! `ae.<class>.<id>` (see `sdk/src/app_events.rs`), so the dictionary is the only place that maps script words to events.
//!
//! ```toml
//! [[script.command]]
//! name = "make new note"
//! event = "notes.make"
//! params = ["title", "body"]
//! result = "note"
//!
//! [[script.class]]
//! name = "note"
//! properties = ["name", "body"]
//! elements = []
//! ```

use crate::json::Json;

/// One `[[script.command]]` table. `name` is lowercase words separated by single spaces.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Command {
    pub name: String,
    /// The event class (the part of `event` before its first dot).
    pub class: String,
    /// The event id (the rest of `event`, after its first dot).
    pub id: String,
    /// Declared parameter labels, in order. A direct argument goes to the first one.
    pub params: Vec<String>,
    /// The class of the result, or empty when the command returns nothing a script can name.
    pub result: String,
}

impl Command {
    /// The control-socket method this command is sent as.
    pub fn method(&self) -> String {
        format!("ae.{}.{}", self.class, self.id)
    }
}

/// One `[[script.class]]` table: an object class with its properties and the classes it contains.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Class {
    pub name: String,
    pub properties: Vec<String>,
    pub elements: Vec<String>,
}

/// An app's scripting dictionary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dictionary {
    pub commands: Vec<Command>,
    pub classes: Vec<Class>,
}

/// Lowercase words separated by single spaces: the form command and class names are compared in.
pub fn normalize_name(text: &str) -> String {
    text.split_whitespace().map(|w| w.to_lowercase()).collect::<Vec<_>>().join(" ")
}

/// A name that can be used as a label, class or id: ASCII letters, digits, `-` and `_`.
fn safe_word(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// `"x"` without quotes, or the text as written when unquoted.
fn unquote(text: &str) -> String {
    let t = text.trim();
    t.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(t).to_string()
}

/// `["a", "b"]` as its items; empty items are dropped.
fn list(text: &str) -> Vec<String> {
    text.trim().trim_start_matches('[').trim_end_matches(']').split(',').map(unquote).filter(|s| !s.is_empty()).collect()
}

impl Dictionary {
    /// Reads the `[[script.*]]` tables out of a manifest's text. Other tables and keys are ignored; a table with a
    /// missing or unsafe name or event is dropped rather than failing the whole manifest.
    pub fn parse(text: &str) -> Dictionary {
        let mut d = Dictionary::default();
        let mut cmd: Option<Command> = None;
        let mut cls: Option<Class> = None;
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && !line.contains('=') {
                if let Some(c) = cmd.take() {
                    d.push_command(c);
                }
                if let Some(c) = cls.take() {
                    d.push_class(c);
                }
                match line {
                    "[[script.command]]" => cmd = Some(Command::default()),
                    "[[script.class]]" => cls = Some(Class::default()),
                    _ => {}
                }
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            if let Some(c) = cmd.as_mut() {
                match k {
                    "name" => c.name = normalize_name(&unquote(v)),
                    "event" => {
                        if let Some((class, id)) = unquote(v).split_once('.') {
                            c.class = class.trim().to_string();
                            c.id = id.trim().to_string();
                        }
                    }
                    "params" => c.params = list(v),
                    "result" => c.result = normalize_name(&unquote(v)),
                    _ => {}
                }
            } else if let Some(c) = cls.as_mut() {
                match k {
                    "name" => c.name = normalize_name(&unquote(v)),
                    "properties" => c.properties = list(v),
                    "elements" => c.elements = list(v),
                    _ => {}
                }
            }
        }
        if let Some(c) = cmd {
            d.push_command(c);
        }
        if let Some(c) = cls {
            d.push_class(c);
        }
        d
    }

    fn push_command(&mut self, mut c: Command) {
        c.params.retain(|p| safe_word(p));
        let valid = !c.name.is_empty() && safe_word(&c.class) && safe_word(&c.id) && !self.commands.iter().any(|o| o.name == c.name);
        if valid {
            self.commands.push(c);
        }
    }

    fn push_class(&mut self, mut c: Class) {
        c.name = normalize_name(&c.name);
        c.properties.retain(|p| safe_word(p));
        c.elements.retain(|p| safe_word(p));
        if !c.name.is_empty() && !self.classes.iter().any(|o| o.name == c.name) {
            self.classes.push(c);
        }
    }

    /// No commands and no classes: an app with nothing to script.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty() && self.classes.is_empty()
    }

    /// The command a script phrase names (case and spacing do not matter).
    pub fn command(&self, name: &str) -> Option<&Command> {
        let name = normalize_name(name);
        self.commands.iter().find(|c| c.name == name)
    }

    /// The object class a script names (case and spacing do not matter).
    pub fn class(&self, name: &str) -> Option<&Class> {
        let name = normalize_name(name);
        self.classes.iter().find(|c| c.name == name)
    }

    /// `{"commands": [{name, event, params, result}], "classes": [{name, properties, elements}]}`.
    pub fn to_json(&self) -> Json {
        let strs = |v: &[String]| Json::Arr(v.iter().cloned().map(Json::Str).collect());
        let commands = self
            .commands
            .iter()
            .map(|c| {
                Json::Obj(vec![
                    ("name".into(), Json::Str(c.name.clone())),
                    ("event".into(), Json::Str(format!("{}.{}", c.class, c.id))),
                    ("params".into(), strs(&c.params)),
                    ("result".into(), Json::Str(c.result.clone())),
                ])
            })
            .collect();
        let classes = self
            .classes
            .iter()
            .map(|c| Json::Obj(vec![("name".into(), Json::Str(c.name.clone())), ("properties".into(), strs(&c.properties)), ("elements".into(), strs(&c.elements))]))
            .collect();
        Json::Obj(vec![("commands".into(), Json::Arr(commands)), ("classes".into(), Json::Arr(classes))])
    }
}

/// The parameter record of a command call. The direct argument goes to the first declared parameter (or `direct`
/// when the command declares none); `with label value` arguments go under their labels. A label the command does not
/// declare is an error, and so is a label given twice.
pub fn command_params(cmd: &Command, direct: Option<Json>, with: Vec<(String, Json)>) -> Result<Json, String> {
    let mut fields: Vec<(String, Json)> = Vec::new();
    if let Some(value) = direct {
        let key = cmd.params.first().cloned().unwrap_or_else(|| "direct".to_string());
        fields.push((key, value));
    }
    for (label, value) in with {
        let label = label.to_lowercase();
        if !cmd.params.is_empty() && !cmd.params.contains(&label) {
            return Err(format!("{} has no parameter {label}", cmd.name));
        }
        if fields.iter().any(|(k, _)| *k == label) {
            return Err(format!("{} gets {label} twice", cmd.name));
        }
        fields.push((label, value));
    }
    Ok(Json::Obj(fields))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"
id = "is.example.notes"
name = "Notes"
exec = "notes"

[privacy]
location = "Shows notes near you"

[[script.command]]
name = "Make   New Note"
event = "notes.make"
params = ["title", "body"]
result = "note"

[[script.command]]
name = "get name"
event = "core.getd"

[[script.command]]
name = "bad"
event = "nodot"

[[script.class]]
name = "Note"
properties = ["name", "body", "bad prop"]
elements = []

[[intent]]
id = "add"
title = "Add"
"#;

    #[test]
    fn parses_commands_and_classes_and_drops_invalid_ones() {
        let d = Dictionary::parse(MANIFEST);
        assert_eq!(d.commands.len(), 2, "the command without a dot in its event is dropped");
        let make = d.command("make new note").expect("names are normalized");
        assert_eq!((make.class.as_str(), make.id.as_str()), ("notes", "make"));
        assert_eq!(make.params, vec!["title", "body"]);
        assert_eq!(make.result, "note");
        assert_eq!(make.method(), "ae.notes.make");
        assert_eq!(d.classes.len(), 1);
        let note = d.class("NOTE").unwrap();
        assert_eq!(note.properties, vec!["name", "body"], "a property with a space is unsafe and dropped");
        assert!(d.command("delete note").is_none());
    }

    #[test]
    fn a_manifest_without_script_tables_has_an_empty_dictionary() {
        assert!(Dictionary::parse("id = \"x\"\n[[intent]]\nid = \"a\"\n").is_empty());
    }

    #[test]
    fn direct_argument_goes_to_the_first_parameter() {
        let d = Dictionary::parse(MANIFEST);
        let make = d.command("make new note").unwrap();
        let params = command_params(make, Some(Json::Str("Hi".into())), vec![("Body".into(), Json::Str("x".into()))]).unwrap();
        assert_eq!(params.str_field("title").as_deref(), Some("Hi"));
        assert_eq!(params.str_field("body").as_deref(), Some("x"));
    }

    #[test]
    fn undeclared_and_repeated_labels_are_errors() {
        let d = Dictionary::parse(MANIFEST);
        let make = d.command("make new note").unwrap();
        assert!(command_params(make, None, vec![("colour".into(), Json::Null)]).is_err());
        assert!(command_params(make, Some(Json::Null), vec![("title".into(), Json::Null)]).is_err());
        let bare = d.command("get name").unwrap();
        assert!(command_params(bare, Some(Json::Num(1.0)), vec![]).unwrap().get("direct").is_some());
    }

    #[test]
    fn dictionary_json_lists_events_as_class_dot_id() {
        let text = Dictionary::parse(MANIFEST).to_json().encode();
        assert!(text.contains(r#""event":"notes.make""#), "{text}");
        assert!(text.contains(r#""classes":[{"name":"note""#), "{text}");
    }
}
