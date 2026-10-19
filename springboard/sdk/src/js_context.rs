//! A standalone JavaScript context for apps (`JSContext`, `JSValue`, `JSExport`): run script text with no web view,
//! read typed results, pass values in, expose Rust functions to scripts, and catch exceptions and console output.
//!
//! The engine is the shell's Boa realm (`springboard::shell::js_context`), re-exported here; this module adds the
//! bridge to the SDK's [`JsValue`] (the type web view results use, so one value type serves both paths).
//!
//! ```ignore
//! let mut ctx = js_context::JsContext::new();
//! ctx.set_function("storeGet", |args| Ok(Json::Str(lookup(args[0].as_str().unwrap_or("")))))?;
//! ctx.set_value("settings", &json)?;
//! let total = js_context::evaluate(&mut ctx, "settings.items.map(x => x.price).reduce((a, b) => a + b, 0)")?;
//! ```
//!
//! A context has no DOM, network, timers or file access, and a script that loops or recurses without end is stopped
//! with an exception. What is not covered (object identity across calls, `JSManagedValue`, shared virtual machines,
//! exporting Rust types as classes) is listed in `docs/bifrost-ui.md`.

use bifrost_core::json::Json;

pub use springboard::shell::js_context::{HostFunction, JsContext, JsException, JsOut};

use crate::js_value::JsValue;

/// A script value as an SDK [`JsValue`]. A function, a symbol or a bigint has no `JsValue` form and is refused.
pub fn to_js_value(out: &JsOut) -> Result<JsValue, String> {
    Ok(match out {
        JsOut::Undefined => JsValue::Undefined,
        JsOut::Null => JsValue::Null,
        JsOut::Bool(b) => JsValue::Bool(*b),
        JsOut::Number(n) => JsValue::Number(*n),
        JsOut::String(s) => JsValue::String(s.clone()),
        JsOut::Array(items) => JsValue::Array(items.iter().map(to_js_value).collect::<Result<_, _>>()?),
        JsOut::Object(fields) => JsValue::Object(fields.iter().map(|(k, v)| Ok((k.clone(), to_js_value(v)?))).collect::<Result<_, String>>()?),
        JsOut::BigInt(_) | JsOut::Symbol(_) | JsOut::Function(_) => return Err(format!("a {} has no JsValue form", out.type_name())),
    })
}

/// An SDK [`JsValue`] as JSON for `JsContext::set_value`. Undefined becomes null; a number that is not finite is refused.
pub fn to_json(value: &JsValue) -> Result<Json, String> {
    Ok(match value {
        JsValue::Undefined | JsValue::Null => Json::Null,
        JsValue::Bool(b) => Json::Bool(*b),
        JsValue::Number(n) if n.is_finite() => Json::Num(*n),
        JsValue::Number(n) => return Err(format!("{n} has no JSON form")),
        JsValue::String(s) => Json::Str(s.clone()),
        JsValue::Array(items) => Json::Arr(items.iter().map(to_json).collect::<Result<_, _>>()?),
        JsValue::Object(fields) => Json::Obj(fields.iter().map(|(k, v)| Ok((k.clone(), to_json(v)?))).collect::<Result<_, String>>()?),
    })
}

/// `evaluateScript` with the result as a [`JsValue`]. The error is the exception as `Name: message`.
pub fn evaluate(ctx: &mut JsContext, source: &str) -> Result<JsValue, String> {
    let out = ctx.evaluate_script(source).map_err(|e| e.to_string())?;
    to_js_value(&out)
}

/// Writes an SDK [`JsValue`] into the global `name`.
pub fn set_global(ctx: &mut JsContext, name: &str, value: &JsValue) -> Result<(), String> {
    ctx.set_value(name, &to_json(value)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_arrive_as_sdk_values_and_values_go_back_in() {
        let mut ctx = JsContext::new();
        let v = evaluate(&mut ctx, "({n: 1, list: [true, null, 'x'], nested: {k: 2}})").unwrap();
        assert_eq!(v.get("n"), Some(&JsValue::Number(1.0)));
        assert_eq!(v.get("list"), Some(&JsValue::Array(vec![JsValue::Bool(true), JsValue::Null, JsValue::String("x".into())])));
        set_global(&mut ctx, "input", &v).unwrap();
        assert_eq!(evaluate(&mut ctx, "input.nested.k + input.list.length").unwrap(), JsValue::Number(5.0));
        // the literal form of a value, run as a script, gives the same value back
        let literal = v.to_literal().unwrap();
        assert_eq!(evaluate(&mut ctx, &format!("({literal})")).unwrap(), v);
    }

    #[test]
    fn values_without_a_form_are_refused_and_exceptions_are_text() {
        let mut ctx = JsContext::new();
        assert!(evaluate(&mut ctx, "(function () {})").unwrap_err().contains("function"));
        assert!(evaluate(&mut ctx, "10n").unwrap_err().contains("bigint"));
        assert_eq!(evaluate(&mut ctx, "undefined").unwrap(), JsValue::Undefined);
        assert_eq!(evaluate(&mut ctx, "throw new RangeError('x')").unwrap_err(), "RangeError: x");
        assert!(set_global(&mut ctx, "bad", &JsValue::Number(f64::NAN)).is_err());
        assert_eq!(to_json(&JsValue::Undefined), Ok(Json::Null));
    }

    #[test]
    fn host_functions_work_through_the_sdk_module() {
        let mut ctx = JsContext::new();
        ctx.set_function("upper", |args| Ok(Json::Str(args.first().and_then(Json::as_str).unwrap_or("").to_uppercase()))).unwrap();
        assert_eq!(evaluate(&mut ctx, "upper('abc') + '!'").unwrap(), JsValue::String("ABC!".into()));
    }
}
