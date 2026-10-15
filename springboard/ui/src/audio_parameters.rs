//! Audio parameters: named float settings with a range that a render unit exposes, such as a volume, a filter
//! cutoff or a wet/dry mix. A table keeps each parameter's range, default and current value, clamps writes into
//! the range and refuses ids that were never defined. The table is pure state: the render callback reads the
//! values, and the shell or an app writes them. Nothing here schedules a change over time.

/// Why a parameter operation is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParameterError {
    /// No parameter with that id is defined.
    Unknown(u32),
    /// A parameter with that id is already defined.
    Duplicate(u32),
    /// The range is not finite or runs backwards, or the default lies outside it.
    InvalidRange,
    /// The value is not finite.
    InvalidValue,
}

/// One defined parameter: its id, name, range, default and current value.
#[derive(Clone, Debug, PartialEq)]
pub struct Parameter {
    pub id: u32,
    pub name: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    value: f32,
}

impl Parameter {
    /// The current value, always within `min..=max`.
    pub fn value(&self) -> f32 {
        self.value
    }
}

/// The parameters of one render unit, in definition order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParameterTable {
    parameters: Vec<Parameter>,
}

impl ParameterTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Defines a parameter with its range and default. The default must lie inside the range.
    pub fn define(&mut self, id: u32, name: &str, min: f32, max: f32, default: f32) -> Result<(), ParameterError> {
        if self.get(id).is_some() {
            return Err(ParameterError::Duplicate(id));
        }
        let finite = min.is_finite() && max.is_finite() && default.is_finite();
        if !finite || min > max || default < min || default > max {
            return Err(ParameterError::InvalidRange);
        }
        self.parameters.push(Parameter { id, name: name.to_string(), min, max, default, value: default });
        Ok(())
    }

    /// Writes a value, clamped into the parameter's range. Returns the value that was stored.
    pub fn set(&mut self, id: u32, value: f32) -> Result<f32, ParameterError> {
        if !value.is_finite() {
            return Err(ParameterError::InvalidValue);
        }
        let parameter = self.get_mut(id)?;
        parameter.value = value.clamp(parameter.min, parameter.max);
        Ok(parameter.value)
    }

    /// The current value of a parameter.
    pub fn value(&self, id: u32) -> Result<f32, ParameterError> {
        self.get(id).map(Parameter::value).ok_or(ParameterError::Unknown(id))
    }

    /// Returns a parameter to its default value.
    pub fn reset(&mut self, id: u32) -> Result<(), ParameterError> {
        let parameter = self.get_mut(id)?;
        parameter.value = parameter.default;
        Ok(())
    }

    /// The parameter with this id, if it is defined.
    pub fn get(&self, id: u32) -> Option<&Parameter> {
        self.parameters.iter().find(|p| p.id == id)
    }

    /// Every defined parameter, in definition order.
    pub fn parameters(&self) -> &[Parameter] {
        &self.parameters
    }

    fn get_mut(&mut self, id: u32) -> Result<&mut Parameter, ParameterError> {
        self.parameters.iter_mut().find(|p| p.id == id).ok_or(ParameterError::Unknown(id))
    }
}
