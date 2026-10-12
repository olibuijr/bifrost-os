//! The settings a user can give a widget (the user-configurable options of a widget configuration): choices, on/off
//! flags and bounded whole numbers, each with a default. Pure model: it checks the definitions, checks every value
//! set, and reports the value in effect (the one set, or else the default). The shell draws the editor and stores
//! the values; the app reads them when it builds the widget's content.

use std::collections::BTreeMap;

/// One user-editable option of a widget.
#[derive(Clone, Debug, PartialEq)]
pub enum Parameter {
    /// One of a list of names; the value is the index of the chosen name.
    Choice { key: String, options: Vec<String>, default: usize },
    /// An on or off option.
    Flag { key: String, default: bool },
    /// A whole number from `min` to `max`, both included.
    Number { key: String, min: i64, max: i64, default: i64 },
}

impl Parameter {
    /// The name the option is stored and read under.
    pub fn key(&self) -> &str {
        match self {
            Parameter::Choice { key, .. } | Parameter::Flag { key, .. } | Parameter::Number { key, .. } => key.as_str(),
        }
    }

    fn default_value(&self) -> Value {
        match self {
            Parameter::Choice { default, .. } => Value::Choice(*default),
            Parameter::Flag { default, .. } => Value::Flag(*default),
            Parameter::Number { default, .. } => Value::Number(*default),
        }
    }
}

/// A value a user has chosen for an option.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Choice(usize),
    Flag(bool),
    Number(i64),
}

/// Why a parameter list or a value was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigError {
    /// An option has an empty name.
    EmptyKey,
    /// Two options share a name.
    DuplicateKey(String),
    /// A choice option lists no names.
    NoOptions(String),
    /// A default lies outside its choices or its number range.
    BadDefault(String),
    /// A number option has `min` above `max`.
    BadRange(String),
    /// No option has this name.
    UnknownKey(String),
    /// The value's kind does not match the option (for example a number for a flag).
    WrongType(String),
    /// A choice index or a number is outside the option's range.
    OutOfRange(String),
}

fn check_parameter(p: &Parameter) -> Result<(), ConfigError> {
    let key = p.key();
    if key.is_empty() {
        return Err(ConfigError::EmptyKey);
    }
    match p {
        Parameter::Choice { options, default, .. } => {
            if options.is_empty() {
                return Err(ConfigError::NoOptions(key.to_string()));
            }
            if *default >= options.len() {
                return Err(ConfigError::BadDefault(key.to_string()));
            }
        }
        Parameter::Flag { .. } => {}
        Parameter::Number { min, max, default, .. } => {
            if min > max {
                return Err(ConfigError::BadRange(key.to_string()));
            }
            if default < min || default > max {
                return Err(ConfigError::BadDefault(key.to_string()));
            }
        }
    }
    Ok(())
}

fn check_value(p: &Parameter, v: &Value) -> Result<(), ConfigError> {
    let key = p.key().to_string();
    match (p, v) {
        (Parameter::Choice { options, .. }, Value::Choice(i)) => {
            if *i >= options.len() {
                Err(ConfigError::OutOfRange(key))
            } else {
                Ok(())
            }
        }
        (Parameter::Flag { .. }, Value::Flag(_)) => Ok(()),
        (Parameter::Number { min, max, .. }, Value::Number(n)) => {
            if n < min || n > max {
                Err(ConfigError::OutOfRange(key))
            } else {
                Ok(())
            }
        }
        _ => Err(ConfigError::WrongType(key)),
    }
}

/// The options of one widget and the values the user has set. Options that were never set report their default.
#[derive(Clone, Debug, PartialEq)]
pub struct Configuration {
    params: Vec<Parameter>,
    values: BTreeMap<String, Value>,
}

impl Configuration {
    /// A configuration with these options. Names must be unique and every definition must be consistent.
    pub fn new(params: Vec<Parameter>) -> Result<Self, ConfigError> {
        for (i, p) in params.iter().enumerate() {
            check_parameter(p)?;
            if params[..i].iter().any(|q| q.key() == p.key()) {
                return Err(ConfigError::DuplicateKey(p.key().to_string()));
            }
        }
        Ok(Configuration { params, values: BTreeMap::new() })
    }

    /// The options in definition order.
    pub fn params(&self) -> &[Parameter] {
        &self.params
    }

    fn param(&self, key: &str) -> Option<&Parameter> {
        self.params.iter().find(|p| p.key() == key)
    }

    /// Sets an option. The value must match the option's kind and range; a refused value changes nothing.
    pub fn set(&mut self, key: &str, value: Value) -> Result<(), ConfigError> {
        {
            let p = self.param(key).ok_or_else(|| ConfigError::UnknownKey(key.to_string()))?;
            check_value(p, &value)?;
        }
        self.values.insert(key.to_string(), value);
        Ok(())
    }

    /// The value in effect for an option: the one set, or else the default. None when the option does not exist.
    pub fn value(&self, key: &str) -> Option<Value> {
        let p = self.param(key)?;
        Some(self.values.get(key).cloned().unwrap_or_else(|| p.default_value()))
    }

    /// True when the user has set the option (the default alone does not count).
    pub fn is_set(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    /// The name chosen for a choice option, if the option exists and is a choice.
    pub fn choice_label(&self, key: &str) -> Option<&str> {
        match (self.param(key)?, self.value(key)?) {
            (Parameter::Choice { options, .. }, Value::Choice(i)) => options.get(i).map(|s| s.as_str()),
            _ => None,
        }
    }

    /// Drops the user's value for one option, so the default applies again.
    pub fn reset(&mut self, key: &str) -> Result<(), ConfigError> {
        if self.param(key).is_none() {
            return Err(ConfigError::UnknownKey(key.to_string()));
        }
        self.values.remove(key);
        Ok(())
    }

    /// Drops every value the user has set.
    pub fn reset_all(&mut self) {
        self.values.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn city() -> Parameter {
        Parameter::Choice {
            key: "city".to_string(),
            options: vec!["Reykjavik".to_string(), "Akureyri".to_string()],
            default: 0,
        }
    }

    fn sample() -> Configuration {
        Configuration::new(vec![
            city(),
            Parameter::Flag { key: "compact".to_string(), default: false },
            Parameter::Number { key: "days".to_string(), min: 1, max: 7, default: 3 },
        ])
        .unwrap()
    }

    #[test]
    fn unset_options_report_their_defaults() {
        let c = sample();
        assert_eq!(c.value("city"), Some(Value::Choice(0)));
        assert_eq!(c.value("compact"), Some(Value::Flag(false)));
        assert_eq!(c.value("days"), Some(Value::Number(3)));
        assert!(!c.is_set("days"));
        assert_eq!(c.value("missing"), None);
    }

    #[test]
    fn a_set_value_replaces_the_default_and_names_the_choice() {
        let mut c = sample();
        c.set("city", Value::Choice(1)).unwrap();
        assert_eq!(c.value("city"), Some(Value::Choice(1)));
        assert_eq!(c.choice_label("city"), Some("Akureyri"));
        assert!(c.is_set("city"));
        c.reset("city").unwrap();
        assert_eq!(c.value("city"), Some(Value::Choice(0)));
        assert_eq!(c.choice_label("city"), Some("Reykjavik"));
    }

    #[test]
    fn numbers_are_kept_inside_their_range() {
        let mut c = sample();
        assert_eq!(c.set("days", Value::Number(8)), Err(ConfigError::OutOfRange("days".to_string())));
        assert_eq!(c.set("days", Value::Number(0)), Err(ConfigError::OutOfRange("days".to_string())));
        assert_eq!(c.value("days"), Some(Value::Number(3)), "a refused value changes nothing");
        c.set("days", Value::Number(7)).unwrap();
        assert_eq!(c.value("days"), Some(Value::Number(7)));
    }

    #[test]
    fn a_value_of_the_wrong_kind_is_refused() {
        let mut c = sample();
        assert_eq!(c.set("compact", Value::Number(1)), Err(ConfigError::WrongType("compact".to_string())));
        assert_eq!(c.set("city", Value::Choice(2)), Err(ConfigError::OutOfRange("city".to_string())));
        assert_eq!(c.set("nope", Value::Flag(true)), Err(ConfigError::UnknownKey("nope".to_string())));
        assert_eq!(c.reset("nope"), Err(ConfigError::UnknownKey("nope".to_string())));
    }

    #[test]
    fn reset_all_clears_every_value() {
        let mut c = sample();
        c.set("compact", Value::Flag(true)).unwrap();
        c.set("days", Value::Number(5)).unwrap();
        c.reset_all();
        assert_eq!(c.value("compact"), Some(Value::Flag(false)));
        assert_eq!(c.value("days"), Some(Value::Number(3)));
    }

    #[test]
    fn inconsistent_definitions_are_refused() {
        let dup = Configuration::new(vec![
            Parameter::Flag { key: "a".to_string(), default: true },
            Parameter::Flag { key: "a".to_string(), default: false },
        ]);
        assert_eq!(dup.unwrap_err(), ConfigError::DuplicateKey("a".to_string()));

        let no_opts = Configuration::new(vec![Parameter::Choice { key: "c".to_string(), options: vec![], default: 0 }]);
        assert_eq!(no_opts.unwrap_err(), ConfigError::NoOptions("c".to_string()));

        let bad_default =
            Configuration::new(vec![Parameter::Choice { key: "c".to_string(), options: vec!["x".to_string()], default: 1 }]);
        assert_eq!(bad_default.unwrap_err(), ConfigError::BadDefault("c".to_string()));

        let range = Configuration::new(vec![Parameter::Number { key: "n".to_string(), min: 5, max: 2, default: 3 }]);
        assert_eq!(range.unwrap_err(), ConfigError::BadRange("n".to_string()));

        let outside = Configuration::new(vec![Parameter::Number { key: "n".to_string(), min: 1, max: 2, default: 9 }]);
        assert_eq!(outside.unwrap_err(), ConfigError::BadDefault("n".to_string()));
    }
}
