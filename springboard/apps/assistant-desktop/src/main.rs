//! Assistant as an installed BifrOSt desktop app: hosts `view::Assistant` through the SDK.

use assistant::view::{self, Assistant};
use bifrost_app::control::{RpcError, METHOD_NOT_FOUND};
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::{App, AppMenu, AppMenuItem, Command, Commands, Ctx, Input};
use bifrost_core::json::Json;

struct AssistantApp {
    chat: Option<Assistant>,
}

impl App for AssistantApp {
    fn id(&self) -> &str {
        "is.olibuijr.assistant.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let (w, h) = (ctx.cfg.w, ctx.cfg.h);
        match &mut self.chat {
            Some(c) => c.set_size(w, h),
            None => {
                self.chat = Some(Assistant::new(ctx));
                ctx.wants_keyboard(true);
            }
        }
    }

    fn rpc_methods(&self) -> Vec<&'static str> {
        assistant::rpc_methods()
    }

    fn rpc(&mut self, _ctx: Option<&Ctx>, data_dir: &std::path::Path, method: &str, _params: &Json) -> Result<Json, RpcError> {
        assistant::rpc_call(data_dir, method).ok_or_else(|| RpcError::new(METHOD_NOT_FOUND, format!("no method {method}")))
    }

    fn controls(&self, ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.chat.as_ref().map_or_else(Vec::new, |c| c.controls(&ctx.ui))
    }

    fn commands(&self, _ctx: &Ctx) -> Commands {
        Commands(Command::Paste.bit())
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let Some(c) = &self.chat else { return Vec::new() };
        vec![AppMenu {
            title: "Chat".into(),
            items: vec![
                AppMenuItem::new(view::MENU_NEW, "New Chat").key("cmd+n"),
                AppMenuItem::new(view::MENU_DELETE, "Delete Chat").key("cmd+backspace").enabled(c.can_delete()),
                AppMenuItem::separator(),
                AppMenuItem::new(view::MENU_COPY, "Copy Answer").key("cmd+shift+c").enabled(c.can_copy()),
                AppMenuItem::separator(),
                AppMenuItem::new(view::MENU_SETTINGS, "Assistant Settings...").key("cmd+,"),
            ],
        }]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        self.chat.as_mut().is_some_and(|c| c.menu(ctx, id))
    }

    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        self.chat.as_mut().is_some_and(|c| c.service_reply(ctx, request, result))
    }

    fn service_event(&mut self, _ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        self.chat.as_mut().is_some_and(|c| c.service_event(service, event, body))
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let Some(c) = &mut self.chat else { return false };
        match ev {
            Input::Touch { kind, x, y, t, .. } => c.touch(ctx, kind, x, y, t),
            Input::Key(k) => c.key(ctx, k),
            Input::Activate(id) => c.activate(id),
            Input::Command(Command::Paste) => c.paste(),
            _ => false,
        }
    }

    fn keychain(&mut self, ctx: &Ctx, reply: bifrost_app::keychain::Reply) -> bool {
        self.chat.as_mut().is_some_and(|c| c.keychain(ctx, reply))
    }

    fn suspend(&mut self, _ctx: &Ctx) {
        if let Some(c) = &self.chat {
            c.flush();
        }
    }

    fn tick(&mut self, _ctx: &Ctx, t: f64) -> bool {
        self.chat.as_mut().is_some_and(|c| c.tick(t))
    }


    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        if let Some(c) = &mut self.chat {
            c.draw(s, &ctx.ui);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.chat.as_ref().and_then(|c| c.next_wake(t))
    }

    fn wake_fd(&self) -> Option<i32> {
        self.chat.as_ref().and_then(|c| c.wake_fd())
    }
}

fn main() {
    bifrost_app::run(AssistantApp { chat: None });
}
