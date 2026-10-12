//! Typed client for the `focus_filter` shell service: an app stores its values for a Focus (its Focus filter) and reads the
//! values that apply while that Focus is on. The shell side is `springboard/src/shell/services/focus_filter.rs`; the model
//! is `bifrost_ui::focus_filter`. The shell's only Focus is Do Not Disturb, named [`DO_NOT_DISTURB`].
//!
//! ```ignore
//! let id = focus_filter::set_context(ctx, focus_filter::DO_NOT_DISTURB, &[("mode", "Quiet")]);
//! let id = focus_filter::current(ctx);
//! // in App::service_reply, when the request is `id`:
//! //   let now = focus_filter::decode_current(&result?)?;  // now.focus, now.context
//! // in App::service_event, for service "focus_filter":
//! //   if let Some(focus) = focus_filter::changed_focus(service, event, body) { ... }
//! ```
//! The calls return the request id; the reply comes back through `App::service_reply` (or `service::take_reply`).
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;
pub use bifrost_ui::focus_filter::AppContext;

/// The name the service is registered under in the shell.
pub const SERVICE: &str = "focus_filter";
/// The Focus name for Do Not Disturb, the Focus the shell has.
pub const DO_NOT_DISTURB: &str = "Do Not Disturb";

/// The Focus that is on, and this app's values for it. `focus` is None when no Focus is on; `context` is None when the
/// app has no values for the Focus that is on.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Current {
    pub focus: Option<String>,
    pub context: Option<AppContext>,
}

/// Stores `values` (key and value text) as this app's Focus filter for `focus`, replacing what it had.
pub fn set_context(ctx: &Ctx, focus: &str, values: &[(&str, &str)]) -> u32 {
    let pairs = values.iter().map(|(k, v)| (k.to_string(), Json::Str(v.to_string()))).collect();
    service::call(ctx, SERVICE, "set_context", &obj(vec![("focus", Json::Str(focus.into())), ("values", Json::Obj(pairs))]))
}

/// Removes this app's values for `focus`. The reply says whether there were any.
pub fn remove_context(ctx: &Ctx, focus: &str) -> u32 {
    service::call(ctx, SERVICE, "remove_context", &obj(vec![("focus", Json::Str(focus.into()))]))
}

/// Reads the Focus that is on and this app's values for it.
pub fn current(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "current", &Json::Null)
}

/// Decodes a `current` reply.
pub fn decode_current(v: &Json) -> Result<Current, String> {
    let focus = v.str_field("focus");
    let context = match v.get("values") {
        Some(Json::Obj(fields)) => {
            let mut c = AppContext::new();
            for (key, value) in fields {
                let text = value.as_str().ok_or_else(|| format!("the value for {key} is not text"))?;
                c.set(key, text);
            }
            Some(c)
        }
        _ => None,
    };
    Ok(Current { focus, context })
}

/// Decodes a `remove_context` reply: true when there were values to remove.
pub fn decode_removed(v: &Json) -> Result<bool, String> {
    match v.get("removed") {
        Some(Json::Bool(b)) => Ok(*b),
        _ => Err("the reply has no removed flag".to_string()),
    }
}

/// Reads a service event. Returns the Focus that is on now (None for no Focus) for a `changed` event from this service,
/// and None for anything else.
pub fn changed_focus(service: &str, event: &str, body: &Json) -> Option<Option<String>> {
    if service == SERVICE && event == "changed" {
        Some(body.str_field("focus"))
    } else {
        None
    }
}

fn obj(pairs: Vec<(&str, Json)>) -> Json {
    Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(text: &str) -> Json {
        bifrost_core::json::parse(text).expect("valid json")
    }

    #[test]
    fn a_current_reply_with_a_focus_and_values_decodes_both() {
        let now = decode_current(&body(r#"{"focus":"Do Not Disturb","values":{"mode":"Quiet"}}"#)).unwrap();
        assert_eq!(now.focus.as_deref(), Some("Do Not Disturb"));
        assert_eq!(now.context.as_ref().and_then(|c| c.get("mode")), Some("Quiet"));
    }

    #[test]
    fn nulls_decode_to_no_focus_and_no_values() {
        let now = decode_current(&body(r#"{"focus":null,"values":null}"#)).unwrap();
        assert_eq!(now, Current::default());
    }

    #[test]
    fn a_value_that_is_not_text_is_an_error_naming_the_key() {
        let err = decode_current(&body(r#"{"focus":"Work","values":{"room":3}}"#)).unwrap_err();
        assert!(err.contains("room"), "{err}");
    }

    #[test]
    fn remove_reply_reads_the_flag() {
        assert_eq!(decode_removed(&body(r#"{"removed":true}"#)), Ok(true));
        assert_eq!(decode_removed(&body(r#"{"removed":false}"#)), Ok(false));
        assert!(decode_removed(&body(r#"{}"#)).is_err());
    }

    #[test]
    fn only_the_changed_event_of_this_service_is_read() {
        assert_eq!(changed_focus("focus_filter", "changed", &body(r#"{"focus":"Do Not Disturb"}"#)), Some(Some("Do Not Disturb".to_string())));
        assert_eq!(changed_focus("focus_filter", "changed", &body(r#"{"focus":null}"#)), Some(None));
        assert_eq!(changed_focus("focus_filter", "other", &body(r#"{}"#)), None);
        assert_eq!(changed_focus("audio-devices", "changed", &body(r#"{"focus":"x"}"#)), None);
    }
}
