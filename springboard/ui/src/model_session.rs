//! A model session with profiles, dynamic instructions and typed session properties.
//!
//! A [`Profile`] holds the instructions that define a model's behaviour. Those instructions are a list of
//! [`DynamicInstructions`] parts: fixed text, or a closure that runs before every request and sees the turn number and
//! the session's [`SessionProperties`]. A part that returns nothing is left out, so instructions can depend on app state
//! without a separate branch per case.
//!
//! A [`Session`] keeps the history (prompts and responses), the current profile and the property values. Switching the
//! profile keeps the history and changes the instructions of the next request. [`Session::prepare`] builds the transcript
//! for that request and trims old turns with [`crate::context_window::fit`], keeping the instructions.
//!
//! This is a pure model. It runs no model, makes no request and stores nothing on disk. Property keys are typed by the
//! variant of their default value, not by a Rust associated type, so a value of another kind is refused.

use crate::context_window::{fit, Fitted};
use crate::transcript::{Entry, Segment, Transcript};
use std::collections::BTreeMap;
use std::fmt;

/// A typed session property value.
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    Bool(bool),
    Int(i64),
    Text(String),
}

/// A named session property with the value it has until it is set.
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyKey {
    name: &'static str,
    default: PropertyValue,
}

impl PropertyKey {
    /// The key `name`, whose unset value is `default`. The kind of `default` fixes the kind of every value of the key.
    pub fn new(name: &'static str, default: PropertyValue) -> Self {
        PropertyKey { name, default }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }
}

/// Why a property could not be set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PropertyError {
    /// The value is of a different kind from the key's default value.
    TypeMismatch { key: &'static str },
}

impl fmt::Display for PropertyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PropertyError::TypeMismatch { key } => write!(f, "value for session property `{key}` has the wrong kind"),
        }
    }
}

/// The values of a session's properties. Keys that were never set read as their default.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionProperties {
    values: BTreeMap<&'static str, PropertyValue>,
}

impl SessionProperties {
    /// The value of `key`: the stored value, or the default when none was set.
    pub fn get(&self, key: &PropertyKey) -> PropertyValue {
        self.values.get(key.name).cloned().unwrap_or_else(|| key.default.clone())
    }

    /// Stores `value` for `key`. A value of another kind than the default is refused and nothing changes.
    pub fn set(&mut self, key: &PropertyKey, value: PropertyValue) -> Result<(), PropertyError> {
        if std::mem::discriminant(&value) != std::mem::discriminant(&key.default) {
            return Err(PropertyError::TypeMismatch { key: key.name });
        }
        self.values.insert(key.name, value);
        Ok(())
    }
}

/// What a dynamic part sees when it is evaluated for a request.
#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    /// The number of prompts in the history, so the first request sees 1.
    pub turn: usize,
    pub properties: &'a SessionProperties,
}

type PartFn = Box<dyn Fn(&Frame<'_>) -> Option<String>>;

/// One part of a profile's instructions.
enum Part {
    Fixed(String),
    Dynamic(PartFn),
}

/// The instructions of a profile: an ordered list of parts, evaluated before every request.
#[derive(Default)]
pub struct DynamicInstructions {
    parts: Vec<Part>,
}

impl fmt::Debug for DynamicInstructions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DynamicInstructions").field("parts", &self.parts.len()).finish()
    }
}

impl DynamicInstructions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds text that is always included.
    pub fn fixed(mut self, text: impl Into<String>) -> Self {
        self.parts.push(Part::Fixed(text.into()));
        self
    }

    /// Adds a part computed for each request. `None` leaves it out of that request.
    pub fn dynamic(mut self, part: impl Fn(&Frame<'_>) -> Option<String> + 'static) -> Self {
        self.parts.push(Part::Dynamic(Box::new(part)));
        self
    }

    /// The instructions text for one request: the non-empty parts in order, one per line.
    pub fn evaluate(&self, frame: &Frame<'_>) -> String {
        let mut lines = Vec::new();
        for part in &self.parts {
            let text = match part {
                Part::Fixed(text) => Some(text.clone()),
                Part::Dynamic(f) => f(frame),
            };
            if let Some(text) = text.filter(|t| !t.is_empty()) {
                lines.push(text);
            }
        }
        lines.join("\n")
    }
}

/// A named set of instructions a session can run under.
#[derive(Debug)]
pub struct Profile {
    name: String,
    instructions: DynamicInstructions,
}

impl Profile {
    pub fn new(name: impl Into<String>, instructions: DynamicInstructions) -> Self {
        Profile { name: name.into(), instructions }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

/// A conversation with a model under one profile.
#[derive(Debug)]
pub struct Session {
    profile: Profile,
    properties: SessionProperties,
    history: Vec<Entry>,
}

impl Session {
    /// A session with no history under `profile`.
    pub fn new(profile: Profile) -> Self {
        Session { profile, properties: SessionProperties::default(), history: Vec::new() }
    }

    /// A session that continues `history` under `profile`, for example one restored from storage.
    pub fn with_history(profile: Profile, history: Vec<Entry>) -> Self {
        Session { profile, properties: SessionProperties::default(), history }
    }

    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// Replaces the profile. The history stays, and the next request uses the new instructions.
    pub fn switch_profile(&mut self, profile: Profile) {
        self.profile = profile;
    }

    pub fn property(&self, key: &PropertyKey) -> PropertyValue {
        self.properties.get(key)
    }

    pub fn set_property(&mut self, key: &PropertyKey, value: PropertyValue) -> Result<(), PropertyError> {
        self.properties.set(key, value)
    }

    pub fn history(&self) -> &[Entry] {
        &self.history
    }

    /// Records a prompt.
    pub fn prompt(&mut self, segments: Vec<Segment>) {
        self.history.push(Entry::Prompt(segments));
    }

    /// Records a response.
    pub fn respond(&mut self, segments: Vec<Segment>) {
        self.history.push(Entry::Response(segments));
    }

    /// The instructions for the next request, evaluated now. Empty when every part left itself out.
    pub fn instructions(&self) -> String {
        let frame = Frame { turn: self.turn(), properties: &self.properties };
        self.profile.instructions.evaluate(&frame)
    }

    /// The transcript for the next request, with the current instructions first and the newest history that fits
    /// `budget` tokens after them. `reserve` tokens stay free for the response.
    pub fn prepare(&self, budget: usize, reserve: usize) -> Fitted {
        let mut entries = Vec::with_capacity(self.history.len() + 1);
        let text = self.instructions();
        if !text.is_empty() {
            entries.push(Entry::Instructions(vec![Segment::Text(text)]));
        }
        entries.extend(self.history.iter().cloned());
        fit(&Transcript::new(entries), budget, reserve)
    }

    fn turn(&self) -> usize {
        self.history.iter().filter(|e| matches!(e, Entry::Prompt(_))).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Segment {
        Segment::Text(s.into())
    }

    fn brief() -> PropertyKey {
        PropertyKey::new("brief", PropertyValue::Bool(false))
    }

    fn locale() -> PropertyKey {
        PropertyKey::new("locale", PropertyValue::Text("en".into()))
    }

    #[test]
    fn fixed_and_dynamic_parts_appear_in_order_and_empty_parts_are_left_out() {
        let instructions = DynamicInstructions::new()
            .fixed("You are a tide clerk.")
            .dynamic(|_| None)
            .dynamic(|f| Some(format!("This is turn {}.", f.turn)))
            .fixed("");
        let profile = Profile::new("tides", instructions);
        let mut session = Session::new(profile);
        session.prompt(vec![text("high tide?")]);
        assert_eq!(session.instructions(), "You are a tide clerk.\nThis is turn 1.");
    }

    #[test]
    fn dynamic_instructions_are_evaluated_before_each_request() {
        let profile = Profile::new(
            "count",
            DynamicInstructions::new().dynamic(|f| Some(format!("turn {}", f.turn))),
        );
        let mut session = Session::new(profile);
        session.prompt(vec![text("one")]);
        session.respond(vec![text("a")]);
        assert_eq!(session.instructions(), "turn 1");
        session.prompt(vec![text("two")]);
        assert_eq!(session.instructions(), "turn 2", "the second request sees the second turn");
    }

    #[test]
    fn a_part_reads_a_property_and_changes_when_the_property_is_set() {
        let brief = brief();
        let key = brief.clone();
        let profile = Profile::new(
            "answers",
            DynamicInstructions::new()
                .fixed("Answer.")
                .dynamic(move |f| match f.properties.get(&key) {
                    PropertyValue::Bool(true) => Some("Keep it to one sentence.".into()),
                    _ => None,
                }),
        );
        let mut session = Session::new(profile);
        assert_eq!(session.instructions(), "Answer.");
        session.set_property(&brief, PropertyValue::Bool(true)).unwrap();
        assert_eq!(session.instructions(), "Answer.\nKeep it to one sentence.");
    }

    #[test]
    fn a_property_reads_its_default_until_set_and_refuses_a_value_of_another_kind() {
        let locale = locale();
        let mut session = Session::new(Profile::new("empty", DynamicInstructions::new()));
        assert_eq!(session.property(&locale), PropertyValue::Text("en".into()));
        assert_eq!(
            session.set_property(&locale, PropertyValue::Int(3)),
            Err(PropertyError::TypeMismatch { key: "locale" }),
        );
        assert_eq!(session.property(&locale), PropertyValue::Text("en".into()), "a refused value changes nothing");
        session.set_property(&locale, PropertyValue::Text("is".into())).unwrap();
        assert_eq!(session.property(&locale), PropertyValue::Text("is".into()));
    }

    #[test]
    fn switching_the_profile_keeps_the_history_and_changes_the_next_instructions() {
        let mut session = Session::new(Profile::new("plain", DynamicInstructions::new().fixed("Plain.")));
        session.prompt(vec![text("hi")]);
        session.respond(vec![text("hello")]);
        session.switch_profile(Profile::new("formal", DynamicInstructions::new().fixed("Formal.")));
        assert_eq!(session.profile().name(), "formal");
        assert_eq!(session.history().len(), 2);
        assert_eq!(session.instructions(), "Formal.");
    }

    #[test]
    fn prepare_keeps_the_instructions_and_drops_old_turns_over_budget() {
        // Instructions of eight characters: two tokens. Each prompt and response is forty characters: ten tokens.
        let mut session = Session::new(Profile::new("short", DynamicInstructions::new().fixed("be brief")));
        let forty: String = "a".repeat(40);
        session.prompt(vec![text(&forty)]);
        session.respond(vec![text(&forty)]);
        session.prompt(vec![text(&forty)]);
        session.respond(vec![text(&forty)]);
        let fitted = session.prepare(22, 0);
        assert!(!fitted.over_budget);
        assert_eq!(fitted.dropped, 2, "the first prompt and response are older than the budget allows");
        assert!(fitted.transcript.instructions().is_some());
        assert_eq!(fitted.transcript.history().len(), 2);
        assert_eq!(fitted.tokens, 22);
    }

    #[test]
    fn prepare_adds_no_instructions_entry_when_every_part_is_empty() {
        let mut session = Session::new(Profile::new("none", DynamicInstructions::new().dynamic(|_| None)));
        session.prompt(vec![text("hi")]);
        let fitted = session.prepare(100, 0);
        assert!(fitted.transcript.instructions().is_none());
        assert_eq!(fitted.transcript.entries().len(), 1);
    }
}
