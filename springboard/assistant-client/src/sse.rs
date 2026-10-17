//! Server-Sent Events reader for the Messages stream. Pure: bytes in, [`Piece`]s out.

use bifrost_core::json::{self, Json};

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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_split_line_waits_for_its_newline() {
        let mut p = SseParser::default();
        assert!(p.feed(b"data: {\"type\":\"message_s").is_empty());
        assert_eq!(p.feed(b"top\"}\n"), vec![Piece::MessageStop]);
    }
}
