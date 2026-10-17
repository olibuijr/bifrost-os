//! The on-device language model for apps (`SystemLanguageModel`): sessions, instructions and profiles, guided
//! generation, tool calling and the token budget.
//!
//! The session, profile, tool, schema and budget types are `bifrost_ui` models, re-exported here. [`run_turn`] runs one
//! turn of a [`Session`] against a [`ModelProvider`]: the provider is the only part that reaches a model, and the SDK
//! ships no provider. [`on_device_availability`] tells an app whether this machine can run a model itself. BifrOSt has
//! no on-device model yet, so it reports [`Availability::Unavailable`] and an app must fall back to something else.
//! A provider that sends the conversation off the device is the app's own choice, and the app has to tell the user.

pub use bifrost_ui::context_window::{estimate_tokens, fit, Fitted};
pub use bifrost_ui::generation_schema::{instructions, parse, read_reply, validate, Issue, Kind, ParseError, ReplyError, Value};
pub use bifrost_ui::model_provider::{run_turn, ModelProvider, ProviderError, Reply, TurnError};
pub use bifrost_ui::model_session::{DynamicInstructions, Frame, Profile, PropertyKey, PropertyValue, Session, SessionProperties};
pub use bifrost_ui::model_tool::{Tool, ToolError, ToolSet};
pub use bifrost_ui::transcript::{Entry, Segment, ToolCall, ToolOutput, Transcript};

/// Why the on-device model cannot be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// No language model is installed on this machine.
    NoOnDeviceModel,
}

/// Whether the on-device language model can answer on this machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Available,
    Unavailable(Unavailable),
}

/// The availability of the on-device model. BifrOSt ships no on-device language model, so this is always
/// unavailable; it is the one place that answers, so it changes when a model runtime is added.
pub fn on_device_availability() -> Availability {
    Availability::Unavailable(Unavailable::NoOnDeviceModel)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A provider that answers every request with the same text and never calls a tool.
    struct Canned;

    impl ModelProvider for Canned {
        fn name(&self) -> &str {
            "canned"
        }

        fn generate(&self, _request: &Transcript) -> Result<Reply, ProviderError> {
            Ok(Reply::Respond(vec![Segment::Text("Hello".into())]))
        }
    }

    #[test]
    fn the_on_device_model_is_reported_unavailable_until_a_runtime_exists() {
        assert_eq!(on_device_availability(), Availability::Unavailable(Unavailable::NoOnDeviceModel));
    }

    #[test]
    fn a_turn_runs_against_a_provider_and_records_the_answer() {
        let mut session = Session::new(Profile::new("brief", DynamicInstructions::new().fixed("Be brief.")));
        session.prompt(vec![Segment::Text("Hi".into())]);
        let out = run_turn(&mut session, &ToolSet::new(), &Canned, 1000, 0, 2);
        assert_eq!(out, Ok(vec![Segment::Text("Hello".into())]));
        assert_eq!(session.history().len(), 2);
    }
}
