//! The permission sheet: when an app asks for location, the camera, the microphone or screen recording through the
//! `privacy` service, the shell asks the person Allow or Don't Allow on an alert card.
//!
//! The sheet is the permission question model (`bifrost_ui::permission_question`). It sends one question about the
//! privacy topic to its own `AskCenter`, and the person's button is the answer. The default choice is Don't Allow, so a
//! question nobody answers never grants. The answer is written with `privacy_permissions::decide`, the same store the
//! Settings switches use. The decisions are pure (`Sheet::new`, `Sheet::finish`), so they are testable without a
//! screen; `Sheet` only draws its card.
//!
//! The queue of requests is in `services::privacy_permissions` (`queue_prompt`, `take_prompt`). The shell takes one
//! request at a time, from its frame tick, when no sheet is on screen.

use super::services::privacy_permissions::{self, Prompt};
use super::ui::{AlertCard, Ui, Weight, LINK, MAC_BODY_PT};
use crate::gfx::Surface;
use bifrost_ui::permission_question::{AskCenter, PermissionQuestion, QuestionTopic};

/// The choice ids the question offers. Don't Allow is the default, the second one.
pub const ALLOW: &str = "allow";
pub const DONT_ALLOW: &str = "dont_allow";
const DEFAULT_INDEX: usize = 1;

/// The one question a sheet asks. Each sheet has its own queue, so the number only needs to be unique there.
const QUESTION_ID: u64 = 1;

/// Where the person changes the answer later, as the sheet's text says.
pub const SETTINGS_PATH: &str = "Settings > Privacy & Security > App Permissions";

/// The privacy topic: which permission the question is about, by its wire name (`location`, `camera`, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrivacyTopic(pub &'static str);

impl QuestionTopic for PrivacyTopic {
    fn topic_name(&self) -> &'static str {
        self.0
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

/// The text under the title: what allowing does, and where to change it.
pub fn detail() -> String {
    format!("Allowing lets the app use this while it is open. You can change this at any time in {SETTINGS_PATH}.")
}

/// One request on screen: its question, the centre that holds it, and the card that draws it.
pub struct Sheet {
    prompt: Prompt,
    center: AskCenter,
    card: AlertCard,
}

impl Sheet {
    /// The sheet for one request, sent to a fresh queue at `now` (seconds). None only if the model refuses the question,
    /// which the fixed choices here never do.
    pub fn new(ui: &Ui, prompt: Prompt, now: u64) -> Option<Self> {
        let question = PermissionQuestion::for_topic(PrivacyTopic(prompt.kind.name()), QUESTION_ID, None)
            .with_choices(&[ALLOW, DONT_ALLOW], DEFAULT_INDEX)
            .ok()?;
        let mut center = AskCenter::new();
        center.ask(&question, now).ok()?;
        let labels = BUTTONS
            .iter()
            .map(|b| match b {
                Button::Allow => ui.text(b.label(), MAC_BODY_PT, Weight::Semibold, LINK),
                Button::DontAllow => ui.text(b.label(), MAC_BODY_PT, Weight::Regular, LINK),
            })
            .collect();
        let card = AlertCard::new(ui, &prompt.kind.title(&prompt.app_name), &detail(), labels);
        Some(Sheet { prompt, center, card })
    }

    /// The request this sheet answers.
    pub fn prompt(&self) -> &Prompt {
        &self.prompt
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

    /// The person pressed a button at `now`. The question takes the answer and the store is written. Returns whether
    /// the request was allowed, or None when the question refused the answer.
    pub fn finish(mut self, button: Button, now: u64) -> Option<bool> {
        let response = self.center.answer(QUESTION_ID, button.choice(), now).ok()?;
        let allowed = response.choice == ALLOW;
        privacy_permissions::decide(&self.prompt, allowed);
        Some(allowed)
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        self.card.draw(s, ui, w, h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::services::privacy_permissions::{self as store, Kind};
    use bifrost_ui::capture_access::Media;
    use bifrost_ui::location_auth::AuthorizationStatus;

    /// A request for `kind` from `id`, queued and taken the way the shell takes it.
    fn request(id: &str, name: &str, kind: Kind) -> Prompt {
        store::queue_prompt(id, name, kind);
        store::take_prompt().expect("a request the person has not answered")
    }

    #[test]
    fn allow_writes_the_location_switch_on() {
        let ui = crate::shell::tests::ui();
        let id = "is.example.sheet.allow";
        let sheet = Sheet::new(&ui, request(id, "Maps", Kind::Location), 0).unwrap();
        assert_eq!(sheet.prompt().app_name, "Maps");
        let (x, y) = sheet.button(&ui, 1200, 1920, Button::Allow).unwrap();
        assert_eq!(sheet.hit(&ui, 1200, 1920, x, y), Some(Button::Allow));
        assert_eq!(sheet.finish(Button::Allow, 5), Some(true));
        assert_eq!(store::location_status(id), AuthorizationStatus::AuthorizedWhenInUse);
    }

    #[test]
    fn dont_allow_writes_the_location_switch_off() {
        let ui = crate::shell::tests::ui();
        let id = "is.example.sheet.deny";
        let sheet = Sheet::new(&ui, request(id, "Maps", Kind::Location), 0).unwrap();
        let (x, y) = sheet.button(&ui, 1200, 1920, Button::DontAllow).unwrap();
        assert_eq!(sheet.hit(&ui, 1200, 1920, x, y), Some(Button::DontAllow));
        assert_eq!(sheet.finish(Button::DontAllow, 5), Some(false));
        assert_eq!(store::location_status(id), AuthorizationStatus::Denied);
    }

    #[test]
    fn a_camera_sheet_grants_only_the_camera() {
        let ui = crate::shell::tests::ui();
        let id = "is.example.sheet.cam";
        let sheet = Sheet::new(&ui, request(id, "Camera", Kind::Media(Media::Video)), 0).unwrap();
        assert_eq!(sheet.finish(Button::Allow, 1), Some(true));
        assert!(store::access(id).can_capture(Media::Video));
        assert!(!store::access(id).can_capture(Media::Audio));
    }

    #[test]
    fn a_tap_outside_the_buttons_hits_nothing() {
        let ui = crate::shell::tests::ui();
        let sheet = Sheet::new(&ui, request("is.example.sheet.miss", "Miss", Kind::Location), 0).unwrap();
        assert_eq!(sheet.hit(&ui, 1200, 1920, 2.0, 2.0), None);
    }

    #[test]
    fn the_detail_names_the_settings_path() {
        assert!(detail().contains("Settings > Privacy & Security > App Permissions"));
        assert_eq!(Button::Allow.choice(), "allow");
        assert_eq!(Button::DontAllow.choice(), "dont_allow");
    }
}
