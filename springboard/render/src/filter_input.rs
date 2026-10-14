//! Filter input checks against the catalogue: the rules a filter object applies to a value before the filter runs. A filter
//! name and key must be catalogued, the value must be finite, and it must lie in the attribute's range. `resolve_inputs`
//! fills every attribute a filter has, using the given values where they are present and the defaults elsewhere, so a
//! renderer receives a complete input list. Pure: no pixels are touched.

use crate::filter_catalogue::{attributes, Attribute};

/// Why an input was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum InputError {
    /// No filter of this reference name is catalogued.
    UnknownFilter(String),
    /// The filter has no input with this key.
    UnknownKey { filter: String, key: String },
    /// The value is NaN or infinite.
    NotFinite,
    /// The value lies outside the attribute's `min..=max` range.
    OutOfRange { min: f32, max: f32 },
}

/// Checks one value for one input of a filter, and returns it.
pub fn check_input(name: &str, key: &str, value: f32) -> Result<f32, InputError> {
    let info = attributes(name).ok_or_else(|| InputError::UnknownFilter(name.to_string()))?;
    let attribute = info
        .attributes
        .iter()
        .find(|a| a.key == key)
        .ok_or_else(|| InputError::UnknownKey { filter: name.to_string(), key: key.to_string() })?;
    check_value(attribute, value)
}

/// Checks every given value and returns the complete input list of the filter, in the catalogue's order: each attribute
/// with its given value, or its default when none was given. A key the filter does not have is refused.
pub fn resolve_inputs(name: &str, given: &[(&str, f32)]) -> Result<Vec<(&'static str, f32)>, InputError> {
    let info = attributes(name).ok_or_else(|| InputError::UnknownFilter(name.to_string()))?;
    for (key, value) in given {
        check_input(name, key, *value)?;
    }
    Ok(info
        .attributes
        .iter()
        .map(|a| {
            let value = given.iter().find(|(k, _)| *k == a.key).map_or(a.default, |(_, v)| *v);
            (a.key, value)
        })
        .collect())
}

fn check_value(attribute: &Attribute, value: f32) -> Result<f32, InputError> {
    if !value.is_finite() {
        return Err(InputError::NotFinite);
    }
    if value < attribute.min || value > attribute.max {
        return Err(InputError::OutOfRange { min: attribute.min, max: attribute.max });
    }
    Ok(value)
}
