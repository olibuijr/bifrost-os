//! The model behind a session. A [`ModelProvider`] answers one request with text or with tool calls. [`run_turn`] runs one
//! turn of a [`Session`]: it asks the provider, runs the tools it calls, records their output, and asks again until the
//! provider answers with text. Each request is the session's history fitted to the budget, with the tool descriptions
//! added to its instructions.
//!
//! The provider is the only part that reaches a model. This module has no network code: an app or a service implements
//! [`ModelProvider`] with its own transport, and a test implements it with a script.

use crate::context_window::estimate_tokens;
use crate::model_attachment::Media;
use crate::model_session::Session;
use crate::model_tool::ToolSet;
use crate::transcript::{Entry, Segment, ToolCall, Transcript};
use std::fmt;

/// What a provider returns for one request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The answer. The turn ends with it.
    Respond(Vec<Segment>),
    /// Tools the model wants run. The turn goes on with their output.
    Call(Vec<ToolCall>),
}

/// A failure reported by a provider, with the message it gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderError {
    pub message: String,
}

impl ProviderError {
    pub fn new(message: impl Into<String>) -> Self {
        ProviderError { message: message.into() }
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "model error: {}", self.message)
    }
}

/// A model that answers requests. Implementations decide how a request is sent.
pub trait ModelProvider {
    /// The model's name, for display and logs.
    fn name(&self) -> &str;

    /// Whether the model accepts attachments of this media type. A provider accepts none unless it says so.
    fn accepts(&self, _media: Media) -> bool {
        false
    }

    /// Answers one request. The transcript starts with an instructions entry when there are any instructions.
    fn generate(&self, request: &Transcript) -> Result<Reply, ProviderError>;
}

/// Why a turn did not finish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnError {
    /// The instructions and the tool descriptions alone do not fit the budget, so no request can be sent.
    OverBudget,
    /// The provider still asked for tools after `rounds` rounds. Nothing more was run.
    ToolRoundsExceeded { rounds: usize },
    /// The provider failed, or it asked for tools without naming any.
    Provider(ProviderError),
}

impl fmt::Display for TurnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TurnError::OverBudget => write!(f, "the instructions and tool descriptions do not fit the budget"),
            TurnError::ToolRoundsExceeded { rounds } => write!(f, "the model still wanted tools after {rounds} rounds"),
            TurnError::Provider(error) => write!(f, "{error}"),
        }
    }
}

/// Runs one turn of `session`. The caller records the prompt first. The turn then asks `provider` with the fitted history
/// and the tool descriptions. Tool calls are run, their outputs recorded, and the provider is asked again. At most
/// `max_tool_rounds` rounds of calls run. The answer is recorded in the session and returned.
///
/// `budget` counts estimated tokens for the whole request, and `reserve` is kept free for the answer. The tool
/// descriptions count against `budget` too.
pub fn run_turn(
    session: &mut Session,
    tools: &ToolSet,
    provider: &dyn ModelProvider,
    budget: usize,
    reserve: usize,
    max_tool_rounds: usize,
) -> Result<Vec<Segment>, TurnError> {
    let description = tools.instructions();
    let description_tokens = estimate_tokens(&description);
    if description_tokens > budget {
        return Err(TurnError::OverBudget);
    }
    let mut rounds = 0;
    loop {
        let fitted = session.prepare(budget - description_tokens, reserve);
        if fitted.over_budget {
            return Err(TurnError::OverBudget);
        }
        let request = with_descriptions(fitted.transcript, &description);
        match provider.generate(&request).map_err(TurnError::Provider)? {
            Reply::Respond(segments) => {
                session.respond(segments.clone());
                return Ok(segments);
            }
            Reply::Call(calls) => {
                if calls.is_empty() {
                    return Err(TurnError::Provider(ProviderError::new("the reply asked for no tool")));
                }
                if rounds == max_tool_rounds {
                    return Err(TurnError::ToolRoundsExceeded { rounds });
                }
                rounds += 1;
                let outputs: Vec<_> = calls.iter().map(|call| tools.output_for(call)).collect();
                session.record(Entry::ToolCalls(calls));
                for output in outputs {
                    session.record(Entry::ToolOutput(output));
                }
            }
        }
    }
}

/// The request with the tool descriptions appended to its instructions. The request is unchanged when there are no tools.
fn with_descriptions(transcript: Transcript, description: &str) -> Transcript {
    if description.is_empty() {
        return transcript;
    }
    let mut entries = transcript.entries().to_vec();
    if let Some(Entry::Instructions(segments)) = entries.first_mut() {
        segments.push(Segment::Text(description.into()));
    } else {
        entries.insert(0, Entry::Instructions(vec![Segment::Text(description.into())]));
    }
    Transcript::new(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation_schema::{Kind, Property};
    use crate::model_session::{DynamicInstructions, Profile};
    use crate::model_tool::Tool;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    /// A provider that returns the scripted replies in order and keeps every request it was sent.
    struct Scripted {
        replies: RefCell<VecDeque<Result<Reply, ProviderError>>>,
        seen: RefCell<Vec<Transcript>>,
    }

    impl Scripted {
        fn new(replies: Vec<Result<Reply, ProviderError>>) -> Self {
            Scripted { replies: RefCell::new(replies.into()), seen: RefCell::new(Vec::new()) }
        }
    }

    impl ModelProvider for Scripted {
        fn name(&self) -> &str {
            "scripted"
        }

        fn generate(&self, request: &Transcript) -> Result<Reply, ProviderError> {
            self.seen.borrow_mut().push(request.clone());
            self.replies.borrow_mut().pop_front().unwrap_or_else(|| Err(ProviderError::new("no reply left")))
        }
    }

    fn text(s: &str) -> Segment {
        Segment::Text(s.into())
    }

    fn session() -> Session {
        Session::new(Profile::new("brief", DynamicInstructions::new().fixed("Be brief.")))
    }

    /// A tool set with one tool, `weather`, whose handler returns `result`.
    fn tools(result: Result<String, String>) -> ToolSet {
        let schema = Kind::Object(vec![Property::new("city", Kind::Text).described("city name")]);
        let mut set = ToolSet::new();
        set.register(Tool::new("weather", "Current weather for a city.", schema, move |_| result.clone())).unwrap();
        set
    }

    fn weather_call() -> Reply {
        Reply::Call(vec![ToolCall { name: "weather".into(), arguments: "{\"city\":\"Reykjavik\"}".into() }])
    }

    #[test]
    fn a_plain_answer_ends_the_turn_and_is_recorded() {
        let provider = Scripted::new(vec![Ok(Reply::Respond(vec![text("Hello")]))]);
        let mut s = session();
        s.prompt(vec![text("Hi")]);
        let out = run_turn(&mut s, &ToolSet::new(), &provider, 1000, 0, 2);
        assert_eq!(out, Ok(vec![text("Hello")]));
        assert_eq!(s.history().len(), 2);
        let seen = provider.seen.borrow();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].instructions().unwrap()[0], text("Be brief."));
        assert!(!provider.accepts(Media::Png), "a provider accepts no attachment unless it says so");
        assert_eq!(provider.name(), "scripted");
    }

    #[test]
    fn a_tool_call_is_run_and_its_output_is_sent_with_the_next_request() {
        let provider = Scripted::new(vec![Ok(weather_call()), Ok(Reply::Respond(vec![text("It is 4 C.")]))]);
        let mut s = session();
        s.prompt(vec![text("Weather in Reykjavik?")]);
        let out = run_turn(&mut s, &tools(Ok("4 C, rain".into())), &provider, 1000, 0, 2);
        assert_eq!(out, Ok(vec![text("It is 4 C.")]));
        let recorded = Transcript::new(s.history().to_vec());
        let typed = recorded.structured();
        assert_eq!(typed.tool_calls.len(), 1);
        assert_eq!(typed.tool_outputs[0].text, "4 C, rain");
        assert_eq!(typed.responses.len(), 1);
        let seen = provider.seen.borrow();
        assert_eq!(seen.len(), 2);
        let described = "You may call these tools.\n- weather: Current weather for a city. Arguments: object { city: text (city name) }";
        assert!(
            seen[0].instructions().unwrap().iter().any(|segment| *segment == text(described)),
            "the first request describes the tool",
        );
        assert_eq!(seen[1].structured().tool_outputs.len(), 1, "the second request carries the tool output");
    }

    #[test]
    fn a_failed_tool_goes_back_to_the_model_as_text_and_the_turn_goes_on() {
        let provider = Scripted::new(vec![Ok(weather_call()), Ok(Reply::Respond(vec![text("Sorry.")]))]);
        let mut s = session();
        s.prompt(vec![text("Weather?")]);
        let out = run_turn(&mut s, &tools(Err("station offline".into())), &provider, 1000, 0, 2);
        assert_eq!(out, Ok(vec![text("Sorry.")]));
        let outputs = Transcript::new(s.history().to_vec()).structured().tool_outputs.iter().map(|o| o.text.clone()).collect::<Vec<_>>();
        assert_eq!(outputs, vec!["error: tool `weather` failed: station offline".to_string()]);
    }

    #[test]
    fn tool_rounds_stop_at_the_limit_without_running_another_round() {
        let provider = Scripted::new(vec![Ok(weather_call()), Ok(weather_call()), Ok(weather_call())]);
        let mut s = session();
        s.prompt(vec![text("Again and again")]);
        let out = run_turn(&mut s, &tools(Ok("4 C".into())), &provider, 1000, 0, 1);
        assert_eq!(out, Err(TurnError::ToolRoundsExceeded { rounds: 1 }));
        assert_eq!(provider.seen.borrow().len(), 2, "one call and one round of output, then the limit");
        assert_eq!(Transcript::new(s.history().to_vec()).structured().tool_outputs.len(), 1);
    }

    #[test]
    fn descriptions_that_do_not_fit_the_budget_stop_the_turn_before_any_request() {
        let provider = Scripted::new(vec![Ok(Reply::Respond(vec![text("never")]))]);
        let mut s = session();
        s.prompt(vec![text("Hi")]);
        let out = run_turn(&mut s, &tools(Ok(String::new())), &provider, 5, 0, 2);
        assert_eq!(out, Err(TurnError::OverBudget));
        assert!(provider.seen.borrow().is_empty());
        assert_eq!(s.history().len(), 1, "the prompt stays and no answer is recorded");
    }

    #[test]
    fn a_provider_failure_is_reported_and_nothing_is_recorded() {
        let provider = Scripted::new(vec![Err(ProviderError::new("offline"))]);
        let mut s = session();
        s.prompt(vec![text("Hi")]);
        let out = run_turn(&mut s, &ToolSet::new(), &provider, 1000, 0, 2);
        assert_eq!(out, Err(TurnError::Provider(ProviderError::new("offline"))));
        assert_eq!(s.history().len(), 1);
        assert_eq!(out.unwrap_err().to_string(), "model error: offline");
    }

    #[test]
    fn a_call_that_names_no_tool_is_a_provider_error() {
        let provider = Scripted::new(vec![Ok(Reply::Call(vec![]))]);
        let mut s = session();
        s.prompt(vec![text("Hi")]);
        let out = run_turn(&mut s, &ToolSet::new(), &provider, 1000, 0, 2);
        assert_eq!(out, Err(TurnError::Provider(ProviderError::new("the reply asked for no tool"))));
    }
}
