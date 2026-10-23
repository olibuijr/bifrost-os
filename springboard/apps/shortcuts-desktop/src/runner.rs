//! Running a shortcut inside the app. `springboard::shortcuts::flow` walks the steps, passes text from one to the next and
//! runs the If and Repeat blocks; this host performs the steps that only need the app: copying text to the clipboard,
//! showing a notification and showing a result. Start Timer goes to the shell's `system_intents` service
//! (`timer_start`), which starts the timer in the shell's Clock service, so the Clock app lists it and the shell rings
//! it. Steps that need the shell's own services that no service offers yet (Wi-Fi, Low Power, Do Not Disturb,
//! brightness, apps, screenshots, folders, files, the network) or an answer from the user are refused with a reason,
//! which stops the shortcut and is shown in the result panel.

use bifrost_app::{service, Ctx};
use bifrost_core::json::Json;
use bifrost_core::script::dictionary::Dictionary;
use bifrost_core::script::interp::{self as script, ScriptError};
use springboard::shortcuts::flow::{Done, Host};
use springboard::shortcuts::Action;

/// The title notifications from a shortcut carry.
const NOTIFICATION_TITLE: &str = "Shortcuts";

/// The host a "Run Script" step runs its script on. Notifications are shown; anything that would reach another app, a
/// dialog or the shell is refused with a reason, so a script cannot do more here than the other shortcut steps can.
pub struct ScriptHost<'a> {
    ctx: &'a Ctx,
}

impl script::Host for ScriptHost<'_> {
    fn send_event(&mut self, app: &str, _class: &str, _id: &str, _params: &Json, _timeout_secs: u64) -> Result<Json, ScriptError> {
        Err(ScriptError::general(format!("tell app \"{app}\" needs the shell's scripting service, which this app cannot reach yet")))
    }
    fn dictionary(&mut self, app: &str) -> Result<Dictionary, ScriptError> {
        Err(ScriptError::general(format!("{app} has no scripting dictionary available here")))
    }
    fn display_dialog(&mut self, _text: &str, _title: Option<&str>) -> Result<String, ScriptError> {
        Err(ScriptError::general("display dialog needs an answer from the person, which a shortcut cannot ask for yet"))
    }
    fn display_notification(&mut self, text: &str, title: Option<&str>) -> Result<(), ScriptError> {
        self.ctx.notify(title.unwrap_or(NOTIFICATION_TITLE), text);
        Ok(())
    }
    fn shell(&mut self, _command: &str) -> Result<String, ScriptError> {
        Err(ScriptError::general("do shell script needs the person's confirmation, which is not built yet"))
    }
    /// Delays are skipped: a shortcut step must not hold the window's thread.
    fn sleep(&mut self, _secs: f64) {}
}

/// The source a "Run Script" step runs: its one-line text, with `\n` standing for a line break.
pub fn script_source(text: &str) -> String {
    text.replace("\\n", "\n")
}

/// The steps the app can perform for itself, over the app's context.
pub struct AppHost<'a> {
    ctx: &'a Ctx,
    /// the `system_intents` calls this run made (Start Timer); a failed reply is shown when it arrives
    pub service_requests: Vec<u32>,
}

impl<'a> AppHost<'a> {
    pub fn new(ctx: &'a Ctx) -> Self {
        AppHost { ctx, service_requests: Vec::new() }
    }
}

/// The shell service that runs the system intents, and the parameters of a Start Timer step of `minutes`.
pub const SYSTEM_INTENTS: &str = "system_intents";
pub fn timer_params(minutes: u32) -> Json {
    Json::Obj(vec![
        ("intent".into(), Json::Str("timer_start".into())),
        ("duration".into(), Json::Num(minutes as f64 * 60.0)),
        ("label".into(), Json::Str(format!("{minutes} min"))),
    ])
}

impl Host for AppHost<'_> {
    fn perform(&mut self, action: &Action, input: &str) -> Result<Done, String> {
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
            Action::Script(source) => {
                let mut host = ScriptHost { ctx: self.ctx };
                match script::run_source(&script_source(source), &mut host, &[input.to_string()]) {
                    Ok(value) => {
                        let text = value.text();
                        Ok(Done::with_output(&text, &text))
                    }
                    Err(e) => Err(e.message),
                }
            }
            Action::Timer(minutes) => {
                let id = service::call(self.ctx, SYSTEM_INTENTS, "run", &timer_params(*minutes));
                self.service_requests.push(id);
                Ok(Done::said(&format!("Started a {minutes}-minute timer in Clock")))
            }
            other => Err(format!("{} is not available from Shortcuts on this computer yet", other.title())),
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
    fn start_timer_asks_the_shells_clock_through_the_system_intent() {
        let ctx = ctx();
        let mut host = AppHost::new(&ctx);
        let steps = flow::run(&[Action::Timer(10)], &mut host);
        assert_eq!(steps[0].outcome, Ok("Started a 10-minute timer in Clock".into()));
        let sent = ctx.take_sent();
        let ToShell::ServiceCall { request, service, method, body } = &sent[0] else { panic!("a service call") };
        assert_eq!((service.as_str(), method.as_str()), ("system_intents", "run"));
        assert!(body.contains("timer_start") && body.contains("600"), "{body}");
        assert_eq!(host.service_requests, vec![*request], "the reply is watched for a failure");
    }

    #[test]
    fn a_step_that_needs_the_shell_stops_the_shortcut_with_a_reason() {
        let ctx = ctx();
        let list = vec![Action::Text("a".into()), Action::Wifi(true), Action::ShowResult("never".into())];
        let steps = flow::run(&list, &mut AppHost::new(&ctx));
        assert!(steps[1].outcome.as_ref().unwrap_err().contains("Set Wi-Fi is not available"));
        assert!(steps[2].outcome.is_err(), "the result after it is not shown");
        assert!(ctx.take_sent().is_empty(), "nothing reached the shell");
    }

    #[test]
    fn a_run_script_step_gets_the_previous_output_as_its_first_argument() {
        let ctx = ctx();
        let list = vec![Action::Text("Hello".into()), Action::Script(springboard::shortcuts::DEFAULT_SCRIPT.into())];
        let steps = flow::run(&list, &mut AppHost::new(&ctx));
        assert_eq!(steps[1].outcome, Ok("Hello".into()));
    }

    #[test]
    fn a_run_script_step_can_notify_and_is_refused_a_shell_command() {
        let ctx = ctx();
        let notify = vec![Action::Script("display notification \"Built\" with title \"Scripts\"".into())];
        let steps = flow::run(&notify, &mut AppHost::new(&ctx));
        assert!(steps[0].outcome.is_ok());
        assert!(ctx.take_sent().iter().any(|m| matches!(m, ToShell::Notify { title, body } if title == "Scripts" && body == "Built")));

        let shell = vec![Action::Script("do shell script \"rm -rf ~\"".into()), Action::ShowResult("never".into())];
        let steps = flow::run(&shell, &mut AppHost::new(&ctx));
        assert!(steps[0].outcome.as_ref().unwrap_err().contains("confirmation"));
        assert!(steps[1].outcome.is_err());
        assert!(ctx.take_sent().is_empty());
    }
}
