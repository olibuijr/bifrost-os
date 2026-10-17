//! Conversations, settings and the Markdown subset the transcript renders. Everything here is pure (no drawing,
//! no network); persistence is JSON files in the app's data directory.

use bifrost_app::coding::{self, CodingError, Decodable, Encodable, JSONDecoder, JSONEncoder, Value};
use bifrost_core::json::{self, quote};
use std::path::Path;

pub const MODELS: [(&str, &str); 3] = [("claude-sonnet-5-5", "Claude Sonnet 5.5"), ("claude-opus-5-5", "Claude Opus 5.5"), ("claude-haiku-5-5", "Claude Haiku 5.5")];
pub const DEFAULT_MODEL: &str = "claude-sonnet-5-5";
pub use assistant_client::{KEY_ACCOUNT, KEY_SERVICE};

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub user: bool,
    pub text: String,
    /// The tool calls the model made while writing this reply, in order. Empty for a message the user wrote.
    pub tools: Vec<ToolRun>,
}

/// A tool the model called in a reply: its name, the arguments it chose (JSON text) and the result it was given.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolRun {
    pub name: String,
    pub arguments: String,
    pub result: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub updated: u64,
    pub messages: Vec<Message>,
}

impl Conversation {
    pub fn new(id: String, now: u64) -> Self {
        Conversation { id, title: "New Chat".into(), updated: now, messages: Vec::new() }
    }

    /// The sidebar title: the first line of the first user message, cut at 40 characters.
    pub fn retitle(&mut self) {
        if let Some(m) = self.messages.iter().find(|m| m.user) {
            let line = m.text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
            let mut t: String = line.chars().take(40).collect();
            if line.chars().count() > 40 {
                t.push('…');
            }
            if !t.is_empty() {
                self.title = t;
            }
        }
    }

    /// `(is_user, text)` pairs for the API; empty messages (a reply that has not started) are left out.
    pub fn history(&self) -> Vec<(bool, String)> {
        self.messages.iter().filter(|m| !m.text.trim().is_empty()).map(|m| (m.user, m.text.clone())).collect()
    }
}

bifrost_app::codable!(Conversation { id: String, title: String = "New Chat".into(), updated: u64 = 0, messages: Vec<Message> = Vec::new() });

impl Encodable for Message {
    fn encode(&self) -> Value {
        Value::Dict(vec![
            ("role".into(), Value::Str(if self.user { "user" } else { "assistant" }.into())),
            ("text".into(), Value::Str(self.text.clone())),
            ("tools".into(), self.tools.encode()),
        ])
    }
}
impl Decodable for Message {
    fn decode(v: &Value) -> Result<Self, CodingError> {
        // Conversations saved before tool calls existed have no "tools" member: they load with none.
        let tools = coding::field::<Vec<ToolRun>>(v, "tools").unwrap_or_default();
        Ok(Message { user: coding::field::<String>(v, "role")? == "user", text: coding::field(v, "text")?, tools })
    }
}
impl Encodable for ToolRun {
    fn encode(&self) -> Value {
        Value::Dict(vec![
            ("name".into(), Value::Str(self.name.clone())),
            ("arguments".into(), Value::Str(self.arguments.clone())),
            ("result".into(), Value::Str(self.result.clone())),
        ])
    }
}
impl Decodable for ToolRun {
    fn decode(v: &Value) -> Result<Self, CodingError> {
        Ok(ToolRun { name: coding::field(v, "name")?, arguments: coding::field(v, "arguments")?, result: coding::field(v, "result")? })
    }
}

/// The file's top-level object: `{"conversations": [...]}`.
struct Store(Vec<Conversation>);
impl Encodable for Store {
    fn encode(&self) -> Value {
        Value::Dict(vec![("conversations".into(), self.0.encode())])
    }
}
impl Decodable for Store {
    fn decode(v: &Value) -> Result<Self, CodingError> {
        Ok(Store(coding::field(v, "conversations")?))
    }
}

pub fn to_json(convs: &[Conversation]) -> String {
    JSONEncoder::new().encode(&Store(convs.to_vec())) + "\n"
}

/// A missing or malformed file gives no conversations.
pub fn from_json(text: &str) -> Vec<Conversation> {
    JSONDecoder::new().decode::<Store>(text).map(|s| s.0).unwrap_or_default()
}

/// Newest first, as the sidebar lists them.
pub fn load(dir: &Path) -> Vec<Conversation> {
    let mut v = std::fs::read_to_string(dir.join("conversations.json")).map(|t| from_json(&t)).unwrap_or_default();
    v.sort_by(|a, b| b.updated.cmp(&a.updated));
    v
}

/// Written to a temporary file and renamed, so a crash never leaves half a file.
pub fn save(dir: &Path, convs: &[Conversation]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join("conversations.json.tmp");
    std::fs::write(&tmp, to_json(convs))?;
    std::fs::rename(tmp, dir.join("conversations.json"))
}

pub fn load_model(dir: &Path) -> String {
    let saved = std::fs::read_to_string(dir.join("settings.json")).ok().and_then(|t| json::parse(&t).ok()).and_then(|v| v.str_field("model"));
    saved.filter(|m| MODELS.iter().any(|(id, _)| id == m)).unwrap_or_else(|| DEFAULT_MODEL.to_string())
}

pub fn save_model(dir: &Path, model: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("settings.json"), format!("{{\"model\":{}}}\n", quote(model)))
}

// Markdown --------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub code: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    /// hard lines of one paragraph, each a list of styled spans
    Para(Vec<Vec<Span>>),
    /// a fenced code block's lines (an unclosed fence, as while streaming, runs to the end)
    Code(Vec<String>),
}

/// Inline `**bold**` and `` `code` `` of one line. An unmatched marker just styles the rest of the line.
pub fn inline(line: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let (mut cur, mut bold, mut code) = (String::new(), false, false);
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let flush = |cur: &mut String, spans: &mut Vec<Span>, bold: bool, code: bool| {
        if !cur.is_empty() {
            spans.push(Span { text: std::mem::take(cur), bold, code });
        }
    };
    while i < chars.len() {
        if chars[i] == '`' {
            flush(&mut cur, &mut spans, bold, code);
            code = !code;
            i += 1;
        } else if !code && chars[i] == '*' && chars.get(i + 1) == Some(&'*') {
            flush(&mut cur, &mut spans, bold, code);
            bold = !bold;
            i += 2;
        } else {
            cur.push(chars[i]);
            i += 1;
        }
    }
    flush(&mut cur, &mut spans, bold, code);
    spans
}

/// Paragraphs, headings (shown bold), `-`/`*` bullets and fenced code blocks.
pub fn parse(md: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut para: Vec<Vec<Span>> = Vec::new();
    let mut fence: Option<Vec<String>> = None;
    for raw in md.lines() {
        if let Some(lines) = fence.as_mut() {
            if raw.trim_start().starts_with("```") {
                blocks.push(Block::Code(fence.take().unwrap_or_default()));
            } else {
                lines.push(raw.to_string());
            }
            continue;
        }
        let line = raw.trim_end();
        if line.trim_start().starts_with("```") {
            if !para.is_empty() {
                blocks.push(Block::Para(std::mem::take(&mut para)));
            }
            fence = Some(Vec::new());
        } else if line.trim().is_empty() {
            if !para.is_empty() {
                blocks.push(Block::Para(std::mem::take(&mut para)));
            }
        } else if line.trim_start().starts_with('#') {
            let mut spans = inline(line.trim_start().trim_start_matches('#').trim());
            for s in &mut spans {
                s.bold = true;
            }
            para.push(spans);
        } else if let Some(item) = line.trim_start().strip_prefix("- ").or_else(|| line.trim_start().strip_prefix("* ")) {
            let mut spans = vec![Span { text: "• ".into(), bold: false, code: false }];
            spans.extend(inline(item));
            para.push(spans);
        } else {
            para.push(inline(line));
        }
    }
    if let Some(lines) = fence {
        blocks.push(Block::Code(lines));
    }
    if !para.is_empty() {
        blocks.push(Block::Para(para));
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversations_round_trip_through_json() {
        let mut c = Conversation::new("1".into(), 5);
        c.messages.push(Message { user: true, text: "Hi \"there\"\nline two".into(), tools: Vec::new() });
        c.messages.push(Message { user: false, text: "Hello".into(), tools: vec![ToolRun { name: "count_words".into(), arguments: "{\"text\":\"a\"}".into(), result: "1 words".into() }] });
        c.retitle();
        assert_eq!(c.title, "Hi \"there\"");
        assert_eq!(from_json(&to_json(&[c.clone()])), vec![c]);
    }

    #[test]
    fn a_saved_message_without_tools_loads_with_none() {
        let old = "{\"conversations\":[{\"id\":\"1\",\"title\":\"T\",\"updated\":5,\"messages\":[{\"role\":\"assistant\",\"text\":\"Hello\"}]}]}";
        let convs = from_json(old);
        assert_eq!(convs[0].messages, vec![Message { user: false, text: "Hello".into(), tools: Vec::new() }]);
    }

    #[test]
    fn markdown_blocks_and_inline_styles() {
        let b = parse("Hello **bold** and `code`\n\n```rust\nfn main() {}\n```\n- item");
        assert_eq!(b.len(), 3);
        let Block::Para(lines) = &b[0] else { panic!() };
        assert_eq!(lines[0].iter().map(|s| (s.text.as_str(), s.bold, s.code)).collect::<Vec<_>>(), vec![("Hello ", false, false), ("bold", true, false), (" and ", false, false), ("code", false, true)]);
        assert_eq!(b[1], Block::Code(vec!["fn main() {}".into()]));
        let Block::Para(items) = &b[2] else { panic!() };
        assert_eq!(items[0][0].text, "• ");
    }

    #[test]
    fn an_open_fence_runs_to_the_end() {
        assert_eq!(parse("```\nlet x"), vec![Block::Code(vec!["let x".into()])]);
    }

    #[test]
    fn history_skips_the_empty_reply() {
        let mut c = Conversation::new("1".into(), 0);
        c.messages.push(Message { user: true, text: "q".into(), tools: Vec::new() });
        c.messages.push(Message { user: false, text: String::new(), tools: Vec::new() });
        assert_eq!(c.history(), vec![(true, "q".to_string())]);
    }
}
