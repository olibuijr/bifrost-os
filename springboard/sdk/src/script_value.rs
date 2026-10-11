//! Script value parity: a typed model of the values that cross between an app and a script.
//!
//! Scripts run in a web view (`webview::WebView::eval`, `webview_dev::EvalValue`). The answer comes back as a type
//! name and JSON text. [`ScriptValue`] keeps that answer as a typed tree, applies the JavaScript conversion rules
//! (`ToBoolean`, `ToNumber`, `ToString`) and builds values to hand back to a script. Object keys keep their order.
//! Functions, symbols, promises, dates and DOM elements have no counterpart here: an `element` answer is rejected.

use bifrost_core::json::{self, Json};

#[derive(Clone, Debug, PartialEq)]
pub enum ScriptValue {
    Undefined,
    Null,
    Bool(bool),
    /// IEEE 754 double, as in a script. NaN and the infinities are kept in memory and written as `null` in JSON.
    Number(f64),
    String(String),
    Array(Vec<ScriptValue>),
    /// Own enumerable string keys in insertion order. A repeated key keeps both entries; lookups use the last one.
    Object(Vec<(String, ScriptValue)>),
}

impl ScriptValue {
    /// Reads an eval answer: `ty` is the type name the script reported, `json_text` its JSON text.
    /// `undefined` ignores the text. `element` is a DOM node and has no value model here.
    pub fn from_eval(ty: &str, json_text: &str) -> Result<Self, String> {
        match ty {
            "undefined" => Ok(Self::Undefined),
            "element" => Err("a DOM element has no value model".to_string()),
            "null" | "boolean" | "number" | "string" | "array" | "object" => Self::from_json_text(json_text),
            other => Err(format!("unknown script type {other}")),
        }
    }

    /// Parses JSON text into a value tree.
    pub fn from_json_text(text: &str) -> Result<Self, String> {
        json::parse(text).map(|j| Self::from_json(&j))
    }

    fn from_json(j: &Json) -> Self {
        match j {
            Json::Null => Self::Null,
            Json::Bool(b) => Self::Bool(*b),
            Json::Num(n) => Self::Number(*n),
            Json::Str(s) => Self::String(s.clone()),
            Json::Arr(items) => Self::Array(items.iter().map(Self::from_json).collect()),
            Json::Obj(fields) => Self::Object(fields.iter().map(|(k, v)| (k.clone(), Self::from_json(v))).collect()),
        }
    }

    /// The type name a script reports for this value (the names used in `EvalValue::ty`).
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Undefined => "undefined",
            Self::Null => "null",
            Self::Bool(_) => "boolean",
            Self::Number(_) => "number",
            Self::String(_) => "string",
            Self::Array(_) => "array",
            Self::Object(_) => "object",
        }
    }

    /// Writes the value as JSON the way `JSON.stringify` does for plain data: `undefined` and non-finite numbers
    /// become `null`. Inside an object an `undefined` field is written as `null` too, because the key is kept.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        self.write_json(&mut out);
        out
    }

    fn write_json(&self, out: &mut String) {
        match self {
            Self::Undefined | Self::Null => out.push_str("null"),
            Self::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Self::Number(n) => {
                if n.is_finite() {
                    out.push_str(&number_text(*n));
                } else {
                    out.push_str("null");
                }
            }
            Self::String(s) => out.push_str(&json::quote(s)),
            Self::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write_json(out);
                }
                out.push(']');
            }
            Self::Object(fields) => {
                out.push('{');
                for (i, (key, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&json::quote(key));
                    out.push(':');
                    value.write_json(out);
                }
                out.push('}');
            }
        }
    }

    /// `ToBoolean`: the truth value a script would see in an `if`.
    pub fn to_bool(&self) -> bool {
        match self {
            Self::Undefined | Self::Null => false,
            Self::Bool(b) => *b,
            Self::Number(n) => *n != 0.0 && !n.is_nan(),
            Self::String(s) => !s.is_empty(),
            Self::Array(_) | Self::Object(_) => true,
        }
    }

    /// `ToNumber`. Arrays and objects go through their text first, as `ToPrimitive` does, so `[]` is 0 and `[7]` is 7.
    pub fn to_number(&self) -> f64 {
        match self {
            Self::Undefined => f64::NAN,
            Self::Null => 0.0,
            Self::Bool(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Number(n) => *n,
            Self::String(s) => string_to_number(s),
            Self::Array(_) | Self::Object(_) => string_to_number(&self.to_text()),
        }
    }

    /// `ToString`: the text a script sees when this value is concatenated into a string.
    pub fn to_text(&self) -> String {
        match self {
            Self::Undefined => "undefined".to_string(),
            Self::Null => "null".to_string(),
            Self::Bool(b) => b.to_string(),
            Self::Number(n) => number_text(*n),
            Self::String(s) => s.clone(),
            Self::Array(items) => items
                .iter()
                .map(|v| match v {
                    Self::Undefined | Self::Null => String::new(),
                    other => other.to_text(),
                })
                .collect::<Vec<_>>()
                .join(","),
            Self::Object(_) => "[object Object]".to_string(),
        }
    }

    pub fn is_undefined(&self) -> bool {
        matches!(self, Self::Undefined)
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The named property of an object. A repeated key resolves to its last entry.
    pub fn property(&self, name: &str) -> Option<&ScriptValue> {
        match self {
            Self::Object(fields) => fields.iter().rev().find(|(k, _)| k.as_str() == name).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The element at `index` of an array.
    pub fn element(&self, index: usize) -> Option<&ScriptValue> {
        match self {
            Self::Array(items) => items.get(index),
            _ => None,
        }
    }
}

/// The text `Number::toString` gives for the common cases: whole numbers have no fraction part and the special values
/// have their names. Very large or very small magnitudes are printed in full, not in exponent form.
pub fn number_text(n: f64) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if n == n.trunc() && n.abs() < 9.0e15 {
        return format!("{}", n as i64);
    }
    format!("{n}")
}

/// `StringToNumber` for decimal text: blank is 0, the infinity names are accepted, anything else that does not
/// parse as a decimal number is NaN.
fn string_to_number(s: &str) -> f64 {
    let t = s.trim();
    if t.is_empty() {
        return 0.0;
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    if t.chars().any(|c| c.is_ascii_alphabetic() && c != 'e' && c != 'E') {
        return f64::NAN;
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

impl From<bool> for ScriptValue {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<f64> for ScriptValue {
    fn from(v: f64) -> Self {
        Self::Number(v)
    }
}

impl From<&str> for ScriptValue {
    fn from(v: &str) -> Self {
        Self::String(v.to_string())
    }
}

impl From<String> for ScriptValue {
    fn from(v: String) -> Self {
        Self::String(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_answer_parses_into_a_typed_tree_and_keeps_key_order() {
        let v = ScriptValue::from_eval("object", r#"{"b":1,"a":[true,null,"x"]}"#).unwrap();
        assert_eq!(v.type_name(), "object");
        let keys: Vec<&str> = match &v {
            ScriptValue::Object(fields) => fields.iter().map(|(k, _)| k.as_str()).collect(),
            _ => panic!("not an object"),
        };
        assert_eq!(keys, ["b", "a"]);
        assert_eq!(v.property("b").and_then(|x| x.as_number()), Some(1.0));
        let a = v.property("a").unwrap();
        assert_eq!(a.element(2).and_then(|x| x.as_str()), Some("x"));
        assert!(a.element(1).unwrap().is_null());
        assert!(a.element(3).is_none());
    }

    #[test]
    fn undefined_and_element_answers_are_handled_without_json() {
        assert!(ScriptValue::from_eval("undefined", "null").unwrap().is_undefined());
        assert!(ScriptValue::from_eval("element", "{}").is_err());
        assert!(ScriptValue::from_eval("function", "1").is_err());
        assert!(ScriptValue::from_eval("number", "not json").is_err());
    }

    #[test]
    fn to_json_writes_plain_data_and_escapes_strings() {
        let v = ScriptValue::Object(vec![
            ("n".to_string(), ScriptValue::Number(3.0)),
            ("half".to_string(), ScriptValue::from(0.5)),
            ("nan".to_string(), ScriptValue::Number(f64::NAN)),
            ("u".to_string(), ScriptValue::Undefined),
            ("s".to_string(), ScriptValue::from("a\"b\n")),
        ]);
        assert_eq!(v.to_json(), r#"{"n":3,"half":0.5,"nan":null,"u":null,"s":"a\"b\n"}"#);
        // The text round-trips through the parser into the same tree, apart from undefined becoming null.
        let back = ScriptValue::from_json_text(&v.to_json()).unwrap();
        assert_eq!(back.property("s").and_then(|x| x.as_str()), Some("a\"b\n"));
    }

    #[test]
    fn truthiness_follows_to_boolean() {
        assert!(!ScriptValue::Undefined.to_bool());
        assert!(!ScriptValue::Null.to_bool());
        assert!(!ScriptValue::from(0.0).to_bool());
        assert!(!ScriptValue::Number(f64::NAN).to_bool());
        assert!(!ScriptValue::from("").to_bool());
        assert!(ScriptValue::from("0").to_bool());
        assert!(ScriptValue::Array(vec![]).to_bool());
        assert!(ScriptValue::Object(vec![]).to_bool());
    }

    #[test]
    fn numbers_convert_from_text_the_way_to_number_does() {
        assert_eq!(ScriptValue::from("  42 ").to_number(), 42.0);
        assert_eq!(ScriptValue::from("").to_number(), 0.0);
        assert_eq!(ScriptValue::from("1e3").to_number(), 1000.0);
        assert!(ScriptValue::from("abc").to_number().is_nan());
        assert_eq!(ScriptValue::from("-Infinity").to_number(), f64::NEG_INFINITY);
        assert_eq!(ScriptValue::Null.to_number(), 0.0);
        assert!(ScriptValue::Undefined.to_number().is_nan());
        assert_eq!(ScriptValue::Array(vec![]).to_number(), 0.0);
        assert_eq!(ScriptValue::Array(vec![ScriptValue::from(7.0)]).to_number(), 7.0);
        assert!(ScriptValue::Object(vec![]).to_number().is_nan());
    }

    #[test]
    fn text_conversion_matches_string_concatenation() {
        assert_eq!(number_text(3.0), "3");
        assert_eq!(number_text(-0.0), "0");
        assert_eq!(number_text(0.25), "0.25");
        assert_eq!(number_text(f64::INFINITY), "Infinity");
        assert_eq!(ScriptValue::Undefined.to_text(), "undefined");
        assert_eq!(ScriptValue::Bool(true).to_text(), "true");
        let arr = ScriptValue::Array(vec![ScriptValue::from(1.0), ScriptValue::Null, ScriptValue::from("b")]);
        assert_eq!(arr.to_text(), "1,,b");
        assert_eq!(ScriptValue::Object(vec![]).to_text(), "[object Object]");
    }

    #[test]
    fn accessors_return_none_for_other_types() {
        let s = ScriptValue::from("x");
        assert_eq!(s.as_number(), None);
        assert_eq!(s.as_bool(), None);
        assert_eq!(ScriptValue::from(true).as_bool(), Some(true));
        assert!(s.property("a").is_none());
        assert!(s.element(0).is_none());
    }
}
