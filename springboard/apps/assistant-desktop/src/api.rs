//! The Anthropic Messages API over HTTPS (`bifrost_app::url`) with `stream: true`. `stream` runs on a background thread and
//! sends [`Event`]s back. A reply may call tools: each call the model makes is run by the app's tool set (`tools.rs`), its
//! result goes back to the model, and the reply goes on. The wire format and the stream reader live in the shared
//! `assistant_client` crate (re-exported here so the app's code is unchanged).

use crate::tools;
use bifrost_app::controls::transcript::ToolCall;
use bifrost_app::dispatch::{DispatchQueue, Qos};
use bifrost_app::url::{URLError, URLRequest, URLSession};
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

pub use assistant_client::sse::{Piece, SseParser};
pub use assistant_client::wire::{assistant_message, error_message, is_unreachable, request_body, stream_body, text_message, tool_result, Turn, Use, HOST, VERSION};

pub const MAX_TOKENS: u32 = assistant_client::wire::STREAM_MAX_TOKENS;
/// Tool rounds one reply may take. Each round is one more request; a reply past the limit ends as it stands.
pub const MAX_TOOL_ROUNDS: usize = 4;

/// What a request reports back, tagged with the request generation by the caller.
#[derive(Debug, PartialEq)]
pub enum Event {
    Delta(String),
    /// A tool the model called and the app ran: its name, the arguments it chose (JSON text) and the result it was given.
    Tool { name: String, arguments: String, result: String },
    Done,
    Error(String),
}

/// The JSON request body for `messages` with no tools, streamed (see [`assistant_client::wire::request_body`]).
fn body(model: &str, system: Option<&str>, wire: &[String], tools: Option<&str>) -> String {
    stream_body(model, MAX_TOKENS, system, wire, tools)
}

/// Runs one reply, blocking; call it from a background thread. The model may call tools; each call is run and its
/// result sent back, up to [`MAX_TOOL_ROUNDS`] rounds. Always ends with `Done` or `Error` unless `cancel` is set (then
/// it just returns). Send failures (the app is gone) end the request quietly.
pub fn stream(key: &str, model: &str, system: Option<&str>, messages: &[(bool, String)], cancel: &AtomicBool, send: &dyn Fn(Event) -> bool) {
    let result = run(key, model, system, messages, cancel, send);
    if cancel.load(Ordering::Relaxed) {
        return;
    }
    if let Err(e) = result {
        send(Event::Error(e));
    }
}

fn run(key: &str, model: &str, system: Option<&str>, messages: &[(bool, String)], cancel: &AtomicBool, send: &dyn Fn(Event) -> bool) -> Result<(), String> {
    let set = tools::tool_set();
    let offered = tools::tools_json();
    let mut wire: Vec<String> = messages.iter().map(|(user, text)| text_message(*user, text)).collect();
    for _ in 0..=MAX_TOOL_ROUNDS {
        let Some(turn) = run_turn(key, body(model, system, &wire, Some(offered.as_str())), cancel, send)? else { return Ok(()) };
        if turn.calls.is_empty() {
            send(Event::Done);
            return Ok(());
        }
        wire.push(assistant_message(&turn));
        let mut results = Vec::new();
        for call in &turn.calls {
            let out = set.output_for(&ToolCall { name: call.name.clone(), arguments: call.input.clone() });
            if !send(Event::Tool { name: call.name.clone(), arguments: call.input.clone(), result: out.text.clone() }) {
                return Ok(());
            }
            results.push(tool_result(&call.id, &out.text));
        }
        wire.push(format!("{{\"role\":\"user\",\"content\":[{}]}}", results.join(",")));
    }
    send(Event::Done);
    Ok(())
}

/// One streaming request. `Ok(None)` when the reader stopped it (cancelled, or the app is gone).
fn run_turn(key: &str, body: String, cancel: &AtomicBool, send: &dyn Fn(Event) -> bool) -> Result<Option<Turn>, String> {
    let mut req = URLRequest::post(&format!("https://{HOST}/v1/messages"), body.into_bytes());
    req.set_header("x-api-key", key);
    req.set_header("anthropic-version", VERSION);
    req.set_header("content-type", "application/json");
    req.set_header("accept", "text/event-stream");
    req.timeout = Duration::from_secs(600);

    let status = std::cell::Cell::new(0u16);
    let mut sse = SseParser::default();
    let mut raw_body: Vec<u8> = Vec::new();
    let mut turn = Turn { text: String::new(), calls: Vec::new() };
    let mut open: Vec<Use> = Vec::new();
    let mut finished = false;
    let mut stopped = false;
    let mut failure: Option<String> = None;
    let result = URLSession::shared().bytes(&req, cancel, &mut |r| status.set(r.status_code), &mut |data| {
        if status.get() != 200 {
            if raw_body.len() < 64 * 1024 {
                raw_body.extend_from_slice(data);
            }
            return true;
        }
        for piece in sse.feed(data) {
            match piece {
                Piece::Text(t) => {
                    turn.text.push_str(&t);
                    if !send(Event::Delta(t)) {
                        stopped = true;
                        return false;
                    }
                }
                Piece::ToolStart { index, id, name } => open.push(Use { index, id, name, input: String::new() }),
                Piece::ToolInput { index, json } => {
                    if let Some(u) = open.iter_mut().find(|u| u.index == index) {
                        u.input.push_str(&json);
                    }
                }
                Piece::BlockStop(index) => {
                    if let Some(pos) = open.iter().position(|u| u.index == index) {
                        let mut call = open.remove(pos);
                        if call.input.trim().is_empty() {
                            call.input = "{}".into();
                        }
                        turn.calls.push(call);
                    }
                }
                Piece::Error(e) => {
                    failure = Some(e);
                    return false;
                }
                Piece::MessageStop => {
                    finished = true;
                    return false;
                }
            }
        }
        true
    });
    if let Some(e) = failure {
        return Err(e);
    }
    if stopped || cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    match result {
        Err(URLError::Cancelled) if finished => {}
        Err(URLError::CannotConnect(e)) => return Err(format!("Cannot reach {HOST}: {e}")),
        Err(e) => return Err(e.to_string()),
        Ok(_) => {}
    }
    if status.get() != 200 {
        return Err(error_message(status.get(), &String::from_utf8_lossy(&raw_body)));
    }
    // A stream that ended without message_stop keeps what arrived.
    Ok(Some(turn))
}

/// A handle the app keeps for one running request.
pub struct Running {
    pub generation: u64,
    pub cancel: Arc<AtomicBool>,
}

/// Starts [`stream`] on a background thread; events arrive on `tx` tagged with `generation`.
/// `wake` is written to after every event so the app loop reads `tx` at once.
pub fn spawn(generation: u64, key: String, model: String, system: Option<String>, messages: Vec<(bool, String)>, tx: Sender<(u64, Event)>, wake: Option<UnixStream>) -> Running {
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
        stream(&key, &model, system.as_deref(), &messages, &flag, &emit);
    });
    Running { generation, cancel }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::json::Json;

    fn wire_of(body: &str) -> Json {
        bifrost_core::json::parse(body).unwrap()
    }

    #[test]
    fn a_turn_body_offers_the_tools() {
        let wire = vec![text_message(true, "what time is it")];
        let offered = tools::tools_json();
        let b = body("claude-haiku-5-5", None, &wire, Some(offered.as_str()));
        let v = wire_of(&b);
        let names: Vec<&str> = v.get("tools").and_then(Json::as_array).unwrap().iter().filter_map(|t| t.get("name").and_then(Json::as_str)).collect();
        assert_eq!(names, vec!["current_time", "count_words"]);
    }
}
