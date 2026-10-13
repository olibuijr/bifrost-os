//! Running a shortcut inside the app. `springboard::shortcuts::flow` walks the steps, passes text from one to the next and
//! runs the If and Repeat blocks; this host performs the steps that only need the app: copying text to the clipboard,
//! showing a notification and showing a result. Steps that need the shell's own services (Wi-Fi, Low Power, Do Not
//! Disturb, brightness, apps, screenshots, timers, folders, scripts, files, the network) or an answer from the user are
//! refused with a reason, which stops the shortcut and is shown in the result panel.

use bifrost_app::Ctx;
use springboard::shortcuts::flow::{Done, Host};
use springboard::shortcuts::Action;

/// The title notifications from a shortcut carry.
const NOTIFICATION_TITLE: &str = "Shortcuts";

/// The steps the app can perform for itself, over the app's context.
pub struct AppHost<'a> {
    ctx: &'a Ctx,
}

impl<'a> AppHost<'a> {
    pub fn new(ctx: &'a Ctx) -> Self {
        AppHost { ctx }
    }
}

impl Host for AppHost<'_> {
    fn perform(&mut self, action: &Action, _input: &str) -> Result<Done, String> {
        match action {
            Action::CopyText(text) => {
                self.ctx.set_pasteboard_string(text);
                Ok(Done::said("Copied to the clipboard"))
            }
            Action::Notify(message) => {
                self.ctx.notify(NOTIFICATION_TITLE, message);
                Ok(Done::said("Notification shown"))
            }
            Action::ShowResult(text) => Ok(Done::said(text)),
            other => Err(format!("{} needs the shell's services, which this app cannot reach yet", other.title())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::kit::{Font, Ui};
    use bifrost_app::proto::{Configure, ToShell};
    use springboard::shortcuts::flow;

    fn ctx() -> Ctx {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
        let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
    }

    #[test]
    fn a_notification_step_reaches_the_shell() {
        let ctx = ctx();
        let steps = flow::run(&[Action::Notify("Dinner is ready".into())], &mut AppHost::new(&ctx));
        assert_eq!(steps[0].outcome, Ok("Notification shown".into()));
        let sent = ctx.take_sent();
        assert!(sent.iter().any(|m| matches!(m, ToShell::Notify { title, body } if title == "Shortcuts" && body == "Dinner is ready")));
    }

    #[test]
    fn copying_and_showing_a_result_are_done_in_the_app() {
        let ctx = ctx();
        let list = vec![Action::CopyText("Thanks".into()), Action::Text("Hello".into()), Action::ShowResult("{Output}!".into())];
        let steps = flow::run(&list, &mut AppHost::new(&ctx));
        assert_eq!(steps[0].outcome, Ok("Copied to the clipboard".into()));
        assert_eq!(steps[2].outcome, Ok("Hello!".into()));
    }

    #[test]
    fn a_step_that_needs_the_shell_stops_the_shortcut_with_a_reason() {
        let ctx = ctx();
        let list = vec![Action::Text("a".into()), Action::Wifi(true), Action::ShowResult("never".into())];
        let steps = flow::run(&list, &mut AppHost::new(&ctx));
        assert!(steps[1].outcome.as_ref().unwrap_err().contains("Set Wi-Fi needs the shell's services"));
        assert!(steps[2].outcome.is_err(), "the result after it is not shown");
        assert!(ctx.take_sent().is_empty(), "nothing reached the shell");
    }
}
