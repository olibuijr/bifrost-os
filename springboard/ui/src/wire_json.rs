//! Small helpers shared by the models that cross the shell service wire (displays, windows, input events): build an
//! object from fields, read typed fields back, and encode a rectangle. The shell encodes with these and the SDK
//! clients decode with them, so both sides agree on the field names by construction.

use crate::Rect;
use bifrost_core::json::Json;

/// An object from `(name, value)` pairs.
pub fn obj(fields: Vec<(&str, Json)>) -> Json {
    Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

pub fn num(v: impl Into<f64>) -> Json {
    Json::Num(v.into())
}

pub fn text(s: &str) -> Json {
    Json::Str(s.to_string())
}

/// A number field, or `None` when absent or not a number.
pub fn f64_of(j: &Json, key: &str) -> Option<f64> {
    j.get(key)?.number()
}

pub fn f32_of(j: &Json, key: &str) -> Option<f32> {
    f64_of(j, key).map(|v| v as f32)
}

pub fn u32_of(j: &Json, key: &str) -> Option<u32> {
    f64_of(j, key).filter(|v| *v >= 0.0).map(|v| v as u32)
}

pub fn i32_of(j: &Json, key: &str) -> Option<i32> {
    f64_of(j, key).map(|v| v as i32)
}

pub fn i64_of(j: &Json, key: &str) -> Option<i64> {
    f64_of(j, key).map(|v| v as i64)
}

pub fn bool_of(j: &Json, key: &str) -> Option<bool> {
    match j.get(key)? {
        Json::Bool(b) => Some(*b),
        _ => None,
    }
}

pub fn str_of(j: &Json, key: &str) -> Option<String> {
    j.get(key)?.as_str().map(str::to_string)
}

/// `{x, y, w, h}`.
pub fn rect_json(r: Rect) -> Json {
    obj(vec![("x", num(r.x)), ("y", num(r.y)), ("w", num(r.w)), ("h", num(r.h))])
}

/// The rectangle in field `key`, or `None` when it is missing or incomplete.
pub fn rect_of(j: &Json, key: &str) -> Option<Rect> {
    let r = j.get(key)?;
    Some(Rect { x: f32_of(r, "x")?, y: f32_of(r, "y")?, w: f32_of(r, "w")?, h: f32_of(r, "h")? })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rectangle_survives_a_round_trip() {
        let r = Rect { x: 1.0, y: -2.5, w: 300.0, h: 200.0 };
        let j = obj(vec![("frame", rect_json(r)), ("n", num(7u32)), ("ok", Json::Bool(true)), ("s", text("a"))]);
        assert_eq!(rect_of(&j, "frame"), Some(r));
        assert_eq!(u32_of(&j, "n"), Some(7));
        assert_eq!(bool_of(&j, "ok"), Some(true));
        assert_eq!(str_of(&j, "s").as_deref(), Some("a"));
        assert_eq!(u32_of(&j, "missing"), None);
        assert_eq!(rect_of(&j, "n"), None);
    }

    #[test]
    fn a_negative_number_is_not_an_unsigned_field() {
        let j = obj(vec![("n", num(-1.0))]);
        assert_eq!(u32_of(&j, "n"), None);
        assert_eq!(i32_of(&j, "n"), Some(-1));
    }
}
