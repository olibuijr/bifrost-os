//! The value types under every model call: a model's identifier, what a model can do, the request an app sends and the
//! response it gets back, and a registry that picks a model for a request.
//!
//! - [`ModelId`] is `name` or `name:tag` (`"local-3b"`, `"local-3b:q4"`), lower-case ASCII, so ids compare and sort as text.
//! - [`ModelInfo`] states a model's context size and whether it takes tools and images.
//! - [`Request`] is a [`Transcript`] with [`GenerationOptions`] (the limit on the answer, the temperature, a seed, stop
//!   strings). [`Request::check`] says whether a model can take it before anything is sent.
//! - [`Response`] is the provider's [`Reply`] with the model that gave it, an estimated [`Usage`] and why it ended
//!   ([`FinishReason`]).
//! - [`ModelRegistry`] holds the models a machine offers and resolves a name to one of them.
//!
//! Pure: no network, no files. [`crate::model_provider::ModelProvider`] stays the one trait that reaches a model; these
//! are the types it and its callers share. Token counts are estimates (`context_window::estimate_tokens`), not what a
//! model's own tokenizer would count.

use crate::context_window::{entry_tokens, estimate_tokens};
use crate::model_attachment::Media;
use crate::model_provider::Reply;
use crate::transcript::{Entry, Segment, Transcript};
use std::fmt;

/// The longest model id, in bytes.
pub const MAX_ID_LEN: usize = 64;
/// The most stop strings one request may carry.
pub const MAX_STOPS: usize = 4;
/// The highest temperature a request may ask for.
pub const MAX_TEMPERATURE: f32 = 2.0;

/// Why a model id was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdError {
    Empty,
    TooLong,
    /// The id has a character outside `a-z 0-9 . _ -`, or a second `:`.
    BadCharacter(char),
    /// The name or the tag is empty (`":q4"`, `"x:"`).
    EmptyPart,
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdError::Empty => write!(f, "the model id is empty"),
            IdError::TooLong => write!(f, "the model id is longer than {MAX_ID_LEN} bytes"),
            IdError::BadCharacter(c) => write!(f, "the model id has a character it may not: {c:?}"),
            IdError::EmptyPart => write!(f, "the model id has an empty name or tag"),
        }
    }
}

/// A model's identifier: `name` or `name:tag`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModelId {
    name: String,
    tag: Option<String>,
}

impl ModelId {
    /// Reads an id. Upper case is folded to lower case, surrounding blanks are dropped, and anything outside
    /// `a-z 0-9 . _ -` (and one `:` between name and tag) is refused.
    pub fn parse(text: &str) -> Result<ModelId, IdError> {
        let text = text.trim().to_ascii_lowercase();
        if text.is_empty() {
            return Err(IdError::Empty);
        }
        if text.len() > MAX_ID_LEN {
            return Err(IdError::TooLong);
        }
        let part_ok = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-');
        let (name, tag) = match text.split_once(':') {
            Some((n, t)) => (n, Some(t)),
            None => (text.as_str(), None),
        };
        for c in name.chars().chain(tag.unwrap_or("").chars()) {
            if !part_ok(c) {
                return Err(IdError::BadCharacter(c));
            }
        }
        if name.is_empty() || tag.is_some_and(str::is_empty) {
            return Err(IdError::EmptyPart);
        }
        Ok(ModelId { name: name.to_string(), tag: tag.map(str::to_string) })
    }

    /// The name, without the tag: the model family.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn tag(&self) -> Option<&str> {
        self.tag.as_deref()
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.tag {
            Some(tag) => write!(f, "{}:{tag}", self.name),
            None => write!(f, "{}", self.name),
        }
    }
}

/// What one model offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: ModelId,
    /// The most tokens a request and its answer may hold together.
    pub context_tokens: usize,
    /// The model can ask for tools.
    pub tools: bool,
    /// The image formats it accepts; empty for a text-only model.
    pub media: Vec<Media>,
    /// The model runs on this machine. A model that sends the conversation elsewhere says `false`.
    pub on_device: bool,
}

impl ModelInfo {
    pub fn new(id: ModelId, context_tokens: usize) -> ModelInfo {
        ModelInfo { id, context_tokens, tools: false, media: Vec::new(), on_device: true }
    }

    pub fn with_tools(mut self) -> ModelInfo {
        self.tools = true;
        self
    }

    pub fn with_media(mut self, media: &[Media]) -> ModelInfo {
        self.media = media.to_vec();
        self
    }

    pub fn remote(mut self) -> ModelInfo {
        self.on_device = false;
        self
    }
}

/// How the answer is produced.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GenerationOptions {
    /// The most tokens in the answer. `None` leaves it to the model.
    pub max_tokens: Option<usize>,
    /// 0 is the most deterministic. `None` leaves it to the model.
    pub temperature: Option<f32>,
    pub seed: Option<u64>,
    /// The answer ends before any of these.
    pub stop: Vec<String>,
}

/// Why a request cannot be sent to a model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestError {
    ZeroMaxTokens,
    /// The temperature is not in `0..=MAX_TEMPERATURE` (NaN counts as outside).
    BadTemperature,
    TooManyStops,
    EmptyStop,
    /// The request and the answer it leaves room for need `needed` tokens; the model holds `available`.
    ContextTooSmall { needed: usize, available: usize },
    /// The transcript carries an attachment and the model takes no images.
    MediaUnsupported,
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RequestError::ZeroMaxTokens => write!(f, "the answer limit is zero tokens"),
            RequestError::BadTemperature => write!(f, "the temperature must be between 0 and {MAX_TEMPERATURE}"),
            RequestError::TooManyStops => write!(f, "a request takes at most {MAX_STOPS} stop strings"),
            RequestError::EmptyStop => write!(f, "a stop string is empty"),
            RequestError::ContextTooSmall { needed, available } => write!(f, "the request needs {needed} tokens and the model holds {available}"),
            RequestError::MediaUnsupported => write!(f, "the model takes no attachments"),
        }
    }
}

/// One call to a model.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    /// The model asked for. `None` takes the registry's default.
    pub model: Option<ModelId>,
    pub transcript: Transcript,
    pub options: GenerationOptions,
}

/// The tokens a request with no `max_tokens` leaves room for.
pub const DEFAULT_ANSWER_TOKENS: usize = 512;

impl Request {
    pub fn new(transcript: Transcript) -> Request {
        Request { model: None, transcript, options: GenerationOptions::default() }
    }

    /// The estimated tokens of the transcript.
    pub fn prompt_tokens(&self) -> usize {
        self.transcript.entries().iter().map(entry_tokens).sum()
    }

    fn has_attachment(&self) -> bool {
        self.transcript.entries().iter().any(|e| match e {
            Entry::Instructions(s) | Entry::Prompt(s) | Entry::Response(s) => s.iter().any(|seg| matches!(seg, Segment::Attachment { .. })),
            _ => false,
        })
    }

    /// Checks the options alone, without a model.
    pub fn validate(&self) -> Result<(), RequestError> {
        let o = &self.options;
        if o.max_tokens == Some(0) {
            return Err(RequestError::ZeroMaxTokens);
        }
        if o.temperature.is_some_and(|t| !(0.0..=MAX_TEMPERATURE).contains(&t)) {
            return Err(RequestError::BadTemperature);
        }
        if o.stop.len() > MAX_STOPS {
            return Err(RequestError::TooManyStops);
        }
        if o.stop.iter().any(String::is_empty) {
            return Err(RequestError::EmptyStop);
        }
        Ok(())
    }

    /// Checks the request against `model`: the options, the context (the transcript plus the answer it leaves room for)
    /// and attachments.
    pub fn check(&self, model: &ModelInfo) -> Result<(), RequestError> {
        self.validate()?;
        if self.has_attachment() && model.media.is_empty() {
            return Err(RequestError::MediaUnsupported);
        }
        let needed = self.prompt_tokens() + self.options.max_tokens.unwrap_or(DEFAULT_ANSWER_TOKENS);
        if needed > model.context_tokens {
            return Err(RequestError::ContextTooSmall { needed, available: model.context_tokens });
        }
        Ok(())
    }
}

/// Why an answer ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinishReason {
    /// The model finished its answer.
    Stop,
    /// The answer reached the request's `max_tokens`.
    Length,
    /// The model stopped to ask for tools.
    ToolCalls,
}

/// Estimated token use of one call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
}

impl Usage {
    pub fn total(&self) -> usize {
        self.prompt_tokens + self.completion_tokens
    }
}

/// A provider's answer with the model that gave it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub model: ModelId,
    pub reply: Reply,
    pub usage: Usage,
    pub finish: FinishReason,
}

impl Response {
    /// Wraps a provider's reply to `request`: the usage is estimated from the transcript and the reply, and the finish
    /// reason is `ToolCalls` for a call, `Length` when the answer reached `max_tokens`, else `Stop`.
    pub fn from_reply(model: ModelId, request: &Request, reply: Reply) -> Response {
        let completion_tokens = match &reply {
            Reply::Respond(segments) => segments
                .iter()
                .map(|s| match s {
                    Segment::Text(t) | Segment::Structured(t) => estimate_tokens(t),
                    Segment::Attachment { name, .. } => estimate_tokens(name),
                })
                .sum(),
            Reply::Call(calls) => calls.iter().map(|c| estimate_tokens(&c.name) + estimate_tokens(&c.arguments)).sum(),
        };
        let finish = match &reply {
            Reply::Call(_) => FinishReason::ToolCalls,
            Reply::Respond(_) if request.options.max_tokens.is_some_and(|m| completion_tokens >= m) => FinishReason::Length,
            Reply::Respond(_) => FinishReason::Stop,
        };
        Response { model, reply, usage: Usage { prompt_tokens: request.prompt_tokens(), completion_tokens }, finish }
    }
}

/// Why a model could not be registered or found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryError {
    /// A model with this id is registered already.
    Duplicate(ModelId),
    /// No model matches the name.
    Unknown(String),
    /// The name matches more than one model and none is its exact id. The ids are listed.
    Ambiguous(Vec<ModelId>),
    /// The name is not a model id.
    BadId(IdError),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::Duplicate(id) => write!(f, "model {id} is registered already"),
            RegistryError::Unknown(name) => write!(f, "no model is called {name}"),
            RegistryError::Ambiguous(ids) => write!(f, "the name matches {} models; say which: {}", ids.len(), ids.iter().map(ModelId::to_string).collect::<Vec<_>>().join(", ")),
            RegistryError::BadId(e) => write!(f, "{e}"),
        }
    }
}

/// The models a machine offers, in the order they were registered. The first one registered is the default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelRegistry {
    models: Vec<ModelInfo>,
}

impl ModelRegistry {
    pub fn new() -> ModelRegistry {
        ModelRegistry::default()
    }

    pub fn register(&mut self, info: ModelInfo) -> Result<(), RegistryError> {
        if self.models.iter().any(|m| m.id == info.id) {
            return Err(RegistryError::Duplicate(info.id));
        }
        self.models.push(info);
        Ok(())
    }

    pub fn models(&self) -> &[ModelInfo] {
        &self.models
    }

    pub fn default_model(&self) -> Option<&ModelInfo> {
        self.models.first()
    }

    /// The model a name refers to: an exact id (`"local:q4"`), or a bare name (`"local"`) that stands for the only model
    /// of that name, or for the one with no tag when there are several tags.
    pub fn resolve(&self, name: &str) -> Result<&ModelInfo, RegistryError> {
        let id = ModelId::parse(name).map_err(RegistryError::BadId)?;
        if let Some(exact) = self.models.iter().find(|m| m.id == id) {
            return Ok(exact);
        }
        if id.tag().is_some() {
            return Err(RegistryError::Unknown(id.to_string()));
        }
        let family: Vec<&ModelInfo> = self.models.iter().filter(|m| m.id.name() == id.name()).collect();
        match family.as_slice() {
            [] => Err(RegistryError::Unknown(id.to_string())),
            [only] => Ok(only),
            many => Err(RegistryError::Ambiguous(many.iter().map(|m| m.id.clone()).collect())),
        }
    }

    /// The model a request is for: the one it names, or the default.
    pub fn for_request(&self, request: &Request) -> Result<&ModelInfo, RegistryError> {
        match &request.model {
            Some(id) => self.resolve(&id.to_string()),
            None => self.default_model().ok_or_else(|| RegistryError::Unknown("(no model is registered)".into())),
        }
    }

    /// The models that take tools, and images when `images` is set, that stay on the machine when `on_device` is set.
    pub fn matching(&self, tools: bool, images: bool, on_device: bool) -> Vec<&ModelInfo> {
        self.models.iter().filter(|m| (!tools || m.tools) && (!images || !m.media.is_empty()) && (!on_device || m.on_device)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::ToolCall;

    fn id(text: &str) -> ModelId {
        ModelId::parse(text).unwrap()
    }

    fn prompt(text: &str) -> Request {
        Request::new(Transcript::new(vec![Entry::Prompt(vec![Segment::Text(text.into())])]))
    }

    #[test]
    fn ids_fold_to_lower_case_and_round_trip_through_display() {
        let a = id("  Local-3B:Q4 ");
        assert_eq!((a.name(), a.tag()), ("local-3b", Some("q4")));
        assert_eq!(a.to_string(), "local-3b:q4");
        assert_eq!(id("local").to_string(), "local");
        assert_eq!(ModelId::parse(&a.to_string()), Ok(a));
    }

    #[test]
    fn bad_ids_are_refused_with_the_reason() {
        assert_eq!(ModelId::parse(" "), Err(IdError::Empty));
        assert_eq!(ModelId::parse("a b"), Err(IdError::BadCharacter(' ')));
        assert_eq!(ModelId::parse("a/b"), Err(IdError::BadCharacter('/')));
        assert_eq!(ModelId::parse("a:b:c"), Err(IdError::BadCharacter(':')));
        assert_eq!(ModelId::parse(":q4"), Err(IdError::EmptyPart));
        assert_eq!(ModelId::parse("x:"), Err(IdError::EmptyPart));
        assert_eq!(ModelId::parse(&"a".repeat(MAX_ID_LEN + 1)), Err(IdError::TooLong));
        assert!(ModelId::parse(&"a".repeat(MAX_ID_LEN)).is_ok());
    }

    #[test]
    fn options_are_validated_without_a_model() {
        let with = |f: &dyn Fn(&mut GenerationOptions)| {
            let mut r = prompt("hi");
            f(&mut r.options);
            r.validate()
        };
        assert_eq!(with(&|_| {}), Ok(()));
        assert_eq!(with(&|o| o.max_tokens = Some(0)), Err(RequestError::ZeroMaxTokens));
        assert_eq!(with(&|o| o.temperature = Some(2.5)), Err(RequestError::BadTemperature));
        assert_eq!(with(&|o| o.temperature = Some(-0.1)), Err(RequestError::BadTemperature));
        assert_eq!(with(&|o| o.temperature = Some(f32::NAN)), Err(RequestError::BadTemperature));
        assert_eq!(with(&|o| o.temperature = Some(2.0)), Ok(()));
        assert_eq!(with(&|o| o.stop = vec!["a".into(); MAX_STOPS + 1]), Err(RequestError::TooManyStops));
        assert_eq!(with(&|o| o.stop = vec!["".into()]), Err(RequestError::EmptyStop));
    }

    #[test]
    fn a_request_must_fit_the_model_context_with_room_for_the_answer() {
        let text = "x".repeat(400);
        let mut request = prompt(&text);
        let prompt_tokens = request.prompt_tokens();
        assert!(prompt_tokens > 0);
        request.options.max_tokens = Some(100);
        let exact = ModelInfo::new(id("m"), prompt_tokens + 100);
        assert_eq!(request.check(&exact), Ok(()));
        let small = ModelInfo::new(id("m"), prompt_tokens + 99);
        assert_eq!(request.check(&small), Err(RequestError::ContextTooSmall { needed: prompt_tokens + 100, available: prompt_tokens + 99 }));
        // with no limit the default answer size is reserved
        request.options.max_tokens = None;
        assert_eq!(request.check(&exact), Err(RequestError::ContextTooSmall { needed: prompt_tokens + DEFAULT_ANSWER_TOKENS, available: prompt_tokens + 100 }));
    }

    #[test]
    fn an_attachment_needs_a_model_that_takes_images() {
        let request = Request::new(Transcript::new(vec![Entry::Prompt(vec![Segment::Text("look".into()), Segment::Attachment { name: "a.png".into(), bytes: 10 }])]));
        let text_only = ModelInfo::new(id("t"), 100_000);
        assert_eq!(request.check(&text_only), Err(RequestError::MediaUnsupported));
        let vision = ModelInfo::new(id("v"), 100_000).with_media(&[Media::Png]);
        assert_eq!(request.check(&vision), Ok(()));
    }

    #[test]
    fn a_response_estimates_usage_and_says_why_it_ended() {
        let mut request = prompt("Hello there");
        let answer = Reply::Respond(vec![Segment::Text("y".repeat(40))]);
        let done = Response::from_reply(id("m"), &request, answer.clone());
        assert_eq!(done.finish, FinishReason::Stop);
        assert_eq!(done.usage.prompt_tokens, request.prompt_tokens());
        assert_eq!(done.usage.completion_tokens, estimate_tokens(&"y".repeat(40)));
        assert_eq!(done.usage.total(), done.usage.prompt_tokens + done.usage.completion_tokens);
        request.options.max_tokens = Some(done.usage.completion_tokens);
        assert_eq!(Response::from_reply(id("m"), &request, answer.clone()).finish, FinishReason::Length);
        request.options.max_tokens = Some(done.usage.completion_tokens + 1);
        assert_eq!(Response::from_reply(id("m"), &request, answer).finish, FinishReason::Stop);
        let call = Reply::Call(vec![ToolCall { name: "weather".into(), arguments: "{}".into() }]);
        let asked = Response::from_reply(id("m"), &request, call);
        assert_eq!(asked.finish, FinishReason::ToolCalls);
        assert!(asked.usage.completion_tokens > 0);
    }

    fn registry() -> ModelRegistry {
        let mut r = ModelRegistry::new();
        r.register(ModelInfo::new(id("local:q4"), 8192).with_tools()).unwrap();
        r.register(ModelInfo::new(id("local:q8"), 8192)).unwrap();
        r.register(ModelInfo::new(id("vision"), 4096).with_media(&[Media::Png, Media::Jpeg])).unwrap();
        r.register(ModelInfo::new(id("hosted"), 32768).with_tools().remote()).unwrap();
        r
    }

    #[test]
    fn registering_the_same_id_twice_is_refused_and_the_first_is_the_default() {
        let mut r = registry();
        assert_eq!(r.register(ModelInfo::new(id("vision"), 1)), Err(RegistryError::Duplicate(id("vision"))));
        assert_eq!(r.models().len(), 4);
        assert_eq!(r.default_model().map(|m| m.id.to_string()).as_deref(), Some("local:q4"));
        assert!(ModelRegistry::new().default_model().is_none());
    }

    #[test]
    fn a_name_resolves_to_an_exact_id_or_to_the_only_model_of_that_family() {
        let r = registry();
        assert_eq!(r.resolve("LOCAL:q8").unwrap().id, id("local:q8"));
        assert_eq!(r.resolve("vision").unwrap().context_tokens, 4096);
        assert_eq!(r.resolve("local"), Err(RegistryError::Ambiguous(vec![id("local:q4"), id("local:q8")])));
        assert_eq!(r.resolve("local:q2"), Err(RegistryError::Unknown("local:q2".into())));
        assert_eq!(r.resolve("nothing"), Err(RegistryError::Unknown("nothing".into())));
        assert!(matches!(r.resolve("bad name"), Err(RegistryError::BadId(IdError::BadCharacter(' ')))));
        let mut one_tag = ModelRegistry::new();
        one_tag.register(ModelInfo::new(id("solo:q4"), 10)).unwrap();
        assert_eq!(one_tag.resolve("solo").unwrap().id, id("solo:q4"));
    }

    #[test]
    fn a_request_without_a_model_takes_the_default_and_one_with_a_model_gets_that_one() {
        let r = registry();
        let mut request = prompt("hi");
        assert_eq!(r.for_request(&request).unwrap().id, id("local:q4"));
        request.model = Some(id("hosted"));
        assert_eq!(r.for_request(&request).unwrap().id, id("hosted"));
        request.model = Some(id("gone"));
        assert!(r.for_request(&request).is_err());
        assert!(ModelRegistry::new().for_request(&prompt("hi")).is_err());
    }

    #[test]
    fn models_are_filtered_by_what_they_can_do_and_where_they_run() {
        let r = registry();
        let names = |v: Vec<&ModelInfo>| v.iter().map(|m| m.id.to_string()).collect::<Vec<_>>();
        assert_eq!(names(r.matching(false, false, false)).len(), 4);
        assert_eq!(names(r.matching(true, false, false)), ["local:q4", "hosted"]);
        assert_eq!(names(r.matching(true, false, true)), ["local:q4"], "a remote model is left out when it must stay on the device");
        assert_eq!(names(r.matching(false, true, false)), ["vision"]);
        assert!(r.matching(true, true, false).is_empty());
    }
}
