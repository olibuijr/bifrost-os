//! The account access sheet: when an app asks for access to an account type (`accounts` service, `request_access`), the
//! shell asks the person Allow or Don't Allow on an alert card. Don't Allow is the default, so an unanswered question
//! never grants. The answer goes back through the service's `answer` call, which writes the grant to the accounts
//! store (`bifrost_ui::account_store::set_access`).
//!
//! The requests come from `services::accounts_access` (`take_request`), one at a time, and the sheet takes every touch
//! until the person answers it.

use super::services::accounts_access::{self, Request};
use super::ui::{AlertCard, Ui, Weight, LINK, MAC_BODY_PT};
use super::Drawn;
use crate::gfx::{rgba, Surface};
use bifrost_core::json::Json;

/// The buttons, in order: Allow, then Don't Allow (the default).
const ALLOW: &str = "Allow";
const DONT_ALLOW: &str = "Don't Allow";

/// The card's title: the question the sheet asks.
pub fn title(request: &Request) -> String {
    format!("Allow “{}” to use your {} accounts?", request.app_name, request.type_description)
}

/// The text under the title.
pub const DETAIL: &str = "You can change this at any time in Settings.";

/// The sheet on screen and the request it answers.
pub struct AccessSheet {
    pub request: Request,
    card: AlertCard,
}

/// The `answer` call's params for a button: the app, the type and the person's choice.
pub fn answer_params(request: &Request, granted: bool) -> Json {
    Json::Obj(vec![
        ("app".into(), Json::Str(request.app.clone())),
        ("type".into(), Json::Str(request.account_type.clone())),
        ("granted".into(), Json::Bool(granted)),
    ])
}

impl super::Shell {
    /// Whether the access sheet is on screen. It takes every touch while it is.
    pub(super) fn access_showing(&self) -> bool {
        self.access_sheet.is_some()
    }

    /// Shows the next queued request when no sheet is on screen.
    pub(super) fn access_tick(&mut self) {
        if self.access_sheet.is_some() {
            return;
        }
        let Some(request) = accounts_access::take_request() else { return };
        let labels = [ALLOW, DONT_ALLOW]
            .iter()
            .map(|label| self.ui.text(label, MAC_BODY_PT, if *label == ALLOW { Weight::Semibold } else { Weight::Regular }, LINK))
            .collect();
        let card = AlertCard::new(&self.ui, &title(&request), DETAIL, labels);
        self.access_sheet = Some(AccessSheet { request, card });
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// A touch on the sheet: a button answers it, anything else is ignored.
    pub(super) fn access_touch(&mut self, x: f32, y: f32, up: bool, t: f64) {
        if up {
            let hit = self.access_sheet.as_ref().and_then(|a| a.card.hit(&self.ui, self.w, self.h, x, y));
            let granted = match hit {
                Some(0) => Some(true),
                Some(1) => Some(false),
                _ => None,
            };
            if let Some(granted) = granted {
                self.access_close(granted, t);
            }
        }
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// Takes the sheet down and records the person's answer with the `accounts` service.
    fn access_close(&mut self, granted: bool, t: f64) {
        let Some(sheet) = self.access_sheet.take() else { return };
        let body = answer_params(&sheet.request, granted).encode();
        if self.service_call("", "accounts", "answer", &body, t).is_err() {
            // the answer did not reach the service: the app must not wait forever
            accounts_access::finish(&sheet.request.app, &sheet.request.account_type, false);
        }
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// Draws the card over a dimmed screen.
    pub(super) fn access_draw(&self, s: &mut Surface) {
        if let Some(a) = &self.access_sheet {
            s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, rgba(0, 0, 0, 120));
            a.card.draw(s, &self.ui, self.w, self.h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_names_the_app_and_the_account_type() {
        let r = Request { app: "app.notes".into(), app_name: "Notes".into(), account_type: "bifrost.mail".into(), type_description: "Mail".into() };
        assert_eq!(title(&r), "Allow “Notes” to use your Mail accounts?");
    }

    #[test]
    fn the_answer_names_the_app_the_type_and_the_choice() {
        let r = Request { app: "app.notes".into(), app_name: "Notes".into(), account_type: "bifrost.mail".into(), type_description: "Mail".into() };
        let a = answer_params(&r, true);
        assert_eq!(a.get("app").and_then(Json::as_str), Some("app.notes"));
        assert_eq!(a.get("type").and_then(Json::as_str), Some("bifrost.mail"));
        assert_eq!(a.get("granted"), Some(&Json::Bool(true)));
    }
}
