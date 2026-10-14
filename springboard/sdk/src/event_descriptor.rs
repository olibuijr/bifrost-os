//! Event descriptors: the typed values an event carries and the object specifiers that name the object an event acts on.
//!
//! A [`Descriptor`] is a value (text, integer, real, boolean, list or null). [`Descriptor::coerce`] converts it to the
//! kind a handler asks for: text reads as a number or a boolean when it spells one, numbers round to the nearest
//! integer, and lists and null do not convert.
//!
//! An [`ObjectSpecifier`] names one element of the target's object tree by class and form: by index (1-based, a
//! negative index counts from the end), by name, by identifier, first or last. A specifier can sit inside a container
//! specifier, so "word 2 of paragraph 1 of document Notes" is three specifiers nested through
//! [`ObjectSpecifier::within`]. The target app holds its tree as [`Element`]s and resolves specifiers against it with
//! [`ObjectSpecifier::resolve`]. The model is pure: no socket, no clock and no device.

use bifrost_core::json::Json;

/// A typed value carried by an event parameter or a reply.
#[derive(Clone, Debug, PartialEq)]
pub enum Descriptor {
    Null,
    Boolean(bool),
    Integer(i64),
    Real(f64),
    Text(String),
    List(Vec<Descriptor>),
}

/// The kind a descriptor is converted to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Boolean,
    Integer,
    Real,
    Text,
}

/// Why a conversion is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoercionError {
    /// text that is not a finite number
    NotANumber,
    /// text that is neither a boolean word nor a boolean spelling
    NotABoolean,
    /// a real that is not finite or does not fit an integer
    OutOfRange,
    /// a list or null, which only converts to nothing
    NoConversion,
}

impl Descriptor {
    /// Converts the value to `to`. Text is trimmed before it is read as a number or a boolean. Booleans read as
    /// 1 and 0 and as "true" and "false". A real rounds to the nearest integer when it fits one.
    pub fn coerce(&self, to: Kind) -> Result<Descriptor, CoercionError> {
        use Descriptor::{Boolean, Integer, List, Null, Real, Text};
        match (self, to) {
            (Boolean(b), Kind::Boolean) => Ok(Boolean(*b)),
            (Boolean(b), Kind::Integer) => Ok(Integer(i64::from(*b))),
            (Boolean(b), Kind::Real) => Ok(Real(f64::from(u8::from(*b)))),
            (Boolean(b), Kind::Text) => Ok(Text(if *b { "true" } else { "false" }.into())),
            (Integer(n), Kind::Boolean) => Ok(Boolean(*n != 0)),
            (Integer(n), Kind::Integer) => Ok(Integer(*n)),
            (Integer(n), Kind::Real) => Ok(Real(*n as f64)),
            (Integer(n), Kind::Text) => Ok(Text(n.to_string())),
            (Real(x), Kind::Boolean) => Ok(Boolean(*x != 0.0)),
            (Real(x), Kind::Integer) => real_to_integer(*x),
            (Real(x), Kind::Real) => Ok(Real(*x)),
            (Real(x), Kind::Text) => Ok(Text(x.to_string())),
            (Text(s), Kind::Boolean) => match s.trim().to_ascii_lowercase().as_str() {
                "true" | "yes" => Ok(Boolean(true)),
                "false" | "no" => Ok(Boolean(false)),
                _ => Err(CoercionError::NotABoolean),
            },
            (Text(s), Kind::Integer) => s.trim().parse::<i64>().map(Integer).map_err(|_| CoercionError::NotANumber),
            (Text(s), Kind::Real) => match s.trim().parse::<f64>() {
                Ok(x) if x.is_finite() => Ok(Real(x)),
                _ => Err(CoercionError::NotANumber),
            },
            (Text(s), Kind::Text) => Ok(Text(s.clone())),
            (List(_), _) | (Null, _) => Err(CoercionError::NoConversion),
        }
    }

    /// Reads a JSON value as a descriptor. A whole number becomes an integer, any other number a real. An object has
    /// no descriptor and reads as `None`.
    pub fn from_json(value: &Json) -> Option<Descriptor> {
        match value {
            Json::Null => Some(Descriptor::Null),
            Json::Bool(b) => Some(Descriptor::Boolean(*b)),
            Json::Num(x) if x.fract() == 0.0 && x.abs() < 9.0e15 => Some(Descriptor::Integer(*x as i64)),
            Json::Num(x) => Some(Descriptor::Real(*x)),
            Json::Str(s) => Some(Descriptor::Text(s.clone())),
            Json::Arr(items) => items.iter().map(Descriptor::from_json).collect::<Option<Vec<_>>>().map(Descriptor::List),
            Json::Obj(_) => None,
        }
    }
}

fn real_to_integer(x: f64) -> Result<Descriptor, CoercionError> {
    let rounded = x.round();
    if rounded.is_finite() && rounded >= i64::MIN as f64 && rounded < i64::MAX as f64 {
        Ok(Descriptor::Integer(rounded as i64))
    } else {
        Err(CoercionError::OutOfRange)
    }
}

/// An element of the target's object tree: its class, identifier, optional name and the elements it contains.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Element {
    pub class: String,
    pub id: i64,
    pub name: Option<String>,
    pub children: Vec<Element>,
}

/// How a specifier picks one element among those of its class in its scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Form {
    /// 1-based position; a negative position counts from the end (-1 is the last element)
    Index(i64),
    /// the element whose name is this text
    Name(String),
    /// the element with this identifier
    Id(i64),
    /// the first element of the class
    First,
    /// the last element of the class
    Last,
}

/// Why a specifier does not name an element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveError {
    /// no element of the class matches the form
    NoSuchObject { class: String },
    /// an index that is zero or lies outside the elements of the class
    IndexOutOfRange { class: String, index: i64 },
}

/// A reference to one object: its class, the form that picks it and, optionally, the container it lives in.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectSpecifier {
    pub class: String,
    pub form: Form,
    pub container: Option<Box<ObjectSpecifier>>,
}

impl ObjectSpecifier {
    /// A specifier for the objects of `class` picked by `form`, at the top level of the tree.
    pub fn new(class: &str, form: Form) -> Self {
        ObjectSpecifier { class: class.into(), form, container: None }
    }

    /// Places the specifier inside `container`, so the elements are searched among the container's children.
    pub fn within(mut self, container: ObjectSpecifier) -> Self {
        self.container = Some(Box::new(container));
        self
    }

    /// Finds the element this specifier names in the tree `roots`. A container is resolved first; a missing container
    /// is reported as its own error.
    pub fn resolve<'a>(&self, roots: &'a [Element]) -> Result<&'a Element, ResolveError> {
        match &self.container {
            Some(container) => {
                let parent = container.resolve(roots)?;
                self.pick(&parent.children)
            }
            None => self.pick(roots),
        }
    }

    fn pick<'a>(&self, scope: &'a [Element]) -> Result<&'a Element, ResolveError> {
        let matches: Vec<&'a Element> = scope.iter().filter(|e| e.class == self.class).collect();
        match &self.form {
            Form::Index(index) => {
                let index = *index;
                let len = matches.len() as i64;
                let position = if index > 0 { index - 1 } else { len + index };
                if index == 0 || !(0..len).contains(&position) {
                    return Err(ResolveError::IndexOutOfRange { class: self.class.clone(), index });
                }
                Ok(matches[position as usize])
            }
            Form::First => matches.first().copied().ok_or_else(|| self.missing()),
            Form::Last => matches.last().copied().ok_or_else(|| self.missing()),
            Form::Name(name) => matches.into_iter().find(|e| e.name.as_deref() == Some(name.as_str())).ok_or_else(|| self.missing()),
            Form::Id(id) => matches.into_iter().find(|e| e.id == *id).ok_or_else(|| self.missing()),
        }
    }

    fn missing(&self) -> ResolveError {
        ResolveError::NoSuchObject { class: self.class.clone() }
    }
}
