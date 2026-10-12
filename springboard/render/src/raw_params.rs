//! Processing parameters for a RAW image processor, as a pure model (std only).
//!
//! A RAW processor exposes a list of named parameters, such as exposure or white
//! balance. Each parameter is a boolean, an integer or a floating-point value with a
//! range, an initial value, and optionally a neutral value (no adjustment) and a
//! camera value (the value the camera recorded). This module holds those parameters,
//! clamps values into their range, and resets them. It decodes no RAW data and runs
//! no processing; the processor that owns the parameters does that.

use std::fmt;

/// A value a parameter can hold.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RawValue {
    Boolean(bool),
    Integer(i64),
    Float(f64),
}

/// The type and bounds of a parameter. Integer and float bounds are inclusive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RawRange {
    Boolean,
    Integer { minimum: i64, maximum: i64 },
    Float { minimum: f64, maximum: f64 },
}

/// Why a parameter or a parameter set refused a request.
#[derive(Clone, Debug, PartialEq)]
pub enum RawError {
    /// The key is empty.
    EmptyKey,
    /// Two parameters in one set share a key.
    DuplicateKey(String),
    /// No parameter in the set has this key.
    UnknownKey(String),
    /// The minimum is above the maximum, or a float bound is not finite.
    InvalidRange,
    /// The value kind does not match the parameter's range.
    TypeMismatch,
    /// A constructor value (initial, neutral or camera) lies outside the range.
    OutOfRange,
    /// A float value is NaN or infinite.
    NotFinite,
}

impl fmt::Display for RawError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RawError::EmptyKey => f.write_str("parameter key is empty"),
            RawError::DuplicateKey(key) => write!(f, "parameter {key} is already present"),
            RawError::UnknownKey(key) => write!(f, "no parameter {key}"),
            RawError::InvalidRange => f.write_str("parameter range is inverted or not finite"),
            RawError::TypeMismatch => f.write_str("value kind does not match the parameter"),
            RawError::OutOfRange => f.write_str("value lies outside the parameter range"),
            RawError::NotFinite => f.write_str("float value is not finite"),
        }
    }
}

/// Checks that a range is well formed: minimum at most maximum, and finite float bounds.
fn check_range(range: RawRange) -> Result<(), RawError> {
    match range {
        RawRange::Boolean => Ok(()),
        RawRange::Integer { minimum, maximum } => {
            if minimum <= maximum {
                Ok(())
            } else {
                Err(RawError::InvalidRange)
            }
        }
        RawRange::Float { minimum, maximum } => {
            if minimum.is_finite() && maximum.is_finite() && minimum <= maximum {
                Ok(())
            } else {
                Err(RawError::InvalidRange)
            }
        }
    }
}

/// Checks that a value fits the range exactly, without clamping. Used for constructor values.
fn check_exact(range: RawRange, value: RawValue) -> Result<(), RawError> {
    match (range, value) {
        (RawRange::Boolean, RawValue::Boolean(_)) => Ok(()),
        (RawRange::Integer { minimum, maximum }, RawValue::Integer(v)) => {
            if (minimum..=maximum).contains(&v) {
                Ok(())
            } else {
                Err(RawError::OutOfRange)
            }
        }
        (RawRange::Float { minimum, maximum }, RawValue::Float(v)) => {
            if !v.is_finite() {
                Err(RawError::NotFinite)
            } else if v < minimum || v > maximum {
                Err(RawError::OutOfRange)
            } else {
                Ok(())
            }
        }
        _ => Err(RawError::TypeMismatch),
    }
}

/// Clamps a value into the range. Integers and floats are clamped; a boolean is never changed.
fn clamp_value(range: RawRange, value: RawValue) -> Result<RawValue, RawError> {
    match (range, value) {
        (RawRange::Boolean, RawValue::Boolean(b)) => Ok(RawValue::Boolean(b)),
        (RawRange::Integer { minimum, maximum }, RawValue::Integer(v)) => {
            Ok(RawValue::Integer(v.clamp(minimum, maximum)))
        }
        (RawRange::Float { minimum, maximum }, RawValue::Float(v)) => {
            if v.is_finite() {
                Ok(RawValue::Float(v.clamp(minimum, maximum)))
            } else {
                Err(RawError::NotFinite)
            }
        }
        _ => Err(RawError::TypeMismatch),
    }
}

/// One processing parameter with its range, its reference values and its current value.
#[derive(Clone, Debug, PartialEq)]
pub struct RawParameter {
    key: String,
    name: String,
    description: String,
    range: RawRange,
    initial: RawValue,
    neutral: Option<RawValue>,
    camera: Option<RawValue>,
    current: RawValue,
    enabled: bool,
}

impl RawParameter {
    /// Builds a parameter. The initial, neutral and camera values must lie inside the range;
    /// they are not clamped, so a bad value is an error.
    pub fn new(
        key: &str,
        name: &str,
        description: &str,
        range: RawRange,
        initial: RawValue,
        neutral: Option<RawValue>,
        camera: Option<RawValue>,
    ) -> Result<RawParameter, RawError> {
        if key.is_empty() {
            return Err(RawError::EmptyKey);
        }
        check_range(range)?;
        check_exact(range, initial)?;
        if let Some(value) = neutral {
            check_exact(range, value)?;
        }
        if let Some(value) = camera {
            check_exact(range, value)?;
        }
        Ok(RawParameter {
            key: key.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            range,
            initial,
            neutral,
            camera,
            current: initial,
            enabled: true,
        })
    }

    /// The unique key, such as `exposure`.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The name shown in UI.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The tool-tip text.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The range.
    pub fn range(&self) -> RawRange {
        self.range
    }

    /// The value the parameter holds now.
    pub fn value(&self) -> RawValue {
        self.current
    }

    /// The value that means no adjustment, if the processor defines one.
    pub fn neutral(&self) -> Option<RawValue> {
        self.neutral
    }

    /// The value the camera recorded, if the processor defines one.
    pub fn camera(&self) -> Option<RawValue> {
        self.camera
    }

    /// Whether the processor applies this parameter.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Enables or disables the parameter.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Stores a value, clamped into the range, and returns the stored value.
    /// A value of the wrong kind is refused and the current value is kept.
    pub fn set(&mut self, value: RawValue) -> Result<RawValue, RawError> {
        let clamped = clamp_value(self.range, value)?;
        self.current = clamped;
        Ok(clamped)
    }

    /// Returns the value to the initial value.
    pub fn reset(&mut self) {
        self.current = self.initial;
    }
}

/// An ordered set of parameters with unique keys.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RawProcessingParameters {
    params: Vec<RawParameter>,
}

impl RawProcessingParameters {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a parameter at the end. A key already in the set is refused.
    pub fn add(&mut self, parameter: RawParameter) -> Result<(), RawError> {
        if self.get(parameter.key()).is_some() {
            return Err(RawError::DuplicateKey(parameter.key().to_string()));
        }
        self.params.push(parameter);
        Ok(())
    }

    /// The parameter with this key.
    pub fn get(&self, key: &str) -> Option<&RawParameter> {
        self.params.iter().find(|p| p.key() == key)
    }

    /// Sets the value of the parameter with this key. Returns the stored (clamped) value.
    pub fn set(&mut self, key: &str, value: RawValue) -> Result<RawValue, RawError> {
        self.find_mut(key)?.set(value)
    }

    /// Enables or disables the parameter with this key.
    pub fn set_enabled(&mut self, key: &str, enabled: bool) -> Result<(), RawError> {
        self.find_mut(key)?.set_enabled(enabled);
        Ok(())
    }

    /// Returns every parameter to its initial value.
    pub fn reset_all(&mut self) {
        for p in &mut self.params {
            p.reset();
        }
    }

    /// The keys in the order they were added.
    pub fn keys(&self) -> Vec<&str> {
        self.params.iter().map(|p| p.key()).collect()
    }

    /// Number of parameters.
    pub fn len(&self) -> usize {
        self.params.len()
    }

    /// True when the set holds no parameter.
    pub fn is_empty(&self) -> bool {
        self.params.is_empty()
    }

    fn find_mut(&mut self, key: &str) -> Result<&mut RawParameter, RawError> {
        self.params
            .iter_mut()
            .find(|p| p.key() == key)
            .ok_or_else(|| RawError::UnknownKey(key.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exposure() -> RawParameter {
        RawParameter::new(
            "exposure",
            "Exposure",
            "Brightness of the picture",
            RawRange::Integer { minimum: -100, maximum: 100 },
            RawValue::Integer(20),
            Some(RawValue::Integer(0)),
            Some(RawValue::Integer(35)),
        )
        .unwrap()
    }

    #[test]
    fn integer_values_are_clamped_into_the_range() {
        let mut p = exposure();
        assert_eq!(p.set(RawValue::Integer(250)), Ok(RawValue::Integer(100)));
        assert_eq!(p.set(RawValue::Integer(-300)), Ok(RawValue::Integer(-100)));
        assert_eq!(p.set(RawValue::Integer(42)), Ok(RawValue::Integer(42)));
        assert_eq!(p.value(), RawValue::Integer(42));
    }

    #[test]
    fn float_values_are_clamped_and_non_finite_values_are_refused() {
        let mut p = RawParameter::new(
            "white",
            "Warmth",
            "Colour temperature shift",
            RawRange::Float { minimum: 0.0, maximum: 2.0 },
            RawValue::Float(1.0),
            Some(RawValue::Float(1.0)),
            None,
        )
        .unwrap();
        assert_eq!(p.set(RawValue::Float(3.5)), Ok(RawValue::Float(2.0)));
        assert_eq!(p.set(RawValue::Float(-0.25)), Ok(RawValue::Float(0.0)));
        assert_eq!(p.set(RawValue::Float(f64::NAN)), Err(RawError::NotFinite));
        assert_eq!(p.value(), RawValue::Float(0.0));
    }

    #[test]
    fn a_value_of_the_wrong_kind_is_refused_and_the_value_is_kept() {
        let mut p = exposure();
        assert_eq!(p.set(RawValue::Boolean(true)), Err(RawError::TypeMismatch));
        assert_eq!(p.set(RawValue::Float(1.0)), Err(RawError::TypeMismatch));
        assert_eq!(p.value(), RawValue::Integer(20));
    }

    #[test]
    fn constructor_rejects_an_inverted_range_and_values_outside_it() {
        let inverted = RawParameter::new(
            "k",
            "K",
            "",
            RawRange::Integer { minimum: 10, maximum: 5 },
            RawValue::Integer(7),
            None,
            None,
        );
        assert_eq!(inverted.unwrap_err(), RawError::InvalidRange);

        let outside = RawParameter::new(
            "k",
            "K",
            "",
            RawRange::Integer { minimum: 0, maximum: 100 },
            RawValue::Integer(200),
            None,
            None,
        );
        assert_eq!(outside.unwrap_err(), RawError::OutOfRange);
    }

    #[test]
    fn reset_returns_to_the_initial_value_and_reference_values_stay_fixed() {
        let mut p = exposure();
        p.set(RawValue::Integer(80)).unwrap();
        p.reset();
        assert_eq!(p.value(), RawValue::Integer(20));
        assert_eq!(p.neutral(), Some(RawValue::Integer(0)));
        assert_eq!(p.camera(), Some(RawValue::Integer(35)));
    }

    #[test]
    fn set_keeps_keys_unique_and_routes_by_key() {
        let mut set = RawProcessingParameters::new();
        set.add(exposure()).unwrap();
        let again = set.add(exposure()).unwrap_err();
        assert_eq!(again, RawError::DuplicateKey("exposure".to_string()));
        assert_eq!(set.len(), 1);

        assert_eq!(set.set("exposure", RawValue::Integer(-500)), Ok(RawValue::Integer(-100)));
        assert_eq!(
            set.set("contrast", RawValue::Integer(1)),
            Err(RawError::UnknownKey("contrast".to_string()))
        );
        assert_eq!(set.get("exposure").unwrap().value(), RawValue::Integer(-100));
    }

    #[test]
    fn reset_all_and_enabled_flags_apply_per_parameter() {
        let mut set = RawProcessingParameters::new();
        set.add(exposure()).unwrap();
        let shadows = RawParameter::new(
            "shadows",
            "Shadows",
            "",
            RawRange::Boolean,
            RawValue::Boolean(false),
            None,
            None,
        )
        .unwrap();
        set.add(shadows).unwrap();
        assert_eq!(set.keys(), vec!["exposure", "shadows"]);

        set.set("exposure", RawValue::Integer(60)).unwrap();
        set.set("shadows", RawValue::Boolean(true)).unwrap();
        set.set_enabled("shadows", false).unwrap();
        assert!(!set.get("shadows").unwrap().enabled());
        assert!(set.get("exposure").unwrap().enabled());

        set.reset_all();
        assert_eq!(set.get("exposure").unwrap().value(), RawValue::Integer(20));
        assert_eq!(set.get("shadows").unwrap().value(), RawValue::Boolean(false));
    }
}
