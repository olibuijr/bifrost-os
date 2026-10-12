//! Authorization events: a request is held until the client answers allow or deny, and the answer decides whether
//! the operation runs. This is the model only. It has a rule policy that decides a request, and a queue that
//! holds pending requests, enforces one answer per request and applies the deadline-miss mode when a request
//! runs out of time. No shell path produces these requests yet, so nothing is blocked by it.

use std::collections::BTreeMap;

/// The operations that can be held for an answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuthOp {
    Exec,
    Open,
    Rename,
    Unlink,
    Mount,
}

impl AuthOp {
    pub fn name(self) -> &'static str {
        match self {
            AuthOp::Exec => "exec",
            AuthOp::Open => "open",
            AuthOp::Rename => "rename",
            AuthOp::Unlink => "unlink",
            AuthOp::Mount => "mount",
        }
    }
}

/// The answer to one request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthResult {
    Allow,
    Deny,
}

/// What happens when a client does not answer in time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeadlineMode {
    /// Deny the operation, keep the client running.
    FailClosed,
    /// Allow the operation, keep the client running.
    FailOpen,
    /// Stop the client. This is the default.
    #[default]
    Kill,
}

/// One held request. `deadline_ms` is an absolute time on the caller's clock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthRequest {
    pub id: u64,
    pub op: AuthOp,
    pub source_pid: u32,
    pub path: String,
    pub deadline_ms: u64,
}

/// A policy rule. A missing field matches anything; `path_prefix` matches whole path components, so `/tmp`
/// matches `/tmp` and `/tmp/x` but not `/tmpfoo`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub op: Option<AuthOp>,
    pub path_prefix: Option<String>,
    pub source_pid: Option<u32>,
    pub result: AuthResult,
}

impl Rule {
    fn matches(&self, req: &AuthRequest) -> bool {
        self.op.is_none_or(|op| op == req.op)
            && self.source_pid.is_none_or(|pid| pid == req.source_pid)
            && self.path_prefix.as_deref().is_none_or(|prefix| path_has_prefix(&req.path, prefix))
    }
}

fn path_has_prefix(path: &str, prefix: &str) -> bool {
    let prefix = prefix.trim_end_matches('/');
    if prefix.is_empty() {
        return path.starts_with('/');
    }
    path == prefix || path.strip_prefix(prefix).is_some_and(|rest| rest.starts_with('/'))
}

/// Ordered rules with a default. The first matching rule decides; with no match, `default` decides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    pub rules: Vec<Rule>,
    pub default: AuthResult,
}

impl Policy {
    pub fn new(default: AuthResult) -> Self {
        Policy { rules: Vec::new(), default }
    }

    pub fn push(&mut self, rule: Rule) {
        self.rules.push(rule);
    }

    pub fn decide(&self, req: &AuthRequest) -> AuthResult {
        self.rules.iter().find(|rule| rule.matches(req)).map_or(self.default, |rule| rule.result)
    }
}

/// Errors from the queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthError {
    /// A request with this id is already held.
    Duplicate(u64),
    /// No request with this id is held: it was never received, or it was already answered or expired.
    Unknown(u64),
}

/// What the queue did for a request whose deadline passed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timeout {
    Answer(AuthResult),
    KillClient,
}

/// Held requests, answered once each.
#[derive(Clone, Debug)]
pub struct AuthQueue {
    pending: BTreeMap<u64, AuthRequest>,
    mode: DeadlineMode,
}

impl AuthQueue {
    pub fn new(mode: DeadlineMode) -> Self {
        AuthQueue { pending: BTreeMap::new(), mode }
    }

    pub fn deadline_mode(&self) -> DeadlineMode {
        self.mode
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Holds a request until it is answered or expires.
    pub fn receive(&mut self, req: AuthRequest) -> Result<(), AuthError> {
        if self.pending.contains_key(&req.id) {
            return Err(AuthError::Duplicate(req.id));
        }
        self.pending.insert(req.id, req);
        Ok(())
    }

    /// Answers a held request and releases it.
    pub fn respond(&mut self, id: u64, result: AuthResult) -> Result<AuthResult, AuthError> {
        self.pending.remove(&id).map(|_| result).ok_or(AuthError::Unknown(id))
    }

    /// Releases every request whose deadline is at or before `now_ms`, in id order. Each expired id comes back
    /// with what the deadline mode does to it: an answer for the fail modes, or a kill of the client.
    pub fn expire(&mut self, now_ms: u64) -> Vec<(u64, Timeout)> {
        let mode = self.mode;
        let due: Vec<u64> = self.pending.values().filter(|r| r.deadline_ms <= now_ms).map(|r| r.id).collect();
        let mut out = Vec::with_capacity(due.len());
        for id in due {
            self.pending.remove(&id);
            let timeout = match mode {
                DeadlineMode::FailClosed => Timeout::Answer(AuthResult::Deny),
                DeadlineMode::FailOpen => Timeout::Answer(AuthResult::Allow),
                DeadlineMode::Kill => Timeout::KillClient,
            };
            out.push((id, timeout));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(id: u64, op: AuthOp, pid: u32, path: &str, deadline_ms: u64) -> AuthRequest {
        AuthRequest { id, op, source_pid: pid, path: path.to_string(), deadline_ms }
    }

    #[test]
    fn first_matching_rule_decides_and_default_applies_otherwise() {
        let mut policy = Policy::new(AuthResult::Allow);
        policy.push(Rule { op: Some(AuthOp::Exec), path_prefix: Some("/tmp".into()), source_pid: None, result: AuthResult::Deny });
        policy.push(Rule { op: Some(AuthOp::Exec), path_prefix: None, source_pid: None, result: AuthResult::Allow });
        assert_eq!(policy.decide(&req(1, AuthOp::Exec, 9, "/tmp/x", 0)), AuthResult::Deny);
        assert_eq!(policy.decide(&req(2, AuthOp::Exec, 9, "/usr/bin/ls", 0)), AuthResult::Allow);
        assert_eq!(policy.decide(&req(3, AuthOp::Open, 9, "/tmp/x", 0)), AuthResult::Allow);
    }

    #[test]
    fn path_prefix_matches_whole_components_only() {
        let mut policy = Policy::new(AuthResult::Allow);
        policy.push(Rule { op: None, path_prefix: Some("/tmp".into()), source_pid: None, result: AuthResult::Deny });
        assert_eq!(policy.decide(&req(1, AuthOp::Open, 1, "/tmp", 0)), AuthResult::Deny);
        assert_eq!(policy.decide(&req(2, AuthOp::Open, 1, "/tmp/a/b", 0)), AuthResult::Deny);
        assert_eq!(policy.decide(&req(3, AuthOp::Open, 1, "/tmpfoo", 0)), AuthResult::Allow);
    }

    #[test]
    fn source_pid_rule_only_matches_that_process() {
        let mut policy = Policy::new(AuthResult::Allow);
        policy.push(Rule { op: Some(AuthOp::Mount), path_prefix: None, source_pid: Some(77), result: AuthResult::Deny });
        assert_eq!(policy.decide(&req(1, AuthOp::Mount, 77, "/mnt", 0)), AuthResult::Deny);
        assert_eq!(policy.decide(&req(2, AuthOp::Mount, 78, "/mnt", 0)), AuthResult::Allow);
    }

    #[test]
    fn each_request_is_answered_once() {
        let mut q = AuthQueue::new(DeadlineMode::Kill);
        q.receive(req(5, AuthOp::Unlink, 3, "/a", 100)).unwrap();
        assert_eq!(q.receive(req(5, AuthOp::Unlink, 3, "/a", 100)), Err(AuthError::Duplicate(5)));
        assert_eq!(q.respond(5, AuthResult::Allow), Ok(AuthResult::Allow));
        assert_eq!(q.respond(5, AuthResult::Deny), Err(AuthError::Unknown(5)));
        assert_eq!(q.pending_count(), 0);
    }

    #[test]
    fn fail_closed_denies_a_request_that_misses_its_deadline() {
        let mut q = AuthQueue::new(DeadlineMode::FailClosed);
        q.receive(req(1, AuthOp::Open, 3, "/a", 100)).unwrap();
        assert!(q.expire(99).is_empty());
        assert_eq!(q.expire(100), vec![(1, Timeout::Answer(AuthResult::Deny))]);
        assert_eq!(q.respond(1, AuthResult::Allow), Err(AuthError::Unknown(1)));
    }

    #[test]
    fn fail_open_allows_and_kill_stops_the_client() {
        let mut open = AuthQueue::new(DeadlineMode::FailOpen);
        open.receive(req(1, AuthOp::Rename, 3, "/a", 10)).unwrap();
        assert_eq!(open.expire(50), vec![(1, Timeout::Answer(AuthResult::Allow))]);

        let mut kill = AuthQueue::new(DeadlineMode::default());
        kill.receive(req(2, AuthOp::Exec, 3, "/bin/x", 10)).unwrap();
        kill.receive(req(3, AuthOp::Exec, 3, "/bin/y", 500)).unwrap();
        assert_eq!(kill.expire(20), vec![(2, Timeout::KillClient)]);
        assert_eq!(kill.pending_count(), 1);
    }

    #[test]
    fn op_names_are_stable() {
        assert_eq!(AuthOp::Mount.name(), "mount");
        assert_eq!(DeadlineMode::default(), DeadlineMode::Kill);
    }
}
