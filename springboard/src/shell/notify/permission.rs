//! The first-time notification prompt. An app with the `notifications` permission that posts a request before it was
//! ever asked gets one Allow / Don't Allow alert. The official rule is that the first `requestAuthorization` prompts
//! the person and records the answer, and later requests do not prompt again. Here the answer is the app's Allow
//! Notifications switch in Settings: Allow clears the mute, Don't Allow sets it. The requests the app made while the
//! alert waited are answered with their status once the person chooses (`usernotif.rs`).
//!
//! The question is the shared permission model (`bifrost_ui::permission_question`), routed by the `notifications`
//! topic; the card is the shell's `AlertCard`. Don't Allow is the default choice, so a question nobody answers never
//! grants. The decisions are pure (`apply_answer`, `Sheet::new`, `Sheet::finish`) so they are testable without a
//! screen; `Sheet` only draws its card.

use super::super::ui::{AlertCard, Ui, Weight, LINK, MAC_BODY_PT};
use super::super::AppId;
use super::store::Store;
use crate::gfx::Surface;
use bifrost_ui::permission_question::{AskCenter, PermissionQuestion, QuestionTopic};

/// The choice ids the question offers. Don't Allow is the default, the second one.
pub const ALLOW: &str = "allow";
pub const DONT_ALLOW: &str = "dont_allow";
const DEFAULT_INDEX: usize = 1;

/// The one question a sheet asks. Each sheet has its own queue, so the number only needs to be unique there.
const QUESTION_ID: u64 = 1;

/// The topic the question is routed by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotifTopic;

impl QuestionTopic for NotifTopic {
    fn topic_name(&self) -> &'static str {
        "notifications"
    }
}

/// The two buttons, left to right: Don't Allow, then Allow (the one that is bold).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    DontAllow,
    Allow,
}

const BUTTONS: [Button; 2] = [Button::DontAllow, Button::Allow];

impl Button {
    /// The choice id this button sends.
    pub fn choice(self) -> &'static str {
        match self {
            Button::Allow => ALLOW,
            Button::DontAllow => DONT_ALLOW,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Button::Allow => "Allow",
            Button::DontAllow => "Don't Allow",
        }
    }
}

/// The alert's title: the app and what it wants.
pub fn title(app_name: &str) -> String {
    format!("\u{201c}{app_name}\u{201d} Would Like to Send You Notifications")
}

/// The text under the title: what allowing does, and where to change it.
pub fn detail() -> &'static str {
    "Notifications may include alerts, sounds, and icon badges. These can be configured in Settings > Notifications."
}

/// The store writes of an answer: the app is not asked again, and its Allow Notifications switch follows the answer.
pub fn apply_answer(store: &mut Store, app: AppId, allowed: bool) {
    store.set_asked(app);
    store.set_muted(app, !allowed);
}

/// One alert on screen: the app it is about, the centre that holds its question, and the card that draws it.
pub struct Sheet {
    app: AppId,
    center: AskCenter,
    card: AlertCard,
}

impl Sheet {
    /// The alert for `app` (shown as `app_name`), asked at `now`. None only if the model refuses the question, which the
    /// fixed choices here never do.
    pub fn new(ui: &Ui, app: AppId, app_name: &str, now: u64) -> Option<Self> {
        let question = PermissionQuestion::for_topic(NotifTopic, QUESTION_ID, None).with_choices(&[ALLOW, DONT_ALLOW], DEFAULT_INDEX).ok()?;
        let mut center = AskCenter::new();
        center.ask(&question, now).ok()?;
        let labels = BUTTONS
            .iter()
            .map(|b| match b {
                Button::Allow => ui.text(b.label(), MAC_BODY_PT, Weight::Semibold, LINK),
                Button::DontAllow => ui.text(b.label(), MAC_BODY_PT, Weight::Regular, LINK),
            })
            .collect();
        let card = AlertCard::new(ui, &title(app_name), detail(), labels);
        Some(Sheet { app, center, card })
    }

    /// The app this alert is about.
    pub fn app(&self) -> AppId {
        self.app
    }

    /// The button under (x, y), if any.
    pub fn hit(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<Button> {
        self.card.hit(ui, w, h, x, y).map(|i| BUTTONS[i])
    }

    /// Centre of a button, for tests.
    pub fn button(&self, ui: &Ui, w: i32, h: i32, which: Button) -> Option<(f32, f32)> {
        let i = BUTTONS.iter().position(|b| *b == which)?;
        Some(self.card.button_centre(ui, w, h, i))
    }

    /// The person pressed `button` at `now`. Returns whether the notifications are allowed, or None when the question
    /// refused the answer.
    pub fn finish(mut self, button: Button, now: u64) -> Option<bool> {
        let response = self.center.answer(QUESTION_ID, button.choice(), now).ok()?;
        Some(response.choice == ALLOW)
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        self.card.draw(s, ui, w, h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_clears_the_mute_and_dont_allow_sets_it_and_both_stop_the_prompt() {
        let mut s = Store::memory();
        apply_answer(&mut s, AppId::Files, true);
        assert!(s.is_asked(AppId::Files) && !s.is_muted(AppId::Files));
        apply_answer(&mut s, AppId::Terminal, false);
        assert!(s.is_asked(AppId::Terminal) && s.is_muted(AppId::Terminal));
        assert!(!s.is_asked(AppId::Photos), "an app nobody answered for is still asked");
    }

    #[test]
    fn the_question_is_routed_by_the_notifications_topic_and_defaults_to_dont_allow() {
        let q = PermissionQuestion::for_topic(NotifTopic, QUESTION_ID, None).with_choices(&[ALLOW, DONT_ALLOW], DEFAULT_INDEX).unwrap();
        assert_eq!(q.topic().topic_name(), "notifications");
        assert_eq!(q.choices(), &[ALLOW.to_string(), DONT_ALLOW.to_string()]);
        assert_eq!(q.default_choice(), DONT_ALLOW);
    }

    #[test]
    fn the_answer_comes_back_as_allowed_or_not() {
        let mut center = AskCenter::new();
        let q = PermissionQuestion::for_topic(NotifTopic, QUESTION_ID, None).with_choices(&[ALLOW, DONT_ALLOW], DEFAULT_INDEX).unwrap();
        center.ask(&q, 0).unwrap();
        assert_eq!(center.answer(QUESTION_ID, ALLOW, 0).unwrap().choice, ALLOW);
        let mut center = AskCenter::new();
        center.ask(&q, 0).unwrap();
        assert_eq!(center.answer(QUESTION_ID, DONT_ALLOW, 0).unwrap().choice, DONT_ALLOW);
        assert_eq!(Button::Allow.choice(), ALLOW);
        assert_eq!(Button::DontAllow.choice(), DONT_ALLOW);
    }

    #[test]
    fn the_title_names_the_app() {
        assert_eq!(title("Chat"), "\u{201c}Chat\u{201d} Would Like to Send You Notifications");
    }
}
