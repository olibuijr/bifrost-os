//! Endpoint security events: the file open, exec, modify and close events a security app can subscribe to, taken from
//! the Linux fanotify feed. This module reads the kernel's event records, matches them to subscriptions, decides
//! authorization events, and builds the reply the kernel waits for. It opens no descriptor and talks to no kernel:
//! the root service that owns the fanotify descriptor feeds it bytes and writes back the bytes it returns.
//!
//! Rules of this model:
//! - A record is accepted only if its version is 3, its metadata length is at least 24 bytes and its length fits the
//!   buffer. Anything else stops the parse with an error, so a damaged buffer never yields a wrong event.
//! - A notification event is delivered to every subscription that asked for its kind and does not mute its path. A path
//!   is muted by a prefix that ends at a path boundary (`/home/a` mutes `/home/a/x`, not `/home/ab`).
//! - An authorization event (`open`/`exec` before it happens) is answered by every subscription that asked for it. One
//!   deny from any of them denies it. When the deadline passes before every subscriber answered, the event is
//!   allowed: a stuck security app must not freeze the computer. The round reports that it timed out.
//! - Only the event's kinds, process id and path leave this module. File contents are never read.
//! - Who may subscribe: only an app whose manifest declares the [`ENTITLEMENT`]. The shell service that apps call
//!   makes that check (`shell/services/endpoint_security.rs`); the root service behind it (`privd/endpoint_security.rs`)
//!   serves only the shell, so an app never reaches it directly.

use crate::json::Json;

/// The manifest entitlement (`entitlements = ["endpoint-security"]`) an app must declare to subscribe to file events.
pub const ENTITLEMENT: &str = "endpoint-security";

/// Kernel event bits (`FAN_*`).
pub const ACCESS: u64 = 0x1;
pub const MODIFY: u64 = 0x2;
pub const CLOSE_WRITE: u64 = 0x8;
pub const CLOSE_NOWRITE: u64 = 0x10;
pub const OPEN: u64 = 0x20;
pub const OPEN_EXEC: u64 = 0x1000;
pub const OPEN_PERM: u64 = 0x1_0000;
pub const OPEN_EXEC_PERM: u64 = 0x4_0000;
/// Set when the kernel dropped events because its queue was full.
pub const Q_OVERFLOW: u64 = 0x4000;

const VERSION: u8 = 3;
const METADATA_LEN: usize = 24;

/// What happened to a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Access,
    Modify,
    CloseWrite,
    CloseNoWrite,
    Open,
    Exec,
}

impl Kind {
    pub const ALL: [Kind; 6] = [Kind::Access, Kind::Modify, Kind::CloseWrite, Kind::CloseNoWrite, Kind::Open, Kind::Exec];

    /// The name on the wire.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Access => "access",
            Kind::Modify => "modify",
            Kind::CloseWrite => "close_write",
            Kind::CloseNoWrite => "close_nowrite",
            Kind::Open => "open",
            Kind::Exec => "exec",
        }
    }

    pub fn from_name(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// The kernel bit for the notification of this kind.
    pub fn bit(self) -> u64 {
        match self {
            Kind::Access => ACCESS,
            Kind::Modify => MODIFY,
            Kind::CloseWrite => CLOSE_WRITE,
            Kind::CloseNoWrite => CLOSE_NOWRITE,
            Kind::Open => OPEN,
            Kind::Exec => OPEN_EXEC,
        }
    }
}

/// An event the kernel asks to be allowed before it happens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthKind {
    Open,
    Exec,
}

impl AuthKind {
    pub fn name(self) -> &'static str {
        match self {
            AuthKind::Open => "open",
            AuthKind::Exec => "exec",
        }
    }

    pub fn from_name(name: &str) -> Option<AuthKind> {
        [AuthKind::Open, AuthKind::Exec].into_iter().find(|k| k.name() == name)
    }

    pub fn bit(self) -> u64 {
        match self {
            AuthKind::Open => OPEN_PERM,
            AuthKind::Exec => OPEN_EXEC_PERM,
        }
    }
}

/// One kernel record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawEvent {
    pub mask: u64,
    /// The open file descriptor of the file (`-1` for an overflow record). The service reads its path from
    /// `/proc/self/fd/<fd>`, closes it, and answers an authorization event on it.
    pub fd: i32,
    pub pid: i32,
}

impl RawEvent {
    /// The notification kinds in the mask, in [`Kind::ALL`] order.
    pub fn kinds(&self) -> Vec<Kind> {
        Kind::ALL.iter().copied().filter(|k| self.mask & k.bit() != 0).collect()
    }

    /// The authorization kind, when the kernel waits for an answer.
    pub fn auth(&self) -> Option<AuthKind> {
        if self.mask & OPEN_EXEC_PERM != 0 {
            Some(AuthKind::Exec)
        } else if self.mask & OPEN_PERM != 0 {
            Some(AuthKind::Open)
        } else {
            None
        }
    }

    pub fn overflowed(&self) -> bool {
        self.mask & Q_OVERFLOW != 0
    }
}

/// Why a buffer was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The record's version is not 3.
    Version(u8),
    /// The lengths are inconsistent or run past the buffer.
    Length,
}

/// Reads the records in a buffer returned by the kernel. A buffer that ends in the middle of a record is an error.
pub fn parse_events(buf: &[u8]) -> Result<Vec<RawEvent>, ParseError> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < buf.len() {
        let rec = &buf[at..];
        if rec.len() < METADATA_LEN {
            return Err(ParseError::Length);
        }
        let event_len = u32::from_ne_bytes(rec[0..4].try_into().unwrap()) as usize;
        let vers = rec[4];
        let meta_len = u16::from_ne_bytes(rec[6..8].try_into().unwrap()) as usize;
        if vers != VERSION {
            return Err(ParseError::Version(vers));
        }
        if meta_len < METADATA_LEN || event_len < meta_len || event_len > rec.len() {
            return Err(ParseError::Length);
        }
        out.push(RawEvent {
            mask: u64::from_ne_bytes(rec[8..16].try_into().unwrap()),
            fd: i32::from_ne_bytes(rec[16..20].try_into().unwrap()),
            pid: i32::from_ne_bytes(rec[20..24].try_into().unwrap()),
        });
        at += event_len;
    }
    Ok(out)
}

/// An answer to an authorization event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
}

/// The 8 bytes the kernel reads to learn the answer for `fd`.
pub fn response_bytes(fd: i32, decision: Decision) -> [u8; 8] {
    let mut out = [0u8; 8];
    out[..4].copy_from_slice(&fd.to_ne_bytes());
    let code: u32 = match decision {
        Decision::Allow => 1,
        Decision::Deny => 2,
    };
    out[4..].copy_from_slice(&code.to_ne_bytes());
    out
}

/// What a security app asked to see.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Subscription {
    pub kinds: Vec<Kind>,
    pub auth: Vec<AuthKind>,
    /// Path prefixes whose events this subscriber never gets.
    pub muted: Vec<String>,
}

fn under(path: &str, prefix: &str) -> bool {
    let prefix = prefix.trim_end_matches('/');
    path == prefix || path.strip_prefix(prefix).is_some_and(|rest| rest.starts_with('/'))
}

impl Subscription {
    fn muted(&self, path: &str) -> bool {
        self.muted.iter().any(|m| !m.is_empty() && under(path, m))
    }

    /// True when a notification of any of `kinds` on `path` is wanted.
    pub fn wants_notify(&self, kinds: &[Kind], path: &str) -> bool {
        !self.muted(path) && kinds.iter().any(|k| self.kinds.contains(k))
    }

    /// True when the authorization of `kind` on `path` is to be asked of this subscriber.
    pub fn wants_auth(&self, kind: AuthKind, path: &str) -> bool {
        !self.muted(path) && self.auth.contains(&kind)
    }

    /// The kernel mask the service must mark so that every wanted event is reported.
    pub fn mask(&self) -> u64 {
        self.kinds.iter().fold(0, |m, k| m | k.bit()) | self.auth.iter().fold(0, |m, k| m | k.bit())
    }
}

/// The subscribers of one service.
#[derive(Debug, Default)]
pub struct Router {
    subs: Vec<(u32, Subscription)>,
    next: u32,
}

impl Router {
    pub fn new() -> Router {
        Router::default()
    }

    /// Adds a subscription and returns its id.
    pub fn subscribe(&mut self, s: Subscription) -> u32 {
        self.next += 1;
        self.subs.push((self.next, s));
        self.next
    }

    /// Removes a subscription (an app that exited). True when it existed.
    pub fn unsubscribe(&mut self, id: u32) -> bool {
        let before = self.subs.len();
        self.subs.retain(|(i, _)| *i != id);
        self.subs.len() != before
    }

    /// The kernel mask that covers every subscription.
    pub fn mask(&self) -> u64 {
        self.subs.iter().fold(0, |m, (_, s)| m | s.mask())
    }

    /// The subscribers that get a notification of `event` on `path`.
    pub fn notify_targets(&self, event: &RawEvent, path: &str) -> Vec<u32> {
        let kinds = event.kinds();
        self.subs.iter().filter(|(_, s)| s.wants_notify(&kinds, path)).map(|(i, _)| *i).collect()
    }

    /// The round for an authorization event on `path`: who must answer. `None` when `event` asks for no answer.
    pub fn auth_round(&self, event: &RawEvent, path: &str) -> Option<AuthRound> {
        let kind = event.auth()?;
        let asked: Vec<u32> = self.subs.iter().filter(|(_, s)| s.wants_auth(kind, path)).map(|(i, _)| *i).collect();
        Some(AuthRound { fd: event.fd, asked, answers: Vec::new() })
    }
}

/// How an authorization round ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub decision: Decision,
    /// True when the deadline passed before every subscriber answered.
    pub timed_out: bool,
}

/// The answers collected for one authorization event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthRound {
    pub fd: i32,
    asked: Vec<u32>,
    answers: Vec<(u32, Decision)>,
}

impl AuthRound {
    /// The subscribers that were asked to answer.
    pub fn asked(&self) -> &[u32] {
        &self.asked
    }

    /// Records a subscriber's answer. An answer from a subscriber that was not asked, or a second one, is ignored.
    pub fn answer(&mut self, id: u32, decision: Decision) {
        if self.asked.contains(&id) && !self.answers.iter().any(|(i, _)| *i == id) {
            self.answers.push((id, decision));
        }
    }

    /// The result so far: `None` while it can still change. A deny ends the round at once; so does the last allow, and
    /// a round nobody was asked in. `deadline_passed` ends it with what has arrived.
    pub fn outcome(&self, deadline_passed: bool) -> Option<Outcome> {
        if self.answers.iter().any(|(_, d)| *d == Decision::Deny) {
            return Some(Outcome { decision: Decision::Deny, timed_out: false });
        }
        if self.answers.len() == self.asked.len() {
            return Some(Outcome { decision: Decision::Allow, timed_out: false });
        }
        deadline_passed.then_some(Outcome { decision: Decision::Allow, timed_out: true })
    }

    /// The bytes to write to the kernel for the outcome.
    pub fn reply(&self, outcome: Outcome) -> [u8; 8] {
        response_bytes(self.fd, outcome.decision)
    }
}

/// Most path prefixes one subscription may mute.
pub const MAX_MUTED: usize = 32;
/// Longest path the wire carries.
pub const MAX_PATH: usize = 4096;

fn names(j: &Json, key: &str) -> Result<Vec<String>, String> {
    match j.get(key) {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::Arr(items)) => items.iter().map(|v| v.as_str().map(str::to_string).ok_or_else(|| format!("{key} must hold strings"))).collect(),
        Some(_) => Err(format!("{key} must be a list")),
    }
}

impl Subscription {
    /// Reads `{kinds: [..], auth: [..], muted: [..]}`. Unknown names, relative or oversized paths and an empty
    /// subscription are refused, so a damaged request never subscribes to more than it names.
    pub fn from_json(j: &Json) -> Result<Subscription, String> {
        let mut s = Subscription::default();
        for n in names(j, "kinds")? {
            let k = Kind::from_name(&n).ok_or_else(|| format!("unknown event kind {n}"))?;
            if !s.kinds.contains(&k) {
                s.kinds.push(k);
            }
        }
        for n in names(j, "auth")? {
            let k = AuthKind::from_name(&n).ok_or_else(|| format!("unknown authorization kind {n}"))?;
            if !s.auth.contains(&k) {
                s.auth.push(k);
            }
        }
        s.muted = names(j, "muted")?;
        if s.muted.len() > MAX_MUTED {
            return Err("too many muted paths".into());
        }
        if s.muted.iter().any(|m| !m.starts_with('/') || m.len() > MAX_PATH || m.contains('\0')) {
            return Err("a muted path must be absolute".into());
        }
        if s.kinds.is_empty() && s.auth.is_empty() {
            return Err("subscribe to at least one event kind".into());
        }
        Ok(s)
    }

    pub fn to_json(&self) -> Json {
        let list = |v: Vec<&str>| Json::Arr(v.into_iter().map(|n| Json::Str(n.into())).collect());
        Json::Obj(vec![
            ("kinds".into(), list(self.kinds.iter().map(|k| k.name()).collect())),
            ("auth".into(), list(self.auth.iter().map(|k| k.name()).collect())),
            ("muted".into(), Json::Arr(self.muted.iter().map(|m| Json::Str(m.clone())).collect())),
        ])
    }
}

/// One event as an app receives it: what happened, who did it, and where. `round` is set when the app must answer an
/// authorization (`auth` says which), and is the number to answer with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delivered {
    pub kinds: Vec<Kind>,
    pub auth: Option<AuthKind>,
    pub pid: i32,
    pub path: String,
    pub round: Option<u64>,
}

impl Delivered {
    pub fn to_json(&self) -> Json {
        let mut f = vec![
            ("kinds".to_string(), Json::Arr(self.kinds.iter().map(|k| Json::Str(k.name().into())).collect())),
            ("pid".to_string(), Json::Num(f64::from(self.pid))),
            ("path".to_string(), Json::Str(self.path.clone())),
        ];
        if let Some(a) = self.auth {
            f.push(("auth".into(), Json::Str(a.name().into())));
        }
        if let Some(r) = self.round {
            f.push(("round".into(), Json::Num(r as f64)));
        }
        Json::Obj(f)
    }

    pub fn from_json(j: &Json) -> Option<Delivered> {
        let kinds = j.get("kinds")?.as_array()?.iter().filter_map(|k| Kind::from_name(k.as_str()?)).collect();
        Some(Delivered {
            kinds,
            auth: j.get("auth").and_then(Json::as_str).and_then(AuthKind::from_name),
            pid: j.get("pid")?.number()? as i32,
            path: j.get("path")?.as_str()?.to_string(),
            round: j.get("round").and_then(Json::as_u64),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subscription_travels_as_json_and_comes_back_the_same() {
        let sub = Subscription { kinds: vec![Kind::Open, Kind::Exec], auth: vec![AuthKind::Exec], muted: vec!["/proc".into(), "/tmp/x".into()] };
        assert_eq!(Subscription::from_json(&sub.to_json()), Ok(sub));
    }

    #[test]
    fn a_damaged_subscription_is_refused() {
        let j = |text: &str| crate::json::parse(text).unwrap();
        assert!(Subscription::from_json(&j(r#"{"kinds":["teleport"]}"#)).is_err());
        assert!(Subscription::from_json(&j(r#"{"auth":["modify"]}"#)).is_err(), "only open and exec can be authorized");
        assert!(Subscription::from_json(&j(r#"{}"#)).is_err(), "nothing subscribed");
        assert!(Subscription::from_json(&j(r#"{"kinds":["open"],"muted":["relative/path"]}"#)).is_err());
        assert!(Subscription::from_json(&j(r#"{"kinds":"open"}"#)).is_err());
        let many = format!(r#"{{"kinds":["open"],"muted":[{}]}}"#, vec![r#""/a""#; MAX_MUTED + 1].join(","));
        assert!(Subscription::from_json(&j(&many)).is_err());
        let one = Subscription::from_json(&j(r#"{"kinds":["open","open"],"auth":["open"]}"#)).unwrap();
        assert_eq!(one.kinds, [Kind::Open]);
        assert_eq!(one.auth, [AuthKind::Open]);
    }

    #[test]
    fn a_delivered_event_round_trips() {
        let e = Delivered { kinds: vec![Kind::Open], auth: Some(AuthKind::Open), pid: 42, path: "/etc/passwd".into(), round: Some(7) };
        assert_eq!(Delivered::from_json(&e.to_json()), Some(e));
        let n = Delivered { kinds: vec![Kind::Modify, Kind::CloseWrite], auth: None, pid: 1, path: "/home/a/f".into(), round: None };
        assert_eq!(Delivered::from_json(&n.to_json()), Some(n));
    }

    #[test]
    fn the_round_names_who_was_asked() {
        let mut r = Router::new();
        let a = r.subscribe(Subscription { auth: vec![AuthKind::Exec], ..Default::default() });
        let _b = r.subscribe(Subscription { kinds: vec![Kind::Open], ..Default::default() });
        let ev = RawEvent { mask: OPEN_EXEC_PERM, fd: 5, pid: 9 };
        assert_eq!(r.auth_round(&ev, "/usr/bin/x").unwrap().asked(), [a]);
    }

    fn record(mask: u64, fd: i32, pid: i32) -> Vec<u8> {
        let mut r = Vec::new();
        r.extend_from_slice(&24u32.to_ne_bytes());
        r.push(3);
        r.push(0);
        r.extend_from_slice(&24u16.to_ne_bytes());
        r.extend_from_slice(&mask.to_ne_bytes());
        r.extend_from_slice(&fd.to_ne_bytes());
        r.extend_from_slice(&pid.to_ne_bytes());
        r
    }

    #[test]
    fn records_are_read_in_order() {
        let mut buf = record(OPEN | OPEN_EXEC, 7, 100);
        buf.extend(record(MODIFY | CLOSE_WRITE, 8, 101));
        let ev = parse_events(&buf).unwrap();
        assert_eq!(ev, [RawEvent { mask: OPEN | OPEN_EXEC, fd: 7, pid: 100 }, RawEvent { mask: MODIFY | CLOSE_WRITE, fd: 8, pid: 101 }]);
        assert_eq!(ev[0].kinds(), [Kind::Open, Kind::Exec]);
        assert_eq!(ev[1].kinds(), [Kind::Modify, Kind::CloseWrite]);
        assert_eq!(parse_events(&[]).unwrap(), []);
    }

    #[test]
    fn damaged_buffers_give_an_error_not_a_wrong_event() {
        let good = record(OPEN, 3, 1);
        assert_eq!(parse_events(&good[..20]), Err(ParseError::Length));
        let mut v2 = good.clone();
        v2[4] = 2;
        assert_eq!(parse_events(&v2), Err(ParseError::Version(2)));
        let mut long = good.clone();
        long[0..4].copy_from_slice(&64u32.to_ne_bytes());
        assert_eq!(parse_events(&long), Err(ParseError::Length), "the record claims more than the buffer holds");
        let mut short = good.clone();
        short[6..8].copy_from_slice(&8u16.to_ne_bytes());
        assert_eq!(parse_events(&short), Err(ParseError::Length), "metadata shorter than the fixed part");
        let mut inverted = good;
        inverted[0..4].copy_from_slice(&16u32.to_ne_bytes());
        assert_eq!(parse_events(&inverted), Err(ParseError::Length));
    }

    #[test]
    fn authorization_and_overflow_bits_are_told_apart() {
        let e = RawEvent { mask: OPEN_PERM, fd: 4, pid: 9 };
        assert_eq!((e.auth(), e.kinds()), (Some(AuthKind::Open), vec![]));
        assert_eq!(RawEvent { mask: OPEN_PERM | OPEN_EXEC_PERM, fd: 4, pid: 9 }.auth(), Some(AuthKind::Exec));
        assert_eq!(RawEvent { mask: OPEN, fd: 4, pid: 9 }.auth(), None);
        assert!(RawEvent { mask: Q_OVERFLOW, fd: -1, pid: 0 }.overflowed());
    }

    #[test]
    fn muting_stops_at_a_path_boundary() {
        let s = Subscription { kinds: vec![Kind::Open], muted: vec!["/home/a/".into(), "".into()], ..Default::default() };
        assert!(!s.wants_notify(&[Kind::Open], "/home/a/x"));
        assert!(!s.wants_notify(&[Kind::Open], "/home/a"));
        assert!(s.wants_notify(&[Kind::Open], "/home/ab/x"), "an empty or longer-named prefix mutes nothing else");
        assert!(!s.wants_notify(&[Kind::Modify], "/etc/passwd"), "kind not asked for");
    }

    #[test]
    fn the_router_targets_subscribers_and_covers_their_masks() {
        let mut r = Router::new();
        let av = r.subscribe(Subscription { kinds: vec![Kind::Open, Kind::Modify], ..Default::default() });
        let audit = r.subscribe(Subscription { kinds: vec![Kind::Exec], muted: vec!["/usr".into()], ..Default::default() });
        assert_eq!(r.mask(), OPEN | MODIFY | OPEN_EXEC);
        let ev = RawEvent { mask: OPEN | OPEN_EXEC, fd: 5, pid: 2 };
        assert_eq!(r.notify_targets(&ev, "/tmp/x"), [av, audit]);
        assert_eq!(r.notify_targets(&ev, "/usr/bin/ls"), [av]);
        assert!(r.unsubscribe(av) && !r.unsubscribe(av));
        assert_eq!(r.notify_targets(&ev, "/tmp/x"), [audit]);
    }

    fn exec_event() -> RawEvent {
        RawEvent { mask: OPEN_EXEC_PERM, fd: 12, pid: 77 }
    }

    #[test]
    fn one_deny_denies_and_ends_the_round_at_once() {
        let mut r = Router::new();
        let a = r.subscribe(Subscription { auth: vec![AuthKind::Exec], ..Default::default() });
        let b = r.subscribe(Subscription { auth: vec![AuthKind::Exec], ..Default::default() });
        let mut round = r.auth_round(&exec_event(), "/tmp/evil").unwrap();
        assert_eq!(round.outcome(false), None);
        round.answer(a, Decision::Allow);
        assert_eq!(round.outcome(false), None, "b has not answered");
        round.answer(b, Decision::Deny);
        let o = round.outcome(false).unwrap();
        assert_eq!(o, Outcome { decision: Decision::Deny, timed_out: false });
        assert_eq!(round.reply(o), response_bytes(12, Decision::Deny));
    }

    #[test]
    fn all_allows_allow_and_a_missing_answer_allows_at_the_deadline() {
        let mut r = Router::new();
        let a = r.subscribe(Subscription { auth: vec![AuthKind::Exec], ..Default::default() });
        let b = r.subscribe(Subscription { auth: vec![AuthKind::Exec], ..Default::default() });
        let mut round = r.auth_round(&exec_event(), "/bin/ls").unwrap();
        round.answer(a, Decision::Allow);
        round.answer(a, Decision::Deny);
        round.answer(99, Decision::Deny);
        assert_eq!(round.outcome(true), Some(Outcome { decision: Decision::Allow, timed_out: true }));
        round.answer(b, Decision::Allow);
        assert_eq!(round.outcome(false), Some(Outcome { decision: Decision::Allow, timed_out: false }));
    }

    #[test]
    fn a_round_nobody_is_asked_in_allows_and_a_plain_event_has_no_round() {
        let r = Router::new();
        assert_eq!(r.auth_round(&exec_event(), "/x").unwrap().outcome(false), Some(Outcome { decision: Decision::Allow, timed_out: false }));
        assert_eq!(r.auth_round(&RawEvent { mask: OPEN, fd: 1, pid: 1 }, "/x"), None);
    }

    #[test]
    fn the_reply_is_the_descriptor_then_the_code() {
        let b = response_bytes(0x0102, Decision::Deny);
        assert_eq!(i32::from_ne_bytes(b[..4].try_into().unwrap()), 0x0102);
        assert_eq!(u32::from_ne_bytes(b[4..].try_into().unwrap()), 2);
        assert_eq!(u32::from_ne_bytes(response_bytes(1, Decision::Allow)[4..].try_into().unwrap()), 1);
    }
}
