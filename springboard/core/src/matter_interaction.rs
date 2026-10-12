//! Matter interaction model: attribute and command paths, read, write and invoke requests, and subscriptions (std only).
//!
//! A controller talks to an accessory through paths. An attribute path names an endpoint, a cluster and an attribute,
//! and any of the three may be a wildcard. A command path names one concrete command. This module holds the path
//! types, checks which paths a request may carry, and keeps the set of subscriptions and the paths each one covers.
//! It sends nothing: the caller owns the transport, the session and the encoding of values on the wire.
//!
//! The paths and request shapes follow the official controller reference (read paths may hold wildcards, writes and
//! invokes name one concrete attribute or command, and subscriptions carry a min and a max interval). The rules for
//! an empty path list, for duplicate command fields and for interval order are this module's own rules.

/// A path to one or more attributes. A `None` field is a wildcard that matches every value of that field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AttributePath {
    pub endpoint: Option<u32>,
    pub cluster: Option<u32>,
    pub attribute: Option<u32>,
}

impl AttributePath {
    /// A path with the given fields. Pass `None` for a wildcard.
    pub fn new(endpoint: Option<u32>, cluster: Option<u32>, attribute: Option<u32>) -> Self {
        AttributePath { endpoint, cluster, attribute }
    }

    /// A path that names exactly one attribute.
    pub fn exact(endpoint: u32, cluster: u32, attribute: u32) -> Self {
        AttributePath::new(Some(endpoint), Some(cluster), Some(attribute))
    }

    /// True when no field is a wildcard.
    pub fn is_concrete(&self) -> bool {
        self.endpoint.is_some() && self.cluster.is_some() && self.attribute.is_some()
    }

    /// True when this path matches the attribute at the given endpoint, cluster and attribute.
    pub fn matches(&self, endpoint: u32, cluster: u32, attribute: u32) -> bool {
        field_matches(self.endpoint, endpoint)
            && field_matches(self.cluster, cluster)
            && field_matches(self.attribute, attribute)
    }

    /// True when every attribute this path matches is also matched by `other`. A wildcard in `other` covers any
    /// value; a wildcard in this path is covered only by a wildcard in `other`.
    pub fn is_within(&self, other: &AttributePath) -> bool {
        covered_by(self.endpoint, other.endpoint)
            && covered_by(self.cluster, other.cluster)
            && covered_by(self.attribute, other.attribute)
    }
}

fn field_matches(pattern: Option<u32>, value: u32) -> bool {
    pattern.map_or(true, |p| p == value)
}

fn covered_by(inner: Option<u32>, outer: Option<u32>) -> bool {
    match (inner, outer) {
        (_, None) => true,
        (Some(a), Some(b)) => a == b,
        (None, Some(_)) => false,
    }
}

/// A path to one command. Commands are always invoked with every field concrete.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CommandPath {
    pub endpoint: u32,
    pub cluster: u32,
    pub command: u32,
}

/// A value carried by a write or a command field. Wire encoding belongs to the transport.
#[derive(Clone, Debug, PartialEq)]
pub enum AttributeValue {
    Null,
    Bool(bool),
    Int(i64),
    Text(String),
}

/// A read of one or more attribute paths. The paths may hold wildcards. A read is refused when it names no path.
#[derive(Clone, Debug, PartialEq)]
pub struct ReadRequest {
    pub paths: Vec<AttributePath>,
    /// Limit the answer to the fabric the controller is on. Off unless the caller turns it on.
    pub fabric_filtered: bool,
}

impl ReadRequest {
    /// `None` when `paths` is empty.
    pub fn new(paths: Vec<AttributePath>) -> Option<Self> {
        if paths.is_empty() {
            None
        } else {
            Some(ReadRequest { paths, fabric_filtered: false })
        }
    }
}

/// A write to one attribute. The path must be concrete.
#[derive(Clone, Debug, PartialEq)]
pub struct WriteRequest {
    pub path: AttributePath,
    pub value: AttributeValue,
}

impl WriteRequest {
    /// `None` when the path has a wildcard.
    pub fn new(path: AttributePath, value: AttributeValue) -> Option<Self> {
        if path.is_concrete() {
            Some(WriteRequest { path, value })
        } else {
            None
        }
    }
}

/// An invoke of one command with its fields. Field IDs must be unique, so `None` comes back for a repeated field.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandRequest {
    pub path: CommandPath,
    pub fields: Vec<(u32, AttributeValue)>,
}

impl CommandRequest {
    pub fn new(path: CommandPath, fields: Vec<(u32, AttributeValue)>) -> Option<Self> {
        let mut ids: Vec<u32> = fields.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        if ids.windows(2).any(|pair| pair[0] == pair[1]) {
            return None;
        }
        Some(CommandRequest { path, fields })
    }
}

/// How often a subscription reports. The two intervals use the same unit, which the caller chooses. The minimum may
/// not be above the maximum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubscribeParams {
    pub min_interval: u16,
    pub max_interval: u16,
}

impl SubscribeParams {
    /// `None` when `min_interval` is greater than `max_interval`.
    pub fn new(min_interval: u16, max_interval: u16) -> Option<Self> {
        if min_interval > max_interval {
            None
        } else {
            Some(SubscribeParams { min_interval, max_interval })
        }
    }
}

/// One live subscription: its identifier, the paths it reports and its intervals.
#[derive(Clone, Debug, PartialEq)]
pub struct Subscription {
    pub id: u32,
    pub paths: Vec<AttributePath>,
    pub params: SubscribeParams,
}

/// The subscriptions held by one controller. Identifiers start at 1 and only grow.
#[derive(Clone, Debug, PartialEq)]
pub struct SubscriptionSet {
    subs: Vec<Subscription>,
    next_id: u32,
}

impl Default for SubscriptionSet {
    fn default() -> Self {
        SubscriptionSet::new()
    }
}

impl SubscriptionSet {
    pub fn new() -> Self {
        SubscriptionSet { subs: Vec::new(), next_id: 1 }
    }

    /// Adds a subscription and returns its identifier. With `replace_existing`, every earlier subscription is dropped
    /// first. `None` when `paths` is empty.
    pub fn subscribe(&mut self, paths: Vec<AttributePath>, params: SubscribeParams, replace_existing: bool) -> Option<u32> {
        if paths.is_empty() {
            return None;
        }
        if replace_existing {
            self.subs.clear();
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.subs.push(Subscription { id, paths, params });
        Some(id)
    }

    /// Removes a subscription. Returns `false` when no subscription has that identifier.
    pub fn unsubscribe(&mut self, id: u32) -> bool {
        let before = self.subs.len();
        self.subs.retain(|s| s.id != id);
        self.subs.len() != before
    }

    /// Identifiers of the subscriptions that report the attribute at this endpoint, cluster and attribute, in the
    /// order they were added.
    pub fn interested(&self, endpoint: u32, cluster: u32, attribute: u32) -> Vec<u32> {
        self.subs
            .iter()
            .filter(|s| s.paths.iter().any(|p| p.matches(endpoint, cluster, attribute)))
            .map(|s| s.id)
            .collect()
    }

    pub fn len(&self) -> usize {
        self.subs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.subs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards_match_any_value_of_their_field() {
        let path = AttributePath::new(Some(1), None, Some(0));
        assert!(path.matches(1, 6, 0));
        assert!(path.matches(1, 8, 0));
        assert!(!path.matches(2, 6, 0), "endpoint 2 is not endpoint 1");
        assert!(!path.matches(1, 6, 5), "attribute 5 is not attribute 0");
        assert!(!path.is_concrete());
        assert!(AttributePath::exact(1, 6, 0).is_concrete());
    }

    #[test]
    fn a_path_is_within_another_only_when_every_field_is_covered() {
        let exact = AttributePath::exact(1, 6, 0);
        let endpoint_only = AttributePath::new(Some(1), None, None);
        let everything = AttributePath::new(None, None, None);
        assert!(exact.is_within(&endpoint_only));
        assert!(exact.is_within(&everything));
        assert!(!endpoint_only.is_within(&exact), "a wildcard is not inside a single attribute");
        assert!(!AttributePath::exact(2, 6, 0).is_within(&endpoint_only));
    }

    #[test]
    fn read_and_write_requests_refuse_what_they_cannot_carry() {
        assert!(ReadRequest::new(Vec::new()).is_none());
        let read = ReadRequest::new(vec![AttributePath::new(None, Some(6), None)]).expect("one path");
        assert!(!read.fabric_filtered, "reads are not fabric filtered unless asked");

        assert!(WriteRequest::new(AttributePath::new(Some(1), Some(6), None), AttributeValue::Bool(true)).is_none());
        let write = WriteRequest::new(AttributePath::exact(1, 6, 0), AttributeValue::Bool(true)).expect("concrete");
        assert_eq!(write.path, AttributePath::exact(1, 6, 0));
    }

    #[test]
    fn a_command_cannot_repeat_a_field() {
        let path = CommandPath { endpoint: 1, cluster: 8, command: 0 };
        let repeated = vec![(0, AttributeValue::Int(1)), (0, AttributeValue::Int(2))];
        assert!(CommandRequest::new(path, repeated).is_none());
        let distinct = vec![(0, AttributeValue::Int(1)), (1, AttributeValue::Null)];
        let request = CommandRequest::new(path, distinct).expect("distinct fields");
        assert_eq!(request.fields.len(), 2);
        assert!(CommandRequest::new(path, Vec::new()).is_some(), "a command with no fields is fine");
    }

    #[test]
    fn subscription_intervals_must_not_run_backwards() {
        assert!(SubscribeParams::new(10, 5).is_none());
        let equal = SubscribeParams::new(5, 5).expect("equal intervals");
        assert_eq!((equal.min_interval, equal.max_interval), (5, 5));
    }

    #[test]
    fn interest_follows_the_subscribed_paths_and_replace_clears_the_rest() {
        let params = SubscribeParams::new(1, 60).expect("valid");
        let mut set = SubscriptionSet::new();
        let a = set.subscribe(vec![AttributePath::new(Some(1), Some(6), None)], params, false).expect("paths");
        let b = set.subscribe(vec![AttributePath::exact(2, 8, 0)], params, false).expect("paths");
        assert_eq!((a, b), (1, 2));
        assert_eq!(set.interested(1, 6, 0), vec![a]);
        assert_eq!(set.interested(2, 8, 0), vec![b]);
        assert!(set.interested(1, 8, 0).is_empty());
        assert!(set.subscribe(Vec::new(), params, false).is_none());

        let c = set.subscribe(vec![AttributePath::exact(3, 3, 3)], params, true).expect("paths");
        assert_eq!(c, 3, "identifiers keep counting after a replace");
        assert_eq!(set.len(), 1);
        assert!(set.interested(1, 6, 0).is_empty());
        assert!(set.unsubscribe(c));
        assert!(!set.unsubscribe(c));
        assert!(set.is_empty());
    }
}
