//! Incoming-call presentation: the update an app reports for a call that rings (`CXCallUpdate`) and the card the
//! shell shows for it (the system incoming-call interface, `CXAnswerCallAction` and `CXEndCallAction` behind its
//! two buttons). A card is a banner while the screen is in use and a full-screen card while it is locked. Drawing
//! belongs to the shell; this module decides the text, the presentation and the call action a button press sends.

use crate::call::{Call, CallAction, CallCenter, CallError, CallState, Handle};

/// What the system knows about a call that rings (`CXCallUpdate`). Hold, group and DTMF default to supported, as
/// they do on the reference platform; video defaults to off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallUpdate {
    pub handle: Handle,
    /// The caller's name from the app. A blank name is treated as absent and the handle value is shown.
    pub display_name: Option<String>,
    pub has_video: bool,
    pub supports_hold: bool,
    pub supports_group: bool,
    pub supports_dtmf: bool,
}

impl CallUpdate {
    pub fn new(handle: Handle) -> Self {
        Self { handle, display_name: None, has_video: false, supports_hold: true, supports_group: true, supports_dtmf: true }
    }

    /// The text the card titles the call with: the caller's name when it is not blank, otherwise the handle value.
    pub fn title(&self) -> String {
        match self.display_name.as_deref().map(str::trim) {
            Some(name) if !name.is_empty() => name.to_string(),
            _ => handle_text(&self.handle).to_string(),
        }
    }
}

fn handle_text(handle: &Handle) -> &str {
    match handle {
        Handle::Phone(value) | Handle::Email(value) | Handle::Generic(value) => value.as_str(),
    }
}

/// How the incoming-call interface is shown (`CXCallUpdate` is shown by the system, not by the app).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presentation {
    /// A banner at the top of the screen, while the device is in use.
    Banner,
    /// A full-screen card, while the screen is locked.
    FullScreen,
}

/// The presentation for the current device state.
pub fn presentation(screen_locked: bool) -> Presentation {
    if screen_locked {
        Presentation::FullScreen
    } else {
        Presentation::Banner
    }
}

/// The two buttons of the incoming-call card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardButton {
    Answer,
    /// Declines a ringing call. It is an end action, so the call leaves the active list.
    Decline,
}

/// The card for one ringing call (`CXAnswerCallAction` and `CXEndCallAction` on its buttons).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncomingCard {
    pub call_id: u64,
    pub title: String,
    pub subtitle: &'static str,
    pub presentation: Presentation,
}

impl IncomingCard {
    /// Builds the card for a call that rings. The update's handle must match the call's handle, and a call that is
    /// not ringing has no card.
    pub fn for_call(call: &Call, update: &CallUpdate, screen_locked: bool) -> Result<Self, CallError> {
        if call.state != CallState::Ringing {
            return Err(CallError::InvalidState);
        }
        if call.handle != update.handle {
            return Err(CallError::InvalidHandle);
        }
        Ok(Self {
            call_id: call.id,
            title: update.title(),
            subtitle: if update.has_video { "Video call" } else { "Call" },
            presentation: presentation(screen_locked),
        })
    }

    /// The call request a button press sends to the call controller: `Answer` or `End`.
    pub fn press(&self, button: CardButton) -> (u64, CallAction) {
        match button {
            CardButton::Answer => (self.call_id, CallAction::Answer),
            CardButton::Decline => (self.call_id, CallAction::End),
        }
    }

    /// True while the card's call still rings in `center`. A card whose call was answered or ended elsewhere is
    /// removed by the shell.
    pub fn is_showing(&self, center: &CallCenter) -> bool {
        center.call(self.call_id).map_or(false, |call| call.state == CallState::Ringing)
    }
}

/// The ringing cards the shell holds, oldest first. The oldest card that still rings is the one on screen; the others
/// wait behind it. The stack keeps no call state of its own: [`CardStack::prune`] drops the cards whose calls the
/// call controller no longer reports as ringing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CardStack {
    cards: Vec<IncomingCard>,
}

impl CardStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a card at the back. Returns `false` when a card for the same call is already held.
    pub fn push(&mut self, card: IncomingCard) -> bool {
        if self.cards.iter().any(|held| held.call_id == card.call_id) {
            return false;
        }
        self.cards.push(card);
        true
    }

    /// The card on screen: the oldest one held.
    pub fn on_screen(&self) -> Option<&IncomingCard> {
        self.cards.first()
    }

    /// The cards waiting behind the one on screen, oldest first.
    pub fn waiting(&self) -> &[IncomingCard] {
        self.cards.get(1..).unwrap_or(&[])
    }

    /// Drops every card whose call no longer rings in `center`, and returns how many were dropped.
    pub fn prune(&mut self, center: &CallCenter) -> usize {
        let before = self.cards.len();
        self.cards.retain(|card| card.is_showing(center));
        before - self.cards.len()
    }

    /// The call request a button press on the card on screen sends. The card stays until [`CardStack::prune`] sees
    /// that its call has left the ringing state, so a request the controller refuses leaves the card in place.
    pub fn press(&self, button: CardButton) -> Option<(u64, CallAction)> {
        self.on_screen().map(|card| card.press(button))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phone(number: &str) -> Handle {
        Handle::Phone(number.to_string())
    }

    #[test]
    fn title_uses_the_name_then_falls_back_to_the_handle() {
        let mut update = CallUpdate::new(phone("5551234"));
        assert_eq!(update.title(), "5551234");
        update.display_name = Some("   ".into());
        assert_eq!(update.title(), "5551234");
        update.display_name = Some(" Ann ".into());
        assert_eq!(update.title(), "Ann");
        assert!(update.supports_hold && update.supports_group && update.supports_dtmf);
        assert!(!update.has_video);
    }

    #[test]
    fn presentation_follows_the_lock_state() {
        assert_eq!(presentation(true), Presentation::FullScreen);
        assert_eq!(presentation(false), Presentation::Banner);
    }

    #[test]
    fn card_answers_or_declines_the_ringing_call() {
        let mut center = CallCenter::new();
        let id = center.report_incoming(phone("5551234")).unwrap();
        let call = center.call(id).unwrap().clone();
        let mut update = CallUpdate::new(phone("5551234"));
        update.has_video = true;

        let card = IncomingCard::for_call(&call, &update, true).unwrap();
        assert_eq!(card.subtitle, "Video call");
        assert_eq!(card.presentation, Presentation::FullScreen);
        assert_eq!(card.press(CardButton::Answer), (id, CallAction::Answer));
        assert_eq!(card.press(CardButton::Decline), (id, CallAction::End));
        assert!(card.is_showing(&center));

        center.perform(id, CallAction::Answer).unwrap();
        assert!(!card.is_showing(&center));
    }

    #[test]
    fn card_is_refused_for_a_connected_call_or_a_different_handle() {
        let mut center = CallCenter::new();
        let id = center.start_outgoing(phone("5551234")).unwrap();
        let dialing = center.call(id).unwrap().clone();
        let update = CallUpdate::new(phone("5551234"));
        assert_eq!(IncomingCard::for_call(&dialing, &update, false), Err(CallError::InvalidState));

        let ringing_id = center.report_incoming(phone("5559999")).unwrap();
        let ringing = center.call(ringing_id).unwrap().clone();
        assert_eq!(
            IncomingCard::for_call(&ringing, &update, false),
            Err(CallError::InvalidHandle)
        );
    }

    #[test]
    fn card_on_an_unlocked_screen_is_a_banner_without_video_subtitle() {
        let mut center = CallCenter::new();
        let id = center.report_incoming(Handle::Generic("app-user-7".into())).unwrap();
        let call = center.call(id).unwrap().clone();
        let mut update = CallUpdate::new(Handle::Generic("app-user-7".into()));
        update.display_name = Some("Ari".into());
        let card = IncomingCard::for_call(&call, &update, false).unwrap();
        assert_eq!(card.title, "Ari");
        assert_eq!(card.subtitle, "Call");
        assert_eq!(card.presentation, Presentation::Banner);
    }
}
