//! JavaScript values for apps. A script's result arrives from a web view as a type name and JSON text (`EvalValue`);
//! `JsValue::from_eval` reads it into a Rust value tree. `to_literal` writes a value back as a script literal, so an app
//! can pass numbers, strings, booleans, null, undefined, arrays and objects into a script. A DOM element has no Rust
//! form, and a number that is not finite has no literal. Only the web view engine runs scripts.

use bifrost_core::json::{parse, quote, Json};

use crate::webview_dev::EvalValue;

/// One JavaScript value, as an app sees it.
#[derive(Clone, Debug, PartialEq)]
pub enum JsValue {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsValue>),
    /// The fields in the order the script wrote them.
    Object(Vec<(String, JsValue)>),
}

impl JsValue {
    /// Reads the typed result of a finished evaluation. The type name must agree with the JSON text.
    pub fn from_eval(value: &EvalValue) -> Result<JsValue, String> {
        match value.ty.as_str() {
            "undefined" => Ok(JsValue::Undefined),
            "element" => Err("a DOM element has no value form in apps".into()),
            ty @ ("null" | "boolean" | "number" | "string" | "array" | "object") => {
                let read = Self::from_json(&parse(&value.json)?);
                if read.type_name() == ty {
                    Ok(read)
                } else {
                    Err(format!("the text of a {ty} result reads as {}", read.type_name()))
                }
            }
            other => Err(format!("unknown value type {other}")),
        }
    }

    fn from_json(json: &Json) -> JsValue {
        match json {
            Json::Null => JsValue::Null,
            Json::Bool(b) => JsValue::Bool(*b),
            Json::Num(n) => JsValue::Number(*n),
            Json::Str(s) => JsValue::String(s.clone()),
            Json::Arr(items) => JsValue::Array(items.iter().map(Self::from_json).collect()),
            Json::Obj(fields) => JsValue::Object(fields.iter().map(|(k, v)| (k.clone(), Self::from_json(v))).collect()),
        }
    }

    /// The name a script gives this value's type, as `typeof` and `EvalValue::ty` spell it (null is "null").
    pub fn type_name(&self) -> &'static str {
        match self {
            JsValue::Undefined => "undefined",
            JsValue::Null => "null",
            JsValue::Bool(_) => "boolean",
            JsValue::Number(_) => "number",
            JsValue::String(_) => "string",
            JsValue::Array(_) => "array",
            JsValue::Object(_) => "object",
        }
    }

    /// The first field of an object called `key`. Other kinds of value have no fields.
    pub fn get(&self, key: &str) -> Option<&JsValue> {
        match self {
            JsValue::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The JavaScript expression for this value. Undefined is `undefined`; a number that is not finite is refused.
    pub fn to_literal(&self) -> Result<String, String> {
        Ok(match self {
            JsValue::Undefined => "undefined".into(),
            JsValue::Null => "null".into(),
            JsValue::Bool(b) => b.to_string(),
            JsValue::Number(n) if n.is_finite() => format!("{n}"),
            JsValue::Number(n) => return Err(format!("{n} has no JavaScript literal")),
            JsValue::String(s) => quote(s),
            JsValue::Array(items) => {
                let parts = items.iter().map(JsValue::to_literal).collect::<Result<Vec<_>, _>>()?;
                format!("[{}]", parts.join(","))
            }
            JsValue::Object(fields) => {
                let parts = fields
                    .iter()
                    .map(|(k, v)| -> Result<String, String> { Ok(format!("{}:{}", quote(k), v.to_literal()?)) })
                    .collect::<Result<Vec<_>, String>>()?;
                format!("{{{}}}", parts.join(","))
            }
        })
    }
}
