//! Freeform's connections to the shell's system services:
//!
//! - `cursor`: the pointer shape of the tool under the pointer (see `Freeform::pointer_for`). The shape is pushed once,
//!   changed when it differs, and popped when the app is suspended, so the stack never keeps a shape of a background app.
//! - `pasteboard`: Copy puts the selected strokes on the general board as a PNG and as the app's own stroke text
//!   (`STROKES_UTI`).
//! - `screens`: the main display's backing scale, which sets the export factor, so a denser display gets a sharper PNG.
//!
//! Every call is made on a real change (a new pointer shape, a Copy, a changed display list), never per frame. A
//! missing service (an older shell answers "no service ...") is logged and the app carries on.
use crate::ink;
use bifrost_app::controls::cursor::Kind;
use bifrost_app::controls::drawing::Drawing;
use bifrost_app::pasteboard_types::{self as pb, Board, Item};
use bifrost_app::{cursor_service, screens, Ctx};
use bifrost_core::json::Json;

/// The app's own pasteboard type: the strokes as `Drawing::to_text`.
pub const STROKES_UTI: &str = "is.olibuijr.freeform.strokes";

/// Calls waiting for their reply. The oldest is dropped past this, so a shell that never answers cannot grow it.
const MAX_PENDING: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Req {
    Cursor,
    Copy,
    Screens,
}

#[derive(Default)]
pub struct Services {
    pending: Vec<(u32, Req)>,
    /// Our shape is on the cursor stack (pushed on first use, popped on suspend).
    pushed: bool,
    /// The shape the pointer shows for us.
    shown: Option<Kind>,
    /// The window scale the display list was last asked for (0 before the first ask).
    asked_scale: f32,
    /// The main display's backing scale, once the shell has said.
    backing: Option<f32>,
}

impl Services {
    fn track(&mut self, request: u32, req: Req) {
        if self.pending.len() >= MAX_PENDING {
            self.pending.remove(0);
        }
        self.pending.push((request, req));
    }

    /// The main display's backing scale, if the shell has told us.
    pub fn backing(&self) -> Option<f32> {
        self.backing
    }

    /// Asks for the display list when the window's scale is new.
    pub fn configure(&mut self, ctx: &Ctx) {
        if ctx.cfg.scale != self.asked_scale {
            self.asked_scale = ctx.cfg.scale;
            let id = screens::list(ctx);
            self.track(id, Req::Screens);
        }
    }

    /// Shows `kind` as the pointer shape, unless it is the shape already shown.
    pub fn show_pointer(&mut self, ctx: &Ctx, kind: Kind) {
        if self.shown == Some(kind) {
            return;
        }
        let id = if self.pushed { cursor_service::set(ctx, kind) } else { cursor_service::push(ctx, kind) };
        self.pushed = true;
        self.shown = Some(kind);
        self.track(id, Req::Cursor);
    }

    /// The app left the foreground: its shape comes off the cursor stack.
    pub fn suspend(&mut self, ctx: &Ctx) {
        if self.pushed {
            let id = cursor_service::pop(ctx);
            self.track(id, Req::Cursor);
        }
        self.pushed = false;
        self.shown = None;
    }

    /// Copy: the strokes go to the general board as a PNG and as the app's own stroke text. `scale` is the canvas
    /// pixels per point, so the PNG is as sharp as the board.
    pub fn copy(&mut self, ctx: &Ctx, strokes: &Drawing, scale: f32) {
        let png = ink::render_png(strokes, scale);
        let text = strokes.to_text();
        let item = Item::default().with(pb::PNG, &png).with(STROKES_UTI, text.as_bytes());
        let id = Board::general().write(ctx, &[item]);
        self.track(id, Req::Copy);
    }

    /// The reply to one of our calls. True when the call was ours, so the app takes it.
    pub fn reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        let Some(at) = self.pending.iter().position(|(id, _)| *id == request) else { return false };
        let (_, req) = self.pending.remove(at);
        match req {
            Req::Screens => match screens::from_reply(result) {
                Ok(list) => self.backing = list.main().map(|m| m.backing_scale),
                Err(e) => ctx.log(&format!("freeform: screens: {e}")),
            },
            Req::Copy => {
                if let Err(e) = result {
                    ctx.log(&format!("freeform: pasteboard: {e}"));
                }
            }
            Req::Cursor => {
                if let Err(e) = result {
                    ctx.log(&format!("freeform: cursor: {e}"));
                }
            }
        }
        true
    }

    /// A pushed event. True when it was the display list's change.
    pub fn event(&mut self, ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        match screens::changed(service, event, body) {
            Some(Ok(list)) => {
                self.backing = list.main().map(|m| m.backing_scale);
                true
            }
            Some(Err(e)) => {
                ctx.log(&format!("freeform: screens: {e}"));
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::controls::drawing::{Ink, InkKind, Stroke, StrokePoint};
    use bifrost_app::proto::{Configure, ToShell};

    fn test_ctx(scale: f32) -> Ctx {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
        let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
        let ui = bifrost_app::kit::ui::Ui::new(bifrost_app::kit::Font::parse(data).unwrap(), scale);
        let cfg = Configure { w: 200, h: 200, scale, safe_top: 0.0, safe_bottom: 0.0, dark: false, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(ui, cfg)
    }

    /// (request, service, method, body) of each service call sent to the shell, oldest first.
    fn calls(ctx: &Ctx) -> Vec<(u32, String, String, String)> {
        ctx.take_sent()
            .into_iter()
            .filter_map(|m| match m {
                ToShell::ServiceCall { request, service, method, body } => Some((request, service, method, body)),
                _ => None,
            })
            .collect()
    }

    fn screens_json(backing: f32) -> Json {
        bifrost_core::json::parse(&format!(
            r#"[{{"name":"Display","frame":{{"x":0,"y":0,"w":1440,"h":900}},"visible_frame":{{"x":0,"y":24,"w":1440,"h":876}},"scale":{backing},"main":true}}]"#
        ))
        .unwrap()
    }

    fn one_stroke() -> Drawing {
        Drawing::from_strokes(vec![Stroke {
            ink: Ink { kind: InkKind::Pen, color: 0xFF00_0000, width: 2.0 },
            points: vec![StrokePoint::at(10.0, 10.0), StrokePoint::at(30.0, 10.0)],
        }])
    }

    #[test]
    fn the_pointer_is_pushed_once_then_changed_only_when_the_shape_changes() {
        let ctx = test_ctx(1.0);
        let mut s = Services::default();
        s.show_pointer(&ctx, Kind::Crosshair);
        s.show_pointer(&ctx, Kind::Crosshair);
        s.show_pointer(&ctx, Kind::DisappearingItem);
        let sent = calls(&ctx);
        assert_eq!(sent.len(), 2, "a repeat of the shape sends nothing");
        assert_eq!((sent[0].1.as_str(), sent[0].2.as_str(), sent[0].3.as_str()), ("cursor", "push", "\"crosshair\""));
        assert_eq!((sent[1].2.as_str(), sent[1].3.as_str()), ("set", "\"disappearing_item\""));
    }

    #[test]
    fn suspend_pops_our_shape_and_the_next_show_pushes_again() {
        let ctx = test_ctx(1.0);
        let mut s = Services::default();
        s.suspend(&ctx);
        assert!(calls(&ctx).is_empty(), "nothing to pop before the first show");
        s.show_pointer(&ctx, Kind::Crosshair);
        s.suspend(&ctx);
        s.show_pointer(&ctx, Kind::Crosshair);
        let methods: Vec<String> = calls(&ctx).into_iter().map(|c| c.2).collect();
        assert_eq!(methods, ["push", "pop", "push"]);
    }

    #[test]
    fn the_display_list_is_asked_once_per_scale() {
        let ctx = test_ctx(2.0);
        let mut s = Services::default();
        s.configure(&ctx);
        s.configure(&ctx);
        let sent = calls(&ctx);
        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0].1.as_str(), sent[0].2.as_str()), ("screens", "list"));
    }

    #[test]
    fn a_list_reply_sets_the_backing_scale_and_an_unknown_reply_is_left_alone() {
        let ctx = test_ctx(2.0);
        let mut s = Services::default();
        s.configure(&ctx);
        let (id, ..) = calls(&ctx).remove(0);
        assert!(!s.reply(&ctx, id + 1000, &Ok(Json::Null)), "a reply that is not ours is kept for others");
        assert!(s.reply(&ctx, id, &Ok(screens_json(2.0))));
        assert_eq!(s.backing(), Some(2.0));
    }

    #[test]
    fn a_changed_display_list_event_updates_the_backing_scale() {
        let ctx = test_ctx(1.0);
        let mut s = Services::default();
        assert!(!s.event(&ctx, "dock", "changed", &screens_json(2.0)), "other services are not ours");
        assert_eq!(s.backing(), None);
        assert!(s.event(&ctx, screens::SERVICE, screens::EVENT_CHANGED, &screens_json(1.5)));
        assert_eq!(s.backing(), Some(1.5));
    }

    #[test]
    fn copy_writes_a_png_and_the_stroke_text_to_the_general_board() {
        let ctx = test_ctx(1.0);
        let mut s = Services::default();
        s.copy(&ctx, &one_stroke(), 1.0);
        let sent = calls(&ctx);
        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0].1.as_str(), sent[0].2.as_str()), ("pasteboard", "write"));
        let body = &sent[0].3;
        assert!(body.contains("\"general\""), "the shared clipboard board");
        assert!(body.contains("public.png"), "a PNG representation");
        assert!(body.contains(STROKES_UTI), "the app's own representation");
    }

    #[test]
    fn a_failed_call_is_taken_and_does_not_panic() {
        let ctx = test_ctx(1.0);
        let mut s = Services::default();
        s.show_pointer(&ctx, Kind::Crosshair);
        let (id, ..) = calls(&ctx).remove(0);
        assert!(s.reply(&ctx, id, &Err("no service cursor".into())));
        assert!(s.pending.is_empty());
    }
}
