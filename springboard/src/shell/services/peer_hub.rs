//! The `peer` service: connection-based messages between apps, the shell's side of XPC-style services. The model is
//! `bifrost_core::peer_session` (listeners, sessions, requests and replies). The hub lives in the shell process, so
//! every session joins two apps that run under this shell, with no sockets.
//!
//! Naming: a listener is always named by its owner. An app that listens as `chat` is known to other apps as
//! `<its app id>/chat`, so no app can take another app's name. Only the owner can accept on it.
//!
//! Methods (JSON parameters; a session is a handle number, and only the app that holds a handle can use it; message
//! bodies are hex):
//! - `listen {name, requirement?}` registers `<app>/<name>`. `requirement` is `"same_user"` (the default), `"any"`, or
//!   `{"app": "<id>"}`, the peer app that may connect.
//! - `connect {owner, name}` opens a session to `<owner>/<name>` and answers `{session}`. A refused peer is an error.
//! - `accept {name}` takes a pending connection to one of the app's own listeners and answers `{session}`, or `null`
//!   when nobody is waiting.
//! - `send {session, body, wants_reply?}` answers `{message}`, the id to match replies with.
//! - `receive {session}` answers `{message}`: `{id, body, wants_reply}` for the next message waiting for this app, or
//!   `null`.
//! - `reply {session, message, body}` answers the message this app received with `wants_reply`.
//! - `take_reply {session, message}` answers `{body}` once the peer has replied, or `null` while it has not.
//! - `close {session}` ends the session. Messages already queued can still be received.
//!
//! Not built: sessions that outlive the shell, the code-signing and entitlement check (a peer is identified by its app
//! id and user id only), and starting a service on demand.
use super::{Shell, SystemService};
use crate::keychain::{hex, unhex};
use bifrost_core::json::Json;
use bifrost_core::peer_session::{Hub, Peer, Requirement, ReceivedMessage, Side, SessionId, Target};

/// The service name.
pub const SERVICE: &str = "peer";

/// One session handle the apps hold: who holds it, which end it is, and the session while it is open.
struct Handle {
    app: String,
    side: Side,
    session: Option<SessionId>,
}

/// The `peer` service.
pub struct PeerHub {
    hub: Hub,
    handles: Vec<Handle>,
}

impl PeerHub {
    pub fn new() -> Self {
        Self { hub: Hub::new(), handles: Vec::new() }
    }

    /// Runs one method for the app `app`.
    fn answer(&mut self, app: &str, method: &str, params: &Json) -> Result<Json, String> {
        match method {
            "listen" => {
                let name = text(params, "name")?;
                if name.is_empty() || name.contains('/') {
                    return Err("a listener name must be non-empty and have no '/'".into());
                }
                let requirement = requirement(params)?;
                self.hub.listen(&format!("{app}/{name}"), requirement).map_err(|e| format!("{e:?}"))?;
                Ok(Json::Obj(vec![("listening".into(), Json::Bool(true))]))
            }
            "connect" => {
                let target = Target::Named(format!("{}/{}", text(params, "owner")?, text(params, "name")?));
                let peer = Peer { uid: own_uid(), app_id: app.to_string() };
                let session = self.hub.connect(&target, peer).map_err(|e| format!("{e:?}"))?;
                Ok(self.hold(app, Side::Client, session))
            }
            "accept" => {
                let target = Target::Named(format!("{app}/{}", text(params, "name")?));
                Ok(match self.hub.accept(&target) {
                    Some(session) => self.hold(app, Side::Server, session),
                    None => Json::Obj(vec![("session".into(), Json::Null)]),
                })
            }
            "send" => {
                let (session, side) = self.open(app, params)?;
                let body = bytes(params, "body")?;
                let wants = params.get("wants_reply") == Some(&Json::Bool(true));
                let message = self.hub.send(session, side, &body, wants).map_err(|e| format!("{e:?}"))?;
                Ok(Json::Obj(vec![("message".into(), Json::Num(message as f64))]))
            }
            "receive" => {
                let (session, side) = self.open(app, params)?;
                let next = self.hub.receive(session, side).map_err(|e| format!("{e:?}"))?;
                Ok(Json::Obj(vec![("message".into(), next.map_or(Json::Null, message_json))]))
            }
            "reply" => {
                let (session, side) = self.open(app, params)?;
                let message = whole(params, "message")?;
                let body = bytes(params, "body")?;
                self.hub.reply(session, side, message, &body).map_err(|e| format!("{e:?}"))?;
                Ok(Json::Obj(vec![("ok".into(), Json::Bool(true))]))
            }
            "take_reply" => {
                let (session, side) = self.open(app, params)?;
                let message = whole(params, "message")?;
                let body = self.hub.take_reply(session, side, message).map_err(|e| format!("{e:?}"))?;
                Ok(Json::Obj(vec![("body".into(), body.map_or(Json::Null, |b| Json::Str(hex(&b))))]))
            }
            "close" => {
                let (session, _) = self.open(app, params)?;
                self.hub.close(session).map_err(|e| format!("{e:?}"))?;
                if let Some(h) = self.handles.iter_mut().find(|h| h.session == Some(session)) {
                    h.session = None;
                }
                Ok(Json::Obj(vec![("ok".into(), Json::Bool(true))]))
            }
            _ => Err(format!("peer has no method {method}")),
        }
    }

    /// Gives the app a handle for a session and answers it.
    fn hold(&mut self, app: &str, side: Side, session: SessionId) -> Json {
        self.handles.push(Handle { app: app.to_string(), side, session: Some(session) });
        Json::Obj(vec![("session".into(), Json::Num((self.handles.len() - 1) as f64))])
    }

    /// The open session and this app's side behind a handle, when the app holds it.
    fn open(&self, app: &str, params: &Json) -> Result<(SessionId, Side), String> {
        let n = whole(params, "session")? as usize;
        match self.handles.get(n) {
            Some(h) if h.app == app => h.session.map(|s| (s, h.side)).ok_or_else(|| "that session is closed".to_string()),
            _ => Err("no such session for this app".to_string()),
        }
    }
}

impl SystemService for PeerHub {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        self.answer(app_id, method, params)
    }
}

/// The listener's requirement from the request. Without one, only the same user may connect.
fn requirement(params: &Json) -> Result<Requirement, String> {
    match params.get("requirement") {
        None => Ok(Requirement::SameUser(own_uid())),
        Some(Json::Str(s)) if s == "same_user" => Ok(Requirement::SameUser(own_uid())),
        Some(Json::Str(s)) if s == "any" => Ok(Requirement::Any),
        Some(Json::Obj(_)) => {
            let app = params.get("requirement").and_then(|r| r.str_field("app")).ok_or("requirement.app must be an app id")?;
            Ok(Requirement::App(app))
        }
        Some(_) => Err("requirement must be \"same_user\", \"any\" or {\"app\": id}".into()),
    }
}

/// The user id this shell runs as, read from `/proc`. When it cannot be read, no peer matches a same-user rule.
fn own_uid() -> u32 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("Uid:")).and_then(|l| l.split_whitespace().nth(1).map(str::to_string)))
        .and_then(|n| n.parse().ok())
        .unwrap_or(u32::MAX)
}

fn text(params: &Json, key: &str) -> Result<String, String> {
    params.str_field(key).ok_or_else(|| format!("{key} must be a string"))
}

fn whole(params: &Json, key: &str) -> Result<u64, String> {
    params.get(key).and_then(Json::as_u64).ok_or_else(|| format!("{key} must be a whole number"))
}

fn bytes(params: &Json, key: &str) -> Result<Vec<u8>, String> {
    unhex(&text(params, key)?).ok_or_else(|| format!("{key} must be hex"))
}

fn message_json(m: ReceivedMessage) -> Json {
    Json::Obj(vec![
        ("id".into(), Json::Num(m.id as f64)),
        ("body".into(), Json::Str(hex(&m.body))),
        ("wants_reply".into(), Json::Bool(m.wants_reply)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(pairs: Vec<(&str, Json)>) -> Json {
        Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    fn handle_of(r: &Json) -> f64 {
        r.get("session").and_then(Json::number).unwrap()
    }

    #[test]
    fn request_and_reply_between_two_apps() {
        let mut hub = PeerHub::new();
        let listen = obj(vec![("name", Json::Str("chat".into())), ("requirement", Json::Str("any".into()))]);
        hub.answer("owner.app", "listen", &listen).unwrap();
        let connect = obj(vec![("owner", Json::Str("owner.app".into())), ("name", Json::Str("chat".into()))]);
        let client = hub.answer("client.app", "connect", &connect).unwrap();
        let accepted = hub.answer("owner.app", "accept", &obj(vec![("name", Json::Str("chat".into()))])).unwrap();
        let server = handle_of(&accepted);

        let send = obj(vec![("session", Json::Num(handle_of(&client))), ("body", Json::Str("6869".into())), ("wants_reply", Json::Bool(true))]);
        let sent = hub.answer("client.app", "send", &send).unwrap();
        let message = sent.get("message").and_then(Json::number).unwrap();

        let got = hub.answer("owner.app", "receive", &obj(vec![("session", Json::Num(server))])).unwrap();
        let m = got.get("message").unwrap();
        assert_eq!(m.get("body").and_then(Json::as_str), Some("6869"));

        let reply = obj(vec![("session", Json::Num(server)), ("message", Json::Num(message)), ("body", Json::Str("6f6b".into()))]);
        hub.answer("owner.app", "reply", &reply).unwrap();
        let take = obj(vec![("session", Json::Num(handle_of(&client))), ("message", Json::Num(message))]);
        let answer = hub.answer("client.app", "take_reply", &take).unwrap();
        assert_eq!(answer.get("body").and_then(Json::as_str), Some("6f6b"));
    }

    #[test]
    fn only_the_holder_of_a_handle_can_use_it() {
        let mut hub = PeerHub::new();
        hub.answer("owner.app", "listen", &obj(vec![("name", Json::Str("svc".into())), ("requirement", Json::Str("any".into()))])).unwrap();
        let client = hub.answer("client.app", "connect", &obj(vec![("owner", Json::Str("owner.app".into())), ("name", Json::Str("svc".into()))])).unwrap();
        let steal = obj(vec![("session", Json::Num(handle_of(&client))), ("body", Json::Str("00".into()))]);
        assert!(hub.answer("intruder.app", "send", &steal).is_err());
    }

    #[test]
    fn listeners_are_named_by_their_owner_and_bad_names_are_refused() {
        let mut hub = PeerHub::new();
        assert!(hub.answer("owner.app", "listen", &obj(vec![("name", Json::Str("a/b".into()))])).is_err());
        assert!(hub.answer("owner.app", "listen", &obj(vec![("name", Json::Str(String::new()))])).is_err());
        // a connect to a name nobody listens on is refused
        let nowhere = obj(vec![("owner", Json::Str("owner.app".into())), ("name", Json::Str("none".into()))]);
        assert!(hub.answer("client.app", "connect", &nowhere).is_err());
        assert!(hub.answer("owner.app", "accept", &obj(vec![("name", Json::Str("none".into()))])).unwrap().get("session") == Some(&Json::Null));
    }
}
