//! The contact picker sheet: the Golden Gate alert card (the same card as the privacy sheet) that shows the open pick of
//! an app. A contact pick lists the contacts as buttons; a property pick lists the values of the contact the person
//! opened. Each sheet offers up to `MAX_CHOICES` choices and Cancel. The model is `contactpick`; this module draws the
//! card and turns a tap into the pick's next step.
use super::contactpick;
use super::ui::{AlertCard, Ui, Weight, LINK, MAC_BODY_PT};
use crate::gfx::Surface;
use bifrost_ui::contact_card::Key;

/// Most choices one sheet offers. A longer list needs a scrolling sheet, which is not built yet.
pub const MAX_CHOICES: usize = 4;

/// What a tap on the sheet means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tap {
    /// Pick this contact (a contact pick ends here; a property pick opens it).
    Contact(u64),
    /// Choose the `nth` value under `key` of the open contact.
    Value(Key, usize),
    /// End the pick without a choice.
    Cancel,
}

pub struct PickSheet {
    app_id: String,
    app_name: String,
    taps: Vec<Tap>,
    card: AlertCard,
}

impl PickSheet {
    /// The sheet for the app's open pick, or `None` when the app has no pick open.
    pub fn new(ui: &Ui, app_id: &str, app_name: &str) -> Option<Self> {
        let (title, detail, options): (&str, String, Vec<(Tap, String)>) = match contactpick::values(app_id) {
            Some(values) => (
                "Choose a Value",
                format!("Choose the value that \u{201c}{app_name}\u{201d} receives. The app gets only this one."),
                values.into_iter().map(|(key, nth, value)| (Tap::Value(key, nth), value)).collect(),
            ),
            None => (
                "Choose a Contact",
                format!("Choose the contact that \u{201c}{app_name}\u{201d} receives. The app gets only the one you choose."),
                contactpick::choices(app_id)?.into_iter().map(|(id, name)| (Tap::Contact(id), name)).collect(),
            ),
        };
        let mut taps = Vec::new();
        let mut labels = Vec::new();
        for (tap, label) in options.into_iter().take(MAX_CHOICES) {
            labels.push(ui.text(&label, MAC_BODY_PT, Weight::Regular, LINK));
            taps.push(tap);
        }
        labels.push(ui.text("Cancel", MAC_BODY_PT, Weight::Regular, LINK));
        taps.push(Tap::Cancel);
        let card = AlertCard::new(ui, title, &detail, labels);
        Some(PickSheet { app_id: app_id.to_string(), app_name: app_name.to_string(), taps, card })
    }

    /// What each button does, left to right.
    pub fn taps(&self) -> &[Tap] {
        &self.taps
    }

    /// The tap under (x, y), if any.
    pub fn hit(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<Tap> {
        self.card.hit(ui, w, h, x, y).map(|i| self.taps[i].clone())
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        self.card.draw(s, ui, w, h);
    }

    /// Applies a tap to the pick and returns the sheet to show next: the same sheet with the contact's values after a
    /// contact tap in a property pick, or `None` when the pick is over (a choice or Cancel).
    pub fn tapped(self, ui: &Ui, tap: Tap) -> Option<PickSheet> {
        let continues = match tap {
            Tap::Contact(id) => matches!(contactpick::select(&self.app_id, id), Ok(false)),
            Tap::Value(key, nth) => {
                let _ = contactpick::choose(&self.app_id, key, nth);
                false
            }
            Tap::Cancel => {
                contactpick::cancel(&self.app_id);
                false
            }
        };
        if continues {
            PickSheet::new(ui, &self.app_id, &self.app_name)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::contact_card::Entry;
    use bifrost_ui::contact_picker::{Mode, Picker};

    fn entry(id: u64, name: &str) -> Entry {
        Entry { id, name: name.into(), fields: Vec::new() }
    }

    #[test]
    fn a_contact_pick_offers_each_contact_then_cancel() {
        let ui = crate::shell::tests::ui();
        let app = "is.example.pick-sheet-contact";
        let entries = vec![entry(1, "Ada"), entry(2, "Grace")];
        contactpick::start(app, "Notes", Picker::new(entries.clone(), Mode::Contact { need: None }), &entries);
        let sheet = PickSheet::new(&ui, app, "Notes").unwrap();
        assert_eq!(sheet.taps(), &[Tap::Contact(1), Tap::Contact(2), Tap::Cancel]);
        assert!(sheet.tapped(&ui, Tap::Contact(2)).is_none());
        assert!(matches!(contactpick::outcome(app), contactpick::Outcome::Chosen(_)));
    }

    #[test]
    fn cancel_ends_the_pick_and_no_pick_means_no_sheet() {
        let ui = crate::shell::tests::ui();
        let app = "is.example.pick-sheet-cancel";
        let entries = vec![entry(1, "Ada")];
        contactpick::start(app, "Notes", Picker::new(entries.clone(), Mode::Contact { need: None }), &entries);
        let sheet = PickSheet::new(&ui, app, "Notes").unwrap();
        assert!(sheet.tapped(&ui, Tap::Cancel).is_none());
        assert!(matches!(contactpick::outcome(app), contactpick::Outcome::None));
        assert!(PickSheet::new(&ui, app, "Notes").is_none());
    }
}
