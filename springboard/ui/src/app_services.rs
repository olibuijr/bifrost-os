//! App services: the Services menu entries an app declares in `manifest.toml` as `[[service]]` tables, the types of a
//! selection they take, and the control call that hands the selection to the providing app.
//!
//! ```toml
//! [[service]]
//! title = "Open in Viewer"
//! send_types = ["public.image", "public.pdf"]
//! return_types = []
//! method = "open_images"
//! ```
//!
//! The shell lists the services of the installed apps whose `send_types` include a type of the current selection
//! ([`selection_types`]). Choosing one opens the provider and calls `service.<method>` on its control socket with
//! [`request_line`]; the params are `{"type": <uti>, "text": ...}` or `{"type": <uti>, "paths": [...]}`. A provider
//! decodes them with [`ServiceCall::from_params`] (the SDK re-exports it as `bifrost_app::services`). When a service
//! declares `return_types`, the reply's `result.value` text ([`value_result`]) goes on the pasteboard, so the user
//! pastes it where they like; the shell does not type it into the frontmost app.

use crate::services_menu::Selection;
use bifrost_core::json::{self, Json};
use std::path::Path;

/// Plain text (`public.utf8-plain-text`).
pub const PLAIN_TEXT: &str = "public.utf8-plain-text";
/// A file URL: every file selection carries it.
pub const FILE_URL: &str = "public.file-url";
/// An image file, by extension (see [`is_image_path`]).
pub const IMAGE: &str = "public.image";
/// A PDF file, by extension.
pub const PDF: &str = "public.pdf";
/// Text that is one absolute path to a file. The shell's own type, not an Apple UTI, so a path copied as text can
/// reach a service that takes files.
pub const FILE_PATH_TEXT: &str = "bifrost.file-path-text";

const IMAGE_EXTENSIONS: [&str; 8] = ["png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff"];

/// One `[[service]]` table of a manifest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServiceDecl {
    pub title: String,
    pub send_types: Vec<String>,
    pub return_types: Vec<String>,
    pub method: String,
}

impl ServiceDecl {
    /// Sets one key of the table from its raw manifest value (`"text"`, or `["a", "b"]` for a list). Unknown keys are
    /// ignored.
    pub fn set(&mut self, key: &str, value: &str) {
        match key {
            "title" => self.title = unquote(value),
            "send_types" => self.send_types = list(value),
            "return_types" => self.return_types = list(value),
            "method" => self.method = unquote(value),
            _ => {}
        }
    }

    /// A service the shell offers: a title, at least one send type, and a method of `[a-z0-9_-]`.
    pub fn valid(&self) -> bool {
        !self.title.trim().is_empty()
            && !self.send_types.is_empty()
            && !self.method.is_empty()
            && self.method.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    }

    /// True when the service takes a selection with these types.
    pub fn accepts(&self, types: &[String]) -> bool {
        self.valid() && self.send_type_for(types).is_some()
    }

    /// The first of the service's send types that the selection carries: the type the call names.
    pub fn send_type_for(&self, types: &[String]) -> Option<&str> {
        self.send_types.iter().find(|s| types.contains(s)).map(String::as_str)
    }

    /// The control method the shell calls: `service.<method>`.
    pub fn rpc_method(&self) -> String {
        format!("service.{}", self.method)
    }
}

fn unquote(v: &str) -> String {
    v.trim().trim_matches('"').to_string()
}

/// `["a", "b"]` as its items; a bare value is a one-item list.
fn list(v: &str) -> Vec<String> {
    v.trim().trim_start_matches('[').trim_end_matches(']').split(',').map(unquote).filter(|s| !s.is_empty()).collect()
}

/// True for a file whose extension is a picture format the Viewer opens.
pub fn is_image_path(path: &str) -> bool {
    extension(path).is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.as_str()))
}

/// True for a file a viewer app can show: a picture or a PDF.
pub fn is_viewable_path(path: &str) -> bool {
    is_image_path(path) || extension(path).as_deref() == Some("pdf")
}

fn extension(path: &str) -> Option<String> {
    Path::new(path).extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase())
}

/// The types a text selection carries: plain text, and [`FILE_PATH_TEXT`] when the text is one absolute path that
/// `is_file` accepts (the shell passes the filesystem check; a test passes a fixed answer).
pub fn text_types(text: &str, is_file: impl Fn(&str) -> bool) -> Vec<String> {
    let mut types = vec![PLAIN_TEXT.to_string()];
    let t = text.trim();
    if t.starts_with('/') && !t.contains('\n') && is_file(t) {
        types.push(FILE_PATH_TEXT.to_string());
    }
    types
}

/// The types a file selection carries: [`FILE_URL`], plus [`IMAGE`] and [`PDF`] when a file has that extension.
pub fn file_types(paths: &[String]) -> Vec<String> {
    let mut types = vec![FILE_URL.to_string()];
    if paths.iter().any(|p| is_image_path(p)) {
        types.push(IMAGE.to_string());
    }
    if paths.iter().any(|p| extension(p).as_deref() == Some("pdf")) {
        types.push(PDF.to_string());
    }
    types
}

/// The types of `selection`, as the services match them (see [`text_types`] and [`file_types`]).
pub fn selection_types(selection: &Selection, is_file: impl Fn(&str) -> bool) -> Vec<String> {
    match selection {
        Selection::Text(t) => text_types(t, is_file),
        Selection::Files(files) => file_types(files),
    }
}

/// What a service receives: the selected text, or the selected paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Payload {
    Text(String),
    Paths(Vec<String>),
}

/// The payload for the send type `uti`: a path-as-text selection becomes its one path, other text stays text, and
/// files stay paths.
pub fn payload_for(uti: &str, selection: &Selection) -> Payload {
    match selection {
        Selection::Text(t) if uti == FILE_PATH_TEXT => Payload::Paths(vec![t.trim().to_string()]),
        Selection::Text(t) => Payload::Text(t.clone()),
        Selection::Files(files) => Payload::Paths(files.clone()),
    }
}

/// The JSON-RPC request line (one line, newline-terminated) that calls `service.<method>` with the selection.
pub fn request_line(method: &str, uti: &str, payload: &Payload) -> String {
    let body = match payload {
        Payload::Text(t) => format!("\"text\":{}", json_string(t)),
        Payload::Paths(paths) => format!("\"paths\":[{}]", paths.iter().map(|p| json_string(p)).collect::<Vec<_>>().join(",")),
    };
    format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"service.{method}\",\"params\":{{\"type\":{},{body}}}}}\n", json_string(uti))
}

/// The text a service returned (`result.value`), None when it returned none. An error object is `Err(message)`.
pub fn parse_reply(reply: &str) -> Result<Option<String>, String> {
    let v = json::parse(reply.trim()).map_err(|e| format!("malformed reply: {e}"))?;
    if let Some(e) = v.get("error") {
        return Err(e.str_field("message").unwrap_or_else(|| "error".into()));
    }
    Ok(v.get("result").and_then(|r| r.get("value")).and_then(Json::as_str).map(str::to_string))
}

/// The result a provider returns for a text value: `{"value": text}`.
pub fn value_result(text: &str) -> Json {
    Json::Obj(vec![("value".to_string(), Json::Str(text.to_string()))])
}

/// The call a provider receives on `service.<method>`: the send type it was matched on, and the text or the paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceCall {
    pub uti: String,
    pub text: Option<String>,
    pub paths: Vec<String>,
}

impl ServiceCall {
    /// Decodes the params of a service call. None when there is no type, or neither text nor paths.
    pub fn from_params(params: &Json) -> Option<ServiceCall> {
        let uti = params.get("type").and_then(Json::as_str)?.to_string();
        let text = params.get("text").and_then(Json::as_str).map(str::to_string);
        let paths: Vec<String> = match params.get("paths") {
            Some(Json::Arr(items)) => items.iter().filter_map(|p| p.as_str().map(str::to_string)).collect(),
            _ => Vec::new(),
        };
        (text.is_some() || !paths.is_empty()).then_some(ServiceCall { uti, text, paths })
    }
}

/// A JSON string literal for `s`.
fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(send: &[&str], method: &str) -> ServiceDecl {
        ServiceDecl {
            title: "Open".into(),
            send_types: send.iter().map(|s| s.to_string()).collect(),
            return_types: Vec::new(),
            method: method.into(),
        }
    }

    fn types(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_table_sets_its_keys_from_raw_values() {
        let mut d = ServiceDecl::default();
        d.set("title", "\"Open in Viewer\"");
        d.set("send_types", r#"["public.image", "public.pdf"]"#);
        d.set("return_types", "[]");
        d.set("method", "\"open_images\"");
        d.set("unknown", "\"ignored\"");
        assert_eq!(d.title, "Open in Viewer");
        assert_eq!(d.send_types, types(&["public.image", "public.pdf"]));
        assert!(d.return_types.is_empty());
        assert_eq!(d.method, "open_images");
        assert_eq!(d.rpc_method(), "service.open_images");
        assert!(d.valid());
    }

    #[test]
    fn a_service_needs_a_title_a_send_type_and_a_plain_method() {
        assert!(!decl(&[], "open").valid());
        assert!(!decl(&["public.image"], "Open Images").valid());
        let mut no_title = decl(&["public.image"], "open");
        no_title.title.clear();
        assert!(!no_title.valid());
        assert!(decl(&["public.image"], "open-images_2").valid());
    }

    #[test]
    fn a_service_is_offered_when_a_send_type_matches_the_selection() {
        let viewer = decl(&["public.image", "public.pdf"], "open_images");
        assert!(viewer.accepts(&types(&["public.file-url", "public.pdf"])));
        assert_eq!(viewer.send_type_for(&types(&["public.file-url", "public.pdf"])), Some("public.pdf"));
        assert!(!viewer.accepts(&types(&["public.utf8-plain-text"])));
        assert!(!viewer.accepts(&types(&[])));
    }

    #[test]
    fn text_is_plain_text_and_a_file_path_as_text_is_also_a_path() {
        assert_eq!(text_types("hello", |_| false), types(&[PLAIN_TEXT]));
        assert_eq!(text_types(" /home/me/a.txt \n", |p| p == "/home/me/a.txt"), types(&[PLAIN_TEXT, FILE_PATH_TEXT]));
        // a multi-line selection or a missing file is not a path
        assert_eq!(text_types("/home/me/a.txt\nmore", |_| true), types(&[PLAIN_TEXT]));
        assert_eq!(text_types("/nope", |_| false), types(&[PLAIN_TEXT]));
    }

    #[test]
    fn files_carry_their_url_and_the_picture_and_pdf_types_by_extension() {
        assert_eq!(file_types(&["/a/notes.txt".into()]), types(&[FILE_URL]));
        assert_eq!(file_types(&["/a/Photo.JPG".into(), "/a/x.txt".into()]), types(&[FILE_URL, IMAGE]));
        assert_eq!(file_types(&["/a/doc.pdf".into()]), types(&[FILE_URL, PDF]));
        assert!(is_viewable_path("/a/b.webp") && is_viewable_path("/a/b.pdf"));
        assert!(!is_viewable_path("/a/b.txt") && !is_viewable_path("/a.png/b"));
    }

    #[test]
    fn selection_types_follow_the_selection_kind() {
        assert_eq!(selection_types(&Selection::Text("x".into()), |_| false), types(&[PLAIN_TEXT]));
        assert_eq!(selection_types(&Selection::Files(vec!["/a.png".into()]), |_| false), types(&[FILE_URL, IMAGE]));
    }

    #[test]
    fn the_payload_is_the_text_or_the_paths_the_type_names() {
        assert_eq!(payload_for(PLAIN_TEXT, &Selection::Text("hi".into())), Payload::Text("hi".into()));
        assert_eq!(payload_for(FILE_PATH_TEXT, &Selection::Text(" /a/b \n".into())), Payload::Paths(vec!["/a/b".into()]));
        assert_eq!(payload_for(IMAGE, &Selection::Files(vec!["/a.png".into(), "/b.txt".into()])), Payload::Paths(vec!["/a.png".into(), "/b.txt".into()]));
    }

    #[test]
    fn the_request_line_carries_the_escaped_payload() {
        assert_eq!(
            request_line("new_window_with_text", PLAIN_TEXT, &Payload::Text("a \"q\"\nb".into())),
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"service.new_window_with_text\",\"params\":{\"type\":\"public.utf8-plain-text\",\"text\":\"a \\\"q\\\"\\nb\"}}\n"
        );
        assert_eq!(
            request_line("open_images", IMAGE, &Payload::Paths(vec!["/x y/a.png".into()])),
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"service.open_images\",\"params\":{\"type\":\"public.image\",\"paths\":[\"/x y/a.png\"]}}\n"
        );
    }

    #[test]
    fn a_provider_decodes_the_call_it_was_sent() {
        let line = request_line("open_selected_file", FILE_PATH_TEXT, &Payload::Paths(vec!["/a/b.txt".into()]));
        let params = json::parse(line.trim()).unwrap();
        let params = params.get("params").unwrap();
        assert_eq!(
            ServiceCall::from_params(params),
            Some(ServiceCall { uti: FILE_PATH_TEXT.into(), text: None, paths: vec!["/a/b.txt".into()] })
        );
        let text = json::parse(r#"{"type":"public.utf8-plain-text","text":"hi"}"#).unwrap();
        assert_eq!(ServiceCall::from_params(&text), Some(ServiceCall { uti: PLAIN_TEXT.into(), text: Some("hi".into()), paths: vec![] }));
        assert_eq!(ServiceCall::from_params(&json::parse(r#"{"type":"public.image"}"#).unwrap()), None);
        assert_eq!(ServiceCall::from_params(&json::parse("[]").unwrap()), None);
    }

    #[test]
    fn a_returned_value_is_read_from_the_result_and_errors_are_refused() {
        assert_eq!(parse_reply(r#"{"jsonrpc":"2.0","id":1,"result":{"value":"done"}}"#), Ok(Some("done".into())));
        assert_eq!(parse_reply(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#), Ok(None));
        assert_eq!(parse_reply(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"no method"}}"#), Err("no method".into()));
        assert!(parse_reply("nonsense").is_err());
        assert_eq!(value_result("x").get("value").and_then(Json::as_str), Some("x"));
    }
}
