//! Extension properties: the named values a camera provider, device or stream exposes (a brightness
//! control, a mirror switch, a label), the attributes that limit them (read-only, a minimum and maximum,
//! or a list of valid values) and the rules for setting one. This is pure. A camera source reads and
//! writes its values through `PropertyStore`; the store checks every write against the attributes that
//! were declared when the property was defined.

use std::collections::BTreeMap;

/// A property value. A property keeps one kind for its whole life, so a write of another kind is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Text(String),
}

impl Value {
    /// The kind of the value, for matching a write against the property.
    fn kind(&self) -> u8 {
        match self {
            Value::Bool(_) => 0,
            Value::Int(_) => 1,
            Value::Text(_) => 2,
        }
    }
}

/// What limits a property's values. Integer properties are limited by `valid` when it is not empty, and
/// otherwise by the inclusive `min` and `max` bounds where they are set. Bool and text values have no
/// limits beyond their kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attributes {
    pub read_only: bool,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub valid: Vec<i64>,
}

impl Attributes {
    /// Attributes that allow any value of the property's kind.
    pub fn free() -> Attributes {
        Attributes { read_only: false, min: None, max: None, valid: Vec::new() }
    }

    /// Attributes for a property the app can read but not write.
    pub fn read_only() -> Attributes {
        Attributes { read_only: true, ..Attributes::free() }
    }

    /// Attributes that keep an integer between two bounds, inclusive.
    pub fn range(min: i64, max: i64) -> Attributes {
        Attributes { min: Some(min), max: Some(max), ..Attributes::free() }
    }

    /// Attributes that allow only the listed integers.
    pub fn one_of(valid: Vec<i64>) -> Attributes {
        Attributes { valid, ..Attributes::free() }
    }

    /// Checks a value against these limits, without looking at `read_only`.
    pub fn check(&self, value: &Value) -> Result<(), PropertyError> {
        let Value::Int(n) = value else { return Ok(()) };
        if !self.valid.is_empty() {
            return if self.valid.contains(n) { Ok(()) } else { Err(PropertyError::OutOfRange) };
        }
        if self.min.is_some_and(|min| *n < min) || self.max.is_some_and(|max| *n > max) {
            return Err(PropertyError::OutOfRange);
        }
        Ok(())
    }
}

/// A property's current value with the attributes that limit it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropertyState {
    pub value: Value,
    pub attributes: Attributes,
}

/// Why a property could not be defined or written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropertyError {
    /// No property has this key.
    Unknown,
    /// The property is read-only.
    ReadOnly,
    /// The value is of a different kind from the property's value.
    WrongType,
    /// The value is outside the property's limits.
    OutOfRange,
}

impl std::fmt::Display for PropertyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            PropertyError::Unknown => "unknown property",
            PropertyError::ReadOnly => "property is read-only",
            PropertyError::WrongType => "value has the wrong type",
            PropertyError::OutOfRange => "value is out of range",
        };
        write!(f, "{text}")
    }
}

/// The properties of one provider, device or stream, by key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PropertyStore {
    properties: BTreeMap<String, PropertyState>,
}

impl PropertyStore {
    pub fn new() -> PropertyStore {
        PropertyStore::default()
    }

    /// Defines a property with its first value. The value must pass the attributes' limits.
    pub fn define(&mut self, key: &str, value: Value, attributes: Attributes) -> Result<(), PropertyError> {
        attributes.check(&value)?;
        self.properties.insert(key.to_string(), PropertyState { value, attributes });
        Ok(())
    }

    /// The current state of a property, if it is defined.
    pub fn get(&self, key: &str) -> Option<&PropertyState> {
        self.properties.get(key)
    }

    /// Writes a new value. The key must exist, the property must be writable, the kind must match and the
    /// value must pass the limits. A refused write leaves the stored value as it was.
    pub fn set(&mut self, key: &str, value: Value) -> Result<(), PropertyError> {
        let state = self.properties.get_mut(key).ok_or(PropertyError::Unknown)?;
        if state.attributes.read_only {
            return Err(PropertyError::ReadOnly);
        }
        if state.value.kind() != value.kind() {
            return Err(PropertyError::WrongType);
        }
        state.attributes.check(&value)?;
        state.value = value;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> PropertyStore {
        let mut s = PropertyStore::new();
        s.define("brightness", Value::Int(50), Attributes::range(0, 100)).unwrap();
        s.define("rotation", Value::Int(0), Attributes::one_of(vec![0, 90, 180, 270])).unwrap();
        s.define("mirror", Value::Bool(false), Attributes::free()).unwrap();
        s.define("model", Value::Text("cam".to_string()), Attributes::read_only()).unwrap();
        s
    }

    #[test]
    fn a_write_inside_the_range_is_stored() {
        let mut s = store();
        assert_eq!(s.set("brightness", Value::Int(100)), Ok(()));
        assert_eq!(s.get("brightness").unwrap().value, Value::Int(100));
    }

    #[test]
    fn a_write_outside_the_range_is_refused_and_keeps_the_old_value() {
        let mut s = store();
        assert_eq!(s.set("brightness", Value::Int(101)), Err(PropertyError::OutOfRange));
        assert_eq!(s.set("brightness", Value::Int(-1)), Err(PropertyError::OutOfRange));
        assert_eq!(s.get("brightness").unwrap().value, Value::Int(50));
    }

    #[test]
    fn valid_values_replace_the_bounds() {
        let mut s = store();
        assert_eq!(s.set("rotation", Value::Int(270)), Ok(()));
        // 45 is inside 0 to 270 but is not one of the listed values.
        assert_eq!(s.set("rotation", Value::Int(45)), Err(PropertyError::OutOfRange));
        assert_eq!(s.get("rotation").unwrap().value, Value::Int(270));
    }

    #[test]
    fn read_only_properties_refuse_every_write() {
        let mut s = store();
        assert_eq!(s.set("model", Value::Text("other".to_string())), Err(PropertyError::ReadOnly));
        assert_eq!(s.get("model").unwrap().value, Value::Text("cam".to_string()));
    }

    #[test]
    fn a_write_of_another_kind_is_refused() {
        let mut s = store();
        assert_eq!(s.set("mirror", Value::Int(1)), Err(PropertyError::WrongType));
        assert_eq!(s.set("mirror", Value::Bool(true)), Ok(()));
        assert_eq!(s.get("mirror").unwrap().value, Value::Bool(true));
    }

    #[test]
    fn unknown_keys_and_bad_first_values_are_refused() {
        let mut s = store();
        assert_eq!(s.set("zoom", Value::Int(2)), Err(PropertyError::Unknown));
        assert!(s.get("zoom").is_none());
        assert_eq!(s.define("zoom", Value::Int(9), Attributes::range(1, 4)), Err(PropertyError::OutOfRange));
        assert!(s.get("zoom").is_none());
    }

    #[test]
    fn error_text_is_plain() {
        assert_eq!(PropertyError::OutOfRange.to_string(), "value is out of range");
        assert_eq!(PropertyError::Unknown.to_string(), "unknown property");
    }
}
