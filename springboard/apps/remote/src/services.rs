//! Shell services the Remote window uses: the network path (`network`), the screen list (`screens`) and the
//! shared clipboard (`pasteboard`). The SDK clients build the calls and decode the replies; this file only chooses
//! the calls and words the results. Every call is made on a real event: the first configure (start), a shell
//! push (`pathUpdate`, `changed`) or a tap on an address. Nothing is sent per frame.

use bifrost_app::network_path::{self, Path};
use bifrost_app::pasteboard_types::{self as pasteboard, Board, Item};
use bifrost_app::screens::{self, Screens};
use bifrost_app::Ctx;
use bifrost_core::json::Json;

/// The intro line when no screen is known (before the first reply, or with no screen at all).
pub const INTRO: &str = "Share this desktop in your browser.";

/// Asks the shell for the current network path. The reply is read with [`path_reply`].
pub fn request_path(ctx: &Ctx) -> u32 {
    network_path::request_path(ctx)
}

/// Asks the shell for the screen list. The reply is read with `screens::from_reply`.
pub fn request_screens(ctx: &Ctx) -> u32 {
    screens::list(ctx)
}

/// Writes `text` to the shared clipboard as one plain-text item on the general board. One change.
pub fn copy_text(ctx: &Ctx, text: &str) -> u32 {
    Board::general().write(ctx, &[Item::default().with(pasteboard::STRING, text.as_bytes())])
}

/// The path from a reply to `network.path`. An error (an older shell answers "no service network") is passed on,
/// so the caller can fall back to reading the link itself.
pub fn path_reply(reply: &Result<Json, String>) -> Result<Path, String> {
    network_path::decode_reply(reply.clone())
}

/// The intro line for a screen list: the size of the main screen, which is the desktop this window shares.
pub fn intro(screens: &Screens) -> String {
    match screens.main() {
        Some(s) => format!("Share this {} x {} desktop in your browser.", s.frame.w.round() as i64, s.frame.h.round() as i64),
        None => INTRO.to_string(),
    }
}

/// The notice after a copy, from the reply to [`copy_text`].
pub fn copy_notice(reply: &Result<Json, String>) -> String {
    match reply {
        Ok(_) => "Copied to the clipboard.".to_string(),
        Err(e) => format!("Could not copy: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::kit::{Font, Ui};
    use bifrost_app::proto::{ToShell, Configure};

    fn test_ctx() -> Ctx {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
        let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
    }

    fn parse(text: &str) -> Json {
        bifrost_core::json::parse(text).unwrap()
    }

    #[test]
    fn start_asks_the_network_and_screens_services_once_each() {
        let ctx = test_ctx();
        let p = request_path(&ctx);
        let s = request_screens(&ctx);
        let sent = ctx.take_sent();
        assert_eq!(sent[0], ToShell::ServiceCall { request: p, service: "network".into(), method: "path".into(), body: "null".into() });
        assert_eq!(sent[1], ToShell::ServiceCall { request: s, service: "screens".into(), method: "list".into(), body: "null".into() });
    }

    #[test]
    fn copy_sends_one_plain_text_write_to_the_general_board() {
        let ctx = test_ctx();
        let id = copy_text(&ctx, "http://10.0.0.5:12000");
        let sent = ctx.take_sent();
        assert_eq!(sent.len(), 1);
        let ToShell::ServiceCall { request, service, method, body } = &sent[0] else { panic!("not a service call") };
        assert_eq!((*request, service.as_str(), method.as_str()), (id, "pasteboard", "write"));
        let v = parse(body);
        assert_eq!(v.str_field("board").as_deref(), Some("general"));
        let rep = v.get("items").and_then(|i| i.as_array()).and_then(|i| i.first()).and_then(|i| i.get("reps")).and_then(|r| r.as_array()).and_then(|r| r.first()).unwrap();
        assert_eq!(rep.str_field("uti").as_deref(), Some("public.utf8-plain-text"));
        assert_eq!(rep.str_field("text").as_deref(), Some("http://10.0.0.5:12000"));
    }

    #[test]
    fn copy_reports_success_or_the_shell_error() {
        assert_eq!(copy_notice(&Ok(Json::Num(1.0))), "Copied to the clipboard.");
        assert_eq!(copy_notice(&Err("no service pasteboard".into())), "Could not copy: no service pasteboard");
    }

    #[test]
    fn a_path_reply_decodes_or_passes_the_error_on() {
        let ok = parse(r#"{"status":"satisfied","interfaces":[{"name":"eth0","kind":"wired"}],"expensive":false,"ipv4":true,"ipv6":false}"#);
        assert_eq!(path_reply(&Ok(ok)).unwrap().status, network_path::Status::Satisfied);
        assert_eq!(path_reply(&Err("no service network".into())), Err("no service network".into()));
    }

    #[test]
    fn the_intro_names_the_main_screen_size() {
        let list = screens::from_json(&parse(
            r#"[{"name":"Display","frame":{"x":0,"y":0,"w":1440,"h":900},"visible_frame":{"x":0,"y":24,"w":1440,"h":876},"scale":2,"main":true}]"#,
        ))
        .unwrap();
        assert_eq!(intro(&list), "Share this 1440 x 900 desktop in your browser.");
    }

    #[test]
    fn with_no_screen_the_intro_is_the_plain_line() {
        let none = screens::from_json(&parse("[]")).unwrap();
        assert_eq!(intro(&none), INTRO);
    }
}
