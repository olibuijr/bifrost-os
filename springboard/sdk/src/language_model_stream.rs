//! Streamed answers and the availability text for views that show a language model session (the part of
//! `LanguageModelSession` a window or a SwiftUI-style view binds to: partial text while the model answers, a way to
//! stop it, and a plain sentence when no model can run).
//!
//! - [`StreamingModelProvider`] is a [`ModelProvider`] that can hand over its answer in pieces. A provider that cannot
//!   keeps the default, which delivers the whole answer as one piece, so every provider works with [`run_turn_streamed`].
//! - [`run_turn_streamed`] runs a turn like `run_turn` (fitted history, tool rounds, the answer recorded in the
//!   session) and calls the view's callback with the text so far after every piece. The callback returns `false` to
//!   stop; a stopped turn records nothing in the session, so the history never holds half an answer.
//! - [`StreamedResponse`] is the view state: the text so far, how many pieces arrived and whether it is still going.
//! - [`availability_text`] is the sentence a view shows for [`Availability`].
//!
//! Nothing here reaches a model or the network; the provider does.

use crate::language_model::{Availability, Unavailable};
use bifrost_ui::model_provider::{with_descriptions, ModelProvider, ProviderError, Reply, TurnError};
use bifrost_ui::model_session::Session;
use bifrost_ui::model_tool::ToolSet;
use bifrost_ui::context_window::estimate_tokens;
use bifrost_ui::transcript::{Entry, Segment, Transcript};

/// A provider that can deliver its answer in pieces.
pub trait StreamingModelProvider: ModelProvider {
    /// Answers one request, calling `piece` with each new piece of answer text as it arrives. `piece` returns `false`
    /// to ask the provider to stop; the provider then returns the reply it has so far. The default calls
    /// [`ModelProvider::generate`] and passes each text segment of a `Respond` reply as one piece.
    fn generate_stream(&self, request: &Transcript, piece: &mut dyn FnMut(&str) -> bool) -> Result<Reply, ProviderError> {
        let reply = self.generate(request)?;
        if let Reply::Respond(segments) = &reply {
            for s in segments {
                if let Segment::Text(t) = s {
                    if !piece(t) {
                        break;
                    }
                }
            }
        }
        Ok(reply)
    }
}

/// How a streamed turn ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamOutcome {
    /// The answer, recorded in the session.
    Finished(Vec<Segment>),
    /// The callback stopped the turn. The text so far is returned and nothing was recorded.
    Stopped(String),
}

/// The state a view binds to while an answer streams in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamedResponse {
    text: String,
    pieces: usize,
    done: bool,
}

impl StreamedResponse {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a piece of answer text. A piece after the end is ignored.
    pub fn push(&mut self, piece: &str) {
        if !self.done && !piece.is_empty() {
            self.text.push_str(piece);
            self.pieces += 1;
        }
    }

    /// Ends the stream. The text stays.
    pub fn finish(&mut self) {
        self.done = true;
    }

    /// The answer text so far.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// How many pieces have arrived.
    pub fn pieces(&self) -> usize {
        self.pieces
    }

    /// True until [`StreamedResponse::finish`]; a view shows its progress indicator while this holds.
    pub fn is_streaming(&self) -> bool {
        !self.done
    }
}

/// Runs one turn of `session` against a streaming provider. `on_text` gets the text so far after every piece and
/// returns `false` to stop the turn. Budgets and tool rounds work as in `run_turn`. A turn whose model wants tools runs
/// them, records their output and asks again; only the pieces of the final answer reach `on_text`.
pub fn run_turn_streamed(
    session: &mut Session,
    tools: &ToolSet,
    provider: &dyn StreamingModelProvider,
    budget: usize,
    reserve: usize,
    max_tool_rounds: usize,
    on_text: &mut dyn FnMut(&str) -> bool,
) -> Result<StreamOutcome, TurnError> {
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
        let mut so_far = String::new();
        let mut stopped = false;
        let reply = provider
            .generate_stream(&request, &mut |piece| {
                so_far.push_str(piece);
                if !on_text(&so_far) {
                    stopped = true;
                }
                !stopped
            })
            .map_err(TurnError::Provider)?;
        if stopped {
            return Ok(StreamOutcome::Stopped(so_far));
        }
        match reply {
            Reply::Respond(segments) => {
                session.respond(segments.clone());
                return Ok(StreamOutcome::Finished(segments));
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

/// The sentence a view shows for the model's availability. Empty when it is available.
pub fn availability_text(availability: Availability) -> &'static str {
    match availability {
        Availability::Available => "",
        Availability::Unavailable(Unavailable::NoOnDeviceModel) => "No language model is installed on this computer.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::model_session::{DynamicInstructions, Profile};
    use std::cell::RefCell;

    /// Streams its answer in three pieces; a `Call` script is used first when given.
    struct Pieces {
        script: RefCell<Vec<Reply>>,
        pieces: Vec<&'static str>,
    }

    impl ModelProvider for Pieces {
        fn name(&self) -> &str {
            "pieces"
        }
        fn generate(&self, _request: &Transcript) -> Result<Reply, ProviderError> {
            Ok(Reply::Respond(vec![Segment::Text(self.pieces.concat())]))
        }
    }

    impl StreamingModelProvider for Pieces {
        fn generate_stream(&self, request: &Transcript, piece: &mut dyn FnMut(&str) -> bool) -> Result<Reply, ProviderError> {
            if !self.script.borrow().is_empty() {
                return Ok(self.script.borrow_mut().remove(0));
            }
            for p in &self.pieces {
                if !piece(p) {
                    break;
                }
            }
            self.generate(request)
        }
    }

    /// Does not stream: the default `generate_stream` delivers the whole answer at once.
    struct Whole;

    impl ModelProvider for Whole {
        fn name(&self) -> &str {
            "whole"
        }
        fn generate(&self, _request: &Transcript) -> Result<Reply, ProviderError> {
            Ok(Reply::Respond(vec![Segment::Text("Hello there".into())]))
        }
    }

    impl StreamingModelProvider for Whole {}

    fn session() -> Session {
        let mut s = Session::new(Profile::new("t", DynamicInstructions::new().fixed("Be brief.")));
        s.prompt(vec![Segment::Text("Hi".into())]);
        s
    }

    fn streamer(script: Vec<Reply>) -> Pieces {
        Pieces { script: RefCell::new(script), pieces: vec!["Hel", "lo ", "world"] }
    }

    #[test]
    fn the_view_sees_the_text_grow_and_the_answer_is_recorded() {
        let mut s = session();
        let mut seen = Vec::new();
        let out = run_turn_streamed(&mut s, &ToolSet::default(), &streamer(vec![]), 4000, 200, 3, &mut |t| {
            seen.push(t.to_string());
            true
        })
        .unwrap();
        assert_eq!(seen, ["Hel", "Hello ", "Hello world"]);
        assert_eq!(out, StreamOutcome::Finished(vec![Segment::Text("Hello world".into())]));
        assert!(matches!(s.history().last(), Some(Entry::Response(_))));
    }

    #[test]
    fn stopping_keeps_the_partial_text_out_of_the_history() {
        let mut s = session();
        let before = s.history().len();
        let out = run_turn_streamed(&mut s, &ToolSet::default(), &streamer(vec![]), 4000, 200, 3, &mut |t| t.len() < 5).unwrap();
        assert_eq!(out, StreamOutcome::Stopped("Hello ".into()));
        assert_eq!(s.history().len(), before, "nothing recorded");
    }

    #[test]
    fn a_provider_that_cannot_stream_delivers_one_piece() {
        let mut s = session();
        let mut seen = 0;
        let out = run_turn_streamed(&mut s, &ToolSet::default(), &Whole, 4000, 200, 3, &mut |t| {
            seen += 1;
            assert_eq!(t, "Hello there");
            true
        })
        .unwrap();
        assert_eq!(seen, 1);
        assert_eq!(out, StreamOutcome::Finished(vec![Segment::Text("Hello there".into())]));
    }

    #[test]
    fn an_empty_tool_call_reply_and_a_tiny_budget_are_refused() {
        let mut s = session();
        let err = run_turn_streamed(&mut s, &ToolSet::default(), &streamer(vec![Reply::Call(vec![])]), 4000, 200, 3, &mut |_| true).unwrap_err();
        assert!(matches!(err, TurnError::Provider(_)));
        let err = run_turn_streamed(&mut s, &ToolSet::default(), &Whole, 1, 1, 3, &mut |_| true).unwrap_err();
        assert_eq!(err, TurnError::OverBudget);
    }

    #[test]
    fn the_view_state_collects_pieces_until_it_is_finished() {
        let mut r = StreamedResponse::new();
        assert!(r.is_streaming());
        r.push("Hel");
        r.push("");
        r.push("lo");
        assert_eq!((r.text(), r.pieces()), ("Hello", 2));
        r.finish();
        r.push("late");
        assert_eq!(r.text(), "Hello");
        assert!(!r.is_streaming());
    }

    #[test]
    fn the_availability_sentence_is_empty_only_when_the_model_is_available() {
        assert_eq!(availability_text(Availability::Available), "");
        assert!(availability_text(crate::language_model::on_device_availability()).contains("No language model"));
    }
}
