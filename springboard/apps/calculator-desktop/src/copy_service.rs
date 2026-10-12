//! Copy puts the result on the shell's general pasteboard through the `pasteboard` system service, as one item with
//! two representations: the number as plain digits (what a text field or another app's Paste reads, as before) and
//! the same number in the calculator's language as HTML. Other apps see the copy as a `changed` event on the general
//! board. A shell without the service, or a failed write, falls back to the plain clipboard.

use bifrost_app::pasteboard_types::{Board, Item, HTML, STRING};
use bifrost_app::Ctx;

/// One Copy: the number as digits and the number as shown in the calculator's language.
#[derive(Clone, Debug, PartialEq)]
pub struct CopyRequest {
    pub number: String,
    pub shown: String,
}

/// The pasteboard item for a copy: plain text first (the shell publishes the text-like type first), then HTML.
pub fn item(req: &CopyRequest) -> Item {
    Item::default().with(STRING, req.number.as_bytes()).with(HTML, span(&req.shown).as_bytes())
}

/// Sends the copy to the general board (one change). The reply arrives in `App::service_reply`.
pub fn send(ctx: &Ctx, req: &CopyRequest) -> u32 {
    Board::general().write(ctx, &[item(req)])
}

/// The HTML form of a shown number: a span, with the characters HTML reserves escaped.
fn span(shown: &str) -> String {
    let mut out = String::from("<span>");
    for ch in shown.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out.push_str("</span>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calc::Calculator;
    use bifrost_app::proto::{Configure, ToShell};

    fn test_ctx() -> Ctx {
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(crate::test_ui(), cfg)
    }

    fn req(number: &str, shown: &str) -> CopyRequest {
        CopyRequest { number: number.into(), shown: shown.into() }
    }

    #[test]
    fn the_item_carries_the_number_as_text_and_the_shown_number_as_html() {
        let it = item(&req("1234.5", "1234,5"));
        assert_eq!(it.data(STRING), Some(&b"1234.5"[..]));
        assert_eq!(it.data(HTML), Some(&b"<span>1234,5</span>"[..]));
        assert_eq!(it.representations.len(), 2);
    }

    #[test]
    fn html_reserved_characters_are_escaped() {
        assert_eq!(span("<1&2>\"x\""), "<span>&lt;1&amp;2&gt;&quot;x&quot;</span>");
    }

    #[test]
    fn a_copy_is_one_write_to_the_general_board() {
        let ctx = test_ctx();
        let id = send(&ctx, &req("-5", "-5"));
        let sent = ctx.take_sent();
        assert_eq!(sent.len(), 1);
        match &sent[0] {
            ToShell::ServiceCall { request, service, method, body } => {
                assert_eq!(*request, id);
                assert_eq!(service, "pasteboard");
                assert_eq!(method, "write");
                assert!(body.contains(r#""board":"general""#), "{body}");
                assert!(body.contains(r#""text":"-5""#), "{body}");
                assert!(body.contains("public.html"), "{body}");
            }
            other => panic!("unexpected message {other:?}"),
        }
    }

    #[test]
    fn copy_hands_the_number_to_the_app_once() {
        let ui = crate::test_ui();
        let mut calc = Calculator::new(&ui, 1200, 1920);
        assert!(calc.take_copy().is_none(), "nothing copied yet");
        assert!(calc.paste_text(&ui, "1234.5"));
        calc.copy(&ui);
        let taken = calc.take_copy().expect("a copy");
        assert_eq!(taken.number, "1234.5");
        assert!(!taken.shown.is_empty());
        assert!(calc.take_copy().is_none(), "taken once");
    }
}
