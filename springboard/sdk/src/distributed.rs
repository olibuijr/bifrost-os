//! Distributed parity: remote call targets, typed invocation encoding and decoding, the result handler protocol,
//! a local testing actor system, and the errors a call can end with.
//!
//! BifrOSt apps are not location-transparent: there is no remote actor reference and no actor system that
//! carries calls between processes. What this module provides is the call side of the model. An [`Invocation`]
//! names a target and carries labelled arguments encoded with the [`coding`](crate::coding) traits.
//! [`LocalTestingSystem`] runs invocations in one process, so a handler can be driven the way a real system
//! would deliver it. [`DistributedError`] maps onto the control socket's JSON-RPC error codes.
//!
//! ```ignore
//! let mut sys = LocalTestingSystem::new();
//! let actor = sys.make_actor_id();
//! sys.register(&actor, "Counter.add(by:)", |d| Ok(Value::Int(d.next::<i64>(Some("by"))? + 1))).unwrap();
//! let mut enc = InvocationEncoder::new();
//! enc.record_argument(Some("by"), &41i64).unwrap();
//! let call = enc.done_recording("Counter.add(by:)").unwrap();
//! let mut out = CollectingResultHandler::default();
//! sys.execute(&actor, &call, &mut out);
//! assert_eq!(out.value, Some(Value::Int(42)));
//! ```

use crate::coding::{CodingError, Decodable, Encodable, Value};
use crate::control::{RpcError, INTERNAL_ERROR, INVALID_PARAMS, METHOD_NOT_FOUND};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

/// Longest target identifier or argument label, in bytes.
pub const MAX_NAME_LEN: usize = 256;

/// A name is non-empty, short enough, and free of whitespace and control characters.
fn valid_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_NAME_LEN && !s.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Error thrown while an invocation cannot be encoded or decoded (a bad name, a missing argument, a label or
/// type mismatch, or a leftover argument).
#[derive(Clone, Debug, PartialEq)]
pub struct DistributedActorCodingError(pub String);

impl fmt::Display for DistributedActorCodingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for DistributedActorCodingError {}

/// The errors a call can end with: the actor or the target is unknown, the arguments do not decode, or the
/// handler failed. This one type covers the framework's protocol errors and the execution error.
#[derive(Clone, Debug, PartialEq)]
pub enum DistributedError {
    /// The actor id names no actor of this system.
    UnknownActor(String),
    /// The actor has no handler registered for the target.
    UnknownTarget(String),
    /// The arguments did not match what the handler expects.
    Coding(DistributedActorCodingError),
    /// The handler ran and failed with its own message.
    Handler(String),
}

impl fmt::Display for DistributedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DistributedError::UnknownActor(a) => write!(f, "unknown actor {a}"),
            DistributedError::UnknownTarget(t) => write!(f, "unknown target {t}"),
            DistributedError::Coding(e) => write!(f, "{e}"),
            DistributedError::Handler(m) => f.write_str(m),
        }
    }
}
impl std::error::Error for DistributedError {}

impl From<DistributedActorCodingError> for DistributedError {
    fn from(e: DistributedActorCodingError) -> Self {
        DistributedError::Coding(e)
    }
}

/// Control socket replies: a bad argument is INVALID_PARAMS, an unknown target METHOD_NOT_FOUND, and a handler
/// failure INTERNAL_ERROR. An unknown actor is also INVALID_PARAMS, because the caller named it.
impl From<DistributedError> for RpcError {
    fn from(e: DistributedError) -> RpcError {
        match e {
            DistributedError::UnknownActor(a) => RpcError::new(INVALID_PARAMS, format!("unknown actor {a}")),
            DistributedError::UnknownTarget(t) => RpcError::new(METHOD_NOT_FOUND, format!("unknown target {t}")),
            DistributedError::Coding(c) => RpcError::new(INVALID_PARAMS, c.0),
            DistributedError::Handler(m) => RpcError::new(INTERNAL_ERROR, m),
        }
    }
}

/// The target of a call, such as `Counter.add(by:)`: a validated identifier.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RemoteCallTarget(String);

impl RemoteCallTarget {
    /// `None` when the identifier is empty, too long, or contains whitespace or control characters.
    pub fn new(identifier: &str) -> Option<Self> {
        valid_name(identifier).then(|| RemoteCallTarget(identifier.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One argument of an invocation: its external label (if it has one) and its encoded value.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteCallArgument {
    pub label: Option<String>,
    pub value: Value,
}

/// Records the arguments of a call in order, encoding each with its [`Encodable`] value.
#[derive(Debug, Default)]
pub struct InvocationEncoder {
    arguments: Vec<RemoteCallArgument>,
}

impl InvocationEncoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the next argument. `label` is its external name when it has one; an empty or malformed label is
    /// refused.
    pub fn record_argument<T: Encodable>(&mut self, label: Option<&str>, value: &T) -> Result<(), DistributedActorCodingError> {
        if let Some(l) = label {
            if !valid_name(l) {
                return Err(DistributedActorCodingError(format!("bad argument label {l:?}")));
            }
        }
        self.arguments.push(RemoteCallArgument { label: label.map(str::to_string), value: value.encode() });
        Ok(())
    }

    /// Ends recording and names the target the invocation is for.
    pub fn done_recording(self, target: &str) -> Result<Invocation, DistributedActorCodingError> {
        let target = RemoteCallTarget::new(target).ok_or_else(|| DistributedActorCodingError(format!("bad target identifier {target:?}")))?;
        Ok(Invocation { target, arguments: self.arguments })
    }
}

/// A finished call: a target and its encoded arguments in the order they were recorded.
#[derive(Clone, Debug, PartialEq)]
pub struct Invocation {
    target: RemoteCallTarget,
    arguments: Vec<RemoteCallArgument>,
}

impl Invocation {
    pub fn target(&self) -> &RemoteCallTarget {
        &self.target
    }

    pub fn arguments(&self) -> &[RemoteCallArgument] {
        &self.arguments
    }
}

/// Wire shape: `{"target": "...", "arguments": [{"label": "by" | null, "value": ...}]}`.
impl Encodable for Invocation {
    fn encode(&self) -> Value {
        let arguments: Vec<Value> = self
            .arguments
            .iter()
            .map(|a| {
                Value::Dict(vec![
                    ("label".to_string(), a.label.as_ref().map_or(Value::Null, |l| Value::Str(l.clone()))),
                    ("value".to_string(), a.value.clone()),
                ])
            })
            .collect();
        Value::Dict(vec![
            ("target".to_string(), Value::Str(self.target.0.clone())),
            ("arguments".to_string(), Value::Arr(arguments)),
        ])
    }
}

impl Decodable for Invocation {
    fn decode(v: &Value) -> Result<Self, CodingError> {
        let Value::Dict(fields) = v else { return Err(CodingError("expected an invocation object".into())) };
        let get = |k: &str| fields.iter().find(|(n, _)| n == k).map(|(_, v)| v);
        let Some(Value::Str(t)) = get("target") else { return Err(CodingError("missing target".into())) };
        let target = RemoteCallTarget::new(t).ok_or_else(|| CodingError(format!("bad target identifier {t:?}")))?;
        let Some(Value::Arr(items)) = get("arguments") else { return Err(CodingError("missing arguments".into())) };
        let mut arguments = Vec::with_capacity(items.len());
        for item in items {
            let Value::Dict(f) = item else { return Err(CodingError("expected an argument object".into())) };
            let g = |k: &str| f.iter().find(|(n, _)| n == k).map(|(_, v)| v);
            let label = match g("label") {
                None | Some(Value::Null) => None,
                Some(Value::Str(s)) => Some(s.clone()),
                Some(_) => return Err(CodingError("argument label must be a string".into())),
            };
            let value = g("value").cloned().unwrap_or(Value::Null);
            arguments.push(RemoteCallArgument { label, value });
        }
        Ok(Invocation { target, arguments })
    }
}

/// Reads an invocation's arguments back in the order they were recorded, checking each label and type.
#[derive(Debug)]
pub struct InvocationDecoder {
    arguments: Vec<RemoteCallArgument>,
    next: usize,
}

impl InvocationDecoder {
    pub fn new(invocation: &Invocation) -> Self {
        InvocationDecoder { arguments: invocation.arguments.clone(), next: 0 }
    }

    /// Decodes the next argument as `T`. `label` must equal the argument's label (`None` for an unlabelled one).
    pub fn next<T: Decodable>(&mut self, label: Option<&str>) -> Result<T, DistributedActorCodingError> {
        let Some(arg) = self.arguments.get(self.next) else {
            return Err(DistributedActorCodingError(format!("missing argument {}", self.next)));
        };
        if arg.label.as_deref() != label {
            return Err(DistributedActorCodingError(format!(
                "argument {} has label {:?}, expected {:?}",
                self.next, arg.label, label
            )));
        }
        let value = T::decode(&arg.value)
            .map_err(|e| DistributedActorCodingError(format!("argument {}: {e}", self.next)))?;
        self.next += 1;
        Ok(value)
    }

    /// Arguments not decoded yet.
    pub fn remaining(&self) -> usize {
        self.arguments.len() - self.next
    }
}

/// Receives the outcome of one invocation. Exactly one callback runs: [`on_return`](Self::on_return) for a value,
/// [`on_return_void`](Self::on_return_void) for a handler that returned nothing, or [`on_throw`](Self::on_throw).
pub trait InvocationResultHandler {
    fn on_return(&mut self, value: Value);
    fn on_return_void(&mut self);
    fn on_throw(&mut self, error: &DistributedError);
}

/// Stores the outcome of an invocation, for tests and for callers that run a call in process.
#[derive(Debug, Default, PartialEq)]
pub struct CollectingResultHandler {
    pub value: Option<Value>,
    pub returned_void: bool,
    pub error: Option<DistributedError>,
}

impl InvocationResultHandler for CollectingResultHandler {
    fn on_return(&mut self, value: Value) {
        self.value = Some(value);
    }
    fn on_return_void(&mut self) {
        self.returned_void = true;
    }
    fn on_throw(&mut self, error: &DistributedError) {
        self.error = Some(error.clone());
    }
}

/// A handler for one target. It decodes its arguments from the decoder and returns its value; `Value::Null`
/// means the handler returns nothing.
pub type TargetHandler = Box<dyn FnMut(&mut InvocationDecoder) -> Result<Value, DistributedError>>;

/// Identifies an actor of a [`LocalTestingSystem`]. Ids are unique within the process.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ActorId(String);

impl ActorId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

static NEXT_ACTOR: AtomicU64 = AtomicU64::new(1);

/// An in-process actor system for tests. Calls run synchronously on the caller's thread; nothing crosses a
/// process or a network boundary.
#[derive(Default)]
pub struct LocalTestingSystem {
    actors: BTreeMap<ActorId, BTreeMap<String, TargetHandler>>,
}

impl LocalTestingSystem {
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new actor in this system and returns its id.
    pub fn make_actor_id(&mut self) -> ActorId {
        let id = ActorId(format!("local-{}", NEXT_ACTOR.fetch_add(1, Ordering::Relaxed)));
        self.actors.insert(id.clone(), BTreeMap::new());
        id
    }

    /// Registers `handler` for `target` on `actor`, replacing any handler already there for that target.
    pub fn register<F>(&mut self, actor: &ActorId, target: &str, handler: F) -> Result<(), DistributedError>
    where
        F: FnMut(&mut InvocationDecoder) -> Result<Value, DistributedError> + 'static,
    {
        let target = RemoteCallTarget::new(target)
            .ok_or_else(|| DistributedError::Coding(DistributedActorCodingError(format!("bad target identifier {target:?}"))))?;
        let table = self.actors.get_mut(actor).ok_or_else(|| DistributedError::UnknownActor(actor.0.clone()))?;
        table.insert(target.0, Box::new(handler));
        Ok(())
    }

    /// Runs `invocation` on `actor` and reports the outcome to `result`.
    pub fn execute(&mut self, actor: &ActorId, invocation: &Invocation, result: &mut dyn InvocationResultHandler) {
        let Some(table) = self.actors.get_mut(actor) else {
            result.on_throw(&DistributedError::UnknownActor(actor.0.clone()));
            return;
        };
        let key = invocation.target.0.clone();
        let Some(handler) = table.get_mut(&key) else {
            result.on_throw(&DistributedError::UnknownTarget(key));
            return;
        };
        let mut decoder = InvocationDecoder::new(invocation);
        match handler(&mut decoder) {
            Err(e) => result.on_throw(&e),
            Ok(_) if decoder.remaining() > 0 => result.on_throw(&DistributedError::Coding(DistributedActorCodingError(
                format!("{} unused argument(s)", decoder.remaining()),
            ))),
            Ok(Value::Null) => result.on_return_void(),
            Ok(v) => result.on_return(v),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::{JSONDecoder, JSONEncoder};

    /// A system with one actor and an `add(by:)` target that returns `by + 1`.
    fn counter() -> (LocalTestingSystem, ActorId) {
        let mut sys = LocalTestingSystem::new();
        let actor = sys.make_actor_id();
        sys.register(&actor, "Counter.add(by:)", |d| Ok(Value::Int(d.next::<i64>(Some("by"))? + 1))).unwrap();
        (sys, actor)
    }

    fn call(target: &str, by: i64) -> Invocation {
        let mut enc = InvocationEncoder::new();
        enc.record_argument(Some("by"), &by).unwrap();
        enc.done_recording(target).unwrap()
    }

    #[test]
    fn target_identifiers_are_validated() {
        assert!(RemoteCallTarget::new("Counter.add(by:)").is_some());
        assert!(RemoteCallTarget::new("").is_none());
        assert!(RemoteCallTarget::new("Counter add").is_none());
        assert!(RemoteCallTarget::new("Counter\n").is_none());
        assert!(RemoteCallTarget::new(&"x".repeat(MAX_NAME_LEN + 1)).is_none());
    }

    #[test]
    fn typed_arguments_survive_a_json_round_trip() {
        let mut enc = InvocationEncoder::new();
        enc.record_argument(Some("name"), &"Ada".to_string()).unwrap();
        enc.record_argument(None, &true).unwrap();
        enc.record_argument(Some("count"), &7i64).unwrap();
        enc.record_argument(Some("note"), &Option::<String>::None).unwrap();
        let call = enc.done_recording("Profile.update(name:)").unwrap();

        let text = JSONEncoder::new().encode(&call);
        let back: Invocation = JSONDecoder::new().decode(&text).unwrap();
        assert_eq!(back.target().as_str(), "Profile.update(name:)");

        let mut d = InvocationDecoder::new(&back);
        assert_eq!(d.next::<String>(Some("name")).unwrap(), "Ada");
        assert!(d.next::<bool>(None).unwrap());
        assert_eq!(d.next::<i64>(Some("count")).unwrap(), 7);
        assert_eq!(d.next::<Option<String>>(Some("note")).unwrap(), None);
        assert_eq!(d.remaining(), 0);
    }

    #[test]
    fn decoder_checks_label_type_and_count() {
        let call = call("Counter.add(by:)", 3);
        let mut wrong_label = InvocationDecoder::new(&call);
        assert!(wrong_label.next::<i64>(Some("count")).is_err());
        let mut wrong_type = InvocationDecoder::new(&call);
        assert!(wrong_type.next::<String>(Some("by")).is_err());
        let mut missing = InvocationDecoder::new(&call);
        assert_eq!(missing.next::<i64>(Some("by")).unwrap(), 3);
        let err = missing.next::<i64>(None).unwrap_err();
        assert!(err.0.contains("missing argument 1"));
    }

    #[test]
    fn encoder_refuses_empty_labels_and_targets() {
        let mut enc = InvocationEncoder::new();
        assert!(enc.record_argument(Some(""), &1i64).is_err());
        assert!(InvocationEncoder::new().done_recording("bad target").is_err());
    }

    #[test]
    fn execute_returns_the_handler_value() {
        let (mut sys, actor) = counter();
        let mut out = CollectingResultHandler::default();
        sys.execute(&actor, &call("Counter.add(by:)", 41), &mut out);
        assert_eq!(out.value, Some(Value::Int(42)));
        assert_eq!(out.error, None);
        assert!(!out.returned_void);
    }

    #[test]
    fn execute_reports_unknown_actor_and_target() {
        let (mut sys, actor) = counter();
        // An actor from another system is unknown here, even though it was created in this process.
        let stranger = LocalTestingSystem::new().make_actor_id();
        let mut out = CollectingResultHandler::default();
        sys.execute(&stranger, &call("Counter.add(by:)", 1), &mut out);
        assert!(matches!(out.error, Some(DistributedError::UnknownActor(_))));

        let mut out = CollectingResultHandler::default();
        sys.execute(&actor, &call("Counter.reset()", 1), &mut out);
        assert_eq!(out.error, Some(DistributedError::UnknownTarget("Counter.reset()".into())));
    }

    #[test]
    fn execute_rejects_arguments_the_handler_did_not_read() {
        let (mut sys, actor) = counter();
        let mut enc = InvocationEncoder::new();
        enc.record_argument(Some("by"), &1i64).unwrap();
        enc.record_argument(None, &"extra".to_string()).unwrap();
        let extra = enc.done_recording("Counter.add(by:)").unwrap();
        let mut out = CollectingResultHandler::default();
        sys.execute(&actor, &extra, &mut out);
        match out.error {
            Some(DistributedError::Coding(e)) => assert!(e.0.contains("1 unused")),
            other => panic!("expected a coding error, got {other:?}"),
        }
        assert_eq!(out.value, None);
    }

    #[test]
    fn void_and_failing_handlers_report_through_the_result_handler() {
        let mut sys = LocalTestingSystem::new();
        let actor = sys.make_actor_id();
        sys.register(&actor, "Counter.reset()", |_| Ok(Value::Null)).unwrap();
        sys.register(&actor, "Counter.fail()", |_| Err(DistributedError::Handler("boom".into()))).unwrap();

        let mut out = CollectingResultHandler::default();
        let reset = InvocationEncoder::new().done_recording("Counter.reset()").unwrap();
        sys.execute(&actor, &reset, &mut out);
        assert!(out.returned_void);
        assert_eq!(out.value, None);

        let mut out = CollectingResultHandler::default();
        let fail = InvocationEncoder::new().done_recording("Counter.fail()").unwrap();
        sys.execute(&actor, &fail, &mut out);
        assert_eq!(out.error, Some(DistributedError::Handler("boom".into())));
    }

    #[test]
    fn errors_map_to_json_rpc_codes() {
        assert_eq!(RpcError::from(DistributedError::UnknownTarget("t".into())).code, METHOD_NOT_FOUND);
        assert_eq!(RpcError::from(DistributedError::UnknownActor("a".into())).code, INVALID_PARAMS);
        let coding = DistributedError::from(DistributedActorCodingError("bad".into()));
        assert_eq!(RpcError::from(coding).code, INVALID_PARAMS);
        assert_eq!(RpcError::from(DistributedError::Handler("x".into())).code, INTERNAL_ERROR);
    }
}
