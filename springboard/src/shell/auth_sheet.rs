//! The password sheet for the `local_auth` service (`LAContext.evaluatePolicy` with the device-owner policy).
//!
//! An app that asks for the device password gets a pending reply, and the shell draws this sheet: the app's reason and
//! one secure field, Cancel and Continue. The password is typed here and never reaches the app. Continue hands it to
//! the `local_auth` service's `answer` method, which checks it through the login backend (the same PAM path as the
//! lock screen). A wrong password keeps the sheet open for another try, up to `SHEET_ATTEMPTS` tries; the last wrong
//! password, a lockout or a cancel ends it and the app gets the answer on its next call.
//!
//! The sheet is the Wi-Fi password card (`wifi_page::Sheet::form`) with the one-field prompt of
//! `forms::Purpose::AppAuthentication`. The requests come from `services::local_auth` (`take_request`), one at a time.

use super::forms::{self, Purpose};
use super::keyboard::Out;
use super::services::local_auth::{self, Failure, SHEET_ATTEMPTS};
use super::wifi_page::{Sheet, SheetHit};
use super::Drawn;
use crate::gfx::{rgba, Surface};
use bifrost_core::json::Json;

/// The sheet on screen: the app it answers for, the reason, the wrong passwords so far and the card.
pub struct AuthSheet {
    pub app: String,
    pub reason: String,
    pub attempt: u32,
    pub sheet: Sheet,
}

/// The one-field prompt for a request: the app's reason as the message, with a note when a try was wrong.
pub fn prompt_for(app: &str, reason: &str, attempt: u32) -> forms::Prompt {
    let mut message = if reason.trim().is_empty() { format!("“{app}” asks for your password.") } else { reason.trim().to_string() };
    if attempt > 0 {
        let left = SHEET_ATTEMPTS.saturating_sub(attempt);
        message = format!("{message}\nThat password is wrong. {left} {} left.", if left == 1 { "try" } else { "tries" });
    }
    forms::Prompt {
        purpose: Purpose::AppAuthentication { app: app.into() },
        title: "Enter Password".into(),
        message,
        label: "Password",
        ok: "Continue",
        secure: true,
        max: 128,
        initial: String::new(),
    }
}

/// The `answer` call's params: the app, how many wrong passwords this sheet had, and the typed password or a cancel.
pub fn answer_params(app: &str, password: Option<&str>, attempt: u32) -> Json {
    let mut fields = vec![("app".to_string(), Json::Str(app.to_string())), ("attempt".to_string(), Json::Num(f64::from(attempt)))];
    match password {
        Some(p) => fields.push(("password".to_string(), Json::Str(p.to_string()))),
        None => fields.push(("cancel".to_string(), Json::Bool(true))),
    }
    Json::Obj(fields)
}

impl super::Shell {
    /// Whether the password sheet is on screen. Touches and typing belong to it while it is.
    pub(super) fn auth_showing(&self) -> bool {
        self.auth_sheet.is_some()
    }

    /// Shows the next queued request as the password sheet when none is on screen. `t` is the shell clock.
    pub(super) fn auth_tick(&mut self, t: f64) {
        self.auth_clock = t;
        if self.auth_sheet.is_some() {
            return;
        }
        let Some(request) = local_auth::take_request() else { return };
        let sheet = Sheet::form(&self.ui, self.w, t, prompt_for(&request.app, &request.reason, 0));
        self.auth_sheet = Some(AuthSheet { app: request.app, reason: request.reason, attempt: 0, sheet });
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// Typing into the sheet: Return continues with a password that is not empty, Escape cancels, other keys edit
    /// the field.
    pub(super) fn auth_type(&mut self, out: Out, t: f64) {
        if out == Out::Named("Escape") {
            self.auth_submit(None, t);
            return;
        }
        let typed = match self.auth_sheet.as_mut() {
            Some(auth) if out == Out::Enter => auth.sheet.form_result().map(|(_, text)| text),
            Some(auth) => {
                auth.sheet.edit(&self.ui, out, t);
                None
            }
            None => return,
        };
        if let Some(password) = typed {
            self.auth_submit(Some(password), t);
        }
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// A touch on the sheet: Cancel or Continue, anything else is ignored. The sheet takes every touch.
    pub(super) fn auth_touch(&mut self, x: f32, y: f32, up: bool) {
        if up {
            let hit = self.auth_sheet.as_ref().and_then(|a| a.sheet.hit(&self.ui, self.w, self.h, x, y));
            let t = self.auth_clock;
            match hit {
                Some(SheetHit::Cancel) => self.auth_submit(None, t),
                Some(SheetHit::Join) => {
                    let typed = self.auth_sheet.as_ref().and_then(|a| a.sheet.form_result()).map(|(_, text)| text);
                    if let Some(password) = typed {
                        self.auth_submit(Some(password), t);
                    }
                }
                _ => {}
            }
        }
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// Sends the typed password (or a cancel) to the `local_auth` service. A wrong password with tries left keeps the
    /// sheet open with the count; any other answer takes the sheet down, and the service has already given it to the app.
    fn auth_submit(&mut self, password: Option<String>, t: f64) {
        let Some(auth) = self.auth_sheet.as_ref() else { return };
        let (app, reason, attempt) = (auth.app.clone(), auth.reason.clone(), auth.attempt);
        let body = answer_params(&app, password.as_deref(), attempt).encode();
        let retry = match self.service_call("", "local_auth", "answer", &body, t) {
            Ok(reply) => matches!(reply.get("retry"), Some(Json::Bool(true))),
            Err(_) => {
                // the answer did not reach the service: the app must not wait forever
                local_auth::finish(&app, Err(Failure::SystemCancel));
                false
            }
        };
        if retry {
            let sheet = Sheet::form(&self.ui, self.w, t, prompt_for(&app, &reason, attempt + 1));
            self.auth_sheet = Some(AuthSheet { app, reason, attempt: attempt + 1, sheet });
        } else {
            self.auth_sheet = None;
        }
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// Draws the sheet over a dimmed screen.
    pub(super) fn auth_draw(&mut self, s: &mut Surface, t: f64) {
        let (w, h) = (self.w, self.h);
        if let Some(auth) = self.auth_sheet.as_mut() {
            s.fill_rrect(0.0, 0.0, w as f32, h as f32, 0.0, rgba(0, 0, 0, 120));
            auth.sheet.draw(s, &self.ui, w, h, t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_shows_the_reason_and_counts_the_tries_left() {
        let p = prompt_for("app.notes", "  Reveal the saved note  ", 0);
        assert_eq!(p.message, "Reveal the saved note");
        assert!(p.secure);
        assert_eq!(p.purpose, Purpose::AppAuthentication { app: "app.notes".into() });
        assert!(prompt_for("app.notes", "  ", 0).message.contains("app.notes"));
        assert!(prompt_for("app.notes", "Reveal", 1).message.ends_with("2 tries left."));
        assert!(prompt_for("app.notes", "Reveal", 2).message.ends_with("1 try left."));
    }

    #[test]
    fn the_answer_carries_the_password_or_a_cancel_the_app_and_the_try() {
        let typed = answer_params("app.notes", Some("hunter2"), 1);
        assert_eq!(typed.get("app").and_then(Json::as_str), Some("app.notes"));
        assert_eq!(typed.get("password").and_then(Json::as_str), Some("hunter2"));
        assert_eq!(typed.get("attempt"), Some(&Json::Num(1.0)));
        assert!(typed.get("cancel").is_none());
        let cancelled = answer_params("app.notes", None, 0);
        assert_eq!(cancelled.get("cancel"), Some(&Json::Bool(true)));
        assert!(cancelled.get("password").is_none());
    }
}
