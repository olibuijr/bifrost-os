//! Video encoder session properties and the catalogue of encoders. A session holds named properties. Each property has a
//! spec: its kind (a flag, an integer in a range, or one of a list of names), its default and whether a caller may
//! write it. Reading a property that has no value returns its default. Writing checks that the name is known, that the
//! property is writable, that the value has the property's kind and that it lies in the range or the list. The host
//! publishes read-only properties, such as an encoder's identifier, with `publish`. The catalogue lists the encoders a
//! codec backend has registered, with the properties each one supports. It encodes nothing itself.

use std::collections::BTreeMap;
use std::fmt;

/// The kind of a property value, with its range or its list of names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// True or false.
    Bool,
    /// An integer from `min` to `max`, both included.
    Int { min: i64, max: i64 },
    /// One of the listed names.
    Choice(&'static [&'static str]),
}

/// A property value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Text(String),
}

/// The description of one property.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub name: &'static str,
    pub kind: Kind,
    /// The value read while no value has been set.
    pub default: Value,
    /// Whether a caller may set it. Read-only properties are published by the host.
    pub writable: bool,
}

/// Why a property write was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropertyError {
    /// No property has this name.
    Unsupported,
    /// The property is read-only.
    ReadOnly,
    /// The value has another kind than the property.
    WrongKind,
    /// The value is outside the range, or is not one of the names.
    OutOfRange,
}

impl fmt::Display for PropertyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PropertyError::Unsupported => write!(f, "the property is not supported"),
            PropertyError::ReadOnly => write!(f, "the property is read-only"),
            PropertyError::WrongKind => write!(f, "the value has the wrong kind for the property"),
            PropertyError::OutOfRange => write!(f, "the value is outside the property's range"),
        }
    }
}

/// The properties of one session, with the values set so far.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PropertySet {
    specs: BTreeMap<&'static str, Spec>,
    values: BTreeMap<&'static str, Value>,
}

impl PropertySet {
    /// A set with the given specs and no values set. A later spec with the same name replaces an earlier one.
    pub fn new(specs: Vec<Spec>) -> Self {
        let mut map = BTreeMap::new();
        for spec in specs {
            let name = spec.name;
            map.insert(name, spec);
        }
        PropertySet { specs: map, values: BTreeMap::new() }
    }

    /// The property dictionary: every spec, in name order.
    pub fn supported(&self) -> Vec<&Spec> {
        self.specs.values().collect()
    }

    /// The value of `name`: the one set, or the default. None when the name is unknown.
    pub fn copy_property(&self, name: &str) -> Option<Value> {
        let spec = self.specs.get(name)?;
        Some(self.values.get(spec.name).cloned().unwrap_or_else(|| spec.default.clone()))
    }

    /// Sets a property from a caller. A read-only property refuses the write.
    pub fn set_property(&mut self, name: &str, value: Value) -> Result<(), PropertyError> {
        let writable = self.specs.get(name).ok_or(PropertyError::Unsupported)?.writable;
        if !writable {
            return Err(PropertyError::ReadOnly);
        }
        self.publish(name, value)
    }

    /// Sets a property from the host, read-only ones included. The kind and the range are still checked.
    pub fn publish(&mut self, name: &str, value: Value) -> Result<(), PropertyError> {
        let spec = self.specs.get(name).ok_or(PropertyError::Unsupported)?;
        check(&spec.kind, &value)?;
        let key = spec.name;
        self.values.insert(key, value);
        Ok(())
    }
}

/// Checks that `value` has the kind of `kind`, and lies in its range or list.
fn check(kind: &Kind, value: &Value) -> Result<(), PropertyError> {
    match (kind, value) {
        (Kind::Bool, Value::Bool(_)) => Ok(()),
        (Kind::Int { min, max }, Value::Int(v)) => {
            if v < min || v > max {
                Err(PropertyError::OutOfRange)
            } else {
                Ok(())
            }
        }
        (Kind::Choice(names), Value::Text(text)) => {
            if names.contains(&text.as_str()) {
                Ok(())
            } else {
                Err(PropertyError::OutOfRange)
            }
        }
        _ => Err(PropertyError::WrongKind),
    }
}

/// An encoder that a codec backend has registered.
#[derive(Clone, Debug, PartialEq)]
pub struct EncoderInfo {
    /// A unique identifier, such as the backend's name for the encoder.
    pub id: String,
    /// The codec the encoder produces, such as a four-character code.
    pub codec: String,
    /// Whether the encoder runs on dedicated hardware.
    pub hardware: bool,
    /// The properties the encoder supports. Their values are not set.
    pub properties: PropertySet,
}

/// Why the catalogue refused an encoder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogueError {
    /// An encoder with this identifier is already registered.
    Duplicate,
}

impl fmt::Display for CatalogueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CatalogueError::Duplicate => write!(f, "an encoder with this identifier is already registered"),
        }
    }
}

/// The encoders a host knows about, in registration order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Catalogue {
    encoders: Vec<EncoderInfo>,
}

impl Catalogue {
    pub fn new() -> Self {
        Catalogue::default()
    }

    /// Adds an encoder. Its identifier must not be registered yet.
    pub fn register(&mut self, info: EncoderInfo) -> Result<(), CatalogueError> {
        if self.encoders.iter().any(|e| e.id == info.id) {
            return Err(CatalogueError::Duplicate);
        }
        self.encoders.push(info);
        Ok(())
    }

    /// The encoders for `codec`, or every encoder when `codec` is None, in registration order.
    pub fn list(&self, codec: Option<&str>) -> Vec<&EncoderInfo> {
        self.encoders.iter().filter(|e| codec.map_or(true, |c| e.codec == c)).collect()
    }

    /// A new session for encoder `id`: the encoder's properties with no values set. None when the encoder is unknown.
    pub fn open(&self, id: &str) -> Option<PropertySet> {
        let info = self.encoders.iter().find(|e| e.id == id)?;
        Some(PropertySet::new(info.properties.specs.values().cloned().collect()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODES: &[&str] = &["Main", "High"];

    fn specs() -> Vec<Spec> {
        vec![
            Spec { name: "RealTime", kind: Kind::Bool, default: Value::Bool(false), writable: true },
            Spec { name: "Quality", kind: Kind::Int { min: 0, max: 100 }, default: Value::Int(50), writable: true },
            Spec {
                name: "Profile",
                kind: Kind::Choice(MODES),
                default: Value::Text("Main".to_string()),
                writable: true,
            },
            Spec { name: "EncoderID", kind: Kind::Int { min: 0, max: i64::MAX }, default: Value::Int(0), writable: false },
        ]
    }

    fn encoder(id: &str, codec: &str) -> EncoderInfo {
        EncoderInfo {
            id: id.to_string(),
            codec: codec.to_string(),
            hardware: codec == "hvc1",
            properties: PropertySet::new(specs()),
        }
    }

    #[test]
    fn an_unset_property_reads_its_default() {
        let set = PropertySet::new(specs());
        assert_eq!(set.copy_property("Quality"), Some(Value::Int(50)));
        assert_eq!(set.copy_property("Profile"), Some(Value::Text("Main".to_string())));
        assert_eq!(set.copy_property("Missing"), None);
    }

    #[test]
    fn a_written_value_is_read_back() {
        let mut set = PropertySet::new(specs());
        assert_eq!(set.set_property("Quality", Value::Int(80)), Ok(()));
        assert_eq!(set.copy_property("Quality"), Some(Value::Int(80)));
    }

    #[test]
    fn range_kind_and_names_are_checked_before_a_value_is_kept() {
        let mut set = PropertySet::new(specs());
        assert_eq!(set.set_property("Quality", Value::Int(101)), Err(PropertyError::OutOfRange));
        assert_eq!(set.copy_property("Quality"), Some(Value::Int(50)));
        assert_eq!(set.set_property("RealTime", Value::Int(1)), Err(PropertyError::WrongKind));
        assert_eq!(
            set.set_property("Profile", Value::Text("Baseline".to_string())),
            Err(PropertyError::OutOfRange)
        );
        assert_eq!(set.set_property("Profile", Value::Text("High".to_string())), Ok(()));
        assert_eq!(set.copy_property("Profile"), Some(Value::Text("High".to_string())));
    }

    #[test]
    fn read_only_properties_are_published_by_the_host_only() {
        let mut set = PropertySet::new(specs());
        assert_eq!(set.set_property("EncoderID", Value::Int(3)), Err(PropertyError::ReadOnly));
        assert_eq!(set.publish("EncoderID", Value::Int(3)), Ok(()));
        assert_eq!(set.copy_property("EncoderID"), Some(Value::Int(3)));
        assert_eq!(set.publish("EncoderID", Value::Bool(true)), Err(PropertyError::WrongKind));
    }

    #[test]
    fn the_dictionary_lists_every_spec_in_name_order() {
        let mut set = PropertySet::new(specs());
        assert_eq!(set.set_property("Bogus", Value::Bool(true)), Err(PropertyError::Unsupported));
        assert_eq!(set.publish("Bogus", Value::Bool(true)), Err(PropertyError::Unsupported));
        let names: Vec<&str> = set.supported().iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["EncoderID", "Profile", "Quality", "RealTime"]);
    }

    #[test]
    fn the_catalogue_lists_by_codec_and_refuses_a_repeated_identifier() {
        let mut cat = Catalogue::new();
        assert_eq!(cat.register(encoder("a", "avc1")), Ok(()));
        assert_eq!(cat.register(encoder("b", "hvc1")), Ok(()));
        assert_eq!(cat.register(encoder("a", "hvc1")), Err(CatalogueError::Duplicate));
        assert_eq!(cat.list(None).len(), 2);
        let avc: Vec<&str> = cat.list(Some("avc1")).iter().map(|e| e.id.as_str()).collect();
        assert_eq!(avc, vec!["a"]);
        assert!(cat.list(Some("vp09")).is_empty());
        assert!(cat.open("missing").is_none());
    }

    #[test]
    fn an_opened_session_has_the_encoder_properties_and_its_own_values() {
        let mut cat = Catalogue::new();
        assert_eq!(cat.register(encoder("b", "hvc1")), Ok(()));
        let mut session = cat.open("b").unwrap();
        assert_eq!(session.copy_property("Quality"), Some(Value::Int(50)));
        assert_eq!(session.set_property("Quality", Value::Int(90)), Ok(()));
        let other = cat.open("b").unwrap();
        assert_eq!(other.copy_property("Quality"), Some(Value::Int(50)));
    }
}
