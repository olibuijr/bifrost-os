//! Typed client for the shell's `cursor` service: push and pop pointer shapes (`bifrost_app::controls::cursor::Kind`),
//! set the top shape, hide the pointer, and hide it until it moves. Each call returns the request id; a reply comes
//! back as `App::service_reply` (or [`service::take_reply`](crate::service::take_reply)), so an app that does not
//! need the answer can drop the id.
//!
//! ```ignore
//! use bifrost_app::controls::cursor::Kind;
//! cursor_service::push(ctx, Kind::IBeam);      // over a text field
//! cursor_service::pop(ctx);                    // leaving it
//! cursor_service::hide_until_moved(ctx);       // while typing, the pointer stays out of the way
//! ```
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;
use bifrost_ui::cursor::Kind;

fn shape(ctx: &Ctx, method: &str, kind: Kind) -> u32 {
    service::call(ctx, "cursor", method, &Json::Str(kind.name().into()))
}

fn no_params(ctx: &Ctx, method: &str) -> u32 {
    service::call(ctx, "cursor", method, &Json::Null)
}

/// Puts `kind` on the cursor stack and shows it.
pub fn push(ctx: &Ctx, kind: Kind) -> u32 {
    shape(ctx, "push", kind)
}

/// Removes the top shape; the one below shows again (or the shell's own pointer when the stack is empty).
pub fn pop(ctx: &Ctx) -> u32 {
    no_params(ctx, "pop")
}

/// Replaces the top shape without growing the stack.
pub fn set(ctx: &Ctx, kind: Kind) -> u32 {
    shape(ctx, "set", kind)
}

/// Asks for the shape on top of the stack; the reply is its name.
pub fn current(ctx: &Ctx) -> u32 {
    no_params(ctx, "current")
}

/// Asks for the stack depth; the reply is a number.
pub fn depth(ctx: &Ctx) -> u32 {
    no_params(ctx, "depth")
}

/// Hides the pointer. Each `hide` needs its own [`unhide`].
pub fn hide(ctx: &Ctx) -> u32 {
    no_params(ctx, "hide")
}

/// Undoes one [`hide`].
pub fn unhide(ctx: &Ctx) -> u32 {
    no_params(ctx, "unhide")
}

/// Hides the pointer until the user moves it.
pub fn hide_until_moved(ctx: &Ctx) -> u32 {
    service::call(ctx, "cursor", "set_hidden_until_mouse_moves", &Json::Bool(true))
}

/// Clears the hide-until-moved flag without a move.
pub fn show_now(ctx: &Ctx) -> u32 {
    service::call(ctx, "cursor", "set_hidden_until_mouse_moves", &Json::Bool(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::ToShell;

    fn test_ctx() -> Ctx {
        use crate::kit::{Font, Ui};
        use crate::proto::Configure;
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
    }

    /// Every service call sent so far, as ("service.method", body); take_sent drains, so read them all at once.
    fn sent_calls(ctx: &Ctx) -> Vec<(String, String)> {
        ctx.take_sent()
            .into_iter()
            .map(|m| match m {
                ToShell::ServiceCall { service, method, body, .. } => (format!("{service}.{method}"), body),
                other => panic!("not a service call: {other:?}"),
            })
            .collect()
    }

    fn sent_call(ctx: &Ctx, i: usize) -> (String, String) {
        sent_calls(ctx).swap_remove(i)
    }

    #[test]
    fn push_sends_the_shape_name_to_the_cursor_service() {
        let ctx = test_ctx();
        push(&ctx, Kind::IBeam);
        assert_eq!(sent_call(&ctx, 0), ("cursor.push".into(), "\"i_beam\"".into()));
    }

    #[test]
    fn hide_methods_send_the_flags_the_service_expects() {
        let ctx = test_ctx();
        hide_until_moved(&ctx);
        show_now(&ctx);
        pop(&ctx);
        let calls = sent_calls(&ctx);
        assert_eq!(calls[0], ("cursor.set_hidden_until_mouse_moves".into(), "true".into()));
        assert_eq!(calls[1], ("cursor.set_hidden_until_mouse_moves".into(), "false".into()));
        assert_eq!(calls[2], ("cursor.pop".into(), "null".into()));
    }
}
