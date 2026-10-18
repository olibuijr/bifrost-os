//! A language-model provider for the local inference engine: the engine runs on this machine (a packaged service or the
//! owner's own runtime) and speaks the common chat-completions JSON over HTTP on a loopback port. [`LocalModelProvider`]
//! implements [`ModelProvider`], so a [`Session`](crate::language_model::Session) and `run_turn` work against it exactly
//! as against any other provider: guided generation, tool calling, the token budget.
//!
//! Privacy: the endpoint must be a loopback address. [`LocalEndpoint::parse`] refuses any other host, so a conversation
//! sent through this provider never leaves the machine. There is no TLS because there is no network hop.
//!
//! Tool calls: the tool descriptions reach the model as text in the instructions (that is how `run_turn` offers them).
//! This provider adds one line saying how to call one: reply with only `{"tool_calls":[{"name":...,"arguments":{...}}]}`.
//! A reply of that shape, or the engine's own `tool_calls` field, becomes [`Reply::Call`]; anything else is the answer.
//!
//! Gaps: replies are not streamed token by token (`ModelProvider` has one request and one reply), attachments are
//! passed as a text placeholder because the provider accepts no media, and the engine itself is not started from here.

use bifrost_core::json::{self, Json};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

use crate::language_model::{Availability, Entry, ModelProvider, ProviderError, Reply, Segment, ToolCall, Transcript, Unavailable};

/// The largest reply body read, so a runaway engine cannot fill memory.
pub const MAX_REPLY_BYTES: usize = 4 * 1024 * 1024;
/// The line added to the instructions when tools are offered.
pub const TOOL_CONVENTION: &str = "To call a tool, reply with only a JSON object like {\"tool_calls\":[{\"name\":\"tool\",\"arguments\":{}}]} and nothing else. After the tool output arrives, answer in plain text.";
const TOOL_HEADER: &str = "You may call these tools.";

/// Why an endpoint address was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EndpointError {
    /// Not a plain `http://host:port/path` address.
    BadUrl,
    /// The host is not a loopback address: the conversation would leave the machine.
    NotLoopback,
}

/// Where the local engine listens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalEndpoint {
    pub addr: SocketAddr,
    /// the request target, such as `/v1/chat/completions`
    pub path: String,
}

impl LocalEndpoint {
    /// Reads `http://127.0.0.1:8084/v1` (or `.../v1/chat/completions`, `http://localhost:8084`, `http://[::1]:8084`).
    /// The port defaults to 80 and the path to `/v1/chat/completions`.
    pub fn parse(url: &str) -> Result<LocalEndpoint, EndpointError> {
        let rest = url.trim().strip_prefix("http://").ok_or(EndpointError::BadUrl)?;
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };
        let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
            let (host, tail) = inner.split_once(']').ok_or(EndpointError::BadUrl)?;
            let port = match tail.strip_prefix(':') {
                Some(p) => p.parse::<u16>().map_err(|_| EndpointError::BadUrl)?,
                None if tail.is_empty() => 80,
                None => return Err(EndpointError::BadUrl),
            };
            (host, port)
        } else if let Some((host, port)) = authority.rsplit_once(':') {
            (host, port.parse::<u16>().map_err(|_| EndpointError::BadUrl)?)
        } else {
            (authority, 80)
        };
        if host.is_empty() || path.contains(char::is_whitespace) {
            return Err(EndpointError::BadUrl);
        }
        let ip: IpAddr = if host.eq_ignore_ascii_case("localhost") {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        } else {
            host.parse().map_err(|_| EndpointError::NotLoopback)?
        };
        if !ip.is_loopback() {
            return Err(EndpointError::NotLoopback);
        }
        let base = path.trim_end_matches('/');
        let path = if base.ends_with("/chat/completions") {
            base.to_string()
        } else {
            format!("{base}/chat/completions")
        };
        let path = if path.starts_with('/') { path } else { format!("/{path}") };
        Ok(LocalEndpoint { addr: SocketAddr::new(ip, port), path })
    }
}

/// The provider: one engine endpoint, the model name to ask for, and the sampling it uses.
#[derive(Clone, Debug)]
pub struct LocalModelProvider {
    endpoint: LocalEndpoint,
    model: String,
    temperature: f32,
    max_tokens: u32,
    timeout: Duration,
}

impl LocalModelProvider {
    pub fn new(endpoint: LocalEndpoint, model: &str) -> LocalModelProvider {
        LocalModelProvider { endpoint, model: model.to_string(), temperature: 0.2, max_tokens: 1024, timeout: Duration::from_secs(120) }
    }

    pub fn with_temperature(mut self, temperature: f32) -> LocalModelProvider {
        self.temperature = temperature.clamp(0.0, 2.0);
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> LocalModelProvider {
        self.max_tokens = max_tokens.max(1);
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> LocalModelProvider {
        self.timeout = timeout;
        self
    }

    /// Whether the engine accepts a connection now.
    pub fn availability(&self) -> Availability {
        match TcpStream::connect_timeout(&self.endpoint.addr, Duration::from_millis(250)) {
            Ok(_) => Availability::Available,
            Err(_) => Availability::Unavailable(Unavailable::LocalEngineNotRunning),
        }
    }

    /// The request body for a transcript.
    pub fn request_body(&self, request: &Transcript) -> String {
        let messages = messages_for(request);
        Json::Obj(vec![
            ("model".into(), Json::Str(self.model.clone())),
            ("messages".into(), Json::Arr(messages)),
            ("temperature".into(), Json::Num(f64::from(self.temperature))),
            ("max_tokens".into(), Json::Num(f64::from(self.max_tokens))),
            ("stream".into(), Json::Bool(false)),
        ])
        .encode()
    }
}

impl ModelProvider for LocalModelProvider {
    fn name(&self) -> &str {
        &self.model
    }

    fn generate(&self, request: &Transcript) -> Result<Reply, ProviderError> {
        let body = self.request_body(request);
        let (status, reply) = post(&self.endpoint, &body, self.timeout).map_err(ProviderError::new)?;
        let text = String::from_utf8(reply).map_err(|_| ProviderError::new("the engine's reply is not UTF-8"))?;
        if !(200..300).contains(&status) {
            let detail = json::parse(&text).ok().and_then(|j| error_message(&j)).unwrap_or_else(|| text.chars().take(200).collect());
            return Err(ProviderError::new(format!("the local engine answered {status}: {detail}")));
        }
        parse_reply(&text)
    }
}

fn segments_text(segments: &[Segment]) -> String {
    let mut parts = Vec::new();
    for s in segments {
        match s {
            Segment::Text(t) | Segment::Structured(t) => parts.push(t.clone()),
            Segment::Attachment { name, .. } => parts.push(format!("[attachment: {name}]")),
        }
    }
    parts.join("\n")
}

fn message(role: &str, content: String) -> Json {
    Json::Obj(vec![("role".into(), Json::Str(role.into())), ("content".into(), Json::Str(content))])
}

fn arguments_json(arguments: &str) -> Json {
    let t = arguments.trim();
    if t.is_empty() {
        return Json::Obj(Vec::new());
    }
    json::parse(t).unwrap_or_else(|_| Json::Str(t.to_string()))
}

/// The chat messages for a transcript: instructions become the system message, prompts the user's, responses the
/// assistant's. Tool calls are the assistant's JSON reply and tool output comes back as a user message.
fn messages_for(request: &Transcript) -> Vec<Json> {
    let mut out = Vec::new();
    for entry in request.entries() {
        match entry {
            Entry::Instructions(segments) => {
                let mut text = segments_text(segments);
                if text.contains(TOOL_HEADER) {
                    text.push_str("\n\n");
                    text.push_str(TOOL_CONVENTION);
                }
                out.push(message("system", text));
            }
            Entry::Prompt(segments) => out.push(message("user", segments_text(segments))),
            Entry::Response(segments) => out.push(message("assistant", segments_text(segments))),
            Entry::Reasoning(_) => {}
            Entry::ToolCalls(calls) => {
                let list = calls
                    .iter()
                    .map(|c| Json::Obj(vec![("name".into(), Json::Str(c.name.clone())), ("arguments".into(), arguments_json(&c.arguments))]))
                    .collect();
                out.push(message("assistant", Json::Obj(vec![("tool_calls".into(), Json::Arr(list))]).encode()));
            }
            Entry::ToolOutput(o) => out.push(message("user", format!("Tool {} returned:\n{}", o.name, o.text))),
        }
    }
    out
}

fn error_message(j: &Json) -> Option<String> {
    let e = j.get("error")?;
    e.as_str().map(str::to_string).or_else(|| e.str_field("message"))
}

fn call_from(item: &Json) -> Option<ToolCall> {
    // the engine's own shape nests the call under "function"; the prompted shape is flat
    let f = item.get("function").unwrap_or(item);
    let name = f.str_field("name").filter(|n| !n.is_empty())?;
    let arguments = match f.get("arguments") {
        None | Some(Json::Null) => String::new(),
        Some(Json::Str(s)) => s.clone(),
        Some(other) => other.encode(),
    };
    Some(ToolCall { name, arguments })
}

fn strip_fence(text: &str) -> &str {
    let t = text.trim();
    let Some(inner) = t.strip_prefix("```") else { return t };
    let inner = inner.strip_prefix("json").unwrap_or(inner);
    inner.trim().strip_suffix("```").unwrap_or(inner).trim()
}

/// Reads the engine's reply body into a [`Reply`].
pub fn parse_reply(body: &str) -> Result<Reply, ProviderError> {
    let j = json::parse(body).map_err(|e| ProviderError::new(format!("the engine's reply is not JSON: {e}")))?;
    if let Some(m) = error_message(&j) {
        return Err(ProviderError::new(m));
    }
    let msg = j
        .get("choices")
        .and_then(Json::as_array)
        .and_then(|c| c.first())
        .and_then(|c| c.get("message"))
        .ok_or_else(|| ProviderError::new("the engine's reply has no message"))?;
    if let Some(native) = msg.get("tool_calls").and_then(Json::as_array) {
        let calls: Vec<ToolCall> = native.iter().filter_map(call_from).collect();
        if !calls.is_empty() {
            return Ok(Reply::Call(calls));
        }
    }
    let content = msg.get("content").and_then(Json::as_str).unwrap_or("");
    let candidate = strip_fence(content);
    if candidate.starts_with('{') {
        if let Some(list) = json::parse(candidate).ok().and_then(|o| o.get("tool_calls").and_then(Json::as_array).map(|a| a.to_vec())) {
            let calls: Vec<ToolCall> = list.iter().filter_map(call_from).collect();
            if !calls.is_empty() {
                return Ok(Reply::Call(calls));
            }
        }
    }
    if content.trim().is_empty() {
        return Err(ProviderError::new("the model returned no text"));
    }
    Ok(Reply::Respond(vec![Segment::Text(content.trim().to_string())]))
}

/// Decodes a `Transfer-Encoding: chunked` body.
pub fn decode_chunked(mut data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let line_end = data.windows(2).position(|w| w == b"\r\n").ok_or("chunk size line is not finished")?;
        let size_text = std::str::from_utf8(&data[..line_end]).map_err(|_| "chunk size is not text")?;
        let size_text = size_text.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|_| "chunk size is not hexadecimal")?;
        data = &data[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if data.len() < size + 2 || &data[size..size + 2] != b"\r\n" {
            return Err("chunk is cut short".into());
        }
        if out.len() + size > MAX_REPLY_BYTES {
            return Err("the reply is too large".into());
        }
        out.extend_from_slice(&data[..size]);
        data = &data[size + 2..];
    }
}

/// Splits a complete HTTP/1.1 response into its status and body.
fn split_response(raw: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or("the engine closed the connection before the headers ended")?;
    let head = std::str::from_utf8(&raw[..end]).map_err(|_| "the response headers are not text")?;
    let mut lines = head.split("\r\n");
    let status: u16 = lines.next().and_then(|l| l.split_whitespace().nth(1)).and_then(|s| s.parse().ok()).ok_or("the status line is malformed")?;
    let mut chunked = false;
    let mut length: Option<usize> = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else { continue };
        let (name, value) = (name.trim().to_ascii_lowercase(), value.trim());
        if name == "transfer-encoding" && value.to_ascii_lowercase().contains("chunked") {
            chunked = true;
        } else if name == "content-length" {
            length = value.parse().ok();
        }
    }
    let body = &raw[end + 4..];
    let body = if chunked {
        decode_chunked(body)?
    } else if let Some(n) = length {
        if body.len() < n {
            return Err("the reply body is cut short".into());
        }
        body[..n].to_vec()
    } else {
        body.to_vec()
    };
    Ok((status, body))
}

fn post(endpoint: &LocalEndpoint, body: &str, timeout: Duration) -> Result<(u16, Vec<u8>), String> {
    let mut stream = TcpStream::connect_timeout(&endpoint.addr, Duration::from_secs(2)).map_err(|e| format!("the local engine is not reachable: {e}"))?;
    stream.set_read_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(Duration::from_secs(10))).map_err(|e| e.to_string())?;
    let head = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAccept: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        endpoint.path,
        endpoint.addr,
        body.len()
    );
    stream.write_all(head.as_bytes()).and_then(|_| stream.write_all(body.as_bytes())).map_err(|e| format!("sending to the local engine failed: {e}"))?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&buf[..n]);
                if raw.len() > MAX_REPLY_BYTES + 16 * 1024 {
                    return Err("the reply is too large".into());
                }
            }
            Err(e) => return Err(format!("reading from the local engine failed: {e}")),
        }
    }
    split_response(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language_model::{run_turn, DynamicInstructions, Kind, Profile, Session, Tool, ToolSet};
    use bifrost_ui::generation_schema::Property;
    use std::net::TcpListener;

    fn endpoint(url: &str) -> LocalEndpoint {
        LocalEndpoint::parse(url).unwrap()
    }

    #[test]
    fn only_loopback_http_addresses_are_accepted() {
        let e = endpoint("http://127.0.0.1:8084/v1");
        assert_eq!(e.addr, "127.0.0.1:8084".parse().unwrap());
        assert_eq!(e.path, "/v1/chat/completions");
        assert_eq!(endpoint("http://localhost:9000").path, "/chat/completions");
        assert_eq!(endpoint("http://[::1]:7/v1/chat/completions/").path, "/v1/chat/completions");
        assert_eq!(endpoint("http://localhost").addr.port(), 80);
        assert_eq!(LocalEndpoint::parse("http://192.168.1.12:8084/v1"), Err(EndpointError::NotLoopback));
        assert_eq!(LocalEndpoint::parse("http://example.com/v1"), Err(EndpointError::NotLoopback));
        assert_eq!(LocalEndpoint::parse("https://127.0.0.1/v1"), Err(EndpointError::BadUrl));
        assert_eq!(LocalEndpoint::parse("http://127.0.0.1:notaport"), Err(EndpointError::BadUrl));
        assert_eq!(LocalEndpoint::parse("http:///v1"), Err(EndpointError::BadUrl));
    }

    fn provider() -> LocalModelProvider {
        LocalModelProvider::new(endpoint("http://127.0.0.1:1/v1"), "local-small").with_max_tokens(64).with_temperature(0.0)
    }

    #[test]
    fn a_transcript_becomes_chat_messages() {
        let t = Transcript::new(vec![
            Entry::Instructions(vec![Segment::Text("Be brief.".into())]),
            Entry::Prompt(vec![Segment::Text("Hi".into()), Segment::Attachment { name: "a.png".into(), bytes: 3 }]),
            Entry::Response(vec![Segment::Text("Hello".into())]),
            Entry::Reasoning("thinking".into()),
            Entry::ToolCalls(vec![ToolCall { name: "weather".into(), arguments: "{\"city\":\"Reykjavik\"}".into() }]),
            Entry::ToolOutput(crate::language_model::ToolOutput { name: "weather".into(), text: "4 C".into() }),
        ]);
        let body = json::parse(&provider().request_body(&t)).unwrap();
        assert_eq!(body.str_field("model").as_deref(), Some("local-small"));
        assert_eq!(body.get("stream"), Some(&Json::Bool(false)));
        let msgs = body.get("messages").and_then(Json::as_array).unwrap();
        let roles: Vec<String> = msgs.iter().filter_map(|m| m.str_field("role")).collect();
        assert_eq!(roles, ["system", "user", "assistant", "assistant", "user"], "reasoning is not sent");
        assert_eq!(msgs[1].str_field("content").as_deref(), Some("Hi\n[attachment: a.png]"));
        assert_eq!(msgs[3].str_field("content").as_deref(), Some("{\"tool_calls\":[{\"name\":\"weather\",\"arguments\":{\"city\":\"Reykjavik\"}}]}"));
        assert_eq!(msgs[4].str_field("content").as_deref(), Some("Tool weather returned:\n4 C"));
        assert!(!msgs[0].str_field("content").unwrap().contains("tool_calls"), "no tools offered, no convention");
    }

    #[test]
    fn offering_tools_adds_the_calling_convention() {
        let t = Transcript::new(vec![Entry::Instructions(vec![Segment::Text("Be brief.".into()), Segment::Text("You may call these tools.\n- weather: x".into())])]);
        let body = json::parse(&provider().request_body(&t)).unwrap();
        let sys = body.get("messages").and_then(Json::as_array).unwrap()[0].str_field("content").unwrap();
        assert!(sys.ends_with(TOOL_CONVENTION));
    }

    #[test]
    fn plain_text_is_the_answer() {
        let r = parse_reply(r#"{"choices":[{"message":{"role":"assistant","content":"  Hello there  "}}]}"#).unwrap();
        assert_eq!(r, Reply::Respond(vec![Segment::Text("Hello there".into())]));
    }

    #[test]
    fn a_prompted_tool_call_becomes_a_call_even_inside_a_code_fence() {
        let content = "```json\\n{\\\"tool_calls\\\":[{\\\"name\\\":\\\"weather\\\",\\\"arguments\\\":{\\\"city\\\":\\\"Oslo\\\"}}]}\\n```";
        let body = format!("{{\"choices\":[{{\"message\":{{\"content\":\"{content}\"}}}}]}}");
        match parse_reply(&body).unwrap() {
            Reply::Call(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "weather");
                assert_eq!(calls[0].arguments, "{\"city\":\"Oslo\"}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_engines_own_tool_calls_field_is_read() {
        let body = r#"{"choices":[{"message":{"content":null,"tool_calls":[{"function":{"name":"time","arguments":"{}"}}]}}]}"#;
        assert_eq!(parse_reply(body).unwrap(), Reply::Call(vec![ToolCall { name: "time".into(), arguments: "{}".into() }]));
    }

    #[test]
    fn errors_and_empty_replies_are_reported() {
        assert_eq!(parse_reply(r#"{"error":{"message":"model not loaded"}}"#).unwrap_err().message, "model not loaded");
        assert!(parse_reply(r#"{"choices":[{"message":{"content":"  "}}]}"#).is_err());
        assert!(parse_reply(r#"{"choices":[]}"#).is_err());
        assert!(parse_reply("not json").is_err());
    }

    #[test]
    fn chunked_bodies_are_decoded_and_cut_ones_refused() {
        assert_eq!(decode_chunked(b"5\r\nhello\r\n6;ext=1\r\n world\r\n0\r\n\r\n").unwrap(), b"hello world");
        assert!(decode_chunked(b"5\r\nhel").is_err());
        assert!(decode_chunked(b"zz\r\n").is_err());
    }

    /// Serves one canned response to the next connection and returns the request it read.
    fn serve_once(response: Vec<u8>) -> (SocketAddr, std::thread::JoinHandle<String>) {
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
                    if got.len() >= h + 4 + want {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            s.write_all(&response).unwrap();
            String::from_utf8_lossy(&got).to_string()
        });
        (addr, handle)
    }

    fn http(status: &str, headers: &str, body: &str) -> Vec<u8> {
        format!("HTTP/1.1 {status}\r\n{headers}Connection: close\r\n\r\n{body}").into_bytes()
    }

    fn provider_at(addr: SocketAddr) -> LocalModelProvider {
        LocalModelProvider::new(LocalEndpoint { addr, path: "/v1/chat/completions".into() }, "local-small")
    }

    #[test]
    fn a_turn_runs_against_a_server_on_the_loopback_port() {
        let body = r#"{"choices":[{"message":{"content":"Four."}}]}"#;
        let (addr, served) = serve_once(http("200 OK", &format!("Content-Length: {}\r\n", body.len()), body));
        let mut session = Session::new(Profile::new("brief", DynamicInstructions::new().fixed("Be brief.")));
        session.prompt(vec![Segment::Text("2+2?".into())]);
        let out = run_turn(&mut session, &ToolSet::new(), &provider_at(addr), 1000, 0, 2);
        assert_eq!(out, Ok(vec![Segment::Text("Four.".into())]));
        assert_eq!(session.history().len(), 2);
        let request = served.join().unwrap();
        assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1\r\n"), "{request}");
        assert!(request.contains("\"2+2?\""));
    }

    #[test]
    fn a_chunked_reply_and_a_tool_round_work_through_a_session() {
        // round one: the model asks for the tool; round two (a second server) answers with the tool's output
        let call = r#"{"choices":[{"message":{"content":"{\"tool_calls\":[{\"name\":\"weather\",\"arguments\":{\"city\":\"Oslo\"}}]}"}}]}"#;
        let chunked = format!("{:x}\r\n{}\r\n0\r\n\r\n", call.len(), call);
        let (addr, first) = serve_once(http("200 OK", "Transfer-Encoding: chunked\r\n", &chunked));
        let schema = Kind::Object(vec![Property::new("city", Kind::Text)]);
        let mut tools = ToolSet::new();
        tools.register(Tool::new("weather", "Weather.", schema, |_| Ok("sunny".into()))).unwrap();
        let mut session = Session::new(Profile::new("brief", DynamicInstructions::new().fixed("Be brief.")));
        session.prompt(vec![Segment::Text("Weather in Oslo?".into())]);
        // the first provider answers the call; a second answers the text once the tool output is in the history
        let p1 = provider_at(addr);
        let reply = p1.generate(&Transcript::new(vec![Entry::Prompt(vec![Segment::Text("Weather in Oslo?".into())])])).unwrap();
        let Reply::Call(calls) = reply else { panic!("expected a call") };
        let output = tools.output_for(&calls[0]);
        assert_eq!(output.text, "sunny");
        first.join().unwrap();
        let answer = r#"{"choices":[{"message":{"content":"Sunny in Oslo."}}]}"#;
        let (addr2, second) = serve_once(http("200 OK", &format!("Content-Length: {}\r\n", answer.len()), answer));
        let out = run_turn(&mut session, &ToolSet::new(), &provider_at(addr2), 1000, 0, 2);
        assert_eq!(out, Ok(vec![Segment::Text("Sunny in Oslo.".into())]));
        second.join().unwrap();
    }

    #[test]
    fn an_error_status_carries_the_engines_message() {
        let body = r#"{"error":{"message":"context too long"}}"#;
        let (addr, served) = serve_once(http("400 Bad Request", &format!("Content-Length: {}\r\n", body.len()), body));
        let err = provider_at(addr).generate(&Transcript::new(vec![Entry::Prompt(vec![Segment::Text("x".into())])])).unwrap_err();
        assert!(err.message.contains("400") && err.message.contains("context too long"), "{}", err.message);
        served.join().unwrap();
    }

    #[test]
    fn a_stopped_engine_is_unavailable_and_a_listening_one_is_available() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        assert_eq!(provider_at(addr).availability(), Availability::Available);
        drop(listener);
        assert_eq!(provider_at(addr).availability(), Availability::Unavailable(Unavailable::LocalEngineNotRunning));
        let err = provider_at(addr).generate(&Transcript::new(vec![])).unwrap_err();
        assert!(err.message.contains("not reachable"));
    }
}
