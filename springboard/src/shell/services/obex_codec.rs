//! The `obex` service: OBEX packets for apps that speak the object exchange protocol over a Bluetooth link. The model is
//! `bifrost_core::obex_packet` (encode and decode of the packet bytes; it opens no link). The link itself is the
//! Linux Bluetooth stack's job (BlueZ and its obex daemon), so this service only turns bytes into fields and back.
//!
//! Methods (JSON parameters; packets are hex text):
//! - `decode {hex, response?, op?}` answers `{code, fixed, headers}`. A request packet is read as-is. A response
//!   (`response: true`) needs `op`, the request opcode it answers, because the length of its fixed part depends on the
//!   operation. `headers` is a list of `{id, kind, value}`: `kind` is `text`, `bytes`, `byte` or `dword`, and `value`
//!   is the text, the hex bytes, or the number.
//! - `connect {max_len}` answers `{hex}`: a CONNECT request for version 1.0 with that largest packet length.
//! - `exchange {addr, channel, hex}` opens an RFCOMM channel to the device `addr` (`channel` 1 to 30) and sends the
//!   request packet `hex`. The link runs on a worker thread (`bare::rfcomm::request`), so the call answers at once
//!   with `{id}`; the app then calls `reply {id}`, which answers `{state: "pending"}` until the exchange ends, then
//!   `{state: "done", hex}` (the one packet that came back) or `{state: "failed", error}`. An id is answered once, and
//!   only to the app that started it. Links are open to first-party apps only.
//!
//! A packet the model cannot read (a short or inconsistent length, a value of the wrong type) is a service error.
use super::keychain_groups::first_party;
use super::{Shell, SystemService};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use crate::keychain::{hex, unhex};
use bifrost_core::json::Json;
use bifrost_core::obex_packet::{connect_request, decode, fixed_len, Header, Packet, Value, FINAL};

/// The service name.
pub const SERVICE: &str = "obex";

/// A link being exchanged: the app that started it, and the worker's answer when it comes.
type Pending = (String, Receiver<Result<Vec<u8>, String>>);

/// The `obex` service. The links in flight are kept until their reply is polled.
#[derive(Default)]
pub struct ObexCodec {
    next_id: u64,
    links: HashMap<u64, Pending>,
}

impl SystemService for ObexCodec {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        match method {
            "exchange" => self.start(app_id, params),
            "reply" => self.reply(app_id, params),
            _ => answer(method, params),
        }
    }
}

impl ObexCodec {
    /// Checks the request, then starts the link on a worker thread and answers its id.
    fn start(&mut self, app_id: &str, params: &Json) -> Result<Json, String> {
        let installed = crate::shell::apphost::installed();
        let first = installed.iter().find(|a| a.manifest.id == app_id).is_some_and(|a| first_party(&a.manifest));
        if !first {
            return Err("OBEX links are open to first-party apps only".into());
        }
        let addr = text(params, "addr")?;
        let channel = params.get("channel").and_then(Json::as_u64).and_then(|c| u8::try_from(c).ok()).ok_or("channel must be 1 to 30")?;
        let packet = unhex(&text(params, "hex")?).ok_or("hex must be an even run of hex digits")?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(link(&addr, channel, &packet));
        });
        self.next_id += 1;
        self.links.insert(self.next_id, (app_id.to_string(), rx));
        Ok(obj(vec![("id", Json::Num(self.next_id as f64))]))
    }

    /// Answers a started link: pending, done or failed. A finished id is forgotten.
    fn reply(&mut self, app_id: &str, params: &Json) -> Result<Json, String> {
        let id = params.get("id").and_then(Json::as_u64).ok_or("id must be a whole number")?;
        let owned = self.links.get(&id).is_some_and(|(owner, _)| owner == app_id);
        if !owned {
            return Err(format!("no OBEX link {id}"));
        }
        let outcome = match self.links.get(&id).map(|(_, rx)| rx.try_recv()) {
            Some(Err(TryRecvError::Empty)) => return Ok(obj(vec![("state", Json::Str("pending".into()))])),
            Some(Ok(outcome)) => outcome,
            _ => Err("the link worker stopped".to_string()),
        };
        self.links.remove(&id);
        Ok(match outcome {
            Ok(bytes) => obj(vec![("state", Json::Str("done".into())), ("hex", Json::Str(hex(&bytes)))]),
            Err(error) => obj(vec![("state", Json::Str("failed".into())), ("error", Json::Str(error))]),
        })
    }
}

fn obj(fields: Vec<(&str, Json)>) -> Json {
    Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// One request over an RFCOMM channel to `addr`, the answer packet's bytes.
#[cfg(target_os = "linux")]
fn link(addr: &str, channel: u8, packet: &[u8]) -> Result<Vec<u8>, String> {
    crate::bare::rfcomm::request(addr, channel, packet).map_err(|e| format!("obex link: {e}"))
}

#[cfg(not(target_os = "linux"))]
fn link(_addr: &str, _channel: u8, _packet: &[u8]) -> Result<Vec<u8>, String> {
    Err("RFCOMM links need Linux".into())
}

/// Runs one method. Pure: it reads only the parameters.
pub fn answer(method: &str, params: &Json) -> Result<Json, String> {
    match method {
        "decode" => {
            let bytes = unhex(&text(params, "hex")?).ok_or("hex must be an even run of hex digits")?;
            let response = params.get("response") == Some(&Json::Bool(true));
            let op = match params.get("op") {
                Some(n) => n.number().filter(|v| (0.0..=255.0).contains(v)).map(|v| v as u8).ok_or("op must be a byte")?,
                None if response => return Err("a response needs op, the request opcode it answers".into()),
                None => bytes.first().map_or(0, |code| code & !FINAL),
            };
            let packet = decode(&bytes, fixed_len(op, response)).map_err(|e| format!("{e:?}"))?;
            Ok(packet_json(&packet))
        }
        "connect" => {
            let max_len = params.get("max_len").and_then(Json::as_u64).and_then(|n| u16::try_from(n).ok()).ok_or("max_len must be 0 to 65535")?;
            let bytes = connect_request(max_len, Vec::new()).encode().map_err(|e| format!("{e:?}"))?;
            Ok(Json::Obj(vec![("hex".into(), Json::Str(hex(&bytes)))]))
        }
        _ => Err(format!("obex has no method {method}")),
    }
}

fn text(params: &Json, key: &str) -> Result<String, String> {
    params.str_field(key).ok_or_else(|| format!("{key} must be a string"))
}

fn packet_json(packet: &Packet) -> Json {
    let headers = packet.headers.iter().map(header_json).collect();
    Json::Obj(vec![
        ("code".into(), Json::Num(packet.code as f64)),
        ("fixed".into(), Json::Str(hex(&packet.fixed))),
        ("headers".into(), Json::Arr(headers)),
    ])
}

fn header_json(h: &Header) -> Json {
    let (kind, value) = match &h.value {
        Value::Text(s) => ("text", Json::Str(s.clone())),
        Value::Bytes(b) => ("bytes", Json::Str(hex(b))),
        Value::Byte(b) => ("byte", Json::Num(*b as f64)),
        Value::Dword(d) => ("dword", Json::Num(*d as f64)),
    };
    Json::Obj(vec![("id".into(), Json::Num(h.id as f64)), ("kind".into(), Json::Str(kind.into())), ("value".into(), value)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::obex_packet::{H_NAME, OP_GET, RSP_SUCCESS};

    fn obj(pairs: Vec<(&str, Json)>) -> Json {
        Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    fn started(svc: &mut ObexCodec, app: &str, outcome: Option<Result<Vec<u8>, String>>) -> u64 {
        let (tx, rx) = mpsc::channel();
        if let Some(o) = outcome {
            tx.send(o).unwrap();
        } else {
            std::mem::forget(tx);
        }
        svc.next_id += 1;
        svc.links.insert(svc.next_id, (app.to_string(), rx));
        svc.next_id
    }

    #[test]
    fn a_reply_is_pending_then_done_once_and_only_for_its_owner() {
        let mut svc = ObexCodec::default();
        let waiting = started(&mut svc, "app.one", None);
        let r = svc.reply("app.one", &obj(vec![("id", Json::Num(waiting as f64))])).unwrap();
        assert_eq!(r.get("state").and_then(Json::as_str), Some("pending"));
        assert!(svc.reply("app.two", &obj(vec![("id", Json::Num(waiting as f64))])).is_err());

        let done = started(&mut svc, "app.one", Some(Ok(vec![0xA0, 0x00, 0x03])));
        let r = svc.reply("app.one", &obj(vec![("id", Json::Num(done as f64))])).unwrap();
        assert_eq!(r.get("state").and_then(Json::as_str), Some("done"));
        assert_eq!(r.get("hex").and_then(Json::as_str), Some("a00003"));
        assert!(svc.reply("app.one", &obj(vec![("id", Json::Num(done as f64))])).is_err());
    }

    #[test]
    fn a_failed_link_is_reported_with_its_error() {
        let mut svc = ObexCodec::default();
        let id = started(&mut svc, "app.one", Some(Err("obex link: refused".into())));
        let r = svc.reply("app.one", &obj(vec![("id", Json::Num(id as f64))])).unwrap();
        assert_eq!(r.get("state").and_then(Json::as_str), Some("failed"));
        assert_eq!(r.get("error").and_then(Json::as_str), Some("obex link: refused"));
    }

    #[test]
    fn an_app_that_is_not_installed_gets_no_link() {
        let req = obj(vec![("addr", Json::Str("AA:BB:CC:DD:EE:FF".into())), ("channel", Json::Num(3.0)), ("hex", Json::Str("8000".into()))]);
        assert!(exchange("app.nobody", &req).unwrap_err().contains("first-party"));
    }

    #[test]
    fn connect_then_decode_round_trips() {
        let made = answer("connect", &obj(vec![("max_len", Json::Num(4096.0))])).unwrap();
        let wire = made.get("hex").and_then(Json::as_str).unwrap().to_string();
        assert_eq!(&wire[..2], "80");
        let back = answer("decode", &obj(vec![("hex", Json::Str(wire))])).unwrap();
        assert_eq!(back.get("code").and_then(Json::number), Some(128.0));
        // version, flags and the 16-bit maximum length: four bytes, eight hex digits
        assert_eq!(back.get("fixed").and_then(Json::as_str).map(|s| s.len()), Some(8));
        let headers = back.get("headers").and_then(Json::as_array).unwrap();
        assert!(headers.is_empty());
    }

    #[test]
    fn a_name_header_comes_back_as_text() {
        let packet = connect_request(512, vec![Header { id: H_NAME, value: Value::Text("ab".into()) }]);
        let wire = hex(&packet.encode().unwrap());
        let back = answer("decode", &obj(vec![("hex", Json::Str(wire))])).unwrap();
        let headers = back.get("headers").and_then(Json::as_array).unwrap();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].get("kind").and_then(Json::as_str), Some("text"));
        assert_eq!(headers[0].get("value").and_then(Json::as_str), Some("ab"));
    }

    #[test]
    fn a_response_needs_the_opcode_it_answers() {
        // a bare success response to GET: code 0xA0, length 3, no fixed part and no headers
        let resp = obj(vec![("hex", Json::Str("a00003".into())), ("response", Json::Bool(true))]);
        assert!(answer("decode", &resp).is_err());
        let with_op = obj(vec![("hex", Json::Str("a00003".into())), ("response", Json::Bool(true)), ("op", Json::Num(OP_GET as f64))]);
        let r = answer("decode", &with_op).unwrap();
        assert_eq!(r.get("code").and_then(Json::number), Some(RSP_SUCCESS as f64));
    }

    #[test]
    fn bad_input_is_refused() {
        assert!(answer("decode", &obj(vec![("hex", Json::Str("8".into()))])).is_err());
        assert!(answer("decode", &obj(vec![("hex", Json::Str("zz".into()))])).is_err());
        assert!(answer("connect", &obj(vec![("max_len", Json::Num(70_000.0))])).is_err());
        assert!(answer("nope", &Json::Null).is_err());
    }
}
