//! The guardian's sheet: a question an app sent through PermissionKit (`services::permission_kit::ask`), shown to the
//! guardian as an alert card with one button per choice, in the order the app gave them. The button's answer goes back
//! with `respond`. A question nobody answers is dropped by the expiry timer in the shell's tick, and the sheet goes with
//! it.
//!
//! The sheet is a pure model plus its card (`GuardianSheet::new`, `hit`, `finish`), so the answers are testable
//! without a screen.

use super::services::permission_kit::{self, Pending};
use super::ui::{AlertCard, Ui, Weight, LINK, MAC_BODY_PT};
use crate::gfx::Surface;

/// One pending question on screen.
pub struct GuardianSheet {
    question_id: u64,
    /// The choice ids, in button order.
    choices: Vec<String>,
    card: AlertCard,
}

impl GuardianSheet {
    /// The sheet for `pending`: one button per choice. None when the question offers no choice, since there would be
    /// nothing to answer with.
    pub fn new(ui: &Ui, pending: &Pending) -> Option<Self> {
        if pending.choices.is_empty() {
            return None;
        }
        let app = pending.topic.split('/').next().unwrap_or_default();
        let title = format!("Approve the update of {app}?");
        let labels = pending.choices.iter().map(|(_, label)| ui.text(label, MAC_BODY_PT, Weight::Regular, LINK)).collect();
        let card = AlertCard::new(ui, &title, "Your answer is sent back to the app that asked. It applies to that app only.", labels);
        let choices = pending.choices.iter().map(|(id, _)| id.clone()).collect();
        Some(GuardianSheet { question_id: pending.id, choices, card })
    }

    /// The question this sheet asks.
    pub fn question_id(&self) -> u64 {
        self.question_id
    }

    /// The choice id of the button under (x, y), if any.
    pub fn hit(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<String> {
        self.card.hit(ui, w, h, x, y).and_then(|i| self.choices.get(i).cloned())
    }

    /// Centre of a button, for tests.
    pub fn button(&self, ui: &Ui, w: i32, h: i32, index: usize) -> Option<(f32, f32)> {
        (index < self.choices.len()).then(|| self.card.button_centre(ui, w, h, index))
    }

    /// The guardian picked `choice` at `now` (seconds). Returns whether the answer was accepted: an unknown or expired
    /// question, or a choice it does not offer, is refused and leaves the queue as it was.
    pub fn finish(self, choice: &str, now: u64) -> bool {
        permission_kit::respond(self.question_id, choice, now).is_ok()
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        self.card.draw(s, ui, w, h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(id: u64) -> Pending {
        Pending {
            id,
            topic: "is.example.guard/significant-app-update".into(),
            choices: vec![("approve".into(), "Allow".into()), ("decline".into(), "Not now".into())],
        }
    }

    #[test]
    fn each_choice_gets_its_button_in_the_apps_order() {
        let ui = crate::shell::tests::ui();
        let sheet = GuardianSheet::new(&ui, &pending(7)).unwrap();
        assert_eq!(sheet.question_id(), 7);
        let (x, y) = sheet.button(&ui, 1200, 1920, 1).unwrap();
        assert_eq!(sheet.hit(&ui, 1200, 1920, x, y), Some("decline".to_string()));
        assert!(sheet.button(&ui, 1200, 1920, 2).is_none(), "no third button");
        assert_eq!(sheet.hit(&ui, 1200, 1920, 2.0, 2.0), None, "a tap outside the buttons hits nothing");
    }

    #[test]
    fn a_question_without_choices_has_no_sheet() {
        let ui = crate::shell::tests::ui();
        let empty = Pending { choices: vec![], ..pending(1) };
        assert!(GuardianSheet::new(&ui, &empty).is_none());
    }

    #[test]
    fn an_answer_to_a_question_that_is_gone_is_refused() {
        let ui = crate::shell::tests::ui();
        let sheet = GuardianSheet::new(&ui, &pending(u64::MAX)).unwrap();
        assert!(!sheet.finish("approve", 0), "no such question in the queue");
    }
}
