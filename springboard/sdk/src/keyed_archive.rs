//! Keyed archives: an object graph written with named fields, read back with an allow-list of classes (secure coding).
//!
//! An object is a class chain (the concrete class first, then its superclasses) and named fields. A field is a
//! scalar, a string, bytes, a list, or a reference to another object by `Uid`. The objects sit in one flat table, so an
//! object referenced from two fields is stored once and keeps its identity, and cycles are allowed. Decoding follows
//! references by index and never recurses through the graph, so a cyclic or deep graph cannot overflow the stack.
//!
//! Each object is its own namespace: a field key only has to be unique inside its object. Keys that start with `$`
//! are reserved for the format.
//!
//! Secure coding: an unarchiver is built with the classes it may decode. An object whose concrete class is not on the
//! list is refused, and so is a reference whose object is not a kind of the class the caller expects. Lookups go by
//! index and never create objects, so a reference cannot swap a class in.
//!
//! Formats. JSON: the archive is written through `coding::JSONEncoder` and read with `coding::JSONDecoder`. The
//! keys are `$archiver`, `$version`, `$top`, `$objects` (index 0 is the `$null` marker), `$classes` and `$uid`, and
//! bytes are a hex string under `$bytes`.
//!
//! Binary (`to_binary`, `from_binary`): the keyed-archive layout inside a binary property list. The top level is a
//! dictionary with `$archiver`, `$version` (100000), `$top` (`{root: uid}`) and `$objects`. Slot 0 of `$objects` is
//! the `$null` string, then one dictionary per object holding `$class` (a uid) and its fields, then one class
//! description (`$classname`, `$classes`) per distinct class chain. Scalars, strings, bytes and lists are stored in
//! the field itself; a `Ref` is a uid, and nil is uid 0. On read, a uid that points at a plain string, number or
//! data slot (how other writers store them) is read as that value. The `$archiver` name is this system's own, so
//! only archives written here are accepted, and the layout is not claimed to be readable by other implementations.
//!
//! Secure coding: `KeyedUnarchiver::new` requires secure coding, as `requiresSecureCoding` does; with it,
//! `decode_object_of` accepts a field only when the object is a kind of one of the classes the caller names.

use std::fmt;

use crate::binary_plist::{self, BValue};
use crate::coding::{CodingError, Decodable, Encodable, JSONDecoder, JSONEncoder, Value};

/// The value of `$version` in the binary layout.
pub const BINARY_ARCHIVE_VERSION: i64 = 100_000;
const CLASS_KEY: &str = "$class";
const CLASSNAME_KEY: &str = "$classname";

/// The value of `$archiver`, which identifies the format.
pub const ARCHIVER_NAME: &str = "BifrostKeyedArchiver";
/// The value of `$version`. Other versions are refused.
pub const ARCHIVE_VERSION: i64 = 1;
/// The most objects one archive may hold, so a hostile document cannot ask for unbounded memory.
pub const MAX_OBJECTS: usize = 1 << 20;

const NULL_MARKER: &str = "$null";
const UID_KEY: &str = "$uid";
const CLASSES_KEY: &str = "$classes";
const BYTES_KEY: &str = "$bytes";

/// An index into an archive's object table. `Uid(0)` is nil and refers to no object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Uid(pub u64);

impl Uid {
    pub const NIL: Uid = Uid(0);

    pub fn is_nil(self) -> bool {
        self.0 == 0
    }
}

/// One stored value in an object's field.
#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    Null,
    Bool(bool),
    Int(i64),
    Num(f64),
    Str(String),
    Bytes(Vec<u8>),
    /// A reference to another object (or nil).
    Ref(Uid),
    List(Vec<Field>),
}

impl Field {
    pub fn as_int(&self) -> Option<i64> {
        if let Field::Int(i) = self { Some(*i) } else { None }
    }

    pub fn as_bool(&self) -> Option<bool> {
        if let Field::Bool(b) = self { Some(*b) } else { None }
    }

    pub fn as_str(&self) -> Option<&str> {
        if let Field::Str(s) = self { Some(s) } else { None }
    }

    /// The referenced `Uid`, when the field is a reference.
    pub fn as_uid(&self) -> Option<Uid> {
        if let Field::Ref(u) = self { Some(*u) } else { None }
    }
}

/// One object: its class chain and its fields in the order they were encoded.
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    classes: Vec<String>,
    fields: Vec<(String, Field)>,
}

impl Object {
    /// The concrete class, which is the first in the chain.
    pub fn class(&self) -> &str {
        &self.classes[0]
    }

    /// The class chain, concrete class first.
    pub fn classes(&self) -> &[String] {
        &self.classes
    }

    /// True when `class` is the concrete class or one of its superclasses.
    pub fn is_kind_of(&self, class: &str) -> bool {
        self.classes.iter().any(|c| c == class)
    }

    pub fn field(&self, key: &str) -> Option<&Field> {
        self.fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn fields(&self) -> &[(String, Field)] {
        &self.fields
    }
}

/// Failures while encoding, validating, reading or decoding an archive.
#[derive(Clone, Debug, PartialEq)]
pub enum ArchiveError {
    /// The text or value is not a keyed archive of this format.
    Malformed(String),
    /// A version other than `ARCHIVE_VERSION`.
    UnsupportedVersion(i64),
    /// A `Uid` that names no object.
    UnknownObject(Uid),
    /// A reserved object was never given fields before `finish`.
    Unfinished(Uid),
    /// `set_fields` was called twice for one object.
    AlreadyEncoded(Uid),
    /// A field key that starts with `$`.
    ReservedKey(String),
    /// A reference to an object outside the table.
    DanglingReference(Uid),
    /// The concrete class of an object is not on the unarchiver's allow-list.
    ClassNotAllowed(String),
    /// The object is not a kind of the class the caller asked for: (expected, concrete class).
    ClassMismatch(String, String),
    /// A field of a different kind than the caller asked for.
    TypeMismatch(&'static str),
    /// More objects than `MAX_OBJECTS`.
    TooLarge,
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(m) => write!(f, "malformed archive: {m}"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported archive version {v}"),
            Self::UnknownObject(u) => write!(f, "no object for uid {}", u.0),
            Self::Unfinished(u) => write!(f, "object {} was reserved but never filled", u.0),
            Self::AlreadyEncoded(u) => write!(f, "object {} already has fields", u.0),
            Self::ReservedKey(k) => write!(f, "field key {k} is reserved"),
            Self::DanglingReference(u) => write!(f, "reference to missing object {}", u.0),
            Self::ClassNotAllowed(c) => write!(f, "class {c} is not allowed"),
            Self::ClassMismatch(expected, found) => write!(f, "expected a {expected}, found a {found}"),
            Self::TypeMismatch(what) => write!(f, "expected {what}"),
            Self::TooLarge => write!(f, "archive has more than {} objects", MAX_OBJECTS),
        }
    }
}

impl std::error::Error for ArchiveError {}

/// A finished, validated archive.
#[derive(Clone, Debug, PartialEq)]
pub struct Archive {
    top: Uid,
    objects: Vec<Object>,
}

impl Archive {
    /// The root object's `Uid` (nil when the archive has no root).
    pub fn top(&self) -> Uid {
        self.top
    }

    /// The object for `uid`; `None` for nil or an index outside the table.
    pub fn object(&self, uid: Uid) -> Option<&Object> {
        let i = usize::try_from(uid.0.checked_sub(1)?).ok()?;
        self.objects.get(i)
    }

    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    /// The archive as JSON text.
    pub fn to_json(&self) -> String {
        JSONEncoder::new().encode(&Wire(self.to_value()))
    }

    /// Reads and validates an archive from JSON text.
    pub fn from_json(text: &str) -> Result<Archive, ArchiveError> {
        let Wire(value) = JSONDecoder::new().decode::<Wire>(text).map_err(|e: CodingError| ArchiveError::Malformed(e.0))?;
        Self::from_value(&value)
    }

    /// The archive as a binary property list in the keyed-archive layout (see the module docs).
    pub fn to_binary(&self) -> Vec<u8> {
        let n = self.objects.len() as u64;
        let mut chains: Vec<&[String]> = Vec::new();
        let mut table = vec![BValue::Str(NULL_MARKER.to_string())];
        for o in &self.objects {
            let at = match chains.iter().position(|c| *c == o.classes.as_slice()) {
                Some(i) => i,
                None => {
                    chains.push(&o.classes);
                    chains.len() - 1
                }
            };
            let mut d = vec![(CLASS_KEY.to_string(), BValue::Uid(n + 1 + at as u64))];
            d.extend(o.fields.iter().map(|(k, f)| (k.clone(), field_bvalue(f))));
            table.push(BValue::Dict(d));
        }
        for chain in chains {
            table.push(BValue::Dict(vec![
                (CLASSNAME_KEY.to_string(), BValue::Str(chain[0].clone())),
                (CLASSES_KEY.to_string(), BValue::Array(chain.iter().map(|c| BValue::Str(c.clone())).collect())),
            ]));
        }
        binary_plist::write(&BValue::Dict(vec![
            ("$archiver".to_string(), BValue::Str(ARCHIVER_NAME.to_string())),
            ("$version".to_string(), BValue::Int(BINARY_ARCHIVE_VERSION)),
            ("$top".to_string(), BValue::Dict(vec![("root".to_string(), BValue::Uid(self.top.0))])),
            ("$objects".to_string(), BValue::Array(table)),
        ]))
    }

    /// Reads and validates an archive from its binary form.
    pub fn from_binary(bytes: &[u8]) -> Result<Archive, ArchiveError> {
        let root = binary_plist::read(bytes).map_err(|e| ArchiveError::Malformed(e.0))?;
        if root.get("$archiver") != Some(&BValue::Str(ARCHIVER_NAME.to_string())) {
            return Err(malformed("$archiver does not name this format"));
        }
        match root.get("$version") {
            Some(BValue::Int(BINARY_ARCHIVE_VERSION)) => {}
            Some(BValue::Int(other)) => return Err(ArchiveError::UnsupportedVersion(*other)),
            _ => return Err(malformed("$version is missing")),
        }
        let top_uid = match root.get("$top").and_then(|t| t.get("root")) {
            Some(BValue::Uid(u)) => *u,
            _ => return Err(malformed("$top has no root")),
        };
        let items = match root.get("$objects") {
            Some(BValue::Array(items)) => items,
            _ => return Err(malformed("$objects is missing")),
        };
        if items.first() != Some(&BValue::Str(NULL_MARKER.to_string())) {
            return Err(malformed("$objects does not start with the null marker"));
        }
        if items.len() - 1 > MAX_OBJECTS {
            return Err(ArchiveError::TooLarge);
        }
        // An object is a slot with a `$class`; its new uid is its position among the objects.
        let mut map = vec![0u64; items.len()];
        let mut next = 0u64;
        for (i, item) in items.iter().enumerate().skip(1) {
            if matches!(item.get(CLASS_KEY), Some(BValue::Uid(_))) {
                next += 1;
                map[i] = next;
            }
        }
        let mut objects = Vec::new();
        for item in items.iter().skip(1) {
            let (Some(BValue::Uid(class_slot)), BValue::Dict(entries)) = (item.get(CLASS_KEY), item) else { continue };
            let classes = class_chain(items, *class_slot)?;
            let mut fields = Vec::new();
            for (k, v) in entries.iter().filter(|(k, _)| k != CLASS_KEY) {
                if k.starts_with('$') {
                    return Err(ArchiveError::ReservedKey(k.clone()));
                }
                fields.push((k.clone(), field_from_bvalue(v, items, &map)?));
            }
            objects.push(Object { classes, fields });
        }
        let top = match usize::try_from(top_uid).ok().and_then(|i| map.get(i)).copied() {
            Some(m) if m != 0 => Uid(m),
            _ if top_uid == 0 => Uid::NIL,
            _ => return Err(malformed("$top names no object")),
        };
        let archive = Archive { top, objects };
        archive.validate()?;
        Ok(archive)
    }

    fn to_value(&self) -> Value {
        let mut objects = vec![Value::Str(NULL_MARKER.to_string())];
        objects.extend(self.objects.iter().map(object_value));
        Value::Dict(vec![
            ("$archiver".to_string(), Value::Str(ARCHIVER_NAME.to_string())),
            ("$version".to_string(), Value::Int(ARCHIVE_VERSION)),
            ("$top".to_string(), Value::Dict(vec![("root".to_string(), uid_value(self.top))])),
            ("$objects".to_string(), Value::Arr(objects)),
        ])
    }

    fn from_value(v: &Value) -> Result<Archive, ArchiveError> {
        let Value::Dict(root) = v else {
            return Err(malformed("the document is not an object"));
        };
        if entry(root, "$archiver") != Some(&Value::Str(ARCHIVER_NAME.to_string())) {
            return Err(malformed("$archiver does not name this format"));
        }
        match entry(root, "$version") {
            Some(Value::Int(ARCHIVE_VERSION)) => {}
            Some(Value::Int(other)) => return Err(ArchiveError::UnsupportedVersion(*other)),
            _ => return Err(malformed("$version is missing")),
        }
        let top = match entry(root, "$top") {
            Some(Value::Dict(t)) => read_uid(entry(t, "root"))?,
            _ => return Err(malformed("$top is missing")),
        };
        let items = match entry(root, "$objects") {
            Some(Value::Arr(items)) => items,
            _ => return Err(malformed("$objects is missing")),
        };
        if items.first() != Some(&Value::Str(NULL_MARKER.to_string())) {
            return Err(malformed("$objects does not start with the null marker"));
        }
        let rest = &items[1..];
        if rest.len() > MAX_OBJECTS {
            return Err(ArchiveError::TooLarge);
        }
        let objects = rest.iter().map(parse_object).collect::<Result<Vec<_>, _>>()?;
        let archive = Archive { top, objects };
        archive.validate()?;
        Ok(archive)
    }

    /// Every reference, the top included, must be nil or name an object in the table.
    fn validate(&self) -> Result<(), ArchiveError> {
        let len = self.objects.len() as u64;
        check_ref(self.top, len)?;
        for o in &self.objects {
            for (_, f) in &o.fields {
                check_field_refs(f, len)?;
            }
        }
        Ok(())
    }
}

fn check_ref(uid: Uid, len: u64) -> Result<(), ArchiveError> {
    if uid.0 <= len { Ok(()) } else { Err(ArchiveError::DanglingReference(uid)) }
}

fn check_field_refs(f: &Field, len: u64) -> Result<(), ArchiveError> {
    match f {
        Field::Ref(u) => check_ref(*u, len),
        Field::List(items) => items.iter().try_for_each(|i| check_field_refs(i, len)),
        _ => Ok(()),
    }
}

/// Builds an archive. Reserve an object first when the graph needs to point at it before its fields are known (for
/// example, a cycle); otherwise `encode_object` does both steps at once.
#[derive(Default)]
pub struct KeyedArchiver {
    slots: Vec<Slot>,
}

#[derive(Clone, Debug)]
struct Slot {
    object: Object,
    filled: bool,
}

impl KeyedArchiver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reserves an object with its class chain (concrete class first) and returns its `Uid`.
    pub fn reserve(&mut self, classes: &[&str]) -> Result<Uid, ArchiveError> {
        if classes.is_empty() {
            return Err(malformed("an object needs at least one class"));
        }
        if self.slots.len() >= MAX_OBJECTS {
            return Err(ArchiveError::TooLarge);
        }
        let object = Object { classes: classes.iter().map(|c| c.to_string()).collect(), fields: Vec::new() };
        self.slots.push(Slot { object, filled: false });
        Ok(Uid(self.slots.len() as u64))
    }

    /// Gives a reserved object its fields. Each object takes its fields once.
    pub fn set_fields(&mut self, uid: Uid, fields: Vec<(String, Field)>) -> Result<(), ArchiveError> {
        if let Some((k, _)) = fields.iter().find(|(k, _)| k.starts_with('$')) {
            return Err(ArchiveError::ReservedKey(k.clone()));
        }
        let slot = uid
            .0
            .checked_sub(1)
            .and_then(|i| usize::try_from(i).ok())
            .and_then(|i| self.slots.get_mut(i))
            .ok_or(ArchiveError::UnknownObject(uid))?;
        if slot.filled {
            return Err(ArchiveError::AlreadyEncoded(uid));
        }
        slot.object.fields = fields;
        slot.filled = true;
        Ok(())
    }

    /// Reserves and fills an object in one step.
    pub fn encode_object(&mut self, classes: &[&str], fields: Vec<(String, Field)>) -> Result<Uid, ArchiveError> {
        let uid = self.reserve(classes)?;
        self.set_fields(uid, fields)?;
        Ok(uid)
    }

    /// Ends the archive with `top` as its root (nil for none). Every reserved object must be filled and every
    /// reference must name an object.
    pub fn finish(self, top: Uid) -> Result<Archive, ArchiveError> {
        if let Some(i) = self.slots.iter().position(|s| !s.filled) {
            return Err(ArchiveError::Unfinished(Uid(i as u64 + 1)));
        }
        let archive = Archive { top, objects: self.slots.into_iter().map(|s| s.object).collect() };
        archive.validate()?;
        Ok(archive)
    }
}

/// Decodes an archive, allowing only the classes it was built with. Decoding never creates objects; it reads them.
#[derive(Clone, Debug)]
pub struct KeyedUnarchiver {
    allowed: Vec<String>,
    requires_secure_coding: bool,
}

impl KeyedUnarchiver {
    /// An unarchiver that refuses every class not in `allowed_classes`.
    pub fn new(allowed_classes: &[&str]) -> Self {
        KeyedUnarchiver { allowed: allowed_classes.iter().map(|c| c.to_string()).collect(), requires_secure_coding: true }
    }

    /// `requiresSecureCoding = false`: the allow-list is not enforced and `decode_object_of` accepts any class.
    /// Only for archives the app wrote itself.
    pub fn without_secure_coding(mut self) -> Self {
        self.requires_secure_coding = false;
        self
    }

    pub fn requires_secure_coding(&self) -> bool {
        self.requires_secure_coding
    }

    /// `decodeObject(of:forKey:)`: the object `owner` holds under `key`, when it is a kind of one of `classes`.
    /// A missing, null or nil field gives `None`. The classes named here are checked whatever the allow-list says;
    /// the allow-list is not consulted. Without secure coding the class check is skipped.
    pub fn decode_object_of<'a>(
        &self,
        archive: &'a Archive,
        owner: &Object,
        classes: &[&str],
        key: &str,
    ) -> Result<Option<&'a Object>, ArchiveError> {
        let uid = match owner.field(key) {
            None | Some(Field::Null) => return Ok(None),
            Some(Field::Ref(uid)) if uid.is_nil() => return Ok(None),
            Some(Field::Ref(uid)) => *uid,
            Some(_) => return Err(ArchiveError::TypeMismatch("an object reference")),
        };
        let object = archive.object(uid).ok_or(ArchiveError::UnknownObject(uid))?;
        if self.requires_secure_coding && !classes.iter().any(|c| object.is_kind_of(c)) {
            return Err(ArchiveError::ClassNotAllowed(object.class().to_string()));
        }
        Ok(Some(object))
    }

    pub fn is_allowed(&self, class: &str) -> bool {
        self.allowed.iter().any(|c| c == class)
    }

    /// The root object (`unarchivedObject(ofClasses:from:)`). `None` when the root is nil.
    pub fn decode_top<'a>(&self, archive: &'a Archive) -> Result<Option<&'a Object>, ArchiveError> {
        if archive.top.is_nil() {
            return Ok(None);
        }
        self.decode_uid(archive, archive.top).map(Some)
    }

    /// The object at `uid`, when its concrete class is allowed.
    pub fn decode_uid<'a>(&self, archive: &'a Archive, uid: Uid) -> Result<&'a Object, ArchiveError> {
        let object = archive.object(uid).ok_or(ArchiveError::UnknownObject(uid))?;
        if self.requires_secure_coding && !self.is_allowed(object.class()) {
            return Err(ArchiveError::ClassNotAllowed(object.class().to_string()));
        }
        Ok(object)
    }

    /// Decodes a reference field as an object that is a kind of `expected`. A missing, null or nil field gives `None`.
    pub fn decode_field<'a>(
        &self,
        archive: &'a Archive,
        field: Option<&Field>,
        expected: &str,
    ) -> Result<Option<&'a Object>, ArchiveError> {
        match field {
            None | Some(Field::Null) => Ok(None),
            Some(Field::Ref(uid)) if uid.is_nil() => Ok(None),
            Some(Field::Ref(uid)) => {
                let object = self.decode_uid(archive, *uid)?;
                if object.is_kind_of(expected) {
                    Ok(Some(object))
                } else {
                    Err(ArchiveError::ClassMismatch(expected.to_string(), object.class().to_string()))
                }
            }
            Some(_) => Err(ArchiveError::TypeMismatch("an object reference")),
        }
    }
}

/// The JSON document as a `Value`, so the encoder and decoder can take it.
struct Wire(Value);

impl Encodable for Wire {
    fn encode(&self) -> Value {
        self.0.clone()
    }
}

impl Decodable for Wire {
    fn decode(v: &Value) -> Result<Self, CodingError> {
        Ok(Wire(v.clone()))
    }
}

fn malformed(m: &str) -> ArchiveError {
    ArchiveError::Malformed(m.to_string())
}

fn entry<'a>(d: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    d.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn uid_value(uid: Uid) -> Value {
    Value::Dict(vec![(UID_KEY.to_string(), Value::Int(uid.0 as i64))])
}

fn uid_from_int(n: i64) -> Result<Uid, ArchiveError> {
    u64::try_from(n).map(Uid).map_err(|_| malformed("negative uid"))
}

fn read_uid(v: Option<&Value>) -> Result<Uid, ArchiveError> {
    match v {
        Some(Value::Dict(d)) => match d.as_slice() {
            [(k, Value::Int(n))] if k.as_str() == UID_KEY => uid_from_int(*n),
            _ => Err(malformed("expected a uid")),
        },
        _ => Err(malformed("expected a uid")),
    }
}

fn field_bvalue(f: &Field) -> BValue {
    match f {
        Field::Null => BValue::Null,
        Field::Bool(b) => BValue::Bool(*b),
        Field::Int(i) => BValue::Int(*i),
        Field::Num(n) => BValue::Real(*n),
        Field::Str(s) => BValue::Str(s.clone()),
        Field::Bytes(b) => BValue::Data(b.clone()),
        Field::Ref(u) => BValue::Uid(u.0),
        Field::List(items) => BValue::Array(items.iter().map(field_bvalue).collect()),
    }
}

/// The class chain named by the class description in `items[slot]`.
fn class_chain(items: &[BValue], slot: u64) -> Result<Vec<String>, ArchiveError> {
    let desc = usize::try_from(slot).ok().and_then(|i| items.get(i));
    match desc.and_then(|d| d.get(CLASSES_KEY)) {
        Some(BValue::Array(names)) if !names.is_empty() => names
            .iter()
            .map(|n| match n {
                BValue::Str(s) => Ok(s.clone()),
                _ => Err(malformed("a class name is not a string")),
            })
            .collect(),
        _ => Err(malformed("an object has no class description")),
    }
}

/// A field from a binary value. `map` turns a slot of `$objects` into an object uid (0 for a slot that is not an
/// object); a uid at a plain value slot is read as that value.
fn field_from_bvalue(v: &BValue, items: &[BValue], map: &[u64]) -> Result<Field, ArchiveError> {
    Ok(match v {
        BValue::Null => Field::Null,
        BValue::Bool(b) => Field::Bool(*b),
        BValue::Int(i) => Field::Int(*i),
        BValue::Real(r) => Field::Num(*r),
        BValue::Str(s) => Field::Str(s.clone()),
        BValue::Data(d) => Field::Bytes(d.clone()),
        BValue::Array(a) => Field::List(a.iter().map(|x| field_from_bvalue(x, items, map)).collect::<Result<Vec<_>, _>>()?),
        BValue::Uid(0) => Field::Ref(Uid::NIL),
        BValue::Uid(slot) => {
            let i = usize::try_from(*slot).map_err(|_| ArchiveError::DanglingReference(Uid(*slot)))?;
            let target = items.get(i).ok_or(ArchiveError::DanglingReference(Uid(*slot)))?;
            match target {
                BValue::Dict(_) if map[i] != 0 => Field::Ref(Uid(map[i])),
                BValue::Str(_) | BValue::Int(_) | BValue::Real(_) | BValue::Bool(_) | BValue::Data(_) => {
                    field_from_bvalue(target, items, map)?
                }
                _ => return Err(malformed("a reference names a slot that is not an object or a plain value")),
            }
        }
        BValue::Date(_) | BValue::Dict(_) => return Err(malformed("a field holds a value that has no field form")),
    })
}

fn object_value(o: &Object) -> Value {
    let mut d = vec![(CLASSES_KEY.to_string(), Value::Arr(o.classes.iter().map(|c| Value::Str(c.clone())).collect()))];
    d.extend(o.fields.iter().map(|(k, f)| (k.clone(), field_value(f))));
    Value::Dict(d)
}

fn field_value(f: &Field) -> Value {
    match f {
        Field::Null => Value::Null,
        Field::Bool(b) => Value::Bool(*b),
        Field::Int(i) => Value::Int(*i),
        Field::Num(n) => Value::Num(*n),
        Field::Str(s) => Value::Str(s.clone()),
        Field::Bytes(b) => Value::Dict(vec![(BYTES_KEY.to_string(), Value::Str(hex_encode(b)))]),
        Field::Ref(u) => uid_value(*u),
        Field::List(items) => Value::Arr(items.iter().map(field_value).collect()),
    }
}

fn parse_object(v: &Value) -> Result<Object, ArchiveError> {
    let Value::Dict(d) = v else {
        return Err(malformed("an object is not a dictionary"));
    };
    let classes = match entry(d, CLASSES_KEY) {
        Some(Value::Arr(cs)) if !cs.is_empty() => cs
            .iter()
            .map(|c| match c {
                Value::Str(s) => Ok(s.clone()),
                _ => Err(malformed("a class name is not a string")),
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(malformed("an object has no class chain")),
    };
    let mut fields = Vec::new();
    for (k, value) in d.iter().filter(|(k, _)| k != CLASSES_KEY) {
        if k.starts_with('$') {
            return Err(ArchiveError::ReservedKey(k.clone()));
        }
        fields.push((k.clone(), parse_field(value)?));
    }
    Ok(Object { classes, fields })
}

fn parse_field(v: &Value) -> Result<Field, ArchiveError> {
    Ok(match v {
        Value::Null => Field::Null,
        Value::Bool(b) => Field::Bool(*b),
        Value::Int(i) => Field::Int(*i),
        Value::Num(n) => Field::Num(*n),
        Value::Str(s) => Field::Str(s.clone()),
        Value::Arr(items) => Field::List(items.iter().map(parse_field).collect::<Result<Vec<_>, _>>()?),
        Value::Dict(d) => match d.as_slice() {
            [(k, Value::Int(n))] if k.as_str() == UID_KEY => Field::Ref(uid_from_int(*n)?),
            [(k, Value::Str(hex))] if k.as_str() == BYTES_KEY => {
                Field::Bytes(hex_decode(hex).ok_or_else(|| malformed("bytes are not hex"))?)
            }
            _ => return Err(malformed("unknown dictionary field")),
        },
        Value::Date(_) => return Err(malformed("a date is not a field")),
    })
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Document (concrete) -> Base. Its `tag` and `alt` fields both point at one shared Tag object.
    fn sample() -> Archive {
        let mut a = KeyedArchiver::new();
        let tag = a
            .encode_object(&["Tag", "Base"], vec![("name".to_string(), Field::Str("tax".to_string()))])
            .unwrap();
        let doc = a
            .encode_object(
                &["Document", "Base"],
                vec![
                    ("title".to_string(), Field::Str("Invoice".to_string())),
                    ("tag".to_string(), Field::Ref(tag)),
                    ("alt".to_string(), Field::Ref(tag)),
                    ("count".to_string(), Field::Int(3)),
                ],
            )
            .unwrap();
        a.finish(doc).unwrap()
    }

    #[test]
    fn a_shared_object_is_stored_once_and_keeps_its_identity() {
        let archive = sample();
        assert_eq!(archive.object_count(), 2);
        let doc = archive.object(archive.top()).unwrap();
        assert_eq!(doc.class(), "Document");
        assert_eq!(doc.field("tag").and_then(Field::as_uid), Some(Uid(1)));
        assert_eq!(doc.field("alt").and_then(Field::as_uid), Some(Uid(1)));
        assert_eq!(doc.field("count").and_then(Field::as_int), Some(3));
    }

    #[test]
    fn json_round_trip_keeps_the_graph_and_the_bytes() {
        let mut a = KeyedArchiver::new();
        let blob = a
            .encode_object(&["Blob"], vec![("data".to_string(), Field::Bytes(vec![0xde, 0xad, 0x00]))])
            .unwrap();
        let archive = a.finish(blob).unwrap();
        let text = archive.to_json();
        let back = Archive::from_json(&text).unwrap();
        assert_eq!(back, archive);
        assert_eq!(back.object(back.top()).unwrap().field("data"), Some(&Field::Bytes(vec![0xde, 0xad, 0x00])));
        let sample_back = Archive::from_json(&sample().to_json()).unwrap();
        assert_eq!(sample_back, sample());
    }

    #[test]
    fn secure_decoding_refuses_classes_not_on_the_list() {
        let archive = sample();
        let doc = archive.object(archive.top()).unwrap();
        let only_document = KeyedUnarchiver::new(&["Document"]);
        assert!(only_document.decode_top(&archive).is_ok());
        // The tag's concrete class is Tag, which is not allowed, even though its superclass is.
        assert_eq!(only_document.decode_field(&archive, doc.field("tag"), "Base"), Err(ArchiveError::ClassNotAllowed("Tag".to_string())));
        let with_tag = KeyedUnarchiver::new(&["Document", "Tag"]);
        let tag = with_tag.decode_field(&archive, doc.field("tag"), "Base").unwrap().unwrap();
        assert_eq!(tag.field("name").and_then(Field::as_str), Some("tax"));
    }

    #[test]
    fn a_reference_must_be_a_kind_of_the_expected_class() {
        let archive = sample();
        let doc = archive.object(archive.top()).unwrap();
        let u = KeyedUnarchiver::new(&["Document", "Tag"]);
        assert_eq!(
            u.decode_field(&archive, doc.field("tag"), "Document"),
            Err(ArchiveError::ClassMismatch("Document".to_string(), "Tag".to_string()))
        );
        assert_eq!(u.decode_field(&archive, None, "Base"), Ok(None));
        assert_eq!(u.decode_field(&archive, Some(&Field::Ref(Uid::NIL)), "Base"), Ok(None));
        assert_eq!(u.decode_field(&archive, doc.field("count"), "Base"), Err(ArchiveError::TypeMismatch("an object reference")));
    }

    #[test]
    fn a_cycle_is_encoded_and_followed_by_index() {
        let mut a = KeyedArchiver::new();
        let first = a.reserve(&["Node"]).unwrap();
        let second = a.reserve(&["Node"]).unwrap();
        a.set_fields(first, vec![("next".to_string(), Field::Ref(second))]).unwrap();
        a.set_fields(second, vec![("next".to_string(), Field::Ref(first))]).unwrap();
        let archive = a.finish(first).unwrap();
        let u = KeyedUnarchiver::new(&["Node"]);
        let one = u.decode_top(&archive).unwrap().unwrap();
        let two = u.decode_field(&archive, one.field("next"), "Node").unwrap().unwrap();
        let back = u.decode_field(&archive, two.field("next"), "Node").unwrap().unwrap();
        assert_eq!(back, one);
    }

    #[test]
    fn finishing_refuses_unfilled_reservations_and_dangling_references() {
        let mut a = KeyedArchiver::new();
        let _ = a.reserve(&["Pending"]).unwrap();
        assert_eq!(a.finish(Uid(1)), Err(ArchiveError::Unfinished(Uid(1))));

        let mut b = KeyedArchiver::new();
        let u = b.encode_object(&["Holder"], vec![("ref".to_string(), Field::Ref(Uid(9)))]).unwrap();
        assert_eq!(b.finish(u), Err(ArchiveError::DanglingReference(Uid(9))));
    }

    #[test]
    fn keys_starting_with_a_dollar_sign_and_second_fills_are_refused() {
        let mut a = KeyedArchiver::new();
        let u = a.reserve(&["Thing"]).unwrap();
        assert_eq!(
            a.set_fields(u, vec![("$hidden".to_string(), Field::Null)]),
            Err(ArchiveError::ReservedKey("$hidden".to_string()))
        );
        a.set_fields(u, vec![("ok".to_string(), Field::Bool(true))]).unwrap();
        assert_eq!(a.set_fields(u, Vec::new()), Err(ArchiveError::AlreadyEncoded(u)));
        assert_eq!(a.reserve(&[]), Err(ArchiveError::Malformed("an object needs at least one class".to_string())));
    }

    #[test]
    fn a_document_that_is_not_this_format_is_refused() {
        let wrong_name = r#"{"$archiver":"Other","$version":1,"$top":{"root":{"$uid":0}},"$objects":["$null"]}"#;
        assert!(matches!(Archive::from_json(wrong_name), Err(ArchiveError::Malformed(_))));
        let wrong_version = r#"{"$archiver":"BifrostKeyedArchiver","$version":2,"$top":{"root":{"$uid":0}},"$objects":["$null"]}"#;
        assert_eq!(Archive::from_json(wrong_version), Err(ArchiveError::UnsupportedVersion(2)));
        let dangling = r#"{"$archiver":"BifrostKeyedArchiver","$version":1,"$top":{"root":{"$uid":4}},"$objects":["$null"]}"#;
        assert_eq!(Archive::from_json(dangling), Err(ArchiveError::DanglingReference(Uid(4))));
    }
}
