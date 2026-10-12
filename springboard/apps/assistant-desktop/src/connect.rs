//! The Assistant's connections to the desktop's system services. Two of them:
//!
//! - the network path (`network` service): a send made while the machine has no route is held back with a clear
//!   message instead of going out to fail;
//! - the pasteboard (`pasteboard` service): "Copy Answer" puts the newest answer on the general board as HTML and as
//!   plain text, so it pastes formatted into rich editors and as text everywhere else.
//!
//! Both are asked for on a user action or a state change, never per frame. A shell without a service answers
//! "no service ..."; the app then carries on as if the service were not there.

use crate::model::{self, Block, Span};
use bifrost_app::network_path::{self, Path, Status};
use bifrost_app::pasteboard_types::{self as pasteboard, Board, Item};
use bifrost_app::Ctx;
use bifrost_core::json::Json;

/// The error row shown when a send is held back because the network path says traffic cannot leave the machine.
pub const OFFLINE: &str = "You are offline. Connect to a network to send this message.";
/// The hint in an empty conversation while offline.
pub const OFFLINE_HINT: &str = "You are offline. Connect to a network to chat.";

/// The network path as the shell last reported it. Until the shell has answered, the link is not offline, so a shell
/// without the `network` service never blocks a send.
#[derive(Default)]
pub struct Link {
    path: Option<Path>,
    request: Option<u32>,
}

impl Link {
    /// Asks the shell for the current path once. Later changes arrive as `pathUpdate` events.
    pub fn new(ctx: &Ctx) -> Self {
        Link { path: None, request: Some(network_path::request_path(ctx)) }
    }

    /// True when the shell reported a path that cannot carry traffic (no route, or one that needs a connection first).
    pub fn offline(&self) -> bool {
        self.path.as_ref().is_some_and(|p| p.status != Status::Satisfied)
    }

    /// The reply to the path request. True when the reply was ours; an error reply (no `network` service) is taken too,
    /// and leaves the link unknown.
    pub fn reply(&mut self, request: u32, result: &Result<Json, String>) -> bool {
        if self.request != Some(request) {
            return false;
        }
        self.request = None;
        if let Ok(path) = network_path::decode_reply(result.clone()) {
            self.path = Some(path);
        }
        true
    }

    /// A `pathUpdate` event. True when it carried a path, which the app then redraws for.
    pub fn event(&mut self, service: &str, event: &str, body: &Json) -> bool {
        match network_path::path_event(service, event, body) {
            Some(Ok(path)) => {
                self.path = Some(path);
                true
            }
            _ => false,
        }
    }
}

/// Copies `markdown` to the general board as HTML and as plain text. The shell's clipboard takes the plain text;
/// Wayland clients are offered the HTML. Returns the request id; the reply is a change count or an error.
pub fn copy_answer(ctx: &Ctx, markdown: &str) -> u32 {
    let html = answer_html(markdown);
    let plain = answer_plain(markdown);
    let item = Item::default().with(pasteboard::HTML, html.as_bytes()).with(pasteboard::STRING, plain.as_bytes());
    Board::general().write(ctx, &[item])
}

/// The answer as plain text: the paragraphs and code blocks of the Markdown subset, bullets kept as "• ".
pub fn answer_plain(markdown: &str) -> String {
    model::parse(markdown)
        .iter()
        .map(|block| match block {
            Block::Para(lines) => lines.iter().map(|line| line_text(line)).collect::<Vec<_>>().join("\n"),
            Block::Code(lines) => lines.join("\n"),
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The answer as an HTML fragment: paragraphs with `<strong>` and `<code>` spans, code blocks in `<pre>`.
pub fn answer_html(markdown: &str) -> String {
    model::parse(markdown)
        .iter()
        .map(|block| match block {
            Block::Para(lines) => {
                let html: Vec<String> = lines.iter().map(|line| line.iter().map(span_html).collect()).collect();
                format!("<p>{}</p>", html.join("<br>"))
            }
            Block::Code(lines) => format!("<pre><code>{}</code></pre>", escape(&lines.join("\n"))),
        })
        .collect()
}

fn line_text(line: &[Span]) -> String {
    line.iter().map(|s| s.text.as_str()).collect()
}

fn span_html(span: &Span) -> String {
    let text = escape(&span.text);
    if span.code {
        format!("<code>{text}</code>")
    } else if span.bold {
        format!("<strong>{text}</strong>")
    } else {
        text
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::kit::{Font, Ui};
    use bifrost_app::proto::{Configure, ToShell};

    fn test_ctx() -> Ctx {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
    }

    fn path_body(status: &str) -> Json {
        bifrost_core::json::parse(&format!(r#"{{"status":"{status}","interfaces":[],"expensive":false,"ipv4":true,"ipv6":false}}"#)).unwrap()
    }

    #[test]
    fn a_new_link_asks_the_network_service_for_the_path_once() {
        let ctx = test_ctx();
        let link = Link::new(&ctx);
        assert!(link.request.is_some());
        let sent = ctx.take_sent();
        assert!(matches!(&sent[..], [ToShell::ServiceCall { service, method, .. }] if service == "network" && method == "path"));
    }

    #[test]
    fn the_link_is_offline_only_after_the_shell_reports_no_usable_path() {
        let ctx = test_ctx();
        let mut link = Link::new(&ctx);
        assert!(!link.offline(), "an unknown path does not block a send");
        let id = link.request.unwrap();
        assert!(link.reply(id, &Ok(path_body("unsatisfied"))));
        assert!(link.offline());
        assert!(link.event("network", "pathUpdate", &path_body("satisfied")));
        assert!(!link.offline());
        assert!(link.event("network", "pathUpdate", &path_body("requiresConnection")));
        assert!(link.offline());
    }

    #[test]
    fn a_shell_without_the_service_keeps_sending_allowed_and_the_reply_is_taken_once() {
        let ctx = test_ctx();
        let mut link = Link::new(&ctx);
        let id = link.request.unwrap();
        assert!(link.reply(id, &Err("no service network".into())));
        assert!(!link.offline());
        assert!(!link.reply(id, &Ok(path_body("unsatisfied"))), "a reply is taken once");
        assert!(!link.offline());
    }

    #[test]
    fn other_replies_and_events_are_not_ours() {
        let ctx = test_ctx();
        let mut link = Link::new(&ctx);
        assert!(!link.reply(u32::MAX, &Ok(path_body("unsatisfied"))));
        assert!(!link.event("dock", "pathUpdate", &path_body("unsatisfied")));
        assert!(!link.event("network", "wifi", &path_body("unsatisfied")));
        assert!(!link.offline());
    }

    #[test]
    fn copying_an_answer_writes_html_and_plain_text_to_the_general_board() {
        let ctx = test_ctx();
        let id = copy_answer(&ctx, "Hi **there**\n\n```\na < b\n```");
        let sent = ctx.take_sent();
        let ToShell::ServiceCall { request, service, method, body } = &sent[0] else { panic!("expected a service call") };
        assert_eq!((*request, service.as_str(), method.as_str()), (id, "pasteboard", "write"));
        let v = bifrost_core::json::parse(body).unwrap();
        assert_eq!(v.str_field("board").as_deref(), Some("general"));
        let item = &v.get("items").and_then(Json::as_array).unwrap()[0];
        let reps = item.get("reps").and_then(Json::as_array).unwrap();
        assert_eq!(reps[0].str_field("uti").as_deref(), Some(pasteboard::HTML));
        assert_eq!(reps[0].str_field("text").as_deref(), Some("<p>Hi <strong>there</strong></p><pre><code>a &lt; b</code></pre>"));
        assert_eq!(reps[1].str_field("uti").as_deref(), Some(pasteboard::STRING));
        assert_eq!(reps[1].str_field("text").as_deref(), Some("Hi there\n\na < b"));
    }

    #[test]
    fn the_plain_and_html_forms_keep_the_structure_of_the_answer() {
        let md = "Hello **bold** and `code`\n\n```\nlet x = 1 < 2\n```\n- item";
        assert_eq!(answer_plain(md), "Hello bold and code\n\nlet x = 1 < 2\n\n• item");
        assert_eq!(
            answer_html(md),
            "<p>Hello <strong>bold</strong> and <code>code</code></p><pre><code>let x = 1 &lt; 2</code></pre><p>• item</p>"
        );
    }
}
