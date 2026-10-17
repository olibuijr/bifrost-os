//! The password sheet for the `local_auth` service (`LAContext.evaluatePolicy` with the device-owner policy).
//!
//! An app that asks for the device password gets a pending reply, and the shell draws this sheet: the app's reason and
//! one secure field, Cancel and Continue. The password is typed here and never reaches the app. Continue hands it to
//! the `local_auth` service's `answer` method, which checks it through the login backend (the same PAM path as the
//! lock screen) and records the answer for the app's next call. Cancel records `user_cancel`.
//!
//! The sheet is the Wi-Fi password card (`wifi_page::Sheet::form`) with the one-field prompt of
//! `forms::Purpose::AppAuthentication`. The requests come from `services::local_auth` (`take_request`), one at a time.

use super::forms::{self, Purpose};
use super::keyboard::Out;
use super::services::local_auth::{self, Failure};
use super::wifi_page::{Sheet, SheetHit};
use super::Drawn;
use crate::gfx::{rgba, Surface};
use bifrost_core::json::Json;

/// The sheet on screen and the app it answers for.
pub struct AuthSheet {
    pub app: String,
    pub sheet: Sheet,
}

/// The one-field prompt for a request: the app's reason as the message.
pub fn prompt_for(app: &str, reason: &str) -> forms::Prompt {
    let message = if reason.trim().is_empty() { format!("“{app}” asks for your password.") } else { reason.trim().to_string() };
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

/// The `answer` call's params for a sheet that closed: the typed password, or a cancel.
pub fn answer_params(app: &str, password: Option<&str>) -> Json {
    let mut fields = vec![("app".to_string(), Json::Str(app.to_string()))];
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
        let sheet = Sheet::form(&self.ui, self.w, t, prompt_for(&request.app, &request.reason));
        self.auth_sheet = Some(AuthSheet { app: request.app, sheet });
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// Typing into the sheet: Return continues with a password that is not empty, Escape cancels, other keys edit
    /// the field.
    pub(super) fn auth_type(&mut self, out: Out, t: f64) {
        if out == Out::Named("Escape") {
            self.auth_close(None, t);
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
            self.auth_close(Some(password), t);
        }
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// A touch on the sheet: Cancel or Continue close it, anything else is ignored. The sheet takes every touch.
    pub(super) fn auth_touch(&mut self, x: f32, y: f32, up: bool) {
        if up {
            let hit = self.auth_sheet.as_ref().and_then(|a| a.sheet.hit(&self.ui, self.w, self.h, x, y));
            match hit {
                Some(SheetHit::Cancel) => {
                    let t = self.auth_clock;
                    self.auth_close(None, t);
                }
                Some(SheetHit::Join) => {
                    let typed = self.auth_sheet.as_ref().and_then(|a| a.sheet.form_result()).map(|(_, text)| text);
                    if let Some(password) = typed {
                        let t = self.auth_clock;
                        self.auth_close(Some(password), t);
                    }
                }
                _ => {}
            }
        }
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// Takes the sheet down and gives the answer to the `local_auth` service. `password` is `None` for a cancel.
    fn auth_close(&mut self, password: Option<String>, t: f64) {
        let Some(auth) = self.auth_sheet.take() else { return };
        let body = answer_params(&auth.app, password.as_deref()).encode();
        if self.service_call("", "local_auth", "answer", &body, t).is_err() {
            // the answer did not reach the service: the app must not wait forever
            local_auth::finish(&auth.app, Err(Failure::SystemCancel));
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
    fn the_prompt_shows_the_reason_and_hides_nothing_secret() {
        let p = prompt_for("app.notes", "  Reveal the saved note  ");
        assert_eq!(p.message, "Reveal the saved note");
        assert!(p.secure);
        assert_eq!(p.purpose, Purpose::AppAuthentication { app: "app.notes".into() });
        assert!(prompt_for("app.notes", "  ").message.contains("app.notes"));
    }

    #[test]
    fn the_answer_carries_the_password_or_a_cancel_and_the_app() {
        let typed = answer_params("app.notes", Some("hunter2"));
        assert_eq!(typed.get("app").and_then(Json::as_str), Some("app.notes"));
        assert_eq!(typed.get("password").and_then(Json::as_str), Some("hunter2"));
        assert!(typed.get("cancel").is_none());
        let cancelled = answer_params("app.notes", None);
        assert_eq!(cancelled.get("cancel"), Some(&Json::Bool(true)));
        assert!(cancelled.get("password").is_none());
    }
}
