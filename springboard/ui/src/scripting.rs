//! Scripting object model: a scriptable application publishes a `Dictionary` of its classes, their properties and
//! element arrays, and a `Target` reads and writes properties and finds elements by name, id or position. Like a
//! scripting proxy, the target keeps the last error and tells a delegate about each failure.
//!
//! Paths address objects: each `Step` names an element array and picks one object in it. An empty path is the root.

use std::fmt;

/// A property or element value. `Null` is "no value" and any property accepts it.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Text(String),
}

/// The type a property accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Int,
    Text,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Bool => "bool",
            Kind::Int => "int",
            Kind::Text => "text",
        }
    }
}

impl Value {
    /// The kind of a value, `None` for `Null`.
    pub fn kind(&self) -> Option<Kind> {
        match self {
            Value::Null => None,
            Value::Bool(_) => Some(Kind::Bool),
            Value::Int(_) => Some(Kind::Int),
            Value::Text(_) => Some(Kind::Text),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Property {
    pub code: String,
    pub kind: Kind,
    pub writable: bool,
}

/// A named collection of child objects of one class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElementArray {
    pub code: String,
    /// The class of the objects in the array.
    pub class: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Class {
    pub name: String,
    pub properties: Vec<Property>,
    pub elements: Vec<ElementArray>,
}

/// The classes an application exposes to scripting, under one suite name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dictionary {
    pub suite: String,
    pub classes: Vec<Class>,
}

impl Dictionary {
    pub fn new(suite: &str) -> Self {
        Dictionary { suite: suite.into(), classes: Vec::new() }
    }

    pub fn class(&self, name: &str) -> Option<&Class> {
        self.classes.iter().find(|c| c.name == name)
    }

    /// The dictionary as XML text: one `class` per class, with `property` (type and access) and `element` lines.
    /// Clients read it to learn what they can address before they send a request.
    pub fn describe_xml(&self) -> String {
        let mut out = format!("<suite name=\"{}\">\n", escape(&self.suite));
        for c in &self.classes {
            out += &format!("  <class name=\"{}\">\n", escape(&c.name));
            for p in &c.properties {
                let access = if p.writable { "rw" } else { "r" };
                out += &format!(
                    "    <property code=\"{}\" type=\"{}\" access=\"{}\"/>\n",
                    escape(&p.code),
                    p.kind.name(),
                    access
                );
            }
            for e in &c.elements {
                out += &format!("    <element code=\"{}\" class=\"{}\"/>\n", escape(&e.code), escape(&e.class));
            }
            out += "  </class>\n";
        }
        out + "</suite>\n"
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A scriptable object: its class, identifier and name, its property values and its element arrays.
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub class: String,
    pub id: String,
    pub name: String,
    properties: Vec<(String, Value)>,
    elements: Vec<(String, Vec<Object>)>,
}

impl Object {
    pub fn new(class: &str, id: &str, name: &str) -> Self {
        Object { class: class.into(), id: id.into(), name: name.into(), properties: Vec::new(), elements: Vec::new() }
    }

    /// Sets a property as the host, without checks: the host owns the state. Scripts go through `Target::set`.
    pub fn put(&mut self, code: &str, value: Value) {
        match self.properties.iter().position(|(c, _)| c == code) {
            Some(i) => self.properties[i].1 = value,
            None => self.properties.push((code.into(), value)),
        }
    }

    /// Appends a child to the element array `code`, creating the array on first use.
    pub fn add(&mut self, code: &str, child: Object) {
        match self.elements.iter().position(|(c, _)| c == code) {
            Some(i) => self.elements[i].1.push(child),
            None => self.elements.push((code.into(), vec![child])),
        }
    }

    /// The stored value of a property, `None` if the host never set it.
    pub fn property(&self, code: &str) -> Option<&Value> {
        self.properties.iter().find(|(c, _)| c == code).map(|(_, v)| v)
    }

    /// Every child in the element array `code`, empty when there is none.
    pub fn children(&self, code: &str) -> &[Object] {
        self.elements.iter().find(|(c, _)| c == code).map(|(_, v)| v.as_slice()).unwrap_or(&[])
    }
}

/// How one step picks an object out of an element array. `Index` is a 0-based position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locator<'a> {
    Name(&'a str),
    Id(&'a str),
    Index(usize),
}

/// One step of a path: an element array and the object to take from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step<'a> {
    pub element: &'a str,
    pub by: Locator<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// The object's class is not in the dictionary.
    UnknownClass,
    /// The class has no property with this code.
    UnknownProperty,
    /// The class has no element array with this code.
    UnknownElement,
    /// No object in the element array matches the locator.
    NoMatch,
    /// The property is declared read-only.
    ReadOnly,
    /// The value's kind is not the property's kind.
    TypeMismatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptError {
    pub kind: ErrorKind,
    pub message: String,
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ScriptError {}

fn fail(kind: ErrorKind, message: String) -> ScriptError {
    ScriptError { kind, message }
}

fn position(list: &[Object], by: Locator) -> Option<usize> {
    match by {
        Locator::Name(name) => list.iter().position(|o| o.name == name),
        Locator::Id(id) => list.iter().position(|o| o.id == id),
        Locator::Index(i) => (i < list.len()).then_some(i),
    }
}

fn describe(by: Locator) -> String {
    match by {
        Locator::Name(name) => format!("named {name:?}"),
        Locator::Id(id) => format!("with id {id:?}"),
        Locator::Index(i) => format!("at index {i}"),
    }
}

fn find_mut<'a>(obj: &'a mut Object, path: &[Step]) -> Option<&'a mut Object> {
    let Some((step, rest)) = path.split_first() else { return Some(obj) };
    let array = obj.elements.iter_mut().find(|e| e.0 == step.element)?;
    let i = position(&array.1, step.by)?;
    find_mut(&mut array.1[i], rest)
}

/// Addresses one application's objects and records the errors of the calls made on it.
pub struct Target {
    dictionary: Dictionary,
    root: Object,
    last: Option<ScriptError>,
    on_error: Option<Box<dyn FnMut(&ScriptError)>>,
}

impl Target {
    pub fn new(dictionary: Dictionary, root: Object) -> Self {
        Target { dictionary, root, last: None, on_error: None }
    }

    pub fn dictionary(&self) -> &Dictionary {
        &self.dictionary
    }

    /// The error of the last call, `None` after a call that succeeded.
    pub fn last_error(&self) -> Option<&ScriptError> {
        self.last.as_ref()
    }

    /// Registers the delegate that hears every failed call, once per failure.
    pub fn on_error(&mut self, handler: impl FnMut(&ScriptError) + 'static) {
        self.on_error = Some(Box::new(handler));
    }

    /// The object at `path` without recording anything (for read-only peeks such as a view of the tree).
    pub fn find(&self, path: &[Step]) -> Option<&Object> {
        self.locate(path).ok()
    }

    /// The value of property `code` on the object at `path`. A declared property that was never set reads as `Null`.
    pub fn get(&mut self, path: &[Step], code: &str) -> Result<Value, ScriptError> {
        let r = self.get_inner(path, code);
        self.settle(r)
    }

    /// Sets property `code` on the object at `path`. Checks the access and the kind the dictionary declares.
    pub fn set(&mut self, path: &[Step], code: &str, value: Value) -> Result<(), ScriptError> {
        let r = self.set_inner(path, code, value);
        self.settle(r)
    }

    /// Every child of the element array `code` on the object at `path`, in order.
    pub fn children(&mut self, path: &[Step], code: &str) -> Result<Vec<Object>, ScriptError> {
        let r = self.children_inner(path, code);
        self.settle(r)
    }

    fn settle<T>(&mut self, r: Result<T, ScriptError>) -> Result<T, ScriptError> {
        match &r {
            Ok(_) => self.last = None,
            Err(e) => {
                self.last = Some(e.clone());
                if let Some(handler) = self.on_error.as_mut() {
                    handler(e);
                }
            }
        }
        r
    }

    fn class_of(&self, obj: &Object) -> Result<&Class, ScriptError> {
        self.dictionary
            .class(&obj.class)
            .ok_or_else(|| fail(ErrorKind::UnknownClass, format!("class {:?} is not in the dictionary", obj.class)))
    }

    fn locate(&self, path: &[Step]) -> Result<&Object, ScriptError> {
        let mut obj = &self.root;
        for step in path {
            let class = self.class_of(obj)?;
            if !class.elements.iter().any(|e| e.code == step.element) {
                return Err(fail(ErrorKind::UnknownElement, format!("{} has no element array {:?}", class.name, step.element)));
            }
            let list = obj.children(step.element);
            let i = position(list, step.by).ok_or_else(|| {
                fail(ErrorKind::NoMatch, format!("no {:?} object {}", step.element, describe(step.by)))
            })?;
            obj = &list[i];
        }
        Ok(obj)
    }

    fn get_inner(&self, path: &[Step], code: &str) -> Result<Value, ScriptError> {
        let obj = self.locate(path)?;
        let class = self.class_of(obj)?;
        if !class.properties.iter().any(|p| p.code == code) {
            return Err(fail(ErrorKind::UnknownProperty, format!("{} has no property {code:?}", class.name)));
        }
        Ok(obj.property(code).cloned().unwrap_or(Value::Null))
    }

    fn set_inner(&mut self, path: &[Step], code: &str, value: Value) -> Result<(), ScriptError> {
        let (class_name, def) = {
            let obj = self.locate(path)?;
            let class = self.class_of(obj)?;
            let def = class
                .properties
                .iter()
                .find(|p| p.code == code)
                .cloned()
                .ok_or_else(|| fail(ErrorKind::UnknownProperty, format!("{} has no property {code:?}", class.name)))?;
            (class.name.clone(), def)
        };
        if !def.writable {
            return Err(fail(ErrorKind::ReadOnly, format!("{class_name} property {code:?} is read-only")));
        }
        if value.kind().is_some_and(|k| k != def.kind) {
            return Err(fail(
                ErrorKind::TypeMismatch,
                format!("{class_name} property {code:?} takes {}, not {value:?}", def.kind.name()),
            ));
        }
        let obj = find_mut(&mut self.root, path)
            .ok_or_else(|| fail(ErrorKind::NoMatch, "the object moved while it was set".into()))?;
        obj.put(code, value);
        Ok(())
    }

    fn children_inner(&self, path: &[Step], code: &str) -> Result<Vec<Object>, ScriptError> {
        let obj = self.locate(path)?;
        let class = self.class_of(obj)?;
        if !class.elements.iter().any(|e| e.code == code) {
            return Err(fail(ErrorKind::UnknownElement, format!("{} has no element array {code:?}", class.name)));
        }
        Ok(obj.children(code).to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    fn prop(code: &str, kind: Kind, writable: bool) -> Property {
        Property { code: code.into(), kind, writable }
    }

    fn sample() -> Target {
        let mut dict = Dictionary::new("Notes \"suite\" & more");
        dict.classes.push(Class {
            name: "application".into(),
            properties: vec![prop("name", Kind::Text, false)],
            elements: vec![ElementArray { code: "note".into(), class: "note".into() }],
        });
        dict.classes.push(Class {
            name: "note".into(),
            properties: vec![prop("title", Kind::Text, true), prop("pinned", Kind::Bool, true), prop("words", Kind::Int, false)],
            elements: vec![ElementArray { code: "tag".into(), class: "tag".into() }],
        });
        dict.classes.push(Class {
            name: "tag".into(),
            properties: vec![prop("label", Kind::Text, true)],
            elements: vec![],
        });
        let mut root = Object::new("application", "app", "Notes");
        root.put("name", Value::Text("Notes".into()));
        let mut groceries = Object::new("note", "n1", "Groceries");
        groceries.put("title", Value::Text("Groceries".into()));
        groceries.put("words", Value::Int(12));
        let mut tag = Object::new("tag", "t1", "shopping");
        tag.put("label", Value::Text("shopping".into()));
        groceries.add("tag", tag);
        root.add("note", groceries);
        root.add("note", Object::new("note", "n2", "Ideas"));
        Target::new(dict, root)
    }

    fn note(by: Locator) -> [Step; 1] {
        [Step { element: "note", by }]
    }

    #[test]
    fn dictionary_xml_lists_classes_with_access_and_escapes_names() {
        let t = sample();
        let xml = t.dictionary().describe_xml();
        assert!(xml.starts_with("<suite name=\"Notes &quot;suite&quot; &amp; more\">"), "{xml}");
        assert!(xml.contains("<property code=\"title\" type=\"text\" access=\"rw\"/>"));
        assert!(xml.contains("<property code=\"words\" type=\"int\" access=\"r\"/>"));
        assert!(xml.contains("<element code=\"tag\" class=\"tag\"/>"));
        assert!(t.dictionary().class("note").is_some() && t.dictionary().class("folder").is_none());
    }

    #[test]
    fn reads_declared_properties_and_refuses_undeclared_ones() {
        let mut t = sample();
        assert_eq!(t.get(&note(Locator::Name("Groceries")), "title"), Ok(Value::Text("Groceries".into())));
        assert_eq!(t.get(&note(Locator::Id("n2")), "pinned"), Ok(Value::Null), "declared but never set");
        let e = t.get(&note(Locator::Id("n2")), "secret").unwrap_err();
        assert_eq!(e.kind, ErrorKind::UnknownProperty);
        assert_eq!(t.last_error(), Some(&e), "the failure is kept as the last error");
        assert!(t.get(&[], "name").is_ok());
        assert!(t.last_error().is_none(), "a success clears the last error");
    }

    #[test]
    fn writes_respect_access_and_kind() {
        let mut t = sample();
        let n2 = note(Locator::Id("n2"));
        t.set(&n2, "title", Value::Text("Ideas for May".into())).unwrap();
        assert_eq!(t.get(&n2, "title"), Ok(Value::Text("Ideas for May".into())));
        assert_eq!(t.set(&n2, "words", Value::Int(3)).unwrap_err().kind, ErrorKind::ReadOnly);
        assert_eq!(t.set(&n2, "pinned", Value::Text("yes".into())).unwrap_err().kind, ErrorKind::TypeMismatch);
        t.set(&n2, "pinned", Value::Bool(true)).unwrap();
        t.set(&n2, "pinned", Value::Null).unwrap();
        assert_eq!(t.get(&n2, "pinned"), Ok(Value::Null), "Null clears a value of any kind");
        assert_eq!(t.get(&n2, "title"), Ok(Value::Text("Ideas for May".into())), "failed writes change nothing");
    }

    #[test]
    fn elements_are_found_by_name_id_position_and_nested_paths() {
        let mut t = sample();
        assert_eq!(t.get(&note(Locator::Id("n2")), "title"), Ok(Value::Text("Ideas".into())));
        assert_eq!(t.get(&note(Locator::Index(0)), "title"), Ok(Value::Text("Groceries".into())));
        assert_eq!(t.get(&note(Locator::Index(5)), "title").unwrap_err().kind, ErrorKind::NoMatch);
        assert_eq!(t.get(&note(Locator::Name("Nope")), "title").unwrap_err().kind, ErrorKind::NoMatch);
        let nested = [Step { element: "note", by: Locator::Name("Groceries") }, Step { element: "tag", by: Locator::Id("t1") }];
        assert_eq!(t.get(&nested, "label"), Ok(Value::Text("shopping".into())));
        assert!(t.find(&nested).is_some_and(|o| o.id == "t1"));
    }

    #[test]
    fn element_arrays_must_be_declared_and_list_their_children_in_order() {
        let mut t = sample();
        let notes = t.children(&[], "note").unwrap();
        assert_eq!(notes.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["n1", "n2"]);
        assert_eq!(t.children(&[], "folder").unwrap_err().kind, ErrorKind::UnknownElement);
        let mut bad_class = Target::new(Dictionary::new("empty"), Object::new("note", "x", "x"));
        assert_eq!(bad_class.get(&[], "title").unwrap_err().kind, ErrorKind::UnknownClass);
    }

    #[test]
    fn the_delegate_hears_each_failure_once_and_not_successes() {
        let mut t = sample();
        let heard = Rc::new(Cell::new(0));
        let seen = heard.clone();
        t.on_error(move |_| seen.set(seen.get() + 1));
        let _ = t.get(&note(Locator::Id("n2")), "secret");
        let _ = t.get(&note(Locator::Index(9)), "title");
        t.get(&[], "name").unwrap();
        assert_eq!(heard.get(), 2);
    }
}
