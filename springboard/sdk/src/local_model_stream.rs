//! Token-by-token replies from the local inference engine. [`LocalModelProvider`] implements
//! [`StreamingModelProvider`]: it asks the engine for a streamed reply (`"stream": true`, server-sent events over the same
//! loopback connection) and hands each piece of answer text to the caller as it arrives, so a view can show the answer
//! growing and a Stop control can end it.
//!
//! The pieces are small and separately tested: [`ChunkedStream`] decodes `Transfer-Encoding: chunked` as bytes arrive,
//! [`SseParser`] turns the bytes into event payloads, and [`StreamAccumulator`] turns payloads into text pieces and, at the
//! end, a [`Reply`]. The accumulator keeps tool calls out of the text: the engine's own `tool_calls` deltas are gathered, and
//! text that begins like a prompted tool call (`{` or a code fence) is held back and judged once the stream ends, so a view
//! never shows `{"tool_calls":...}` as an answer.
//!
//! An engine that ignores `stream` and answers with plain JSON still works: the whole reply is read and its text is passed
//! as one piece. Stopping closes the connection, which is how an engine learns to stop generating.
//!
//! Gaps: no usage or timing figures are read from the stream, and a reply that mixes answer text with a native tool call
//! keeps the text already shown.

use bifrost_core::json::{self, Json};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use crate::language_model::{ProviderError, Reply, Segment, ToolCall, Transcript};
use crate::language_model_stream::StreamingModelProvider;
use crate::local_model::{error_message, parse_head, parse_reply, reply_from_text, split_response, LocalEndpoint, LocalModelProvider, MAX_REPLY_BYTES};

/// The longest chunk-size line accepted, so a broken engine cannot make the decoder buffer without end.
const MAX_SIZE_LINE: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChunkState {
    Size,
    Data,
    DataEnd,
    Done,
}

/// An incremental `Transfer-Encoding: chunked` decoder: bytes go in as they arrive, decoded body bytes come out.
#[derive(Debug)]
pub struct ChunkedStream {
    buf: Vec<u8>,
    state: ChunkState,
    remaining: usize,
}

impl Default for ChunkedStream {
    fn default() -> Self {
        ChunkedStream { buf: Vec::new(), state: ChunkState::Size, remaining: 0 }
    }
}

impl ChunkedStream {
    pub fn new() -> ChunkedStream {
        ChunkedStream::default()
    }

    /// Whether the last chunk (size 0) has been read.
    pub fn is_done(&self) -> bool {
        self.state == ChunkState::Done
    }

    /// Adds received bytes and returns the body bytes they complete. Bytes after the last chunk are dropped.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        self.buf.extend_from_slice(bytes);
        loop {
            match self.state {
                ChunkState::Done => {
                    self.buf.clear();
                    return Ok(out);
                }
                ChunkState::Size => {
                    let Some(end) = self.buf.windows(2).position(|w| w == b"\r\n") else {
                        if self.buf.len() > MAX_SIZE_LINE {
                            return Err("chunk size line is too long".into());
                        }
                        return Ok(out);
                    };
                    let line = std::str::from_utf8(&self.buf[..end]).map_err(|_| "chunk size is not text")?;
                    let size = usize::from_str_radix(line.split(';').next().unwrap_or("").trim(), 16).map_err(|_| "chunk size is not hexadecimal")?;
                    self.buf.drain(..end + 2);
                    if size == 0 {
                        self.state = ChunkState::Done;
                    } else {
                        self.remaining = size;
                        self.state = ChunkState::Data;
                    }
                }
                ChunkState::Data => {
                    let take = self.remaining.min(self.buf.len());
                    if take == 0 {
                        return Ok(out);
                    }
                    out.extend(self.buf.drain(..take));
                    self.remaining -= take;
                    if self.remaining == 0 {
                        self.state = ChunkState::DataEnd;
                    }
                }
                ChunkState::DataEnd => {
                    if self.buf.len() < 2 {
                        return Ok(out);
                    }
                    if &self.buf[..2] != b"\r\n" {
                        return Err("chunk is cut short".into());
                    }
                    self.buf.drain(..2);
                    self.state = ChunkState::Size;
                }
            }
        }
    }
}

/// An incremental server-sent-events reader: bytes go in, the `data` payload of every finished event comes out.
/// Comment lines (`:`) and the `event`, `id` and `retry` fields are ignored; several `data` lines of one event are joined
/// with a newline, as the format says.
#[derive(Debug, Default)]
pub struct SseParser {
    line: Vec<u8>,
    data: Vec<String>,
}

impl SseParser {
    pub fn new() -> SseParser {
        SseParser::default()
    }

    /// Adds received bytes and returns the payloads of the events they finish.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut events = Vec::new();
        for &b in bytes {
            if b != b'\n' {
                self.line.push(b);
                continue;
            }
            if self.line.last() == Some(&b'\r') {
                self.line.pop();
            }
            let line = String::from_utf8_lossy(&std::mem::take(&mut self.line)).into_owned();
            if line.is_empty() {
                if !self.data.is_empty() {
                    events.push(self.data.join("\n"));
                    self.data.clear();
                }
            } else if let Some(value) = line.strip_prefix("data:") {
                self.data.push(value.strip_prefix(' ').unwrap_or(value).to_string());
            }
        }
        events
    }

    /// Ends the stream: an event the engine never closed with a blank line is still delivered.
    pub fn finish(&mut self) -> Vec<String> {
        let mut events = self.feed(b"\n");
        if !self.data.is_empty() {
            events.push(self.data.join("\n"));
            self.data.clear();
        }
        events
    }
}

/// Whether the text so far can still be a prompted tool call, so it must not be shown yet.
fn looks_like_call(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with('{') || t.starts_with("```")
}

/// Collects the payloads of a streamed chat completion into text pieces and a final [`Reply`].
#[derive(Debug, Default)]
pub struct StreamAccumulator {
    content: String,
    /// bytes of `content` already passed on as pieces
    shown: usize,
    /// text that may be a tool call is not shown until the end
    held: bool,
    calls: Vec<(String, String)>,
    done: bool,
}

impl StreamAccumulator {
    pub fn new() -> StreamAccumulator {
        StreamAccumulator::default()
    }

    /// Whether the `[DONE]` marker has been seen.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// The answer text so far (whether or not it has been shown).
    pub fn text(&self) -> &str {
        &self.content
    }

    /// Reads one event payload. New answer text goes to `piece`; the result is `false` once `piece` asked to stop.
    pub fn push(&mut self, payload: &str, piece: &mut dyn FnMut(&str) -> bool) -> Result<bool, ProviderError> {
        let payload = payload.trim();
        if payload == "[DONE]" {
            self.done = true;
            return Ok(true);
        }
        let j = json::parse(payload).map_err(|e| ProviderError::new(format!("a streamed event is not JSON: {e}")))?;
        if let Some(m) = error_message(&j) {
            return Err(ProviderError::new(m));
        }
        let Some(delta) = j.get("choices").and_then(Json::as_array).and_then(|c| c.first()).and_then(|c| c.get("delta")) else {
            return Ok(true);
        };
        if let Some(list) = delta.get("tool_calls").and_then(Json::as_array) {
            for (position, item) in list.iter().enumerate() {
                let index = item.get("index").and_then(Json::as_u64).map(|i| i as usize).unwrap_or(position).min(16);
                while self.calls.len() <= index {
                    self.calls.push((String::new(), String::new()));
                }
                let f = item.get("function").unwrap_or(item);
                if let Some(name) = f.get("name").and_then(Json::as_str) {
                    self.calls[index].0.push_str(name);
                }
                match f.get("arguments") {
                    Some(Json::Str(a)) => self.calls[index].1.push_str(a),
                    Some(Json::Null) | None => {}
                    Some(other) => self.calls[index].1.push_str(&other.encode()),
                }
            }
        }
        let Some(text) = delta.get("content").and_then(Json::as_str).filter(|t| !t.is_empty()) else {
            return Ok(true);
        };
        if self.content.len() + text.len() > MAX_REPLY_BYTES {
            return Err(ProviderError::new("the reply is too large"));
        }
        self.content.push_str(text);
        if self.shown == 0 && !self.held && !self.content.trim().is_empty() {
            self.held = looks_like_call(&self.content);
        }
        if self.held || self.content.trim().is_empty() {
            return Ok(true);
        }
        let from = if self.shown == 0 { self.content.len() - self.content.trim_start().len() } else { self.shown };
        let new = self.content[from..].to_string();
        self.shown = self.content.len();
        Ok(new.is_empty() || piece(&new))
    }

    /// The reply the stream added up to. Held text that turned out to be an answer (not a call) is passed to `piece` now.
    pub fn finish(self, piece: &mut dyn FnMut(&str) -> bool) -> Result<Reply, ProviderError> {
        let calls: Vec<ToolCall> = self.calls.iter().filter(|(name, _)| !name.is_empty()).map(|(name, args)| ToolCall { name: name.clone(), arguments: args.clone() }).collect();
        if !calls.is_empty() {
            return Ok(Reply::Call(calls));
        }
        let reply = reply_from_text(&self.content)?;
        if let Reply::Respond(_) = &reply {
            if self.shown == 0 {
                piece(self.content.trim());
            }
        }
        Ok(reply)
    }

    /// The reply for a stream the caller stopped: the text so far as the answer, never a tool call.
    pub fn stopped(self) -> Reply {
        let text = self.content.trim();
        Reply::Respond(if text.is_empty() { Vec::new() } else { vec![Segment::Text(text.to_string())] })
    }
}

/// Feeds a streamed response body, as it arrives, through the decoder, the event parser and the accumulator.
struct StreamReader {
    chunked: Option<ChunkedStream>,
    sse: SseParser,
    acc: StreamAccumulator,
    stopped: bool,
}

impl StreamReader {
    fn new(chunked: bool) -> StreamReader {
        StreamReader { chunked: chunked.then(ChunkedStream::new), sse: SseParser::new(), acc: StreamAccumulator::new(), stopped: false }
    }

    /// Reads body bytes; `Ok(false)` when the stream is over (the marker arrived or the caller stopped).
    fn feed(&mut self, bytes: &[u8], piece: &mut dyn FnMut(&str) -> bool) -> Result<bool, String> {
        let decoded;
        let body: &[u8] = match &mut self.chunked {
            Some(c) => {
                decoded = c.feed(bytes)?;
                &decoded
            }
            None => bytes,
        };
        for payload in self.sse.feed(body) {
            if !self.acc.push(&payload, piece).map_err(|e| e.message)? {
                self.stopped = true;
                return Ok(false);
            }
            if self.acc.is_done() {
                return Ok(false);
            }
        }
        Ok(!self.chunked.as_ref().is_some_and(ChunkedStream::is_done))
    }

    /// Ends the stream (the connection closed or the caller stopped) and produces the reply.
    fn finish(mut self, piece: &mut dyn FnMut(&str) -> bool) -> Result<Reply, ProviderError> {
        if self.stopped {
            return Ok(self.acc.stopped());
        }
        for payload in self.sse.finish() {
            if !self.acc.push(&payload, piece)? {
                return Ok(self.acc.stopped());
            }
        }
        self.acc.finish(piece)
    }
}

fn io_error(what: &str, e: std::io::Error) -> ProviderError {
    ProviderError::new(format!("{what}: {e}"))
}

/// Sends the request and passes the streamed answer to `piece`.
fn stream_post(endpoint: &LocalEndpoint, body: &str, timeout: Duration, piece: &mut dyn FnMut(&str) -> bool) -> Result<Reply, ProviderError> {
    let mut stream = TcpStream::connect_timeout(&endpoint.addr, Duration::from_secs(2)).map_err(|e| io_error("the local engine is not reachable", e))?;
    stream.set_read_timeout(Some(timeout)).map_err(|e| ProviderError::new(e.to_string()))?;
    stream.set_write_timeout(Some(Duration::from_secs(10))).map_err(|e| ProviderError::new(e.to_string()))?;
    let head = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAccept: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        endpoint.path,
        endpoint.addr,
        body.len()
    );
    stream.write_all(head.as_bytes()).and_then(|_| stream.write_all(body.as_bytes())).map_err(|e| io_error("sending to the local engine failed", e))?;

    // `raw` holds the head, and the whole response when it is not an event stream (an error or a plain JSON reply)
    let mut raw: Vec<u8> = Vec::new();
    let mut reader: Option<StreamReader> = None;
    let mut buf = [0u8; 8192];
    let mut total = 0usize;
    loop {
        let n = match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => return Err(io_error("reading from the local engine failed", e)),
        };
        total += n;
        if total > MAX_REPLY_BYTES + 16 * 1024 {
            return Err(ProviderError::new("the reply is too large"));
        }
        if let Some(r) = reader.as_mut() {
            if !r.feed(&buf[..n], piece).map_err(ProviderError::new)? {
                break;
            }
            continue;
        }
        raw.extend_from_slice(&buf[..n]);
        if let Some(h) = parse_head(&raw).map_err(ProviderError::new)? {
            if (200..300).contains(&h.status) && h.event_stream {
                let mut r = StreamReader::new(h.chunked);
                let first = raw.split_off(h.size);
                raw.clear();
                let more = r.feed(&first, piece).map_err(ProviderError::new)?;
                reader = Some(r);
                if !more {
                    break;
                }
            }
        }
    }
    if let Some(r) = reader {
        return r.finish(piece);
    }
    let (status, reply) = split_response(&raw).map_err(ProviderError::new)?;
    let text = String::from_utf8(reply).map_err(|_| ProviderError::new("the engine's reply is not UTF-8"))?;
    if !(200..300).contains(&status) {
        let detail = json::parse(&text).ok().and_then(|j| error_message(&j)).unwrap_or_else(|| text.chars().take(200).collect());
        return Err(ProviderError::new(format!("the local engine answered {status}: {detail}")));
    }
    // the engine ignored `stream`: one JSON reply, passed on as a single piece
    let reply = parse_reply(&text)?;
    if let Reply::Respond(segments) = &reply {
        for s in segments {
            if let Segment::Text(t) = s {
                if !piece(t) {
                    break;
                }
            }
        }
    }
    Ok(reply)
}

impl StreamingModelProvider for LocalModelProvider {
    fn generate_stream(&self, request: &Transcript, piece: &mut dyn FnMut(&str) -> bool) -> Result<Reply, ProviderError> {
        let body = self.streaming_request_body(request);
        stream_post(self.endpoint(), &body, self.timeout(), piece)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language_model::{ModelProvider, DynamicInstructions, Entry, Profile, Session, ToolSet};
    use crate::language_model_stream::{run_turn_streamed, StreamOutcome};
    use crate::local_model::LocalEndpoint;
    use std::net::TcpListener;

    fn event(text: &str) -> String {
        format!("{{\"choices\":[{{\"delta\":{{\"content\":{}}}}}]}}", Json::Str(text.into()).encode())
    }

    fn run(payloads: &[&str]) -> (Result<Reply, ProviderError>, Vec<String>) {
        let mut pieces = Vec::new();
        let mut acc = StreamAccumulator::new();
        for p in payloads {
            if let Err(e) = acc.push(p, &mut |t| {
                pieces.push(t.to_string());
                true
            }) {
                return (Err(e), pieces);
            }
        }
        let reply = acc.finish(&mut |t| {
            pieces.push(t.to_string());
            true
        });
        (reply, pieces)
    }

    #[test]
    fn chunked_bytes_decode_across_any_split() {
        let body = b"5\r\nhello\r\n6;ext=1\r\n world\r\n0\r\n\r\n";
        for split in 0..body.len() {
            let mut c = ChunkedStream::new();
            let mut out = c.feed(&body[..split]).unwrap();
            out.extend(c.feed(&body[split..]).unwrap());
            assert_eq!(out, b"hello world", "split at {split}");
            assert!(c.is_done());
        }
        let mut one_by_one = ChunkedStream::new();
        let mut out = Vec::new();
        for b in body {
            out.extend(one_by_one.feed(&[*b]).unwrap());
        }
        assert_eq!(out, b"hello world");
    }

    #[test]
    fn broken_chunks_are_refused() {
        assert!(ChunkedStream::new().feed(b"zz\r\n").is_err());
        assert!(ChunkedStream::new().feed(b"3\r\nabcXX").is_err());
        assert!(ChunkedStream::new().feed(&[b'1'; 100]).is_err(), "a size line that never ends");
    }

    #[test]
    fn sse_events_are_split_joined_and_filtered() {
        let mut p = SseParser::new();
        let mut got = p.feed(b": keep-alive\r\ndata: {\"a\":1}\r\n\r\nevent: x\ndata: one\ndata: two\n");
        assert_eq!(got, ["{\"a\":1}"]);
        got = p.feed(b"\ndata:no-space\n\n");
        assert_eq!(got, ["one\ntwo", "no-space"]);
        // a line cut in the middle waits for the rest
        assert!(p.feed(b"data: [DO").is_empty());
        assert_eq!(p.feed(b"NE]\n\n"), ["[DONE]"]);
        // an event never closed is delivered at the end
        assert!(p.feed(b"data: tail").is_empty());
        assert_eq!(p.finish(), ["tail"]);
    }

    #[test]
    fn answer_text_arrives_in_pieces() {
        let (reply, pieces) = run(&[&event("  Hel"), &event("lo"), &event(" there"), "[DONE]"]);
        assert_eq!(pieces, ["Hel", "lo", " there"], "leading space is dropped, the rest is passed as it came");
        assert_eq!(reply.unwrap(), Reply::Respond(vec![Segment::Text("Hello there".into())]));
    }

    #[test]
    fn text_that_may_be_a_tool_call_is_held_and_not_shown() {
        let (reply, pieces) = run(&[&event("{\"tool_calls\":[{\"name\":\"we"), &event("ather\",\"arguments\":{\"city\":\"Oslo\"}}]}")]);
        assert!(pieces.is_empty(), "{pieces:?}");
        match reply.unwrap() {
            Reply::Call(calls) => assert_eq!((calls[0].name.as_str(), calls[0].arguments.as_str()), ("weather", "{\"city\":\"Oslo\"}")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn held_text_that_is_an_answer_is_shown_at_the_end() {
        let (reply, pieces) = run(&[&event("{not a call}")]);
        assert_eq!(pieces, ["{not a call}"]);
        assert_eq!(reply.unwrap(), Reply::Respond(vec![Segment::Text("{not a call}".into())]));
        let (_, fenced) = run(&[&event("```rust\nfn a() {}\n```")]);
        assert_eq!(fenced, ["```rust\nfn a() {}\n```"]);
    }

    #[test]
    fn native_tool_call_deltas_are_joined_by_index() {
        let a = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"time","arguments":""}},{"index":1,"function":{"name":"weather","arguments":"{\"ci"}}]}}]}"#;
        let b = r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":"ty\":\"Oslo\"}"}}]}}]}"#;
        let (reply, pieces) = run(&[a, b, "[DONE]"]);
        assert!(pieces.is_empty());
        assert_eq!(
            reply.unwrap(),
            Reply::Call(vec![ToolCall { name: "time".into(), arguments: String::new() }, ToolCall { name: "weather".into(), arguments: "{\"city\":\"Oslo\"}".into() }])
        );
    }

    #[test]
    fn engine_errors_and_empty_streams_are_reported() {
        assert_eq!(run(&[r#"{"error":{"message":"out of memory"}}"#]).0.unwrap_err().message, "out of memory");
        assert!(run(&["not json"]).0.is_err());
        assert!(run(&["[DONE]"]).0.is_err(), "no text and no call");
        assert!(run(&[r#"{"choices":[{"delta":{"role":"assistant"}}]}"#, "[DONE]"]).0.is_err());
    }

    #[test]
    fn stopping_ends_the_stream_with_the_text_so_far() {
        let mut acc = StreamAccumulator::new();
        let mut got = String::new();
        let go = acc
            .push(&event("Hello"), &mut |t| {
                got.push_str(t);
                false
            })
            .unwrap();
        assert!(!go);
        assert_eq!(acc.stopped(), Reply::Respond(vec![Segment::Text("Hello".into())]));
        assert_eq!(StreamAccumulator::new().stopped(), Reply::Respond(vec![]));
    }

    /// Serves one response in separate writes (with a pause so the client sees them as separate reads) and returns the request.
    fn serve_pieces(pieces: Vec<Vec<u8>>) -> (std::net::SocketAddr, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut got = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = s.read(&mut buf).unwrap();
                got.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&got).to_string();
                if let Some(h) = text.find("\r\n\r\n") {
                    let want: usize = text[..h].lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse().ok())).unwrap_or(0);
                    if got.len() >= h + 4 + want || n == 0 {
                        break;
                    }
                }
            }
            for p in pieces {
                if s.write_all(&p).is_err() {
                    break;
                }
                let _ = s.flush();
                std::thread::sleep(Duration::from_millis(15));
            }
            String::from_utf8_lossy(&got).to_string()
        });
        (addr, handle)
    }

    fn chunk(data: &str) -> Vec<u8> {
        format!("{:x}\r\n{}\r\n", data.len(), data).into_bytes()
    }

    fn provider_at(addr: std::net::SocketAddr) -> LocalModelProvider {
        LocalModelProvider::new(LocalEndpoint { addr, path: "/v1/chat/completions".into() }, "local-small")
    }

    fn sse(text: &str) -> String {
        format!("data: {}\n\n", event(text))
    }

    fn prompt(text: &str) -> Transcript {
        Transcript::new(vec![Entry::Prompt(vec![Segment::Text(text.into())])])
    }

    #[test]
    fn a_streamed_reply_reaches_the_view_piece_by_piece() {
        let head = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
        let (addr, served) = serve_pieces(vec![
            head,
            chunk(&sse("Four")),
            // an event split across two chunks
            chunk(&sse(" is")[..10]),
            chunk(&sse(" is")[10..]),
            chunk(&sse(" it.")),
            chunk("data: [DONE]\n\n"),
            b"0\r\n\r\n".to_vec(),
        ]);
        let mut session = Session::new(Profile::new("brief", DynamicInstructions::new().fixed("Be brief.")));
        session.prompt(vec![Segment::Text("2+2?".into())]);
        let mut seen = Vec::new();
        let out = run_turn_streamed(&mut session, &ToolSet::new(), &provider_at(addr), 1000, 0, 2, &mut |t| {
            seen.push(t.to_string());
            true
        });
        assert_eq!(out, Ok(StreamOutcome::Finished(vec![Segment::Text("Four is it.".into())])));
        assert_eq!(seen, ["Four", "Four is", "Four is it."], "the view gets the text so far after every piece");
        assert_eq!(session.history().len(), 2);
        let request = served.join().unwrap();
        assert!(request.contains("\"stream\":true"), "{request}");
        assert!(request.contains("Accept: text/event-stream"));
    }

    #[test]
    fn stopping_a_stream_records_nothing_in_the_session() {
        let head = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n".to_vec();
        let (addr, served) = serve_pieces(vec![head, sse("Once").into_bytes(), sse(" upon").into_bytes(), sse(" a time").into_bytes()]);
        let mut session = Session::new(Profile::new("brief", DynamicInstructions::new().fixed("Be brief.")));
        session.prompt(vec![Segment::Text("Story".into())]);
        let out = run_turn_streamed(&mut session, &ToolSet::new(), &provider_at(addr), 1000, 0, 2, &mut |t| t.len() < 8);
        assert_eq!(out, Ok(StreamOutcome::Stopped("Once upon".into())));
        assert_eq!(session.history().len(), 1, "only the prompt");
        let _ = served.join();
    }

    #[test]
    fn a_plain_json_reply_is_passed_as_one_piece() {
        let body = r#"{"choices":[{"message":{"content":"Four."}}]}"#;
        let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).into_bytes();
        let (addr, served) = serve_pieces(vec![resp]);
        let mut pieces = Vec::new();
        let reply = provider_at(addr).generate_stream(&prompt("2+2?"), &mut |t| {
            pieces.push(t.to_string());
            true
        });
        assert_eq!(reply.unwrap(), Reply::Respond(vec![Segment::Text("Four.".into())]));
        assert_eq!(pieces, ["Four."]);
        served.join().unwrap();
    }

    #[test]
    fn an_error_status_carries_the_engines_message() {
        let body = r#"{"error":{"message":"context too long"}}"#;
        let resp = format!("HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).into_bytes();
        let (addr, served) = serve_pieces(vec![resp]);
        let err = provider_at(addr).generate_stream(&prompt("x"), &mut |_| true).unwrap_err();
        assert!(err.message.contains("400") && err.message.contains("context too long"), "{}", err.message);
        served.join().unwrap();
    }

    #[test]
    fn a_streamed_prompted_tool_call_is_a_call_and_shows_no_text() {
        let call = "{\"tool_calls\":[{\"name\":\"time\",\"arguments\":{}}]}";
        let head = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n".to_vec();
        let (addr, served) = serve_pieces(vec![head, sse(&call[..12]).into_bytes(), sse(&call[12..]).into_bytes(), b"data: [DONE]\n\n".to_vec()]);
        let mut pieces = Vec::new();
        let reply = provider_at(addr).generate_stream(&prompt("time?"), &mut |t| {
            pieces.push(t.to_string());
            true
        });
        assert_eq!(reply.unwrap(), Reply::Call(vec![ToolCall { name: "time".into(), arguments: "{}".into() }]));
        assert!(pieces.is_empty());
        served.join().unwrap();
    }

    #[test]
    fn the_streaming_body_asks_for_a_stream() {
        let p = LocalModelProvider::new(LocalEndpoint::parse("http://127.0.0.1:1/v1").unwrap(), "m");
        let streaming = json::parse(&p.streaming_request_body(&prompt("hi"))).unwrap();
        assert_eq!(streaming.get("stream"), Some(&Json::Bool(true)));
        let plain = json::parse(&p.request_body(&prompt("hi"))).unwrap();
        assert_eq!(plain.get("stream"), Some(&Json::Bool(false)));
        assert_eq!(p.name(), "m");
    }
}
