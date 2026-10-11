//! The Anthropic Messages API over HTTPS (`bifrost_app::url`) with `stream: true`. `stream` runs on a background thread and sends
//! [`Event`]s back; [`SseParser`] are pure so they can be tested without a network.

use bifrost_app::dispatch::{DispatchQueue, Qos};
use bifrost_core::json::{self, quote, Json};
use bifrost_app::url::{URLError, URLRequest, URLSession};
use std::io::Write;
use std::time::Duration;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

pub const HOST: &str = "api.anthropic.com";
pub const VERSION: &str = "2023-06-01";
pub const MAX_TOKENS: u32 = 4096;

/// What a request reports back, tagged with the request generation by the caller.
#[derive(Debug, PartialEq)]
pub enum Event {
    Delta(String),
    Done,
    Error(String),
}

/// The JSON request body. `messages` are `(is_user, text)` pairs in order.
pub fn request_body(model: &str, messages: &[(bool, String)]) -> String {
    let list: Vec<String> = messages
        .iter()
        .map(|(user, text)| format!("{{\"role\":\"{}\",\"content\":{}}}", if *user { "user" } else { "assistant" }, quote(text)))
        .collect();
    format!("{{\"model\":{},\"max_tokens\":{MAX_TOKENS},\"messages\":[{}],\"stream\":true}}", quote(model), list.join(","))
}

/// Incremental Server-Sent Events reader for the Messages stream.
#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
}

impl SseParser {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Event> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(nl) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=nl).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\r', '\n']);
            let Some(data) = line.strip_prefix("data:") else { continue };
            if let Some(e) = parse_data(data.trim_start()) {
                out.push(e);
            }
        }
        out
    }
}

fn parse_data(data: &str) -> Option<Event> {
    let v = json::parse(data).ok()?;
    match v.get("type")?.as_str()? {
        "content_block_delta" => {
            let d = v.get("delta")?;
            (d.get("type")?.as_str()? == "text_delta").then(|| Event::Delta(d.get("text").and_then(Json::as_str).unwrap_or("").to_string()))
        }
        "error" => {
            let msg = v.get("error").and_then(|e| e.get("message")).and_then(Json::as_str).unwrap_or("The API reported an error");
            Some(Event::Error(msg.to_string()))
        }
        "message_stop" => Some(Event::Done),
        _ => None,
    }
}

/// The message of an API error body (`{"type":"error","error":{"message":..}}`), else the raw text.
pub fn error_message(status: u16, body: &str) -> String {
    let detail = json::parse(body).ok().and_then(|v| v.get("error").and_then(|e| e.get("message")).and_then(Json::as_str).map(str::to_string));
    let hint = match status {
        401 => " Check the API key in Settings.",
        429 => " Too many requests; try again shortly.",
        _ => "",
    };
    match detail {
        Some(d) => format!("{d} (HTTP {status}).{hint}"),
        None => format!("HTTP {status}.{hint}"),
    }
}

/// Runs one streaming request, blocking; call it from a background thread. Always ends with `Done` or `Error`
/// unless `cancel` is set (then it just returns). Send failures (the app is gone) end the request quietly.
pub fn stream(key: &str, model: &str, messages: &[(bool, String)], cancel: &AtomicBool, send: &dyn Fn(Event) -> bool) {
    let result = run(key, model, messages, cancel, send);
    if cancel.load(Ordering::Relaxed) {
        return;
    }
    if let Err(e) = result {
        send(Event::Error(e));
    }
}

fn run(key: &str, model: &str, messages: &[(bool, String)], cancel: &AtomicBool, send: &dyn Fn(Event) -> bool) -> Result<(), String> {
    let mut req = URLRequest::post(&format!("https://{HOST}/v1/messages"), request_body(model, messages).into_bytes());
    req.set_header("x-api-key", key);
    req.set_header("anthropic-version", VERSION);
    req.set_header("content-type", "application/json");
    req.set_header("accept", "text/event-stream");
    req.timeout = Duration::from_secs(600);

    let status = std::cell::Cell::new(0u16);
    let mut sse = SseParser::default();
    let mut raw_body: Vec<u8> = Vec::new();
    let mut finished = false;
    let mut stopped = false;
    let result = URLSession::shared().bytes(&req, cancel, &mut |r| status.set(r.status_code), &mut |data| {
        if status.get() == 200 {
            for ev in sse.feed(data) {
                let end = matches!(ev, Event::Done | Event::Error(_));
                if !send(ev) {
                    stopped = true;
                    return false;
                }
                if end {
                    finished = true;
                }
            }
            !finished
        } else {
            if raw_body.len() < 64 * 1024 {
                raw_body.extend_from_slice(data);
            }
            true
        }
    });
    match result {
        Err(URLError::Cancelled) if stopped || finished || cancel.load(Ordering::Relaxed) => return Ok(()),
        Err(URLError::CannotConnect(e)) => return Err(format!("Cannot reach {HOST}: {e}")),
        Err(e) => return Err(e.to_string()),
        Ok(_) => {}
    }
    if status.get() != 200 {
        return Err(error_message(status.get(), &String::from_utf8_lossy(&raw_body)));
    }
    if !finished {
        // the stream ended without message_stop: keep what arrived
        send(Event::Done);
    }
    Ok(())
}

/// A handle the app keeps for one running request.
pub struct Running {
    pub generation: u64,
    pub cancel: Arc<AtomicBool>,
}

/// Starts [`stream`] on a background thread; events arrive on `tx` tagged with `generation`.
/// `wake` is written to after every event so the app loop reads `tx` at once.
pub fn spawn(generation: u64, key: String, model: String, messages: Vec<(bool, String)>, tx: Sender<(u64, Event)>, wake: Option<UnixStream>) -> Running {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    DispatchQueue::global(Qos::UserInitiated).async_(move || {
        let emit = |ev: Event| {
            let delivered = tx.send((generation, ev)).is_ok();
            if let Some(mut w) = wake.as_ref() {
                let _ = w.write(&[1]);
            }
            delivered
        };
        stream(&key, &model, &messages, &flag, &emit);
    });
    Running { generation, cancel }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_text_deltas_errors_and_stop() {
        let mut p = SseParser::default();
        let mut got = p.feed(b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\nevent: x\ndata: {\"type\":\"content_block_del");
        got.extend(p.feed(b"ta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"lo\"}}\n\ndata: {\"type\":\"ping\"}\n\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\ndata: {\"type\":\"message_stop\"}\n"));
        assert_eq!(got, vec![Event::Delta("Hel".into()), Event::Delta("lo".into()), Event::Error("Overloaded".into()), Event::Done]);
    }

    #[test]
    fn body_has_model_messages_and_stream() {
        let b = request_body("claude-sonnet-5-5", &[(true, "hi \"x\"".into()), (false, "yo".into())]);
        let v = json::parse(&b).unwrap();
        assert_eq!(v.get("stream"), Some(&Json::Bool(true)));
        assert_eq!(v.get("model").and_then(Json::as_str), Some("claude-sonnet-5-5"));
        assert_eq!(v.get("messages").and_then(Json::as_array).map(|m| m.len()), Some(2));
    }

    #[test]
    fn http_errors_use_the_api_message() {
        assert!(error_message(401, "{\"type\":\"error\",\"error\":{\"type\":\"authentication_error\",\"message\":\"invalid x-api-key\"}}").contains("invalid x-api-key"));
        assert_eq!(error_message(500, "oops"), "HTTP 500.");
    }
}
