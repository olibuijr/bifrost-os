//! Native object export to scripts: a Rust type publishes named methods and properties to a script engine, and the
//! engine calls them by name.
//!
//! This is the pure model of the export side of an embedding. A class is a named set of members, and every member is
//! either a method (called with arguments) or a property (read through a getter, written through an optional setter).
//! Calls follow the script calling convention: a method receives exactly its declared arity, missing arguments arrive
//! as `undefined` and extra arguments are dropped. Values cross as [`ScriptValue`], the same typed tree that
//! `script_value` builds for web view answers.
//!
//! Scope: names are checked as ASCII identifiers (first character a letter, `_` or `$`; the rest also digits). The
//! registry does not reject reserved words and it does not link classes into a prototype chain. No script engine is
//! attached here; the caller decides how a registry is reached from a script.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::script_value::ScriptValue;

type Getter = Box<dyn Fn() -> ScriptValue>;
type Setter = Box<dyn Fn(ScriptValue) -> Result<(), String>>;
type Handler = Box<dyn Fn(&[ScriptValue]) -> Result<ScriptValue, String>>;

/// Why an export or a call failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportError {
    /// The name is empty or is not an ASCII identifier.
    InvalidName(String),
    /// A class with this name is already exported, or the class already has a member of this name.
    Duplicate(String),
    /// No class with this name is exported.
    UnknownClass(String),
    /// The class has no method or property with this name.
    UnknownMember { class: String, member: String },
    /// The property has no setter.
    NotWritable { class: String, member: String },
    /// The handler or setter returned an error; the text is the handler's message.
    Failed(String),
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExportError::InvalidName(name) => write!(f, "invalid name: {name:?}"),
            ExportError::Duplicate(name) => write!(f, "already exported: {name}"),
            ExportError::UnknownClass(class) => write!(f, "no exported class {class}"),
            ExportError::UnknownMember { class, member } => write!(f, "{class} has no member {member}"),
            ExportError::NotWritable { class, member } => write!(f, "{class}.{member} is read-only"),
            ExportError::Failed(message) => write!(f, "{message}"),
        }
    }
}

/// True when `name` is an ASCII identifier: a first character that is a letter, `_` or `$`, then letters, digits,
/// `_` or `$`.
pub fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let first_ok = match chars.next() {
        Some(ch) => ch.is_ascii_alphabetic() || ch == '_' || ch == '$',
        None => false,
    };
    first_ok && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
}

struct Method {
    arity: usize,
    handler: Handler,
}

struct Property {
    getter: Getter,
    setter: Option<Setter>,
}

#[derive(Default)]
struct Class {
    methods: BTreeMap<String, Method>,
    properties: BTreeMap<String, Property>,
}

impl Class {
    fn has_member(&self, name: &str) -> bool {
        self.methods.contains_key(name) || self.properties.contains_key(name)
    }
}

/// The set of classes an embedding exports to scripts.
#[derive(Default)]
pub struct ExportRegistry {
    classes: BTreeMap<String, Class>,
}

impl ExportRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an empty class. Fails when the name is not an identifier or the class already exists.
    pub fn export_class(&mut self, name: &str) -> Result<(), ExportError> {
        check_name(name)?;
        if self.classes.contains_key(name) {
            return Err(ExportError::Duplicate(name.to_string()));
        }
        self.classes.insert(name.to_string(), Class::default());
        Ok(())
    }

    /// Adds a method that takes `arity` arguments. Fails when the class is missing, the name is invalid, or the class
    /// already has a member of that name.
    pub fn export_method<F>(&mut self, class: &str, name: &str, arity: usize, handler: F) -> Result<(), ExportError>
    where
        F: Fn(&[ScriptValue]) -> Result<ScriptValue, String> + 'static,
    {
        let target = self.class_mut(class, name)?;
        target.methods.insert(name.to_string(), Method { arity, handler: Box::new(handler) });
        Ok(())
    }

    /// Adds a property that can be read but not written.
    pub fn export_read_only<G>(&mut self, class: &str, name: &str, getter: G) -> Result<(), ExportError>
    where
        G: Fn() -> ScriptValue + 'static,
    {
        let target = self.class_mut(class, name)?;
        target.properties.insert(name.to_string(), Property { getter: Box::new(getter), setter: None });
        Ok(())
    }

    /// Adds a property that can be read and written. The setter may refuse a value with an error message.
    pub fn export_read_write<G, S>(&mut self, class: &str, name: &str, getter: G, setter: S) -> Result<(), ExportError>
    where
        G: Fn() -> ScriptValue + 'static,
        S: Fn(ScriptValue) -> Result<(), String> + 'static,
    {
        let target = self.class_mut(class, name)?;
        target
            .properties
            .insert(name.to_string(), Property { getter: Box::new(getter), setter: Some(Box::new(setter)) });
        Ok(())
    }

    /// Calls an exported method. The arguments are cut to the declared arity and padded with `undefined`, as a script
    /// call would give them. A handler error comes back as [`ExportError::Failed`].
    pub fn call(&self, class: &str, method: &str, args: &[ScriptValue]) -> Result<ScriptValue, ExportError> {
        let cls = self.class(class)?;
        let m = cls.methods.get(method).ok_or_else(|| unknown_member(class, method))?;
        let mut padded: Vec<ScriptValue> = args.iter().take(m.arity).cloned().collect();
        while padded.len() < m.arity {
            padded.push(ScriptValue::Undefined);
        }
        (m.handler)(&padded[..]).map_err(ExportError::Failed)
    }

    /// Reads an exported property.
    pub fn get(&self, class: &str, property: &str) -> Result<ScriptValue, ExportError> {
        let cls = self.class(class)?;
        let p = cls.properties.get(property).ok_or_else(|| unknown_member(class, property))?;
        Ok((p.getter)())
    }

    /// Writes an exported property. A read-only property fails with [`ExportError::NotWritable`].
    pub fn set(&self, class: &str, property: &str, value: ScriptValue) -> Result<(), ExportError> {
        let cls = self.class(class)?;
        let p = cls.properties.get(property).ok_or_else(|| unknown_member(class, property))?;
        match &p.setter {
            Some(setter) => setter(value).map_err(ExportError::Failed),
            None => Err(ExportError::NotWritable { class: class.to_string(), member: property.to_string() }),
        }
    }

    /// Every method and property name of a class, in sorted order. `None` when the class is not exported.
    pub fn member_names(&self, class: &str) -> Option<Vec<String>> {
        let cls = self.classes.get(class)?;
        let names: BTreeSet<&String> = cls.methods.keys().chain(cls.properties.keys()).collect();
        Some(names.into_iter().cloned().collect())
    }

    fn class(&self, class: &str) -> Result<&Class, ExportError> {
        self.classes.get(class).ok_or_else(|| ExportError::UnknownClass(class.to_string()))
    }

    /// The class for a new member, after checking the member name and that the name is free in the class.
    fn class_mut(&mut self, class: &str, member: &str) -> Result<&mut Class, ExportError> {
        check_name(member)?;
        let target = self.classes.get_mut(class).ok_or_else(|| ExportError::UnknownClass(class.to_string()))?;
        if target.has_member(member) {
            return Err(ExportError::Duplicate(member.to_string()));
        }
        Ok(target)
    }
}

fn check_name(name: &str) -> Result<(), ExportError> {
    if is_identifier(name) {
        Ok(())
    } else {
        Err(ExportError::InvalidName(name.to_string()))
    }
}

fn unknown_member(class: &str, member: &str) -> ExportError {
    ExportError::UnknownMember { class: class.to_string(), member: member.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn echo_args(args: &[ScriptValue]) -> Result<ScriptValue, String> {
        Ok(ScriptValue::Array(args.to_vec()))
    }

    #[test]
    fn identifiers_follow_the_ascii_rule() {
        assert!(is_identifier("$ok_1"));
        assert!(is_identifier("_private"));
        assert!(!is_identifier("2fast"));
        assert!(!is_identifier("a-b"));
        assert!(!is_identifier(""));
        assert!(!is_identifier("café"), "non-ASCII letters are rejected");
    }

    #[test]
    fn a_method_gets_exactly_its_arity_with_undefined_padding() {
        let mut reg = ExportRegistry::new();
        reg.export_class("Player").unwrap();
        reg.export_method("Player", "pair", 2, echo_args).unwrap();

        let one = reg.call("Player", "pair", &[ScriptValue::Number(1.0)]).unwrap();
        assert_eq!(one, ScriptValue::Array(vec![ScriptValue::Number(1.0), ScriptValue::Undefined]));

        let three = [ScriptValue::Number(1.0), ScriptValue::String("b".into()), ScriptValue::Bool(true)];
        let cut = reg.call("Player", "pair", &three).unwrap();
        assert_eq!(cut, ScriptValue::Array(vec![ScriptValue::Number(1.0), ScriptValue::String("b".into())]));
    }

    #[test]
    fn a_read_write_property_passes_values_through_its_setter() {
        let volume = Rc::new(RefCell::new(0.5));
        let (read, write) = (volume.clone(), volume.clone());
        let mut reg = ExportRegistry::new();
        reg.export_class("Player").unwrap();
        reg.export_read_write(
            "Player",
            "volume",
            move || ScriptValue::Number(*read.borrow()),
            move |value| match value {
                ScriptValue::Number(n) if (0.0..=1.0).contains(&n) => {
                    *write.borrow_mut() = n;
                    Ok(())
                }
                _ => Err("volume must be a number from 0 to 1".to_string()),
            },
        )
        .unwrap();

        assert_eq!(reg.get("Player", "volume"), Ok(ScriptValue::Number(0.5)));
        assert_eq!(
            reg.set("Player", "volume", ScriptValue::String("loud".into())),
            Err(ExportError::Failed("volume must be a number from 0 to 1".into()))
        );
        assert_eq!(*volume.borrow(), 0.5, "a refused value leaves the property alone");
        reg.set("Player", "volume", ScriptValue::Number(0.25)).unwrap();
        assert_eq!(reg.get("Player", "volume"), Ok(ScriptValue::Number(0.25)));
    }

    #[test]
    fn a_read_only_property_refuses_writes() {
        let mut reg = ExportRegistry::new();
        reg.export_class("Player").unwrap();
        reg.export_read_only("Player", "count", || ScriptValue::Number(3.0)).unwrap();
        assert_eq!(reg.get("Player", "count"), Ok(ScriptValue::Number(3.0)));
        assert_eq!(
            reg.set("Player", "count", ScriptValue::Number(9.0)),
            Err(ExportError::NotWritable { class: "Player".into(), member: "count".into() })
        );
    }

    #[test]
    fn names_and_duplicates_are_refused_before_anything_is_stored() {
        let mut reg = ExportRegistry::new();
        assert_eq!(reg.export_class("2Player"), Err(ExportError::InvalidName("2Player".into())));
        reg.export_class("Player").unwrap();
        assert_eq!(reg.export_class("Player"), Err(ExportError::Duplicate("Player".into())));
        reg.export_method("Player", "play", 0, |_| Ok(ScriptValue::Null)).unwrap();
        assert_eq!(
            reg.export_read_only("Player", "play", || ScriptValue::Null),
            Err(ExportError::Duplicate("play".into())),
            "methods and properties share one namespace on the class"
        );
        assert_eq!(
            reg.export_method("Radio", "play", 0, |_| Ok(ScriptValue::Null)),
            Err(ExportError::UnknownClass("Radio".into()))
        );
    }

    #[test]
    fn missing_classes_and_members_are_reported_by_name() {
        let mut reg = ExportRegistry::new();
        reg.export_class("Player").unwrap();
        assert_eq!(reg.call("Radio", "play", &[]), Err(ExportError::UnknownClass("Radio".into())));
        assert_eq!(
            reg.call("Player", "stop", &[]),
            Err(ExportError::UnknownMember { class: "Player".into(), member: "stop".into() })
        );
        assert_eq!(reg.get("Player", "volume"), Err(ExportError::UnknownMember { class: "Player".into(), member: "volume".into() }));
    }

    #[test]
    fn a_handler_error_comes_back_as_failed_and_member_names_are_sorted() {
        let mut reg = ExportRegistry::new();
        reg.export_class("Player").unwrap();
        reg.export_method("Player", "zeta", 0, |_| Err("boom".to_string())).unwrap();
        reg.export_method("Player", "alpha", 0, |_| Ok(ScriptValue::Null)).unwrap();
        reg.export_read_only("Player", "mid", || ScriptValue::Null).unwrap();

        assert_eq!(reg.call("Player", "zeta", &[]), Err(ExportError::Failed("boom".into())));
        assert_eq!(
            reg.member_names("Player"),
            Some(vec!["alpha".to_string(), "mid".to_string(), "zeta".to_string()])
        );
        assert_eq!(reg.member_names("Radio"), None);
    }
}
