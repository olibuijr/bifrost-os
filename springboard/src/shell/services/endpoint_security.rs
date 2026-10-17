//! The `endpoint-security` service: file open, exec, modify and close events for a security app, from the root service
//! behind `bifrost-privd` (`privd/endpoint_security.rs`, fanotify). The model is `bifrost_core::endpoint_security`.
//!
//! Who may use it: only an installed app that declares `entitlements = ["endpoint-security"]` in its manifest, and
//! only a first-party system app (the rule of every entitlement, `app_policy::Policy::with_entitlements`). Any other
//! app gets an error from every method. The privilege itself stays in privd, which serves only the shell; an app
//! never holds the descriptor or the socket.
//!
//! Methods (params are a JSON object):
//! - `subscribe {kinds, auth?, muted?}` -> `{}`: `kinds` are `access`, `modify`, `close_write`, `close_nowrite`, `open`,
//!   `exec`; `auth` are `open` and `exec` (the app is asked before the file opens or runs); `muted` are absolute path
//!   prefixes whose events the app never gets. One subscription per app.
//! - `poll {max?}` -> `{events: [{kinds, pid, path, auth?, round?}], dropped}`: what has queued, oldest first. An event
//!   with a `round` waits for `answer`.
//! - `answer {round, allow}` -> `{resolved}`: allows or denies an authorization. One deny denies; an authorization not
//!   answered within two seconds is allowed.
//! - `unsubscribe {}` -> `{}`.
//!
//! A subscription ends when the app exits (this service checks each tick) and privd allows what the app still owed.
//! Only the path and process id of an event leave the root service; file contents are never read.
use super::{app_pid, Event, Shell, SystemService};
use bifrost_core::endpoint_security::{Delivered, Subscription, ENTITLEMENT};
use bifrost_core::json::Json;
use std::collections::BTreeMap;

/// The name the service is registered under.
pub const SERVICE: &str = "endpoint-security";

/// The root side of the feed, as this service uses it (privd in the shell, a fake in the tests).
pub trait Backend {
    fn subscribe(&self, app: &str, sub: &Subscription) -> Result<u32, String>;
    fn unsubscribe(&self, app: &str, id: u32) -> Result<(), String>;
    fn poll(&self, app: &str, id: u32, max: usize) -> Result<(Vec<Delivered>, u64), String>;
    fn answer(&self, app: &str, id: u32, round: u64, allow: bool) -> Result<bool, String>;
}

/// The real backend: privd's `endpoint_security.*` methods.
struct Privd;

impl Backend for Privd {
    fn subscribe(&self, app: &str, sub: &Subscription) -> Result<u32, String> {
        crate::privd_client::connect().endpoint_subscribe(app, sub).map_err(|e| e.message)
    }
    fn unsubscribe(&self, app: &str, id: u32) -> Result<(), String> {
        crate::privd_client::connect().endpoint_unsubscribe(app, id).map_err(|e| e.message)
    }
    fn poll(&self, app: &str, id: u32, max: usize) -> Result<(Vec<Delivered>, u64), String> {
        crate::privd_client::connect().endpoint_poll(app, id, max).map_err(|e| e.message)
    }
    fn answer(&self, app: &str, id: u32, round: u64, allow: bool) -> Result<bool, String> {
        crate::privd_client::connect().endpoint_answer(app, id, round, allow).map_err(|e| e.message)
    }
}

pub struct EndpointSecurity {
    backend: Box<dyn Backend>,
    /// whether an app may hold the entitlement
    entitled: Box<dyn Fn(&str) -> bool>,
    /// the app -> its subscription id at privd
    subs: BTreeMap<String, u32>,
}

impl Default for EndpointSecurity {
    fn default() -> Self {
        EndpointSecurity::with(Box::new(Privd), Box::new(|app| super::app_has_entitlement(app, ENTITLEMENT)))
    }
}

impl EndpointSecurity {
    pub fn with(backend: Box<dyn Backend>, entitled: Box<dyn Fn(&str) -> bool>) -> Self {
        EndpointSecurity { backend, entitled, subs: BTreeMap::new() }
    }

    fn sub_of(&self, app: &str) -> Result<u32, String> {
        self.subs.get(app).copied().ok_or_else(|| "endpoint-security: subscribe first".to_string())
    }

    /// Runs one call for `app_id`. Split from [`SystemService::call`] so the tests need no shell.
    pub fn handle(&mut self, app_id: &str, method: &str, params: &Json) -> Result<Json, String> {
        // an app's id is never empty; the shell's own calls are not for this service
        if app_id.is_empty() {
            return Err("endpoint-security: this service is for apps".into());
        }
        if !(self.entitled)(app_id) {
            return Err(format!("endpoint-security: {app_id} does not declare the {ENTITLEMENT} entitlement"));
        }
        match method {
            "subscribe" => {
                if self.subs.contains_key(app_id) {
                    return Err("endpoint-security: already subscribed".into());
                }
                let sub = Subscription::from_json(params).map_err(|e| format!("endpoint-security: {e}"))?;
                let id = self.backend.subscribe(app_id, &sub)?;
                self.subs.insert(app_id.to_string(), id);
                Ok(Json::Obj(vec![]))
            }
            "poll" => {
                let id = self.sub_of(app_id)?;
                let max = params.get("max").and_then(Json::as_u64).map_or(256, |m| m as usize);
                let (events, dropped) = self.backend.poll(app_id, id, max)?;
                Ok(Json::Obj(vec![
                    ("events".into(), Json::Arr(events.iter().map(Delivered::to_json).collect())),
                    ("dropped".into(), Json::Num(dropped as f64)),
                ]))
            }
            "answer" => {
                let id = self.sub_of(app_id)?;
                let round = params.get("round").and_then(Json::as_u64).ok_or("endpoint-security: answer needs a round")?;
                let allow = match params.get("allow") {
                    Some(Json::Bool(b)) => *b,
                    _ => return Err("endpoint-security: allow must be true or false".into()),
                };
                let resolved = self.backend.answer(app_id, id, round, allow)?;
                Ok(Json::Obj(vec![("resolved".into(), Json::Bool(resolved))]))
            }
            "unsubscribe" => {
                let id = self.sub_of(app_id)?;
                self.backend.unsubscribe(app_id, id)?;
                self.subs.remove(app_id);
                Ok(Json::Obj(vec![]))
            }
            other => Err(format!("endpoint-security: unknown method {other}")),
        }
    }

    /// Ends the subscriptions of apps that are no longer running. `running` says whether an app is.
    pub fn reap(&mut self, running: impl Fn(&str) -> bool) {
        let gone: Vec<String> = self.subs.keys().filter(|app| !running(app)).cloned().collect();
        for app in gone {
            if let Some(id) = self.subs.remove(&app) {
                let _ = self.backend.unsubscribe(&app, id);
            }
        }
    }
}

impl SystemService for EndpointSecurity {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        self.handle(app_id, method, params)
    }

    fn tick(&mut self, shell: &mut Shell, _t: f64) -> Vec<Event> {
        if !self.subs.is_empty() {
            self.reap(|app| app_pid(shell, app).is_some());
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::endpoint_security::{AuthKind, Kind};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    struct Log {
        subscribed: Vec<(String, Subscription)>,
        unsubscribed: Vec<(String, u32)>,
        answers: Vec<(u32, u64, bool)>,
        queue: Vec<Delivered>,
    }

    struct Fake(Rc<RefCell<Log>>);

    impl Backend for Fake {
        fn subscribe(&self, app: &str, sub: &Subscription) -> Result<u32, String> {
            let mut l = self.0.borrow_mut();
            l.subscribed.push((app.to_string(), sub.clone()));
            Ok(l.subscribed.len() as u32)
        }
        fn unsubscribe(&self, app: &str, id: u32) -> Result<(), String> {
            self.0.borrow_mut().unsubscribed.push((app.to_string(), id));
            Ok(())
        }
        fn poll(&self, _app: &str, _id: u32, max: usize) -> Result<(Vec<Delivered>, u64), String> {
            let mut l = self.0.borrow_mut();
            let n = max.min(l.queue.len());
            Ok((l.queue.drain(..n).collect(), 2))
        }
        fn answer(&self, _app: &str, id: u32, round: u64, allow: bool) -> Result<bool, String> {
            self.0.borrow_mut().answers.push((id, round, allow));
            Ok(true)
        }
    }

    fn service(entitled: &'static [&'static str]) -> (EndpointSecurity, Rc<RefCell<Log>>) {
        let log = Rc::new(RefCell::new(Log::default()));
        let s = EndpointSecurity::with(Box::new(Fake(Rc::clone(&log))), Box::new(move |app| entitled.iter().any(|e| *e == app)));
        (s, log)
    }

    fn j(text: &str) -> Json {
        bifrost_core::json::parse(text).unwrap()
    }

    const GUARD: &str = "is.olibuijr.guard";

    #[test]
    fn an_app_without_the_entitlement_gets_nothing_and_reaches_nothing() {
        let (mut s, log) = service(&[GUARD]);
        let err = s.handle("is.example.snoop", "subscribe", &j(r#"{"kinds":["open"]}"#)).unwrap_err();
        assert!(err.contains("endpoint-security entitlement"), "{err}");
        for m in ["poll", "answer", "unsubscribe", "nope"] {
            assert!(s.handle("is.example.snoop", m, &j("{}")).is_err(), "{m}");
        }
        assert!(log.borrow().subscribed.is_empty(), "privd was never asked");
        assert!(s.handle("", "subscribe", &j(r#"{"kinds":["open"]}"#)).is_err(), "the shell's own id is refused");
    }

    #[test]
    fn an_entitled_app_subscribes_polls_answers_and_leaves() {
        let (mut s, log) = service(&[GUARD]);
        s.handle(GUARD, "subscribe", &j(r#"{"kinds":["open","modify"],"auth":["exec"],"muted":["/proc"]}"#)).unwrap();
        assert_eq!(log.borrow().subscribed[0].0, GUARD);
        assert_eq!(log.borrow().subscribed[0].1, Subscription { kinds: vec![Kind::Open, Kind::Modify], auth: vec![AuthKind::Exec], muted: vec!["/proc".into()] });
        assert!(s.handle(GUARD, "subscribe", &j(r#"{"kinds":["open"]}"#)).is_err(), "one subscription per app");

        log.borrow_mut().queue = vec![
            Delivered { kinds: vec![Kind::Open], auth: None, pid: 10, path: "/etc/shadow".into(), round: None },
            Delivered { kinds: vec![], auth: Some(AuthKind::Exec), pid: 11, path: "/tmp/x".into(), round: Some(4) },
        ];
        let r = s.handle(GUARD, "poll", &j(r#"{"max":10}"#)).unwrap();
        let events = r.get("events").and_then(Json::as_array).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].get("round").and_then(Json::as_u64), Some(4));
        assert_eq!(r.get("dropped").and_then(Json::as_u64), Some(2));

        let a = s.handle(GUARD, "answer", &j(r#"{"round":4,"allow":false}"#)).unwrap();
        assert_eq!(a.get("resolved"), Some(&Json::Bool(true)));
        assert_eq!(log.borrow().answers, [(1, 4, false)]);
        assert!(s.handle(GUARD, "answer", &j(r#"{"round":4}"#)).is_err(), "allow is required");

        s.handle(GUARD, "unsubscribe", &j("{}")).unwrap();
        assert_eq!(log.borrow().unsubscribed, [(GUARD.to_string(), 1)]);
        assert!(s.handle(GUARD, "poll", &j("{}")).is_err(), "subscribe first");
    }

    #[test]
    fn a_damaged_subscription_never_reaches_privd() {
        let (mut s, log) = service(&[GUARD]);
        assert!(s.handle(GUARD, "subscribe", &j(r#"{"kinds":["teleport"]}"#)).is_err());
        assert!(s.handle(GUARD, "subscribe", &j("{}")).is_err());
        assert!(log.borrow().subscribed.is_empty());
    }

    #[test]
    fn a_subscription_ends_when_its_app_stops_running() {
        let (mut s, log) = service(&[GUARD, "is.olibuijr.other"]);
        s.handle(GUARD, "subscribe", &j(r#"{"kinds":["open"]}"#)).unwrap();
        s.handle("is.olibuijr.other", "subscribe", &j(r#"{"kinds":["exec"]}"#)).unwrap();
        s.reap(|app| app == "is.olibuijr.other");
        assert_eq!(log.borrow().unsubscribed, [(GUARD.to_string(), 1)]);
        assert!(s.handle(GUARD, "poll", &j("{}")).is_err());
        assert!(s.handle("is.olibuijr.other", "poll", &j("{}")).is_ok());
    }

    #[test]
    fn the_entitlement_is_first_party_system_apps_only() {
        let held = |id: &str, kind: &str, declared: &[&str]| super::super::entitlement_held(id, kind, &declared.iter().map(|d| d.to_string()).collect::<Vec<_>>(), ENTITLEMENT);
        assert!(held("is.olibuijr.guard", "system-app", &["endpoint-security"]));
        assert!(!held("is.olibuijr.guard", "system-app", &["log-store"]), "not declared");
        assert!(!held("is.olibuijr.guard", "app", &["endpoint-security"]), "a store app cannot hold it");
        assert!(!held("is.example.guard", "system-app", &["endpoint-security"]), "a third party cannot hold it");
    }

    #[test]
    fn the_shell_serves_it_and_refuses_an_app_that_is_not_installed_with_the_entitlement() {
        let mut sh = crate::shell::tests::shell();
        let err = sh.service_call("is.example.snoop", SERVICE, "subscribe", r#"{"kinds":["open"]}"#, 0.0).unwrap_err();
        assert!(err.contains("entitlement"), "{err}");
        let err = sh.service_call("is.olibuijr.not-installed", SERVICE, "poll", "{}", 0.0).unwrap_err();
        assert!(err.contains("entitlement"), "{err}");
    }
}
