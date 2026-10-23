//! Card windows: which contact each extra window of the app shows (Card > Open in New Window, or a double-click on a card).
//!
//! The window ids come from the SDK (`Ctx::open_window`, which keeps the app's registry and tells the shell). This model
//! only records the contact behind each id, so a card that is already open is found rather than opened twice, and the
//! window is forgotten when the shell or the app closes it.
//!
//! Card > Open in Separate Window and a double-click on a row open a card here (`main.rs`, `open_card_window`), and the
//! window draws the card (`draw_window`, `Contacts::draw_card`). A card window's touches arrive in window points
//! (`WindowTouch`) and become px for `Contacts::touch_card` through `card_touch`.
#![cfg_attr(not(test), allow(dead_code))]

use bifrost_app::app_windows::{WindowId, WindowSpec};
use bifrost_app::kit::Event;
use bifrost_app::TouchKind;

/// The size of a card window, in points.
pub const CARD_WIDTH: f32 = 480.0;
pub const CARD_HEIGHT: f32 = 560.0;

/// The title of a card window: the contact's name, or "Card" for a contact with no name.
pub fn card_title(name: &str) -> String {
    let name = name.trim();
    if name.is_empty() { "Card".to_string() } else { name.to_string() }
}

/// What to ask the SDK for when a card opens in its own window.
pub fn card_spec(name: &str) -> WindowSpec {
    WindowSpec::new(&card_title(name), CARD_WIDTH, CARD_HEIGHT)
}

/// A card window's touch as `Contacts::touch_card` takes it. The shell sends an extra window's touches in the window's
/// points (`ToApp::WindowTouch`), while the card is laid out and hit-tested from px, as the main window's touches are;
/// `px_per_pt` is `ctx.ui.pt(1.0)`. A cancelled touch ends as a release.
pub fn card_touch(kind: TouchKind, x: f32, y: f32, px_per_pt: f32) -> Event {
    let (x, y) = (x * px_per_pt, y * px_per_pt);
    match kind {
        TouchKind::Down => Event::Down(x, y),
        TouchKind::Move => Event::Move(x, y),
        TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
    }
}

/// The card windows: the window of each open card, with the contact it shows, in the order they were opened.
#[derive(Debug, Default)]
pub struct CardWindows {
    cards: Vec<(WindowId, u64)>,
}

impl CardWindows {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that `window` (opened with `card_spec`) shows `contact`.
    pub fn bind(&mut self, window: WindowId, contact: u64) {
        self.cards.retain(|(w, _)| *w != window);
        self.cards.push((window, contact));
    }

    /// Forgets a card window (the shell or the app closed it). Returns the contact it showed.
    pub fn unbind(&mut self, window: WindowId) -> Option<u64> {
        let index = self.cards.iter().position(|(w, _)| *w == window)?;
        Some(self.cards.remove(index).1)
    }

    /// The window that shows a contact's card, if one is open.
    pub fn window_of(&self, contact: u64) -> Option<WindowId> {
        self.cards.iter().find(|(_, c)| *c == contact).map(|(w, _)| *w)
    }

    /// The contact a card window shows. `None` for the main window and for a window that is not a card.
    pub fn card_of(&self, window: WindowId) -> Option<u64> {
        self.cards.iter().find(|(w, _)| *w == window).map(|(_, c)| *c)
    }

    /// How many card windows are open.
    pub fn len(&self) -> usize {
        self.cards.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_touch_in_window_points_becomes_px_at_the_display_scale() {
        // Midget's 1.5625 scale: a touch at (450, 26) pt is under the pointer at (703.125, 40.625) px
        let k = 1200.0 / 768.0;
        assert!(matches!(card_touch(TouchKind::Down, 450.0, 26.0, k), Event::Down(x, y) if x == 703.125 && y == 40.625));
        assert!(matches!(card_touch(TouchKind::Move, 2.0, 4.0, k), Event::Move(x, y) if x == 3.125 && y == 6.25));
        assert!(matches!(card_touch(TouchKind::Cancel, 2.0, 4.0, k), Event::Up(x, y) if x == 3.125 && y == 6.25));
        assert!(matches!(card_touch(TouchKind::Up, 10.0, 20.0, 1.0), Event::Up(x, y) if x == 10.0 && y == 20.0));
    }

    #[test]
    fn a_card_spec_is_titled_with_the_trimmed_name_at_the_card_size() {
        let spec = card_spec("  Ada Lovelace ");
        assert_eq!(spec.title, "Ada Lovelace");
        assert_eq!((spec.width, spec.height), (CARD_WIDTH, CARD_HEIGHT));
    }

    #[test]
    fn a_nameless_card_gets_the_plain_title() {
        assert_eq!(card_title("   "), "Card");
    }

    #[test]
    fn a_bound_card_is_found_by_contact_and_by_window() {
        let mut cards = CardWindows::new();
        cards.bind(WindowId(1), 7);
        cards.bind(WindowId(2), 9);
        assert_eq!(cards.window_of(9), Some(WindowId(2)));
        assert_eq!(cards.card_of(WindowId(1)), Some(7));
        assert_eq!(cards.card_of(WindowId::MAIN), None);
        assert_eq!(cards.window_of(3), None);
        assert_eq!(cards.len(), 2);
    }

    #[test]
    fn unbinding_forgets_the_window_once() {
        let mut cards = CardWindows::new();
        cards.bind(WindowId(1), 7);
        assert_eq!(cards.unbind(WindowId(1)), Some(7));
        assert_eq!(cards.unbind(WindowId(1)), None);
        assert_eq!(cards.window_of(7), None);
        assert!(cards.is_empty());
    }

    #[test]
    fn binding_a_window_again_replaces_its_contact() {
        let mut cards = CardWindows::new();
        cards.bind(WindowId(1), 7);
        cards.bind(WindowId(1), 8);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards.card_of(WindowId(1)), Some(8));
    }
}
