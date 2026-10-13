//! Event IPC (`AppEvent`, `AppEventReply`, `AppEventHandlers`): typed requests to
//! another app with a reply descriptor and a timeout, carried over the app control socket (JSON-RPC, see
//! `control`). An event is an (event class, event id) pair plus a parameter record; it travels as the method
//! `ae.<class>.<id>` with the record as `params`. The reply descriptor holds the handler's result or its error
//! (`errAEEventNotHandled` when the target has no handler for the event).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use bifrost_core::json::{self, Json};

use crate::control::{RpcError, METHOD_NOT_FOUND};

/// `errAEEventNotHandled`.
pub const ERR_EVENT_NOT_HANDLED: i64 = -1708;
/// `errAETimeout`.
pub const ERR_TIMEOUT: i64 = -1712;
/// `connectionInvalid` style failure: the target socket could not be reached.
pub const ERR_TARGET_UNREACHABLE: i64 = -609;

/// A typed event (`AppEvent`): class, id and parameter record (`AEDesc` record).
#[derive(Clone, Debug, PartialEq)]
pub struct AppEvent {
    pub class: String,
    pub id: String,
    pub params: Json,
}

impl AppEvent {
    /// `AECreateAppleEvent`.
    pub fn new(class: &str, id: &str, params: Json) -> AppEvent {
        AppEvent { class: class.into(), id: id.into(), params }
    }

    /// The control-socket method this event travels as.
    pub fn method(&self) -> String {
        format!("ae.{}.{}", self.class, self.id)
    }

    /// Reads an event back from a method name and params; None when it is not an `ae.` method.
    pub fn from_method(method: &str, params: &Json) -> Option<AppEvent> {
        let rest = method.strip_prefix("ae.")?;
        let (class, id) = rest.split_once('.')?;
        if class.is_empty() || id.is_empty() {
            return None;
        }
        Some(AppEvent::new(class, id, params.clone()))
    }
}

/// The reply descriptor: an error number and a result record.
#[derive(Clone, Debug, PartialEq)]
pub struct AppEventReply {
    pub error_number: i64,
    pub error_string: Option<String>,
    pub result: Json,
}

impl AppEventReply {
    pub fn ok(&self) -> bool {
        self.error_number == 0
    }
}

fn failed(code: i64, text: &str) -> AppEventReply {
    AppEventReply { error_number: code, error_string: Some(text.into()), result: Json::Null }
}

/// `AESendMessage` with `kAEWaitReply`: sends `event` to the app listening at `socket` and waits up to `timeout`.
pub fn send_message(socket: &Path, event: &AppEvent, timeout: Duration) -> AppEventReply {
    let Ok(mut stream) = UnixStream::connect(socket) else {
        return failed(ERR_TARGET_UNREACHABLE, "target app is not reachable");
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    let req = Json::Obj(vec![
        ("jsonrpc".into(), Json::Str("2.0".into())),
        ("id".into(), Json::Num(1.0)),
        ("method".into(), Json::Str(event.method())),
        ("params".into(), event.params.clone()),
    ]);
    if stream.write_all(format!("{}\n", req.encode()).as_bytes()).is_err() {
        return failed(ERR_TARGET_UNREACHABLE, "send failed");
    }
    let mut line = String::new();
    match BufReader::new(stream).read_line(&mut line) {
        Ok(n) if n > 0 => parse_reply(&line),
        Ok(_) => failed(ERR_TARGET_UNREACHABLE, "target closed without a reply"),
        Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
            failed(ERR_TIMEOUT, "timed out waiting for the reply")
        }
        Err(_) => failed(ERR_TARGET_UNREACHABLE, "read failed"),
    }
}

/// Turns one JSON-RPC reply line into a reply descriptor.
pub fn parse_reply(line: &str) -> AppEventReply {
    let Ok(v) = json::parse(line.trim()) else {
        return failed(ERR_TARGET_UNREACHABLE, "malformed reply");
    };
    if let Some(err) = v.get("error") {
        let code = err.get("code").and_then(|c| c.number()).unwrap_or(-1.0) as i64;
        let msg = err.str_field("message");
        let number = if code == METHOD_NOT_FOUND { ERR_EVENT_NOT_HANDLED } else { code };
        return AppEventReply { error_number: number, error_string: msg, result: Json::Null };
    }
    AppEventReply { error_number: 0, error_string: None, result: v.get("result").cloned().unwrap_or(Json::Null) }
}

type EventHandler = Box<dyn FnMut(&AppEvent) -> Result<Json, RpcError>>;

/// The receiving side (`AEInstallEventHandler`): handlers per (class, id). Plug `process` into `Application::rpc`.
#[derive(Default)]
pub struct AppEventHandlers {
    handlers: Vec<(String, String, EventHandler)>,
}

impl AppEventHandlers {
    pub fn new() -> Self {
        Self::default()
    }

    /// `AEInstallEventHandler`: replaces any handler for the same class and id.
    pub fn install(&mut self, class: &str, id: &str, handler: impl FnMut(&AppEvent) -> Result<Json, RpcError> + 'static) {
        self.remove(class, id);
        self.handlers.push((class.into(), id.into(), Box::new(handler)));
    }

    /// `AERemoveEventHandler`.
    pub fn remove(&mut self, class: &str, id: &str) {
        self.handlers.retain(|(c, i, _)| !(c == class && i == id));
    }

    /// `AEProcessAppleEvent`.
    pub fn process(&mut self, event: &AppEvent) -> Result<Json, RpcError> {
        match self.handlers.iter_mut().find(|(c, i, _)| *c == event.class && *i == event.id) {
            Some((_, _, h)) => h(event),
            None => Err(RpcError::new(METHOD_NOT_FOUND, format!("no handler for {}", event.method()))),
        }
    }

    /// For `Application::rpc`: serves `ae.*` methods, answers METHOD_NOT_FOUND for the rest.
    pub fn rpc(&mut self, method: &str, params: &Json) -> Result<Json, RpcError> {
        match AppEvent::from_method(method, params) {
            Some(ev) => self.process(&ev),
            None => Err(RpcError::new(METHOD_NOT_FOUND, format!("no method {method}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::net::UnixListener;

    fn obj(k: &str, v: &str) -> Json {
        Json::Obj(vec![(k.into(), Json::Str(v.into()))])
    }

    #[test]
    fn event_round_trips_through_method_name() {
        let ev = AppEvent::new("core", "open", obj("path", "/a"));
        assert_eq!(ev.method(), "ae.core.open");
        assert_eq!(AppEvent::from_method("ae.core.open", &ev.params), Some(ev));
        assert_eq!(AppEvent::from_method("app.info", &Json::Null), None);
        assert_eq!(AppEvent::from_method("ae.core.", &Json::Null), None);
    }

    #[test]
    fn handlers_dispatch_by_class_and_id() {
        let mut h = AppEventHandlers::new();
        h.install("core", "open", |ev| Ok(ev.params.clone()));
        let r = h.rpc("ae.core.open", &obj("x", "1")).unwrap();
        assert_eq!(r, obj("x", "1"));
        assert_eq!(h.rpc("ae.core.quit", &Json::Null).unwrap_err().code, METHOD_NOT_FOUND);
        h.remove("core", "open");
        assert!(h.rpc("ae.core.open", &Json::Null).is_err());
    }

    #[test]
    fn parse_reply_maps_result_and_errors() {
        let ok = parse_reply("{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"n\":1}}");
        assert!(ok.ok());
        assert_eq!(ok.result.get("n").and_then(|n| n.number()), Some(1.0));
        let nf = parse_reply("{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{\"code\":-32601,\"message\":\"no\"}}");
        assert_eq!(nf.error_number, ERR_EVENT_NOT_HANDLED);
        assert_eq!(parse_reply("garbage").error_number, ERR_TARGET_UNREACHABLE);
    }

    #[test]
    fn send_message_gets_a_reply_from_a_socket() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("c.sock");
        let l = UnixListener::bind(&path).unwrap();
        let t = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut line = String::new();
            BufReader::new(s.try_clone().unwrap()).read_line(&mut line).unwrap();
            assert!(line.contains("\"method\":\"ae.core.open\""));
            s.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"opened\":true}}\n").unwrap();
        });
        let r = send_message(&path, &AppEvent::new("core", "open", Json::Obj(vec![])), Duration::from_secs(5));
        t.join().unwrap();
        assert!(r.ok());
        assert_eq!(r.result.get("opened"), Some(&Json::Bool(true)));
    }

    #[test]
    fn send_message_times_out_without_a_reply() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("c.sock");
        let l = UnixListener::bind(&path).unwrap();
        let t = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut sink = [0u8; 256];
            let _ = s.read(&mut sink);
            std::thread::sleep(Duration::from_millis(400));
        });
        let r = send_message(&path, &AppEvent::new("core", "slow", Json::Null), Duration::from_millis(100));
        t.join().unwrap();
        assert_eq!(r.error_number, ERR_TIMEOUT);
    }

    #[test]
    fn unreachable_target_is_reported() {
        let r = send_message(Path::new("/nonexistent/x.sock"), &AppEvent::new("a", "b", Json::Null), Duration::from_millis(50));
        assert_eq!(r.error_number, ERR_TARGET_UNREACHABLE);
    }
}
