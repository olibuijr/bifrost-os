//! Deferred background work for apps: task requests, a pending queue, and the rule for which requests may run now.
//!
//! This is a pure state model. Nothing here starts a thread or a process. The owner keeps a [`Scheduler`], submits
//! [`Request`]s from the app, and calls [`Scheduler::due`] from its own loop with the current time in seconds since
//! the Unix epoch and the conditions the machine meets right now. Requests that are due come back once and leave the
//! queue; the owner then runs the work.
//!
//! Rules the model follows:
//! - An identifier must not be empty. Submitting a request with an identifier that is already pending replaces it.
//! - `earliest_begin` is a lower bound: a request is never due before that time.
//! - A request that needs the network or external power stays pending until the matching condition holds.
//! - `pending` lists requests sorted by identifier, so the order does not depend on submission order.
//!
//! Scope: the model and its JSON codec, shared by the shell scheduler (`springboard/src/shell/services/background.rs`)
//! and the SDK wrapper (`bifrost_app::background_tasks`). The shell owns time limits, budgets and launching.

use crate::json::Json;
use std::collections::BTreeMap;

/// The two kinds of deferred work an app can ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Short work that refreshes content. Expected to finish quickly.
    Refresh,
    /// Longer work such as sync or indexing. May need network or power.
    Processing,
}

/// One request for deferred work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub identifier: String,
    pub kind: Kind,
    /// Seconds since the Unix epoch before which the work must not run. `None` means no lower bound.
    pub earliest_begin: Option<u64>,
    pub requires_network: bool,
    pub requires_external_power: bool,
}

impl Request {
    /// A refresh request with no lower bound and no requirements.
    pub fn refresh(identifier: &str) -> Self {
        Self::new(identifier, Kind::Refresh)
    }

    /// A processing request with no lower bound and no requirements.
    pub fn processing(identifier: &str) -> Self {
        Self::new(identifier, Kind::Processing)
    }

    fn new(identifier: &str, kind: Kind) -> Self {
        Request {
            identifier: identifier.to_string(),
            kind,
            earliest_begin: None,
            requires_network: false,
            requires_external_power: false,
        }
    }

    /// Sets the earliest time the work may run, in seconds since the Unix epoch.
    pub fn earliest(mut self, at: u64) -> Self {
        self.earliest_begin = Some(at);
        self
    }

    /// Marks the work as needing network access.
    pub fn network(mut self) -> Self {
        self.requires_network = true;
        self
    }

    /// Marks the work as needing external power.
    pub fn external_power(mut self) -> Self {
        self.requires_external_power = true;
        self
    }
}

/// The conditions the machine meets at the moment [`Scheduler::due`] is called.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Conditions {
    pub network: bool,
    pub external_power: bool,
}

/// Errors from submitting a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    EmptyIdentifier,
}

/// The queue of pending requests for one app.
#[derive(Debug, Default)]
pub struct Scheduler {
    pending: BTreeMap<String, Request>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a request. A pending request with the same identifier is replaced.
    pub fn submit(&mut self, request: Request) -> Result<(), Error> {
        if request.identifier.is_empty() {
            return Err(Error::EmptyIdentifier);
        }
        self.pending.insert(request.identifier.clone(), request);
        Ok(())
    }

    /// Removes one pending request. Returns whether it was pending.
    pub fn cancel(&mut self, identifier: &str) -> bool {
        self.pending.remove(identifier).is_some()
    }

    /// Removes every pending request.
    pub fn cancel_all(&mut self) {
        self.pending.clear();
    }

    /// The pending requests, sorted by identifier.
    pub fn pending(&self) -> Vec<&Request> {
        self.pending.values().collect()
    }

    /// Removes and returns the requests that may run at `now` under `conditions`, sorted by identifier.
    pub fn due(&mut self, now: u64, conditions: Conditions) -> Vec<Request> {
        let ready: Vec<String> = self
            .pending
            .values()
            .filter(|r| is_due(r, now, conditions))
            .map(|r| r.identifier.clone())
            .collect();
        ready.iter().filter_map(|id| self.pending.remove(id)).collect()
    }
}

fn is_due(request: &Request, now: u64, conditions: Conditions) -> bool {
    let started = request.earliest_begin.map_or(true, |at| now >= at);
    let network_ok = !request.requires_network || conditions.network;
    let power_ok = !request.requires_external_power || conditions.external_power;
    started && network_ok && power_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONLINE: Conditions = Conditions { network: true, external_power: false };
    const PLUGGED: Conditions = Conditions { network: false, external_power: true };
    const NONE: Conditions = Conditions { network: false, external_power: false };

    fn ids(requests: &[Request]) -> Vec<&str> {
        requests.iter().map(|r| r.identifier.as_str()).collect()
    }

    #[test]
    fn empty_identifier_is_rejected() {
        let mut s = Scheduler::new();
        assert_eq!(s.submit(Request::refresh("")), Err(Error::EmptyIdentifier));
        assert!(s.pending().is_empty());
    }

    #[test]
    fn resubmitting_an_identifier_replaces_the_pending_request() {
        let mut s = Scheduler::new();
        s.submit(Request::refresh("sync").earliest(100)).unwrap();
        s.submit(Request::processing("sync")).unwrap();
        let pending = s.pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].kind, Kind::Processing);
        assert_eq!(pending[0].earliest_begin, None);
    }

    #[test]
    fn pending_is_sorted_by_identifier() {
        let mut s = Scheduler::new();
        s.submit(Request::refresh("b")).unwrap();
        s.submit(Request::refresh("a")).unwrap();
        s.submit(Request::refresh("c")).unwrap();
        let pending = s.pending();
        let names: Vec<&str> = pending.iter().map(|r| r.identifier.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn cancel_reports_whether_the_request_was_pending() {
        let mut s = Scheduler::new();
        s.submit(Request::refresh("a")).unwrap();
        assert!(s.cancel("a"));
        assert!(!s.cancel("a"));
    }

    #[test]
    fn cancel_all_empties_the_queue() {
        let mut s = Scheduler::new();
        s.submit(Request::refresh("a")).unwrap();
        s.submit(Request::processing("b")).unwrap();
        s.cancel_all();
        assert!(s.pending().is_empty());
    }

    #[test]
    fn a_request_is_not_due_before_its_earliest_time() {
        let mut s = Scheduler::new();
        s.submit(Request::refresh("a").earliest(1_000)).unwrap();
        assert!(s.due(999, NONE).is_empty());
        assert_eq!(ids(&s.due(1_000, NONE)), vec!["a"]);
    }

    #[test]
    fn a_request_without_a_lower_bound_is_due_at_once() {
        let mut s = Scheduler::new();
        s.submit(Request::refresh("a")).unwrap();
        assert_eq!(ids(&s.due(0, NONE)), vec!["a"]);
    }

    #[test]
    fn a_network_request_waits_for_the_network() {
        let mut s = Scheduler::new();
        s.submit(Request::processing("sync").network()).unwrap();
        assert!(s.due(10, PLUGGED).is_empty());
        assert_eq!(ids(&s.due(10, ONLINE)), vec!["sync"]);
    }

    #[test]
    fn a_power_request_waits_for_external_power() {
        let mut s = Scheduler::new();
        s.submit(Request::processing("index").external_power()).unwrap();
        assert!(s.due(10, ONLINE).is_empty());
        assert_eq!(ids(&s.due(10, PLUGGED)), vec!["index"]);
    }

    #[test]
    fn due_removes_only_the_requests_it_returns() {
        let mut s = Scheduler::new();
        s.submit(Request::refresh("now")).unwrap();
        s.submit(Request::refresh("later").earliest(500)).unwrap();
        let first = s.due(100, NONE);
        assert_eq!(ids(&first), vec!["now"]);
        let pending = s.pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].identifier, "later");
        assert_eq!(ids(&s.due(500, NONE)), vec!["later"]);
        assert!(s.pending().is_empty());
    }

    #[test]
    fn due_returns_requests_sorted_by_identifier() {
        let mut s = Scheduler::new();
        s.submit(Request::refresh("z")).unwrap();
        s.submit(Request::refresh("m")).unwrap();
        assert_eq!(ids(&s.due(0, NONE)), vec!["m", "z"]);
    }

    #[test]
    fn a_request_with_both_requirements_needs_both_conditions() {
        let mut s = Scheduler::new();
        s.submit(Request::processing("full").network().external_power()).unwrap();
        assert!(s.due(0, ONLINE).is_empty());
        assert!(s.due(0, PLUGGED).is_empty());
        let both = Conditions { network: true, external_power: true };
        assert_eq!(ids(&s.due(0, both)), vec!["full"]);
    }
}

impl Kind {
    /// The wire spelling used in the service JSON and the persisted queue.
    pub fn wire(self) -> &'static str {
        match self {
            Kind::Refresh => "refresh",
            Kind::Processing => "processing",
        }
    }

    /// The kind for a wire spelling; None for anything else.
    pub fn from_wire(text: &str) -> Option<Kind> {
        match text {
            "refresh" => Some(Kind::Refresh),
            "processing" => Some(Kind::Processing),
            _ => None,
        }
    }
}

impl Request {
    /// The JSON the SDK sends: `{"identifier", "kind", "earliest"?, "network", "power"}`. `earliest` is seconds since
    /// the Unix epoch and is left out when there is no lower bound.
    pub fn to_json(&self) -> Json {
        let mut pairs = vec![
            ("identifier".to_string(), Json::Str(self.identifier.clone())),
            ("kind".to_string(), Json::Str(self.kind.wire().to_string())),
            ("network".to_string(), Json::Bool(self.requires_network)),
            ("power".to_string(), Json::Bool(self.requires_external_power)),
        ];
        if let Some(at) = self.earliest_begin {
            pairs.push(("earliest".to_string(), Json::Num(at as f64)));
        }
        Json::Obj(pairs)
    }

    /// Reads the JSON `to_json` writes. Errors name the bad field; the identifier must not be empty.
    pub fn from_json(body: &Json) -> Result<Request, String> {
        let identifier = body.str_field("identifier").filter(|s| !s.is_empty()).ok_or("missing identifier")?;
        let kind = body.get("kind").and_then(Json::as_str).and_then(Kind::from_wire).ok_or("kind must be refresh or processing")?;
        let earliest_begin = match body.get("earliest") {
            None | Some(Json::Null) => None,
            Some(v) => Some(v.as_u64().ok_or("earliest must be whole epoch seconds")?),
        };
        let flag = |key: &str| matches!(body.get(key), Some(Json::Bool(true)));
        Ok(Request { identifier, kind, earliest_begin, requires_network: flag("network"), requires_external_power: flag("power") })
    }
}

#[cfg(test)]
mod codec_tests {
    use super::*;

    #[test]
    fn a_request_round_trips_through_json() {
        let r = Request::processing("com.x.sync").earliest(1_700_000_000).network().external_power();
        let back = Request::from_json(&r.to_json()).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn a_request_without_a_lower_bound_leaves_earliest_out() {
        let j = Request::refresh("com.x.refresh").to_json();
        assert!(j.get("earliest").is_none());
        assert_eq!(Request::from_json(&j).unwrap().earliest_begin, None);
    }

    #[test]
    fn bad_json_names_the_field() {
        let no_id = crate::json::parse(r#"{"kind":"refresh"}"#).unwrap();
        assert_eq!(Request::from_json(&no_id), Err("missing identifier".to_string()));
        let bad_kind = crate::json::parse(r#"{"identifier":"a","kind":"hourly"}"#).unwrap();
        assert!(Request::from_json(&bad_kind).unwrap_err().contains("kind"));
        let bad_time = crate::json::parse(r#"{"identifier":"a","kind":"refresh","earliest":-5}"#).unwrap();
        assert!(Request::from_json(&bad_time).unwrap_err().contains("earliest"));
    }
}
