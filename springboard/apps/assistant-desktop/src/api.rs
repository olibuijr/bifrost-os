//! The Anthropic Messages API over HTTPS (`bifrost_app::url`) with `stream: true`. `stream` runs on a background thread and
//! sends [`Event`]s back. A reply may call tools: each call the model makes is run by the app's tool set (`tools.rs`), its
//! result goes back to the model, and the reply goes on. [`SseParser`] is pure so it can be tested without a network.

use crate::tools;
use bifrost_app::controls::transcript::ToolCall;
use bifrost_app::dispatch::{DispatchQueue, Qos};
use bifrost_app::url::{URLError, URLRequest, URLSession};
use bifrost_core::json::{self, quote, Json};
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

pub const HOST: &str = "api.anthropic.com";
pub const VERSION: &str = "2023-06-01";
pub const MAX_TOKENS: u32 = 4096;
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

/// One thing the stream says, before the reply loop acts on it.
#[derive(Debug, PartialEq)]
pub enum Piece {
    Text(String),
    /// A tool-use content block starts at `index`.
    ToolStart { index: u64, id: String, name: String },
    /// A piece of the JSON arguments of the tool-use block at `index`.
    ToolInput { index: u64, json: String },
    BlockStop(u64),
    MessageStop,
    Error(String),
}

/// A tool call the model made in one request, with its arguments as JSON text.
#[derive(Clone, Debug, PartialEq)]
struct Use {
    index: u64,
    id: String,
    name: String,
    input: String,
}

/// What one request produced: the text it streamed and the tool calls it made, in order.
#[derive(Debug, PartialEq)]
struct Turn {
    text: String,
    calls: Vec<Use>,
}

/// The JSON text of one message in the Messages API form with plain text content.
fn text_message(user: bool, text: &str) -> String {
    format!("{{\"role\":\"{}\",\"content\":{}}}", if user { "user" } else { "assistant" }, quote(text))
}

/// The JSON request body from already formatted messages. `tools` is the JSON array of tool definitions, if any.
fn body(model: &str, system: Option<&str>, wire: &[String], tools: Option<&str>) -> String {
    let system = system.map_or(String::new(), |s| format!("\"system\":{},", quote(s)));
    let tools = tools.map_or(String::new(), |t| format!("\"tools\":{t},"));
    format!("{{\"model\":{},\"max_tokens\":{MAX_TOKENS},{system}{tools}\"messages\":[{}],\"stream\":true}}", quote(model), wire.join(","))
}

/// The JSON request body for `messages` (`(is_user, text)` pairs in order) with no tools. `system` is the skill's
/// instructions, if any.
pub fn request_body(model: &str, system: Option<&str>, messages: &[(bool, String)]) -> String {
    let wire: Vec<String> = messages.iter().map(|(user, text)| text_message(*user, text)).collect();
    body(model, system, &wire, None)
}

/// The assistant message that records one turn: its text, then each tool call it made.
fn assistant_message(turn: &Turn) -> String {
    let mut blocks = Vec::new();
    if !turn.text.trim().is_empty() {
        blocks.push(format!("{{\"type\":\"text\",\"text\":{}}}", quote(&turn.text)));
    }
    for call in &turn.calls {
        let input = if call.input.trim_start().starts_with('{') && json::parse(&call.input).is_ok() { call.input.clone() } else { "{}".into() };
        blocks.push(format!("{{\"type\":\"tool_use\",\"id\":{},\"name\":{},\"input\":{input}}}", quote(&call.id), quote(&call.name)));
    }
    format!("{{\"role\":\"assistant\",\"content\":[{}]}}", blocks.join(","))
}

/// The user message that hands a tool's result back to the model.
fn tool_result(id: &str, text: &str) -> String {
    format!("{{\"type\":\"tool_result\",\"tool_use_id\":{},\"content\":{}}}", quote(id), quote(text))
}

/// Incremental Server-Sent Events reader for the Messages stream.
#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
}

impl SseParser {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Piece> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(nl) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=nl).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\r', '\n']);
            let Some(data) = line.strip_prefix("data:") else { continue };
            if let Some(p) = parse_data(data.trim_start()) {
                out.push(p);
            }
        }
        out
    }
}

fn index_of(v: &Json) -> u64 {
    v.get("index").and_then(Json::as_u64).unwrap_or(0)
}

fn parse_data(data: &str) -> Option<Piece> {
    let v = json::parse(data).ok()?;
    match v.get("type")?.as_str()? {
        "content_block_start" => {
            let block = v.get("content_block")?;
            (block.get("type")?.as_str()? == "tool_use").then(|| Piece::ToolStart {
                index: index_of(&v),
                id: block.str_field("id").unwrap_or_default(),
                name: block.str_field("name").unwrap_or_default(),
            })
        }
        "content_block_delta" => {
            let d = v.get("delta")?;
            match d.get("type")?.as_str()? {
                "text_delta" => Some(Piece::Text(d.str_field("text").unwrap_or_default())),
                "input_json_delta" => Some(Piece::ToolInput { index: index_of(&v), json: d.str_field("partial_json").unwrap_or_default() }),
                _ => None,
            }
        }
        "content_block_stop" => Some(Piece::BlockStop(index_of(&v))),
        "error" => {
            let msg = v.get("error").and_then(|e| e.get("message")).and_then(Json::as_str).unwrap_or("The API reported an error");
            Some(Piece::Error(msg.to_string()))
        }
        "message_stop" => Some(Piece::MessageStop),
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

/// True when an error means no model answered at all (the network or the service is down or overloaded), so the chat
/// shows its availability banner. A rejected key or a bad request is not that.
pub fn is_unreachable(error: &str) -> bool {
    error.starts_with("Cannot reach") || error.starts_with("HTTP 5") || error.contains("(HTTP 5")
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

    fn wire_of(body: &str) -> Json {
        json::parse(body).unwrap()
    }

    #[test]
    fn sse_text_deltas_errors_and_stop() {
        let mut p = SseParser::default();
        let mut got = p.feed(b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\nevent: x\ndata: {\"type\":\"content_block_del");
        got.extend(p.feed(b"ta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"lo\"}}\n\ndata: {\"type\":\"ping\"}\n\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\ndata: {\"type\":\"message_stop\"}\n"));
        assert_eq!(got, vec![Piece::Text("Hel".into()), Piece::Text("lo".into()), Piece::Error("Overloaded".into()), Piece::MessageStop]);
    }

    #[test]
    fn a_tool_block_streams_its_start_arguments_and_stop() {
        let mut p = SseParser::default();
        let got = p.feed(concat!(
            "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"count_words\",\"input\":{}}}\n",
            "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"text\\\":\"}}\n",
            "data: {\"type\":\"content_block_stop\",\"index\":1}\n",
        ).as_bytes());
        assert_eq!(
            got,
            vec![
                Piece::ToolStart { index: 1, id: "toolu_1".into(), name: "count_words".into() },
                Piece::ToolInput { index: 1, json: "{\"text\":".into() },
                Piece::BlockStop(1),
            ]
        );
    }

    #[test]
    fn body_has_model_messages_and_stream() {
        let b = request_body("claude-sonnet-5-5", None, &[(true, "hi \"x\"".into()), (false, "yo".into())]);
        let v = wire_of(&b);
        assert_eq!(v.get("stream"), Some(&Json::Bool(true)));
        assert_eq!(v.get("model").and_then(Json::as_str), Some("claude-sonnet-5-5"));
        assert_eq!(v.get("messages").and_then(Json::as_array).map(|m| m.len()), Some(2));
        assert_eq!(v.get("system"), None, "no skill, no system prompt");
        assert_eq!(v.get("tools"), None, "the plain body offers no tools");
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

    #[test]
    fn a_skill_adds_its_system_prompt() {
        let b = request_body("claude-haiku-5-5", Some("Be \"brief\"."), &[(true, "hi".into())]);
        let v = wire_of(&b);
        assert_eq!(v.get("system").and_then(Json::as_str), Some("Be \"brief\"."));
        assert_eq!(v.get("messages").and_then(Json::as_array).map(|m| m.len()), Some(1));
    }

    #[test]
    fn the_assistant_turn_keeps_its_text_and_calls() {
        let turn = Turn { text: "Counting.".into(), calls: vec![Use { index: 1, id: "t1".into(), name: "count_words".into(), input: "{\"text\":\"a b\"}".into() }] };
        let v = wire_of(&assistant_message(&turn));
        assert_eq!(v.get("role").and_then(Json::as_str), Some("assistant"));
        let blocks = v.get("content").and_then(Json::as_array).unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].str_field("type").as_deref(), Some("text"));
        assert_eq!(blocks[1].str_field("type").as_deref(), Some("tool_use"));
        assert_eq!(blocks[1].get("input").and_then(|i| i.str_field("text")).as_deref(), Some("a b"));
    }

    #[test]
    fn an_empty_or_broken_input_is_sent_as_an_empty_object() {
        let turn = Turn { text: String::new(), calls: vec![Use { index: 0, id: "t2".into(), name: "current_time".into(), input: "{\"x\":".into() }] };
        let v = wire_of(&assistant_message(&turn));
        let blocks = v.get("content").and_then(Json::as_array).unwrap();
        assert_eq!(blocks.len(), 1, "no text block for an empty text");
        assert!(blocks[0].get("input").and_then(|i| i.get("x")).is_none());
    }

    #[test]
    fn http_errors_use_the_api_message() {
        assert!(error_message(401, "{\"type\":\"error\",\"error\":{\"type\":\"authentication_error\",\"message\":\"invalid x-api-key\"}}").contains("invalid x-api-key"));
        assert_eq!(error_message(500, "oops"), "HTTP 500.");
    }

    #[test]
    fn an_unreachable_service_is_an_availability_failure() {
        assert!(is_unreachable("Cannot reach api.anthropic.com: no route"));
        assert!(is_unreachable(&error_message(529, "{\"error\":{\"message\":\"Overloaded\"}}")));
        assert!(is_unreachable("HTTP 500."));
        assert!(!is_unreachable(&error_message(401, "{\"error\":{\"message\":\"invalid x-api-key\"}}")));
        assert!(!is_unreachable("You are offline."));
    }
}
