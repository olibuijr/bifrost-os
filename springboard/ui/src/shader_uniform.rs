//! Shader uniforms: the named values an app sets for a custom shader program. The table checks names and values before
//! they reach the GPU, and it keeps each name's type fixed, so a float cannot later be written as a vector.
//!
//! Pure: it compiles nothing and binds nothing. The renderer reads `iter` and sets each value on the program it owns.
//!
//! Rules: a name is 1 to 64 characters of ASCII letters, digits and underscores, and does not start with a digit.
//! Float and vector components must be finite. Writing a name again keeps its type; a different type is refused.

use std::collections::BTreeMap;

/// A value a shader can read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Float(f32),
    Vec2([f32; 2]),
    Vec4([f32; 4]),
    Int(i32),
}

/// Why a uniform was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UniformError {
    /// The name is empty, too long, or has characters a shader name cannot have.
    BadName,
    /// A float or vector component is NaN or infinite.
    NotFinite,
    /// The name already holds a value of another type.
    TypeChanged,
}

const MAX_NAME: usize = 64;

fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_NAME {
        return false;
    }
    if bytes[0].is_ascii_digit() {
        return false;
    }
    bytes.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_')
}

fn finite(value: &Value) -> bool {
    match value {
        Value::Float(f) => f.is_finite(),
        Value::Vec2(v) => v.iter().all(|c| c.is_finite()),
        Value::Vec4(v) => v.iter().all(|c| c.is_finite()),
        Value::Int(_) => true,
    }
}

/// The uniforms of one shader, in name order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UniformSet {
    values: BTreeMap<String, Value>,
}

impl UniformSet {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a uniform. The name must be valid, the value finite, and the type must match any value already stored
    /// under the name.
    pub fn set(&mut self, name: &str, value: Value) -> Result<(), UniformError> {
        if !valid_name(name) {
            return Err(UniformError::BadName);
        }
        if !finite(&value) {
            return Err(UniformError::NotFinite);
        }
        if let Some(old) = self.values.get(name) {
            if std::mem::discriminant(old) != std::mem::discriminant(&value) {
                return Err(UniformError::TypeChanged);
            }
        }
        self.values.insert(name.to_string(), value);
        Ok(())
    }

    /// The value stored under a name.
    pub fn get(&self, name: &str) -> Option<Value> {
        self.values.get(name).copied()
    }

    /// Removes a uniform. Returns whether it was present.
    pub fn remove(&mut self, name: &str) -> bool {
        self.values.remove(name).is_some()
    }

    /// Every (name, value) pair, sorted by name.
    pub fn iter(&self) -> impl Iterator<Item = (&str, Value)> {
        self.values.iter().map(|(k, v)| (k.as_str(), *v))
    }

    /// The number of uniforms.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_must_be_identifiers() {
        let mut u = UniformSet::new();
        assert_eq!(u.set("", Value::Int(1)), Err(UniformError::BadName));
        assert_eq!(u.set("2fast", Value::Int(1)), Err(UniformError::BadName));
        assert_eq!(u.set("a-b", Value::Int(1)), Err(UniformError::BadName));
        assert_eq!(u.set(&"x".repeat(65), Value::Int(1)), Err(UniformError::BadName));
        assert!(u.set("time_s", Value::Float(0.5)).is_ok());
        assert!(u.set("_tint2", Value::Vec4([1.0, 0.0, 0.0, 1.0])).is_ok());
    }

    #[test]
    fn non_finite_components_are_refused() {
        let mut u = UniformSet::new();
        assert_eq!(u.set("f", Value::Float(f32::NAN)), Err(UniformError::NotFinite));
        assert_eq!(u.set("v", Value::Vec2([0.0, f32::INFINITY])), Err(UniformError::NotFinite));
        assert!(u.is_empty());
    }

    #[test]
    fn a_name_keeps_its_type_but_its_value_can_change() {
        let mut u = UniformSet::new();
        u.set("gain", Value::Float(1.0)).unwrap();
        assert_eq!(u.set("gain", Value::Int(2)), Err(UniformError::TypeChanged));
        u.set("gain", Value::Float(0.25)).unwrap();
        assert_eq!(u.get("gain"), Some(Value::Float(0.25)));
    }

    #[test]
    fn iteration_is_sorted_and_remove_reports_presence() {
        let mut u = UniformSet::new();
        u.set("zeta", Value::Int(3)).unwrap();
        u.set("alpha", Value::Vec2([1.0, 2.0])).unwrap();
        let names: Vec<&str> = u.iter().map(|(n, _)| n).collect();
        assert_eq!(names, vec!["alpha", "zeta"]);
        assert_eq!(u.len(), 2);
        assert!(u.remove("alpha"));
        assert!(!u.remove("alpha"));
        assert_eq!(u.get("alpha"), None);
    }
}
