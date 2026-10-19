//! The network login sheet (`NetAuth`): when an app asks the `network_auth` service for the login of an SMB share or an
//! 802.1X Wi-Fi network and none is saved, the shell asks the person here. The model is `crate::net_auth` (wording,
//! answer checks, where a remembered login goes); the requests come from `services::network_auth` (`take_request`), one
//! at a time.
//!
//! Three steps, the pattern of the account setup prompts (`forms.rs`): the user name (starting from the name that was
//! just refused, if any), the password in a secure field, and an alert card asking whether to remember the login in the
//! keychain. Cancel or Escape at any step ends it with no login. The finished answer goes to the service's `answer`
//! call, which checks it, saves it only when asked, and hands the login to the waiting app. Nothing is kept here.

use super::forms::{self, Purpose};
use super::keyboard::Out;
use super::services::network_auth::{self, Outcome, Request};
use super::ui::{AlertCard, Ui, Weight, LINK, MAC_BODY_PT};
use super::wifi_page::{Sheet, SheetHit};
use super::Drawn;
use crate::gfx::{rgba, Surface};
use crate::net_auth::MAX_FIELD_BYTES;
use bifrost_core::json::Json;

/// The steps of the sheet.
enum Step {
    User(Sheet),
    Password { user: String, sheet: Sheet },
    Remember { user: String, password: String, card: AlertCard },
}

/// The sheet on screen and the request it answers.
pub struct NetAuthSheet {
    request: Request,
    step: Step,
}

/// The one-field prompt for the user name. The wording is the model's (`net_auth::Prompt`).
pub fn user_prompt(request: &Request) -> forms::Prompt {
    forms::Prompt {
        purpose: Purpose::NetworkLogin { password: false },
        title: request.prompt.title.clone(),
        message: request.prompt.message.clone(),
        label: "User Name",
        ok: "Next",
        secure: false,
        max: MAX_FIELD_BYTES,
        initial: request.prompt.user_hint.clone().unwrap_or_default(),
    }
}

/// The one-field prompt for the password of `user`: a secure field.
pub fn password_prompt(request: &Request, user: &str) -> forms::Prompt {
    forms::Prompt {
        purpose: Purpose::NetworkLogin { password: true },
        title: request.prompt.title.clone(),
        message: format!("Enter the password for \u{201c}{user}\u{201d}."),
        label: "Password",
        ok: "Continue",
        secure: true,
        max: MAX_FIELD_BYTES,
        initial: String::new(),
    }
}

/// The card that asks whether to keep the login. "Remember" is the model's default.
fn remember_card(ui: &Ui, request: &Request) -> AlertCard {
    let what = match &request.prompt.target {
        crate::net_auth::Target::Share { share, .. } => format!("\u{201c}{share}\u{201d}"),
        crate::net_auth::Target::Wifi { ssid } => format!("\u{201c}{ssid}\u{201d}"),
    };
    let weight = |remember: bool| if remember == request.prompt.remember { Weight::Semibold } else { Weight::Regular };
    let buttons = vec![ui.text("Don\u{2019}t Remember", MAC_BODY_PT, weight(false), LINK), ui.text("Remember", MAC_BODY_PT, weight(true), LINK)];
    AlertCard::new(ui, "Remember this login?", &format!("The name and password for {what} are kept in your keychain, so you are not asked again."), buttons)
}

/// The `answer` call's params: the request, and the typed login with whether to remember it, or a cancel.
pub fn answer_params(request: &Request, login: Option<(&str, &str, bool)>) -> Json {
    let mut fields = vec![("app".to_string(), Json::Str(request.app.clone())), ("key".to_string(), Json::Str(request.key()))];
    match login {
        Some((user, password, remember)) => {
            fields.push(("user".to_string(), Json::Str(user.to_string())));
            fields.push(("password".to_string(), Json::Str(password.to_string())));
            fields.push(("remember".to_string(), Json::Bool(remember)));
        }
        None => fields.push(("cancel".to_string(), Json::Bool(true))),
    }
    Json::Obj(fields)
}

impl super::Shell {
    /// Whether the login sheet is on screen. Touches and typing belong to it while it is.
    pub(super) fn net_auth_showing(&self) -> bool {
        self.net_auth_sheet.is_some()
    }

    /// Shows the next queued request when the screen is unlocked and no login sheet is on screen. `t` is the shell clock.
    pub(super) fn net_auth_tick(&mut self, t: f64) {
        if self.net_auth_sheet.is_some() || self.is_locked() {
            return;
        }
        let Some(request) = network_auth::take_request() else { return };
        let sheet = Sheet::form(&self.ui, self.w, t, user_prompt(&request));
        self.net_auth_sheet = Some(NetAuthSheet { request, step: Step::User(sheet) });
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// Typing into the sheet: Return goes on with a field that is acceptable, Escape cancels, other keys edit the field.
    /// The remember card takes Return as "Remember" and ignores other keys.
    pub(super) fn net_auth_type(&mut self, out: Out, t: f64) {
        if out == Out::Named("Escape") {
            self.net_auth_finish(None, t);
            return;
        }
        if matches!(self.net_auth_sheet.as_ref().map(|s| &s.step), Some(Step::Remember { .. })) {
            if out == Out::Enter {
                self.net_auth_remember(true, t);
            }
            self.dirty = true;
            self.drawn = Drawn::Nothing;
            return;
        }
        let Some(sheet) = self.net_auth_sheet.as_mut() else { return };
        let typed = match &mut sheet.step {
            Step::User(s) | Step::Password { sheet: s, .. } => {
                if out == Out::Enter {
                    s.form_result().map(|(_, text)| text)
                } else {
                    s.edit(&self.ui, out, t);
                    None
                }
            }
            Step::Remember { .. } => None,
        };
        if let Some(text) = typed {
            self.net_auth_next(text, t);
        }
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// A touch on the sheet: Cancel or the button goes on, the remember card's buttons answer. The sheet takes every touch.
    pub(super) fn net_auth_touch(&mut self, x: f32, y: f32, up: bool, t: f64) {
        if up {
            enum Hit {
                Cancel,
                Next(String),
                Remember(bool),
                Nothing,
            }
            let hit = match self.net_auth_sheet.as_ref().map(|s| &s.step) {
                Some(Step::User(s)) | Some(Step::Password { sheet: s, .. }) => match s.hit(&self.ui, self.w, self.h, x, y) {
                    Some(SheetHit::Cancel) => Hit::Cancel,
                    Some(SheetHit::Join) => s.form_result().map_or(Hit::Nothing, |(_, text)| Hit::Next(text)),
                    _ => Hit::Nothing,
                },
                Some(Step::Remember { card, .. }) => match card.hit(&self.ui, self.w, self.h, x, y) {
                    Some(0) => Hit::Remember(false),
                    Some(1) => Hit::Remember(true),
                    _ => Hit::Nothing,
                },
                None => Hit::Nothing,
            };
            match hit {
                Hit::Cancel => self.net_auth_finish(None, t),
                Hit::Next(text) => self.net_auth_next(text, t),
                Hit::Remember(on) => self.net_auth_remember(on, t),
                Hit::Nothing => {}
            }
        }
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// The step's field was accepted with `text`: on to the next step.
    fn net_auth_next(&mut self, text: String, t: f64) {
        let Some(sheet) = self.net_auth_sheet.as_mut() else { return };
        let next = match &sheet.step {
            Step::User(_) => Step::Password { sheet: Sheet::form(&self.ui, self.w, t, password_prompt(&sheet.request, &text)), user: text },
            Step::Password { user, .. } => Step::Remember { card: remember_card(&self.ui, &sheet.request), user: user.clone(), password: text },
            Step::Remember { .. } => return,
        };
        sheet.step = next;
    }

    /// The remember card was answered: send the login to the service.
    fn net_auth_remember(&mut self, remember: bool, t: f64) {
        let Some(NetAuthSheet { step: Step::Remember { user, password, .. }, .. }) = self.net_auth_sheet.as_ref() else { return };
        let login = (user.clone(), password.clone(), remember);
        self.net_auth_finish(Some(login), t);
    }

    /// Takes the sheet down and gives the person's answer (or a cancel) to the `network_auth` service.
    fn net_auth_finish(&mut self, login: Option<(String, String, bool)>, t: f64) {
        let Some(sheet) = self.net_auth_sheet.take() else { return };
        let body = answer_params(&sheet.request, login.as_ref().map(|(u, p, r)| (u.as_str(), p.as_str(), *r))).encode();
        if self.service_call("", "network_auth", "answer", &body, t).is_err() {
            // the answer did not reach the service: the app must not wait forever
            network_auth::finish(&sheet.request.app, &sheet.request.key(), Outcome::Cancelled);
        }
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// Draws the sheet over a dimmed screen.
    pub(super) fn net_auth_draw(&mut self, s: &mut Surface, t: f64) {
        let (w, h) = (self.w, self.h);
        if let Some(sheet) = self.net_auth_sheet.as_mut() {
            match &mut sheet.step {
                Step::User(f) | Step::Password { sheet: f, .. } => {
                    s.fill_rrect(0.0, 0.0, w as f32, h as f32, 0.0, rgba(0, 0, 0, 120));
                    f.draw(s, &self.ui, w, h, t);
                }
                Step::Remember { card, .. } => card.draw(s, &self.ui, w, h),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keychain::{ShareAccess, Store};
    use crate::net_auth::{self, Target};

    fn shell(name: &str) -> crate::shell::Shell {
        let d = std::env::temp_dir().join(format!("netauth-sheet-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let mut sh = crate::shell::tests::shell();
        sh.data_dir = d;
        // the test shell starts on the lock screen, where no login sheet is shown
        crate::shell::tests::unlock(&mut sh);
        sh
    }

    fn share_params(host: &str, share: &str) -> Json {
        Json::Obj(vec![("kind".into(), Json::Str("share".into())), ("host".into(), Json::Str(host.into())), ("share".into(), Json::Str(share.into()))])
    }

    fn type_text(sh: &mut crate::shell::Shell, text: &str) {
        sh.net_auth_type(Out::Text(text.into()), 1.0);
    }

    fn login_of(reply: &Json) -> (String, String, bool) {
        let s = |k: &str| reply.get(k).and_then(Json::as_str).unwrap_or_default().to_string();
        (s("user"), s("password"), reply.get("saved") == Some(&Json::Bool(true)))
    }

    #[test]
    fn a_share_that_needs_a_login_shows_the_sheet_and_the_answer_reaches_the_app_and_the_keychain() {
        let mut sh = shell("share");
        let params = share_params("nas.sheet.test", "Docs");
        let reply = network_auth::request_login(&sh, "app.files", &params).unwrap();
        assert_eq!(reply.get("pending"), Some(&Json::Bool(true)));
        assert!(!sh.net_auth_showing());
        sh.net_auth_tick(0.0);
        assert!(sh.net_auth_showing());
        // Return with an empty name does not go on
        sh.net_auth_type(Out::Enter, 1.0);
        assert!(matches!(sh.net_auth_sheet.as_ref().unwrap().step, Step::User(_)));
        type_text(&mut sh, "ola");
        sh.net_auth_type(Out::Enter, 1.0);
        assert!(matches!(sh.net_auth_sheet.as_ref().unwrap().step, Step::Password { .. }));
        sh.net_auth_type(Out::Enter, 1.0);
        assert!(matches!(sh.net_auth_sheet.as_ref().unwrap().step, Step::Password { .. }), "an empty password does not go on");
        type_text(&mut sh, "secret");
        sh.net_auth_type(Out::Enter, 1.0);
        let Some(NetAuthSheet { step: Step::Remember { card, .. }, .. }) = sh.net_auth_sheet.as_ref() else { panic!("the remember card") };
        let (x, y) = card.button_centre(&sh.ui, sh.w, sh.h, 1);
        sh.net_auth_touch(x, y, false, 2.0);
        assert!(sh.net_auth_showing(), "a press alone answers nothing");
        sh.net_auth_touch(x, y, true, 2.0);
        assert!(!sh.net_auth_showing());
        // the app asks again: it gets what was typed, then the remembered login
        let got = network_auth::request_login(&sh, "app.files", &params).unwrap();
        assert_eq!(login_of(&got), ("ola".to_string(), "secret".to_string(), false));
        let again = network_auth::request_login(&sh, "app.files", &params).unwrap();
        assert_eq!(login_of(&again), ("ola".to_string(), "secret".to_string(), true));
        let store = Store::open_group(&sh.data_dir, net_auth::GROUP).unwrap();
        assert_eq!(net_auth::access(&store, &Target::Share { host: "nas.sheet.test".into(), share: "Docs".into() }), Ok(ShareAccess::Saved { user: "ola".into(), password: "secret".into() }));
    }

    #[test]
    fn dont_remember_gives_the_app_the_login_and_keeps_nothing() {
        let mut sh = shell("forget");
        let params = Json::Obj(vec![("kind".into(), Json::Str("wifi".into())), ("ssid".into(), Json::Str("Campus".into()))]);
        network_auth::request_login(&sh, "app.wifi", &params).unwrap();
        sh.net_auth_tick(0.0);
        assert!(sh.net_auth_sheet.as_ref().unwrap().request.prompt.title.contains("Campus"));
        type_text(&mut sh, "ola");
        sh.net_auth_type(Out::Enter, 1.0);
        type_text(&mut sh, "pw");
        sh.net_auth_type(Out::Enter, 1.0);
        let Some(NetAuthSheet { step: Step::Remember { card, .. }, .. }) = sh.net_auth_sheet.as_ref() else { panic!("the remember card") };
        let (x, y) = card.button_centre(&sh.ui, sh.w, sh.h, 0);
        sh.net_auth_touch(x, y, true, 2.0);
        let got = network_auth::request_login(&sh, "app.wifi", &params).unwrap();
        assert_eq!(login_of(&got), ("ola".to_string(), "pw".to_string(), false));
        let store = Store::open_group(&sh.data_dir, net_auth::GROUP).unwrap();
        assert_eq!(net_auth::access(&store, &Target::Wifi { ssid: "Campus".into() }), Ok(ShareAccess::Prompt));
    }

    #[test]
    fn escape_cancels_at_any_step_and_the_app_gets_no_login() {
        let mut sh = shell("escape");
        let params = share_params("nas.escape.test", "Docs");
        network_auth::request_login(&sh, "app.files", &params).unwrap();
        sh.net_auth_tick(0.0);
        type_text(&mut sh, "ola");
        sh.net_auth_type(Out::Enter, 1.0);
        sh.net_auth_type(Out::Named("Escape"), 1.0);
        assert!(!sh.net_auth_showing());
        let got = network_auth::request_login(&sh, "app.files", &params).unwrap();
        assert_eq!(got.get("error").and_then(Json::as_str), Some("cancelled"));
        let store = Store::open_group(&sh.data_dir, net_auth::GROUP).unwrap();
        assert_eq!(net_auth::access(&store, &Target::Share { host: "nas.escape.test".into(), share: "Docs".into() }), Ok(ShareAccess::Prompt));
    }

    #[test]
    fn a_refused_login_starts_from_the_refused_name_and_the_sheet_waits_for_the_unlock() {
        let mut sh = shell("retry");
        let mut params = share_params("nas.retry.test", "Docs");
        if let Json::Obj(f) = &mut params {
            f.push(("refused".into(), Json::Bool(true)));
            f.push(("user".into(), Json::Str("ola".into())));
        }
        network_auth::request_login(&sh, "app.files", &params).unwrap();
        let screen = sh.screen;
        sh.screen = crate::shell::Screen::Lock;
        sh.net_auth_tick(0.0);
        assert!(!sh.net_auth_showing(), "no sheet over the lock screen");
        sh.screen = screen;
        sh.net_auth_tick(1.0);
        let request = &sh.net_auth_sheet.as_ref().unwrap().request;
        assert!(request.prompt.retry && request.prompt.message.contains("not accepted"));
        assert_eq!(user_prompt(request).initial, "ola");
        assert!(password_prompt(request, "ola").secure);
        assert_eq!(password_prompt(request, "ola").ok, "Continue");
        sh.net_auth_type(Out::Named("Escape"), 2.0);
        network_auth::take_answer("app.files", &net_auth::Target::Share { host: "nas.retry.test".into(), share: "Docs".into() }.key());
    }

    #[test]
    fn the_answer_names_the_request_and_carries_the_login_or_a_cancel() {
        let request = Request { app: "app.files".into(), prompt: net_auth::share_prompt("nas", "Docs", None) };
        let typed = answer_params(&request, Some(("ola", "pw", true)));
        assert_eq!(typed.get("app").and_then(Json::as_str), Some("app.files"));
        assert_eq!(typed.get("key").and_then(Json::as_str), Some("share:nas/docs"));
        assert_eq!(typed.get("remember"), Some(&Json::Bool(true)));
        let cancelled = answer_params(&request, None);
        assert_eq!(cancelled.get("cancel"), Some(&Json::Bool(true)));
        assert!(cancelled.get("password").is_none());
    }
}
