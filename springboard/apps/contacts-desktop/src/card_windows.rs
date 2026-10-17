//! Card windows: one contact shown in its own window of the app (Card > Open in New Window). The model keeps which card
//! each extra window shows, on top of the SDK window registry (`bifrost_app::app_windows`). A card that is already open
//! is focused rather than opened twice.
//!
//! The shell does not draw a second window yet (see `app_windows`), so the window UI does not call this model. The
//! model and its tests are ready for that.
#![cfg_attr(not(test), allow(dead_code))]

use bifrost_app::app_windows::{AppWindows, WindowError, WindowId, WindowSpec};

/// The size of a card window, in points.
pub const CARD_WIDTH: f32 = 480.0;
pub const CARD_HEIGHT: f32 = 560.0;

/// The windows that show cards, with the contact each one shows.
#[derive(Debug, Default)]
pub struct CardWindows {
    windows: AppWindows,
    /// (window, contact id), in the order the cards were opened.
    cards: Vec<(WindowId, u64)>,
}

/// The title of a card window: the contact's name, or "Card" for a contact with no name.
pub fn card_title(name: &str) -> String {
    let name = name.trim();
    if name.is_empty() { "Card".to_string() } else { name.to_string() }
}

impl CardWindows {
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens a card in a new window, or focuses the window that already shows it. Returns the window.
    pub fn open_card(&mut self, contact: u64, name: &str) -> Result<WindowId, WindowError> {
        if let Some(window) = self.window_of(contact) {
            self.windows.focus(window)?;
            return Ok(window);
        }
        let window = self.windows.open(WindowSpec::new(&card_title(name), CARD_WIDTH, CARD_HEIGHT))?;
        self.cards.push((window, contact));
        Ok(window)
    }

    /// The window that shows a contact's card, if one is open.
    pub fn window_of(&self, contact: u64) -> Option<WindowId> {
        self.cards.iter().find(|(_, c)| *c == contact).map(|(w, _)| *w)
    }

    /// The contact a card window shows. `None` for the main window and for a window that is closed.
    pub fn card_of(&self, window: WindowId) -> Option<u64> {
        self.cards.iter().find(|(w, _)| *w == window).map(|(_, c)| *c)
    }

    /// Closes a card window and returns the contact it showed.
    pub fn close_card(&mut self, window: WindowId) -> Result<u64, WindowError> {
        let contact = self.card_of(window).ok_or(WindowError::Unknown(window))?;
        self.windows.close(window)?;
        self.cards.retain(|(w, _)| *w != window);
        Ok(contact)
    }

    /// Follows a rename: the window's title becomes the new name. False when the contact has no open card.
    pub fn rename(&mut self, contact: u64, name: &str) -> bool {
        let Some(window) = self.window_of(contact) else { return false };
        let Some(open) = self.windows.get(window) else { return false };
        let (width, height) = (open.width, open.height);
        self.windows.set_frame(window, &card_title(name), width, height).is_ok()
    }

    /// The window registry, for the window list and for routing events.
    pub fn windows(&self) -> &AppWindows {
        &self.windows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_opens_in_its_own_window_titled_with_the_contact_name() {
        let mut cards = CardWindows::new();
        let w = cards.open_card(7, "  Ada Lovelace ").unwrap();
        assert_ne!(w, WindowId::MAIN);
        assert_eq!(cards.card_of(w), Some(7));
        let open = cards.windows().get(w).unwrap();
        assert_eq!(open.title, "Ada Lovelace");
        assert_eq!((open.width, open.height), (CARD_WIDTH, CARD_HEIGHT));
    }

    #[test]
    fn a_nameless_card_gets_the_plain_title() {
        assert_eq!(card_title("   "), "Card");
    }

    #[test]
    fn opening_a_card_twice_focuses_its_window_instead_of_making_another() {
        let mut cards = CardWindows::new();
        let a = cards.open_card(1, "Ada").unwrap();
        let b = cards.open_card(2, "Grace").unwrap();
        assert_eq!(cards.windows().focused(), b);
        let again = cards.open_card(1, "Ada").unwrap();
        assert_eq!(again, a);
        assert_eq!(cards.windows().focused(), a);
        assert_eq!(cards.windows().list().len(), 3, "main, Ada and Grace");
    }

    #[test]
    fn closing_a_card_window_returns_its_contact_and_forgets_it() {
        let mut cards = CardWindows::new();
        let w = cards.open_card(5, "Linus").unwrap();
        assert_eq!(cards.close_card(w), Ok(5));
        assert_eq!(cards.card_of(w), None);
        assert_eq!(cards.window_of(5), None);
        assert_eq!(cards.close_card(w), Err(WindowError::Unknown(w)));
    }

    #[test]
    fn the_main_window_shows_no_card_and_cannot_be_closed_as_one() {
        let mut cards = CardWindows::new();
        assert_eq!(cards.card_of(WindowId::MAIN), None);
        assert_eq!(cards.close_card(WindowId::MAIN), Err(WindowError::Unknown(WindowId::MAIN)));
    }

    #[test]
    fn a_rename_changes_the_title_of_an_open_card_only() {
        let mut cards = CardWindows::new();
        let w = cards.open_card(3, "Old Name").unwrap();
        assert!(cards.rename(3, "New Name"));
        assert_eq!(cards.windows().get(w).unwrap().title, "New Name");
        assert!(!cards.rename(99, "Nobody"));
    }
}
