//! Smart card reader slots: the slot list, the state of each slot and the card in it.
//! Pure model: no reader is opened here. A transport reports each slot's state with `SlotManager::report`,
//! and passes the answer to reset only for a valid card. The model turns card insertions and removals into events.
use crate::token_watch::TokenError;

/// The state of one reader slot. `Probing` is the reader checking what is inserted. A card is usable only in
/// `ValidCard`. A `MuteCard` is inserted but does not answer, so it has no answer to reset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotState {
    Missing,
    Empty,
    Probing,
    MuteCard,
    ValidCard,
}

impl SlotState {
    /// True when a card is inserted, whether it is mute or valid.
    pub fn has_card(self) -> bool {
        matches!(self, SlotState::MuteCard | SlotState::ValidCard)
    }
}

/// A change that a report caused: a card went in or came out. A change between two card states is not an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotEvent {
    CardInserted,
    CardRemoved,
}

/// One reader slot. The answer to reset is kept only while the card is valid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    name: String,
    state: SlotState,
    atr: Option<Vec<u8>>,
}

impl Slot {
    fn new(name: &str) -> Self {
        Slot { name: name.to_string(), state: SlotState::Empty, atr: None }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn state(&self) -> SlotState {
        self.state
    }

    /// The answer to reset of the inserted card. None when the slot is empty, missing or the card is mute.
    pub fn atr(&self) -> Option<&[u8]> {
        self.atr.as_deref()
    }

    /// Applies a report. A valid card must come with a non-empty answer to reset, otherwise the report is refused
    /// and the slot does not change.
    fn observe(&mut self, state: SlotState, atr: Option<Vec<u8>>) -> Result<Option<SlotEvent>, TokenError> {
        if state == SlotState::ValidCard && atr.as_ref().map_or(true, |a| a.is_empty()) {
            return Err(TokenError::BadParameter);
        }
        let had_card = self.state.has_card();
        let has_card = state.has_card();
        self.state = state;
        self.atr = if state == SlotState::ValidCard { atr } else { None };
        Ok(match (had_card, has_card) {
            (false, true) => Some(SlotEvent::CardInserted),
            (true, false) => Some(SlotEvent::CardRemoved),
            _ => None,
        })
    }
}

/// The reader slots of the system, in the order they were added. Slot names are unique.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlotManager {
    slots: Vec<Slot>,
}

impl SlotManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a slot in the state `Empty`. An empty or already used name is refused.
    pub fn add_slot(&mut self, name: &str) -> Result<(), TokenError> {
        if name.is_empty() || self.slots.iter().any(|s| s.name == name) {
            return Err(TokenError::BadParameter);
        }
        self.slots.push(Slot::new(name));
        Ok(())
    }

    pub fn slot(&self, name: &str) -> Option<&Slot> {
        self.slots.iter().find(|s| s.name == name)
    }

    pub fn slot_names(&self) -> Vec<&str> {
        self.slots.iter().map(|s| s.name.as_str()).collect()
    }

    /// Names of the slots that hold a card, mute or valid, in the order the slots were added.
    pub fn slots_with_cards(&self) -> Vec<&str> {
        self.slots.iter().filter(|s| s.state.has_card()).map(|s| s.name.as_str()).collect()
    }

    /// Records the state a transport read from a slot. An unknown slot is `ObjectNotFound`.
    pub fn report(
        &mut self,
        name: &str,
        state: SlotState,
        atr: Option<Vec<u8>>,
    ) -> Result<Option<SlotEvent>, TokenError> {
        let slot = self.slots.iter_mut().find(|s| s.name == name).ok_or(TokenError::ObjectNotFound)?;
        slot.observe(state, atr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reader() -> SlotManager {
        let mut m = SlotManager::new();
        m.add_slot("reader A").unwrap();
        m.add_slot("reader B").unwrap();
        m
    }

    #[test]
    fn slot_names_must_be_unique_and_not_empty() {
        let mut m = reader();
        assert_eq!(m.add_slot("reader A"), Err(TokenError::BadParameter));
        assert_eq!(m.add_slot(""), Err(TokenError::BadParameter));
        assert_eq!(m.slot_names(), vec!["reader A", "reader B"]);
    }

    #[test]
    fn valid_card_inserts_and_keeps_its_answer_to_reset() {
        let mut m = reader();
        let event = m.report("reader A", SlotState::ValidCard, Some(vec![0x3b, 0x8f])).unwrap();
        assert_eq!(event, Some(SlotEvent::CardInserted));
        let slot = m.slot("reader A").unwrap();
        assert_eq!(slot.state(), SlotState::ValidCard);
        assert_eq!(slot.atr(), Some(&[0x3b, 0x8f][..]));
        assert_eq!(m.slots_with_cards(), vec!["reader A"]);
    }

    #[test]
    fn valid_card_without_an_answer_to_reset_is_refused_and_leaves_the_slot_alone() {
        let mut m = reader();
        assert_eq!(m.report("reader B", SlotState::ValidCard, None), Err(TokenError::BadParameter));
        assert_eq!(m.report("reader B", SlotState::ValidCard, Some(Vec::new())), Err(TokenError::BadParameter));
        assert_eq!(m.slot("reader B").unwrap().state(), SlotState::Empty);
    }

    #[test]
    fn mute_card_has_a_card_but_no_answer_to_reset() {
        let mut m = reader();
        let event = m.report("reader A", SlotState::MuteCard, Some(vec![1, 2, 3])).unwrap();
        assert_eq!(event, Some(SlotEvent::CardInserted));
        assert_eq!(m.slot("reader A").unwrap().atr(), None);
        assert!(SlotState::MuteCard.has_card());
        // A mute card that starts answering is still the same card: no insert or remove event.
        let event = m.report("reader A", SlotState::ValidCard, Some(vec![9, 9])).unwrap();
        assert_eq!(event, None);
        assert_eq!(m.slot("reader A").unwrap().atr(), Some(&[9, 9][..]));
    }

    #[test]
    fn removal_and_missing_reader_are_both_card_removed() {
        let mut m = reader();
        m.report("reader A", SlotState::ValidCard, Some(vec![0x3b])).unwrap();
        assert_eq!(m.report("reader A", SlotState::Empty, None), Ok(Some(SlotEvent::CardRemoved)));
        assert_eq!(m.report("reader A", SlotState::Empty, None), Ok(None));
        m.report("reader B", SlotState::ValidCard, Some(vec![0x3b])).unwrap();
        assert_eq!(m.report("reader B", SlotState::Missing, None), Ok(Some(SlotEvent::CardRemoved)));
        assert!(m.slots_with_cards().is_empty());
    }

    #[test]
    fn unknown_slot_is_object_not_found() {
        let mut m = reader();
        assert_eq!(m.report("reader C", SlotState::Probing, None), Err(TokenError::ObjectNotFound));
        assert!(m.slot("reader C").is_none());
    }
}
