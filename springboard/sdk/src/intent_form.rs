//! Parameter form for an intent: the fields an action editor shows, the values entered in them, and the arguments
//! they make. The editor draws the fields; this model checks every value against the declared parameter type, so a
//! form can never produce arguments that `Intent::validate` refuses for a type or a missing required parameter.

use crate::intents::{Intent, Param};
use bifrost_core::json::Json;

/// Why a form refuses a value or cannot produce arguments yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormError {
    /// The intent declares no parameter with that name.
    UnknownParameter(String),
    /// The value does not match the parameter's declared type. `expected` is the type's name.
    WrongType { name: String, expected: String },
    /// A required parameter has no value.
    Missing(String),
}

/// The editable state of one intent. Values are kept in declaration order.
#[derive(Clone, Debug, PartialEq)]
pub struct IntentForm {
    intent: Intent,
    values: Vec<Option<Json>>,
}

impl IntentForm {
    /// An empty form for `intent`: no field has a value.
    pub fn new(intent: Intent) -> IntentForm {
        let values = vec![None; intent.params.len()];
        IntentForm { intent, values }
    }

    /// The fields the editor shows, in declaration order.
    pub fn fields(&self) -> &[Param] {
        &self.intent.params
    }

    /// The value entered for a field, if any.
    pub fn value(&self, name: &str) -> Option<&Json> {
        let index = self.index_of(name).ok()?;
        self.values[index].as_ref()
    }

    /// Sets a field. `Json::Null` clears it. Any other value must match the field's declared type.
    pub fn set(&mut self, name: &str, value: Json) -> Result<(), FormError> {
        let index = self.index_of(name)?;
        if matches!(value, Json::Null) {
            self.values[index] = None;
            return Ok(());
        }
        let param = &self.intent.params[index];
        if !param.ty.accepts(&value) {
            return Err(FormError::WrongType { name: param.name.clone(), expected: param.ty.name() });
        }
        self.values[index] = Some(value);
        Ok(())
    }

    /// The names of required fields that have no value, in declaration order.
    pub fn missing(&self) -> Vec<&str> {
        self.intent
            .params
            .iter()
            .zip(&self.values)
            .filter(|(param, value)| !param.optional && value.is_none())
            .map(|(param, _)| param.name.as_str())
            .collect()
    }

    /// The arguments the form makes: a JSON object holding every field that has a value, in declaration order.
    /// Fails with `Missing` for the first required field that is still empty.
    pub fn arguments(&self) -> Result<Json, FormError> {
        if let Some(name) = self.missing().first() {
            return Err(FormError::Missing((*name).to_string()));
        }
        let fields = self
            .intent
            .params
            .iter()
            .zip(&self.values)
            .filter_map(|(param, value)| value.as_ref().map(|v| (param.name.clone(), v.clone())))
            .collect();
        Ok(Json::Obj(fields))
    }

    fn index_of(&self, name: &str) -> Result<usize, FormError> {
        self.intent
            .params
            .iter()
            .position(|param| param.name == name)
            .ok_or_else(|| FormError::UnknownParameter(name.to_string()))
    }
}
