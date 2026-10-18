//! Calendar's scripting answers (`[[script.*]]` in manifest.toml). `make new event with properties {title, start, ...}`
//! arrives as the AppEvent `calendar.make` and creates the event through the Create Event intent, with the same parameters.

use bifrost_app::control::{RpcError, INVALID_PARAMS, METHOD_NOT_FOUND};
use bifrost_core::json::Json;

/// The intent parameters for a `make new event` call. A title is required; the others are the intent's optional ones and
/// pass through unchanged (the dictionary already refused any label the intent does not declare).
pub fn intent_params(params: &Json) -> Result<Json, RpcError> {
    match params.get("title") {
        Some(Json::Str(t)) if !t.trim().is_empty() => Ok(params.clone()),
        _ => Err(RpcError::new(INVALID_PARAMS, "an event needs a title")),
    }
}

/// The reply for an event the dictionary does not declare.
pub fn unhandled(method: &str) -> RpcError {
    RpcError::new(METHOD_NOT_FOUND, format!("Calendar does not handle {method}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::json::parse;

    #[test]
    fn an_event_needs_a_title() {
        let ok = parse(r#"{"title":"Dentist","minutes":30}"#).unwrap();
        assert_eq!(intent_params(&ok).unwrap(), ok);
        assert!(intent_params(&parse(r#"{"minutes":30}"#).unwrap()).is_err());
        assert!(intent_params(&parse(r#"{"title":"  "}"#).unwrap()).is_err());
    }

    #[test]
    fn other_events_are_named_in_the_refusal() {
        assert!(unhandled("ae.calendar.delete").message.contains("ae.calendar.delete"));
    }
}
