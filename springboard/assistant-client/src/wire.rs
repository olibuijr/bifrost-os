//! The Anthropic Messages wire format both clients speak: the request bodies, the message and tool-result JSON, the
//! tool-call records of one streamed turn, and the error text. Pure: no sockets, so every shape is testable.

use bifrost_core::json::{self, quote, Json};

pub const HOST: &str = "api.anthropic.com";
pub const VERSION: &str = "2023-06-01";
/// Output limit of a streamed reply.
pub const STREAM_MAX_TOKENS: u32 = 4096;

/// A tool call the model made in one request, with its arguments as JSON text.
#[derive(Clone, Debug, PartialEq)]
pub struct Use {
    pub index: u64,
    pub id: String,
    pub name: String,
    pub input: String,
}

/// What one streamed request produced: the text it streamed and the tool calls it made, in order.
#[derive(Debug, PartialEq)]
pub struct Turn {
    pub text: String,
    pub calls: Vec<Use>,
}

/// The JSON text of one message in the Messages API form with plain text content.
pub fn text_message(user: bool, text: &str) -> String {
    format!("{{\"role\":\"{}\",\"content\":{}}}", if user { "user" } else { "assistant" }, quote(text))
}

/// The JSON body of a streamed request from already formatted messages. `tools` is the JSON array of tool
/// definitions, if any.
pub fn stream_body(model: &str, max_tokens: u32, system: Option<&str>, wire: &[String], tools: Option<&str>) -> String {
    let system = system.map_or(String::new(), |s| format!("\"system\":{},", quote(s)));
    let tools = tools.map_or(String::new(), |t| format!("\"tools\":{t},"));
    format!("{{\"model\":{},\"max_tokens\":{max_tokens},{system}{tools}\"messages\":[{}],\"stream\":true}}", quote(model), wire.join(","))
}

/// The streamed request body for `messages` (`(is_user, text)` pairs in order) with no tools. `system` is the skill's
/// instructions, if any.
pub fn request_body(model: &str, system: Option<&str>, messages: &[(bool, String)]) -> String {
    let wire: Vec<String> = messages.iter().map(|(user, text)| text_message(*user, text)).collect();
    stream_body(model, STREAM_MAX_TOKENS, system, &wire, None)
}

/// The assistant message that records one turn: its text, then each tool call it made.
pub fn assistant_message(turn: &Turn) -> String {
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

/// The content block that hands a tool's result back to the model.
pub fn tool_result(id: &str, text: &str) -> String {
    format!("{{\"type\":\"tool_result\",\"tool_use_id\":{},\"content\":{}}}", quote(id), quote(text))
}

/// The message of an API error body (`{"type":"error","error":{"message":..}}`), else the raw status.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn wire_of(body: &str) -> Json {
        json::parse(body).unwrap()
    }

    #[test]
    fn request_snapshot_is_exact() {
        let b = request_body("claude-haiku-5-5", Some("Be \"brief\"."), &[(true, "hi \"x\"".into()), (false, "yo".into())]);
        assert_eq!(
            b,
            "{\"model\":\"claude-haiku-5-5\",\"max_tokens\":4096,\"system\":\"Be \\\"brief\\\".\",\"messages\":[{\"role\":\"user\",\"content\":\"hi \\\"x\\\"\"},{\"role\":\"assistant\",\"content\":\"yo\"}],\"stream\":true}"
        );
    }

    #[test]
    fn body_has_model_messages_and_stream() {
        let v = wire_of(&request_body("claude-sonnet-5-5", None, &[(true, "hi \"x\"".into()), (false, "yo".into())]));
        assert_eq!(v.get("stream"), Some(&Json::Bool(true)));
        assert_eq!(v.get("model").and_then(Json::as_str), Some("claude-sonnet-5-5"));
        assert_eq!(v.get("messages").and_then(Json::as_array).map(|m| m.len()), Some(2));
        assert_eq!(v.get("system"), None, "no skill, no system prompt");
        assert_eq!(v.get("tools"), None, "the plain body offers no tools");
    }

    #[test]
    fn a_tool_body_carries_its_tools_array() {
        let wire = vec![text_message(true, "what time is it")];
        let v = wire_of(&stream_body("claude-haiku-5-5", 4096, None, &wire, Some("[{\"name\":\"current_time\"}]")));
        assert_eq!(v.get("tools").and_then(Json::as_array).map(|t| t.len()), Some(1));
    }

    #[test]
    fn a_skill_adds_its_system_prompt() {
        let v = wire_of(&request_body("claude-haiku-5-5", Some("Be \"brief\"."), &[(true, "hi".into())]));
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
    fn a_tool_result_names_its_call() {
        let v = wire_of(&tool_result("toolu_9", "4 words"));
        assert_eq!(v.str_field("tool_use_id").as_deref(), Some("toolu_9"));
        assert_eq!(v.str_field("content").as_deref(), Some("4 words"));
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
